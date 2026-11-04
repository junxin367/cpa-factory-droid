import "./manager-extension.js";

// Loaded before Manager's application script. Observe successful same-origin
// management authentication; never persist a key or modify a request.
if (window.parent === window && !window.__factoryDroidAutoBootstrap) {
  const originalFetch = window.fetch;
  const proto = XMLHttpRequest.prototype;
  const originalOpen = proto.open, originalHeader = proto.setRequestHeader, originalSend = proto.send;
  const xhrs = new WeakMap();
  let active = true, latestAuthorization = "", generation = 0;
  const timers = new Set();
  function managementURL(value) {
    try {
      const url = new URL(value, location.href);
      return url.origin === location.origin && url.pathname.startsWith("/v0/management/") &&
        !url.pathname.startsWith("/v0/management/plugins/factory-droid/");
    } catch { return false; }
  }
  function savedSession() {
    try {
      let text = localStorage.getItem("cli-proxy-auth") || "{}";
      const prefix = text.startsWith("enc::v2::") ? "enc::v2::" : text.startsWith("enc::v1::") ? "enc::v1::" : "";
      if (prefix) {
        const salt = "cli-proxy-api-webui::secure-storage";
        const key = new TextEncoder().encode(prefix === "enc::v2::" ? salt + "|v2|" + location.host : salt + "|" + location.host + "|" + navigator.userAgent);
        const bytes = Uint8Array.from(atob(text.slice(prefix.length)), (c) => c.charCodeAt(0));
        for (let i = 0; i < bytes.length; i++) bytes[i] ^= key[i % key.length];
        text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
      }
      const parsed = JSON.parse(text), saved = parsed.state || parsed;
      const base = new URL(saved.apiBase || "", location.origin);
      if (!saved.apiBase || base.origin !== location.origin || !["/", "/v0/management", "/v0/management/"].includes(base.pathname)) return null;
      return saved;
    } catch { return null; }
  }
  function observe(value) {
    if (!active || typeof value !== "string" || !/^Bearer\s+\S+$/i.test(value)) return "";
    latestAuthorization = value.replace(/^Bearer\s+/i, "");
    return latestAuthorization;
  }
  function accept(candidate, capturedGeneration, attempt = 0) {
    if (!active || !candidate || candidate !== latestAuthorization || capturedGeneration !== generation) return;
    const saved = savedSession();
    if (saved && !/^#\/login(?:[/?]|$)/.test(location.hash)) {
      if (saved.rememberPassword && saved.managementKey && saved.managementKey !== candidate) return;
      window.__factoryDroidQuotaEnhancement?.connect(candidate);
      return;
    }
    if (attempt >= 20) return;
    const timer = setTimeout(() => { timers.delete(timer); accept(candidate, capturedGeneration, attempt + 1); }, 100);
    timers.add(timer);
  }
  function wrappedFetch(input, init) {
    let candidate = "";
    try {
      if (managementURL(typeof input === "string" || input instanceof URL ? input : input.url))
        candidate = observe(new Headers(init?.headers ?? input?.headers).get("Authorization"));
    } catch { /* Do not alter Manager request handling. */ }
    const capturedGeneration = generation;
    const result = originalFetch.call(this, input, init);
    if (candidate) result.then((response) => { if (response.ok) accept(candidate, capturedGeneration); }, () => {});
    return result;
  }
  function wrappedOpen(method, url, ...rest) {
    xhrs.set(this, { management: managementURL(url), candidate: "" });
    return originalOpen.call(this, method, url, ...rest);
  }
  function wrappedHeader(name, value) {
    const state = xhrs.get(this);
    if (state?.management && String(name).toLowerCase() === "authorization") state.candidate = observe(value);
    return originalHeader.call(this, name, value);
  }
  function wrappedSend(...args) {
    const candidate = xhrs.get(this)?.candidate;
    const capturedGeneration = generation;
    if (candidate) this.addEventListener("load", () => {
      if (this.status >= 200 && this.status < 300) accept(candidate, capturedGeneration);
    }, { once: true });
    return originalSend.apply(this, args);
  }
  function clear() {
    generation++; latestAuthorization = "";
    for (const timer of timers) clearTimeout(timer);
    timers.clear();
  }
  function storage(event) { if (event.key === null || event.key === "cli-proxy-auth") clear(); }
  function route() { if (/^#\/login(?:[/?]|$)/.test(location.hash)) clear(); }
  window.fetch = wrappedFetch;
  proto.open = wrappedOpen; proto.setRequestHeader = wrappedHeader; proto.send = wrappedSend;
  addEventListener("unauthorized", clear);
  addEventListener("storage", storage);
  addEventListener("hashchange", route);
  window.__factoryDroidAutoBootstrap = Object.freeze({
    stop() {
      active = false; clear();
      if (window.fetch === wrappedFetch) window.fetch = originalFetch;
      if (proto.open === wrappedOpen) proto.open = originalOpen;
      if (proto.setRequestHeader === wrappedHeader) proto.setRequestHeader = originalHeader;
      if (proto.send === wrappedSend) proto.send = originalSend;
      removeEventListener("unauthorized", clear);
      removeEventListener("storage", storage);
      removeEventListener("hashchange", route);
      delete window.__factoryDroidAutoBootstrap;
    }
  });
  // Also support a Manager that restored authentication before module loading.
  const saved = savedSession();
  if (saved?.rememberPassword === true && typeof saved.managementKey === "string" && saved.managementKey) {
    const candidate = observe("Bearer " + saved.managementKey), capturedGeneration = generation;
    originalFetch("/v0/management/plugins/factory-droid/status", {
      credentials: "omit", cache: "no-store", redirect: "error",
      headers: { Authorization: "Bearer " + candidate }
    }).then((response) => { if (response.ok) accept(candidate, capturedGeneration); }, () => {});
  }
}
