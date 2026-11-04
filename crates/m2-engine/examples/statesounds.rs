//! Lists the effects the island's models' state machines start
//! (`StartEmitter`), each as the template of that name, and what that
//! template's `SoundEffect` plays as its object comes into being: its own
//! cue, nothing (no record, or a record that plays nothing on creation),
//! or nothing for want of a `Sounds` row for its sound set.
//!
//! ```text
//! cargo run --release -p m2-engine --example statesounds -- "<install dir>"
//! ```
//!
//! Prints template names, hashes and counts only.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::model::MODEL_TYPE;
use m2_assets::name_hash;
use m2_engine::Install;
use m2_engine::gun_sounds::GunCues;
use m2_engine::spawn_sound::CREATE_EVENT;

/// The verb `StartEmitter`, whose first operand names the effect.
const START_EMITTER: u32 = 0x3347_914f;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: statesounds <install dir>")?;
    let install = Install::find(Path::new(&root))?;
    let mut source = install.open_world()?;
    let (db_hash, models) = {
        let index = source.archive().index();
        let db = index
            .assets()
            .iter()
            .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
            .map(|a| a.name_hash)
            .ok_or("no template database")?;
        let mut models: Vec<u32> = index
            .assets()
            .iter()
            .filter(|a| index.type_hash(a) == MODEL_TYPE)
            .map(|a| a.name_hash)
            .collect();
        models.sort_unstable();
        models.dedup();
        (db, models)
    };
    let data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&data)?;
    let names = db.names();
    // Of twins by name, the lowest id's (as the alpha resolves them).
    let mut by_hash: HashMap<u32, (u32, &str)> = HashMap::new();
    for (&id, &n) in &names {
        let e = by_hash.entry(name_hash(n)).or_insert((id, n));
        if id < e.0 {
            *e = (id, n);
        }
    }
    let spawn = m2_engine::spawn_sound::read(&db);
    let cues = GunCues::from_source(&mut source)?;
    // Effect name hash -> how many models start it.
    let mut started: BTreeMap<u32, usize> = BTreeMap::new();
    let mut with = 0;
    for &m in &models {
        let Ok(d) = source.model(m) else { continue };
        let w: Vec<u32> = d
            .base
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        let mut any = false;
        for k in 2..w.len().saturating_sub(1) {
            if w[k] != 2 || w[k + 1] != START_EMITTER {
                continue;
            }
            // Its first operand: the first of the (1, x) pairs before it.
            let mut j = k;
            while j >= 2 && w[j - 2] == 1 {
                j -= 2;
            }
            if j < k {
                *started.entry(w[j + 1]).or_default() += 1;
                any = true;
            }
        }
        with += usize::from(any);
    }
    let mut tally = [0usize; 4];
    println!("effect\tmodels\ttemplate\theard");
    for (h, n) in &started {
        let (label, heard, k) = match by_hash.get(h) {
            None => (format!("#{h:08x}"), "no template".to_owned(), 3),
            Some(&(id, name)) => {
                let (heard, k) = match spawn.get(&id) {
                    None => ("silent: no SoundEffect".to_owned(), 1),
                    Some(s) => match s.on_create(0.0) {
                        None => (format!("silent: mode {}", s.mode), 1),
                        Some(set) => match cues.cue_of(set, CREATE_EVENT) {
                            Some(cue) => (format!("own cue {cue:08x} (set {set:08x})"), 0),
                            None => (format!("silent: set {set:08x} has no Sounds row"), 2),
                        },
                    },
                };
                (format!("{name} ({id:08x})"), heard, k)
            }
        };
        tally[k] += 1;
        println!("{h:08x}\t{n}\t{label}\t{heard}");
    }
    println!(
        "{} models of {} start {} effects: {} with their own cue, {} silent as their template says, \
         {} silent for want of a Sounds row, {} naming no template",
        with,
        models.len(),
        started.len(),
        tally[0],
        tally[1],
        tally[2],
        tally[3]
    );
    Ok(())
}
