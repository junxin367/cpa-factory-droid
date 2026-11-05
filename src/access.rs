use chrono::{Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
use uuid::Uuid;

use crate::{
    discovery, factory,
    host::{decode, encode},
    management,
    protocol::{self, Format},
    state::{now, Account, Model, ModelOverride, Store, UsageTokens},
    Fault, Result, Runtime,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscoveryState {
    pub status: String,
    pub source: String,
    pub snapshot_id: Option<String>,
    pub fetched_at: Option<String>,
    pub fresh_until: Option<String>,
    pub stale: bool,
    pub cli_version: Option<String>,
    pub protocol_version: Option<String>,
    pub message: String,
    pub candidates: Vec<Value>,
    pub generation: u64,
    pub credential_fingerprint: String,
}
impl Default for DiscoveryState {
    fn default() -> Self {
        Self {
            status: "not_fetched".into(),
            source: "official_droid_cli".into(),
            snapshot_id: None,
            fetched_at: None,
            fresh_until: None,
            stale: true,
            cli_version: None,
            protocol_version: None,
            message: "尚未刷新官方客户端候选目录".into(),
            candidates: Vec::new(),
            generation: 0,
            credential_fingerprint: String::new(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelAccess {
    pub allowed: bool,
    pub verification_status: String,
    pub verified_at: Option<String>,
    pub verification_source: Option<String>,
    pub last_error: Option<String>,
    pub credential_fingerprint: String,
    pub credential_generation: u64,
    pub route_fingerprint: String,
    pub discovery_snapshot_id: Option<String>,
    pub validation_request_id: Option<String>,
    pub response_model: Option<String>,
    pub operation_generation: u64,
    pub selection_source: Option<String>,
    pub last_probe_error: Option<String>,
    pub last_probe_status: Option<String>,
}
impl Default for ModelAccess {
    fn default() -> Self {
        Self {
            allowed: false,
            verification_status: "not_verified".into(),
            verified_at: None,
            verification_source: None,
            last_error: None,
            credential_fingerprint: String::new(),
            credential_generation: 0,
            route_fingerprint: String::new(),
            discovery_snapshot_id: None,
            validation_request_id: None,
            response_model: None,
            operation_generation: 0,
            selection_source: None,
            last_probe_error: None,
            last_probe_status: None,
        }
    }
}
pub fn fingerprint(value: &Value) -> String {
    encode(Sha256::digest(value.to_string().as_bytes()))
}
pub fn credential_fingerprint(credential: &Value) -> String {
    fingerprint(
        &json!({"type":credential["type"],"id":credential["account_id"],"key":credential["api_key"],"user":credential["user_id"],
        "auth_method":credential["auth_method"],"oauth_user":credential["oauth_user_id"],
        "org":credential["org_id"],"region":credential["region"],"host":credential["prem_base_host"],
        "principal":credential["principal_kind"],"inference_region":credential["inference_region"]}),
    )
}
pub fn route_fingerprint(model: &Model) -> String {
    let mut route = json!({"id":model.id,"upstream":model.upstream_id,"protocol":model.protocol,"provider":model.upstream_provider,
        "images":model.supports_images,"context":model.context_length,"max_output":model.max_output_tokens});
    if let Some((field, value)) = crate::catalog::required_parameter(model) {
        route["required_parameter"] = json!({"field":field,"value":value});
    }
    fingerprint(&route)
}
fn candidate_id(candidate: &Value) -> Option<&str> {
    candidate["id"].as_str().filter(|s| !s.trim().is_empty())
}
fn candidate_blocks(candidate: &Value) -> Vec<String> {
    let mut reasons = Vec::new();
    for (key, reason) in [
        ("disabled", "candidate_disabled"),
        ("deprecated", "candidate_deprecated"),
        ("isCustom", "candidate_custom"),
        ("isRouter", "candidate_router"),
    ] {
        if candidate[key].as_bool() == Some(true) {
            reasons.push(reason.into());
        } else if candidate
            .get(key)
            .is_some_and(|v| !v.is_null() && !v.is_boolean())
        {
            reasons.push("invalid_candidate_metadata".into());
        }
    }
    if candidate["kind"].as_str().is_some_and(|kind| {
        ["router", "auto", "automatic", "local"].contains(&kind.to_ascii_lowercase().as_str())
    }) {
        reasons.push("candidate_router".into());
    }
    if candidate["modelProvider"].as_str() == Some("google") {
        reasons.push("unsupported_native_protocol".into());
    }
    reasons
}
pub fn known_blocks(account: &Account, model: &Model) -> Vec<String> {
    account
        .discovery
        .candidates
        .iter()
        .filter(|c| candidate_id(c) == Some(&model.upstream_id))
        .flat_map(|candidate| {
            let mut reasons = candidate_blocks(candidate);
            if crate::catalog::route(candidate)
                .is_some_and(|(protocol, _, _)| model.protocol != protocol)
            {
                reasons.push("candidate_protocol_mismatch".into());
            }
            reasons
        })
        .collect()
}

pub fn register_discovered_models(store: &mut Store, candidates: &[Value]) -> Result<()> {
    let previous = store.settings.models.clone();
    let mut changed = false;
    for candidate in candidates {
        if !candidate_blocks(candidate).is_empty() {
            continue;
        }
        let Some(model) = crate::catalog::model(candidate) else {
            continue;
        };
        // Native CPA credentials share one public ID per upstream model.
        if let Some(existing) = store
            .settings
            .models
            .iter_mut()
            .find(|existing| existing.id == model.id)
        {
            changed |= crate::catalog::apply_capabilities(existing, Some(candidate));
            continue;
        }
        if store.settings.models.len() < 1024 {
            store.settings.models.push(model);
            changed = true;
        }
    }
    if changed {
        if let Err(error) = store.settings.validate().and_then(|_| store.persist()) {
            store.settings.models = previous;
            return Err(error);
        }
    }
    Ok(())
}
pub fn blocked_reasons(account: &Account, model: &Model) -> Vec<String> {
    let mut reasons = known_blocks(account, model);
    if account.deleted {
        reasons.push("account_deleted".into());
    }
    if !account.enabled {
        reasons.push("account_disabled".into());
    }
    if !model.enabled {
        reasons.push("global_disabled".into());
    }
    if model.id != format!("factory/{}", model.upstream_id) {
        reasons.push("non_canonical_model".into());
    }
    if !account.discovery.candidates.is_empty()
        && !account.discovery.candidates.iter().any(|candidate| {
            candidate_id(candidate) == Some(&model.upstream_id)
                && candidate_blocks(candidate).is_empty()
                && crate::catalog::route(candidate).is_some()
        })
    {
        reasons.push("candidate_missing".into());
    }
    reasons.sort();
    reasons.dedup();
    reasons
}
impl Store {
    pub fn effective_models(&self, account_id: &str) -> Vec<Model> {
        let Some(account) = self.accounts.get(account_id) else {
            return Vec::new();
        };
        self.settings
            .models
            .iter()
            .filter(|model| blocked_reasons(account, model).is_empty())
            .cloned()
            .collect()
    }
    pub fn account_for_auth(&self, auth_id: &str) -> Option<&Account> {
        self.accounts
            .values()
            .find(|a| crate::state::auth_id_matches(auth_id, &a.id, &a.auth_file))
    }
    pub fn ensure_model_allowed(&self, account_id: &str, model: &Model) -> Result<()> {
        let account = self
            .accounts
            .get(account_id)
            .ok_or_else(|| Fault::new(403, "model_not_allowed", "Factory 凭据尚未加载"))?;
        if !self.effective_models(account_id).iter().any(|m| {
            if m.id != model.id && model.id != account.pinned_id(m) {
                return false;
            }
            let mut canonical = model.clone();
            canonical.id = m.id.clone();
            route_fingerprint(m) == route_fingerprint(&canonical)
        }) {
            return Err(Fault::new(
                403,
                "model_not_allowed",
                "当前凭据的 Factory 模型目录不包含此路由",
            ));
        }
        Ok(())
    }
    pub fn account_model(&self, account_id: &str, id: &str) -> Option<Model> {
        let account = self.accounts.get(account_id)?;
        // Public CPA identifiers take precedence over raw upstream identifiers.
        // A configured upstream string can otherwise shadow another model's public ID.
        let rows = &self.settings.models;
        let exact = rows
            .iter()
            .find(|m| m.id == id)
            .or_else(|| rows.iter().find(|m| account.pinned_id(m) == id));
        let model = if let Some(exact) = exact {
            exact.clone()
        } else {
            let mut upstream = rows.iter().filter(|m| m.upstream_id == id);
            let model = upstream.next()?;
            if upstream.next().is_some() {
                return None;
            }
            model.clone()
        };
        Some(model)
    }
}
fn candidate_view(candidate: &Value, store: &Store) -> Value {
    let id = candidate_id(candidate).unwrap_or("");
    let route_models: Vec<_> = store
        .settings
        .models
        .iter()
        .filter(|m| m.upstream_id == id)
        .map(|m| m.id.clone())
        .collect();
    let mut reasons = candidate_blocks(candidate);
    if route_models.is_empty() {
        reasons.push("unknown_route".into());
    }
    let supports_images = candidate["noImageSupport"].as_bool().map(|no| !no);
    json!({"id":id,"display_name":candidate["displayName"].as_str().unwrap_or(id),
        "model_provider":candidate["modelProvider"],"disabled":candidate["disabled"].as_bool(),
        "deprecated":candidate["deprecated"].as_bool(),"is_custom":candidate["isCustom"].as_bool(),"kind":candidate["kind"],
        "supports_images":supports_images,"route_models":route_models,"blocked_reasons":reasons})
}
pub fn discovery_view(account: &Account, store: &Store) -> Value {
    let discovery = &account.discovery;
    let stale = discovery.stale
        || discovery
            .fresh_until
            .as_deref()
            .is_none_or(|end| end <= now().as_str())
        || discovery.credential_fingerprint != account.credential_fingerprint;
    json!({"status":discovery.status,"source":discovery.source,"snapshot_id":discovery.snapshot_id,
        "fetched_at":discovery.fetched_at,"fresh_until":discovery.fresh_until,"stale":stale,
        "cli_version":discovery.cli_version,"protocol_version":discovery.protocol_version,
        "account_policy_authority":"unknown","candidate_count":discovery.candidates.len(),"message":discovery.message,
        "candidates":discovery.candidates.iter().map(|c|candidate_view(c,store)).collect::<Vec<_>>()})
}
pub fn model_view(account: &Account, model: &Model, store: &Store) -> Value {
    let access = account
        .model_access
        .get(&model.id)
        .cloned()
        .unwrap_or_default();
    let blocked = blocked_reasons(account, model);
    let invalid = blocked.iter().any(|r| {
        [
            "credential_changed",
            "route_changed",
            "candidate_disabled",
            "candidate_deprecated",
            "candidate_custom",
            "candidate_router",
            "candidate_protocol_mismatch",
            "unsupported_native_protocol",
        ]
        .contains(&r.as_str())
    });
    let status = if invalid && access.verification_status == "basic_call_verified" {
        "invalidated"
    } else {
        &access.verification_status
    };
    let published = store
        .published_models
        .get(&account.id)
        .is_some_and(|ids| ids.contains(&model.id) || ids.contains(&account.pinned_id(model)));
    let desired = blocked.is_empty();
    let publication_status = if !store.publication_known {
        "unknown"
    } else {
        match (desired, published) {
            (true, true) => "published",
            (true, false) => "pending_add",
            (false, true) => "pending_remove",
            (false, false) => "unpublished",
        }
    };
    json!({"model_id":model.id,"upstream_id":model.upstream_id,"display_name":model.display_name,"protocol":model.protocol,
        "endpoint":Format::for_model(model).map(|f|f.endpoint()).unwrap_or(""),"global_enabled":model.enabled,
        "allowed":access.allowed,"published":published,"publication_status":publication_status,"effective":desired,
        "verification_status":status,"verified_at":access.verified_at,
        "verification_source":access.verification_source,"last_error":access.last_error,"response_model":access.response_model,
        "selection_source":access.selection_source,"last_probe_error":access.last_probe_error,"last_probe_status":access.last_probe_status,
        "selected":account.selected_models.contains(&model.id),"alias":account.model_overrides.get(&model.id).and_then(|m|m.alias.as_deref()),
        "model_override":account.model_overrides.get(&model.id),
        "blocked_reasons":blocked,"pinned_id":account.pinned_id(model)})
}
pub fn account_models(store: &Store, account_id: &str) -> Result<Value> {
    let account = store
        .accounts
        .get(account_id)
        .filter(|a| !a.deleted)
        .ok_or_else(|| Fault::new(404, "account_not_found", "Factory 账号不存在"))?;
    Ok(
        json!({"account_id":account_id,"identity_verified":account.identity_verified,"credential_generation":account.credential_generation,
        "discovery":discovery_view(account,store),"models":store.settings.models.iter().map(|m|model_view(account,m,store)).collect::<Vec<_>>(),
        "effective_models":store.effective_models(account_id).iter().map(|m|m.id.clone()).collect::<Vec<_>>(),
        "publication_known":store.publication_known,
        "validation_usage":store.usage_kind(Some(account_id),Some("validation"))}),
    )
}
pub fn models_view(store: &Store, cli_configured: bool) -> Value {
    let models: Vec<_> = store
        .settings
        .models
        .iter()
        .map(|m| {
            let accounts: Vec<_> = store
                .accounts
                .values()
                .filter(|a| !a.deleted && blocked_reasons(a, m).is_empty())
                .map(|a| a.id.clone())
                .collect();
            let published_accounts: Vec<_> = store
                .published_models
                .iter()
                .filter(|(_, ids)| ids.contains(&m.id))
                .map(|(id, _)| id.clone())
                .collect();
            let mut value = serde_json::to_value(m).unwrap_or(json!({}));
            value["registered"] = json!(!published_accounts.is_empty());
            value["allowed_accounts"] = json!(accounts);
            value["published_accounts"] = json!(published_accounts);
            value["publication_known"] = json!(store.publication_known);
            value["verification_status"] = json!(if value["allowed_accounts"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
            {
                "basic_call_verified"
            } else {
                "not_verified"
            });
            value["endpoint"] = json!(Format::for_model(m).map(|f| f.endpoint()).unwrap_or(""));
            value
        })
        .collect();
    json!({"models":models,"source":"显式原生路由定义；账号实际发布需基础验证与允许",
        "warning":"CLI候选目录不是账号权限证明；基础验证仅证明该时点固定请求成功",
        "cli_discovery_configured":cli_configured})
}

fn safe_candidates(rows: Vec<Value>, key: &str) -> Result<Vec<Value>> {
    if rows.is_empty() || rows.len() > 2000 {
        return Err(Fault::new(
            502,
            "invalid_catalog",
            "CLI候选目录为空或超过2000条，未作为成功目录保存",
        ));
    }
    let mut ids = BTreeSet::new();
    let mut out = Vec::new();
    for mut row in rows {
        let id = candidate_id(&row)
            .filter(|id| id.len() <= 256)
            .ok_or_else(|| Fault::new(502, "invalid_catalog", "CLI候选模型缺少有效ID"))?
            .to_string();
        if !row.is_object() || !ids.insert(id) {
            return Err(Fault::new(
                502,
                "invalid_catalog",
                "CLI候选目录包含重复或无效ID",
            ));
        }
        redact(&mut row, key);
        out.push(row);
    }
    Ok(out)
}
fn redact(value: &mut Value, key: &str) {
    match value {
        Value::Object(map) => {
            for (name, value) in map {
                let normalized: String = name
                    .chars()
                    .filter(|c| !matches!(c, '_' | '-'))
                    .flat_map(char::to_lowercase)
                    .collect();
                if [
                    "apikey",
                    "authorization",
                    "accesstoken",
                    "refreshtoken",
                    "password",
                    "secret",
                    "token",
                    "credentials",
                    "cookie",
                    "setcookie",
                    "clientsecret",
                ]
                .contains(&normalized.as_str())
                {
                    *value = json!("[redacted]");
                } else {
                    redact(value, key);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                redact(value, key);
            }
        }
        Value::String(text) => {
            if !key.is_empty() {
                *text = text.replace(key, "[redacted]");
            }
            if text.contains("fk-") || text.contains("sk-") {
                *text = "[redacted]".into();
            }
        }
        _ => {}
    }
}
fn stale() -> Fault {
    Fault::new(
        409,
        "stale_operation",
        "账号、凭据、允许状态或模型路由已改变，已忽略过期结果",
    )
}
fn required_account<'a>(store: &'a Store, id: &str) -> Result<&'a Account> {
    store
        .accounts
        .get(id)
        .filter(|a| !a.deleted)
        .ok_or_else(stale)
}
struct DiscoveryRun {
    runtime: Arc<Runtime>,
    id: String,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}
impl DiscoveryRun {
    fn begin(runtime: &Arc<Runtime>, id: &str) -> Result<Self> {
        let mut runs = runtime
            .discovery_cancels
            .lock()
            .map_err(|_| Fault::internal())?;
        if runtime.closing.load(Ordering::Acquire) {
            return Err(Fault::new(503, "plugin_stopping", "插件正在停止"));
        }
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        if let Some(previous) = runs.insert(id.into(), cancel.clone()) {
            previous.store(true, Ordering::Release);
        }
        Ok(Self {
            runtime: runtime.clone(),
            id: id.into(),
            cancel,
        })
    }
}
impl Drop for DiscoveryRun {
    fn drop(&mut self) {
        if let Ok(mut runs) = self.runtime.discovery_cancels.lock() {
            if runs
                .get(&self.id)
                .is_some_and(|token| Arc::ptr_eq(token, &self.cancel))
            {
                runs.remove(&self.id);
            }
        }
    }
}
pub fn cancel_discovery(rt: &Runtime, id: &str) {
    if let Ok(mut runs) = rt.discovery_cancels.lock() {
        if let Some(token) = runs.remove(id) {
            token.store(true, Ordering::Release);
        }
    }
}

/// Fetch metadata only. Calling a model remains CPA's normal request path.
pub fn refresh_catalog(rt: &Arc<Runtime>, cred: &mut Value) -> Result<()> {
    let Some(executable) = rt.config.droid_cli_path.clone() else {
        cred["factory_catalog_source"] = json!("bundled_registry");
        cred["factory_catalog_checked_at"] = json!(now());
        return Ok(());
    };
    let id = cred["account_id"].as_str().ok_or_else(Fault::internal)?;
    let run = DiscoveryRun::begin(rt, id)?;
    let work_root = rt
        .lock()?
        .path
        .parent()
        .ok_or_else(Fault::internal)?
        .join("discovery-runs");
    let snapshot = discovery::discover(
        discovery::Options {
            executable,
            work_root,
            timeout: Duration::from_secs(rt.config.discovery_timeout_seconds.unwrap_or(30)),
            cancel: Some(run.cancel.clone()),
        },
        cred["api_key"].as_str().unwrap_or(""),
    )
    .map_err(|error| Fault::new(502, &error.code, &error.message))?;
    let mut candidates = safe_candidates(snapshot.models, factory::bearer(cred))?;
    for candidate in &mut candidates {
        if let Some(fields) = candidate.as_object_mut() {
            fields.retain(|name, _| {
                [
                    "id",
                    "displayName",
                    "modelProvider",
                    "disabled",
                    "deprecated",
                    "isCustom",
                    "isRouter",
                    "kind",
                    "noImageSupport",
                    "maxInputTokens",
                    "maxOutputTokens",
                    "contextLimits",
                    "supportedReasoningEfforts",
                    "tokenMultiplier",
                ]
                .contains(&name.as_str())
            });
        }
    }
    if !candidates.iter().any(|candidate| {
        candidate_blocks(candidate).is_empty() && crate::catalog::model(candidate).is_some()
    }) {
        return Err(Fault::new(
            502,
            "empty_catalog",
            "官方目录中没有支持的原生模型",
        ));
    }
    cred["factory_catalog"] = json!({
        "version":1, "candidates":candidates, "fetched_at":now(),
        "cli_version":snapshot.cli_version, "protocol_version":snapshot.protocol_version
    });
    cred["factory_catalog_source"] = json!("official_droid_cli");
    cred["factory_catalog_checked_at"] = json!(now());
    Ok(())
}

/// Keep the catalog in CPA's credential so export/import and restart need no UI.
pub fn restore_catalog(rt: &Runtime, cred: &Value) -> Result<()> {
    let catalog = &cred["factory_catalog"];
    if catalog["version"].as_u64() != Some(1) {
        return Ok(());
    }
    let rows = catalog["candidates"]
        .as_array()
        .filter(|rows| !rows.is_empty() && rows.len() <= 1024)
        .ok_or_else(|| Fault::input("Factory 凭据中的模型目录无效"))?;
    let rows = safe_candidates(rows.clone(), factory::bearer(cred))?;
    let mut store = rt.lock()?;
    register_discovered_models(&mut store, &rows)?;
    let account = store
        .accounts
        .get_mut(cred["account_id"].as_str().unwrap_or(""))
        .ok_or_else(Fault::internal)?;
    let fetched_at = catalog["fetched_at"].as_str().map(str::to_owned);
    if account.discovery.fetched_at == fetched_at
        && account.discovery.candidates == rows
        && account.discovery.credential_fingerprint == credential_fingerprint(cred)
    {
        return Ok(());
    }
    account.discovery = DiscoveryState {
        status: "catalog".into(),
        source: "official_droid_cli".into(),
        fetched_at,
        stale: false,
        cli_version: catalog["cli_version"].as_str().map(str::to_owned),
        protocol_version: catalog["protocol_version"].as_str().map(str::to_owned),
        candidates: rows,
        credential_fingerprint: credential_fingerprint(cred),
        message: String::new(),
        ..DiscoveryState::default()
    };
    store.persist()
}

pub fn invalidate_routes(store: &mut Store) {
    let routes: std::collections::BTreeMap<_, _> = store
        .settings
        .models
        .iter()
        .map(|m| (m.id.clone(), route_fingerprint(m)))
        .collect();
    for account in store.accounts.values_mut() {
        for (id, access) in &mut account.model_access {
            if routes.get(id) != Some(&access.route_fingerprint) {
                access.verification_status = "invalidated".into();
                access.allowed = false;
                access.last_error = Some("route_changed".into());
                access.operation_generation = access.operation_generation.saturating_add(1);
            }
        }
    }
}
fn invalidate_identity(account: &mut Account, message: &str) {
    account.identity_verified = false;
    account.last_error = Some(message.into());
    account.status = "error".into();
    account.discovery.stale = true;
    account.discovery.status = "auth_failed".into();
    account.discovery.generation = account.discovery.generation.saturating_add(1);
    for access in account.model_access.values_mut() {
        access.allowed = false;
        access.verification_status = "invalidated".into();
        access.last_error = Some(message.into());
        access.operation_generation = access.operation_generation.saturating_add(1);
    }
}
pub fn identity_error(rt: &Arc<Runtime>, id: &str, message: &str) {
    if let Ok(mut store) = rt.lock() {
        if let Some(account) = store.accounts.get_mut(id) {
            invalidate_identity(account, message);
            let _ = store.persist();
        }
    }
    rt.catalog_dirty.store(true, Ordering::Release);
    if let Ok(_mutation) = rt.management_lock.lock() {
        let _ = management::reconcile(rt);
    }
}
pub fn invalidate_upstream(rt: &Arc<Runtime>, cred: &Value, model: &Model, status: u16) {
    if ![401, 403].contains(&status) {
        return;
    }
    if let Ok(mut store) = rt.lock() {
        let id = cred["account_id"].as_str().unwrap_or("");
        let base = store
            .account_model(id, &model.id)
            .map(|m| m.id)
            .unwrap_or_else(|| model.id.clone());
        if let Some(account) = store
            .accounts
            .get_mut(id)
            .filter(|a| a.credential_fingerprint == credential_fingerprint(cred))
        {
            if status == 401 {
                invalidate_identity(account, "credential_invalid");
            } else if let Some(access) = account.model_access.get_mut(&base) {
                access.allowed = false;
                access.verification_status = "invalidated".into();
                access.last_error = Some("permission_denied".into());
                access.operation_generation = access.operation_generation.saturating_add(1);
            }
            let _ = store.persist();
            rt.catalog_dirty.store(true, Ordering::Release);
        }
    }
    if let Ok(_mutation) = rt.management_lock.lock() {
        let _ = management::reconcile(rt);
    }
}
pub fn revoke(rt: &Arc<Runtime>, id: &str, model_id: &str) -> Result<Value> {
    {
        let mut store = rt.lock()?;
        if !store.settings.models.iter().any(|m| m.id == model_id) {
            return Err(Fault::new(404, "model_not_found", "模型路由不存在"));
        }
        let account = store
            .accounts
            .get_mut(id)
            .filter(|a| !a.deleted)
            .ok_or_else(|| Fault::new(404, "account_not_found", "Factory账号不存在"))?;
        let access = account.model_access.entry(model_id.into()).or_default();
        let previous = access.clone();
        access.allowed = false;
        access.operation_generation = access.operation_generation.saturating_add(1);
        if access.verification_status == "validating" {
            access.verification_status = "not_verified".into();
        }
        account.selected_models.remove(model_id);
        if let Err(error) = store.persist() {
            store
                .accounts
                .get_mut(id)
                .unwrap()
                .model_access
                .insert(model_id.into(), previous);
            return Err(error);
        }
    }
    rt.catalog_dirty.store(true, Ordering::Release);
    management::reconcile(rt)?;
    account_models(&*rt.lock()?, id)
}

pub fn refresh_discovery(rt: &Arc<Runtime>, id: &str, callback: &str) -> Result<Value> {
    let _active = rt.begin()?;
    let Some(executable) = rt.config.droid_cli_path.clone() else {
        let mut store = rt.lock()?;
        let account = store
            .accounts
            .get_mut(id)
            .filter(|a| !a.deleted)
            .ok_or_else(|| Fault::new(404, "account_not_found", "Factory账号不存在"))?;
        account.discovery.status = "not_configured".into();
        account.discovery.stale = true;
        account.discovery.message =
            "管理员尚未配置官方Droid CLI，可对明确路由使用手工基础验证".into();
        store.persist()?;
        return Err(Fault::new(
            400,
            "discovery_not_configured",
            "管理员尚未配置droid-cli-path",
        ));
    };
    let (mut cred, name) = factory::load_account(rt, id)?;
    factory::ensure_enabled(rt, &cred)?;
    let (generation, fingerprint, credential_generation) = {
        let _mutation = rt.management_lock.lock().map_err(|_| Fault::internal())?;
        let mut store = rt.lock()?;
        let account = store.accounts.get_mut(id).ok_or_else(stale)?;
        account.discovery.generation = account.discovery.generation.saturating_add(1);
        account.discovery.status = "loading".into();
        account.discovery.message = "正在读取官方CLI候选目录".into();
        account.discovery.stale = true;
        let result = (
            account.discovery.generation,
            account.credential_fingerprint.clone(),
            account.credential_generation,
        );
        store.persist()?;
        result
    };
    if let Err(error) = factory::whoami(rt, &mut cred, callback) {
        let _mutation = rt.management_lock.lock().map_err(|_| Fault::internal())?;
        let mut store = rt.lock()?;
        let account = store
            .accounts
            .get_mut(id)
            .filter(|a| {
                !a.deleted
                    && a.discovery.generation == generation
                    && a.credential_generation == credential_generation
            })
            .ok_or_else(stale)?;
        account.discovery.status = if [401, 403].contains(&error.http_status) {
            "auth_failed"
        } else {
            "auth_unknown"
        }
        .into();
        account.discovery.message = error.message.clone();
        account.discovery.stale = true;
        if [401, 403].contains(&error.http_status) {
            invalidate_identity(account, &error.message);
        }
        store.persist()?;
        drop(store);
        rt.catalog_dirty.store(true, Ordering::Release);
        let _ = management::reconcile(rt);
        return Err(error);
    }
    let (generation, credential_generation, fingerprint, run) = {
        let _mutation = rt.management_lock.lock().map_err(|_| Fault::internal())?;
        {
            let store = rt.lock()?;
            let account = required_account(&store, id)?;
            if account.discovery.generation != generation
                || account.credential_generation != credential_generation
                || account.credential_fingerprint != fingerprint
            {
                return Err(stale());
            }
        }
        factory::save_account(rt, &cred, &name)?;
        factory::upsert_account(rt, &cred, &name, true)?;
        let mut store = rt.lock()?;
        let account = store.accounts.get_mut(id).ok_or_else(stale)?;
        account.discovery.status = "loading".into();
        account.discovery.stale = true;
        let captured = (
            account.discovery.generation,
            account.credential_generation,
            account.credential_fingerprint.clone(),
        );
        store.persist()?;
        drop(store);
        (
            captured.0,
            captured.1,
            captured.2,
            DiscoveryRun::begin(rt, id)?,
        )
    };
    let work_root = {
        let store = rt.lock()?;
        let parent = store.path.parent().ok_or_else(Fault::internal)?;
        std::fs::canonicalize(parent)
            .map_err(|_| Fault::storage("无法定位插件数据目录"))?
            .join("discovery-runs")
    };
    let result = discovery::discover(
        discovery::Options {
            executable,
            work_root,
            timeout: Duration::from_secs(rt.config.discovery_timeout_seconds.unwrap_or(30)),
            cancel: Some(run.cancel.clone()),
        },
        cred["api_key"].as_str().unwrap_or(""),
    );
    let _mutation = rt.management_lock.lock().map_err(|_| Fault::internal())?;
    let mut store = rt.lock()?;
    store
        .accounts
        .get(id)
        .filter(|a| {
            !a.deleted
                && a.enabled
                && a.discovery.generation == generation
                && a.credential_generation == credential_generation
                && a.credential_fingerprint == fingerprint
        })
        .ok_or_else(stale)?;
    match result {
        Ok(snapshot) => {
            let rows = safe_candidates(snapshot.models, cred["api_key"].as_str().unwrap_or(""))?;
            register_discovered_models(&mut store, &rows)?;
            let account = store.accounts.get_mut(id).ok_or_else(stale)?;
            let time = Utc::now();
            account.discovery = DiscoveryState {
                status: "candidate_catalog".into(),
                source: "official_droid_cli".into(),
                snapshot_id: Some(Uuid::new_v4().to_string()),
                fetched_at: Some(time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
                fresh_until: Some(
                    (time + ChronoDuration::minutes(5))
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                ),
                stale: false,
                cli_version: snapshot.cli_version,
                protocol_version: snapshot.protocol_version,
                message: "官方CLI候选目录不是当前账号模型权限证明；调用前仍需基础验证".into(),
                candidates: rows,
                generation,
                credential_fingerprint: fingerprint,
            };
            let models = store.settings.models.clone();
            let account = store.accounts.get_mut(id).unwrap();
            for model in models {
                let blocked = !known_blocks(account, &model).is_empty()
                    || account.model_access.get(&model.id).is_some_and(|a| {
                        a.selection_source.as_deref() == Some("official_droid_cli")
                    }) && !account
                        .discovery
                        .candidates
                        .iter()
                        .any(|c| candidate_id(c) == Some(&model.upstream_id));
                if blocked {
                    if let Some(access) = account.model_access.get_mut(&model.id) {
                        access.allowed = false;
                        access.verification_status = "invalidated".into();
                        access.last_error = Some("candidate_blocked".into());
                        access.operation_generation = access.operation_generation.saturating_add(1);
                    }
                }
            }
            store.persist()?;
            drop(store);
            rt.catalog_dirty.store(true, Ordering::Release);
            management::reconcile(rt)?;
            account_models(&*rt.lock()?, id)
        }
        Err(error) => {
            let account = store.accounts.get_mut(id).ok_or_else(stale)?;
            account.discovery.status = "error".into();
            account.discovery.stale = true;
            account.discovery.message = error.message.clone();
            store.persist()?;
            Err(Fault::new(
                if error.code == "discovery_timeout" {
                    504
                } else {
                    502
                },
                &error.code,
                &error.message,
            ))
        }
    }
}

#[derive(Clone)]
struct Probe {
    credential_generation: u64,
    credential_fingerprint: String,
    route_fingerprint: String,
    operation_generation: u64,
    discovery_snapshot_id: Option<String>,
}
fn check_probe(store: &Store, id: &str, model: &Model, probe: &Probe) -> Result<()> {
    let account = required_account(store, id)?;
    let current = store
        .settings
        .models
        .iter()
        .find(|m| m.id == model.id && m.enabled)
        .ok_or_else(stale)?;
    if !account.enabled
        || account.credential_generation != probe.credential_generation
        || account.credential_fingerprint != probe.credential_fingerprint
        || route_fingerprint(current) != probe.route_fingerprint
        || account
            .model_access
            .get(&model.id)
            .is_none_or(|a| a.operation_generation != probe.operation_generation)
        || !known_blocks(account, current).is_empty()
        || probe
            .discovery_snapshot_id
            .as_ref()
            .is_some_and(|snapshot| account.discovery.snapshot_id.as_ref() != Some(snapshot))
    {
        return Err(stale());
    }
    Ok(())
}
pub fn verify(rt: &Arc<Runtime>, data: &Value, callback: &str) -> Result<Value> {
    if data["confirm"] != true {
        return Err(Fault::new(
            400,
            "confirmation_required",
            "验证将发送一次固定短请求并可能消耗额度，请明确confirm:true",
        ));
    }
    if data.as_object().is_none_or(|o| {
        o.keys()
            .any(|k| !["id", "model_id", "confirm", "discovery_snapshot_id"].contains(&k.as_str()))
    }) {
        return Err(Fault::input(
            "验证接口只接受账号、模型、快照和confirm，不接受自定义请求",
        ));
    }
    let id = data["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Fault::input("缺少账号id"))?;
    let model_id = data["model_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Fault::input("缺少model_id"))?;
    let snapshot_id = match data.get("discovery_snapshot_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) if !id.is_empty() => Some(id.clone()),
        _ => return Err(Fault::input("discovery_snapshot_id无效")),
    };
    let _active = rt.begin()?;
    let (mut cred, name) = factory::load_account(rt, id)?;
    factory::ensure_enabled(rt, &cred)?;
    let (model, mut probe) = {
        let _mutation = rt.management_lock.lock().map_err(|_| Fault::internal())?;
        let mut store = rt.lock()?;
        let model = store
            .account_model(id, model_id)
            .filter(|m| m.id == model_id && m.enabled)
            .ok_or_else(|| Fault::new(404, "model_not_found", "模型路由未启用或不存在"))?;
        let account = store.accounts.get_mut(id).ok_or_else(stale)?;
        if !known_blocks(account, &model).is_empty() {
            return Err(Fault::new(
                400,
                "candidate_blocked",
                "官方候选明确禁用、弃用或属于未支持的自定义/路由模型",
            ));
        }
        if let Some(snapshot) = &snapshot_id {
            if account.discovery.snapshot_id.as_ref() != Some(snapshot)
                || account.discovery.stale
                || account
                    .discovery
                    .fresh_until
                    .as_deref()
                    .is_none_or(|end| end <= now().as_str())
                || account.discovery.credential_fingerprint != account.credential_fingerprint
                || !account
                    .discovery
                    .candidates
                    .iter()
                    .any(|c| candidate_id(c) == Some(&model.upstream_id))
            {
                return Err(Fault::new(
                    409,
                    "discovery_stale",
                    "候选快照已过期、不匹配或不包含此模型，请重新发现",
                ));
            }
        }
        let access = account.model_access.entry(model_id.into()).or_default();
        access.operation_generation = access.operation_generation.saturating_add(1);
        if access.verification_status != "basic_call_verified" {
            access.verification_status = "validating".into();
        }
        access.last_probe_status = Some("validating".into());
        access.last_probe_error = None;
        let probe = Probe {
            credential_generation: account.credential_generation,
            credential_fingerprint: account.credential_fingerprint.clone(),
            route_fingerprint: route_fingerprint(&model),
            operation_generation: access.operation_generation,
            discovery_snapshot_id: snapshot_id.clone(),
        };
        store.persist()?;
        (model, probe)
    };
    if let Err(error) = factory::whoami(rt, &mut cred, callback) {
        finish_probe(rt, id, &model, &probe, None, None, Err(error.clone()))?;
        return Err(error);
    }
    {
        let _mutation = rt.management_lock.lock().map_err(|_| Fault::internal())?;
        check_probe(&*rt.lock()?, id, &model, &probe)?;
        factory::save_account(rt, &cred, &name)?;
        factory::upsert_account(rt, &cred, &name, true)?;
        let mut store = rt.lock()?;
        let account = store.accounts.get_mut(id).ok_or_else(stale)?;
        if account.credential_fingerprint != probe.credential_fingerprint {
            // A newly learned organization or regional host starts a new credential generation.
            // A caller's catalog selection must be refreshed for that new identity.
            if snapshot_id.is_some() {
                drop(store);
                rt.catalog_dirty.store(true, Ordering::Release);
                let _ = management::reconcile(rt);
                return Err(stale());
            }
            let access = account.model_access.entry(model_id.into()).or_default();
            access.operation_generation = access.operation_generation.saturating_add(1);
            access.verification_status = "validating".into();
            access.last_probe_status = Some("validating".into());
            probe.credential_fingerprint = account.credential_fingerprint.clone();
            probe.credential_generation = account.credential_generation;
            probe.operation_generation = access.operation_generation;
        }
        store.persist()?;
    }
    check_probe(&*rt.lock()?, id, &model, &probe)?;
    let (request_id, usage, ledger, response_model, result) =
        basic_call(rt, &cred, &model, callback);
    finish_probe(
        rt,
        id,
        &model,
        &probe,
        Some((&request_id, usage, &ledger)),
        response_model,
        result.clone(),
    )?;
    result?;
    account_models(&*rt.lock()?, id)
}
fn basic_call(
    rt: &Arc<Runtime>,
    cred: &Value,
    model: &Model,
    callback: &str,
) -> (
    String,
    Option<UsageTokens>,
    Value,
    Option<String>,
    Result<()>,
) {
    let request_id = format!("validation-{}", Uuid::new_v4());
    let requested_at = now();
    let started = Instant::now();
    let mut code = None;
    let mut response_model = None;
    let mut usage = None;
    let result = (|| -> Result<()> {
        let payload = match Format::for_model(&model)? {
            Format::Chat => {
                json!({"model":model.upstream_id,"messages":[{"role":"user","content":"Reply OK"}],"max_tokens":128,"stream":false})
            }
            Format::Claude => {
                json!({"model":model.upstream_id,"messages":[{"role":"user","content":"Reply OK"}],"max_tokens":128,"stream":false})
            }
            Format::Responses => {
                json!({"model":model.upstream_id,"input":"Reply OK","max_output_tokens":128,"stream":false})
            }
        };
        let request = protocol::request(payload.to_string().as_bytes(), &model, false)?;
        let op = rt.operation(callback, Some(60))?;
        let http = factory::llm_request(
            rt,
            &cred,
            &model,
            &request,
            &op,
            &json!({}),
            &json!({}),
            None,
        )?;
        let response = rt.host.call("host.http.do", &http)?;
        let status = response["StatusCode"]
            .as_u64()
            .and_then(|n| u16::try_from(n).ok())
            .unwrap_or(502);
        code = Some(status);
        if !(200..300).contains(&status) {
            return Err(factory::upstream_error(status, &response["Headers"]));
        }
        let bytes = decode(&response["Body"])?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(Fault::new(
                502,
                "model_validation_failed",
                "基础验证响应超过安全限制",
            ));
        }
        protocol::validate_response(&bytes, Format::for_model(&model)?)?;
        let response: Value = serde_json::from_slice(&bytes)
            .map_err(|_| Fault::new(502, "model_validation_failed", "基础验证响应无效"))?;
        let text = |value: &Value| value.as_str().is_some_and(|s| !s.trim().is_empty());
        let completed = match Format::for_model(model)? {
            Format::Chat => response["choices"].as_array().is_some_and(|choices| {
                choices.iter().any(|choice| {
                    text(&choice["finish_reason"]) && text(&choice["message"]["content"])
                })
            }),
            Format::Claude => {
                text(&response["stop_reason"])
                    && response["content"].as_array().is_some_and(|content| {
                        content
                            .iter()
                            .any(|part| part["type"] == "text" && text(&part["text"]))
                    })
            }
            Format::Responses => {
                matches!(
                    response["status"].as_str(),
                    Some("completed" | "incomplete")
                ) && response["output"].as_array().is_some_and(|items| {
                    items.iter().any(|item| {
                        item["content"].as_array().is_some_and(|content| {
                            content
                                .iter()
                                .any(|part| part["type"] == "output_text" && text(&part["text"]))
                        })
                    })
                })
            }
        };
        if !completed {
            return Err(Fault::new(
                502,
                "model_validation_failed",
                "基础验证未返回具有原生完成状态的非空文本，未允许此模型",
            ));
        }
        response_model = response["model"]
            .as_str()
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 256
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_/.:".contains(&b))
                    && !s.contains("fk-")
                    && !s.contains("sk-")
                    && cred["api_key"]
                        .as_str()
                        .filter(|key| !key.is_empty())
                        .is_none_or(|key| !s.contains(key))
            })
            .map(str::to_string);
        usage = probe_usage(&response["usage"], &model);
        Ok(())
    })();
    let ledger = json!({"RequestedAt":requested_at,"Latency":(started.elapsed().as_nanos().min(u64::MAX as u128)as u64),
        "Stream":false,"Failure":{"StatusCode":code.unwrap_or(502)}});
    (request_id, usage, ledger, response_model, result)
}
fn finish_probe(
    rt: &Arc<Runtime>,
    id: &str,
    model: &Model,
    probe: &Probe,
    ledger: Option<(&str, Option<UsageTokens>, &Value)>,
    response_model: Option<String>,
    result: Result<()>,
) -> Result<()> {
    let _mutation = rt.management_lock.lock().map_err(|_| Fault::internal())?;
    let mut store = rt.lock()?;
    let request_id = ledger.as_ref().map(|(id, _, _)| (*id).to_string());
    if let Some((request_id, tokens, record)) = ledger {
        store.record_kind(
            request_id,
            id,
            model,
            result.is_ok(),
            tokens,
            record,
            "validation",
        )?;
    }
    if rt.closing.load(Ordering::Acquire) {
        return Err(Fault::new(
            503,
            "plugin_stopping",
            "插件正在停止，验证结果未允许发布",
        ));
    }
    check_probe(&store, id, model, probe)?;
    let account = store.accounts.get_mut(id).ok_or_else(stale)?;
    let previous = account
        .model_access
        .get(&model.id)
        .cloned()
        .unwrap_or_default();
    match &result {
        Ok(()) => {
            account.selected_models.insert(model.id.clone());
            account.model_access.insert(
                model.id.clone(),
                ModelAccess {
                    allowed: true,
                    verification_status: "basic_call_verified".into(),
                    verified_at: Some(now()),
                    verification_source: Some("factory_basic_call".into()),
                    last_error: None,
                    credential_fingerprint: probe.credential_fingerprint.clone(),
                    credential_generation: probe.credential_generation,
                    route_fingerprint: probe.route_fingerprint.clone(),
                    discovery_snapshot_id: probe.discovery_snapshot_id.clone(),
                    validation_request_id: request_id,
                    response_model,
                    operation_generation: probe.operation_generation,
                    selection_source: Some(
                        if probe.discovery_snapshot_id.is_some() {
                            "official_droid_cli"
                        } else {
                            "manual"
                        }
                        .into(),
                    ),
                    last_probe_error: None,
                    last_probe_status: Some("success".into()),
                },
            );
        }
        Err(error) => {
            if error.http_status == 401 {
                invalidate_identity(account, "credential_invalid");
            }
            let access = account.model_access.entry(model.id.clone()).or_default();
            let same = previous.allowed
                && previous.verification_status == "basic_call_verified"
                && previous.credential_fingerprint == probe.credential_fingerprint
                && previous.route_fingerprint == probe.route_fingerprint;
            if !same || [401, 403].contains(&error.http_status) {
                access.allowed = false;
                access.verification_status = if [401, 403].contains(&error.http_status) {
                    "invalidated"
                } else {
                    "failed"
                }
                .into();
                access.last_error = Some(error.code.clone());
            }
            access.last_probe_error = Some(error.code.clone());
            access.last_probe_status = Some(error.http_status.to_string());
        }
    }
    if let Err(error) = store.persist() {
        if let Some(account) = store.accounts.get_mut(id) {
            if result.is_ok() {
                account.model_access.insert(model.id.clone(), previous);
            }
        }
        return Err(error);
    }
    drop(store);
    rt.catalog_dirty.store(true, Ordering::Release);
    if result.is_ok() {
        management::reconcile(rt)
    } else {
        let _ = management::reconcile(rt);
        Ok(())
    }
}
fn probe_usage(usage: &Value, model: &Model) -> Option<UsageTokens> {
    let (input, output, cached, cache_write, reasoning) = match Format::for_model(model).ok()? {
        Format::Chat => (
            usage["prompt_tokens"].as_u64()?,
            usage["completion_tokens"].as_u64()?,
            usage["prompt_tokens_details"]["cached_tokens"]
                .as_u64()
                .unwrap_or(0),
            0,
            usage["completion_tokens_details"]["reasoning_tokens"]
                .as_u64()
                .unwrap_or(0),
        ),
        Format::Responses => (
            usage["input_tokens"].as_u64()?,
            usage["output_tokens"].as_u64()?,
            usage["input_tokens_details"]["cached_tokens"]
                .as_u64()
                .unwrap_or(0),
            0,
            usage["output_tokens_details"]["reasoning_tokens"]
                .as_u64()
                .unwrap_or(0),
        ),
        Format::Claude => {
            let cached = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
            let write = usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
            (
                usage["input_tokens"]
                    .as_u64()?
                    .checked_add(cached)?
                    .checked_add(write)?,
                usage["output_tokens"].as_u64()?,
                cached,
                write,
                0,
            )
        }
    };
    if cached.checked_add(cache_write)? > input
        || reasoning > output
        || input.checked_add(output)? == 0
    {
        return None;
    }
    Some(UsageTokens {
        input,
        output,
        cached,
        cache_write,
        reasoning,
    })
}

#[derive(Clone)]
pub struct Draft {
    id: String,
    credential: Value,
    account: Account,
    selected: BTreeSet<String>,
    expires: Instant,
    expires_at: String,
    generation: u64,
    committed_account_id: Option<String>,
    enabled: bool,
}
fn draft_missing() -> Fault {
    Fault::new(
        404,
        "draft_not_found",
        "草稿不存在、已取消或已过期，请重新输入Key创建",
    )
}
fn draft_entry<'a>(
    drafts: &'a mut std::collections::BTreeMap<String, Draft>,
    id: &str,
) -> Result<&'a mut Draft> {
    drafts.retain(|_, draft| draft.expires > Instant::now());
    drafts.get_mut(id).ok_or_else(draft_missing)
}
fn draft_view(rt: &Runtime, draft: &Draft) -> Result<Value> {
    let persisted = rt.lock()?;
    let mut store = Store::default();
    store.settings = persisted.settings.clone();
    store.ledger = persisted
        .ledger
        .iter()
        .filter(|e| e.account_id == draft.account.id)
        .cloned()
        .collect();
    store
        .accounts
        .insert(draft.account.id.clone(), draft.account.clone());
    let mut view = account_models(&store, &draft.account.id)?;
    view["id"] = json!(draft.id);
    view["label"] = json!(draft.account.label);
    view["enabled"] = json!(draft.enabled);
    view["expires_at"] = json!(draft.expires_at);
    view["selected_model_ids"] = json!(draft.selected);
    view["committed_account_id"] = json!(draft.committed_account_id);
    view["can_commit"] = json!(draft.account.identity_verified);
    view["ready_model_count"] = json!(store
        .effective_models(&draft.account.id)
        .iter()
        .filter(|m| draft.selected.contains(&m.id))
        .count());
    view["pending_model_count"] = json!(draft
        .selected
        .iter()
        .filter(|id| !store
            .effective_models(&draft.account.id)
            .iter()
            .any(|m| &m.id == *id))
        .count());
    view["model_overrides"] = json!(draft
        .account
        .model_overrides
        .iter()
        .map(|(id, overrides)| {
            let mut value = serde_json::to_value(overrides).unwrap_or(json!({}));
            value["model_id"] = json!(id);
            value
        })
        .collect::<Vec<_>>());
    if let Some(models) = view["models"].as_array_mut() {
        for row in models {
            row["selected"] = json!(row["model_id"]
                .as_str()
                .is_some_and(|id| draft.selected.contains(id)));
            row["published"] = json!(false);
            row["publication_status"] = json!("unpublished");
        }
    }
    Ok(view)
}
fn label(value: Option<&Value>, fallback: &str) -> Result<String> {
    let label = match value {
        None => fallback,
        Some(value) => value.as_str().ok_or_else(|| Fault::input("账号名称无效"))?,
    }
    .trim();
    if label.is_empty() || label.chars().count() > 80 || label.chars().any(char::is_control) {
        return Err(Fault::input("账号名称必须为1至80个可见字符"));
    }
    Ok(label.into())
}
pub fn create_draft(rt: &Arc<Runtime>, data: &Value, callback: &str) -> Result<Value> {
    let _active = rt.begin()?;
    let key = data["api_key"]
        .as_str()
        .ok_or_else(|| Fault::input("缺少api_key"))?
        .trim();
    factory::validate_key(key)?;
    let name = label(data.get("label"), "Factory")?;
    let enabled = data
        .get("enabled")
        .map(|v| {
            v.as_bool()
                .ok_or_else(|| Fault::input("enabled必须为布尔值"))
        })
        .transpose()?
        .unwrap_or(true);
    let mut credential = factory::new_credential(key, &name, true);
    factory::whoami(rt, &mut credential, callback)?;
    let account_id = credential["account_id"]
        .as_str()
        .ok_or_else(Fault::internal)?
        .to_string();
    let account = Account {
        id: account_id.clone(),
        label: name,
        kind: "api_key".into(),
        enabled: true,
        credential_hint: factory::key_hint(key),
        auth_file: format!("factory-droid-{account_id}.json"),
        status: if enabled { "ready" } else { "disabled" }.into(),
        created_at: now(),
        updated_at: now(),
        last_checked_at: Some(now()),
        identity_verified: true,
        credential_generation: 1,
        credential_fingerprint: credential_fingerprint(&credential),
        identity: json!({"email":credential["email"],"organization":credential["org_id"],"region":credential["region"]}),
        official_limits: factory::unavailable_limits("not_checked", "草稿尚未查询官方额度"),
        official_usage: factory::unavailable_usage("not_checked", "草稿尚未查询官方用量"),
        ..Account::default()
    };
    let draft = Draft {
        id: format!("draft-{}", Uuid::new_v4()),
        credential,
        account,
        selected: BTreeSet::new(),
        expires: Instant::now() + Duration::from_secs(1800),
        expires_at: (Utc::now() + ChronoDuration::minutes(30))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        generation: 0,
        committed_account_id: None,
        enabled,
    };
    let mut drafts = rt.drafts.lock().map_err(|_| Fault::internal())?;
    if rt.closing.load(Ordering::Acquire) {
        return Err(Fault::new(
            503,
            "plugin_stopping",
            "插件正在停止，草稿未保存",
        ));
    }
    drafts.retain(|_, d| d.expires > Instant::now());
    if drafts.len() >= 32 {
        return Err(Fault::new(
            429,
            "draft_limit",
            "未完成草稿超过32个，请取消旧草稿后重试",
        ));
    }
    let view = draft_view(rt, &draft)?;
    drafts.insert(draft.id.clone(), draft);
    Ok(view)
}
pub fn get_draft(rt: &Runtime, id: &str) -> Result<Value> {
    let mut drafts = rt.drafts.lock().map_err(|_| Fault::internal())?;
    draft_view(rt, draft_entry(&mut drafts, id)?)
}
pub fn delete_draft(rt: &Runtime, id: &str) -> Result<Value> {
    let mut drafts = rt.drafts.lock().map_err(|_| Fault::internal())?;
    draft_entry(&mut drafts, id)?;
    cancel_discovery(rt, id);
    drafts.remove(id);
    Ok(json!({"id":id,"deleted":true}))
}
pub fn select_draft_models(rt: &Runtime, data: &Value) -> Result<Value> {
    let id = data["id"].as_str().ok_or_else(draft_missing)?;
    let rows = data["model_ids"]
        .as_array()
        .filter(|r| r.len() <= 100)
        .ok_or_else(|| Fault::input("model_ids必须为最多100项的数组"))?;
    let selected: BTreeSet<String> = rows
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or_else(|| Fault::input("model_ids必须为字符串数组"))
        })
        .collect::<Result<_>>()?;
    if selected.len() != rows.len() {
        return Err(Fault::input("model_ids包含重复模型"));
    }
    let mut drafts = rt.drafts.lock().map_err(|_| Fault::internal())?;
    let draft = draft_entry(&mut drafts, id)?;
    if draft.committed_account_id.is_some() {
        return Err(Fault::new(
            409,
            "draft_already_committed",
            "草稿已保存，请在账号模型页修改",
        ));
    }
    let store = rt.lock()?;
    let mut overrides = if let Some(rows) = data.get("model_overrides") {
        let rows = rows
            .as_array()
            .filter(|r| r.len() <= 100)
            .ok_or_else(|| Fault::input("model_overrides须为最多100项的数组"))?;
        let mut result = std::collections::BTreeMap::new();
        for row in rows {
            let id = row["model_id"]
                .as_str()
                .filter(|id| selected.contains(*id))
                .ok_or_else(|| Fault::input("override必须属于已选模型"))?;
            let mut value = row.clone();
            value
                .as_object_mut()
                .ok_or_else(|| Fault::input("override必须为对象"))?
                .remove("model_id");
            let value: ModelOverride =
                serde_json::from_value(value).map_err(|_| Fault::input("override字段无效"))?;
            value.validate()?;
            if result.insert(id.to_string(), value).is_some() {
                return Err(Fault::input("override模型重复"));
            }
        }
        result
    } else {
        draft.account.model_overrides.clone()
    };
    overrides.retain(|id, _| selected.contains(id));
    for (id, override_) in &overrides {
        if override_.alias.as_deref().is_some_and(|alias| {
            store
                .settings
                .models
                .iter()
                .any(|m| &m.id != id && m.id.strip_prefix("factory/").unwrap_or(&m.id) == alias)
        }) {
            return Err(Fault::new(
                409,
                "model_alias_conflict",
                "模型别名不能占用其他模型的原始ID后缀",
            ));
        }
    }
    let mut aliases = BTreeSet::new();
    for model in &store.settings.models {
        let suffix = overrides
            .get(&model.id)
            .and_then(|v| v.alias.as_deref())
            .unwrap_or_else(|| model.id.strip_prefix("factory/").unwrap_or(&model.id));
        if !aliases.insert(suffix) {
            return Err(Fault::new(
                409,
                "model_alias_conflict",
                "账号内的模型别名与现有模型ID或别名冲突",
            ));
        }
    }
    for id in &selected {
        let model = store
            .settings
            .models
            .iter()
            .find(|m| &m.id == id && m.enabled)
            .ok_or_else(|| Fault::new(404, "model_not_found", "所选模型路由不存在或已停用"))?;
        if !known_blocks(&draft.account, model).is_empty() {
            return Err(Fault::new(
                400,
                "candidate_blocked",
                "所选候选已禁用、弃用或属于不支持的模型类型",
            ));
        }
    }
    drop(store);
    if draft.selected != selected {
        draft.generation = draft.generation.saturating_add(1);
    }
    for (id, access) in &mut draft.account.model_access {
        if !selected.contains(id) {
            access.allowed = false;
            access.operation_generation = access.operation_generation.saturating_add(1);
        }
    }
    draft.selected = selected;
    draft.account.selected_models = draft.selected.clone();
    draft.account.model_overrides = overrides;
    draft_view(rt, draft)
}
pub fn discover_draft(rt: &Arc<Runtime>, id: &str) -> Result<Value> {
    let _active = rt.begin()?;
    let executable = rt.config.droid_cli_path.clone().ok_or_else(|| {
        Fault::new(
            400,
            "discovery_not_configured",
            "管理员尚未配置droid-cli-path",
        )
    })?;
    let (credential, generation, run) = {
        let _mutation = rt.management_lock.lock().map_err(|_| Fault::internal())?;
        let mut drafts = rt.drafts.lock().map_err(|_| Fault::internal())?;
        let draft = draft_entry(&mut drafts, id)?;
        if draft.committed_account_id.is_some() {
            return Err(Fault::new(409, "draft_already_committed", "草稿已保存"));
        }
        draft.generation = draft.generation.saturating_add(1);
        draft.account.discovery.status = "loading".into();
        draft.account.discovery.stale = true;
        (
            draft.credential.clone(),
            draft.generation,
            DiscoveryRun::begin(rt, id)?,
        )
    };
    let root = rt
        .lock()?
        .path
        .parent()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(Fault::internal)?;
    let root = std::fs::canonicalize(root)
        .map_err(|_| Fault::storage("无法定位数据目录"))?
        .join("discovery-runs");
    let result = discovery::discover(
        discovery::Options {
            executable,
            work_root: root,
            timeout: Duration::from_secs(rt.config.discovery_timeout_seconds.unwrap_or(30)),
            cancel: Some(run.cancel.clone()),
        },
        credential["api_key"].as_str().unwrap_or(""),
    );
    let _mutation = rt.management_lock.lock().map_err(|_| Fault::internal())?;
    let mut drafts = rt.drafts.lock().map_err(|_| Fault::internal())?;
    let draft = draft_entry(&mut drafts, id)?;
    if draft.generation != generation || draft.committed_account_id.is_some() {
        return Err(stale());
    }
    match result {
        Ok(snapshot) => {
            let candidates = safe_candidates(
                snapshot.models,
                credential["api_key"].as_str().unwrap_or(""),
            )?;
            register_discovered_models(&mut *rt.lock()?, &candidates)?;
            draft.account.discovery = DiscoveryState {
                status: "candidate_catalog".into(),
                source: "official_droid_cli".into(),
                snapshot_id: Some(Uuid::new_v4().to_string()),
                fetched_at: Some(now()),
                fresh_until: Some(
                    (Utc::now() + ChronoDuration::minutes(5))
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                ),
                stale: false,
                cli_version: snapshot.cli_version,
                protocol_version: snapshot.protocol_version,
                message: "官方CLI候选不代表账号模型授权".into(),
                candidates,
                generation,
                credential_fingerprint: credential_fingerprint(&credential),
            };
            let store = rt.lock()?;
            for model in &store.settings.models {
                if !known_blocks(&draft.account, model).is_empty() {
                    if let Some(access) = draft.account.model_access.get_mut(&model.id) {
                        access.allowed = false;
                        access.verification_status = "invalidated".into();
                        access.operation_generation = access.operation_generation.saturating_add(1);
                    }
                }
            }
            drop(store);
            draft_view(rt, draft)
        }
        Err(error) => {
            draft.account.discovery.status = "error".into();
            draft.account.discovery.stale = true;
            draft.account.discovery.message = error.message.clone();
            Err(Fault::new(
                if error.code == "discovery_timeout" {
                    504
                } else {
                    502
                },
                &error.code,
                &error.message,
            ))
        }
    }
}
pub fn test_draft(rt: &Arc<Runtime>, data: &Value, callback: &str) -> Result<Value> {
    if data["confirm"] != true {
        return Err(Fault::new(
            400,
            "confirmation_required",
            "基础测试会消耗额度，请明确confirm:true",
        ));
    }
    if data.as_object().is_none_or(|o| {
        o.keys()
            .any(|k| !["id", "model_id", "confirm", "discovery_snapshot_id"].contains(&k.as_str()))
    }) {
        return Err(Fault::input(
            "基础测试不接受自定义prompt、body、tools或路由",
        ));
    }
    let id = data["id"].as_str().ok_or_else(draft_missing)?;
    let model_id = data["model_id"]
        .as_str()
        .ok_or_else(|| Fault::input("缺少model_id"))?;
    let _active = rt.begin()?;
    let (credential, account_id, model, generation, operation_generation, snapshot_id) = {
        let _mutation = rt.management_lock.lock().map_err(|_| Fault::internal())?;
        let mut drafts = rt.drafts.lock().map_err(|_| Fault::internal())?;
        let draft = draft_entry(&mut drafts, id)?;
        if draft.committed_account_id.is_some() {
            return Err(Fault::new(409, "draft_already_committed", "草稿已保存"));
        }
        if !draft.selected.contains(model_id) {
            return Err(Fault::input("请先勾选待测试模型"));
        }
        if !draft.account.identity_verified {
            return Err(Fault::new(
                401,
                "credential_invalid",
                "草稿凭据身份已失效，请重新创建草稿",
            ));
        }
        let store = rt.lock()?;
        let model = store
            .settings
            .models
            .iter()
            .find(|m| m.id == model_id && m.enabled)
            .cloned()
            .ok_or_else(|| Fault::new(404, "model_not_found", "模型路由已停用或不存在"))?;
        if !known_blocks(&draft.account, &model).is_empty() {
            return Err(Fault::new(
                400,
                "candidate_blocked",
                "候选模型已被明确禁用、弃用或类型不支持",
            ));
        }
        let snapshot_id = match data.get("discovery_snapshot_id") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            _ => return Err(Fault::input("快照ID无效")),
        };
        if snapshot_id.as_ref().is_some_and(|snapshot| {
            draft.account.discovery.snapshot_id.as_ref() != Some(snapshot)
                || draft.account.discovery.stale
                || draft
                    .account
                    .discovery
                    .fresh_until
                    .as_deref()
                    .is_none_or(|end| end <= now().as_str())
                || !draft
                    .account
                    .discovery
                    .candidates
                    .iter()
                    .any(|c| candidate_id(c) == Some(&model.upstream_id))
        }) {
            return Err(Fault::new(409, "discovery_stale", "候选快照过期或不匹配"));
        }
        let access = draft
            .account
            .model_access
            .entry(model_id.into())
            .or_default();
        access.operation_generation = access.operation_generation.saturating_add(1);
        if access.verification_status != "basic_call_verified" {
            access.verification_status = "validating".into();
        }
        access.last_probe_status = Some("validating".into());
        (
            draft.credential.clone(),
            draft.account.id.clone(),
            model,
            draft.generation,
            access.operation_generation,
            snapshot_id,
        )
    };
    let (request_id, tokens, ledger, response_model, result) =
        basic_call(rt, &credential, &model, callback);
    rt.lock()?.record_kind(
        &request_id,
        &account_id,
        &model,
        result.is_ok(),
        tokens,
        &ledger,
        "validation",
    )?;
    let mut drafts = rt.drafts.lock().map_err(|_| Fault::internal())?;
    let draft = draft_entry(&mut drafts, id)?;
    let store = rt.lock()?;
    if draft.generation != generation
        || draft.committed_account_id.is_some()
        || !draft.selected.contains(model_id)
        || draft
            .account
            .model_access
            .get(model_id)
            .is_none_or(|a| a.operation_generation != operation_generation)
        || store
            .settings
            .models
            .iter()
            .find(|m| m.id == model.id && m.enabled)
            .is_none_or(|m| route_fingerprint(m) != route_fingerprint(&model))
    {
        return Err(stale());
    }
    drop(store);
    let access = draft
        .account
        .model_access
        .get_mut(model_id)
        .ok_or_else(stale)?;
    match &result {
        Ok(()) => {
            *access = ModelAccess {
                allowed: true,
                verification_status: "basic_call_verified".into(),
                verified_at: Some(now()),
                verification_source: Some("factory_basic_call".into()),
                credential_fingerprint: draft.account.credential_fingerprint.clone(),
                credential_generation: draft.account.credential_generation,
                route_fingerprint: route_fingerprint(&model),
                discovery_snapshot_id: snapshot_id.clone(),
                validation_request_id: Some(request_id),
                response_model,
                operation_generation,
                selection_source: Some(
                    if snapshot_id.is_some() {
                        "official_droid_cli"
                    } else {
                        "manual"
                    }
                    .into(),
                ),
                last_probe_status: Some("success".into()),
                ..ModelAccess::default()
            }
        }
        Err(error) => {
            if !access.allowed
                || access.verification_status != "basic_call_verified"
                || [401, 403].contains(&error.http_status)
            {
                access.allowed = false;
                access.verification_status = "failed".into();
                access.last_error = Some(error.code.clone());
            }
            access.last_probe_error = Some(error.code.clone());
            access.last_probe_status = Some(error.http_status.to_string());
            if error.http_status == 401 {
                invalidate_identity(&mut draft.account, "credential_invalid");
            }
        }
    }
    result?;
    draft_view(rt, draft)
}
pub fn commit_draft(rt: &Arc<Runtime>, data: &Value) -> Result<Value> {
    let id = data["id"].as_str().ok_or_else(draft_missing)?;
    let draft = {
        let mut drafts = rt.drafts.lock().map_err(|_| Fault::internal())?;
        let draft = draft_entry(&mut drafts, id)?;
        if draft.committed_account_id.is_some() {
            let account_id = draft.account.id.clone();
            drop(drafts);
            management::reconcile(rt)?;
            let store = rt.lock()?;
            return store
                .accounts
                .get(&account_id)
                .map(|a| a.view(&store))
                .ok_or_else(draft_missing);
        }
        let store = rt.lock()?;
        if !draft.account.identity_verified {
            return Err(Fault::new(
                401,
                "credential_invalid",
                "草稿凭据身份已失效，请重新创建草稿",
            ));
        }
        drop(store);
        let name = label(data.get("label"), &draft.account.label)?;
        let enabled = data
            .get("enabled")
            .map(|v| {
                v.as_bool()
                    .ok_or_else(|| Fault::input("enabled必须为布尔值"))
            })
            .transpose()?
            .unwrap_or(draft.enabled);
        draft.account.label = name.clone();
        draft.account.enabled = enabled;
        draft.credential["label"] = json!(name);
        draft.credential["disabled"] = json!(!enabled);
        cancel_discovery(rt, id);
        draft.generation = draft.generation.saturating_add(1);
        for access in draft.account.model_access.values_mut() {
            if access.last_probe_status.as_deref() == Some("validating") {
                access.operation_generation = access.operation_generation.saturating_add(1);
                if access.verification_status == "validating" {
                    access.verification_status = "not_verified".into();
                }
                access.last_probe_status = Some("cancelled".into());
                access.last_probe_error = Some("draft_committed".into());
            }
        }
        draft.clone()
    };
    for file in factory::host_accounts(rt)? {
        if let Some(index) = file["auth_index"].as_str().filter(|s| !s.is_empty()) {
            let response = rt
                .host
                .call("host.auth.get", &json!({"auth_index":index}))?;
            if response["json"]["api_key"] == draft.credential["api_key"]
                && response["json"]["account_id"] != draft.credential["account_id"]
            {
                return Err(Fault::new(
                    409,
                    "duplicate_account",
                    "此Factory Key已存在，请管理已有账号",
                ));
            }
        }
    }
    factory::save_account(rt, &draft.credential, &draft.account.auth_file)?;
    {
        let mut store = rt.lock()?;
        let mut account = draft.account.clone();
        account.selected_models = draft.selected.clone();
        account
            .model_access
            .retain(|id, _| draft.selected.contains(id));
        let prior = store.accounts.insert(account.id.clone(), account);
        if let Err(error) = store.persist() {
            if let Some(prior) = prior {
                store.accounts.insert(draft.account.id.clone(), prior);
            } else {
                store.accounts.remove(&draft.account.id);
            }
            return Err(error);
        }
    }
    if let Ok(mut drafts) = rt.drafts.lock() {
        if let Some(current) = drafts.get_mut(id) {
            current.committed_account_id = Some(draft.account.id.clone());
        }
    }
    rt.catalog_dirty.store(true, Ordering::Release);
    management::reconcile(rt)?;
    let store = rt.lock()?;
    store
        .accounts
        .get(&draft.account.id)
        .map(|a| a.view(&store))
        .ok_or_else(draft_missing)
}
