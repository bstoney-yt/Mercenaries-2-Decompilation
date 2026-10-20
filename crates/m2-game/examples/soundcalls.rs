//! Lists every call the scripts in a `.wad` from your own installation make
//! to the sound library (`Sound.*`), with its arguments, and the scripts'
//! string constants that name reverb or music regions: the survey the
//! score's regions and the reverb presets are built from.
//!
//! ```text
//! cargo run -p m2-game --example soundcalls -- <vz.wad> [filter]
//! ```

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;

use m2_assets::lua::{Arg, Chunk, SCRIPT_TYPE};
use m2_assets::{BlockContents, WadArchive};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn arg_text(a: &Arg) -> String {
    match a {
        Arg::Str(s) => format!("{s:?}"),
        Arg::Number(n) => n.to_string(),
        Arg::Bool => "bool".into(),
        Arg::Nil => "nil".into(),
        Arg::Table => "{..}".into(),
        Arg::Function => "fn".into(),
        Arg::Global(g) => g.clone(),
        Arg::Other => "_".into(),
    }
}

fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let [path, rest @ ..] = args else {
        return Err("usage: soundcalls <archive.wad> [filter]".into());
    };
    let filter = rest.first().map(|f| f.to_ascii_lowercase());
    let mut wad = WadArchive::open(BufReader::new(File::open(path)?))?;
    let blocks = wad.index().blocks().len();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut strings: BTreeMap<String, usize> = BTreeMap::new();
    for block in 0..blocks {
        let decoded = wad.read_block(block)?;
        let Ok(contents) = BlockContents::parse(&decoded) else {
            continue;
        };
        for (entry, data) in contents.iter() {
            if entry.type_hash != SCRIPT_TYPE {
                continue;
            }
            let Ok(chunk) = Chunk::from_asset(data) else {
                continue;
            };
            let source = chunk.main.source.clone().unwrap_or_default();
            if let Some(f) = filter.as_ref().and_then(|f| f.strip_prefix("list:"))
                && (f == format!("{:08x}", entry.name_hash) || source.contains(f))
            {
                println!(
                    "== {:08x} {source}\n{}",
                    entry.name_hash,
                    chunk.main.listing()
                );
            }
            for f in chunk.main.all() {
                for call in f.calls() {
                    if !call.callee.starts_with("Sound.") {
                        continue;
                    }
                    *counts.entry(call.callee.clone()).or_default() += 1;
                    let line = format!(
                        "{:08x} {source} {}({})",
                        entry.name_hash,
                        call.callee,
                        call.args
                            .as_ref()
                            .map(|a| a.iter().map(arg_text).collect::<Vec<_>>().join(", "))
                            .unwrap_or_else(|| "...".into())
                    );
                    if filter
                        .as_ref()
                        .is_none_or(|f| line.to_ascii_lowercase().contains(f))
                    {
                        println!("{line}");
                    }
                }
            }
            for s in chunk.main.strings() {
                let l = s.to_ascii_lowercase();
                if l.contains("reverb") || l.contains("region") || l.contains("music") {
                    *strings.entry(s.to_owned()).or_default() += 1;
                }
            }
        }
    }
    println!("\ncallees:");
    for (c, n) in &counts {
        println!("  {n:5} {c}");
    }
    println!("\nstrings:");
    for (s, n) in &strings {
        println!("  {n:5} {s:?}");
    }
    Ok(())
}
