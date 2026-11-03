//! Prints each tank's, boat's and helicopter's `PHY2` rigid bodies beside
//! its template's `Mass`: each body's mass, whether its record gives an
//! inertia or leaves it to the shapes (`f32::MAX`), and the kinds of the
//! shapes under it, which of its two chassis bodies (`CHASSIS` and
//! 0x75F1F74D) its model shows intact, and its tank `CenterOfMassOffset`.
//!
//! ```text
//! cargo run --release -p m2-engine --example vehiclebodies -- "<install dir>"
//! ```
//!
//! Prints numbers and template names only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::physics::{Geometry, Physics};
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(root) = std::env::args().nth(1) else {
        eprintln!("usage: vehiclebodies <install dir>");
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
    let tuning = m2_engine::tuning::read(&db, None);
    for comp in ["_TankPhysics", "_BoatPhysics", "_HelicopterPhysics"] {
        let Some(c) = db.component(comp) else {
            continue;
        };
        let mut seen: BTreeMap<u32, Vec<&str>> = BTreeMap::new();
        for (id, _) in c.records().unwrap_or_default() {
            let Some(&model) = models.get(&id) else {
                continue;
            };
            seen.entry(model)
                .or_default()
                .push(names.get(&id).copied().unwrap_or("?"));
        }
        println!("== {comp}: {} models", seen.len());
        for (model, ts) in &seen {
            let Ok(d) = source.model(*model) else {
                continue;
            };
            let Some(p) = m2_assets::Ucfx::parse(&d.base)
                .ok()
                .and_then(|u| u.top_level(*b"PHY2").map(<[u8]>::to_vec))
                .and_then(|d| Physics::parse(&d).ok())
            else {
                println!("{model:08x} {} no PHY2", ts[0]);
                continue;
            };
            let id = db
                .component(comp)
                .and_then(|c| {
                    c.records()
                        .unwrap_or_default()
                        .into_iter()
                        .find(|(id, _)| models.get(id) == Some(model))
                        .map(|(id, _)| id)
                })
                .unwrap_or(0);
            let tm = tuning.get(&id).and_then(|t| t.mass);
            let offset = tuning
                .get(&id)
                .and_then(|t| t.tank)
                .map(|t| t.centre_of_mass);
            println!(
                "{model:08x} {} (x{}) template Mass {tm:?} CenterOfMassOffset {offset:?}",
                ts[0],
                ts.len()
            );
            if let Ok(m) = m2_assets::model::Model::parse(&d.base) {
                let hidden = m
                    .states
                    .as_ref()
                    .map(|s| s.hidden(&m.nodes, false))
                    .unwrap_or_default();
                let wreck = m
                    .states
                    .as_ref()
                    .map(|s| s.hidden(&m.nodes, true))
                    .unwrap_or_default();
                for q in p.parts.iter().filter(|q| q.body.is_some()) {
                    if matches!(q.node_hash, 0x255e_ab53 | 0x75f1_f74d | 0x765c_d254) {
                        let n = q.node as usize;
                        println!(
                            "  chassis {:08x} body {:?} hidden intact {:?} wrecked {:?}",
                            q.node_hash,
                            q.body,
                            hidden.get(n),
                            wreck.get(n)
                        );
                    }
                }
            }
            let tree: Vec<String> = p
                .parts
                .iter()
                .enumerate()
                .map(|(i, q)| {
                    format!(
                        "{i}:n{}/{:08x}^{:?}b{:?}s{:?}",
                        q.node, q.node_hash, q.parent, q.body, q.shape
                    )
                })
                .collect();
            println!("  parts {}", tree.join(" "));
            for (bi, b) in p.bodies.iter().enumerate() {
                let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
                for (si, s) in p.shapes.iter().enumerate() {
                    if p.body_of(si) == Some(bi) {
                        let k = match s.geometry {
                            Geometry::Box { .. } => "box",
                            Geometry::Sphere => "sphere",
                            Geometry::Capsule { .. } => "capsule",
                            Geometry::Convex { .. } => "convex",
                            Geometry::Mesh { .. } => "mesh",
                        };
                        *kinds.entry(k).or_default() += 1;
                    }
                }
                let part = p.parts.iter().position(|q| q.body == Some(bi));
                println!(
                    "  body {bi} part {part:?} node {:08x} mass {} inertia {} com {:?} shapes {kinds:?}",
                    part.map_or(0, |q| p.parts[q].node_hash),
                    b.mass,
                    if b.inertia[0] == f32::MAX {
                        "shape".to_string()
                    } else {
                        format!("{:?}", b.inertia)
                    },
                    b.centre_of_mass
                );
            }
        }
    }
    Ok(())
}
