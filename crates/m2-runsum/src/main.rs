//! `m2-runsum`: summarizes m2-alpha autoplay runs from their stderr logs.
//!
//! ```text
//! m2-runsum [options] <run.err>...
//!   --json <path|->        also write the JSON summary (one object per log;
//!                          several logs make a JSON array); `-` is stdout
//!   --json-only            print only the JSON to stdout
//!   --stall-seconds <s>    no-progress threshold (default 60)
//!   --still-radius <m>     drift still counted as standing still (default 2)
//!   --osc-radius <m>       same-spot radius for A-B-A loops (default 5)
//!   --osc-min-returns <n>  returns before a loop is reported (default 2)
//!   --mission-seconds <s>  the run's --mission-seconds, to flag the time limit
//! ```
//!
//! The stall and loop thresholds are tooling heuristics over her logged
//! positions (autoplay logs one every few seconds), not game rules.

use std::process::ExitCode;

use m2_runsum::{Config, summarize, to_json, to_text};

const USAGE: &str = "usage: m2-runsum [--json <path|->] [--json-only] [--stall-seconds S] \
[--still-radius M] [--osc-radius M] [--osc-min-returns N] [--mission-seconds S] <run.err>...";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("m2-runsum: {e}");
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut config = Config::default();
    let mut json: Option<String> = None;
    let mut json_only = false;
    let mut logs = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut num = |name: &str| -> Result<f64, String> {
            args.next()
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite() && *v >= 0.0)
                .ok_or_else(|| format!("{name} expects a non-negative number"))
        };
        match a.as_str() {
            "--json" => json = Some(args.next().ok_or("--json expects a path or -")?),
            "--json-only" => json_only = true,
            "--stall-seconds" => config.stall_seconds = num(&a)?,
            "--still-radius" => config.still_radius = num(&a)?,
            "--osc-radius" => config.osc_radius = num(&a)?,
            "--osc-min-returns" => config.osc_min_returns = num(&a)?.max(1.0) as usize,
            "--mission-seconds" => config.mission_seconds = Some(num(&a)?),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            s if s.starts_with("--") => return Err(format!("unknown option {s}")),
            _ => logs.push(a),
        }
    }
    if logs.is_empty() {
        return Err("no log given".into());
    }
    let mut objects = Vec::new();
    for path in &logs {
        let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        let text = String::from_utf8_lossy(&bytes);
        let text = text.trim_start_matches('\u{feff}');
        let summary = summarize(text, config);
        if !json_only {
            print!("{}", to_text(&summary, path));
        }
        objects.push(to_json(&summary, path));
    }
    let out = if objects.len() == 1 {
        objects.remove(0)
    } else {
        format!("[\n{}]\n", objects.join(",\n"))
    };
    match (json.as_deref(), json_only) {
        (Some("-"), _) | (None, true) => print!("{out}"),
        (Some(path), _) => std::fs::write(path, out).map_err(|e| format!("{path}: {e}"))?,
        (None, false) => {}
    }
    Ok(())
}
