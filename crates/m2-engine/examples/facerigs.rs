//! Lists each person template's face rig (its `HumanAnimationSystem`
//! record's `facefxactor` word, [`m2_engine::tuning::face_actors`]) and the
//! face rigs in the archive no template names.
//!
//! ```text
//! cargo run --release -p m2-engine --example facerigs -- "<install dir>"
//! ```
//!
//! Prints template names and hashes only.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use m2_assets::facefx::ACTOR_TYPE;
use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: facerigs <install dir>")?;
    let install = Install::find(Path::new(&root))?;
    let mut source = install.open_world()?;
    let (db_hash, rigs) = {
        let index = source.archive().index();
        let db = index
            .assets()
            .iter()
            .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
            .map(|a| a.name_hash)
            .ok_or("no template database")?;
        let rigs: BTreeSet<u32> = index
            .assets()
            .iter()
            .filter(|a| index.type_hash(a) == ACTOR_TYPE)
            .map(|a| a.name_hash)
            .collect();
        (db, rigs)
    };
    let data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&data)?;
    let names = db.names();
    let mut by_rig: BTreeMap<u32, Vec<&str>> = BTreeMap::new();
    for (id, rig) in m2_engine::tuning::face_actors(&db) {
        by_rig
            .entry(rig)
            .or_default()
            .push(names.get(&id).copied().unwrap_or("?"));
    }
    for (rig, mut templates) in by_rig {
        templates.sort_unstable();
        let there = if rigs.contains(&rig) {
            ""
        } else {
            " (no such asset)"
        };
        println!("rig {rig:08x}{there}: {}", templates.join(", "));
    }
    let named: BTreeSet<u32> = m2_engine::tuning::face_actors(&db).into_values().collect();
    let unnamed: Vec<String> = rigs
        .difference(&named)
        .map(|h| format!("{h:08x}"))
        .collect();
    println!("rigs no template names: {}", unnamed.join(" "));
    Ok(())
}
