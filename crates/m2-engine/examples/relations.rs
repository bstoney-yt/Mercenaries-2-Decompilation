//! Prints how the templates regard each other (`Relationship` records),
//! faction templates first.
//!
//! ```text
//! cargo run --release -p m2-engine --example relations -- "<install dir>"
//! ```
//!
//! Prints template names and numbers only.

use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: relations <install dir>")?;
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
    let mut all = m2_engine::ai::relations(&db);
    all.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    for (a, b, r) in all {
        println!("{a} -> {b}: {r}");
    }
    Ok(())
}
