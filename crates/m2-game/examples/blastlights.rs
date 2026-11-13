//! Lists every object template whose script (`ObjectScript`) changes the
//! light when it activates, as a blast's does (`Graphics.Atmosphere.Begin`
//! .. `End`, read by `m2_game::atmosphere_script`): the template, its
//! script, the seconds until the change, the seconds easing in and back,
//! and each value it sets.
//!
//! ```text
//! cargo run --release -p m2-game --example blastlights -- "<install dir>"
//! ```
//!
//! Prints names, hashes and numbers only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::lua::{Chunk, SCRIPT_TYPE};
use m2_engine::Install;
use m2_engine::atmosphere::EditValue;
use m2_game::atmosphere_script::lights_of;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: blastlights <install dir>")?;
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
    let explosive: std::collections::HashSet<u32> = db.hashes("Explosive").into_keys().collect();
    // Each script once, with the templates that run it.
    let mut by_script: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (id, script) in db.hashes("ObjectScript") {
        by_script.entry(script).or_default().push(id);
    }
    let mut chunks = Vec::new();
    let mut unread = 0;
    for &script in by_script.keys() {
        match source
            .asset(script, SCRIPT_TYPE)
            .ok()
            .and_then(|b| Chunk::from_asset(&b).ok())
        {
            Some(c) => chunks.push((script, c)),
            None => unread += 1,
        }
    }
    let scripts = chunks.len();
    let (lights, failed) = lights_of(chunks);
    println!(
        "{scripts} object scripts read ({unread} not), {} change the light on activation, {} did not run",
        lights.len(),
        failed.len()
    );
    let lights: BTreeMap<u32, _> = lights.into_iter().collect();
    for (script, (delay, light)) in &lights {
        let mut owners: Vec<String> = by_script[script]
            .iter()
            .map(|id| {
                format!(
                    "{}{}",
                    names.get(id).copied().unwrap_or("?"),
                    if explosive.contains(id) {
                        " [blast]"
                    } else {
                        ""
                    }
                )
            })
            .collect();
        owners.sort();
        println!(
            "script {script:08x}: after {delay} s, in over {} s, back over {:?} s; templates: {}",
            light.interpolate,
            light.restore,
            owners.join(", ")
        );
        for (name, value) in &light.edits {
            match value {
                EditValue::Float(v) => println!("    {name} = {v}"),
                EditValue::Colour(w) => println!("    {name} = {w:08x}"),
            }
        }
    }
    // Those the runner could not run (it has the light's library and
    // timers only): their templates, and the first line of why.
    let mut failed: Vec<(u32, String)> = failed;
    failed.sort();
    for (script, why) in &failed {
        let mut owners: Vec<&str> = by_script[script]
            .iter()
            .map(|id| names.get(id).copied().unwrap_or("?"))
            .collect();
        owners.sort_unstable();
        owners.truncate(3);
        println!(
            "did not run {script:08x} ({}): {}",
            owners.join(", "),
            why.lines().next().unwrap_or("")
        );
    }
    Ok(())
}
