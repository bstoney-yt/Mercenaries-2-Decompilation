//! Lists every pickup template (`Pickup` records: ammunition, cash, fuel,
//! health, weapons lying there) with the effect its record names for
//! being taken, and what that effect's template and the pickup's own
//! template play as their objects come into being (their `SoundEffect`
//! create cues).
//!
//! ```text
//! cargo run --release -p m2-engine --example pickupsounds -- "<install dir>"
//! ```
//!
//! Prints template names, hashes and counts only.

use std::collections::HashMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::name_hash;
use m2_engine::Install;
use m2_engine::gun_sounds::GunCues;
use m2_engine::spawn_sound::{CREATE_EVENT, SpawnSound};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: pickupsounds <install dir>")?;
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
    let names = db.names();
    let mut by_hash: HashMap<u32, (u32, &str)> = HashMap::new();
    for (&id, &n) in &names {
        let e = by_hash.entry(name_hash(n)).or_insert((id, n));
        if id < e.0 {
            *e = (id, n);
        }
    }
    let spawn = m2_engine::spawn_sound::read(&db);
    let cues = GunCues::from_source(&mut source)?;
    let heard = |s: Option<&SpawnSound>| match s {
        None => "no SoundEffect".to_owned(),
        Some(s) => match s.on_create(0.0) {
            None => format!("mode {}", s.mode),
            Some(set) => match cues.cue_of(set, CREATE_EVENT) {
                Some(c) => format!("cue {c:08x}"),
                None => format!("set {set:08x} no row"),
            },
        },
    };
    let leavings = m2_engine::pickup::read(&db);
    let mut rows: Vec<_> = leavings.pickups.iter().collect();
    rows.sort_by_key(|(id, _)| names.get(id).copied().unwrap_or("?"));
    println!("pickup\ttype\teffect\teffect heard\tpickup heard\ttaken heard");
    let mut taken = 0;
    for (id, p) in rows {
        let (fx, fx_heard) = match p.effect {
            None => ("-".to_owned(), "-".to_owned()),
            Some(h) => match by_hash.get(&h) {
                Some(&(t, n)) => (format!("{n} ({t:08x})"), heard(spawn.get(&t))),
                None => (format!("#{h:08x}"), "no template".to_owned()),
            },
        };
        // As it is taken: its set's row for the taken event.
        let set = spawn.get(id).map_or(0, |s| s.set);
        let on_taken = (set != 0)
            .then(|| cues.cue_of(set, m2_engine::spawn_sound::TAKEN_EVENT))
            .flatten();
        taken += usize::from(on_taken.is_some());
        println!(
            "{}\t{}\t{fx}\t{fx_heard}\t{}\t{}",
            names.get(id).copied().unwrap_or("?"),
            p.kind,
            heard(spawn.get(id)),
            on_taken.map_or(format!("set {set:08x}: none"), |c| format!(
                "set {set:08x}: cue {c:08x}"
            ))
        );
    }
    println!(
        "{taken} of {} pickup templates play a cue of their own as they are taken",
        leavings.pickups.len()
    );
    // What those cues find in the world's cue tables.
    let tables = m2_engine::audio::world_cue_tables(&mut source);
    let mut seen: Vec<u32> = leavings
        .pickups
        .keys()
        .filter_map(|id| spawn.get(id))
        .filter(|s| s.set != 0)
        .filter_map(|s| cues.cue_of(s.set, m2_engine::spawn_sound::TAKEN_EVENT))
        .collect();
    seen.sort_unstable();
    seen.dedup();
    for cue in seen {
        let found = m2_engine::audio::KeySounds::resolve(&mut source, &tables, cue);
        println!(
            "cue {cue:08x}: {}",
            found.map_or("no sound".to_owned(), |k| format!(
                "{} sounds, volume {:.2}, {}",
                k.sounds.len(),
                k.volume,
                k.how
            ))
        );
    }
    Ok(())
}
