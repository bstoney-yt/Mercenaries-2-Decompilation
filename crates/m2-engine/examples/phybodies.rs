//! Tallies the values of every `PHY2` rigid body's unread fields across
//! the world's models, to tell what they hold.
//!
//! ```text
//! cargo run --release -p m2-engine --example phybodies -- "<install dir>"
//! ```
//!
//! Prints values and counts only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::model::MODEL_TYPE;
use m2_assets::physics::Physics;
use m2_engine::{AssetSource, Install};

fn models<R: std::io::Read + std::io::Seek>(source: &AssetSource<R>) -> Vec<u32> {
    let index = source.archive().index();
    index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == MODEL_TYPE)
        .map(|a| a.name_hash)
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(root) = std::env::args().nth(1) else {
        eprintln!("usage: phybodies <install dir>");
        std::process::exit(2);
    };
    let install = Install::find(Path::new(&root))?;
    let mut source = install.open_world()?;
    let mut floats: BTreeMap<String, usize> = BTreeMap::new();
    let mut words: [BTreeMap<u32, usize>; 8] = Default::default();
    let mut odd = Vec::new();
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let (mut bodies, mut moving) = (0, 0);
    for h in models(&source) {
        let Ok(data) = source.model(h) else { continue };
        let Some(p) = m2_assets::Ucfx::parse(&data.base)
            .ok()
            .and_then(|u| u.top_level(*b"PHY2").map(<[u8]>::to_vec))
            .and_then(|d| Physics::parse(&d).ok())
        else {
            continue;
        };
        for b in &p.bodies {
            let key = format!(
                "inertia max {} mass>0 {}",
                b.inertia[0] == f32::MAX,
                b.mass > 0.0
            );
            *kinds.entry(key).or_default() += 1;
            bodies += 1;
            if !b.is_fixed() {
                moving += 1;
            }
            let k = format!("{:?} fixed={}", b.unk, b.is_fixed());
            if b.unk != [0.3; 4] && odd.len() < 40 {
                odd.push(format!(
                    "{h:08x} mass {} {:?} {:08x?}",
                    b.mass, b.unk, b.unk_words
                ));
            }
            *floats.entry(k).or_default() += 1;
            for (i, w) in b.unk_words.iter().enumerate() {
                *words[i].entry(*w).or_default() += 1;
            }
        }
    }
    println!("{bodies} bodies, {moving} moving");
    for (k, n) in &kinds {
        println!("{k}: {n}");
    }
    for (k, n) in &floats {
        println!("unk {k}: {n}");
    }
    for (i, m) in words.iter().enumerate() {
        let top: Vec<_> = m
            .iter()
            .take(12)
            .map(|(w, n)| format!("{w:08x}x{n}"))
            .collect();
        println!("word {i} ({} values): {}", m.len(), top.join(" "));
    }
    for o in &odd {
        println!("odd {o}");
    }
    Ok(())
}
