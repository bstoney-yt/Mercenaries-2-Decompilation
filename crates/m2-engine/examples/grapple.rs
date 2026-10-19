//! Lists what the template database holds for the grapple: every template
//! whose name has `text` in it (default `grapple`), each component that
//! has a record for it, and that record's fields (name hash, type, value),
//! plus the component tables whose own name has `text` in it.
//!
//! ```text
//! cargo run --release -p m2-engine --example grapple -- "<install dir>" [text]
//! ```
//!
//! Prints names, hashes and numbers only.

use std::path::Path;

use m2_assets::layer::{LAYER_TYPE, Layer, TEMPLATE_DB_TYPE};
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: grapple <install dir> [text]");
        std::process::exit(2);
    };
    let text = args
        .get(1)
        .map_or("grapple".to_owned(), |t| t.to_ascii_lowercase());
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
    // `hijack`: the templates a hijack script takes (their entrances), of
    // those with a `_HelicopterPhysics` record: the helicopters the hook
    // can end in a hijack of.
    if args.get(1).map(String::as_str) == Some("hijack") {
        let names = db.names();
        let models = db.hashes("ModelName");
        let helis: std::collections::HashSet<u32> = db
            .component("_HelicopterPhysics")
            .and_then(|c| c.records())
            .unwrap_or_default()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        let hijack = m2_engine::hijack::read(&db);
        let mut rows: Vec<String> = hijack
            .keys()
            .filter(|id| helis.contains(id))
            .map(|id| {
                format!(
                    "{}\tmodel {:08x}",
                    names.get(id).copied().unwrap_or("?"),
                    models.get(id).copied().unwrap_or(0)
                )
            })
            .collect();
        rows.sort();
        println!(
            "{} hijackable templates, {} of them helicopters",
            hijack.len(),
            rows.len()
        );
        for r in rows {
            println!("{r}");
        }
        return Ok(());
    }
    // `where <text>`: every layer's entities whose template's name holds
    // the text, and where they stand (to find a crewed helicopter).
    if args.get(1).map(String::as_str) == Some("where") {
        let want = args
            .get(2)
            .map_or(String::new(), |t| t.to_ascii_lowercase());
        let templates = db.names();
        let index = source.archive().index().clone();
        for a in index
            .assets()
            .iter()
            .filter(|a| index.type_hash(a) == LAYER_TYPE)
        {
            let Ok(bytes) = source.asset(a.name_hash, LAYER_TYPE) else {
                continue;
            };
            let Ok(layer) = Layer::parse(&bytes) else {
                continue;
            };
            for (_, p) in layer.placements() {
                let Some(t) = templates.get(&p.parent) else {
                    continue;
                };
                if t.to_ascii_lowercase().contains(&want) {
                    let [x, y, z] = p.position;
                    println!("{t}\t{x:.1}\t{y:.1}\t{z:.1}\tlayer {:08x}", a.name_hash);
                }
            }
        }
        return Ok(());
    }
    for c in &db.components {
        if c.name.to_ascii_lowercase().contains(&text) {
            println!(
                "component {} ({} records, {} fields)",
                c.name,
                c.count,
                c.fields.len()
            );
            let names = db.names();
            for (id, r) in c.records().unwrap_or_default() {
                println!(
                    "  record of {:?} ({id:#x}): {}",
                    names.get(&id),
                    fields(c, r)
                );
            }
        }
    }
    let mut names: Vec<(u32, &str)> = db
        .names()
        .into_iter()
        .filter(|(_, n)| n.to_ascii_lowercase().contains(&text))
        .collect();
    names.sort_by_key(|&(_, n)| n);
    for (id, name) in names {
        println!("template {name} ({id:#x})");
        for c in &db.components {
            let Some(records) = c.records() else {
                continue;
            };
            let Some((_, r)) = records.iter().find(|(e, _)| *e == id) else {
                continue;
            };
            println!("  {}: {}", c.name, fields(c, r));
        }
    }
    Ok(())
}

/// A record's fields: name hash, type unless a float, value.
fn fields(c: &m2_assets::layer::Component<'_>, r: &[u8]) -> String {
    c.fields
        .iter()
        .map(|f| {
            let at = usize::from(f.offset);
            let v = r
                .get(at..at + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            match v {
                Some(v) if f.ty == 7 => format!("{:08x}={}", f.name_hash, f32::from_bits(v)),
                Some(v) => format!("{:08x}/{}={v:x}", f.name_hash, f.ty),
                None => format!("{:08x}=-", f.name_hash),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
