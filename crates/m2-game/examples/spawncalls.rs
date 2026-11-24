//! Surveys the scripts' `Pg.Spawn(template, x, y, z, yaw, ...)` calls from
//! your own installation, read statically from the compiled scripts: for
//! each call, its template (when a string constant) and what the template
//! is (a vehicle and its physics kind, a person, a prop), and whether its
//! position and yaw are constants or computed at run time. Counts every
//! other library call whose name has `Spawn` in it too.
//!
//! ```text
//! cargo run --release -p m2-game --example spawncalls -- "<install dir>"
//! ```
//!
//! Prints counts, template names and script hashes only. A computed
//! position (a local, a parameter, another call's result) can only be
//! read in a running game; those sites are listed for collecting there.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use m2_assets::lua::{Arg, Chunk, SCRIPT_TYPE};
use m2_assets::{BlockContents, WadArchive};
use m2_engine::Install;
use m2_engine::world::WorldIndex;

/// What one argument is, short.
fn kind(a: Option<&Arg>) -> String {
    match a {
        Some(Arg::Number(n)) => format!("{n}"),
        Some(Arg::Str(s)) => format!("{s:?}"),
        Some(Arg::Global(g)) => format!("<{g}>"),
        Some(Arg::Other) => "?".into(),
        Some(Arg::Nil) => "nil".into(),
        Some(Arg::Bool) => "bool".into(),
        Some(Arg::Table) => "{}".into(),
        Some(Arg::Function) => "fn".into(),
        None => "-".into(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(root) = std::env::args().nth(1) else {
        return Err("usage: spawncalls <install dir>".into());
    };
    let install = Install::find(Path::new(&root))?;
    let world = WorldIndex::build(&mut install.open_world()?)?;
    let db = &world.template_db;
    let what = |t: &str| match db.get(t) {
        Some(m2_engine::TemplateInfo {
            vehicle: Some(v), ..
        }) => format!("vehicle {v:?}"),
        Some(i) if i.person => "person".to_owned(),
        Some(i) if i.model.is_some() => "prop".to_owned(),
        Some(_) => "no model".to_owned(),
        None => "not a template".to_owned(),
    };

    let path = Path::new(&root).join("data").join("vz.wad");
    let mut wad = WadArchive::open(BufReader::new(File::open(&path)?))?;
    let mut scripts: Vec<(u32, Chunk)> = Vec::new();
    for block in 0..wad.index().blocks().len() {
        let decoded = wad.read_block(block)?;
        let Ok(contents) = BlockContents::parse(&decoded) else {
            continue;
        };
        for (entry, data) in contents.iter() {
            if entry.type_hash == SCRIPT_TYPE
                && let Ok(c) = Chunk::from_asset(data)
            {
                scripts.push((entry.name_hash, c));
            }
        }
    }
    println!("{} scripts", scripts.len());

    let mut others: BTreeMap<String, usize> = BTreeMap::new();
    // (template kind, position constant?) -> count
    let mut tally: BTreeMap<(String, &str), usize> = BTreeMap::new();
    let mut computed = Vec::new();
    for (hash, chunk) in &scripts {
        // `calls` reads the main function and every nested one.
        {
            for call in chunk.main.calls() {
                let fi = call.site.0;
                if call.callee != "Pg.Spawn" {
                    if call.callee.contains("Spawn") {
                        *others.entry(call.callee.clone()).or_default() += 1;
                    }
                    continue;
                }
                let args = call.args.clone().unwrap_or_default();
                let template = match args.first() {
                    Some(Arg::Str(s)) => Some(s.clone()),
                    _ => None,
                };
                let of = template
                    .as_deref()
                    .map_or("computed template".to_owned(), what);
                let constant = (1..=4).all(|k| matches!(args.get(k), Some(Arg::Number(_))));
                let place = if call.args.is_none() {
                    "unknown count"
                } else if constant {
                    "constant"
                } else {
                    "computed"
                };
                *tally.entry((of.clone(), place)).or_default() += 1;
                let line = format!(
                    "{hash:08x} fn{fi} pc{}: Pg.Spawn({}, {}, {}, {}, {}{}) [{of}]",
                    call.site.2,
                    template
                        .as_deref()
                        .map_or("?".to_owned(), |t| format!("{t:?}")),
                    kind(args.get(1)),
                    kind(args.get(2)),
                    kind(args.get(3)),
                    kind(args.get(4)),
                    if args.len() > 5 {
                        format!(", +{} more", args.len() - 5)
                    } else {
                        String::new()
                    },
                );
                if constant {
                    println!("const {line}");
                } else {
                    computed.push(line);
                }
            }
        }
    }
    println!("\nPg.Spawn sites by what they make and where:");
    for ((of, place), n) in &tally {
        println!("  {n:4}  {of:<28} {place}");
    }
    println!("\nsites with a computed place or template (read in a running game):");
    for line in &computed {
        println!("  {line}");
    }
    println!("\nother Spawn calls:");
    for (callee, n) in &others {
        println!("  {n:4}  {callee}");
    }
    Ok(())
}
