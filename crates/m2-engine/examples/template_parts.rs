//! Prints every template-database component that holds a record for the
//! templates whose names contain a pattern, with each of the record's
//! fields: a survey for finding which components a prop carries (its
//! `Ignitor`, its debris) and what they say.
//!
//! ```text
//! cargo run --release -p m2-engine --example template_parts -- "<install dir>" [--names <file>] <pattern>...
//! ```
//!
//! `--names` reads candidate field names, one a line, and prints a field
//! whose hash one of them makes by that name; others print as `#` and
//! their hash. Values print as floats, and as whole numbers too when the
//! word is not a normal float. Prints numbers and template names only.

use std::collections::HashMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::name_hash;
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return Err("usage: template_parts <install dir> [--names <file>] <pattern>...".into());
    }
    let root = args.remove(0);
    let mut dictionary: HashMap<u32, String> = HashMap::new();
    if let Some(i) = args.iter().position(|a| a == "--names") {
        let file = args.get(i + 1).ok_or("--names needs a file")?.clone();
        args.drain(i..i + 2);
        for line in std::fs::read_to_string(file)?.lines() {
            let n = line.trim();
            if !n.is_empty() {
                dictionary.insert(name_hash(n), n.to_owned());
            }
        }
    }
    let patterns: Vec<String> = args.iter().map(|p| p.to_ascii_lowercase()).collect();
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
    let wanted: HashMap<u32, &str> = names
        .iter()
        .filter(|(id, n)| {
            let n = n.to_ascii_lowercase();
            patterns.iter().any(|p| {
                n.contains(p.as_str())
                    || p.strip_prefix("0x") == Some(format!("{:08x}", **id).as_str())
            })
        })
        .map(|(&id, &n)| (id, n))
        .collect();
    let mut templates: Vec<(&str, u32)> = wanted.iter().map(|(&id, &n)| (n, id)).collect();
    templates.sort();
    for (template, id) in templates {
        println!("== {template} ({id:08x})");
        for c in &db.components {
            // A component with strings: its records are not walked, only
            // noted when the template's id is in its data.
            let Some(records) = c.records() else {
                if c.data.windows(4).any(|w| w == id.to_le_bytes()) {
                    println!("  {}: (has strings; not walked)", c.name);
                }
                continue;
            };
            let Some(&(_, r)) = records.iter().find(|(e, _)| *e == id) else {
                continue;
            };
            let fields: Vec<String> = c
                .fields
                .iter()
                .map(|f| {
                    let name = dictionary
                        .get(&f.name_hash)
                        .cloned()
                        .unwrap_or_else(|| format!("#{:08x}", f.name_hash));
                    let a = usize::from(f.offset);
                    let value = r.get(a..a + 4).map_or("-".into(), |b| {
                        let w = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                        if (1..32).contains(&f.bit_width) {
                            return format!("#{}", (w >> f.bit_shift) & ((1 << f.bit_width) - 1));
                        }
                        let v = f32::from_bits(w);
                        if v.is_normal() || w == 0 {
                            format!("{v:.3}")
                        } else {
                            format!("#{w:08x}")
                        }
                    });
                    format!("{name}={value}")
                })
                .collect();
            println!("  {}: {}", c.name, fields.join(" "));
        }
    }
    Ok(())
}
