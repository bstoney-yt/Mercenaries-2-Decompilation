//! Lists the template database's `SocialUse` records as
//! [`m2_engine::social_use::read`] reads them: each distinct record with
//! how many templates carry it, for checking the reader against the
//! game's data. Then, for each distinct set of seats those templates
//! carry, the clips the animation tables give a person getting on,
//! sitting and getting off each seat (`InVehicle` `EnterVehicle`, `Idle`,
//! `ExitVehicle`), by the template's `VehicleAnimationSet`.
//!
//! ```text
//! cargo run --release -p m2-engine --example socialuse -- "<install dir>"
//! ```
//!
//! Prints numbers only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::animation_table::{self as table, AnimationTable};
use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::Install;
use m2_engine::social_use;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: socialuse <install dir>")?;
    let install = Install::find(Path::new(&root))?;
    let mut source = install.open_world()?;
    let db_hash = {
        let index = source.archive().index();
        index
            .assets()
            .iter()
            .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
            .map(|a| a.name_hash)
            .ok_or("no template database")?
    };
    let data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&data)?;
    let records = social_use::read(&db);
    let mut distinct: BTreeMap<(u32, u32, u32, u32), usize> = BTreeMap::new();
    for r in records.values() {
        let key = (
            r.kind,
            r.range.to_bits(),
            r.rest.to_bits(),
            r.duration.to_bits(),
        );
        *distinct.entry(key).or_default() += 1;
    }
    println!(
        "{} templates, {} distinct records",
        records.len(),
        distinct.len()
    );
    println!("kind\trange\trest\tduration\ttemplates");
    for ((kind, range, rest, duration), n) in distinct {
        println!(
            "{kind}\t{}\t{}\t{}\t{n}",
            f32::from_bits(range),
            f32::from_bits(rest),
            f32::from_bits(duration)
        );
    }

    // The seats of the templates with a record, and their clips.
    let seats = m2_engine::seats::read_vehicles(&db);
    let sets = m2_engine::hijack::animation_sets(&db);
    let actions =
        AnimationTable::parse(&source.asset(table::ACTIONS, table::ANIMATION_TABLE_TYPE)?)?;
    let vehicles = AnimationTable::parse(
        &source.asset(table::VEHICLE_ANIMATIONS, table::ANIMATION_TABLE_TYPE)?,
    )?;
    let hex = |c: Option<u32>| c.map_or("-".to_owned(), |c| format!("{c:08x}"));
    let mut rows: BTreeMap<String, usize> = BTreeMap::new();
    let (mut seated, mut unseated) = (0, 0);
    for id in records.keys() {
        let Some(s) = seats.get(id).filter(|s| !s.seats.is_empty()) else {
            unseated += 1;
            continue;
        };
        seated += 1;
        let set = sets.get(id).copied();
        for seat in &s.seats {
            let clip = |action: &str| {
                set.and_then(|set| {
                    table::seat_clip(
                        &actions,
                        &vehicles,
                        "InVehicle",
                        action,
                        set,
                        Some(seat.name),
                    )
                })
            };
            let line = format!(
                "kind {}	set {}	seat {:08x}	entrances {}	enter {}	idle {}	exit {}",
                records[id].kind,
                set.map_or("-".to_owned(), |(c, n)| format!("{c:08x}/{n:08x}")),
                seat.name,
                seat.rows.iter().filter(|r| r.can_enter).count(),
                hex(clip("EnterVehicle")),
                hex(clip("Idle")),
                hex(clip("ExitVehicle")),
            );
            *rows.entry(line).or_default() += 1;
        }
    }
    println!("{seated} templates with seats, {unseated} without");
    for (line, n) in rows {
        println!("{line}	templates {n}");
    }
    Ok(())
}
