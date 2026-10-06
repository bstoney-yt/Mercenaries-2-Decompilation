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
//!   --hijack h,v,c     after the drivers, spawn vehicle template v
//!                      crewed by template c beside the player and
//!                      play hijack script h (its name hash, hex) on it
//!   --pad good|bad|none  how the player answers the hijack's prompts
//!                      (default good: the right button at once; bad:
//!                      the wrong one; none: never)
//!   --action-seconds n how long each character action lasts before the
//!                      game reports it done (default 2; the clips'
//!                      real lengths are the game's)
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
use m2_game::api::Command;
use m2_game::hijack::HijackCommand;
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
    hijack: Option<(u32, String, String)>,
    pad: String,
    action_seconds: f64,
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
        pad: "good".into(),
        action_seconds: 2.0,
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
            "--hijack" => {
                let v = value()?;
                let parts: Vec<&str> = v.splitn(3, ',').collect();
                let [h, vehicle, crew] = parts[..] else {
                    return Err("--hijack takes script,vehicle,crewman".into());
                };
                let h = u32::from_str_radix(h.trim_start_matches("0x"), 16)?;
                o.hijack = Some((h, vehicle.to_owned(), crew.to_owned()));
            }
            "--pad" => o.pad = value()?,
            "--action-seconds" => o.action_seconds = value()?.parse()?,
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

    let mut hijack = match &o.hijack {
        Some((script, vehicle, crew)) => Some(Hijack::start(&mut mission, *script, vehicle, crew)?),
        None => None,
    };

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
        if let Some(h) = &mut hijack {
            h.pad(&mut mission, &o.pad, now);
        }
        mission.tick(o.step);
        match &mut hijack {
            Some(h) => h.follow(&mut mission, o.action_seconds, now),
            None => print(&mut mission),
        }
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

/// A hijack played headless: the game's side, as far as the script needs
/// it (actions end after a set time; the pad answers each prompt).
struct Hijack {
    /// Characters' actions and when they end.
    actions: Vec<(u32, f64)>,
    /// Prompts already answered, by when they showed (seconds left).
    answered: Vec<(String, u32)>,
    /// The button last tapped (alternating).
    last: Option<u32>,
}

impl Hijack {
    fn start(mission: &mut Mission, script: u32, vehicle: &str, crew: &str) -> Result<Self> {
        let hero = mission.world().borrow().player.ok_or("no player")?;
        let at = mission.world().borrow().objects[&hero].position;
        let spawn = |m: &mut Mission, what: &str| -> Result<u32> {
            let v: mlua::Value = m
                .lua()
                .load(format!(
                    "return Pg.Spawn({what:?}, {}, {}, {}, 0)",
                    at[0] + 3.0,
                    at[1],
                    at[2]
                ))
                .eval()?;
            m2_game::events::as_guid(&v).ok_or_else(|| format!("no template {what}").into())
        };
        let vehicle = spawn(mission, vehicle)?;
        let crewman = spawn(mission, crew)?;
        let started = mission.start_hijack(hero, crewman, vehicle, script);
        println!();
        println!(
            "hijack {script:08x}: hero {hero:#x} crewman {crewman:#x} vehicle {vehicle:#x}: started {started}"
        );
        let mut h = Self {
            actions: Vec::new(),
            answered: Vec::new(),
            last: None,
        };
        h.follow(mission, 2.0, 0.0);
        Ok(h)
    }

    fn pad(&mut self, mission: &mut Mission, pad: &str, now: f64) {
        for g in mission.minigames() {
            let key = (g.mode.clone(), g.buttons.first().copied().unwrap_or(0));
            let fresh = !self.answered.contains(&key);
            if fresh {
                println!(
                    "t={now:.1} prompt {} {:?} {:.2}s left",
                    g.mode, g.buttons, g.left
                );
            }
            if pad == "none" {
                self.answered.push(key);
                continue;
            }
            let right = g.buttons.first().copied().unwrap_or(0);
            let wrong = if right == 6 { 5 } else { 6 };
            match g.mode.as_str() {
                "press" if fresh => {
                    mission.press(if pad == "good" { right } else { wrong });
                }
                "hold" if fresh => mission.hold(right, pad == "good"),
                "tap" if pad == "good" => mission.press(right),
                "alternate" if pad == "good" => {
                    let b = match (self.last, g.buttons.as_slice()) {
                        (Some(l), [a, b]) if l == *a => *b,
                        (_, [a, ..]) => *a,
                        _ => right,
                    };
                    self.last = Some(b);
                    mission.press(b);
                }
                _ => {}
            }
            if fresh {
                self.answered.push(key);
            }
        }
        // A prompt that is gone may come again (a section replayed).
        let open: Vec<(String, u32)> = mission
            .minigames()
            .iter()
            .map(|g| (g.mode.clone(), g.buttons.first().copied().unwrap_or(0)))
            .collect();
        self.answered.retain(|k| open.contains(k));
    }

    /// Prints the log and the hijack's commands; ends actions due.
    fn follow(&mut self, mission: &mut Mission, seconds: f64, now: f64) {
        for line in mission.take_log() {
            println!("{line}");
        }
        for c in mission.take_commands() {
            match c {
                Command::Hijack(h) => {
                    if let HijackCommand::State { guid, action, .. } = &h {
                        self.actions.retain(|a| a.0 != *guid);
                        if !action.eq_ignore_ascii_case("Idle") {
                            self.actions.push((*guid, now + seconds));
                        }
                    }
                    println!("t={now:.1} hijack {h:?}");
                }
                Command::Remove(g) | Command::Kill(g) | Command::ExitVehicle(g) => {
                    println!("t={now:.1} {c:?} ({g:#x})");
                }
                _ => {}
            }
        }
        let (done, left): (Vec<_>, Vec<_>) = self.actions.iter().partition(|a| a.1 <= now);
        self.actions = left;
        for (g, _) in done {
            mission.action_finished(g);
        }
    }
}
