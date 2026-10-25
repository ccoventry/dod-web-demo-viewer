//! Work out which game files a demo actually needs.
//!
//! Two independent sources, because neither is sufficient alone:
//!
//! 1. **`svc_resourcelist` (svc 43)** in the demo's signon section — every
//!    model, sprite, sound and event script the server precached.
//! 2. **The map BSP's entity lump** — WAD dependencies live in `worldspawn`'s
//!    `wad` key and appear nowhere in the demo. Miss these and the map renders
//!    as solid purple checkerboard.
//!
//! Both functions here are pure (`&Demo` / `&[u8]` in, data out) so this module
//! stays `wasm32` clean. All filesystem work lives in the CLI.

use std::collections::BTreeSet;

use dem::prelude::*;
use dem::types::{Demo, EngineMessage, FrameData, MessageData, NetMessage};

/// GoldSrc `resourcetype_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceKind {
    Sound,
    Skin,
    Model,
    Decal,
    Generic,
    EventScript,
    World,
    Unknown(u8),
}

impl ResourceKind {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Sound,
            1 => Self::Skin,
            2 => Self::Model,
            3 => Self::Decal,
            4 => Self::Generic,
            5 => Self::EventScript,
            6 => Self::World,
            other => Self::Unknown(other),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Sound => "sound".into(),
            Self::Skin => "skin".into(),
            Self::Model => "model".into(),
            Self::Decal => "decal".into(),
            Self::Generic => "generic".into(),
            Self::EventScript => "event".into(),
            Self::World => "world".into(),
            Self::Unknown(v) => format!("unknown({v})"),
        }
    }

    /// Decals are indices into `decals.wad`, not standalone files, so there is
    /// nothing to pull for them.
    pub fn is_file(&self) -> bool {
        !matches!(self, Self::Decal | Self::Unknown(_))
    }
}

#[derive(Debug, Clone)]
pub struct DemoResource {
    pub kind: ResourceKind,
    /// Exactly as the server sent it.
    pub name: String,
    /// Path relative to the gamedir root, ready to look up on disk.
    pub path: String,
    /// Server-declared size in bytes. Useful for spotting missing/renamed files.
    pub size: u32,
}

/// Extract every resource the demo precaches.
///
/// Requires the demo to be parsed with
/// [`dem::types::MessageDataParseMode::Parse`] — `Raw` leaves messages as
/// undecoded bytes and this returns empty.
///
/// Deduplicated and sorted. A demo spanning a level change carries more than
/// one resource list; all are merged.
pub fn resources(demo: &Demo) -> Vec<DemoResource> {
    let mut seen: BTreeSet<(ResourceKind, String)> = BTreeSet::new();
    let mut out: Vec<DemoResource> = Vec::new();

    for entry in &demo.directory.entries {
        for frame in &entry.frames {
            let FrameData::NetworkMessage(boxed) = &frame.frame_data else {
                continue;
            };
            let MessageData::Parsed(msgs) = &boxed.as_ref().1.messages else {
                continue;
            };

            for msg in msgs {
                let NetMessage::EngineMessage(eng) = msg else {
                    continue;
                };
                let EngineMessage::SvcResourceList(rl) = eng.as_ref() else {
                    continue;
                };

                for r in &rl.resources {
                    let kind = ResourceKind::from_u8(r.type_.to_u8());
                    let name = clean_resource_name(r.name.get_string());
                    if name.is_empty() {
                        continue;
                    }
                    if is_inline_bsp_model(kind, &name) {
                        continue;
                    }

                    let key = (kind, name.clone());
                    if !seen.insert(key) {
                        continue;
                    }

                    out.push(DemoResource {
                        kind,
                        path: resolve_path(kind, &name),
                        name,
                        size: r.size.to_u32(),
                    });
                }
            }
        }
    }

    out.sort_by(|a, b| (a.kind, &a.path).cmp(&(b.kind, &b.path)));
    out
}

/// GoldSrc names brush entities baked into the map (doors, buttons, etc.)
/// `*N` — an index into the BSP's own model lump, not a standalone `.mdl`
/// file. The server precaches these alongside real models, so without this
/// check every one of them shows up as a "missing" file in the pack report.
fn is_inline_bsp_model(kind: ResourceKind, name: &str) -> bool {
    kind == ResourceKind::Model && name.starts_with('*')
}

/// `BitSliceCast::get_string()` (dem-patch/src/bit.rs) dumps a field's
/// fixed-size byte buffer verbatim — any NUL padding after the C-string's
/// real terminator survives into the Rust `String` as embedded `\0` bytes.
/// Left in, those corrupt both the dedup key in `resources()` and every
/// filesystem lookup downstream (a path containing `\0` can never resolve),
/// so truncate at the first NUL here — the field is a null-terminated C
/// string, this is its real end.
fn clean_resource_name(raw: String) -> String {
    raw.split('\0').next().unwrap_or("").to_string()
}

/// Turn a precache name into a gamedir-relative path.
///
/// Sounds are the exception: GoldSrc precaches them relative to `sound/`, so
/// `weapons/garand_fire.wav` really lives at `sound/weapons/garand_fire.wav`.
/// Everything else is already gamedir-relative.
fn resolve_path(kind: ResourceKind, name: &str) -> String {
    let n = name.trim_start_matches('/').replace('\\', "/");
    match kind {
        ResourceKind::Sound if !n.starts_with("sound/") => format!("sound/{n}"),
        _ => n,
    }
}

// ---------------------------------------------------------------------------
// BSP
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct BspInfo {
    pub version: i32,
    /// WAD basenames from `worldspawn`'s `wad` key, in declared order.
    pub wads: Vec<String>,
    pub skyname: Option<String>,
}

/// Read WAD and sky dependencies out of a BSP's entity lump.
///
/// GoldSrc BSP v30 layout: `i32 version`, then 15 × `{ i32 offset, i32 length }`.
/// Lump 0 is the entity string.
pub fn parse_bsp(bsp: &[u8]) -> eyre::Result<BspInfo> {
    if bsp.len() < 12 {
        eyre::bail!("BSP too short");
    }

    let rd = |at: usize| -> i32 {
        i32::from_le_bytes([bsp[at], bsp[at + 1], bsp[at + 2], bsp[at + 3]])
    };

    let version = rd(0);
    // v30 is GoldSrc. v29 is Quake; still worth a try rather than a hard fail.
    let ent_off = rd(4) as usize;
    let ent_len = rd(8) as usize;

    if ent_off == 0 || ent_off + ent_len > bsp.len() {
        eyre::bail!("BSP entity lump out of range (offset {ent_off}, len {ent_len})");
    }

    let ents = String::from_utf8_lossy(&bsp[ent_off..ent_off + ent_len]);
    let worldspawn = first_entity(&ents).unwrap_or(&ents);

    let mut info = BspInfo {
        version,
        ..Default::default()
    };

    if let Some(v) = entity_value(worldspawn, "wad") {
        let mut seen = BTreeSet::new();
        for part in v.split(';') {
            let base = part
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if base.is_empty() || !base.to_ascii_lowercase().ends_with(".wad") {
                continue;
            }
            if seen.insert(base.to_ascii_lowercase()) {
                info.wads.push(base);
            }
        }
    }

    info.skyname = entity_value(worldspawn, "skyname")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    Ok(info)
}

/// The six faces GoldSrc expects for a sky, and the extensions worth trying.
pub fn sky_paths(skyname: &str) -> Vec<String> {
    let mut v = Vec::new();
    for face in ["bk", "dn", "ft", "lf", "rt", "up"] {
        for ext in ["tga", "bmp"] {
            v.push(format!("gfx/env/{skyname}{face}.{ext}"));
        }
    }
    v
}

fn first_entity(ents: &str) -> Option<&str> {
    let start = ents.find('{')?;
    let end = ents[start..].find('}')? + start;
    Some(&ents[start..end])
}

/// Pull `"key" "value"` out of an entity block.
fn entity_value<'a>(block: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let mut rest = block;
    while let Some(k) = rest.find(&needle) {
        let after = &rest[k + needle.len()..];
        // next quoted token is the value
        if let Some(o) = after.find('"') {
            let after_open = &after[o + 1..];
            if let Some(c) = after_open.find('"') {
                return Some(&after_open[..c]);
            }
        }
        rest = &rest[k + needle.len()..];
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_wads_and_sky() {
        let ents = r#"
{
"wad" "\sierra\half-life\valve\halflife.wad;\sierra\half-life\dod\dod.wad;\dod\custom.wad;"
"skyname" "dusk"
"classname" "worldspawn"
}
{
"classname" "info_player_allies"
}
"#;
        let ws = first_entity(ents).unwrap();
        assert_eq!(entity_value(ws, "skyname"), Some("dusk"));

        let raw = entity_value(ws, "wad").unwrap();
        let wads: Vec<_> = raw
            .split(';')
            .filter_map(|p| p.rsplit(['/', '\\']).next())
            .map(|s| s.trim())
            .filter(|s| s.to_ascii_lowercase().ends_with(".wad"))
            .collect();
        assert_eq!(wads, vec!["halflife.wad", "dod.wad", "custom.wad"]);
    }

    #[test]
    fn sound_paths_get_prefixed_once() {
        assert_eq!(
            resolve_path(ResourceKind::Sound, "weapons/garand_fire.wav"),
            "sound/weapons/garand_fire.wav"
        );
        assert_eq!(
            resolve_path(ResourceKind::Sound, "sound/ambience/wind.wav"),
            "sound/ambience/wind.wav"
        );
        assert_eq!(
            resolve_path(ResourceKind::Model, "models/player/us_garand/us_garand.mdl"),
            "models/player/us_garand/us_garand.mdl"
        );
        assert_eq!(
            resolve_path(ResourceKind::Model, "models\\v_garand.mdl"),
            "models/v_garand.mdl"
        );
    }

    #[test]
    fn decals_are_not_files() {
        assert!(!ResourceKind::Decal.is_file());
        assert!(ResourceKind::Model.is_file());
    }

    #[test]
    fn resource_names_are_truncated_at_the_first_nul() {
        assert_eq!(
            clean_resource_name("maps/dod_Emmanuel.bsp\0".to_string()),
            "maps/dod_Emmanuel.bsp"
        );
        // Multiple trailing padding bytes, as a fixed-size buffer would leave.
        assert_eq!(
            clean_resource_name("models/null.mdl\0\0\0\0\0\0".to_string()),
            "models/null.mdl"
        );
        assert_eq!(clean_resource_name("clean/name.mdl".to_string()), "clean/name.mdl");
        assert_eq!(clean_resource_name("\0".to_string()), "");
    }

    #[test]
    fn inline_bsp_models_are_excluded() {
        assert!(is_inline_bsp_model(ResourceKind::Model, "*1"));
        assert!(is_inline_bsp_model(ResourceKind::Model, "*120"));
        assert!(!is_inline_bsp_model(
            ResourceKind::Model,
            "models/player/us_garand/us_garand.mdl"
        ));
        // A sound or sprite named with a leading `*` isn't a BSP submodel —
        // only ResourceKind::Model uses that namespace.
        assert!(!is_inline_bsp_model(ResourceKind::Sound, "*1"));
    }
}
