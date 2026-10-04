//! Lists the entities placed near a spot on the island, with the
//! components each has, to find what a base is made of (which buildings
//! house soldiers, which props blow up).
//!
//! ```text
//! cargo run --release -p m2-engine --example near -- "<install dir>" <x>,<z> [radius]
//! ```
//!
//! Prints names, hashes, positions and component names only.

use std::collections::HashSet;
use std::path::Path;

use m2_assets::layer::{LAYER_TYPE, TEMPLATE_DB_TYPE};
use m2_assets::{BlockContents, Layer};
use m2_engine::Install;
use m2_engine::world::member_stem;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(root), Some(spot)) = (args.first(), args.get(1)) else {
        eprintln!("usage: near <install dir> <x>,<z> [radius]");
        std::process::exit(2);
    };
    let (x, z) = spot.split_once(',').ok_or("spot is <x>,<z>")?;
    let (x, z): (f32, f32) = (x.trim().parse()?, z.trim().parse()?);
    let radius: f32 = args.get(2).map_or(Ok(80.0), |r| r.parse())?;
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let index = source.archive().index();

    let blocks: HashSet<usize> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == LAYER_TYPE)
        .map(|a| usize::from(a.block))
        .collect();
    let mut blocks: Vec<usize> = blocks.into_iter().collect();
    blocks.sort_unstable();
    let blocks: Vec<(usize, String)> = blocks
        .into_iter()
        .map(|b| (b, index.blocks()[b].path.clone()))
        .collect();
    // Template names whose template has each component.
    let db_hash = index
        .assets()
        .iter()
        .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
        .map(|a| a.name_hash);
    let db_data = match db_hash {
        Some(h) => source.asset(h, TEMPLATE_DB_TYPE)?,
        None => Vec::new(),
    };
    let db = Layer::parse(&db_data).ok();
    let template_components = |stem: &str| -> Vec<&str> {
        let Some(db) = &db else { return Vec::new() };
        let Some(id) = db
            .names()
            .into_iter()
            .find(|(_, n)| n.eq_ignore_ascii_case(stem))
            .map(|(id, _)| id)
        else {
            return Vec::new();
        };
        db.components
            .iter()
            .filter(|c| c.records().is_some_and(|r| r.iter().any(|(i, _)| *i == id)))
            .map(|c| c.name)
            .collect()
    };

    let mut found = 0;
    for (block, path) in blocks {
        let decoded = source.block(block)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if e.type_hash != LAYER_TYPE {
                continue;
            }
            let Ok(layer) = Layer::parse(data) else {
                continue;
            };
            let names = layer.names();
            let mut near: Vec<_> = layer
                .placements()
                .into_iter()
                .map(|(id, p)| {
                    let d = ((p.position[0] - x).powi(2) + (p.position[2] - z).powi(2)).sqrt();
                    (d, id, p)
                })
                .filter(|(d, _, _)| *d < radius)
                .collect();
            if near.is_empty() {
                continue;
            }
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            println!(
                "== layer {:08x} in {path}: {} near",
                e.name_hash,
                near.len()
            );
            for (d, id, p) in near {
                let name = names.get(&id).copied().unwrap_or("");
                let has: Vec<&str> = layer
                    .components
                    .iter()
                    .filter(|c| c.records().is_some_and(|r| r.iter().any(|(i, _)| *i == id)))
                    .map(|c| c.name)
                    .filter(|n| !matches!(*n, "Transform" | "Name"))
                    .collect();
                let stem = member_stem(name);
                println!(
                    "  {id:08x} {d:5.1} m at [{:.0}, {:.0}, {:.0}] {name:?}: {}; template {:?}",
                    p.position[0],
                    p.position[1],
                    p.position[2],
                    has.join(" "),
                    template_components(stem).join(" "),
                );
                found += 1;
            }
        }
    }
    println!("{found} entities within {radius} m");
    Ok(())
}
