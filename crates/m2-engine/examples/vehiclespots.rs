//! Finds where the static layers park the most land vehicles in your own
//! installation, for `--parked-settle-check`'s spots: the layers' vehicle
//! placements (`WorldIndex::templates`) that are cars or tanks, counted by
//! 300 m cell, the densest cells first with their middle and how many lie
//! within the game's 400 m reach of that middle.
//!
//! ```text
//! cargo run --release -p m2-engine --example vehiclespots -- "<install dir>" [cells]
//! ```
//!
//! Prints cell middles, counts and template names only.

use std::collections::HashMap;
use std::path::Path;

use m2_engine::world::WorldIndex;
use m2_engine::{Install, VehicleKind};

/// The cell the counts are made by, metres, and the reach the game loads
/// parked vehicles within (`game::VEHICLE_REACH`).
const CELL: f32 = 300.0;
const REACH: f32 = 400.0;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let Some(root) = args.next() else {
        return Err("usage: vehiclespots <install dir> [cells]".into());
    };
    let show: usize = args.next().map_or(Ok(12), |a| a.parse())?;
    let install = Install::find(Path::new(&root))?;
    let world = WorldIndex::build(&mut install.open_world()?)?;
    let land: Vec<_> = world
        .templates
        .iter()
        .filter(|t| matches!(t.kind, VehicleKind::Car | VehicleKind::Tank))
        .collect();
    let mut cells: HashMap<(i32, i32), usize> = HashMap::new();
    for t in &land {
        let key = (
            (t.position.x / CELL).floor() as i32,
            (t.position.z / CELL).floor() as i32,
        );
        *cells.entry(key).or_default() += 1;
    }
    let mut dense: Vec<((i32, i32), usize)> = cells.into_iter().collect();
    dense.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    println!("{} land vehicles placed by the layers", land.len());
    for &((cx, cz), n) in dense.iter().take(show) {
        let (x, z) = ((cx as f32 + 0.5) * CELL, (cz as f32 + 0.5) * CELL);
        let near: Vec<_> = land
            .iter()
            .filter(|t| {
                t.position
                    .with_y(0.0)
                    .distance(m2_math::glam::Vec3::new(x, 0.0, z))
                    <= REACH
            })
            .collect();
        let mut names: Vec<&str> = near.iter().map(|t| t.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        println!(
            "cell ({x:.0}, {z:.0}): {n} in the cell, {} within {REACH} m: {}",
            near.len(),
            names.join(", ")
        );
    }
    Ok(())
}
