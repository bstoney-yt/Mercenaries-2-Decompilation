//! A check's lines reach a piped standard error (a file, the merge
//! queue's log) before the process ends.
//!
//! With standard error not a console, the game pipes it through a log
//! thread (`errlog`); a check that ended with `std::process::exit` lost
//! what was still in the pipe: `--ragdoll-check` and `--binocular-check`
//! printed nothing at all. `--log-check` and `--survival-check` need no
//! game data, so they run here as such a check ends, through
//! `errlog::exit`.

use std::process::{Command, Stdio};

#[test]
fn the_survival_checks_lines_reach_a_piped_stderr() {
    let out = Command::new(env!("CARGO_BIN_EXE_m2-alpha"))
        .arg("--survival-check")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Its log line on standard error, through the log thread's pipe.
    assert!(
        stderr
            .lines()
            .any(|l| l == "combat: survival mode on (the clamp caught a hit)"),
        "stderr:
{stderr}"
    );
    // Its five lines and verdict on standard output.
    let lines: Vec<&str> = stdout
        .lines()
        .filter(|l| l.starts_with("survival-check: "))
        .collect();
    assert_eq!(
        lines.len(),
        6,
        "stdout:
{stdout}"
    );
    assert!(
        lines[..5].iter().all(|l| l.ends_with(": PASS")),
        "stdout:
{stdout}"
    );
    assert_eq!(
        lines[5], "survival-check: PASS",
        "stdout:
{stdout}"
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr:
{stderr}"
    );
}

/// The regression itself: a few lines written and the process ended at
/// once, as `--ragdoll-check` does. Ending with `std::process::exit`
/// lost them all (6 runs in 6), the log thread not having read yet.
#[test]
fn a_check_ending_at_once_loses_none_of_its_log() {
    let out = Command::new(env!("CARGO_BIN_EXE_m2-alpha"))
        .arg("--log-check")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> = stderr
        .lines()
        .filter(|l| l.starts_with("log-check: "))
        .collect();
    assert_eq!(
        lines,
        [
            "log-check: line 0",
            "log-check: line 1",
            "log-check: line 2",
            "log-check: line 3",
            "log-check: line 4",
            "log-check: RESULT PASS",
        ],
        "stderr:
{stderr}"
    );
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn help_reaches_a_piped_stdout() {
    let out = Command::new(env!("CARGO_BIN_EXE_m2-alpha"))
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--survival-check"),
        "stdout:
{stdout}"
    );
    assert_eq!(out.status.code(), Some(0));
}
