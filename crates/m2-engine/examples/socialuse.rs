//! Lists the template database's `SocialUse` records as
//! [`m2_engine::social_use::read`] reads them: each distinct record with
//! how many templates carry it, for checking the reader against the
//! game's data.
//!
//! ```text
//! cargo run --release -p m2-engine --example socialuse -- "<install dir>"
//! ```
//!
//! Prints numbers only.

use std::collections::BTreeMap;
use std::path::Path;

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
    Ok(())
}
