use serde_json::{json, Value};
use std::sync::Arc;
use url::Url;
use uuid::Uuid;

use crate::{
    host::{decode, encode},
    state::{now, Account, Model},
    Fault, Operation, Result, Runtime, PROVIDER,
};

pub fn validate_base(value: &str, local: bool) -> Result<String> {
    let url = Url::parse(value).map_err(|_| Fault::input("Factory API 地址无效"))?;
    let name = url.host_str().unwrap_or("");
    let factory =
        name == "api.factory.ai" || name == "api.eu.factory.ai" || name.ends_with(".factory.ai");
    let mock = local && ["localhost", "127.0.0.1", "[::1]", "::1"].contains(&name);
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !["", "/"].contains(&url.path())
        || !(factory && url.scheme() == "https" && url.port().is_none_or(|p| p == 443)
            || mock && ["http", "https"].contains(&url.scheme()))
    {
        return Err(Fault::input(
            "Factory 地址必须使用受信任 Factory HTTPS 域名；测试模式仅允许本机地址",
        ));
    }
    Ok(value.trim_end_matches('/').to_string())
}

pub fn credential(data: &Value, field: &str) -> Result<Value> {
    let bytes = decode(&data[field])?;
    let cred: Value = serde_json::from_slice(&bytes)
        .map_err(|_| Fault::new(401, "credential_invalid", "Factory 凭据无效，请重新授权"))?;
    if cred["type"].as_str() != Some(PROVIDER) {
        return Err(Fault::new(
            401,
            "credential_invalid",
            "凭据不属于 Factory Droid",
        ));
    }
    Ok(cred)
}
pub fn parse_auth(data: &Value) -> Result<Value> {
    let bytes = decode(&data["RawJSON"])?;
    let cred: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return Ok(json!({"Handled":false})),
    };
    if cred["type"].as_str() != Some(PROVIDER) {
        return Ok(json!({"Handled":false}));
    }
    if cred["deleted"].as_bool() != Some(true) {
        validate_key(cred["api_key"].as_str().unwrap_or(""))?;
    }
    let id = cred["account_id"]
        .as_str()
        .filter(|s| valid_id(s))
        .ok_or_else(|| Fault::input("Factory 账号 ID 无效"))?;
    let file = data["FileName"].as_str().unwrap_or("");
    let file = std::path::Path::new(file)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if !file.is_empty() && !valid_auth_filename(file) {
        return Err(Fault::input("Factory凭据文件名不安全"));
    }
    // Parsing can run before the host has committed a credential save. Keep it pure;
    // only a successful management save or model registration synchronizes local state.
    let auth = auth_data(&cred, data["FileName"].as_str().unwrap_or(file), file);
    let _ = id;
    Ok(json!({"Handled":true,"Auth":auth}))
}
pub fn auth_data(cred: &Value, id: &str, file: &str) -> Value {
    json!({"Provider":PROVIDER,"ID":id,"FileName":file,
        "Label":cred["label"].as_str().unwrap_or("Factory"),
        "Disabled":cred["disabled"].as_bool().unwrap_or(false) || cred["deleted"].as_bool().unwrap_or(false),
        "StorageJSON":encode(cred.to_string()),
        "Metadata":{"type":PROVIDER,"factory_account_id":cred["account_id"],"label":cred["label"],
            "email":cred["email"],"disabled":cred["disabled"].as_bool().unwrap_or(false),
            "factory_catalog_revision":cred["factory_catalog_revision"],
            "priority":cred["priority"].as_i64().unwrap_or(0)},
        "Attributes":{"factory_account_id":cred["account_id"].as_str().unwrap_or(""),
            "priority":cred["priority"].as_i64().unwrap_or(0).to_string()},
        "NextRefreshAfter":(chrono::Utc::now()+chrono::Duration::hours(24)).to_rfc3339()})
}
pub fn validate_key(key: &str) -> Result<()> {
    if !key.starts_with("fk-")
        || key.len() < 12
        || key.len() > 4096
        || !key.bytes().all(|b| b.is_ascii_graphic())
    {
        return Err(Fault::input("请输入有效的 Factory API Key（fk- 开头）"));
    }
    Ok(())
}
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() < 100 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
pub fn ensure_enabled(rt: &Runtime, cred: &Value) -> Result<()> {
    if cred["disabled"].as_bool() == Some(true) || cred["deleted"].as_bool() == Some(true) {
        return Err(Fault::new(403, "account_disabled", "Factory 账号已停用"));
    }
    if let Some(account) = rt
        .lock()?
        .accounts
        .get(cred["account_id"].as_str().unwrap_or(""))
    {
        if !account.enabled || account.deleted {
            return Err(Fault::new(
                403,
                "account_disabled",
                "Factory 账号已停用或删除",
            ));
        }
        if account.credential_fingerprint != crate::access::credential_fingerprint(cred) {
            return Err(Fault::new(
                401,
                "credential_changed",
                "Factory 凭据已改变，请刷新当前账号后重试",
            ));
        }
    } else {
        return Err(Fault::new(
            403,
            "model_not_allowed",
            "Factory 账号尚未完成本插件的模型验证",
        ));
    }
    validate_key(cred["api_key"].as_str().unwrap_or(""))
}
pub fn key_hint(key: &str) -> String {
    let last = key
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("fk-••••••{last}")
}
pub fn upsert_account(rt: &Runtime, cred: &Value, file: &str, checked: bool) -> Result<()> {
    upsert_account_version(rt, cred, file, checked, None)
}
fn upsert_account_version(
    rt: &Runtime,
    cred: &Value,
    file: &str,
    checked: bool,
    expected: Option<u64>,
) -> Result<()> {
    let id = cred["account_id"]
        .as_str()
        .ok_or_else(|| Fault::input("账号 ID 缺失"))?
        .to_string();
    let mut store = rt.lock()?;
    if store
        .settings
        .models
        .iter()
        .any(|m| m.id.starts_with(&format!("factory/{id}/")))
    {
        return Err(Fault::input(
            "账号 ID 与模型映射的固定账号命名空间冲突，请使用其他账号 ID",
        ));
    }
    if let Some(expected) = expected {
        let current = store.accounts.get(&id).ok_or_else(|| {
            Fault::new(409, "credential_changed", "账号已移除，已忽略过期刷新结果")
        })?;
        if current.revision != expected || current.deleted || !current.enabled {
            return Err(Fault::new(
                409,
                "credential_changed",
                "账号已修改或停用，已忽略过期刷新结果",
            ));
        }
    }
    let enabled = cred["disabled"].as_bool() != Some(true);
    let deleted = cred["deleted"].as_bool() == Some(true);
    let mut account = store.accounts.get(&id).cloned().unwrap_or_else(|| Account {
        id: id.clone(),
        label: "Factory".into(),
        kind: "api_key".into(),
        enabled,
        priority: 0,
        credential_hint: String::new(),
        auth_file: file.into(),
        status: "pending".into(),
        last_error: None,
        created_at: cred["created_at"].as_str().unwrap_or(&now()).into(),
        updated_at: now(),
        last_checked_at: None,
        identity: json!({}),
        official_usage: unavailable_usage("not_checked", "尚未查询官方用量"),
        official_limits: unavailable_limits("not_checked", "尚未查询官方额度"),
        deleted,
        revision: 0,
        identity_verified: false,
        credential_generation: 0,
        credential_fingerprint: String::new(),
        discovery: Default::default(),
        model_access: Default::default(),
        selected_models: Default::default(),
        model_overrides: Default::default(),
    });
    if account.deleted && !deleted {
        return Err(Fault::new(
            409,
            "account_removed",
            "账号已删除，已忽略迟到的凭据同步",
        ));
    }
    if account.enabled != enabled {
        account.discovery.generation = account.discovery.generation.saturating_add(1);
        for access in account.model_access.values_mut() {
            access.operation_generation = access.operation_generation.saturating_add(1);
            if access.verification_status == "validating" {
                access.verification_status = "not_verified".into();
            }
        }
    }
    let fingerprint = crate::access::credential_fingerprint(cred);
    if account.credential_fingerprint != fingerprint {
        crate::access::cancel_discovery(rt, &id);
        account.credential_fingerprint = fingerprint;
        account.credential_generation = account.credential_generation.saturating_add(1);
        account.identity_verified = false;
        account.discovery.stale = true;
        account.discovery.generation = account.discovery.generation.saturating_add(1);
        for access in account.model_access.values_mut() {
            access.allowed = false;
            access.verification_status = "invalidated".into();
            access.last_error = Some("credential_changed".into());
            access.operation_generation = access.operation_generation.saturating_add(1);
        }
        rt.catalog_dirty
            .store(true, std::sync::atomic::Ordering::Release);
    }
    account.label = cred["label"]
        .as_str()
        .unwrap_or("Factory")
        .chars()
        .take(80)
        .collect();
    account.enabled = enabled && !deleted;
    account.revision = account.revision.saturating_add(1);
    account.deleted = deleted;
    account.priority = cred["priority"].as_i64().unwrap_or(0).clamp(-100, 100);
    account.credential_hint = key_hint(cred["api_key"].as_str().unwrap_or(""));
    account.auth_file = file.into();
    account.updated_at = now();
    account.identity =
        json!({"email":cred["email"],"organization":cred["org_id"],"region":cred["region"]});
    if checked {
        account.identity_verified = true;
        account.last_checked_at = Some(now());
        account.last_error = None;
        account.status = "ready".into();
    }
    if account.last_checked_at.is_none() {
        account.last_checked_at = cred["last_checked_at"].as_str().map(str::to_string);
    }
    if !account.enabled {
        account.status = "disabled".into();
    } else if account.status == "disabled" || account.status == "pending" {
        account.status = if account.last_error.is_some() {
            "error"
        } else if account.last_checked_at.is_some() {
            "ready"
        } else {
            "pending"
        }
        .into();
    }
    let previous = store.accounts.insert(id.clone(), account);
    if let Err(error) = store.persist() {
        if let Some(previous) = previous {
            store.accounts.insert(id, previous);
        } else {
            store.accounts.remove(&id);
        }
        return Err(error);
    }
    Ok(())
}
pub fn observe_model_account(rt: &Runtime, cred: &Value, file: &str) -> Result<bool> {
    if cred["deleted"] == true {
        return Ok(false);
    }
    validate_key(cred["api_key"].as_str().unwrap_or(""))?;
    let id = cred["account_id"]
        .as_str()
        .filter(|id| valid_id(id))
        .ok_or_else(|| Fault::input("Factory账号ID无效"))?;
    let existing = rt.lock()?.accounts.get(id).cloned();
    // Late callbacks from an already removed/modified credential cannot resurrect it.
    if let Some(account) = &existing {
        if account.deleted {
            return Ok(false);
        }
        if account.credential_fingerprint != crate::access::credential_fingerprint(cred) {
            return Ok(false);
        }
        if account.enabled != (cred["disabled"].as_bool() != Some(true)) {
            return Ok(false);
        }
        return Ok(true);
    }
    if cred["deleted"].as_bool() == Some(true) {
        return Ok(false);
    }
    let fallback = format!("factory-droid-{id}.json");
    if !file.is_empty() && !valid_auth_filename(file) {
        return Err(Fault::input("Factory凭据文件名不安全"));
    }
    let file = if file.is_empty() { &fallback } else { file };
    upsert_account(rt, cred, file, false)?;
    Ok(true)
}
pub fn valid_auth_filename(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 220
        && name.ends_with(".json")
        && !name.starts_with('.')
        && !name
            .chars()
            .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
        && ![
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ]
        .contains(
            &name
                .split('.')
                .next()
                .unwrap_or("")
                .to_ascii_uppercase()
                .as_str(),
        )
}
pub fn unavailable_usage(status: &str, message: &str) -> Value {
    json!({"status":status,"source":"factory_analytics","unit":"FSC","used":null,"window_start":null,"window_end":null,"checked_at":null,"message":message})
}
pub fn unavailable_limits(status: &str, message: &str) -> Value {
    json!({"status":status,"source":"factory_billing_limits","windows":[],"extra_usage_balance_usd":null,"checked_at":null,"message":message})
}
fn headers(rt: &Runtime, cred: &Value) -> Value {
    json!({"Authorization":[format!("Bearer {}",cred["api_key"].as_str().unwrap_or(""))],
        "Accept":["application/json"],"X-Factory-Client":["cli"],"X-Client-Version":[rt.client_version()],
        "User-Agent":[format!("factory-cli/{}",rt.client_version())]})
}
pub fn account_base(rt: &Runtime, cred: &Value) -> String {
    if rt.config.allow_local_mock
        && Url::parse(rt.api_base()).ok().is_some_and(|u| {
            ["localhost", "127.0.0.1", "[::1]", "::1"].contains(&u.host_str().unwrap_or(""))
        })
    {
        return rt.api_base().into();
    }
    if cred["region"]
        .as_str()
        .is_some_and(|s| s.eq_ignore_ascii_case("eu"))
    {
        "https://api.eu.factory.ai".into()
    } else {
        rt.api_base().into()
    }
}
pub fn llm_base(rt: &Runtime, cred: &Value) -> Result<String> {
    let base = account_base(rt, cred);
    if base.starts_with("http://localhost")
        || base.starts_with("http://127.0.0.1")
        || base.starts_with("http://[::1]")
    {
        return Ok(base);
    }
    if let Some(raw) = cred["prem_base_host"].as_str().filter(|s| !s.is_empty()) {
        let with_scheme = if raw.contains("://") {
            raw.into()
        } else {
            format!("https://{raw}")
        };
        return validate_base(&with_scheme, false);
    }
    Ok(base)
}
pub fn llm_request(
    rt: &Runtime,
    cred: &Value,
    model: &Model,
    request: &Value,
    request_id: &str,
    op: &Operation,
    native_headers: &Value,
) -> Result<Value> {
    let base = llm_base(rt, cred)?;
    let path = match model.protocol.as_str() {
        "anthropic" => "/api/llm/a/v1/messages",
        "responses" => "/api/llm/o/v1/responses",
        _ => "/api/llm/o/v1/chat/completions",
    };
    let mut h = headers(rt, cred);
    let allowlist: &[&str] = if model.protocol == "anthropic" {
        &["anthropic-version", "anthropic-beta"]
    } else {
        &["OpenAI-Beta"]
    };
    for name in allowlist {
        let values = crate::guard::header_values(native_headers, name);
        if !values.is_empty() {
            if values.iter().any(|value| {
                value.len() > 8192 || value.bytes().any(|b| b < 0x20 && b != b'\t' || b == 0x7f)
            }) {
                return Err(Fault::new(
                    400,
                    "invalid_native_request",
                    "协议语义请求头无效",
                ));
            }
            h[*name] = json!(values);
        }
    }
    crate::catalog::apply_required_headers(&mut h, model);
    h["Content-Type"] = json!(["application/json"]);
    h["x-api-provider"] = json!([model.upstream_provider]);
    h["x-session-id"] = json!([format!(
        "cpa-{}",
        cred["account_id"].as_str().unwrap_or("factory")
    )]);
    h["x-assistant-message-id"] = json!([request_id]);
    h["x-provider-routing-source"] = json!(["registry_default"]);
    if model.protocol == "anthropic" {
        h["X-Api-Key"] = json!(["placeholder"]);
        if h["anthropic-version"].is_null() {
            h["anthropic-version"] = json!(["2023-06-01"]);
        }
    }
    if model.upstream_provider == "openai" {
        h["OpenAI-Platform"] = json!(["org-bHuLtG1fGmYk5YaOihAAXFBw"]);
    }
    Ok(
        json!({"host_callback_id":op.callback,"operation_id":op.id,"method":"POST","url":format!("{base}{path}"),"headers":h,"body":encode(request.to_string())}),
    )
}
pub fn upstream_error(status: u16, headers: &Value) -> Fault {
    let message = match status {
        401 => "Factory 凭据已失效，请检查 API Key 或重新授权",
        403 => "Factory 拒绝请求：请检查账号模型权限、组织或请求内容",
        404 => "Factory 模型或接口不存在，请检查模型映射",
        413 => "请求超过 Factory 大小限制，请缩短会话或减少附件",
        429 => "Factory 请求频率或额度受限，请稍后重试",
        _ => "Factory 上游请求失败，请稍后重试",
    };
    let mut error = Fault::new(
        if (400..=599).contains(&status) {
            status
        } else {
            502
        },
        "factory_upstream_error",
        message,
    );
    error.retry_after_seconds = headers
        .as_object()
        .and_then(|o| {
            o.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("retry-after"))
        })
        .and_then(|(_, v)| v[0].as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .map(|s| s.min(86400));
    error
}
pub fn get(
    rt: &Arc<Runtime>,
    cred: &Value,
    path: &str,
    callback: &str,
    timeout: u64,
    extra: Option<Value>,
) -> Result<(u16, Value)> {
    let op = rt.operation(callback, timeout)?;
    let mut h = headers(rt, cred);
    if let Some(extra) = extra.and_then(|v| v.as_object().cloned()) {
        for (k, v) in extra {
            h[&k] = v;
        }
    }
    let response = rt.host.call(
        "host.http.do",
        &json!({"host_callback_id":callback,"operation_id":op.id,
        "method":"GET","url":format!("{}{}",account_base(rt,cred),path),"headers":h}),
    )?;
    let status = response["StatusCode"].as_u64().unwrap_or(502) as u16;
    let bytes = decode(&response["Body"])?;
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok((status, value))
}
pub fn whoami(rt: &Arc<Runtime>, cred: &mut Value, callback: &str) -> Result<()> {
    let (status, value) = get(
        rt,
        cred,
        "/api/cli/whoami",
        callback,
        15,
        Some(json!({"X-Factory-Whoami-Extended":["true"]})),
    )?;
    if status != 200 {
        return Err(upstream_error(status, &json!({})));
    }
    if !value.is_object()
        || !["orgId", "userId", "email", "user"]
            .iter()
            .any(|k| !value[k].is_null())
    {
        return Err(Fault::new(
            502,
            "factory_identity_invalid",
            "Factory 身份响应格式无法识别",
        ));
    }
    let secret = cred["api_key"].as_str().unwrap_or("");
    let sensitive = |value: &Value| {
        value.as_str().is_some_and(|text| {
            !secret.is_empty() && text.contains(secret)
                || text.contains("fk-")
                || text.contains("sk-")
        })
    };
    if [
        "orgId",
        "userId",
        "email",
        "region",
        "inferenceRegion",
        "principalKind",
        "premBaseHostV2",
    ]
    .iter()
    .any(|field| sensitive(&value[*field]))
        || sensitive(&value["user"]["email"])
    {
        return Err(Fault::new(
            502,
            "factory_identity_invalid",
            "Factory身份响应包含敏感凭据样式的元数据，已拒绝保存",
        ));
    }
    if let Some(raw) = value["premBaseHostV2"].as_str().filter(|s| !s.is_empty()) {
        let proposed = if raw.contains("://") {
            raw.to_string()
        } else {
            format!("https://{raw}")
        };
        validate_base(&proposed, false).map_err(|_| {
            Fault::new(
                502,
                "factory_host_untrusted",
                "Factory 返回的独立主机不在受信任域名内，请联系管理员确认",
            )
        })?;
        cred["prem_base_host"] = json!(raw);
    } else {
        cred["prem_base_host"] = Value::Null;
    }
    cred["region"] = value["region"]
        .as_str()
        .map(|s| json!(s.chars().take(20).collect::<String>()))
        .unwrap_or(json!("us"));
    cred["org_id"] = safe_string(&value["orgId"], 200);
    cred["email"] = safe_string(
        if value["email"].is_string() {
            &value["email"]
        } else {
            &value["user"]["email"]
        },
        200,
    );
    cred["user_id"] = safe_string(&value["userId"], 200);
    cred["principal_kind"] = safe_string(&value["principalKind"], 64);
    cred["inference_region"] = safe_string(&value["inferenceRegion"], 32);
    cred["last_checked_at"] = json!(now());
    Ok(())
}
fn safe_string(value: &Value, max: usize) -> Value {
    value
        .as_str()
        .filter(|text| !text.contains("fk-") && !text.contains("sk-"))
        .map(|s| {
            json!(s
                .chars()
                .filter(|c| !c.is_control())
                .take(max)
                .collect::<String>())
        })
        .unwrap_or(Value::Null)
}
pub fn refresh_auth(rt: &Arc<Runtime>, data: &Value) -> Result<Value> {
    let mut cred = credential(data, "StorageJSON")?;
    if cred["deleted"].as_bool() == Some(true) {
        return Err(Fault::new(401, "credential_deleted", "Factory 账号已删除"));
    }
    let id = cred["account_id"]
        .as_str()
        .ok_or_else(|| Fault::input("账号 ID 缺失"))?
        .to_string();
    let revision = {
        let store = rt.lock()?;
        store
            .accounts
            .get(&id)
            .filter(|a| !a.deleted && a.enabled)
            .ok_or_else(|| Fault::new(409, "account_disabled", "账号已删除或停用，未执行刷新"))?
            .revision
    };
    if let Err(error) = whoami(
        rt,
        &mut cred,
        data["host_callback_id"].as_str().unwrap_or(""),
    ) {
        if [401, 403].contains(&error.http_status) {
            crate::access::identity_error(rt, &id, &error.message);
        }
        return Err(error);
    }
    let name = rt
        .lock()?
        .accounts
        .get(cred["account_id"].as_str().unwrap_or(""))
        .map(|a| a.auth_file.clone())
        .filter(|name| valid_auth_filename(name))
        .ok_or_else(|| Fault::input("Factory凭据文件名无效"))?;
    upsert_account_version(rt, &cred, &name, true, Some(revision))?;
    Ok(
        json!({"Auth":auth_data(&cred,data["AuthID"].as_str().unwrap_or(&name),&name),
        "NextRefreshAfter":(chrono::Utc::now()+chrono::Duration::hours(24)).to_rfc3339()}),
    )
}
pub fn new_credential(key: &str, label: &str, enabled: bool) -> Value {
    json!({"type":PROVIDER,"account_id":format!("factory-{}",Uuid::new_v4()),"api_key":key,
        "label":label,"disabled":!enabled,"priority":0,"created_at":now()})
}
pub fn host_accounts(rt: &Runtime) -> Result<Vec<Value>> {
    let response = rt.host.call("host.auth.list", &json!({}))?;
    Ok(response["files"]
        .as_array()
        .map(|files| {
            files
                .iter()
                .filter(|f| {
                    f["provider"].as_str() == Some(PROVIDER) || f["type"].as_str() == Some(PROVIDER)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default())
}
pub fn load_account(rt: &Runtime, id: &str) -> Result<(Value, String)> {
    if !valid_id(id) {
        return Err(Fault::input("账号 ID 无效"));
    }
    let name = rt
        .lock()?
        .accounts
        .get(id)
        .filter(|a| !a.deleted)
        .map(|a| a.auth_file.clone())
        .ok_or_else(|| Fault::new(404, "account_not_found", "Factory 账号不存在"))?;
    if !valid_auth_filename(&name) {
        return Err(Fault::input("Factory凭据文件名不安全"));
    }
    let files = host_accounts(rt)?;
    let entry = files
        .iter()
        .find(|f| {
            f["name"].as_str() == Some(&name)
                || f["id"]
                    .as_str()
                    .is_some_and(|v| crate::state::auth_id_matches(v, id, &name))
        })
        .ok_or_else(|| {
            Fault::new(
                409,
                "credential_not_loaded",
                "CPA 尚未加载此凭据；请刷新页面或重启宿主",
            )
        })?;
    let index = entry["auth_index"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Fault::new(409, "credential_not_loaded", "CPA 凭据尚未就绪"))?;
    let response = rt
        .host
        .call("host.auth.get", &json!({"auth_index":index}))?;
    let cred = response["json"].clone();
    if cred["type"].as_str() != Some(PROVIDER) || cred["account_id"].as_str() != Some(id) {
        return Err(Fault::new(
            409,
            "credential_mismatch",
            "CPA 凭据与账号不匹配",
        ));
    }
    Ok((cred, name))
}
pub fn save_account(rt: &Runtime, cred: &Value, name: &str) -> Result<Value> {
    let expected = format!(
        "factory-droid-{}.json",
        cred["account_id"].as_str().unwrap_or("")
    );
    let known = rt
        .lock()?
        .accounts
        .get(cred["account_id"].as_str().unwrap_or(""))
        .is_some_and(|a| a.auth_file == name);
    if !valid_auth_filename(name) || name != expected && !known {
        return Err(Fault::input("Factory 凭据文件名不匹配"));
    }
    rt.host
        .call("host.auth.save", &json!({"name":name,"json":cred}))
}
pub fn read_official(rt: &Arc<Runtime>, cred: &Value, callback: &str) -> (Value, Value) {
    let mut limits = match get(rt, cred, "/api/billing/limits", callback, 15, None) {
        Ok((200, v)) => normalize_limits(&v).unwrap_or_else(|| {
            unavailable_limits("unavailable", "Factory 官方额度响应格式未知，未推导余额")
        }),
        Ok((401 | 403, _)) => unavailable_limits("unavailable", "当前凭据无官方额度查询权限"),
        Ok((_, _)) => unavailable_limits("unavailable", "Factory 官方额度接口暂不可用"),
        Err(_) => unavailable_limits("error", "官方额度查询失败或超时"),
    };
    limits["checked_at"] = json!(now());
    let window = rt.lock().ok().and_then(|s| s.settings.window().ok());
    let mut usage = if let Some((start, end)) = window {
        read_analytics(rt, cred, callback, &start, &end)
    } else {
        unavailable_usage("error", "统计窗口无效")
    };
    usage["checked_at"] = json!(now());
    (limits, usage)
}
fn read_analytics(
    rt: &Arc<Runtime>,
    cred: &Value,
    callback: &str,
    start: &str,
    end: &str,
) -> Value {
    use chrono::{DateTime, Duration, NaiveDate, Timelike, Utc};
    let unknown = |message: &str| unavailable_usage("unavailable", message);
    let (Ok(start_at), Ok(end_at)) = (
        DateTime::parse_from_rfc3339(start),
        DateTime::parse_from_rfc3339(end),
    ) else {
        return unknown("统计窗口无效");
    };
    let start_at = start_at.with_timezone(&Utc);
    let end_at = end_at.with_timezone(&Utc);
    let current = Utc::now();
    let calendar = rt
        .lock()
        .ok()
        .is_some_and(|s| s.settings.usage_window == "calendar_month");
    if start_at.time().num_seconds_from_midnight() != 0
        || start_at.timestamp_subsec_nanos() != 0
        || !calendar
            && (end_at.time().num_seconds_from_midnight() != 0
                || end_at.timestamp_subsec_nanos() != 0)
    {
        return unknown("Factory Analytics 仅提供 UTC 自然日汇总，无法精确统计此子日窗口");
    }
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 14).unwrap();
    if start_at.date_naive() < cutoff {
        return unknown("Factory Analytics 仅支持2026-01-14及之后的数据，未将部分日期当作完整窗口");
    }
    if start_at >= current {
        return unknown("统计窗口尚未开始，没有可查询的官方用量");
    }
    let effective_end = end_at.min(current);
    let mut day = start_at.date_naive();
    // Factory's dates are inclusive. The plugin's configured window is half-open.
    let last = if effective_end.time().num_seconds_from_midnight() == 0
        && effective_end.timestamp_subsec_nanos() == 0
    {
        effective_end.date_naive() - Duration::days(1)
    } else {
        effective_end.date_naive()
    };
    let mut used = 0.0;
    while day <= last {
        let chunk_end = (day + Duration::days(89)).min(last);
        let path=format!("/api/v1/analytics/cost/me/query?queryKey=headline_daily&startDate={day}&endDate={chunk_end}");
        match get(rt, cred, &path, callback, 15, None) {
            Ok((200, value)) => {
                let Some(rows) = value["data"].as_array() else {
                    return unknown("Factory Analytics响应格式未知");
                };
                let mut dates = std::collections::BTreeSet::new();
                for row in rows {
                    let Some(date) = row["date"]
                        .as_str()
                        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                        .filter(|date| *date >= day && *date <= chunk_end && dates.insert(*date))
                    else {
                        return unknown(
                            "Factory Analytics日期超出请求范围、重复或无效，未估算用量",
                        );
                    };
                    let _ = date;
                    let Some(fsc) = row["fsc"].as_f64().filter(|n| n.is_finite() && *n >= 0.0)
                    else {
                        return unknown("Factory Analytics用量字段无效");
                    };
                    used += fsc;
                    if !used.is_finite() {
                        return unknown("Factory Analytics用量超过有效范围");
                    }
                }
            }
            Ok((401 | 403, _)) => {
                return unknown("Analytics需Enterprise并启用Analytics；当前凭据无查询权限")
            }
            Ok((_, _)) => return unknown("Factory Analytics暂不可用，未将部分查询作为完整窗口"),
            Err(_) => return unavailable_usage("error", "官方用量查询失败或超时"),
        }
        day = chunk_end + Duration::days(1);
    }
    json!({"status":"available","source":"factory_analytics","unit":"FSC","used":used,
        "window_start":start,"window_end":effective_end.to_rfc3339_opts(chrono::SecondsFormat::Secs,true),
        "requested_window_end":end,"partial":effective_end<end_at,"checked_at":now(),
        "message":"Factory官方UTC自然日汇总；当前日可继续变化；不代表剩余额度"})
}
pub fn normalize_limits(value: &Value) -> Option<Value> {
    let mut windows = Vec::new();
    for (group, prefix) in [("standard", "标准"), ("core", "Droid Core")] {
        for (name, label) in [
            ("fiveHour", "5 小时"),
            ("weekly", "7 天"),
            ("monthly", "30 天"),
        ] {
            let w = &value["limits"][group][name];
            if let Some(percent) = w["usedPercent"]
                .as_f64()
                .filter(|v| v.is_finite() && *v >= 0.0)
            {
                let reset = w["windowEnd"]
                    .as_str()
                    .and_then(|s| {
                        chrono::DateTime::parse_from_rfc3339(s)
                            .ok()
                            .map(|d| d.with_timezone(&chrono::Utc))
                            .or_else(|| {
                                s.parse::<i64>()
                                    .ok()
                                    .and_then(chrono::DateTime::from_timestamp_millis)
                            })
                    })
                    .or_else(|| {
                        w["windowEnd"]
                            .as_i64()
                            .and_then(chrono::DateTime::from_timestamp_millis)
                    })
                    .or_else(|| {
                        w["secondsRemaining"]
                            .as_i64()
                            .filter(|s| *s >= 0 && *s <= 40 * 86400)
                            .map(|s| chrono::Utc::now() + chrono::Duration::seconds(s))
                    })
                    .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
                if reset.as_ref().is_some_and(|r| r <= &now()) {
                    continue;
                }
                windows.push(
                    json!({"name":format!("{prefix} {label}"),"group":group,"used_percent":percent,
                    "remaining_fraction":(1.0-percent/100.0).clamp(0.0,1.0),"reset_at":reset,"window_end":reset,
                    "window_start":w["windowStart"].as_str().and_then(|s|chrono::DateTime::parse_from_rfc3339(s).ok()).map(|d|d.with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Secs,true)),
                    "window_key":name,"duration_seconds":match name{"fiveHour"=>18000,"weekly"=>604800,_=>2592000},
                    "duration_source":"factory_window_key"}),
                );
            }
        }
    }
    let balance = value["extraUsageBalanceCents"]
        .as_f64()
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map(|v| v / 100.0);
    if windows.is_empty() && balance.is_none() {
        return None;
    }
    Some(
        json!({"status":"available","source":"factory_billing_limits","windows":windows,
        "extra_usage_balance_usd":balance,"extra_usage_allowed":value["extraUsageAllowed"].as_bool(),"plan":safe_string(&value["planType"],100),
        "checked_at":now(),"message":"Factory 官方滚动窗口；额外用量余额仅来自官方字段"}),
    )
}
pub fn quota(rt: &Arc<Runtime>, data: &Value) -> Result<Value> {
    let cred = credential(data, "storage_json")?;
    let id = cred["account_id"].as_str().unwrap_or("");
    let callback = data["host_callback_id"].as_str().unwrap_or("");
    let (limits, official) = read_official(rt, &cred, callback);
    let mut store = rt.lock()?;
    store.sample_quota(id, &limits);
    if let Some(account) = store.accounts.get_mut(id) {
        account.official_limits = limits.clone();
        account.official_usage = official;
    }
    store.persist()?;
    let usage = store.usage(Some(id));
    let mut summary = vec![
        json!({"key":"local_requests","label":"本插件成功请求","value":usage["requests"],"unit":"次","format":"number"}),
        json!({"key":"local_estimated_credits","label":"本地估算消耗","value":usage["estimated_credits"],"unit":"加权 token","format":"number"}),
    ];
    if let Some(balance) = limits["extra_usage_balance_usd"].as_f64() {
        summary.push(json!({"key":"extra_usage_balance","label":"官方额外用量余额","value":balance,"format":"currency","currency":"USD"}));
    }
    if let Some(cost) = usage["estimated_cost_usd"].as_f64() {
        summary.push(json!({"key":"local_estimated_usd","label":"本地估算费用","value":cost,"format":"currency","currency":"USD"}));
    }
    let mut groups = Vec::new();
    for (key, label) in [
        ("standard", "Factory 官方标准额度"),
        ("core", "Factory 官方 Droid Core"),
    ] {
        let buckets: Vec<_>=limits["windows"].as_array().into_iter().flatten().filter(|w|w["group"].as_str()==Some(key)).map(|w|json!({
            "window":w["name"],"remainingFraction":w["remaining_fraction"],"resetTime":w["reset_at"].as_str().unwrap_or(""),
            "description":"Factory 官方返回的滚动窗口"})).collect();
        if !buckets.is_empty() {
            groups.push(json!({"displayName":label,"buckets":buckets}));
        }
    }
    let mut response = json!({"summary":summary,"groups":groups});
    if let Some(plan) = limits["plan"].as_str() {
        response["subscription"] = json!({"plan":plan});
    }
    Ok(response)
}
