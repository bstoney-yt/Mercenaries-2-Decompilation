//! Finds the way by road between two places on the island
//! ([`m2_engine::road_graph`]) from your own installation, and prints how
//! long it is and where it goes.
//!
//! ```text
//! cargo run --release -p m2-engine --example roadway -- "<install dir>" <x>,<z> <x>,<z> [reach]
//! ```
//!
//! Prints numbers only.

use std::path::Path;

use m2_engine::Install;
use m2_engine::road_graph::RoadGraph;
use m2_engine::world::WorldIndex;
use m2_math::glam::Vec3;

fn spot(s: &str) -> Result<Vec3, Box<dyn std::error::Error>> {
    let (x, z) = s.split_once(',').ok_or("a place is <x>,<z>")?;
    Ok(Vec3::new(x.trim().parse()?, 0.0, z.trim().parse()?))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(root), Some(a), Some(b)) = (args.first(), args.get(1), args.get(2)) else {
        eprintln!("usage: roadway <install dir> <x>,<z> <x>,<z> [reach]");
        std::process::exit(2);
    };
    let (from, to) = (spot(a)?, spot(b)?);
    let reach: f32 = args.get(3).map_or(Ok(400.0), |r| r.parse())?;
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let world = WorldIndex::build(&mut source)?;
    let drivable = world.roads.iter().filter(|r| r.drivable()).count();
    let graph = RoadGraph::new(&world.roads);
    println!(
        "{} roads, {drivable} drivable, {} road points",
        world.roads.len(),
        graph.len()
    );
    for (name, at) in [("start", from), ("goal", to)] {
        let near: Vec<_> = world
            .roads
            .iter()
            .filter(|r| r.points.iter().any(|p| p.with_y(0.0).distance(at) <= reach))
            .collect();
        let drivable = near.iter().filter(|r| r.drivable()).count();
        println!(
            "roads within {reach} m of the {name}: {} ({drivable} drivable)",
            near.len()
        );
    }
    let Some(way) = graph.way(from, to, reach) else {
        println!("no way by road within {reach} m of both ends");
        return Ok(());
    };
    let length: f32 = way.windows(2).map(|w| w[0].distance(w[1])).sum();
    let (first, last) = (way[0], way[way.len() - 1]);
    println!(
        "{} points, {length:.0} m by road; on at {:.0?} ({:.0} m off), off at {:.0?} ({:.0} m short)",
        way.len(),
        first.to_array(),
        first.with_y(0.0).distance(from),
        last.to_array(),
        last.with_y(0.0).distance(to)
    );
    let top = way.iter().map(|p| p.y).fold(f32::MIN, f32::max);
    println!("highest {top:.0} m");
    for w in way
        .windows(2)
        .filter(|w| w[0].distance(w[1]) > m2_engine::road_graph::LINK * 4.0)
    {
        println!(
            "  long step {:.0} m: {:.0?} to {:.0?}",
            w[0].distance(w[1]),
            w[0].to_array(),
            w[1].to_array()
        );
    }
    for p in way.iter().step_by((way.len() / 12).max(1)) {
        println!("  {:.0?}", p.to_array());
    }
    Ok(())
}
