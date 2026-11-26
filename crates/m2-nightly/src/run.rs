//! Playing the night's contracts one at a time: wait for the GPU slot,
//! start m2-alpha with autoplay under a hard wall-clock limit, watch its
//! log, stop it when it hangs, sticks or the window closes, then run the
//! detectors and write each run's `result.json` and `summary.txt`.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Context;

use crate::contracts::alpha_args;
use crate::detect::{Exit, Killed, Thresholds, analyse, issue_kinds, stuck_now};
use crate::gpu;
use crate::night::{Night, Outcome, RunResult, report, utc_stamp};

/// Everything a night needs.
#[derive(Debug, Clone)]
pub struct NightConfig {
    /// The m2-alpha exe (on a local disk: the NAS share won't start one).
    pub exe: PathBuf,
    /// Extra arguments put before the contract's own (for tests).
    pub exe_prefix_args: Vec<String>,
    pub install: PathBuf,
    /// The night's folder.
    pub out: PathBuf,
    pub run_id: String,
    pub head: String,
    /// In play order.
    pub contracts: Vec<String>,
    pub contracts_listed: usize,
    pub thresholds: Thresholds,
    /// Each run's wall-clock limit, seconds.
    pub wall_seconds: u64,
    /// Unix time the window closes: the playing run is stopped (aborted)
    /// and the GPU released.
    pub deadline_unix: Option<u64>,
    /// No new run starts with less than this left before the deadline.
    pub min_start_seconds: u64,
    pub lock: PathBuf,
    /// Process-name prefixes that hold the GPU slot.
    pub busy: Vec<String>,
    /// Free checks in a row before the slot counts as free.
    pub free_checks: u32,
    /// Seconds between GPU checks.
    pub gpu_poll_seconds: u64,
    /// A lock this old (seconds) is taken over.
    pub stale_seconds: u64,
    /// Seconds between looks at a playing run.
    pub poll_seconds: u64,
    /// Seconds between live stuck checks.
    pub stuck_check_seconds: u64,
    /// Keep the pictures of passing runs (failures keep theirs always).
    pub keep_pictures: bool,
    /// The contract-to-last-played map for turn taking (`last-played.json`).
    pub state: Option<PathBuf>,
}

fn now() -> u64 {
    gpu::now_unix()
}

fn left(cfg: &NightConfig) -> Option<u64> {
    cfg.deadline_unix.map(|d| d.saturating_sub(now()))
}

/// Waits until no process holds the GPU slot for `free_checks` checks in a
/// row; false if the deadline came first.
fn wait_for_slot(cfg: &NightConfig) -> bool {
    let mut free = 0;
    loop {
        let procs = gpu::processes();
        let held = gpu::busy(&procs, &cfg.busy);
        if held.is_empty() {
            free += 1;
            if free >= cfg.free_checks.max(1) {
                return true;
            }
        } else {
            if free > 0 || cfg.free_checks > 0 {
                eprintln!(
                    "nightly: GPU slot held by {}; waiting",
                    held.iter()
                        .map(|(n, p)| format!("{n} ({p})"))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            free = 0;
        }
        if left(cfg).is_some_and(|l| l <= cfg.min_start_seconds) {
            return false;
        }
        std::thread::sleep(Duration::from_secs(cfg.gpu_poll_seconds));
    }
}

fn spawn(cfg: &NightConfig, id: &str, dir: &Path) -> anyhow::Result<Child> {
    let err = File::create(dir.join(format!("{id}.err")))?;
    let log = File::create(dir.join(format!("{id}.log")))?;
    let picture = dir.join(format!("{id}.ppm"));
    let mut cmd = Command::new(&cfg.exe);
    cmd.args(&cfg.exe_prefix_args)
        .args(alpha_args(
            id,
            &cfg.install,
            &picture,
            cfg.thresholds.mission_seconds as u32,
        ))
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(err);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS: no console, and
        // the VM's other work keeps ahead of it.
        cmd.creation_flags(0x0800_0000 | 0x0000_4000);
    }
    cmd.spawn()
        .with_context(|| format!("starting {}", cfg.exe.display()))
}

/// Plays one contract and returns how it went.
pub fn run_one(cfg: &NightConfig, id: &str) -> anyhow::Result<RunResult> {
    let dir = cfg.out.join(id);
    std::fs::create_dir_all(&dir)?;
    let err_path = dir.join(format!("{id}.err"));
    let mut child = spawn(cfg, id, &dir)?;
    let t0 = Instant::now();
    let mut grown = Instant::now();
    let mut last_len = 0u64;
    let mut last_stuck_check = Instant::now();
    let poll = Duration::from_secs(cfg.poll_seconds.max(1));
    let mut killed: Option<Killed> = None;
    let code = loop {
        if let Some(status) = child.try_wait()? {
            break status.code();
        }
        let len = std::fs::metadata(&err_path).map_or(0, |m| m.len());
        if len != last_len {
            last_len = len;
            grown = Instant::now();
        }
        let why = if t0.elapsed().as_secs() >= cfg.wall_seconds {
            Some(Killed::Wall)
        } else if cfg.deadline_unix.is_some_and(|d| now() >= d) {
            Some(Killed::Deadline)
        } else if grown.elapsed().as_secs_f64() >= cfg.thresholds.hang_seconds {
            Some(Killed::Hang)
        } else if last_stuck_check.elapsed().as_secs() >= cfg.stuck_check_seconds {
            last_stuck_check = Instant::now();
            let text =
                String::from_utf8_lossy(&std::fs::read(&err_path).unwrap_or_default()).into_owned();
            stuck_now(&text, &cfg.thresholds).map(|_| Killed::Stuck)
        } else {
            None
        };
        if let Some(why) = why {
            eprintln!("nightly: {id}: stopping it ({why:?})");
            let _ = child.kill();
            let _ = child.wait();
            killed = Some(why);
            break None;
        }
        std::thread::sleep(poll);
    };
    let exit = Exit {
        code,
        killed,
        wall_seconds: t0.elapsed().as_secs_f64(),
    };
    let text = String::from_utf8_lossy(&std::fs::read(&err_path).unwrap_or_default()).into_owned();
    let text = text.trim_start_matches('\u{feff}');
    let a = analyse(text, &exit, &cfg.thresholds);
    let outcome = if killed == Some(Killed::Deadline) {
        Outcome::Aborted
    } else if a.failures.is_empty() {
        Outcome::Pass
    } else {
        Outcome::Fail
    };
    let result = RunResult {
        contract: id.into(),
        outcome,
        issue_kinds: if outcome == Outcome::Fail {
            issue_kinds(&a.failures)
        } else {
            Vec::new()
        },
        failures: a.failures.clone(),
        exit,
        completed: a.completed,
        played_seconds: a.summary.end.played.or(Some(a.summary.last_time)),
        hops: a.summary.end.hops,
        lua_errors: a.summary.lua_errors.len(),
        fell_through_lines: a.fell_through_lines,
        lowest_y: a.lowest_y,
        worst_tick_ms: a.worst_tick_ms,
        worst_fps: a.worst_fps,
        longest_gap_seconds: a.longest_gap.0,
        note: None,
        folder: id.into(),
    };
    std::fs::write(
        dir.join("result.json"),
        serde_json::to_string_pretty(&result)?,
    )?;
    let mut summary = format!("{id}: {:?}\n", result.outcome);
    for f in &result.failures {
        summary.push_str(&format!("  {}: {}\n", f.kind.name(), f.detail));
    }
    summary.push('\n');
    summary.push_str(&m2_runsum::to_text(&a.summary, &format!("{id}.err")));
    std::fs::write(dir.join("summary.txt"), summary)?;
    if outcome == Outcome::Pass && !cfg.keep_pictures {
        for e in std::fs::read_dir(&dir)?.flatten() {
            if e.path().extension().is_some_and(|x| x == "ppm") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    Ok(result)
}

/// Reads the turn-taking state (empty when missing or unreadable).
pub fn read_state(path: &Path) -> BTreeMap<String, u64> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_night(cfg: &NightConfig, night: &Night) -> anyhow::Result<()> {
    std::fs::write(
        cfg.out.join("night.json"),
        serde_json::to_string_pretty(night)?,
    )?;
    std::fs::write(cfg.out.join("report.md"), report(night, 3000))?;
    Ok(())
}

/// Plays the night: takes the GPU lock (or gives up), runs each contract
/// in turn, and writes `night.json` and `report.md` as it goes, so a
/// night cut short still leaves its results.
pub fn run_night(cfg: &NightConfig) -> anyhow::Result<Night> {
    std::fs::create_dir_all(&cfg.out)?;
    let mut night = Night {
        run_id: cfg.run_id.clone(),
        head: cfg.head.clone(),
        started_unix: now(),
        finished_unix: now(),
        deadline_unix: cfg.deadline_unix,
        contracts_listed: cfg.contracts_listed,
        results: Vec::new(),
    };
    let purpose = format!("nightly autoplay {}", cfg.run_id);
    // Take the lock, waiting for it until the deadline.
    let _lock = loop {
        match gpu::try_lock(&cfg.lock, &purpose, cfg.stale_seconds)? {
            Ok(l) => break Some(l),
            Err(owner) => {
                eprintln!(
                    "nightly: {} held by pid {} ({}); waiting",
                    cfg.lock.display(),
                    owner.pid,
                    owner.purpose
                );
                if left(cfg).is_some_and(|l| l <= cfg.min_start_seconds) {
                    break None;
                }
                std::thread::sleep(Duration::from_secs(cfg.gpu_poll_seconds));
            }
        }
    };
    if _lock.is_none() {
        for id in &cfg.contracts {
            night
                .results
                .push(RunResult::skipped(id, "the GPU lock stayed held"));
        }
        night.finished_unix = now();
        write_night(cfg, &night)?;
        return Ok(night);
    }
    let mut state = cfg.state.as_deref().map(read_state).unwrap_or_default();
    for id in &cfg.contracts {
        if left(cfg).is_some_and(|l| l <= cfg.min_start_seconds) {
            night
                .results
                .push(RunResult::skipped(id, "the window closed"));
            continue;
        }
        if !wait_for_slot(cfg) {
            night
                .results
                .push(RunResult::skipped(id, "the GPU slot stayed busy"));
            continue;
        }
        eprintln!("nightly: {id}: starting ({})", utc_stamp(now()));
        let r = match run_one(cfg, id) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("nightly: {id}: {e:#}");
                RunResult::skipped(id, "m2-alpha could not be started")
            }
        };
        eprintln!(
            "nightly: {id}: {:?} {}",
            r.outcome,
            r.issue_kinds
                .iter()
                .map(|k| k.name())
                .collect::<Vec<_>>()
                .join(",")
        );
        if r.outcome != Outcome::Skipped {
            state.insert(id.clone(), now());
            if let Some(p) = &cfg.state {
                let _ = std::fs::write(p, serde_json::to_string_pretty(&state)?);
            }
        }
        night.results.push(r);
        night.finished_unix = now();
        write_night(cfg, &night)?;
    }
    night.finished_unix = now();
    write_night(cfg, &night)?;
    Ok(night)
}
