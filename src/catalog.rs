//! Native routes from the official Droid model catalogue and CLI 1.2.0 registry.
//! `modelProvider` identifies a model family; Factory-hosted models additionally
//! need the registry's apiModelProvider/apiProviders. Never infer from an ID prefix.
use serde_json::{json, Value};
use std::sync::OnceLock;

use crate::state::Model;

pub fn apply_capabilities(model: &mut Model, candidate: Option<&Value>) -> bool {
    if model.quota_pool_source != "factory_registry" {
        return false;
    }
    static SNAPSHOT: OnceLock<Value> = OnceLock::new();
    let snapshot = SNAPSHOT.get_or_init(|| {
        serde_json::from_str(include_str!("factory-model-capabilities.json"))
            .expect("embedded Factory capability snapshot must be valid JSON")
    });
    let baseline = &snapshot["models"][&model.upstream_id];
    let candidate = candidate.unwrap_or(&Value::Null);
    let before = (
        model.context_length,
        model.max_output_tokens,
        model.supports_images,
    );
    let positive = |value: &Value| value.as_u64().filter(|value| *value > 0);
    model.context_length = positive(&candidate["maxInputTokens"])
        .or_else(|| positive(&candidate["contextLimits"]["maxInputTokens"]))
        .or_else(|| positive(&baseline["context_length"]))
        .unwrap_or(0);
    model.max_output_tokens = positive(&candidate["maxOutputTokens"])
        .or_else(|| positive(&candidate["contextLimits"]["maxOutputTokens"]))
        .or_else(|| positive(&baseline["max_output_tokens"]))
        .unwrap_or(0);
    model.supports_images = candidate["noImageSupport"]
        .as_bool()
        .map(|no| !no)
        .or_else(|| baseline["supports_images"].as_bool());
    before
        != (
            model.context_length,
            model.max_output_tokens,
            model.supports_images,
        )
}

pub fn route(candidate: &Value) -> Option<(&'static str, &'static str, &'static str)> {
    match candidate["modelProvider"].as_str()? {
        "anthropic" => Some(("anthropic", "anthropic", "standard")),
        "openai" => Some(("responses", "openai", "standard")),
        "xai" => Some(("responses", "xai", "standard")),
        "factory" => match candidate["id"].as_str()? {
            "inkling"
            | "glm-5.3-flash"
            | "glm-5.3"
            | "kimi-k3"
            | "qwen3.8-max"
            | "deepseek-v4.1-flash"
            | "deepseek-v4.1-flash-1017"
            | "deepseek-v4-flash-0731"
            | "deepseek-v4-pro"
            | "minimax-m3"
            | "minimax-m3.1-flash" => Some(("chat_completions", "fireworks", "core")),
            "glm-5.2" | "glm-5.2-fast" | "nemotron-3-ultra" | "nemotron-3.5-super" | "beam-1.0" => {
                Some(("chat_completions", "baseten", "core"))
            }
            "mistral-large-4" | "mistral-medium-3.5" => {
                Some(("chat_completions", "mistral", "core"))
            }
            "muse-spark-1.2" => Some(("chat_completions", "fireworks", "standard")),
            "minimax-m2.7" => Some(("anthropic", "fireworks", "core")),
            _ => None,
        },
        // Google uses a native protocol this plugin does not implement.
        // Auto routers and custom/BYOK models cannot be assigned one native route.
        _ => None,
    }
}

pub fn model(candidate: &Value) -> Option<Model> {
    let (protocol, provider, pool) = route(candidate)?;
    let upstream = candidate["id"].as_str()?;
    let id = format!("factory/{upstream}");
    if upstream.is_empty()
        || id.len() > 160
        || !upstream
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        return None;
    }
    let multiplier = candidate["tokenMultiplier"]
        .as_f64()
        .filter(|n| n.is_finite() && (0.0..=1_000_000.0).contains(n))
        .unwrap_or(1.0);
    let name = candidate["displayName"].as_str().unwrap_or(upstream);
    let mut model = Model {
        id,
        upstream_id: upstream.into(),
        display_name: if name.len() <= 200 { name } else { upstream }.into(),
        protocol: protocol.into(),
        upstream_provider: provider.into(),
        input_multiplier: multiplier,
        output_multiplier: multiplier,
        cache_multiplier: multiplier,
        cache_creation_multiplier: multiplier,
        quota_pool: pool.into(),
        quota_pool_source: "factory_registry".into(),
        ..Model::default()
    };
    apply_capabilities(&mut model, Some(candidate));
    Some(model)
}

/// Exact variant settings in the official registry, not a `-fast` suffix guess.
pub fn required_parameter(model: &Model) -> Option<(&'static str, &'static str)> {
    match (
        model.protocol.as_str(),
        model.upstream_provider.as_str(),
        model.upstream_id.as_str(),
    ) {
        (
            "anthropic",
            "anthropic",
            "claude-opus-5-5-fast" | "claude-opus-5-fast" | "claude-opus-4-8-fast",
        ) => Some(("speed", "fast")),
        ("responses", "openai", "gpt-6.1-sol-ultrafast") => Some(("service_tier", "ultrafast")),
        (
            "responses",
            "openai",
            "gpt-6-sol-fast" | "gpt-5.6-sol-fast" | "gpt-6-luna-fast" | "gpt-5.5-fast"
            | "gpt-5.4-fast" | "gpt-5.4-mini-fast" | "gpt-5.3-codex-fast",
        ) => Some(("service_tier", "priority")),
        _ => None,
    }
}

pub fn beta_flag(model: &Model) -> Option<&'static str> {
    (required_parameter(model) == Some(("speed", "fast"))).then_some("fast-mode-2026-02-01")
}

pub fn apply_required_headers(headers: &mut Value, model: &Model) {
    if let Some(beta) = beta_flag(model) {
        let mut values = headers["anthropic-beta"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if !values
            .iter()
            .filter_map(Value::as_str)
            .flat_map(|value| value.split(','))
            .any(|value| value.trim() == beta)
        {
            values.push(json!(beta));
        }
        headers["anthropic-beta"] = json!(values);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovered_capabilities_override_snapshot_and_unknown_models_remain_unknown() {
        let known = model(&json!({"id":"gpt-5.4","modelProvider":"openai"})).unwrap();
        assert_eq!(
            (known.context_length, known.max_output_tokens),
            (922000, 128000)
        );
        let updated = model(&json!({"id":"gpt-5.4","modelProvider":"openai",
            "maxInputTokens":1500000,"maxOutputTokens":250000,"noImageSupport":true}))
        .unwrap();
        assert_eq!(
            (updated.context_length, updated.max_output_tokens),
            (1500000, 250000)
        );
        assert_eq!(updated.supports_images, Some(false));
        let unknown = model(&json!({"id":"future-model","modelProvider":"openai"})).unwrap();
        let public = unknown.host_json();
        for field in [
            "ContextLength",
            "InputTokenLimit",
            "OutputTokenLimit",
            "MaxCompletionTokens",
            "SupportedInputModalities",
        ] {
            assert!(
                public.get(field).is_none(),
                "{field} must not advertise guessed capabilities"
            );
        }
    }

    #[test]
    fn only_explicit_image_incompatibility_rejects_images() {
        let mut unknown = model(&json!({"id":"future-model","modelProvider":"openai"})).unwrap();
        let payload = json!({"input":[{"role":"user","content":[
            {"type":"input_image","image_url":"data:image/png;base64,AAAA"}]}]})
        .to_string();
        assert!(crate::protocol::request(payload.as_bytes(), &unknown, false).is_ok());
        unknown.supports_images = Some(false);
        assert!(crate::protocol::request(payload.as_bytes(), &unknown, false).is_err());
    }

    #[test]
    fn explicit_model_configuration_is_not_replaced_by_registry_refresh() {
        let mut configured = Model {
            upstream_id: "gpt-5.4".into(),
            quota_pool_source: "explicit_config".into(),
            context_length: 123456,
            max_output_tokens: 24000,
            supports_images: Some(false),
            ..Model::default()
        };
        assert!(!apply_capabilities(&mut configured, None));
        assert_eq!(
            (configured.context_length, configured.max_output_tokens),
            (123456, 24000)
        );
        assert_eq!(configured.supports_images, Some(false));
    }
}
