//! Tallies the distance block of the world's cue table records: how many
//! are placed in the world (the byte at 0x14), and the powers their fades
//! follow (the float at 0x24), with the near and far distances of a few
//! of each. A check that the block the 360's sound engine reads
//! (`0x828424A0`) sits where `m2_assets::voice::Record` takes it to.
//!
//! ```text
//! cargo run --release -p m2-engine --example recorddist -- "<install dir>"
//! ```
//!
//! Prints counts and numbers only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_engine::Install;
use m2_engine::audio::world_cue_tables;

/// How many records, and a few of their (near, far).
type Tally = (usize, Vec<(f32, f32)>);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: recorddist <install dir>")?;
    let install = Install::find(Path::new(&root))?;
    let mut source = install.open_world()?;
    let tables = world_cue_tables(&mut source);
    // (placed, the power's bits) -> count, and a few (near, far).
    let mut tally: BTreeMap<(bool, u32), Tally> = BTreeMap::new();
    let mut records = 0;
    for r in tables.iter().flat_map(|t| &t.records) {
        if r.cue == 0 {
            continue;
        }
        records += 1;
        let e = tally
            .entry((r.positional, r.exponent.to_bits()))
            .or_default();
        e.0 += 1;
        if e.1.len() < 4 {
            e.1.push((r.min_distance, r.max_distance));
        }
    }
    println!("{} tables, {records} records", tables.len());
    for ((placed, bits), (n, some)) in tally {
        println!(
            "placed {placed}, power {} ({bits:08x}): {n} records, near and far {some:?}",
            f32::from_bits(bits)
        );
    }
    Ok(())
}
