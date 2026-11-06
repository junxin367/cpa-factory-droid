//! Bound historical image payloads without changing the caller's saved conversation.
use serde_json::{json, Value};

use crate::{
    host::{decode, encode},
    Fault, Result,
};

// 用户要求按最终 UTF-8 JSON 字节控制 Factory 请求体；4 MB 是缩减目标，
// 为约 4.5 MB 的 Vercel 入口限制留余量，不作为模型上下文或纯文本硬上限。
pub const TARGET_BYTES: usize = 4_000_000;
const OMITTED_IMAGE: &str = "[此历史图片已因上游请求大小限制省略。]";
const SIZE_HINT: &str = "请压缩会话、减少附件或携带摘要开启新会话后重试。";

pub struct Body {
    pub bytes: Vec<u8>,
    pub original_bytes: usize,
    pub omitted_images: usize,
}

fn serialize(value: &Value) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|_| Fault::internal())
}

fn image_paths(content: &Value, prefix: &str, paths: &mut Vec<String>) {
    for (index, block) in content.as_array().into_iter().flatten().enumerate() {
        let path = format!("{prefix}/{index}");
        match block["type"].as_str() {
            Some("image") => paths.push(path),
            // 只遍历原生内容块；不能把工具参数、JSON 示例或思考签名当图片处理。
            Some("tool_result") => {
                image_paths(&block["content"], &format!("{path}/content"), paths)
            }
            _ => {}
        }
    }
}

fn reduce(request: &mut Value, target: usize) -> Result<Body> {
    let original = serialize(request)?;
    let original_bytes = original.len();
    if original_bytes <= target {
        return Ok(Body {
            bytes: original,
            original_bytes,
            omitted_images: 0,
        });
    }
    let mut groups = Vec::new();
    for (index, message) in request["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if message["role"].as_str() != Some("user") {
            continue;
        }
        let mut paths = Vec::new();
        image_paths(
            &message["content"],
            &format!("/messages/{index}/content"),
            &mut paths,
        );
        if !paths.is_empty() {
            groups.push(paths);
        }
    }
    // 用户要求保留当前所需图片：最近一条含图片的用户/工具结果消息全部保留，
    // 多图对比不退化成只留一张。其余图片按原有顺序从最旧开始替换。
    groups.pop();
    let mut remaining = original_bytes;
    let mut omitted_images = 0;
    for path in groups.into_iter().flatten() {
        let block = request.pointer_mut(&path).ok_or_else(Fault::internal)?;
        let mut replacement = json!({"type":"text","text":OMITTED_IMAGE});
        if let Some(cache) = block.get("cache_control") {
            replacement["cache_control"] = cache.clone();
        }
        let old_size = serialize(block)?.len();
        let new_size = serialize(&replacement)?.len();
        if old_size <= new_size {
            continue;
        }
        *block = replacement;
        omitted_images += 1;
        remaining -= old_size - new_size;
        if remaining <= target {
            break;
        }
    }
    Ok(Body {
        bytes: if omitted_images == 0 {
            original
        } else {
            serialize(request)?
        },
        original_bytes,
        omitted_images,
    })
}

pub fn prepare(request: &mut Value) -> Result<Body> {
    reduce(request, TARGET_BYTES)
}

pub fn retry(http: &mut Value) -> Result<Option<Body>> {
    let mut request: Value =
        serde_json::from_slice(&decode(&http["body"])?).map_err(|_| Fault::internal())?;
    // 仅在真实 HTTP 413 后再移除剩余旧图；字节数没有下降时不重发相同请求。
    let body = reduce(&mut request, 0)?;
    if body.omitted_images == 0 {
        return Ok(None);
    }
    http["body"] = json!(encode(&body.bytes));
    Ok(Some(body))
}

pub fn with_hint(mut error: Fault) -> Fault {
    error.message = format!("{}\n{SIZE_HINT}", error.message);
    if let Some(raw) = error.response_body.as_deref() {
        if let Ok(mut value) = serde_json::from_str::<Value>(raw) {
            let detail = if value["error"].is_object() {
                &mut value["error"]
            } else {
                &mut value
            };
            if detail["message"].is_string() {
                // 保留原始错误原因、code 和其他诊断字段，仅给最终 413 追加处理提示。
                detail["message"] = json!(error.message);
                error.response_body = Some(value.to_string());
            }
        }
    }
    error
}
