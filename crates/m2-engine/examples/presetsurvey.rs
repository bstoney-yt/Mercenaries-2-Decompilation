//! Counts the island's placed objects by collision preset (building, prop,
//! loose prop; [`m2_engine::collision_layers::Preset`]), and what keeping
//! each model's shapes to split them again costs
//! ([`m2_engine::physics::ShapeSource::bytes`]): per model, over all the
//! models placed, and over those placed within `radius` metres of (x, z).
//!
//! ```text
//! cargo run -p m2-engine --example presetsurvey -- "<install dir>" -1460 2860 300
//! ```
//!
//! Prints numbers only.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use m2_assets::{Model, Ucfx};
use m2_engine::Install;
use m2_engine::collision_layers::Preset;
use m2_engine::physics::CollisionShape;
use m2_engine::world::WorldIndex;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 4 {
        eprintln!("usage: presetsurvey <install dir> <x> <z> <radius>");
        std::process::exit(2);
    }
    let (x, z, radius): (f32, f32, f32) = (a[1].parse()?, a[2].parse()?, a[3].parse()?);
    let install = Install::find(Path::new(&a[0]))?;
    let mut source = install.open_world()?;
    let world = WorldIndex::build(&mut source)?;
    let presets = &world.template_db.presets;
    let mut objects: HashMap<Preset, usize> = HashMap::new();
    let mut models: HashSet<u32> = HashSet::new();
    let mut near: HashSet<u32> = HashSet::new();
    for o in &world.objects {
        let p = presets.get(&o.template).copied().unwrap_or_default();
        *objects.entry(p).or_default() += 1;
        models.insert(o.model);
        let (dx, dz) = (o.position.x - x, o.position.z - z);
        if dx * dx + dz * dz <= radius * radius {
            near.insert(o.model);
        }
    }
    let mut bytes: HashMap<u32, usize> = HashMap::new();
    for &m in &models {
        let Ok(d) = source.model(m) else { continue };
        let Ok(parsed) = Model::parse(&d.base) else {
            continue;
        };
        let phy2 = Ucfx::parse(&d.base)
            .ok()
            .and_then(|u| u.top_level(*b"PHY2").map(<[u8]>::to_vec));
        let kept = CollisionShape::for_model(&parsed, phy2.as_deref(), 0)
            .and_then(|s| s.source.map(|s| s.bytes()))
            .unwrap_or(0);
        bytes.insert(m, kept);
    }
    let with: Vec<usize> = bytes.values().copied().filter(|&b| b > 0).collect();
    let total: usize = with.iter().sum();
    let near_total: usize = near.iter().filter_map(|m| bytes.get(m)).sum();
    println!(
        "placed objects by preset: building {}, prop {}, loose prop {}",
        objects.get(&Preset::Building).unwrap_or(&0),
        objects.get(&Preset::Prop).unwrap_or(&0),
        objects.get(&Preset::LooseProp).unwrap_or(&0)
    );
    println!(
        "models placed {}, with shapes kept {}: {} bytes in all, {} a model on average, {} at most",
        models.len(),
        with.len(),
        total,
        total / with.len().max(1),
        with.iter().max().unwrap_or(&0)
    );
    println!(
        "models placed within {radius} m of ({x}, {z}): {}, their shapes kept {} bytes",
        near.len(),
        near_total
    );
    Ok(())
}
