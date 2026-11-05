//! Finds which template database components hold a given word (a
//! template id, a model or name hash) in their records: the component,
//! the field at that offset, and the template whose record it is. A
//! survey for what attaches a template (the engines' heat haze) to
//! others.
//!
//! ```text
//! cargo run --release -p m2-engine --example refsto -- "<install dir>" <word hex | name:NAME>...
//! ```
//!
//! Prints names, hashes and offsets only.

use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = args
        .first()
        .ok_or("usage: refsto <install dir> <word>...")?;
    let words: Vec<u32> = args[1..]
        .iter()
        .filter_map(|k| match k.strip_prefix("name:") {
            Some(n) => Some(m2_assets::name_hash(n)),
            None => u32::from_str_radix(k, 16).ok(),
        })
        .collect();
    let install = Install::find(Path::new(root))?;
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
    let name = |id: u32| names.get(&id).copied().unwrap_or("?");
    for c in &db.components {
        let field_at = |at: usize| {
            c.fields
                .iter()
                .find(|f| usize::from(f.offset) == at)
                .map_or(format!("+0x{at:x}"), |f| format!("{:08x}", f.name_hash))
        };
        match c.records() {
            Some(records) => {
                for (id, r) in records {
                    for at in (0..r.len().saturating_sub(3)).step_by(4) {
                        let w = u32::from_le_bytes([r[at], r[at + 1], r[at + 2], r[at + 3]]);
                        if words.contains(&w) {
                            println!(
                                "{} field {}: {w:08x} in template {id:08x} {}",
                                c.name,
                                field_at(at),
                                name(id)
                            );
                        }
                    }
                }
            }
            None => {
                let d = c.data;
                for at in (0..d.len().saturating_sub(3)).step_by(4) {
                    let w = u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]]);
                    if words.contains(&w) {
                        println!("{} (raw) @0x{at:x}: {w:08x}", c.name);
                    }
                }
            }
        }
    }
    Ok(())
}
