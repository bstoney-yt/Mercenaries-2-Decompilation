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
    Ok(())
}
