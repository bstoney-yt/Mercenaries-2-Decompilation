//! Prints the named fields of one template-database component for every
//! distinct record, with the templates sharing it: a survey for deciding
//! what a field means from its values.
//!
//! ```text
//! cargo run --release -p m2-engine --example fields -- "<install dir>" <component> <field>...
//! ```
//!
//! A field not yet named is given as `#` and its hash (`#d6988c68`). A
//! field the component lacks prints `-`; a bit field prints its bits. Values print as floats, and as
//! whole numbers too when the word is not a normal float. Prints numbers
//! and template names only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::name_hash;
use m2_engine::Install;

/// A field's name hash: `#` and eight hex digits for a field not yet
/// named, else the name's.
fn field_hash(name: &str) -> u32 {
    name.strip_prefix('#')
        .and_then(|h| u32::from_str_radix(h, 16).ok())
        .unwrap_or_else(|| name_hash(name))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [root, component, fields @ ..] = args.as_slice() else {
        return Err("usage: fields <install dir> <component> <field>...".into());
    };
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
    let c = db.component(component).ok_or("no such component")?;
    let at: Vec<Option<(usize, u8, u8)>> = fields
        .iter()
        .map(|n| {
            c.fields
                .iter()
                .find(|f| f.name_hash == field_hash(n))
                .map(|f| (usize::from(f.offset), f.bit_shift, f.bit_width))
        })
        .collect();
    for (n, a) in fields.iter().zip(&at) {
        println!(
            "# {n} @{}",
            a.map_or("-".to_owned(), |(a, shift, width)| {
                if (1..32).contains(&width) {
                    format!("{a:#x} bits {shift}..{}", shift + width)
                } else {
                    format!("{a:#x}")
                }
            })
        );
    }
    let mut groups: BTreeMap<Vec<u8>, Vec<&str>> = BTreeMap::new();
    for (id, r) in c.records().unwrap_or_default() {
        groups
            .entry(r.to_vec())
            .or_default()
            .push(names.get(&id).copied().unwrap_or("?"));
    }
    println!(
        "records {}, distinct {}",
        c.records().unwrap_or_default().len(),
        groups.len()
    );
    println!("first template\tcount\t{}", fields.join("\t"));
    for (r, ids) in &groups {
        let vals: Vec<String> = at
            .iter()
            .map(|a| {
                a.and_then(|(a, shift, width)| Some((r.get(a..a + 4)?, shift, width)))
                    .map_or("-".into(), |(b, shift, width)| {
                        let w = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                        if (1..32).contains(&width) {
                            return format!("#{}", (w >> shift) & ((1 << width) - 1));
                        }
                        let f = f32::from_bits(w);
                        if f.is_normal() || w == 0 {
                            format!("{f:.3}")
                        } else {
                            format!("#{w}")
                        }
                    })
            })
            .collect();
        println!("{}\t{}\t{}", ids[0], ids.len(), vals.join("\t"));
    }
    Ok(())
}
