//! Local text-token estimation, following CPA's third-party Claude gateway path.
//! Segment selection adapts CLIProxyAPI's claude_input_tokens.go (MIT); see
//! docs/third-party-notices.md. This is not Factory's exact or billable usage.
use serde_json::{json, Value};
use std::sync::Arc;

use crate::{
    factory, guard,
    host::{decode, encode},
    protocol::Format,
    Fault, Result, Runtime,
};

fn text(value: &Value, segments: &mut Vec<String>) {
    let value = value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_number().map(ToString::to_string));
    if let Some(value) = value {
        if !value.trim().is_empty() {
            segments.push(value.trim().into());
        }
    }
}

fn structured(value: &Value, segments: &mut Vec<String>) {
    if value.is_null() {
        return;
    }
    if value.is_string() {
        text(value, segments);
    } else {
        segments.push(value.to_string());
    }
}

fn content(value: &Value, segments: &mut Vec<String>) {
    match value {
        Value::String(_) => text(value, segments),
        Value::Array(values) => {
            for value in values {
                content(value, segments);
            }
        }
        Value::Object(_) => match value["type"].as_str().unwrap_or("") {
            "text" => text(&value["text"], segments),
            "thinking" => text(&value["thinking"], segments),
            "document" if value["source"]["type"].as_str() == Some("text") => {
                for field in ["title", "context"] {
                    text(&value[field], segments);
                }
                for field in ["data", "content"] {
                    text(&value["source"][field], segments);
                }
            }
            "tool_use" | "server_tool_use" | "mcp_tool_use" => {
                text(&value["id"], segments);
                text(&value["name"], segments);
                structured(&value["input"], segments);
            }
            "tool_result"
            | "mcp_tool_result"
            | "web_search_tool_result"
            | "web_fetch_tool_result"
            | "code_execution_tool_result"
            | "bash_code_execution_tool_result"
            | "text_editor_code_execution_tool_result" => {
                text(&value["tool_use_id"], segments);
                text(&value["tool_call_id"], segments);
                content(&value["content"], segments);
            }
            "web_search_result" | "search_result" => {
                if value["source"].is_string() {
                    text(&value["source"], segments);
                }
                for field in ["title", "url", "page_age"] {
                    text(&value[field], segments);
                }
                content(&value["content"], segments);
            }
            "web_fetch_result" => {
                for field in ["url", "retrieved_at"] {
                    text(&value[field], segments);
                }
                content(&value["content"], segments);
            }
            "code_execution_result"
            | "bash_code_execution_result"
            | "text_editor_code_execution_result" => {
                for field in ["stdout", "stderr", "return_code"] {
                    text(&value[field], segments);
                }
                content(&value["content"], segments);
                content(&value["output"], segments);
            }
            "tool_reference" => text(&value["tool_name"], segments),
            "image" | "input_audio" | "audio" | "video" | "redacted_thinking" | "document" => {}
            "" => structured(value, segments),
            _ => text(&value["text"], segments),
        },
        _ => {}
    }
}

fn segments(request: &Value) -> Result<Vec<String>> {
    if !request.is_object() || request["stream"].as_bool() == Some(true) {
        return Err(Fault::input("Messages token 计数需要非流式 JSON 对象"));
    }
    let messages = request["messages"]
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| Fault::input("Messages token 计数需要非空 messages 数组"))?;
    let mut segments = Vec::new();
    if request["system"].is_string() {
        text(&request["system"], &mut segments);
    } else if let Some(system) = request["system"].as_array() {
        for part in system {
            if part.is_string() {
                text(part, &mut segments);
            } else if part["type"].as_str() == Some("text") {
                text(&part["text"], &mut segments);
            }
        }
    } else if !request["system"].is_null() {
        return Err(Fault::input("system 必须是字符串或文本块数组"));
    }
    for message in messages {
        if !matches!(
            message["role"].as_str(),
            Some("user" | "assistant" | "system")
        ) || !(message["content"].is_string() || message["content"].is_array())
        {
            return Err(Fault::input("messages 必须包含有效的 role 与 content"));
        }
        if message["content"].as_array().is_some_and(|parts| {
            parts
                .iter()
                .any(|part| !part.is_object() || !part["type"].is_string())
        }) {
            return Err(Fault::input("消息内容块必须包含 type"));
        }
        text(&message["role"], &mut segments);
        content(&message["content"], &mut segments);
    }
    if !request["tools"].is_null() && !request["tools"].is_array() {
        return Err(Fault::input("tools 必须为数组"));
    }
    for tool in request["tools"].as_array().into_iter().flatten() {
        for field in ["type", "name", "description"] {
            text(&tool[field], &mut segments);
        }
        structured(&tool["input_schema"], &mut segments);
    }
    if request["tool_choice"].is_string() {
        text(&request["tool_choice"], &mut segments);
    } else {
        for field in ["type", "name"] {
            text(&request["tool_choice"][field], &mut segments);
        }
    }
    Ok(segments)
}

pub fn execute(rt: &Arc<Runtime>, data: &Value) -> Result<Value> {
    let _active = rt.begin()?;
    let model = rt.model(data["Model"].as_str().unwrap_or(""))?;
    guard::validate_count(rt, data, Format::for_model(&model)?)?;
    let credential = factory::credential(data, "StorageJSON")?;
    factory::ensure_enabled(rt, &credential)?;
    let account = credential["account_id"].as_str().unwrap_or("");
    rt.lock()?.ensure_model_allowed(account, &model)?;
    let bytes = decode(&data["Payload"])?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(Fault::new(
            413,
            "body_too_large",
            "token 计数请求超过 16 MiB",
        ));
    }
    let request: Value =
        serde_json::from_slice(&bytes).map_err(|_| Fault::input("token 计数请求不是有效 JSON"))?;
    let parts = segments(&request)?;
    let count = tiktoken_rs::o200k_base_singleton()
        .encode_ordinary(&parts.join("\n"))
        .len();
    Ok(json!({
        "Payload": encode(json!({"input_tokens":count,"estimated":true,
            "estimator":"o200k_base","media_tokens_included":false}).to_string()),
        "Headers":{"Content-Type":["application/json"],"X-Factory-Token-Count":["estimated-text-only"]},
        "Metadata":{"token_count_estimated":true}
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_text_tools_and_nested_results_without_counting_transport_or_binary_data() {
        let request = json!({"system":"system text","metadata":{"user_id":"private"},
            "messages":[{"role":"user","content":[{"type":"text","text":"你好"},
                {"type":"image","source":{"type":"base64","data":"BINARY"}},
                {"type":"tool_result","tool_use_id":"call1","content":[{"type":"text","text":"result"}]}]}],
            "tools":[{"name":"lookup","description":"Find data","input_schema":{"type":"object"}}],
            "tool_choice":{"type":"auto"}});
        assert_eq!(
            segments(&request).unwrap(),
            vec![
                "system text",
                "user",
                "你好",
                "call1",
                "result",
                "lookup",
                "Find data",
                r#"{"type":"object"}"#,
                "auto"
            ]
        );
        let first = tiktoken_rs::o200k_base_singleton()
            .encode_ordinary("user\nhello")
            .len();
        assert_eq!(first, 3);
    }

    #[test]
    fn rejects_malformed_count_requests() {
        for request in [
            json!([]),
            json!({"messages":[]}),
            json!({"messages":[{"role":"user","content":4}]}),
            json!({"messages":[{"role":"user","content":[{}]}]}),
            json!({"messages":[{"role":"user","content":"hi"}],"stream":true}),
        ] {
            assert!(segments(&request).is_err());
        }
    }
}
