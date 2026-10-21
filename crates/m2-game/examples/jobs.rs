//! Runs one job from your own installation headless, as the game's scripts
//! run it, and reports whether it starts, what the PDA marks once it is
//! tracked, and whether it counts, pays and completes as its targets fall
//! to the player.
//!
//! ```text
//! cargo run -p m2-game --example jobs -- <vz.wad> <job> [--seconds n] [--before]
//! ```
//!
//! The story is skipped to the job the developer way
//! (`Sys.GetSkipMission`), as `m2-alpha --skip-to` does; play runs until
//! the job is on (at most `--seconds`, default 120). The job is then
//! tracked as the PDA's map does ([`Mission::track_mission`]), and its
//! targets fall one by one, killed by her character
//! ([`Mission::kill_by`]): a set job's named targets, each verified where
//! it lies when the job is to verify them (the prompt taken, her photo's
//! action ended); for a bounty or a collection, the objects in play with
//! its label, up to its quota or 3. `--before` plays it as the engine did
//! before jobs were followed: not tracked, and nobody named as the
//! killer. A line per target, then one for the job:
//!
//! ```text
//! started AllJob002 at 3.0 s: MrxTaskJobVerifySet, 10 targets, label None, quota None
//! tracked: 10 markers on its targets
//! target 0xd12df AllJob002_01_Target: counted 0 -> 1, cash 800000 -> 1300000
//! completed AllJob002: 10 counted, 10 paid, of 10 tried, cash 800000 -> 19675000
//! ```

use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;

use m2_assets::WadArchive;
use m2_game::jobs::{JobView, read};
use m2_game::mission::{GameData, Mission};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const STEP: f64 = 0.5;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Plays `seconds` on. The session's whole account goes to stderr, its
/// errors to the summary too.
fn tick(m: &mut Mission, seconds: f64) {
    let mut t = 0.0;
    while t < seconds {
        m.tick(STEP);
        t += STEP;
    }
    for line in m.take_log() {
        if line.contains("error") || line.contains("ASSERT") {
            println!("  log: {line}");
        }
        eprintln!("{line}");
    }
    m.take_commands();
}

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let (mut wad, mut job, mut seconds, mut before) = (None, None, 120.0, false);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--seconds" => seconds = args.next().ok_or("--seconds n")?.parse()?,
            "--before" => before = true,
            _ if wad.is_none() => wad = Some(a),
            _ if job.is_none() => job = Some(a),
            _ => return Err(format!("unknown argument {a}").into()),
        }
    }
    let (Some(wad), Some(job)) = (wad, job) else {
        return Err("usage: jobs <vz.wad> <job> [--seconds n] [--before]".into());
    };
    let mut archive = WadArchive::open(BufReader::new(File::open(wad)?))?;
    let data = GameData::read(&mut archive)?;
    let mut m = Mission::new(data, "Mattias", [0.0; 3])?;
    m.set_skip_mission(&job, false);
    m.start();
    tick(&mut m, 0.0);
    let mut waited = 0.0;
    let mut s = read(&m, &job);
    while !s.active && !s.completed && waited < seconds {
        tick(&mut m, 1.0);
        waited += 1.0;
        s = read(&m, &job);
    }
    if !s.active {
        println!("not started {job} in {waited:.0} s");
        return Ok(());
    }
    let hero = m.world().borrow().player.ok_or("no player")?;
    let (cash0, _) = m.resources();
    println!(
        "started {job} at {waited:.1} s: {}, {} targets, label {:?}, quota {:?}",
        s.class.as_deref().unwrap_or("?"),
        s.targets.len(),
        s.label,
        s.quota
    );
    if !before {
        let tracked = m.track_mission(&job);
        tick(&mut m, 1.0);
        let w = m.world().borrow();
        let marked = w
            .hud
            .markers
            .values()
            .filter(|k| k.target.is_some_and(|t| s.targets.contains(&t)))
            .count();
        println!(
            "tracked{}: {marked} markers on its targets",
            if tracked { "" } else { " (no map callback)" }
        );
    }
    let targets: Vec<u32> = match &s.label {
        Some(label) if s.targets.is_empty() => {
            let n = s.quota.map_or(3, |q| q.max(1) as usize);
            let w = m.world().borrow();
            m.labelled(label)
                .into_iter()
                .filter(|(g, _)| w.objects.get(g).is_some_and(|o| o.alive))
                .take(n)
                .map(|(g, _)| g)
                .collect()
        }
        _ => s.targets.clone(),
    };
    let (mut parts, mut paid) = (0, 0);
    for &g in &targets {
        let name = m
            .world()
            .borrow()
            .objects
            .get(&g)
            .map_or_else(String::new, |o| o.name.clone());
        let was = read(&m, &job);
        let (c0, _) = m.resources();
        let at = m.world().borrow().objects.get(&g).map(|o| o.position);
        if let Some(at) = at {
            m.put_player(at);
        }
        if before {
            m.kill(g);
        } else {
            m.kill_by(g, hero);
        }
        tick(&mut m, 2.0);
        if s.is_verify() {
            verify(&mut m, g, hero, &name);
        }
        let now = read(&m, &job);
        let (c1, _) = m.resources();
        let counted = now.counted > was.counted || (now.completed && !was.completed);
        parts += usize::from(counted);
        paid += usize::from(c1 > c0);
        println!(
            "target {g:#x} {name}: counted {} -> {}, cash {c0:.0} -> {c1:.0}{}",
            was.counted,
            now.counted,
            if now.completed { ", job completed" } else { "" }
        );
    }
    tick(&mut m, 5.0);
    let end: JobView = read(&m, &job);
    let (cash1, _) = m.resources();
    println!(
        "{} {job}: {parts} counted, {paid} paid, of {} tried, cash {cash0:.0} -> {cash1:.0}",
        if end.completed {
            "completed"
        } else {
            "not completed"
        },
        targets.len()
    );
    Ok(())
}

/// Her verify prompt over the body `g`, once offered, taken; her photo's
/// action then ends, as its clip would.
fn verify(m: &mut Mission, g: u32, hero: u32, name: &str) {
    for _ in 0..20 {
        if let Some((_, _, text, _)) = m.context_actions().into_iter().find(|a| a.0 == g) {
            m.take_context_action(g);
            tick(m, 2.0);
            m.action_finished(hero);
            tick(m, 4.0);
            println!("  took {text} on {name}");
            return;
        }
        tick(m, 0.5);
    }
    println!("  no verify prompt on {name}");
}
