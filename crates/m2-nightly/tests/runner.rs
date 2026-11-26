//! The runner end to end against the stand-in m2-alpha
//! (`m2-nightly-fake-alpha`): no game, no GPU, no game data.

use std::path::PathBuf;

use m2_nightly::detect::{Killed, Kind, Thresholds};
use m2_nightly::night::Outcome;
use m2_nightly::run::{NightConfig, run_night};

fn config(name: &str, contracts: &[&str]) -> NightConfig {
    let root = std::env::temp_dir().join(format!("m2-nightly-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    NightConfig {
        exe: PathBuf::from(env!("CARGO_BIN_EXE_m2-nightly-fake-alpha")),
        exe_prefix_args: Vec::new(),
        install: PathBuf::from("no-install"),
        out: root.join("night"),
        run_id: "night".into(),
        head: "feedface".into(),
        contracts: contracts.iter().map(|s| s.to_string()).collect(),
        contracts_listed: contracts.len(),
        thresholds: Thresholds {
            hang_seconds: 3.0,
            ..Thresholds::default()
        },
        wall_seconds: 60,
        deadline_unix: None,
        min_start_seconds: 0,
        lock: root.join("gpu.lock"),
        busy: Vec::new(),
        free_checks: 1,
        gpu_poll_seconds: 1,
        stale_seconds: 3600,
        poll_seconds: 1,
        stuck_check_seconds: 1,
        keep_pictures: false,
        state: Some(root.join("last-played.json")),
    }
}

#[test]
fn a_night_of_fakes_is_judged_and_written() {
    let cfg = config(
        "mix",
        &[
            "FakePass",
            "FakePanic",
            "FakeCrash",
            "FakeHang",
            "FakeStuck",
        ],
    );
    let n = run_night(&cfg).expect("night");
    let by = |id: &str| n.results.iter().find(|r| r.contract == id).unwrap();
    assert_eq!(by("FakePass").outcome, Outcome::Pass);
    assert_eq!(by("FakePanic").issue_kinds, vec![Kind::Panic]);
    assert_eq!(by("FakeCrash").issue_kinds, vec![Kind::Crash]);
    assert_eq!(by("FakeHang").exit.killed, Some(Killed::Hang));
    assert_eq!(by("FakeHang").issue_kinds, vec![Kind::Hang]);
    assert_eq!(by("FakeStuck").exit.killed, Some(Killed::Stuck));
    assert_eq!(by("FakeStuck").issue_kinds, vec![Kind::Stuck]);
    // Files: per run and for the night; the lock is released.
    assert!(cfg.out.join("FakeStuck").join("result.json").exists());
    assert!(cfg.out.join("FakeStuck").join("summary.txt").exists());
    assert!(cfg.out.join("FakeStuck").join("FakeStuck.err").exists());
    let report = std::fs::read_to_string(cfg.out.join("report.md")).unwrap();
    assert!(report.len() < 3000);
    assert!(report.contains("pass 1 | fail 4"), "{report}");
    assert!(!cfg.lock.exists());
    let state = std::fs::read_to_string(cfg.state.as_ref().unwrap()).unwrap();
    assert!(state.contains("FakeHang"));
    let _ = std::fs::remove_dir_all(cfg.out.parent().unwrap());
}

#[test]
fn a_held_lock_skips_the_night_at_its_deadline() {
    let mut cfg = config("held", &["FakePass"]);
    std::fs::create_dir_all(cfg.lock.parent().unwrap()).unwrap();
    // Held by this (running) process.
    std::fs::write(
        &cfg.lock,
        format!(
            r#"{{"pid":{},"started_unix":{},"purpose":"another run"}}"#,
            std::process::id(),
            m2_nightly::gpu::now_unix()
        ),
    )
    .unwrap();
    cfg.deadline_unix = Some(m2_nightly::gpu::now_unix() + 2);
    cfg.min_start_seconds = 1;
    let n = run_night(&cfg).expect("night");
    assert_eq!(n.results[0].outcome, Outcome::Skipped);
    assert!(cfg.lock.exists(), "someone else's lock is left alone");
    let _ = std::fs::remove_dir_all(cfg.out.parent().unwrap());
}

#[test]
fn the_deadline_aborts_the_run_playing() {
    let mut cfg = config("deadline", &["FakeHang", "FakePass"]);
    cfg.thresholds.hang_seconds = 600.0;
    cfg.deadline_unix = Some(m2_nightly::gpu::now_unix() + 3);
    let n = run_night(&cfg).expect("night");
    assert_eq!(n.results[0].outcome, Outcome::Aborted);
    assert_eq!(n.results[0].exit.killed, Some(Killed::Deadline));
    assert_eq!(n.results[1].outcome, Outcome::Skipped);
    assert!(!cfg.lock.exists());
    let _ = std::fs::remove_dir_all(cfg.out.parent().unwrap());
}
