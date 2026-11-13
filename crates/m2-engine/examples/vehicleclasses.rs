//! Counts the template database's vehicle templates by the AI's class of
//! them ([`m2_engine::vehicle_class`]), with a few names of each.
//!
//! ```text
//! cargo run --release -p m2-engine --example vehicleclasses -- "<install dir>"
//! ```
//!
//! Prints counts and template names only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: vehicleclasses <install dir>")?;
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
    let names = db.names();
    for (what, classes) in [
        ("seat", m2_engine::vehicle_class::read_seats(&db)),
        ("vehicle", m2_engine::vehicle_class::read(&db)),
    ] {
        let mut by_class: BTreeMap<u8, Vec<&str>> = BTreeMap::new();
        for (id, class) in classes {
            by_class
                .entry(class)
                .or_default()
                .push(names.get(&id).copied().unwrap_or("?"));
        }
        for (class, mut names) in by_class {
            names.sort_unstable();
            println!(
                "{what} class {class}: {} templates, e.g. {:?}",
                names.len(),
                &names[..names.len().min(6)]
            );
        }
    }
    Ok(())
}
