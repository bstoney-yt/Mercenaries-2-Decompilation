//! Reads every face rig and face animation set of the world archive
//! ([`m2_assets::facefx`]) and says what each holds, for checking the
//! readers against the game's data: per rig its bones and poses, per set
//! its animations, curves and keys, then the totals and any that failed.
//!
//! ```text
//! cargo run --release -p m2-engine --example facefx -- "<install dir>"
//! ```
//!
//! Prints names, hashes and numbers only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::facefx::{ACTOR_TYPE, ANIMATION_SET_TYPE, Actor, AnimationSet};
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: facefx <install dir>")?;
    let install = Install::find(Path::new(&root))?;
    let mut source = install.open_world()?;
    let of_type = |source: &m2_engine::AssetSource<_>, ty: u32| -> Vec<u32> {
        let index = source.archive().index();
        let mut hashes: Vec<u32> = index
            .assets()
            .iter()
            .filter(|a| index.type_hash(a) == ty)
            .map(|a| a.name_hash)
            .collect();
        hashes.sort_unstable();
        hashes.dedup();
        hashes
    };

    let actors = of_type(&source, ACTOR_TYPE);
    let (mut read, mut failed) = (0, 0);
    let mut pose_names: BTreeMap<String, usize> = BTreeMap::new();
    let mut bone_names: BTreeMap<String, usize> = BTreeMap::new();
    for hash in &actors {
        let data = source.asset(*hash, ACTOR_TYPE)?;
        match Actor::parse(&data) {
            Ok(a) => {
                read += 1;
                println!(
                    "actor {hash:08x} {}: {} bones, {} poses",
                    a.name,
                    a.bones.len(),
                    a.poses.len()
                );
                for p in &a.poses {
                    *pose_names.entry(p.name.clone()).or_default() += 1;
                }
                for b in &a.bones {
                    *bone_names.entry(b.name.clone()).or_default() += 1;
                }
            }
            Err(e) => {
                failed += 1;
                println!("actor {hash:08x}: {e}");
            }
        }
    }
    println!("{read} of {} actors read, {failed} failed", actors.len());
    println!(
        "bones: {}",
        bone_names
            .iter()
            .map(|(n, c)| format!("{n} {c}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "poses: {}",
        pose_names
            .iter()
            .map(|(n, c)| format!("{n} {c}"))
            .collect::<Vec<_>>()
            .join(", ")
    );

    let sets = of_type(&source, ANIMATION_SET_TYPE);
    let (mut read, mut failed, mut animations, mut keys) = (0, 0, 0, 0);
    let mut curve_names: BTreeMap<String, usize> = BTreeMap::new();
    let mut longest = 0.0f32;
    for hash in &sets {
        let data = source.asset(*hash, ANIMATION_SET_TYPE)?;
        match AnimationSet::parse(&data) {
            Ok(s) => {
                read += 1;
                animations += s.animations.len();
                let set_keys: usize = s
                    .animations
                    .iter()
                    .flat_map(|a| &a.curves)
                    .map(|c| c.keys.len())
                    .sum();
                keys += set_keys;
                for a in &s.animations {
                    longest = longest.max(a.seconds());
                    for c in a.curves.iter().filter(|c| !c.keys.is_empty()) {
                        *curve_names.entry(c.name.clone()).or_default() += 1;
                    }
                }
                println!(
                    "set {hash:08x} {} (for {}): {} animations, {set_keys} keys",
                    s.name,
                    s.actor,
                    s.animations.len()
                );
            }
            Err(e) => {
                failed += 1;
                println!("set {hash:08x}: {e}");
            }
        }
    }
    println!(
        "{read} of {} sets read, {failed} failed: {animations} animations, {keys} keys, the longest {longest:.1} s",
        sets.len()
    );
    println!(
        "curves with keys: {}",
        curve_names
            .iter()
            .map(|(n, c)| format!("{n} {c}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(())
}
