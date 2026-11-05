use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;

use crate::{
    host::{decode, encode},
    protocol::{self, Format},
    wire, Fault, Result, Runtime, PROVIDER,
};

pub const HEADER: &str = "X-Factory-Droid-Approval";
const MAX_APPROVALS: usize = 4096;
const TTL: Duration = Duration::from_secs(60);
const COUNT_PATH: &str = "/v1/messages/count_tokens";

struct Approval {
    auth_id: String,
    model: String,
    source: String,
    path: String,
    stream: bool,
    created: Instant,
    identity: Option<wire::Identity>,
}
pub struct ApprovedRequest {
    pub headers: Value,
    pub identity: Option<wire::Identity>,
}
#[derive(Default)]
pub struct Approvals {
    entries: HashMap<String, Approval>,
}
impl Approvals {
    fn issue(
        &mut self,
        auth_id: &str,
        model: &str,
        source: &str,
        path: &str,
        stream: bool,
        identity: Option<wire::Identity>,
    ) -> Result<String> {
        self.entries.retain(|_, a| a.created.elapsed() < TTL);
        if self.entries.len() >= MAX_APPROVALS {
            return Err(Fault::new(
                503,
                "protocol_guard_failed",
                "原生协议校验并发已达上限，请稍后重试",
            ));
        }
        let nonce = Uuid::new_v4().to_string();
        self.entries.insert(
            nonce.clone(),
            Approval {
                auth_id: auth_id.into(),
                model: model.into(),
                source: source.into(),
                path: path.into(),
                stream,
                created: Instant::now(),
                identity,
            },
        );
        Ok(nonce)
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    fn consume(
        &mut self,
        nonce: &str,
        data: &Value,
        expected: Format,
        path: &str,
    ) -> Result<Option<wire::Identity>> {
        let approved = self.entries.remove(nonce).ok_or_else(missing)?;
        if approved.created.elapsed() >= TTL
            || data["AuthID"].as_str() != Some(&approved.auth_id)
            || data["Model"].as_str() != Some(&approved.model)
            || data["SourceFormat"].as_str() != Some(&approved.source)
            || approved.source != expected.cpa()
            || data["Metadata"]["request_path"].as_str() != Some(&approved.path)
            || approved.path != path
            || data["Stream"].as_bool().unwrap_or(false) != approved.stream
        {
            return Err(missing());
        }
        Ok(approved.identity)
    }
}
fn missing() -> Fault {
    Fault::new(
        400,
        "protocol_guard_missing",
        "缺少有效的一次性原生协议校验凭证，请通过该模型对应的标准 CPA HTTP 端点调用",
    )
}
pub fn error_body(error: &Fault) -> Value {
    let kind = match error.http_status {
        401 => "authentication_error",
        403 => "permission_error",
        404 => "not_found_error",
        413 => "request_too_large",
        429 => "rate_limit_error",
        529 => "overloaded_error",
        500..=599 => "api_error",
        _ => "invalid_request_error",
    };
    let kind = error.upstream_type.as_deref().unwrap_or(kind);
    let mut body = json!({"error":{"type":kind,"code":error.code,"message":error.message}});
    if let Some(kind) = &error.upstream_type {
        body["error"]["upstream_type"] = json!(kind);
    }
    if let Some(id) = &error.request_id {
        body["request_id"] = json!(id);
    }
    body
}
pub fn http_fault(mut error: Fault) -> Fault {
    error.message = error
        .response_body
        .take()
        .unwrap_or_else(|| error_body(&error).to_string());
    error
}
pub fn terminate(error: Fault) -> Value {
    json!({"Terminate":true,"StatusCode":error.http_status,
        "ResponseHeaders":{"Content-Type":["application/json; charset=utf-8"],"Cache-Control":["no-store"]},
        "ResponseBody":encode(error_body(&error).to_string())})
}
pub fn header_values<'a>(headers: &'a Value, name: &str) -> Vec<&'a str> {
    headers
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| key.eq_ignore_ascii_case(name))
        .flat_map(|(_, value)| {
            value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
        })
        .collect()
}
pub fn clean_headers(headers: &Value) -> Value {
    let mut clean = headers.clone();
    if let Some(map) = clean.as_object_mut() {
        map.retain(|key, _| !key.eq_ignore_ascii_case(HEADER));
    }
    clean
}
fn check_route(data: &Value, expected: Format, path: &str) -> Result<()> {
    if data["Metadata"]["request_path"].as_str() != Some(path) {
        return Err(expected.mismatch(
            data["SourceFormat"].as_str().unwrap_or("未知"),
            "请求路径或操作不匹配",
        ));
    }
    if path == COUNT_PATH && data["Stream"].as_bool() == Some(true) {
        return Err(Fault::input("token 计数不支持流式请求"));
    }
    // Proxies can send Connection: upgrade on ordinary HTTP requests.
    // Require an explicit protocol offer before rejecting the request.
    let requests_upgrade = header_values(&data["Headers"], "Upgrade")
        .iter()
        .any(|v| !v.trim().is_empty());
    if requests_upgrade {
        return Err(Fault::new(
            400,
            "unsupported_operation",
            "Factory Droid 只支持对应的标准 HTTP JSON/SSE 调用，不支持 WebSocket/Upgrade",
        ));
    }
    Ok(())
}
pub fn validate(rt: &Runtime, data: &Value, expected: Format) -> Result<ApprovedRequest> {
    validate_path(rt, data, expected, expected.endpoint())
}
pub fn validate_count(rt: &Runtime, data: &Value, expected: Format) -> Result<ApprovedRequest> {
    if expected != Format::Claude {
        return Err(Fault::new(
            400,
            "unsupported_operation",
            "仅支持 Messages 原生 token 计数",
        ));
    }
    validate_path(rt, data, expected, COUNT_PATH)
}
fn validate_path(
    rt: &Runtime,
    data: &Value,
    expected: Format,
    path: &str,
) -> Result<ApprovedRequest> {
    expected.check_formats(data)?;
    check_route(data, expected, path)?;
    if data["AuthProvider"].as_str() != Some(PROVIDER)
        || data["AuthID"].as_str().is_none_or(str::is_empty)
    {
        return Err(missing());
    }
    if data["Alt"].as_str().is_some_and(|alt| !alt.is_empty()) {
        return Err(Fault::new(
            400,
            "unsupported_operation",
            "Factory Droid 未实现此原生操作后缀，请使用模型对应的标准生成端点",
        ));
    }
    let values = header_values(&data["Headers"], HEADER);
    if values.len() != 1 || values[0].len() > 64 {
        return Err(missing());
    }
    let identity = rt
        .approvals
        .lock()
        .map_err(|_| Fault::new(503, "protocol_guard_failed", "原生协议校验状态不可用"))?
        .consume(values[0], data, expected, path)?;
    Ok(ApprovedRequest {
        headers: clean_headers(&data["Headers"]),
        identity,
    })
}
fn known_factory_auth(rt: &Runtime, auth_id: &str) -> bool {
    !auth_id.is_empty()
        && rt
            .lock()
            .ok()
            .is_some_and(|store| store.account_for_auth(auth_id).is_some())
}
pub fn after(rt: &Arc<Runtime>, data: &Value) -> Value {
    let auth_id = data["Metadata"]["selected_auth_id"].as_str().unwrap_or("");
    let auth_index = data["Metadata"]["selected_auth_index"]
        .as_str()
        .unwrap_or("");
    let mut factory_scope = known_factory_auth(rt, auth_id);
    let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Value> {
        if auth_index.is_empty() {
            return if factory_scope {
                Err(Fault::new(
                    503,
                    "protocol_guard_failed",
                    "Factory 凭据缺少宿主运行时索引，已停止请求",
                ))
            } else {
                Ok(json!({}))
            };
        }
        let runtime = match rt
            .host
            .call("host.auth.get_runtime", &json!({"auth_index":auth_index}))
        {
            Ok(v) => v,
            Err(_) => {
                return if factory_scope {
                    Err(Fault::new(
                        503,
                        "protocol_guard_failed",
                        "无法确认 Factory 凭据运行时状态，已停止请求",
                    ))
                } else {
                    Ok(json!({}))
                }
            }
        };
        let auth = &runtime["auth"];
        let provider = auth["provider"]
            .as_str()
            .or_else(|| auth["type"].as_str())
            .unwrap_or("");
        if provider.is_empty() && factory_scope {
            return Err(Fault::new(
                503,
                "protocol_guard_failed",
                "Factory 宿主运行时缺少 provider，已停止请求",
            ));
        }
        if provider != PROVIDER {
            factory_scope = false;
            return Ok(json!({}));
        }
        factory_scope = true;
        if auth_id.is_empty()
            || auth["id"].as_str() != Some(auth_id)
            || auth["auth_index"].as_str() != Some(auth_index)
        {
            return Err(Fault::new(
                503,
                "protocol_guard_failed",
                "Factory 凭据标识与宿主运行时不一致，已停止请求",
            ));
        }
        if auth["disabled"].as_bool() == Some(true) {
            return Err(Fault::new(403, "account_disabled", "Factory 账号已停用"));
        }
        let model_id = data["Model"].as_str().unwrap_or("");
        let model = rt.model(model_id)?;
        let expected = Format::for_model(&model)?;
        let source = data["SourceFormat"].as_str().unwrap_or("");
        if source != expected.cpa() || data["ToFormat"].as_str() != Some(expected.cpa()) {
            return Err(expected.mismatch(source, "调用协议与模型协议不匹配"));
        }
        let path = if expected == Format::Claude
            && data["Metadata"]["request_path"].as_str() == Some(COUNT_PATH)
        {
            COUNT_PATH
        } else {
            expected.endpoint()
        };
        check_route(data, expected, path)?;
        let (credential, selected) = {
            let store = rt.lock()?;
            let account = store.account_for_auth(auth_id).ok_or_else(|| {
                Fault::new(
                    403,
                    "model_not_allowed",
                    "Factory账号尚未完成当前模型的允许验证",
                )
            })?;
            store.ensure_model_allowed(&account.id, &model)?;
            let selected = store
                .account_model(&account.id, &model.id)
                .ok_or_else(missing)?;
            (json!({"account_id":account.id}), selected)
        };
        let stream = data["Stream"].as_bool().unwrap_or(false);
        let mut headers = json!({});
        // Factory wrapping happens before CPA applies the final user payload rules.
        // Keep only the opaque identity in the one-use approval, never the body.
        let (body, identity) = if path == COUNT_PATH {
            (None, None)
        } else {
            let request = protocol::request(&decode(&data["Body"])?, &selected, stream)?;
            let identity = wire::identity(
                rt,
                &credential,
                &data["Headers"],
                &data["Metadata"],
                &request,
            )?;
            headers = wire::request_beta_headers(&data["Headers"], &selected, &request)?;
            let prepared = wire::payload(&request, &selected, &identity)?;
            (Some(encode(prepared.to_string())), Some(identity))
        };
        let nonce = rt
            .approvals
            .lock()
            .map_err(|_| Fault::new(503, "protocol_guard_failed", "原生协议校验状态不可用"))?
            .issue(auth_id, model_id, source, path, stream, identity)?;
        headers[HEADER] = json!([nonce]);
        let mut result = json!({"ClearHeaders":[HEADER],"Headers":headers});
        if let Some(body) = body {
            result["Body"] = json!(body);
        }
        Ok(result)
    }));
    match attempt {
        Ok(Ok(value)) => value,
        Ok(Err(error)) if factory_scope => terminate(error),
        Err(_) if factory_scope => terminate(Fault::new(
            503,
            "protocol_guard_failed",
            "Factory 原生协议校验失败，已停止请求",
        )),
        _ => json!({}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_and_generation_approvals_cannot_be_replayed_across_paths() {
        let mut approvals = Approvals::default();
        for (issued, attempted) in [(COUNT_PATH, "/v1/messages"), ("/v1/messages", COUNT_PATH)] {
            let nonce = approvals
                .issue("account", "model", "claude", issued, false, None)
                .unwrap();
            let data = json!({"AuthID":"account","Model":"model","SourceFormat":"claude",
                "Stream":false,"Metadata":{"request_path":attempted}});
            assert!(approvals
                .consume(&nonce, &data, Format::Claude, attempted)
                .is_err());
        }
        let nonce = approvals
            .issue("account", "model", "claude", COUNT_PATH, false, None)
            .unwrap();
        let data = json!({"AuthID":"account","Model":"model","SourceFormat":"claude",
            "Stream":false,"Metadata":{"request_path":COUNT_PATH}});
        assert!(approvals
            .consume(&nonce, &data, Format::Claude, COUNT_PATH)
            .is_ok());
        assert!(approvals
            .consume(&nonce, &data, Format::Claude, COUNT_PATH)
            .is_err());
    }
}
