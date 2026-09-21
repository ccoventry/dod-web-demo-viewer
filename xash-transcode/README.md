# xash-transcode

GoldSrc **HLDEMO** → Xash3D **IDEM** container transcoder, with clip cutting.

Standalone crate — it declares its own `[workspace]`, so it does not join the
`dod-tools` workspace and the root `Cargo.toml` is untouched. Add it to
`members` when you want it in `cargo build --workspace`.

```bash
cd xash-transcode
cargo run --release -- inspect  ../demos/analysis_target_pov.dem
cargo run --release -- convert  ../demos/analysis_target_pov.dem /tmp/full.dem
cargo run --release -- cut      ../demos/analysis_target_pov.dem /tmp/clip.dem 300 320
cargo run --release -- validate /tmp/clip.dem

# content pack containing only what this demo references
cargo run --release -- pack ../demos/analysis_target_pov.dem \
    ../../dod-web-demo-viewer/assets/dod_custom_pack.zip \
    --game-root "C:/Program Files (x86)/Steam/steamapps/common/Half-Life"
```

## The problem it solves

Xash3D cannot open a DoD demo. Different containers:

| | GoldSrc | Xash3D |
|---|---|---|
| magic | `"HLDEMO\0\0"` (8 bytes) | `"IDEM"` (i32) |
| demo protocol | 5 | 3 |
| path fields | `[260]` | `[64]` |
| extras | `map_checksum` | `host_fps` (f64), `comment` |

`CL_ParseDemoHeader` rejects on `id != IDEMOHEADER` before reading anything else.

But Xash accepts one foreign value in `net_protocol`:

```c
#define PROTOCOL_GOLDSRC_VERSION_DEMO (PROTOCOL_GOLDSRC_VERSION | BIT(7))  // 176
```

With that set, `CL_GetProtocolFromDemo` returns `PROTO_GOLDSRC` and the engine
decodes the contained messages with GoldSrc protocol-48 semantics — which is
what a DoD demo already carries. Only the wrapper is wrong. This crate rewrites
the wrapper.

## Frame mapping

GoldSrc has nine frame types, Xash six commands:

| GoldSrc | Xash | |
|---|---|---|
| `NetworkMessage(Start)` | `dem_norewind` (1) | signon |
| `NetworkMessage(Normal)` | `dem_read` (2) | gameplay stream |
| `DemoStart` | `dem_jumptime` (3) | resets section clock |
| `NextSection` | `dem_stop` (6) | terminator |
| `DemoBuffer` | `dem_userdata` (4) | opt-in via `--userdata` |
| `ConsoleCommand` | — | **dropped** |
| `ClientData` | — | dropped |
| `Event` / `WeaponAnimation` / `Sound` | — | dropped |

`SequenceInfo` maps field-for-field onto `CL_ReadDemoSequence` — same seven
i32s, same order. That part is a straight copy.

**`ConsoleCommand` frames are where your injected director commands and
bookmarks live.** Xash has no equivalent frame type, so they do not survive.
Fine for triage preview; it means transcoded demos are not a capture path.

## Measured on real demos

`analysis_target_pov.dem`, dod_Emmanuel, 66.5 MB, 986 s:

| output | size | % of source |
|---|---|---|
| full transcode | 16.2 MB | 24.3% |
| 86 s clip | 1.5 MB | 2.3% |
| 30 s clip | 524 KB | 0.8% |
| 20 s clip | 369 KB | 0.6% |

The shrink comes from dropping `ClientData` + `DemoBuffer`, which together are
~76% of frames and carry nothing the engine needs for playback.

## Parse mode matters

Use `MessageDataParseMode::Raw`. Message payloads stay borrowed bytes, so the
transcode is byte-exact and much faster. `Parse` mode forces a re-serialisation
round trip through `write_netmsg` — slower, and a fidelity risk for no benefit
unless you are also mutating messages.

## Cutting and delta compression

`svc_deltapacketentities` encodes against an earlier frame, so a cut landing
mid-stream shows corrupt entities until the next full update. `--preroll`
(default 3 s) is a blunt mitigation that usually works because full updates are
frequent.

The correct fix — walk forward from the cut point to the first frame carrying a
non-delta `svc_packetentities` — needs message-level parsing via
`dem::netmsg_doer`, i.e. a second pass in `Parse` mode. Marked TODO in
`lib.rs::cut`. Do it if preroll proves unreliable in practice.

## The pack builder

`pack` reads what a demo actually references and pulls only those files out of
your install. Two independent sources, because neither is sufficient alone:

1. **`svc_resourcelist` (svc 43)** from the demo's signon section — every model,
   sprite, sound and event script the server precached.
2. **The map BSP's entity lump** — WAD dependencies live in `worldspawn`'s `wad`
   key and appear *nowhere* in the demo. Miss these and the map renders as
   purple checkerboard. `skyname` comes from here too.

Resolution follows the engine's own order: gamedir first, then `valve`, with a
case-insensitive retry because demos record whatever case the server used.

Sounds are skipped by default — they are most of the payload and the preview has
no audio. `--sound` includes them.

Other behaviour worth knowing:

- **Decals are skipped.** They are indices into `decals.wad`, not files.
- **Size mismatches are reported.** If a local file's size differs from what the
  server declared, it is probably the wrong version and may desync playback.
  This is the fastest way to spot a custom-content mismatch.
- **`liblist.gam` is synthesised** if your install doesn't have one. Xash refuses
  a gamedir without it.
- **Generic and event-script resources are optional** — servers routinely
  precache things clients legitimately lack, so their absence isn't an error.

`pack` needs `MessageDataParseMode::Parse` to reach the resource list, so it is
much slower than the transcode path. It runs once per demo, not per preview.

## wasm

`lib.rs` has no I/O — all file access lives in `main.rs`. It compiles to
`wasm32-unknown-unknown` as-is, so the same transcoder can run in the browser
viewer and in the Tauri app.

## Validation

`validate()` re-implements `CL_ParseDemoHeader`, `CL_PlayDemo_f` and the
`CL_DemoReadMessage` walk: magic, protocols, directory bounds, per-entry frame
walk landing exactly on the declared length, `MAX_INIT_MSG` ceiling, and a
terminating `dem_stop`.

Passing means **structurally acceptable to Xash**. It does not prove the
contents replay correctly — that needs a real engine, and is the open question.

## Reference implementation

`reference/` holds the Python prototype this crate was derived from. It was run
against real DoD demos first to confirm every struct size: all four sections of
both test demos walked to *exactly* their declared `file_length`, which is what
validates the 436-byte `DemoInfo` block and the rest of the layout.

Keep it as an oracle — if the Rust output ever disagrees with the Python
output for the same input, one of them has drifted.

```bash
cd reference && python3 -c "
import hldemo, idem
buf, hdr, entries = hldemo.load('../../demos/analysis_target_pov.dem')
data, oe, stats = idem.cut(buf, hdr, entries, 300, 320)
open('/tmp/clip.dem','wb').write(data)
print(stats); print(idem.validate(data)[0] or 'PASS')"
```
