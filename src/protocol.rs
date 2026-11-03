use serde_json::{json, Value};
use std::{collections::HashSet, sync::Arc};

use crate::{
    host::{decode, encode},
    state::Model,
    Fault, Result, Runtime,
};

// Factory's Droid gateway requires this client identity in the system channel.
// All caller-provided native messages and opaque blocks remain in their own schema.
const DROID_IDENTITY: &str = "You are Droid, an AI software engineering agent built by Factory.";
const MAX_FRAME: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Format {
    Chat,
    Claude,
    Responses,
}
impl Format {
    pub fn for_model(model: &Model) -> Result<Self> {
        match model.protocol.as_str() {
            "chat_completions" => Ok(Self::Chat),
            "anthropic" => Ok(Self::Claude),
            "responses" => Ok(Self::Responses),
            _ => Err(Fault::new(
                400,
                "protocol_mismatch",
                "模型未配置受支持的原生协议",
            )),
        }
    }
    pub fn cpa(self) -> &'static str {
        match self {
            Self::Chat => "openai",
            Self::Claude => "claude",
            Self::Responses => "openai-response",
        }
    }
    pub fn endpoint(self) -> &'static str {
        match self {
            Self::Chat => "/v1/chat/completions",
            Self::Claude => "/v1/messages",
            Self::Responses => "/v1/responses",
        }
    }
    pub fn mismatch(self, source: &str, reason: &str) -> Fault {
        Fault::new(
            400,
            "protocol_mismatch",
            &format!(
                "{reason}；该模型只接受 {} 原生协议，当前为 {}。请使用 POST {}。插件不执行协议转换",
                self.cpa(),
                if source.is_empty() { "未知" } else { source },
                self.endpoint()
            ),
        )
    }
    pub fn check_formats(self, data: &Value) -> Result<()> {
        let source = data["SourceFormat"].as_str().unwrap_or("");
        if source != self.cpa() || data["Format"].as_str() != Some(self.cpa()) {
            return Err(self.mismatch(source, "执行器输入或输出格式与模型协议不匹配"));
        }
        Ok(())
    }
}
fn invalid(message: &str) -> Fault {
    Fault::new(400, "invalid_native_request", message)
}
fn has_identity(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| s.starts_with(DROID_IDENTITY))
        || value
            .as_array()
            .and_then(|parts| parts.first())
            .is_some_and(|p| {
                p["type"].as_str() == Some("text")
                    && p["text"]
                        .as_str()
                        .is_some_and(|s| s.starts_with(DROID_IDENTITY))
            })
}
fn prepend_identity(value: &mut Value, arrays: bool) -> Result<()> {
    if has_identity(value) {
        return Ok(());
    }
    match value {
        Value::Null => *value = json!(DROID_IDENTITY),
        Value::String(text) => {
            *text = if text.is_empty() {
                DROID_IDENTITY.into()
            } else {
                format!("{DROID_IDENTITY}\n{text}")
            }
        }
        Value::Array(parts) if arrays => {
            parts.insert(0, json!({"type":"text","text":DROID_IDENTITY}))
        }
        _ => {
            return Err(invalid(
                "原生系统指令格式无效，无法保留其结构并添加 Factory 客户端身份",
            ))
        }
    }
    Ok(())
}
fn contains_image(content: &Value) -> bool {
    content.as_array().is_some_and(|parts| {
        parts.iter().any(|part| {
            matches!(
                part["type"].as_str(),
                Some("image" | "image_url" | "input_image")
            ) || part["type"].as_str() == Some("tool_result") && contains_image(&part["content"])
        })
    })
}
pub fn request(bytes: &[u8], model: &Model, stream: bool) -> Result<Value> {
    let format = Format::for_model(model)?;
    let mut payload: Value =
        serde_json::from_slice(bytes).map_err(|_| invalid("原生请求不是有效 JSON"))?;
    if !payload.is_object() {
        return Err(invalid("原生请求必须为 JSON 对象"));
    }
    if payload
        .get("stream")
        .is_some_and(|v| !v.is_null() && v.as_bool() != Some(stream))
    {
        return Err(invalid("原生请求 stream 字段与宿主执行方式不一致"));
    }
    if stream && payload.get("stream").is_none_or(Value::is_null) {
        payload["stream"] = json!(true);
    }
    if !model.supports_images {
        let messages = if format == Format::Responses {
            &payload["input"]
        } else {
            &payload["messages"]
        };
        if messages
            .as_array()
            .is_some_and(|messages| messages.iter().any(|m| contains_image(&m["content"])))
        {
            return Err(invalid("此模型未声明图片能力，请使用支持图片的原生模型"));
        }
    }
    payload["model"] = json!(model.upstream_id);
    if let Some((field, value)) = crate::catalog::required_parameter(model) {
        if payload
            .get(field)
            .is_some_and(|current| !current.is_null() && current.as_str() != Some(value))
        {
            return Err(invalid(&format!(
                "当前模型要求 {field}={value}；请移除冲突字段或选择对应的标准模型"
            )));
        }
        payload[field] = json!(value);
    }
    match format {
        Format::Chat => {
            let messages = payload["messages"]
                .as_array_mut()
                .ok_or_else(|| invalid("Chat 原生请求必须提供 messages 数组"))?;
            let already = messages
                .iter()
                .find(|m| matches!(m["role"].as_str(), Some("system" | "developer")))
                .is_some_and(|m| has_identity(&m["content"]));
            if !already {
                messages.insert(0, json!({"role":"system","content":DROID_IDENTITY}));
            }
        }
        Format::Claude => {
            if !payload["messages"].is_array() {
                return Err(invalid("Messages 原生请求必须提供 messages 数组"));
            }
            prepend_identity(&mut payload["system"], true)?;
        }
        Format::Responses => prepend_identity(&mut payload["instructions"], false)?,
    }
    Ok(payload)
}
pub fn validate_response(bytes: &[u8], format: Format) -> Result<()> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| {
        Fault::new(
            502,
            "upstream_protocol_error",
            "Factory 原生响应不是有效 JSON",
        )
    })?;
    if !value.is_object() {
        return Err(Fault::new(
            502,
            "upstream_protocol_error",
            "Factory 原生响应必须为 JSON 对象",
        ));
    }
    if value.get("error").is_some_and(|v| !v.is_null())
        || value["type"].as_str() == Some("error")
        || value["status"].as_str() == Some("failed")
    {
        return Err(Fault::new(
            502,
            "factory_response_error",
            "Factory 原生响应包含上游错误",
        ));
    }
    let valid = match format {
        Format::Chat => value["choices"].is_array(),
        Format::Claude => value["content"].is_array(),
        Format::Responses => value["output"].is_array(),
    };
    if !valid {
        return Err(Fault::new(
            502,
            "upstream_protocol_error",
            "Factory 返回的响应与模型原生协议不一致",
        ));
    }
    Ok(())
}

// Observe termination beside the wire stream. It never rewrites event JSON,
// opaque content, IDs, signatures, usage, unknown events, or native framing.
struct Observer {
    format: Format,
    seen: HashSet<u64>,
    finished: HashSet<u64>,
    claude_stop: bool,
    terminal: bool,
}
impl Observer {
    fn new(format: Format) -> Self {
        Self {
            format,
            seen: HashSet::new(),
            finished: HashSet::new(),
            claude_stop: false,
            terminal: false,
        }
    }
    fn observe(&mut self, frame: &[u8]) -> Result<()> {
        let frame = std::str::from_utf8(frame).map_err(|_| {
            Fault::new(502, "upstream_protocol_error", "Factory SSE 不是有效 UTF-8")
        })?;
        let mut event = "";
        let mut data = Vec::new();
        for line in frame.lines() {
            if let Some(value) = line.strip_prefix("event:") {
                event = value.trim();
            }
            if let Some(value) = line.strip_prefix("data:") {
                data.push(value.strip_prefix(' ').unwrap_or(value));
            }
        }
        if data.is_empty() {
            return Ok(());
        }
        let text = data.join("\n");
        if text.trim() == "[DONE]" {
            if self.format != Format::Chat
                || self.seen.is_empty()
                || self.seen.iter().any(|i| !self.finished.contains(i))
            {
                return Err(Fault::new(
                    502,
                    "factory_stream_truncated",
                    "Factory 流缺少原生完成状态，不能以 [DONE] 替代",
                ));
            }
            self.terminal = true;
            return Ok(());
        }
        let value: Value = match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(_)
                if !event.is_empty()
                    && event != "error"
                    && ![
                        "response.created",
                        "response.in_progress",
                        "response.output_item.added",
                        "response.output_item.done",
                        "response.content_part.added",
                        "response.content_part.done",
                        "response.output_text.delta",
                        "response.output_text.done",
                        "response.function_call_arguments.delta",
                        "response.function_call_arguments.done",
                        "response.reasoning_summary_part.added",
                        "response.reasoning_summary_part.done",
                        "response.reasoning_summary_text.delta",
                        "response.reasoning_summary_text.done",
                        "response.reasoning_text.delta",
                        "response.reasoning_text.done",
                        "response.refusal.delta",
                        "response.refusal.done",
                        "response.output_text.annotation.added",
                        "response.completed",
                        "response.incomplete",
                        "response.failed",
                        "response.cancelled",
                    ]
                    .contains(&event)
                    && ![
                        "message_start",
                        "message_delta",
                        "message_stop",
                        "content_block_start",
                        "content_block_delta",
                        "content_block_stop",
                    ]
                    .contains(&event) =>
            {
                return Ok(())
            }
            Err(_) => {
                return Err(Fault::new(
                    502,
                    "upstream_protocol_error",
                    "Factory 已知原生 SSE 事件不是有效 JSON",
                ))
            }
        };
        if event == "error"
            || value.get("error").is_some_and(|v| !v.is_null())
            || value["type"].as_str() == Some("error")
            || matches!(
                value["type"].as_str(),
                Some("response.failed" | "response.cancelled")
            )
        {
            return Err(Fault::new(
                502,
                "factory_stream_error",
                "Factory 原生流返回错误事件",
            ));
        }
        match self.format {
            Format::Chat => {
                for choice in value["choices"].as_array().into_iter().flatten() {
                    let index = choice["index"].as_u64().unwrap_or(0);
                    self.seen.insert(index);
                    if choice["finish_reason"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty())
                    {
                        self.finished.insert(index);
                    }
                }
            }
            Format::Claude => match value["type"].as_str() {
                Some("message_delta") => {
                    if value["delta"]["stop_reason"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty())
                    {
                        self.claude_stop = true;
                    }
                }
                Some("message_stop") => {
                    if !self.claude_stop {
                        return Err(Fault::new(
                            502,
                            "factory_stream_truncated",
                            "Factory Messages 流缺少 stop_reason",
                        ));
                    }
                    self.terminal = true;
                }
                _ => {}
            },
            Format::Responses => {
                if matches!(
                    value["type"].as_str(),
                    Some("response.completed" | "response.incomplete")
                ) {
                    self.terminal = true;
                }
            }
        }
        Ok(())
    }
}
pub fn pump(rt: &Arc<Runtime>, format: Format, upstream: &str, downstream: &str) -> Result<()> {
    let mut buffer = Vec::new();
    let mut observer = Observer::new(format);
    loop {
        let chunk = rt
            .host
            .call("host.http.stream_read", &json!({"stream_id":upstream}))?;
        if chunk["error"].as_str().is_some_and(|s| !s.is_empty()) {
            return Err(Fault::new(
                502,
                "factory_stream_error",
                "Factory 上游连接中断或请求已取消",
            ));
        }
        buffer.extend_from_slice(&decode(&chunk["payload"])?);
        loop {
            let lf = buffer.windows(2).position(|w| w == b"\n\n").map(|p| (p, 2));
            let crlf = buffer
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|p| (p, 4));
            let boundary = match (lf, crlf) {
                (Some(a), Some(b)) => Some(if a.0 < b.0 { a } else { b }),
                (a, b) => a.or(b),
            };
            let Some((end, separator)) = boundary else {
                break;
            };
            if end > MAX_FRAME {
                return Err(Fault::new(
                    502,
                    "stream_frame_too_large",
                    "Factory SSE 单帧超过大小限制",
                ));
            }
            let frame: Vec<u8> = buffer.drain(..end + separator).collect();
            let observed = observer.observe(&frame);
            // A valid provider error event remains native on the wire, then
            // closes with a host error. Invalid JSON/termination must never be
            // delivered as if it were a usable completion signal.
            if let Err(error) = &observed {
                if error.code != "factory_stream_error" {
                    return observed;
                }
            }
            rt.host.call(
                "host.stream.emit",
                &json!({"stream_id":downstream,"payload":encode(&frame)}),
            )?;
            observed?;
            if observer.terminal {
                return Ok(());
            }
        }
        if buffer.len() > MAX_FRAME {
            return Err(Fault::new(
                502,
                "stream_frame_too_large",
                "Factory SSE 单帧超过大小限制",
            ));
        }
        if chunk["done"].as_bool() == Some(true) {
            return Err(Fault::new(
                502,
                "factory_stream_truncated",
                "Factory 流式连接提前结束，未收到原生完成事件",
            ));
        }
    }
}
