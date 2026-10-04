//! Runs the game's scripts from your own installation against a world
//! built from its layers, headless, and reports what they do: engine calls
//! not written yet, commands (spawns, kills, moves, AI goals), printed
//! messages and event errors, tick by tick.
//!
//! ```text
//! cargo run -p m2-game --example mission -- <vz.wad> [options]
//!   --layers a,b       also load layers whose name contains any of these
//!                      at the start (layers_static always); scripts load
//!                      the rest
//!   --at x,y,z         where the player stands (default 0,0,0)
//!   --hero name        the player character's name (default Mattias)
//!   --driver file.lua  Lua to run before play starts (repeatable), such
//!                      as examples/drivers/vzacon001.lua
//!   --run <lua>        the same, inline (repeatable)
//!   --no-start         don't start play (the bootstrap's Init and the
//!                      "GUI loaded" step); only set the modules up
//!   --teleport t,x,y,z move the player at time t (repeatable)
//!   --seconds n        how long to run (default 60)
//!   --step s           tick length (default 0.5)
//! ```
//!
//! Every module some script loads is imported first, then every other
//! script runs once, as in the `scripts` example; then the drivers and
//! `--run` code, then play starts (unless `--no-start`), then the ticks. In
//! driver code every module is a global, and `Scripts.<hash>` is an entry
//! script's environment. The player has the label of its name in lower
//! case (`mattias`), which is how scripts tell the heroes apart.

use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;

use m2_assets::WadArchive;
use m2_game::mission::{GameData, Mission};

#[derive(Default)]
struct Options {
    wad: String,
    layers: Vec<String>,
    at: [f32; 3],
    hero: String,
    /// Code to run before play starts: a name for errors, and the code.
    run: Vec<(String, String)>,
    start: bool,
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
        start: true,
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
            "--driver" => {
                let path = value()?;
                let code = std::fs::read_to_string(&path)?;
                o.run.push((path, code));
            }
            "--run" => {
                let n = o.run.len() + 1;
                o.run.push((format!("run {n}"), value()?));
            }
            "--no-start" => o.start = false,
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

fn print(mission: &mut Mission) {
    for line in mission.take_log() {
        println!("{line}");
    }
    // Commands are in the log; the game would carry them out.
    mission.take_commands();
}

fn run(o: &Options) -> Result<()> {
    let mut wad = WadArchive::open(BufReader::new(File::open(&o.wad)?))?;
    let data = GameData::read(&mut wad)?;
    let mut mission = Mission::new(data, &o.hero, o.at)?;
    for part in &o.layers {
        println!("preloaded {}", mission.preload(part).join(", "));
    }
    print(&mut mission);
    for (name, code) in &o.run {
        println!();
        println!("{name}");
        if let Err(e) = mission.run(code, name) {
            println!("  error: {e}");
        }
        print(&mut mission);
    }
    if o.start {
        println!();
        println!("start");
        mission.start();
        print(&mut mission);
    }

    let mut teleports = o.teleports.clone();
    teleports.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut teleports = teleports.into_iter().peekable();
    while mission.now() < o.seconds {
        let now = mission.now() + o.step;
        while let Some(&(_, at)) = teleports.peek().filter(|t| t.0 <= now) {
            teleports.next();
            mission.put_player(at);
            println!();
            println!("t={now:.1} player moved to {at:?}");
        }
        mission.tick(o.step);
        print(&mut mission);
    }
    println!();
    println!("modules loaded while running (name -> script):");
    for (name, hash) in mission.late_modules() {
        println!(
            "  {name} -> {}",
            hash.map_or("unresolved".into(), |h| format!("{h:08x}"))
        );
    }
    println!();
    let pending = mission.pending_events();
    println!("{} events pending at the end: {pending:?}", pending.len());
    Ok(())
}
