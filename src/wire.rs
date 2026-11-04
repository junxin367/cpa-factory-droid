//! Outbound identity follows the captured Droid CLI 1.2.0 application protocol.
//! Downstream credentials, routing IDs and tracing context never become upstream IDs.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{guard, state::Model, Fault, Result, Runtime};

pub const CLIENT_VERSION: &str = "1.2.0";
pub const FETCH_USER_AGENT: &str = "Bun/1.4.4";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const ANTHROPIC_BETA: &str = "fine-grained-tool-streaming-2025-05-14";
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
    if model.protocol == "anthropic" {
        // SDK transport options are not Messages API body fields. Capability
        // headers below are derived from supported payload features, never copied.
        fields.remove("betas");
    }
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

fn beta(flags: &mut Vec<&'static str>, value: &'static str) {
    if !flags.contains(&value) {
        flags.push(value);
    }
}

fn cache_beta(value: &Value, flags: &mut Vec<&'static str>) {
    if !value["cache_control"]["scope"].is_null() {
        beta(flags, "prompt-caching-scope-2026-01-05");
    }
}

fn tool_betas(tool: &Value, flags: &mut Vec<&'static str>) {
    cache_beta(tool, flags);
    let kind = tool["type"].as_str().unwrap_or("");
    if kind.starts_with("tool_search_tool_")
        || tool["defer_loading"].as_bool() == Some(true)
        || !tool["input_examples"].is_null()
        || !tool["allowed_callers"].is_null()
    {
        beta(flags, "advanced-tool-use-2025-11-20");
    }
    if kind.starts_with("advisor_") {
        beta(flags, "advisor-tool-2026-03-01");
    }
}

fn content_betas(content: &Value, flags: &mut Vec<&'static str>) {
    if let Some(parts) = content.as_array() {
        for part in parts {
            content_betas(part, flags);
        }
        return;
    }
    cache_beta(content, flags);
    match content["type"].as_str().unwrap_or("") {
        "tool_reference" => beta(flags, "advanced-tool-use-2025-11-20"),
        "tool_addition" => {
            beta(flags, "mid-conversation-tool-changes-2026-07-01");
            beta(flags, "inline-tools-2026-09-15");
            tool_betas(&content["tool"]["definition"], flags);
        }
        "tool_result" | "mcp_tool_result" => content_betas(&content["content"], flags),
        _ => {}
    }
}

pub fn apply_protocol_defaults(headers: &mut Value, model: &Model, payload: &Value) -> Result<()> {
    // Identity/version defaults follow Factory CLI 1.2.0. Its feature flags
    // change with the request: e.g. thinking.display=updates and per-turn effort.
    // Native Claude features use an explicit protocol allowlist like CPA's Claude
    // executor. Caller headers and body "betas" never select upstream flags.
    if model.protocol == "anthropic" {
        if payload["fallbacks"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
            || !payload["fallback_credit_token"].is_null()
        {
            return Err(Fault::new(
                400,
                "unsupported_native_feature",
                "Factory 插件尚未启用跨模型 fallback，请移除 fallbacks / fallback_credit_token",
            ));
        }
        let mut flags = vec![ANTHROPIC_BETA];
        if payload["thinking"]["type"].as_str() == Some("enabled") {
            beta(&mut flags, "interleaved-thinking-2025-05-14");
        }
        if payload["thinking"]["display"].as_str() == Some("updates") {
            beta(&mut flags, "thinking-display-updates-2026-08-18");
        }
        if !payload["thinking"]["block_binding"].is_null() {
            beta(&mut flags, "thinking-binding-controls-2026-08-01");
        }
        if payload["speed"].as_str() == Some("fast") {
            beta(&mut flags, "fast-mode-2026-02-01");
        }
        for (field, flag) in [
            ("context_management", "context-management-2025-06-27"),
            ("diagnostics", "cache-diagnosis-2026-04-07"),
            ("safeguards", "dangerous-tool-use-2026-09-03"),
            ("output_format", "structured-outputs-2025-12-15"),
        ] {
            if !payload[field].is_null() {
                beta(&mut flags, flag);
            }
        }
        if !payload["output_config"]["timing"].is_null() {
            beta(&mut flags, "timing-2026-09-09");
        }
        cache_beta(payload, &mut flags);
        content_betas(&payload["system"], &mut flags);
        for tool in payload["tools"].as_array().into_iter().flatten() {
            tool_betas(tool, &mut flags);
        }
        for message in payload["messages"].as_array().into_iter().flatten() {
            if message["role"].as_str() == Some("system") {
                beta(&mut flags, "mid-conversation-system-2026-04-07");
            }
            if !message["clear_at"].is_null() {
                beta(&mut flags, "mid-conversation-system-clear-at-2026-08-21");
            }
            if !message["tools"].is_null() {
                beta(&mut flags, "mid-conversation-tool-changes-2026-07-01");
            }
            if !message["output_config"]["effort"].is_null() {
                beta(&mut flags, "mid-conversation-effort-2026-08-01");
            }
            if !message["output_config"]["timing"].is_null() {
                beta(&mut flags, "timing-2026-09-09");
            }
            content_betas(&message["content"], &mut flags);
        }
        headers["anthropic-version"] = json!([ANTHROPIC_VERSION]);
        headers["anthropic-beta"] = json!([flags.join(",")]);
    }
    crate::catalog::apply_required_headers(headers, model);
    if let Some(values) = headers["anthropic-beta"].as_array() {
        let joined = values
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(",");
        headers["anthropic-beta"] = json!([joined]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_protocol_defaults_match_the_factory_cli_baseline() {
        let model = Model {
            protocol: "anthropic".into(),
            upstream_provider: "anthropic".into(),
            ..Model::default()
        };
        let mut outgoing = json!({});
        apply_protocol_defaults(&mut outgoing, &model, &json!({})).unwrap();
        assert_eq!(outgoing["anthropic-version"], json!(["2023-06-01"]));
        assert_eq!(
            outgoing["anthropic-beta"],
            json!(["fine-grained-tool-streaming-2025-05-14"])
        );
        assert_eq!(outgoing.as_object().unwrap().len(), 2);
    }

    #[test]
    fn trusted_factory_model_requirements_are_applied_after_defaults() {
        let model = Model {
            protocol: "anthropic".into(),
            upstream_provider: "anthropic".into(),
            upstream_id: "claude-opus-5-5-fast".into(),
            ..Model::default()
        };
        let mut outgoing = json!({});
        apply_protocol_defaults(&mut outgoing, &model, &json!({})).unwrap();
        assert_eq!(
            outgoing["anthropic-beta"],
            json!(["fine-grained-tool-streaming-2025-05-14,fast-mode-2026-02-01"])
        );
    }

    #[test]
    fn openai_protocol_defaults_do_not_invent_beta_headers() {
        for protocol in ["responses", "chat_completions"] {
            let model = Model {
                protocol: protocol.into(),
                ..Model::default()
            };
            let mut outgoing = json!({});
            apply_protocol_defaults(&mut outgoing, &model, &json!({})).unwrap();
            assert_eq!(outgoing, json!({}));
        }
    }

    #[test]
    fn feature_flags_follow_protocol_fields_not_client_headers_or_tool_arguments() {
        let model = Model {
            protocol: "anthropic".into(),
            upstream_provider: "anthropic".into(),
            ..Model::default()
        };
        let mut headers = json!({});
        apply_protocol_defaults(&mut headers, &model, &json!({
            "thinking":{"type":"adaptive","display":"updates"},
            "output_config":{"effort":"xhigh"},
            "context_management":{"edits":[{"type":"clear_thinking_20251015","keep":"all"}]},
            "betas":["claude-code-20250219","untrusted-client-beta"],
            "tools":[{"name":"plain","input_schema":{"thinking":{"display":"updates"}}}],
            "messages":[{"role":"assistant","content":[{"type":"tool_use","input":{"diagnostics":{},"safeguards":{}}}]}]
        })).unwrap();
        assert_eq!(headers["anthropic-beta"], json!([
            "fine-grained-tool-streaming-2025-05-14,thinking-display-updates-2026-08-18,context-management-2025-06-27"]));
        let mut ordinary = json!({});
        apply_protocol_defaults(
            &mut ordinary,
            &model,
            &json!({"thinking":{"type":"adaptive"},
            "output_config":{"effort":"xhigh"}}),
        )
        .unwrap();
        assert_eq!(ordinary["anthropic-beta"], json!([ANTHROPIC_BETA]));
    }

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
