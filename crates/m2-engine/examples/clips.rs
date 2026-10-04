//! Measures every animation clip that fits a character, to find clips by
//! what they do (clips are named only by hash): where the hands are,
//! whether the feet leave the ground, how fast the body moves.
//!
//! ```text
//! cargo run --release -p m2-engine --example clips -- "<install dir>" [model hash] > clips.tsv
//! ```
//!
//! Prints one tab-separated row per clip to stdout and, to stderr, the
//! blocks holding animations and the best candidates for a shouldered
//! rifle aim, a rifle held at the ready while moving, and a jump.
//! Positions are metres in the character's own frame: `up` above the
//! clip's hips, `fwd` along the way the character faces, `side` to its
//! right. Prints names, hashes and numbers only.

use std::collections::HashMap;
use std::path::Path;

use m2_assets::BlockContents;
use m2_assets::animation::{ANIMATION_TYPE, Clip};
use m2_engine::anim::{BoundClip, Locomotion, Skeleton};
use m2_engine::{Install, Pose};
use m2_math::glam::{Mat4, Vec2, Vec3};

const RIGHT_HAND: u32 = 0xeb6f_1b2d;
const LEFT_HAND: u32 = 0xd34f_7713;
/// Jennifer.
const DEFAULT_MODEL: u32 = 0x15d8_9832;
/// Samples taken over each clip.
const SAMPLES: usize = 24;

struct Row {
    hash: u32,
    block: String,
    duration: f32,
    speed: f32,
    /// Hands at the clip's middle: (up, fwd, side).
    right: Vec3,
    left: Vec3,
    /// The highest node at the middle (the head), above the hips.
    top: f32,
    /// How far the lowest node (the feet) rises above where it starts:
    /// at most, and at the end.
    rise: f32,
    end_rise: f32,
    /// The hands' widest spread over the clip, metres.
    spread: f32,
}

fn origin(m: &Mat4) -> Vec3 {
    m.transform_point3(Vec3::ZERO)
}

fn lowest(nodes: &[Mat4]) -> f32 {
    nodes
        .iter()
        .map(|m| origin(m).y)
        .fold(f32::INFINITY, f32::min)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: clips <install dir> [model hash]");
        std::process::exit(2);
    };
    let model = match args.get(1) {
        Some(h) => u32::from_str_radix(h.trim_start_matches("0x"), 16)?,
        None => DEFAULT_MODEL,
    };
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let skeleton = Skeleton::from_nodes(&source.model(model)?.parse()?.nodes);
    let node = |h| skeleton.name_hashes.iter().position(|&n| n == h);
    let rh = node(RIGHT_HAND).ok_or("no right hand")?;
    let lh = node(LEFT_HAND).ok_or("no left hand")?;
    match skeleton.upper_body(rh, lh) {
        Some(mask) => {
            let upper: Vec<String> = (0..skeleton.len())
                .filter(|&i| mask[i])
                .map(|i| format!("{:08x}", skeleton.name_hashes[i]))
                .collect();
            eprintln!(
                "upper body: {} of {} nodes: {}",
                upper.len(),
                skeleton.len(),
                upper.join(" ")
            );
        }
        None => eprintln!("upper body: not found"),
    }
    // The bones from the root to each hand, to check how the body hangs
    // together: index, name hash, children, rest height, and the lowest
    // rest height under each child off the path.
    let rest = skeleton.model_matrices(&skeleton.rest_pose());
    let below = |n: usize| -> f32 {
        (0..skeleton.len())
            .filter(|&i| {
                let mut at = Some(i);
                while let Some(j) = at {
                    if j == n {
                        return true;
                    }
                    at = skeleton.parents[j];
                }
                false
            })
            .map(|i| rest[i].w_axis.y)
            .fold(f32::INFINITY, f32::min)
    };
    for (name, hand) in [("right hand", rh), ("left hand", lh)] {
        let mut path = vec![hand];
        while let Some(p) = skeleton.parents[*path.last().unwrap()] {
            path.push(p);
        }
        path.reverse();
        eprintln!("root to {name}:");
        for (k, &n) in path.iter().enumerate() {
            let off: Vec<String> = (0..skeleton.len())
                .filter(|&c| skeleton.parents[c] == Some(n) && path.get(k + 1) != Some(&c))
                .map(|c| format!("{c}:{:.2}", below(c)))
                .collect();
            eprintln!(
                "  {n:3} {:08x} y {:5.2}  off-path children (lowest y under) {}",
                skeleton.name_hashes[n],
                rest[n].w_axis.y,
                off.join(" ")
            );
        }
    }

    let index = source.archive().index();
    let paths: Vec<String> = index.blocks().iter().map(|b| b.path.clone()).collect();
    let mut per_block: HashMap<usize, usize> = HashMap::new();
    for a in index.assets() {
        if index.type_hash(a) == ANIMATION_TYPE {
            *per_block.entry(usize::from(a.block)).or_default() += 1;
        }
    }
    let mut blocks: Vec<usize> = per_block.keys().copied().collect();
    blocks.sort_unstable();
    let mut clips: Vec<(BoundClip, String)> = Vec::new();
    for b in blocks {
        let decoded = source.block(b)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if e.type_hash != ANIMATION_TYPE {
                continue;
            }
            if let Ok(clip) = Clip::parse(data)
                && let Some(bound) = BoundClip::bind(e.name_hash, clip, &skeleton)
            {
                clips.push((bound, paths[b].clone()));
            }
        }
    }
    let mut by_block: Vec<(&String, usize)> =
        per_block.iter().map(|(&b, &n)| (&paths[b], n)).collect();
    by_block.sort();
    eprintln!("animation blocks ({}):", by_block.len());
    for (path, n) in &by_block {
        eprintln!("  {n:5}  {path}");
    }
    let bound: Vec<BoundClip> = clips.iter().map(|c| c.0.clone()).collect();
    let forward = Locomotion::choose(&bound).map_or(Vec2::Y, |l| l.forward);
    let right_of = Vec2::new(forward.y, -forward.x);
    eprintln!("{} clips fit {model:08x}; forward {forward}", clips.len());

    let local = |p: Vec3, hips: f32| {
        let flat = Vec2::new(p.x, p.z);
        Vec3::new(p.y - hips, flat.dot(forward), flat.dot(right_of))
    };
    let mut rows = Vec::new();
    for (c, block) in &clips {
        let d = c.info.duration;
        let hips = c.info.hips;
        let at = |t: f32| -> Vec<Mat4> {
            let pose: Pose = c.sample(&skeleton, t);
            skeleton.model_matrices(&pose)
        };
        let floor = lowest(&at(0.0));
        // Hands are measured from the root's spot under the body on the
        // same frame, so a clip's travel doesn't count as reach.
        let from_root = |m: &[Mat4], node: usize| {
            let root = origin(&m[0]);
            let p = origin(&m[node]);
            local(Vec3::new(p.x - root.x, p.y, p.z - root.z), hips)
        };
        let (mut rise, mut end_rise, mut spread) = (0.0f32, 0.0f32, 0.0f32);
        for i in 0..=SAMPLES {
            let m = at(d * i as f32 / SAMPLES as f32);
            let r = lowest(&m) - floor;
            rise = rise.max(r);
            if i == SAMPLES {
                end_rise = r;
            }
            spread = spread.max(origin(&m[rh]).distance(origin(&m[lh])));
        }
        let mid = at(d * 0.5);
        let top = mid
            .iter()
            .map(|m| origin(m).y)
            .fold(f32::NEG_INFINITY, f32::max)
            - hips;
        rows.push(Row {
            hash: c.hash,
            block: block.clone(),
            duration: d,
            speed: c.info.speed,
            right: from_root(&mid, rh),
            left: from_root(&mid, lh),
            top,
            rise,
            end_rise,
            spread,
        });
    }
    println!(
        "hash\tblock\tseconds\tspeed\trh_up\trh_fwd\trh_side\tlh_up\tlh_fwd\tlh_side\ttop\trise\tend_rise\tspread"
    );
    for r in &rows {
        println!(
            "{:08x}\t{}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}",
            r.hash,
            r.block,
            r.duration,
            r.speed,
            r.right.x,
            r.right.y,
            r.right.z,
            r.left.x,
            r.left.y,
            r.left.z,
            r.top,
            r.rise,
            r.end_rise,
            r.spread
        );
    }
    let print = |title: &str, mut list: Vec<(&Row, f32)>, most: usize| {
        list.sort_by(|a, b| b.1.total_cmp(&a.1));
        eprintln!("{title}:");
        for (r, score) in list.iter().take(most) {
            eprintln!(
                "  {:08x} {:5.2}s {:4.1}m/s  rh {:5.2} {:5.2} {:5.2}  lh {:5.2} {:5.2} {:5.2}  top {:4.2} rise {:4.2}/{:4.2}  score {score:.2}  {}",
                r.hash,
                r.duration,
                r.speed,
                r.right.x,
                r.right.y,
                r.right.z,
                r.left.x,
                r.left.y,
                r.left.z,
                r.top,
                r.rise,
                r.end_rise,
                r.block
            );
        }
    };
    let show = |title: &str, list: Vec<(&Row, f32)>| print(title, list, 20);
    let show_all = |title: &str, list: Vec<(&Row, f32)>| print(title, list, usize::MAX);
    // A shouldered rifle: standing, the right hand by the cheek (high, a
    // little forward), the left hand further forward along the barrel.
    let aim = |r: &Row| -> f32 {
        let shoulder = r.top - 0.25;
        -(r.right.x - shoulder).abs() - (r.left.x - shoulder + 0.1).abs()
            + (r.left.y - r.right.y).clamp(0.0, 0.5)
            + r.left.y.clamp(0.0, 0.6)
    };
    show(
        "shouldered aim (standing, hands at the shoulder, left hand forward)",
        rows.iter()
            .filter(|r| r.speed < 0.3 && r.duration >= 0.3 && r.spread < 0.8)
            .map(|r| (r, aim(r)))
            .collect(),
    );
    show(
        "rifle at the ready while moving (hands together in front)",
        rows.iter()
            .filter(|r| r.speed > 1.0 && r.spread < 0.7)
            .map(|r| (r, r.left.y.min(r.right.y) - (r.spread - 0.4).abs()))
            .collect(),
    );
    show(
        "rifle held standing (hands together in front, low)",
        rows.iter()
            .filter(|r| r.speed < 0.3 && r.duration >= 1.0 && r.spread < 0.7)
            .map(|r| (r, r.left.y.min(r.right.y) - (r.spread - 0.4).abs()))
            .collect(),
    );
    // Jennifer's own set, all of it: her weapon clips should be there.
    let own: Vec<(&Row, f32)> = rows
        .iter()
        .filter(|r| r.block.contains("characternameanimgroup_jennifer"))
        .map(|r| (r, r.speed))
        .collect();
    show_all("characternameanimgroup_jennifer, by speed", own);
    show(
        "jumps (the feet leave the ground and come back)",
        rows.iter()
            .filter(|r| (0.3..3.0).contains(&r.duration) && r.end_rise < 0.15)
            .map(|r| (r, r.rise))
            .collect(),
    );
    Ok(())
}
