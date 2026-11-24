//! Lists the vehicle templates that carry a `PhysicsPropertyGravityScaler`
//! in your own installation, with its `Scale` and their physics kind,
//! grouped by scale.
//!
//! ```text
//! cargo run --release -p m2-engine --example vehiclegravity -- "<install dir>"
//! ```
//!
//! Prints template names, kinds and scales only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_engine::Install;
use m2_engine::world::WorldIndex;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(root) = std::env::args().nth(1) else {
        return Err("usage: vehiclegravity <install dir>".into());
    };
    let install = Install::find(Path::new(&root))?;
    let world = WorldIndex::build(&mut install.open_world()?)?;
    let db = &world.template_db;
    // Scale (as text, to group) -> (kind, name).
    let mut by: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let (mut vehicles, mut scaled) = (0, 0);
    for (&id, info) in &db.infos {
        let Some(kind) = info.vehicle else { continue };
        vehicles += 1;
        let Some(&s) = db.gravity_scales.get(&id) else {
            continue;
        };
        scaled += 1;
        let name = db
            .by_id(id)
            .map_or(format!("{id:#x}"), |(n, _)| n.to_owned());
        by.entry(format!("{s:.3}"))
            .or_default()
            .push((format!("{kind:?}"), name));
    }
    println!("{vehicles} vehicle templates, {scaled} with a gravity scale");
    for (s, mut list) in by {
        list.sort();
        let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
        for (k, _) in &list {
            *kinds.entry(k.as_str()).or_default() += 1;
        }
        println!("scale {s}: {} templates {kinds:?}", list.len());
        for (k, n) in &list {
            println!("  {k:<10} {n}");
        }
    }
    Ok(())
}
