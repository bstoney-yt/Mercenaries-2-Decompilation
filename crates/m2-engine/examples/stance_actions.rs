//! Lists the stance and action table's rows for the stances given: each
//! row's stance, action, other filters and animation handles, as name
//! hashes (names where `names` lists them), for working out what a stance
//! plays. Prints hashes and numbers only.
//!
//! ```text
//! cargo run --release -p m2-engine --example stance_actions -- "<install dir>" <stance>... [-- <name>...]
//! ```

use std::collections::HashMap;
use std::path::Path;

use m2_assets::animation_table::{self as table, AnimationTable};
use m2_assets::name_hash;
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (root, rest) = args
        .split_first()
        .ok_or("usage: stance_actions <install> <stance>...")?;
    let split = rest.iter().position(|a| a == "--").unwrap_or(rest.len());
    let (stances, names) = (&rest[..split], rest.get(split + 1..).unwrap_or(&[]));
    let known: HashMap<u32, &str> = stances
        .iter()
        .chain(names)
        .map(|n| (name_hash(n), n.as_str()))
        .collect();
    let show = |h: u32| known.get(&h).map_or(format!("{h:08x}"), |n| n.to_string());
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let t = AnimationTable::parse(&source.asset(table::ACTIONS, table::ANIMATION_TABLE_TYPE)?)?;
    let cols = [
        "Stance",
        "Action",
        "AimState",
        "Tandem",
        "Seat",
        "Target",
        "ActionDirection",
        "DamageDirection",
    ]
    .map(|c| t.column(c));
    let handles = t.column("AnimationHandles");
    let want: Vec<u32> = stances.iter().map(|s| name_hash(s)).collect();
    for row in 0..t.rows() {
        let stance = cols[0].and_then(|c| t.cell(row, c));
        if !stance.is_some_and(|s| want.contains(&s)) {
            continue;
        }
        let cells: Vec<String> = cols
            .iter()
            .map(|c| c.and_then(|c| t.cell(row, c)).map_or("-".into(), show))
            .collect();
        let h = handles
            .and_then(|c| t.cell(row, c))
            .map_or("-".into(), show);
        println!("{} -> {h}", cells.join(" "));
    }
    // The on-foot table's rows for each handle named `#<hex>`.
    let c = AnimationTable::parse(
        &source.asset(table::CHARACTER_ANIMATIONS, table::ANIMATION_TABLE_TYPE)?,
    )?;
    let col = |n: &str| c.column(n);
    for h in names.iter().filter_map(|n| n.strip_prefix('#')) {
        let h = u32::from_str_radix(h, 16)?;
        for row in 0..c.rows() {
            if col(table::HANDLE).and_then(|k| c.cell(row, k)) != Some(h) {
                continue;
            }
            let cell = |n: &str| col(n).and_then(|k| c.cell(row, k)).map_or("-".into(), show);
            let clips: Vec<String> = col(table::ANIMATION)
                .map(|k| c.list(row, k).iter().map(|v| format!("{v:08x}")).collect())
                .unwrap_or_default();
            println!(
                "handle {h:08x}: {} {} {} -> {}",
                cell(table::GENDER),
                cell(table::CHARACTER_NAME),
                cell(table::PRIMARY_EQUIPMENT_CLASS),
                clips.join(",")
            );
        }
    }
    if names.iter().any(|n| n == "!handles") {
        let mut all: Vec<u32> = (0..c.rows())
            .filter_map(|r| col(table::HANDLE).and_then(|k| c.cell(r, k)))
            .collect();
        all.sort_unstable();
        all.dedup();
        for h in all {
            println!("handle {}", show(h));
        }
    }
    // Every animation table that holds a word named `@<hex>`.
    let want: Vec<u32> = names
        .iter()
        .filter_map(|n| n.strip_prefix('@'))
        .filter_map(|h| u32::from_str_radix(h, 16).ok())
        .collect();
    if !want.is_empty() {
        let tables: Vec<u32> = {
            let index = source.archive().index();
            index
                .assets()
                .iter()
                .filter(|a| index.type_hash(a) == table::ANIMATION_TABLE_TYPE)
                .map(|a| a.name_hash)
                .collect()
        };
        for t in tables {
            let data = source.asset(t, table::ANIMATION_TABLE_TYPE)?;
            for w in &want {
                let hits = data
                    .chunks_exact(4)
                    .filter(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]) == *w)
                    .count();
                if hits > 0 {
                    println!("table {t:08x} ({} bytes) holds {w:08x} x{hits}", data.len());
                }
            }
        }
    }
    Ok(())
}
