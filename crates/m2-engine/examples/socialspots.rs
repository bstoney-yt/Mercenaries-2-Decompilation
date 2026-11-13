//! Lists where on the island the usable things with seats (benches,
//! chairs, telephones: [`m2_engine::social_use`]) stand thickest, for
//! picking a spot to watch people use them: the island cut into squares,
//! the fullest first, each with the middle of its things, how many of
//! each kind, and how many people the layers place within 100 m.
//!
//! ```text
//! cargo run --release -p m2-engine --example socialspots -- "<install dir>" [square metres]
//! ```
//!
//! Prints numbers only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_engine::{Install, WorldIndex};
use m2_math::glam::Vec3;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = args
        .first()
        .ok_or("usage: socialspots <install dir> [square metres]")?;
    let square: f32 = args.get(1).map_or(Ok(60.0), |s| s.parse())?;
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let world = WorldIndex::build(&mut source)?;
    let db = &world.template_db;
    let mut cells: BTreeMap<(i32, i32), Vec<(Vec3, u32)>> = BTreeMap::new();
    let mut total = 0;
    for o in &world.objects {
        let Some(record) = db.social_use.get(&o.template) else {
            continue;
        };
        if db
            .vehicle_seats
            .get(&o.template)
            .is_none_or(|s| s.seats.is_empty())
        {
            continue;
        }
        total += 1;
        let cell = (
            (o.position.x / square).floor() as i32,
            (o.position.z / square).floor() as i32,
        );
        cells
            .entry(cell)
            .or_default()
            .push((o.position, record.kind));
    }
    println!(
        "{total} placed usable things with seats, in {} squares of {square} m",
        cells.len()
    );
    let mut full: Vec<_> = cells.into_values().collect();
    full.sort_by_key(|c| std::cmp::Reverse(c.len()));
    println!("things\tx\ty\tz\tkinds (kind:count)\tplaced people within 100 m");
    for things in full.iter().take(15) {
        let mid = things.iter().map(|t| t.0).sum::<Vec3>() / things.len() as f32;
        let mut kinds: BTreeMap<u32, usize> = BTreeMap::new();
        for t in things {
            *kinds.entry(t.1).or_default() += 1;
        }
        let kinds: Vec<String> = kinds.iter().map(|(k, n)| format!("{k}:{n}")).collect();
        let people = world
            .spawns
            .iter()
            .filter(|s| s.position.distance(mid) < 100.0)
            .count();
        println!(
            "{}\t{:.0}\t{:.0}\t{:.0}\t{}\t{people}",
            things.len(),
            mid.x,
            mid.y,
            mid.z,
            kinds.join(" ")
        );
    }
    Ok(())
}
