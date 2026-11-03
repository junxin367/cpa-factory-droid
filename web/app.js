"use strict";
(() => {
  const $ = (id) => document.getElementById(id);
  const API = "/v0/management/plugins/factory-droid";
  const state = { token: "", epoch: 0, busy: false, connected: false };
  const requests = new Set();
  function error(id, message = "") {
    $(id).textContent = message;
    $(id).hidden = !message;
  }
  function sanitize(message, secrets = []) {
    let text = String(message || "操作失败").slice(0, 500);
    for (const secret of [state.token, ...secrets]) {
      if (secret) text = text.split(secret).join("[已脱敏]");
    }
    return text.replace(/\b(?:fk-|sk-|cpamp_)[\w-]{8,}/gi, "[已脱敏]");
  }
  async function api(path, method = "GET", body, token = state.token) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 90000);
    requests.add(controller);
    try {
      const response = await fetch(API + path, {
        method, credentials: "omit", cache: "no-store", redirect: "error",
        signal: controller.signal,
        headers: { Authorization: "Bearer " + token, Accept: "application/json",
          ...(body ? { "Content-Type": "application/json" } : {}) },
        ...(body ? { body: JSON.stringify(body) } : {})
      });
      const data = await response.json();
      if (!response.ok || !data.ok) {
        if ([401, 403].includes(response.status) && !data.error?.code && token === state.token) disconnect();
        throw new Error(sanitize(data.error?.message || ([401, 403].includes(response.status) ? "管理授权失败，请检查当前管理器密钥。" : "CPA 请求失败。"), [token, body?.api_key]));
      }
      return data.data;
    } catch (failure) {
      if (failure.name === "AbortError") throw new Error("请求超时，请在 CPA 凭证管理中确认添加结果。");
      throw failure;
    } finally {
      clearTimeout(timer);
      requests.delete(controller);
    }
  }
  function setConnected(status) {
    state.connected = true;
    $("account-fields").disabled = state.busy;
    $("authorize").textContent = "管理授权";
    $("connection-status").textContent = `已连接 · ${status.account_count ?? 0} 个凭证`;
    $("connection-status").classList.add("connected");
    $("version").textContent = status.version ? "v" + status.version : "CPA 凭证接入";
    $("disconnect").hidden = false;
  }
  function disconnect() {
    state.epoch++;
    for (const request of requests) request.abort();
    state.token = "";
    state.connected = false;
    $("account-fields").disabled = true;
    $("account-key").value = "";
    $("account-key").type = "password";
    $("show-key").textContent = "显示";
    $("show-key").setAttribute("aria-pressed", "false");
    $("show-key").setAttribute("aria-label", "显示 Factory API Key");
    $("account-success").hidden = true;
    $("connection-status").textContent = "尚未连接";
    $("connection-status").classList.remove("connected");
    $("authorize").textContent = "连接管理端";
    $("disconnect").hidden = true;
  }
  $("authorize").onclick = () => { error("auth-error"); $("auth-dialog").showModal(); };
  $("close-auth").onclick = () => { if (!state.busy) $("auth-dialog").close(); };
  $("auth-dialog").addEventListener("close", () => { $("management-key").value = ""; });
  $("auth-dialog").addEventListener("cancel", (event) => { if (state.busy) event.preventDefault(); });
  $("auth-dialog").onclick = (event) => {
    if (event.target !== $("auth-dialog") || state.busy) return;
    const bounds = $("auth-dialog").getBoundingClientRect();
    if (event.clientX < bounds.left || event.clientX > bounds.right || event.clientY < bounds.top || event.clientY > bounds.bottom) $("auth-dialog").close();
  };
  $("disconnect").onclick = () => { disconnect(); $("auth-dialog").close(); };
  $("auth-form").onsubmit = async (event) => {
    event.preventDefault();
    if (state.busy) return;
    const token = $("management-key").value.trim();
    if (!token) return;
    state.busy = true;
    $("connect").disabled = true;
    $("close-auth").disabled = true;
    $("disconnect").disabled = true;
    error("auth-error");
    try {
      const status = await api("/status", "GET", undefined, token);
      disconnect();
      state.token = token;
      setConnected(status);
      $("auth-dialog").close();
    } catch (failure) { error("auth-error", sanitize(failure.message, [token])); }
    finally {
      state.busy = false;
      $("connect").disabled = false;
      $("close-auth").disabled = false;
      $("disconnect").disabled = false;
      $("account-fields").disabled = !state.connected;
    }
  };
  $("show-key").onclick = () => {
    const show = $("account-key").type === "password";
    $("account-key").type = show ? "text" : "password";
    $("show-key").textContent = show ? "隐藏" : "显示";
    $("show-key").setAttribute("aria-pressed", String(show));
    $("show-key").setAttribute("aria-label", (show ? "隐藏" : "显示") + " Factory API Key");
  };
  $("account-form").onsubmit = async (event) => {
    event.preventDefault();
    if (state.busy || !state.connected) return;
    const key = $("account-key").value.trim();
    const label = $("account-label").value.trim();
    if (!label || !/^fk-[\x21-\x7e]{9,}$/.test(key)) return error("account-error", "请输入名称和有效的 Factory API Key。");
    state.busy = true;
    $("account-fields").disabled = true;
    $("authorize").disabled = true;
    $("add-account").textContent = "正在添加并同步模型…";
    $("account-success").hidden = true;
    error("account-error");
    const epoch = state.epoch;
    try {
      const account = await api("/accounts", "POST", { label, api_key: key, enabled: true });
      if (epoch !== state.epoch) return;
      $("account-key").value = "";
      $("account-key").type = "password";
      $("show-key").textContent = "显示";
      $("show-key").setAttribute("aria-pressed", "false");
      $("show-key").setAttribute("aria-label", "显示 Factory API Key");
      $("success-message").textContent = `已添加 ${account.label} · ${account.ready_model_count} 个模型`;
      $("account-success").hidden = false;
      try { setConnected(await api("/status")); } catch { /* 添加结果保留，避免引导重复提交。 */ }
    } catch (failure) {
      if (epoch === state.epoch) error("account-error", sanitize(failure.message, [key]));
    } finally {
      state.busy = false;
      $("account-fields").disabled = !state.connected;
      $("authorize").disabled = false;
      $("add-account").textContent = "添加凭证";
    }
  };
  async function initializeManagerLinks() {
    let plus = false;
    try {
      const response = await fetch("/usage-service/info", { cache: "no-store", credentials: "omit" });
      if (response.ok) plus = (await response.json()).service === "cpa-manager-plus";
    } catch { /* CPA 官方管理端没有 Manager Server 的识别接口。 */ }
    const href = "/management.html#/" + (plus ? "accounts?provider=factory-droid&layout=grid" : "auth-files");
    $("manage-credentials").href = href;
    $("success-link").href = href;
    $("oauth-login").href = "/management.html#/oauth" + (plus ? "#oauth-provider-factory-droid" : "");
  }
  function initializeTheme() {
    const media = matchMedia("(prefers-color-scheme: dark)");
    let parentRoot = null;
    try { if (window.parent !== window && window.parent.location.origin === location.origin) parentRoot = window.parent.document.documentElement; }
    catch { /* 跨源 iframe 使用系统主题。 */ }
    const apply = () => {
      if (document.documentElement.getAttribute("data-cpamp-plugin-host") === "true") return;
      const theme = parentRoot?.getAttribute("data-theme");
      const dark = parentRoot ? theme === "dark" : media.matches;
      document.documentElement.setAttribute("data-theme", dark ? "dark" : "light");
    };
    apply();
    media.addEventListener("change", apply);
    if (parentRoot) new MutationObserver(apply).observe(parentRoot, { attributes: true, attributeFilter: ["data-theme"] });
  }
  function readSavedHostSession() {
    try {
      const raw = localStorage.getItem("cli-proxy-auth");
      if (!raw) return "";
      let decoded = raw;
      const prefix = raw.startsWith("enc::v2::") ? "enc::v2::" : raw.startsWith("enc::v1::") ? "enc::v1::" : "";
      if (prefix) {
        const salt = "cli-proxy-api-webui::secure-storage";
        const key = new TextEncoder().encode(prefix === "enc::v2::" ? salt + "|v2|" + location.host : salt + "|" + location.host + "|" + navigator.userAgent);
        const bytes = Uint8Array.from(atob(raw.slice(prefix.length)), (character) => character.charCodeAt(0));
        for (let index = 0; index < bytes.length; index++) bytes[index] ^= key[index % key.length];
        decoded = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
      }
      const persisted = JSON.parse(decoded);
      const saved = persisted?.state || persisted;
      if (saved?.rememberPassword !== true || typeof saved?.managementKey !== "string") return "";
      if (saved.apiBase) {
        const base = new URL(saved.apiBase, location.origin);
        if (base.origin !== location.origin || !["", "/", "/v0/management", "/v0/management/"].includes(base.pathname)) return "";
      }
      return saved.managementKey.trim();
    } catch { return ""; }
  }

  initializeTheme();
  initializeManagerLinks();
  async function restoreHostSession() {
    const token = readSavedHostSession();
    if (!token || state.busy || state.connected) return;
    const epoch = state.epoch;
    try {
      const status = await api("/status", "GET", undefined, token);
      if (epoch !== state.epoch || state.busy || state.connected) return;
      state.token = token;
      setConnected(status);
    } catch { /* 用户可通过连接管理端输入当前密钥。 */ }
  }
  restoreHostSession();
  addEventListener("storage", (event) => {
    if (event.key !== "cli-proxy-auth") return;
    disconnect();
    restoreHostSession();
  });
})();
