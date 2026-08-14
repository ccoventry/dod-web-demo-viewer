/*! coi-serviceworker v0.1.7 — Guido Zuidhof and contributors, licensed under MIT
 *
 *  ---------------------------------------------------------------------------
 *  WHY THIS FILE EXISTS
 *  ---------------------------------------------------------------------------
 *  SharedArrayBuffer — and therefore multi-threaded WebAssembly — is only
 *  exposed to documents that are "cross-origin isolated". A document is
 *  cross-origin isolated only when the server sends BOTH:
 *
 *      Cross-Origin-Opener-Policy:   same-origin
 *      Cross-Origin-Embedder-Policy: require-corp
 *
 *  GitHub Pages does not let you set response headers. This script installs a
 *  service worker that intercepts every same-origin fetch and re-emits the
 *  response with those headers attached. Because a service worker's response
 *  is what the document actually sees, the browser grants isolation.
 *
 *  ---------------------------------------------------------------------------
 *  HOW TO USE
 *  ---------------------------------------------------------------------------
 *  Load it as the FIRST script in <head>, before anything else:
 *
 *      <script src="./coi-serviceworker.js"></script>
 *
 *  This one file plays two roles. Loaded in the page it registers itself;
 *  loaded as a service worker it does the header rewriting. It detects which
 *  context it is in via `self.document`.
 *
 *  On the very first visit the worker is not yet controlling the page, so the
 *  script triggers ONE reload. That is expected, happens once per origin, and
 *  is guarded against reload loops.
 *
 *  ---------------------------------------------------------------------------
 *  CAVEAT — COEP AND CROSS-ORIGIN SUBRESOURCES
 *  ---------------------------------------------------------------------------
 *  Under `require-corp`, every cross-origin subresource must either be fetched
 *  with CORS or send `Cross-Origin-Resource-Policy: cross-origin`. jsDelivr
 *  sends `Access-Control-Allow-Origin: *` and ES module / WASM fetches use CORS
 *  mode, so the engine bundle loads fine.
 *
 *  If you add a CDN that does NOT send permissive CORS headers, its assets will
 *  be blocked. Set `coepCredentialless` (see options below) — Chromium's
 *  `credentialless` value keeps isolation while allowing no-CORS subresources
 *  to load without credentials.
 *
 *  ---------------------------------------------------------------------------
 *  CONFIGURATION — define `window.coi` BEFORE this script tag
 *  ---------------------------------------------------------------------------
 *      window.coi = {
 *        shouldRegister:     () => true,      // gate registration
 *        shouldDeregister:   () => false,     // uninstall the worker
 *        coepCredentialless: () => true,      // use COEP: credentialless
 *        coepDegrades:       true,            // retry as require-corp on failure
 *        doReload:           () => window.location.reload(),
 *        quiet:              false,           // silence console output
 *      };
 *
 *  GitHub Pages note: also commit an empty `.nojekyll` file at the repo root,
 *  otherwise Jekyll may interfere with asset paths.
 */

let coepCredentialless = false;

if (typeof window === 'undefined') {
    /* =======================================================================
     * SERVICE WORKER CONTEXT
     * =====================================================================*/

    self.addEventListener("install", () => self.skipWaiting());
    self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));

    self.addEventListener("message", (ev) => {
        if (!ev.data) return;

        if (ev.data.type === "deregister") {
            self.registration
                .unregister()
                .then(() => self.clients.matchAll())
                .then((clients) => {
                    clients.forEach((client) => client.navigate(client.url));
                });
        } else if (ev.data.type === "coepCredentialless") {
            coepCredentialless = ev.data.value;
        }
    });

    self.addEventListener("fetch", function (event) {
        const r = event.request;

        // Range requests (media seeking) must pass through untouched, otherwise
        // the 206 semantics break.
        if (r.cache === "only-if-cached" && r.mode !== "same-origin") return;

        const request = (coepCredentialless && r.mode === "no-cors")
            ? new Request(r, { credentials: "omit" })
            : r;

        event.respondWith(
            fetch(request)
                .then((response) => {
                    // Opaque responses carry no readable headers; leave them be.
                    if (response.status === 0) return response;

                    const newHeaders = new Headers(response.headers);

                    // The two headers that actually grant isolation.
                    newHeaders.set("Cross-Origin-Embedder-Policy",
                        coepCredentialless ? "credentialless" : "require-corp");

                    if (!coepCredentialless) {
                        newHeaders.set("Cross-Origin-Resource-Policy", "cross-origin");
                    }

                    newHeaders.set("Cross-Origin-Opener-Policy", "same-origin");

                    return new Response(response.body, {
                        status: response.status,
                        statusText: response.statusText,
                        headers: newHeaders,
                    });
                })
                .catch((e) => console.error("[coi-sw]", e))
        );
    });

} else {
    /* =======================================================================
     * PAGE CONTEXT — self-registration
     * =====================================================================*/

    (() => {
        const reloadedBySelf = window.sessionStorage.getItem("coiReloadedBySelf");
        window.sessionStorage.removeItem("coiReloadedBySelf");
        const coepDegrading = (reloadedBySelf === "coepdegrade");

        // Ensure we are running in a secure context (https or localhost).
        const n = navigator;
        const controlling = n.serviceWorker && n.serviceWorker.controller;

        // Read user configuration, falling back to sensible defaults.
        const coi = {
            shouldRegister: () => !reloadedBySelf,
            shouldDeregister: () => false,
            coepCredentialless: () => true,
            coepDegrades: true,
            doReload: () => window.location.reload(),
            quiet: false,
            ...window.coi,
        };

        const cred = typeof coi.coepCredentialless === "function"
            ? coi.coepCredentialless()
            : coi.coepCredentialless;

        if (controlling) {
            // Worker is live — push the current credentialless preference to it.
            n.serviceWorker.controller.postMessage({
                type: "coepCredentialless",
                value: coepDegrading ? false : cred,
            });

            if (coi.shouldDeregister()) {
                n.serviceWorker.controller.postMessage({ type: "deregister" });
            }
        }

        // Already isolated (either the SW worked, or real headers are present).
        if (window.crossOriginIsolated !== false || !coi.shouldRegister()) return;

        if (!window.isSecureContext) {
            !coi.quiet && console.log(
                "[coi-sw] Not a secure context. COOP/COEP require https:// or localhost."
            );
            return;
        }

        if (!n.serviceWorker) {
            !coi.quiet && console.log(
                "[coi-sw] Service workers unavailable (private window?). " +
                "SharedArrayBuffer will not be enabled."
            );
            return;
        }

        // Register THIS file as the service worker. `document.currentScript`
        // keeps the path correct on GitHub Pages project sites, where the app
        // lives under /<repo>/ rather than the domain root.
        const scriptUrl = (document.currentScript && document.currentScript.src)
            || new URL("coi-serviceworker.js", document.baseURI).href;

        n.serviceWorker.register(scriptUrl).then(
            (registration) => {
                !coi.quiet && console.log("[coi-sw] Registered. Scope:", registration.scope);

                registration.addEventListener("updatefound", () => {
                    !coi.quiet && console.log("[coi-sw] Update found, reloading.");
                    window.sessionStorage.setItem("coiReloadedBySelf", "updatefound");
                    coi.doReload();
                });

                // If it is already controlling, isolation should have applied —
                // if it has not, the page was loaded before the worker claimed it.
                if (registration.active && !n.serviceWorker.controller) {
                    !coi.quiet && console.log("[coi-sw] Worker active but not controlling. Reloading.");
                    window.sessionStorage.setItem("coiReloadedBySelf", "notcontrolling");
                    coi.doReload();
                }
            },
            (err) => {
                !coi.quiet && console.error("[coi-sw] Registration failed:", err);
            }
        );

        // COEP degradation path: if `credentialless` isolation failed (Safari,
        // Firefox), fall back to `require-corp` once rather than loop.
        if (coi.coepDegrades && cred && !coepDegrading && reloadedBySelf === null) {
            window.addEventListener("load", () => {
                setTimeout(() => {
                    if (window.crossOriginIsolated === false) {
                        !coi.quiet && console.log(
                            "[coi-sw] credentialless did not isolate; degrading to require-corp."
                        );
                        window.sessionStorage.setItem("coiReloadedBySelf", "coepdegrade");
                        coi.doReload();
                    }
                }, 0);
            });
        }
    })();
}
