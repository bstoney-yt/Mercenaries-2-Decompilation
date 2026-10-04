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
//! and the calls they make are listed in order.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;
use std::rc::Rc;

use m2_assets::lua::{Chunk, SCRIPT_TYPE};
use m2_assets::{BlockContents, WadArchive};
use m2_game::library::Library;
use m2_game::script::ScriptHost;
use mlua::{HookTriggers, VmState};

/// Instructions one script may run before it is stopped.
const BUDGET: u64 = 50_000_000;

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
    for block in 0..wad.index().blocks().len() {
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
            }
        }
    }
    println!("{} scripts", library.scripts().len());
    let library = Rc::new(library);

    let host = ScriptHost::new()?;
    host.stub_missing_globals()?;
    host.install_library(Rc::clone(&library), true)?;
    let spent = Rc::new(Cell::new(0u64));
    let counter = Rc::clone(&spent);
    host.lua().set_hook(
        HookTriggers::new().every_nth_instruction(100_000),
        move |_, _| {
            counter.set(counter.get() + 100_000);
            if counter.get() > BUDGET {
                return Err(mlua::Error::runtime("instruction budget spent"));
            }
            Ok(VmState::Continue)
        },
    )?;

    let mut failures = Vec::new();
    if only.is_empty() {
        let import: mlua::Function = host.lua().globals().get("import")?;
        for name in library.module_names() {
            spent.set(0);
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
        spent.set(0);
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
