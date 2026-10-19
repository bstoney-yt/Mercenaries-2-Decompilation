//! Counts the people templates (those with an `AiAttributes` `Accuracy`)
//! by the first gun their `EquipmentLink`s hand them: one that fires
//! while the trigger is held (`FireType` 0, what play gives them), one
//! shot a pull that fires rounds (a pistol, the shotgun, a sniper rifle),
//! a launcher, or none (play hands those the Assault Rifle).
//!
//! ```text
//! cargo run --release -p m2-engine --example soldierguns -- "<install dir>"
//! ```
//!
//! Prints numbers and template names only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::Install;
use m2_engine::weapons::Flight;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: soldierguns <install dir>")?;
    let install = Install::find(Path::new(&root))?;
    let mut source = install.open_world()?;
    let db_hash = {
        let index = source.archive().index();
        index
            .assets()
            .iter()
            .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
            .map(|a| a.name_hash)
            .ok_or("no template database")?
    };
    let data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&data)?;
    let arms = m2_engine::weapons::read(&db);
    let guns = m2_engine::tuning::hand_guns(&db);
    let equipment = m2_engine::tuning::equipment(&db);
    let ai = m2_engine::ai::read(&db);
    let mut kinds: BTreeMap<&str, u32> = BTreeMap::new();
    let mut semi: BTreeMap<String, u32> = BTreeMap::new();
    for id in ai.keys() {
        let list = equipment.get(id).map_or(&[][..], Vec::as_slice);
        let kind = if list.iter().any(|t| guns.contains_key(t)) {
            "held trigger (played as its own)"
        } else if let Some(w) = list
            .iter()
            .filter_map(|t| arms.weapons.get(t))
            .find(|w| w.flight() == Flight::Round)
        {
            *semi.entry(w.name.clone()).or_default() += 1;
            "one shot a pull, rounds (played as the Assault Rifle)"
        } else if list.iter().any(|t| arms.weapons.contains_key(t)) {
            "launcher only (played as the Assault Rifle)"
        } else {
            "no gun (played as the Assault Rifle)"
        };
        *kinds.entry(kind).or_default() += 1;
    }
    println!("people with Accuracy: {}", ai.len());
    for (k, n) in &kinds {
        println!("{n}\t{k}");
    }
    for (g, n) in &semi {
        println!("  {n}\t{g}");
    }
    Ok(())
}
