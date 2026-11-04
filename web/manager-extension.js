"use strict";
(() => {
  // Run in the parent's realm: the plugin iframe is destroyed on route changes.
  if (window.parent !== window || window.__factoryDroidQuotaEnhancement) return;
  const API = "/v0/management";
  const OWN = "data-factory-quota";
  const MAX_AGE = 60000;
  const accounts = new Map();
  const cache = new Map();
  const requests = new Set();
  const queue = [];
  let token = "", scope = "", epoch = 0, running = 0, timer = 0;
  let accountsAt = 0, accountsRequest = null;
  let pending = new Map();

  function sessionScope() {
    try {
      const raw = localStorage.getItem("cli-proxy-auth");
      if (!raw) return "";
      let text = raw;
      const prefix = raw.startsWith("enc::v2::") ? "enc::v2::" : raw.startsWith("enc::v1::") ? "enc::v1::" : "";
      if (prefix) {
        const salt = "cli-proxy-api-webui::secure-storage";
        const key = new TextEncoder().encode(prefix === "enc::v2::" ? salt + "|v2|" + location.host : salt + "|" + location.host + "|" + navigator.userAgent);
        const bytes = Uint8Array.from(atob(raw.slice(prefix.length)), (c) => c.charCodeAt(0));
        for (let i = 0; i < bytes.length; i++) bytes[i] ^= key[i % key.length];
        text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
      }
      const parsed = JSON.parse(text);
      const saved = parsed?.state || parsed;
      if (!saved.apiBase) return "";
      const base = new URL(saved.apiBase, location.origin);
      if (base.origin !== location.origin || !["/", "/v0/management", "/v0/management/"].includes(base.pathname)) return "";
      // Server version updates also persist this store; they are not a session change.
      return JSON.stringify([base.href, saved.managementKey || "", saved.rememberPassword === true, saved.sessionMode || "", saved.sessionPanelBase || ""]);
    } catch { return ""; }
  }
  function removeUI() {
    document.querySelectorAll(`[${OWN}]`).forEach((node) => node.remove());
    document.querySelectorAll("[data-factory-quota-hidden]").forEach((node) => node.removeAttribute("data-factory-quota-hidden"));
    document.querySelectorAll("[data-factory-quota-label]").forEach((node) => {
      for (const [name, value] of Object.entries(node.factoryQuotaLabels || {})) {
        if (value === null) node.removeAttribute(name);
        else node.setAttribute(name, value);
      }
      node.removeAttribute("data-factory-quota-label");
      delete node.factoryQuotaLabels;
    });
  }
  function disconnect() {
    epoch++;
    token = ""; scope = ""; accountsAt = 0; accountsRequest = null;
    for (const controller of requests) controller.abort();
    requests.clear(); accounts.clear(); cache.clear(); pending = new Map();
    queue.splice(0).forEach((job) => job.resolve());
    removeUI();
  }
  function authorized() {
    if (token && (scope !== sessionScope() || /^#\/login(?:[/?]|$)/.test(location.hash))) disconnect();
    return Boolean(token);
  }
  function connect(nextToken) {
    const nextScope = sessionScope();
    if (typeof nextToken !== "string" || !nextToken.trim() || !nextScope) return false;
    if (token !== nextToken || scope !== nextScope) {
      disconnect();
      token = nextToken; scope = nextScope;
    }
    schedule();
    return true;
  }
  async function request(path, body) {
    if (!authorized()) throw new Error("请先打开 Factory 页连接管理端");
    const currentEpoch = epoch;
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 30000);
    requests.add(controller);
    try {
      const response = await fetch(API + path, {
        method: body ? "POST" : "GET", credentials: "omit", cache: "no-store", redirect: "error",
        headers: { Authorization: "Bearer " + token, Accept: "application/json", ...(body ? { "Content-Type": "application/json" } : {}) },
        ...(body ? { body: JSON.stringify(body) } : {}), signal: controller.signal
      });
      if (currentEpoch !== epoch || !authorized()) throw new Error("管理连接已变更");
      if ([401, 403].includes(response.status)) {
        // The quota endpoint can also relay an upstream 401; do not log the Manager out.
        throw new Error("授权失败，请检查管理连接或 Factory 凭证");
      }
      if (!response.ok) throw new Error(response.status === 429 ? "请求过于频繁，请稍后刷新" : `刷新失败（HTTP ${response.status}），请重试`);
      return await response.json();
    } catch (failure) {
      if (failure.name === "AbortError") throw new Error("刷新超时，请重试");
      if (failure instanceof TypeError) throw new Error("连接失败，请重试");
      throw failure;
    } finally {
      clearTimeout(timeout); requests.delete(controller);
    }
  }
  async function loadAccounts() {
    if (accountsRequest) return accountsRequest;
    if (Date.now() - accountsAt < MAX_AGE) return;
    const currentEpoch = epoch;
    const task = (async () => {
      const data = await request("/auth-files");
      if (currentEpoch !== epoch || !authorized()) return;
      if (!Array.isArray(data.files)) throw new Error("无法读取凭证列表");
      const next = new Map();
      for (const file of data.files) {
        const provider = String(file.provider || file.type || "").trim().toLowerCase();
        const index = String(file.authIndex ?? file.auth_index ?? file["auth-index"] ?? "").trim();
        const name = String(file.physicalName || file.name || "").trim();
        if (provider !== "factory-droid" || !index || !name) continue;
        const key = name + "\0" + index;
        next.set(key, { key, index });
      }
      accounts.clear();
      for (const [key, account] of next) accounts.set(key, account);
      for (const key of cache.keys()) if (!next.has(key)) cache.delete(key);
      accountsAt = Date.now();
    })();
    accountsRequest = task;
    try { await task; } finally { if (accountsRequest === task) accountsRequest = null; }
  }
  function normalizeQuota(data) {
    const quota = data?.quota || data?.data || data;
    if (!Array.isArray(quota?.groups)) throw new Error("Factory 未返回可识别的额度");
    const windows = [];
    for (const group of quota.groups) {
      // Only official Standard quotas. Never infer a balance from local usage.
      if (!/标准|standard/i.test(group.displayName || "") || /core/i.test(group.displayName || "")) continue;
      for (const bucket of group.buckets || []) {
        const rawName = String(bucket.window || "");
        const name = { "标准 5 小时": "5h", "标准 7 天": "7d", "标准 30 天": "30d" }[rawName] || rawName;
        const label = { "5h": "5 小时", "7d": "7 天", "30d": "标准月额度" }[name];
        if (!label || windows.some((item) => item.name === name)) continue;
        const fraction = bucket.remainingFraction;
        const percent = typeof fraction === "number" && Number.isFinite(fraction)
          ? Math.max(0, Math.min(100, fraction * 100)) : null;
        const reset = Date.parse(bucket.resetTime);
        windows.push({ name, label, percent, reset: Number.isFinite(reset) ? reset : null });
      }
    }
    return windows.sort((a, b) => ["5h", "7d", "30d"].indexOf(a.name) - ["5h", "7d", "30d"].indexOf(b.name));
  }
  function pump() {
    while (running < 2 && queue.length) {
      const job = queue.shift();
      if (job.epoch !== epoch) { job.resolve(); continue; }
      running++;
      job.run().finally(() => { running--; job.resolve(); pump(); });
    }
  }
  function refresh(key, force = false) {
    if (!authorized() || !accounts.has(key)) return Promise.resolve();
    if (pending.has(key)) return pending.get(key);
    const previous = cache.get(key);
    // Failed automatic loads wait for an explicit click, avoiding a retry loop.
    if (!force && previous && (previous.error || Date.now() - previous.at < MAX_AGE)) return Promise.resolve();
    const account = accounts.get(key);
    const currentEpoch = epoch;
    cache.set(key, { ...previous, loading: true, error: "", at: previous?.at || 0 });
    const task = new Promise((resolve) => queue.push({
      epoch: currentEpoch, resolve,
      run: async () => {
        try {
          const data = await request("/plugins/factory-droid/quota", { auth_index: account.index });
          if (currentEpoch !== epoch || !authorized() || !accounts.has(key)) return;
          cache.set(key, { windows: normalizeQuota(data), at: Date.now(), loading: false, error: "" });
        } catch (failure) {
          if (currentEpoch === epoch && authorized() && accounts.has(key))
            cache.set(key, { ...previous, loading: false, at: previous?.at || 0, error: failure.message });
        } finally { schedule(); }
      }
    }));
    pending.set(key, task);
    task.finally(() => { if (pending.get(key) === task) pending.delete(key); });
    pump(); schedule();
    return task;
  }
  function element(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }
  function icon() {
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    for (const [key, value] of Object.entries({ width: "15", height: "15", viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "2", "stroke-linecap": "round", "stroke-linejoin": "round", "aria-hidden": "true" })) svg.setAttribute(key, value);
    for (const d of ["M3 12a9 9 0 0 1 15.4-6.4L21 8", "M21 3v5h-5", "M21 12a9 9 0 0 1-15.4 6.4L3 16", "M3 21v-5h5"]) {
      const path = document.createElementNS(svg.namespaceURI, "path");
      path.setAttribute("d", d); svg.append(path);
    }
    return svg;
  }
  function ensureButton(container, key, compact) {
    if (!container) return;
    let button = container.querySelector(`[${OWN}="refresh"]`);
    // Yield to future Manager versions that implement Factory quota natively.
    if (!button && [...container.querySelectorAll("button")].some((node) => /刷新额度|Refresh quota/i.test(node.getAttribute("aria-label") || node.title || node.textContent))) return;
    if (!button) {
      button = element("button", "btn btn-secondary btn-sm" + (compact ? " btn-icon-only" : ""));
      button.type = "button"; button.setAttribute(OWN, "refresh");
      button.title = "刷新额度"; button.setAttribute("aria-label", "刷新额度");
      button.append(icon());
      if (!compact) button.append(document.createTextNode("刷新额度"));
      button.onclick = (event) => { event.preventDefault(); event.stopPropagation(); void refresh(button.factoryAccountKey, true); };
      container.prepend(button);
    }
    button.factoryAccountKey = key;
    const loading = Boolean(cache.get(key)?.loading);
    if (button.disabled !== loading) button.disabled = loading;
    if (button.getAttribute("aria-busy") !== String(loading)) button.setAttribute("aria-busy", String(loading));
  }
  function renderQuota(root, key) {
    const value = cache.get(key) || {};
    const signature = JSON.stringify([value.at, value.loading, value.error, value.windows]);
    if (root.factorySignature === signature) return;
    root.factorySignature = signature;
    const content = document.createDocumentFragment();
    for (const item of value.windows || []) {
      const row = element("div", "fd-quota-window");
      row.dataset.window = item.name;
      const header = element("div", "fd-quota-header");
      header.append(element("span", "fd-quota-label", item.label), element("strong", "", item.percent === null ? "—" : `剩余 ${new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 1 }).format(item.percent)}%`));
      const track = element("div", "fd-quota-track");
      track.setAttribute("role", "progressbar");
      track.setAttribute("aria-label", item.label + "剩余额度");
      track.setAttribute("aria-valuemin", "0"); track.setAttribute("aria-valuemax", "100");
      if (item.percent !== null) track.setAttribute("aria-valuenow", String(item.percent));
      else track.setAttribute("aria-valuetext", "未知");
      const bar = element("span", "fd-quota-bar");
      bar.style.width = `${item.percent ?? 0}%`;
      bar.dataset.tone = item.percent === null ? "neutral" : item.percent <= 0 ? "bad" : item.percent < 20 ? "warn" : "good";
      track.append(bar); row.append(header, track);
      if (item.reset) {
        const time = new Date(item.reset).toLocaleString("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", hour12: false });
        row.append(element("div", "fd-quota-meta", (item.reset <= Date.now() ? "窗口已到期 · " : "重置于 ") + time));
      }
      content.append(row);
    }
    if (!value.windows?.length) content.append(element("div", "fd-quota-meta", value.loading ? "正在刷新额度…" : value.error ? "" : "暂无标准额度数据"));
    if (value.error) {
      const message = element("div", "fd-quota-error", value.error + (value.windows?.length ? " · 显示上次结果" : ""));
      message.setAttribute("role", "status"); content.append(message);
    }
    root.replaceChildren(content);
  }
  function mountQuota(container, key, detail = false) {
    if (!container) return;
    let root = container.querySelector(`[${OWN}="windows"]`);
    if (!root) {
      root = element("div", "fd-quota-windows" + (detail ? " fd-quota-detail" : ""));
      root.setAttribute(OWN, "windows"); container.append(root);
    }
    renderQuota(root, key);
    const value = cache.get(key);
    const trigger = !detail && container.closest('button, [role="button"]');
    if (trigger && value?.windows?.length) {
      if (!trigger.hasAttribute("data-factory-quota-label")) {
        trigger.factoryQuotaLabels = { title: trigger.getAttribute("title"), "aria-label": trigger.getAttribute("aria-label") };
        trigger.setAttribute("data-factory-quota-label", "");
      }
      const label = "查看 Factory 标准额度 · " + value.windows.map((item) => `${item.label}：${item.percent === null ? "未知" : `剩余 ${Math.round(item.percent * 10) / 10}%`}`).join(" · ");
      if (trigger.title !== label) trigger.title = label;
      if (trigger.getAttribute("aria-label") !== label) trigger.setAttribute("aria-label", label);
    }
  }
  function scan() {
    if (!authorized() || !location.hash.startsWith("#/accounts")) return;
    for (const card of document.querySelectorAll("article[data-account-card]")) {
      const key = card.getAttribute("data-account-card");
      if (!accounts.has(key)) continue;
      ensureButton(card.querySelector('[class*="accountGridCardPrimaryActions"], [class*="accountQuickActionsGrid"]'), key, true);
      const empty = card.querySelector("[data-account-quota-empty]");
      if (empty) {
        if (!empty.hasAttribute("data-factory-quota-hidden")) empty.setAttribute("data-factory-quota-hidden", "");
        mountQuota(empty.parentElement, key);
        void refresh(key);
      }
    }
    const params = new URLSearchParams(location.hash.split("?")[1] || "");
    const key = params.get("account");
    if (!accounts.has(key)) return;
    const provider = document.querySelector('[data-account-provider-icon="factory-droid"]');
    const drawer = provider?.closest('[role="dialog"]');
    if (!drawer) return;
    ensureButton(drawer.querySelector('[class*="drawerActions"]'), key, false);
    const section = drawer.querySelector('[data-account-quota-tab] [data-quota-window-group="standard"]');
    if (section && !section.querySelector("[data-quota-card-mode]")) {
      const empty = section.querySelector("p");
      if (empty && !empty.hasAttribute("data-factory-quota-hidden")) empty.setAttribute("data-factory-quota-hidden", "");
      mountQuota(section, key, true); void refresh(key);
    }
  }
  function schedule() {
    if (timer) return;
    timer = setTimeout(async () => {
      timer = 0;
      if (!authorized() || !location.hash.startsWith("#/accounts")) return;
      try { await loadAccounts(); } catch { return; }
      scan();
    }, 80);
  }
  const observer = new MutationObserver((records) => {
    if (records.some((record) => !record.target.closest?.(`[${OWN}]`))) schedule();
  });
  observer.observe(document.body, { subtree: true, childList: true });
  addEventListener("hashchange", schedule);
  addEventListener("popstate", schedule);
  addEventListener("storage", (event) => { if (event.key === "cli-proxy-auth" || event.key === null) { authorized(); schedule(); } });
  addEventListener("unauthorized", disconnect);
  // Also detect same-tab logout/connection switches without patching Manager's stores.
  setInterval(() => { if (authorized() && !document.hidden) schedule(); }, 15000);
  const stylesheet = document.createElement("link");
  stylesheet.rel = "stylesheet";
  stylesheet.href = new URL("manager-extension.css", document.currentScript.src).href;
  document.head.append(stylesheet);
  window.__factoryDroidQuotaEnhancement = Object.freeze({ connect, disconnect });
})();
