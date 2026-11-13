//! Tallies, over the world's cue table records, the fields the 360's
//! sound engine starts a sound from: the flag byte at 0xc, the float at
//! 0x10, and either the floats at 0x2c and 0x30 (flag 0: the level and
//! pitch, `0x82842E40`, `0x82842DA8`) or the ranges at 0x50..0x54 and
//! 0x5c..0x60 (flag set).
//!
//! ```text
//! cargo run --release -p m2-engine --example recordvol -- "<install dir>"
//! ```
//!
//! Prints counts and numbers only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_engine::Install;
use m2_engine::audio::world_cue_tables;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: recordvol <install dir>")?;
    let install = Install::find(Path::new(&root))?;
    let mut source = install.open_world()?;
    let hashes: Vec<u32> = world_cue_tables(&mut source)
        .iter()
        .map(|t| t.hash)
        .collect();
    // (flag, size) -> count and a few rows.
    let mut tally: BTreeMap<(u8, usize), (usize, Vec<String>)> = BTreeMap::new();
    for hash in hashes {
        let Ok(asset) = source.asset(hash, m2_assets::voice::CUE_TABLE_TYPE) else {
            continue;
        };
        let Some(d) = m2_assets::Ucfx::parse(&asset)
            .ok()
            .and_then(|u| u.top_level(*b"data"))
        else {
            continue;
        };
        let word = |at: usize| {
            d.get(at..at + 4)
                .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let f = |at: usize| f32::from_bits(word(at));
        let n = usize::from(u16::from_le_bytes([d[8], d[9]]));
        let (records, table) = (word(0x10) as usize, word(0x14) as usize);
        let offsets: Vec<usize> = (0..n)
            .map(|i| word(table + 4 * i) as usize + records)
            .collect();
        for (i, &at) in offsets.iter().enumerate() {
            let next = offsets.get(i + 1).copied().unwrap_or(table);
            let size = next.saturating_sub(at);
            if size < 0x40 || at + size > d.len() {
                continue;
            }
            let flag = d[at + 0xc];
            let e = tally.entry((flag, size.min(0x68))).or_default();
            e.0 += 1;
            if e.1.len() < 6 {
                e.1.push(if size >= 0x64 {
                    format!(
                        "0x10 {:.3}; 0x2c {:08x}; 0x4c {:.4} range {:.4}..{:.4}; 0x58 {:.2} range {:.2}..{:.2}",
                        f(at + 0x10),
                        word(at + 0x2c),
                        f(at + 0x4c),
                        f(at + 0x50),
                        f(at + 0x54),
                        f(at + 0x58),
                        f(at + 0x5c),
                        f(at + 0x60)
                    )
                } else {
                    format!(
                        "0x10 {:.3}; 0x2c {:08x} ({:.4}); 0x30 {:08x} ({:.3})",
                        f(at + 0x10),
                        word(at + 0x2c),
                        f(at + 0x2c),
                        word(at + 0x30),
                        f(at + 0x30)
                    )
                });
            }
        }
    }
    for ((flag, size), (n, rows)) in tally {
        println!(
            "flag {flag}, size {size:#x}{}: {n} records",
            if size == 0x68 { "+" } else { "" }
        );
        for r in rows {
            println!("    {r}");
        }
    }
    Ok(())
}
