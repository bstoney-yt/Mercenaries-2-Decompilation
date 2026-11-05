//! Finds where cue keys are kept: every asset of `vz.wad` (and the other
//! archives) whose bytes hold one of the given keys as a little-endian
//! word, with the asset's type, its hash, the word's offset and the eight
//! words around it. A survey for cues the world's cue tables don't
//! resolve (the vehicles' engine start and stop cues).
//!
//! ```text
//! cargo run --release -p m2-engine --example cuewords -- "<install dir>" <key hex>...
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
        .filter_map(|k| u32::from_str_radix(k, 16).ok())
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
