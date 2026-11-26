//! The detectors: what in a run's log and exit says the contract broke.
//!
//! Every threshold here is a tooling heuristic (invented), not a game rule;
//! each is a [`Thresholds`] field so a run can change it. The log lines
//! read are m2-alpha's own (`autoplay <t> s: ...`, `pace: ...`, the
//! end-of-run `autoplay: <t> s played` line, panics), parsed with
//! [`m2_runsum`] where it already reads them.

use serde::{Deserialize, Serialize};

use m2_runsum::{Config, Summary, summarize};

/// What broke. The order is the order of precedence when one cause
/// explains another (see [`issue_kinds`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// The Rust code panicked (a `panicked at` line, or exit code 101).
    Panic,
    /// The process died abnormally (an access violation, a stack overflow,
    /// an abort: any exit code other than 0, 1 and 101).
    Crash,
    /// m2-alpha stopped on an error of its own (`Error: ...`, exit code 1).
    Error,
    /// No log output for a long stretch of wall time: the process hung.
    Hang,
    /// The hero fell through the world: autoplay's own `fell through` line,
    /// her position not a number, or a logged height under the floor.
    FellThrough,
    /// The failed contract's fanfare was answered with Retry, but the
    /// contract did not start again (autoplay's next state is no contract).
    RetryFailed,
    /// The scripts cancelled (failed) the contract: a race's timer ran
    /// out, a target lost, she died... (autoplay's contract cancelled).
    ContractCancelled,
    /// The contract's objectives and mission state stayed the same for a
    /// long stretch of play.
    Stuck,
    /// The frame time stayed over its ceiling (or the frame rate under its
    /// floor) for a sustained stretch.
    FpsCliff,
    /// The run went past its wall-clock limit and was stopped.
    Timeout,
    /// The run ended without the contract completed (and nothing above
    /// explains it).
    NotCompleted,
}

impl Kind {
    pub const ALL: [Kind; 11] = [
        Kind::Panic,
        Kind::Crash,
        Kind::Error,
        Kind::Hang,
        Kind::FellThrough,
        Kind::RetryFailed,
        Kind::ContractCancelled,
        Kind::Stuck,
        Kind::FpsCliff,
        Kind::Timeout,
        Kind::NotCompleted,
    ];

    /// The kebab-case name used in JSON, issue keys and titles.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Panic => "panic",
            Kind::Crash => "crash",
            Kind::Error => "error",
            Kind::Hang => "hang",
            Kind::FellThrough => "fell-through",
            Kind::RetryFailed => "retry-failed",
            Kind::ContractCancelled => "contract-cancelled",
            Kind::Stuck => "stuck",
            Kind::FpsCliff => "fps-cliff",
            Kind::Timeout => "timeout",
            Kind::NotCompleted => "not-completed",
        }
    }

    pub fn from_name(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == s)
    }

    /// A cause that, when present, explains a contract not completing.
    fn explains_incomplete(self) -> bool {
        matches!(
            self,
            Kind::Panic
                | Kind::Crash
                | Kind::Error
                | Kind::Hang
                | Kind::FellThrough
                | Kind::RetryFailed
                | Kind::ContractCancelled
                | Kind::Stuck
        )
    }
}

/// One thing found wrong in a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Failure {
    pub kind: Kind,
    /// Seconds of play where it began, when the log says.
    pub at: Option<f64>,
    /// Numbers and our own code locations only (it goes into issues):
    /// never game text, log lines or paths on the VM.
    pub detail: String,
}

/// Why the runner stopped the process itself, if it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Killed {
    /// Its wall-clock limit was up.
    Wall,
    /// No log output for [`Thresholds::hang_seconds`].
    Hang,
    /// The live stuck check fired.
    Stuck,
    /// The night's window closed: the run is aborted, not failed.
    Deadline,
}

/// How the process ended.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Exit {
    /// The exit code (on Windows an NTSTATUS for a crash, read as i32).
    pub code: Option<i32>,
    pub killed: Option<Killed>,
    /// Wall seconds the process ran.
    pub wall_seconds: f64,
}

/// The detectors' thresholds. All invented tooling heuristics.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Thresholds {
    /// Seconds of play with no objective or mission-state change before a
    /// run counts as stuck.
    pub stuck_seconds: f64,
    /// Wall seconds with no log output before the process counts as hung.
    pub hang_seconds: f64,
    /// A logged height (y, metres) under this is under the world. The
    /// story's beaches log about -36 (sea level), so this is far below it.
    pub floor_y: f64,
    /// `pace: autoplay` ms a tick over this is a slow minute (a healthy
    /// offscreen run logs about 10).
    pub tick_ms_ceiling: f64,
    /// `pace: <n> fps` under this is a slow stretch (windowed runs).
    pub fps_floor: f64,
    /// Consecutive slow `pace:` lines that make a cliff (each `pace:
    /// autoplay` line covers a minute of play).
    pub slow_lines: usize,
    /// The run's `--mission-seconds`, to tell the time limit apart.
    pub mission_seconds: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            stuck_seconds: 600.0,
            hang_seconds: 300.0,
            floor_y: -150.0,
            tick_ms_ceiling: 50.0,
            fps_floor: 15.0,
            slow_lines: 3,
            mission_seconds: 1500.0,
        }
    }
}

/// What the detectors found in one run.
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    pub summary: Summary,
    pub completed: bool,
    pub failures: Vec<Failure>,
    pub fell_through_lines: usize,
    pub lowest_y: Option<f64>,
    pub worst_tick_ms: Option<f64>,
    pub worst_fps: Option<f64>,
    /// The longest stretch without progress, seconds of play, and where it
    /// began.
    pub longest_gap: (f64, f64),
}

/// `autoplay <t> s: <body>`.
fn autoplay_line(line: &str) -> Option<(f64, &str)> {
    let rest = line.strip_prefix("autoplay ")?;
    let (num, after) = rest.split_once(' ')?;
    let t = num.parse::<f64>().ok()?;
    Some((t, after.strip_prefix("s: ")?))
}

/// `x, y, z]` at the start of `s`.
fn triple(s: &str) -> Option<[f64; 3]> {
    let inner = &s[..s.find(']')?];
    let mut it = inner.split(',').map(|v| v.trim().parse::<f64>());
    let p = [it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?];
    it.next().is_none().then_some(p)
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = s.find(start)? + start.len();
    let rest = &s[i..];
    Some(&rest[..rest.find(end)?])
}

/// A slow `pace:` line: (seconds of play, the measure, whether it is ms a
/// tick rather than fps).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pace {
    t: f64,
    value: f64,
    tick: bool,
}

/// `pace: autoplay <t> s of play: <r> s of play a second, <ms> ms a tick, ...`
/// or `pace: <fps> fps over <s> s, ...` (timed by the last autoplay line).
fn pace(line: &str, clock: f64) -> Option<Pace> {
    let rest = line.strip_prefix("pace: ")?;
    if let Some(rest) = rest.strip_prefix("autoplay ") {
        let t = rest.split(' ').next()?.parse::<f64>().ok()?;
        let ms = between(rest, " s of play a second, ", " ms a tick")?
            .trim()
            .parse::<f64>()
            .ok()?;
        return Some(Pace {
            t,
            value: ms,
            tick: true,
        });
    }
    let (n, _) = rest.split_once(" fps over ")?;
    Some(Pace {
        t: clock,
        value: n.trim().parse().ok()?,
        tick: false,
    })
}

/// The `file.rs:line:col` a panic line names, if it names one of ours.
pub fn panic_location(message: &str) -> Option<String> {
    let i = message.find("panicked at ")? + "panicked at ".len();
    let loc = message[i..]
        .split_whitespace()
        .next()?
        .trim_end_matches([':', ',']);
    let loc = loc.replace('\\', "/");
    // Only a source path of this repository's shape: `crates/...rs:L:C`
    // (or `src/...`); anything else could be a path on the VM.
    let ok = loc.contains(".rs:")
        && (loc.starts_with("crates/") || loc.starts_with("src/") || loc.starts_with("xtask/"))
        && loc
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/_-.:".contains(c));
    ok.then_some(loc)
}

/// A name for a Windows crash status.
pub fn exit_name(code: i32) -> &'static str {
    match code as u32 {
        0xC000_0005 => "access violation",
        0xC000_00FD => "stack overflow",
        0xC000_0409 => "fast fail (abort)",
        0xC000_001D => "illegal instruction",
        0xC000_0374 => "heap corruption",
        0xC000_0094 => "integer divide by zero",
        0x8000_0003 => "breakpoint",
        0xC000_013A => "control-c",
        _ => "abnormal exit",
    }
}

/// The longest stretch of play without progress, as (length, start),
/// counted from 0 to `end`: progress is any objective change, mission
/// state change, contract accepted or completed, or marker first shown.
/// After the first completion nothing counts.
pub fn longest_gap(s: &Summary, end: f64) -> (f64, f64) {
    let done = s.completed.first().map(|e| e.t);
    let end = done.map_or(end, |d| d.min(end));
    let mut times: Vec<f64> = s
        .objectives
        .iter()
        .chain(&s.contract_states)
        .chain(&s.accepted)
        .chain(&s.completed)
        .map(|e| e.t)
        .chain(s.markers.iter().map(|m| m.first))
        .filter(|t| t.is_finite() && *t <= end)
        .collect();
    times.push(0.0);
    times.push(end);
    times.sort_by(f64::total_cmp);
    times
        .windows(2)
        .map(|w| (w[1] - w[0], w[0]))
        .fold((0.0, 0.0), |a, b| if b.0 > a.0 { b } else { a })
}

/// Runs every detector over one run's stderr log and exit.
pub fn analyse(log: &str, exit: &Exit, th: &Thresholds) -> Analysis {
    let summary = summarize(
        log,
        Config {
            mission_seconds: Some(th.mission_seconds),
            ..Config::default()
        },
    );
    let mut failures = Vec::new();
    let killed = exit.killed;

    // Line scans the summary doesn't keep: heights, falls, pace.
    let mut clock = 0.0;
    let mut fell: Vec<(f64, [f64; 3])> = Vec::new();
    let mut lost: Option<f64> = None;
    let mut lowest: Option<(f64, f64)> = None;
    let mut paces: Vec<Pace> = Vec::new();
    // Cancels (time, whether she was racing just before), Retries and
    // Continues taken on their fanfares (times), the last racing line.
    let mut cancels: Vec<(f64, bool)> = Vec::new();
    let mut retries: Vec<f64> = Vec::new();
    let mut continues = 0usize;
    let mut last_racing: Option<f64> = None;
    for line in log.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("autoplay: contract cancelled (") {
            let t = between(rest, " after ", " s of play")
                .and_then(|n| n.parse::<f64>().ok())
                .unwrap_or(clock);
            cancels.push((t, last_racing.is_some_and(|r| t - r <= 5.0)));
            continue;
        }
        if line.starts_with("autoplay: she takes Retry") {
            retries.push(clock);
            continue;
        }
        if line.starts_with("autoplay: she takes Continue") {
            continues += 1;
            continue;
        }
        if let Some((t, body)) = autoplay_line(line) {
            clock = t;
            if body.starts_with("racing") {
                last_racing = Some(t);
            }
            let at = body
                .strip_prefix("at [")
                .or_else(|| body.strip_prefix("fell through at ["));
            if let Some(p) = at.and_then(triple) {
                if lowest.is_none_or(|(_, y)| p[1] < y) {
                    lowest = Some((t, p[1]));
                }
                if body.starts_with("fell through") {
                    fell.push((t, p));
                }
            }
        } else if line.starts_with("autoplay: her position is not a number") {
            lost.get_or_insert(clock);
        } else if let Some(p) = pace(line, clock) {
            paces.push(p);
        }
    }

    // Panics, crashes and errors.
    let panicked = summary.end.verdict == "panic" || exit.code == Some(101);
    if panicked && killed.is_none() {
        let loc = summary
            .end
            .message
            .as_deref()
            .and_then(panic_location)
            .unwrap_or_else(|| "unknown location".into());
        failures.push(Failure {
            kind: Kind::Panic,
            at: Some(summary.last_time),
            detail: format!("panicked at {loc} after {:.0} s of play", summary.last_time),
        });
    } else if killed.is_none()
        && let Some(code) = exit.code.filter(|c| ![0, 1, 101].contains(c))
    {
        failures.push(Failure {
            kind: Kind::Crash,
            at: Some(summary.last_time),
            detail: format!(
                "exit code {:#010x} ({}) after {:.0} s of play",
                code as u32,
                exit_name(code),
                summary.last_time
            ),
        });
    } else if killed.is_none() && (summary.end.verdict == "error" || exit.code == Some(1)) {
        failures.push(Failure {
            kind: Kind::Error,
            at: Some(summary.last_time),
            detail: format!(
                "m2-alpha stopped on an error after {:.0} s of play",
                summary.last_time
            ),
        });
    }

    match killed {
        Some(Killed::Hang) => failures.push(Failure {
            kind: Kind::Hang,
            at: Some(summary.last_time),
            detail: format!(
                "no log output for {:.0} s of wall time, after {:.0} s of play",
                th.hang_seconds, summary.last_time
            ),
        }),
        Some(Killed::Wall) => failures.push(Failure {
            kind: Kind::Timeout,
            at: Some(summary.last_time),
            detail: format!(
                "stopped at the wall limit ({:.0} s) after {:.0} s of play",
                exit.wall_seconds, summary.last_time
            ),
        }),
        _ => {}
    }

    // Falling through the world.
    if let Some(t) = lost {
        failures.push(Failure {
            kind: Kind::FellThrough,
            at: Some(t),
            detail: format!("her position was not a number at {t:.0} s of play"),
        });
    } else if let Some(&(t, p)) = fell.first() {
        failures.push(Failure {
            kind: Kind::FellThrough,
            at: Some(t),
            detail: format!(
                "fell through {} time(s), first at {t:.0} s of play at [{:.0}, {:.0}, {:.0}]",
                fell.len(),
                p[0],
                p[1],
                p[2]
            ),
        });
    } else if let Some((t, y)) = lowest.filter(|&(_, y)| y < th.floor_y) {
        failures.push(Failure {
            kind: Kind::FellThrough,
            at: Some(t),
            detail: format!(
                "logged height {y:.0} m at {t:.0} s of play, under the floor {:.0} m",
                th.floor_y
            ),
        });
    }

    let completed = !summary.completed.is_empty();

    // The contract cancelled, and what came of the Retries taken on it:
    // a Retry that works starts the contract again (autoplay goes back on
    // it); one whose next contract state is none put it back on offer.
    if !completed && let Some(&(t, racing)) = cancels.first() {
        let times: Vec<String> = cancels.iter().map(|(t, _)| format!("{t:.1}")).collect();
        failures.push(Failure {
            kind: Kind::ContractCancelled,
            at: Some(t),
            detail: format!(
                "the scripts cancelled the contract {} time(s), at {} s of play{}; answered with {} Retry and {} Continue",
                cancels.len(),
                times.join(", "),
                if racing { ", the first while racing" } else { "" },
                retries.len(),
                continues
            ),
        });
    }
    let failed_retry = retries.iter().find_map(|&r| {
        summary
            .contract_states
            .iter()
            .find(|e| e.t >= r)
            .filter(|e| e.what == "none")
            .map(|e| (r, e.t))
    });
    if let Some((r, none)) = failed_retry {
        failures.push(Failure {
            kind: Kind::RetryFailed,
            at: Some(r),
            detail: format!(
                "took Retry at {r:.1} s of play, but by {none:.1} s she was on no contract: it was put back on offer, not restarted"
            ),
        });
    }

    // Stuck: nothing moved on for too long.
    let gap = longest_gap(&summary, summary.last_time);
    if gap.0 >= th.stuck_seconds {
        failures.push(Failure {
            kind: Kind::Stuck,
            at: Some(gap.1),
            detail: format!(
                "no objective or mission-state change for {:.0} s of play, from {:.0} s",
                gap.0, gap.1
            ),
        });
    }

    // FPS cliffs: `slow_lines` slow pace lines in a row.
    let slow = |p: &Pace| {
        if p.tick {
            p.value > th.tick_ms_ceiling
        } else {
            p.value < th.fps_floor
        }
    };
    let mut run = 0usize;
    let mut cliff: Option<(f64, f64, bool)> = None;
    let mut start = 0.0;
    for p in &paces {
        if slow(p) {
            if run == 0 {
                start = p.t;
            }
            run += 1;
            if run >= th.slow_lines.max(1) && cliff.is_none() {
                cliff = Some((start, p.value, p.tick));
            }
        } else {
            run = 0;
        }
    }
    let worst_tick_ms = paces
        .iter()
        .filter(|p| p.tick)
        .map(|p| p.value)
        .reduce(f64::max);
    let worst_fps = paces
        .iter()
        .filter(|p| !p.tick)
        .map(|p| p.value)
        .reduce(f64::min);
    if let Some((t, _, tick)) = cliff {
        let detail = if tick {
            format!(
                "over {:.0} ms a tick for {}+ minutes from {t:.0} s of play (worst {:.0} ms)",
                th.tick_ms_ceiling,
                th.slow_lines,
                worst_tick_ms.unwrap_or(0.0)
            )
        } else {
            format!(
                "under {:.0} fps for {}+ pace lines from {t:.0} s of play (worst {:.0} fps)",
                th.fps_floor,
                th.slow_lines,
                worst_fps.unwrap_or(0.0)
            )
        };
        failures.push(Failure {
            kind: Kind::FpsCliff,
            at: Some(t),
            detail,
        });
    }

    // Not completed, when the run ended by itself or on a limit.
    if !completed && killed != Some(Killed::Deadline) {
        let played = summary.end.played.unwrap_or(summary.last_time);
        let detail = if summary.end.time_limit {
            format!(
                "played the whole {:.0} s limit without completing",
                th.mission_seconds
            )
        } else {
            format!("ended after {played:.0} s of play without completing")
        };
        failures.push(Failure {
            kind: Kind::NotCompleted,
            at: None,
            detail,
        });
    }

    Analysis {
        completed,
        failures,
        fell_through_lines: fell.len(),
        lowest_y: lowest.map(|(_, y)| y),
        worst_tick_ms,
        worst_fps,
        longest_gap: gap,
        summary,
    }
}

/// The kinds worth an issue each: a cause that explains a contract not
/// completing (a panic, a hang, being stuck...) stands for it, so
/// `not-completed` and `timeout` are dropped then; `timeout` stands for
/// `not-completed` too. A stretch stuck that began at or after the
/// contract was cancelled is that cancel's doing, so `stuck` is dropped
/// then. FPS cliffs always stand on their own.
pub fn issue_kinds(failures: &[Failure]) -> Vec<Kind> {
    let first = |k: Kind| failures.iter().find(|f| f.kind == k).and_then(|f| f.at);
    let cancelled = first(Kind::ContractCancelled).or(first(Kind::RetryFailed));
    let stuck_after_cancel = matches!(
        (cancelled, first(Kind::Stuck)),
        (Some(c), Some(s)) if s >= c
    );
    let mut kinds: Vec<Kind> = failures.iter().map(|f| f.kind).collect();
    kinds.sort();
    kinds.dedup();
    let explained = kinds.iter().any(|k| k.explains_incomplete());
    let timed_out = kinds.contains(&Kind::Timeout);
    kinds.retain(|k| match k {
        Kind::NotCompleted => !explained && !timed_out,
        Kind::Timeout => !explained,
        Kind::Stuck => !stuck_after_cancel,
        _ => true,
    });
    kinds
}

/// The live check while a run plays: whether it is stuck already, from
/// the log so far.
pub fn stuck_now(log: &str, th: &Thresholds) -> Option<f64> {
    let s = summarize(log, Config::default());
    if !s.completed.is_empty() {
        return None;
    }
    let gap = longest_gap(&s, s.last_time);
    (gap.0 >= th.stuck_seconds).then_some(gap.0)
}
