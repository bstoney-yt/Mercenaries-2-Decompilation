//! Loads every compiled script in a `.wad` from your own installation into
//! the script host, with every engine library stubbed, and reports what
//! fails and what the scripts call while they start.
//!
//! ```text
//! cargo run -p m2-game --example scripts -- <vz.wad> [script hash...]
//! ```
//!
//! First every module some script loads is imported; then every other
//! script runs once as an entry point (its top level only: it defines its
//! functions and loads its modules). With hashes, only those scripts run,
//! and the calls they make are listed in order. Without hashes it also
//! lists which entity components in the archive's layers name a script,
//! and how the scripts call each engine library function: argument counts
//! and kinds, the engine API to provide.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;
use std::rc::Rc;

use m2_assets::layer::LAYER_TYPE;
use m2_assets::lua::{Arg, Chunk, SCRIPT_TYPE};
use m2_assets::{BlockContents, Layer, WadArchive};
use m2_game::library::Library;
use m2_game::script::ScriptHost;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let [path, hashes @ ..] = args else {
        return Err("usage: scripts <archive.wad> [script hash...]".into());
    };
    let only: Vec<u32> = hashes
        .iter()
        .map(|h| u32::from_str_radix(h, 16))
        .collect::<Result<_, _>>()?;
    let mut wad = WadArchive::open(BufReader::new(File::open(path)?))?;
    let mut library = Library::default();
    let paths: Vec<String> = wad
        .index()
        .blocks()
        .iter()
        .map(|b| b.path.clone())
        .collect();
    let mut layers: Vec<(usize, Vec<u8>)> = Vec::new();
    for block in 0..paths.len() {
        let decoded = wad.read_block(block)?;
        let Ok(contents) = BlockContents::parse(&decoded) else {
            continue;
        };
        for (entry, data) in contents.iter() {
            if entry.type_hash == SCRIPT_TYPE {
                match Chunk::from_asset(data) {
                    Ok(c) => library.add(entry.name_hash, c),
                    Err(e) => println!("script {:08x}: {e}", entry.name_hash),
                }
            } else if entry.type_hash == LAYER_TYPE && only.is_empty() {
                layers.push((block, data.to_vec()));
            }
        }
    }
    println!("{} scripts", library.scripts().len());
    if only.is_empty() {
        script_references(&library, &paths, &layers);
        signatures(&library);
    }
    let library = Rc::new(library);

    let host = ScriptHost::new()?;
    host.stub_missing_globals()?;
    host.install_library(Rc::clone(&library), true)?;
    let mut failures = Vec::new();
    if only.is_empty() {
        let import: mlua::Function = host.lua().globals().get("import")?;
        for name in library.module_names() {
            if let Err(e) = import.call::<mlua::Value>(name) {
                failures.push(format!("module {name}: {e}"));
            }
        }
    }
    let modules: Vec<u32> = host.modules().iter().filter_map(|m| m.1).collect();
    let mut entries = 0;
    for script in library.scripts() {
        let wanted = if only.is_empty() {
            !modules.contains(&script.hash)
        } else {
            only.contains(&script.hash)
        };
        if !wanted {
            continue;
        }
        entries += 1;
        if let Err(e) = host.run_script(&library, script.hash) {
            failures.push(format!("script {:08x}: {e}", script.hash));
        }
    }

    let loaded = host.modules();
    let unresolved: Vec<&str> = loaded
        .iter()
        .filter(|m| m.1.is_none())
        .map(|m| m.0.as_str())
        .collect();
    println!(
        "{} modules loaded ({} unresolved, as stubs), {entries} scripts run, {} failed",
        loaded.len(),
        unresolved.len(),
        failures.len()
    );
    println!("unresolved: {}", unresolved.join(" "));
    println!();
    println!("modules (name -> script):");
    for (name, hash) in &loaded {
        println!(
            "  {name} -> {}",
            hash.map_or("?".into(), |h| format!("{h:08x}"))
        );
    }
    println!();
    println!("failures:");
    for f in &failures {
        // Error messages carry a traceback; the first lines say enough.
        let short: Vec<&str> = f.lines().take(4).collect();
        println!("  {}", short.join(" | "));
    }
    let calls = host.take_calls();
    if only.is_empty() {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for c in &calls {
            *counts.entry(c).or_default() += 1;
        }
        let mut sorted: Vec<_> = counts.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        println!();
        println!("stub calls while starting ({} in all):", calls.len());
        for (c, n) in sorted.iter().take(150) {
            println!("  {c} {n}");
        }
    } else {
        println!();
        println!("stub calls, in order:");
        for c in calls.iter().take(400) {
            println!("  {c}");
        }
    }
    Ok(())
}

/// Which entity components name scripts: every aligned word of every
/// fixed-size record that equals a script's hash, counted by component and
/// offset, with a few examples.
fn script_references(library: &Library, paths: &[String], layers: &[(usize, Vec<u8>)]) {
    let scripts: std::collections::HashSet<u32> =
        library.scripts().iter().map(|s| s.hash).collect();
    // (component, offset) -> (count, distinct scripts, examples)
    #[allow(clippy::type_complexity)]
    let mut found: BTreeMap<
        (String, usize),
        (usize, std::collections::BTreeSet<u32>, Vec<String>),
    > = BTreeMap::new();
    let mut referenced = std::collections::BTreeSet::new();
    let mut components: BTreeMap<String, usize> = BTreeMap::new();
    for (block, data) in layers {
        let Ok(layer) = Layer::parse(data) else {
            continue;
        };
        let names = layer.names();
        for c in &layer.components {
            if c.name.to_ascii_lowercase().contains("script") {
                *components
                    .entry(format!(
                        "{} (record {} bytes, {} fields)",
                        c.name,
                        c.record_size,
                        c.fields.len()
                    ))
                    .or_default() += c.count as usize;
            }
            let Some(records) = c.records() else { continue };
            for (entity, record) in records {
                for (i, w) in record.chunks_exact(4).enumerate() {
                    let word = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
                    if !scripts.contains(&word) {
                        continue;
                    }
                    referenced.insert(word);
                    let e = found.entry((c.name.to_owned(), i * 4)).or_default();
                    e.0 += 1;
                    e.1.insert(word);
                    if e.2.len() < 6 {
                        e.2.push(format!(
                            "{word:08x} on {:?} in {}",
                            names.get(&entity).copied().unwrap_or("?"),
                            paths[*block]
                        ));
                    }
                }
            }
        }
    }
    println!();
    println!("components with \"script\" in their name (component: entities):");
    for (c, n) in &components {
        println!("  {c}: {n}");
    }
    println!();
    println!(
        "entity components naming a script ({} scripts referenced):",
        referenced.len()
    );
    for ((c, offset), (n, distinct, examples)) in &found {
        println!("  {c} +{offset}: {n} records, {} scripts", distinct.len());
        for e in examples {
            println!("      {e}");
        }
    }
}

/// Libraries the engine provides (the rest are the scripts' own modules).
const ENGINE: [&str; 26] = [
    "Ai",
    "Airstrike",
    "Camera",
    "Controller",
    "Debug",
    "Event",
    "Graphics",
    "Gui",
    "Hud",
    "Human",
    "Marker",
    "Math",
    "Net",
    "Object",
    "ObjectFilter",
    "ObjectState",
    "Pda",
    "Pg",
    "Player",
    "Sound",
    "String",
    "Sys",
    "Vehicle",
    "Weapon",
    "Munitions",
    "VO",
];

/// Every call to an engine library function, by the shape of its
/// arguments: `Event.Create(Event.ObjectDeath, {}, fn, {}) x 12`.
fn signatures(library: &Library) {
    let mut shapes: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for script in library.scripts() {
        for call in script.chunk.main.calls() {
            let Some((lib, _)) = call.callee.split_once('.') else {
                continue;
            };
            if !ENGINE.contains(&lib) {
                continue;
            }
            let shape = match &call.args {
                None => "(...)".to_owned(),
                Some(args) => {
                    let parts: Vec<String> = args
                        .iter()
                        .map(|a| match a {
                            Arg::Str(_) => "str".into(),
                            Arg::Number(_) => "num".into(),
                            Arg::Bool => "bool".into(),
                            Arg::Nil => "nil".into(),
                            Arg::Table => "{}".into(),
                            Arg::Function => "fn".into(),
                            Arg::Global(g) if g.contains('.') => g.clone(),
                            Arg::Global(_) => "global".into(),
                            Arg::Other => "_".into(),
                        })
                        .collect();
                    format!("({})", parts.join(", "))
                }
            };
            *shapes
                .entry(call.callee)
                .or_default()
                .entry(shape)
                .or_default() += 1;
        }
    }
    let mut sorted: Vec<_> = shapes.into_iter().collect();
    sorted.sort_by_key(|(name, s)| (std::cmp::Reverse(s.values().sum::<usize>()), name.clone()));
    println!();
    println!(
        "engine calls by argument shape ({} functions):",
        sorted.len()
    );
    for (name, s) in sorted {
        let total: usize = s.values().sum();
        let mut forms: Vec<_> = s.into_iter().collect();
        forms.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let shown: Vec<String> = forms
            .iter()
            .take(3)
            .map(|(f, n)| format!("{f} x{n}"))
            .collect();
        println!("  {name} {total}: {}", shown.join("; "));
    }
}
