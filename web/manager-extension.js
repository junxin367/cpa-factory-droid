import { normalizeQuota, buildTargets, mergeUsage, actual, forecast } from "./manager-usage.js";
import "./manager-models.js";
import { ENTRY_REFRESH_INTERVAL, canRefreshOnEntry, isExpiredWindow, nextCycleRefreshAt, nextCycleWakeAt, relativeReset, restoreRefreshSnapshot, serializeRefreshSnapshot } from "./manager-refresh.js";
(() => {
  // Run in the parent's realm: the plugin iframe is destroyed on route changes.
  if (window.parent !== window || window.__factoryDroidQuotaEnhancement) return;
  const API = "/v0/management";
  const OWN = "data-factory-quota";
  const MAX_AGE = ENTRY_REFRESH_INTERVAL;
  const accounts = new Map();
  const cache = new Map();
  const requests = new Set();
  const queue = [];
  let token = "", scope = "", epoch = 0, running = 0, timer = 0;
  let accountsAt = 0, accountsRequest = null;
  let pending = new Map();
  let pageRoute = "", accountsChecked = false;
  const checkedThisVisit = new Set();
  const checkedCyclesThisVisit = new Map();
  let cycleTimer = 0, cycleTimerAt = 0;
  let snapshotKey = "", snapshotReady = Promise.resolve();

  const accountsPage = () => /^#\/accounts(?:\?|$)/.test(location.hash);
  function updatePageEntry() {
    const active = accountsPage();
    const provider = new URLSearchParams(location.hash.split("?")[1] || "").get("provider") || "all";
    const route = active ? "accounts:" + provider : "";
    if (route !== pageRoute) {
      pageRoute = route; accountsChecked = false; checkedThisVisit.clear();
      checkedCyclesThisVisit.clear(); clearCycleTimer();
    }
    return active;
  }
  function saveSnapshot() {
    if (!snapshotKey) return;
    try { sessionStorage.setItem(snapshotKey, serializeRefreshSnapshot(cache)); } catch { /* Storage may be disabled. */ }
  }
  async function restoreSnapshot(currentToken, currentScope, currentEpoch) {
    try {
      const bytes = new TextEncoder().encode(location.origin + "\0" + currentScope + "\0" + currentToken);
      const hash = await crypto.subtle.digest("SHA-256", bytes);
      if (currentEpoch !== epoch || !authorized()) return;
      snapshotKey = "factory-droid:quota-page:v1:" + [...new Uint8Array(hash)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
      for (let index = sessionStorage.length - 1; index >= 0; index--) {
        const key = sessionStorage.key(index);
        if (key?.startsWith("factory-droid:quota-page:v1:") && key !== snapshotKey) sessionStorage.removeItem(key);
      }
      for (const [key, value] of restoreRefreshSnapshot(sessionStorage.getItem(snapshotKey))) if (!cache.has(key)) cache.set(key, value);
    } catch { /* Continue with in-memory cooldown when storage/crypto is unavailable. */ }
  }

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
    if (snapshotKey) { try { sessionStorage.removeItem(snapshotKey); } catch { /* No storage access. */ } }
    snapshotKey = ""; snapshotReady = Promise.resolve();
    pageRoute = ""; accountsChecked = false; checkedThisVisit.clear();
    checkedCyclesThisVisit.clear(); clearCycleTimer();
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
      snapshotReady = restoreSnapshot(token, scope, epoch);
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
        const account = [file.account, file.email, file.display_account, file.displayAccount].find((value) => typeof value === "string" && value.trim()) || "";
        next.set(key, { key, index, name, account: account.trim(), label: String(file.label || file.note || "").trim() });
      }
      accounts.clear();
      for (const [key, account] of next) accounts.set(key, account);
      for (const key of cache.keys()) if (!next.has(key)) cache.delete(key);
      accountsAt = Date.now();
    })();
    accountsRequest = task;
    try { await task; } finally { if (accountsRequest === task) accountsRequest = null; }
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
    if (!force && !canRefreshOnEntry(previous?.attemptAt)) return Promise.resolve();
    const account = accounts.get(key);
    const currentEpoch = epoch;
    const attemptAt = Date.now();
    const attemptedResets = checkedCyclesThisVisit.get(key) || new Set();
    for (const window of previous?.windows || []) if (isExpiredWindow(window, attemptAt)) attemptedResets.add(window.reset);
    checkedCyclesThisVisit.set(key, attemptedResets);
    cache.set(key, { ...previous, attemptAt, loading: true, error: "", usageError: "", at: previous?.at || 0 });
    saveSnapshot();
    const task = new Promise((resolve) => queue.push({
      epoch: currentEpoch, resolve,
      run: async () => {
        try {
          const data = await request("/plugins/factory-droid/accounts/quota-windows", { auth_index: account.index, name: account.name });
          if (currentEpoch !== epoch || !authorized() || !accounts.has(key)) return;
          let windows = normalizeQuota(data, account);
          const targets = buildTargets(account, windows, previous?.windows);
          const plan = data.data.subscription?.plan;
          const current = { windows, plan: typeof plan === "string" ? plan.trim().slice(0, 100) : null,
            attemptAt, at: Date.now(), loading: false, usageLoading: targets.length > 0, error: "", usageError: "" };
          cache.set(key, current); schedule();
          if (targets.length) {
            try {
              const result = await request("/monitoring/account-window-usage", { windows: targets });
              if (currentEpoch !== epoch || !authorized() || !accounts.has(key)) return;
              windows = mergeUsage(windows, targets, result);
              cache.set(key, { ...current, windows, usageLoading: false });
            } catch {
              if (currentEpoch !== epoch || !authorized() || !accounts.has(key)) return;
              // A newly reset quota must never display the previous cycle's usage.
              cache.set(key, { ...current, usageLoading: false, usageError: "窗口统计读取失败，请重试" });
            }
          }
        } catch (failure) {
          if (currentEpoch === epoch && authorized() && accounts.has(key))
            cache.set(key, { ...previous, attemptAt, loading: false, usageLoading: false, at: previous?.at || 0, error: failure.message });
        } finally {
          if (currentEpoch === epoch && authorized()) saveSnapshot();
          schedule();
        }
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
  function icon(paths, size = "15") {
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    for (const [key, value] of Object.entries({ width: size, height: size, viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "2", "stroke-linecap": "round", "stroke-linejoin": "round", "aria-hidden": "true" })) svg.setAttribute(key, value);
    for (const d of paths || ["M3 12a9 9 0 0 1 15.4-6.4L21 8", "M21 3v5h-5", "M21 12a9 9 0 0 1-15.4 6.4L3 16", "M3 21v-5h5"]) {
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
    const loading = Boolean(cache.get(key)?.loading || cache.get(key)?.usageLoading);
    if (button.disabled !== loading) button.disabled = loading;
    if (button.getAttribute("aria-busy") !== String(loading)) button.setAttribute("aria-busy", String(loading));
  }
  const compactNumber = (number) => new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 1 }).format(number);
  const money = (number) => "$" + new Intl.NumberFormat("en-US", { notation: number >= 1000 ? "compact" : "standard", maximumFractionDigits: 2, minimumFractionDigits: number >= 1000 ? 0 : 2 }).format(number);
  const dateLabel = (ms) => ms ? new Date(ms).toLocaleString("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", hour12: false }) : "—";
  function resetLabel(reset, loading = false) {
    const expired = reset <= Date.now();
    const label = element("span", "fd-quota-reset", expired && loading ? "正在同步…" : relativeReset(reset));
    label.title = (expired ? "等待官方返回新周期；上次重置时间：" : "重置时间：") + new Date(reset).toLocaleString("zh-CN", {
      year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false
    });
    label.setAttribute("aria-label", label.textContent + "，" + label.title);
    return label;
  }
  function emptyUsage(window, value, usage) {
    if (isExpiredWindow(window)) return value.loading ? "正在同步新周期…" : "新周期数据待更新";
    if (!window.start) return "窗口边界未返回";
    if (value.usageLoading) return "正在同步…";
    if (value.usageError) return "统计读取失败";
    if (usage?.scope_match_status && usage.scope_match_status !== "complete") return "统计范围不完整";
    return "窗口统计暂未采集";
  }
  function forecastEmpty(window, value) {
    if (value.error || value.usageError || Date.now() - window.observedAt > 120000 || window.end && Date.now() >= window.end) return "请刷新后查看预测";
    if (!window.start) return "窗口边界未返回，暂不预测";
    if (window.forecastReason === "mixed_core_usage") return "存在 Core 用量，暂不预测";
    if (window.forecastReason === "standard_exhausted") return "标准额度已耗尽，暂不预测";
    if (!window.forecastEligible) return "额度池范围未确认，暂不预测";
    return "暂无可用预测依据";
  }
  function usageColumn(title, subtitle, usage, prediction, message, kind) {
    const column = element("section", "fd-usage-column fd-usage-" + kind);
    column.append(element("strong", "fd-usage-title", title), element("div", "fd-usage-range", subtitle));
    if (!usage && !prediction) column.append(element("div", "fd-usage-empty", message));
    else {
      const metrics = prediction
        ? [["预计请求", compactNumber(prediction.requests)], ["预计 Token", compactNumber(prediction.tokens)], ["预计花费", money(prediction.cost)]]
        : [["请求数", compactNumber(usage.total_requests)], ["Token 数", compactNumber(usage.total_tokens)], ["预估花费", money(usage.total_cost)], ["成功率", usage.success_rate === null ? "—" : (usage.success_rate * 100).toFixed(1) + "%"]];
      for (const [label, text] of metrics) {
        const row = element("div", "fd-usage-metric");
        row.append(element("span", "", label), element("strong", "", text)); column.append(row);
      }
      column.title = prediction
        ? `基于本 CPA 记录与额度消耗比例估算，非官方总容量；${prediction.tokens.toLocaleString("zh-CN")} Token`
        : `该 Factory 凭证在此区间的 CPA 请求；费用采用 Manager 统一价格；${usage.total_tokens.toLocaleString("zh-CN")} Token`;
    }
    return column;
  }
  function usageAmount(kind, cost, tokens, title) {
    const label = element("span", "fd-usage-group fd-usage-" + kind);
    const marker = element("span", "fd-usage-icon");
    marker.append(icon(kind === "actual"
      ? ["M3 3v18h18", "m7 14 4-4 4 3 5-7"]
      : ["m3 17 6-6 4 4 8-10", "M15 5h6v6"], "10"));
    marker.setAttribute("aria-hidden", "true");
    label.append(marker, element("span", "fd-usage-cost", money(cost)),
      element("span", "fd-usage-slash", "/"), element("span", "fd-usage-tokens", compactNumber(tokens)));
    label.title = title;
    label.setAttribute("aria-label", `${kind === "actual" ? "已用" : "预测"} ${money(cost)}，${compactNumber(tokens)} Token`);
    return label;
  }
  function renderQuota(root, key) {
    const value = cache.get(key) || {};
    const detail = root.classList.contains("fd-quota-detail");
    const signature = JSON.stringify([value.at, value.loading, value.usageLoading, value.error, value.usageError, value.windows, value.windows?.map((item) => [relativeReset(item.reset), Date.now() >= item.end || Date.now() - item.observedAt > 120000])]);
    if (root.factorySignature === signature) return;
    root.factorySignature = signature;
    const content = document.createDocumentFragment();
    for (const item of value.windows || []) {
      const row = element("div", "fd-quota-window");
      row.dataset.window = item.name;
      const expired = isExpiredWindow(item);
      const used = expired ? null : actual(item.current);
      const percent = expired ? null : item.percent;
      const predicted = forecast(item, { stale: Boolean(value.error || value.loading), usageError: value.usageError });
      const header = element("div", "fd-quota-header");
      header.append(element("span", "fd-quota-label", item.label), element("strong", "", percent === null ? "—" : `剩余 ${new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 1 }).format(percent)}%`));
      if (detail && item.reset) header.insertBefore(resetLabel(item.reset, value.loading), header.lastChild);
      const track = element("div", "fd-quota-track");
      track.setAttribute("role", "progressbar");
      track.setAttribute("aria-label", item.label + "剩余额度");
      track.setAttribute("aria-valuemin", "0"); track.setAttribute("aria-valuemax", "100");
      if (percent !== null) track.setAttribute("aria-valuenow", String(percent));
      else track.setAttribute("aria-valuetext", "未知");
      const bar = element("span", "fd-quota-bar");
      bar.style.width = `${percent ?? 0}%`;
      bar.dataset.tone = percent === null ? "neutral" : percent <= 0 ? "bad" : percent < 20 ? "warn" : "good";
      track.append(bar); row.append(header, track);
      if (detail) {
        const columns = element("div", "fd-usage-columns");
        columns.append(
          usageColumn(item.previousLabel, item.previousRange ? `${dateLabel(item.previousRange.from)} — ${dateLabel(item.previousRange.to)}` : "—", actual(item.previous), null, emptyUsage(item, value, item.previous), "previous"),
          usageColumn("当前窗口已用", !expired && item.start ? `${dateLabel(item.start)} — ${dateLabel(item.end)}` : "—", used, null, emptyUsage(item, value, item.current), "current"),
          usageColumn("当前窗口预测", "按额度消耗比例估算", null, predicted, forecastEmpty(item, value), "forecast")
        );
        row.append(columns);
        const evidence = element("div", "fd-quota-meta fd-usage-evidence", `额度观测 ${dateLabel(item.observedAt)} · ${item.accuracy === "exact" ? "官方窗口时间" : item.accuracy === "derived" ? "由重置时间推导窗口" : "窗口边界未知"}`);
        evidence.title = "窗口内请求和费用来自 CPA Manager；Core 用量混合、标准额度耗尽或范围不明确时不做容量预测。";
        row.append(evidence);
      } else {
        const line = element("div", "fd-usage-inline");
        const amounts = element("span", "fd-usage-amounts");
        if (used) {
          amounts.append(usageAmount("actual", used.total_cost, used.total_tokens,
            `当前窗口 CPA 用量：${used.total_tokens.toLocaleString("zh-CN")} Token；费用采用 Manager 统一价格。`));
        } else amounts.append(element("span", "fd-quota-meta", emptyUsage(item, value, item.current)));
        if (predicted) {
          amounts.append(usageAmount("prediction", predicted.cost, predicted.tokens,
            "按本 CPA 已用量 ÷ 额度消耗比例估算窗口总容量，非官方余额。"));
        }
        line.append(amounts);
        if (item.reset) line.append(resetLabel(item.reset, value.loading));
        row.append(line);
      }
      content.append(row);
    }
    if (!value.windows?.length) content.append(element("div", "fd-quota-meta", value.loading ? "正在刷新额度…" : value.error ? "" : value.attemptAt && !canRefreshOnEntry(value.attemptAt) ? "3 分钟内已检查，可手动刷新" : "暂无标准额度数据"));
    if (value.error) {
      const message = element("div", "fd-quota-error", value.error + (value.windows?.length ? " · 显示上次结果" : ""));
      message.setAttribute("role", "status"); content.append(message);
    }
    if (value.usageError) {
      const message = element("div", "fd-quota-error", value.usageError);
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
      const label = "查看 Factory 标准额度 · " + value.windows.map((item) => `${item.label}：${isExpiredWindow(item) ? "新周期待更新" : item.percent === null ? "未知" : `剩余 ${Math.round(item.percent * 10) / 10}%`}`).join(" · ");
      if (trigger.title !== label) trigger.title = label;
      if (trigger.getAttribute("aria-label") !== label) trigger.setAttribute("aria-label", label);
    }
  }
  function mountPlan(card, key) {
    const badges = card.querySelector('[class*="accountGridCardMetaBadges"]');
    let badge = card.querySelector(`[${OWN}="plan"]`);
    const plan = cache.get(key)?.plan;
    // Let Manager own the label when it gains native Factory plan support.
    if (!badges || !plan || badges.querySelector(`[class*="accountPlanBadge"]:not([${OWN}])`)) {
      badge?.remove();
      return;
    }
    if (!badge) {
      badge = element("span", "fd-plan-badge");
      badge.setAttribute(OWN, "plan");
    }
    if (badge.textContent !== plan) {
      badge.textContent = plan;
      badge.title = "Factory 套餐：" + plan;
    }
    if (badges.firstChild !== badge) badges.prepend(badge);
  }
  function scan() {
    if (!authorized() || !accountsPage()) return;
    const visibleKeys = new Set();
    for (const card of document.querySelectorAll("article[data-account-card]")) {
      const key = card.getAttribute("data-account-card");
      if (!accounts.has(key)) continue;
      visibleKeys.add(key);
      mountPlan(card, key);
      ensureButton(card.querySelector('[class*="accountGridCardPrimaryActions"], [class*="accountQuickActionsGrid"]'), key, true);
      const empty = card.querySelector("[data-account-quota-empty]");
      if (empty) {
        if (!empty.hasAttribute("data-factory-quota-hidden")) empty.setAttribute("data-factory-quota-hidden", "");
        mountQuota(empty.parentElement, key);
        refreshOnEntry(key);
      }
    }
    const params = new URLSearchParams(location.hash.split("?")[1] || "");
    const key = params.get("account");
    if (accounts.has(key)) visibleKeys.add(key);
    planCycleRefresh(visibleKeys);
    if (!accounts.has(key)) return;
    const provider = document.querySelector('[data-account-provider-icon="factory-droid"]');
    const drawer = provider?.closest('[role="dialog"]');
    if (!drawer) return;
    ensureButton(drawer.querySelector('[class*="drawerActions"]'), key, false);
    const section = drawer.querySelector('[data-account-quota-tab] [data-quota-window-group="standard"]');
    if (section && !section.querySelector("[data-quota-card-mode]")) {
      const empty = section.querySelector("p");
      if (empty && !empty.hasAttribute("data-factory-quota-hidden")) empty.setAttribute("data-factory-quota-hidden", "");
      mountQuota(section, key, true); refreshOnEntry(key);
    }
    const historyButton = drawer.querySelector('[class*="quotaTabActions"] button');
    if (historyButton) {
      historyButton.factoryAccountKey = key;
      if (!historyButton.factoryQuotaListener) {
        historyButton.factoryQuotaListener = () => { void refresh(historyButton.factoryAccountKey, true); };
        historyButton.addEventListener("click", historyButton.factoryQuotaListener);
      }
    }
  }
  function refreshOnEntry(key) {
    if (checkedThisVisit.has(key)) return;
    checkedThisVisit.add(key);
    void refresh(key);
  }
  function clearCycleTimer() {
    if (cycleTimer) clearTimeout(cycleTimer);
    cycleTimer = 0; cycleTimerAt = 0;
  }
  function planCycleRefresh(keys) {
    if (document.visibilityState === "hidden") { clearCycleTimer(); return; }
    let nextAt = Infinity;
    const now = Date.now();
    for (const key of keys) {
      const due = nextCycleRefreshAt(cache.get(key), checkedCyclesThisVisit.get(key));
      if (due !== null && due <= now) void refresh(key);
      // Render the boundary even if the request is still in its cooldown:
      // old usage must disappear as soon as its observed cycle has ended.
      const wake = nextCycleWakeAt(cache.get(key), checkedCyclesThisVisit.get(key), now);
      if (wake !== null && wake > now) nextAt = Math.min(nextAt, wake);
    }
    if (!Number.isFinite(nextAt)) { clearCycleTimer(); return; }
    if (cycleTimer && cycleTimerAt === nextAt) return;
    clearCycleTimer();
    cycleTimerAt = nextAt;
    cycleTimer = setTimeout(() => {
      cycleTimer = 0; cycleTimerAt = 0; schedule();
    }, Math.min(2147483647, Math.max(1, nextAt - Date.now() + 25)));
  }
  async function refreshVisibleAccounts() {
    if (!accountsPage() || !authorized()) return;
    const currentEpoch = epoch;
    const keys = new Set([...document.querySelectorAll("article[data-account-card]")].map((card) => card.getAttribute("data-account-card")));
    const selected = new URLSearchParams(location.hash.split("?")[1] || "").get("account");
    if (selected) keys.add(selected);
    accountsAt = 0;
    try { await loadAccounts(); } catch { return; }
    if (currentEpoch !== epoch || !accountsPage() || !authorized()) return;
    for (const key of keys) {
      if (!accounts.has(key)) continue;
      checkedThisVisit.add(key);
      void refresh(key, true);
    }
    schedule();
  }
  function schedule() {
    if (timer) return;
    timer = setTimeout(async () => {
      timer = 0;
      if (!authorized() || !updatePageEntry()) return;
      await snapshotReady;
      if (!accountsPage() || !authorized()) return;
      if (!accountsChecked) {
        // One discovery attempt per entry; DOM changes never trigger polling.
        accountsChecked = true;
        try { await loadAccounts(); } catch { return; }
      }
      scan();
    }, 80);
  }
  const observer = new MutationObserver((records) => {
    if (records.some((record) => !record.target.closest?.(`[${OWN}]`))) schedule();
  });
  observer.observe(document.body, { subtree: true, childList: true });
  const routeChanged = () => { updatePageEntry(); authorized(); schedule(); };
  addEventListener("hashchange", routeChanged);
  addEventListener("popstate", routeChanged);
  addEventListener("storage", (event) => { if (event.key === "cli-proxy-auth" || event.key === null) { authorized(); schedule(); } });
  addEventListener("unauthorized", disconnect);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") clearCycleTimer();
    else schedule();
  });
  document.addEventListener("click", (event) => {
    const button = event.target.closest?.("button");
    if (!button || button.hasAttribute(OWN)) return;
    const label = button.getAttribute("aria-label") || button.title || button.textContent.trim();
    if (label === "刷新" || label === "刷新全部") void refreshVisibleAccounts();
  });
  // No polling interval. In addition to entry/manual refresh, an observed
  // cycle boundary permits one refresh, respecting the same 3-minute cooldown.
  const stylesheet = document.createElement("link");
  stylesheet.rel = "stylesheet";
  stylesheet.href = new URL("manager-extension.css", import.meta.url).href;
  document.head.append(stylesheet);
  window.__factoryDroidQuotaEnhancement = Object.freeze({ connect, disconnect });
})();
