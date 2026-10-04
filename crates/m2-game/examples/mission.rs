//! Runs the game's scripts from your own installation against a world
//! built from its layers, headless, and reports what they do: engine calls
//! not written yet, commands (spawns, kills, moves, AI goals), printed
//! messages and event errors, tick by tick.
//!
//! ```text
//! cargo run -p m2-game --example mission -- <vz.wad> [options]
//!   --layers a,b       also take entities from blocks whose path contains
//!                      any of these (layers_static always)
//!   --at x,y,z         where the player stands (default 0,0,0)
//!   --run <lua>        Lua to run after the scripts start (repeatable)
//!   --teleport t,x,y,z move the player at time t (repeatable)
//!   --seconds n        how long to run (default 60)
//!   --step s           tick length (default 0.5)
//! ```
//!
//! Every module some script loads is imported first, then every other
//! script runs once, as in the `scripts` example; then the `--run` code,
//! then the ticks.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;
use std::rc::Rc;

use m2_assets::layer::LAYER_TYPE;
use m2_assets::lua::{Chunk, SCRIPT_TYPE};
use m2_assets::{BlockContents, Layer, WadArchive};
use m2_game::api::{self, Object, ScriptWorld, Shared};
use m2_game::events::Events;
use m2_game::library::Library;
use m2_game::script::ScriptHost;

#[derive(Default)]
struct Options {
    wad: String,
    layers: Vec<String>,
    at: [f32; 3],
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

/// Entities with a name, from the layers asked for, as objects scripts can
/// find with `Pg.GetGuidByName`; guids are the layers' entity ids.
fn add_entities(world: &mut ScriptWorld, layer: &Layer<'_>) -> usize {
    let placements = layer.placements();
    let mut added = 0;
    for (entity, name) in layer.names() {
        let at = placements.get(&entity).map_or([0.0; 3], |p| p.position);
        world.insert(entity, Object::new(name, at));
        added += 1;
    }
    added
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
    let mut taken: BTreeMap<String, usize> = BTreeMap::new();
    for (block, path) in paths.iter().enumerate() {
        let wanted =
            path.contains("layers_static") || o.layers.iter().any(|l| path.contains(l.as_str()));
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
                && wanted
                && let Ok(layer) = Layer::parse(data)
            {
                *taken.entry(path.clone()).or_default() +=
                    add_entities(&mut world.borrow_mut(), &layer);
            }
        }
    }
    {
        let mut w = world.borrow_mut();
        let mut hero = Object::new("hero", o.at);
        hero.player_controlled = true;
        w.insert(1, hero);
        w.player = Some(1);
    }
    println!(
        "{} scripts; {} named entities from {} blocks:",
        library.scripts().len(),
        world.borrow().objects.len() - 1,
        taken.len()
    );
    for (path, n) in &taken {
        println!("  {path}: {n}");
    }
    let library = Rc::new(library);

    let host = ScriptHost::new()?;
    host.stub_missing_globals()?;
    api::install(host.lua(), &world)?;
    let events = Events::default();
    events.install(host.lua())?;
    host.install_library(Rc::clone(&library), true)?;

    let mut failures = Vec::new();
    let import: mlua::Function = host.lua().globals().get("import")?;
    for name in library.module_names() {
        if let Err(e) = import.call::<mlua::Value>(name) {
            failures.push(format!("module {name}: {e}"));
        }
    }
    let modules: Vec<u32> = host.modules().iter().filter_map(|m| m.1).collect();
    for script in library.scripts() {
        if !modules.contains(&script.hash)
            && let Err(e) = host.run_script(&library, script.hash)
        {
            failures.push(format!("script {:08x}: {e}", script.hash));
        }
    }
    println!(
        "started: {} modules, {} failures",
        host.modules().len(),
        failures.len()
    );
    for f in &failures {
        println!("  {}", first_lines(f));
    }
    report("start", &host, &world, &events);

    for (i, code) in o.run.iter().enumerate() {
        println!();
        println!("run {}: {code}", i + 1);
        if let Err(e) = host.lua().load(code.as_str()).exec() {
            println!("  error: {}", first_lines(&e.to_string()));
        }
        report("after run", &host, &world, &events);
    }

    let mut now = 0.0;
    let mut teleports = o.teleports.clone();
    teleports.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut teleports = teleports.into_iter().peekable();
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
        let errors = events.update_shared(o.step, &world);
        for e in &errors {
            println!(
                "  t={now:.1} {} callback: {}",
                e.kind,
                first_lines(&e.error.to_string())
            );
        }
        report(&format!("t={now:.1}"), &host, &world, &events);
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

/// What happened since the last report, if anything.
fn report(when: &str, host: &ScriptHost, world: &Shared, events: &Events) {
    let calls = host.take_calls();
    let (commands, printed) = {
        let mut w = world.borrow_mut();
        (w.take_commands(), std::mem::take(&mut w.printed))
    };
    if calls.is_empty() && commands.is_empty() && printed.is_empty() {
        return;
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
}
