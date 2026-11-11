// Manager combines runtime models with provider definitions to show exclusions.
// Keep routing credential-bound; only redirect the Factory metadata lookup.
export function factoryDefinitionsURL(value, base) {
  try {
    const url = new URL(value, base);
    const origin = new URL(base).origin;
    if (url.origin !== origin || url.pathname !== "/v0/management/model-definitions/factory-droid") return null;
    url.pathname = "/v0/management/plugins/factory-droid/model-definitions";
    return url.href;
  } catch { return null; }
}

export function installModelDefinitionsBridge(target) {
  const originalFetch = target.fetch;
  const proto = target.XMLHttpRequest.prototype;
  const originalOpen = proto.open;
  function wrappedFetch(input, init) {
    const method = String(init?.method ?? input?.method ?? "GET").toUpperCase();
    const value = typeof input === "string" || input instanceof URL ? input : input?.url;
    const url = method === "GET" ? factoryDefinitionsURL(value, target.location.href) : null;
    if (!url) return originalFetch.call(this, input, init);
    const redirected = typeof input === "string" || input instanceof URL ? url : new Request(url, input);
    return originalFetch.call(this, redirected, init);
  }
  function wrappedOpen(method, url, ...rest) {
    const redirected = String(method).toUpperCase() === "GET"
      ? factoryDefinitionsURL(url, target.location.href) : null;
    return originalOpen.call(this, method, redirected ?? url, ...rest);
  }
  target.fetch = wrappedFetch;
  proto.open = wrappedOpen;
  return Object.freeze({
    stop() {
      if (target.fetch === wrappedFetch) target.fetch = originalFetch;
      if (proto.open === wrappedOpen) proto.open = originalOpen;
    }
  });
}

if (typeof window !== "undefined" && window.parent === window && !window.__factoryDroidModelDefinitions) {
  window.__factoryDroidModelDefinitions = installModelDefinitionsBridge(window);
}
