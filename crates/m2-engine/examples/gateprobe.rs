//! Probes a model's way through: its standing `PHY2` shapes' boxes (model
//! space), then, for each x across it, whether a box of the given width and
//! height swept along z at the model's base (plus a lift) passes.
//!
//! ```text
//! cargo run --release -p m2-engine --example gateprobe -- "<install dir>" _jungle_wallgate 3.8 4.2
//! ```
//!
//! Prints numbers and template names only.

use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::{Model, Ucfx};
use m2_engine::physics::{CollisionShape, StaticKey};
use m2_engine::{Install, Physics};
use m2_math::glam::{Quat, Vec3};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 4 {
        eprintln!("usage: gateprobe <install dir> <template> <width> <height>");
        std::process::exit(2);
    }
    let (width, height): (f32, f32) = (a[2].parse()?, a[3].parse()?);
    let install = Install::find(Path::new(&a[0]))?;
    let mut source = install.open_world()?;
    let index = source.archive().index();
    let db_hash = index
        .assets()
        .iter()
        .find(|x| index.type_hash(x) == TEMPLATE_DB_TYPE)
        .map(|x| x.name_hash)
        .ok_or("no template database")?;
    let data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&data)?;
    let names = db.names();
    let models = db.hashes("ModelName");
    let id = names
        .iter()
        .find(|(_, n)| **n == a[1])
        .map(|(id, _)| *id)
        .ok_or("no such template")?;
    let d = source.model(models[&id])?;
    let m = Model::parse(&d.base)?;
    let phy2 = Ucfx::parse(&d.base)?
        .top_level(*b"PHY2")
        .map(<[u8]>::to_vec);
    for (node, v, _, ai) in CollisionShape::shape_meshes(&m, phy2.as_deref()) {
        let lo = v.iter().copied().fold(Vec3::splat(f32::MAX), Vec3::min);
        let hi = v.iter().copied().fold(Vec3::splat(f32::MIN), Vec3::max);
        println!("shape node {node:3} ai {ai:?} box {lo:.1} .. {hi:.1}");
    }
    if let Some(p) = phy2
        .as_deref()
        .and_then(|d| m2_assets::physics::Physics::parse(d).ok())
    {
        let hid = |w: bool| {
            m.states
                .as_ref()
                .map(|s| s.hidden(&m.nodes, w))
                .unwrap_or_default()
        };
        let (h0, h1) = (hid(false), hid(true));
        for k in 0..p.shapes.len() {
            let node = p.parts[p.shapes[k].part].node as usize;
            let body = p.body_of(k);
            println!(
                "shape {k} part node {node} body {body:?} fixed {:?} hidden intact {:?} wrecked {:?} kind {:?}",
                body.map(|b| p.bodies[b].is_fixed()),
                h0.get(node),
                h1.get(node),
                std::mem::discriminant(&p.shapes[k].geometry)
            );
        }
        for (k, s) in p.shapes.iter().enumerate() {
            if let m2_assets::physics::Geometry::Mesh {
                vertices,
                triangles,
                flags,
            } = &s.geometry
            {
                let mut by: std::collections::BTreeMap<u16, (usize, Vec3, Vec3)> =
                    Default::default();
                for (t, f) in triangles.iter().zip(flags) {
                    let e =
                        by.entry(*f)
                            .or_insert((0, Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)));
                    e.0 += 1;
                    for &i in t {
                        let v = Vec3::from(vertices[i as usize]);
                        e.1 = e.1.min(v);
                        e.2 = e.2.max(v);
                    }
                }
                // Each 1 m column's lowest point a triangle covers there
                // (y over 9 shown as '9').
                let mut row = String::new();
                for c in -34..34 {
                    let (x0, x1) = (c as f32, c as f32 + 1.0);
                    let low = triangles
                        .iter()
                        .filter_map(|t| {
                            let v: Vec<Vec3> = t
                                .iter()
                                .map(|&i| Vec3::from(vertices[i as usize]))
                                .collect();
                            let (a, b) = (
                                v.iter().map(|p| p.x).fold(f32::MAX, f32::min),
                                v.iter().map(|p| p.x).fold(f32::MIN, f32::max),
                            );
                            (b >= x0 && a <= x1)
                                .then(|| v.iter().map(|p| p.y).fold(f32::MAX, f32::min))
                        })
                        .fold(f32::MAX, f32::min);
                    row.push(if low == f32::MAX {
                        ' '
                    } else {
                        char::from(b'0' + low.clamp(0.0, 9.0) as u8)
                    });
                }
                println!("shape {k} lowest from x -34: {row}");
                for (f, (n, lo, hi)) in by {
                    println!(
                        "shape {k} flags {:#x} tri flag {f:#06x}: {n} triangles, {lo:.1} .. {hi:.1}",
                        s.flags()
                    );
                }
            }
        }
    }
    let shape = CollisionShape::for_model(&m, phy2.as_deref(), 0).ok_or("no collision")?;
    let (lo, hi) = shape.bounds();
    println!("model box {lo:.1} .. {hi:.1}");
    let mut p = Physics::new();
    p.add_static(StaticKey::Object(0), &shape, Vec3::ZERO, Quat::IDENTITY);
    p.step(0.01);
    let half = Vec3::new(width, height - 0.25, 0.1) * 0.5;
    for lift in [0.0f32, 1.0, 2.0, 3.0] {
        let mut row = String::new();
        let mut x = lo.x - 2.0;
        while x <= hi.x + 2.0 {
            let y = lo.y + height * 0.5 + 0.25 + lift;
            let from = Vec3::new(x, y, lo.z - 10.0);
            let to = Vec3::new(x, y, hi.z + 10.0);
            row.push(if p.sweep_obstacle(from, to, half, None).is_some() {
                '#'
            } else {
                '.'
            });
            x += 1.0;
        }
        println!("lift {lift}: x {:.0}.. {row}", lo.x - 2.0);
    }
    Ok(())
}
