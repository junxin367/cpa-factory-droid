//! Preserve upstream diagnostics with bounded memory and the host request lifecycle.
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::{json, Value};
use std::collections::HashSet;

use crate::{factory, guard, host::Host, Fault};

pub const DEFAULT_BODY_LIMIT: usize = 1024 * 1024;

pub fn body_prefix(value: &Value, limit: usize) -> Vec<u8> {
    let Some(encoded) = value.as_str() else {
        return Vec::new();
    };
    let end = encoded.len().min(limit.div_ceil(3) * 4);
    let mut bytes = STANDARD
        .decode(&encoded.as_bytes()[..end])
        .unwrap_or_default();
    bytes.truncate(limit);
    bytes
}

pub fn read_error_stream(host: Host, stream: &str, limit: usize) -> Vec<u8> {
    if stream.is_empty() {
        return Vec::new();
    }
    let mut body = Vec::new();
    // One extra byte distinguishes truncation from an exactly-sized response.
    while body.len() <= limit {
        let Ok(chunk) = host.call("host.http.stream_read", &json!({"stream_id":stream})) else {
            break;
        };
        let bytes = body_prefix(&chunk["payload"], limit + 1 - body.len());
        body.extend_from_slice(&bytes);
        if body.len() > limit
            || chunk["done"].as_bool() == Some(true)
            || chunk["error"].as_str().is_some_and(|e| !e.is_empty())
            || serde_json::from_slice::<Value>(&body).is_ok()
        {
            break;
        }
    }
    let _ = host.call("host.http.stream_close", &json!({"stream_id":stream}));
    body
}

pub fn fault(status: u16, headers: &Value, body: &[u8], limit: usize) -> Fault {
    let mut error = factory::upstream_error(status, headers);
    let raw = String::from_utf8_lossy(&body[..body.len().min(limit)]).into_owned();
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
        } else if body.len() > limit {
            error
                .message
                .push_str(&format!("\n[上游错误正文超过 {limit} 字节，已截断]"));
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

pub fn retry_after(headers: &Value, now: DateTime<Utc>) -> Option<u64> {
    guard::header_values(headers, "retry-after")
        .into_iter()
        .find_map(|raw| {
            let raw = raw.trim();
            if let Ok(seconds) = raw.parse::<i64>() {
                return (seconds > 0).then_some(seconds as u64);
            }
            let date = DateTime::parse_from_rfc2822(raw)
                .map(|date| date.with_timezone(&Utc))
                .ok()
                .or_else(|| {
                    ["%A, %d-%b-%y %H:%M:%S GMT", "%a %b %e %H:%M:%S %Y"]
                        .into_iter()
                        .find_map(|format| {
                            NaiveDateTime::parse_from_str(raw, format)
                                .ok()
                                .map(|date| date.and_utc())
                        })
                })?;
            let delay = date.signed_duration_since(now).to_std().ok()?;
            let seconds = delay.as_secs() + u64::from(delay.subsec_nanos() != 0);
            (seconds > 0).then_some(seconds)
        })
}

pub fn response_headers(headers: &Value, stream: bool) -> Value {
    // CPA applies its passthrough setting and header filter after this response.
    let scoped: HashSet<_> = guard::header_values(headers, "connection")
        .into_iter()
        .flat_map(|value| value.split(','))
        .map(|name| name.trim().to_ascii_lowercase())
        .collect();
    let mut output = serde_json::Map::new();
    for (name, values) in headers.as_object().into_iter().flatten() {
        let lower = name.to_ascii_lowercase();
        if scoped.contains(&lower)
            || [
                "connection",
                "keep-alive",
                "proxy-authenticate",
                "proxy-authorization",
                "te",
                "trailer",
                "transfer-encoding",
                "upgrade",
                "set-cookie",
                "content-length",
                "content-encoding",
                "content-type",
                "x-cpa-trace-id",
            ]
            .contains(&lower.as_str())
            || lower.starts_with("access-control-")
        {
            continue;
        }
        output.insert(name.clone(), values.clone());
    }
    output.insert(
        "Content-Type".into(),
        json!([if stream {
            "text/event-stream"
        } else {
            "application/json"
        }]),
    );
    if stream
        && !output
            .keys()
            .any(|key| key.eq_ignore_ascii_case("cache-control"))
    {
        output.insert("Cache-Control".into(), json!(["no-cache"]));
    }
    Value::Object(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_error_body_and_message_are_not_sanitized_or_replaced() {
        let original = "{ \"error\": {\"type\":\"invalid_request_error\", \"message\":\"thinking.display: 'updates' rejected; api_key=fk-test-private; user@example.test\"}, \"request_id\":\"req_123\" }";
        let error = fault(400, &json!({}), original.as_bytes(), DEFAULT_BODY_LIMIT);
        assert_eq!(error.request_id.as_deref(), Some("req_123"));
        assert!(error.message.contains("api_key=fk-test-private"));
        assert!(error.message.contains("'updates'"));
        assert_eq!(guard::http_fault(error).message, original);
    }

    #[test]
    fn final_size_error_keeps_original_diagnostics_and_adds_actionable_guidance() {
        let original = json!({"error":{"code":"413","type":"request_too_large",
            "message":"Request Entity Too Large","param":"body"},"request_id":"req_size"});
        let error = crate::request_size::with_hint(fault(
            413,
            &json!({"Retry-After":["7"]}),
            original.to_string().as_bytes(),
            DEFAULT_BODY_LIMIT,
        ));
        assert_eq!(error.http_status, 413);
        assert_eq!(error.retry_after_seconds, Some(7));
        let forwarded: Value = serde_json::from_str(&guard::http_fault(error).message).unwrap();
        assert!(forwarded["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("Request Entity Too Large\n"));
        assert!(forwarded["error"]["message"]
            .as_str()
            .unwrap()
            .contains("压缩会话"));
        let mut expected = original;
        expected["error"]["message"] = forwarded["error"]["message"].clone();
        assert_eq!(forwarded, expected);
    }

    #[test]
    fn text_errors_and_empty_errors_keep_http_status_and_retry_metadata() {
        let error = fault(
            429,
            &json!({"Retry-After":["7"],"CF-Ray":["trace-PDX"]}),
            b"<html>original gateway error</html>",
            DEFAULT_BODY_LIMIT,
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
            body_prefix(
                &json!(STANDARD.encode(vec![b'x'; DEFAULT_BODY_LIMIT * 2])),
                DEFAULT_BODY_LIMIT
            )
            .len(),
            DEFAULT_BODY_LIMIT
        );
        assert!(body_prefix(&json!("不是 base64"), DEFAULT_BODY_LIMIT).is_empty());
        let capped = fault(
            400,
            &json!({}),
            &vec![b'x'; DEFAULT_BODY_LIMIT + 1],
            DEFAULT_BODY_LIMIT,
        );
        assert!(capped
            .message
            .ends_with("[上游错误正文超过 1048576 字节，已截断]"));
        assert_eq!(
            fault(503, &json!({}), b"", DEFAULT_BODY_LIMIT).http_status,
            503
        );
    }

    #[test]
    fn retry_after_accepts_long_delays_http_dates_and_rounds_up() {
        let now = DateTime::parse_from_rfc3339("2026-11-05T00:00:00.500Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            retry_after(&json!({"Retry-After":["604800"]}), now),
            Some(604800)
        );
        for date in [
            "Thu, 05 Nov 2026 00:00:08 GMT",
            "Thursday, 05-Nov-26 00:00:08 GMT",
            "Thu Nov  5 00:00:08 2026",
        ] {
            assert_eq!(retry_after(&json!({"Retry-After":[date]}), now), Some(8));
        }
        for raw in ["-1", "0", "invalid", "Wed, 04 Nov 2026 00:00:00 GMT"] {
            assert_eq!(retry_after(&json!({"Retry-After":[raw]}), now), None);
        }
    }

    #[test]
    fn response_diagnostics_survive_without_transport_or_cookie_headers() {
        let headers = response_headers(
            &json!({
                "Request-Id":["req-test"], "X-RateLimit-Remaining":["12"], "Retry-After":["3"],
                "Connection":["keep-alive, X-Internal"], "X-Internal":["private"],
                "Content-Length":["123"], "Set-Cookie":["private"], "Content-Encoding":["gzip"],
                "Access-Control-Allow-Origin":["*"], "X-Cpa-Trace-Id":["untrusted"]
            }),
            true,
        );
        assert_eq!(headers["Request-Id"], json!(["req-test"]));
        assert_eq!(headers["X-RateLimit-Remaining"], json!(["12"]));
        assert_eq!(headers["Retry-After"], json!(["3"]));
        assert_eq!(headers["Content-Type"], json!(["text/event-stream"]));
        assert_eq!(headers.as_object().unwrap().len(), 5);
    }
}
