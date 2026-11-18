//! Checks the game's human ragdoll offscreen, from your own installation,
//! as the 360 builds it: one section per change, each printing a line and
//! PASS or FAIL, then the whole.
//!
//! ```text
//! cargo run --release -p m2-engine --example ragdollcheck -- <vz.wad>
//! ```
//!
//! Prints numbers only.

use std::fs::File;
use std::io::BufReader;

use m2_assets::ragdoll::{HIPS, HumanRagdoll, RAGDOLL};
use m2_assets::{BlockContents, Model, Physics as Phy2, Ucfx, WadArchive};
use m2_engine::{CollisionShape, Physics, StaticKey};
use m2_math::glam::{Mat4, Quat, Vec3};

/// The game's ragdoll and its skeleton standing (bound), feet at 0.
struct Doll {
    doll: HumanRagdoll,
    hashes: Vec<u32>,
    parents: Vec<Option<usize>>,
    bind: Vec<Mat4>,
    hips: usize,
}

fn load(path: &str) -> Result<Doll, Box<dyn std::error::Error>> {
    let mut wad = WadArchive::open(BufReader::new(File::open(path)?))?;
    let block = wad
        .index()
        .find_block(r"blocks\VZ\resident_P000_Q3.block")
        .ok_or("no resident block")?;
    let decoded = wad.read_block(block)?;
    let data = BlockContents::parse(&decoded)?
        .iter()
        .find(|(e, _)| e.name_hash == RAGDOLL && e.type_hash == m2_assets::model::MODEL_TYPE)
        .map(|(_, d)| d.to_vec())
        .ok_or("no ragdoll asset")?;
    let model = Model::parse(&data)?;
    let u = Ucfx::parse(&data)?;
    let doll = HumanRagdoll::from_model(
        &model,
        &Phy2::parse(u.top_level(*b"PHY2").ok_or("no PHY2")?)?,
    )?;
    let hashes: Vec<u32> = model.nodes.iter().map(|n| n.name_hash).collect();
    let parents: Vec<Option<usize>> = model
        .nodes
        .iter()
        .map(|n| n.parent.map(usize::from))
        .collect();
    let mut bind: Vec<Mat4> = Vec::new();
    for n in &model.nodes {
        let local = Mat4::from(n.local);
        let m = match n.parent.map(usize::from) {
            Some(p) => bind[p] * local,
            None => local,
        };
        bind.push(m);
    }
    let hips = hashes.iter().position(|&h| h == HIPS).ok_or("no hips")?;
    Ok(Doll {
        doll,
        hashes,
        parents,
        bind,
        hips,
    })
}

/// A world of flat ground at height 0.
fn ground() -> Result<Physics, Box<dyn std::error::Error>> {
    let mut p = Physics::new();
    let half = 200.0;
    let g = CollisionShape::mesh(
        &[
            Vec3::new(-half, 0.0, -half),
            Vec3::new(half, 0.0, -half),
            Vec3::new(half, 0.0, half),
            Vec3::new(-half, 0.0, half),
        ],
        &[[0, 1, 2], [0, 2, 3]],
    )
    .ok_or("no ground")?;
    p.add_static(StaticKey::Cell(1), &g, Vec3::ZERO, Quat::IDENTITY);
    Ok(p)
}

/// How far, at most, a node strayed from its parent's distance when bound
/// (a limb pulled off its joint shows as a change of that distance).
fn worst_stretch(d: &Doll, nodes: &[Mat4]) -> f32 {
    let mut worst = 0.0f32;
    for (i, p) in d.parents.iter().enumerate() {
        let Some(p) = *p else { continue };
        let bound = d.bind[i].w_axis.distance(d.bind[p].w_axis);
        let now = nodes[i].w_axis.distance(nodes[p].w_axis);
        worst = worst.max((now - bound).abs());
    }
    worst
}

fn verdict(ok: bool) -> &'static str {
    if ok { "PASS" } else { "FAIL" }
}

/// #703: the game's ragdoll, let go standing, goes limp and lies: its
/// hips under 0.6 m after 5 s, every limb slower than 0.5 m/s, no limb
/// more than 0.40 m off its joint on the way.
fn limp(d: &Doll) -> Result<bool, Box<dyn std::error::Error>> {
    let mut p = ground()?;
    let id = p
        .add_asset_ragdoll(&d.doll, &d.hashes, &d.parents, &d.bind, Vec3::ZERO)
        .ok_or("ragdoll not built")?;
    let dt = 1.0 / 60.0;
    let mut stretch = 0.0f32;
    for _ in 0..300 {
        p.step(dt);
        let nodes = p.ragdoll_nodes(id).ok_or("gone")?;
        stretch = stretch.max(worst_stretch(d, &nodes));
    }
    let nodes = p.ragdoll_nodes(id).ok_or("gone")?;
    let hips = nodes[d.hips].w_axis.y;
    let speed = p.ragdoll_speed(id).unwrap_or(f32::INFINITY);
    let ok = hips < 0.6 && speed < 0.5 && stretch <= 0.40;
    println!(
        "ragdoll-check 703: let go standing: hips {hips:.2} m up after 5 s, fastest limb {speed:.2} m/s, \
         worst joint stretch {stretch:.2} m: {}",
        verdict(ok)
    );
    Ok(ok)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: ragdollcheck <vz.wad>")?;
    let d = load(&path)?;
    let mut all = true;
    all &= limp(&d)?;
    println!("ragdoll-check: {}", verdict(all));
    Ok(())
}
