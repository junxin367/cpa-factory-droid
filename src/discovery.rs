//! Sessionless Droid model discovery. A successful catalog is never an entitlement proof.
//! Only the caller's explicitly configured native executable is run.

use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use uuid::Uuid;

const MAX_STDOUT: usize = 4 * 1024 * 1024;
const MAX_STDERR: usize = 256 * 1024;
const MAX_FRAME: usize = 2 * 1024 * 1024;
const MAX_MODELS: usize = 1024;
const MAX_NOTIFICATIONS: usize = 64;
const POLL: Duration = Duration::from_millis(20);
const RESPONSE_GRACE: Duration = Duration::from_millis(80);

pub struct Options {
    pub executable: PathBuf,
    pub work_root: PathBuf,
    pub timeout: Duration,
    pub cancel: Option<Arc<AtomicBool>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    /// No reliable CLI version is present in list_models; do not substitute protocol version.
    pub cli_version: Option<String>,
    pub protocol_version: Option<String>,
    pub legacy_api_version: Option<String>,
    /// Raw new metadata is retained in memory. Management APIs must expose their own allowlist.
    pub models: Vec<Value>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Error {
    pub code: String,
    pub message: String,
}

impl Error {
    fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_owned(),
            message: message.to_owned(),
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

/// Returns catalog metadata only. Identity and native routes are checked separately;
/// model permissions are enforced by Factory when CPA executes a request.
pub fn discover(options: Options, api_key: &str) -> Result<Snapshot> {
    check_cancel(&options)?;
    if options.timeout.is_zero() || options.timeout > Duration::from_secs(120) {
        return Err(Error::new(
            "discovery_invalid_timeout",
            "模型发现超时须大于 0 且不超过 120 秒",
        ));
    }
    if !api_key.is_empty()
        && (api_key.len() < 4
            || api_key.len() > 8192
            || !api_key.starts_with("fk-")
            || api_key.chars().any(char::is_control))
    {
        return Err(Error::new(
            "discovery_invalid_key",
            "请提供有效格式的 Factory API Key",
        ));
    }
    let executable = validate_executable(&options.executable)?;
    let mut workspace = Workspace::create(&options.work_root)?;
    let result = run(&options, &executable, &workspace.path, api_key);
    match (result, workspace.cleanup()) {
        (Ok(snapshot), Ok(())) => {
            check_cancel(&options)?;
            Ok(snapshot)
        }
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(_)) => Err(Error::new(
            "discovery_cleanup_failed",
            "模型发现已结束，但隔离运行目录未能清理",
        )),
        (Err(mut error), Err(_)) => {
            error.message.push_str("；隔离运行目录未能清理");
            Err(error)
        }
    }
}

fn check_cancel(options: &Options) -> Result<()> {
    if options
        .cancel
        .as_ref()
        .is_some_and(|cancel| cancel.load(Ordering::Acquire))
    {
        Err(Error::new("discovery_cancelled", "模型发现已取消"))
    } else {
        Ok(())
    }
}

fn validate_executable(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(Error::new(
            "discovery_invalid_executable",
            "Droid 可执行文件须由管理员配置为绝对路径",
        ));
    }
    let path = fs::canonicalize(path).map_err(|_| {
        Error::new(
            "discovery_executable_missing",
            "配置的 Droid 可执行文件不可访问",
        )
    })?;
    if !path.is_file() {
        return Err(Error::new(
            "discovery_invalid_executable",
            "配置的 Droid 路径不是原生可执行文件",
        ));
    }
    let mut magic = [0u8; 4];
    File::open(&path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .map_err(|_| Error::new("discovery_invalid_executable", "无法读取 Droid 可执行文件"))?;
    #[cfg(windows)]
    let native = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
        && magic[..2] == *b"MZ";
    #[cfg(unix)]
    let native = {
        use std::os::unix::fs::PermissionsExt;
        let executable = fs::metadata(&path)
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
        executable
            && (magic == *b"\x7fELF"
                || [
                    [0xfe, 0xed, 0xfa, 0xce],
                    [0xce, 0xfa, 0xed, 0xfe],
                    [0xfe, 0xed, 0xfa, 0xcf],
                    [0xcf, 0xfa, 0xed, 0xfe],
                    [0xca, 0xfe, 0xba, 0xbe],
                    [0xbe, 0xba, 0xfe, 0xca],
                    [0xca, 0xfe, 0xba, 0xbf],
                    [0xbf, 0xba, 0xfe, 0xca],
                ]
                .contains(&magic))
    };
    #[cfg(not(any(windows, unix)))]
    let native = false;
    if !native {
        return Err(Error::new(
            "discovery_invalid_executable",
            "Droid 必须是本机原生可执行文件，不能是脚本或命令包装器",
        ));
    }
    Ok(path)
}

fn run(options: &Options, executable: &Path, run_root: &Path, api_key: &str) -> Result<Snapshot> {
    check_cancel(options)?;
    let home = run_root.join("home");
    let cwd = run_root.join("work");
    let temp = run_root.join("temp");
    for path in [&home, &cwd, &temp] {
        create_private_dir(path)?;
    }
    let mut command = Command::new(executable);
    command
        .args([
            "exec",
            "--input-format",
            "stream-jsonrpc",
            "--output-format",
            "stream-jsonrpc",
        ])
        .current_dir(&cwd)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // No inherited Factory, cloud, shell-hook, proxy or browser credentials.
    for name in ["SystemRoot", "WINDIR", "ComSpec", "PATH", "PATHEXT"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    // list_models is sessionless. OAuth tokens never enter the API-key variable or CLI storage.
    if !api_key.is_empty() {
        command.env("FACTORY_API_KEY", api_key);
    }
    command
        .env("FACTORY_HOME_OVERRIDE", &home)
        .env("FACTORY_DISABLE_KEYRING", "1")
        .env("FACTORY_DISABLE_SETTINGS_PERSISTENCE", "1")
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("APPDATA", home.join("AppData").join("Roaming"))
        .env("LOCALAPPDATA", home.join("AppData").join("Local"))
        .env("TEMP", &temp)
        .env("TMP", &temp)
        .env("TMPDIR", &temp)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("XDG_DATA_HOME", home.join(".local").join("share"))
        .env("CI", "1");

    let started = Instant::now();
    let mut process = ManagedProcess::spawn(&mut command)?;
    let mut stdin = process.child.stdin.take().ok_or_else(pipe_error)?;
    let stdout = process.child.stdout.take().ok_or_else(pipe_error)?;
    let stderr = process.child.stderr.take().ok_or_else(pipe_error)?;
    let (sender, receiver) = mpsc::channel();
    let stdout_reader = start_reader(stdout, MAX_STDOUT, true, sender.clone())?;
    let stderr_reader = match start_reader(stderr, MAX_STDERR, false, sender) {
        Ok(reader) => reader,
        Err(error) => {
            process.stop()?;
            let _ = stdout_reader.join();
            return Err(error);
        }
    };
    let request_id = Uuid::new_v4().to_string();
    let request = json!({
        "jsonrpc":"2.0",
        "factoryApiVersion":"1.0.0",
        "type":"request",
        "id":request_id,
        "method":"droid.list_models",
        "params":{"includeDisabled":true}
    });
    let sent = writeln!(stdin, "{request}").and_then(|()| stdin.flush());
    let result = if sent.is_err() {
        Err(Error::new(
            "discovery_stdin_failed",
            "无法向 Droid 发送模型发现请求",
        ))
    } else {
        receive(
            options,
            started,
            &request_id,
            api_key,
            &receiver,
            &mut process,
        )
    };
    drop(stdin);
    let stopped = process.stop();
    let stdout_join = stdout_reader.join();
    let stderr_join = stderr_reader.join();
    if stopped.is_err() || stdout_join.is_err() || stderr_join.is_err() {
        return Err(Error::new(
            "discovery_process_cleanup_failed",
            "Droid 发现进程或其输出读取任务未能正常回收",
        ));
    }
    // Parse anything already emitted before termination, including duplicate responses.
    let mut received = result?;
    while let Ok(event) = receiver.try_recv() {
        received.event(event, &request_id, api_key)?;
    }
    check_cancel(options)?;
    received.finish(&request_id, api_key)
}

fn pipe_error() -> Error {
    Error::new("discovery_pipe_failed", "无法建立 Droid 模型发现数据通道")
}

enum Event {
    Data(Vec<u8>),
    StdoutEof,
    StderrEof,
    ReadFailed,
    LimitExceeded,
}
fn start_reader(
    mut stream: impl Read + Send + 'static,
    limit: usize,
    stdout: bool,
    sender: Sender<Event>,
) -> Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("factory-model-discovery-io".to_owned())
        .spawn(move || {
            let mut buffer = [0u8; 8192];
            let mut total = 0usize;
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) => {
                        let _ = sender.send(if stdout {
                            Event::StdoutEof
                        } else {
                            Event::StderrEof
                        });
                        return;
                    }
                    Ok(count) => {
                        total += count;
                        if total > limit {
                            let _ = sender.send(Event::LimitExceeded);
                            return;
                        }
                        // stderr is consumed without retaining or exposing any of its content.
                        if stdout && sender.send(Event::Data(buffer[..count].to_vec())).is_err() {
                            return;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        let _ = sender.send(Event::ReadFailed);
                        return;
                    }
                }
            }
        })
        .map_err(|_| Error::new("discovery_reader_failed", "无法启动 Droid 输出读取任务"))
}

#[derive(Default)]
struct Received {
    pending: Vec<u8>,
    response: Option<Value>,
    response_at: Option<Instant>,
    notifications: usize,
    stdout_eof: bool,
}

fn receive(
    options: &Options,
    started: Instant,
    request_id: &str,
    api_key: &str,
    receiver: &Receiver<Event>,
    process: &mut ManagedProcess,
) -> Result<Received> {
    let mut received = Received::default();
    loop {
        check_cancel(options)?;
        if started.elapsed() >= options.timeout {
            return Err(Error::new("discovery_timeout", "Droid 模型发现超时"));
        }
        match receiver.recv_timeout(POLL.min(options.timeout.saturating_sub(started.elapsed()))) {
            Ok(event) => received.event(event, request_id, api_key)?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                received.stdout_eof = true;
            }
        }
        if let Some(status) = process
            .child
            .try_wait()
            .map_err(|_| Error::new("discovery_process_failed", "无法检查 Droid 进程状态"))?
        {
            if !status.success() {
                return Err(Error::new(
                    "discovery_cli_failed",
                    "Droid 模型发现进程异常退出",
                ));
            }
            // A successful exit may precede the reader delivering the last buffered frame.
            if received.stdout_eof {
                return Ok(received);
            }
        }
        if received
            .response_at
            .is_some_and(|at| at.elapsed() >= RESPONSE_GRACE)
        {
            return Ok(received);
        }
        if received.stdout_eof && received.response.is_none() {
            return Err(Error::new("discovery_no_response", "Droid 未返回模型目录"));
        }
    }
}

impl Received {
    fn event(&mut self, event: Event, request_id: &str, api_key: &str) -> Result<()> {
        match event {
            Event::Data(bytes) => {
                for byte in bytes {
                    if byte == b'\n' {
                        let line = std::mem::take(&mut self.pending);
                        self.line(&line, request_id, api_key)?;
                    } else {
                        if self.pending.len() >= MAX_FRAME {
                            return Err(output_limit());
                        }
                        self.pending.push(byte);
                    }
                }
            }
            Event::StdoutEof => self.stdout_eof = true,
            Event::StderrEof => {}
            Event::LimitExceeded => return Err(output_limit()),
            Event::ReadFailed => {
                return Err(Error::new("discovery_read_failed", "读取 Droid 输出失败"));
            }
        }
        Ok(())
    }
    fn line(&mut self, line: &[u8], request_id: &str, api_key: &str) -> Result<()> {
        let line = trim_ascii(line);
        if line.is_empty() {
            return Ok(());
        }
        if !api_key.is_empty()
            && line
                .windows(api_key.len())
                .any(|bytes| bytes == api_key.as_bytes())
        {
            return Err(Error::new(
                "discovery_sensitive_output",
                "Droid 输出包含敏感凭据，已拒绝接收",
            ));
        }
        let frame: Value = serde_json::from_slice(line).map_err(|_| {
            Error::new(
                "discovery_invalid_json",
                "Droid 输出包含无效 JSON 或非协议文本",
            )
        })?;
        if contains_secret(&frame, api_key) {
            return Err(Error::new(
                "discovery_sensitive_output",
                "Droid 输出包含敏感凭据，已拒绝接收",
            ));
        }
        if frame["jsonrpc"] != "2.0" {
            return Err(Error::new(
                "discovery_invalid_envelope",
                "Droid 返回了无效协议封包",
            ));
        }
        if frame["type"] == "notification" {
            self.notifications += 1;
            if self.notifications > MAX_NOTIFICATIONS {
                return Err(output_limit());
            }
            return Ok(());
        }
        if frame["type"] != "response" || frame["id"].as_str() != Some(request_id) {
            return Err(Error::new(
                "discovery_unexpected_response",
                "Droid 返回了未请求的协议消息或错误请求标识",
            ));
        }
        if self.response.is_some() {
            return Err(Error::new(
                "discovery_duplicate_response",
                "Droid 对同一发现请求返回了重复响应",
            ));
        }
        if frame.get("error").is_some() {
            // Upstream errors may contain credentials, file paths or request content.
            return Err(Error::new(
                "discovery_rpc_failed",
                "Droid 拒绝了模型发现请求",
            ));
        }
        self.response = Some(frame);
        self.response_at = Some(Instant::now());
        Ok(())
    }
    fn finish(mut self, request_id: &str, api_key: &str) -> Result<Snapshot> {
        if !trim_ascii(&self.pending).is_empty() {
            let trailing = std::mem::take(&mut self.pending);
            // Accept a complete final JSON frame even if the process omitted its newline.
            self.line(&trailing, request_id, api_key)?;
        }
        let response = self
            .response
            .ok_or_else(|| Error::new("discovery_no_response", "Droid 未返回模型目录"))?;
        let legacy_api_version = optional_version(&response, "factoryApiVersion")?;
        if legacy_api_version.as_deref() != Some("1.0.0") {
            return Err(Error::new(
                "discovery_incompatible_protocol",
                "Droid 模型发现协议版本不受支持",
            ));
        }
        let protocol_version = optional_version(&response, "factoryProtocolVersion")?;
        let models = response["result"]["models"]
            .as_array()
            .ok_or_else(|| Error::new("discovery_invalid_catalog", "Droid 未返回有效模型数组"))?;
        if models.is_empty() || models.len() > MAX_MODELS {
            return Err(Error::new(
                "discovery_invalid_catalog",
                "Droid 模型目录为空或超过数量限制",
            ));
        }
        let mut ids = HashSet::new();
        for model in models {
            validate_model(model)?;
            if !ids.insert(model["id"].as_str().unwrap_or_default()) {
                return Err(Error::new(
                    "discovery_duplicate_model",
                    "Droid 模型目录包含重复 ID",
                ));
            }
        }
        Ok(Snapshot {
            cli_version: None,
            protocol_version,
            legacy_api_version,
            models: models.clone(),
        })
    }
}

fn optional_version(value: &Value, key: &str) -> Result<Option<String>> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(version))
            if !version.is_empty()
                && version.len() <= 64
                && version
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b".-+".contains(&byte)) =>
        {
            Ok(Some(version.clone()))
        }
        _ => Err(Error::new(
            "discovery_invalid_envelope",
            "Droid 返回了无效版本字段",
        )),
    }
}
fn validate_model(model: &Value) -> Result<()> {
    let invalid = || Error::new("discovery_invalid_model", "Droid 模型目录包含无效条目");
    let object = model.as_object().ok_or_else(invalid)?;
    let id = model["id"].as_str().ok_or_else(invalid)?;
    for key in ["displayName", "modelProvider"] {
        if !model[key].as_str().is_some_and(|value| !value.is_empty()) {
            return Err(invalid());
        }
    }
    if id.is_empty()
        || id.len() > 256
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-:/".contains(&byte))
    {
        return Err(invalid());
    }
    for key in [
        "modelId",
        "displayName",
        "shortDisplayName",
        "modelProvider",
        "disabledReason",
        "deprecationDate",
        "kind",
    ] {
        if let Some(value) = object.get(key) {
            if !value.is_null()
                && !value.as_str().is_some_and(|text| {
                    !text.is_empty() && text.len() <= 4096 && !text.chars().any(char::is_control)
                })
            {
                return Err(invalid());
            }
        }
    }
    for value in [
        &model["maxInputTokens"],
        &model["maxOutputTokens"],
        &model["contextLimits"]["maxInputTokens"],
        &model["contextLimits"]["maxOutputTokens"],
    ] {
        if !value.is_null() && value.as_u64().is_none_or(|value| value == 0) {
            return Err(invalid());
        }
    }
    for key in [
        "disabled",
        "deprecated",
        "isCustom",
        "noImageSupport",
        "supportsPDFs",
        "supportsImageGeneration",
    ] {
        if object.get(key).is_some_and(|value| !value.is_boolean()) {
            return Err(invalid());
        }
    }
    if model["disabled"] == true
        && !model["disabledReason"]
            .as_str()
            .is_some_and(|reason| !reason.trim().is_empty())
    {
        return Err(Error::new(
            "discovery_invalid_model",
            "Droid 禁用模型条目缺少禁用原因",
        ));
    }
    if let Some(efforts) = object.get("supportedReasoningEfforts") {
        if !efforts.as_array().is_some_and(|values| {
            values.len() <= 32
                && values.iter().all(|value| {
                    value.as_str().is_some_and(|text| {
                        !text.is_empty()
                            && text.len() <= 64
                            && text
                                .bytes()
                                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                    })
                })
        }) {
            return Err(invalid());
        }
    }
    if has_sensitive_field(model, 0) {
        return Err(Error::new(
            "discovery_sensitive_output",
            "Droid 模型条目包含敏感字段，已拒绝接收",
        ));
    }
    Ok(())
}
fn has_sensitive_field(value: &Value, depth: usize) -> bool {
    if depth > 32 {
        return true;
    }
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            let key: String = key
                .chars()
                .filter(|character| !matches!(character, '_' | '-'))
                .flat_map(char::to_lowercase)
                .collect();
            matches!(
                key.as_str(),
                "authorization"
                    | "apikey"
                    | "accesstoken"
                    | "refreshtoken"
                    | "password"
                    | "cookie"
                    | "setcookie"
                    | "clientsecret"
                    | "secret"
                    | "credentials"
            ) || has_sensitive_field(value, depth + 1)
        }),
        Value::Array(values) => values
            .iter()
            .any(|value| has_sensitive_field(value, depth + 1)),
        _ => false,
    }
}
fn contains_secret(value: &Value, secret: &str) -> bool {
    if secret.is_empty() {
        return false;
    }
    match value {
        Value::String(text) => text.contains(secret),
        Value::Object(object) => object
            .iter()
            .any(|(key, value)| key.contains(secret) || contains_secret(value, secret)),
        Value::Array(values) => values.iter().any(|value| contains_secret(value, secret)),
        _ => false,
    }
}
fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}
fn output_limit() -> Error {
    Error::new("discovery_output_limit", "Droid 模型发现输出超过安全限制")
}

struct Workspace {
    path: PathBuf,
    parent: PathBuf,
    removed: bool,
}
impl Workspace {
    fn create(root: &Path) -> Result<Self> {
        if !root.is_absolute() {
            return Err(Error::new(
                "discovery_invalid_work_root",
                "模型发现运行目录须使用绝对路径",
            ));
        }
        fs::create_dir_all(root).map_err(|_| workspace_error())?;
        let parent = fs::canonicalize(root).map_err(|_| workspace_error())?;
        for _ in 0..4 {
            let path = parent.join(format!("discovery-{}", Uuid::new_v4()));
            match create_private_dir(&path) {
                Ok(()) => {
                    return Ok(Self {
                        path,
                        parent,
                        removed: false,
                    })
                }
                Err(_) if path.exists() => continue,
                Err(error) => return Err(error),
            }
        }
        Err(workspace_error())
    }
    fn cleanup(&mut self) -> Result<()> {
        if self.removed {
            return Ok(());
        }
        // Only our freshly created direct child is eligible. Never remove work_root itself.
        if self.path.parent() != Some(self.parent.as_path()) {
            return Err(workspace_error());
        }
        for attempt in 0..4 {
            if remove_owned_tree(&self.path, 0).is_ok() {
                self.removed = true;
                return Ok(());
            }
            if attempt < 3 {
                thread::sleep(Duration::from_millis(25));
            }
        }
        Err(workspace_error())
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
fn workspace_error() -> Error {
    Error::new(
        "discovery_workspace_failed",
        "无法创建或清理隔离模型发现目录",
    )
}
fn create_private_dir(path: &Path) -> Result<()> {
    #[allow(unused_mut)]
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|_| workspace_error())
}
fn remove_owned_tree(path: &Path, depth: usize) -> std::io::Result<()> {
    if depth > 64 {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let reparse = metadata.file_type().is_symlink();
    // Do not follow symlinks, junctions or any other reparse point made by the child.
    if reparse {
        return if metadata.is_dir() {
            fs::remove_dir(path)
        } else {
            fs::remove_file(path)
        };
    }
    if !metadata.is_dir() {
        return fs::remove_file(path);
    }
    for entry in fs::read_dir(path)? {
        remove_owned_tree(&entry?.path(), depth + 1)?;
    }
    fs::remove_dir(path)
}

struct ManagedProcess {
    child: Child,
    #[cfg(windows)]
    job: platform::Job,
    stopped: bool,
}
impl ManagedProcess {
    fn spawn(command: &mut Command) -> Result<Self> {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let job = platform::Job::create()?;
            command.creation_flags(0x0800_0000 | 0x0000_0004); // NO_WINDOW | SUSPENDED
            let mut child = command.spawn().map_err(|_| {
                Error::new("discovery_spawn_failed", "无法启动配置的 Droid 可执行文件")
            })?;
            if let Err(error) = job.attach_and_resume(&child) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
            Ok(Self {
                child,
                job,
                stopped: false,
            })
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
            Ok(Self {
                child: command.spawn().map_err(|_| {
                    Error::new("discovery_spawn_failed", "无法启动配置的 Droid 可执行文件")
                })?,
                stopped: false,
            })
        }
        #[cfg(not(any(windows, unix)))]
        {
            let _ = command;
            Err(Error::new(
                "discovery_unsupported_platform",
                "当前平台不支持安全模型发现",
            ))
        }
    }
    fn stop(&mut self) -> Result<()> {
        if self.stopped {
            return Ok(());
        }
        #[cfg(windows)]
        self.job.terminate();
        #[cfg(unix)]
        {
            unsafe extern "C" {
                fn kill(pid: i32, signal: i32) -> i32;
            }
            // process_group(0) made the child its own group leader before exec.
            let _ = unsafe { kill(-(self.child.id() as i32), 9) };
        }
        let _ = self.child.kill();
        self.child.wait().map_err(|_| {
            Error::new(
                "discovery_process_cleanup_failed",
                "Droid 发现进程未能正常回收",
            )
        })?;
        self.stopped = true;
        Ok(())
    }
}
impl Drop for ManagedProcess {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(windows)]
mod platform {
    use super::{Error, Result};
    use std::{ffi::c_void, mem::size_of, os::windows::io::AsRawHandle, process::Child};

    type Handle = *mut c_void;
    #[repr(C)]
    #[derive(Default)]
    struct BasicLimits {
        process_time: i64,
        job_time: i64,
        flags: u32,
        min_working_set: usize,
        max_working_set: usize,
        active_process_limit: u32,
        affinity: usize,
        priority: u32,
        scheduling: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimits {
        basic: BasicLimits,
        io_counters: [u64; 6],
        process_memory: usize,
        job_memory: usize,
        peak_process_memory: usize,
        peak_job_memory: usize,
    }
    #[repr(C)]
    #[derive(Default)]
    struct ThreadEntry {
        size: u32,
        usage: u32,
        id: u32,
        owner: u32,
        base_priority: i32,
        delta_priority: i32,
        flags: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(job: Handle, class: i32, info: *const c_void, len: u32) -> i32;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn TerminateJobObject(job: Handle, code: u32) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
        fn CreateToolhelp32Snapshot(flags: u32, process: u32) -> Handle;
        fn Thread32First(snapshot: Handle, entry: *mut ThreadEntry) -> i32;
        fn Thread32Next(snapshot: Handle, entry: *mut ThreadEntry) -> i32;
        fn OpenThread(access: u32, inherit: i32, id: u32) -> Handle;
        fn ResumeThread(thread: Handle) -> u32;
    }
    fn containment_error() -> Error {
        Error::new(
            "discovery_containment_failed",
            "无法建立 Droid 发现进程的安全回收边界",
        )
    }
    struct OwnedHandle(Handle);
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }
    pub struct Job(Option<OwnedHandle>);
    impl Job {
        pub fn create() -> Result<Self> {
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return Err(containment_error());
            }
            let job = Self(Some(OwnedHandle(handle)));
            let limits = ExtendedLimits {
                basic: BasicLimits {
                    flags: 0x2000, // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, no breakaway.
                    ..BasicLimits::default()
                },
                ..ExtendedLimits::default()
            };
            if unsafe {
                SetInformationJobObject(
                    handle,
                    9,
                    (&limits as *const ExtendedLimits).cast(),
                    size_of::<ExtendedLimits>() as u32,
                )
            } == 0
            {
                return Err(containment_error());
            }
            Ok(job)
        }
        pub fn attach_and_resume(&self, child: &Child) -> Result<()> {
            let handle = self.0.as_ref().ok_or_else(containment_error)?.0;
            if unsafe { AssignProcessToJobObject(handle, child.as_raw_handle()) } == 0 {
                return Err(containment_error());
            }
            // std::process closes the initial thread handle. Enumerate the still-suspended
            // child's thread and use the documented ResumeThread API after assignment.
            let snapshot = unsafe { CreateToolhelp32Snapshot(4, 0) }; // SNAPTHREAD
            if snapshot as isize == -1 || snapshot.is_null() {
                return Err(containment_error());
            }
            let snapshot = OwnedHandle(snapshot);
            let mut entry = ThreadEntry {
                size: size_of::<ThreadEntry>() as u32,
                ..ThreadEntry::default()
            };
            let mut has_entry = unsafe { Thread32First(snapshot.0, &mut entry) } != 0;
            while has_entry {
                if entry.owner == child.id() {
                    let thread = unsafe { OpenThread(2, 0, entry.id) }; // SUSPEND_RESUME
                    if thread.is_null() {
                        return Err(containment_error());
                    }
                    let thread = OwnedHandle(thread);
                    if unsafe { ResumeThread(thread.0) } == u32::MAX {
                        return Err(containment_error());
                    }
                    return Ok(());
                }
                has_entry = unsafe { Thread32Next(snapshot.0, &mut entry) } != 0;
            }
            Err(containment_error())
        }
        pub fn terminate(&mut self) {
            if let Some(handle) = self.0.take() {
                unsafe { TerminateJobObject(handle.0, 1) };
                // Closing our last handle also applies KILL_ON_JOB_CLOSE if explicit
                // termination failed. Do this before waiting for pipe-reader threads.
                drop(handle);
            }
        }
    }
}
