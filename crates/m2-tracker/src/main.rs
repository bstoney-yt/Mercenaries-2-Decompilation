//! `m2-tracker`: checks and updates `docs/contracts-status.toml`.
//!
//! ```text
//! m2-tracker check [<status.toml>]
//! m2-tracker update --night <night.json> [--status <status.toml>] [--dry-run]
//! m2-tracker summary [<status.toml>]
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use m2_tracker::{Night, Status, Tracker, update_from_night};

const DEFAULT: &str = "docs/contracts-status.toml";

fn main() -> ExitCode {
    match run() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("m2-tracker: {e}");
            ExitCode::from(2)
        }
    }
}

fn read(path: &PathBuf) -> Result<Tracker, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Tracker::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn run() -> Result<ExitCode, String> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_default();
    match cmd.as_str() {
        "check" | "summary" => {
            let path = PathBuf::from(args.next().unwrap_or_else(|| DEFAULT.into()));
            let t = read(&path)?;
            let counts = t.counts();
            let line: Vec<String> = Status::ALL
                .iter()
                .map(|s| format!("{} {}", s.label(), counts[s]))
                .collect();
            println!(
                "{}: ok, {} contracts (updated {}): {}",
                path.display(),
                t.contracts.len(),
                t.updated,
                line.join(", ")
            );
            Ok(ExitCode::SUCCESS)
        }
        "update" => {
            let (mut night, mut status, mut dry) = (None, PathBuf::from(DEFAULT), false);
            while let Some(a) = args.next() {
                match a.as_str() {
                    "--night" => night = args.next().map(PathBuf::from),
                    "--status" => {
                        status = args
                            .next()
                            .map(PathBuf::from)
                            .ok_or("--status expects a path")?
                    }
                    "--dry-run" => dry = true,
                    _ => return Err(format!("unknown option {a}")),
                }
            }
            let night = night.ok_or("--night <night.json> is needed")?;
            let text =
                std::fs::read_to_string(&night).map_err(|e| format!("{}: {e}", night.display()))?;
            let n: Night =
                serde_json::from_str(&text).map_err(|e| format!("{}: {e}", night.display()))?;
            let mut t = read(&status)?;
            let changed = update_from_night(&mut t, &n);
            let problems = m2_tracker::validate(&t);
            if !problems.is_empty() {
                return Err(problems.join("; "));
            }
            println!(
                "{} changed from night {}: {}",
                changed.len(),
                n.run_id,
                changed.join(" ")
            );
            if !dry && !changed.is_empty() {
                std::fs::write(&status, t.to_toml()).map_err(|e| e.to_string())?;
                println!("wrote {}", status.display());
            }
            Ok(ExitCode::SUCCESS)
        }
        _ => {
            println!(
                "m2-tracker check [<status.toml>]\nm2-tracker update --night <night.json> [--status <status.toml>] [--dry-run]\nm2-tracker summary [<status.toml>]"
            );
            Ok(if cmd.is_empty() || cmd == "--help" || cmd == "-h" {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            })
        }
    }
}
