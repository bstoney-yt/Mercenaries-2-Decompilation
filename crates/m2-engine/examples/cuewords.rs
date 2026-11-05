//! Finds where cue keys are kept: every asset of `vz.wad` (and the other
//! archives) whose bytes hold one of the given keys as a little-endian
//! word, with the asset's type, its hash, the word's offset and the eight
//! words around it. A survey for cues the world's cue tables don't
//! resolve (the vehicles' engine start and stop cues).
//!
//! ```text
//! cargo run --release -p m2-engine --example cuewords -- "<install dir>" <key hex | name:NAME>...
//! ```
//!
//! Prints hashes, offsets and words only.

use std::path::Path;

use m2_assets::BlockContents;
use m2_engine::{AssetSource, Install};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = args
        .first()
        .ok_or("usage: cuewords <install dir> <key>...")?;
    let keys: Vec<u32> = args[1..]
        .iter()
        .filter(|k| !k.starts_with("str:"))
        .filter_map(|k| match k.strip_prefix("name:") {
            // A name, hashed as the archives hash names.
            Some(n) => {
                let h = m2_assets::name_hash(n);
                println!("{n} = {h:08x}");
                Some(h)
            }
            None => u32::from_str_radix(k, 16).ok(),
        })
        .collect();
    // Text to find as bytes, any case (`str:heathaze`).
    let texts: Vec<Vec<u8>> = args[1..]
        .iter()
        .filter_map(|k| k.strip_prefix("str:"))
        .map(|t| t.to_ascii_lowercase().into_bytes())
        .collect();
    let install = Install::find(Path::new(root))?;
    for wad in ["vz.wad", "shell.wad", "English.wad", "Loading.wad"] {
        let Ok(f) = std::fs::File::open(install.data.join(wad)) else {
            continue;
        };
        let Ok(mut source) = AssetSource::new(std::io::BufReader::new(f)) else {
            continue;
        };
        let blocks = source.archive().index().blocks().len();
        for b in 0..blocks {
            let Ok(decoded) = source.block(b) else {
                continue;
            };
            let Ok(contents) = BlockContents::parse(&decoded) else {
                continue;
            };
            for (e, data) in contents.iter() {
                for t in &texts {
                    let lower = data.to_ascii_lowercase();
                    for (at, w) in lower.windows(t.len()).enumerate() {
                        if w != t.as_slice() {
                            continue;
                        }
                        let from = at.saturating_sub(48);
                        let to = (at + t.len() + 48).min(data.len());
                        let text: String = data[from..to]
                            .iter()
                            .map(|&b| {
                                if (0x20..0x7f).contains(&b) {
                                    b as char
                                } else {
                                    '.'
                                }
                            })
                            .collect();
                        println!(
                            "{wad} block {b} type {:08x} asset {:08x} @0x{at:x}: {text}",
                            e.type_hash, e.name_hash
                        );
                    }
                }
                for at in (0..data.len().saturating_sub(3)).step_by(4) {
                    let w =
                        u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
                    if !keys.contains(&w) {
                        continue;
                    }
                    let around: Vec<String> = (at.saturating_sub(16)
                        ..(at
                            + 4 * std::env::var("AFTER")
                                .ok()
                                .and_then(|v| v.parse::<usize>().ok())
                                .unwrap_or(5))
                        .min(data.len()))
                        .step_by(4)
                        .filter(|&i| i + 4 <= data.len())
                        .map(|i| {
                            format!(
                                "{:08x}",
                                u32::from_le_bytes([
                                    data[i],
                                    data[i + 1],
                                    data[i + 2],
                                    data[i + 3]
                                ])
                            )
                        })
                        .collect();
                    println!(
                        "{wad} block {b} type {:08x} asset {:08x} len {} @0x{at:x}: {w:08x} [{}]",
                        e.type_hash,
                        e.name_hash,
                        data.len(),
                        around.join(" ")
                    );
                }
            }
        }
    }
    Ok(())
}
