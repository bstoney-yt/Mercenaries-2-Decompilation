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
//! (`antiair:` lines). For a `tree_` script it makes an island tree of a
//! template that runs it, burns it down and prints the material
//! animations it plays. Prints names, counts and the scripts'
//! errors only.

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
    if name.starts_with("tree_") {
        // An island tree of a template that runs it, burnt down: what
        // its script plays.
        let template = mission
            .world()
            .borrow()
            .template_scripts
            .iter()
            .find(|(_, (s, _))| s == hash)
            .map(|(t, _)| t.clone());
        println!("a template that runs it: {template:?}");
        if let Some(template) = template {
            let tree = 0xf001_0002;
            mission.put_player([3.0, 0.0, 0.0]);
            let adopted = mission.adopt_scripted(tree, &template, [0.0; 3]);
            // As a burnt tree's part machines go (`m2_engine::part_runner`):
            // a palm's trunk, then a canopy's crown, `Slice00`.
            let trunk = 0x501d_5fd7; // the palm's trunk part's node
            let crown = m2_assets::name_hash("Slice00");
            for (machine, state) in [
                (trunk, 0x0753_c8f7),
                (trunk, 0x0046_41a5),
                (crown, 0x3cc4_e770),
                (crown, 0x0753_c8f7),
            ] {
                mission.object_state_changed(tree, machine, state);
            }
            let mut played = Vec::new();
            for _ in 0..8 {
                mission.tick(0.25);
                for c in mission.take_commands() {
                    if let Command::MaterialAnimation { name, looped, .. } = c {
                        played.push((name, looped));
                    }
                }
            }
            println!("adopted {adopted}; burnt down, it plays {played:?}");
        }
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
    println!("{name} lines logged: {failures}");
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
