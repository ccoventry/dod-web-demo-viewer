//! Filesystem + zip layer for the pack builder.
//!
//! Part of the binary, not the library — `xash_transcode::resources` stays
//! wasm-clean and does the actual thinking; this just resolves paths against a
//! real DoD install and writes the archive.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use dem::types::Demo;
use xash_transcode::resources::{self, DemoResource, ResourceKind};
use zip::write::SimpleFileOptions;

pub struct PackOptions {
    /// Root of the Half-Life install, i.e. the folder containing `dod/` and `valve/`.
    pub game_root: PathBuf,
    pub gamedir: String,
    pub fallback: String,
    /// Sounds are ~most of the payload and the preview has no audio.
    pub include_sound: bool,
    pub verbose: bool,
}

impl Default for PackOptions {
    fn default() -> Self {
        Self {
            game_root: PathBuf::new(),
            gamedir: "dod".into(),
            fallback: "valve".into(),
            include_sound: false,
            verbose: false,
        }
    }
}

#[derive(Debug, Clone)]
struct Wanted {
    kind: String,
    /// gamedir-relative
    path: String,
    /// server-declared size, 0 when unknown (wads, sky, liblist)
    declared: u32,
    /// optional resources don't count as failures when absent
    optional: bool,
}

const LIBLIST_FALLBACK: &str = "\
game \"Day of Defeat\"
gamedir \"dod\"
fallback_dir \"valve\"
startmap \"dod_flash\"
trainingmap \"dod_flash\"
mpentity \"info_player_allies\"
gamedll \"dlls/dod.dll\"
gamedll_linux \"dlls/dod.so\"
type \"multiplayer_only\"
";

fn human(n: u64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

impl PackOptions {
    /// Try the gamedir first, then the fallback dir — same order the engine uses.
    fn resolve(&self, rel: &str) -> Option<PathBuf> {
        for dir in [&self.gamedir, &self.fallback] {
            let p = self.game_root.join(dir).join(rel);
            if p.is_file() {
                return Some(p);
            }
        }
        // Case-insensitive retry: demos record whatever case the server used,
        // which bites on Linux/macOS and on case-sensitive volumes.
        for dir in [&self.gamedir, &self.fallback] {
            if let Some(p) = ci_lookup(&self.game_root.join(dir), rel) {
                return Some(p);
            }
        }
        None
    }
}

/// Walk a relative path component by component, matching case-insensitively.
fn ci_lookup(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut cur = root.to_path_buf();
    for comp in rel.split('/').filter(|c| !c.is_empty()) {
        let want = comp.to_ascii_lowercase();
        let mut hit = None;
        for e in std::fs::read_dir(&cur).ok()? {
            let e = e.ok()?;
            if e.file_name().to_string_lossy().to_ascii_lowercase() == want {
                hit = Some(e.path());
                break;
            }
        }
        cur = hit?;
    }
    cur.is_file().then_some(cur)
}

pub fn build(demo: &Demo, out_zip: &Path, o: &PackOptions) -> eyre::Result<()> {
    let map_name = demo
        .header
        .map_name
        .to_str()
        .unwrap_or_default()
        .trim_end_matches(".bsp")
        .to_string();

    if map_name.is_empty() {
        eyre::bail!("demo header has no map name");
    }

    println!("map:      {map_name}");
    println!("gamedir:  {}", o.gamedir);
    println!("source:   {}", o.game_root.display());

    let mut wanted: Vec<Wanted> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    let push = |w: Wanted, wanted: &mut Vec<Wanted>, seen: &mut BTreeSet<String>| {
        if seen.insert(w.path.to_ascii_lowercase()) {
            wanted.push(w);
        }
    };

    // ---- 1. the map itself ------------------------------------------------
    let bsp_rel = format!("maps/{map_name}.bsp");
    push(
        Wanted {
            kind: "map".into(),
            path: bsp_rel.clone(),
            declared: 0,
            optional: false,
        },
        &mut wanted,
        &mut seen,
    );

    // ---- 2. WADs + sky, from the BSP entity lump --------------------------
    // These appear nowhere in the demo. Without them the map is purple.
    match o.resolve(&bsp_rel) {
        Some(p) => {
            let bytes = std::fs::read(&p)?;
            match resources::parse_bsp(&bytes) {
                Ok(info) => {
                    println!(
                        "bsp:      v{} — {} wad(s){}",
                        info.version,
                        info.wads.len(),
                        info.skyname
                            .as_ref()
                            .map(|s| format!(", sky {s:?}"))
                            .unwrap_or_default()
                    );
                    for w in &info.wads {
                        push(
                            Wanted {
                                kind: "wad".into(),
                                path: w.clone(),
                                declared: 0,
                                optional: false,
                            },
                            &mut wanted,
                            &mut seen,
                        );
                    }
                    if let Some(sky) = &info.skyname {
                        // Only some of the six faces × two extensions exist.
                        for p in resources::sky_paths(sky) {
                            push(
                                Wanted {
                                    kind: "sky".into(),
                                    path: p,
                                    declared: 0,
                                    optional: true,
                                },
                                &mut wanted,
                                &mut seen,
                            );
                        }
                    }
                }
                Err(e) => eprintln!("warning: could not read BSP entity lump: {e}"),
            }
        }
        None => eprintln!(
            "warning: {bsp_rel} not found under {} — WAD list unavailable, \
             the map will render untextured",
            o.game_root.display()
        ),
    }

    // ---- 3. everything the demo precached ---------------------------------
    let res: Vec<DemoResource> = resources::resources(demo);
    if res.is_empty() {
        eprintln!(
            "warning: no svc_resourcelist found. Parse the demo with \
             MessageDataParseMode::Parse — Raw mode leaves messages undecoded."
        );
    }

    let mut skipped_sound = 0usize;
    for r in &res {
        if !r.kind.is_file() {
            continue;
        }
        if r.kind == ResourceKind::Sound && !o.include_sound {
            skipped_sound += 1;
            continue;
        }
        push(
            Wanted {
                kind: r.kind.label(),
                path: r.path.clone(),
                declared: r.size,
                // Servers precache things clients legitimately lack.
                optional: matches!(r.kind, ResourceKind::Generic | ResourceKind::EventScript),
            },
            &mut wanted,
            &mut seen,
        );
    }

    // ---- 4. gamedir metadata ---------------------------------------------
    push(
        Wanted {
            kind: "meta".into(),
            path: "liblist.gam".into(),
            declared: 0,
            optional: true,
        },
        &mut wanted,
        &mut seen,
    );

    println!("wanted:   {} file(s)", wanted.len());
    if skipped_sound > 0 {
        println!("          ({skipped_sound} sounds skipped — pass --sound to include)");
    }

    // ---- 5. resolve + write ----------------------------------------------
    let file = std::fs::File::create(out_zip)?;
    let mut zw = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut found: BTreeMap<String, (usize, u64)> = BTreeMap::new();
    let mut missing: Vec<Wanted> = Vec::new();
    let mut size_mismatch: Vec<(String, u32, u64)> = Vec::new();
    let mut total_raw = 0u64;
    let mut has_liblist = false;

    for w in &wanted {
        let Some(src) = o.resolve(&w.path) else {
            if !w.optional {
                missing.push(w.clone());
            }
            continue;
        };

        let bytes = std::fs::read(&src)?;
        if w.declared > 0 && bytes.len() as u32 != w.declared {
            size_mismatch.push((w.path.clone(), w.declared, bytes.len() as u64));
        }

        zw.start_file(&w.path, opts)?;
        zw.write_all(&bytes)?;

        let e = found.entry(w.kind.clone()).or_insert((0, 0));
        e.0 += 1;
        e.1 += bytes.len() as u64;
        total_raw += bytes.len() as u64;

        if w.path == "liblist.gam" {
            has_liblist = true;
        }
        if o.verbose {
            println!("  + {:<8} {}", w.kind, w.path);
        }
    }

    if !has_liblist {
        // Xash refuses a gamedir without one, so synthesise rather than fail.
        zw.start_file("liblist.gam", opts)?;
        zw.write_all(LIBLIST_FALLBACK.as_bytes())?;
        println!("note:     liblist.gam not found in install — synthesised a minimal one");
    }

    zw.finish()?;
    let packed = std::fs::metadata(out_zip)?.len();

    // ---- 6. report --------------------------------------------------------
    println!("\n{:<10} {:>6}  {:>12}", "kind", "files", "bytes");
    println!("{}", "-".repeat(31));
    for (k, (n, b)) in &found {
        println!("{k:<10} {n:>6}  {:>12}", human(*b));
    }
    println!("{}", "-".repeat(31));
    println!(
        "{:<10} {:>6}  {:>12}  ->  {} zipped ({:.0}%)",
        "total",
        found.values().map(|v| v.0).sum::<usize>(),
        human(total_raw),
        human(packed),
        100.0 * packed as f64 / total_raw.max(1) as f64
    );

    if !size_mismatch.is_empty() {
        println!(
            "\n{} file(s) differ in size from what the server declared — \
             these are likely the wrong version and may desync playback:",
            size_mismatch.len()
        );
        for (p, want, got) in size_mismatch.iter().take(15) {
            println!("  {p}  server={want} local={got}");
        }
        if size_mismatch.len() > 15 {
            println!("  ... and {} more", size_mismatch.len() - 15);
        }
    }

    if missing.is_empty() {
        println!("\nAll required files found.");
    } else {
        println!("\n{} required file(s) MISSING:", missing.len());
        for m in missing.iter().take(25) {
            println!("  {:<8} {}", m.kind, m.path);
        }
        if missing.len() > 25 {
            println!("  ... and {} more", missing.len() - 25);
        }
        println!(
            "\nMissing player models or sprites usually mean custom client content \
             the recorder had and this install does not."
        );
    }

    println!("\nwrote {}", out_zip.display());
    Ok(())
}
