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
//! what they ask of the first one. Prints names, counts and the scripts'
//! errors only.

use std::fs::File;
use std::io::BufReader;

use m2_assets::WadArchive;
use m2_game::api::Command;
use m2_game::mission::{GameData, Mission, OBJECT_SCRIPTS};
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
    if name.eq_ignore_ascii_case("islandfortress")
        && let Some(&(_, first, at)) = on.first()
    {
        fortress(&mut mission, first, at, args.get(2).map(String::as_str))?;
    }
    println!("{name} lines logged: {failures}");
    Ok(())
}

/// The island fortress's part whose `DestroyMsg` its script waits for.
const FORTRESS_PART: u32 = 0xcf37_044a;

/// For `islandfortress`: the first fortress's node and object health; its
/// part `cf37044a` destroyed by `ObjectState.SendDamage` of 1; where its
/// model's state machine sends that part on `DestroyMsg` (with `<install
/// dir>` given: the model is read from it), told to the script's
/// `OnStateChange`; then the script's slices coming down over time, and
/// whether the whole fortress was destroyed.
fn fortress(mission: &mut Mission, guid: u32, at: [f32; 3], install: Option<&str>) -> Result<()> {
    mission.put_player([at[0] + 40.0, at[1], at[2]]);
    let status = |m: &Mission| {
        let mut w = m.world().borrow_mut();
        w.node_table(guid);
        let (health, most, alive) = w
            .objects
            .get(&guid)
            .map_or((0.0, 0.0, false), |o| (o.health, o.max_health, o.alive));
        let (left, nodes) = w.node_table(guid).map_or((0, 0), |t| {
            (
                t.entries().iter().filter(|e| e.health > 0).count(),
                t.entries().len(),
            )
        });
        format!(
            "object health {health:.1}/{most:.1}, alive {alive}, nodes with health {left}/{nodes}"
        )
    };
    println!("fortress {guid:#x}: {}", status(mission));
    let template = mission
        .world()
        .borrow()
        .placed_as
        .get(&guid)
        .cloned()
        .unwrap_or_default();
    let state = match install {
        Some(root) => part_state(root, &template)?,
        None => None,
    };
    mission
        .run(
            &format!(
                "ObjectState.SendDamage(Sys.StringToGuid('{guid:#x}'), \
                 Sys.StringToGuid('{FORTRESS_PART:#x}'), 1)"
            ),
            "fortress part",
        )
        .map_err(|e| e.to_string())?;
    let messages: Vec<String> = mission
        .take_commands()
        .into_iter()
        .filter_map(|c| match c {
            Command::NodeDamage { node, message, .. } => Some(format!("{node:#x}:{message:x?}")),
            _ => None,
        })
        .collect();
    println!(
        "part {FORTRESS_PART:#x} sent 1: messages {messages:?}; {}",
        status(mission)
    );
    match state {
        Some(s) => {
            println!("model: DestroyMsg sends part {FORTRESS_PART:#x} to state {s:#x}");
            mission.object_state_changed(guid, FORTRESS_PART, s);
        }
        None => println!("model: no DestroyMsg state found for part {FORTRESS_PART:#x}"),
    }
    let (mut kills, mut destroyed_at) = (0, None);
    let end = mission.now() + 120.0;
    while mission.now() < end {
        mission.tick(0.25);
        for c in mission.take_commands() {
            match c {
                Command::NodeDamage {
                    message: Some(_), ..
                } => kills += 1,
                Command::Kill(g) if g == guid && destroyed_at.is_none() => {
                    destroyed_at = Some(mission.now());
                }
                _ => {}
            }
        }
        mission.take_log();
    }
    println!(
        "after 120 s: {kills} node messages, {}; {}",
        status(mission),
        destroyed_at.map_or("not destroyed".into(), |t| format!(
            "destroyed (Kill at t={t:.1})"
        ))
    );
    // What the cascade left (nodes no part's breaking hides here, with no
    // game to hide them): each sent 1 until the fortress dies.
    let left: Vec<u32> = mission
        .world()
        .borrow_mut()
        .node_table(guid)
        .map(|t| {
            t.entries()
                .iter()
                .filter(|e| e.health > 0)
                .map(|e| e.node)
                .collect()
        })
        .unwrap_or_default();
    let mut sent = 0;
    for n in left {
        if !mission.world().borrow().objects[&guid].alive {
            break;
        }
        mission
            .run(
                &format!(
                    "ObjectState.SendDamage(Sys.StringToGuid('{guid:#x}'),                      Sys.StringToGuid('{n:#x}'), 1)"
                ),
                "fortress node",
            )
            .map_err(|e| e.to_string())?;
        sent += 1;
    }
    let killed = mission.take_commands().contains(&Command::Kill(guid));
    mission.tick(0.25);
    let deaths: Vec<String> = mission
        .take_log()
        .into_iter()
        .filter(|l| l.contains("OnDeath"))
        .collect();
    println!(
        "the rest sent 1, {sent} nodes: {}; Kill sent {killed}; OnDeath errors {deaths:?}",
        status(mission)
    );
    Ok(())
}

/// Where template `template`'s model sends part `FORTRESS_PART` on
/// `DestroyMsg` from its intact state.
fn part_state(root: &str, template: &str) -> Result<Option<u32>> {
    use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
    use m2_assets::model::{DESTROY_MSG, PartStates};
    let install = m2_engine::Install::find(std::path::Path::new(root))?;
    let mut source = install.open_world()?;
    let index = source.archive().index();
    let db_hash = index
        .assets()
        .iter()
        .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
        .map(|a| a.name_hash)
        .ok_or("no template database")?;
    let data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&data)?;
    let names = db.names();
    let Some(id) = names
        .iter()
        .find(|(_, n)| n.eq_ignore_ascii_case(template))
        .map(|(id, _)| *id)
    else {
        return Ok(None);
    };
    let Some(&model) = db.hashes("ModelName").get(&id) else {
        return Ok(None);
    };
    let data = source.model(model)?;
    let model = data.parse()?;
    let Some(states) = model.states.as_ref() else {
        return Ok(None);
    };
    let Some(m) = states.machines.iter().find(|m| {
        m.node
            .and_then(|n| model.nodes.get(usize::from(n)))
            .is_some_and(|n| n.name_hash == FORTRESS_PART)
    }) else {
        return Ok(None);
    };
    // A part whose InitState puts it nowhere stays there.
    let Some(s) = PartStates::intact_state(m).or_else(|| {
        m.states
            .iter()
            .position(|x| x.0 == m2_assets::model::STATE_INIT)
    }) else {
        return Ok(None);
    };
    Ok(m.acts.get(s).and_then(|a| {
        a.on.iter()
            .find(|(msg, _)| *msg == DESTROY_MSG)
            .map(|&(_, to)| to)
    }))
}
