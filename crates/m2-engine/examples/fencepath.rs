//! Runs a tank-sized path search across a line of a fence's real model, on
//! flat ground, with its shapes' 360 layers and with every shape in the
//! way, and prints what each search finds.
//!
//! ```text
//! cargo run --release -p m2-engine --example fencepath -- "<install dir>" _global_fencechainlong
//! ```
//!
//! Prints numbers and template names only.

use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::{Model, Ucfx};
use m2_engine::physics::{CollisionShape, PathSolid, StaticKey};
use m2_engine::{Install, Physics, path_search};
use m2_math::glam::{Quat, Vec3};

struct Flat<'a> {
    physics: &'a Physics,
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
        Vec::new()
    }
    fn body(&mut self, id: u64) -> Option<path_search::Obstacle> {
        self.physics.obstacle(id)
    }
}

/// A line of `count` of `shape` along x, each `step` m on.
fn search(shape: &CollisionShape, count: usize, step: f32) -> String {
    let mut p = Physics::new();
    for k in 0..count {
        let x = (k as f32 - (count as f32 - 1.0) * 0.5) * step;
        p.add_static(
            StaticKey::Object(k),
            shape,
            Vec3::new(x, 0.0, 0.0),
            Quat::IDENTITY,
        );
    }
    p.step(0.01);
    let mut world = Flat {
        physics: &p,
        sweeps: 0,
        bodies: 0,
    };
    let size = path_search::Size {
        width: 3.2,
        height: 2.6,
    };
    let way = path_search::find(
        &mut world,
        size,
        Vec3::new(0.0, 0.0, -40.0),
        Vec3::new(0.0, 0.0, 40.0),
    );
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
        eprintln!("usage: fencepath <install dir> <template>...");
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
    for want in args {
        let Some(id) = names.iter().find(|(_, n)| **n == want).map(|(id, _)| *id) else {
            println!("{want}: no such template");
            continue;
        };
        let d = source.model(models[&id])?;
        let m = Model::parse(&d.base)?;
        let phy2 = Ucfx::parse(&d.base)?
            .top_level(*b"PHY2")
            .map(<[u8]>::to_vec);
        let Some(shape) = CollisionShape::for_model(&m, phy2.as_deref(), 0) else {
            println!("{want}: no collision");
            continue;
        };
        let (lo, hi) = shape.bounds();
        let step = (hi.x - lo.x).max(1.0);
        let count = (120.0 / step).ceil() as usize;
        let mut every = shape.clone();
        every.path = PathSolid::All;
        println!(
            "{want}: {:.1} m wide, {count} in line, its path view {:?}",
            hi.x - lo.x,
            shape.path
        );
        println!("  every shape in the way: {}", search(&every, count, step));
        println!("  by its 360 layers:      {}", search(&shape, count, step));
    }
    Ok(())
}
