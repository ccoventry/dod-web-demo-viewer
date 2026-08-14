# Day of Defeat 1.3 — Web Demo Viewer

Standalone, dependency-free browser client for playing back DoD 1.3 `.dem` files,
built on [Xash3D-FWGS](https://github.com/FWGS/xash3d-fwgs) compiled to
WebAssembly via [`xash3d-fwgs`](https://www.npmjs.com/package/xash3d-fwgs).
Deploys to GitHub Pages as static files — no build step, no bundler, no server.

```
.
├── index.html              engine host + canvas + ZIP mount + query-string wrapper
├── coi-serviceworker.js    COOP/COEP shim (GitHub Pages cannot set headers)
├── .nojekyll               stop Jekyll from mangling asset paths
└── assets/
    ├── README.md           content layout + the game-DLL blocker (read this)
    ├── dod_custom_pack.zip your game content — NOT committed
    └── dod/
        ├── cl_dlls/        client_emscripten_wasm32.wasm
        └── dlls/           dod_emscripten_wasm32.wasm
```

## Deploy

```bash
git init && git add . && git commit -m "DoD web demo viewer"
git push origin main
# Settings → Pages → Source: main branch, / (root)
```

Then: `https://<user>.github.io/<repo>/?demo=match`

## URL parameters

| Param | Example | Effect |
|---|---|---|
| `demo` | `?demo=match.dem` | Auto-plays that demo. `.dem` optional. Resolved from the gamedir root. |
| `map` | `?map=dod_avalanche` | Loads a map instead of a demo. |
| `pack` | `?pack=assets/eu2004.zip` | Override the asset pack URL. |
| `renderer` | `?renderer=soft` | `gl4es` (default), `gles3compat`, or `soft`. |
| `dev` | `?dev=1` | Engine console output overlay. First stop when debugging. |
| `autostart` | `?autostart=1` | Skip the click gate. Audio may not initialise. |

Both are sanitised — path traversal and shell-ish characters are rejected before
they reach the engine console, since these values arrive from shared links.

## Boot sequence

The order in `index.html` is load-bearing:

1. `new Xash3D({...})` — configure; nothing is fetched
2. `await xash.init()` — WASM runtime up, Emscripten VFS now exists
3. `xash.FS.downloadAndExtractZip(...)` — write game files into the VFS
4. `xash.main()` — start the frame loop

Writing assets before (2) throws (no filesystem yet). Writing after (4) races the
engine, which has already scanned the gamedir and built its file index.

## Notes on the implementation

**`downloadAndExtractZip` is ours, not the library's.** `xash3d-fwgs` exposes
the raw Emscripten FS (`mkdir`, `mkdirTree`, `writeFile`, `chdir`, `syncfs`) and
has no archive support. `index.html` implements a ZIP central-directory reader
(store + deflate, with ZIP64) using the browser's native `DecompressionStream`,
then grafts it onto `xash.FS` via `Object.create(xash.em.FS)` — so the real FS
stays the prototype and nothing is mutated.

**Every engine `.wasm` is mapped explicitly in `filesMap`.** The wrapper's
`locateFile` is `filesMap[path] ?? path` with no fallback to the module's own
origin, so an unmapped binary resolves against the *page* URL and 404s on Pages.

**Root-absolute paths are rewritten.** On a project site the app lives at
`/<repo>/`, so `/assets/x.zip` would 404. `appUrl()` resolves against
`document.baseURI`.

**Do not "fix" the CDN import to `/dist/index.js`.** That file is TypeScript
output — `export * from './xash3d'` with no file extension — which native browser
ESM cannot resolve. jsDelivr's `/+esm` endpoint bundles the package with
extensions resolved, so it is the only import specifier that works without a
bundler. The version is pinned; `@latest` will eventually break the WASM ABI.

**`-nomouse`** is passed as requested; it is not a documented Xash3D-FWGS switch,
and unknown arguments are ignored rather than fatal. `m_ignore 1` is issued
post-boot as the switch that actually does the job.

**Cross-origin isolation is probably optional here.** The published build has no
`xash.worker.js`, which means it is almost certainly a single-threaded (non-
pthreads) build that never touches SharedArrayBuffer. The service worker costs
nothing and future-proofs a threaded rebuild, so it stays — but if you hit COEP
blocking on a third-party asset host, dropping the shim is a legitimate fix.

## Known blocker

**There is no published WebAssembly build of Day of Defeat's game libraries.**
The engine loads mod gameplay from `cl_dlls/client_*.wasm` and `dlls/*.wasm`,
and those must be compiled from the DoD SDK. See `assets/README.md`.

Everything else in this repo is complete and testable today — point `CONFIG` at
`hlsdk-portable` with `-game valve` to verify the full pipeline end to end.

## Credits

Engine port: [yohimik/webxash3d-fwgs](https://github.com/yohimik/webxash3d-fwgs).
COOP/COEP shim derived from [gzuidhof/coi-serviceworker](https://github.com/gzuidhof/coi-serviceworker) (MIT).
Day of Defeat and Half-Life are trademarks of Valve Corporation.
