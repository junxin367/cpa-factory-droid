//! Public Factory model metadata; no credentials or inference requests are used.
use serde_json::{json, Value};
use std::collections::BTreeSet;

use crate::{host::decode, state::now, Fault, Result, Runtime};

pub const SOURCE: &str = "official_factory_docs";
pub const URL: &str = "https://docs.factory.com/models.md";
pub const REFRESH_HOURS: i64 = 24;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const RETRY_SECONDS: i64 = 300;

#[derive(Default)]
pub struct Cache {
    catalog: Option<Value>,
    checked_at: Option<chrono::DateTime<chrono::Utc>>,
    failed: bool,
}

pub fn bundled_candidates() -> Vec<Value> {
    static SNAPSHOT: std::sync::OnceLock<Vec<Value>> = std::sync::OnceLock::new();
    SNAPSHOT
        .get_or_init(|| {
            serde_json::from_str::<Value>(include_str!("factory-model-catalog.json"))
                .expect("embedded Factory model catalog must be valid JSON")["candidates"]
                .as_array()
                .expect("embedded Factory model catalog must contain candidates")
                .clone()
        })
        .clone()
}

pub fn refresh_due(cred: &Value) -> bool {
    cred["factory_catalog_source"].as_str() != Some(SOURCE)
        || cred["factory_catalog_checked_at"]
            .as_str()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .is_none_or(|at| {
                chrono::Utc::now() - at.with_timezone(&chrono::Utc)
                    >= chrono::Duration::seconds(if cred["factory_catalog_stale"] == true {
                        RETRY_SECONDS
                    } else {
                        REFRESH_HOURS * 3600
                    })
            })
}

pub fn invalidate(rt: &Runtime, min_age_seconds: i64) -> Result<()> {
    let mut cache = rt.public_catalog.lock().map_err(|_| Fault::internal())?;
    if cache
        .checked_at
        .is_none_or(|at| chrono::Utc::now() - at >= chrono::Duration::seconds(min_age_seconds))
    {
        cache.checked_at = None;
    }
    Ok(())
}

pub fn refresh(rt: &std::sync::Arc<Runtime>, cred: &mut Value, callback: &str) -> Result<()> {
    // Serialize the public fetch across credentials. Reuse a successful result for
    // 24 hours and throttle failed attempts without claiming that they succeeded.
    let mut cache = rt.public_catalog.lock().map_err(|_| Fault::internal())?;
    let current = chrono::Utc::now();
    let interval = chrono::Duration::seconds(if cache.failed {
        RETRY_SECONDS
    } else {
        REFRESH_HOURS * 3600
    });
    if cache.checked_at.is_none_or(|at| current - at >= interval) {
        match fetch(rt, callback) {
            Ok(candidates) => {
                cache.catalog = Some(json!({
                    "version":1, "source":SOURCE, "candidates":candidates, "fetched_at":now()
                }));
                cache.failed = false;
            }
            Err(_) => {
                cache.failed = true;
                rt.host
                    .log("Factory 官方模型目录同步失败，保留上次目录，5 分钟后重试");
            }
        }
        cache.checked_at = Some(current);
    }
    apply(&cache, cred, current);
    Ok(())
}

pub fn apply_cached(rt: &Runtime, cred: &mut Value) -> Result<()> {
    let cache = rt.public_catalog.lock().map_err(|_| Fault::internal())?;
    apply(&cache, cred, chrono::Utc::now());
    Ok(())
}

fn apply(cache: &Cache, cred: &mut Value, current: chrono::DateTime<chrono::Utc>) {
    let catalog = cache.catalog.clone().unwrap_or_else(|| {
        if cred["factory_catalog"]["version"] == 1
            && cred["factory_catalog"]["candidates"]
                .as_array()
                .is_some_and(|rows| !rows.is_empty())
        {
            cred["factory_catalog"].clone()
        } else {
            json!({
                "version":1, "source":"bundled_registry",
                "candidates":bundled_candidates(), "fetched_at":"2026-11-11T00:00:00Z"
            })
        }
    });
    cred["factory_catalog_source"] = catalog["source"]
        .as_str()
        .map(|source| json!(source))
        .unwrap_or_else(|| json!("official_droid_cli"));
    cred["factory_catalog"] = catalog;
    cred["factory_catalog_checked_at"] = json!(cache
        .checked_at
        .unwrap_or(current)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    cred["factory_catalog_stale"] = json!(cache.failed);
}

fn fetch(rt: &std::sync::Arc<Runtime>, callback: &str) -> Result<Vec<Value>> {
    let url = if rt.config.allow_local_mock {
        format!("{}/models.md", rt.api_base())
    } else {
        URL.into()
    };
    let operation = rt.operation(callback, Some(15))?;
    let response = rt.host.call(
        "host.http.do",
        &json!({
            "host_callback_id":callback,"operation_id":operation.id,
            "method":"GET","url":url,
            "headers":{"Accept":["text/markdown"],"User-Agent":["cpa-factory-droid"]}
        }),
    )?;
    if response["StatusCode"].as_u64() != Some(200) {
        return Err(invalid());
    }
    let body = decode(&response["Body"])?;
    if body.len() > MAX_BYTES {
        return Err(invalid());
    }
    parse(std::str::from_utf8(&body).map_err(|_| invalid())?)
}

fn invalid() -> Fault {
    Fault::new(
        502,
        "invalid_public_catalog",
        "Factory 官方模型目录格式无效，已保留上次目录",
    )
}

fn plain_text(value: &str) -> String {
    let mut text = String::new();
    let mut tag = false;
    for ch in value.chars() {
        match ch {
            '<' => tag = true,
            '>' => tag = false,
            _ if !tag => text.push(ch),
            _ => {}
        }
    }
    text.trim().to_owned()
}

pub fn parse(markdown: &str) -> Result<Vec<Value>> {
    if markdown.len() > MAX_BYTES || !markdown.starts_with("# Available Models") {
        return Err(invalid());
    }
    let mut provider = None;
    let mut seen = BTreeSet::new();
    let mut providers = BTreeSet::new();
    let mut candidates = Vec::new();
    for line in markdown.lines().map(str::trim) {
        if let Some(heading) = line.strip_prefix("## ") {
            provider = match plain_text(heading).as_str() {
                "Anthropic" => Some("anthropic"),
                "OpenAI" => Some("openai"),
                "Google" => Some("google"),
                "xAI" => Some("xai"),
                "Droid Core (Open Models)" => Some("factory"),
                _ => None,
            };
            continue;
        }
        let Some(provider) = provider else {
            continue;
        };
        if !line.starts_with('|') {
            continue;
        }
        let columns: Vec<_> = line.trim_matches('|').split('|').map(str::trim).collect();
        if columns.len() != 4 || !columns[1].starts_with('`') {
            continue;
        }
        let id = columns[1]
            .strip_prefix('`')
            .and_then(|value| value.strip_suffix('`'))
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= 152
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            })
            .ok_or_else(invalid)?;
        let multiplier = columns[2]
            .trim_end_matches('×')
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && (0.0..=1_000_000.0).contains(value))
            .ok_or_else(invalid)?;
        if !seen.insert(id.to_owned()) || candidates.len() >= 1024 {
            return Err(invalid());
        }
        providers.insert(provider);
        let name = plain_text(columns[0]);
        let name = name.trim_end_matches(['*', '†', '‡', '\\']).trim();
        if name.is_empty() || name.len() > 200 {
            return Err(invalid());
        }
        candidates.push(json!({
            "id":id, "displayName":name, "modelProvider":provider,
            "tokenMultiplier":multiplier, "deprecated":columns[0].contains('‡')
        }));
    }
    // Reject login/error pages and broken/partial tables rather than replacing
    // a working catalog with an apparently successful incomplete snapshot.
    if !["anthropic", "openai", "factory"]
        .iter()
        .all(|provider| providers.contains(provider))
    {
        return Err(invalid());
    }
    Ok(candidates)
}
