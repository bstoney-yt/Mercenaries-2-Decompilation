//! Counts what vehicles meeting things are heard as by the sound table
//! (`KeyTable::contact_cue`): each vehicle template's `SoundKey` against
//! the ground's surface words, the placed objects' templates' and the
//! other vehicles', damaging and not; how many of those keys are the
//! table's own (the rest look up as its first row or column).
//!
//! ```text
//! cargo run --release -p m2-engine --example contactcues -- "<install dir>"
//! ```
//!
//! Prints counts and keys only.

use std::collections::BTreeSet;
use std::path::Path;

use m2_assets::name_hash;
use m2_engine::Install;
use m2_engine::world::WorldIndex;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(root) = std::env::args().nth(1) else {
        return Err("usage: contactcues <install dir>".into());
    };
    let install = Install::find(Path::new(&root))?;
    let mut source = install.open_world()?;
    let world = WorldIndex::build(&mut source)?;
    let db = &world.template_db;
    let Some(table) = db.sound_table.as_ref() else {
        return Err("no sound table".into());
    };
    let vehicles: BTreeSet<u32> = db
        .infos
        .iter()
        .filter(|(_, i)| i.vehicle.is_some())
        .filter_map(|(t, _)| db.sound_keys.get(t).copied())
        .collect();
    let unkeyed = db
        .infos
        .iter()
        .filter(|(t, i)| i.vehicle.is_some() && !db.sound_keys.contains_key(t))
        .count();
    let ground: BTreeSet<u32> = m2_engine::surface::words().map(name_hash).collect();
    let objects: BTreeSet<u32> = world
        .objects
        .iter()
        .filter_map(|o| db.sound_keys.get(&o.template).copied())
        .collect();
    println!(
        "vehicle SoundKeys: {} ({} the table's own); vehicle templates with none: {unkeyed}",
        vehicles.len(),
        vehicles.iter().filter(|&&k| table.has(k)).count()
    );
    for (what, others) in [
        ("ground words", &ground),
        ("placed objects' keys", &objects),
        ("vehicles' keys", &vehicles),
    ] {
        let own = others.iter().filter(|&&k| table.has(k)).count();
        for damaging in [false, true] {
            let pairs = vehicles.len() * others.len();
            let heard = vehicles
                .iter()
                .flat_map(|&v| others.iter().map(move |&o| (v, o)))
                .filter(|&(v, o)| table.contact_cue(v, o, damaging).is_some())
                .count();
            let cues: BTreeSet<u32> = vehicles
                .iter()
                .flat_map(|&v| others.iter().map(move |&o| (v, o)))
                .filter_map(|(v, o)| table.contact_cue(v, o, damaging))
                .collect();
            println!(
                "{what}: {} ({own} the table's own); {} pairs: {heard} with a cue ({} cues), damaging {damaging}",
                others.len(),
                pairs,
                cues.len()
            );
        }
    }
    // Rounds striking things (0x8235E190): the rounds' keys (each gun's
    // ammunition's template's) against people's, vehicles', objects' and
    // the ground's.
    let rounds: BTreeSet<u32> = db
        .leavings
        .ammo
        .values()
        .filter_map(|a| db.sound_keys.get(a).copied())
        .collect();
    let people: BTreeSet<u32> = db
        .infos
        .iter()
        .filter(|(_, i)| i.person)
        .filter_map(|(t, _)| db.sound_keys.get(t).copied())
        .collect();
    println!(
        "round SoundKeys: {} ({} the table's own); people's: {}",
        rounds.len(),
        rounds.iter().filter(|&&k| table.has(k)).count(),
        people.len()
    );
    for (what, struck) in [
        ("people", &people),
        ("vehicles", &vehicles),
        ("placed objects", &objects),
        ("ground words", &ground),
    ] {
        for damaging in [false, true] {
            let pairs: Vec<(u32, u32)> = struck
                .iter()
                .flat_map(|&s| rounds.iter().map(move |&r| (s, r)))
                .collect();
            let heard = pairs
                .iter()
                .filter(|&&(s, r)| table.contact_cue(s, r, damaging).is_some())
                .count();
            println!(
                "rounds on {what}: {} pairs: {heard} with a cue, damaging {damaging}",
                pairs.len()
            );
        }
    }
    Ok(())
}
