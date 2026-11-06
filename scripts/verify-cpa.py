"""使用固定版本的真实 CPA 宿主验证安装库，只连接本机模拟上游。"""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
from email.utils import formatdate
import hashlib
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import platform
import secrets
import shutil
import socket
import subprocess
import tarfile
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
import zipfile

ROOT = Path(__file__).resolve().parent.parent
UPSTREAM_ERROR_MESSAGE = ("thinking.display requires a compatible beta; "
    "api_key=fk-local-integration-only; Bearer mock-access-private; 'private test conversation'")
# 固定官方带原生插件能力的包；不接受 no-plugin 变体或动态 latest。
HOST_PACKAGES = {
    "8.0.47": {
        ("Windows", "amd64"): ("windows_amd64.zip", "8e23cbb2f9f695411b7df46320855820b352fb886c3c3bdf45ab4634f2f14632"),
        ("Linux", "amd64"): ("linux_amd64.tar.gz", "2b5111dc6a0dfe0d607457787f794330d1b477bc9daa6337c26b0cd032a7827e"),
        ("Darwin", "amd64"): ("darwin_amd64.tar.gz", "2e72cb7b28905b9135563e4fc01af00009ce72a444729eb95bdb634543ce7d41"),
        ("Darwin", "arm64"): ("darwin_aarch64.tar.gz", "a5b260ec0f843d2291488acd76e5595232da8f3b8ec2d334154f1570f5f3ca08"),
    },
    "8.0.52": {
        ("Windows", "amd64"): ("windows_amd64.zip", "395324dd124bd657bccd5fdf06119587cd7c87bddacff191d5f45431e8f08d23"),
        ("Linux", "amd64"): ("linux_amd64.tar.gz", "4f8855647b3107b83e6b4a2d9418f95d8c53c57827b7f0910cf5ee9c849ff88b"),
        ("Darwin", "amd64"): ("darwin_amd64.tar.gz", "a9f91c8ae68536b43a6e34178bcc9bd3233c4f684e4b83015ca331eebf5292bb"),
        ("Darwin", "arm64"): ("darwin_aarch64.tar.gz", "2294797c40ca0dffb89984e88efb242378cd538f3086ab301ad733c7c0c151c9"),
    },
}


def download_host(directory, version):
    arch = "arm64" if platform.machine().lower() in ("arm64", "aarch64") else "amd64"
    suffix, expected = HOST_PACKAGES[version][(platform.system(), arch)]
    name = f"CLIProxyAPI_{version}_{suffix}"
    archive = directory / name
    url = f"https://github.com/router-for-me/CLIProxyAPI/releases/download/v{version}/{name}"
    for attempt in range(3):
        try:
            with urllib.request.urlopen(url, timeout=120) as response:
                archive.write_bytes(response.read())
            break
        except (urllib.error.URLError, TimeoutError):
            if attempt == 2:
                raise
            time.sleep(attempt + 1)
    if hashlib.sha256(archive.read_bytes()).hexdigest() != expected:
        raise RuntimeError("CPA 宿主包 SHA256 不匹配")
    executable = directory / ("cli-proxy-api.exe" if os.name == "nt" else "cli-proxy-api")
    if name.endswith(".zip"):
        with zipfile.ZipFile(archive) as package:
            matches = [entry for entry in package.namelist() if Path(entry).name.lower() == executable.name]
            assert len(matches) == 1, "CPA 包中没有唯一的可执行文件"
            executable.write_bytes(package.read(matches[0]))
    else:
        with tarfile.open(archive) as package:
            matches = [entry for entry in package.getmembers() if entry.isfile() and Path(entry.name).name == executable.name]
            assert len(matches) == 1, "CPA 包中没有唯一的可执行文件"
            executable.write_bytes(package.extractfile(matches[0]).read())
        executable.chmod(0o700)
    return executable


class Mock(BaseHTTPRequestHandler):
    whoami_calls = 0
    rotations = 0
    oauth_logins = 0
    user_id = "mock-user"
    org_id = "mock-org"
    inference_calls = 0
    upgrade_calls = 0
    lock = threading.Lock()
    error_mode = None
    error_status = 400
    retry_after = "7"
    inference_delay = 0
    media_limit = None
    media_observations = []
    media_retry_delay = 0

    def log_message(self, *_):
        pass

    def reply(self, value, content_type="application/json", status=200, headers=None):
        raw = value.encode() if isinstance(value, str) else json.dumps(value).encode()
        inference = self.path.endswith(("/messages", "/responses", "/chat/completions"))
        delay = Mock.inference_delay if inference else 0
        if inference and 200 <= status < 300 and Mock.media_retry_delay:
            delay = Mock.media_retry_delay
        heartbeats = delay if content_type == "text/event-stream" else 0
        heartbeat = b": waiting\n\n"
        if inference and 200 <= status < 300:
            headers = {
                "Request-Id": "req_factory_success",
                "X-RateLimit-Remaining": "12",
                "Connection": "X-Upstream-Private",
                "X-Upstream-Private": "private",
                "Set-Cookie": "private=1",
                **(headers or {}),
            }
        if delay and not heartbeats:
            time.sleep(delay)
        try:
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(raw) + len(heartbeat) * heartbeats))
            for name, value in (headers or {}).items():
                self.send_header(name, value)
            self.end_headers()
            for _ in range(heartbeats):
                self.wfile.write(heartbeat)
                self.wfile.flush()
                time.sleep(1)
            self.wfile.write(raw)
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
            pass

    def do_GET(self):
        if self.path == "/api/billing/limits":
            observed = int(time.time() * 1000)
            window = lambda used, duration: {"usedPercent": used,
                "windowStart": observed - 3600000, "windowEnd": observed - 3600000 + duration}
            return self.reply({"limits": {
                "standard": {"fiveHour": window(2, 18000000), "weekly": window(14, 604800000),
                             "monthly": window(25, 2592000000)},
                "core": {"fiveHour": {"usedPercent": 0}, "weekly": {"usedPercent": 0},
                         "monthly": {"usedPercent": 0}}}, "extraUsageAllowed": False})
        with self.lock:
            Mock.whoami_calls += 1
        self.reply({"userId": Mock.user_id, "email": "mock@example.test", "orgId": Mock.org_id, "region": "us"})

    def do_POST(self):
        raw = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        if self.path.startswith("/user_management/"):
            fields = urllib.parse.parse_qs(raw.decode())
            if self.path.endswith("/authorize/device"):
                return self.reply({"device_code": "mock-device-authorization-code", "user_code": "MOCK-CODE",
                    "verification_uri_complete": "https://auth.factory.ai/activate?user_code=MOCK-CODE",
                    "expires_in": 300, "interval": 1})
            if fields.get("grant_type") == ["refresh_token"]:
                with self.lock:
                    Mock.rotations += 1
                access_token = f"mock-access-token-{Mock.rotations}"
                refresh_token = f"mock-refresh-token-{Mock.rotations}"
            else:
                with self.lock:
                    Mock.oauth_logins += 1
                access_token = f"mock-access-login-{Mock.oauth_logins}"
                refresh_token = f"mock-refresh-login-{Mock.oauth_logins}"
            return self.reply({"access_token": access_token,
                "refresh_token": refresh_token, "expires_in": 3600,
                "organization_id": "org_mock", "user": {"id": "user_mock"}})
        data = json.loads(raw)
        with self.lock:
            Mock.inference_calls += 1
            if self.headers.get("Upgrade", "").strip() or any(
                token.strip().lower() == "upgrade" for token in self.headers.get("Connection", "").split(",")
            ):
                Mock.upgrade_calls += 1
        if Mock.media_limit is not None:
            with self.lock:
                Mock.media_observations.append({"bytes": len(raw), "messages": data.get("messages"),
                    "session": self.headers.get("x-session-id"),
                    "message": self.headers.get("x-assistant-message-id"),
                    "trace": self.headers.get("traceparent")})
            if len(raw) > Mock.media_limit:
                return self.reply('{"error": {"code": "413", "message": "Request Entity Too Large"}}',
                    status=413, headers={"Server": "Vercel", "X-Vercel-Error": "FUNCTION_PAYLOAD_TOO_LARGE"})
        if Mock.error_mode:
            headers = {"Request-Id": "req_factory_diagnostic", "Retry-After": Mock.retry_after}
            if Mock.error_mode == "html":
                return self.reply("<html>private proxy diagnostics</html>", "text/html", Mock.error_status, headers)
            message = ("L" * (1024 * 1024 + 100) if Mock.error_mode == "oversized" else
                       "L" * 40000 if Mock.error_mode == "large" else UPSTREAM_ERROR_MESSAGE)
            raw = json.dumps({"error": {"type": "invalid_request_error", "message": message,
                "code": "invalid_feature_combination", "param": "thinking.display"},
                "request_id": "req_factory_diagnostic"}).encode()
            if Mock.error_mode in ("split", "stall"):
                self.send_response(Mock.error_status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(raw)))
                for name, value in headers.items():
                    self.send_header(name, value)
                self.end_headers()
                self.wfile.write(raw[:12])
                self.wfile.flush()
                time.sleep(3.5 if Mock.error_mode == "stall" else 0.02)
                try:
                    self.wfile.write(raw[12:])
                except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                    pass
                return
            return self.reply(raw.decode(), status=Mock.error_status, headers=headers)
        if self.path.endswith("/messages"):
            flags = set(self.headers.get("Anthropic-Beta", "").split(","))
            required = []
            if data.get("thinking", {}).get("display") == "updates":
                required.append("thinking-display-updates-2026-08-18")
            if data.get("thinking", {}).get("block_binding") is not None:
                required.append("thinking-binding-controls-2026-08-01")
            if data.get("context_management") is not None:
                required.append("context-management-2025-06-27")
            if any(tool.get("defer_loading") or tool.get("type", "").startswith("tool_search_tool_")
                   for tool in data.get("tools", [])):
                required.append("advanced-tool-use-2025-11-20")
            for message in data.get("messages", []):
                if message.get("role") == "system":
                    required.append("mid-conversation-system-2026-04-07")
                if message.get("output_config", {}).get("effort") is not None:
                    required.append("mid-conversation-effort-2026-08-01")
            if set(required) - flags or "betas" in data:
                return self.reply({"error": {"type": "invalid_request_error",
                    "message": "Messages feature beta missing or SDK-only betas body field present"}},
                    status=400, headers={"Request-Id": "req_strict_contract"})
        probe = {"session": self.headers.get("x-session-id"), "cache": data.get("prompt_cache_key"),
            "upstream_model": data.get("model"),
            "instructions": data.get("instructions"), "system": data.get("system"),
            "max_tokens": data.get("max_tokens"), "max_output_tokens": data.get("max_output_tokens"),
            "safety": data.get("safety_identifier"), "input": data.get("input"), "messages": data.get("messages"),
            "conversation": data.get("conversation"), "previous_response_id": data.get("previous_response_id"),
            "metadata": data.get("metadata"), "cache_control": data.get("cache_control"),
            "thinking": data.get("thinking"), "output_config": data.get("output_config"),
            "tools": data.get("tools"), "betas": data.get("betas"),
            "user_agent": self.headers.get("User-Agent"), "forwarded": self.headers.get("X-Forwarded-For"),
            "wire_headers": {name: self.headers.get(name) for name in [
                "Anthropic-Version", "Anthropic-Beta", "OpenAI-Beta", "X-Factory-Client",
                "X-Client-Version", "X-Stainless-OS", "X-Factory-Org-Id", "Forwarded"]}}
        if self.path.endswith("/messages"):
            value = {"id": "msg_mock", "type": "message", "role": "assistant", "model": data["model"],
                "content": [{"type": "text", "text": json.dumps(probe)}], "stop_reason": "end_turn",
                "usage": {"input_tokens": 10, "output_tokens": 1}}
            if data.get("stream"):
                return self.reply('event: message_start\ndata: ' + json.dumps({"type": "message_start",
                    "message": {**value, "content": [], "stop_reason": None}}) +
                    '\n\nevent: message_delta\ndata: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}'
                    '\n\nevent: message_stop\ndata: {"type":"message_stop"}\n\n', "text/event-stream")
        elif self.path.endswith("/chat/completions"):
            value = {"id": "chat_mock", "object": "chat.completion", "model": data["model"],
                "choices": [{"index": 0, "message": {"role": "assistant", "content": json.dumps(probe)}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 1, "total_tokens": 11}}
            if data.get("stream"):
                chunk = {**value, "object": "chat.completion.chunk", "choices": [
                    {"index": 0, "delta": {"content": "ok"}, "finish_reason": "stop"}]}
                return self.reply("data: " + json.dumps(chunk) + "\n\ndata: [DONE]\n\n", "text/event-stream")
        else:
            value = {"id": "resp_mock", "object": "response", "status": "completed", "model": data["model"],
                "output": [{"id": "msg_mock", "type": "message", "role": "assistant", "status": "completed",
                    "content": [{"type": "output_text", "text": json.dumps(probe), "annotations": []}]}],
                "usage": {"input_tokens": 10, "output_tokens": 1, "total_tokens": 11}}
            if data.get("stream"):
                return self.reply("event: response.completed\ndata: " +
                    json.dumps({"type": "response.completed", "response": value}) + "\n\n", "text/event-stream")
        self.reply(value)


class CPARequestError(RuntimeError):
    def __init__(self, path, status, code, body=None):
        super().__init__(f"CPA {path} 返回 HTTP {status}，错误码 {code}")
        self.status = status
        self.code = code
        self.body = body


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--library", required=True, type=Path)
    parser.add_argument("--host-version", choices=tuple(HOST_PACKAGES), default="8.0.52")
    parser.add_argument("--host", type=Path, help="复用与 --host-version 一致的已核验宿主")
    parser.add_argument("--report", type=Path)
    parser.add_argument("--observe", action="store_true", help="记录旧版本失败项，用于修复前对照")
    parser.add_argument("--registration-only", action="store_true", help="仅验证宿主版本、注册和菜单能力")
    parser.add_argument("--replay-request", type=Path,
        help="只向本机 Mock 回放原生 Messages JSON 或含 requestBody 的本机请求记录")
    args = parser.parse_args()
    library = args.library.resolve(strict=True)
    checks = []
    temp_root = ROOT / "temp"
    temp_root.mkdir(exist_ok=True)
    workspace = tempfile.TemporaryDirectory(prefix="cpa-native-", dir=temp_root)
    profile = Path(workspace.name).resolve()
    assert profile.is_relative_to(temp_root.resolve()) and profile.parent == temp_root.resolve()
    mock = ThreadingHTTPServer(("127.0.0.1", 0), Mock)
    threading.Thread(target=mock.serve_forever, daemon=True).start()
    process = None
    output = []
    actual_host_version = None

    def check(name, passed):
        checks.append({"name": name, "passed": bool(passed)})
        print(json.dumps(checks[-1], ensure_ascii=False), flush=True)
        if not passed and not args.observe:
            raise AssertionError(name)

    def wait_until(predicate, timeout=15):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                result = predicate()
                if result:
                    return result
            except (OSError, ValueError, KeyError, RuntimeError):
                pass
            time.sleep(0.2)
        return None

    try:
        executable = args.host.resolve(strict=True) if args.host else download_host(profile, args.host_version)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        management, client, other = (secrets.token_urlsafe(32) for _ in range(3))
        for folder in ["auth", "plugins", "data"]:
            (profile / folder).mkdir()
        (profile / "data/state.json").write_text(
            json.dumps({"version": 1, "settings": {"request_timeout_seconds": 10}}),
            encoding="utf-8")
        shutil.copy2(library, profile / "plugins" / ("factory-droid" + library.suffix))
        config = profile / "config.yaml"
        config.write_text(f"""config-version: 8
server:
  host: "127.0.0.1"
  port: {port}
  discovery:
    enabled: false
management:
  secret-key: "{management}"
  disable-control-panel: true
requests:
  passthrough-headers: true
access:
  api-keys: ["{client}", "{other}"]
oauth:
  auth-dir: "{(profile / 'auth').as_posix()}"
plugins:
  enabled: true
  dir: "{(profile / 'plugins').as_posix()}"
  configs:
    factory-droid:
      enabled: true
      data-dir: "{(profile / 'data').as_posix()}"
      api-base-url: "http://127.0.0.1:{mock.server_port}"
      allow-local-mock: true
observability:
  logs:
    logging-to-file: false
  usage:
    usage-statistics-enabled: true
routing:
  force-model-prefix: true
  retry:
    request-retry: 0
""", encoding="utf-8")

        def request(path, body=None, key=None, headers=None, stream=False, method=None, response_headers=None):
            nonlocal actual_host_version
            # urllib forces Connection: close, hiding reverse-proxy upgrade-header regressions.
            connection = http.client.HTTPConnection("127.0.0.1", port, timeout=30)
            try:
                connection.request(method or ("GET" if body is None else "POST"), path,
                    body=None if body is None else json.dumps(body).encode(),
                    headers={"Authorization": "Bearer " + (key or management), "Content-Type": "application/json", **(headers or {})})
                response = connection.getresponse()
                if response_headers is not None:
                    response_headers.update(dict(response.getheaders()))
                if path == "/v0/management/plugins":
                    actual_host_version = response.headers.get("X-CPA-VERSION", "").removeprefix("v")
                raw = response.read()
                if response.status >= 400:
                    # Do not expose credentials or upstream error bodies in diagnostics.
                    try:
                        error_body = json.loads(raw)
                        code = error_body.get("error", {}).get("code")
                    except (ValueError, AttributeError):
                        code = None
                        error_body = None
                    raise CPARequestError(path, response.status, code, error_body)
                return raw.decode() if stream else json.loads(raw)
            finally:
                connection.close()

        def drain(pipe):
            for line in iter(pipe.readline, b""):
                output.append(line.decode("utf-8", "replace"))
                if len(output) > 100:
                    output.pop(0)

        def start():
            nonlocal process
            process = subprocess.Popen([str(executable), "-config", str(config), "-local-model"],
                cwd=profile, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
            threading.Thread(target=drain, args=(process.stdout,), daemon=True).start()
            assert wait_until(lambda: request("/v0/management/plugins"), 25), "CPA 宿主未就绪"

        def stop():
            if process and process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=10)

        def models():
            return sorted(row["id"] for row in request("/v1/models", key=client)["data"])

        def status():
            return request("/v0/management/plugins/factory-droid/status")["data"]

        def call(text="A", headers=None, key=None, protocol="responses", stream=False, response_headers=None, **fields):
            paths = {"responses": "/v1/responses", "anthropic": "/v1/messages", "chat": "/v1/chat/completions"}
            body = ({"model": "factory/gpt-5.4", "input": text, "store": False} if protocol == "responses" else
                {"model": "factory/claude-opus-5-5" if protocol == "anthropic" else "factory/glm-5.3",
                    "messages": [{"role": "user", "content": text}], "max_tokens": 32})
            result = request(paths[protocol], {**body, "stream": stream, **fields}, key or client, headers, stream,
                             response_headers=response_headers)
            if stream:
                return result
            if protocol == "anthropic":
                return json.loads(result["content"][0]["text"])
            if protocol == "chat":
                return json.loads(result["choices"][0]["message"]["content"])
            return json.loads(result["output"][0]["content"][0]["text"])

        def update_auth(path, **fields):
            request("/v0/management/auth-files/fields", {"name": path.name, **fields}, method="PATCH")

        start()
        check("实际 CPA 宿主版本与验收目标一致", actual_host_version == args.host_version)
        entry = next((entry for entry in request("/v0/management/plugins")["plugins"]
            if entry["id"] == "factory-droid"), {})
        check("插件完成注册并启用", entry.get("registered") is True and entry.get("effective_enabled") is True)
        check("插件菜单已注册", any(menu.get("menu") == "Factory Droid" for menu in entry.get("menus", [])))
        check("原生 OAuth 与额度能力已注册", entry.get("supports_oauth") is True and entry.get("supports_quota") is True)
        if args.registration_only:
            return 0 if args.observe or all(row["passed"] for row in checks) else 1
        page = request("/v0/resource/plugins/factory-droid/index.html", stream=True)
        check("插件页面资源可访问", "Factory Droid" in page and "account-form" in page)
        enhancement = request("/v0/resource/plugins/factory-droid/manager-extension.js", stream=True)
        enhancement_css = request("/v0/resource/plugins/factory-droid/manager-extension.css", stream=True)
        check("原生凭证额度增强资源已内嵌", "__factoryDroidQuotaEnhancement" in enhancement
              and ".fd-quota-window" in enhancement_css and "native-quota-status" in page)
        added = request("/v0/management/plugins/factory-droid/accounts",
            {"label": "本机模拟凭证", "api_key": "fk-local-integration-only", "enabled": True})
        check("原生凭据创建并发布模型", added["ok"] and "factory/gpt-5.4" in models())
        quota_auth = next(file for file in request("/v0/management/auth-files")["files"]
                          if file.get("provider", file.get("type")) == "factory-droid")
        quota = request("/v0/management/plugins/factory-droid/quota", {"auth_index": quota_auth["auth_index"]})
        standard = next(group for group in quota["groups"] if group["displayName"] == "Factory 官方标准额度")
        check("原生额度接口保留标准窗口名称和比例",
              [bucket["window"] for bucket in standard["buckets"]] == ["标准 5 小时", "标准 7 天", "标准 30 天"]
              and abs(standard["buckets"][2]["remainingFraction"] - 0.75) < 0.00001)
        metadata = request("/v0/management/plugins/factory-droid/accounts/quota-windows",
                           {"auth_index": quota_auth["auth_index"], "name": quota_auth["name"]})
        detail = metadata["data"]
        check("窗口元数据绑定原生凭证且只返回标准额度",
              metadata["ok"] and detail["auth_index"] == quota_auth["auth_index"]
              and detail["name"] == quota_auth["name"] and len(detail["windows"]) == 3
              and all(window["group"] == "standard" for window in detail["windows"]))
        check("窗口元数据保留毫秒时间戳边界与官方观测时间",
              all(window["window_start"] and window["window_end"] and window["reset_source"] == "window_end"
                  for window in detail["windows"]) and detail["observed_at_ms"] > 0)
        check("窗口数据不包含密钥且不重复统计费用",
              "fk-local-integration-only" not in json.dumps(metadata)
              and "total_cost" not in detail and detail["model_scope"] == {"kind": "all", "complete": True})
        try:
            request("/v0/management/plugins/factory-droid/accounts/quota-windows",
                    {"auth_index": quota_auth["auth_index"], "name": "factory-droid-other.json"})
        except CPARequestError as failure:
            check("窗口查询拒绝错配凭证身份", failure.status == 404)
        else:
            check("窗口查询拒绝错配凭证身份", False)
        usage_module = request("/v0/resource/plugins/factory-droid/manager-usage.js", stream=True)
        check("窗口统计模块已内嵌并可加载", "export function buildTargets" in usage_module)
        refresh_module = request("/v0/resource/plugins/factory-droid/manager-refresh.js", stream=True)
        check("进入页面刷新策略与相对重置时间模块已内嵌",
              "ENTRY_REFRESH_INTERVAL = 180000" in refresh_module
              and "export function relativeReset" in refresh_module
              and "export function nextCycleRefreshAt" in refresh_module
              and "export function nextCycleWakeAt" in refresh_module)
        frontend_check = subprocess.run(["node", "--input-type=module", "-e", r"""
import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';
const root = new URL(process.argv[1]);
const load = async (path) => import('data:text/javascript;base64,' + readFileSync(new URL(path, root)).toString('base64'));
const { canRefreshOnEntry, isExpiredWindow, nextCycleRefreshAt, nextCycleWakeAt, relativeReset, restoreRefreshSnapshot } = await load('web/manager-refresh.js');
const { normalizeQuota, buildTargets } = await load('web/manager-usage.js');
const checks = [];
const check = (name, fn) => { fn(); checks.push(name); };
const now = Date.UTC(2026, 10, 4, 9);
const ended = now - 1000, future = now + 7 * 86400000;
const value = { attemptAt: now - 181000, windows: [{reset: ended}, {reset: future}] };
check('周期结束安排一次官方刷新，不人为推移旧周期', () =>
  assert.equal(nextCycleRefreshAt(value), ended));
check('周期刷新继续遵守三分钟间隔', () => {
  assert.equal(nextCycleRefreshAt({...value, attemptAt: now - 60000}), now + 120000);
  assert.equal(canRefreshOnEntry(now - 60000, now), false);
});
check('周期结束时立即更新显示，不等待查询冷却结束', () => {
  const expiring = {attemptAt:now,windows:[{reset:now+60000}]};
  assert.equal(nextCycleWakeAt(expiring, new Set(), now), now+60000);
  assert.equal(nextCycleWakeAt(expiring, new Set(), now+60001), now+180000);
});
check('同一周期失败或未变化不自动重复查询', () =>
  assert.equal(nextCycleRefreshAt(value, new Set([ended])), future));
check('周期查询进行中不再安排重复请求', () =>
  assert.equal(nextCycleRefreshAt({...value, loading:true}), null));
check('旧周期进入等待状态，不显示已到期或虚构新时间', () => {
  assert.equal(relativeReset(ended, now), '等待新周期');
  assert.equal(isExpiredWindow({reset: ended}, now), true);
  assert.equal(isExpiredWindow({reset: future}, now), false);
});
check('新周期重新建立统计区间且不沿用旧消耗', () => {
  const oldNow = Date.now;
  Date.now = () => now;
  try {
    const account = {key:'fixture', index:'index', name:'fixture.json'};
    const windows = normalizeQuota({ok:true, data:{provider:'factory-droid',
      auth_index:account.index, name:account.name, observed_at_ms:now, forecast_eligible:true,
      windows:[{group:'standard',window_key:'fiveHour',window_start:now-3600000,
        window_end:now+14400000,reset_source:'window_end',remaining_fraction:0.88}]}}, account);
    const earlier = {name:'5h',start:now-21600000,end:now-3600000,duration:18000000,
      current:{total_cost:8.23,total_tokens:49800000}};
    const targets = buildTargets(account, windows, [earlier]);
    assert.equal(windows[0].reset, now+14400000);
    assert.equal(windows[0].current, null);
    assert.equal(windows[0].previousLabel, '上个窗口用量');
    assert.equal(targets.find(v=>v.period==='current').from_ms, now-3600000);
    assert.equal(targets.find(v=>v.period==='previous').to_ms, now-3600000);
  } finally { Date.now=oldNow; }
});
check('月额度名称迁移保留已有快照和刷新间隔', () => {
  const value = {at:now-1000,attemptAt:now-1000,error:'',usageError:'',
    windows:[{name:'30d',label:'标准月额度',start:now-86400000,end:future,
      reset:future,observedAt:now-1000,duration:2592000000,percent:98,
      accuracy:'exact',forecastEligible:true,forecastReason:'',current:null,previous:null,
      previousRange:null,previousLabel:'前一等长区间'}]};
  const restored = restoreRefreshSnapshot(JSON.stringify({version:1,entries:[['fixture',value]]}),now);
  assert.equal(restored[0][1].windows[0].label,'月额度');
  assert.equal(restored[0][1].attemptAt,value.attemptAt);
  assert.equal(canRefreshOnEntry(restored[0][1].attemptAt,now),false);
});
console.log(JSON.stringify(checks));
""", ROOT.as_uri() + "/"], capture_output=True, text=True, encoding="utf-8", check=True)
        for name in json.loads(frontend_check.stdout):
            check(name, True)
        worker_headers = {}
        worker = request("/v0/resource/plugins/factory-droid/manager-worker.js", stream=True,
                         response_headers=worker_headers)
        check("自动加载 Worker 资源与管理页作用域声明有效",
              "Service-Worker-Allowed" in worker_headers
              and worker_headers["Service-Worker-Allowed"] == "/management.html"
              and 'url.pathname !== "/management.html"' in worker)
        bootstrap = request("/v0/resource/plugins/factory-droid/manager-bootstrap.js", stream=True)
        check("自动激活模块可访问且提供关闭入口",
              "__factoryDroidAutoBootstrap" in bootstrap and "自动加载额度增强" in page)
        auth_file = next((profile / "auth").glob("*.json"))
        original = json.loads(auth_file.read_text(encoding="utf-8"))
        original_models = models()
        check("初始目录已同步", status()["health"] == "ready")
        persisted_settings = json.loads((profile / "data/state.json").read_text(encoding="utf-8"))["settings"]
        check("升级兼容旧超时配置且不再持久化请求时限", "request_timeout_seconds" not in persisted_settings)
        try:
            Mock.inference_delay = 12

            def delayed_call(protocol, stream, marker):
                started = time.monotonic()
                try:
                    result = call(protocol=protocol, stream=stream)
                    completed = marker in result if stream else bool(result.get("session"))
                except (OSError, RuntimeError, ValueError, http.client.HTTPException):
                    completed = False
                return protocol, stream, completed and time.monotonic() - started >= 12

            with ThreadPoolExecutor(max_workers=6) as pool:
                pending = [pool.submit(delayed_call, protocol, stream, marker)
                    for protocol, marker in [("anthropic", "message_stop"),
                        ("responses", "response.completed"), ("chat", "[DONE]")]
                    for stream in (False, True)]
                for task in pending:
                    protocol, stream, completed = task.result()
                    check(f"{protocol} {'持续 SSE' if stream else '等待首响应'}超过旧时限仍完整返回", completed)
            check("长请求正常结束后释放所有操作", bool(wait_until(lambda: status()["active_requests"] == 0)))

            for stream in (False, True):
                connection = http.client.HTTPConnection("127.0.0.1", port, timeout=20)
                connection.connect()
                transport = connection.sock

                def receive():
                    try:
                        connection.request("POST", "/v1/responses",
                            body=json.dumps({"model": "factory/gpt-5.4", "input": "cancel", "stream": stream}).encode(),
                            headers={"Authorization": "Bearer " + client, "Content-Type": "application/json"})
                        connection.getresponse().read()
                    except (OSError, http.client.HTTPException):
                        pass

                with ThreadPoolExecutor(max_workers=1) as pool:
                    pending = pool.submit(receive)
                    try:
                        check("取消前请求进入执行状态", bool(wait_until(lambda: status()["active_requests"] == 1)))
                    finally:
                        transport.shutdown(socket.SHUT_RDWR)
                        connection.close()
                    pending.result(timeout=5)
                check(f"客户端取消{'流式' if stream else '非流式'}长请求后及时释放操作",
                    bool(wait_until(lambda: status()["active_requests"] == 0, timeout=5)))
        finally:
            Mock.inference_delay = 0
        a = call("one", conversation="conv-a", prompt_cache_key="shared")
        b = call("two", conversation="conv-b", prompt_cache_key="shared")
        check("共享缓存组不合并明确对话", a["session"] != b["session"] and a["cache"] == b["cache"])
        c = call("three", conversation="conv-a", prompt_cache_key="changed")
        check("切换缓存组不改变会话身份", a["session"] == c["session"] and a["cache"] != c["cache"])
        threads = lambda name: {"Session-Id": "root", "Thread-Id": name,
            "X-Codex-Turn-Metadata": json.dumps({"agent_name": "worker"})}
        first, second = call(headers=threads("first")), call(headers=threads("second"))
        check("同名子代理的不同线程独立", first["session"] != second["session"])
        check("同一子线程跨轮次稳定", first["session"] == call(headers=threads("first"))["session"])
        check("不同调用方隔离", first["session"] != call(headers=threads("first"), key=other)["session"])
        safety_a = call(conversation="safety-a", safety_identifier="same-user")
        safety_b = call(conversation="safety-b", safety_identifier="same-user")
        check("安全标识与对话分别映射", safety_a["safety"] == safety_b["safety"] and safety_a["session"] != safety_b["session"])
        anonymous_a, anonymous_b = call(prompt_cache_key="only-cache"), call(prompt_cache_key="only-cache")
        check("只有缓存键不建立共享对话", anonymous_a["session"] != anonymous_b["session"] and anonymous_a["cache"] == anonymous_b["cache"])
        with ThreadPoolExecutor(max_workers=8) as pool:
            concurrent = list(pool.map(lambda i: call(str(i), conversation=f"parallel-{i}", prompt_cache_key="shared"), range(16)))
        check("16 个并发会话身份与正文独立", len({row["session"] for row in concurrent}) == 16
            and all(row["input"] == str(i) for i, row in enumerate(concurrent)))
        native = call(conversation={"id": "conv-native"}, previous_response_id="resp-native",
            metadata={"proxy": "cpa"}, headers={"User-Agent": "cpa-test", "X-Forwarded-For": "127.0.0.99"})
        check("保留原生续接字段", native["conversation"] == {"id": "conv-native"} and native["previous_response_id"] == "resp-native")
        check("上游包装移除反代元数据", native["metadata"] is None and native["forwarded"] is None
            and native["user_agent"] == "factory-cli/1.2.0")
        for protocol, marker in [("anthropic", "message_stop"), ("responses", "response.completed"), ("chat", "[DONE]")]:
            check(f"{protocol} 原生非流式请求", bool(call(protocol=protocol)["session"]))
            check(f"{protocol} 原生流式请求", marker in call(protocol=protocol, stream=True))
            for stream in (False, True):
                returned_headers = {}
                call(protocol=protocol, stream=stream, response_headers=returned_headers)
                returned_headers = {name.lower(): value for name, value in returned_headers.items()}
                check(f"{protocol} {'SSE' if stream else 'JSON'} 保留诊断和额度头且过滤连接与 Cookie",
                    returned_headers.get("request-id") == "req_factory_success"
                    and returned_headers.get("x-ratelimit-remaining") == "12"
                    and "set-cookie" not in returned_headers and "x-upstream-private" not in returned_headers)
            client_headers = {
                "anthropic-beta": "claude-code-20250219,cpa-client-2025-01-01",
                "anthropic-version": "caller-version",
                "OpenAI-Beta": "cpa=caller-proxy",
                "User-Agent": "caller-claude-code",
                "X-Factory-Client": "caller-proxy",
                "X-Client-Version": "caller-version",
                "X-Stainless-OS": "caller-os",
                "X-Factory-Org-Id": "caller-org",
                "Forwarded": "for=caller-proxy",
                "X-Forwarded-For": "127.0.0.99",
            }
            header_probe = call(protocol=protocol, headers=client_headers)
            expected_headers = {name: None for name in header_probe["wire_headers"]}
            expected_headers.update({"X-Factory-Client": "cli", "X-Client-Version": "1.2.0"})
            if protocol == "anthropic":
                expected_headers.update({"Anthropic-Version": "2023-06-01",
                    "Anthropic-Beta": "fine-grained-tool-streaming-2025-05-14,claude-code-20250219,cpa-client-2025-01-01"})
            else:
                expected_headers["OpenAI-Beta"] = "cpa=caller-proxy"
            check(f"{protocol} 合并能力 beta 且继续隔离调用者身份头",
                  header_probe["wire_headers"] == expected_headers
                  and header_probe["user_agent"] == "factory-cli/1.2.0"
                  and header_probe["forwarded"] is None)
            check(f"{protocol} 带调用者版本和 beta 的 SSE 请求正常",
                  marker in call(protocol=protocol, stream=True, headers=client_headers))
            for case, headers in [
                ("孤立 Connection upgrade", {"Connection": "upgrade"}),
                ("混合大小写连接标记", {"Connection": "keep-alive, UpGrAdE"}),
                ("空 Upgrade", {"connection": "upgrade", "upgrade": ""}),
            ]:
                check(f"{protocol} 反代{case}保留 JSON 调用", bool(call(protocol=protocol, headers=headers)["session"]))
                check(f"{protocol} 反代{case}保留 SSE 调用", marker in call(protocol=protocol, stream=True, headers=headers))
            for case, headers in [
                ("WebSocket", {"Connection": "upgrade", "Upgrade": "websocket"}),
                ("独立 Upgrade", {"uPgRaDe": "h2c"}),
            ]:
                for stream in (False, True):
                    before = Mock.inference_calls
                    try:
                        call(protocol=protocol, stream=stream, headers=headers)
                    except CPARequestError as failure:
                        rejected = failure.status == 400 and failure.code == "unsupported_operation"
                    else:
                        rejected = False
                    check(f"{protocol} {'SSE' if stream else 'JSON'} 拒绝{case}且不访问上游",
                        rejected and Mock.inference_calls == before)
        check("反代升级标记不透传 Factory 上游", Mock.upgrade_calls == 0)
        cached = call(protocol="anthropic", cache_control={"type": "ephemeral", "ttl": "1h"})
        check("缓存断点原样保留", cached["cache_control"] == {"type": "ephemeral", "ttl": "1h"})

        # Strict feature cases must fail against the pre-fix library: accepting
        # arbitrary JSON in a mock cannot verify a body/header protocol contract.
        feature_request = {
            "thinking": {"type": "adaptive", "display": "updates",
                "block_binding": {"prefix_mismatch_behavior": "drop_block"}},
            "output_config": {"effort": "xhigh"},
            "context_management": {"edits": [{"type": "clear_thinking_20251015", "keep": "all"}]},
            "tools": [{"name": "read_file", "defer_loading": True,
                "input_schema": {"type": "object", "properties": {"path": {"type": "string"}}}},
                {"type": "tool_search_tool_regex_20251119", "name": "tool_search_tool_regex"}],
            "messages": [
                {"role": "user", "content": "Read the file"},
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "Inspect the file", "signature": "opaque-signature-kept"},
                    {"type": "tool_use", "id": "toolu_read1", "name": "read_file", "input": {"path": "README.md"}}]},
                {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_read1",
                    "content": [{"type": "text", "text": "File contents"}]}]},
            ],
            "betas": ["claude-code-20250219", "arbitrary-caller-beta"],
        }
        feature_probe = call(protocol="anthropic", **feature_request,
            headers={"Anthropic-Beta": "claude-code-20250219,arbitrary-caller-beta"})
        expected_flags = {"fine-grained-tool-streaming-2025-05-14",
            "thinking-display-updates-2026-08-18", "thinking-binding-controls-2026-08-01",
            "context-management-2025-06-27", "advanced-tool-use-2025-11-20",
            "claude-code-20250219", "arbitrary-caller-beta"}
        check("完整 Claude 工具请求按功能生成 beta 且保留原生内容",
            set(feature_probe["wire_headers"]["Anthropic-Beta"].split(",")) == expected_flags
            and feature_probe["messages"] == feature_request["messages"]
            and feature_probe["thinking"] == feature_request["thinking"]
            and feature_probe["output_config"] == feature_request["output_config"]
            and feature_probe["tools"] == feature_request["tools"])
        check("SDK betas 合并到能力头且不进入上游正文", feature_probe["betas"] is None)
        check("完整 Claude 功能请求可通过严格上游 SSE 校验",
            "message_stop" in call(protocol="anthropic", stream=True, **feature_request))

        def image(size):
            return {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "A" * size}}

        def image_history(old_size, new_size):
            return [
                {"role": "user", "content": [image(old_size)]},
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "分析截图", "signature": "opaque-history-signature"},
                    {"type": "tool_use", "id": "capture-1", "name": "capture", "input": {}}]},
                {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "capture-1",
                    "content": [{"type": "text", "text": "保留工具结果"}, image(new_size), image(new_size)]}]},
            ]

        try:
            for stream in (False, True):
                # 真实宿主负责 Payload 与流式生命周期，Mock 按实际收到的 UTF-8 字节拒绝超限请求。
                Mock.media_limit, Mock.media_observations = 4_000_000, []
                history = image_history(2_100_000, 1_000_000)
                result = call(protocol="anthropic", stream=stream, messages=history)
                observed = Mock.media_observations
                check(f"历史图片超预算时预先缩减且 {'SSE' if stream else 'JSON'} 一次成功",
                    len(observed) == 1 and observed[0]["bytes"] < 4_000_000
                    and observed[0]["messages"][0]["content"][0]["type"] == "text"
                    and observed[0]["messages"][1:] == history[1:]
                    and ("message_stop" in result if stream else bool(result.get("session"))))

                Mock.media_limit, Mock.media_observations = 9_000, []
                history = image_history(12_000, 400)
                result = call(protocol="anthropic", stream=stream, messages=history)
                observed = Mock.media_observations
                check(f"真实 413 触发 {'SSE' if stream else 'JSON'} 一次缩减重试并保留工具配对",
                    len(observed) == 2 and observed[0]["bytes"] > 9_000 > observed[1]["bytes"]
                    and observed[0]["messages"] == history
                    and observed[1]["messages"][1:] == history[1:]
                    and observed[1]["messages"][0]["content"][0]["type"] == "text"
                    and all(observed[0][key] == observed[1][key] for key in ("session", "message", "trace"))
                    and ("message_stop" in result if stream else bool(result.get("session"))))

                for case, history, expected_calls in [
                    ("持续拒绝", image_history(12_000, 400), 2),
                    ("只有最新图片", [{"role": "user", "content": [image(12_000)]}], 1),
                    ("纯文本", [{"role": "user", "content": "text" * 3_000}], 1),
                ]:
                    Mock.media_limit, Mock.media_observations = 1, []
                    try:
                        call(protocol="anthropic", stream=stream, messages=history)
                    except CPARequestError as failure:
                        message = (failure.body or {}).get("error", {}).get("message", "")
                        valid = failure.status == 413 and message.startswith("Request Entity Too Large")
                        valid &= "压缩会话" in message
                    else:
                        valid = False
                    check(f"413 {'SSE' if stream else 'JSON'} {case}停止重试并给出处理提示",
                        valid and len(Mock.media_observations) == expected_calls)
                check("413 结束后释放宿主活动请求", bool(wait_until(lambda: status()["active_requests"] == 0)))

                Mock.media_limit, Mock.media_observations, Mock.media_retry_delay = 9_000, [], 12
                connection = http.client.HTTPConnection("127.0.0.1", port, timeout=20)
                connection.connect()
                transport = connection.sock

                def cancel_size_retry():
                    try:
                        connection.request("POST", "/v1/messages", body=json.dumps({
                            "model": "factory/claude-opus-5-5", "max_tokens": 32, "stream": stream,
                            "messages": image_history(12_000, 400)}).encode(),
                            headers={"Authorization": "Bearer " + client, "Content-Type": "application/json"})
                        connection.getresponse().read()
                    except (OSError, http.client.HTTPException):
                        pass

                try:
                    with ThreadPoolExecutor(max_workers=1) as pool:
                        pending = pool.submit(cancel_size_retry)
                        try:
                            check("取消前 413 恢复已进入第二次 HTTP 调用",
                                bool(wait_until(lambda: len(Mock.media_observations) == 2)))
                        finally:
                            transport.shutdown(socket.SHUT_RDWR)
                            connection.close()
                        pending.result(timeout=5)
                    check(f"取消 413 {'SSE' if stream else 'JSON'} 重试后及时释放宿主操作",
                        bool(wait_until(lambda: status()["active_requests"] == 0, timeout=5))
                        and len(Mock.media_observations) == 2)
                finally:
                    Mock.media_retry_delay = 0

            Mock.media_limit, Mock.media_observations = 1, []
            for protocol in ("responses", "chat"):
                before = len(Mock.media_observations)
                try:
                    call(protocol=protocol)
                except CPARequestError as failure:
                    valid = failure.status == 413
                    valid &= (failure.body or {}).get("error", {}).get("message") == "Request Entity Too Large"
                else:
                    valid = False
                check(f"{protocol} 不执行 Claude 图片缩减重试",
                    valid and len(Mock.media_observations) == before + 1)
        finally:
            Mock.media_limit, Mock.media_observations = None, []
            Mock.media_retry_delay = 0

        if args.replay_request:
            recorded = json.loads(args.replay_request.read_text(encoding="utf-8"))
            replay = json.loads(recorded["requestBody"]) if isinstance(recorded.get("requestBody"), str) else recorded
            assert isinstance(replay.get("messages"), list), "回放文件必须是原生 Messages 请求"
            replay = {key: value for key, value in replay.items() if key not in ("model", "stream")}
            try:
                for stream in (False, True):
                    Mock.media_limit, Mock.media_observations = 4_000_000, []
                    try:
                        result = call(protocol="anthropic", stream=stream, **replay)
                        valid = "message_stop" in result if stream else bool(result.get("session"))
                    except CPARequestError:
                        valid = False
                    observed = Mock.media_observations
                    sent_bytes = observed[-1]["bytes"] if observed else 0
                    check(f"本机历史请求 {'SSE' if stream else 'JSON'} 回放一次成功，发送 {sent_bytes} 字节",
                        valid and len(observed) == 1 and 0 < sent_bytes <= 4_000_000)
            finally:
                Mock.media_limit, Mock.media_observations = None, []

        changed_turn = call(protocol="anthropic", messages=[
            {"role": "system", "content": [{"type": "text", "text": "New instruction"}]},
            {"role": "user", "content": "Continue", "output_config": {"effort": "high"}}])
        check("中途系统消息与推理档位携带对应能力标志",
            {"mid-conversation-system-2026-04-07", "mid-conversation-effort-2026-08-01"}
            <= set(changed_turn["wire_headers"]["Anthropic-Beta"].split(",")))
        before = Mock.inference_calls
        try:
            call(protocol="anthropic", fallbacks=[{"model": "unverified-model"}])
        except CPARequestError as failure:
            # CPA's Claude error writer retains only type/message for executor
            # errors. Interceptor errors take its separate DirectResponse path.
            rejected = failure.status == 400 and "fallback" in (failure.body or {}).get("error", {}).get("message", "")
        else:
            rejected = False
        check("未实现的 fallback 在访问上游前明确拒绝", rejected and Mock.inference_calls == before)

        count_path = "/v1/messages/count_tokens"
        count_body = {"model": "factory/claude-opus-5-5", "messages": [{"role": "user", "content": "hello"}]}
        before = Mock.inference_calls
        counted = request(count_path, count_body, key=client)
        check("真实 CPA token 计数入口返回明确的本地 O200k 估算",
            counted.get("input_tokens") == 3 and counted.get("estimated") is True
            and counted.get("estimator") == "o200k_base"
            and counted.get("media_tokens_included") is False)
        rich_count = request(count_path, {**count_body, "system": "You help with files.",
            "tools": feature_request["tools"], "messages": feature_request["messages"]}, key=client)
        check("token 估算覆盖 system 与工具历史且不发起推理",
            rich_count["input_tokens"] > counted["input_tokens"] and Mock.inference_calls == before)
        for name, body, key, expected_status in [
            ("空消息", {**count_body, "messages": []}, client, 400),
            ("流式计数", {**count_body, "stream": True}, client, 400),
            ("未授权调用方", count_body, "invalid-local-test-client", 401),
        ]:
            try:
                request(count_path, body, key=key)
            except CPARequestError as failure:
                rejected = failure.status == expected_status
            else:
                rejected = False
            check(f"token 计数拒绝{name}且不访问上游", rejected and Mock.inference_calls == before)

        try:
            for mode, stream, protocol in [
                ("json", False, "anthropic"), ("json", True, "anthropic"),
                ("split", True, "anthropic"), ("html", False, "anthropic"),
                ("oversized", True, "anthropic"), ("large", False, "anthropic"), ("stall", True, "anthropic"),
                ("json", False, "responses"), ("json", True, "responses"),
                ("json", False, "chat"), ("json", True, "chat"),
            ]:
                Mock.error_mode = mode
                started = time.monotonic()
                try:
                    call("private test conversation", protocol=protocol, stream=stream)
                except CPARequestError as failure:
                    error_body = failure.body or {}
                    message = error_body.get("error", {}).get("message", "")
                    valid = failure.status == 400 and len(message) <= 1024 * 1024 + 128
                    if mode in ("json", "split", "stall"):
                        expected_message = UPSTREAM_ERROR_MESSAGE
                        if protocol == "responses" and stream:
                            # This transformation belongs to CPA v8.0.52's
                            # Responses writer, after the plugin returns raw JSON.
                            expected_message = expected_message.replace(
                                "fk-local-integration-only", "[REDACTED]").replace(
                                "mock-access-private", "[REDACTED]")
                        valid &= message == expected_message
                        valid &= error_body.get("error", {}).get("type") == "invalid_request_error"
                        if protocol != "anthropic":
                            if not (protocol == "responses" and stream):
                                valid &= error_body.get("request_id") == "req_factory_diagnostic"
                            valid &= failure.code == "invalid_feature_combination"
                            valid &= error_body.get("error", {}).get("param") == "thinking.display"
                    elif mode == "html":
                        valid &= message == "<html>private proxy diagnostics</html>"
                    elif mode == "oversized":
                        valid &= message.endswith("[上游错误正文超过 1048576 字节，已截断]")
                    elif mode == "large":
                        valid &= message == "L" * 40000
                else:
                    valid = False
                if mode == "stall":
                    valid &= time.monotonic() - started >= 3.5
                label = (f"{protocol} 上游错误 {mode}/{'SSE' if stream else 'JSON'} 原因不脱敏不替换且读取有界"
                    if not (protocol == "responses" and stream) else
                    "Responses SSE 原始错误经过宿主自身脱敏的边界已验证")
                check(label, valid)
            Mock.error_mode = "json"
            for retry, expected in [("604800", 604800), (formatdate(time.time() + 259200, usegmt=True), 259200)]:
                Mock.retry_after = retry
                returned_headers = {}
                try:
                    call(protocol="responses", response_headers=returned_headers)
                except CPARequestError as failure:
                    retry_header = next((value for name, value in returned_headers.items()
                        if name.lower() == "retry-after"), "0")
                    check("上游 Retry-After 支持长等待和 HTTP 日期且不截断为一天",
                        failure.status == 400 and expected - 5 <= int(retry_header) <= expected + 1)
                else:
                    check("上游 Retry-After 支持长等待和 HTTP 日期且不截断为一天", False)
        finally:
            Mock.error_mode = None
            Mock.retry_after = "7"
        check("错误响应关闭后后续正常请求仍可调用", bool(call(protocol="anthropic")["session"]))

        original_config = config.read_text(encoding="utf-8")
        try:
            for remove_cache in (False, True):
                stop()
                payload_rules = """
  payload:
    override:
      - models:
          - name: "factory/gpt-5.4"
        params:
          instructions: "configured instructions"
          metadata.owner: "configured"
          prompt_cache_key: "configured-cache"
          model: "configured-upstream-model"
          max_output_tokens: 24000
      - models:
          - name: "factory/claude-opus-5-5"
        params:
          system: "configured system"
          max_tokens: 24000
    filter:
      - models:
          - name: "factory/gpt-5.4"
        params: ["safety_identifier"%s]
""" % (', "prompt_cache_key"' if remove_cache else "")
                config.write_text(original_config.replace(
                    "  passthrough-headers: true\n", "  passthrough-headers: true\n" + payload_rules),
                    encoding="utf-8")
                start()
                assert wait_until(lambda: "factory/gpt-5.4" in models())
                configured = call(conversation="payload-final", prompt_cache_key="client-cache",
                    safety_identifier="client-user")
                check(f"CPA 最终 payload {'filter' if remove_cache else 'override'} 不被插件重写",
                    configured["instructions"] == "configured instructions"
                    and configured["metadata"] == {"owner": "configured"}
                    and configured["cache"] == (None if remove_cache else "configured-cache")
                    and configured["safety"] is None
                    and configured["upstream_model"] == "configured-upstream-model"
                    and configured["max_output_tokens"] == 24000)
                claude_configured = call(protocol="anthropic")
                check("CPA 配置的 Claude system 和输出参数在封装后保持最终值",
                    claude_configured["system"] == "configured system"
                    and claude_configured["max_tokens"] == 24000)
                check("CPA payload 配置下三协议 SSE 正常结束", all(
                    marker in call(protocol=protocol, stream=True)
                    for protocol, marker in [("anthropic", "message_stop"),
                        ("responses", "response.completed"), ("chat", "[DONE]")]))
        finally:
            stop()
            config.write_text(original_config, encoding="utf-8")
        stop()
        start()
        assert wait_until(lambda: "factory/gpt-5.4" in models())
        resumed = call("one", conversation="conv-a", prompt_cache_key="shared")
        check("重启后匿名会话和缓存稳定", resumed["session"] == a["session"] and resumed["cache"] == a["cache"])

        update_auth(auth_file, prefix="team", excluded_models=["factory/glm-5.3"],
            model_aliases=[{"name": "factory/gpt-5.4", "alias": "friendly-gpt"}])
        expected = sorted("team/" + ("friendly-gpt" if name == "factory/gpt-5.4" else name)
            for name in original_models if name != "factory/glm-5.3")
        check("CPA 原生排除、别名和前缀生效", bool(wait_until(lambda: models() == expected)))
        check("原生规则不导致目录降级", status()["health"] == "ready")
        try:
            reconciled = request("/v0/management/plugins/factory-droid/models/reconcile", {})
            check("原生规则下手工同步成功", reconciled["ok"])
        except RuntimeError:
            check("原生规则下手工同步成功", False)
        check("原生别名可调用", bool(call(model="team/friendly-gpt")["session"]))
        update_auth(auth_file, excluded_models=["*"])
        check("CPA 支持隐藏全部模型", bool(wait_until(lambda: not models())))
        check("全部模型被排除仍视为正常", status()["health"] == "ready")
        update_auth(auth_file, prefix="", excluded_models=[], model_aliases=[])
        # CPA v8.0.47 can retain an old model_aliases attribute after a generic
        # fields PATCH. Reset the fixture through a fresh host load; this does
        # not claim that hot removal of native aliases has been repaired.
        stop()
        start()
        restored = wait_until(lambda: models() == original_models)
        if not restored:
            print(json.dumps({"expected_models": original_models, "actual_models": models(),
                "rule_fields": {key: json.loads(auth_file.read_text(encoding="utf-8")).get(key)
                    for key in ("prefix", "excluded_models", "model_aliases", "disabled")}}, ensure_ascii=False))
        assert restored, "原生模型规则恢复失败"

        before_refresh = Mock.whoami_calls
        update_auth(auth_file, factory_catalog_checked_at="2020-01-01T00:00:00Z")
        stop()
        start()
        refreshed = wait_until(lambda: Mock.whoami_calls > before_refresh
            and json.loads(auth_file.read_text(encoding="utf-8")).get("factory_catalog_checked_at", "").startswith(str(datetime.now().year)))
        check("API Key 目录到期后由 CPA 自动刷新", bool(refreshed))
        check("目录刷新不改变凭证标识", json.loads(auth_file.read_text(encoding="utf-8"))["account_id"] == original["account_id"])

        login = request("/v0/management/factory-droid-auth-url")
        check("原生 OAuth 创建设备授权", login.get("url", "").startswith("https://auth.factory.ai/"))
        oauth_state = urllib.parse.quote(login["state"])
        authorized = wait_until(lambda: request("/v0/management/get-auth-status?state=" + oauth_state).get("status") == "ok", 20)
        check("原生 OAuth 完成模拟授权", bool(authorized))
        oauth_file = next(path for path in (profile / "auth").glob("*.json")
            if json.loads(path.read_text(encoding="utf-8")).get("auth_method") == "oauth")
        check("同身份 API Key 与 OAuth 凭证分别保存",
              len(list((profile / "auth").glob("*.json"))) == 2
              and json.loads(auth_file.read_text(encoding="utf-8"))["account_id"] == original["account_id"])

        def complete_oauth_login():
            login = request("/v0/management/factory-droid-auth-url")
            state = urllib.parse.quote(login["state"])
            return wait_until(lambda: request("/v0/management/get-auth-status?state=" + state).get("status") == "ok", 20)

        def auth_records():
            return {path.name: json.loads(path.read_text(encoding="utf-8"))
                    for path in (profile / "auth").glob("*.json")}

        initial = auth_records()
        check("同账号重新授权成功", bool(complete_oauth_login()))
        repeated = auth_records()
        check("重新授权更新令牌而不增加凭证",
              repeated.keys() == initial.keys()
              and repeated[oauth_file.name]["account_id"] == initial[oauth_file.name]["account_id"]
              and repeated[oauth_file.name]["access_token"] != initial[oauth_file.name]["access_token"]
              and repeated[oauth_file.name]["refresh_token"] != initial[oauth_file.name]["refresh_token"])

        # Simulate the random IDs and filenames emitted by older plugin versions.
        # These are disposable fixtures inside this verifier's isolated profile.
        stop()
        legacy = repeated[oauth_file.name]
        legacy.update(account_id="factory-legacy-oauth", label="保留的 OAuth 名称", priority=7, disabled=True,
                      note="保留备注", prefix="oauth-team", excluded_models=["factory/glm-5.3"],
                      model_aliases=[{"name": "factory/gpt-5.4", "alias": "oauth-friendly"}])
        legacy_file = profile / "auth/factory-droid-factory-legacy-oauth.json"
        assert oauth_file.is_relative_to(profile) and legacy_file.is_relative_to(profile)
        oauth_file.rename(legacy_file)
        oauth_file = legacy_file
        oauth_file.write_text(json.dumps(legacy), encoding="utf-8")
        start()
        legacy_index = next(file["auth_index"] for file in request("/v0/management/auth-files")["files"]
                            if file["name"] == oauth_file.name)
        before_legacy = auth_records()
        check("旧版随机文件名的 OAuth 凭证可重新授权", bool(complete_oauth_login()))
        after_legacy = auth_records()
        check("旧版重新授权保留文件名和原账号 ID",
              after_legacy.keys() == before_legacy.keys()
              and after_legacy[oauth_file.name]["account_id"] == "factory-legacy-oauth")
        check("重新授权保留名称、优先级、备注和原生模型规则",
              all(after_legacy[oauth_file.name].get(field) == legacy.get(field)
                  for field in ["label", "disabled", "priority", "created_at", "note", "prefix", "excluded_models", "model_aliases"]))
        check("重新授权不启用用户手动停用的凭证",
              after_legacy[oauth_file.name].get("disabled") is True
              and next(file for file in request("/v0/management/auth-files")["files"]
                       if file["name"] == oauth_file.name).get("disabled") is True)
        check("重新授权保留 CPA 原生凭证索引",
              next(file["auth_index"] for file in request("/v0/management/auth-files")["files"]
                   if file["name"] == oauth_file.name) == legacy_index)

        Mock.org_id = "mock-other-org"
        before_other_org = auth_records()
        check("同一用户在其他组织可独立授权", bool(complete_oauth_login()))
        after_other_org = auth_records()
        check("不同组织不合并且不覆盖原凭证",
              len(after_other_org) == len(before_other_org) + 1
              and after_other_org[oauth_file.name] == before_other_org[oauth_file.name])
        Mock.user_id, Mock.org_id = "mock-other-user", "mock-org"
        before_other_user = auth_records()
        check("相同邮箱的其他用户可独立授权", bool(complete_oauth_login()))
        after_other_user = auth_records()
        check("不同用户不依赖相同邮箱合并",
              len(after_other_user) == len(before_other_user) + 1
              and after_other_user[oauth_file.name] == before_other_user[oauth_file.name])

        Mock.user_id = "mock-concurrent-user"
        before_concurrent = auth_records()
        with ThreadPoolExecutor(max_workers=2) as pool:
            simultaneous = list(pool.map(lambda _: complete_oauth_login(), range(2)))
        after_concurrent = auth_records()
        check("同身份并发首次授权均完成且只新增一个凭证",
              all(simultaneous) and len(after_concurrent) == len(before_concurrent) + 1)
        concurrent_name = next(name for name in after_concurrent if name not in before_concurrent)
        deleted_id = after_concurrent[concurrent_name]["account_id"]
        request("/v0/management/auth-files?name=" + urllib.parse.quote(concurrent_name), method="DELETE")
        check("隔离凭证按 CPA 原生删除流程删除", concurrent_name not in auth_records())
        # Older plugin versions can retain a deletion tombstone in their cache.
        stop()
        cached = json.loads((profile / "data/state.json").read_text(encoding="utf-8"))
        cached["accounts"][deleted_id].update(deleted=True, enabled=False)
        (profile / "data/state.json").write_text(json.dumps(cached), encoding="utf-8")
        start()
        check("保留历史删除记录时允许同账号再次授权", bool(complete_oauth_login()))
        after_recreation = auth_records()
        recreated_name = next(name for name in after_recreation if name not in before_concurrent)
        check("删除后重新授权使用新标识且不复活删除记录",
              len(after_recreation) == len(after_concurrent)
              and after_recreation[recreated_name]["account_id"] != deleted_id)
        check("删除后再次重复授权仍复用同一凭证", bool(complete_oauth_login())
              and auth_records().keys() == after_recreation.keys())
        Mock.user_id = None
        before_missing_identity = auth_records()
        login = request("/v0/management/factory-droid-auth-url")
        state = urllib.parse.quote(login["state"])
        rejected_identity = wait_until(lambda:
            request("/v0/management/get-auth-status?state=" + state).get("status") == "error", 20)
        check("缺少官方用户 ID 时拒绝保存且不增加凭证",
              bool(rejected_identity) and auth_records() == before_missing_identity)
        Mock.user_id, Mock.org_id = "mock-user", "mock-org"

        # Reauthorization does not clean up pre-existing duplicate files.
        stop()
        duplicate_file = profile / "auth/factory-droid-z-existing-duplicate.json"
        duplicate = dict(after_legacy[oauth_file.name], account_id="factory-existing-duplicate", disabled=True)
        duplicate_file.write_text(json.dumps(duplicate), encoding="utf-8")
        legacy = json.loads(oauth_file.read_text(encoding="utf-8"))
        legacy.update(prefix="", excluded_models=[], model_aliases=[], disabled=False)
        oauth_file.write_text(json.dumps(legacy), encoding="utf-8")
        start()
        before_duplicates = auth_records()
        check("已有重复项时仍可重新授权", bool(complete_oauth_login()))
        after_duplicates = auth_records()
        check("已有重复项不自动清理也不再新增",
              after_duplicates.keys() == before_duplicates.keys()
              and after_duplicates[duplicate_file.name] == before_duplicates[duplicate_file.name])

        update_auth(oauth_file, expires_at="2020-01-01T00:00:00Z", last_refresh="2019-01-01T00:00:00Z")
        stop()
        start()
        check("OAuth 到期后轮换并持久化令牌", bool(wait_until(lambda: Mock.rotations > 0
            and json.loads(oauth_file.read_text(encoding="utf-8")).get("refresh_token") == f"mock-refresh-token-{Mock.rotations}", 20)))
        persisted = (profile / "data/state.json").read_text(encoding="utf-8")
        check("插件状态不保存凭证或令牌", all(secret not in persisted for secret in
            ["fk-local-integration-only", "mock-access-token-", "mock-refresh-token-",
             "mock-access-login-", "mock-refresh-login-"]))
        check("最终目录和健康状态正常", bool(wait_until(lambda: status()["health"] == "ready")) and models() == original_models)

        future_id = "future-openai-without-capability-data"
        stored_models = json.loads((profile / "data/state.json").read_text(encoding="utf-8"))["settings"]["models"]
        candidates = [{"id": model["upstream_id"], "displayName": model["display_name"],
            "modelProvider": model["upstream_provider"] if model["upstream_provider"] in ("openai", "anthropic", "xai") else "factory"}
            for model in stored_models]
        future = {"id": future_id, "displayName": "Future model", "modelProvider": "openai"}
        candidates.append(future)
        update_auth(auth_file, factory_catalog={"version": 1, "candidates": candidates})
        assert wait_until(lambda: "factory/" + future_id in models()), "未发布能力未知的新模型"
        image_input = [{"role": "user", "content": [{"type": "input_image",
            "image_url": "data:image/png;base64,AAAA"}]}]
        forwarded = call(model="factory/" + future_id, input=image_input)
        check("能力数据缺失的新模型仍将图片完整交给上游", forwarded["input"] == image_input)
        check("能力未知的新模型图片 SSE 请求正常完成",
            "response.completed" in call(model="factory/" + future_id, input=image_input, stream=True))
        future_state = next(model for model in json.loads(
            (profile / "data/state.json").read_text(encoding="utf-8"))["settings"]["models"]
            if model["upstream_id"] == future_id)
        check("未知模型不伪造 8192 输出限制或无图片能力",
            future_state["max_output_tokens"] == 0 and future_state.get("supports_images") is None)
        future["noImageSupport"] = True
        update_auth(auth_file, factory_catalog={"version": 1, "candidates": candidates})
        assert wait_until(lambda: any(model["upstream_id"] == future_id and model.get("supports_images") is False
            for model in json.loads((profile / "data/state.json").read_text(encoding="utf-8"))["settings"]["models"]))
        before = Mock.inference_calls
        try:
            call(model="factory/" + future_id, input=image_input)
        except CPARequestError as failure:
            rejected = failure.status == 400
        else:
            rejected = False
        check("仅明确不支持图片的模型在访问上游前拒绝图片", rejected and Mock.inference_calls == before)
    finally:
        if process and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)
        mock.shutdown()
        mock.server_close()
        report = {"at": datetime.now(timezone.utc).isoformat(), "host": f"CPA v{args.host_version}",
            "actual_host_version": actual_host_version,
            "platform": platform.system(), "library_sha256": hashlib.sha256(library.read_bytes()).hexdigest(),
            "real_upstream": False, "checks": checks}
        if args.report:
            args.report.parent.mkdir(parents=True, exist_ok=True)
            args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        if not checks or any(not row["passed"] for row in checks):
            # 输出脱敏宿主尾部，便于定位启动/协议失败。
            for line in output[-12:]:
                if not any(term in line.lower() for term in ["bearer", "token", "api_key", "secret"]):
                    print(line.rstrip())
        assert profile.parent == temp_root.resolve() and profile.is_relative_to(temp_root.resolve())
        workspace.cleanup()
    return 0 if args.observe or all(row["passed"] for row in checks) else 1


if __name__ == "__main__":
    raise SystemExit(main())
