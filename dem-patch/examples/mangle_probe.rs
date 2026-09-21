//! Feeds structurally-plausible-but-wrong demos to the parser and records how
//! it fails: clean `Err`, catchable panic, or an abort the host process cannot
//! survive (#225).
//!
//! Each candidate is parsed in a **child process**, because the failure this
//! exists to find is a stack overflow -- `STATUS_STACK_BUFFER_OVERRUN`,
//! `0xc0000409` on Windows -- which unwinds nothing and which
//! `std::panic::catch_unwind` does not catch. A crash therefore costs one
//! child, and the parent keeps going and reports the seed that produced it.
//!
//!     cargo run -p dem --release --example mangle_probe -- <demo.dem> [count]
//!     cargo run -p dem --release --example mangle_probe -- --one <mutant.bin>
//!     cargo run -p dem --release --example mangle_probe -- --roundtrip <dir>
//!
//! `--roundtrip` is the other half of the check: hardening the parser must not
//! change what it reads out of a *valid* demo. It parses and re-serialises
//! every demo under a directory and prints an FNV-1a of the result, so the
//! same run before and after a change can be diffed.
//!
//! The mutation strategies are the ones that produced #225's original crash:
//! splice a valid header/signon onto a tail slice taken at an arbitrary byte
//! offset, truncate mid-frame, and flip bytes in the frame headers (where a
//! length or a frame type lives) rather than uniformly across the file.

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

/// xorshift64*, so a reported seed reproduces its mutant exactly.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}

const HEADER_SIZE: usize = 544;

#[derive(Clone, Copy, Debug)]
enum Strategy {
    /// Header + signon, then the file resumes from an unrelated offset.
    Splice,
    /// Cut the file mid-frame.
    Truncate,
    /// Flip a handful of bytes past the header.
    Flip,
    /// Header + a tail slice, with the directory offset left pointing past EOF.
    DanglingDirectory,
}

fn mutate(src: &[u8], seed: u64) -> (Strategy, Vec<u8>) {
    let mut rng = Rng(seed | 1);
    let strategy = match rng.below(4) {
        0 => Strategy::Splice,
        1 => Strategy::Truncate,
        2 => Strategy::Flip,
        _ => Strategy::DanglingDirectory,
    };
    let body = src.len().saturating_sub(HEADER_SIZE);
    let out = match strategy {
        Strategy::Splice => {
            let keep = HEADER_SIZE + rng.below(body / 4);
            let resume = HEADER_SIZE + rng.below(body);
            let mut v = src[..keep.min(src.len())].to_vec();
            v.extend_from_slice(&src[resume.min(src.len())..]);
            v
        }
        Strategy::Truncate => {
            let at = HEADER_SIZE + rng.below(body);
            src[..at.min(src.len())].to_vec()
        }
        Strategy::Flip => {
            let mut v = src.to_vec();
            for _ in 0..1 + rng.below(8) {
                let at = HEADER_SIZE + rng.below(body);
                if at < v.len() {
                    v[at] = (rng.next() & 0xff) as u8;
                }
            }
            v
        }
        Strategy::DanglingDirectory => {
            let at = HEADER_SIZE + rng.below(body);
            let mut v = src[..at.min(src.len())].to_vec();
            // The directory offset lives at 540 and is what `parse_directory`
            // seeks to; point it somewhere inside the truncated body.
            if v.len() > HEADER_SIZE {
                let bogus = (HEADER_SIZE + rng.below(v.len() - HEADER_SIZE)) as u32;
                v[540..544].copy_from_slice(&bogus.to_le_bytes());
            }
            v
        }
    };
    (strategy, out)
}

fn parse_one(path: &str) -> ! {
    let bytes = std::fs::read(path).expect("child: read mutant");
    let res = std::panic::catch_unwind(|| dem::open_demo_from_bytes(&bytes));
    match res {
        Ok(Ok(_)) => {
            println!("OK");
            std::process::exit(0)
        }
        Ok(Err(e)) => {
            println!("ERR {}", first_line(&e.to_string()));
            std::process::exit(10)
        }
        Err(_) => {
            println!("PANIC");
            std::process::exit(11)
        }
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

/// Parses and re-serialises every demo under `dir`, printing
/// `<fnv1a> <bytes> <path>` per file. Diff two runs to prove a parser change
/// is invisible to valid input.
fn roundtrip_dir(dir: &str) {
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut stack = vec![PathBuf::from(dir)];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|s| s.to_str()) == Some("dem") {
                paths.push(p);
            }
        }
    }
    paths.sort();

    let (mut done, mut refused) = (0u64, 0u64);
    let mut total_bytes = 0u64;
    for p in &paths {
        match dem::open_demo(p) {
            Ok(demo) => {
                let bytes = demo.write_to_bytes();
                println!("{:016x} {:>12} {}", fnv1a(&bytes), bytes.len(), p.display());
                total_bytes += bytes.len() as u64;
                done += 1;
            }
            Err(e) => {
                println!("PARSE-ERR {} -- {}", p.display(), first_line(&e.to_string()));
                refused += 1;
            }
        }
    }
    eprintln!(
        "round-tripped {done} demos ({:.2} GB), refused {refused}",
        total_bytes as f64 / 1e9
    );
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().take(120).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1] == "--one" {
        parse_one(&args[2]);
    }
    if args.len() >= 3 && args[1] == "--roundtrip" {
        roundtrip_dir(&args[2]);
        return;
    }

    let Some(src_path) = args.get(1) else {
        eprintln!("usage: mangle_probe <demo.dem> [count]   |   mangle_probe --one <mutant>");
        std::process::exit(2);
    };
    let count: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(200);

    let src = std::fs::read(src_path).expect("read source demo");
    println!("source {} ({} bytes), {} mutants", src_path, src.len(), count);

    let exe = std::env::current_exe().expect("current_exe");
    let scratch = std::env::temp_dir().join(format!("dem_mangle_{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("scratch dir");

    let (mut ok, mut err, mut panicked) = (0u64, 0u64, 0u64);
    let mut sites: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    let mut aborts: Vec<(u64, Strategy, i32)> = Vec::new();
    let mut kept: Vec<PathBuf> = Vec::new();

    for seed in 1..=count {
        let (strategy, bytes) = mutate(&src, seed);
        let mutant = scratch.join(format!("m{seed}.dem"));
        std::fs::write(&mutant, &bytes).expect("write mutant");

        let out = Command::new(&exe)
            .arg("--one")
            .arg(&mutant)
            .output()
            .expect("spawn child");
        let code = out.status.code().unwrap_or(-1);
        match code {
            0 => ok += 1,
            10 => err += 1,
            11 => panicked += 1,
            other => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let site = stderr
                    .lines()
                    .find(|l| l.contains("panicked at"))
                    .map(|l| l.trim().to_string())
                    .unwrap_or_else(|| "<no panic message -- genuine stack overflow?>".into());
                let detail = stderr
                    .lines()
                    .skip_while(|l| !l.contains("panicked at"))
                    .nth(1)
                    .map(|l| l.trim().to_string())
                    .unwrap_or_default();
                *sites.entry(format!("{site} | {detail}")).or_insert(0u64) += 1;
                aborts.push((seed, strategy, other));
                if kept.len() < 5 {
                    let keep = scratch.join(format!("ABORT_seed{seed}.dem"));
                    let _ = std::fs::write(&keep, &bytes);
                    kept.push(keep);
                }
                if aborts.len() <= 5 {
                    println!(
                        "  ABORT seed {seed} ({strategy:?}) exit {other:#x} -- {} bytes",
                        bytes.len()
                    );
                }
            }
        }
        if code != 0 && code != 10 && code != 11 {
            // keep the mutant
        } else {
            let _ = std::fs::remove_file(&mutant);
        }
        if seed % 25 == 0 {
            print!(".");
            let _ = std::io::stdout().flush();
        }
    }

    println!();
    println!("ok {ok}  err {err}  caught-panic {panicked}  ABORTS {}", aborts.len());
    if !sites.is_empty() {
        println!("
distinct abort sites:");
        let mut rows: Vec<_> = sites.iter().collect();
        rows.sort_by(|a, b| b.1.cmp(a.1));
        for (site, n) in rows {
            println!("  {n:4}x  {site}");
        }
    }
    for k in kept.iter().take(5) {
        println!("kept {}", k.display());
    }
    if aborts.is_empty() {
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
