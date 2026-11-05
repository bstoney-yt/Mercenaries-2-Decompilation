//! Counts the `Sounds` table's rows for each event hash given, with how
//! many sound sets answer it and a few of those sets: a survey for which
//! of the sound system's events the data gives cues to.
//!
//! ```text
//! cargo run --release -p m2-engine --example eventrows -- "<install dir>" <event hex>...
//! ```
//!
//! Prints hashes and counts only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::animation_table::{ANIMATION_TABLE_TYPE, AnimationTable};
use m2_engine::Install;
use m2_engine::gun_sounds::SOUNDS_TABLE;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = args
        .first()
        .ok_or("usage: eventrows <install dir> <event hex>...")?;
    let events: Vec<u32> = args[1..]
        .iter()
        .filter_map(|a| u32::from_str_radix(a.trim_start_matches("0x"), 16).ok())
        .collect();
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let table = AnimationTable::parse(&source.asset(SOUNDS_TABLE, ANIMATION_TABLE_TYPE)?)?;
    let cols = [
        table.column("SoundSet").ok_or("col")?,
        table.column("EventHandle").ok_or("col")?,
    ];
    let mut by_event: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for r in 0..table.rows() {
        if let (Some(s), Some(e)) = (table.cell(r, cols[0]), table.cell(r, cols[1]))
            && events.contains(&e)
        {
            by_event.entry(e).or_default().push(s);
        }
    }
    for e in &events {
        let sets = by_event.get(e).cloned().unwrap_or_default();
        let mut distinct = sets.clone();
        distinct.sort_unstable();
        distinct.dedup();
        let some: Vec<String> = distinct
            .iter()
            .take(6)
            .map(|s| format!("{s:08x}"))
            .collect();
        println!(
            "event {e:08x}: {} rows, {} sets ({})",
            sets.len(),
            distinct.len(),
            some.join(" ")
        );
    }
    Ok(())
}
