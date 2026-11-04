//! Prints a data table (an animation-table asset) by its hash: its
//! columns, then each row's cells, names where the template database or
//! a hash of a known word gives them.
//!
//! ```text
//! cargo run --release -p m2-engine --example tabledump -- "<install dir>" <table hash>
//! ```
//!
//! Prints names and numbers only.

use std::path::Path;

use m2_assets::animation_table::{ANIMATION_TABLE_TYPE, AnimationTable};
use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(root), Some(hash)) = (args.first(), args.get(1)) else {
        eprintln!("usage: tabledump <install dir> <table hash>");
        std::process::exit(2);
    };
    let hash = u32::from_str_radix(hash.trim_start_matches("0x"), 16)?;
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let index = source.archive().index();
    let db_hash = index
        .assets()
        .iter()
        .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
        .map(|a| a.name_hash)
        .ok_or("no template database")?;
    let data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&data)?;
    let names = db.names();
    let table = AnimationTable::parse(&source.asset(hash, ANIMATION_TABLE_TYPE)?)?;
    let mut by_hash: std::collections::HashMap<u32, String> = names
        .values()
        .map(|n| (m2_assets::name_hash(n), (*n).to_owned()))
        .collect();
    for w in [
        "Heli", "Car", "Tank", "Boat", "Man", "Driving", "Stopped", "OffRoad", "Helicopter",
        "Always", "Hostile", "Neutral", "Friendly", "Default", "Vehicle", "Infantry", "Air",
    ] {
        by_hash.insert(m2_assets::name_hash(w), w.to_owned());
    }
    let name = |v: u32| {
        names
            .get(&v)
            .map(|n| (*n).to_owned())
            .or_else(|| by_hash.get(&v).cloned())
            .unwrap_or(format!("{v:08x}"))
    };
    println!(
        "{}",
        table
            .columns
            .iter()
            .map(|c| format!("{} ({:?})", c.name, c.kind))
            .collect::<Vec<_>>()
            .join("\t")
    );
    for r in 0..table.rows() {
        let cells: Vec<String> = (0..table.columns.len())
            .map(|c| {
                let list = table.list(r, c);
                if list.is_empty() {
                    table.cell(r, c).map_or("-".into(), name)
                } else {
                    list.iter().map(|&v| name(v)).collect::<Vec<_>>().join(",")
                }
            })
            .collect();
        println!("{}", cells.join("\t"));
    }
    Ok(())
}
