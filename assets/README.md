# /assets — content layout

Two separate things live here, and they load by completely different mechanisms.
Getting them confused is the most common reason the viewer boots to a black screen.

---

## 1. `dod_custom_pack.zip` — game content (loaded into the virtual FS)

Fetched at boot and unpacked into the Emscripten VFS at `/dod`. **Not committed
to this repo by default** — it is your own content, and it is large.

The archive's internal root becomes the gamedir root. So an entry named
`maps/dod_avalanche.bsp` lands at `/dod/maps/dod_avalanche.bsp`.

```
dod_custom_pack.zip
├── liblist.gam                 ← REQUIRED. Engine refuses the gamedir without it.
├── halflife.wad                ← base textures
├── dod.wad
├── <custom>.wad                ← any WADs your custom maps reference
├── maps/
│   ├── dod_avalanche.bsp
│   └── dod_avalanche.res
├── models/
│   ├── player/
│   │   └── us_garand/
│   │       ├── us_garand.mdl
│   │       └── us_garandT.mdl  ← the T model is what other players see
│   ├── v_garand.mdl            ← viewmodels
│   └── p_garand.mdl            ← world models
├── sprites/
├── sound/
└── <demo>.dem                  ← playdemo resolves from the gamedir ROOT
```

### Build it

```bash
cd /path/to/your/dod
zip -r -9 dod_custom_pack.zip liblist.gam *.wad maps models sprites sound *.dem
```

Deflate (`-9`) and store (`-0`) are both supported — the viewer decompresses
with the browser's native `DecompressionStream`. Use `-0` only if you need to
support a browser without it; the download will be much larger.

### Size discipline

Everything in this archive is decompressed into browser RAM before the first
frame. There is no streaming or lazy loading. Practical ceiling is roughly
**300–500 MB uncompressed** before mobile Safari starts killing the tab.

For a demo viewer you almost never need the whole game. Read the demo's `.res`
requirements and pack only:

- the one map the demo takes place on
- the WADs that map references
- player models for the classes that actually appear
- viewmodels/worldmodels for weapons used
- the HUD sprites

### `liblist.gam`

Minimal working file:

```
game "Day of Defeat"
gamedir "dod"
fallback_dir "valve"
startmap "dod_flash"
trainingmap "dod_flash"
mpentity "info_player_allies"
gamedll "dlls/dod.dll"
gamedll_linux "dlls/dod.so"
type "multiplayer_only"
```

---

## 2. `dod/cl_dlls/` and `dod/dlls/` — game logic (loaded as WASM)

**These are not game content. They are compiled code, and they are the one
hard blocker in this project.**

Xash3D is only the engine. GoldSrc mods ship their gameplay in native libraries
(`client.dll` / `dod.dll` on Windows, `.so` on Linux), and the engine dlopen()s
them at startup. In the browser those become Emscripten side modules:

| Path | What it is | Where to get it |
|---|---|---|
| `dod/cl_dlls/client_emscripten_wasm32.wasm` | HUD, viewmodel, studio-model rendering, demo playback client hooks | **You must compile it** |
| `dod/dlls/dod_emscripten_wasm32.wasm` | Server/game rules, entity logic | **You must compile it** |

The `xash3d-fwgs` npm package ships **engine only** — no game code. Half-Life's
equivalents are published as [`hlsdk-portable`](https://www.npmjs.com/package/hlsdk-portable)
and Counter-Strike's as [`cs16-client`](https://www.npmjs.com/package/cs16-client).
**No published WASM build of Day of Defeat exists.** You have to produce it from
the DoD SDK against `hlsdk-portable`'s Emscripten toolchain:

```bash
git clone --recurse-submodules https://github.com/yohimik/webxash3d-fwgs
# add the DoD client/server sources as a mod target, then:
emcmake cmake -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build
```

Until those two files exist, the engine aborts during dynamic-library loading —
the wrapper queues both paths unconditionally, so a 404 is fatal, not degraded.

### Testing before you have them

To confirm the rest of the pipeline (canvas, COOP/COEP, ZIP mount, query
string) works end to end, temporarily point `index.html` at the Half-Life
libraries and run with `-game valve`:

```js
// index.html → CONFIG
gameDir: 'valve',
clientLib: 'https://cdn.jsdelivr.net/npm/hlsdk-portable@0.1.3/dist/valve/cl_dlls/client_emscripten_wasm32.wasm',
serverLib: 'https://cdn.jsdelivr.net/npm/hlsdk-portable@0.1.3/dist/valve/dlls/hl_emscripten_wasm32.wasm',
```

Pin the version (as above — confirmed working 2026-08-14). Note the `valve/`
layer under `dist/`: it wasn't there in earlier package versions, so if this
404s again on a future version bump, check the package's actual file layout
(`https://data.jsdelivr.com/v1/packages/npm/hlsdk-portable@<version>`) rather
than assuming the path shape is stable.

You'll also need the base `valve/` WADs (`gfx.wad` at minimum) present in
whatever content pack you mount — they're engine-level, not something a
map/demo-driven `pack` tool has any reason to include, but the engine throws
an opaque, non-Error `Infinity` during `Host_InitCommon` without them.

---

## Licensing

Day of Defeat and Half-Life assets are Valve property. Do not commit game
content to a public repository. Host the pack behind your own access control,
or require users to supply their own via the file picker.
