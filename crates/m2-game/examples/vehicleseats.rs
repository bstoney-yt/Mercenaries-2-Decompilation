//! Prints the seat data of vehicle templates of your own installation:
//! each seat (`SeatLink` name, driver/gunner/hijackable/player-use bits and
//! its `EntranceToSeat` rows) and each entrance (`EntranceLink` name and
//! its parameters).
//!
//! ```text
//! cargo run --release -p m2-game --example vehicleseats -- <vz.wad> <template name part>...
//! ```
//!
//! Matches template names containing any part given (any case). Prints
//! names, hashes and flags only.

use std::fs::File;
use std::io::BufReader;

use m2_assets::layer::TEMPLATE_DB_TYPE;
use m2_assets::{BlockContents, Layer, WadArchive};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `--hash <names>...`: the engine's name hash of each (to match an
    // address's constants), nothing read.
    if args.first().is_some_and(|a| a == "--hash") {
        for n in &args[1..] {
            println!("{:08x} {n}", m2_assets::name_hash(n));
        }
        return Ok(());
    }
    let wad = args
        .first()
        .ok_or("usage: vehicleseats <vz.wad> <template name part>...")?;
    let parts: Vec<String> = args[1..].iter().map(|a| a.to_ascii_lowercase()).collect();
    let mut archive = WadArchive::open(BufReader::new(File::open(wad)?))?;
    let blocks = archive.index().blocks().len();
    for block in 0..blocks {
        let decoded = archive.read_block(block)?;
        let Ok(contents) = BlockContents::parse(&decoded) else {
            continue;
        };
        for (entry, asset) in contents.iter() {
            if entry.type_hash != TEMPLATE_DB_TYPE {
                continue;
            }
            let Ok(db) = Layer::parse(asset) else {
                continue;
            };
            let seats = m2_engine::seats::read_vehicles(&db);
            let mut names: Vec<(u32, &str)> = db.names().into_iter().collect();
            names.sort_by_key(|&(_, n)| n.to_ascii_lowercase());
            for (id, name) in names {
                let lower = name.to_ascii_lowercase();
                if !parts.iter().any(|p| lower.contains(p.as_str())) {
                    continue;
                }
                let Some(v) = seats.get(&id) else {
                    println!("{name} ({id:#x}): no seat data");
                    continue;
                };
                println!("{name} ({id:#x}):");
                for s in &v.seats {
                    println!(
                        "  seat {:08x} hardpoint {:x?} driver {} gunner {} hijackable {} player_use {} rows {:x?}",
                        s.name, s.hardpoint, s.driver, s.gunner, s.hijackable, s.player_use, s.rows
                    );
                }
                for e in &v.entrances {
                    println!(
                        "  entrance {:08x} hardpoint {:x?} {:?}",
                        e.name, e.hardpoint, e.params
                    );
                }
            }
        }
    }
    Ok(())
}
