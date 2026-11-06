//! Prints, for templates whose name holds any of the given words, which
//! physics components they have and the collision layers their `PHY2`
//! shapes hold, the flags the 360 picks their layers by
//! (`m2_engine::collision_layers`).
//!
//! ```text
//! cargo run --release -p m2-engine --example bodylayers -- "<install dir>" fence wall
//! ```
//!
//! Prints numbers and template names only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::physics::Physics;
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let Some(root) = args.next() else {
        eprintln!("usage: bodylayers <install dir> <word>...");
        std::process::exit(2);
    };
    let words: Vec<String> = args.map(|a| a.to_lowercase()).collect();
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
    let models = db.hashes("ModelName");
    let comps = [
        "_PropPhysics",
        "_DebrisPhysics",
        "_BuildingPhysics",
        "PhysicsPropertyCrashable",
        "PhysicsPropertyUncrushable",
        "Health",
    ];
    let mut has: BTreeMap<u32, Vec<&str>> = BTreeMap::new();
    for c in comps {
        let Some(t) = db.component(c) else { continue };
        for (id, _) in t.records().unwrap_or_default() {
            has.entry(id).or_default().push(c);
        }
    }
    let mut lines: Vec<(String, String)> = Vec::new();
    for (&id, &name) in &names {
        let lower = name.to_lowercase();
        if !words.iter().any(|w| lower.contains(w.as_str())) {
            continue;
        }
        let mut tally: BTreeMap<u32, u32> = BTreeMap::new();
        if let Some(&model) = models.get(&id)
            && let Ok(d) = source.model(model)
            && let Some(p) = m2_assets::Ucfx::parse(&d.base)
                .ok()
                .and_then(|u| u.top_level(*b"PHY2").map(<[u8]>::to_vec))
                .and_then(|d| Physics::parse(&d).ok())
        {
            for s in &p.shapes {
                *tally.entry(s.flags()).or_default() += 1;
            }
        }
        lines.push((
            name.to_string(),
            format!(
                "{:?} word4 {:x?}",
                has.get(&id).cloned().unwrap_or_default(),
                tally
            ),
        ));
    }
    lines.sort();
    for (n, l) in lines {
        println!("{n}: {l}");
    }
    Ok(())
}
