//! Yielding the GPU: a lock file the runner holds all night, and the dev
//! VM's existing slot rule (no `m2-alpha*` process running: the Xenia
//! launcher names its exe `m2-alpha-xenia.exe` so this rule sees it too).
//!
//! The lock file holds the owner's pid, start time and purpose as JSON.
//! A lock whose pid is gone, or older than its stale age, is taken over.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Image-name prefixes (lower case) that hold the GPU slot.
pub const BUSY_PREFIXES: [&str; 2] = ["m2-alpha", "xenia"];

/// What the lock file says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    pub pid: u32,
    pub started_unix: u64,
    pub purpose: String,
}

/// A held lock; dropping it removes the file.
#[derive(Debug)]
pub struct Lock {
    path: PathBuf,
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A running process: (image name, pid).
pub type Proc = (String, u32);

/// Parses `tasklist /FO CSV /NH` output: `"name","pid",...` per line.
pub fn parse_tasklist(csv: &str) -> Vec<Proc> {
    csv.lines()
        .filter_map(|l| {
            let mut f = l.trim().split("\",\"");
            let name = f.next()?.trim_start_matches('"').to_string();
            let pid = f.next()?.trim_matches('"').parse().ok()?;
            Some((name, pid))
        })
        .collect()
}

/// The processes running now (empty when they can't be listed).
pub fn processes() -> Vec<Proc> {
    if cfg!(windows) {
        Command::new("tasklist")
            .args(["/FO", "CSV", "/NH"])
            .output()
            .map(|o| parse_tasklist(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or_default()
    } else {
        Command::new("ps")
            .args(["-eo", "comm=,pid="])
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter_map(|l| {
                        let (n, p) = l.trim().rsplit_once(char::is_whitespace)?;
                        Some((n.trim().to_string(), p.parse().ok()?))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The processes among `procs` that hold the GPU slot by name.
pub fn busy<'a>(procs: &'a [Proc], prefixes: &[String]) -> Vec<&'a Proc> {
    procs
        .iter()
        .filter(|(n, _)| {
            let n = n.to_ascii_lowercase();
            prefixes.iter().any(|p| n.starts_with(p.as_str()))
        })
        .collect()
}

/// Whether a lock held by `owner` may be taken over: its pid is not
/// running, or it is older than `stale_seconds`.
pub fn is_stale(owner: &Owner, procs: &[Proc], now: u64, stale_seconds: u64) -> bool {
    let alive = procs.iter().any(|(_, p)| *p == owner.pid);
    !alive || now.saturating_sub(owner.started_unix) > stale_seconds
}

/// Takes the lock at `path`, or says who holds it. A stale lock (see
/// [`is_stale`]) is removed and taken.
pub fn try_lock(
    path: &Path,
    purpose: &str,
    stale_seconds: u64,
) -> std::io::Result<Result<Lock, Owner>> {
    for _ in 0..2 {
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(mut f) => {
                let owner = Owner {
                    pid: std::process::id(),
                    started_unix: now_unix(),
                    purpose: purpose.into(),
                };
                f.write_all(serde_json::to_string(&owner).unwrap_or_default().as_bytes())?;
                return Ok(Ok(Lock {
                    path: path.to_path_buf(),
                }));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let text = std::fs::read_to_string(path).unwrap_or_default();
                let owner: Owner = serde_json::from_str(&text).unwrap_or(Owner {
                    pid: 0,
                    started_unix: 0,
                    purpose: "unreadable lock file".into(),
                });
                if is_stale(&owner, &processes(), now_unix(), stale_seconds) {
                    std::fs::remove_file(path)?;
                    continue;
                }
                return Ok(Err(owner));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(Err(Owner {
        pid: 0,
        started_unix: 0,
        purpose: "lock contended".into(),
    }))
}
