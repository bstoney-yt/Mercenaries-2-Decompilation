//! Lists what the templates that come down as buildings (those with
//! `BuildingCollapseAnim`) play as they collapse, as the 360's sound
//! system has it: the messages their model's `CollapseState` sends
//! (`SendMessage`), the events each starts
//! ([`m2_engine::spawn_sound::object_event`]), and the cue the `Sounds`
//! table gives their `SoundEffect` set for each.
//!
//! ```text
//! cargo run --release -p m2-engine --example collapsesounds -- "<install dir>"
//! ```
//!
//! Prints template names, hashes and counts only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::gun_sounds::GunCues;
use m2_engine::{Install, StateFx, StateMessage};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: collapsesounds <install dir>")?;
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
    let models = db.hashes("ModelName");
    let spawn = m2_engine::spawn_sound::read(&db);
    let cues = GunCues::from_source(&mut source)?;
    let mut buildings: Vec<u32> = db
        .component("BuildingCollapseAnim")
        .ok_or("no BuildingCollapseAnim")?
        .records()
        .unwrap_or_default()
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    buildings.sort_unstable();
    buildings.dedup();
    // What each heard as: its own cues, by line, with the templates so.
    let mut heard: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    let mut tally = [0usize; 4];
    let mut ended = 0;
    for &id in &buildings {
        let name = names.get(&id).copied().unwrap_or("?");
        let set = spawn.get(&id).map_or(0, |s| s.set);
        let all: Vec<StateMessage> = models
            .get(&id)
            .and_then(|&m| source.model(m).ok())
            .and_then(|d| d.parse().ok().map(|m| StateMessage::of(&m)))
            .unwrap_or_default()
            .into_iter()
            .filter(|s| s.state == StateFx::COLLAPSE)
            .collect();
        // Leaving it: does it start a cue of its own (the crash)?
        let ends = all.iter().filter(|s| s.on_exit).any(|s| {
            m2_engine::spawn_sound::object_event(s.message)
                .iter()
                .any(|&(start, e)| start && set != 0 && cues.cue_of(set, e).is_some())
        });
        ended += usize::from(ends);
        let sent: Vec<StateMessage> = all.into_iter().filter(|s| !s.on_exit).collect();
        let mut parts = Vec::new();
        let mut own = false;
        let mut started = false;
        let mut seen = Vec::new();
        for s in &sent {
            if seen.contains(&s.message) {
                continue;
            }
            seen.push(s.message);
            for &(start, event) in m2_engine::spawn_sound::object_event(s.message) {
                if !start {
                    continue;
                }
                started = true;
                match (set != 0).then(|| cues.cue_of(set, event)).flatten() {
                    Some(cue) => {
                        own = true;
                        parts.push(format!("{:08x}->{event:08x}: cue {cue:08x}", s.message));
                    }
                    None => parts.push(format!("{:08x}->{event:08x}: no row", s.message)),
                }
            }
        }
        let k = if sent.is_empty() {
            3
        } else if !started {
            2
        } else if own {
            0
        } else {
            1
        };
        tally[k] += 1;
        let line = if sent.is_empty() {
            "no CollapseState messages".to_owned()
        } else {
            format!("set {set:08x}: {}", parts.join(", "))
        };
        heard.entry(line).or_default().push(name);
    }
    for (line, ts) in &heard {
        println!("{}\t{line}\t(first {})", ts.len(), ts[0]);
    }
    println!(
        "{} building templates: {} play their own collapse cue, {} start events their set has \
         no row for, {} send messages the sound system doesn't hear, {} send none",
        buildings.len(),
        tally[0],
        tally[1],
        tally[2],
        tally[3]
    );
    println!("{ended} building templates also play a cue of their own as their collapse ends");
    Ok(())
}
