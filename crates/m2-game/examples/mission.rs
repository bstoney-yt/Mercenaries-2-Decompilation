//! Runs the game's scripts from your own installation against a world
//! built from its layers, headless, and reports what they do: engine calls
//! not written yet, commands (spawns, kills, moves, AI goals), printed
//! messages and event errors, tick by tick.
//!
//! ```text
//! cargo run -p m2-game --example mission -- <vz.wad> [options]
//!   --layers a,b       load layers whose name contains any of these at the
//!                      start (layers_static always); scripts load the rest
//!   --at x,y,z         where the player stands (default 0,0,0)
//!   --hero name        the player character's name (default Mattias)
//!   --run <lua>        Lua to run after the scripts start (repeatable)
//!   --teleport t,x,y,z move the player at time t (repeatable)
//!   --seconds n        how long to run (default 60)
//!   --step s           tick length (default 0.5)
//! ```
//!
//! Every module some script loads is imported first, then every other
//! script runs once, as in the `scripts` example; then the `--run` code,
//! then the ticks. In `--run` code every module is a global, and
//! `Scripts.<hash>` is an entry script's environment (to call the
//! bootstrap's `Init`, say). The player has the label of its name in lower
//! case (`mattias`), which is how scripts tell the heroes apart.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;
use std::rc::Rc;

use m2_assets::layer::LAYER_TYPE;
use m2_assets::lua::{Chunk, SCRIPT_TYPE};
use m2_assets::{BlockContents, Layer, WadArchive};
use m2_game::api::{self, Command, Object, ScriptWorld, Shared};
use m2_game::events::Events;
use m2_game::layers::{LayerOp, LayerState, finished, layer_name};
use m2_game::library::{KNOWN_MODULES, LOADERS, Library};
use m2_game::script::ScriptHost;

#[derive(Default)]
struct Options {
    wad: String,
    layers: Vec<String>,
    at: [f32; 3],
    hero: String,
    run: Vec<String>,
    teleports: Vec<(f64, [f32; 3])>,
    seconds: f64,
    step: f64,
}

fn main() -> ExitCode {
    match options().and_then(|o| run(&o)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn numbers(s: &str) -> Result<Vec<f64>> {
    Ok(s.split(',')
        .map(|n| n.trim().parse::<f64>())
        .collect::<std::result::Result<_, _>>()?)
}

fn point(n: &[f64]) -> [f32; 3] {
    [n[0] as f32, n[1] as f32, n[2] as f32]
}

fn options() -> Result<Options> {
    let mut o = Options {
        seconds: 60.0,
        step: 0.5,
        hero: "Mattias".into(),
        ..Options::default()
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--layers" => o.layers = value()?.split(',').map(str::to_owned).collect(),
            "--at" => {
                let n = numbers(&value()?)?;
                if n.len() != 3 {
                    return Err("--at takes x,y,z".into());
                }
                o.at = point(&n);
            }
            "--run" => o.run.push(value()?),
            "--hero" => o.hero = value()?,
            "--teleport" => {
                let n = numbers(&value()?)?;
                if n.len() != 4 {
                    return Err("--teleport takes t,x,y,z".into());
                }
                o.teleports.push((n[0], point(&n[1..])));
            }
            "--seconds" => o.seconds = value()?.parse()?,
            "--step" => o.step = value()?.parse()?,
            _ if o.wad.is_empty() && !a.starts_with("--") => o.wad = a,
            _ => return Err(format!("unknown argument {a}").into()),
        }
    }
    if o.wad.is_empty() {
        return Err("usage: mission <vz.wad> [options]; see the source".into());
    }
    if o.step <= 0.0 {
        return Err("--step must be positive".into());
    }
    Ok(o)
}

/// A layer's named entities: guid (the layer's entity id), name and
/// position.
type Entities = Vec<(u32, String, [f32; 3])>;

fn entities(layer: &Layer<'_>) -> Entities {
    let placements = layer.placements();
    layer
        .names()
        .into_iter()
        .map(|(entity, name)| {
            let at = placements.get(&entity).map_or([0.0; 3], |p| p.position);
            (entity, name.to_owned(), at)
        })
        .collect()
}

/// Puts a layer's entities in the world as objects scripts can find with
/// `Pg.GetGuidByName`, or takes them out.
fn place(world: &Shared, entities: &Entities, load: bool) {
    let mut w = world.borrow_mut();
    for (guid, name, at) in entities {
        if load {
            w.insert(*guid, Object::new(name, *at));
        } else {
            w.objects.remove(guid);
        }
    }
}

fn run(o: &Options) -> Result<()> {
    let mut wad = WadArchive::open(BufReader::new(File::open(&o.wad)?))?;
    let paths: Vec<String> = wad
        .index()
        .blocks()
        .iter()
        .map(|b| b.path.clone())
        .collect();
    let mut library = Library::default();
    let world: Shared = Rc::new(RefCell::new(ScriptWorld::default()));
    let mut layers: BTreeMap<String, Entities> = BTreeMap::new();
    for (block, path) in paths.iter().enumerate() {
        let decoded = wad.read_block(block)?;
        let Ok(contents) = BlockContents::parse(&decoded) else {
            continue;
        };
        for (entry, data) in contents.iter() {
            if entry.type_hash == SCRIPT_TYPE {
                if let Ok(c) = Chunk::from_asset(data) {
                    library.add(entry.name_hash, c);
                }
            } else if entry.type_hash == LAYER_TYPE
                && let Ok(layer) = Layer::parse(data)
            {
                layers
                    .entry(layer_name(path))
                    .or_default()
                    .extend(entities(&layer));
            }
        }
    }
    world.borrow_mut().layers.known = layers.keys().cloned().collect();
    // The static layers are always in; `--layers` adds others up front.
    let preload: Vec<String> = layers
        .keys()
        .filter(|n| {
            LayerState::is_static(n) || o.layers.iter().any(|l| n.contains(&l.to_ascii_lowercase()))
        })
        .cloned()
        .collect();
    for name in &preload {
        place(&world, &layers[name], true);
        world.borrow_mut().layers.loaded.insert(name.clone());
    }
    {
        let mut w = world.borrow_mut();
        let mut hero = Object::new(&o.hero, o.at);
        hero.player_controlled = true;
        // Scripts tell the heroes apart by these labels.
        hero.labels.push(o.hero.to_ascii_lowercase());
        w.insert(1, hero);
        w.player = Some(1);
    }
    println!(
        "{} scripts; {} layers; {} named entities in from {}",
        library.scripts().len(),
        layers.len(),
        world.borrow().objects.len() - 1,
        preload.join(", ")
    );
    for (name, hash) in KNOWN_MODULES {
        library.pin(name, hash);
    }
    let library = Rc::new(library);

    let host = ScriptHost::new()?;
    host.stub_missing_globals()?;
    api::install(host.lua(), &world)?;
    host.complete_libraries(&api::LIBRARIES)?;
    let events = Events::default();
    events.install(host.lua())?;
    host.install_library(Rc::clone(&library), true)?;

    let mut failures = Vec::new();
    // Modules by the name scripts spell them with, so `--run` code can
    // use them as globals without importing them first.
    let mut spelled: BTreeMap<String, String> = BTreeMap::new();
    for script in library.scripts() {
        for call in script.chunk.main.calls() {
            if let Some(name) = call
                .first
                .filter(|_| LOADERS.contains(&call.callee.as_str()))
            {
                spelled.entry(name.to_ascii_lowercase()).or_insert(name);
            }
        }
    }
    let globals = host.lua().globals();
    let import: mlua::Function = globals.get("import")?;
    for name in spelled.values() {
        match import.call::<mlua::Value>(name.as_str()) {
            Ok(module) => globals.raw_set(name.as_str(), module)?,
            Err(e) => failures.push(format!("module {name}: {e}")),
        }
    }
    let modules: Vec<u32> = host.modules().iter().filter_map(|m| m.1).collect();
    // Entry scripts' environments, for `--run` code: `Scripts.b4420059`.
    let entries = host.lua().create_table()?;
    for script in library.scripts() {
        if modules.contains(&script.hash) {
            continue;
        }
        match host.run_script(&library, script.hash) {
            Ok(env) => entries.raw_set(format!("{:08x}", script.hash), env)?,
            Err(e) => failures.push(format!("script {:08x}: {e}", script.hash)),
        }
    }
    globals.raw_set("Scripts", entries)?;
    // The bootstraps' module set-up.
    for f in host.init_modules()? {
        failures.push(format!("Init of {f}"));
    }
    println!(
        "started: {} modules, {} failures",
        host.modules().len(),
        failures.len()
    );
    for f in &failures {
        println!("  {}", first_lines(f));
    }
    settle("start", &host, &world, &events, &layers);

    for (i, code) in o.run.iter().enumerate() {
        println!();
        println!("run {}: {code}", i + 1);
        if let Err(e) = host.lua().load(code.as_str()).exec() {
            println!("  error: {}", first_lines(&e.to_string()));
        }
        settle("after run", &host, &world, &events, &layers);
    }

    let mut now = 0.0;
    let mut teleports = o.teleports.clone();
    teleports.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut teleports = teleports.into_iter().peekable();
    let mut waiting_since: Option<f64> = None;
    while now < o.seconds {
        now += o.step;
        while let Some(&(_, at)) = teleports.peek().filter(|t| t.0 <= now) {
            teleports.next();
            if let Some(hero) = world.borrow_mut().objects.get_mut(&1) {
                hero.position = at;
            }
            println!();
            println!("t={now:.1} player moved to {at:?}");
        }
        // Streaming is instant here: a waiting state ends a second after
        // it was asked for, and the scripts hear that it did.
        {
            let mut w = world.borrow_mut();
            if w.game_state.starts_with("WaitFor") {
                let since = *waiting_since.get_or_insert(now);
                if now - since >= 1.0 {
                    println!();
                    println!("t={now:.1} game state {} done", w.game_state);
                    w.game_state = "Game".into();
                    waiting_since = None;
                }
            }
        }
        let errors = events.update_shared(o.step, &world);
        for e in &errors {
            println!(
                "  t={now:.1} {} callback: {}",
                e.kind,
                first_lines(&e.error.to_string())
            );
        }
        settle(&format!("t={now:.1}"), &host, &world, &events, &layers);
    }
    println!();
    println!(
        "{} events pending at the end: {:?}",
        events.len(),
        events.kinds()
    );
    Ok(())
}

fn first_lines(s: &str) -> String {
    s.lines().take(4).collect::<Vec<_>>().join(" | ")
}

/// Reports what happened, then carries out the layer requests (at once:
/// headless, nothing has to stream) and reports what their callbacks did,
/// until no requests are left.
fn settle(
    when: &str,
    host: &ScriptHost,
    world: &Shared,
    events: &Events,
    layers: &BTreeMap<String, Entities>,
) {
    let mut commands = report(when, host, world, events);
    for _ in 0..50 {
        // Modules a script loaded "in the background" are ready.
        let ran = match host.run_deferred() {
            Ok((ran, errors)) => {
                for e in errors {
                    println!("  dynamic_import callback: {}", first_lines(&e));
                }
                ran
            }
            Err(e) => {
                println!("  dynamic_import callbacks: {e}");
                0
            }
        };
        if ran > 0 {
            commands.extend(report(
                &format!("{when}, {ran} modules loaded"),
                host,
                world,
                events,
            ));
        }
        let requests: Vec<(LayerOp, String)> = commands
            .into_iter()
            .filter_map(|c| match c {
                Command::Layer { op, name } => Some((op, name)),
                _ => None,
            })
            .collect();
        if requests.is_empty() {
            if ran == 0 {
                return;
            }
            commands = Vec::new();
            continue;
        }
        for (op, name) in requests {
            let found = layers.get(&name);
            if let Some(entities) = found {
                match op {
                    LayerOp::Load => place(world, entities, true),
                    LayerOp::Unload => place(world, entities, false),
                    LayerOp::Reload => {
                        place(world, entities, false);
                        place(world, entities, true);
                    }
                }
            }
            println!(
                "  layer {op:?} {name}: {}",
                found.map_or("part of layers_static".to_owned(), |e| format!(
                    "{} entities",
                    e.len()
                ))
            );
            for e in finished(world, &name, true) {
                println!("  layer callback: {}", first_lines(&e.to_string()));
            }
        }
        commands = report(&format!("{when}, layers done"), host, world, events);
    }
    println!("  layer requests still coming after 50 rounds; carrying on");
}

/// What happened since the last report, if anything; returns the commands.
fn report(when: &str, host: &ScriptHost, world: &Shared, events: &Events) -> Vec<Command> {
    let calls = host.take_calls();
    let (commands, printed) = {
        let mut w = world.borrow_mut();
        (w.take_commands(), std::mem::take(&mut w.printed))
    };
    if calls.is_empty() && commands.is_empty() && printed.is_empty() {
        return commands;
    }
    println!();
    println!("{when} ({} events pending):", events.len());
    for p in printed.iter().take(60) {
        println!("  print {p}");
    }
    for c in commands.iter().take(60) {
        println!("  command {c:?}");
    }
    // Repeated calls in a row collapse to one line with a count.
    let mut last: Option<(&str, usize)> = None;
    let mut lines = 0;
    let mut flush = |l: Option<(&str, usize)>| {
        if let Some((c, n)) = l
            && lines < 120
        {
            lines += 1;
            match n {
                1 => println!("  stub {c}"),
                _ => println!("  stub {c} x{n}"),
            }
        }
    };
    for c in &calls {
        match &mut last {
            Some((l, n)) if *l == c.as_str() => *n += 1,
            _ => {
                flush(last.take());
                last = Some((c.as_str(), 1));
            }
        }
    }
    flush(last);
    if lines >= 120 {
        println!("  ... {} stub calls in all", calls.len());
    }
    commands
}
