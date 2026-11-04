//! Lists the boats the state layers place (`WorldIndex::state_vehicles`
//! whose template is a boat): each one's template, layer and place, so a
//! contract's boats can be checked against where the alpha loads them.
//!
//! ```text
//! cargo run --release -p m2-engine --example stateboats -- "<install dir>" [layer name]
//! ```
//!
//! Prints names, hashes and positions only.

use std::path::Path;

use m2_assets::name_hash;
use m2_engine::world::WorldIndex;
use m2_engine::{Install, VehicleKind};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: stateboats <install dir> [layer name]");
        std::process::exit(2);
    };
    let word = args.get(1).cloned();
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let world = WorldIndex::build(&mut source)?;
    let mut rows: Vec<_> = world
        .state_vehicles
        .values()
        .filter(|v| v.kind == VehicleKind::Boat)
        .map(|v| (format!("{:08x}", v.layer), v))
        .filter(|(_, v)| word.as_ref().is_none_or(|w| v.layer == name_hash(w)))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    println!("layer\tentity\ttemplate\tx\ty\tz\tmodel");
    for (layer, v) in &rows {
        println!(
            "{layer}\t{:08x}\t{}\t{:.1}\t{:.1}\t{:.1}\t{}",
            v.entity,
            v.name,
            v.position.x,
            v.position.y,
            v.position.z,
            v.model.map_or("-".into(), |m| format!("{m:08x}")),
        );
    }
    println!("{} boats", rows.len());
    // And those out on their own patrols ([`m2_engine::world::TrafficPoint`]).
    println!("patrol	entity	template	static	layer	x	z	points	haste");
    let mut n = 0;
    for t in &world.traffic {
        if t.spawner.seconds_per_cycle.is_finite() {
            continue;
        }
        n += 1;
        println!(
            "patrol	{:08x}	{}	{}	{:08x}	{:.0}	{:.0}	{}	{:?}",
            t.entity,
            t.vehicles[0].0,
            t.static_layer,
            t.layer.unwrap_or(0),
            t.position.x,
            t.position.z,
            t.route.len(),
            t.haste,
        );
    }
    println!("{n} boats on patrol");
    Ok(())
}
