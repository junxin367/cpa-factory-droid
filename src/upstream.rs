//! Preserve upstream errors, with bounded reads and a separate error-body deadline.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::{
    sync::{Condvar, Mutex},
    time::Duration,
};

use crate::{factory, guard, host::Host, Fault};

const BODY_LIMIT: usize = 32 * 1024;

pub fn body_prefix(value: &Value) -> Vec<u8> {
    let Some(encoded) = value.as_str() else {
        return Vec::new();
    };
    let end = encoded.len().min(BODY_LIMIT.div_ceil(3) * 4);
    let mut bytes = STANDARD
        .decode(&encoded.as_bytes()[..end])
        .unwrap_or_default();
    bytes.truncate(BODY_LIMIT);
    bytes
}

pub fn read_error_stream(host: Host, stream: &str) -> Vec<u8> {
    if stream.is_empty() {
        return Vec::new();
    }
    let finished = (Mutex::new(false), Condvar::new());
    std::thread::scope(|scope| {
        // Closing the error stream cancels only its own HTTP operation.
        scope.spawn(|| {
            let (lock, cv) = &finished;
            if let Ok(done) = lock.lock() {
                if let Ok((done, wait)) =
                    cv.wait_timeout_while(done, Duration::from_secs(2), |v| !*v)
                {
                    if !*done && wait.timed_out() {
                        let _ = host.call("host.http.stream_close", &json!({"stream_id":stream}));
                    }
                }
            }
        });
        let mut body = Vec::new();
        for _ in 0..64 {
            let Ok(chunk) = host.call("host.http.stream_read", &json!({"stream_id":stream})) else {
                break;
            };
            let bytes = body_prefix(&chunk["payload"]);
            body.extend_from_slice(&bytes[..bytes.len().min(BODY_LIMIT - body.len())]);
            if body.len() == BODY_LIMIT
                || chunk["done"].as_bool() == Some(true)
                || chunk["error"].as_str().is_some_and(|e| !e.is_empty())
                || serde_json::from_slice::<Value>(&body).is_ok()
            {
                break;
            }
        }
        if let Ok(mut done) = finished.0.lock() {
            *done = true;
            finished.1.notify_all();
        }
        let _ = host.call("host.http.stream_close", &json!({"stream_id":stream}));
        body
    })
}

pub fn fault(status: u16, headers: &Value, body: &[u8]) -> Fault {
    let mut error = factory::upstream_error(status, headers);
    let raw = String::from_utf8_lossy(&body[..body.len().min(BODY_LIMIT)]).into_owned();
    let parsed = serde_json::from_str::<Value>(&raw).unwrap_or(Value::Null);
    let detail = if parsed["error"].is_object() {
        &parsed["error"]
    } else {
        &parsed
    };
    if !raw.is_empty() {
        error.message = detail["message"].as_str().unwrap_or(&raw).to_owned();
        if parsed.is_object() {
            // Original error contents are intentional: no sanitizing,
            // translated prefix, field removal, or upstream message replacement.
            error.response_body = Some(raw);
        } else if body.len() >= BODY_LIMIT {
            error
                .message
                .push_str("\n[上游错误正文已达到 32 KiB 读取上限]");
        }
    }
    error.upstream_type = detail["type"].as_str().map(str::to_owned);
    error.request_id = ["request-id", "x-request-id", "cf-ray"]
        .iter()
        .find_map(|name| guard::header_values(headers, name).into_iter().next())
        .or_else(|| parsed["request_id"].as_str())
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .map(str::to_owned);
    error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_error_body_and_message_are_not_sanitized_or_replaced() {
        let original = "{ \"error\": {\"type\":\"invalid_request_error\", \"message\":\"thinking.display: 'updates' rejected; api_key=fk-test-private; user@example.test\"}, \"request_id\":\"req_123\" }";
        let error = fault(400, &json!({}), original.as_bytes());
        assert_eq!(error.request_id.as_deref(), Some("req_123"));
        assert!(error.message.contains("api_key=fk-test-private"));
        assert!(error.message.contains("'updates'"));
        assert_eq!(guard::http_fault(error).message, original);
    }

    #[test]
    fn text_errors_and_empty_errors_keep_http_status_and_retry_metadata() {
        let error = fault(
            429,
            &json!({"Retry-After":["7"],"CF-Ray":["trace-PDX"]}),
            b"<html>original gateway error</html>",
        );
        assert_eq!(error.message, "<html>original gateway error</html>");
        assert_eq!(error.http_status, 429);
        assert_eq!(error.retry_after_seconds, Some(7));
        assert_eq!(error.request_id.as_deref(), Some("trace-PDX"));
        assert_eq!(
            guard::error_body(&error)["error"]["type"],
            "rate_limit_error"
        );
        assert_eq!(
            body_prefix(&json!(STANDARD.encode(vec![b'x'; BODY_LIMIT * 2]))).len(),
            BODY_LIMIT
        );
        assert!(body_prefix(&json!("不是 base64")).is_empty());
        let capped = fault(400, &json!({}), &vec![b'x'; BODY_LIMIT + 1]);
        assert!(capped
            .message
            .ends_with("[上游错误正文已达到 32 KiB 读取上限]"));
        assert_eq!(fault(503, &json!({}), b"").http_status, 503);
    }
}
