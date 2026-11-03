"use strict";

(() => {
  const API = "/v0/management/plugins/factory-droid";
  const $ = (id) => document.getElementById(id);
  const state = {
    token: "", credentialSource: "", connected: false, epoch: 0, loading: false,
    status: null, accounts: [], models: [], settings: null, usage: null,
    view: "accounts", accountSearch: "", accountFilter: "all", modelSearch: "",
    usageAccount: "", usageKind: "all", pinnedAccount: "", editingAccount: null, deletingAccount: null,
    modelsSection: "access", modelAccount: "", accountModelCache: new Map(), accessLoading: false,
    accessError: null, accessLoadSequence: 0, accessSearch: "", candidateSearch: "", candidatePage: 1,
    candidateRoutes: new Map(), verifyTarget: null, verifyPublicationOnly: false,
    modelDraft: new Map(), budgetDirty: false, busy: new Set(), requests: new Set(),
    createDraft: null, createModels: new Map(), createSequence: 0, createSearch: "", createShowRoutes: false,
    createCatalogError: "", createProgress: "", createCommitPending: false, createKeyReplaced: false,
    draftPickerSelection: new Set(), createTestModel: "",
    priceTarget: null, priceRules: [], detailAccount: null, detailTab: "overview", detailConfigDirty: false,
    detailSequence: 0, detailConfigDraft: null, detailConfigError: ""
  };
  let toastTimer;
  let quotaHintSequence = 0;
  const numberFormat = new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 2 });
  const integerFormat = new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 0 });
  const compactFormat = new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 1 });
  const mtokenFormat = new Intl.NumberFormat("en-US", { maximumFractionDigits: 6 });
  const smallUsdFormat = new Intl.NumberFormat("en-US", { maximumSignificantDigits: 4 });
  const PROTOCOLS = Object.freeze({
    anthropic: { label: "Anthropic 消息协议", endpoint: "/v1/messages" },
    responses: { label: "OpenAI Responses 响应协议", endpoint: "/v1/responses" },
    chat_completions: { label: "OpenAI 聊天补全协议", endpoint: "/v1/chat/completions" }
  });
  const icons = {
    key: ["M15.5 3a5.5 5.5 0 0 0-5.2 7.3L3 17.5V21h3.5v-3.5H10V14l.7-.7A5.5 5.5 0 1 0 15.5 3Z", "M16.5 7.5h.01"],
    shield: ["M12 3 4 6v6c0 5 8 9 8 9s8-4 8-9V6l-8-3Z", "m8.5 12 2.5 2.5 4.5-5"],
    refresh: ["M20 7v5h-5", "M4 17v-5h5", "M6.1 7a7 7 0 0 1 11.6-2L20 8", "M4 16l2.3 3A7 7 0 0 0 18 17"],
    plus: ["M12 5v14", "M5 12h14"],
    search: ["M11 3a8 8 0 1 0 0 16 8 8 0 0 0 0-16Z", "m17 17 4 4"],
    chart: ["M4 3v17h17", "M8 15v-4", "M13 15V7", "M18 15v-6"],
    grid: ["M3 3h7v7H3z", "M14 3h7v7h-7z", "M3 14h7v7H3z", "M14 14h7v7h-7z"],
    help: ["M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18Z", "M9.5 9a2.5 2.5 0 0 1 5 0c0 2-2.5 2-2.5 4", "M12 17h.01"],
    info: ["M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18Z", "M12 11v6", "M12 7h.01"],
    arrow: ["M4 12h16", "m14 6 6 6-6 6"],
    close: ["m6 6 12 12", "M18 6 6 18"],
    trash: ["M3 6h18", "M9 6V3h6v3", "M5 6v15h14V6", "M10 10v7", "M14 10v7"],
    settings: ["M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8Z", "m9 3-.7 2.2-2 .9L4 5.7 2 9l1.5 1.8v2.4L2 15l2 3.3 2.3-.4 2 .9L9 21h6l.7-2.2 2-.9 2.3.4 2-3.3-1.5-1.8v-2.4L22 9l-2-3.3-2.3.4-2-.9L15 3H9Z"],
    copy: ["M8 8h13v13H8z", "M16 3H3v13"],
    send: ["m22 2-7 20-4-9L2 9l20-7Z", "m22 2-11 11"],
    token: ["M7 3v6", "M17 15v6", "M4 6h6", "M14 18h6", "M17 3v6", "M7 15v6", "M14 6h6", "M4 18h6"],
    dollar: ["M12 2v20", "M17 5H9a4 4 0 0 0 0 8h6a4 4 0 0 1 0 8H6"],
    check: ["m5 12 4 4L19 6"],
    calendar: ["M3 5h18v16H3z", "M7 2v6", "M17 2v6", "M3 10h18"],
    note: ["M5 3h10l4 4v14H5z", "M14 3v5h5", "M8 12h8", "M8 16h5"],
    terminal: ["m5 7 5 5-5 5", "M13 17h6"],
    clock: ["M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18Z", "M12 7v5l3 2"]
  };
  function icon(name) {
    const node = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    node.setAttribute("viewBox", "0 0 24 24");
    node.setAttribute("class", "icon");
    node.setAttribute("aria-hidden", "true");
    for (const d of icons[name] || icons.info) {
      const path = document.createElementNS(node.namespaceURI, "path");
      path.setAttribute("d", d);
      node.append(path);
    }
    return node;
  }
  function el(tag, attrs = {}, ...children) {
    const node = document.createElement(tag);
    for (const [key, value] of Object.entries(attrs)) {
      if (key === "class") node.className = value;
      else if (key === "text") node.textContent = value;
      else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
      else if (key in node && !key.startsWith("aria") && !key.startsWith("data")) node[key] = value;
      else node.setAttribute(key, String(value));
    }
    for (const child of children.flat()) {
      if (child !== null && child !== undefined && child !== false) node.append(child instanceof Node ? child : document.createTextNode(String(child)));
    }
    return node;
  }
  document.querySelectorAll("[data-icon]").forEach((node) => node.append(icon(node.dataset.icon)));
  $("account-models-content").append($("model-access-section"));
  $("global-models-content").append($("model-definitions-section"));
  $("global-models-actions").append($("definition-actions"));
  const finite = (value) => typeof value === "number" && Number.isFinite(value);
  const format = (value) => finite(value) ? numberFormat.format(value) : "—";
  const count = (value) => finite(value) ? integerFormat.format(value) : "—";
  const compact = (value) => finite(value) ? compactFormat.format(value) : "—";
  const usd = (value) => finite(value) ? "$" + (value !== 0 && Math.abs(value) < .01 ? smallUsdFormat.format(value) : numberFormat.format(value)) : "未知";
  const asText = (value, fallback = "—") => typeof value === "string" && value.trim() ? value : fallback;
  function date(value, withTime = true) {
    if (!value || !Number.isFinite(new Date(value).getTime())) return "未知";
    return new Intl.DateTimeFormat("zh-CN", {
      year: "numeric", month: "2-digit", day: "2-digit",
      ...(withTime ? { hour: "2-digit", minute: "2-digit" } : {}),
      timeZone: "UTC", hour12: false
    }).format(new Date(value)) + (withTime ? " UTC" : "");
  }
  function period(start, end) {
    return start && end ? date(start, false) + " — " + date(end, false) + " UTC" : "周期未知";
  }
  function remainingTime(end) {
    const ms = new Date(end || "").getTime() - Date.now();
    if (!Number.isFinite(ms)) return "重置时间未知";
    if (ms <= 0) return "周期已结束";
    const hours = Math.ceil(ms / 3600000);
    return hours >= 24 ? Math.ceil(hours / 24) + " 天后结束" : hours + " 小时后结束";
  }
  function sanitize(message, extraSecrets = []) {
    let text = String(message || "操作失败，请重试。").slice(0, 600);
    for (const secret of [state.token, ...extraSecrets]) if (typeof secret === "string" && secret.length) text = text.split(secret).join("[已脱敏]");
    return text.replace(/\b(?:fk-|sk-)[A-Za-z0-9_-]{8,}/gi, "[已脱敏]").replace(/Bearer\s+\S+/gi, "Bearer [已脱敏]");
  }
  function showError(id, message) {
    $(id).textContent = message ? sanitize(message) : "";
    $(id).hidden = !message;
  }
  function notify(message) {
    clearTimeout(toastTimer);
    $("toast").textContent = sanitize(message);
    $("toast").hidden = false;
    toastTimer = setTimeout(() => { $("toast").hidden = true; }, 3800);
  }
  function pending(button, text) {
    const children = [...button.childNodes];
    const disabled = button.disabled;
    button.disabled = true;
    button.setAttribute("aria-busy", "true");
    button.classList.add("is-pending");
    button.replaceChildren(icon("refresh"), text);
    return () => {
      button.replaceChildren(...children);
      button.disabled = disabled;
      button.removeAttribute("aria-busy");
      button.classList.remove("is-pending");
    };
  }
  function lockDialog(dialog, busy) {
    dialog.dataset.busy = String(busy);
    dialog.querySelectorAll("[data-close]").forEach((button) => { button.disabled = busy; });
  }
  function openDialog(id) {
    const dialog = $(id);
    if (!dialog.open) dialog.showModal();
  }
  function closeDialog(id) {
    const dialog = $(id);
    if (dialog.dataset.busy !== "true") dialog.close();
  }
  document.querySelectorAll("[data-close]").forEach((button) => {
    button.addEventListener("click", () => closeDialog(button.dataset.close));
  });
  document.querySelectorAll("dialog").forEach((dialog) => {
    dialog.addEventListener("click", (event) => {
      if (event.target !== dialog || dialog.dataset.busy === "true") return;
      const rect = dialog.getBoundingClientRect();
      if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) closeDialog(dialog.id);
    });
    dialog.addEventListener("cancel", (event) => {
      if (dialog.dataset.busy === "true") event.preventDefault();
    });
    dialog.addEventListener("close", () => {
      if (dialog.id === "auth-dialog") $("management-key").value = "";
      if (dialog.id === "account-dialog") { $("account-key").value = ""; if (!state.editingAccount) discardCreateDraft(); }
      if (dialog.id === "detail-dialog") { state.accessLoadSequence++; state.accessLoading = false; state.detailSequence++; state.detailConfigDirty = false; state.detailConfigDraft = null; state.detailConfigError = ""; }
    });
  });

  async function api(path, method = "GET", body, credential = state.token) {
    if (!credential) throw Object.assign(new Error("请先完成管理授权。"), { status: 401 });
    const requestEpoch = state.epoch;
    const controller = new AbortController();
    state.requests.add(controller);
    const timeout = setTimeout(() => controller.abort(), 190000);
    try {
      const response = await fetch(API + path, {
        method, headers: { Authorization: "Bearer " + credential, Accept: "application/json", ...(body ? { "Content-Type": "application/json" } : {}) },
        ...(body ? { body: JSON.stringify(body) } : {}),
        credentials: "omit", cache: "no-store", redirect: "error", signal: controller.signal
      });
      let payload;
      try { payload = await response.json(); }
      catch { throw Object.assign(new Error(response.status === 404 ? "插件接口不存在，请确认插件已成功加载。" : "管理端返回了无法识别的响应。"), { status: response.status }); }
      if (!response.ok || payload?.ok !== true) {
        const pluginError = payload?.ok === false && payload.error && typeof payload.error === "object" && typeof payload.error.code === "string";
        const managementAuth = (response.status === 401 || response.status === 403) && !pluginError;
        const messages = { 401: "管理授权已失效，请重新输入当前管理器密钥。", 403: "管理端拒绝访问，请检查密钥和宿主访问策略。", 404: "请求的数据或插件接口不存在。", 409: "数据已存在或状态冲突，请刷新后重试。" };
        const message = managementAuth ? messages[response.status] : payload?.error?.message || messages[response.status] || "操作失败，请稍后重试。";
        if (managementAuth && credential === state.token && requestEpoch === state.epoch) {
          disconnect();
          showError("global-error", message);
        }
        throw Object.assign(new Error(sanitize(message, [credential, body?.api_key])), { status: response.status, code: payload?.error?.code, managementAuth, retryAfter: response.headers.get("Retry-After") });
      }
      return payload.data;
    } catch (error) {
      if (error.status) throw error;
      throw new Error(error.name === "AbortError" ? "请求已取消或超时，请刷新确认实际结果。" : "无法连接 CPA 管理端，请检查服务连接。");
    } finally {
      clearTimeout(timeout);
      state.requests.delete(controller);
    }
  }
  function setConnection(connected) {
    state.connected = connected;
    const status = $("connection-status");
    status.classList.toggle("connected", connected);
    status.classList.toggle("degraded", connected && state.status?.health === "degraded");
    status.lastElementChild.textContent = state.loading && !connected ? "正在连接…" : connected ? (state.status?.health === "degraded" ? "已连接 · 部分异常" : "已连接") : "尚未授权";
    $("authorize-label").textContent = connected ? "管理授权" : "管理授权";
    $("authorization-notice").hidden = connected;
    $("disconnect").hidden = !connected;
    $("refresh-all").disabled = !connected || state.loading;
    $("add-account").disabled = !connected || state.loading;
    $("manage-models").disabled = !connected || state.loading;
    $("add-model").disabled = !connected || !state.settings || state.busy.has("settings");
    $("account-add-route").disabled = !connected || !state.settings || state.busy.has("settings");
    $("usage-account").disabled = !connected;
    $("usage-kind").disabled = !connected;
    for (const id of ["budget-window", "budget-amount", "budget-start", "budget-end"]) $(id).disabled = !connected || !state.settings;
    $("save-budget").disabled = !connected || !state.settings || state.busy.has("settings");
    $("save-mapping").disabled = !connected || state.busy.has("settings");
    $("version").textContent = state.status?.version ? "v" + state.status.version : "版本待连接";
    $("account-count").textContent = connected ? state.accounts.length : "—";
    $("save-models").disabled = !connected || state.modelDraft.size === 0 || state.busy.has("settings");
    updateAccessControls();
  }
  function disconnect() {
    state.epoch++;
    for (const controller of state.requests) controller.abort();
    state.token = "";
    state.credentialSource = "";
    state.status = state.settings = state.usage = null;
    state.accounts = [];
    state.models = [];
    state.modelDraft.clear();
    state.budgetDirty = false;
    state.usageAccount = "";
    state.usageKind = "all";
    state.pinnedAccount = "";
    state.modelAccount = "";
    state.accountModelCache.clear();
    state.accessError = null;
    state.accessLoading = false;
    state.accessLoadSequence++;
    state.candidateRoutes.clear();
    state.verifyTarget = null;
    state.createSequence++;
    state.createDraft = null;
    state.createModels.clear();
    state.draftPickerSelection.clear();
    state.createTestModel = "";
    if ($("draft-candidate-dialog").open) $("draft-candidate-dialog").close();
    for (const id of ["global-models-dialog", "price-dialog", "detail-dialog"]) if ($(id).open) $(id).close();
    $("account-key").value = "";
    if ($("account-dialog").open) $("account-dialog").close();
    state.loading = false;
    setConnection(false);
    renderAll();
    showError("global-error", "");
  }
  async function loadAll() {
    if (!state.token || state.loading) return;
    state.loading = true;
    const epoch = state.epoch;
    setConnection(state.connected);
    const restore = pending($("refresh-all"), "");
    showError("global-error", "");
    try {
      const [status, accounts] = await Promise.all([api("/status"), api("/accounts")]);
      if (epoch !== state.epoch) return;
      state.status = status;
      state.accounts = Array.isArray(accounts?.accounts) ? accounts.accounts : [];
      state.accountModelCache.clear();
      state.accessLoadSequence++;
      state.accessLoading = false;
      state.connected = true;
      renderAll();
      if ($("detail-dialog").open && state.detailTab === "models") ensureAccountModels(true);
      if (status?.message) showError("global-error", status.message);
    } catch (error) {
      if (epoch !== state.epoch) return;
      if (error.managementAuth) {
        disconnect();
        showError("global-error", error.message);
      } else showError("global-error", error.message);
      throw error;
    } finally {
      restore();
      if (epoch === state.epoch) { state.loading = false; setConnection(state.connected); }
    }
  }
  function openAuth() { showError("auth-error", ""); openDialog("auth-dialog"); }
  $("authorize").onclick = openAuth;
  $("authorize-inline").onclick = openAuth;
  $("disconnect").onclick = () => { disconnect(); $("auth-dialog").close(); notify("已断开当前页面授权"); };
  $("auth-form").onsubmit = async (event) => {
    event.preventDefault();
    if (state.busy.has("auth")) return;
    const credential = $("management-key").value.trim();
    if (!credential) return;
    state.busy.add("auth");
    showError("auth-error", "");
    const restore = pending($("connect"), "正在连接…");
    lockDialog($("auth-dialog"), true);
    try {
      const status = await api("/status", "GET", undefined, credential);
      for (const controller of state.requests) controller.abort();
      state.epoch++;
      state.loading = false;
      state.token = credential;
      state.credentialSource = "manual";
      state.status = status;
      state.connected = true;
      state.accounts = [];
      state.models = [];
      state.settings = state.usage = null;
      state.usageAccount = "";
      state.modelDraft.clear();
      state.budgetDirty = false;
      $("management-key").value = "";
      await loadAll();
      if (!state.connected) throw new Error("管理授权未完成，请检查当前管理器密钥。");
      $("auth-dialog").close();
      notify("管理授权成功");
    } catch (error) {
      showError("auth-error", error.message);
    } finally {
      state.busy.delete("auth");
      restore();
      lockDialog($("auth-dialog"), false);
      setConnection(state.connected);
    }
  };
  $("refresh-all").onclick = () => { loadAll().then(() => { if (state.connected) notify("数据已刷新"); }).catch(() => {}); };

  function setView(view) {
    view = "accounts";
    state.view = view;
    document.querySelectorAll("[data-view]").forEach((button) => {
      const active = button.dataset.view === view;
      button.classList.toggle("active", active);
      if (active) button.setAttribute("aria-current", "page"); else button.removeAttribute("aria-current");
    });
    document.querySelectorAll(".view").forEach((section) => { section.hidden = section.id !== "view-" + view; });
  }
  document.querySelectorAll("[data-view]").forEach((button) => { button.onclick = () => setView(button.dataset.view); });
  addEventListener("hashchange", () => setView("accounts"));
  async function loadModelMetadata(force = false) {
    if (!force && state.settings && state.models.length) return;
    const epoch = state.epoch;
    const [models, settings] = await Promise.all([api("/models"), api("/settings")]);
    if (epoch !== state.epoch) return;
    state.models = Array.isArray(models?.models) ? models.models : [];
    state.settings = settings;
    setConnection(state.connected);
  }

  function emptyState(title, description, action, actionText, symbol = "key") {
    return el("div", { class: "empty-state" }, el("span", { class: "empty-icon" }, icon(symbol)),
      el("h3", {}, title), el("p", {}, description),
      action ? el("button", { type: "button", class: "button button-primary", onclick: action }, icon(actionText === "添加账号" ? "plus" : "shield"), actionText) : null);
  }
  function accountStatus(account) {
    if (account.enabled === false) return ["已停用", "neutral"];
    if (account.status === "ready") return account.identity_verified === false ? ["身份待验证", "warning"] : ["账号已验证", "success"];
    return ({ disabled: ["已停用", "neutral"], error: ["验证异常", "danger"], pending: ["待验证", "warning"] })[account.status] || ["状态未知", "neutral"];
  }
  function keyHint(hint) {
    if (typeof hint !== "string" || !hint.trim()) return "密钥已脱敏";
    const suffix = hint.replace(/[•*·\s]/g, "").slice(-4);
    return suffix ? "•••• " + suffix : "密钥已脱敏";
  }
  function requestSummary(usage) {
    const successful = finite(usage?.requests) ? usage.requests : null;
    const failed = finite(usage?.failed_requests) ? usage.failed_requests : null;
    const total = finite(usage?.total_requests) ? usage.total_requests : successful !== null && failed !== null ? successful + failed : null;
    return { successful, failed, total, rate: finite(total) && total > 0 ? finite(usage?.success_rate) ? usage.success_rate * 100 : successful !== null ? successful / total * 100 : null : null };
  }
  function miniStat(label, value, symbol, small = false, title = "") {
    return el("div", { class: "mini-stat", ...(title ? { title } : {}) }, el("span", { class: "mini-stat-label" }, icon(symbol), label), el("strong", { class: "mini-stat-value" + (small ? " small" : "") }, value));
  }
  function cardButton(account, symbol, label, action, danger = false, text = false) {
    return el("button", {
      type: "button", class: "button " + (danger ? "button-danger-quiet" : "button-soft") + (text ? "" : " icon-button"),
      title: label, "aria-label": label + "：" + account.label, disabled: state.busy.has(account.id),
      "data-focus-key": account.id + ":" + symbol, onclick: action
    }, icon(symbol), text ? label : null);
  }
  function progress(used, budget, label = "本地预算已使用比例") {
    if (!finite(used) || !finite(budget)) return null;
    const ratio = budget > 0 ? used / budget : used > 0 ? 1 : 0;
    const remaining = label === "官方额度剩余比例";
    const danger = remaining ? ratio <= .05 : ratio >= 1;
    const warning = remaining ? ratio <= .2 : ratio >= .8;
    const fill = el("div", { class: "progress-fill" + (danger ? " danger" : warning ? " warning" : "") });
    fill.style.width = Math.min(100, Math.max(0, ratio * 100)) + "%";
    return el("div", { class: "progress-track", role: "progressbar", "aria-label": label, "aria-valuemin": 0, "aria-valuemax": 100, "aria-valuenow": Math.min(100, Math.max(0, Math.round(ratio * 100))), "aria-valuetext": label === "本地预算已使用比例" ? format(used) + " / " + format(budget) + " 加权 token" : format(used) + "%" }, fill);
  }
  function officialLabel(official) {
    if (official?.status === "available" && finite(official.used)) return format(official.used) + " FSC";
    return ({ unavailable: "不可读取", error: "查询失败", not_checked: "尚未查询" })[official?.status] || "未知";
  }
  function renderRecent(usage) {
    const recent = usage?.recent;
    const items = Array.isArray(recent?.requests) ? recent.requests : [];
    const successes = finite(recent?.success_count) ? recent.success_count : null;
    const failures = finite(recent?.failure_count) ? recent.failure_count : null;
    const section = el("div", { class: "card-section" });
    const head = el("div", { class: "card-section-title", title: "计数覆盖最近 " + items.length + " 条记录；状态条展示最后 " + Math.min(24, items.length) + " 条。" }, el("h4", {}, "最近请求"));
    if (successes !== null || failures !== null) head.append(el("div", { class: "recent-counts" }, el("span", { class: "text-success" }, "● " + count(successes)), el("span", { class: "text-danger" }, "● " + count(failures))));
    else head.append(el("span", {}, "暂无观测记录"));
    section.append(head);
    if (!items.length) { section.append(el("p", { class: "card-caption" }, "暂无请求")); return section; }
    const bars = el("div", { class: "recent-bars", "aria-label": items.length ? "最近 " + Math.min(24, items.length) + " 次请求状态" : "暂无最近请求数据" });
    const visibleItems = items.slice(-24);
    for (let index = 0; index < 24; index++) {
      const item = visibleItems[index - (24 - visibleItems.length)];
      const status = item?.success === true || item?.status === "success" ? "success" : item?.success === false || item?.status === "failure" || item?.status === "error" ? "failure" : "";
      bars.append(el("span", { class: "recent-bar" + (status ? " " + status : ""), ...(item ? { title: (item.kind === "validation" ? "基础验证" : item.kind === "inference" ? "业务调用" : "历史记录（类型未标注）") + " · " + (status === "success" ? "成功" : status === "failure" ? "失败" : "未知") + (item.at ? " · " + date(item.at) : "") } : { "aria-hidden": "true" }) }));
    }
    const rate = successes !== null && failures !== null && successes + failures > 0 ? successes / (successes + failures) * 100 : null;
    section.append(el("div", { class: "recent-strip" }, bars, rate !== null ? el("span", { class: "recent-rate" }, format(rate) + "%") : null));
    return section;
  }
  function quotaTimestamp(value) {
    if (typeof value !== "string" || !value.trim()) return null;
    const timestamp = new Date(value).getTime();
    return Number.isFinite(timestamp) ? timestamp : null;
  }
  function quotaDateField(...values) {
    for (const value of values) {
      if (value === null || value === undefined || value === "") continue;
      return { value, timestamp: quotaTimestamp(value), invalid: quotaTimestamp(value) === null };
    }
    return { value: null, timestamp: null, invalid: false };
  }
  function matchingQuotaPrediction(window, prediction) {
    if (!["standard", "core"].includes(window?.group) || typeof window?.window_key !== "string" || !window.window_key) return null;
    const end = quotaDateField(window.reset_at, window.window_end, window.period_stats?.window_end);
    const start = quotaDateField(window.period_stats?.window_start, window.window_start);
    if (end.invalid || end.timestamp === null || start.invalid) return null;
    return (Array.isArray(prediction?.windows) ? prediction.windows : []).find((candidate) => {
      if (candidate.group !== window.group || candidate.window_key !== window.window_key) return false;
      const otherEnd = quotaDateField(candidate.window_end, candidate.reset_at, candidate.period_stats?.window_end);
      const otherStart = quotaDateField(candidate.period_stats?.window_start, candidate.window_start);
      if (otherEnd.invalid || otherEnd.timestamp !== end.timestamp || otherStart.invalid) return false;
      return start.timestamp === null || otherStart.timestamp === null || start.timestamp === otherStart.timestamp;
    }) || null;
  }
  function millionTokens(value) {
    if (!finite(value)) return "—";
    if (value > 0 && value < .000001) return "<0.000001 M";
    return mtokenFormat.format(value) + " M";
  }
  function quotaCountdown(value) {
    const end = quotaTimestamp(value);
    if (end === null) return "—";
    if (end <= Date.now()) return "待刷新";
    const hours = Math.ceil((end - Date.now()) / 3600000);
    return hours >= 24 ? Math.ceil(hours / 24) + " 天后" : hours + " 小时后";
  }
  function quotaValues(window, prediction) {
    const stats = window.period_stats || {};
    const matched = matchingQuotaPrediction(window, prediction);
    const end = quotaDateField(window.reset_at, window.window_end, stats.window_end);
    const expired = end.timestamp !== null && end.timestamp <= Date.now();
    const available = stats.status === "available";
    const invalidRecords = finite(stats.invalid_timestamp_requests) && stats.invalid_timestamp_requests > 0;
    const usageComplete = available && stats.usage_complete === true && !invalidRecords && !(finite(stats.missing_usage_requests) && stats.missing_usage_requests > 0);
    const costComplete = available && stats.cost_complete === true && !invalidRecords;
    const estimated = !expired && matched?.status === "estimated";
    return {
      stats, matched, end: end.value, expired,
      usedM: usageComplete && finite(stats.total_mtokens) ? stats.total_mtokens : null,
      usedUsd: costComplete && finite(stats.estimated_cost_usd) ? stats.estimated_cost_usd : null,
      totalM: estimated && finite(matched.estimated_total_mtokens) ? matched.estimated_total_mtokens : null,
      totalUsd: estimated && finite(matched.estimated_total_cost_usd) ? matched.estimated_total_cost_usd : null
    };
  }
  function quotaExplanation(window, values, limits) {
    const s = values.stats;
    const coverage = ({ complete: "本插件记录覆盖此周期", partial: "本插件仅覆盖部分周期", unknown: "本插件记录范围未知" })[s.coverage] || "本插件记录范围未知";
    const parts = ["剩余比例来自官方；已用仅为本插件该窗口记录，含验证。", coverage + "。≈ 为样本预测总量，非官方硬限额。"];
    if (values.expired) parts.push("官方周期已结束，待刷新后重新确认预测。");
    else if (values.totalM === null || values.totalUsd === null) parts.push("样本或价格不足的预测显示 —。");
    if (values.usedUsd === null) parts.push("已用费用未知。");
    if (values.usedM === null) parts.push("完整已用 Token 未知。");
    return sanitize(parts.join(" "));
  }
  function visibleQuotaWindows(windows) {
    return (Array.isArray(windows) ? windows : []).filter((window) => window && window.group !== "core");
  }
  function renderOfficialLimits(limits, prediction, compact = false) {
    const allWindows = Array.isArray(limits?.windows) ? limits.windows : [];
    const windows = visibleQuotaWindows(allWindows);
    if (allWindows.length && !windows.length) return null;
    const section = el("div", { class: "card-section compact-quota-section" });
    if (limits?.status !== "available" || !windows.length) {
      section.append(el("span", { class: "quota-unavailable", title: asText(limits?.message, "刷新账号后读取官方额度。") }, limits?.status === "not_checked" ? "额度尚未查询" : "额度暂不可读取"));
      return section;
    }
    for (const window of compact ? windows.slice(0, 3) : windows) {
      const values = quotaValues(window, prediction);
      const explanation = quotaExplanation(window, values, limits);
      const hintId = "quota-hint-" + (++quotaHintSequence);
      const remaining = finite(window.remaining_fraction) ? Math.max(0, Math.min(1, window.remaining_fraction)) : null;
      const info = el("span", { class: "quota-info-button", tabIndex: 0, role: "note", title: explanation, "aria-label": asText(window.name, "额度") + "用量说明", "aria-describedby": hintId }, icon("info"));
      const bar = el("div", { class: "compact-quota-track", ...(remaining === null ? { role: "img", "aria-label": "官方剩余比例未知" } : { role: "progressbar", "aria-label": "官方额度剩余比例", "aria-valuemin": 0, "aria-valuemax": 100, "aria-valuenow": remaining * 100 }) });
      if (remaining !== null) { const fill = el("div", { class: "compact-quota-fill" }); fill.style.width = remaining * 100 + "%"; bar.append(fill); }
      const usedUsd = values.usedUsd === null ? "—" : usd(values.usedUsd);
      const usedM = millionTokens(values.usedM);
      const totalUsd = values.totalUsd === null ? "—" : usd(values.totalUsd);
      const totalM = millionTokens(values.totalM);
      const group = (forecast) => el("span", {
        class: "quota-pair " + (forecast ? "quota-forecast" : "quota-used"), role: "group",
        title: (forecast ? "预计周期总量" : "本插件本周期已用") + "：" + String((forecast ? values.totalUsd : values.usedUsd) ?? "—") + " USD / " + String((forecast ? values.totalM : values.usedM) ?? "—") + " M。" + explanation,
        "aria-label": (forecast ? "样本预计周期总量 " + totalUsd + "，" + totalM : "本插件已用 " + usedUsd + "，" + usedM),
        "aria-describedby": hintId
      }, icon("chart"), ...(forecast && values.totalUsd === null && values.totalM === null ? [el("span", {}, "—")] : [
        forecast ? el("span", { class: "quota-estimate-mark" }, "≈") : null,
        el("strong", {}, forecast ? totalUsd : usedUsd), el("span", { class: "quota-ratio-divider" }, "/"), el("span", {}, forecast ? totalM : usedM)
      ]));
      section.append(el("div", { class: "compact-quota-window" },
        el("div", { class: "compact-quota-heading" }, el("span", { class: "compact-quota-label" }, asText(window.name, window.group === "core" ? "Core 额度" : "标准额度")), info, el("strong", {}, remaining === null ? "剩余 —" : "剩余 " + format(remaining * 100) + "%")),
        bar,
        el("div", { class: "compact-quota-footer" },
          el("div", { class: "compact-quota-metrics" }, group(false), group(true)),
          el("span", { class: "compact-quota-reset", title: "官方重置时间：" + date(values.end), "data-countdown": values.end || "", "data-countdown-mode": "quota" }, quotaCountdown(values.end))),
        el("span", { id: hintId, class: "sr-only" }, explanation)));
    }
    return section;
  }
  function renderAccountCard(account) {
    const usage = account.usage || {};
    const official = account.official_usage;
    const summary = requestSummary(usage);
    const status = accountStatus(account);
    const statusLabel = account.enabled && status[0] === "账号已验证" ? "可用" : status[0];
    const isBusy = state.busy.has(account.id);
    const toggle = el("input", { type: "checkbox", role: "switch", checked: account.enabled === true, disabled: isBusy, "aria-label": (account.enabled ? "停用" : "启用") + "账号：" + account.label, "data-focus-key": account.id + ":toggle" });
    toggle.onchange = () => runAccount(account, "/accounts", "PATCH", { id: account.id, enabled: toggle.checked }, toggle.checked ? "账号已启用" : "账号已停用");
    const card = el("article", { class: "account-card" + (account.enabled === false ? " disabled" : ""), "aria-label": account.label || "Factory 账号", ...(isBusy ? { "aria-busy": "true" } : {}) });
    card.append(el("div", { class: "account-top" },
      el("span", { class: "account-avatar" }, icon("terminal")),
      el("div", { class: "account-title" }, el("h3", { title: account.label || "未命名账号" }, account.label || "未命名账号"), el("span", { class: "account-subtitle", title: asText(account.identity?.email, keyHint(account.credential_hint)) }, asText(account.identity?.email, keyHint(account.credential_hint)))),
      el("div", { class: "account-top-actions" }, el("span", { class: "badge badge-" + status[1] }, statusLabel), el("label", { class: "switch" }, toggle))));
    card.append(el("div", { class: "account-meta" }, el("span", { class: "badge badge-brand" }, account.kind === "oauth" ? "OAuth" : "API Key"), el("span", { class: "badge badge-neutral" }, "优先级 " + (finite(account.priority) ? account.priority : 0)), account.plan ? el("span", { class: "badge badge-neutral" }, String(account.plan)) : null));
    if (asText(account.note, "")) card.append(el("div", { class: "account-note" }, icon("note"), account.note));
    const accountModels = state.accountModelCache.get(account.id)?.models;
    const modelAccess = Array.isArray(accountModels) ? accountModels : Array.isArray(account.model_access) ? account.model_access : null;
    const publishedCount = modelAccess ? modelAccess.filter((model) => model.published === true).length : Array.isArray(account.pinned_models) ? account.pinned_models.length : null;
    const allowedCount = modelAccess ? modelAccess.filter((model) => model.allowed === true).length : null;
    const verifiedCount = modelAccess ? modelAccess.filter((model) => model.verification_status === "basic_call_verified").length : null;
    card.append(el("div", { class: "account-model-summary" },
      el("span", { title: "已允许 " + count(allowedCount) + "；基础已验证 " + count(verifiedCount) }, "模型已发布 ", el("strong", {}, count(publishedCount)), finite(account.pending_model_count) && account.pending_model_count > 0 ? " · 待验证 " + count(account.pending_model_count) : "")));
    const completeTokens = usage.credits_complete !== false && !(finite(usage.requests_with_usage) && finite(summary.successful) && usage.requests_with_usage < summary.successful);
    const tokenTotal = completeTokens && finite(usage.input_tokens) && finite(usage.output_tokens) ? usage.input_tokens + usage.output_tokens : null;
    const history = el("div", { class: "card-section" },
      el("div", { class: "card-section-title" }, el("h4", {}, "历史用量")),
      el("div", { class: "mini-stats" },
        miniStat("请求", compact(summary.total), "send", false, count(summary.total) + " 次，成功 " + count(summary.successful) + " / 失败 " + count(summary.failed)),
        miniStat("Token", compact(tokenTotal), "token", false, "本周期输入与输出合计，含验证；缓存及 reasoning 不重复相加"),
        miniStat("费用", finite(usage.estimated_cost_usd) ? usd(usage.estimated_cost_usd) : "—", "dollar", false, "本周期估算费用（USD），含验证；非官方账单"),
        miniStat("成功率", summary.rate !== null ? format(summary.rate) + "%" : "—", "check")),
      el("p", { class: "card-caption", title: date(usage.window_start) + " — " + date(usage.window_end) + "；含基础验证" }, period(usage.window_start, usage.window_end)));
    card.append(...[history, renderRecent(usage), renderOfficialLimits(account.official_limits, account.prediction, true)].filter(Boolean));
    if (account.last_error) card.append(el("p", { class: "account-error-message" }, sanitize(account.last_error)));
    const refresh = cardButton(account, "refresh", "验证 / 刷新", () => runAccount(account, "/accounts/refresh", "POST", { id: account.id }, "验证与用量状态已更新"));
    if (isBusy) refresh.classList.add("is-pending");
    card.append(el("div", { class: "account-footer" },
      cardButton(account, "trash", "删除账号", () => openDelete(account), true),
      el("span", { class: "footer-spacer" }),
      refresh, cardButton(account, "grid", "模型管理", () => manageAccountModels(account.id)), cardButton(account, "settings", "账号设置", () => openAccount(account)), cardButton(account, "info", "详情", () => openDetail(account), false, true)));
    return card;
  }
  function renderAccounts() {
    const grid = $("accounts-grid");
    const focus = document.activeElement?.dataset?.focusKey;
    grid.replaceChildren();
    if (!state.connected) {
      grid.append(emptyState("连接后查看 Factory 账号", "账号凭据与用量保存在 CPA 服务端。完成管理授权后，即可添加第一个 Factory 账号。", openAuth, "管理授权"));
      $("account-summary").textContent = "连接后显示账号状态";
      $("accounts-subtitle").textContent = "通过 API Key 连接 Factory，为 CPA 提供模型服务。";
      return;
    }
    const enabled = state.accounts.filter((account) => account.enabled).length;
    $("account-summary").textContent = "共 " + state.accounts.length + " 个账号 · 已启用 " + enabled + " 个";
    $("accounts-subtitle").textContent = "管理账号凭据、查看实际记录，并按需设置使用优先级。";
    const query = state.accountSearch.toLocaleLowerCase();
    const filtered = state.accounts.filter((account) => {
      if (query && ![account.label, account.identity?.email, keyHint(account.credential_hint)].some((value) => String(value || "").toLocaleLowerCase().includes(query))) return false;
      return state.accountFilter === "all" || state.accountFilter === "enabled" && account.enabled || state.accountFilter === "disabled" && !account.enabled || state.accountFilter === "error" && account.status === "error";
    });
    if (filtered.length) grid.append(...filtered.map(renderAccountCard));
    else if (!state.accounts.length) grid.append(emptyState("添加你的第一个 Factory 账号", "先验证账号身份，再逐个模型执行基础验证并允许；未经验证允许的模型不会发布到 CPA。", () => openAccount(), "添加账号"));
    else grid.append(emptyState("没有匹配的账号", "试试其他名称，或调整账号状态筛选。", null, ""));
    if (focus) {
      const target = [...grid.querySelectorAll("[data-focus-key]")].find((node) => node.dataset.focusKey === focus && !node.disabled);
      target?.focus({ preventScroll: true });
    }
  }
  $("account-search").oninput = (event) => { state.accountSearch = event.target.value.trim(); renderAccounts(); };
  $("account-filter").onchange = (event) => { state.accountFilter = event.target.value; renderAccounts(); };
  function openAccount(account = null) {
    if (!state.connected) return openAuth();
    if (account) return openDetail(account, false, "config");
    state.editingAccount = account;
    $("account-form").reset();
    $("account-dialog-title").textContent = account ? "账号设置" : "新建 Factory 账号";
    $("close-account-dialog").setAttribute("aria-label", account ? "关闭账号设置" : "关闭新建账号抽屉");
    $("account-dialog-description").textContent = account ? "调整名称、启停状态与 CPA 路由优先级。" : "模型和测试均可稍后配置，也可直接创建账号查看额度。";
    $("account-label").value = account?.label || "";
    $("account-enabled").checked = account ? account.enabled === true : true;
    $("account-key").value = "";
    $("account-key").required = !account;
    $("account-key-field").hidden = !!account;
    $("account-settings-fields").hidden = !account;
    $("account-priority").value = account?.priority ?? 0;
    $("account-dialog").classList.toggle("is-creating", !account);
    $("account-credentials").hidden = false;
    $("account-draft-workspace").hidden = !!account;
    $("draft-test-section").hidden = !!account;
    $("draft-footer-hint").hidden = !!account;
    $("account-label").disabled = $("account-enabled").disabled = false;
    $("save-account").textContent = account ? "保存设置" : "创建";
    $("save-account").disabled = false;
    if (!account) {
      state.createSequence++;
      state.createDraft = null;
      state.createModels.clear();
      state.createSearch = "";
      state.createShowRoutes = false;
      state.createCatalogError = state.createProgress = "";
      state.createCommitPending = false;
      state.createKeyReplaced = false;
      state.draftPickerSelection.clear();
      state.createTestModel = "";
      $("draft-candidate-search").value = "";
      $("draft-test-confirmation").checked = false;
      renderCreateAccount();
    }
    showError("account-error", "");
    $("account-form-scroll").scrollTop = 0;
    openDialog("account-dialog");
    $("account-label").focus({ preventScroll: true });
  }
  $("add-account").onclick = () => openAccount();
  $("account-form").onsubmit = async (event) => {
    event.preventDefault();
    if (state.busy.has("account-form")) return;
    const account = state.editingAccount;
    const label = $("account-label").value.trim();
    if (!label) return showError("account-error", "请填写账号名称。");
    if (!account) {
      if (!state.createDraft) await startCreateDraft();
      if (state.createDraft && state.connected && $("account-dialog").open) await commitCreateDraft();
      return;
    }
    const body = { id: account.id, label, enabled: $("account-enabled").checked, priority: Number($("account-priority").value) };
    state.busy.add("account-form");
    const epoch = state.epoch;
    showError("account-error", "");
    const restore = pending($("save-account"), account ? "正在保存…" : "正在验证…");
    lockDialog($("account-dialog"), true);
    try {
      await api("/accounts", "PATCH", body);
      if (epoch !== state.epoch) return;
      $("account-key").value = "";
      if (body.api_key) body.api_key = "";
      $("account-dialog").close();
      await loadAll();
      if (epoch !== state.epoch) return;
      notify("账号设置已保存");
    } catch (error) {
      showError("account-error", error.message);
      if (!$("account-dialog").open) notify("账号已保存，但刷新失败；请刷新确认最新状态。");
    } finally {
      if (body.api_key) body.api_key = "";
      state.busy.delete("account-form");
      restore();
      lockDialog($("account-dialog"), false);
    }
  };
  function draftExpired() {
    return !!state.createDraft && (!Number.isFinite(new Date(state.createDraft.expires_at).getTime()) || new Date(state.createDraft.expires_at).getTime() <= Date.now());
  }
  function draftModelProof(modelId) {
    return state.createDraft?.models?.find((model) => model.model_id === modelId);
  }
  function draftModelVerified(modelId) {
    return draftModelProof(modelId)?.verification_status === "basic_call_verified";
  }
  function adoptCreateDraft(data) {
    if (!data || typeof data.id !== "string" || !Array.isArray(data.models)) throw new Error("草稿接口格式无效，请检查插件版本。");
    state.createDraft = data;
    state.createCommitPending = !!data.committed_account_id;
    state.createKeyReplaced = false;
    $("account-key").value = "";
    $("account-key").required = false;
  }
  function draftRouteProblem(model) {
    if (!model || !protocolMeta(model.protocol)) return "尚无明确原生路由";
    if (model.global_enabled === false) return "模型定义已停用";
    return reasonList((model.blocked_reasons || []).filter((reason) => HARD_VERIFY_REASONS.has(reason)));
  }
  function draftCandidateOptions() {
    const data = state.createDraft;
    if (!data) return [];
    const models = data.models || [];
    const discovery = data.discovery;
    const candidates = (Array.isArray(discovery?.candidates) ? discovery.candidates : []).flatMap((candidate) => {
      const routes = (candidate.route_models || []).map((id) => models.find((model) => model.model_id === id)).filter(Boolean);
      const problem = candidateProblem(candidate, data);
      if (!routes.length) return [{ key: "candidate:" + candidate.id, id: candidate.id, name: candidate.display_name || candidate.id, model: null, problem: problem || "尚无明确原生路由，请先配置模型定义", source: "cli_candidate" }];
      return routes.map((model) => ({ key: model.model_id, id: candidate.id, name: candidate.display_name || model.display_name || candidate.id, model, problem: problem || draftRouteProblem(model), source: "cli_candidate", snapshotId: discovery?.snapshot_id }));
    });
    const discovered = new Set(candidates.map((option) => option.model?.model_id).filter(Boolean));
    const discoveredUpstreamIds = new Set((Array.isArray(discovery?.candidates) ? discovery.candidates : []).map((candidate) => candidate.id));
    // Merge configured routes even when the CLI omits feature-gated models.
    // A present but blocked/stale candidate must never get a second manual entry.
    return [...candidates, ...models.filter((model) => !discovered.has(model.model_id) && !discoveredUpstreamIds.has(model.upstream_id)).map((model) => ({
      key: model.model_id, name: model.display_name || model.model_id, id: model.upstream_id || model.model_id,
      model, problem: draftRouteProblem(model), source: "manual"
    }))];
  }
  function addDraftModel(option, render = true) {
    if (!option?.model || option.problem || state.busy.has("account-form") || state.createCommitPending) return;
    const model = option.model;
    if (state.createModels.has(model.model_id)) return;
    if (state.createModels.size >= 100) return showError("account-error", "每个账号最多选择 100 个模型。");
    state.createModels.set(model.model_id, {
      modelId: model.model_id, displayName: model.display_name || model.model_id,
      alias: model.alias || "", protocol: model.protocol, upstreamId: model.upstream_id,
      source: option.source, snapshotId: option.snapshotId || null
    });
    $("draft-test-confirmation").checked = false;
    if (render) renderCreateAccount();
  }
  function removeDraftModel(modelId) {
    if (state.busy.has("account-form") || state.createCommitPending) return;
    state.createModels.delete(modelId);
    $("draft-test-confirmation").checked = false;
    renderCreateAccount();
  }
  function renderDraftCandidates() {
    const query = state.createSearch.toLocaleLowerCase();
    const compactQuery = query.replace(/[\s._-]+/g, "");
    const all = draftCandidateOptions();
    const visible = all.filter((item) => !query || [item.name, item.id, item.model?.model_id, item.model?.alias].some((value) => {
      const text = String(value || "").toLocaleLowerCase();
      return text.includes(query) || !!compactQuery && text.replace(/[\s._-]+/g, "").includes(compactQuery);
    }));
    const list = $("draft-candidate-list");
    list.replaceChildren(...visible.map((option) => {
      const selected = option.model && state.createModels.has(option.model.model_id);
      const checkbox = el("input", { type: "checkbox", checked: !!selected || state.draftPickerSelection.has(option.key), disabled: !!option.problem || selected || state.busy.has("account-form") || state.createCommitPending, "aria-label": "选择 " + option.id + (option.problem ? "，" + option.problem : "") });
      checkbox.onchange = () => { if (checkbox.checked) state.draftPickerSelection.add(option.key); else state.draftPickerSelection.delete(option.key); updateDraftPickerControls(); };
      const supplemental = option.source === "manual";
      const verified = option.model?.verification_status === "basic_call_verified";
      const description = option.problem || (protocolMeta(option.model?.protocol)?.label || "协议未知") + (supplemental ? " · 已配置路由，本次 CLI 目录未返回；调用权限需通过测试确认" : "");
      return el("label", { class: "draft-candidate" + (selected ? " is-added" : option.problem ? " is-unavailable" : ""), title: option.id + " · " + option.name + " · " + description },
        checkbox,
        el("div", { class: "draft-candidate-text" }, el("strong", {}, option.id), option.name !== option.id ? el("em", {}, option.name) : null),
        selected ? el("span", { class: "badge badge-brand" }, "已添加") : option.problem ? el("span", { class: "badge badge-neutral", "aria-label": description }, "暂不可选") : supplemental ? el("span", { class: "badge badge-neutral", title: description }, verified ? "已验证" : "待验证") : null);
    }));
    if (!visible.length) list.append(el("div", { class: "draft-selected-empty" }, all.length ? "没有匹配的模型。" : "暂无 CLI 候选；可手工选择已有明确路由。"));
    $("draft-candidate-count").textContent = "显示 " + visible.length + " / " + all.length;
    $("draft-add-visible").disabled = state.busy.has("account-form") || state.createCommitPending || !visible.some((item) => !item.problem && item.model && !state.createModels.has(item.model.model_id));
    $("draft-add-visible").onclick = () => {
      for (const option of visible) if (!option.problem && option.model && !state.createModels.has(option.model.model_id)) state.draftPickerSelection.add(option.key);
      renderDraftCandidates();
    };
    updateDraftPickerControls();
  }
  function updateDraftPickerControls() {
    $("draft-picker-selected").textContent = "已选 " + state.draftPickerSelection.size + " 项";
    $("draft-picker-apply").disabled = !state.draftPickerSelection.size || state.busy.has("account-form") || !state.createDraft || draftExpired();
  }
  function openDraftPicker() {
    if (!state.createDraft || !$("account-dialog").open) return;
    state.draftPickerSelection.clear();
    state.createSearch = "";
    $("draft-candidate-search").value = "";
    renderDraftCandidates();
    openDialog("draft-candidate-dialog");
    $("draft-candidate-search").focus();
  }
  $("draft-picker-clear").onclick = () => { state.draftPickerSelection.clear(); renderDraftCandidates(); };
  $("draft-picker-apply").onclick = () => {
    if (!state.createDraft || draftExpired() || state.busy.has("account-form")) return;
    for (const option of draftCandidateOptions()) if (state.draftPickerSelection.has(option.key)) addDraftModel(option, false);
    $("draft-candidate-dialog").close();
    state.draftPickerSelection.clear();
    renderCreateAccount();
  };
  function renderDraftModelCard(model) {
    const proof = draftModelProof(model.modelId);
    const verified = draftModelVerified(model.modelId);
    const busy = state.busy.has("account-form") || state.createCommitPending;
    const protocol = protocolMeta(model.protocol);
    const aliasInput = el("input", { type: "text", value: model.alias, placeholder: model.modelId.slice("factory/".length), maxLength: 80, pattern: "[A-Za-z0-9._-]+", spellcheck: false, disabled: busy, "aria-label": model.displayName + " 固定账号调用别名" });
    aliasInput.oninput = () => { model.alias = aliasInput.value; updateDraftControls(); };
    const error = proof?.last_probe_error || proof?.last_error;
    const status = verified ? ["基础测试通过", "success"] : proof?.verification_status === "failed" ? ["基础测试失败", "danger"] : ["尚未测试", "neutral"];
    return el("article", { class: "draft-model-card" + (verified ? " is-verified" : "") },
      el("div", { class: "draft-model-line" }, el("strong", { class: "draft-model-name", title: model.modelId }, model.displayName), el("span", { class: "muted", "aria-hidden": "true" }, "→"), aliasInput, el("button", { type: "button", class: "button button-quiet icon-button", "aria-label": "移除 " + model.displayName, disabled: busy, onclick: () => removeDraftModel(model.modelId) }, icon("close"))),
      el("div", { class: "draft-model-meta" }, el("span", { class: "draft-native-protocol" }, protocol?.label || "协议未知", el("code", {}, protocol ? "POST " + protocol.endpoint : "端点未知")), el("span", { class: "badge badge-" + status[1], title: verified ? "验证时间：" + date(proof.verified_at) : error ? sanitize(error) : "创建后仍需验证才可发布" }, status[0])),
      el("span", { class: "sr-only" }, "别名仅改变固定账号 ID；共享模型 ID 不变。"));
  }
  function draftConfigurationProblem() {
    const aliases = new Set();
    for (const model of state.createModels.values()) {
      const alias = model.alias.trim();
      if (alias && !/^[A-Za-z0-9._-]{1,80}$/.test(alias)) return "调用别名须为 1 到 80 个字母、数字或 . _ -，或留空继承。";
      const effective = alias || model.modelId.slice("factory/".length);
      if (aliases.has(effective)) return "调用别名不能重复。";
      aliases.add(effective);
    }
    return "";
  }
  function updateDraftControls() {
    if (state.editingAccount) return;
    const data = state.createDraft;
    const busy = state.busy.has("account-form");
    const selected = [...state.createModels.keys()];
    const untested = selected.filter((id) => !draftModelVerified(id));
    if (!state.createModels.has(state.createTestModel)) state.createTestModel = selected[0] || "";
    const testModel = state.createModels.get(state.createTestModel);
    const testVerified = testModel && draftModelVerified(testModel.modelId);
    $("draft-test-model").replaceChildren(...(selected.length ? [...state.createModels.values()].map((model) => el("option", { value: model.modelId }, model.displayName + (model.alias ? " → " + model.alias : ""))) : [el("option", { value: "" }, "先添加模型")]));
    $("draft-test-model").value = state.createTestModel;
    const expired = draftExpired();
    const named = !!$("account-label").value.trim();
    const credentialsReady = named && (!!data || !!$("account-key").value.trim());
    const configurationProblem = draftConfigurationProblem();
    const configured = !configurationProblem;
    const ready = credentialsReady && configured && !expired && (!data || data.can_commit === true);
    $("account-label").disabled = $("account-key").disabled = $("account-enabled").disabled = busy || state.createCommitPending;
    $("account-key").required = !data;
    $("account-key").placeholder = data ? "草稿已持有 Key；留空沿用，输入新 Key 则重新验证" : "输入你的 Factory API Key";
    $("draft-test-models").hidden = state.createCommitPending;
    $("draft-test-models").disabled = !data || !named || busy || expired || !testModel || testVerified || !$("draft-test-confirmation").checked || !configured;
    $("draft-test-models").textContent = testVerified ? "已通过" : "测试";
    $("draft-test-model").disabled = busy || !selected.length || state.createCommitPending;
    $("save-account").disabled = busy || (!ready && !state.createCommitPending);
    $("save-account").textContent = busy ? "处理中…" : state.createCommitPending ? "重试同步（不测试）" : "创建";
    $("draft-refresh-catalog").disabled = busy || !credentialsReady || expired || state.createCommitPending;
    $("draft-refresh-catalog").lastChild.textContent = data?.discovery?.status === "candidate_catalog" ? "重新获取模型" : "获取模型";
    $("draft-clear-models").disabled = busy || !selected.length || state.createCommitPending;
    $("draft-add-manual").disabled = busy || !credentialsReady || expired || state.createCommitPending || state.createShowRoutes;
    $("draft-add-manual").textContent = state.createShowRoutes ? "已显示全部明确路由" : "手工选择路由";
    $("draft-test-confirmation").disabled = !data || busy || expired || state.createCommitPending || !testModel || testVerified;
    $("draft-cost-description").textContent = "仅测试下方选中的一个模型，会消耗额度，最多 128 输出 Token。创建不会自动测试。";
    $("draft-test-summary").textContent = expired ? "草稿已过期，请重新填写 Key。" : state.createProgress || (testVerified ? testModel.displayName + " 基础测试成功；创建不重测。" : testModel ? "该模型尚未通过测试；可先保存为待验证。" : "测试可稍后进行。");
    $("draft-test-summary").classList.toggle("test-success", !!testVerified);
    $("draft-footer-hint").textContent = configurationProblem || (state.createCommitPending ? "仅重试发布，不重复测试。" : untested.length ? untested.length + " 个模型待验证，不发布。" : selected.length ? "创建不会重复测试。" : "零模型账号也可创建。");
    const held = !!data;
    $("account-key-hint").hidden = held || state.createKeyReplaced;
    $("draft-key-status").hidden = !held && !state.createKeyReplaced;
    $("draft-key-status").textContent = held ? "当前草稿已持有 Key，输入框已清空，无需重填。输入新 Key 将清除原草稿和测试。" : state.createKeyReplaced ? "Key 已更改，原草稿和测试已清除；获取模型或创建时验证新 Key。" : "";
  }
  function renderCreateAccount() {
    if (state.editingAccount) return;
    const data = state.createDraft;
    $("account-dialog").classList.toggle("has-draft", !!data);
    $("account-credentials").hidden = false;
    $("account-draft-workspace").hidden = false;
    $("draft-test-section").hidden = false;
    $("draft-summary").hidden = !data;
    $("draft-model-picker").hidden = !data;
    if (data) {
      $("draft-identity").textContent = data.identity_verified ? "身份已验证" : "身份待确认";
      $("draft-expiry").textContent = "草稿至 " + date(data.expires_at);
      renderDraftCandidates();
    }
    const discoveryMessage = state.createCatalogError || (data?.discovery?.status === "not_configured" ? "未配置官方 CLI，可手工选择已有明确路由。" : data?.discovery?.status === "candidate_catalog" ? "候选目录不代表调用权限；测试由你主动发起。" : "");
    $("draft-catalog-notice").replaceChildren(el("p", {}, sanitize(discoveryMessage)));
    $("draft-catalog-notice").hidden = !discoveryMessage;
    $("draft-selected-count").textContent = state.createModels.size;
    $("draft-selected-models").replaceChildren(...(state.createModels.size ? [...state.createModels.values()].map(renderDraftModelCard) : [el("p", { class: "draft-selected-empty" }, "未选择模型，仍可创建账号查看额度。")]));
    updateDraftControls();
  }
  async function discardCreateDraft({ clearKey = true } = {}) {
    const draft = state.createDraft;
    state.createSequence++;
    state.createDraft = null;
    state.createModels.clear();
    state.draftPickerSelection.clear();
    state.createTestModel = "";
    state.createCommitPending = false;
    if (clearKey) $("account-key").value = "";
    if (!draft?.id || !state.token) return;
    try { await api("/drafts", "DELETE", { id: draft.id }); }
    catch (error) { if (error.status !== 404 && state.connected) notify("草稿未能立即清除，将于服务端到期失效。"); }
  }
  async function withCreateAction(work) {
    if (!state.connected || state.busy.has("account-form")) return;
    state.busy.add("account-form");
    lockDialog($("account-dialog"), true);
    showError("account-error", "");
    renderCreateAccount();
    const epoch = state.epoch;
    const sequence = state.createSequence;
    const current = () => epoch === state.epoch && sequence === state.createSequence && $("account-dialog").open;
    try { await work(current); }
    catch (error) { if (current()) showError("account-error", accessErrorMessage(error)); }
    finally {
      state.busy.delete("account-form");
      lockDialog($("account-dialog"), false);
      if (current()) renderCreateAccount();
    }
  }
  async function startCreateDraft({ discover = false, manual = false } = {}) {
    const body = { label: $("account-label").value.trim(), api_key: $("account-key").value.trim(), enabled: $("account-enabled").checked };
    if (!body.label) return showError("account-error", "请填写账号名称。");
    if (!body.api_key) return showError("account-error", "请填写 Factory API Key。");
    await withCreateAction(async (current) => {
      try {
        const data = await api("/drafts", "POST", body);
        body.api_key = "";
        if (!current()) return;
        adoptCreateDraft(data);
        state.createShowRoutes = manual;
        if (discover || manual) await loadModelMetadata();
        if (!current()) return;
        renderCreateAccount();
        if (!discover) return;
        try {
          const discovered = await api("/drafts/discovery", "POST", { id: data.id });
          if (current()) adoptCreateDraft(discovered);
        } catch (error) {
          if (!current()) return;
          state.createCatalogError = accessErrorMessage(error);
          state.createShowRoutes = true;
          const refreshed = await api("/drafts?draft_id=" + encodeURIComponent(data.id));
          if (current()) adoptCreateDraft(refreshed);
        }
      } finally { body.api_key = ""; }
    });
  }
  async function saveDraftSelection() {
    const problem = draftConfigurationProblem();
    if (problem) throw new Error(problem);
    if (draftExpired()) throw new Error("草稿已过期，请重新填写账号。");
    const modelOverrides = [...state.createModels.values()].map((model) => ({
      model_id: model.modelId, alias: model.alias.trim() || null,
    }));
    return api("/drafts/models", "PUT", { id: state.createDraft.id, model_ids: [...state.createModels.keys()], model_overrides: modelOverrides });
  }
  $("draft-candidate-search").oninput = (event) => { state.createSearch = event.target.value.trim(); renderDraftCandidates(); };
  $("draft-add-manual").onclick = async () => {
    if (!state.createDraft) await startCreateDraft({ manual: true });
    if (!state.createDraft) return;
    state.createShowRoutes = true;
    renderCreateAccount();
    openDraftPicker();
  };
  $("draft-clear-models").onclick = () => { if (!state.busy.has("account-form") && !state.createCommitPending) { state.createModels.clear(); $("draft-test-confirmation").checked = false; renderCreateAccount(); } };
  $("draft-test-confirmation").onchange = updateDraftControls;
  $("draft-test-model").onchange = (event) => { state.createTestModel = event.target.value; $("draft-test-confirmation").checked = false; updateDraftControls(); };
  $("account-label").oninput = () => { if (!state.editingAccount) updateDraftControls(); };
  $("account-key").oninput = () => {
    if (state.editingAccount || state.busy.has("account-form") || state.createCommitPending) return;
    if (state.createDraft && $("account-key").value.length) {
      state.createKeyReplaced = true;
      state.createCatalogError = state.createProgress = "";
      state.createShowRoutes = false;
      state.createSearch = "";
      $("draft-test-confirmation").checked = false;
      discardCreateDraft({ clearKey: false });
      renderCreateAccount();
    } else updateDraftControls();
  };
  $("draft-refresh-catalog").onclick = async () => {
    if (!state.createDraft) await startCreateDraft({ discover: true });
    else await withCreateAction(async (current) => {
      const id = state.createDraft?.id;
      if (!id) return;
      try {
        const data = await api("/drafts/discovery", "POST", { id });
        if (!current()) return;
        adoptCreateDraft(data);
        state.createCatalogError = "";
        $("draft-test-confirmation").checked = false;
        for (const model of state.createModels.values()) if (model.source === "cli_candidate") model.snapshotId = null;
      } catch (error) {
        if (!current()) return;
        state.createCatalogError = accessErrorMessage(error);
        const data = await api("/drafts?draft_id=" + encodeURIComponent(id));
        if (current()) adoptCreateDraft(data);
        throw error;
      }
    });
    if (state.createDraft && $("account-dialog").open) openDraftPicker();
  };
  $("draft-test-models").onclick = () => {
    const model = state.createModels.get(state.createTestModel);
    if (!$("draft-test-confirmation").checked || !model || draftModelVerified(model.modelId)) return;
    return withCreateAction(async (current) => {
      const data = await saveDraftSelection();
      if (!current()) return;
      adoptCreateDraft(data);
      if (draftModelVerified(model.modelId)) return;
      state.createProgress = "正在测试 " + model.displayName + "，此次请求会消耗额度。";
      renderCreateAccount();
      const body = { id: state.createDraft.id, model_id: model.modelId, confirm: true };
      if (model.source === "cli_candidate") {
        const discovery = state.createDraft.discovery;
        const candidate = discovery?.candidates?.find((item) => item.route_models?.includes(model.modelId));
        if (!candidate || candidateProblem(candidate, state.createDraft)) throw new Error("候选已变化或过期，请重新获取后选择。");
        body.discovery_snapshot_id = discovery.snapshot_id;
      }
      try {
        const tested = await api("/drafts/models/test", "POST", body);
        if (!current()) return;
        adoptCreateDraft(tested);
        state.createProgress = draftModelVerified(model.modelId) ? "" : "基础测试未通过，可稍后手工重试。";
      } catch (error) {
        if (!current()) return;
        state.createProgress = "测试未成功，不会自动重试。";
        if (error.managementAuth || error.code === "draft_not_found") throw error;
        const refreshed = await api("/drafts?draft_id=" + encodeURIComponent(state.createDraft.id));
        if (current()) adoptCreateDraft(refreshed);
        throw error;
      } finally { $("draft-test-confirmation").checked = false; }
    });
  };
  async function commitCreateDraft() {
    return withCreateAction(async (current) => {
      if (!state.createCommitPending) {
        const selected = await saveDraftSelection();
        if (!current()) return;
        adoptCreateDraft(selected);
        if (selected.can_commit !== true) throw new Error("草稿身份尚未确认有效，请重新填写账号。不会自动发送推理请求。");
      }
      const id = state.createDraft.id;
      const selectedCount = state.createModels.size;
      try {
        const account = await api("/drafts/commit", "POST", { id, label: $("account-label").value.trim(), enabled: $("account-enabled").checked });
        if (!current()) return;
        state.createDraft = null;
        state.createModels.clear();
        $("account-dialog").close();
        await loadAll();
        if (selectedCount && account?.id) manageAccountModels(account.id);
        else setView("accounts");
        notify(selectedCount ? "账号已创建；模型发布以 CPA 确认状态为准" : "账号已创建，尚未发布模型；可刷新读取官方额度");
      } catch (error) {
        if (!current()) return;
        const refreshed = await api("/drafts?draft_id=" + encodeURIComponent(id));
        if (!current()) return;
        adoptCreateDraft(refreshed);
        if (refreshed.committed_account_id) state.createProgress = "账号已保存，CPA 发布等待同步。请点击“重试同步（不测试）”。";
        throw error;
      }
    });
  }
  async function runAccount(account, path, method, body, message) {
    if (state.busy.has(account.id)) return;
    const actionFocus = document.activeElement?.dataset?.focusKey;
    const originalView = state.view;
    state.busy.add(account.id);
    const epoch = state.epoch;
    if (path === "/accounts" && method === "PATCH" && typeof body.enabled === "boolean") {
      state.accounts = state.accounts.map((item) => item.id === account.id ? { ...item, enabled: body.enabled } : item);
    }
    renderAccounts();
    try {
      const updated = await api(path, method, body);
      if (epoch !== state.epoch) return;
      if (updated?.id) state.accounts = state.accounts.map((item) => item.id === updated.id ? updated : item);
      await loadAll();
      notify(message);
    } catch (error) {
      if (epoch !== state.epoch) return;
      state.accounts = state.accounts.map((item) => item.id === account.id ? account : item);
      showError("global-error", error.message);
      notify(error.message);
    } finally {
      state.busy.delete(account.id);
      renderAccounts();
      if (actionFocus && state.view === originalView && document.activeElement === document.body) {
        [...$("accounts-grid").querySelectorAll("[data-focus-key]")].find((node) => node.dataset.focusKey === actionFocus && !node.disabled)?.focus({ preventScroll: true });
      }
    }
  }
  function openDelete(account) {
    state.deletingAccount = account;
    $("delete-description").textContent = "即将删除“" + account.label + "”。此账号将不再用于后续模型请求。";
    showError("delete-error", "");
    openDialog("delete-dialog");
  }
  $("delete-form").onsubmit = async (event) => {
    event.preventDefault();
    const account = state.deletingAccount;
    if (!account || state.busy.has("delete")) return;
    state.busy.add("delete");
    const epoch = state.epoch;
    const restore = pending($("confirm-delete"), "正在删除…");
    lockDialog($("delete-dialog"), true);
    showError("delete-error", "");
    try {
      await api("/accounts", "DELETE", { id: account.id });
      if (epoch !== state.epoch) return;
      if (state.usageAccount === account.id) state.usageAccount = "";
      $("delete-dialog").close();
      await loadAll();
      notify("账号已删除，历史用量记录已保留");
    } catch (error) { showError("delete-error", error.message); if (!$("delete-dialog").open) notify("删除已完成，但刷新失败；请刷新确认。"); }
    finally { state.busy.delete("delete"); restore(); lockDialog($("delete-dialog"), false); }
  };
  function detailRows(rows) {
    const list = el("dl", { class: "detail-list" });
    for (const [label, value] of rows) list.append(el("dt", {}, label), el("dd", {}, value));
    return list;
  }
  function pinnedModelList(account) {
    const list = el("div", { class: "pinned-model-list" });
    const models = Array.isArray(account?.pinned_models) ? account.pinned_models : [];
    if (!models.length) list.append(el("p", { class: "muted" }, "此账号尚无实际发布的固定模型。请在“模型允许”中完成基础验证并允许，再确认 CPA 发布状态。"));
    for (const model of models) {
      if (typeof model.id !== "string" || !model.id) continue;
      const configured = (state.settings?.models || []).find((item) => item.id === model.base_id) || state.models.find((item) => item.id === model.base_id) || (account.model_access || []).find((item) => item.model_id === model.base_id);
      const pinned = { ...configured, ...model, protocol: configured ? configured.protocol : model.protocol };
      const copy = el("button", { type: "button", class: "button button-soft icon-button", title: "复制固定账号模型 ID", "aria-label": "复制固定账号模型 ID：" + model.id }, icon("copy"));
      copy.onclick = () => copyModel(model.id, copy);
      list.append(el("div", { class: "pinned-model-row" },
        el("div", { class: "pinned-model-heading" }, el("strong", {}, asText(model.display_name, model.base_id || model.id)), copy),
        el("code", { class: "pinned-model-id" }, model.id),
        el("small", {}, "共享模型：" + asText(model.base_id)), modelProtocolInfo(pinned)));
    }
    return list;
  }
  function pinnedDetail(account, expanded = false) {
    return el("details", { class: "detail-section pinned-detail", open: expanded },
      el("summary", {}, "固定账号模型", el("span", { class: "badge badge-" + (account.enabled ? "brand" : "neutral") }, account.enabled ? "固定使用此账号" : "账号已停用")),
      pinnedModelList(account));
  }
  function detailSection(title, ...children) {
    return el("section", { class: "detail-section" }, el("h3", {}, title), ...children.filter(Boolean));
  }
  function detailConfigForm(account) {
    if (state.detailConfigDraft?.accountId !== account.id) {
      state.detailConfigDraft = { accountId: account.id, label: account.label || "", priority: account.priority ?? 0, enabled: account.enabled === true };
    }
    const draft = state.detailConfigDraft;
    const busy = state.busy.has("detail-config");
    const reset = el("button", { type: "button", class: "button button-secondary", disabled: busy || !state.detailConfigDirty, onclick: () => { state.detailConfigDraft = null; state.detailConfigDirty = false; state.detailConfigError = ""; renderAccountDetail(); } }, "重置");
    const save = el("button", { type: "submit", class: "button button-primary", disabled: busy || !state.detailConfigDirty }, "保存");
    const label = el("input", { type: "text", value: draft.label, maxLength: 80, required: true, autocomplete: "off", disabled: busy });
    const priority = el("input", { type: "number", value: draft.priority, min: -100, max: 100, step: 1, title: "数值较高的账号优先", disabled: busy });
    const enabled = el("input", { type: "checkbox", checked: draft.enabled, disabled: busy });
    const markDirty = () => { state.detailConfigDirty = true; state.detailConfigError = ""; reset.disabled = save.disabled = false; error.hidden = true; };
    label.oninput = () => { draft.label = label.value; markDirty(); };
    priority.oninput = () => { draft.priority = priority.value; markDirty(); };
    enabled.onchange = () => { draft.enabled = enabled.checked; markDirty(); };
    const error = el("div", { id: "detail-config-error", class: "field-error", role: "alert", hidden: !state.detailConfigError }, sanitize(state.detailConfigError));
    const form = el("form", { class: "detail-config-form" },
      el("div", { class: "detail-config-actions" }, reset, save),
      el("section", { class: "detail-config-fields" }, el("label", {}, "账号名称", label), el("label", {}, "优先级", priority), el("label", { class: "checkbox-label" }, enabled, el("span", {}, "启用此账号"))),
      error,
      detailSection("账号信息", detailRows([["账号类型", account.kind === "oauth" ? "OAuth" : "API Key"], ["凭据提示", keyHint(account.credential_hint)], ["邮箱", asText(account.identity?.email)], ["组织", asText(account.identity?.organization)], ["区域", asText(account.identity?.region)], ["最近验证", date(account.last_checked_at)], ["最近更新", date(account.updated_at)], ["创建时间", date(account.created_at)]])));
    form.onsubmit = async (event) => {
      event.preventDefault();
      if (state.busy.has("detail-config")) return;
      if (!label.value.trim()) { state.detailConfigError = "请填写账号名称。"; error.textContent = state.detailConfigError; error.hidden = false; return; }
      const accountId = account.id, sequence = state.detailSequence, epoch = state.epoch;
      const current = () => epoch === state.epoch && sequence === state.detailSequence && state.detailAccount?.id === accountId && $("detail-dialog").open;
      state.busy.add("detail-config");
      state.detailConfigError = "";
      save.disabled = reset.disabled = label.disabled = priority.disabled = enabled.disabled = true;
      error.hidden = true;
      try {
        await api("/accounts", "PATCH", { id: accountId, label: label.value.trim(), enabled: enabled.checked, priority: Number(priority.value) });
        await loadAll();
        if (!current()) return;
        state.detailAccount = state.accounts.find((item) => item.id === accountId) || account;
        state.detailConfigDraft = null;
        state.detailConfigDirty = false;
        notify("账号设置已保存");
      } catch (error) { if (current()) state.detailConfigError = error.message; }
      finally { state.busy.delete("detail-config"); if (current()) renderAccountDetail(); }
    };
    return form;
  }

  function renderAccountDetail() {
    const account = state.detailAccount;
    if (!account) return;
    const usage = account.usage || {};
    const official = account.official_usage || {};
    const summary = requestSummary(usage);
    const completeTokens = usage.credits_complete !== false;
    const tokenTotal = completeTokens && finite(usage.input_tokens) && finite(usage.output_tokens) ? usage.input_tokens + usage.output_tokens : null;
    const status = accountStatus(account);
    const showModels = state.detailTab === "models";
    $("account-detail").hidden = showModels;
    $("account-models-content").hidden = !showModels;
    $("detail-title").textContent = account.label || "账号详情";
    $("detail-subtitle").textContent = [account.plan, account.identity?.email, account.auth_file].filter(Boolean).join(" · ") || keyHint(account.credential_hint);
    $("detail-toggle").textContent = account.enabled ? "禁用" : "启用";
    $("detail-toggle").disabled = $("detail-refresh").disabled = state.busy.has(account.id);
    document.querySelectorAll("[data-detail-tab]").forEach((button) => button.setAttribute("aria-selected", String(button.dataset.detailTab === state.detailTab)));
    let content = [];
    if (state.detailTab === "overview") {
      content = [
        el("section", { class: "detail-availability" }, el("div", {}, icon("shield"), el("strong", {}, "当前可用性")), el("span", { class: "badge badge-" + status[1] }, account.enabled && status[0] === "账号已验证" ? "可用" : status[0]), detailRows([["验证状态", status[0]], ["最近检查", date(account.last_checked_at)]])),
        detailSection("历史用量", el("div", { class: "mini-stats" }, miniStat("请求", compact(summary.total), "send"), miniStat("Token", compact(tokenTotal), "token"), miniStat("费用", finite(usage.estimated_cost_usd) ? usd(usage.estimated_cost_usd) : "—", "dollar"), miniStat("成功率", summary.rate !== null ? format(summary.rate) + "%" : "—", "check"))),
        renderRecent(usage),
        renderOfficialLimits(account.official_limits, account.prediction, true)
      ];
    } else if (state.detailTab === "quota") {
      const estimatedWindows = visibleQuotaWindows(account.prediction?.windows).filter((window) => window.status === "estimated");
      content = [detailSection("官方用量", detailRows([["已使用", officialLabel(official)], ["额外余额", finite(account.official_limits?.extra_usage_balance_usd) ? usd(account.official_limits.extra_usage_balance_usd) + " USD" : "—"], ["周期", period(official.window_start, official.window_end)], ["查询时间", date(official.checked_at)]])), detailSection("额度与恢复", renderOfficialLimits(account.official_limits, account.prediction), ...estimatedWindows.map((window) => detailRows([["估算窗口", asText(window.name)], ["基准快照", date(window.baseline_at)], ["采样快照", date(window.sampled_at)], ["额度变化", format(window.delta_used_percent) + "%"], ["样本 token", count(window.sample_tokens)], ["样本请求", count(window.sample_requests)]])))];
    } else if (state.detailTab === "config") {
      content = [detailConfigForm(account)];
    } else if (state.detailTab === "models") {
      content = [];
    } else {
      content = [detailSection("本地记录", detailRows([["统计周期", period(usage.window_start, usage.window_end)], ["成功 / 失败", count(summary.successful) + " / " + count(summary.failed)], ["包含 usage", count(usage.requests_with_usage) + " 次"], ["输入 token", count(usage.input_tokens)], ["输出 token", count(usage.output_tokens)], ["缓存输入 token", count(usage.cached_input_tokens)], ["缓存写入 token", count(usage.cache_write_tokens)], ["推理 token", count(usage.reasoning_tokens)], ["本地估算", format(usage.estimated_credits) + " 加权 token"], ["估算费用", finite(usage.estimated_cost_usd) ? usd(usage.estimated_cost_usd) + " USD" : "—"], ["记录范围", asText(usage.coverage, "—")]])), detailSection("官方查询", detailRows([["额度状态", asText(account.official_limits?.status)], ["额度信息", asText(account.official_limits?.message)], ["消耗状态", asText(official.status)], ["消耗信息", asText(official.message)]])), account.last_error ? el("p", { class: "field-error" }, sanitize(account.last_error)) : null];
    }
    $("account-detail").replaceChildren(...content.filter(Boolean));
  }
  function openDetail(account, modelsFirst = false, tab = "overview") {
    if (state.detailAccount?.id !== account.id) {
      state.detailConfigDirty = false;
      state.detailConfigDraft = null;
      state.detailConfigError = "";
    }
    state.detailSequence++;
    state.detailAccount = account;
    state.detailTab = modelsFirst ? "models" : tab;
    renderAccountDetail();
    openDialog("detail-dialog");
  }
  document.querySelectorAll("[data-detail-tab]").forEach((button) => { button.onclick = () => {
    if (button.dataset.detailTab === "models" && state.detailAccount) return manageAccountModels(state.detailAccount.id);
    state.detailTab = button.dataset.detailTab;
    renderAccountDetail();
  }; });
  $("detail-refresh").onclick = async () => {
    const account = state.detailAccount;
    if (!account) return;
    const accountId = account.id, sequence = state.detailSequence, epoch = state.epoch;
    await runAccount(account, "/accounts/refresh", "POST", { id: account.id }, "额度已刷新");
    if (epoch === state.epoch && sequence === state.detailSequence && state.detailAccount?.id === accountId && $("detail-dialog").open) {
      state.detailAccount = state.accounts.find((item) => item.id === accountId) || account;
      renderAccountDetail();
    }
  };
  $("detail-toggle").onclick = async () => {
    const account = state.detailAccount;
    if (!account) return;
    const accountId = account.id, sequence = state.detailSequence, epoch = state.epoch;
    await runAccount(account, "/accounts", "PATCH", { id: account.id, enabled: !account.enabled }, account.enabled ? "账号已停用" : "账号已启用");
    if (epoch === state.epoch && sequence === state.detailSequence && state.detailAccount?.id === accountId && $("detail-dialog").open) {
      state.detailAccount = state.accounts.find((item) => item.id === accountId) || account;
      renderAccountDetail();
    }
  };

  function renderUsage() {
    const usage = state.usage;
    const budgetUsage = state.status?.usage || (!state.usageAccount ? usage : null);
    const summary = requestSummary(usage);
    $("usage-window").lastElementChild.textContent = usage ? period(usage.window_start, usage.window_end) : state.busy.has("usage") ? "正在加载统计…" : "统计周期待连接";
    $("usage-stats").setAttribute("aria-busy", String(state.busy.has("usage")));
    const stats = [
      [state.usageKind === "validation" ? "基础验证请求" : state.usageKind === "inference" ? "业务调用请求" : "总请求（含验证）", compact(summary.total), "成功 " + count(summary.successful) + " · 失败 " + count(summary.failed)],
      ["输入 token", compact(usage?.input_tokens), "包含缓存输入"],
      ["输出 token", compact(usage?.output_tokens), "上游实际返回的 usage"],
      ["缓存输入 token", compact(usage?.cached_input_tokens), "输入 token 中命中的缓存"],
      ["缓存写入 token", compact(usage?.cache_write_tokens), "缓存创建量 · 不重复加总"],
      ["推理 token", compact(usage?.reasoning_tokens), "已包含在输出 token 中"],
      ["本地估算", compact(usage?.estimated_credits), "加权 token · 非官方 FSC"],
      ["估算费用", finite(usage?.estimated_cost_usd) ? usd(usage.estimated_cost_usd) : "—", "USD · 按用户配置单价"],
      ["验证请求", count(usage?.validation_requests), "失败验证 " + count(usage?.failed_validation_requests) + " · 计入总预算"],
      ["业务请求", count(usage?.inference_requests), "验证与业务分别记录"]
    ];
    $("usage-stats").replaceChildren(...stats.map(([label, value, hint]) => el("div", { class: "usage-stat" }, el("span", {}, label), el("strong", {}, value), el("small", {}, hint))));
    const overview = $("budget-overview");
    overview.replaceChildren();
    if (!budgetUsage) overview.append(el("p", { class: "muted" }, "连接后查看当前周期预算。"));
    else if (finite(budgetUsage.budget_credits)) {
      overview.append(el("div", { class: "budget-values" }, el("strong", {}, finite(budgetUsage.remaining_credits) ? format(budgetUsage.remaining_credits) : "未知"), el("span", {}, "剩余 / " + format(budgetUsage.budget_credits) + " 加权 token")),
        progress(budgetUsage.estimated_credits, budgetUsage.budget_credits), el("p", { class: "muted" }, "全部账号已估算 " + format(budgetUsage.estimated_credits) + " · " + remainingTime(budgetUsage.window_end)));
    } else overview.append(el("div", { class: "budget-values" }, el("strong", {}, "未设置"), el("span", {}, "本地预算")), el("p", { class: "muted" }, "设置预算后显示估算进度；官方余额仍以官方接口为准。"));
    const accounts = state.accounts.filter((account) => !state.usageAccount || account.id === state.usageAccount);
    $("official-usage").replaceChildren(...(accounts.length ? accounts.map((account) => el("div", { class: "official-entry" },
      el("div", { class: "official-entry-head" }, el("strong", {}, account.label), el("span", {}, officialLabel(account.official_usage))),
      el("p", {}, account.official_usage?.status === "available" ? period(account.official_usage.window_start, account.official_usage.window_end) : asText(account.official_usage?.message, "尚无官方用量数据")),
      el("p", {}, "更新时间：" + date(account.official_usage?.checked_at))
    )) : [el("p", { class: "muted" }, state.connected ? "添加账号后可查询官方用量。" : "连接后按账号显示官方用量。")]));
    const modelUsage = Array.isArray(usage?.models) ? usage.models : [];
    $("usage-model-rows").replaceChildren(...(modelUsage.length ? modelUsage.map((model) => el("tr", {},
      el("td", {}, el("code", {}, asText(model.model))), el("td", {}, count(model.requests)), el("td", {}, count(model.input_tokens)), el("td", {}, count(model.output_tokens)), el("td", {}, count(model.cached_input_tokens)), el("td", {}, format(model.estimated_credits)), el("td", {}, finite(model.estimated_cost_usd) ? usd(model.estimated_cost_usd) : "—")
    )) : [el("tr", {}, el("td", { colspan: 7, class: "table-empty" }, state.connected ? "当前周期暂无模型用量记录" : "管理授权后查看模型用量"))]));
    $("usage-coverage").textContent = asText(usage?.coverage, "仅统计本插件记录的请求；缺少上游 usage 的请求无法估算 token。") + " 全局预算包含基础验证和业务调用，验证记录不会伪装为业务请求。";
  }
  function syncUsageScope() {
    const select = $("usage-account");
    const known = state.accounts.some((account) => account.id === state.usageAccount);
    if (state.usageAccount && !known) state.usageAccount = "";
    select.replaceChildren(el("option", { value: "" }, "全部账号"), ...state.accounts.map((account) => el("option", { value: account.id }, account.label)));
    select.value = state.usageAccount;
    $("usage-kind").value = state.usageKind;
  }
  function usagePath() {
    const query = new URLSearchParams({ kind: state.usageKind });
    if (state.usageAccount) query.set("account_id", state.usageAccount);
    return "/usage?" + query.toString();
  }
  async function loadUsageSelection() {
    if (state.busy.has("usage")) return;
    state.usage = null;
    state.busy.add("usage");
    renderUsage();
    $("usage-account").disabled = $("usage-kind").disabled = true;
    const epoch = state.epoch;
    try {
      const usage = await api(usagePath());
      if (epoch !== state.epoch) return;
      state.usage = usage;
      renderUsage();
    } catch (error) { showError("global-error", error.message); }
    finally { state.busy.delete("usage"); $("usage-account").disabled = $("usage-kind").disabled = !state.connected; renderUsage(); }
  }
  $("usage-account").onchange = (event) => { state.usageAccount = event.target.value; loadUsageSelection(); };
  $("usage-kind").onchange = (event) => { state.usageKind = event.target.value; loadUsageSelection(); };
  function syncBudgetForm() {
    if (state.budgetDirty || !state.settings) return;
    const settings = state.settings;
    $("budget-window").value = settings.usage_window || "calendar_month";
    $("budget-amount").value = finite(settings.budget_credits) ? settings.budget_credits : "";
    $("budget-start").value = settings.period_start ? new Date(settings.period_start).toISOString().slice(0, 16) : "";
    $("budget-end").value = settings.period_end ? new Date(settings.period_end).toISOString().slice(0, 16) : "";
    $("custom-dates").hidden = $("budget-window").value !== "custom";
    $("budget-start").required = $("budget-end").required = $("budget-window").value === "custom";
    $("budget-unsaved").textContent = "";
  }
  $("budget-form").addEventListener("input", () => { state.budgetDirty = true; $("budget-unsaved").textContent = "有未保存的修改"; });
  $("budget-window").onchange = () => {
    const custom = $("budget-window").value === "custom";
    $("custom-dates").hidden = !custom;
    $("budget-start").required = $("budget-end").required = custom;
    state.budgetDirty = true;
    $("budget-unsaved").textContent = "有未保存的修改";
  };
  function optionalNumber(value, label) {
    if (value === "") return null;
    const number = Number(value);
    if (!Number.isFinite(number) || number < 0) throw new Error(label + "必须是非负有限数。");
    return number;
  }
  $("budget-form").onsubmit = async (event) => {
    event.preventDefault();
    if (state.busy.has("settings")) return;
    showError("budget-error", "");
    let patch;
    try {
      const custom = $("budget-window").value === "custom";
      const start = custom ? new Date($("budget-start").value + "Z") : null;
      const end = custom ? new Date($("budget-end").value + "Z") : null;
      if (custom && (!Number.isFinite(start.getTime()) || !Number.isFinite(end.getTime()) || end <= start)) throw new Error("结束时间必须晚于开始时间，请按 UTC 填写。");
      patch = { usage_window: custom ? "custom" : "calendar_month", budget_credits: optionalNumber($("budget-amount").value, "预算"), period_start: start?.toISOString() || null, period_end: end?.toISOString() || null };
    } catch (error) { return showError("budget-error", error.message); }
    state.busy.add("settings");
    const epoch = state.epoch;
    const credential = state.token;
    setConnection(state.connected);
    const restore = pending($("save-budget"), "正在保存…");
    try {
      const current = await api("/settings", "GET", undefined, credential);
      if (epoch !== state.epoch) return;
      const saved = await api("/settings", "PUT", { ...current, ...patch }, credential);
      if (epoch !== state.epoch) return;
      state.settings = saved;
      state.budgetDirty = false;
      await loadAll();
      notify("周期预算已保存");
    } catch (error) { showError("budget-error", error.message); }
    finally { state.busy.delete("settings"); restore(); setConnection(state.connected); }
  };

  async function copyText(textValue, button, successMessage = "内容已复制") {
    try {
      if (!navigator.clipboard?.writeText) throw new Error("clipboard-unavailable");
      await navigator.clipboard.writeText(textValue);
      notify(successMessage);
    } catch {
      const text = el("textarea", { value: textValue, readonly: true, class: "sr-only", "aria-label": "待复制的内容" });
      document.body.append(text);
      text.select();
      let copied = false;
      try { copied = document.execCommand("copy"); } catch { /* 保留明确失败反馈。 */ }
      text.remove();
      button?.focus();
      notify(copied ? successMessage : "浏览器未允许复制，请手动选中内容复制。");
    }
  }
  function copyModel(id, button) { return copyText(id, button, "模型 ID 已复制"); }
  function protocolMeta(protocol) {
    return typeof protocol === "string" && Object.hasOwn(PROTOCOLS, protocol) ? PROTOCOLS[protocol] : null;
  }
  function modelProtocolInfo(model) {
    const protocol = protocolMeta(model.protocol);
    if (!protocol) return el("div", { class: "model-native-info" }, el("span", { class: "badge badge-warning" }, "原生协议配置未知"), el("p", { class: "form-hint" }, "请检查模型 protocol，不能根据名称推断调用方式。"));
    return el("div", { class: "model-native-info" },
      el("span", { class: "model-protocol-name" }, protocol.label),
      el("div", { class: "model-endpoint" }, el("code", {}, "POST " + protocol.endpoint),
        el("button", { type: "button", class: "button button-soft button-call", "aria-label": "查看调用示例：" + model.id, onclick: () => openCallExample(model) }, icon("terminal"), "调用示例")));
  }
  function openCallExample(model) {
    const protocol = protocolMeta(model.protocol);
    if (!protocol) return notify("模型原生协议配置未知，请检查 protocol。");
    const body = model.protocol === "anthropic"
      ? { model: model.id, max_tokens: 1024, messages: [{ role: "user", content: "你好" }], stream: false }
      : model.protocol === "responses"
        ? { model: model.id, input: "你好", stream: false }
        : { model: model.id, messages: [{ role: "user", content: "你好" }], stream: false };
    const headers = [
      "POST <CPA_INFERENCE_BASE_URL>" + protocol.endpoint,
      "Authorization: Bearer <CPA_CLIENT_API_KEY>",
      "Content-Type: application/json",
      ...(model.protocol === "anthropic" ? ["anthropic-version: 2023-06-01"] : [])
    ];
    $("call-title").textContent = asText(model.display_name, model.id) + " · 调用示例";
    $("call-protocol").replaceChildren(el("span", { class: "badge badge-brand" }, protocol.label), el("code", {}, "POST " + protocol.endpoint));
    $("call-example").textContent = headers.join("\n") + "\n\n" + JSON.stringify(body, null, 2);
    openDialog("call-dialog");
  }
  $("copy-call-example").onclick = () => copyText($("call-example").textContent, $("copy-call-example"), "调用示例已复制");
  const ACCESS_REASONS = Object.freeze({
    account_deleted: "账号已删除", account_disabled: "账号已停用", identity_not_verified: "账号身份未验证",
    global_disabled: "模型定义已停用", basic_call_not_verified: "尚无有效基础调用验证", not_allowed: "尚未允许",
    credential_changed: "凭据或组织已变化，旧验证失效", route_changed: "原生路由或能力配置已变化，旧验证失效",
    candidate_disabled: "目录明确禁用", candidate_deprecated: "目录已弃用", candidate_custom: "自定义候选不可直接验证",
    candidate_router: "路由/自动选择候选不可直接验证", invalid_candidate_metadata: "候选元数据无效",
    candidate_protocol_mismatch: "模型配置的协议与官方目录不一致，请修正模型配置",
    unsupported_native_protocol: "Google 原生协议暂未支持，不能通过其他协议调用",
    unknown_route: "模型的原生协议或上游尚未确认，需补充模型配置", discovery_stale: "目录快照已过期",
    catalog_refresh_incomplete: "CPA 发布尚未同步", publication_pending: "CPA 发布待同步",
    host_not_registered: "CPA 尚未注册", candidate_blocked: "候选受到目录限制"
  });
  const HARD_VERIFY_REASONS = new Set(["account_deleted", "account_disabled", "identity_not_verified", "global_disabled", "candidate_disabled", "candidate_deprecated", "candidate_custom", "candidate_router", "invalid_candidate_metadata", "candidate_protocol_mismatch", "unsupported_native_protocol"]);
  const CANDIDATE_PAGE_SIZE = 10;
  const selectedModelAccount = () => state.accounts.find((account) => account.id === state.modelAccount);
  const selectedAccountModels = () => state.accountModelCache.get(state.modelAccount) || null;
  const reasonText = (code) => ACCESS_REASONS[code] || "未识别限制：" + String(code);
  function reasonList(reasons) {
    return Array.isArray(reasons) && reasons.length ? reasons.map(reasonText).join("；") : "";
  }
  function verificationLabel(status) {
    return ({ not_verified: "未验证", validating: "正在基础验证", basic_call_verified: "基础调用已验证", failed: "基础验证失败", invalidated: "原验证已失效" })[status] || "验证状态未知";
  }
  function selectionLabel(source) {
    if (source === "official_droid_cli" || source === "cli_candidate") return "CLI 候选选择";
    if (source === "manual" || source === "manual_route") return "手工配置路由";
    return "未记录选择方式";
  }
  function publicationPending(model) {
    if (model?.publication_status === "pending_add" || model?.publication_status === "pending_remove") return true;
    if (model?.published === true) return model.allowed === false;
    if (model?.published !== false || model.allowed !== true || model.verification_status !== "basic_call_verified") return false;
    return !(model.blocked_reasons || []).some((reason) => HARD_VERIFY_REASONS.has(reason) || reason === "credential_changed" || reason === "route_changed");
  }
  function publicationLabel(model) {
    if (model?.publication_status === "pending_remove") return ["撤销待同步", "warning"];
    if (model?.publication_status === "pending_add") return ["发布待同步", "warning"];
    if (model?.publication_status === "unknown") return ["发布状态未知", "neutral"];
    if (model?.published === true) return model.allowed === false ? ["撤销待同步", "warning"] : ["CPA 已发布", "success"];
    if (publicationPending(model)) return ["发布待同步", "warning"];
    return model?.published === false ? ["未发布", "neutral"] : ["发布状态未知", "neutral"];
  }
  function accessErrorMessage(error) {
    const status = error.status ? "（HTTP " + error.status + "）" : "";
    return sanitize(error.message) + status + (error.retryAfter ? "；Retry-After：" + sanitize(error.retryAfter) : "");
  }
  function discoveryFresh(discovery) {
    return discovery?.status === "candidate_catalog" && discovery.stale === false && typeof discovery.snapshot_id === "string" && discovery.snapshot_id.length > 0 && new Date(discovery.fresh_until || "").getTime() > Date.now();
  }
  function verificationProblem(model, data = selectedAccountModels()) {
    if (!state.connected || !data) return "请先读取账号模型状态。";
    if (state.accessLoading || state.accessError) return "请先刷新账号状态。";
    const account = selectedModelAccount();
    if (!account?.enabled) return "账号已停用，请先启用账号。";
    if (data.identity_verified !== true) return "账号身份尚未验证，请先检查凭据。";
    if (!model || !protocolMeta(model.protocol)) return "没有协议明确的已配置原生路由。";
    if (model.global_enabled === false) return "模型定义已停用。";
    if (model.verification_status === "validating") return "此模型正在基础验证，请刷新状态确认结果。";
    if (publicationPending(model)) return "CPA 发布状态等待同步，请使用“重试同步（不验证）”，不重复发送基础验证。";
    const hard = (model.blocked_reasons || []).filter((reason) => HARD_VERIFY_REASONS.has(reason));
    return hard.length ? reasonList(hard) : "";
  }
  function candidateProblem(candidate, data = selectedAccountModels()) {
    if (!discoveryFresh(data?.discovery)) return "候选快照已过期或尚未取得，请刷新目录。";
    const reasons = [...(Array.isArray(candidate.blocked_reasons) ? candidate.blocked_reasons : [])];
    if (candidate.disabled === true) reasons.push("candidate_disabled");
    if (candidate.deprecated === true) reasons.push("candidate_deprecated");
    if (candidate.is_custom === true) reasons.push("candidate_custom");
    if (["router", "auto", "automatic", "local"].includes(String(candidate.kind || "").toLowerCase())) reasons.push("candidate_router");
    if (reasons.length) return reasonList([...new Set(reasons)]);
    return "";
  }
  function syncModelAccountScope() {
    if (!state.accounts.some((account) => account.id === state.modelAccount)) {
      state.modelAccount = state.accounts.find((account) => account.enabled)?.id || state.accounts[0]?.id || "";
      state.accessError = null;
      state.candidateRoutes.clear();
      state.candidatePage = 1;
    }
    $("model-account").replaceChildren(
      ...(state.accounts.length ? state.accounts.map((account) => el("option", { value: account.id }, account.label + (account.enabled ? "" : "（已停用）"))) : [el("option", { value: "" }, "请先添加账号")]));
    $("model-account").value = state.modelAccount;
    updateAccessControls();
  }
  function renderModelSection() {
    const access = state.modelsSection === "access";
    $("model-access-section").hidden = !access;
    $("model-definitions-section").hidden = access;
    $("definition-actions").hidden = access;
    document.querySelectorAll("[data-model-section]").forEach((button) => {
      const selected = button.dataset.modelSection === state.modelsSection;
      button.classList.toggle("button-soft", selected);
      button.classList.toggle("button-quiet", !selected);
      button.setAttribute("aria-pressed", String(selected));
    });
  }
  document.querySelectorAll("[data-model-section]").forEach((button) => {
    button.onclick = () => {
      state.modelsSection = button.dataset.modelSection;
      renderModelSection();
      if (state.modelsSection === "access") ensureAccountModels();
    };
  });
  async function manageGlobalModels() {
    if (!state.connected) return;
    state.modelsSection = "definitions";
    renderModelSection();
    renderModels();
    openDialog("global-models-dialog");
    try {
      await loadModelMetadata(true);
      if ($("global-models-dialog").open) renderModels();
    } catch (error) { showError("model-error", error.message); }
  }
  $("manage-models").onclick = manageGlobalModels;

  async function manageAccountModels(accountId) {
    const account = state.accounts.find((item) => item.id === accountId);
    if (!account) return;
    openDetail(account, true);
    if (state.modelAccount !== accountId) {
      state.modelAccount = accountId;
      state.accessError = null;
      state.accessLoadSequence++;
      state.accessLoading = false;
      state.candidateRoutes.clear();
      state.candidatePage = 1;
    }
    state.modelsSection = "access";
    syncModelAccountScope();
    renderModelSection();
    renderAccountModels();
    try {
      await loadModelMetadata();
      if (!$("detail-dialog").open || state.detailTab !== "models" || state.modelAccount !== accountId) return;
      await ensureAccountModels(true);
    } catch (error) { showError("access-error", error.message); }
  }
  function updateAccessControls() {
    const account = selectedModelAccount();
    const busy = state.busy.has("access");
    $("model-account").disabled = !state.connected || !state.accounts.length || busy;
    $("reload-account-models").disabled = !state.connected || !account || busy || state.accessLoading;
    const discovery = selectedAccountModels()?.discovery || account?.discovery;
    const noCli = state.status?.capabilities?.cli_discovery_configured === false || discovery?.status === "not_configured";
    $("refresh-discovery").disabled = !state.connected || !account || busy || state.accessLoading || noCli;
    $("refresh-discovery").title = noCli ? "请管理员先配置 droid-cli-path；仍可手工基础验证已配置路由。" : "读取 CLI 候选目录，不发送推理请求";
    $("reconcile-models").disabled = !state.connected || !account || busy;
    updateVerifyControls();
  }
  function adoptAccountModels(data) {
    if (!data || typeof data.account_id !== "string" || !Array.isArray(data.models)) throw new Error("账号模型接口返回格式无效，请检查插件版本。");
    const previous = state.accountModelCache.get(data.account_id);
    if (previous?.discovery?.snapshot_id !== data.discovery?.snapshot_id) state.candidateRoutes.clear();
    state.accountModelCache.set(data.account_id, data);
  }
  async function ensureAccountModels(force = false) {
    if (!$("detail-dialog").open || state.detailTab !== "models") return;
    if (!state.connected || !state.modelAccount) { renderAccountModels(); return; }
    if (!force && (state.accessLoading || state.accountModelCache.has(state.modelAccount))) { renderAccountModels(); return; }
    const accountId = state.modelAccount;
    const epoch = state.epoch;
    const sequence = ++state.accessLoadSequence;
    state.accessLoading = true;
    state.accessError = null;
    renderAccountModels();
    try {
      const data = await api("/accounts/models?account_id=" + encodeURIComponent(accountId));
      if (epoch !== state.epoch || sequence !== state.accessLoadSequence || accountId !== state.modelAccount) return;
      adoptAccountModels(data);
      state.accessError = null;
    } catch (error) {
      if (epoch !== state.epoch || sequence !== state.accessLoadSequence) return;
      state.accessError = error;
    } finally {
      if (epoch === state.epoch && sequence === state.accessLoadSequence) {
        state.accessLoading = false;
        renderAccountModels();
        renderAccounts();
      }
    }
  }
  $("model-account").onchange = (event) => {
    if (state.busy.has("access")) return;
    state.modelAccount = event.target.value;
    state.accessError = null;
    state.accessLoading = false;
    state.accessLoadSequence++;
    state.candidateRoutes.clear();
    state.candidatePage = 1;
    renderAccountModels();
    if ($("detail-dialog").open && state.detailTab === "models") {
      state.detailAccount = selectedModelAccount() || state.detailAccount;
      renderAccountDetail();
    }
    ensureAccountModels(true);
  };
  $("reload-account-models").onclick = () => ensureAccountModels(true);
  function renderDiscovery(data) {
    const account = selectedModelAccount();
    const discovery = data?.discovery || account?.discovery;
    const labels = { not_configured: ["CLI 未配置", "neutral"], not_fetched: ["尚未读取", "neutral"], loading: ["正在读取候选", "warning"], candidate_catalog: ["候选目录", "brand"], error: ["读取失败", "danger"], auth_failed: ["账号认证失败", "danger"], auth_unknown: ["账号认证状态未知", "warning"] };
    const label = labels[discovery?.status] || [account ? "目录状态未知" : "待选择账号", "neutral"];
    $("discovery-status").className = "badge badge-" + label[1];
    $("discovery-status").textContent = label[0] + (discovery?.stale ? " · stale / 已过期" : "");
    const metadata = [
      ["目录来源", discovery?.source === "official_droid_cli" ? "官方 Droid CLI（仅候选）" : asText(discovery?.source, "未知")],
      ["上次成功读取", date(discovery?.fetched_at)], ["候选有效期至", date(discovery?.fresh_until)],
      ["CLI 版本", asText(discovery?.cli_version, "未知")], ["目录协议版本", asText(discovery?.protocol_version, "未知")],
      ["账号权限证明", "候选目录不提供调用权限证明"]
    ];
    $("discovery-metadata").replaceChildren(...metadata.map(([labelText, value]) => el("div", { class: "discovery-meta" }, el("span", {}, labelText), el("strong", {}, value))));
    const message = discovery ? asText(discovery.message, "候选目录不能替代基础调用验证。") + (discovery.stale ? " 当前保留的候选已标为过期，不能据此发起候选验证。" : "") : "选择账号后读取服务端目录状态；不会自动运行 CLI 或发送验证请求。";
    $("discovery-message").replaceChildren(el("p", {}, message));
    $("discovery-message").hidden = false;
    $("cli-config-help").hidden = !(discovery?.status === "not_configured" || state.status?.capabilities?.cli_discovery_configured === false);
    renderCandidates(data);
  }
  function renderCandidates(data) {
    const discovery = data?.discovery;
    const candidates = Array.isArray(discovery?.candidates) ? discovery.candidates : [];
    $("candidate-count").textContent = candidates.length;
    const query = state.candidateSearch.toLocaleLowerCase();
    const filtered = candidates.filter((candidate) => [candidate.id, candidate.display_name].some((value) => String(value || "").toLocaleLowerCase().includes(query)));
    const pages = Math.max(1, Math.ceil(filtered.length / CANDIDATE_PAGE_SIZE));
    state.candidatePage = Math.min(pages, Math.max(1, state.candidatePage));
    const visible = filtered.slice((state.candidatePage - 1) * CANDIDATE_PAGE_SIZE, state.candidatePage * CANDIDATE_PAGE_SIZE);
    $("candidate-filter-summary").textContent = "候选 " + candidates.length + " 项 · 匹配 " + filtered.length + " 项 · 不是已允许模型数";
    $("candidate-page").textContent = state.candidatePage + " / " + pages;
    $("candidate-prev").disabled = state.candidatePage <= 1;
    $("candidate-next").disabled = state.candidatePage >= pages;
    const rows = visible.map((candidate) => {
      const badges = [];
      if (candidate.disabled === true) badges.push(el("span", { class: "badge badge-danger" }, "disabled / 禁用"));
      if (candidate.deprecated === true) badges.push(el("span", { class: "badge badge-warning" }, "deprecated / 弃用"));
      if (candidate.is_custom === true) badges.push(el("span", { class: "badge badge-warning" }, "custom / 自定义"));
      if (candidate.kind) badges.push(el("span", { class: "badge badge-neutral" }, "kind: " + String(candidate.kind)));
      if (!badges.length) badges.push(el("span", { class: "badge badge-neutral" }, "候选（未验证权限）"));
      const problem = candidateProblem(candidate, data);
      const routes = Array.isArray(candidate.route_models) ? candidate.route_models.map((id) => data.models.find((model) => model.model_id === id)).filter(Boolean) : [];
      const selector = el("select", { "aria-label": "选择候选 " + candidate.id + " 的明确映射", disabled: state.busy.has("access") || state.accessLoading || !!problem || !routes.length }, el("option", { value: "" }, routes.length ? "选择已配置的原生路由" : "无明确映射（仅候选）"));
      for (const model of routes) selector.append(el("option", { value: model.model_id, disabled: model.global_enabled === false || !protocolMeta(model.protocol) }, model.model_id + (protocolMeta(model.protocol) ? " · " + protocolMeta(model.protocol).label : " · 协议未知") + (model.global_enabled === false ? "（定义停用）" : "")));
      selector.value = state.candidateRoutes.get(candidate.id) || "";
      const hint = el("p", { class: "candidate-route-hint" });
      const verify = el("button", { type: "button", class: "button button-primary" }, "验证并允许");
      const update = () => {
        const model = routes.find((route) => route.model_id === selector.value);
        const modelProblem = model ? verificationProblem(model, data) : "请先选择已有的明确原生映射。";
        verify.disabled = state.busy.has("access") || !!problem || !!modelProblem;
        verify.title = problem || modelProblem || "发送一次固定短请求，最多 128 输出 Token，会消耗模型额度";
        hint.textContent = model ? "唯一端点：POST " + (protocolMeta(model.protocol)?.endpoint || "未知") : "不会根据候选名称或提供方猜测协议，也不会自动创建映射。";
      };
      selector.onchange = () => { state.candidateRoutes.set(candidate.id, selector.value); update(); };
      verify.onclick = () => {
        const model = routes.find((route) => route.model_id === selector.value);
        if (model) openModelVerification(model, candidate);
      };
      update();
      return el("tr", {},
        el("td", {}, el("code", { class: "candidate-id" }, asText(candidate.id)), el("div", { class: "candidate-name" }, asText(candidate.display_name)), el("div", { class: "candidate-name" }, "model_provider: " + asText(candidate.model_provider, "未知") + " · 图像：" + (candidate.supports_images === true ? "目录标注支持" : candidate.supports_images === false ? "目录标注不支持" : "未知"))),
        el("td", {}, el("div", { class: "candidate-badges" }, badges), el("p", { class: "candidate-reasons" }, reasonList(candidate.blocked_reasons) || "无明确禁用标记仍不代表账号有调用权限。")),
        el("td", {}, selector, hint, !routes.length ? el("button", { type: "button", class: "button button-quiet button-call", onclick: () => $("add-model").click() }, "添加明确路由") : null),
        el("td", {}, verify, el("small", { class: "probe-cost" }, "一次固定短请求 · 最多 128 输出 Token · 会消耗额度"), problem ? el("p", { class: "candidate-route-hint" }, problem) : null));
    });
    $("candidate-rows").replaceChildren(...(rows.length ? rows : [el("tr", {}, el("td", { colspan: 4, class: "table-empty" }, state.accessLoading ? "正在读取候选状态…" : candidates.length ? "没有匹配的候选" : "没有实际读取到的候选目录；手工路由不冒充发现结果"))]));
  }
  $("candidate-search").oninput = (event) => { state.candidateSearch = event.target.value.trim(); state.candidatePage = 1; renderCandidates(selectedAccountModels()); };
  $("candidate-prev").onclick = () => { state.candidatePage--; renderCandidates(selectedAccountModels()); };
  $("candidate-next").onclick = () => { state.candidatePage++; renderCandidates(selectedAccountModels()); };
  function accountScopedModelInfo(model) {
    if (typeof model.pinned_id !== "string" || !model.pinned_id) return null;
    const copy = el("button", { type: "button", class: "button button-soft icon-button", "aria-label": "复制此账号实际模型 ID：" + model.pinned_id }, icon("copy"));
    copy.onclick = () => copyModel(model.pinned_id, copy);
    return el("div", { class: "access-pinned-route" },
      el("span", {}, "固定此账号的实际调用 ID" + (model.published === true ? "" : " · 尚未发布")),
      el("div", {}, el("code", {}, model.pinned_id), copy),
      el("button", { type: "button", class: "button button-quiet button-call", onclick: () => openCallExample({ ...model, id: model.pinned_id }) }, icon("terminal"), "固定账号调用示例"));
  }
  function renderAccessRoute(model, data) {
    const published = publicationLabel(model);
    const problem = verificationProblem(model, data);
    const busy = state.busy.has("access");
    const verify = el("button", { type: "button", class: "button button-primary", disabled: busy || !!problem, title: problem || "发送一次固定短请求，最多 128 输出 Token，会消耗模型额度", onclick: () => openModelVerification(model) }, model.verification_status === "basic_call_verified" ? "重新验证并允许" : "手工验证并允许");
    const actions = el("div", { class: "access-route-actions" },
      el("div", {}, verify, el("small", { class: "probe-cost" }, "手工基础验证 · 最多 128 输出 Token · 会消耗额度")));
    if (model.allowed === true) actions.append(el("button", { type: "button", class: "button button-danger-quiet", disabled: busy, onclick: () => revokeModelAllowance(model) }, "撤销允许"));
    if (publicationPending(model)) actions.append(el("button", { type: "button", class: "button button-secondary", disabled: busy, onclick: () => reconcileModels() }, "重试同步（不验证）"));
    const row = el("article", { class: "access-route-card", "aria-label": model.display_name || model.model_id },
      el("div", { class: "access-route-head" }, el("div", {}, el("h4", {}, asText(model.display_name, model.model_id)), el("code", { class: "access-route-id" }, model.model_id)), el("span", { class: "badge badge-" + published[1] }, model.selected === true && model.verification_status !== "basic_call_verified" ? "已选 · 待验证" : published[0])),
      modelProtocolInfo({ ...model, id: model.model_id }),
      accountScopedModelInfo(model),
      el("div", { class: "access-route-states" },
        el("div", { class: "access-route-state" }, el("span", {}, "基础调用验证"), el("strong", {}, verificationLabel(model.verification_status))),
        el("div", { class: "access-route-state" }, el("span", {}, "允许意图"), el("strong", {}, model.allowed === true ? "已允许" : model.allowed === false ? "未允许" : "未知")),
        el("div", { class: "access-route-state" }, el("span", {}, "CPA 实际发布"), el("strong", { class: publicationPending(model) ? "status-pending" : "" }, published[0]))),
      el("p", { class: "access-route-time" }, "验证时间：" + date(model.verified_at)),
      el("p", { class: "access-route-source" }, "选择方式：" + selectionLabel(model.selection_source) + "。基础验证依据为固定原生调用，CLI 目录不是权限证明。"),
      el("p", { class: "access-route-reasons" }, reasonList(model.blocked_reasons) || "当前未返回阻断原因；仍以实际发布字段为准。"),
      model.last_error ? el("p", { class: "account-error-message" }, sanitize(model.last_error)) : null,
      problem ? el("p", { class: "access-route-reasons" }, problem) : null,
      model.verification_status === "basic_call_verified" ? el("p", { class: "form-hint" }, "基础调用已验证不代表工具、图像等全部能力已验证。") : null,
      actions);
    return row;
  }
  function renderAccountModels() {
    const data = selectedAccountModels();
    const account = selectedModelAccount();
    updateAccessControls();
    $("model-access-title").textContent = account ? account.label + " · 模型状态" : "当前账号的模型状态";
    const rows = Array.isArray(data?.models) ? data.models : [];
    const summary = [
      ["已配置路由", data ? rows.length : null],
      ["基础调用已验证", data ? rows.filter((model) => model.verification_status === "basic_call_verified").length : null],
      ["已允许", data ? rows.filter((model) => model.allowed === true).length : null],
      ["CPA 实际已发布", data ? rows.filter((model) => model.published === true).length : null],
      ["验证请求记录", data?.validation_usage?.validation_requests ?? data?.validation_usage?.total_requests]
    ];
    $("access-summary").replaceChildren(...summary.map(([label, value]) => el("div", { class: "access-summary-item" }, el("span", {}, label), el("strong", {}, count(value)))));
    let error = state.accessError ? accessErrorMessage(state.accessError) : "";
    if (state.accessError && data) error += " 当前保留上次读取的数据，请先刷新状态；不能据此重复验证。";
    showError("access-error", error);
    renderDiscovery(data);
    const query = state.accessSearch.toLocaleLowerCase();
    const filtered = rows.filter((model) => [model.model_id, model.display_name, model.upstream_id].some((value) => String(value || "").toLocaleLowerCase().includes(query)));
    if (!state.connected) $("access-route-grid").replaceChildren(emptyState("先连接管理端", "管理授权后，选择账号查看候选目录、基础验证、允许与发布状态。", openAuth, "管理授权"));
    else if (!account) $("access-route-grid").replaceChildren(emptyState("先添加 Factory 账号", "新账号默认不会发布未验证的模型。添加后再逐个模型验证并允许。", () => { setView("accounts"); openAccount(); }, "添加账号"));
    else if (state.accessLoading && !data) $("access-route-grid").replaceChildren(el("div", { class: "empty-state", "aria-busy": "true" }, el("h3", {}, "正在读取账号模型状态…"), el("p", {}, "仅读取状态，不会执行目录发现或基础验证。")));
    else if (!data) $("access-route-grid").replaceChildren(emptyState("尚无账号模型状态", state.accessError ? "请查看上方接口错误并刷新状态。模型定义不会代替账号权限数据。" : "点击刷新状态读取实际接口结果。", () => ensureAccountModels(true), "刷新状态", "refresh"));
    else if (!filtered.length) $("access-route-grid").replaceChildren(emptyState("没有匹配的已配置路由", rows.length ? "调整搜索条件查看其他路由。" : "先在“模型定义与估算”中配置明确的原生路由，再返回手工基础验证。", null, ""));
    else $("access-route-grid").replaceChildren(...filtered.map((model) => renderAccessRoute(model, data)));
  }
  $("access-model-search").oninput = (event) => { state.accessSearch = event.target.value.trim(); renderAccountModels(); };
  async function refreshGlobalAccessState() {
    try { await loadAll(); }
    catch (error) { showError("global-error", error.message); }
  }
  async function runAccessAction(operation, accountId, successMessage) {
    if (!state.connected || state.busy.has("access")) return null;
    const epoch = state.epoch;
    state.busy.add("access");
    if (accountId) state.busy.add(accountId);
    state.accessError = null;
    renderAccountModels();
    renderAccounts();
    try {
      const result = await operation();
      if (epoch !== state.epoch) return null;
      if (result?.account_id) adoptAccountModels(result);
      await refreshGlobalAccessState();
      if (epoch !== state.epoch) return null;
      if (successMessage) notify(successMessage);
      return result;
    } catch (error) {
      if (epoch !== state.epoch) return null;
      await refreshGlobalAccessState();
      if (epoch !== state.epoch) return null;
      state.accessError = error;
      notify(accessErrorMessage(error));
      throw error;
    } finally {
      state.busy.delete("access");
      if (accountId) state.busy.delete(accountId);
      if (epoch === state.epoch) { renderAccountModels(); renderAccounts(); }
    }
  }
  $("refresh-discovery").onclick = async () => {
    const accountId = state.modelAccount;
    if (!accountId) return;
    $("candidate-details").open = true;
    try { await runAccessAction(() => api("/accounts/discovery", "POST", { id: accountId }), accountId, "候选目录已更新；仍需逐个模型基础验证与允许"); }
    catch { /* 错误与旧候选 stale 状态均由页面展示。 */ }
  };
  async function revokeModelAllowance(model) {
    const accountId = state.modelAccount;
    if (!accountId || model.allowed !== true) return;
    try { await runAccessAction(() => api("/accounts/models/allow", "DELETE", { id: accountId, model_id: model.model_id }), accountId, "允许意图已撤销，请查看 CPA 实际注销状态"); }
    catch { /* 待同步时只提供发布重试，不重复验证。 */ }
  }
  async function reconcileModels() {
    try {
      const result = await runAccessAction(() => api("/models/reconcile", "POST", {}), state.modelAccount, "已重试 CPA 发布同步，未发送基础验证请求");
      if (result?.synchronized === true && state.verifyTarget) {
        state.verifyPublicationOnly = true;
        $("verify-publication-note").textContent = "已重试发布同步；实际发布状态请以下方账号模型记录为准。没有重复发送基础验证请求。";
        $("verify-publication-note").hidden = false;
      }
      return result;
    } catch (error) {
      if ($("verify-model-dialog").open) showError("verify-model-error", accessErrorMessage(error));
      return null;
    } finally { updateVerifyControls(); }
  }
  $("reconcile-models").onclick = () => reconcileModels();
  $("verify-sync-only").onclick = () => reconcileModels();
  function openModelVerification(model, candidate = null) {
    const data = selectedAccountModels();
    const problem = verificationProblem(model, data) || (candidate ? candidateProblem(candidate, data) : "");
    if (problem) return notify(problem);
    if (candidate && (!Array.isArray(candidate.route_models) || !candidate.route_models.includes(model.model_id))) return notify("此候选没有所选的明确映射，请重新读取目录。");
    state.verifyTarget = {
      accountId: state.modelAccount, modelId: model.model_id, model,
      credentialGeneration: data.credential_generation,
      snapshotId: candidate ? data.discovery.snapshot_id : null,
      candidateId: candidate?.id || null, retryUntil: 0
    };
    state.verifyPublicationOnly = false;
    $("verify-model-confirmation").checked = false;
    showError("verify-model-error", "");
    $("verify-publication-note").hidden = true;
    $("verify-publication-note").textContent = "基础验证已有服务端记录，请使用发布同步重试，不要重复发送验证请求。";
    const protocol = protocolMeta(model.protocol);
    $("verify-model-details").replaceChildren(detailRows([
      ["使用账号", asText(selectedModelAccount()?.label)],
      ["明确路由 ID", model.model_id],
      ["上游模型 ID", asText(model.upstream_id)],
      ["原生协议", protocol?.label || "未知"],
      ["唯一端点", "POST " + (protocol?.endpoint || "未知")],
      ["选择方式", candidate ? "CLI 候选选择（不是权限证明）" : "手工配置路由（非 CLI 权限证明）"]
    ]));
    updateVerifyControls();
    openDialog("verify-model-dialog");
  }
  function updateVerifyControls() {
    const target = state.verifyTarget;
    const busy = state.busy.has("access");
    const current = selectedAccountModels()?.models?.find((model) => model.model_id === target?.modelId);
    const stillValid = target && target.accountId === state.modelAccount && !verificationProblem(current);
    $("confirm-model-verify").hidden = state.verifyPublicationOnly;
    $("confirm-model-verify").disabled = !target || busy || !stillValid || !$("verify-model-confirmation").checked || (target.retryUntil || 0) > Date.now();
    $("verify-model-confirmation").disabled = busy || state.verifyPublicationOnly;
    $("verify-sync-only").hidden = !state.verifyPublicationOnly;
    $("verify-sync-only").disabled = busy || !state.connected;
  }
  $("verify-model-confirmation").onchange = updateVerifyControls;
  $("verify-model-form").onsubmit = async (event) => {
    event.preventDefault();
    const target = state.verifyTarget;
    if (!target || state.busy.has("access") || state.verifyPublicationOnly || !$("verify-model-confirmation").checked || (target.retryUntil || 0) > Date.now()) return;
    const data = selectedAccountModels();
    const model = data?.models?.find((item) => item.model_id === target.modelId);
    const candidate = target.candidateId ? data?.discovery?.candidates?.find((item) => item.id === target.candidateId) : null;
    const changed = target.accountId !== state.modelAccount || target.credentialGeneration !== data?.credential_generation;
    const snapshotChanged = target.snapshotId && (!candidate || !discoveryFresh(data?.discovery) || data.discovery.snapshot_id !== target.snapshotId);
    const problem = changed ? "账号或凭据已变化，请关闭窗口并刷新状态后重新确认。" : snapshotChanged ? "候选快照已变化或过期，请刷新目录后重新选择。" : verificationProblem(model, data) || (candidate ? candidateProblem(candidate, data) : "");
    if (problem) { $("verify-model-confirmation").checked = false; showError("verify-model-error", problem); updateVerifyControls(); return; }
    const body = { id: target.accountId, model_id: target.modelId, confirm: true };
    if (target.snapshotId) body.discovery_snapshot_id = target.snapshotId;
    showError("verify-model-error", "");
    const restore = pending($("confirm-model-verify"), "正在基础验证…");
    lockDialog($("verify-model-dialog"), true);
    try {
      await runAccessAction(() => api("/accounts/models/verify", "POST", body), target.accountId, "");
      const current = selectedAccountModels()?.models?.find((item) => item.model_id === target.modelId);
      if (current?.verification_status === "basic_call_verified" && current.allowed === true) {
        if (current.published === true) {
          $("verify-model-dialog").close();
          notify("基础调用已验证、允许，且已确认 CPA 发布");
        } else {
          state.verifyPublicationOnly = true;
          $("verify-publication-note").hidden = false;
          showError("verify-model-error", "基础调用验证已有记录，但 CPA 尚未发布。请只重试发布同步，不要重复发送验证请求。");
        }
      } else {
        $("verify-model-confirmation").checked = false;
        showError("verify-model-error", "操作已结束，请查看服务端基础验证、允许及实际发布状态；不要将目录读取视为验证成功。");
      }
    } catch (error) {
      $("verify-model-confirmation").checked = false;
      showError("verify-model-error", accessErrorMessage(error));
      const current = selectedAccountModels()?.models?.find((item) => item.model_id === target.modelId);
      if (error.code === "catalog_refresh_incomplete" || current?.verification_status === "basic_call_verified" && current.allowed === true) {
        state.verifyPublicationOnly = true;
        $("verify-publication-note").hidden = false;
      }
      if (error.status === 429 && error.retryAfter) {
        const seconds = Number(error.retryAfter);
        const until = Number.isFinite(seconds) ? Date.now() + Math.max(0, seconds) * 1000 : Date.parse(error.retryAfter);
        if (Number.isFinite(until) && until > Date.now()) {
          target.retryUntil = until;
          setTimeout(updateVerifyControls, Math.min(2147483647, until - Date.now() + 50));
        }
      }
    } finally {
      restore();
      lockDialog($("verify-model-dialog"), false);
      updateVerifyControls();
    }
  };
  const numericModelFields = ["input_multiplier", "output_multiplier", "cache_multiplier", "cache_creation_multiplier", "input_price_usd_per_million", "output_price_usd_per_million", "cache_read_price_usd_per_million", "cache_write_price_usd_per_million"];
  const MODEL_PRICE_FIELDS = [
    ["input_price_usd_per_million", "input", "price-input", "输入价"],
    ["output_price_usd_per_million", "output", "price-output", "输出价"],
    ["cache_read_price_usd_per_million", "cache_read", "price-cache-read", "缓存读取价"],
    ["cache_write_price_usd_per_million", "cache_write", "price-cache-write", "缓存写入价"]
  ];
  function priceText(value) {
    return finite(Number(value)) ? String(Number(value)) : "—";
  }
  function priceRuleLabel(rule) {
    const threshold = rule.input_tokens_above;
    const context = threshold === null || threshold === undefined ? "" : ">" + (threshold % 1000 === 0 ? threshold / 1000 + "K" : count(threshold));
    const tier = rule.service_tier === "priority" ? "fast/priority" : rule.service_tier || "";
    return [context, tier].filter(Boolean).join(" · ");
  }
  function readPriceNumber(input, label, required = false) {
    const text = input.value.trim();
    if (!text) {
      if (required) throw new Error("请填写" + label + "。");
      return null;
    }
    const value = Number(text);
    if (!Number.isFinite(value) || value < 0 || value > 1000000) throw new Error(label + "须为 0 到 1000000 的有限数。");
    return value;
  }
  function priceRuleBadges(rules) {
    if (!rules.length) return null;
    return el("div", { class: "price-rules" }, rules.map((rule) => el("span", { class: "price-rule", title: "输入 $" + priceText(rule.input) + " · 输出 $" + priceText(rule.output) + " · 缓存读取 $" + priceText(rule.cache_read ?? rule.input) + " · 缓存写入 $" + priceText(rule.cache_write ?? rule.input) }, el("strong", {}, priceRuleLabel(rule)), el("span", {}, "$" + priceText(rule.input) + " / $" + priceText(rule.output)))));
  }
  function addPriceRule(value = {}, open = true) {
    if (state.priceRules.length >= 64) return;
    const threshold = el("input", { type: "number", min: "0", step: "1", value: value.input_tokens_above ?? "", placeholder: "不限制", "aria-label": "规则输入 Token 大于" });
    const tier = el("select", { "aria-label": "规则服务等级" }, ...[["", "标准服务"], ["priority", "fast / priority"], ["flex", "flex"], ["batch", "batch"], ["ultrafast", "ultrafast"]].map(([id, label]) => el("option", { value: id }, label)));
    tier.value = value.service_tier || "";
    const fields = {};
    const grid = el("div", { class: "price-rule-fields" });
    for (const [, key, , label] of MODEL_PRICE_FIELDS) {
      const required = key === "input" || key === "output";
      const input = el("input", { type: "number", min: "0", max: "1000000", step: "0.000001", inputmode: "decimal", value: value[key] ?? "", placeholder: required ? "必填" : "沿用输入价", "aria-label": "规则" + label });
      fields[key] = input;
      grid.append(el("label", {}, label, input));
    }
    const card = el("details", { class: "price-rule-card", open });
    const title = el("strong"), rates = el("span", { class: "price-rule-rates" });
    const entry = { card, read: () => {
      const inputTokensAbove = threshold.value.trim() ? Number(threshold.value) : null;
      if (inputTokensAbove !== null && (!Number.isSafeInteger(inputTokensAbove) || inputTokensAbove <= 0)) throw new Error("输入 Token 门槛须为大于 0 的整数。");
      if (inputTokensAbove === null && !tier.value) throw new Error("计费规则需设置 Token 门槛或服务等级。");
      return { input_tokens_above: inputTokensAbove, service_tier: tier.value || null, ...Object.fromEntries(Object.entries(fields).map(([key, input]) => [key, readPriceNumber(input, "规则" + key, key === "input" || key === "output")])) };
    } };
    const summarize = () => {
      title.textContent = priceRuleLabel({ input_tokens_above: threshold.value ? Number(threshold.value) : null, service_tier: tier.value || null }) || "新计费规则";
      rates.textContent = "输入 " + (fields.input.value ? "$" + fields.input.value : "待填写") + " · 输出 " + (fields.output.value ? "$" + fields.output.value : "待填写");
    };
    card.addEventListener("input", summarize);
    tier.onchange = summarize;
    const remove = el("button", { type: "button", class: "button button-quiet icon-button", "aria-label": "删除计费规则", onclick: () => {
      state.priceRules.splice(state.priceRules.indexOf(entry), 1);
      card.remove();
      $("add-price-rule").disabled = false;
    } }, icon("trash"));
    card.append(el("summary", { class: "price-rule-summary" }, title, rates), el("div", { class: "price-rule-body" }, el("div", { class: "price-rule-conditions" }, el("label", {}, "输入 Token 大于", threshold), el("label", {}, "服务等级", tier), remove), grid));
    state.priceRules.push(entry);
    $("price-rule-editor").append(card);
    $("add-price-rule").disabled = state.priceRules.length >= 64;
    summarize();
  }
  function openPriceEditor(model) {
    const effective = { ...model, ...(state.modelDraft.get(model.id) || {}) };
    state.priceTarget = model;
    state.priceRules = [];
    $("price-model-name").textContent = model.display_name + " · " + model.id;
    for (const [field, , id] of MODEL_PRICE_FIELDS) $(id).value = effective[field] ?? "";
    $("price-rule-editor").replaceChildren();
    for (const rule of effective.price_rules || []) addPriceRule(rule, false);
    $("add-price-rule").disabled = state.priceRules.length >= 64;
    showError("price-error", "");
    openDialog("price-dialog");
  }
  $("add-price-rule").onclick = () => addPriceRule();
  $("price-form").addEventListener("input", () => showError("price-error", ""));
  $("price-form").onsubmit = (event) => {
    event.preventDefault();
    if (!state.priceTarget) return;
    try {
      const base = Object.fromEntries(MODEL_PRICE_FIELDS.map(([field, , id, label]) => [field, readPriceNumber($(id), label)]));
      if ((base.input_price_usd_per_million === null) !== (base.output_price_usd_per_million === null)) throw new Error("输入价和输出价需同时设置，或同时留空。");
      const rules = state.priceRules.map((entry) => entry.read());
      const conditions = new Set();
      for (const rule of rules) {
        const key = String(rule.input_tokens_above) + ":" + String(rule.service_tier);
        if (conditions.has(key)) throw new Error("相同 Token 门槛和服务等级的规则不能重复。");
        conditions.add(key);
      }
      for (const [field, value] of Object.entries(base)) draftModel(state.priceTarget, field, value);
      draftModel(state.priceTarget, "price_rules", rules);
      $("price-dialog").close();
      renderModels();
    } catch (error) { showError("price-error", error.message); }
  };
  function modelValue(model, field) {
    const draft = state.modelDraft.get(model.id);
    return draft && Object.hasOwn(draft, field) ? draft[field] : model[field];
  }
  function draftModel(model, field, value) {
    const draft = { ...(state.modelDraft.get(model.id) || {}), [field]: value };
    state.modelDraft.set(model.id, draft);
    $("models-unsaved").hidden = false;
    $("save-models").disabled = !state.connected || state.busy.has("settings");
  }
  function routeStatus(model) {
    if (model.enabled === false) return ["已停用", "neutral"];
    if (!state.accounts.some((account) => account.enabled && account.status === "ready")) return ["无可用账号", "neutral"];
    if (model.registered === true || model.available === true) return ["已注册", "success"];
    if (model.registered === false || model.available === false) return ["未注册", "warning"];
    return ["目录已启用", "brand"];
  }
  function renderModels() {
    renderPinnedModels();
    const models = Array.isArray(state.settings?.models) ? state.settings.models : state.models;
    const query = state.modelSearch.toLocaleLowerCase();
    const filtered = models.filter((model) => [model.id, model.display_name, model.upstream_id].some((value) => String(value || "").toLocaleLowerCase().includes(query)));
    $("model-count").textContent = state.connected ? "共 " + models.length + " 个模型 · " + models.filter((model) => model.enabled).length + " 个已启用" : "连接后显示模型目录";
    $("models-unsaved").hidden = state.modelDraft.size === 0;
    $("save-models").disabled = !state.connected || state.modelDraft.size === 0 || state.busy.has("settings");
    const rows = filtered.map((model) => {
      const copy = el("button", { type: "button", class: "button button-quiet copy-button", title: "复制模型 ID", "aria-label": "复制模型 ID：" + model.id }, icon("copy"));
      copy.onclick = () => copyModel(model.id, copy);
      const actual = state.models.find((item) => item.id === model.id) || model;
      const status = routeStatus(actual);
      const row = el("tr", {},
        el("td", {}, el("div", { class: "model-name" }, asText(model.display_name, model.id)), el("div", { class: "model-id" }, el("code", {}, model.id), copy), modelProtocolInfo(model), el("div", { class: "model-support" }, (model.supports_images ? "图文输入" : "文本输入") + " · 上游 " + asText(model.upstream_id))),
        el("td", {}, el("span", { class: "badge badge-" + status[1] }, status[0])));
      const labels = ["输入倍率", "输出倍率", "缓存读取倍率", "缓存写入倍率"];
      numericModelFields.slice(0, 4).forEach((field, index) => {
        const value = modelValue(model, field);
        const input = el("input", { type: "number", min: 0, step: "any", value: value ?? "", placeholder: index > 3 ? "未设置" : "1", disabled: !state.connected, "aria-label": model.display_name + " " + labels[index] });
        input.oninput = () => draftModel(model, field, input.value);
        row.append(el("td", {}, input));
      });
      const inputPrice = modelValue(model, "input_price_usd_per_million");
      const outputPrice = modelValue(model, "output_price_usd_per_million");
      const rules = modelValue(model, "price_rules") || [];
      const priceButton = el("button", { type: "button", class: "button button-secondary", disabled: !state.connected, onclick: () => openPriceEditor(model) }, icon("settings"), finite(inputPrice) && finite(outputPrice) ? "编辑价格" : "设置价格");
      row.append(el("td", { class: "model-price-cell" }, el("div", { class: "model-price-base" }, finite(inputPrice) && finite(outputPrice) ? "$" + priceText(inputPrice) + " / $" + priceText(outputPrice) : rules.length ? "按规则计价" : "未设置"), priceRuleBadges(rules), priceButton));
      const toggle = el("input", { type: "checkbox", role: "switch", checked: modelValue(model, "enabled") === true, disabled: !state.connected, "aria-label": "启用模型：" + model.id });
      toggle.onchange = () => draftModel(model, "enabled", toggle.checked);
      row.append(el("td", {}, el("label", { class: "switch" }, toggle)));
      return row;
    });
    $("model-rows").replaceChildren(...(rows.length ? rows : [el("tr", {}, el("td", { colspan: 8, class: "table-empty" }, state.connected ? "没有匹配的模型" : "管理授权后查看模型与估算配置"))]));
  }
  function renderPinnedModels() {
    $("pinned-model-panel").hidden = !state.connected || !state.accounts.length;
    if (!state.accounts.some((account) => account.id === state.pinnedAccount)) state.pinnedAccount = state.accounts.find((account) => account.enabled && account.status === "ready")?.id || state.accounts[0]?.id || "";
    $("pinned-account").replaceChildren(...state.accounts.map((account) => el("option", { value: account.id }, account.label + (account.enabled ? "" : "（已停用）"))));
    $("pinned-account").value = state.pinnedAccount;
    const account = state.accounts.find((item) => item.id === state.pinnedAccount);
    $("pinned-model-list").replaceChildren(...(account ? [...pinnedModelList(account).childNodes] : []));
  }
  $("pinned-account").onchange = (event) => { state.pinnedAccount = event.target.value; renderPinnedModels(); };
  $("model-search").oninput = (event) => { state.modelSearch = event.target.value.trim(); renderModels(); };
  $("save-models").onclick = async () => {
    if (state.busy.has("settings") || !state.modelDraft.size) return;
    showError("model-error", "");
    state.busy.add("settings");
    const submittedDraft = new Map([...state.modelDraft].map(([id, draft]) => [id, { ...draft }]));
    const epoch = state.epoch;
    const credential = state.token;
    setConnection(state.connected);
    const restore = pending($("save-models"), "正在保存…");
    try {
      const current = await api("/settings", "GET", undefined, credential);
      if (epoch !== state.epoch) return;
      const models = current.models.map((model) => {
        const draft = submittedDraft.get(model.id);
        if (!draft) return model;
        const updated = { ...model, ...draft };
        for (const field of numericModelFields) {
          const value = optionalNumber(updated[field] === null || updated[field] === undefined ? "" : String(updated[field]), field.includes("price") ? "美元单价" : "模型倍率");
          if (value === null && !field.includes("price")) throw new Error("模型倍率不能为空。");
          updated[field] = value;
        }
        return updated;
      });
      const saved = await api("/settings", "PUT", { ...current, models }, credential);
      if (epoch !== state.epoch) return;
      state.settings = saved;
      for (const [id, submitted] of submittedDraft) {
        if (JSON.stringify(state.modelDraft.get(id)) === JSON.stringify(submitted)) state.modelDraft.delete(id);
      }
      await loadAll();
      await loadModelMetadata(true);
      if ($("global-models-dialog").open) renderModels();
      notify("模型配置已保存");
    } catch (error) { showError("model-error", error.message); }
    finally { state.busy.delete("settings"); restore(); setConnection(state.connected); $("models-unsaved").hidden = state.modelDraft.size === 0; }
  };
  $("add-model").onclick = () => {
    $("model-form").reset();
    $("mapping-id").value = "factory/";
    updateMappingProtocolHint();
    showError("mapping-error", "");
    openDialog("model-dialog");
  };
  $("account-add-route").onclick = () => $("add-model").click();
  function updateMappingProtocolHint() {
    const protocol = protocolMeta($("mapping-protocol").value);
    $("mapping-protocol-hint").textContent = (protocol ? "唯一调用端点：POST " + protocol.endpoint + "。" : "原生协议配置未知。") + "请按上游模型实际协议选择，模型名称不会决定协议。";
  }
  $("mapping-protocol").onchange = updateMappingProtocolHint;
  $("model-form").onsubmit = async (event) => {
    event.preventDefault();
    if (state.busy.has("settings")) return;
    const id = $("mapping-id").value.trim();
    if (!id.startsWith("factory/") || id.length <= 8) return showError("mapping-error", "CPA 模型 ID 必须以 factory/ 开头，并填写模型名称。");
    const mapping = {
      id, upstream_id: $("mapping-upstream").value.trim(), display_name: $("mapping-name").value.trim(),
      protocol: $("mapping-protocol").value, upstream_provider: $("mapping-provider").value.trim(), enabled: true,
      context_length: 0, max_output_tokens: 8192, supports_images: false, input_multiplier: 1, output_multiplier: 1, cache_multiplier: 1, cache_creation_multiplier: 1,
      input_price_usd_per_million: null, output_price_usd_per_million: null, cache_read_price_usd_per_million: null, cache_write_price_usd_per_million: null,
      price_rules: []
    };
    state.busy.add("settings");
    const epoch = state.epoch;
    const credential = state.token;
    setConnection(state.connected);
    const restore = pending($("save-mapping"), "正在保存…");
    lockDialog($("model-dialog"), true);
    showError("mapping-error", "");
    try {
      const current = await api("/settings", "GET", undefined, credential);
      if (epoch !== state.epoch) return;
      if (current.models.some((model) => model.id === id)) throw new Error("该模型 ID 已存在，请使用其他名称。");
      const saved = await api("/settings", "PUT", { ...current, models: [...current.models, mapping] }, credential);
      if (epoch !== state.epoch) return;
      state.settings = saved;
      $("model-dialog").close();
      await loadAll();
      notify("模型映射已添加");
    } catch (error) { showError("mapping-error", error.message); if (!$("model-dialog").open) notify("映射已保存，但刷新失败；请刷新确认。"); }
    finally { state.busy.delete("settings"); restore(); lockDialog($("model-dialog"), false); setConnection(state.connected); }
  };
  function renderAll() {
    setConnection(state.connected);
    renderAccounts();
    setView("accounts");
    if ($("detail-dialog").open && state.detailTab === "models") { syncModelAccountScope(); renderAccountModels(); }
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
  initializeTheme();
  renderAll();
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
  async function restoreHostSession() {
    const saved = readSavedHostSession();
    if (!saved || state.connected || state.busy.has("auth") || state.loading) return;
    state.token = saved;
    state.credentialSource = "host";
    try { await loadAll(); }
    catch { state.token = ""; state.credentialSource = ""; }
  }
  restoreHostSession();
  addEventListener("storage", (event) => {
    if (event.key !== "cli-proxy-auth") return;
    if (state.credentialSource === "host") disconnect();
    if (!state.connected) restoreHostSession();
  });
  setInterval(() => document.querySelectorAll("[data-countdown]").forEach((node) => {
    node.textContent = node.dataset.countdownMode === "quota" ? quotaCountdown(node.dataset.countdown) : node.dataset.countdownMode === "reset" ? remainingTime(node.dataset.countdown).replace("结束", "重置") : remainingTime(node.dataset.countdown);
  }), 60000);
  addEventListener("pagehide", () => { disconnect(); $("management-key").value = ""; $("account-key").value = ""; });
  addEventListener("pageshow", (event) => { if (event.persisted) restoreHostSession(); });
})();
