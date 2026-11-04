"""使用固定版本的真实 CPA 宿主验证安装库，只连接本机模拟上游。"""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
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
    inference_calls = 0
    upgrade_calls = 0
    lock = threading.Lock()

    def log_message(self, *_):
        pass

    def reply(self, value, content_type="application/json"):
        raw = value.encode() if isinstance(value, str) else json.dumps(value).encode()
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

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
        self.reply({"userId": "mock-user", "email": "mock@example.test", "orgId": "mock-org", "region": "us"})

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
            return self.reply({"access_token": f"mock-access-token-{Mock.rotations}",
                "refresh_token": f"mock-refresh-token-{Mock.rotations}", "expires_in": 3600,
                "organization_id": "org_mock", "user": {"id": "user_mock"}})
        data = json.loads(raw)
        with self.lock:
            Mock.inference_calls += 1
            if self.headers.get("Upgrade", "").strip() or any(
                token.strip().lower() == "upgrade" for token in self.headers.get("Connection", "").split(",")
            ):
                Mock.upgrade_calls += 1
        probe = {"session": self.headers.get("x-session-id"), "cache": data.get("prompt_cache_key"),
            "safety": data.get("safety_identifier"), "input": data.get("input"), "messages": data.get("messages"),
            "conversation": data.get("conversation"), "previous_response_id": data.get("previous_response_id"),
            "metadata": data.get("metadata"), "cache_control": data.get("cache_control"),
            "user_agent": self.headers.get("User-Agent"), "forwarded": self.headers.get("X-Forwarded-For")}
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
    def __init__(self, path, status, code):
        super().__init__(f"CPA {path} 返回 HTTP {status}")
        self.status = status
        self.code = code


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--library", required=True, type=Path)
    parser.add_argument("--host-version", choices=tuple(HOST_PACKAGES), default="8.0.52")
    parser.add_argument("--host", type=Path, help="复用与 --host-version 一致的已核验宿主")
    parser.add_argument("--report", type=Path)
    parser.add_argument("--observe", action="store_true", help="记录旧版本失败项，用于修复前对照")
    parser.add_argument("--registration-only", action="store_true", help="仅验证宿主版本、注册和菜单能力")
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
                        code = json.loads(raw).get("error", {}).get("code")
                    except (ValueError, AttributeError):
                        code = None
                    raise CPARequestError(path, response.status, code)
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

        def call(text="A", headers=None, key=None, protocol="responses", stream=False, **fields):
            paths = {"responses": "/v1/responses", "anthropic": "/v1/messages", "chat": "/v1/chat/completions"}
            body = ({"model": "factory/gpt-5.4", "input": text, "store": False} if protocol == "responses" else
                {"model": "factory/claude-opus-5-5" if protocol == "anthropic" else "factory/glm-5.3",
                    "messages": [{"role": "user", "content": text}], "max_tokens": 32})
            result = request(paths[protocol], {**body, "stream": stream, **fields}, key or client, headers, stream)
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
        update_auth(oauth_file, expires_at="2020-01-01T00:00:00Z", last_refresh="2019-01-01T00:00:00Z")
        stop()
        start()
        check("OAuth 到期后轮换并持久化令牌", bool(wait_until(lambda: Mock.rotations > 0
            and json.loads(oauth_file.read_text(encoding="utf-8")).get("refresh_token") == f"mock-refresh-token-{Mock.rotations}", 20)))
        persisted = (profile / "data/state.json").read_text(encoding="utf-8")
        check("插件状态不保存凭证或令牌", all(secret not in persisted for secret in
            ["fk-local-integration-only", "mock-access-token-", "mock-refresh-token-"]))
        check("最终目录和健康状态正常", bool(wait_until(lambda: status()["health"] == "ready")) and models() == original_models)
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
