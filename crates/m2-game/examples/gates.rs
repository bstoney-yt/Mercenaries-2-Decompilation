//! Runs the friendly faction gates (`friendlygate`) of your own
//! installation headless: every layer entity the engine runs it on, by
//! layer and faction; their layers loaded; then at each gate in turn, the
//! player walks up (8 m off), waits, honks, waits, and walks away (60 m),
//! and what the gate did each time is counted.
//!
//! ```text
//! cargo run -p m2-game --example gates -- <vz.wad> [--disguise <faction>]
//! ```
//!
//! `--disguise` puts her in a vehicle of that faction (`Allied`, `China`,
//! ...) with her disguise holding first. Prints names, counts and the
//! scripts' errors only.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;

use m2_assets::WadArchive;
use m2_game::api::Command;
use m2_game::mission::{FRIENDLY_GATE, GameData, Mission, OBJECT_SCRIPTS};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Faction labels a gate may carry, as `MrxUtil.GetFaction` reads them.
const FACTIONS: [&str; 7] = ["VZ", "Allied", "China", "Guerilla", "OC", "Pirate", "PMC"];

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let wad = args
        .first()
        .ok_or("usage: gates <vz.wad> [--disguise <faction>]")?;
    let disguise = args
        .iter()
        .position(|a| a == "--disguise")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let mut archive = WadArchive::open(BufReader::new(File::open(wad)?))?;
    let data = GameData::read(&mut archive)?;
    let mut gates: Vec<(String, u32, String, [f32; 3])> = Vec::new();
    for (layer, entities) in &data.layers {
        for e in entities.iter().filter(|e| e.script == Some(FRIENDLY_GATE)) {
            gates.push((layer.clone(), e.guid, e.name.clone(), e.at));
        }
    }
    for (module, hash) in OBJECT_SCRIPTS {
        let n: usize = data
            .layers
            .values()
            .map(|l| l.iter().filter(|e| e.script == Some(hash)).count())
            .sum();
        println!("{n} entities run {module}");
    }
    let layers: Vec<String> = {
        let mut l: Vec<String> = gates.iter().map(|g| g.0.clone()).collect();
        l.dedup();
        l
    };
    println!(
        "{} entities run friendlygate, in {} layers",
        gates.len(),
        layers.len()
    );
    let mut mission = Mission::new(data, "Mattias", [0.0, 0.0, 0.0])?;
    mission.set_hero("Mattias", "mattias", 120.0);
    mission.start();
    if args.iter().any(|a| a == "--mutable") {
        // As the story makes them later: attitudes that can change.
        let _ = mission
            .lua()
            .load(
                "for _, f in ipairs({'All','Chi','Gur','Oil','Pir','Vza'}) do \
                 pcall(MrxFactionManager.SetAttitudeMutable, f, true) end",
            )
            .exec();
    }
    for layer in &layers {
        mission.preload(layer);
    }
    let mut failures = 0;
    let mut run = |m: &mut Mission, seconds: f64| -> Vec<Command> {
        let mut out = Vec::new();
        let end = m.now() + seconds;
        while m.now() < end {
            m.tick(0.25);
            out.extend(m.take_commands());
            for line in m.take_log() {
                if line.contains("friendlygate") || line.contains("FriendlyGate") {
                    failures += 1;
                    if failures <= 10 {
                        println!("{line}");
                    }
                }
            }
        }
        out
    };
    run(&mut mission, 3.0);
    let dbg: Option<String> = mission
        .lua()
        .load(
            "local g = rawget(_G, 'friendlygate') if type(g) ~= 'table' then return 'no module' end \
             local n = 0 for _ in pairs(g._tGates or {}) do n = n + 1 end \
             local out = {'gates started ' .. n} \
             for _, f in ipairs({'All','Chi','Gur','Oil','Pir','Vza'}) do \
               local ok, m = pcall(MrxFactionManager.IsAttitudeMutable, f) out[#out+1] = f .. ' mutable ' .. tostring(ok and m) end \
             out[#out+1] = 'hero label ' .. tostring(Object.HasLabel(Player.GetLocalCharacter(), 'Hero')) \
             return table.concat(out, ', ')",
        )
        .eval()
        .ok();
    println!("state: {}", dbg.unwrap_or_default());
    let template = args
        .iter()
        .position(|a| a == "--disguise")
        .and_then(|i| args.get(i + 2))
        .map_or("WZ551", String::as_str);
    let vehicle = disguise.as_ref().map(|faction| {
        let v: mlua::Value = mission
            .lua()
            .load(format!(
                "local v = Pg.Spawn({template:?}, 0, 0, 0, 0) \
                 Object.AddLabel(v, {faction:?}) Object.AddLabel(v, 'Vehicle') \
                 Vehicle.Enter(v, Player.GetLocalCharacter(), 'seat_driver', true) return v"
            ))
            .eval()
            .unwrap_or_else(|e| {
                println!("disguise vehicle: {e}");
                mlua::Value::Nil
            });
        mission.world().borrow_mut().disguise = Some((100.0, true));
        v
    });
    if vehicle.as_ref().is_some_and(mlua::Value::is_nil) {
        println!("no disguise vehicle spawned");
    }
    // Per gate: opened before a honk, after it, shut once she left.
    let mut tally: BTreeMap<String, [usize; 4]> = BTreeMap::new();
    for (_, guid, _, at) in &gates {
        let faction = {
            let w = mission.world().borrow();
            let labels = w
                .objects
                .get(guid)
                .map(|o| o.label_hashes.clone())
                .unwrap_or_default();
            FACTIONS
                .iter()
                .find(|f| labels.contains(&m2_assets::name_hash(f)))
                .map_or("none", |f| *f)
                .to_owned()
        };
        let opened = |c: &[Command]| {
            c.iter()
                .any(|c| matches!(c, Command::OpenGate(g) if g == guid))
        };
        let shut = |c: &[Command]| {
            c.iter()
                .any(|c| matches!(c, Command::CloseGate(g) if g == guid))
        };
        mission.put_player([at[0] + 8.0, at[1], at[2]]);
        let before = run(&mut mission, 2.0);
        mission.honk();
        let after = run(&mut mission, 2.0);
        mission.put_player([at[0] + 60.0, at[1], at[2]]);
        let left = run(&mut mission, 2.0);
        let t = tally.entry(faction).or_default();
        t[0] += 1;
        t[1] += usize::from(opened(&before));
        t[2] += usize::from(opened(&after) || opened(&before));
        t[3] += usize::from(shut(&left));
    }
    println!("faction\tgates\topen before honk\topen after honk\tshut after leaving");
    for (f, t) in &tally {
        println!("{f}\t{}\t{}\t{}\t{}", t[0], t[1], t[2], t[3]);
    }
    let attitudes: Option<String> = mission
        .lua()
        .load(
            "local out = {} for _, f in ipairs({'All','Chi','Gur','Oil','Pir','Vza'}) do \
             local ok, a = pcall(MrxFactionManager.GetAttitudeLabel, f, 'Pmc') \
             out[#out + 1] = f .. '=' .. tostring(ok and (type(a) == 'table' and a.sLabel or a)) \
             end return table.concat(out, ' ')",
        )
        .eval()
        .ok();
    println!("attitudes to the PMC: {}", attitudes.unwrap_or_default());
    println!("friendlygate errors logged: {failures}");
    Ok(())
}
