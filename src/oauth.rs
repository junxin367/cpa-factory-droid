//! Factory's WorkOS device flow, exposed through CPA's native OAuth hooks.
//! Device codes and pending tokens stay in memory; completed credentials belong to CPA.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, TryLockError},
    time::{Duration, Instant},
};
use url::Url;
use uuid::Uuid;

use crate::{
    access, factory,
    host::{decode, encode},
    state::now,
    Fault, Result, Runtime, PROVIDER,
};

// Public native-client identifier shipped by the official Droid CLI.
const CLIENT_ID: &str = "client_01HNM792M5G5G1A2THWPXKFMXB";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const MAX_SESSIONS: usize = 32;

pub struct Session {
    expires: DateTime<Utc>,
    device_code: String,
    interval: Duration,
    next_poll: Instant,
    credential: Option<Value>,
    completed: Option<Value>,
}

#[derive(Default)]
pub struct Rotation {
    spent: Vec<String>,
    latest: Option<Value>,
}

pub type Sessions = Mutex<HashMap<String, Arc<Mutex<Session>>>>;
pub type Rotations = Mutex<HashMap<String, Arc<Mutex<Rotation>>>>;

pub fn is_oauth(cred: &Value) -> bool {
    cred["auth_method"].as_str() == Some("oauth")
}

fn token(value: &Value, field: &str) -> Result<String> {
    value[field]
        .as_str()
        .filter(|s| (8..=32768).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_graphic()))
        .map(str::to_owned)
        .ok_or_else(|| {
            Fault::new(
                401,
                "oauth_token_invalid",
                "Factory OAuth 凭据无效，请重新登录",
            )
        })
}

pub fn validate(cred: &Value) -> Result<()> {
    token(cred, "access_token")?;
    token(cred, "refresh_token")?;
    if expires_at(cred).is_none() {
        return Err(Fault::input("Factory OAuth 凭据缺少有效的到期时间"));
    }
    Ok(())
}

pub fn expires_at(cred: &Value) -> Option<DateTime<Utc>> {
    cred["expires_at"]
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Utc))
}

pub fn refresh_at(cred: &Value) -> DateTime<Utc> {
    let due = expires_at(cred).unwrap_or_else(Utc::now) - ChronoDuration::seconds(120);
    due.max(Utc::now())
}

fn claims(access: &str) -> Value {
    // Claims are scheduling/routing hints only. Factory whoami verifies identity.
    access
        .split('.')
        .nth(1)
        .and_then(|part| URL_SAFE_NO_PAD.decode(part).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null)
}

fn apply_tokens(cred: &mut Value, response: &Value) -> Result<()> {
    let access = token(response, "access_token")?;
    let refresh = if response["refresh_token"].is_null() {
        token(cred, "refresh_token")?
    } else {
        token(response, "refresh_token")?
    };
    let payload = claims(&access);
    let jwt_expiry = payload["exp"]
        .as_i64()
        .and_then(|t| DateTime::from_timestamp(t, 0));
    let response_expiry = response["expires_in"]
        .as_i64()
        .filter(|seconds| *seconds > 0 && *seconds <= 366 * 86400)
        .map(|seconds| Utc::now() + ChronoDuration::seconds(seconds));
    let expiry = match (jwt_expiry, response_expiry) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) | (None, Some(a)) => a,
        // An opaque token without expiry is retried conservatively, never assumed to last 30 days.
        _ => Utc::now() + ChronoDuration::minutes(5),
    };
    if expiry <= Utc::now() {
        return Err(Fault::new(
            502,
            "oauth_token_expired",
            "Factory 返回的登录令牌已过期",
        ));
    }
    cred["access_token"] = json!(access);
    cred["refresh_token"] = json!(refresh);
    cred["expires_at"] = json!(expiry.to_rfc3339());
    cred["last_refresh"] = json!(now());
    for (target, source) in [
        ("workos_org_id", &response["organization_id"]),
        ("oauth_user_id", &response["user"]["id"]),
    ] {
        if let Some(value) = source.as_str().filter(|s| safe_identifier(s)) {
            cred[target] = json!(value);
        }
    }
    if cred["workos_org_id"].as_str().is_none_or(str::is_empty) {
        if let Some(org) = payload["org_id"].as_str().filter(|s| safe_identifier(s)) {
            cred["workos_org_id"] = json!(org);
        }
    }
    Ok(())
}

fn safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn endpoint(rt: &Runtime, path: &str) -> String {
    // Explicit loopback test profiles simulate both Factory and WorkOS.
    let base = if rt.config.allow_local_mock
        && Url::parse(rt.api_base()).ok().is_some_and(|url| {
            ["localhost", "127.0.0.1", "::1", "[::1]"].contains(&url.host_str().unwrap_or(""))
        }) {
        rt.api_base()
    } else {
        "https://api.workos.com"
    };
    format!("{base}/user_management{path}")
}

fn post(
    rt: &Arc<Runtime>,
    path: &str,
    fields: &[(&str, &str)],
    callback: &str,
) -> Result<(u16, Value)> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter().copied())
        .finish();
    let op = rt.operation(callback, Some(20))?;
    let response = rt.host.call(
        "host.http.do",
        &json!({"host_callback_id":callback,"operation_id":op.id,
            "method":"POST","url":endpoint(rt,path),
            "headers":{"Content-Type":["application/x-www-form-urlencoded"],"Accept":["application/json"],
                "User-Agent":[crate::wire::FETCH_USER_AGENT]},
            "body":encode(body)}),
    )?;
    let status = response["StatusCode"].as_u64().unwrap_or(502) as u16;
    let bytes = decode(&response["Body"])?;
    if bytes.len() > 256 * 1024 {
        return Err(Fault::new(
            502,
            "oauth_response_invalid",
            "Factory 授权响应过大",
        ));
    }
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

fn login_error(message: &str) -> Value {
    json!({"Status":"error","Message":message})
}

fn pending() -> Value {
    json!({"Status":"pending"})
}

pub fn start(rt: &Arc<Runtime>, data: &Value) -> Result<Value> {
    let _active = rt.begin()?;
    let mut sessions = rt.oauth_sessions.lock().map_err(|_| Fault::internal())?;
    sessions.retain(|_, session| session.try_lock().map_or(true, |s| s.expires > Utc::now()));
    if sessions.len() >= MAX_SESSIONS {
        return Err(Fault::new(
            429,
            "oauth_busy",
            "待完成的 Factory 登录过多，请稍后重试",
        ));
    }
    let callback = data["host_callback_id"].as_str().unwrap_or("");
    let (status, response) = post(
        rt,
        "/authorize/device",
        &[("client_id", CLIENT_ID)],
        callback,
    )?;
    if status != 200 {
        return Err(Fault::new(
            502,
            "oauth_start_failed",
            "无法创建 Factory 登录，请检查代理连接后重试",
        ));
    }
    let device_code = token(&response, "device_code")?;
    let seconds = response["expires_in"]
        .as_i64()
        .filter(|s| (1..=1800).contains(s))
        .ok_or_else(|| Fault::new(502, "oauth_response_invalid", "Factory 登录有效期无效"))?;
    let interval = response["interval"].as_u64().unwrap_or(5).clamp(1, 60);
    let mut url = Url::parse(
        response["verification_uri_complete"]
            .as_str()
            .or_else(|| response["verification_uri"].as_str())
            .unwrap_or(""),
    )
    .map_err(|_| Fault::new(502, "oauth_response_invalid", "Factory 登录地址无效"))?;
    let domain = url.host_str().unwrap_or("");
    let trusted = domain == "auth.factory.ai"
        || domain == "auth.factory.com"
        || domain.ends_with(".authkit.app")
        || domain.ends_with(".workos.com");
    if url.scheme() != "https"
        || !trusted
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|p| p != 443)
    {
        return Err(Fault::new(
            502,
            "oauth_url_untrusted",
            "Factory 返回的登录地址不受信任",
        ));
    }
    if response["verification_uri_complete"].as_str().is_none() {
        let user_code = response["user_code"]
            .as_str()
            .filter(|s| safe_identifier(s))
            .ok_or_else(|| {
                Fault::new(502, "oauth_response_invalid", "Factory 登录缺少设备验证码")
            })?;
        url.query_pairs_mut().append_pair("user_code", user_code);
    }
    let state = Uuid::new_v4().to_string();
    let expires = Utc::now() + ChronoDuration::seconds(seconds);
    sessions.insert(
        state.clone(),
        Arc::new(Mutex::new(Session {
            expires,
            device_code,
            interval: Duration::from_secs(interval),
            next_poll: Instant::now() + Duration::from_secs(interval),
            credential: None,
            completed: None,
        })),
    );
    Ok(
        json!({"Provider":PROVIDER,"URL":url.as_str(),"State":state,"ExpiresAt":expires.to_rfc3339()}),
    )
}

pub fn poll(rt: &Arc<Runtime>, data: &Value) -> Result<Value> {
    let _active = rt.begin()?;
    let state = data["State"].as_str().unwrap_or("");
    let session = rt
        .oauth_sessions
        .lock()
        .map_err(|_| Fault::internal())?
        .get(state)
        .cloned();
    let Some(session) = session else {
        return Ok(login_error("Factory 登录已失效，请重新开始"));
    };
    let mut session = match session.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::WouldBlock) => return Ok(pending()),
        Err(_) => return Err(Fault::internal()),
    };
    if session.expires <= Utc::now() {
        rt.oauth_sessions
            .lock()
            .map_err(|_| Fault::internal())?
            .remove(state);
        return Ok(login_error("Factory 登录已过期，请重新开始"));
    }
    if let Some(auth) = &session.completed {
        return Ok(json!({"Status":"success","Auth":auth}));
    }
    if Instant::now() < session.next_poll {
        return Ok(pending());
    }
    session.next_poll = Instant::now() + session.interval;
    let callback = data["host_callback_id"].as_str().unwrap_or("");
    if session.credential.is_none() {
        let response = post(
            rt,
            "/authenticate",
            &[
                ("grant_type", DEVICE_GRANT),
                ("device_code", &session.device_code),
                ("client_id", CLIENT_ID),
            ],
            callback,
        );
        let (status, response) = match response {
            Ok(response) => response,
            Err(error) if error.http_status >= 500 => return Ok(pending()),
            Err(error) => return Err(error),
        };
        if status != 200 {
            match response["error"].as_str().unwrap_or("") {
                "authorization_pending" => return Ok(pending()),
                "slow_down" => {
                    session.interval += Duration::from_secs(5);
                    session.next_poll = Instant::now() + session.interval;
                    return Ok(pending());
                }
                _ if status >= 500 || status == 429 => return Ok(pending()),
                _ => {
                    rt.oauth_sessions
                        .lock()
                        .map_err(|_| Fault::internal())?
                        .remove(state);
                    return Ok(login_error(
                        match response["error"].as_str().unwrap_or("") {
                            "access_denied" => "Factory 登录已取消",
                            "expired_token" => "Factory 登录已过期，请重新开始",
                            _ => "Factory 授权失败，请重新登录",
                        },
                    ));
                }
            }
        }
        let mut cred = json!({"type":PROVIDER,"auth_method":"oauth",
            "account_id":format!("factory-{}",Uuid::new_v4()),"label":"Factory",
            "disabled":false,"priority":0,"created_at":now()});
        apply_tokens(&mut cred, &response)?;
        session.device_code.clear();
        session.credential = Some(cred);
    }
    let cred = session.credential.as_mut().ok_or_else(Fault::internal)?;
    // Retain tokens after successful exchange so a transient identity/catalog failure
    // never attempts to spend the single-use device code a second time.
    if let Err(error) = prepare_identity(rt, cred, callback) {
        if error.http_status >= 500 || error.http_status == 429 {
            return Ok(pending());
        }
        rt.oauth_sessions
            .lock()
            .map_err(|_| Fault::internal())?
            .remove(state);
        return Ok(login_error(&error.message));
    }
    if let Some(email) = cred["email"].as_str().filter(|s| !s.is_empty()) {
        cred["label"] = json!(email.chars().take(80).collect::<String>());
    }
    if access::refresh_catalog(rt, cred).is_err() {
        rt.host
            .log("Factory OAuth 已授权，官方目录暂不可用；使用内置目录并等待后续刷新");
        cred["factory_catalog_source"] = json!("bundled_registry");
        cred["factory_catalog_checked_at"] = json!(now());
    }
    let name = format!(
        "factory-droid-{}.json",
        cred["account_id"].as_str().ok_or_else(Fault::internal)?
    );
    let auth = factory::auth_data(cred, &name, &name);
    session.completed = Some(auth.clone());
    session.credential = None;
    Ok(json!({"Status":"success","Auth":auth}))
}

fn prepare_identity(rt: &Arc<Runtime>, cred: &mut Value, callback: &str) -> Result<()> {
    let payload = claims(cred["access_token"].as_str().unwrap_or(""));
    if cred["workos_org_id"].as_str().is_none_or(str::is_empty)
        && payload["external_org_id"]
            .as_str()
            .is_none_or(str::is_empty)
    {
        let (status, response) = factory::get(rt, cred, "/api/cli/org", callback, 15, None)?;
        if status != 200 {
            return Err(factory::upstream_error(status, &json!({})));
        }
        let organizations = response["workosOrgIds"]
            .as_array()
            .ok_or_else(|| Fault::new(502, "oauth_org_invalid", "Factory 未返回组织信息"))?;
        let org = organizations
            .iter()
            .filter_map(Value::as_str)
            .find(|s| safe_identifier(s))
            .ok_or_else(|| Fault::new(403, "oauth_org_missing", "Factory 账号尚未加入可用组织"))?;
        let refresh = token(cred, "refresh_token")?;
        let (status, tokens) = post(
            rt,
            "/authenticate",
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", &refresh),
                ("client_id", CLIENT_ID),
                ("organization_id", org),
            ],
            callback,
        )?;
        if status != 200 {
            return Err(refresh_error(status, &tokens));
        }
        apply_tokens(cred, &tokens)?;
        cred["workos_org_id"] = json!(org);
    }
    factory::whoami(rt, cred, callback)
}

fn refresh_error(status: u16, response: &Value) -> Fault {
    if ["invalid_grant", "invalid_client", "access_denied"]
        .contains(&response["error"].as_str().unwrap_or(""))
    {
        Fault::new(
            401,
            "oauth_login_required",
            "Factory 登录已失效，请通过 CPA OAuth 重新登录",
        )
    } else {
        Fault::new(
            if status == 429 { 429 } else { 502 },
            "oauth_refresh_failed",
            "Factory 令牌续期暂时失败，请检查网络后重试",
        )
    }
}

/// Called only by CPA's native auth.refresh. Serialize token rotation per credential.
pub fn refresh(rt: &Arc<Runtime>, data: &Value, mut cred: Value) -> Result<Value> {
    let _active = rt.begin()?;
    let id = cred["account_id"]
        .as_str()
        .filter(|s| factory::valid_id(s))
        .ok_or_else(|| Fault::input("Factory 账号 ID 无效"))?
        .to_string();
    let slot = rt
        .oauth_rotations
        .lock()
        .map_err(|_| Fault::internal())?
        .entry(id.clone())
        .or_default()
        .clone();
    let mut rotation = slot.lock().map_err(|_| Fault::internal())?;
    // CPA can schedule refresh immediately after saving a login, before model.for_auth.
    // Bootstrap only a missing cache entry; an existing disabled/deleted entry stays authoritative.
    if !rt.lock()?.accounts.contains_key(&id) {
        let fallback = format!("factory-droid-{id}.json");
        let file = std::path::Path::new(data["AuthID"].as_str().unwrap_or(&fallback))
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| factory::valid_auth_filename(name))
            .unwrap_or(&fallback);
        factory::observe_model_account(rt, &cred, file)?;
    }
    let (name, enabled, revision) = rt
        .lock()?
        .accounts
        .get(&id)
        .map(|account| {
            (
                account.auth_file.clone(),
                account.enabled && !account.deleted,
                account.revision,
            )
        })
        .ok_or_else(|| Fault::new(404, "account_not_found", "Factory 凭据尚未加载"))?;
    if !enabled || cred["disabled"] == true || cred["deleted"] == true {
        return Err(Fault::new(
            409,
            "account_disabled",
            "Factory 凭据已停用或删除",
        ));
    }
    let spent = access::fingerprint(&cred["refresh_token"]);
    if rotation.spent.contains(&spent) {
        if let Some(latest) = &rotation.latest {
            for key in [
                "access_token",
                "refresh_token",
                "expires_at",
                "last_refresh",
                "workos_org_id",
            ] {
                cred[key] = latest[key].clone();
            }
        }
    }
    let callback = data["host_callback_id"].as_str().unwrap_or("");
    let mut rotated = false;
    if refresh_at(&cred) <= Utc::now() {
        let old_refresh = token(&cred, "refresh_token")?;
        let mut fields = vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", old_refresh.as_str()),
            ("client_id", CLIENT_ID),
        ];
        if let Some(org) = cred["workos_org_id"]
            .as_str()
            .filter(|s| safe_identifier(s))
        {
            fields.push(("organization_id", org));
        }
        let (status, response) = post(rt, "/authenticate", &fields, callback)?;
        if status != 200 {
            return Err(refresh_error(status, &response));
        }
        apply_tokens(&mut cred, &response)?;
        rotation
            .spent
            .push(access::fingerprint(&json!(old_refresh)));
        if rotation.spent.len() > 16 {
            rotation.spent.remove(0);
        }
        rotation.latest = Some(cred.clone());
        rotated = true;
    }
    let mut retry_identity = false;
    if let Err(error) = factory::whoami(rt, &mut cred, callback) {
        if !rotated && rotation.latest.is_none() {
            return Err(error);
        }
        // Returning an error here would discard a freshly rotated refresh token.
        rt.host.log("Factory 令牌已续期，身份信息等待重试");
        retry_identity = true;
    }
    let catalog_due = cred["factory_catalog_checked_at"]
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .is_none_or(|checked| {
            Utc::now() - checked.with_timezone(&Utc) >= ChronoDuration::hours(24)
        });
    if catalog_due && access::refresh_catalog(rt, &mut cred).is_err() {
        rt.host
            .log("Factory 官方目录刷新暂不可用，保留已有模型目录");
    }
    // A concurrent native edit must not be undone by an older refresh snapshot.
    if let Some(current) = rt.lock()?.accounts.get(&id) {
        if current.revision != revision {
            cred["disabled"] = json!(!current.enabled);
            cred["deleted"] = json!(current.deleted);
            cred["label"] = json!(current.label);
            cred["priority"] = json!(current.priority);
        }
    }
    // Local metadata is a cache. A failure must not prevent CPA saving renewed tokens.
    if factory::upsert_account(rt, &cred, &name, !retry_identity).is_err()
        || access::restore_catalog(rt, &cred).is_err()
    {
        rt.host.log("Factory 本地目录缓存等待 CPA 凭据重载");
    }
    let auth = factory::auth_data(&cred, data["AuthID"].as_str().unwrap_or(&name), &name);
    let next = if retry_identity {
        (Utc::now() + ChronoDuration::minutes(5)).min(refresh_at(&cred))
    } else {
        refresh_at(&cred)
    };
    Ok(json!({"Auth":auth,"NextRefreshAfter":next.to_rfc3339()}))
}
