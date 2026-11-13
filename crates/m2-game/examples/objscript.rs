//! Runs one of the object scripts the port runs (`OBJECT_SCRIPTS`) of
//! your own installation headless: counts the layer entities it runs on,
//! loads their layers, lets time pass, and prints how many markers the
//! scripts put up and what failed.
//!
//! ```text
//! cargo run -p m2-game --example objscript -- <vz.wad> <module>
//! ```
//!
//! For `crate` it then stands the player beside the first one, hangs it
//! on a winch, lets it go and destroys it, printing the marker count
//! after each. For `collectable` it stands her beside the first one and
//! takes its context action; for `pmcgate` and `repairpad` it counts
//! what they ask of the first one. For `AntiAir` it spawns a crewed
//! `M113 AA (VZ) (Driver)` where the first stands, comes near it, has it
//! lock on her and fire, and counts the radar calls the scripts make
//! (`antiair:` lines). For `autogunship` it flies the gunship over her
//! with a hostile vehicle near her and prints the shells it fires
//! (`gunship:` lines). Prints names, counts and the scripts' errors
//! only.

use std::fs::File;
use std::io::BufReader;

use m2_assets::WadArchive;
use m2_game::api::Command;
use m2_game::mission::{GameData, HomingKind, HomingPost, Mission, OBJECT_SCRIPTS};
use m2_game::vehicle_state::VehicleCommand;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(wad), Some(module)) = (args.first(), args.get(1)) else {
        return Err("usage: objscript <vz.wad> <module>".into());
    };
    let Some((name, hash)) = OBJECT_SCRIPTS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(module))
    else {
        return Err(format!("{module} is not a script the port runs").into());
    };
    let mut archive = WadArchive::open(BufReader::new(File::open(wad)?))?;
    let data = GameData::read(&mut archive)?;
    let mut on: Vec<(String, u32, [f32; 3])> = Vec::new();
    for (layer, entities) in &data.layers {
        for e in entities.iter().filter(|e| e.script == Some(*hash)) {
            on.push((layer.clone(), e.guid, e.at));
        }
    }
    let mut layers: Vec<&str> = on.iter().map(|(l, _, _)| l.as_str()).collect();
    layers.dedup();
    println!(
        "{} entities run {name}, in {} layers",
        on.len(),
        layers.len()
    );
    let mut mission = Mission::new(data, "Mattias", [0.0, 0.0, 0.0])?;
    mission.set_hero("Mattias", "mattias", 120.0);
    mission.start();
    mission.take_log();
    let markers = |m: &Mission| m.world().borrow().hud.markers.len();
    let before = markers(&mission);
    for layer in &layers {
        mission.preload(layer);
    }
    let mut failures = 0;
    let mut run = |m: &mut Mission, seconds: f64| {
        let end = m.now() + seconds;
        while m.now() < end {
            m.tick(0.25);
            m.take_commands();
            for line in m.take_log() {
                if line
                    .to_ascii_lowercase()
                    .contains(&name.to_ascii_lowercase())
                {
                    failures += 1;
                    if failures <= 10 {
                        println!("{line}");
                    }
                }
            }
        }
    };
    run(&mut mission, 3.0);
    println!(
        "markers: {before} before, {} once loaded",
        markers(&mission)
    );
    if name.eq_ignore_ascii_case("crate")
        && let Some(&(_, first, at)) = on.first()
    {
        // Beside it, so that it is awake.
        mission.put_player([at[0] + 5.0, at[1], at[2]]);
        run(&mut mission, 1.0);
        println!("markers beside the first: {}", markers(&mission));
        mission.set_winched(first, Some(0x7777));
        run(&mut mission, 1.0);
        println!("markers with one on a winch: {}", markers(&mission));
        mission.set_winched(first, None);
        run(&mut mission, 1.0);
        println!("markers with it let go: {}", markers(&mission));
        mission.kill(first);
        run(&mut mission, 1.0);
        println!("markers with it destroyed: {}", markers(&mission));
    }
    if name.eq_ignore_ascii_case("collectable")
        && let Some(&(_, first, at)) = on.first()
    {
        let offered = |m: &Mission| m.context_text(first).unwrap_or_else(|| "none".into());
        let alive = |m: &Mission| m.world().borrow().objects.get(&first).map(|o| o.alive);
        mission.put_player([at[0] + 1.0, at[1], at[2]]);
        run(&mut mission, 1.0);
        println!(
            "beside the first: offered {}, alive {:?}; {} offered in all",
            offered(&mission),
            alive(&mission),
            mission.context_actions().len()
        );
        mission.take_context_action(first);
        run(&mut mission, 1.0);
        println!(
            "taken: offered {}, alive {:?}",
            offered(&mission),
            alive(&mission)
        );
    }
    if name.eq_ignore_ascii_case("AntiAir")
        && let Some(&(_, _, at)) = on.first()
    {
        antiair(&mut mission, at, &mut run)?;
    }
    if ["pmcgate", "repairpad"].contains(&name.to_ascii_lowercase().as_str())
        && let Some(&(_, first, at)) = on.first()
    {
        // Beside the first, for four seconds: what the scripts ask of it.
        mission.put_player([at[0] + 5.0, at[1], at[2]]);
        let (mut opened, mut parts) = (0, 0);
        let end = mission.now() + 4.0;
        while mission.now() < end {
            mission.tick(0.25);
            for c in mission.take_commands() {
                match c {
                    Command::OpenGate(g) if g == first => opened += 1,
                    Command::Vehicle(VehicleCommand::Part { vehicle, on, .. })
                        if vehicle == first && on =>
                    {
                        parts += 1;
                    }
                    _ => {}
                }
            }
        }
        println!("beside the first: {opened} gate openings, {parts} parts turned on");
    }
    if name.eq_ignore_ascii_case("pursuitcopter") {
        // One comes in as a pursuit's air unit does: 350 m off her, 70 m up.
        let template = args
            .get(2)
            .map_or("Alouette3 Transport (VZ) (Pursuit)", String::as_str);
        mission.put_player([100.0, 0.0, 100.0]);
        let copter = mission.spawn(template, [450.0, 70.0, 100.0]);
        println!("spawned {template}: {copter:?}");
        let mut seen: Vec<String> = Vec::new();
        let end = mission.now() + 20.0;
        while mission.now() < end {
            mission.tick(0.25);
            for c in mission.take_commands() {
                let text = format!("{c:?}");
                let short: String = text.chars().take(150).collect();
                if !matches!(c, Command::Spawn { .. }) || seen.len() < 12 {
                    seen.push(format!("t={:.2} {short}", mission.now()));
                }
            }
            for line in mission.take_log() {
                if seen.len() < 60 {
                    seen.push(format!(
                        "log {}",
                        line.chars().take(150).collect::<String>()
                    ));
                }
            }
        }
        // The game reports the landing, as m2-alpha does once it is down.
        let pilot = copter.and_then(|c| mission.world().borrow().driver_of(c));
        if let Some(p) = pilot {
            mission.arrived(p, [100.0, 0.0, 100.0]);
            seen.push(format!("t={:.2} arrived {p}", mission.now()));
            let end = mission.now() + 10.0;
            while mission.now() < end {
                mission.tick(0.25);
                for c in mission.take_commands() {
                    let short: String = format!("{c:?}").chars().take(150).collect();
                    seen.push(format!("t={:.2} {short}", mission.now()));
                }
            }
        }
        let exists = copter.is_some_and(|c| mission.world().borrow().objects.contains_key(&c));
        seen.push(format!("copter still in the scripts' world: {exists}"));
        for s in seen.iter().rev().take(40).rev() {
            println!("{s}");
        }
    }
    if name.eq_ignore_ascii_case("autogunship") {
        autogunship(&mut mission, args.get(2).map(String::as_str))?;
    }
    println!("{name} lines logged: {failures}");
    Ok(())
}

/// `autogunship`: she stands at (100, 0, 100) with a hostile vehicle
/// (`target`, default `M113 AA (VZ) (Driver)`) 60 m off; the gunship is
/// flown over her
/// as AllCon003 flies it (`Airstrike.Flyby` from 300 m north of her to
/// her, 100 m up, at 40 m/s, and on past her), and 15 s pass. Prints each
/// `Gunship Shell` it fires (`gunship:` lines: when, from, which way)
/// and the scripts' errors.
fn autogunship(mission: &mut Mission, target: Option<&str>) -> Result<()> {
    let target = target.unwrap_or("M113 AA (VZ) (Driver)");
    mission.put_player([100.0, 0.0, 100.0]);
    let tank = mission.spawn(target, [160.0, 0.0, 100.0]);
    let labels: String = mission
        .lua()
        .load(
            "local t = ... local s = ''              for _, l in ipairs({'Vehicle', 'VZ', 'China', 'Guerilla'}) do                if Object.HasLabel(t, l) then s = s .. l .. ' ' end end              return s .. #Pg.FastCollectGroundVehicles(100, 0, 100, 200) .. ' near her'",
        )
        .call(tank.map(m2_game::events::guid_value))
        .unwrap_or_default();
    println!("gunship: target {target} {tank:?}: {labels}");
    mission.take_commands();
    mission.take_log();
    let start = mission.now();
    // AllCon003's call: from 50 m west and 300 m north of her to her, 100
    // m over her, at 40 m/s.
    mission.run(
        "local x, y, z = Object.GetPosition(Player.GetLocalCharacter())          __gunship = Airstrike.Flyby('Support Vehicle (Autogunship)',          x - 50, z + 300, x, z, y + 100, 40)",
        "gunship-check",
    )?;
    let gunship: Option<u32> = mission
        .lua()
        .load("return __gunship")
        .eval::<mlua::Value>()
        .ok()
        .and_then(|v| m2_game::events::as_guid(&v));
    let (from, to) = ([50.0_f32, 100.0, 400.0], [100.0_f32, 100.0, 100.0]);
    let len = ((to[0] - from[0]).powi(2) + (to[2] - from[2]).powi(2)).sqrt();
    let mut shells = 0;
    while mission.now() < start + 15.0 {
        // Flown on its line at 40 m/s, on past her (as m2-alpha flies it).
        if let Some(g) = gunship {
            let k = 40.0 * (mission.now() - start) as f32 / len;
            let at = [
                from[0] + (to[0] - from[0]) * k,
                100.0,
                from[2] + (to[2] - from[2]) * k,
            ];
            mission.place(g, at, 0.0);
        }
        mission.tick(0.25);
        for c in mission.take_commands() {
            if let Command::Ordnance {
                what, at, launch, ..
            } = &c
            {
                shells += 1;
                if shells <= 12 {
                    println!(
                        "gunship: t={:.2} {what} at {at:?} {launch:?}",
                        mission.now() - start
                    );
                }
            }
        }
        for l in mission.take_log() {
            if l.contains("rror") || l.contains("SALVO") || l.contains("target") {
                println!("gunship: log {}", l.chars().take(160).collect::<String>());
            }
        }
    }
    println!("gunship: {shells} shells in 15 s");
    Ok(())
}

/// `AntiAir`: a crewed one (the layers' have no driver here, and only a
/// hostile driver's is blipped: its other colours are `false`) with a VZ
/// crew hostile to the PMC; she comes inside its `nAARange`; it locks on
/// her for two seconds (`HomingLockUpdate` every 0.25 s, the share rising
/// to 1); it fires a missile, which bursts after a second. Counts the
/// scripts' `Hud.Radar:AddObjective` / `RemoveObjective` calls by blip
/// (the game's own `MrxGuiInterface` takes them on to the HUD's widget,
/// which is not run here).
fn antiair(
    mission: &mut Mission,
    at: [f32; 3],
    run: &mut impl FnMut(&mut Mission, f64),
) -> Result<()> {
    let aa = mission
        .spawn("M113 AA (VZ) (Driver)", at)
        .ok_or("no M113 AA (VZ) (Driver)")?;
    let crew: Vec<u32> = mission
        .world()
        .borrow()
        .objects
        .iter()
        .filter(|(_, o)| o.vehicle == Some(aa))
        .map(|(g, _)| *g)
        .collect();
    for g in crew {
        if let Some(o) = mission.world().borrow_mut().objects.get_mut(&g) {
            o.faction = Some(m2_assets::name_hash("VZ"));
        }
    }
    mission.run(
        "Ai.SetRelation('VZ', 'PMC', -100)
        __radar = {}
        local function note(t, what)
            local k = tostring(t.sName)
            __radar[k] = (__radar[k] or '') .. what
        end
        local add, remove = Hud.Radar.AddObjective, Hud.Radar.RemoveObjective
        Hud.Radar.AddObjective = function(s, t)
            note(t, t.nR == 255 and t.nG == 255 and t.nB == 255 and 'F' or 'A')
            return add(s, t)
        end
        Hud.Radar.RemoveObjective = function(s, t) note(t, 'R') return remove(s, t) end",
        "antiair-check",
    )?;
    // The calls on a blip so far: A added in its colour, F in its flash
    // colour (white), R removed.
    let calls = |m: &Mission, g: u32| -> String {
        m.lua()
            .load("return __radar[tostring(...)] or ''")
            .call::<String>(m2_game::events::guid_value(g))
            .unwrap_or_default()
    };
    mission.put_player([at[0] + 30.0, at[1], at[2]]);
    run(mission, 3.0);
    println!(
        "antiair: radar calls on it beside it: {:?}",
        calls(mission, aa)
    );
    let post = |kind, ammo| HomingPost {
        kind,
        player: 1,
        owner: aa,
        vehicle: Some(aa),
        ammo,
    };
    let mut ok = mission.homing_post(post(HomingKind::Start, None));
    for step in 1..=8 {
        ok &= mission.homing_post(post(HomingKind::Update(step as f32 / 8.0), None));
        run(mission, 0.25);
    }
    println!(
        "antiair: lock posts ran {ok}; radar calls on it: {:?}",
        calls(mission, aa)
    );
    let round = mission.homing_round("AA Missile", [at[0], at[1] + 3.0, at[2]]);
    ok = mission.homing_post(post(HomingKind::Clear, None));
    ok &= mission.homing_post(post(HomingKind::Launched, Some(round)));
    run(mission, 1.0);
    println!(
        "antiair: launch posts ran {ok}; radar calls on the missile: {:?}",
        calls(mission, round)
    );
    mission.round_gone(round);
    run(mission, 0.5);
    println!("antiair: once it burst: {:?}", calls(mission, round));
    Ok(())
}
