"use strict";
// Scope is /management.html. No API interception, response cache or credentials.
const BOOTSTRAP = "/v0/resource/plugins/factory-droid/manager-bootstrap.js";
self.addEventListener("install", (event) => event.waitUntil(self.skipWaiting()));
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));
self.addEventListener("fetch", (event) => {
  const url = new URL(event.request.url);
  if (event.request.method !== "GET" || event.request.mode !== "navigate" ||
      url.origin !== self.location.origin || url.pathname !== "/management.html") return;
  event.respondWith((async () => {
    const original = await fetch(event.request);
    if (!original.ok || !original.headers.get("Content-Type")?.includes("text/html")) return original;
    try {
      const probe = await fetch(new URL(BOOTSTRAP, self.location.origin), {
        credentials: "omit", cache: "no-store", redirect: "error"
      });
      if (!probe.ok || !probe.headers.get("Content-Type")?.includes("javascript")) {
        if (probe.status === 404 || probe.status === 410) await self.registration.unregister();
        return original;
      }
      const html = await original.clone().text();
      if (!/<head(?:\s[^>]*)?>/i.test(html) || html.includes("data-factory-auto-enhancement")) return original;
      // Do not insert ahead of a document's meta CSP. HTTP CSP is preserved below.
      if ((html.match(/<meta\b[^>]*>/gi) || []).some((meta) => /http-equiv\s*=\s*["']?content-security-policy\b/i.test(meta))) return original;
      const script = `<script type="module" data-factory-auto-enhancement src="${BOOTSTRAP}"></script>`;
      const headers = new Headers(original.headers);
      for (const key of ["Content-Length", "Content-Encoding", "ETag", "Content-MD5", "Last-Modified"]) headers.delete(key);
      headers.set("Cache-Control", "no-store");
      return new Response(html.replace(/<head(?:\s[^>]*)?>/i, (head) => head + script), {
        status: original.status, statusText: original.statusText, headers
      });
    } catch {
      // A failed/disabled plugin must never prevent Manager from opening.
      return original;
    }
  })());
});
