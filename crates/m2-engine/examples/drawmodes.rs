//! Counts the particle effects' emitters by their draw mode (the `PTYP`
//! setting `c3592bb7`, which the 360's effect reader keeps at emitter
//! +0x1ec and its sprite batching reads as the batch's material: mode 3
//! the refracting one), and lists the emitters of mode 3 with their
//! effect, the templates showing it, textures and settings.
//!
//! ```text
//! cargo run --release -p m2-engine --example drawmodes -- "<install dir>"
//! ```
//!
//! Prints names, hashes and numbers only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::effect::{self, EFFECT_TYPE};
use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::Install;
use m2_engine::particles::template_effects;

const DRAW_MODE: u32 = 0xc359_2bb7;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: drawmodes <install dir>")?;
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
    let shown = template_effects(&db);
    let mut hashes: Vec<u32> = {
        let index = source.archive().index();
        index
            .assets()
            .iter()
            .filter(|a| index.type_hash(a) == EFFECT_TYPE)
            .map(|a| a.name_hash)
            .collect()
    };
    hashes.sort_unstable();
    hashes.dedup();
    let mut modes: BTreeMap<String, usize> = BTreeMap::new();
    for h in hashes {
        let Ok(fx) = effect::parse(&source.asset(h, EFFECT_TYPE)?) else {
            continue;
        };
        for (i, e) in fx.emitters.iter().enumerate() {
            let mode = effect::find(&e.particle, DRAW_MODE);
            let key = mode.map_or("none".to_owned(), |a| {
                format!(
                    "{} k{} f{} {:?} keyed {}",
                    a.int(),
                    a.kind,
                    a.flags,
                    a.value,
                    a.keyed()
                )
            });
            *modes.entry(key).or_default() += 1;
            if mode.map(|a| a.int()) != Some(3) {
                continue;
            }
            let mut by: Vec<&str> = shown
                .iter()
                .filter(|(_, fx)| fx.contains(&h))
                .filter_map(|(id, _)| names.get(id).copied())
                .collect();
            by.sort_unstable();
            by.dedup();
            let settings: Vec<String> = e
                .particle
                .iter()
                .map(|a| {
                    if a.keyed() {
                        format!(
                            "{:08x}=keyed({:.3}..{:.3})",
                            a.name.unwrap_or(0),
                            a.at(0.0),
                            a.at(99.0)
                        )
                    } else {
                        format!("{:08x}={:.3}/{}", a.name.unwrap_or(0), a.float(), a.int())
                    }
                })
                .collect();
            let textures: Vec<String> = e.textures.iter().map(|t| format!("{t:08x}")).collect();
            println!(
                "effect {h:08x} emitter {i}: shown by {} {:?}; textures [{}]; {}",
                by.len(),
                by.iter().take(8).collect::<Vec<_>>(),
                textures.join(" "),
                settings.join(" ")
            );
        }
    }
    println!("emitters by draw mode: {modes:?}");
    Ok(())
}
