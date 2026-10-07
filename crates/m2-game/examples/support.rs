//! Surveys the support system (the PDA shop, supply drops and air
//! strikes) of your own installation: what the scripts' support tables
//! hold once a new game has started, and which engine library functions
//! each support script calls, so the game side can be written to match.
//!
//! ```text
//! cargo run -p m2-game --example support -- <vz.wad> [table...] [--calls]
//!     [--items] [--call <key>] [--listing <module>]
//! ```
//!
//! Each `table` is a Lua expression read after start-up (default
//! `MrxSupportData.tSupportData`); its keys and fields are printed, nested
//! tables to a few levels. With `--calls`, every module the support data
//! script imports is listed with the engine calls it makes (callee and
//! argument count), most used first. `--items` lists the catalogue as the
//! game reads it. `--call <key>` stocks the item and calls it in as the
//! game would, standing in for the player and the world: she throws her
//! designator 15 m ahead, it lies its 4 s, aircraft arrive at once and
//! ordnance goes off where it set off; every command and log line is
//! printed. `--listing <module>` prints a script's Lua 5.1 listing, to read
//! its control flow locally. Nothing is written but stdout.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;

use m2_assets::{WadArchive, name_hash};
use m2_game::api::LIBRARIES;
use m2_game::mission::{GameData, Mission};

/// Prints a value's keys and fields, `depth` levels down.
const DUMP: &str = r#"
local function key(k) return type(k) == "string" and k or ("[" .. tostring(k) .. "]") end
local function show(v)
  if type(v) == "string" then return string.format("%q", v) end
  return tostring(v)
end
function __m2_dump(t, indent, depth)
  if type(t) ~= "table" then print(indent .. show(t)) return end
  local keys = {}
  for k in pairs(t) do keys[#keys + 1] = k end
  table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
  for _, k in ipairs(keys) do
    local v = t[k]
    if type(v) == "table" and depth > 0 then
      print(indent .. key(k) .. " = {")
      __m2_dump(v, indent .. "  ", depth - 1)
      print(indent .. "}")
    elseif type(v) == "table" then
      local n = 0
      for _ in pairs(v) do n = n + 1 end
      print(indent .. key(k) .. " = {" .. n .. " entries}")
    elseif type(v) == "function" then
      print(indent .. key(k) .. " = function")
    else
      print(indent .. key(k) .. " = " .. show(v))
    end
  end
end
"#;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        return Err("usage: support <vz.wad> [table...] [--calls]".into());
    };
    let calls = args.iter().any(|a| a == "--calls");
    let list_items = args.iter().any(|a| a == "--items");
    let call = args
        .iter()
        .position(|a| a == "--call")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let listing = args
        .iter()
        .position(|a| a == "--listing")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let mut tables: Vec<&str> = args[1..]
        .iter()
        .filter(|a| {
            !a.starts_with("--") && Some(*a) != call.as_ref() && Some(*a) != listing.as_ref()
        })
        .map(String::as_str)
        .collect();
    if tables.is_empty() && !list_items && call.is_none() {
        tables.push("MrxSupportData.tSupportData");
    }
    let mut wad = WadArchive::open(BufReader::new(File::open(path)?))?;
    let data = GameData::read(&mut wad)?;
    if calls {
        list_calls(&data);
    }
    if let Some(m) = &listing {
        match data.library.by_hash(name_hash(m)) {
            Some(s) => print!("{}", s.chunk.main.listing()),
            None => println!("no script {m}"),
        }
        return Ok(());
    }
    let mut mission = Mission::new(data, "Mattias", [0.0; 3])?;
    mission.start();
    for _ in 0..10 {
        mission.tick(0.5);
    }
    for line in mission.take_log() {
        println!("log {line}");
    }
    mission.take_commands();
    if list_items {
        for i in mission.support_items() {
            println!(
                "item {} {} {} cash {} fuel {} {} {} via {} brings {:?} stock {} free {}",
                i.key,
                i.kind,
                i.module,
                i.cash,
                i.fuel,
                i.designator,
                i.icon,
                i.vehicle,
                i.cargo,
                i.stock,
                i.free
            );
        }
    }
    if let Some(key) = &call {
        call_in(&mut mission, key);
    }
    mission.lua().load(DUMP).set_name("=dump").exec()?;
    for t in tables {
        println!("== {t}");
        let code = format!("__m2_dump({t}, '  ', 3)");
        if let Err(e) = mission.lua().load(&code).set_name("=dump").exec() {
            println!("  error: {}", e.to_string().lines().next().unwrap_or(""));
        }
    }
    Ok(())
}

/// Calls `key` in and plays the world's part, printing what happens.
fn call_in(mission: &mut Mission, key: &str) {
    use m2_game::airstrike::Launch;
    use m2_game::api::Command;
    // Her recruits (the pilot and the helicopter pilot) join over the
    // story; here they are all with her, and the shop's requirements
    // waived.
    let recruit = "MrxSupportData.SetHeliPilotRecruited(true) \
        MrxSupportData.SetJetPilotRecruited(true) \
        MrxSupportData.SetMechanicRecruited(true) \
        MrxSupportData.SetIgnoreRequirements(true)";
    if let Err(e) = mission.run(recruit, "recruit") {
        println!("recruit: {e}");
    }
    mission.stock_support(key, 1, 1.0e6, 1000.0);
    let pmc = "print('cash', MrxPmc.GetCashQty(), 'fuel', MrxPmc.GetFuelQty(), \
        'capacity', MrxPmc.GetFuelCapacity(), 'stock', MrxPmc.GetSupportQty(__m2_probe), \
        'pilot', MrxSupportManager.IsRecruitAvailable('Pilot'), \
        'copter', MrxSupportManager.IsRecruitAvailable('Copter'), \
        'fiona', MrxSupportManager.IsRecruitAvailable('Fiona'))";
    let _ = mission.lua().globals().set("__m2_probe", key);
    if let Err(e) = mission.run(pmc, "pmc") {
        println!("pmc: {e}");
    }
    let took = mission.use_support(key);
    println!("call {key}: {took}");
    let mut pending: Vec<Command> = Vec::new();
    for round in 0..40 {
        for line in mission.take_log() {
            println!("log {line}");
        }
        pending.extend(mission.take_commands());
        if pending.is_empty() {
            mission.tick(0.5);
            if round > 30 {
                break;
            }
            continue;
        }
        for c in std::mem::take(&mut pending) {
            println!("cmd {c:?}");
            match c {
                Command::EquipDesignator { who, kind, .. } => {
                    let at = mission
                        .world()
                        .borrow()
                        .objects
                        .get(&who)
                        .map_or([0.0; 3], |o| {
                            let (s, c) = o.yaw.sin_cos();
                            [
                                o.position[0] + s * 15.0,
                                o.position[1],
                                o.position[2] + c * 15.0,
                            ]
                        });
                    let thrown = if kind.contains("Smoke") {
                        "Smoke Grenade Projectile"
                    } else {
                        "Designator"
                    };
                    let g = mission.designator_thrown(who, thrown, at);
                    println!("world: {kind} thrown as {g} to {at:?}");
                    for _ in 0..8 {
                        mission.tick(0.5);
                    }
                    mission.designator_settled(g, at);
                }
                Command::Flyby { guid, to, .. } => mission.flyby_arrived(guid, to),
                Command::Ordnance {
                    guid, at, launch, ..
                } => {
                    let v = match launch {
                        Launch::Velocity(v) | Launch::Along(v) => v,
                    };
                    println!("world: ordnance {guid} heading {v:?}");
                    mission.ordnance_burst(guid, at);
                }
                _ => {}
            }
        }
    }
}

/// The support data script's imports, each with its engine calls.
fn list_calls(data: &GameData) {
    let lib = &data.library;
    let Some(support) = lib.by_hash(name_hash("MrxSupportData")) else {
        println!("no MrxSupportData script");
        return;
    };
    let mut modules: Vec<String> = support.loads.iter().cloned().collect();
    modules.push("mrxsupportdata".into());
    modules.push("mrxshop".into());
    for m in modules {
        let Some(s) = lib.by_hash(name_hash(&m)) else {
            println!("== {m}: no script");
            continue;
        };
        let mut counts: BTreeMap<String, (usize, Option<usize>)> = BTreeMap::new();
        for f in s.chunk.main.all() {
            for c in f.calls() {
                let engine = c
                    .callee
                    .split_once('.')
                    .is_some_and(|(l, _)| LIBRARIES.contains(&l));
                if engine {
                    let e = counts.entry(c.callee.clone()).or_default();
                    e.0 += 1;
                    e.1 = c.args.as_ref().map(Vec::len).or(e.1);
                }
            }
        }
        let mut list: Vec<_> = counts.into_iter().collect();
        list.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(&b.0)));
        println!("== {m} {:08x}", s.hash);
        for (callee, (n, args)) in list {
            let args = args.map_or("?".to_owned(), |a| a.to_string());
            println!("  {callee}({args}) x{n}");
        }
    }
}
