//! Resolves every gun firing and reload cue the `Sounds` table names
//! ([`m2_engine::gun_sounds::GunCues::firing_cues`]) as the alpha reads
//! them ahead, and prints those that find no sound: whether any cue table
//! of `vz.wad` or of the other archives holds a record for the key, and
//! the sets and events that use it.
//!
//! ```text
//! cargo run --release -p m2-engine --example firecues -- "<install dir>"
//! ```
//!
//! Prints hashes and counts only.

use std::path::Path;

use m2_assets::name_hash;
use m2_engine::audio::{KeySounds, world_cue_tables};
use m2_engine::gun_sounds::{self as g, GunCues};
use m2_engine::{AssetSource, Install};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: firecues <install dir>")?;
    let install = Install::find(Path::new(&root))?;
    let cues = GunCues::read(&install)?;
    let mut source = install.open_world()?;
    let tables = world_cue_tables(&mut source);
    // The other archives' cue tables, to tell a cue kept elsewhere from
    // one kept nowhere.
    let mut elsewhere: Vec<(&str, Vec<m2_assets::CueTable>)> = Vec::new();
    for wad in ["shell.wad", "English.wad", "Loading.wad"] {
        if let Ok(f) = std::fs::File::open(install.data.join(wad))
            && let Ok(mut s) = AssetSource::new(std::io::BufReader::new(f))
        {
            elsewhere.push((wad, world_cue_tables(&mut s)));
        }
    }
    let events: Vec<(u32, &str)> = [
        g::FIRE,
        g::FIRE_NPC,
        g::DRY_FIRE,
        g::LOOP,
        g::LOOP_NPC,
        g::FIRE_TAIL,
        g::GRAPPLE_FIRE,
        g::RELOAD,
    ]
    .iter()
    .map(|e| (name_hash(e), *e))
    .collect();
    let event_name = |h: u32| {
        events
            .iter()
            .find(|e| e.0 == h)
            .map_or(format!("{h:08x}"), |e| e.1.to_owned())
    };
    // The guns firing each sound set, by name.
    let arms = {
        let index = source.archive().index();
        let db_hash = index
            .assets()
            .iter()
            .find(|a| index.type_hash(a) == m2_assets::layer::TEMPLATE_DB_TYPE)
            .map(|a| a.name_hash)
            .ok_or("no template database")?;
        let data = source.asset(db_hash, m2_assets::layer::TEMPLATE_DB_TYPE)?;
        let db = m2_assets::layer::Layer::parse(&data)?;
        m2_engine::weapons::read(&db)
    };
    let guns_of = |set: u32| {
        let mut n: Vec<&str> = arms
            .weapons
            .values()
            .filter(|w| w.feel.sound_set == Some(set))
            .map(|w| w.name.as_str())
            .collect();
        n.sort_unstable();
        n
    };
    let keys = cues.firing_cues();
    let mut found = 0;
    for &key in &keys {
        if KeySounds::resolve(&mut source, &tables, key).is_some() {
            found += 1;
            continue;
        }
        let records = tables.iter().filter(|t| t.record_of(key).is_some()).count();
        let kept: Vec<&str> = elsewhere
            .iter()
            .filter(|(_, ts)| ts.iter().any(|t| t.record_of(key).is_some()))
            .map(|(w, _)| *w)
            .collect();
        let uses = cues.uses_of(key);
        let mut guns: Vec<&str> = uses.iter().flat_map(|(set, _)| guns_of(*set)).collect();
        guns.sort_unstable();
        guns.dedup();
        let mut by_event: Vec<String> = uses.iter().map(|(_, e)| event_name(*e)).collect();
        by_event.sort();
        by_event.dedup();
        println!(
            "{key:08x}: no sound; records in {records} of {} vz.wad cue tables, elsewhere {kept:?}; {} sets, events {by_event:?}; guns {}: {:?}",
            tables.len(),
            uses.len(),
            guns.len(),
            guns.iter().take(6).collect::<Vec<_>>(),
        );
    }
    println!(
        "{found} of {} firing and reload cues find sounds",
        keys.len()
    );
    Ok(())
}
