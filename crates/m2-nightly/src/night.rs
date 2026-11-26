//! A night's results (`night.json`) and the short report (`report.md`).

use serde::{Deserialize, Serialize};

use crate::detect::{Exit, Failure, Kind};

/// How one contract's run came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Pass,
    Fail,
    /// Not started (the GPU stayed busy, or the window was too short).
    Skipped,
    /// Stopped by the window's end: neither pass nor fail.
    Aborted,
}

/// One contract's run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunResult {
    pub contract: String,
    pub outcome: Outcome,
    pub failures: Vec<Failure>,
    /// The kinds an issue is filed for ([`crate::detect::issue_kinds`]).
    pub issue_kinds: Vec<Kind>,
    pub exit: Exit,
    pub completed: bool,
    pub played_seconds: Option<f64>,
    /// Autoplay's hops ahead (test shortcuts past where she was blocked).
    pub hops: Option<usize>,
    pub lua_errors: usize,
    pub fell_through_lines: usize,
    pub lowest_y: Option<f64>,
    pub worst_tick_ms: Option<f64>,
    pub worst_fps: Option<f64>,
    pub longest_gap_seconds: f64,
    /// Why it was skipped, when it was.
    pub note: Option<String>,
    /// The run's folder, relative to the night's.
    pub folder: String,
}

impl RunResult {
    pub fn skipped(contract: &str, why: &str) -> Self {
        Self {
            contract: contract.into(),
            outcome: Outcome::Skipped,
            failures: Vec::new(),
            issue_kinds: Vec::new(),
            exit: Exit::default(),
            completed: false,
            played_seconds: None,
            hops: None,
            lua_errors: 0,
            fell_through_lines: 0,
            lowest_y: None,
            worst_tick_ms: None,
            worst_fps: None,
            longest_gap_seconds: 0.0,
            note: Some(why.into()),
            folder: contract.into(),
        }
    }
}

/// The whole night.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Night {
    /// The night's folder name (`20261126-0100Z`).
    pub run_id: String,
    /// The commit the m2-alpha exe was built from.
    pub head: String,
    pub started_unix: u64,
    pub finished_unix: u64,
    pub deadline_unix: Option<u64>,
    /// Contracts found in the install.
    pub contracts_listed: usize,
    pub results: Vec<RunResult>,
}

/// `YYYYMMDD-HHMMZ` for a unix time (UTC).
pub fn utc_stamp(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    // Days to civil date (proleptic Gregorian), H. Hinnant's algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}Z",
        secs / 3600,
        (secs % 3600) / 60
    )
}

/// The short report, kept under `limit` bytes: counts, then one line per
/// failed or aborted contract (most issue kinds first), then the passes
/// as a list of ids; whatever doesn't fit is counted.
pub fn report(n: &Night, limit: usize) -> String {
    let count = |o: Outcome| n.results.iter().filter(|r| r.outcome == o).count();
    let mut out = format!(
        "# Nightly autoplay {}\nhead {} | {} contracts listed, {} run | pass {} | fail {} | aborted {} | skipped {} | {:.0} min\n\n",
        n.run_id,
        short(&n.head),
        n.contracts_listed,
        n.results
            .iter()
            .filter(|r| r.outcome != Outcome::Skipped)
            .count(),
        count(Outcome::Pass),
        count(Outcome::Fail),
        count(Outcome::Aborted),
        count(Outcome::Skipped),
        n.finished_unix.saturating_sub(n.started_unix) as f64 / 60.0,
    );
    let mut lines: Vec<String> = Vec::new();
    let mut bad: Vec<&RunResult> = n
        .results
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Fail | Outcome::Aborted))
        .collect();
    bad.sort_by_key(|r| (std::cmp::Reverse(r.issue_kinds.len()), r.contract.clone()));
    for r in bad {
        let what = if r.outcome == Outcome::Aborted {
            "ABORTED at the window's end".to_string()
        } else {
            "FAIL ".to_string()
                + r.issue_kinds
                    .iter()
                    .map(|k| {
                        let at = r
                            .failures
                            .iter()
                            .find(|f| f.kind == *k)
                            .and_then(|f| f.at)
                            .map(|t| format!("@{t:.0}s"))
                            .unwrap_or_default();
                        format!("{}{at}", k.name())
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
                    .as_str()
        };
        lines.push(format!("- {} {what}\n", r.contract));
    }
    let passes: Vec<&str> = n
        .results
        .iter()
        .filter(|r| r.outcome == Outcome::Pass)
        .map(|r| r.contract.as_str())
        .collect();
    if !passes.is_empty() {
        lines.push(format!("\npass: {}\n", passes.join(" ")));
    }
    let skipped: Vec<&str> = n
        .results
        .iter()
        .filter(|r| r.outcome == Outcome::Skipped)
        .map(|r| r.contract.as_str())
        .collect();
    if !skipped.is_empty() {
        let why = n
            .results
            .iter()
            .find_map(|r| r.note.as_deref())
            .unwrap_or("");
        lines.push(format!("not run ({why}): {}\n", skipped.join(" ")));
    }
    let tail = "\nfull results: night.json; per contract: <id>/summary.txt\n";
    let mut dropped = 0;
    for l in &lines {
        if out.len() + l.len() + tail.len() + 40 > limit {
            dropped += 1;
            continue;
        }
        out.push_str(l);
    }
    if dropped > 0 {
        out.push_str(&format!("... {dropped} more line(s) in night.json\n"));
    }
    out.push_str(tail);
    out
}

/// The first 10 characters of a commit id.
pub fn short(head: &str) -> &str {
    &head[..head.len().min(10)]
}
