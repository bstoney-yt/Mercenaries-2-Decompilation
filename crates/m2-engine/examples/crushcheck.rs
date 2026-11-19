//! Checks the crush step's data from your own installation
//! ([`m2_engine::crush`]): which templates carry `Crusher`, and which
//! objects' broken pieces become one. Prints counts, names and PASS or
//! FAIL.
//!
//! ```text
//! cargo run --release -p m2-engine --example crushcheck -- <vz.wad>
//! ```

use std::fs::File;
use std::io::BufReader;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::{BlockContents, WadArchive, name_hash};
use m2_engine::crush::{crusher_modifiers, piece_crushers};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: crushcheck <vz.wad>")?;
    let mut wad = WadArchive::open(BufReader::new(File::open(path)?))?;
    for block in 0..wad.index().blocks().len() {
        let Ok(decoded) = wad.read_block(block) else {
            continue;
        };
        let Ok(contents) = BlockContents::parse(&decoded) else {
            continue;
        };
        for (entry, asset) in contents.iter() {
            if entry.type_hash != TEMPLATE_DB_TYPE {
                continue;
            }
            let db = Layer::parse(asset)?;
            let names = db.names();
            let crushers = crusher_modifiers(&db);
            let pieces = piece_crushers(&db, &names);
            let params: usize = pieces.values().map(Vec::len).sum();
            let ok_crushers = crushers.len() == 16;
            println!(
                "crush-check: {} templates carry Crusher: {}",
                crushers.len(),
                if ok_crushers { "PASS" } else { "FAIL" }
            );
            let by_name = |n: &str| names.iter().find(|(_, v)| **v == n).map(|(k, _)| *k);
            let stack = by_name("_industrial_bld_smokestack")
                .and_then(|id| pieces.get(&id))
                .and_then(|p| {
                    p.iter().find(|(_, t, _)| {
                        names.get(t).copied() == Some("CrushingDebrisMedTemplate")
                    })
                })
                .map(|&(_, _, m)| m);
            let ok_stack = stack == Some(1000.0);
            println!(
                "crush-check: {} object templates' pieces crush ({params} parameters); \
                 _industrial_bld_smokestack's CrushingDebrisMedTemplate {:?}: {}",
                pieces.len(),
                stack,
                if ok_stack { "PASS" } else { "FAIL" }
            );
            let prop = pieces
                .values()
                .flatten()
                .filter(|(n, _, _)| *n == name_hash("PropTemplate"))
                .count();
            let debris = pieces
                .values()
                .flatten()
                .filter(|(n, _, _)| *n == name_hash("DebrisTemplate"))
                .count();
            let mut other: Vec<u32> = pieces
                .values()
                .flatten()
                .map(|(n, _, _)| *n)
                .filter(|n| *n != name_hash("PropTemplate") && *n != name_hash("DebrisTemplate"))
                .collect();
            other.sort_unstable();
            other.dedup();
            println!(
                "crush-check: by parameter: PropTemplate {prop}, DebrisTemplate {debris}, other {other:08x?}"
            );
            println!(
                "crush-check: {}",
                if ok_crushers && ok_stack {
                    "PASS"
                } else {
                    "FAIL"
                }
            );
            return Ok(());
        }
    }
    println!("crush-check: FAIL no template database");
    Ok(())
}
