// Cross-origin isolation where the server cannot send the headers (GitHub Pages).
//
// Loaded by a page as a classic script (`<script src="coi-worker.js"></script>`, before anything
// else), it registers itself as a service worker and reloads the page once, if the page is not yet
// cross-origin isolated. As the service worker it adds the two headers to every response of its
// scope, so the reloaded page may use shared memory and the threaded package. Where service
// workers are unavailable, or a reload did not help, the page stays as it is and the engine falls
// back to the single-threaded package (`crispmesh-dense.js` checks `crossOriginIsolated`).
//
// The same approach as coi-serviceworker (MIT), written here in a few lines. Every cross-origin
// resource the page loads must then allow embedding (CORS or Cross-Origin-Resource-Policy);
// `credentialless` is used where the browser supports it, which relaxes that for no-cors loads.

if (typeof window === "undefined") {
  // The service worker.
  self.addEventListener("install", () => self.skipWaiting());
  self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));
  self.addEventListener("fetch", (event) => {
    const request = event.request;
    // Only documents, scripts and data the page loads need the headers; uploads (POST with a
    // body stream) are left to the browser, which a re-issued fetch would break.
    if (request.method !== "GET") return;
    if (request.cache === "only-if-cached" && request.mode !== "same-origin") return;
    event.respondWith(
      fetch(request).then((response) => {
        if (response.status === 0) return response;
        const headers = new Headers(response.headers);
        headers.set("Cross-Origin-Opener-Policy", "same-origin");
        headers.set("Cross-Origin-Embedder-Policy", "credentialless" in Request.prototype ? "credentialless" : "require-corp");
        headers.set("Cross-Origin-Resource-Policy", "cross-origin");
        return new Response(response.body, { status: response.status, statusText: response.statusText, headers });
      }),
    );
  });
} else if (!window.crossOriginIsolated && window.isSecureContext && "serviceWorker" in navigator) {
  // The page: register, then reload once under the worker's control.
  const key = "crispmesh-coi-reloaded";
  let tried = false;
  try {
    tried = sessionStorage.getItem(key) === "1";
  } catch {
    // no session storage: a reload loop is still prevented by the controller check below
  }
  if (!tried) {
    window.crispmeshReloading = true;
    navigator.serviceWorker.register(document.currentScript.src).then((registration) => {
      const reload = () => {
        try {
          sessionStorage.setItem(key, "1");
        } catch {
          // ignore
        }
        location.reload();
      };
      if (navigator.serviceWorker.controller) reload();
      else if (registration.active) reload();
      else navigator.serviceWorker.addEventListener("controllerchange", reload, { once: true });
    }, () => {
      window.crispmeshReloading = false;
      window.dispatchEvent(new Event("crispmesh-coi-settled"));
    });
  }
}
