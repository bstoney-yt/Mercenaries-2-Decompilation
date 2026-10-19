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
    // Per armour class (none, light, medium, heavy), the people whose gun
    // (their own, else the Assault Rifle that stands in) rates it 0 in
    // its `WeaponHint`: they hold their fire at her in such a vehicle.
    let rifle = arms
        .weapons
        .values()
        .find(|w| w.name == "Assault Rifle")
        .and_then(|w| w.hint);
    let mut no_use = [0u32; 4];
    let mut launchers: BTreeMap<String, u32> = BTreeMap::new();
    let mut kinds: BTreeMap<&str, u32> = BTreeMap::new();
    let mut semi: BTreeMap<String, u32> = BTreeMap::new();
    for id in ai.keys() {
        let list = equipment.get(id).map_or(&[][..], Vec::as_slice);
        if let Some(l) = list
            .iter()
            .filter_map(|t| arms.weapons.get(t))
            .find(|w| m2_engine::weapons::NpcLauncher::of(w).is_some())
        {
            let h = l.hint.map_or([0; 4], |h| h.against);
            *launchers
                .entry(format!("{} (rates {h:?})", l.name))
                .or_default() += 1;
        }
        let hint = list
            .iter()
            .filter_map(|t| arms.weapons.get(t))
            .find(|w| !w.semi || w.flight() == Flight::Round)
            .and_then(|w| w.hint)
            .or(rifle);
        if let Some(h) = hint {
            for (k, n) in no_use.iter_mut().enumerate() {
                *n += u32::from(h.against[k] == 0);
            }
        }
        let kind = if list.iter().any(|t| guns.contains_key(t)) {
            "held trigger (played as its own)"
        } else if let Some(w) = list
            .iter()
            .filter_map(|t| arms.weapons.get(t))
            .find(|w| w.flight() == Flight::Round)
        {
            *semi
                .entry(format!(
                    "{} ({:.0} a minute, {} a round, WeaponHint {})",
                    w.name,
                    w.rate_of_fire,
                    w.ammo.damage,
                    if w.hint.is_some() { "yes" } else { "no" }
                ))
                .or_default() += 1;
            "one shot a pull, rounds (played as its own since the gap-list sweep; the Assault Rifle before)"
        } else if list.iter().any(|t| arms.weapons.contains_key(t)) {
            "launcher only (played as the Assault Rifle)"
        } else {
            "no gun (played as the Assault Rifle)"
        };
        *kinds.entry(kind).or_default() += 1;
    }
    println!("people with Accuracy: {}", ai.len());
    println!("gun rated 0 against armour none, light, medium, heavy: {no_use:?}");
    for (k, n) in &kinds {
        println!("{n}\t{k}");
    }
    println!(
        "carrying a launcher as well: {}",
        launchers.values().sum::<u32>()
    );
    for (l, n) in &launchers {
        println!("  {n}	{l}");
    }
    for (g, n) in &semi {
        println!("  {n}\t{g}");
    }
    Ok(())
}
