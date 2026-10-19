//! Prints each distinct `_BoatPhysics` record's steering and inertia
//! fields beside its `MaxSpeed` (gap-list sweep: `Steering` rises with
//! the boat's size, so it is no turn rate; see `m2_engine::tuning::BoatTuning`).
//!
//! ```text
//! cargo run --release -p m2-engine --example boatsteer -- "<install dir>"
//! ```
//!
//! Prints numbers and template names only.
use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::name_hash;
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: boatsteer <install>")?;
    let install = Install::find(Path::new(&root))?;
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
    let c = db.component("_BoatPhysics").ok_or("no _BoatPhysics")?;
    let wanted = [
        "MaxSpeed",
        "Steering",
        "LowSpeedSteering",
        "LowSpeedLimit",
        "SteerInertia",
        "SteeringInertia",
        "HandbrakeSteerBoost",
        "HandbrakeInertia",
        "RollInertia",
        "PitchUpSpeed",
        "PitchDownInertia",
        "HullFriction",
        "EngineInertia",
    ];
    let at: Vec<(&str, Option<usize>)> = wanted
        .iter()
        .map(|n| {
            (
                *n,
                c.fields
                    .iter()
                    .find(|f| f.name_hash == name_hash(n))
                    .map(|f| usize::from(f.offset)),
            )
        })
        .collect();
    for (n, a) in &at {
        println!("# {n} @{a:?}");
    }
    let mut groups: BTreeMap<Vec<u8>, Vec<&str>> = BTreeMap::new();
    for (id, r) in c.records().unwrap_or_default() {
        groups
            .entry(r.to_vec())
            .or_default()
            .push(names.get(&id).copied().unwrap_or("?"));
    }
    println!("name\tn\t{}", wanted.join("\t"));
    for (r, ids) in &groups {
        let vals: Vec<String> = at
            .iter()
            .map(|(_, a)| {
                a.and_then(|a| r.get(a..a + 4)).map_or("-".into(), |b| {
                    format!("{:.3}", f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                })
            })
            .collect();
        println!("{}\t{}\t{}", ids[0], ids.len(), vals.join("\t"));
    }
    println!("# fields: {}", c.fields.len());
    Ok(())
}
