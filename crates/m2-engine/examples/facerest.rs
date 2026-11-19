//! Compares each face rig's bones at rest ([`m2_assets::facefx`]) with
//! the same-named nodes of the head models that carry them: the node's
//! own rest local (to its parent) against the rig's rest, for checking
//! what space the rig's transforms are in before they replace a node's
//! local ([`m2_engine::lip_sync::apply`]).
//!
//! ```text
//! cargo run --release -p m2-engine --example facerest -- "<install dir>" [rig hash...] [set:<hash>]
//! ```
//!
//! With `set:<hash>` (a face animation set), each rig's first head is also
//! posed through every animation of the set, a tenth of a second at a
//! time, and the farthest any face bone gets from its head's own rest is
//! printed: as the rig's transforms are written in raw, and as
//! [`m2_engine::lip_sync::apply`] puts them now (the rig's moves).
//!
//! Prints hashes and numbers only.

use std::path::Path;

use m2_assets::facefx::{ACTOR_TYPE, Actor};
use m2_assets::model::{MODEL_TYPE, Model};
use m2_assets::name_hash;
use m2_engine::Install;
use m2_engine::anim::Skeleton;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = args
        .next()
        .ok_or("usage: facerest <install dir> [rig...]")?;
    let args: Vec<String> = args.collect();
    let install = Install::find(Path::new(&root))?;
    let set = args
        .iter()
        .find_map(|a| a.strip_prefix("set:"))
        .and_then(|h| u32::from_str_radix(h, 16).ok());
    let wanted: Vec<u32> = args
        .iter()
        .filter(|a| !a.starts_with("set:"))
        .filter_map(|a| u32::from_str_radix(a.trim_start_matches("0x"), 16).ok())
        .collect();
    let animations = match set {
        Some(h) => {
            let bytes = install
                .open_world()?
                .asset(h, m2_assets::facefx::ANIMATION_SET_TYPE)?;
            m2_assets::facefx::AnimationSet::parse(&bytes)?.animations
        }
        None => Vec::new(),
    };
    let of_type = |source: &m2_engine::AssetSource<_>, ty: u32| -> Vec<u32> {
        let index = source.archive().index();
        index
            .assets()
            .iter()
            .filter(|a| index.type_hash(a) == ty)
            .map(|a| a.name_hash)
            .collect()
    };
    let mut source = install.open_world()?;
    let rigs = of_type(&source, ACTOR_TYPE);
    let models = of_type(&source, MODEL_TYPE);
    for rig in rigs
        .iter()
        .filter(|r| wanted.is_empty() || wanted.contains(r))
    {
        let Ok(bytes) = source.asset(*rig, ACTOR_TYPE) else {
            continue;
        };
        let Ok(actor) = Actor::parse(&bytes) else {
            continue;
        };
        let bones: Vec<u32> = actor.bones.iter().map(|b| name_hash(&b.name)).collect();
        println!("rig {rig:08x}: {} bones", bones.len());
        let mut found = 0;
        for &m in &models {
            if found >= 2 {
                break;
            }
            let Ok(data) = source.asset(m, MODEL_TYPE) else {
                continue;
            };
            let Ok(model) = Model::parse(&data) else {
                continue;
            };
            let hits = bones
                .iter()
                .filter(|b| model.nodes.iter().any(|n| n.name_hash == **b))
                .count();
            if hits * 2 < bones.len() {
                continue;
            }
            found += 1;
            let skeleton = Skeleton::from_nodes(&model.nodes);
            println!("  model {m:08x}: {hits} of the rig's bones");
            let mut worst = 0.0f32;
            let mut worst_mirrored = 0.0f32;
            for (b, h) in actor.bones.iter().zip(&bones) {
                let Some(k) = skeleton.name_hashes.iter().position(|n| n == h) else {
                    continue;
                };
                let rest = skeleton.rest[k];
                let p = b.rest.position;
                let d = (rest.translation - m2_math::glam::Vec3::from(p)).length();
                worst = worst.max(d);
                let m = (rest.translation - m2_math::glam::Vec3::new(-p[0], p[1], p[2])).length();
                worst_mirrored = worst_mirrored.max(m);
                println!(
                    "    {:08x}: node local t [{:.4} {:.4} {:.4}] r [{:.3} {:.3} {:.3} {:.3}]; rig rest t [{:.4} {:.4} {:.4}] r [{:.3} {:.3} {:.3} {:.3}]; apart {d:.4} m",
                    h,
                    rest.translation.x,
                    rest.translation.y,
                    rest.translation.z,
                    rest.rotation.x,
                    rest.rotation.y,
                    rest.rotation.z,
                    rest.rotation.w,
                    p[0],
                    p[1],
                    p[2],
                    b.rest.rotation[0],
                    b.rest.rotation[1],
                    b.rest.rotation[2],
                    b.rest.rotation[3],
                );
            }
            println!("  worst apart {worst:.4} m as read, {worst_mirrored:.4} m mirrored in x");
            if found == 1 && !animations.is_empty() {
                let graph = m2_engine::face_graph::FaceGraph::new(&actor);
                let (mut raw, mut now) = (0.0f32, 0.0f32);
                for a in &animations {
                    let mut t = 0.0;
                    while t <= a.seconds() {
                        let poses = graph.at(a, t);
                        for p in &poses {
                            let Some(k) = skeleton.name_hashes.iter().position(|n| *n == p.bone)
                            else {
                                continue;
                            };
                            let rest = skeleton.rest[k];
                            raw = raw.max((p.position - rest.translation).length());
                            let mut locals = skeleton.rest.clone();
                            m2_engine::lip_sync::apply(
                                &mut locals,
                                &skeleton.name_hashes,
                                std::slice::from_ref(p),
                            );
                            now = now.max((locals[k].translation - rest.translation).length());
                        }
                        t += 0.1;
                    }
                }
                println!(
                    "  over {} lines: face bones as far as {raw:.4} m from the head's rest written in raw, {now:.4} m as moves",
                    animations.len()
                );
            }
        }
    }
    Ok(())
}
