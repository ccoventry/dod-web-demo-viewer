//! CLI harness for the HLDEMO → IDEM transcoder.
//!
//!     xash-transcode inspect   <in.dem>
//!     xash-transcode convert   <in.dem> <out.dem> [--userdata] [--gamedir dod]
//!     xash-transcode cut       <in.dem> <out.dem> <start_s> <end_s> [--preroll 3.0]
//!     xash-transcode validate  <file.dem>
//!
//! All file I/O lives here so `lib.rs` stays wasm-clean.

use std::process::ExitCode;

use dem::types::{Demo, FrameData, MessageDataParseMode};
use xash_transcode::{cut, transcode, validate, Options};

mod packer;

fn usage() -> ExitCode {
    eprintln!(
        "\
xash-transcode — GoldSrc HLDEMO -> Xash3D IDEM

USAGE
  inspect  <in.dem>                              structural report
  convert  <in.dem> <out.dem>                    full transcode
  cut      <in.dem> <out.dem> <start> <end>      transcode a time window
  validate <file.dem>                            re-check an IDEM file
  pack     <in.dem> <out.zip> --game-root <dir>  build a minimal content pack

OPTIONS (transcode)
  --userdata           forward DemoBuffer frames as dem_userdata (bigger)
  --gamedir <name>     override the gamedir field (default: from source)
  --preroll <seconds>  extra lead-in before a cut (default 3.0)
  --fps <n>            host_fps written to the header (default 100)

OPTIONS (pack)
  --game-root <dir>    Half-Life install root (the folder holding dod/ and valve/)
  --gamedir <name>     mod directory to read from (default dod)
  --fallback <name>    fallback directory (default valve)
  --sound              include precached sounds (large; preview has no audio)
  --verbose            list every file added
"
    );
    ExitCode::from(2)
}

fn parse_opts(args: &[String]) -> (Options, f32) {
    let mut o = Options::default();
    let mut preroll = 3.0f32;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--userdata" => o.keep_userdata = true,
            "--no-rebase" => o.rebase_time = false,
            "--gamedir" => {
                i += 1;
                o.gamedir = args.get(i).cloned();
            }
            "--preroll" => {
                i += 1;
                preroll = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(3.0);
            }
            "--fps" => {
                i += 1;
                o.host_fps = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(100.0);
            }
            _ => {}
        }
        i += 1;
    }
    (o, preroll)
}

/// Raw mode keeps message payloads as borrowed bytes — no re-serialisation, so
/// the transcode is byte-exact and roughly an order of magnitude faster than
/// `Parse`.
fn load(path: &str) -> eyre::Result<Demo> {
    let bytes = std::fs::read(path)?;
    Demo::parse_from_bytes(&bytes, MessageDataParseMode::Raw)
}

/// `pack` needs decoded messages to reach `svc_resourcelist`, so it pays for a
/// full parse. Slower than `Raw` — but it runs once per demo, not per preview.
fn load_parsed(path: &str) -> eyre::Result<Demo> {
    let bytes = std::fs::read(path)?;
    Demo::parse_from_bytes(&bytes, MessageDataParseMode::Parse)
}

fn parse_pack_opts(args: &[String]) -> packer::PackOptions {
    let mut o = packer::PackOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--sound" => o.include_sound = true,
            "--verbose" | "-v" => o.verbose = true,
            "--game-root" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    o.game_root = v.into();
                }
            }
            "--gamedir" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    o.gamedir = v.clone();
                }
            }
            "--fallback" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    o.fallback = v.clone();
                }
            }
            _ => {}
        }
        i += 1;
    }
    o
}

fn human(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn inspect(path: &str) -> eyre::Result<()> {
    let demo = load(path)?;
    let h = &demo.header;

    println!("{path}");
    println!(
        "  demo_protocol={}  network_protocol={}",
        h.demo_protocol, h.network_protocol
    );
    println!(
        "  map={:?}  gamedir={:?}  checksum=0x{:08x}",
        h.map_name.to_str().unwrap_or("<invalid>"),
        h.game_directory.to_str().unwrap_or("<invalid>"),
        h.map_checksum
    );

    for (i, e) in demo.directory.entries.iter().enumerate() {
        let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
        let mut max_msg = 0usize;
        let mut span = (f32::MAX, f32::MIN);

        for f in &e.frames {
            let k = match &f.frame_data {
                FrameData::NetworkMessage(b) => {
                    if let dem::types::MessageData::Raw(r) = &b.as_ref().1.messages {
                        max_msg = max_msg.max(r.len());
                    }
                    "NetworkMessage"
                }
                FrameData::DemoStart => "DemoStart",
                FrameData::ConsoleCommand(_) => "ConsoleCommand",
                FrameData::ClientData(_) => "ClientData",
                FrameData::NextSection => "NextSection",
                FrameData::Event(_) => "Event",
                FrameData::WeaponAnimation(_) => "WeaponAnimation",
                FrameData::Sound(_) => "Sound",
                FrameData::DemoBuffer(_) => "DemoBuffer",
            };
            *counts.entry(k).or_default() += 1;
            span.0 = span.0.min(f.time);
            span.1 = span.1.max(f.time);
        }

        println!(
            "  entry {i}: type={} desc={:?} frames={} span={:.1}s..{:.1}s",
            e.type_,
            e.description.to_str().unwrap_or(""),
            human(e.frames.len()),
            span.0,
            span.1
        );
        println!("           largest netmsg: {} B", human(max_msg));
        println!("           {counts:?}");
    }
    Ok(())
}

fn report(out: &xash_transcode::Output, src_len: usize) {
    println!(
        "  {} B -> {} B  ({:.1}%)",
        human(src_len),
        human(out.bytes.len()),
        100.0 * out.bytes.len() as f64 / src_len as f64
    );
    for (k, v) in &out.stats {
        println!("    {:<34} {}", k, human(*v));
    }
    for e in &out.entries {
        println!(
            "    entry type={} {:?} frames={} offset={} length={}",
            e.entrytype,
            e.description,
            human(e.frames as usize),
            human(e.offset as usize),
            human(e.length as usize)
        );
    }
    let errs = validate(&out.bytes);
    if errs.is_empty() {
        println!("  VALIDATION: PASS");
    } else {
        println!("  VALIDATION: FAIL");
        for e in errs {
            println!("    ! {e}");
        }
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        return usage();
    }

    let run = || -> eyre::Result<()> {
        match argv[0].as_str() {
            "inspect" if argv.len() >= 2 => inspect(&argv[1]),

            "convert" if argv.len() >= 3 => {
                let (opts, _) = parse_opts(&argv[3..]);
                let src = std::fs::metadata(&argv[1])?.len() as usize;
                let demo = load(&argv[1])?;
                let out = transcode(&demo, &opts)?;
                std::fs::write(&argv[2], &out.bytes)?;
                println!("{} -> {}", argv[1], argv[2]);
                report(&out, src);
                Ok(())
            }

            "cut" if argv.len() >= 5 => {
                let (opts, preroll) = parse_opts(&argv[5..]);
                let start: f32 = argv[3].parse()?;
                let end: f32 = argv[4].parse()?;
                let src = std::fs::metadata(&argv[1])?.len() as usize;
                let demo = load(&argv[1])?;
                // `cut` needs a second, Parse-mode copy of the same source to
                // find a valid entity-delta baseline for the cut boundary —
                // see `xash_transcode::cut`'s doc comment.
                let parsed = load_parsed(&argv[1])?;
                let out = cut(&demo, &parsed, start, end, preroll, &opts)?;
                std::fs::write(&argv[2], &out.bytes)?;
                println!("{} [{start}s..{end}s] -> {}", argv[1], argv[2]);
                report(&out, src);
                Ok(())
            }

            "pack" if argv.len() >= 3 => {
                let o = parse_pack_opts(&argv[3..]);
                if o.game_root.as_os_str().is_empty() {
                    return Err(eyre::eyre!(
                        "pack requires --game-root (the folder containing dod/ and valve/)"
                    ));
                }
                let demo = load_parsed(&argv[1])?;
                packer::build(&demo, std::path::Path::new(&argv[2]), &o)
            }

            "validate" if argv.len() >= 2 => {
                let data = std::fs::read(&argv[1])?;
                let errs = validate(&data);
                if errs.is_empty() {
                    println!("PASS — {} would accept this", argv[1]);
                } else {
                    println!("FAIL");
                    for e in errs {
                        println!("  ! {e}");
                    }
                }
                Ok(())
            }

            _ => Err(eyre::eyre!("bad arguments")),
        }
    };

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            usage()
        }
    }
}
