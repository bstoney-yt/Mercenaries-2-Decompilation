//! Throws the game's ragdoll from your own installation with a blast, as
//! the 360's ragdoll controller does ([`m2_engine::ragdoll_launch`]), and
//! prints how far and how high it goes: a check against footage of the
//! 360 (a wrecked car's rider lands 7.5 to 11 m out, 1.6 to 2.9 m up).
//!
//! ```text
//! cargo run --release -p m2-engine --example ragdolllaunch -- <vz.wad> [max force] [seat height] [throws]
//! ```
//!
//! The body is the ragdoll asset's own skeleton as bound (standing),
//! raised by the seat height (default 0.6 m, a car seat), the blast 1, 2
//! and 3 m away across the ground at its feet, the push's point at the
//! middle of its jitter. With a number of throws, that many from 2 m
//! away, each with the point jittered at random as the 360 does
//! (0x824281C8: up to half a metre across, up to 1.75 m up), and where
//! they first come down summed up (the 360's throws vary with the
//! point: the struck bone is the one nearest it). Prints numbers only.

use std::fs::File;
use std::io::BufReader;

use m2_assets::ragdoll::{HumanRagdoll, RAGDOLL};
use m2_assets::{BlockContents, Model, Physics as Phy2, Ucfx, WadArchive};
use m2_engine::ragdoll_launch::{Launch, blast_impulse};
use m2_engine::{CollisionShape, Physics, StaticKey};
use m2_math::glam::{Mat4, Quat, Vec3};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: ragdolllaunch <vz.wad> [max force] [seat]")?;
    let max_force: f32 = args.next().map_or(Ok(500.0), |a| a.parse())?;
    let seat: f32 = args.next().map_or(Ok(0.6), |a| a.parse())?;
    let throws: u32 = args.next().map_or(Ok(0), |a| a.parse())?;
    let mut wad = WadArchive::open(BufReader::new(File::open(path)?))?;
    let block = wad
        .index()
        .find_block(r"blocks\VZ\resident_P000_Q3.block")
        .ok_or("no resident block")?;
    let decoded = wad.read_block(block)?;
    let (e, data) = BlockContents::parse(&decoded)?
        .iter()
        .find(|(e, _)| e.name_hash == RAGDOLL && e.type_hash == m2_assets::model::MODEL_TYPE)
        .map(|(e, d)| (*e, d.to_vec()))
        .ok_or("no ragdoll asset")?;
    let _ = e;
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
    let lift = Mat4::from_translation(Vec3::Y * seat);
    let nodes: Vec<Mat4> = bind.iter().map(|m| lift * *m).collect();
    let hips = hashes
        .iter()
        .position(|&h| h == m2_assets::ragdoll::HIPS)
        .ok_or("no hips")?;
    println!(
        "{} kg, MaxForce {max_force}, seat {seat} m, hips {:.2} m up",
        doll.mass(),
        nodes[hips].w_axis.y
    );
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut random = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 40) as f32 / (1u64 << 24) as f32
    };
    let runs: Vec<(f32, [f32; 3])> = if throws == 0 {
        [1.0, 2.0, 3.0].map(|a| (a, [0.5; 3])).to_vec()
    } else {
        (0..throws)
            .map(|_| (2.0, [random(), random(), random()]))
            .collect()
    };
    let mut downs = Vec::new();
    for (away, jitter) in runs {
        let mut p = Physics::new();
        let half = 200.0;
        let ground = CollisionShape::mesh(
            &[
                Vec3::new(-half, 0.0, -half),
                Vec3::new(half, 0.0, -half),
                Vec3::new(half, 0.0, half),
                Vec3::new(-half, 0.0, half),
            ],
            &[[0, 1, 2], [0, 2, 3]],
        )
        .ok_or("no ground")?;
        p.add_static(StaticKey::Cell(1), &ground, Vec3::ZERO, Quat::IDENTITY);
        let id = p
            .add_asset_ragdoll(&doll, &hashes, &parents, &nodes, Vec3::ZERO)
            .ok_or("ragdoll not built")?;
        let feet = Vec3::new(0.0, seat, 0.0);
        let centre = Vec3::new(-away, 0.0, 0.0);
        // The vehicle wreck's blast: reach 10 m, least falloff 0.3.
        let i = blast_impulse(centre, feet, 10.0, max_force, 0.3, 1.0, jitter);
        let l = Launch::new(&i, Vec3::Y);
        let start = nodes[hips].w_axis.truncate();
        let dt = 1.0 / 60.0;
        let mut top: f32 = 0.0;
        let mut first_down = None;
        for k in 0..600 {
            if k == 0 {
                p.launch_ragdoll(id, l.velocity, l.point);
            }
            p.step(dt);
            let now = p.ragdoll_nodes(id).ok_or("gone")?[hips].w_axis.truncate();
            top = top.max(now.y);
            if first_down.is_none() && k > 10 && now.y < 0.3 {
                first_down = Some(((now - start).with_y(0.0).length(), k as f32 * dt));
            }
        }
        downs.extend(first_down.map(|d| d.0));
        let end = p.ragdoll_nodes(id).ok_or("gone")?[hips].w_axis.truncate();
        println!(
            "blast {away} m away: launch {:.1} m/s, hips up to {:.2} m, first down {:.2?} (m, s), rests {:.2} m out",
            l.velocity.length(),
            top,
            first_down,
            (end - start).with_y(0.0).length()
        );
    }
    if throws > 0 && !downs.is_empty() {
        downs.sort_by(f32::total_cmp);
        let at = |q: f32| downs[((downs.len() - 1) as f32 * q).round() as usize];
        println!(
            "{} throws first down: least {:.2}, quarter {:.2}, median {:.2}, three quarters {:.2}, most {:.2} m",
            downs.len(),
            at(0.0),
            at(0.25),
            at(0.5),
            at(0.75),
            at(1.0)
        );
    }
    Ok(())
}
