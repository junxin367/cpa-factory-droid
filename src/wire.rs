//! Outbound identity follows the captured Droid CLI 1.2.0 application protocol.
//! Downstream credentials, routing IDs and tracing context never become upstream IDs.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{guard, state::Model, Fault, Result, Runtime};

pub const CLIENT_VERSION: &str = "1.2.0";
pub const FETCH_USER_AGENT: &str = "Bun/1.4.4";
pub struct Identifiers {
    namespace: Uuid,
}

impl Identifiers {
    pub fn new(namespace: &str) -> Result<Self> {
        Ok(Self {
            namespace: Uuid::parse_str(namespace)
                .map_err(|_| Fault::storage("会话匿名化命名空间无效"))?,
        })
    }

    fn scoped(&self, kind: &str, account: &str, caller: &str, value: &str) -> String {
        let mut digest = Sha256::new();
        digest.update(b"factory-wire-identity-v1");
        digest.update(self.namespace.as_bytes());
        for part in [kind, account, caller, value] {
            digest.update((part.len() as u64).to_le_bytes());
            digest.update(part.as_bytes());
        }
        // A persistent private namespace keeps the opaque UUID stable without
        // retaining client IDs, messages or an unbounded per-conversation map.
        let mut bytes = [0; 16];
        bytes.copy_from_slice(&digest.finalize()[..16]);
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Uuid::from_bytes(bytes).to_string()
    }
}

pub struct Identity {
    pub session: String,
    pub cache: String,
    pub safety: String,
    pub message: String,
    pub trace: String,
}

fn bounded(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|value| !value.trim().is_empty() && value.len() <= 4096)
}

fn header<'a>(headers: &'a Value, name: &str) -> Option<&'a str> {
    guard::header_values(headers, name)
        .into_iter()
        .find(|value| !value.trim().is_empty() && value.len() <= 4096)
}

fn json_header(headers: &Value, name: &str) -> Value {
    header(headers, name)
        .and_then(|value| serde_json::from_str(value).ok())
        .unwrap_or(Value::Null)
}

fn claude_identity(payload: &Value) -> Value {
    let Some(value) = bounded(&payload["metadata"]["user_id"]) else {
        return Value::Null;
    };
    if value.trim_start().starts_with('{') {
        return serde_json::from_str(value).unwrap_or(Value::Null);
    }
    value
        .rsplit_once("_session_")
        .and_then(|(_, session)| Uuid::parse_str(session).ok())
        .map(|id| json!({"session_id":id.to_string()}))
        .unwrap_or(Value::Null)
}

fn session_hint(headers: &Value, metadata: &Value, payload: &Value) -> Option<String> {
    // Native server-side conversation handles identify actual context. A cache
    // group or a user ID must not override them via the host's canonical alias.
    if let Some(id) =
        bounded(&payload["conversation"]).or_else(|| bounded(&payload["conversation"]["id"]))
    {
        return Some(id.into());
    }
    let claude = claude_identity(payload);
    if let Some(root) =
        header(headers, "X-Claude-Code-Session-Id").or_else(|| bounded(&claude["session_id"]))
    {
        let agent = header(headers, "X-Claude-Code-Agent-Id")
            .or_else(|| bounded(&payload["metadata"]["agent_id"]))
            .or_else(|| bounded(&payload["metadata"]["subagent_id"]))
            .or_else(|| bounded(&claude["agent_id"]));
        return Some(match agent.filter(|agent| *agent != "main") {
            Some(agent) => json!(["claude-agent", root, agent]).to_string(),
            None => root.into(),
        });
    }
    let turn = json_header(headers, "X-Codex-Turn-Metadata");
    let root = header(headers, "Session-Id")
        .or_else(|| header(headers, "Session_id"))
        .or_else(|| bounded(&turn["session_id"]));
    let thread = header(headers, "Thread-Id")
        .or_else(|| header(headers, "Thread_id"))
        .or_else(|| bounded(&turn["thread_id"]))
        .or_else(|| {
            root.and_then(|_| {
                bounded(&payload["thread_id"])
                    .or_else(|| bounded(&payload["threadId"]))
                    .or_else(|| bounded(&payload["metadata"]["thread_id"]))
            })
        });
    if root.is_some() || thread.is_some() {
        // An explicit thread identifies the actual conversation. Agent names can
        // be reused by separate threads under the same root session.
        if let Some(thread) = thread {
            return Some(thread.into());
        }
        let agent = bounded(&turn["agent_name"])
            .map(|agent| agent.trim_start_matches('/').trim_start_matches("root/"))
            .filter(|agent| !["", "root", "main"].contains(agent));
        return Some(match (root, agent) {
            (Some(root), Some(agent)) => json!(["codex-agent", root, agent]).to_string(),
            _ => root?.into(),
        });
    }
    for name in [
        "X-Session-Id",
        "X-Http-Session-Id",
        "X-Conversation-Id",
        "Conversation_id",
        "X-Thread-Id",
        "X-Task-Id",
        "X-Slot-Session-Id",
    ] {
        if let Some(value) = header(headers, name) {
            return Some(value.into());
        }
    }
    for field in [
        "thread_id",
        "threadId",
        "session_id",
        "sessionId",
        "conversation_id",
        "conversationId",
        "chat_id",
        "chatId",
    ] {
        if let Some(value) =
            bounded(&payload[field]).or_else(|| bounded(&payload["metadata"][field]))
        {
            return Some(value.into());
        }
    }
    if let Some(value) = bounded(&metadata["execution_session_id"]) {
        return Some(value.into());
    }
    // CPA's canonical/derived/affinity identifiers can represent a cache group,
    // account or shared prompt prefix. Without an explicit conversation source,
    // they must not become the upstream conversation identity.
    None
}

fn explicit_identifier<'a>(payload: &'a Value, name: &str) -> Result<Option<&'a str>> {
    match payload.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.is_empty() => Ok(None),
        Some(value) => bounded(value)
            .map(Some)
            .ok_or_else(|| Fault::input("缓存键或安全标识必须是非空且不超过 4096 字节的字符串")),
    }
}

pub fn identity(
    rt: &Runtime,
    cred: &Value,
    headers: &Value,
    metadata: &Value,
    payload: &Value,
) -> Result<Identity> {
    identify(&rt.wire_ids, cred, headers, metadata, payload)
}

fn identify(
    ids: &Identifiers,
    cred: &Value,
    headers: &Value,
    metadata: &Value,
    payload: &Value,
) -> Result<Identity> {
    let caller = caller_hint(headers, metadata).unwrap_or("");
    let account = cred["account_id"].as_str().unwrap_or("");
    let hint = session_hint(headers, metadata, payload);
    let scoped = |kind, value: &str| ids.scoped(kind, account, caller, value);
    let session = hint
        .as_deref()
        .map(|id| scoped("conversation", id))
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let cache = match explicit_identifier(payload, "prompt_cache_key")? {
        Some(key) => scoped("cache-group", key),
        None if hint.is_some() => session.clone(),
        None => scoped(
            "cache-derived",
            bounded(&metadata["derived_session_id"]).unwrap_or("caller-default"),
        ),
    };
    let safety = explicit_identifier(payload, "safety_identifier")?
        .map(|value| scoped("safety-user", value))
        .unwrap_or_else(|| session.clone());
    Ok(Identity {
        session,
        cache,
        safety,
        message: Uuid::new_v4().to_string(),
        trace: traceparent(),
    })
}

fn caller_hint<'a>(headers: &'a Value, metadata: &'a Value) -> Option<&'a str> {
    bounded(&metadata["caller_scope"]).or_else(|| {
        ["Authorization", "X-Api-Key"].into_iter().find_map(|name| {
            guard::header_values(headers, name)
                .into_iter()
                .find(|value| !value.trim().is_empty() && value.len() <= 8192)
        })
    })
}

pub fn traceparent() -> String {
    format!(
        "00-{}-{}-01",
        Uuid::new_v4().simple(),
        &Uuid::new_v4().simple().to_string()[..16]
    )
}

fn transport_field(name: &str) -> bool {
    let name: String = name
        .chars()
        .filter(|c| !matches!(c, '-' | '_' | '.'))
        .flat_map(char::to_lowercase)
        .collect();
    [
        "metadata",
        "user",
        "userid",
        "sessionid",
        "conversationid",
        "threadid",
        "requestid",
        "traceid",
        "traceparent",
        "tracestate",
        "baggage",
        "tags",
        "useragent",
        "client",
        "clientname",
        "clientid",
        "clientinfo",
        "clientmetadata",
        "clientcontext",
        "requestmetadata",
        "trackingmetadata",
        "trace",
        "tracing",
        "telemetry",
        "appname",
        "appversion",
        "sdk",
        "sdkversion",
        "runtime",
        "runtimeversion",
        "source",
        "headers",
        "extraheaders",
        "httpheaders",
        "requestheaders",
        "authorization",
        "apikey",
        "baseurl",
        "apibase",
        "extrabody",
        "cookies",
        "cookie",
    ]
    .contains(&name.as_str())
        || [
            "cpa",
            "cliproxy",
            "cpamp",
            "proxy",
            "reverseproxy",
            "litellm",
            "openrouter",
            "forwarded",
            "xforwarded",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

pub fn payload(request: &Value, model: &Model, identity: &Identity) -> Result<Value> {
    let mut payload = request.clone();
    let fields = payload
        .as_object_mut()
        .ok_or_else(|| Fault::input("原生请求必须为 JSON 对象"))?;
    // Only envelope metadata is removed. Messages, tool schemas, tool results,
    // signatures, encrypted reasoning and native continuation IDs remain opaque.
    fields.retain(|name, _| !transport_field(name));
    let cache = fields.remove("prompt_cache_key").is_some();
    let safety = fields.remove("safety_identifier").is_some();
    let openai_responses = model.protocol == "responses" && model.upstream_provider == "openai";
    if cache || openai_responses {
        fields.insert("prompt_cache_key".into(), json!(identity.cache));
    }
    if safety || openai_responses {
        fields.insert("safety_identifier".into(), json!(identity.safety));
    }
    Ok(payload)
}

fn proxy_marker(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| word == "cpa")
        || [
            "cpa-",
            "cpa_",
            "cliproxy",
            "cpamp",
            "proxy",
            "litellm",
            "openrouter",
            "forwarded",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
}

fn beta_token(value: &str) -> bool {
    if value.len() > 160 || proxy_marker(value) {
        return false;
    }
    let Some((prefix, date)) = value
        .len()
        .checked_sub(10)
        .map(|offset| value.split_at(offset))
    else {
        return false;
    };
    !prefix.is_empty()
        && prefix.as_bytes()[0].is_ascii_lowercase()
        && prefix.ends_with('-')
        && prefix
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok()
}

pub fn protocol_headers(headers: &mut Value, incoming: &Value, model: &Model) -> Result<()> {
    let invalid = || {
        Fault::new(
            400,
            "invalid_native_request",
            "原生协议请求头无效或包含代理标识",
        )
    };
    if model.protocol == "anthropic" {
        let versions = guard::header_values(incoming, "anthropic-version");
        if versions.len() > 1
            || versions.first().is_some_and(|value| {
                value.len() != 10 || chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_err()
            })
        {
            return Err(invalid());
        }
        headers["anthropic-version"] = json!([versions.first().copied().unwrap_or("2023-06-01")]);
        let mut flags = Vec::new();
        for value in guard::header_values(incoming, "anthropic-beta") {
            if value.len() > 4096 || !value.is_ascii() {
                return Err(invalid());
            }
            for flag in value.split(',').map(str::trim) {
                if !beta_token(flag) {
                    return Err(invalid());
                }
                if !flags.contains(&flag) {
                    flags.push(flag);
                }
            }
        }
        if !flags.is_empty() {
            headers["anthropic-beta"] = json!([flags.join(",")]);
        }
    } else {
        let values = guard::header_values(incoming, "OpenAI-Beta");
        for value in &values {
            if value.len() > 4096
                || proxy_marker(value)
                || !value.is_ascii()
                || !value.split(',').all(|item| {
                    item.trim().split_once('=').is_some_and(|(key, value)| {
                        !key.is_empty()
                            && !value.is_empty()
                            && key
                                .bytes()
                                .chain(value.bytes())
                                .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b))
                    })
                })
            {
                return Err(invalid());
            }
        }
        if !values.is_empty() {
            headers["OpenAI-Beta"] = json!([values.join(",")]);
        }
    }
    crate::catalog::apply_required_headers(headers, model);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> Identifiers {
        Identifiers::new("afac50e3-b106-4935-a576-7cd096c35de4").unwrap()
    }

    #[test]
    fn explicit_codex_threads_override_reused_agent_names() {
        let request = |thread: &str, agent: &str| {
            json!({"Session-Id":["root"],"Thread-Id":[thread],
                "X-Codex-Turn-Metadata":[json!({"agent_name":agent}).to_string()]})
        };
        let hint = |headers: Value| session_hint(&headers, &json!({}), &json!({})).unwrap();
        assert_ne!(
            hint(request("first", "worker")),
            hint(request("second", "worker"))
        );
        assert_eq!(
            hint(request("first", "worker")),
            hint(request("first", "renamed"))
        );
        assert_eq!(
            hint(request("first", "/root/worker")),
            hint(request("first", "root"))
        );
        let metadata = json!({"X-Codex-Turn-Metadata":[
            json!({"session_id":"root","thread_id":"first","agent_name":"worker"}).to_string()]});
        assert_eq!(hint(metadata), hint(request("first", "worker")));
    }

    #[test]
    fn named_agents_without_threads_remain_stable_and_separate() {
        let hint = |agent: &str| {
            session_hint(
                &json!({"Session-Id":["root"],
            "X-Codex-Turn-Metadata":[json!({"agent_name":agent}).to_string()]}),
                &json!({}),
                &json!({}),
            )
        };
        assert_eq!(hint("worker"), hint("/root/worker"));
        assert_ne!(hint("worker"), hint("reviewer"));
    }

    #[test]
    fn conversation_cache_and_safety_have_independent_scopes() {
        let credential = json!({"account_id":"account"});
        let metadata = json!({"caller_scope":"caller"});
        let make = |conversation: &str, cache: &str| {
            identify(&ids(), &credential, &json!({}),
            &metadata, &json!({"conversation":conversation,"prompt_cache_key":cache,"safety_identifier":"user"})).unwrap()
        };
        let a = make("conversation-a", "shared");
        let b = make("conversation-b", "shared");
        let c = make("conversation-a", "different");
        assert_ne!(a.session, b.session);
        assert_eq!(a.session, c.session);
        assert_eq!(a.cache, b.cache);
        assert_ne!(a.cache, c.cache);
        assert_eq!(a.safety, b.safety);
        assert_ne!(a.safety, a.cache);
        assert_ne!(a.message, make("conversation-a", "shared").message);
        assert_eq!(a.session, make("conversation-a", "shared").session);
    }

    #[test]
    fn shared_host_hints_do_not_create_a_conversation() {
        let metadata = json!({"caller_scope":"caller","canonical_session_id":"pck:shared",
            "derived_session_id":"shared"});
        let payload = json!({"prompt_cache_key":"shared","metadata":{"user_id":"shared-user"}});
        let a = identify(
            &ids(),
            &json!({"account_id":"a"}),
            &json!({}),
            &metadata,
            &payload,
        )
        .unwrap();
        let b = identify(
            &ids(),
            &json!({"account_id":"a"}),
            &json!({}),
            &metadata,
            &payload,
        )
        .unwrap();
        assert_ne!(a.session, b.session);
        assert_eq!(a.cache, b.cache);
        let other = identify(
            &ids(),
            &json!({"account_id":"b"}),
            &json!({}),
            &metadata,
            &payload,
        )
        .unwrap();
        assert_ne!(a.cache, other.cache);
        let caller = identify(
            &ids(),
            &json!({"account_id":"a"}),
            &json!({}),
            &json!({"caller_scope":"other"}),
            &payload,
        )
        .unwrap();
        assert_ne!(a.cache, caller.cache);
    }
}
