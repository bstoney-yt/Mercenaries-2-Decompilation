//! Lists the clip every seat of every vehicle template is sat in: what
//! the animation tables give `InVehicle`, `Idle` for the seat
//! ([`m2_assets::animation_table::seat_clip_row`]), from your own
//! installation's `vz.wad`.
//!
//! ```text
//! cargo run --release -p m2-game --example seats -- <vz.wad> > seats.txt
//! ```
//!
//! One line a seat: the template, the seat's name hash, the clip, the
//! vehicle table's row and whether that row is the vehicle's own, its
//! class's, or the table's catch-all (row 1, every cell any), and
//! whether the clip is in the archive's vehicle clip groups. Then the
//! counts. Prints names, hashes and counts only.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;

use m2_assets::WadArchive;
use m2_assets::animation_table::{self as table, ANY};
use m2_game::hijack::ClipTimes;

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: seats <vz.wad>");
        return ExitCode::FAILURE;
    };
    match run(&path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut wad = WadArchive::open(BufReader::new(File::open(path)?))?;
    let times = ClipTimes::read(&mut wad)?;
    let (actions, vehicles) = times.tables.as_ref().ok_or("no animation tables")?;
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut clips: BTreeMap<u32, usize> = BTreeMap::new();
    let mut names: Vec<&String> = times.sets.keys().collect();
    names.sort();
    for name in names {
        let set = times.sets[name];
        for &(seat, _) in times.seats.get(name).into_iter().flatten() {
            let found =
                table::seat_clip_row(actions, vehicles, "InVehicle", "Idle", set, Some(seat))
                    .or_else(|| {
                        table::seat_clip_row(actions, vehicles, "InVehicle", "Idle", set, None)
                    });
            let Some((clip, row)) = found else {
                println!("{name}\tseat {seat:08x}\tnone");
                *counts.entry("none").or_default() += 1;
                continue;
            };
            let cell = |c: &str| vehicles.column(c).and_then(|c| vehicles.cell(row, c));
            let named = |c: &str| cell(c).is_some_and(|v| v != ANY);
            // Why a seat ends on the catch-all: the stance and action
            // table has no `Idle` row naming it, or the handle its row
            // gives has no row in the vehicle table.
            let handle = actions.lookup(
                &[
                    ("Stance", Some(m2_assets::name_hash("InVehicle"))),
                    ("Action", Some(m2_assets::name_hash("Idle"))),
                    ("Seat", Some(seat)),
                ],
                "AnimationHandles",
            );
            let seat_named = actions
                .lookup_row(&[
                    ("Stance", Some(m2_assets::name_hash("InVehicle"))),
                    ("Action", Some(m2_assets::name_hash("Idle"))),
                    ("Seat", Some(seat)),
                ])
                .and_then(|r| actions.cell(r, actions.column("Seat")?))
                == Some(seat);
            let handle_rows = handle.is_some_and(|h| {
                let c = vehicles.column(table::HANDLE);
                (1..vehicles.rows()).any(|r| c.and_then(|c| vehicles.cell(r, c)) == Some(h))
            });
            let kind = if named("VehicleName") {
                "the vehicle's own"
            } else if named("VehicleClass") {
                "its class's"
            } else if named("SeatName") {
                "the seat's, any vehicle"
            } else if !seat_named {
                "the catch-all: no Idle row names the seat"
            } else if !handle_rows {
                "the catch-all: the seat's handle has no vehicle row"
            } else {
                "the catch-all"
            };
            let loaded = if times.lengths.contains_key(&clip) {
                "in a vehicle group"
            } else {
                "not in a vehicle group"
            };
            println!("{name}\tseat {seat:08x}\tclip {clip:08x}\trow {row}\t{kind}\t{loaded}");
            *counts.entry(kind).or_default() += 1;
            *clips.entry(clip).or_default() += 1;
        }
    }
    let total: usize = counts.values().sum();
    println!("{total} seats; {} distinct clips", clips.len());
    for (kind, n) in &counts {
        println!("  {n:5} {kind}");
    }
    Ok(())
}
