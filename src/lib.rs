mod access;
mod catalog;
mod discovery;
mod factory;
mod guard;
pub mod host;
mod management;
mod oauth;
mod pricing;
mod protocol;
mod quota_window_usage;
mod state;
mod wire;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Condvar, Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use host::{decode, encode, Host};
use state::{Model, Store};

pub const PROVIDER: &str = "factory-droid";
pub type Result<T> = std::result::Result<T, Fault>;
#[derive(Clone, Debug, Serialize)]
pub struct Fault {
    pub code: String,
    pub message: String,
    pub http_status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<u64>,
}
impl Fault {
    pub fn new(status: u16, code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            http_status: status,
            retry_after_seconds: None,
        }
    }
    pub fn input(message: &str) -> Self {
        Self::new(400, "invalid_input", message)
    }
    pub fn storage(message: &str) -> Self {
        Self::new(503, "storage_error", message)
    }
    pub fn internal() -> Self {
        Self::new(500, "internal_error", "插件内部操作失败")
    }
    pub fn json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(
            json!({"code":"internal_error","message":"插件内部操作失败","http_status":500}),
        )
    }
}

#[derive(Clone, Deserialize, Default)]
#[serde(default)]
struct Config {
    #[serde(rename = "data-dir")]
    data_dir: Option<PathBuf>,
    #[serde(rename = "api-base-url")]
    api_base_url: Option<String>,
    #[serde(rename = "allow-local-mock")]
    allow_local_mock: bool,
    #[serde(rename = "factory-client-version")]
    factory_client_version: Option<String>,
    #[serde(rename = "droid-cli-path")]
    droid_cli_path: Option<PathBuf>,
    #[serde(rename = "discovery-timeout-seconds")]
    discovery_timeout_seconds: Option<u64>,
}

pub struct Runtime {
    pub store: Arc<Mutex<Store>>,
    pub host: Host,
    config: Config,
    active: AtomicUsize,
    closing: Arc<AtomicBool>,
    operations: Mutex<HashMap<String, String>>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    downstreams: Mutex<HashSet<String>>,
    approvals: Mutex<guard::Approvals>,
    catalog_observed: Mutex<HashMap<String, management::CatalogObservation>>,
    catalog_dirty: AtomicBool,
    catalog_error: Mutex<Option<String>>,
    management_lock: Mutex<()>,
    drafts: Mutex<BTreeMap<String, access::Draft>>,
    discovery_cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    lifecycle_lock: Mutex<()>,
    stop_lock: Mutex<()>,
    oauth_sessions: oauth::Sessions,
    oauth_rotations: oauth::Rotations,
    wire_ids: wire::Identifiers,
    active_wait: (Mutex<()>, Condvar),
}
static RUNTIME: OnceLock<Mutex<Option<Arc<Runtime>>>> = OnceLock::new();
fn runtime() -> Result<Arc<Runtime>> {
    RUNTIME
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| Fault::internal())?
        .clone()
        .ok_or_else(|| Fault::new(503, "not_initialized", "插件尚未完成配置"))
}
impl Runtime {
    fn open(config: Config) -> Result<Arc<Self>> {
        let dir = config
            .data_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("data").join(PROVIDER));
        let api = config
            .api_base_url
            .as_deref()
            .unwrap_or("https://api.factory.ai");
        factory::validate_base(api, config.allow_local_mock)?;
        if config
            .droid_cli_path
            .as_ref()
            .is_some_and(|p| !p.is_absolute())
        {
            return Err(Fault::input(
                "droid-cli-path 必须为官方 Droid 原生可执行文件的绝对路径",
            ));
        }
        if config
            .discovery_timeout_seconds
            .is_some_and(|s| !(10..=120).contains(&s))
        {
            return Err(Fault::input("模型发现超时必须为10至120秒"));
        }
        if config.factory_client_version.as_ref().is_some_and(|s| {
            s.len() > 32
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_digit() || b == b'.' || b == b'-')
        }) {
            return Err(Fault::input("Factory 客户端版本无效"));
        }
        let store = Store::open(dir)?;
        {
            let mut state = store.lock().map_err(|_| Fault::internal())?;
            let fingerprint =
                access::fingerprint(&json!({"api_base":api,"local_mock":config.allow_local_mock}));
            if state.network_fingerprint != fingerprint {
                state.network_fingerprint = fingerprint;
                for account in state.accounts.values_mut() {
                    account.identity_verified = false;
                    account.discovery.stale = true;
                    account.discovery.generation = account.discovery.generation.saturating_add(1);
                    for entry in account.model_access.values_mut() {
                        entry.allowed = false;
                        entry.verification_status = "invalidated".into();
                        entry.last_error = Some("route_origin_changed".into());
                        entry.operation_generation = entry.operation_generation.saturating_add(1);
                    }
                }
                state.persist()?;
            }
            let catalogs: Vec<_> = state
                .accounts
                .values()
                .filter(|account| !account.deleted)
                .map(|account| account.discovery.candidates.clone())
                .collect();
            for candidates in catalogs {
                access::register_discovered_models(&mut state, &candidates)?;
            }
        }
        let wire_ids =
            wire::Identifiers::new(&store.lock().map_err(|_| Fault::internal())?.wire_namespace)?;
        Ok(Arc::new(Self {
            store,
            host: Host::get()?,
            config,
            active: AtomicUsize::new(0),
            closing: Arc::new(AtomicBool::new(false)),
            operations: Mutex::new(HashMap::new()),
            workers: Mutex::new(Vec::new()),
            downstreams: Mutex::new(HashSet::new()),
            approvals: Mutex::new(guard::Approvals::default()),
            catalog_observed: Mutex::new(HashMap::new()),
            catalog_dirty: AtomicBool::new(false),
            catalog_error: Mutex::new(None),
            management_lock: Mutex::new(()),
            drafts: Mutex::new(BTreeMap::new()),
            discovery_cancels: Mutex::new(HashMap::new()),
            lifecycle_lock: Mutex::new(()),
            stop_lock: Mutex::new(()),
            oauth_sessions: Mutex::new(HashMap::new()),
            oauth_rotations: Mutex::new(HashMap::new()),
            wire_ids,
            active_wait: (Mutex::new(()), Condvar::new()),
        }))
    }
    pub fn lock(&self) -> Result<std::sync::MutexGuard<'_, Store>> {
        self.store.lock().map_err(|_| Fault::internal())
    }
    pub fn api_base(&self) -> &str {
        self.config
            .api_base_url
            .as_deref()
            .unwrap_or("https://api.factory.ai")
            .trim_end_matches('/')
    }
    pub fn client_version(&self) -> &str {
        self.config
            .factory_client_version
            .as_deref()
            .unwrap_or(wire::CLIENT_VERSION)
    }
    pub fn model(&self, id: &str) -> Result<Model> {
        let store = self.lock()?;
        if let Some(model) = store
            .settings
            .models
            .iter()
            .find(|m| m.enabled && m.id == id)
        {
            return Ok(model.clone());
        }
        if let Some((account, suffix)) = id.strip_prefix("factory/").and_then(|s| s.split_once('/'))
        {
            if store.accounts.contains_key(account) {
                let _ = suffix;
                if let Some(mut model) = store.account_model(account, id).filter(|m| m.enabled) {
                    model.id = id.into();
                    return Ok(model);
                }
            }
        }
        Err(Fault::new(
            404,
            "model_not_found",
            "Factory 模型未启用或映射不存在",
        ))
    }
    pub fn operation(self: &Arc<Self>, callback: &str, timeout: u64) -> Result<Operation> {
        let _lifecycle = self.lifecycle_lock.lock().map_err(|_| Fault::internal())?;
        if self.closing.load(Ordering::Acquire) {
            return Err(Fault::new(503, "plugin_stopping", "插件正在停止"));
        }
        let response = self.host.call(
            "host.http.operation_open",
            &json!({"host_callback_id":callback}),
        )?;
        let id = response["operation_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(Fault::internal)?
            .to_string();
        self.operations
            .lock()
            .map_err(|_| Fault::internal())?
            .insert(id.clone(), callback.into());
        let finished = Arc::new((Mutex::new(false), Condvar::new()));
        let waiter = finished.clone();
        let host = self.host;
        let op_id = id.clone();
        let cb = callback.to_string();
        let downstream = Arc::new(Mutex::new(None::<String>));
        let timed_downstream = downstream.clone();
        let timer = thread::spawn(move || {
            let (mutex, cv) = &*waiter;
            if let Ok(done) = mutex.lock() {
                if let Ok((done, elapsed)) =
                    cv.wait_timeout_while(done, Duration::from_secs(timeout), |done| !*done)
                {
                    if !*done && elapsed.timed_out() {
                        let _ = host.call(
                            "host.http.cancel",
                            &json!({"operation_id":op_id,"host_callback_id":cb}),
                        );
                        if let Ok(bound) = timed_downstream.lock() {
                            if let Some(stream) = bound.as_ref() {
                                let _=host.call("host.stream.close",&json!({"stream_id":stream,
                                    "error_detail":Fault::new(504,"request_timeout","Factory 请求超过当前请求的超时限制").json()}));
                            }
                        }
                    }
                }
            }
        });
        Ok(Operation {
            runtime: self.clone(),
            id,
            callback: callback.into(),
            finished,
            timer: Some(timer),
            downstream,
        })
    }
    pub fn begin(self: &Arc<Self>) -> Result<Active> {
        let _lifecycle = self.lifecycle_lock.lock().map_err(|_| Fault::internal())?;
        if self.closing.load(Ordering::Acquire) {
            return Err(Fault::new(503, "plugin_stopping", "插件正在停止"));
        }
        self.active.fetch_add(1, Ordering::AcqRel);
        Ok(Active(self.clone()))
    }
    fn spawn(&self, task: impl FnOnce() + Send + 'static) -> Result<()> {
        let mut workers = self.workers.lock().map_err(|_| Fault::internal())?;
        if self.closing.load(Ordering::Acquire) {
            return Err(Fault::new(503, "plugin_stopping", "插件正在停止"));
        }
        let mut i = 0;
        while i < workers.len() {
            if workers[i].is_finished() {
                let _ = workers.swap_remove(i).join();
            } else {
                i += 1;
            }
        }
        workers.push(thread::spawn(task));
        Ok(())
    }
    fn stop(&self) -> Result<()> {
        let _stop = self.stop_lock.lock().map_err(|_| Fault::internal())?;
        if let Ok(mut approvals) = self.approvals.lock() {
            approvals.clear();
        }
        if let Ok(mut drafts) = self.drafts.lock() {
            drafts.clear();
        }
        let operations = {
            let _lifecycle = self.lifecycle_lock.lock().map_err(|_| Fault::internal())?;
            self.closing.store(true, Ordering::Release);
            self.operations
                .lock()
                .map(|m| m.clone())
                .unwrap_or_default()
        };
        if let Ok(mut cancels) = self.discovery_cancels.lock() {
            for token in cancels.values() {
                token.store(true, Ordering::Release);
            }
            cancels.clear();
        }
        for (id, callback) in operations {
            let _ = self.host.call(
                "host.http.cancel",
                &json!({"operation_id":id,"host_callback_id":callback}),
            );
        }
        let downstreams = self
            .downstreams
            .lock()
            .map(|s| s.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        for id in downstreams {
            let _=self.host.call("host.stream.close",&json!({"stream_id":id,"error_detail":Fault::new(503,"plugin_stopping","插件正在停止").json()}));
        }
        let workers = self
            .workers
            .lock()
            .map(|mut w| std::mem::take(&mut *w))
            .unwrap_or_default();
        for worker in workers {
            let _ = worker.join();
        }
        let (mutex, cv) = &self.active_wait;
        let guard = mutex.lock().map_err(|_| Fault::internal())?;
        let (drain_guard, wait) = cv
            .wait_timeout_while(guard, Duration::from_secs(5), |_| {
                self.active.load(Ordering::Acquire) > 0
            })
            .map_err(|_| Fault::internal())?;
        if wait.timed_out() && self.active.load(Ordering::Acquire) > 0 {
            return Err(Fault::new(
                503,
                "plugin_draining",
                "插件仍在结束旧请求，请稍后重新启用",
            ));
        }
        drop(drain_guard);
        self.oauth_sessions
            .lock()
            .map_err(|_| Fault::internal())?
            .clear();
        self.oauth_rotations
            .lock()
            .map_err(|_| Fault::internal())?
            .clear();
        let workers = self
            .workers
            .lock()
            .map(|mut w| std::mem::take(&mut *w))
            .unwrap_or_default();
        for worker in workers {
            let _ = worker.join();
        }
        Ok(())
    }
    fn resume(&self) -> Result<()> {
        let _stop = self.stop_lock.lock().map_err(|_| Fault::internal())?;
        let _lifecycle = self.lifecycle_lock.lock().map_err(|_| Fault::internal())?;
        if !self.closing.load(Ordering::Acquire) {
            return Ok(());
        }
        if self.active.load(Ordering::Acquire) > 0
            || !self
                .operations
                .lock()
                .map_err(|_| Fault::internal())?
                .is_empty()
        {
            return Err(Fault::new(
                503,
                "plugin_draining",
                "旧请求尚未结束，暂不能恢复插件",
            ));
        }
        self.closing.store(false, Ordering::Release);
        Ok(())
    }
}
pub struct Active(Arc<Runtime>);
impl Drop for Active {
    fn drop(&mut self) {
        let (mutex, cv) = &self.0.active_wait;
        let _guard = mutex.lock();
        self.0.active.fetch_sub(1, Ordering::AcqRel);
        cv.notify_all();
    }
}
pub struct Operation {
    runtime: Arc<Runtime>,
    pub id: String,
    pub callback: String,
    finished: Arc<(Mutex<bool>, Condvar)>,
    timer: Option<JoinHandle<()>>,
    downstream: Arc<Mutex<Option<String>>>,
}
impl Operation {
    fn bind_downstream(&self, id: &str) -> Result<()> {
        *self.downstream.lock().map_err(|_| Fault::internal())? = Some(id.into());
        Ok(())
    }
}
impl Drop for Operation {
    fn drop(&mut self) {
        let (mutex, cv) = &*self.finished;
        if let Ok(mut done) = mutex.lock() {
            *done = true;
            cv.notify_all();
        }
        if let Some(timer) = self.timer.take() {
            let _ = timer.join();
        }
        let _ = self.runtime.host.call(
            "host.http.cancel",
            &json!({"operation_id":self.id,"host_callback_id":self.callback}),
        );
        if let Ok(mut ops) = self.runtime.operations.lock() {
            ops.remove(&self.id);
        }
    }
}

fn register() -> Value {
    json!({"schema_version":10,"metadata":{"Name":"Factory Droid","Version":env!("CARGO_PKG_VERSION"),
        "Author":"junxin367","GitHubRepository":env!("CARGO_PKG_REPOSITORY"),
        "ConfigFields":[
            {"Name":"data-dir","Type":"string","Description":"脱敏状态与模型目录缓存，默认 data/factory-droid"},
            {"Name":"factory-client-version","Type":"string","Description":"Factory CLI 协议版本，默认 1.2.0；升级前核对官方请求"},
            {"Name":"droid-cli-path","Type":"string","Description":"自动同步官方目录的 Droid 原生可执行文件绝对路径；不配置时使用内置目录"},
            {"Name":"discovery-timeout-seconds","Type":"number","Description":"候选发现超时，默认30秒，范围10至120"},
            {"Name":"api-base-url","Type":"string","Description":"Factory API 地址；本地模拟需同时启用 allow-local-mock"},
            {"Name":"allow-local-mock","Type":"boolean","Description":"仅允许显式 localhost/127.0.0.1/[::1] 模拟服务"}]},
        "capabilities":{"auth_provider":true,"model_provider":true,"executor":true,"executor_model_scope":"oauth",
        "executor_input_formats":["openai","claude","openai-response"],"executor_output_formats":["openai","claude","openai-response"],"quota_provider":true,
        "request_interceptor":true,"request_interceptor_omit_body_before":true,"request_interceptor_omit_body_after":true,
        "usage_plugin":false,"management_api":true}})
}

pub fn shutdown() {
    let current = RUNTIME
        .get()
        .and_then(|m| m.lock().ok().and_then(|mut v| v.take()));
    if let Some(current) = current {
        let _ = current.stop();
    }
}
pub fn dispatch(method: &str, data: &Value) -> Result<Value> {
    match method {
        "plugin.register" | "plugin.reconfigure" => {
            let bytes = decode(&data["config_yaml"])?;
            let config: Config = if bytes.is_empty() {
                Config::default()
            } else {
                serde_yaml::from_slice(&bytes).map_err(|_| Fault::input("插件 YAML 配置无效"))?
            };
            let slot = RUNTIME.get_or_init(|| Mutex::new(None));
            let existing = slot.lock().map_err(|_| Fault::internal())?.clone();
            if let Some(old) = existing {
                let old_dir = old
                    .config
                    .data_dir
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("data").join(PROVIDER));
                let new_dir = config
                    .data_dir
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("data").join(PROVIDER));
                if old_dir != new_dir
                    || old.config.api_base_url != config.api_base_url
                    || old.config.allow_local_mock != config.allow_local_mock
                    || old.config.factory_client_version != config.factory_client_version
                    || old.config.droid_cli_path != config.droid_cli_path
                    || old.config.discovery_timeout_seconds != config.discovery_timeout_seconds
                {
                    return Err(Fault::input("修改数据目录或网络配置需要重启插件"));
                }
                old.resume()?;
            } else {
                let instance = Runtime::open(config)?;
                *slot.lock().map_err(|_| Fault::internal())? = Some(instance);
            }
            Ok(register())
        }
        "plugin.quiesce" => {
            if let Ok(rt) = runtime() {
                rt.stop()?;
            }
            Ok(json!({}))
        }
        "plugin.shutdown" => {
            shutdown();
            Ok(json!({}))
        }
        "auth.identifier" | "executor.identifier" | "quota.identifier" => {
            Ok(json!({"identifier":PROVIDER}))
        }
        "auth.parse" => factory::parse_auth(data),
        "auth.login.start" => oauth::start(&runtime()?, data),
        "auth.login.poll" => oauth::poll(&runtime()?, data),
        "auth.refresh" => factory::refresh_auth(&runtime()?, data),
        "model.static" => Ok(json!({"Provider":PROVIDER,"Models":[]})),
        "model.for_auth" => {
            let rt = runtime()?;
            let auth = factory::credential(data, "StorageJSON")?;
            let account = auth["account_id"]
                .as_str()
                .ok_or_else(|| Fault::input("账号 ID 缺失"))?;
            let file = data["FileName"]
                .as_str()
                .or_else(|| data["AuthID"].as_str())
                .unwrap_or("");
            let file = std::path::Path::new(file)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if !factory::observe_model_account(&rt, &auth, file)? {
                return Ok(json!({"Provider":PROVIDER,"Models":[]}));
            }
            let store = rt.lock()?;
            let models: Vec<_> = store
                .effective_models(account)
                .iter()
                .map(Model::host_json)
                .collect();
            drop(store);
            let offered = models
                .iter()
                .filter_map(|model| model["ID"].as_str().map(str::to_string))
                .collect();
            rt.catalog_observed
                .lock()
                .map_err(|_| Fault::internal())?
                .insert(
                    account.into(),
                    management::CatalogObservation::new(data, &auth, offered),
                );
            Ok(json!({"Provider":PROVIDER,"Models":models}))
        }
        "management.register" => Ok(management::registration(data)),
        "management.handle" => Ok(management::handle(&runtime()?, data)),
        "request.intercept_before" => Ok(json!({})),
        "request.intercept_after" => Ok(match runtime() {
            Ok(rt) => guard::after(&rt, data),
            Err(_) => json!({}),
        }),
        "executor.execute" => execute(&runtime()?, data, false).map_err(guard::http_fault),
        "executor.execute_stream" => execute(&runtime()?, data, true).map_err(guard::http_fault),
        "executor.count_tokens" => {
            let rt = runtime()?;
            let model = rt.model(data["Model"].as_str().unwrap_or(""))?;
            guard::validate(&rt, data, protocol::Format::for_model(&model)?)
                .map_err(guard::http_fault)?;
            Err(guard::http_fault(Fault::new(
                501,
                "count_tokens_unavailable",
                "Factory 未提供此模型的通用精确 token 计数接口",
            )))
        }
        "executor.http_request" => Err(guard::http_fault(Fault::new(
            400,
            "unsupported_operation",
            "请通过 CPA 模型接口调用 Factory",
        ))),
        // CPA publishes usage and its management server owns history and pricing.
        // Ignore any callback already queued by an older plugin instance.
        "usage.handle" => Ok(json!({})),
        "quota.describe" => Ok(
            json!({"supported_providers":[PROVIDER],"display_name":"Factory Droid","supports_reset":false}),
        ),
        "quota.fetch" => factory::quota(&runtime()?, data),
        "quota.reset" => Ok(json!({"success":false,"message":"Factory 官方额度不能由插件重置"})),
        _ => Err(Fault::new(400, "unknown_method", "不支持的宿主调用方法")),
    }
}

fn execute(rt: &Arc<Runtime>, data: &Value, stream: bool) -> Result<Value> {
    let active = rt.begin()?;
    let model = rt.model(data["Model"].as_str().unwrap_or(""))?;
    let format = protocol::Format::for_model(&model)?;
    let native_headers = guard::validate(rt, data, format)?;
    let cred = factory::credential(data, "StorageJSON")?;
    factory::ensure_enabled(rt, &cred)?;
    let payload = decode(&data["Payload"])?;
    let account_id = cred["account_id"]
        .as_str()
        .ok_or_else(|| Fault::input("账号 ID 缺失"))?
        .to_string();
    let mut selected = rt
        .lock()?
        .account_model(&account_id, &model.id)
        .ok_or_else(|| Fault::new(403, "model_not_allowed", "账号没有当前模型路由"))?;
    selected.id = model.id.clone();
    let model = selected;
    protocol::Format::for_model(&model)?.check_formats(data)?;
    let pinned = model.id.starts_with(&format!("factory/{account_id}/"));
    rt.lock()?.ensure_model_allowed(&account_id, &model)?;
    if model
        .id
        .strip_prefix("factory/")
        .and_then(|s| s.split_once('/'))
        .is_some_and(|(account, _)| {
            rt.lock()
                .ok()
                .is_some_and(|s| s.accounts.contains_key(account))
        })
        && !pinned
    {
        return Err(Fault::new(
            403,
            "account_model_mismatch",
            "此固定账号模型不属于当前凭据",
        ));
    }
    let request = protocol::request(&payload, &model, stream)?;
    let cb = data["host_callback_id"].as_str().unwrap_or("");
    let timeout = rt.lock()?.settings.request_timeout_seconds;
    let op = rt.operation(cb, timeout)?;
    if stream {
        let downstream = data["stream_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Fault::input("宿主流 ID 缺失"))?;
        op.bind_downstream(downstream)?;
    }
    let http = factory::llm_request(
        rt,
        &cred,
        &model,
        &request,
        &op,
        &native_headers,
        &data["Metadata"],
    )?;
    if !stream {
        let response = rt.host.call("host.http.do", &http)?;
        let code = response["StatusCode"].as_u64().unwrap_or(502) as u16;
        if !(200..300).contains(&code) {
            access::invalidate_upstream(rt, &cred, &model, code);
            return Err(factory::upstream_error(code, &response["Headers"]));
        }
        let body = decode(&response["Body"])?;
        protocol::validate_response(&body, format)?;
        return Ok(
            json!({"Payload":encode(&body),"Headers":{"Content-Type":["application/json"]}}),
        );
    }
    let response = rt.host.call("host.http.do_stream", &http)?;
    let code = response["status_code"].as_u64().unwrap_or(502) as u16;
    let upstream_stream = response["stream_id"].as_str().unwrap_or("").to_string();
    if !(200..300).contains(&code) {
        access::invalidate_upstream(rt, &cred, &model, code);
        if !upstream_stream.is_empty() {
            let _ = rt.host.call(
                "host.http.stream_close",
                &json!({"stream_id":upstream_stream}),
            );
        }
        return Err(factory::upstream_error(code, &response["headers"]));
    }
    if upstream_stream.is_empty() {
        return Err(Fault::new(
            502,
            "upstream_protocol_error",
            "Factory 流式响应缺少宿主流 ID",
        ));
    }
    let downstream = data["stream_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Fault::input("宿主流 ID 缺失"))?
        .to_string();
    let stream_key = downstream.clone();
    rt.downstreams
        .lock()
        .map_err(|_| Fault::internal())?
        .insert(stream_key.clone());
    let runtime = rt.clone();
    let spawned = rt.spawn(move || {
        let _active = active;
        let _operation = op;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            protocol::pump(&runtime, format, &upstream_stream, &downstream)
        }));
        let _ = runtime.host.call(
            "host.http.stream_close",
            &json!({"stream_id":upstream_stream}),
        );
        let finish = match result {
            Ok(Ok(())) => json!({"stream_id":downstream}),
            Ok(Err(e)) => json!({"stream_id":downstream,"error_detail":e.json()}),
            Err(_) => json!({"stream_id":downstream,"error_detail":Fault::internal().json()}),
        };
        let _ = runtime.host.call("host.stream.close", &finish);
        if let Ok(mut streams) = runtime.downstreams.lock() {
            streams.remove(&downstream);
        }
    });
    if spawned.is_err() {
        if let Ok(mut streams) = rt.downstreams.lock() {
            streams.remove(&stream_key);
        }
    }
    spawned?;
    Ok(json!({"headers":{"Content-Type":["text/event-stream"],"Cache-Control":["no-cache"]}}))
}
