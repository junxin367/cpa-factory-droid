use chrono::{Datelike, Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

use crate::{
    access::{DiscoveryState, ModelAccess},
    pricing::{Price, PriceRule},
    Fault, Result, PROVIDER,
};
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelOverride {
    pub alias: Option<String>,
    pub input_price_usd_per_million: Option<f64>,
    pub output_price_usd_per_million: Option<f64>,
    pub cache_read_price_usd_per_million: Option<f64>,
    pub cache_write_price_usd_per_million: Option<f64>,
}
impl ModelOverride {
    fn has_legacy_prices(&self) -> bool {
        [
            self.input_price_usd_per_million,
            self.output_price_usd_per_million,
            self.cache_read_price_usd_per_million,
            self.cache_write_price_usd_per_million,
        ]
        .into_iter()
        .any(|price| price.is_some())
    }
    pub fn validate(&self) -> Result<()> {
        if self.alias.as_ref().is_some_and(|s| {
            s.is_empty()
                || s.len() > 80
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        }) {
            return Err(Fault::input(
                "账号模型别名须为1至80个字母、数字、点、下划线或短横线，不能包含factory/前缀",
            ));
        }
        for price in [
            self.input_price_usd_per_million,
            self.output_price_usd_per_million,
            self.cache_read_price_usd_per_million,
            self.cache_write_price_usd_per_million,
        ]
        .into_iter()
        .flatten()
        {
            if !price.is_finite() || !(0.0..=1_000_000.0).contains(&price) {
                return Err(Fault::input("模型价格须为非负有限数或null继承全局"));
            }
        }
        if self.has_legacy_prices() {
            return Err(Fault::input("模型价格请在全局模型设置中统一维护"));
        }
        Ok(())
    }
}

pub fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
pub fn auth_id_matches(auth_id: &str, account_id: &str, file: &str) -> bool {
    !auth_id.is_empty()
        && (auth_id == account_id
            || !file.is_empty()
                && (auth_id == file
                    || auth_id
                        .strip_suffix(file)
                        .is_some_and(|prefix| prefix.ends_with('/') || prefix.ends_with('\\'))))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Model {
    pub id: String,
    pub upstream_id: String,
    pub display_name: String,
    pub protocol: String,
    pub upstream_provider: String,
    pub enabled: bool,
    pub supports_images: bool,
    pub context_length: u64,
    pub max_output_tokens: u64,
    pub input_multiplier: f64,
    pub output_multiplier: f64,
    pub cache_multiplier: f64,
    pub cache_creation_multiplier: f64,
    pub input_price_usd_per_million: Option<f64>,
    pub output_price_usd_per_million: Option<f64>,
    pub cache_read_price_usd_per_million: Option<f64>,
    pub cache_write_price_usd_per_million: Option<f64>,
    pub quota_pool: String,
    pub price_rules: Vec<PriceRule>,
    pub quota_pool_source: String,
}
impl Default for Model {
    fn default() -> Self {
        Self {
            id: String::new(),
            upstream_id: String::new(),
            display_name: String::new(),
            protocol: "chat_completions".into(),
            upstream_provider: "openai".into(),
            enabled: true,
            supports_images: false,
            context_length: 0,
            max_output_tokens: 8192,
            input_multiplier: 1.0,
            output_multiplier: 1.0,
            cache_multiplier: 1.0,
            cache_creation_multiplier: 1.0,
            input_price_usd_per_million: None,
            output_price_usd_per_million: None,
            cache_read_price_usd_per_million: None,
            cache_write_price_usd_per_million: None,
            quota_pool: "unknown".into(),
            price_rules: Vec::new(),
            quota_pool_source: "unknown".into(),
        }
    }
}
impl Model {
    pub fn price(&self) -> Price {
        Price {
            input: self.input_price_usd_per_million,
            output: self.output_price_usd_per_million,
            cache_read: self.cache_read_price_usd_per_million,
            cache_write: self.cache_write_price_usd_per_million,
            rules: self.price_rules.clone(),
        }
    }

    pub fn host_json(&self) -> Value {
        json!({"ID":self.id,"Object":"model","OwnedBy":"factory","Type":PROVIDER,
            "DisplayName":self.display_name,"Name":self.id,
            "Description":format!("Factory Droid 原生 {} 协议；可用性取决于账号权限",self.protocol),
            "ContextLength":self.context_length,"InputTokenLimit":self.context_length,
            "MaxCompletionTokens":self.max_output_tokens,"OutputTokenLimit":self.max_output_tokens,
            "SupportedInputModalities":if self.supports_images{vec!["text","image"]}else{vec!["text"]},"SupportedOutputModalities":["text"],
            "SupportedParameters":match self.protocol.as_str(){
                "anthropic"=>vec!["tools","tool_choice","thinking","output_config"],
                "responses"=>vec!["tools","tool_choice","reasoning","include"],
                _=>vec!["tools","tool_choice","reasoning_effort"]},
            "UserDefined":true})
    }
}

pub fn default_models() -> Vec<Model> {
    // Snapshot checked against Factory Droid 0.242 registry on 2026-11-03.
    // Deliberately excludes Fast/Ultra variants requiring separate opt-in.
    let rows = [
        (
            "claude-opus-5-5",
            "Opus 5.5",
            "anthropic",
            "anthropic",
            872000,
            128000,
            1.6,
            "standard",
        ),
        (
            "claude-sonnet-5-5",
            "Sonnet 5.5",
            "anthropic",
            "anthropic",
            872000,
            128000,
            0.8,
            "standard",
        ),
        (
            "claude-haiku-5-5",
            "Haiku 5.5",
            "anthropic",
            "anthropic",
            872000,
            128000,
            0.04,
            "standard",
        ),
        (
            "claude-sonnet-4-6",
            "Sonnet 4.6",
            "anthropic",
            "anthropic",
            931000,
            64000,
            1.2,
            "standard",
        ),
        (
            "gpt-6.1-sol",
            "GPT-6.1 Sol",
            "responses",
            "openai",
            1050000,
            128000,
            0.8,
            "standard",
        ),
        (
            "gpt-6-sol",
            "GPT-6 Sol",
            "responses",
            "openai",
            1050000,
            128000,
            0.8,
            "standard",
        ),
        (
            "gpt-6-astra",
            "GPT-6 Astra",
            "responses",
            "openai",
            1050000,
            128000,
            4.0,
            "standard",
        ),
        (
            "gpt-5.4",
            "GPT-5.4",
            "responses",
            "openai",
            1050000,
            128000,
            1.0,
            "standard",
        ),
        (
            "gpt-5.3-codex",
            "GPT-5.3 Codex",
            "responses",
            "openai",
            400000,
            128000,
            0.7,
            "standard",
        ),
        (
            "glm-5.3",
            "GLM 5.3",
            "chat_completions",
            "fireworks",
            1040000,
            131072,
            0.56,
            "core",
        ),
        (
            "kimi-k3",
            "Kimi K3",
            "chat_completions",
            "fireworks",
            262144,
            65536,
            1.2,
            "core",
        ),
        (
            "deepseek-v4.1-flash",
            "DeepSeek V4.1 Flash",
            "chat_completions",
            "fireworks",
            1040000,
            131072,
            0.12,
            "core",
        ),
        (
            "minimax-m3",
            "MiniMax M3",
            "chat_completions",
            "fireworks",
            512000,
            64000,
            0.12,
            "core",
        ),
    ];
    rows.into_iter()
        .map(
            |(id, name, protocol, provider, context, output, rate, pool)| Model {
                id: format!("factory/{id}"),
                upstream_id: id.into(),
                display_name: name.into(),
                protocol: protocol.into(),
                upstream_provider: provider.into(),
                enabled: true,
                context_length: context,
                max_output_tokens: output,
                input_multiplier: rate,
                output_multiplier: rate,
                cache_multiplier: rate,
                cache_creation_multiplier: rate,
                supports_images: id != "glm-5.3",
                quota_pool: pool.into(),
                quota_pool_source: "factory_registry".into(),
                ..Model::default()
            },
        )
        .collect()
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub request_timeout_seconds: u64,
    pub usage_window: String,
    pub period_start: Option<String>,
    pub period_end: Option<String>,
    pub budget_credits: Option<f64>,
    pub models: Vec<Model>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            request_timeout_seconds: 180,
            usage_window: "calendar_month".into(),
            period_start: None,
            period_end: None,
            budget_credits: None,
            models: default_models(),
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        if !(10..=1800).contains(&self.request_timeout_seconds) {
            return Err(Fault::input("超时必须为 10 至 1800 秒"));
        }
        if !["calendar_month", "custom"].contains(&self.usage_window.as_str()) {
            return Err(Fault::input("统计窗口必须为 calendar_month 或 custom"));
        }
        if self.usage_window == "custom" {
            let (start, end) = self.window()?;
            if start >= end {
                return Err(Fault::input("自定义窗口结束时间必须晚于开始时间"));
            }
        }
        if self
            .budget_credits
            .is_some_and(|v| !v.is_finite() || v < 0.0)
        {
            return Err(Fault::input("预算必须为空或非负有限数"));
        }
        if self.models.is_empty() || self.models.len() > 1024 {
            return Err(Fault::input("模型数量必须为 1 至 1024"));
        }
        let mut ids = std::collections::HashSet::new();
        for m in &self.models {
            if m.id
                .strip_prefix("factory/")
                .and_then(|s| s.split_once('/'))
                .is_some_and(|(scope, _)| scope.starts_with("factory-"))
            {
                return Err(Fault::input(
                    "factory/factory-<账号ID>/ 是固定账号模型的保留命名空间",
                ));
            }
            if !m.id.starts_with("factory/")
                || m.id.len() > 160
                || m.id.len() < 9
                || !m
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_/.".contains(&b))
                || !ids.insert(&m.id)
                || m.upstream_id.is_empty()
                || m.upstream_id.len() > 160
                || !m
                    .upstream_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_/.:".contains(&b))
                || !["anthropic", "responses", "chat_completions"].contains(&m.protocol.as_str())
                || ![
                    "anthropic",
                    "openai",
                    "fireworks",
                    "baseten",
                    "xai",
                    "mistral",
                ]
                .contains(&m.upstream_provider.as_str())
                || m.display_name.len() > 200
                || m.max_output_tokens == 0
                || m.max_output_tokens > 1_000_000
                || !["standard", "core", "unknown"].contains(&m.quota_pool.as_str())
                || !["explicit_config", "factory_registry", "unknown"]
                    .contains(&m.quota_pool_source.as_str())
                || m.quota_pool != "unknown" && m.quota_pool_source == "unknown"
            {
                return Err(Fault::input(
                    "模型映射无效：检查前缀、重复 ID、协议、供应商与输出上限",
                ));
            }
            for n in [
                Some(m.input_multiplier),
                Some(m.output_multiplier),
                Some(m.cache_multiplier),
                Some(m.cache_creation_multiplier),
                m.input_price_usd_per_million,
                m.output_price_usd_per_million,
                m.cache_read_price_usd_per_million,
                m.cache_write_price_usd_per_million,
            ]
            .into_iter()
            .flatten()
            {
                if !n.is_finite() || !(0.0..=1_000_000.0).contains(&n) {
                    return Err(Fault::input("模型倍率与价格必须为非负有限数"));
                }
            }
            m.price()
                .validate()
                .map_err(|message| Fault::input(&message))?;
        }
        Ok(())
    }
    pub fn window(&self) -> Result<(String, String)> {
        if self.usage_window == "custom" {
            let parse = |v: &Option<String>| -> Result<String> {
                chrono::DateTime::parse_from_rfc3339(v.as_deref().unwrap_or(""))
                    .map(|d| {
                        d.with_timezone(&Utc)
                            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
                    })
                    .map_err(|_| Fault::input("自定义窗口需提供 ISO8601 开始和结束时间"))
            };
            return Ok((parse(&self.period_start)?, parse(&self.period_end)?));
        }
        let now = Utc::now();
        let start = Utc
            .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
            .single()
            .unwrap();
        let end = (start + Duration::days(32)).with_day(1).unwrap();
        Ok((
            start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            end.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        ))
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct UsageTokens {
    pub input: u64,
    pub output: u64,
    pub cached: u64,
    #[serde(default)]
    pub cache_write: u64,
    #[serde(default)]
    pub reasoning: u64,
}
fn service_tier_for_record<'a>(host_record: &'a Value, model: &Model) -> Option<&'a str> {
    host_record["ResponseServiceTier"]
        .as_str()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            host_record["ServiceTier"]
                .as_str()
                .filter(|value| !value.is_empty())
        })
        .or_else(|| {
            crate::catalog::required_parameter(model).and_then(|(key, value)| {
                ((key == "service_tier") || (key == "speed" && value == "fast")).then_some(value)
            })
        })
}
#[derive(Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub request_id: String,
    pub account_id: String,
    pub model: String,
    pub at: String,
    pub success: bool,
    pub tokens: Option<UsageTokens>,
    pub estimated_credits: f64,
    pub estimated_cost_usd: Option<f64>,
    #[serde(default)]
    pub status_code: Option<u16>,
    #[serde(default)]
    pub latency_ms: Option<u64>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub pool: String,
    #[serde(default = "inference_kind")]
    pub kind: String,
    #[serde(default)]
    pub pool_source: String,
}
fn inference_kind() -> String {
    "inference".into()
}
#[derive(Clone, Serialize, Deserialize)]
pub struct QuotaSample {
    pub account_id: String,
    pub group: String,
    pub name: String,
    pub reset_at: String,
    pub at: String,
    pub used_percent: f64,
    pub tokens: u64,
    pub cost: Option<f64>,
    pub requests: u64,
    pub measured: u64,
    #[serde(default)]
    pub sample_version: u32,
    #[serde(default)]
    pub ledger_epoch: String,
    #[serde(default)]
    pub window_key: String,
    #[serde(default)]
    pub window_start: String,
    #[serde(default)]
    pub totals: crate::quota_window_usage::CumulativeUsage,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub enabled: bool,
    pub priority: i64,
    pub credential_hint: String,
    pub auth_file: String,
    pub status: String,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_checked_at: Option<String>,
    pub identity: Value,
    pub official_usage: Value,
    #[serde(default)]
    pub official_limits: Value,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub identity_verified: bool,
    #[serde(default)]
    pub credential_generation: u64,
    #[serde(default)]
    pub credential_fingerprint: String,
    #[serde(default)]
    pub discovery: DiscoveryState,
    #[serde(default)]
    pub model_access: BTreeMap<String, ModelAccess>,
    #[serde(default)]
    pub selected_models: std::collections::BTreeSet<String>,
    #[serde(default)]
    pub model_overrides: BTreeMap<String, ModelOverride>,
}
impl Account {
    pub fn view(&self, state: &Store) -> Value {
        json!({
            "id":self.id, "label":self.label, "kind":self.kind,
            "enabled":self.enabled, "priority":self.priority,
            "credential_hint":self.credential_hint, "auth_file":self.auth_file,
            "status":self.status, "last_error":self.last_error,
            "created_at":self.created_at, "updated_at":self.updated_at,
            "last_checked_at":self.last_checked_at,
            "ready_model_count":state.effective_models(&self.id).len(),
            "model_source":self.discovery.source,
            "catalog_checked_at":self.discovery.fetched_at
        })
    }
    pub fn pinned_id(&self, model: &Model) -> String {
        let suffix = self
            .model_overrides
            .get(&model.id)
            .and_then(|m| m.alias.as_deref())
            .unwrap_or_else(|| model.id.strip_prefix("factory/").unwrap_or(&model.id));
        format!("factory/{}/{suffix}", self.id)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    pub version: u32,
    pub settings: Settings,
    pub accounts: BTreeMap<String, Account>,
    pub ledger: Vec<LedgerEntry>,
    pub quota_samples: Vec<QuotaSample>,
    #[serde(skip)]
    pub path: PathBuf,
    #[serde(skip)]
    pub health_error: Option<String>,
    #[serde(skip)]
    pub published_models: BTreeMap<String, std::collections::BTreeSet<String>>,
    #[serde(skip)]
    pub publication_known: bool,
    pub observation_started_at: Option<String>,
    pub ledger_retained_since: Option<String>,
    pub ledger_epoch: String,
    pub network_fingerprint: String,
    pub(crate) wire_namespace: String,
    #[serde(default)]
    legacy_price_migration_completed: bool,
}
impl Default for Store {
    fn default() -> Self {
        Self {
            version: 1,
            settings: Settings::default(),
            accounts: BTreeMap::new(),
            ledger: Vec::new(),
            quota_samples: Vec::new(),
            path: PathBuf::new(),
            health_error: None,
            published_models: BTreeMap::new(),
            publication_known: false,
            observation_started_at: None,
            ledger_retained_since: None,
            ledger_epoch: String::new(),
            network_fingerprint: String::new(),
            wire_namespace: String::new(),
            legacy_price_migration_completed: false,
        }
    }
}
impl Store {
    fn migrate_legacy_account_prices(&mut self) {
        if self.legacy_price_migration_completed {
            return;
        }
        for model in &mut self.settings.models {
            if [
                model.input_price_usd_per_million,
                model.output_price_usd_per_million,
                model.cache_read_price_usd_per_million,
                model.cache_write_price_usd_per_million,
            ]
            .into_iter()
            .any(|price| price.is_some())
            {
                continue;
            }
            let mut candidates = Vec::new();
            for account in self.accounts.values() {
                let Some(override_) = account.model_overrides.get(&model.id) else {
                    continue;
                };
                let prices = [
                    override_.input_price_usd_per_million,
                    override_.output_price_usd_per_million,
                    override_.cache_read_price_usd_per_million,
                    override_.cache_write_price_usd_per_million,
                ];
                if prices.iter().any(Option::is_some) && !candidates.contains(&prices) {
                    candidates.push(prices);
                }
            }
            if candidates.len() != 1 {
                continue;
            }
            let [input, output, cache_read, cache_write] = candidates[0];
            if ![input, output, cache_read, cache_write]
                .into_iter()
                .flatten()
                .all(|price| price.is_finite() && (0.0..=1_000_000.0).contains(&price))
            {
                continue;
            }
            model.input_price_usd_per_million = input;
            model.output_price_usd_per_million = output;
            model.cache_read_price_usd_per_million = cache_read;
            model.cache_write_price_usd_per_million = cache_write;
        }
        self.legacy_price_migration_completed = true;
    }

    pub fn open(dir: PathBuf) -> Result<Arc<Mutex<Self>>> {
        fs::create_dir_all(&dir).map_err(|_| Fault::storage("无法创建插件数据目录"))?;
        let path = dir.join("state.json");
        let mut store: Self = if path.exists() {
            let data = fs::read(&path).map_err(|_| Fault::storage("无法读取插件状态文件"))?;
            serde_json::from_slice(&data)
                .map_err(|_| Fault::storage("插件状态文件损坏；请保留文件并从备份恢复"))?
        } else {
            Self::default()
        };
        if store.version != 1 {
            return Err(Fault::storage("插件状态版本不受支持"));
        }
        store.migrate_legacy_account_prices();
        store.settings.validate()?;
        if store.wire_namespace.is_empty() {
            store.wire_namespace = Uuid::new_v4().to_string();
        } else if Uuid::parse_str(&store.wire_namespace).map_or(true, |value| value.is_nil()) {
            return Err(Fault::storage(
                "会话匿名化命名空间损坏，请从备份恢复插件状态",
            ));
        }
        if store.ledger_epoch.is_empty() {
            store.ledger_epoch = Uuid::new_v4().to_string();
            store.observation_started_at = Some(now());
            store.ledger_retained_since = Some(now());
        }
        for entry in &mut store.ledger {
            if entry.pool_source.is_empty() {
                entry.pool = "unknown".into();
                entry.pool_source = "legacy_unverified".into();
            }
        }
        store.path = path;
        store.persist()?;
        Ok(Arc::new(Mutex::new(store)))
    }
    pub fn persist(&mut self) -> Result<()> {
        let tmp = self.path.with_extension(format!("{}.tmp", Uuid::new_v4()));
        let result = (|| {
            let data = serde_json::to_vec(self).map_err(|_| Fault::storage("无法编码插件状态"))?;
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&tmp)
                .map_err(|_| Fault::storage("无法创建状态临时文件"))?;
            file.write_all(&data)
                .and_then(|_| file.sync_all())
                .map_err(|_| Fault::storage("无法同步插件状态"))?;
            fs::rename(&tmp, &self.path).map_err(|_| Fault::storage("无法原子替换插件状态"))?;
            #[cfg(unix)]
            if let Some(parent) = self.path.parent() {
                let _ = fs::File::open(parent).and_then(|f| f.sync_all());
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
            self.health_error = Some("用量或设置未能持久保存，请检查数据目录".into());
        } else {
            self.health_error = None;
        }
        result
    }
    pub fn record(
        &mut self,
        request_id: &str,
        account_id: &str,
        model: &Model,
        success: bool,
        tokens: Option<UsageTokens>,
        host_record: &Value,
    ) -> Result<()> {
        self.record_kind(
            request_id,
            account_id,
            model,
            success,
            tokens,
            host_record,
            "inference",
        )
    }
    pub fn record_kind(
        &mut self,
        request_id: &str,
        account_id: &str,
        model: &Model,
        success: bool,
        tokens: Option<UsageTokens>,
        host_record: &Value,
        kind: &str,
    ) -> Result<()> {
        if self.ledger.iter().any(|e| e.request_id == request_id) {
            return if self.health_error.is_some() {
                self.persist()
            } else {
                Ok(())
            };
        }
        let tokens = if success { tokens } else { None };
        let credits = tokens
            .as_ref()
            .map(|t| {
                t.input
                    .saturating_sub(t.cached)
                    .saturating_sub(t.cache_write) as f64
                    * model.input_multiplier
                    + t.cached as f64 * model.cache_multiplier
                    + t.cache_write as f64 * model.cache_creation_multiplier
                    + t.output as f64 * model.output_multiplier
            })
            .unwrap_or(0.0);
        let cost = tokens.as_ref().and_then(|t| {
            let uncached = t
                .input
                .saturating_sub(t.cached)
                .saturating_sub(t.cache_write);
            let tier = service_tier_for_record(host_record, model);
            model
                .price()
                .cost(uncached, t.output, t.cached, t.cache_write, tier)
        });
        let at = host_record["RequestedAt"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| {
                d.with_timezone(&Utc)
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            })
            .unwrap_or_else(now);
        let status_code = if success {
            Some(200)
        } else {
            host_record["Failure"]["StatusCode"]
                .as_u64()
                .and_then(|v| u16::try_from(v).ok())
        };
        let pool = if ["standard", "core"].contains(&model.quota_pool.as_str())
            && model.quota_pool_source != "unknown"
        {
            model.quota_pool.clone()
        } else {
            "unknown".into()
        };
        self.ledger.push(LedgerEntry {
            request_id: request_id.into(),
            account_id: account_id.into(),
            model: model.id.clone(),
            at,
            success,
            tokens,
            estimated_credits: credits,
            estimated_cost_usd: cost,
            status_code,
            latency_ms: host_record["Latency"].as_u64().map(|ns| ns / 1_000_000),
            stream: host_record["Stream"].as_bool().unwrap_or(false),
            pool,
            kind: kind.into(),
            pool_source: model.quota_pool_source.clone(),
        });
        // Preserve a full year; totals explicitly describe this coverage.
        let cutoff =
            (Utc::now() - Duration::days(400)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let previous_len = self.ledger.len();
        self.ledger.retain(|e| e.at >= cutoff);
        if self.ledger.len() != previous_len {
            self.ledger_epoch = Uuid::new_v4().to_string();
            self.ledger_retained_since = Some(cutoff);
        }
        self.persist()
    }
    pub fn usage(&self, account: Option<&str>) -> Value {
        self.usage_kind(account, None)
    }
    pub fn usage_kind(&self, account: Option<&str>, kind: Option<&str>) -> Value {
        let (start, end) = self.settings.window().unwrap_or_default();
        let mut requests = 0u64;
        let mut failed = 0u64;
        let mut measured = 0u64;
        let mut input = 0u64;
        let mut output = 0u64;
        let mut cached = 0u64;
        let mut cache_write = 0u64;
        let mut reasoning = 0u64;
        let mut credits = 0.0;
        let mut cost = 0.0;
        let mut all_priced = true;
        let mut models: BTreeMap<String, Value> = BTreeMap::new();
        let scope = kind.filter(|k| *k != "all");
        let mut validation_requests = 0u64;
        let mut failed_validation_requests = 0u64;
        let mut inference_requests = 0u64;
        for e in self.ledger.iter().filter(|e| {
            e.at >= start
                && e.at < end
                && account.is_none_or(|a| a == e.account_id)
                && scope.is_none_or(|k| e.kind == k)
        }) {
            if e.kind == "validation" {
                if e.success {
                    validation_requests += 1;
                } else {
                    failed_validation_requests += 1;
                }
            } else if e.success {
                inference_requests += 1;
            }
            if !e.success {
                failed += 1;
                continue;
            }
            requests += 1;
            let row = models.entry(e.model.clone()).or_insert_with(|| json!({"model":e.model,"requests":0,"input_tokens":0,"output_tokens":0,"cached_input_tokens":0,"estimated_credits":0.0,"estimated_cost_usd":null}));
            row["requests"] = json!(row["requests"].as_u64().unwrap_or(0) + 1);
            if let Some(t) = &e.tokens {
                measured += 1;
                input = input.saturating_add(t.input);
                output = output.saturating_add(t.output);
                cached = cached.saturating_add(t.cached);
                cache_write = cache_write.saturating_add(t.cache_write);
                reasoning = reasoning.saturating_add(t.reasoning);
                credits += e.estimated_credits;
                row["input_tokens"] = json!(row["input_tokens"]
                    .as_u64()
                    .unwrap_or(0)
                    .saturating_add(t.input));
                row["output_tokens"] = json!(row["output_tokens"]
                    .as_u64()
                    .unwrap_or(0)
                    .saturating_add(t.output));
                row["cached_input_tokens"] = json!(row["cached_input_tokens"]
                    .as_u64()
                    .unwrap_or(0)
                    .saturating_add(t.cached));
                row["estimated_credits"] =
                    json!(row["estimated_credits"].as_f64().unwrap_or(0.0) + e.estimated_credits);
                if let Some(c) = e.estimated_cost_usd {
                    cost += c;
                    row["estimated_cost_usd"] =
                        json!(row["estimated_cost_usd"].as_f64().unwrap_or(0.0) + c);
                } else {
                    all_priced = false;
                    row["cost_incomplete"] = json!(true);
                }
            } else {
                all_priced = false;
                row["cost_incomplete"] = json!(true);
            }
        }
        for row in models.values_mut() {
            if row["cost_incomplete"] == true {
                row["estimated_cost_usd"] = Value::Null;
            }
            row.as_object_mut().unwrap().remove("cost_incomplete");
        }
        let budget = if account.is_none() {
            self.settings.budget_credits
        } else {
            None
        };
        let budget_entries: Vec<_> = self
            .ledger
            .iter()
            .filter(|e| e.success && e.at >= start && e.at < end)
            .collect();
        let budget_complete = budget_entries.iter().all(|e| e.tokens.is_some());
        let budget_consumed: f64 = budget_entries.iter().map(|e| e.estimated_credits).sum();
        let mut entries: Vec<_> = self
            .ledger
            .iter()
            .filter(|e| {
                e.at >= start
                    && e.at < end
                    && account.is_none_or(|a| a == e.account_id)
                    && scope.is_none_or(|k| e.kind == k)
            })
            .collect();
        entries.sort_by(|a, b| a.at.cmp(&b.at));
        let recent:Vec<_>=entries.iter().rev().take(60).rev().map(|e|json!({
            "request_id":e.request_id,"at":e.at,"model":e.model,"success":e.success,"kind":e.kind,
            "status":if e.success{"success"}else{"failure"},"status_code":e.status_code,
            "latency_ms":e.latency_ms,"stream":e.stream,"input_tokens":e.tokens.as_ref().map(|t|t.input),
            "output_tokens":e.tokens.as_ref().map(|t|t.output),"cached_input_tokens":e.tokens.as_ref().map(|t|t.cached),
            "cache_write_tokens":e.tokens.as_ref().map(|t|t.cache_write),
            "estimated_credits":e.estimated_credits,"estimated_cost_usd":e.estimated_cost_usd})).collect();
        let recent_success = recent.iter().filter(|r| r["success"] == true).count();
        let recent_failure = recent.len() - recent_success;
        json!({"requests":requests,"failed_requests":failed,"requests_with_usage":measured,
            "successful_requests":requests,"total_requests":requests+failed,
            "success_rate":if requests+failed>0{Some(requests as f64/(requests+failed)as f64)}else{None},
            "input_tokens":input,"output_tokens":output,"cached_input_tokens":cached,"cache_write_tokens":cache_write,"reasoning_tokens":reasoning,
            "estimated_credits":credits,"estimated_cost_usd":if all_priced && measured>0 {Some(cost)} else {None},
            "remaining_credits":budget.filter(|_|budget_complete).map(|b|(b-budget_consumed).max(0.0)),"budget_credits":budget,
            "budget_consumed_credits":if budget.is_some()&&budget_complete{Some(budget_consumed)}else{None},"credits_complete":requests==measured,
            "coverage":"仅本插件观测到的成功请求；缺少 usage 的请求不估算 token；保留最近 400 天",
            "estimated":true,"unit":"加权 token","window_start":start,"window_end":end,
            "budget_scope":"all_accounts","observed_from":entries.first().map(|e|&e.at),
            "recent":{"requests":recent,"success_count":recent_success,"failure_count":recent_failure},
            "kind_scope":scope.unwrap_or("all"),"validation_requests":validation_requests,"failed_validation_requests":failed_validation_requests,"inference_requests":inference_requests,
            "accounts":account.map(|a|vec![a]),"models":models.values().collect::<Vec<_>>()})
    }
    fn period_records(&self) -> Vec<crate::quota_window_usage::PeriodRecord<'_>> {
        use crate::quota_window_usage::{PeriodRecord, RecordTokens};
        self.ledger
            .iter()
            .map(|e| PeriodRecord {
                account_id: &e.account_id,
                pool: if ["explicit_config", "factory_registry"].contains(&e.pool_source.as_str()) {
                    &e.pool
                } else {
                    "unknown"
                },
                at: &e.at,
                success: e.success,
                kind: &e.kind,
                estimated_cost_usd: e.estimated_cost_usd,
                tokens: e.tokens.as_ref().map(|t| RecordTokens {
                    input: t.input,
                    output: t.output,
                    cached: t.cached,
                    cache_write: t.cache_write,
                    reasoning: t.reasoning,
                }),
            })
            .collect()
    }
    pub fn period_stats(&self, account: &str, window: &Value) -> Value {
        crate::quota_window_usage::aggregate_window_usage(
            &self.period_records(),
            account,
            window,
            &crate::quota_window_usage::CoverageEvidence {
                observed_since: self.observation_started_at.clone(),
                retained_since: self.ledger_retained_since.clone(),
            },
            &now(),
        )
    }
    pub fn sample_quota(&mut self, account: &str, limits: &Value) {
        let at = now();
        for w in limits["windows"].as_array().into_iter().flatten() {
            let (Some(reset), Some(used), Some(group), Some(name)) = (
                w["reset_at"].as_str(),
                w["used_percent"].as_f64(),
                w["group"].as_str(),
                w["name"].as_str(),
            ) else {
                continue;
            };
            if reset <= at.as_str() {
                continue;
            }
            let bounds = crate::quota_window_usage::resolve_window_bounds(w, &at);
            let (Some(start), Some(end), Some(key)) = (
                bounds.window_start,
                bounds.window_end,
                w["window_key"].as_str(),
            ) else {
                continue;
            };
            if bounds.status != "available" {
                continue;
            }
            let Some(totals) = crate::quota_window_usage::cumulative_pool_usage(
                &self.period_records(),
                account,
                group,
                &at,
            ) else {
                continue;
            };
            let tokens = totals.tokens;
            let cost = (totals.unpriced_requests == 0).then_some(totals.known_cost_usd);
            let requests = totals.requests;
            let measured = totals.measured;
            self.quota_samples.push(QuotaSample {
                account_id: account.into(),
                group: group.into(),
                name: name.into(),
                reset_at: end,
                at: at.clone(),
                used_percent: used,
                tokens,
                cost,
                requests,
                measured,
                sample_version: crate::quota_window_usage::SAMPLE_VERSION,
                ledger_epoch: self.ledger_epoch.clone(),
                window_key: key.into(),
                window_start: start,
                totals,
            });
        }
        let cutoff =
            (Utc::now() - Duration::days(40)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        self.quota_samples.retain(|s| s.at >= cutoff);
        if self.quota_samples.len() > 12000 {
            self.quota_samples.drain(..self.quota_samples.len() - 12000);
        }
    }
    pub fn prediction(&self, account: &str, limits: &Value) -> Value {
        let mut windows = Vec::new();
        let mut estimated = false;
        for w in limits["windows"].as_array().into_iter().flatten() {
            let name = w["name"].as_str().unwrap_or("");
            let group = w["group"].as_str().unwrap_or("");
            let mut row = json!({"name":name,"group":group,"window_key":w["window_key"],"window_end":w["reset_at"],"remaining_fraction":w["remaining_fraction"],
                "status":"unknown","estimated_total_tokens":null,"estimated_remaining_tokens":null,"estimated_total_mtokens":null,"estimated_remaining_mtokens":null,
                "estimated_total_cost_usd":null,"estimated_remaining_cost_usd":null,"period_stats":self.period_stats(account,w),
                "message":"需要同一官方周期两次有效快照、至少0.5%额度变化及完整的本插件分类用量样本"});
            let samples: Vec<_> = self
                .quota_samples
                .iter()
                .filter(|s| {
                    s.account_id == account
                        && s.group == group
                        && s.name == name
                        && Some(s.reset_at.as_str()) == w["reset_at"].as_str()
                        && s.sample_version == crate::quota_window_usage::SAMPLE_VERSION
                        && s.ledger_epoch == self.ledger_epoch
                })
                .collect();
            if let Some(current) = samples.last() {
                let point = |sample: &QuotaSample| crate::quota_window_usage::QuotaPoint {
                    version: sample.sample_version,
                    ledger_epoch: sample.ledger_epoch.clone(),
                    account_id: sample.account_id.clone(),
                    pool: sample.group.clone(),
                    window_key: sample.window_key.clone(),
                    window_start: sample.window_start.clone(),
                    window_end: sample.reset_at.clone(),
                    at: sample.at.clone(),
                    used_percent: sample.used_percent,
                    totals: sample.totals.clone(),
                };
                for base in samples.iter().filter(|s| s.at < current.at) {
                    let estimate = crate::quota_window_usage::estimate_sample_pair(
                        &point(base),
                        &point(current),
                        w["remaining_fraction"].as_f64(),
                        &now(),
                    );
                    row.as_object_mut()
                        .unwrap()
                        .extend(estimate.as_object().unwrap().clone());
                    if estimate["status"] == "estimated" {
                        estimated = true;
                        break;
                    }
                }
            }
            windows.push(row);
        }
        json!({"status":if estimated{"estimated"}else{"unknown"},"scope":"同账号、同官方额度池及同一滚动窗口；仅本插件样本",
            "unit":"原始输入+输出 token（缓存已包含于输入）","estimated":true,"windows":windows,
            "message":"这是样本推算，不是官方 token 余额或官方美元余额；FSC 消耗不参与推算"})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_legacy_prices(first: Option<f64>, second: Option<f64>) -> Store {
        let mut store = Store::default();
        let model_id = "factory/test".to_string();
        store.settings.models = vec![Model {
            id: model_id.clone(),
            ..Model::default()
        }];
        for (id, input) in [("first", first), ("second", second)] {
            store.accounts.insert(
                id.into(),
                Account {
                    id: id.into(),
                    model_overrides: BTreeMap::from([(
                        model_id.clone(),
                        ModelOverride {
                            input_price_usd_per_million: input,
                            output_price_usd_per_million: Some(50.0),
                            ..ModelOverride::default()
                        },
                    )]),
                    ..Account::default()
                },
            );
        }
        store
    }

    #[test]
    fn unique_legacy_price_is_promoted_once_without_data_loss() {
        let mut store = store_with_legacy_prices(Some(10.0), Some(10.0));
        store.migrate_legacy_account_prices();
        assert_eq!(
            store.settings.models[0].input_price_usd_per_million,
            Some(10.0)
        );
        assert!(store
            .accounts
            .values()
            .all(|account| account.model_overrides["factory/test"]
                .input_price_usd_per_million
                .is_some()));
        store.settings.models[0].input_price_usd_per_million = None;
        store.migrate_legacy_account_prices();
        assert_eq!(store.settings.models[0].input_price_usd_per_million, None);
    }

    #[test]
    fn conflicting_legacy_prices_remain_preserved_without_promotion() {
        let mut store = store_with_legacy_prices(Some(10.0), Some(20.0));
        store.migrate_legacy_account_prices();
        assert_eq!(store.settings.models[0].input_price_usd_per_million, None);
        assert_eq!(
            store.accounts["first"].model_overrides["factory/test"].input_price_usd_per_million,
            Some(10.0)
        );
        assert_eq!(
            store.accounts["second"].model_overrides["factory/test"].input_price_usd_per_million,
            Some(20.0)
        );
    }

    #[test]
    fn actual_service_tier_precedes_route_fallback() {
        let model = Model {
            protocol: "responses".into(),
            upstream_provider: "openai".into(),
            upstream_id: "gpt-5.4-fast".into(),
            ..Model::default()
        };
        assert_eq!(
            service_tier_for_record(
                &json!({"ResponseServiceTier":"standard","ServiceTier":"flex"}),
                &model
            ),
            Some("standard")
        );
        assert_eq!(
            service_tier_for_record(
                &json!({"ResponseServiceTier":"","ServiceTier":"flex"}),
                &model
            ),
            Some("flex")
        );
        assert_eq!(
            service_tier_for_record(&json!({}), &model),
            Some("priority")
        );
    }
}
