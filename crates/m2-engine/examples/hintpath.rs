//! Runs a tank-sized path search across one of a template's real model,
//! on flat ground, from 40 m before it to 40 m past it, with and without
//! its `AiHintNode` points, and prints what each search finds.
//!
//! ```text
//! cargo run --release -p m2-engine --example hintpath -- "<install dir>" _proutpost_helipad
//! ```
//!
//! With no template named, lists the templates that have hints.
//! Prints numbers and template names only.

use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::{Model, Ucfx};
use m2_engine::physics::{CollisionShape, StaticKey};
use m2_engine::{Install, Physics, path_search};
use m2_math::glam::{Quat, Vec3};

struct Flat<'a> {
    physics: &'a Physics,
    hints: &'a [(u32, Vec3)],
    sweeps: usize,
    bodies: usize,
}

impl path_search::PathWorld for Flat<'_> {
    fn sweep(&mut self, from: Vec3, to: Vec3, half: Vec3) -> Option<path_search::Obstacle> {
        self.sweeps += 1;
        let o = self.physics.sweep_obstacle(from, to, half, None);
        self.bodies += usize::from(o.is_some());
        o
    }
    fn ground(&mut self, at: Vec3) -> Option<(Vec3, Option<u64>)> {
        Some((at.with_y(0.0), None))
    }
    fn hints(&mut self, _: u64) -> Vec<(u32, Vec3)> {
        self.hints.to_vec()
    }
    fn body(&mut self, id: u64) -> Option<path_search::Obstacle> {
        self.physics.obstacle(id)
    }
}

fn search(shape: &CollisionShape, hints: &[(u32, Vec3)], start: Vec3, goal: Vec3) -> String {
    let mut p = Physics::new();
    p.add_static(StaticKey::Object(0), shape, Vec3::ZERO, Quat::IDENTITY);
    p.step(0.01);
    let mut world = Flat {
        physics: &p,
        hints,
        sweeps: 0,
        bodies: 0,
    };
    let size = path_search::Size {
        width: 3.2,
        height: 2.6,
    };
    let way = path_search::find(&mut world, size, start, goal);
    let length = way.as_ref().map(|w| {
        w.points
            .windows(2)
            .map(|p| p[0].distance(p[1]))
            .sum::<f32>()
    });
    format!(
        "{} points, {:.0?} m, {} sweeps, {} met a body",
        way.as_ref().map_or(0, |w| w.points.len()),
        length,
        world.sweeps,
        world.bodies
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let Some(root) = args.next() else {
        eprintln!("usage: hintpath <install dir> [template]...");
        std::process::exit(2);
    };
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
    let all = path_search::read_hints(&db);
    let wanted: Vec<String> = args.collect();
    let mut ids: Vec<(String, u32)> = all
        .keys()
        .filter_map(|id| Some((names.get(id)?.to_string(), *id)))
        .filter(|(n, _)| wanted.is_empty() || wanted.contains(n))
        .collect();
    ids.sort();
    for (name, id) in ids {
        let hints = &all[&id];
        let Some(&model) = models.get(&id) else {
            println!("{name}: {} hints, no model", hints.len());
            continue;
        };
        let d = source.model(model)?;
        let m = Model::parse(&d.base)?;
        let points = path_search::hint_points(&m.nodes, hints);
        println!(
            "{name}: {} hints, {} placed on its model: {:.1?}",
            hints.len(),
            points.len(),
            points.iter().map(|p| p.1).collect::<Vec<_>>()
        );
        if wanted.is_empty() {
            continue;
        }
        let phy2 = Ucfx::parse(&d.base)?
            .top_level(*b"PHY2")
            .map(<[u8]>::to_vec);
        let Some(shape) = CollisionShape::for_model(&m, phy2.as_deref(), 0) else {
            println!("  no collision");
            continue;
        };
        let (lo, hi) = shape.bounds();
        println!("  box {:.1?} to {:.1?}", lo, hi);
        for (from, to) in [
            (
                Vec3::new(0.0, 0.0, lo.z - 40.0),
                Vec3::new(0.0, 0.0, hi.z + 40.0),
            ),
            (
                Vec3::new(lo.x - 40.0, 0.0, 0.0),
                Vec3::new(hi.x + 40.0, 0.0, 0.0),
            ),
            (
                Vec3::new(lo.x - 30.0, 0.0, lo.z - 30.0),
                Vec3::new(hi.x + 30.0, 0.0, hi.z + 30.0),
            ),
        ] {
            println!("  {:.0?} -> {:.0?}", from, to);
            println!("    without hints: {}", search(&shape, &[], from, to));
            println!("    with hints:    {}", search(&shape, &points, from, to));
        }
    }
    Ok(())
}
