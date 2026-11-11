//! Lists the engine calls a contract's scripts make and which of them the
//! game's libraries here don't write yet (they answer nil from a stub).
//!
//! ```text
//! cargo run -p m2-game --example natives -- <vz.wad> <name part...>
//! ```
//!
//! A script is listed when its source name holds any of the name parts,
//! ignoring case (`pmccon004`). Calls are read from the compiled scripts
//! ([`Function::calls`](m2_assets::lua::Function::calls)), nested
//! functions included, so this counts every call site, run or not.
//! Whether a function is written is asked of the scripts' state the
//! headless mission builds: a member the library table holds itself, not
//! one its stub metatable makes up.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;

use m2_assets::BlockContents;
use m2_assets::WadArchive;
use m2_assets::lua::{Chunk, SCRIPT_TYPE};
use m2_game::mission::{GameData, Mission};

/// The engine's libraries, as in the `scripts` example.
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
    let [path, parts @ ..] = args else {
        return Err("usage: natives <vz.wad> <name part...>".into());
    };
    if parts.is_empty() {
        return Err("give at least one part of a script's name".into());
    }
    let parts: Vec<String> = parts.iter().map(|p| p.to_ascii_lowercase()).collect();
    let mut wad = WadArchive::open(BufReader::new(File::open(path)?))?;

    // Callee -> (call sites, scripts).
    let mut calls: BTreeMap<String, (usize, BTreeMap<String, ()>)> = BTreeMap::new();
    // Callee -> its first arguments when they are string constants.
    let mut firsts: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut scripts = Vec::new();
    for block in 0..wad.index().blocks().len() {
        let decoded = wad.read_block(block)?;
        let Ok(contents) = BlockContents::parse(&decoded) else {
            continue;
        };
        for (entry, data) in contents.iter() {
            if entry.type_hash != SCRIPT_TYPE {
                continue;
            }
            let Ok(chunk) = Chunk::from_asset(data) else {
                continue;
            };
            let source = chunk.main.source.clone().unwrap_or_default();
            let lower = source.to_ascii_lowercase();
            if !parts.iter().any(|p| lower.contains(p.as_str())) {
                continue;
            }
            let name = format!("{:08x} {source}", entry.name_hash);
            if scripts.contains(&name) {
                continue;
            }
            // A listing of each script to read beside the calls, when
            // asked for (it stays on your machine, with the game's data).
            if let Ok(dir) = std::env::var("NATIVES_LISTING") {
                let all: Vec<String> = chunk.main.all().iter().map(|f| f.listing()).collect();
                std::fs::write(
                    std::path::Path::new(&dir).join(format!("{:08x}.txt", entry.name_hash)),
                    all.join("\n"),
                )?;
            }
            for call in chunk.main.calls() {
                let Some((lib, _)) = call.callee.split_once('.') else {
                    continue;
                };
                if !ENGINE.contains(&lib) {
                    continue;
                }
                if let Some(first) = &call.first {
                    *firsts
                        .entry(call.callee.clone())
                        .or_default()
                        .entry(first.clone())
                        .or_default() += 1;
                }
                let e = calls.entry(call.callee).or_default();
                e.0 += 1;
                e.1.insert(name.clone(), ());
            }
            scripts.push(name);
        }
    }

    let data = GameData::read(&mut wad)?;
    let mission = Mission::new(data, "Mattias", [0.0; 3])?;
    let lua = mission.lua();
    let written = lua.load(
        "return function(path)
           local t = _G
           for part in string.gmatch(path, '[^.]+') do
             if type(t) ~= 'table' then return false end
             t = rawget(t, part)
           end
           return t ~= nil
         end",
    );
    let written: mlua::Function = written.eval()?;

    println!("{} scripts:", scripts.len());
    for s in &scripts {
        println!("  {s}");
    }
    let mut missing = Vec::new();
    println!();
    println!("{} engine functions called:", calls.len());
    for (callee, (sites, users)) in &calls {
        let ok: bool = written.call(callee.as_str())?;
        let mark = if ok { "   " } else { "NO " };
        println!("  {mark}{callee} ({sites} sites, {} scripts)", users.len());
        if !ok {
            missing.push((callee, *sites, users));
        }
    }
    println!();
    println!("not written ({}):", missing.len());
    for (callee, sites, users) in missing {
        let names: Vec<&str> = users.keys().map(String::as_str).collect();
        println!("  {callee} x{sites}: {}", names.join("; "));
        if let Some(f) = firsts.get(callee) {
            let f: Vec<String> = f.iter().map(|(s, n)| format!("{s:?} x{n}")).collect();
            println!("    first argument: {}", f.join(", "));
        }
    }
    Ok(())
}
