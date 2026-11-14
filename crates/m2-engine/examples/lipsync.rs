//! Measures what lip-sync has to work with ([`m2_engine::lip_sync`]): the
//! person templates naming a face rig and whether the rig is in the world
//! archive, then the voice lines the clips speak (their `vo` events) and
//! how many have a face animation of the same name in some set.
//!
//! ```text
//! cargo run --release -p m2-engine --example lipsync -- "<install dir>"
//! ```
//!
//! Prints hashes and numbers only.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use m2_assets::animation::{ANIMATION_TYPE, Clip};
use m2_assets::facefx::{ACTOR_TYPE, ANIMATION_SET_TYPE, AnimationSet};
use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::name_hash;
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: lipsync <install dir>")?;
    let install = Install::find(Path::new(&root))?;
    let mut source = install.open_world()?;
    let of_type = |source: &m2_engine::AssetSource<_>, ty: u32| -> BTreeSet<u32> {
        let index = source.archive().index();
        index
            .assets()
            .iter()
            .filter(|a| index.type_hash(a) == ty)
            .map(|a| a.name_hash)
            .collect()
    };
    let actors = of_type(&source, ACTOR_TYPE);
    let db_hash = *of_type(&source, TEMPLATE_DB_TYPE)
        .first()
        .ok_or("no template database")?;
    let db_bytes = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&db_bytes)?;
    let faces = &m2_engine::tuning::face_actors(&db);
    let named: BTreeSet<u32> = faces.values().copied().collect();
    let present = named.iter().filter(|h| actors.contains(h)).count();
    println!(
        "templates naming a face rig: {} ({} distinct rigs, {} in the archive, {} rigs named by none)",
        faces.len(),
        named.len(),
        present,
        actors.difference(&named).count()
    );
    for h in named.difference(&actors) {
        println!("  rig {h:08x} named but not in the archive");
    }

    // Every face animation, by name hash: (set, curves with keys).
    let mut animations: HashMap<u32, (u32, usize)> = HashMap::new();
    for set in of_type(&source, ANIMATION_SET_TYPE) {
        let Ok(s) = AnimationSet::parse(&source.asset(set, ANIMATION_SET_TYPE)?) else {
            continue;
        };
        for a in &s.animations {
            let curves = a.curves.iter().filter(|c| !c.keys.is_empty()).count();
            animations.insert(name_hash(&a.name), (set, curves));
        }
    }
    println!("face animations: {}", animations.len());

    // The clips' voice lines.
    let mut lines: BTreeSet<u32> = BTreeSet::new();
    for clip in of_type(&source, ANIMATION_TYPE) {
        let Ok(bytes) = source.asset(clip, ANIMATION_TYPE) else {
            continue;
        };
        let Ok(c) = Clip::parse(&bytes) else {
            continue;
        };
        for e in c.events.iter().filter(|e| e.category == "vo") {
            lines.insert(name_hash(&e.name));
        }
    }
    let matched: Vec<u32> = lines
        .iter()
        .copied()
        .filter(|h| animations.contains_key(h))
        .collect();
    println!(
        "clip voice lines: {}, with a face animation of that name: {}",
        lines.len(),
        matched.len()
    );
    for h in matched.iter().take(5) {
        let (set, curves) = animations[h];
        println!("  line {h:08x}: set {set:08x}, {curves} curves");
    }
    Ok(())
}
