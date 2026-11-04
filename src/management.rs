use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
use uuid::Uuid;

use crate::{
    access, factory,
    host::{decode, encode},
    state::{now, Settings},
    Fault, Result, Runtime, PROVIDER,
};

pub const PREFIX: &str = "/v0/management/plugins/factory-droid";
pub const RESOURCE: &str = "/v0/resource/plugins/factory-droid";

#[derive(Clone)]
pub(crate) struct CatalogObservation {
    pub revision: String,
    offered: BTreeSet<String>,
    host_managed: bool,
    empty_allowed: bool,
}

impl CatalogObservation {
    pub fn new(data: &Value, credential: &Value, offered: BTreeSet<String>) -> Self {
        let populated = |value: &Value| match value {
            Value::Array(rows) => !rows.is_empty(),
            Value::String(text) => {
                let text = text.trim();
                !text.is_empty() && text != "[]" && text != "{}"
            }
            _ => false,
        };
        let host_managed = populated(&data["Host"]["ExcludedModels"][PROVIDER])
            || populated(&data["Host"]["OAuthModelAlias"][PROVIDER])
            || populated(&data["Attributes"]["excluded_models"])
            || populated(&data["Attributes"]["model_aliases"])
            || populated(&credential["prefix"]);
        let local = data["Attributes"]["excluded_models"].as_str().unwrap_or("");
        let exclusions: Vec<&str> = if !local.trim().is_empty() {
            local
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .collect()
        } else {
            data["Host"]["ExcludedModels"][PROVIDER]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect()
        };
        let empty_allowed = offered
            .iter()
            .all(|model| exclusions.iter().any(|pattern| wildcard(pattern, model)));
        Self {
            revision: credential["factory_catalog_revision"]
                .as_str()
                .unwrap_or("")
                .into(),
            offered,
            host_managed,
            empty_allowed,
        }
    }
}

fn publication_matches(
    enabled: bool,
    desired: &BTreeSet<String>,
    published: Option<&BTreeSet<String>>,
    observed: Option<&CatalogObservation>,
) -> bool {
    if !enabled {
        return published.is_none_or(BTreeSet::is_empty);
    }
    let (Some(published), Some(observed)) = (published, observed) else {
        return false;
    };
    if observed.offered != *desired {
        return false;
    }
    // CPA owns exclusion, alias (including fork), and prefix rules. ABI 10
    // exposes the final registry, but not the complete global rule semantics.
    // Confirm delivery of this catalog, then accept the host's visible set.
    // Without host rules, keep the strict full-catalog publication check.
    (observed.host_managed && (!published.is_empty() || observed.empty_allowed))
        || published == desired
}

fn wildcard(pattern: &str, value: &str) -> bool {
    let pattern = pattern.trim().to_lowercase();
    let value = value.trim().to_lowercase();
    let parts: Vec<_> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == value;
    }
    let Some(mut rest) = value.strip_prefix(parts[0]) else {
        return false;
    };
    for part in &parts[1..parts.len() - 1] {
        let Some(offset) = rest.find(part) else {
            return false;
        };
        rest = &rest[offset + part.len()..];
    }
    rest.ends_with(parts[parts.len() - 1])
}

#[cfg(test)]
mod publication_tests {
    use super::*;

    fn models(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn native_rules_may_filter_rename_or_hide_all_models() {
        let offered = models(&["factory/a", "factory/b"]);
        for context in [
            json!({"Host":{"ExcludedModels":{PROVIDER:["factory/*"]}}}),
            json!({"Host":{"OAuthModelAlias":{PROVIDER:[{"Name":"factory/a","Alias":"custom"}]}}}),
            json!({"Attributes":{"excluded_models":"factory/b"}}),
            json!({"Attributes":{"model_aliases":"[{\"name\":\"factory/a\",\"alias\":\"custom\"}]"}}),
        ] {
            let observed = CatalogObservation::new(&context, &json!({}), offered.clone());
            for published in [models(&["factory/a"]), models(&["custom"])] {
                assert!(publication_matches(
                    true,
                    &offered,
                    Some(&published),
                    Some(&observed)
                ));
            }
        }
        let observed =
            CatalogObservation::new(&json!({}), &json!({"prefix":"team"}), offered.clone());
        assert!(publication_matches(
            true,
            &offered,
            Some(&models(&["team/factory/a"])),
            Some(&observed)
        ));
        assert!(!publication_matches(
            true,
            &offered,
            Some(&BTreeSet::new()),
            Some(&observed)
        ));
        let hidden = CatalogObservation::new(
            &json!({"Attributes":{"excluded_models":"FACTORY/*"}}),
            &json!({}),
            offered.clone(),
        );
        assert!(publication_matches(
            true,
            &offered,
            Some(&BTreeSet::new()),
            Some(&hidden)
        ));
    }

    #[test]
    fn missing_delivery_or_unexpected_unfiltered_loss_is_not_ready() {
        let offered = models(&["factory/a", "factory/b"]);
        let plain = CatalogObservation::new(&json!({}), &json!({}), offered.clone());
        assert!(!publication_matches(
            true,
            &offered,
            Some(&models(&["factory/a"])),
            Some(&plain)
        ));
        assert!(!publication_matches(true, &offered, Some(&offered), None));
        let managed =
            CatalogObservation::new(&json!({}), &json!({"prefix":"team"}), offered.clone());
        assert!(!publication_matches(true, &offered, None, Some(&managed)));
        assert!(!publication_matches(
            true,
            &models(&["new"]),
            Some(&offered),
            Some(&managed)
        ));
        assert!(!publication_matches(
            false,
            &BTreeSet::new(),
            Some(&offered),
            Some(&managed)
        ));
    }
}
pub fn registration(data: &Value) -> Value {
    let base = data["BasePath"].as_str().unwrap_or("/v0/management");
    let routes:Vec<_>=[
        ("GET","status"),("GET","accounts"),("POST","accounts"),
        ("POST","accounts/refresh"),
        ("POST","accounts/quota-windows"),
        ("POST","models/reconcile"),
    ].into_iter().map(|(method,path)|json!({"Method":method,"Path":format!("{base}/plugins/{PROVIDER}/{path}")})).collect();
    json!({"routes":routes,"resources":[
        {"Path":"/index.html","Menu":"Factory Droid","Description":"添加 Factory 凭据并自动同步模型"},
        {"Path":"/app.js"},{"Path":"/style.css"},
        {"Path":"/manager-extension.js"},{"Path":"/manager-extension.css"},{"Path":"/manager-usage.js"},
        {"Path":"/manager-worker.js"},{"Path":"/manager-bootstrap.js"},{"Path":"/manager-refresh.js"}
    ]})
}
fn response(status: u16, content_type: &str, body: impl AsRef<[u8]>) -> Value {
    json!({"StatusCode":status,"Headers":{
        "Content-Type":[content_type],"Cache-Control":["no-store"],
        "X-Content-Type-Options":["nosniff"],"Referrer-Policy":["no-referrer"],
        "Content-Security-Policy":["default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'"]
    },"Body":encode(body)})
}
pub fn handle(rt: &Arc<Runtime>, req: &Value) -> Value {
    let path = req["Path"].as_str().unwrap_or("");
    if let Some(resource) = path.strip_prefix(RESOURCE) {
        return match resource {
            "/index.html" => response(
                200,
                "text/html; charset=utf-8",
                include_bytes!("../web/index.html"),
            ),
            "/app.js" => response(
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../web/app.js"),
            ),
            "/style.css" => response(
                200,
                "text/css; charset=utf-8",
                include_bytes!("../web/style.css"),
            ),
            "/manager-extension.js" => response(
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../web/manager-extension.js"),
            ),
            "/manager-extension.css" => response(
                200,
                "text/css; charset=utf-8",
                include_bytes!("../web/manager-extension.css"),
            ),
            "/manager-usage.js" => response(
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../web/manager-usage.js"),
            ),
            "/manager-bootstrap.js" => response(
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../web/manager-bootstrap.js"),
            ),
            "/manager-refresh.js" => response(
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../web/manager-refresh.js"),
            ),
            "/manager-worker.js" => {
                let mut result = response(
                    200,
                    "text/javascript; charset=utf-8",
                    include_bytes!("../web/manager-worker.js"),
                );
                result["Headers"]["Service-Worker-Allowed"] = json!(["/management.html"]);
                result
            }
            _ => response(404, "text/plain; charset=utf-8", "资源不存在"),
        };
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(rt, req)))
        .unwrap_or_else(|_| Err(Fault::internal()));
    match result {
        Ok(data) => response(
            200,
            "application/json; charset=utf-8",
            json!({"ok":true,"data":data}).to_string(),
        ),
        Err(error) => {
            let mut value=response(error.http_status,"application/json; charset=utf-8",
                json!({"ok":false,"error":{"code":error.code,"message":error.message,"retry_after_seconds":error.retry_after_seconds}}).to_string());
            if let Some(seconds) = error.retry_after_seconds {
                value["Headers"]["Retry-After"] = json!([seconds.to_string()]);
            }
            value
        }
    }
}
fn body(req: &Value) -> Result<Value> {
    let bytes = decode(&req["Body"])?;
    if bytes.len() > 256 * 1024 {
        return Err(Fault::new(413, "body_too_large", "管理请求超过 256 KiB"));
    }
    if bytes.is_empty() {
        Ok(json!({}))
    } else {
        serde_json::from_slice(&bytes).map_err(|_| Fault::input("请求 JSON 无效"))
    }
}
fn required<'a>(data: &'a Value, key: &str) -> Result<&'a str> {
    data[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Fault::input(&format!("缺少字段 {key}")))
}
pub fn sync_accounts(rt: &Runtime) -> Result<()> {
    let files = factory::host_accounts(rt)?;
    for file in &files {
        let Some(index) = file["auth_index"].as_str().filter(|s| !s.is_empty()) else {
            continue;
        };
        let current = rt
            .host
            .call("host.auth.get", &json!({"auth_index":index}))?;
        let cred = &current["json"];
        let Some(id) = cred["account_id"]
            .as_str()
            .filter(|id| factory::valid_id(id))
        else {
            continue;
        };
        let name = file["name"].as_str().unwrap_or("");
        if !factory::valid_auth_filename(name) {
            continue;
        }
        let existing = rt.lock()?.accounts.get(id).cloned();
        if existing.as_ref().is_some_and(|a| a.deleted) {
            continue;
        }
        let changed = existing.as_ref().is_none_or(|a| {
            a.credential_fingerprint != access::credential_fingerprint(cred)
                || a.enabled != (cred["disabled"].as_bool() != Some(true))
                || a.label != cred["label"].as_str().unwrap_or("Factory")
                || a.priority != cred["priority"].as_i64().unwrap_or(0).clamp(-100, 100)
                || a.auth_file != name
        });
        if changed {
            factory::upsert_account(rt, cred, name, false)?;
            access::restore_catalog(rt, cred)?;
            rt.catalog_dirty.store(true, Ordering::Release);
        }
    }
    let mut store = rt.lock()?;
    let mut changed = false;
    for account in store.accounts.values_mut().filter(|a| !a.deleted) {
        let found = files.iter().find(|f| {
            f["name"].as_str() == Some(&account.auth_file)
                || f["id"].as_str().is_some_and(|v| {
                    crate::state::auth_id_matches(v, &account.id, &account.auth_file)
                })
        });
        if let Some(entry) = found {
            let enabled = entry["disabled"].as_bool() != Some(true);
            if account.enabled != enabled {
                account.enabled = enabled;
                account.status = if !enabled {
                    "disabled"
                } else if account.last_error.is_some() {
                    "error"
                } else if account.last_checked_at.is_some() {
                    "ready"
                } else {
                    "pending"
                }
                .into();
                changed = true;
            }
        } else if account.enabled {
            account.enabled = false;
            account.status = "error".into();
            account.last_error = Some("CPA 凭据已移除或尚未加载".into());
            changed = true;
        }
    }
    if changed {
        store.persist()?;
        rt.catalog_dirty.store(true, Ordering::Release);
    }
    drop(store);
    observe_publication(rt)?;
    Ok(())
}
pub fn observe_publication(rt: &Runtime) -> Result<()> {
    let response = match rt.host.call("host.auth.models", &json!({})) {
        Ok(response) => response,
        Err(error) => {
            rt.lock()?.publication_known = false;
            return Err(error);
        }
    };
    let Some(credentials) = response["credentials"].as_array() else {
        rt.lock()?.publication_known = false;
        return Err(Fault::new(
            503,
            "catalog_refresh_incomplete",
            "CPA未返回有效的凭据模型注册集合",
        ));
    };
    let observed = rt
        .catalog_observed
        .lock()
        .map_err(|_| Fault::internal())?
        .clone();
    let mut store = rt.lock()?;
    let mut published = BTreeMap::new();
    for account in store.accounts.values() {
        let row = credentials.iter().find(|row| {
            row["provider"].as_str() == Some(PROVIDER)
                && row["id"].as_str().is_some_and(|id| {
                    crate::state::auth_id_matches(id, &account.id, &account.auth_file)
                })
        });
        let ids: BTreeSet<_> = row
            .into_iter()
            .flat_map(|row| row["models"].as_array().into_iter().flatten())
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        if row.is_some() {
            published.insert(account.id.clone(), ids);
        }
    }
    store.published_models = published;
    store.publication_known = true;
    let differs = store.accounts.values().any(|account| {
        let desired: BTreeSet<_> = store
            .effective_models(&account.id)
            .iter()
            .map(|m| m.id.clone())
            .collect();
        !publication_matches(
            account.enabled && !account.deleted,
            &desired,
            store.published_models.get(&account.id),
            observed.get(&account.id),
        )
    });
    rt.catalog_dirty.store(differs, Ordering::Release);
    Ok(())
}
fn account_view(rt: &Runtime, id: &str) -> Result<Value> {
    let store = rt.lock()?;
    store
        .accounts
        .get(id)
        .filter(|a| !a.deleted)
        .map(|a| a.view(&store))
        .ok_or_else(|| Fault::new(404, "account_not_found", "Factory 账号不存在"))
}
fn catalog_shape(settings: &Settings) -> Value {
    let models: BTreeMap<_, _> = settings
        .models
        .iter()
        .filter(|m| m.enabled)
        .map(|m| {
            (
                &m.id,
                json!({
        "model":m.host_json(),"upstream_id":m.upstream_id,"upstream_provider":m.upstream_provider}),
            )
        })
        .collect();
    json!(models)
}
struct CatalogPending {
    account: String,
    auth_file: String,
    revision: String,
    models: BTreeSet<String>,
    enabled: bool,
}
fn await_catalog(rt: &Runtime, pending: &[CatalogPending]) -> Result<()> {
    if pending.is_empty() {
        return Ok(());
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let observed = rt
            .catalog_observed
            .lock()
            .map_err(|_| Fault::internal())?
            .clone();
        let discovered = pending.iter().all(|p| {
            !p.enabled
                || observed
                    .get(&p.account)
                    .is_some_and(|entry| entry.revision == p.revision)
        });
        if discovered {
            let published = rt.host.call("host.auth.models", &json!({}))?;
            observe_publication(rt)?;
            let committed = pending.iter().all(|p| {
                published["credentials"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|a| {
                        a["provider"].as_str() == Some(PROVIDER)
                            && a["id"].as_str().is_some_and(|id| {
                                crate::state::auth_id_matches(id, &p.account, &p.auth_file)
                            })
                    })
                    .is_some_and(|a| {
                        let models: BTreeSet<_> = a["models"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect();
                        publication_matches(
                            p.enabled,
                            &p.models,
                            Some(&models),
                            observed.get(&p.account),
                        )
                    })
                    || !p.enabled
                        && p.models.is_empty()
                        && !published["credentials"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .any(|a| {
                                a["provider"].as_str() == Some(PROVIDER)
                                    && a["id"].as_str().is_some_and(|id| {
                                        crate::state::auth_id_matches(id, &p.account, &p.auth_file)
                                    })
                            })
            });
            if committed {
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            let _ = observe_publication(rt);
            return Err(Fault::new(503,"catalog_refresh_incomplete","设置已保存，但 CPA 尚未确认新模型目录；请检查宿主模型排除/别名配置和凭据文件监听状态"));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
pub fn reconcile(rt: &Runtime) -> Result<()> {
    rt.catalog_dirty.store(true, Ordering::Release);
    let result = reconcile_accounts(rt);
    match &result {
        Ok(()) => {
            *rt.catalog_error.lock().map_err(|_| Fault::internal())? = None;
            rt.catalog_dirty.store(false, Ordering::Release);
        }
        Err(error) => {
            *rt.catalog_error.lock().map_err(|_| Fault::internal())? = Some(error.message.clone())
        }
    }
    result
}
fn reconcile_accounts(rt: &Runtime) -> Result<()> {
    let ids: Vec<_> = rt.lock()?.accounts.values().map(|a| a.id.clone()).collect();
    let mut pending = Vec::new();
    for id in ids {
        let snapshot = rt
            .lock()?
            .accounts
            .get(&id)
            .cloned()
            .ok_or_else(Fault::internal)?;
        if snapshot.deleted {
            pending.push(CatalogPending {
                account: id,
                auth_file: snapshot.auth_file,
                revision: String::new(),
                models: BTreeSet::new(),
                enabled: false,
            });
            continue;
        }
        let (mut cred, name) = match factory::load_account(rt, &id) {
            Ok(value) => value,
            Err(error)
                if error.code == "credential_not_loaded"
                    && rt.lock()?.accounts.get(&id).is_some_and(|a| !a.enabled) =>
            {
                pending.push(CatalogPending {
                    account: id,
                    auth_file: snapshot.auth_file,
                    revision: String::new(),
                    models: BTreeSet::new(),
                    enabled: false,
                });
                continue;
            }
            Err(error) => return Err(error),
        };
        let revision = Uuid::new_v4().to_string();
        cred["factory_catalog_revision"] = json!(revision);
        factory::save_account(rt, &cred, &name)?;
        let enabled = cred["disabled"].as_bool() != Some(true)
            && rt
                .lock()?
                .accounts
                .get(&id)
                .is_some_and(|a| a.enabled && !a.deleted);
        let models = if enabled {
            let store = rt.lock()?;
            store
                .effective_models(&id)
                .iter()
                .map(|m| m.id.clone())
                .collect()
        } else {
            BTreeSet::new()
        };
        pending.push(CatalogPending {
            account: id,
            auth_file: name,
            revision,
            models,
            enabled,
        });
    }
    await_catalog(rt, &pending)?;
    observe_publication(rt)?;
    if rt.catalog_dirty.load(Ordering::Acquire) {
        return Err(Fault::new(
            503,
            "catalog_refresh_incomplete",
            "CPA模型集合再次发生变化，请重试模型目录同步",
        ));
    }
    Ok(())
}
fn action(rt: &Arc<Runtime>, req: &Value) -> Result<Value> {
    let method = req["Method"].as_str().unwrap_or("");
    let path = req["Path"]
        .as_str()
        .unwrap_or("")
        .strip_prefix(PREFIX)
        .ok_or_else(|| Fault::new(404, "not_found", "接口不存在"))?;
    let _mutation = if method != "GET"
        && ![
            "/accounts/discovery",
            "/accounts/models/verify",
            "/drafts/discovery",
            "/drafts/models/test",
            "/accounts/quota-windows",
        ]
        .contains(&path)
    {
        Some(rt.management_lock.lock().map_err(|_| Fault::internal())?)
    } else {
        None
    };
    let callback = req["host_callback_id"].as_str().unwrap_or("");
    let data = body(req)?;
    let _active = if method != "GET" {
        Some(rt.begin()?)
    } else {
        None
    };
    match (method, path) {
        ("GET", "/status") => {
            sync_accounts(rt)?;
            let store = rt.lock()?;
            let count = store.accounts.values().filter(|a| !a.deleted).count();
            let enabled = store
                .accounts
                .values()
                .filter(|a| !a.deleted && a.enabled)
                .count();
            let catalog_dirty = rt.catalog_dirty.load(Ordering::Acquire);
            let catalog_error = rt
                .catalog_error
                .lock()
                .map_err(|_| Fault::internal())?
                .clone();
            Ok(
                json!({"version":env!("CARGO_PKG_VERSION"),"provider":PROVIDER,
                "health":if store.health_error.is_some()||catalog_dirty{"degraded"}else{"ready"},
                "message":store.health_error.clone().or_else(||catalog_dirty.then(||catalog_error.unwrap_or_else(||"模型目录尚未交付完成，请检查宿主凭据加载状态".into()))),
                "account_count":count,"enabled_accounts":enabled,"active_requests":rt.active.load(Ordering::Acquire),
                "storage":{"persistent":true},"catalog_synchronized":!catalog_dirty,"capabilities":{"api_key":true,"oauth_import":false,"oauth_device":true,"official_balance":true,"official_analytics":false,
                    "per_account_allowlist":false,"model_validation":false,"cli_discovery_configured":rt.config.droid_cli_path.is_some()},
                "statistics_provider":"cpa","automatic_models":true}),
            )
        }
        ("GET", "/accounts") => {
            sync_accounts(rt)?;
            let store = rt.lock()?;
            Ok(
                json!({"accounts":store.accounts.values().filter(|a|!a.deleted).map(|a|a.view(&store)).collect::<Vec<_>>()}),
            )
        }
        ("POST", "/accounts/quota-windows") => factory::manager_quota(rt, &data, callback),
        ("POST", "/accounts") => {
            let key = required(&data, "api_key")?.trim();
            factory::validate_key(key)?;
            let label = data["label"].as_str().unwrap_or("Factory").trim();
            if label.is_empty() || label.chars().count() > 80 || label.chars().any(char::is_control)
            {
                return Err(Fault::input("账号名称必须为 1 至 80 个可见字符"));
            }
            // Compare secrets only inside this server call, never persist their hashes or return them.
            for file in factory::host_accounts(rt)? {
                if let Some(index) = file["auth_index"].as_str().filter(|s| !s.is_empty()) {
                    let existing = rt
                        .host
                        .call("host.auth.get", &json!({"auth_index":index}))?;
                    if existing["json"]["api_key"].as_str() == Some(key) {
                        return Err(Fault::new(
                            409,
                            "duplicate_account",
                            "此 Factory API Key 已添加",
                        ));
                    }
                }
            }
            let mut cred =
                factory::new_credential(key, label, data["enabled"].as_bool().unwrap_or(true));
            factory::whoami(rt, &mut cred, callback)?;
            access::refresh_catalog(rt, &mut cred)?;
            let id = cred["account_id"].as_str().unwrap_or("").to_string();
            let name = format!("factory-droid-{id}.json");
            factory::save_account(rt, &cred, &name)?;
            factory::upsert_account(rt, &cred, &name, true)?;
            access::restore_catalog(rt, &cred)?;
            reconcile(rt)?;
            account_view(rt, &id)
        }
        ("PATCH", "/accounts") => {
            let id = required(&data, "id")?;
            let (mut cred, name) = factory::load_account(rt, id)?;
            if let Some(label) = data.get("label") {
                let label = label
                    .as_str()
                    .ok_or_else(|| Fault::input("账号名称无效"))?
                    .trim();
                if label.is_empty()
                    || label.chars().count() > 80
                    || label.chars().any(char::is_control)
                {
                    return Err(Fault::input("账号名称必须为 1 至 80 个可见字符"));
                }
                cred["label"] = json!(label);
            }
            if let Some(enabled) = data.get("enabled") {
                cred["disabled"] = json!(!enabled
                    .as_bool()
                    .ok_or_else(|| Fault::input("enabled 必须为布尔值"))?);
            }
            if let Some(priority) = data.get("priority") {
                let p = priority
                    .as_i64()
                    .filter(|v| (-100..=100).contains(v))
                    .ok_or_else(|| Fault::input("优先级必须为 -100 至 100 的整数"))?;
                cred["priority"] = json!(p);
            }
            factory::save_account(rt, &cred, &name)?;
            factory::upsert_account(rt, &cred, &name, false)?;
            if cred["disabled"] == true {
                access::cancel_discovery(rt, id);
            }
            rt.catalog_dirty.store(true, Ordering::Release);
            reconcile(rt)?;
            account_view(rt, id)
        }
        ("DELETE", "/accounts") => {
            let id = required(&data, "id")?;
            let (cred, name) = factory::load_account(rt, id)?;
            let was_dirty = rt.catalog_dirty.load(Ordering::Acquire);
            // Persist a secret-free disabled tombstone through CPA first so routing stops synchronously.
            let tombstone = json!({"type":PROVIDER,"account_id":id,"label":cred["label"],"disabled":true,"deleted":true,"created_at":cred["created_at"]});
            let saved = factory::save_account(rt, &tombstone, &name)?;
            factory::upsert_account(rt, &tombstone, &name, false)?;
            access::cancel_discovery(rt, id);
            rt.catalog_dirty.store(true, Ordering::Release);
            await_catalog(
                rt,
                &[CatalogPending {
                    account: id.into(),
                    auth_file: name.clone(),
                    revision: String::new(),
                    models: BTreeSet::new(),
                    enabled: false,
                }],
            )?;
            let file = saved["path"].as_str().ok_or_else(Fault::internal)?;
            let path = std::path::Path::new(file);
            if !path.is_absolute() || path.file_name().and_then(|s| s.to_str()) != Some(&name) {
                return Err(Fault::new(
                    503,
                    "credential_delete_failed",
                    "宿主返回的凭据路径无效；账号已停用",
                ));
            }
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => {
                    return Err(Fault::new(
                        503,
                        "credential_delete_failed",
                        "账号已停用且密钥已清除，但凭据文件未能删除",
                    ))
                }
            }
            {
                let mut store = rt.lock()?;
                if let Some(a) = store.accounts.get_mut(id) {
                    a.enabled = false;
                    a.deleted = true;
                    a.status = "disabled".into();
                    a.credential_hint.clear();
                    a.updated_at = now();
                }
                store.persist()?;
            }
            rt.catalog_dirty.store(true, Ordering::Release);
            observe_publication(rt)?;
            if !was_dirty {
                rt.catalog_dirty.store(false, Ordering::Release);
            }
            Ok(json!({"id":id,"deleted":true}))
        }
        ("POST", "/accounts/refresh") => {
            let id = required(&data, "id")?;
            let (mut cred, name) = factory::load_account(rt, id)?;
            if let Err(error) = factory::whoami(rt, &mut cred, callback) {
                let mut store = rt.lock()?;
                if let Some(a) = store.accounts.get_mut(id) {
                    a.status = "error".into();
                    a.last_error = Some(error.message.clone());
                    a.last_checked_at = Some(now());
                    if [401, 403].contains(&error.http_status) {
                        a.identity_verified = false;
                        for access in a.model_access.values_mut() {
                            access.allowed = false;
                            access.verification_status = "invalidated".into();
                            access.last_error = Some("credential_invalid".into());
                            access.operation_generation =
                                access.operation_generation.saturating_add(1);
                        }
                    }
                }
                store.persist()?;
                drop(store);
                rt.catalog_dirty.store(true, Ordering::Release);
                let _ = reconcile(rt);
                return Err(error);
            }
            access::refresh_catalog(rt, &mut cred)?;
            factory::save_account(rt, &cred, &name)?;
            factory::upsert_account(rt, &cred, &name, true)?;
            access::restore_catalog(rt, &cred)?;
            reconcile(rt)?;
            account_view(rt, id)
        }
        ("GET", "/settings") => {
            Ok(serde_json::to_value(&rt.lock()?.settings).map_err(|_| Fault::internal())?)
        }
        ("PUT", "/settings") => {
            let settings: Settings =
                serde_json::from_value(data).map_err(|_| Fault::input("设置字段无效"))?;
            settings.validate()?;
            let catalog_changed = {
                let mut store = rt.lock()?;
                for model in &settings.models {
                    if model
                        .id
                        .strip_prefix("factory/")
                        .and_then(|s| s.split_once('/'))
                        .is_some_and(|(scope, _)| store.accounts.contains_key(scope))
                    {
                        return Err(Fault::input(
                            "模型 ID 使用了账号专属保留命名空间，请使用 factory/<模型名称>",
                        ));
                    }
                }
                for account in store.accounts.values().filter(|a| !a.deleted) {
                    let mut ids = BTreeSet::new();
                    for model in &settings.models {
                        if account
                            .model_overrides
                            .get(&model.id)
                            .and_then(|m| m.alias.as_deref())
                            .is_some_and(|alias| {
                                settings.models.iter().any(|m| {
                                    m.id != model.id
                                        && m.id.strip_prefix("factory/").unwrap_or(&m.id) == alias
                                })
                            })
                        {
                            return Err(Fault::new(
                                409,
                                "model_alias_conflict",
                                "模型别名不能占用其他模型的原始ID后缀",
                            ));
                        }
                        if !ids.insert(account.pinned_id(model)) {
                            return Err(Fault::new(
                                409,
                                "model_alias_conflict",
                                "新增模型ID与已有账号的模型别名冲突，请先调整该账号别名",
                            ));
                        }
                    }
                }
                let changed = catalog_shape(&store.settings) != catalog_shape(&settings);
                let previous = std::mem::replace(&mut store.settings, settings.clone());
                let previous_accounts = store.accounts.clone();
                access::invalidate_routes(&mut store);
                if changed {
                    for account in store.accounts.values_mut() {
                        account.discovery.generation =
                            account.discovery.generation.saturating_add(1);
                        for access in account.model_access.values_mut() {
                            access.operation_generation =
                                access.operation_generation.saturating_add(1);
                            if access.verification_status == "validating" {
                                access.verification_status = "not_verified".into();
                            }
                        }
                    }
                }
                if let Err(error) = store.persist() {
                    store.settings = previous;
                    store.accounts = previous_accounts;
                    return Err(error);
                }
                changed
            };
            if catalog_changed || rt.catalog_dirty.load(Ordering::Acquire) {
                reconcile(rt)?;
            }
            Ok(serde_json::to_value(settings).map_err(|_| Fault::internal())?)
        }
        ("GET", "/models") => {
            sync_accounts(rt)?;
            let mut data = access::models_view(&*rt.lock()?, rt.config.droid_cli_path.is_some());
            data["catalog_synchronized"] = json!(!rt.catalog_dirty.load(Ordering::Acquire));
            Ok(data)
        }
        ("GET", "/accounts/models") => {
            sync_accounts(rt)?;
            let id = req["Query"]["account_id"][0]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| Fault::input("缺少account_id"))?;
            let mut value = access::account_models(&*rt.lock()?, id)?;
            value["catalog_synchronized"] = json!(!rt.catalog_dirty.load(Ordering::Acquire));
            Ok(value)
        }
        ("POST", "/accounts/discovery") => {
            access::refresh_discovery(rt, required(&data, "id")?, callback)
        }
        ("POST", "/accounts/models/verify") => access::verify(rt, &data, callback),
        ("DELETE", "/accounts/models/allow") => {
            access::revoke(rt, required(&data, "id")?, required(&data, "model_id")?)
        }
        ("POST", "/models/reconcile") => {
            reconcile(rt)?;
            Ok(json!({"synchronized":true}))
        }
        ("POST", "/drafts") => access::create_draft(rt, &data, callback),
        ("GET", "/drafts") => access::get_draft(
            rt,
            req["Query"]["draft_id"][0]
                .as_str()
                .ok_or_else(|| Fault::input("缺少draft_id"))?,
        ),
        ("DELETE", "/drafts") => access::delete_draft(rt, required(&data, "id")?),
        ("POST", "/drafts/discovery") => access::discover_draft(rt, required(&data, "id")?),
        ("PUT", "/drafts/models") => access::select_draft_models(rt, &data),
        ("POST", "/drafts/models/test") => access::test_draft(rt, &data, callback),
        ("POST", "/drafts/commit") => access::commit_draft(rt, &data),
        ("GET", "/usage") => {
            let account = req["Query"]["account_id"][0]
                .as_str()
                .filter(|s| !s.is_empty());
            let kind = req["Query"]["kind"][0].as_str();
            if kind.is_some_and(|k| !["all", "inference", "validation"].contains(&k)) {
                return Err(Fault::input("kind必须为all、inference或validation"));
            }
            Ok(rt.lock()?.usage_kind(account, kind))
        }
        _ => Err(Fault::new(404, "not_found", "接口不存在")),
    }
}
