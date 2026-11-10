//! Prints the seats and entrances of every vehicle template whose name
//! contains the second argument (case-insensitive), as
//! [`m2_engine::seats::read_vehicles`] reads them: for checking why an
//! entrance leads to no seat.
//!
//! ```text
//! cargo run --release -p m2-engine --example seatdump -- "<install dir>" scooter
//! ```
//!
//! Prints template names and hashes only.

use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(root), Some(want)) = (args.first(), args.get(1)) else {
        eprintln!("usage: seatdump <install dir> <name part>");
        std::process::exit(2);
    };
    let want = want.to_ascii_lowercase();
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let index = source.archive().index();
    let db_hash = index
        .assets()
        .iter()
        .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
        .map(|a| a.name_hash)
        .ok_or("no template database")?;
    let data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&data)?;
    let names = db.names();
    let vehicles = m2_engine::seats::read_vehicles(&db);
    let mut ids: Vec<_> = vehicles
        .keys()
        .filter(|id| {
            names
                .get(id)
                .is_some_and(|n| n.to_ascii_lowercase().contains(&want))
        })
        .copied()
        .collect();
    ids.sort();
    for id in ids {
        let v = &vehicles[&id];
        println!("{} ({id:08x})", names[&id]);
        for s in &v.seats {
            println!(
                "  seat {:08x} driver {} use {} hijackable {} rows {:x?}",
                s.name,
                s.driver,
                s.player_use,
                s.hijackable,
                s.rows
                    .iter()
                    .map(|r| (r.link_name, r.can_enter, r.can_exit))
                    .collect::<Vec<_>>()
            );
        }
        for e in &v.entrances {
            println!(
                "  entrance {:08x} hijack-only {}",
                e.name, e.params.hijack_only
            );
        }
    }
    Ok(())
}
