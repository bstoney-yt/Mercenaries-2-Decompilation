//! Detector, report and issue-dedupe tests over synthetic logs (no game
//! data).

use std::collections::{BTreeMap, HashSet};

use crate::contracts::{FIRST, alpha_args, ids_in, schedule};
use crate::detect::{
    Exit, Failure, Killed, Kind, Thresholds, analyse, issue_kinds, panic_location, stuck_now,
};
use crate::gpu::{Owner, busy, is_stale, parse_tasklist, try_lock};
use crate::issues::{
    Action, Existing, INDEX_MARKER, body, breaks, index_body, key, marker, open_after, plan,
};
use crate::night::{Night, Outcome, RunResult, report, utc_stamp};

fn th() -> Thresholds {
    Thresholds::default()
}

fn clean() -> Exit {
    Exit {
        code: Some(0),
        killed: None,
        wall_seconds: 100.0,
    }
}

fn kinds(fs: &[Failure]) -> Vec<Kind> {
    fs.iter().map(|f| f.kind).collect()
}

/// A run that plays through: objectives change every minute, it
/// completes, healthy pace.
fn passing_log() -> String {
    let mut s = String::new();
    for i in 0..10 {
        let t = i as f64 * 60.0 + 1.0;
        s += &format!("autoplay {t:.1} s: objectives [\"Step {i}\"]\n");
        s += &format!(
            "autoplay {:.1} s: at [{}.0, -30.0, 4.0], on foot\n",
            t + 5.0,
            i * 20
        );
        s += &format!(
            "pace: autoplay {:.0} s of play: 1.60 s of play a second, 10.5 ms a tick, 30 people, 6 vehicles; ms a tick: x 1\n",
            t + 59.0
        );
    }
    s += "autoplay: contract 1 completed after 600.0 s of play\n";
    s += "autoplay: 601.0 s played, 0 hops;\n";
    s
}

#[test]
fn a_clean_completed_run_passes() {
    let a = analyse(&passing_log(), &clean(), &th());
    assert!(a.completed);
    assert_eq!(a.failures, vec![]);
    assert_eq!(a.lowest_y, Some(-30.0));
    assert_eq!(a.worst_tick_ms, Some(10.5));
}

#[test]
fn a_panic_is_found_with_our_location_only() {
    let log = "autoplay 5.0 s: objectives [\"A\"]\n\
thread 'main' panicked at crates\\m2-alpha\\src\\mission.rs:120:9:\n\
index out of bounds\n";
    let a = analyse(
        log,
        &Exit {
            code: Some(101),
            ..clean()
        },
        &th(),
    );
    assert_eq!(kinds(&a.failures), vec![Kind::Panic, Kind::NotCompleted]);
    assert!(
        a.failures[0]
            .detail
            .contains("crates/m2-alpha/src/mission.rs:120:9")
    );
    assert_eq!(issue_kinds(&a.failures), vec![Kind::Panic]);
}

#[test]
fn panic_locations_outside_the_repo_are_dropped() {
    assert_eq!(
        panic_location("thread 'main' (42) panicked at crates/m2-game/src/a.rs:1:2:"),
        Some("crates/m2-game/src/a.rs:1:2".into())
    );
    assert_eq!(
        panic_location("thread 'main' panicked at C:\\Users\\x\\a.rs:1:2:"),
        None
    );
    assert_eq!(panic_location("no panic here"), None);
}

#[test]
fn crash_exit_codes_are_named() {
    let a = analyse(
        "autoplay 9.0 s: objectives [\"A\"]\n",
        &Exit {
            code: Some(0xC000_0005_u32 as i32),
            ..clean()
        },
        &th(),
    );
    assert_eq!(a.failures[0].kind, Kind::Crash);
    assert!(
        a.failures[0]
            .detail
            .contains("0xc0000005 (access violation)"),
        "{}",
        a.failures[0].detail
    );
    assert_eq!(issue_kinds(&a.failures), vec![Kind::Crash]);
}

#[test]
fn an_error_exit_is_an_error() {
    let a = analyse(
        "Error: no install found\n",
        &Exit {
            code: Some(1),
            ..clean()
        },
        &th(),
    );
    assert_eq!(kinds(&a.failures), vec![Kind::Error, Kind::NotCompleted]);
    assert_eq!(issue_kinds(&a.failures), vec![Kind::Error]);
}

#[test]
fn a_hang_kill_is_a_hang_not_a_crash() {
    let a = analyse(
        "autoplay 5.0 s: objectives [\"A\"]\n",
        &Exit {
            code: None,
            killed: Some(Killed::Hang),
            wall_seconds: 400.0,
        },
        &th(),
    );
    assert_eq!(kinds(&a.failures), vec![Kind::Hang, Kind::NotCompleted]);
    assert_eq!(issue_kinds(&a.failures), vec![Kind::Hang]);
}

#[test]
fn a_wall_kill_is_a_timeout_standing_for_not_completed() {
    let mut log = String::new();
    for i in 0..30 {
        log += &format!("autoplay {}.0 s: objectives [\"S{i}\"]\n", i * 50);
    }
    let a = analyse(
        &log,
        &Exit {
            code: None,
            killed: Some(Killed::Wall),
            wall_seconds: 2700.0,
        },
        &th(),
    );
    assert_eq!(kinds(&a.failures), vec![Kind::Timeout, Kind::NotCompleted]);
    assert_eq!(issue_kinds(&a.failures), vec![Kind::Timeout]);
}

#[test]
fn objectives_unchanged_for_ten_minutes_is_stuck() {
    let mut log = String::from("autoplay 3.0 s: objectives [\"Kill enemies (0/4)\"]\n");
    // The count changes but the objective list stays the same text after
    // 100 s; positions keep coming.
    log += "autoplay 100.0 s: objectives [\"Kill enemies (1/4)\"]\n";
    for t in (110..900).step_by(15) {
        log += &format!("autoplay {t}.0 s: at [1.0, -30.0, 1.0], on foot\n");
    }
    log += "autoplay: 900.0 s played, 0 hops; the contract was not completed\n";
    let a = analyse(&log, &clean(), &th());
    let stuck = a
        .failures
        .iter()
        .find(|f| f.kind == Kind::Stuck)
        .expect("stuck");
    assert_eq!(stuck.at, Some(100.0));
    assert!(stuck.detail.contains("for 790 s"), "{}", stuck.detail);
    assert_eq!(issue_kinds(&a.failures), vec![Kind::Stuck]);
    assert_eq!(stuck_now(&log, &th()), Some(790.0));
    // Not yet stuck at a shorter gap.
    let short: String = log.lines().take(30).map(|l| format!("{l}\n")).collect();
    assert_eq!(stuck_now(&short, &th()), None);
}

#[test]
fn nothing_counts_after_completion() {
    let mut log = passing_log();
    for t in (700..2000).step_by(30) {
        log += &format!("after contract: {t}.0 s, she is at [1.0, -30.0, 1.0]\n");
    }
    let a = analyse(&log, &clean(), &th());
    assert_eq!(a.failures, vec![]);
    assert_eq!(stuck_now(&log, &th()), None);
}

#[test]
fn autoplay_fell_through_lines_are_falls() {
    let mut log = passing_log();
    log = log.replace(
        "autoplay: contract 1",
        "autoplay 200.0 s: fell through at [10.0, -90.6, 20.0]; back up on the ground at [10.0, -30.0, 20.0]\nautoplay: contract 1",
    );
    let a = analyse(&log, &clean(), &th());
    assert_eq!(kinds(&a.failures), vec![Kind::FellThrough]);
    assert_eq!(a.fell_through_lines, 1);
    assert!(
        a.failures[0]
            .detail
            .contains("first at 200 s of play at [10, -91, 20]"),
        "{}",
        a.failures[0].detail
    );
}

#[test]
fn a_height_under_the_floor_is_a_fall() {
    let log = passing_log().replace("at [100.0, -30.0", "at [100.0, -400.0");
    let a = analyse(&log, &clean(), &th());
    assert_eq!(kinds(&a.failures), vec![Kind::FellThrough]);
    assert_eq!(a.lowest_y, Some(-400.0));
    // A deeper floor lets it pass.
    let a = analyse(
        &log,
        &clean(),
        &Thresholds {
            floor_y: -500.0,
            ..th()
        },
    );
    assert_eq!(a.failures, vec![]);
}

#[test]
fn a_lost_position_is_a_fall() {
    let log = "autoplay 50.0 s: objectives [\"A\"]\nautoplay: her position is not a number after 60.0 s of play: stopping\n";
    let a = analyse(log, &clean(), &th());
    assert_eq!(a.failures[0].kind, Kind::FellThrough);
    assert_eq!(issue_kinds(&a.failures), vec![Kind::FellThrough]);
}

#[test]
fn sustained_slow_ticks_are_a_cliff_but_a_spike_is_not() {
    let slow = |log: &str, at: &str| {
        log.replacen(
            &format!("{at} s of play: 1.60 s of play a second, 10.5 ms"),
            &format!("{at} s of play: 0.20 s of play a second, 80.0 ms"),
            1,
        )
    };
    // One slow minute: no cliff.
    let one = slow(&passing_log(), "120");
    assert_eq!(analyse(&one, &clean(), &th()).failures, vec![]);
    // Three in a row: a cliff from the first of them.
    let three = slow(&slow(&slow(&passing_log(), "120"), "180"), "240");
    let a = analyse(&three, &clean(), &th());
    assert_eq!(kinds(&a.failures), vec![Kind::FpsCliff]);
    assert_eq!(a.failures[0].at, Some(120.0));
    assert_eq!(a.worst_tick_ms, Some(80.0));
    // Two, a fast one, then two: no cliff.
    let broken = slow(
        &slow(&slow(&slow(&passing_log(), "120"), "180"), "300"),
        "360",
    );
    assert_eq!(analyse(&broken, &clean(), &th()).failures, vec![]);
}

#[test]
fn windowed_fps_lines_count_too() {
    let mut log = passing_log();
    for _ in 0..3 {
        log += "pace: 9 fps over 5 s, slowest frame 300 ms, 40 over 50 ms, 640x360, 1% low 4 fps, frame ms p50 100.0 p99 290.0\n";
    }
    let a = analyse(&log, &clean(), &th());
    assert_eq!(kinds(&a.failures), vec![Kind::FpsCliff]);
    assert_eq!(a.worst_fps, Some(9.0));
}

#[test]
fn a_cliff_and_a_stuck_run_file_both() {
    let fs = vec![
        Failure {
            kind: Kind::FpsCliff,
            at: None,
            detail: String::new(),
        },
        Failure {
            kind: Kind::Stuck,
            at: None,
            detail: String::new(),
        },
        Failure {
            kind: Kind::NotCompleted,
            at: None,
            detail: String::new(),
        },
    ];
    assert_eq!(issue_kinds(&fs), vec![Kind::Stuck, Kind::FpsCliff]);
}

#[test]
fn a_deadline_kill_is_not_a_failure_to_complete() {
    let a = analyse(
        "autoplay 5.0 s: objectives [\"A\"]\n",
        &Exit {
            code: None,
            killed: Some(Killed::Deadline),
            wall_seconds: 30.0,
        },
        &th(),
    );
    assert_eq!(a.failures, vec![]);
}

#[test]
fn kind_names_round_trip() {
    for k in Kind::ALL {
        assert_eq!(Kind::from_name(k.name()), Some(k));
        let j = serde_json::to_string(&k).unwrap();
        assert_eq!(j, format!("\"{}\"", k.name()));
    }
}

// ---- contracts --------------------------------------------------------------

#[test]
fn contract_ids_are_names_of_scripts() {
    let scripts: HashSet<u32> = ["PirCon001", "VzaCon001", "AllCon050", "AllJob002"]
        .iter()
        .map(|n| m2_assets::name_hash(n))
        .collect();
    assert_eq!(
        ids_in(&scripts, "Con"),
        vec!["VzaCon001", "PirCon001", "AllCon050"]
    );
    assert_eq!(ids_in(&scripts, "Job"), vec!["AllJob002"]);
}

#[test]
fn the_schedule_takes_turns() {
    let ids: Vec<String> = ["VzaCon001", "PmcCon001", "PirCon001", "AllCon001"]
        .map(String::from)
        .to_vec();
    let mut last = BTreeMap::new();
    assert_eq!(schedule(&ids, &last), ids);
    last.insert("VzaCon001".to_string(), 300);
    last.insert("PmcCon001".to_string(), 100);
    assert_eq!(
        schedule(&ids, &last),
        ["PirCon001", "AllCon001", "PmcCon001", "VzaCon001"]
    );
}

#[test]
fn the_first_contract_plays_without_skip_to() {
    let p = std::path::Path::new("out.ppm");
    let i = std::path::Path::new("install");
    assert!(!alpha_args(FIRST, i, p, 1500).contains(&"--skip-to".to_string()));
    let a = alpha_args("PirCon001", i, p, 1500);
    assert_eq!(&a[a.len() - 2..], ["--skip-to", "PirCon001"]);
    assert!(a.windows(2).any(|w| w == ["--mission-seconds", "1500"]));
}

// ---- GPU lock ----------------------------------------------------------------

#[test]
fn tasklist_csv_parses_and_busy_matches_prefixes() {
    let csv = "\"System Idle Process\",\"0\",\"Services\",\"0\",\"8 K\"\r\n\
\"m2-alpha.exe\",\"4242\",\"Console\",\"1\",\"4,100,000 K\"\r\n\
\"m2-alpha-xenia.exe\",\"77\",\"Console\",\"1\",\"9 K\"\r\n\
\"cargo.exe\",\"5\",\"Console\",\"1\",\"9 K\"\r\n";
    let procs = parse_tasklist(csv);
    assert_eq!(procs.len(), 4);
    assert_eq!(procs[1], ("m2-alpha.exe".to_string(), 4242));
    let held = busy(&procs, &["m2-alpha".into(), "xenia".into()]);
    assert_eq!(held.len(), 2);
    assert!(busy(&procs, &[]).is_empty());
}

#[test]
fn stale_locks_are_dead_pids_or_old() {
    let o = Owner {
        pid: 9,
        started_unix: 1000,
        purpose: "x".into(),
    };
    let alive = vec![("m2-nightly.exe".to_string(), 9)];
    assert!(!is_stale(&o, &alive, 2000, 3600));
    assert!(is_stale(&o, &alive, 1000 + 3601, 3600));
    assert!(is_stale(&o, &[], 2000, 3600));
}

#[test]
fn the_lock_is_exclusive_and_released_on_drop() {
    let dir = std::env::temp_dir().join(format!("m2-nightly-lock-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("gpu.lock");
    let _ = std::fs::remove_file(&path);
    let held = try_lock(&path, "test", 3600).unwrap().expect("free");
    // Our own pid is running, so a second take is refused.
    let owner = try_lock(&path, "second", 3600).unwrap().expect_err("held");
    assert_eq!(owner.pid, std::process::id());
    assert_eq!(owner.purpose, "test");
    drop(held);
    assert!(!path.exists());
    // A lock left by a pid that isn't running is taken over.
    std::fs::write(
        &path,
        r#"{"pid":4000000000,"started_unix":1,"purpose":"old"}"#,
    )
    .unwrap();
    let l = try_lock(&path, "new", 3600).unwrap().expect("stale taken");
    drop(l);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- night report ------------------------------------------------------------

fn result(id: &str, outcome: Outcome, kinds: &[Kind]) -> RunResult {
    let mut r = RunResult::skipped(id, "x");
    r.outcome = outcome;
    r.note = None;
    r.issue_kinds = kinds.to_vec();
    r.failures = kinds
        .iter()
        .map(|&k| Failure {
            kind: k,
            at: Some(120.0),
            detail: format!("{} at 120 s", k.name()),
        })
        .collect();
    r
}

fn night(results: Vec<RunResult>) -> Night {
    Night {
        run_id: "20261126-0100Z".into(),
        head: "0123456789abcdef".into(),
        started_unix: 0,
        finished_unix: 3600,
        deadline_unix: None,
        contracts_listed: results.len(),
        results,
    }
}

#[test]
fn utc_stamps() {
    assert_eq!(utc_stamp(0), "19700101-0000Z");
    // 2026-11-26 01:00 UTC.
    assert_eq!(utc_stamp(1_795_654_800), "20261126-0100Z");
}

#[test]
fn the_report_stays_under_3_kb() {
    let mut rs = Vec::new();
    for i in 0..160 {
        let k = if i % 2 == 0 {
            vec![Kind::Stuck, Kind::FpsCliff]
        } else {
            vec![]
        };
        let o = if k.is_empty() {
            Outcome::Pass
        } else {
            Outcome::Fail
        };
        rs.push(result(&format!("PirCon{i:03}"), o, &k));
    }
    let r = report(&night(rs), 3000);
    assert!(r.len() < 3000, "{} bytes", r.len());
    assert!(r.contains("pass 80 | fail 80"), "{r}");
    assert!(r.contains("more line(s)"), "{r}");
    assert!(
        r.contains("- PirCon000 FAIL stuck@120s, fps-cliff@120s"),
        "{r}"
    );
}

#[test]
fn the_report_lists_aborted_and_skipped() {
    let mut s = RunResult::skipped("ChiCon001", "the window closed");
    s.outcome = Outcome::Skipped;
    let r = report(
        &night(vec![
            result("PirCon001", Outcome::Aborted, &[]),
            result("AllCon001", Outcome::Pass, &[]),
            s,
        ]),
        3000,
    );
    assert!(r.contains("- PirCon001 ABORTED"), "{r}");
    assert!(r.contains("pass: AllCon001"), "{r}");
    assert!(r.contains("not run (the window closed): ChiCon001"), "{r}");
}

// ---- issue dedupe ------------------------------------------------------------

fn existing(
    number: u64,
    state: &str,
    contract: &str,
    kind: Kind,
    last_night: &str,
    seen: u64,
) -> Existing {
    let b = crate::issues::Break {
        contract: contract.into(),
        kind,
        detail: "d".into(),
    };
    Existing {
        number,
        state: state.into(),
        title: crate::issues::title(&b),
        body: body(
            &b,
            ("20261120-0100Z", "aaaaaaaaaa"),
            (last_night, "bbbbbbbbbb"),
            seen,
            698,
        ),
    }
}

#[test]
fn bodies_carry_their_markers_and_no_vm_paths() {
    let e = existing(5, "OPEN", "PirCon001", Kind::Stuck, "20261121-0100Z", 2);
    assert_eq!(e.key(), Some("PirCon001/stuck"));
    assert_eq!(marker(&e.body, "nightly-seen"), Some("2"));
    assert_eq!(
        marker(&e.body, "nightly-first"),
        Some("20261120-0100Z aaaaaaaaaa")
    );
    assert!(e.body.contains("--skip-to PirCon001"));
    assert!(e.body.contains("<install dir>"));
    assert!(!e.body.contains("C:\\") && !e.body.contains("Z:\\"));
}

#[test]
fn one_issue_per_contract_and_kind() {
    // Two runs of the same contract in one night (a re-run) give one break.
    let n = night(vec![
        result("PirCon001", Outcome::Fail, &[Kind::Stuck]),
        result("PirCon001", Outcome::Fail, &[Kind::Stuck]),
        result("AllCon001", Outcome::Fail, &[Kind::Panic, Kind::FpsCliff]),
        result("ChiCon001", Outcome::Pass, &[]),
    ]);
    let b = breaks(&n);
    let keys: Vec<String> = b.iter().map(|b| key(&b.contract, b.kind)).collect();
    assert_eq!(
        keys,
        ["PirCon001/stuck", "AllCon001/panic", "AllCon001/fps-cliff"]
    );
    let actions = plan(&n, &[], 698);
    assert_eq!(actions.len(), 3);
    assert!(
        matches!(&actions[0], Action::Create { title, .. } if title == "Nightly autoplay: PirCon001 stuck")
    );
}

#[test]
fn a_known_break_edits_its_issue_and_counts_it() {
    let n = night(vec![result("PirCon001", Outcome::Fail, &[Kind::Stuck])]);
    let ex = vec![
        existing(5, "OPEN", "PirCon001", Kind::Stuck, "20261121-0100Z", 2),
        // Another kind of the same contract is a different issue.
        existing(6, "OPEN", "PirCon001", Kind::Panic, "20261121-0100Z", 1),
    ];
    let actions = plan(&n, &ex, 698);
    assert_eq!(actions.len(), 1);
    let Action::Edit { number, body, .. } = &actions[0] else {
        panic!("{actions:?}")
    };
    assert_eq!(*number, 5);
    assert_eq!(marker(body, "nightly-seen"), Some("3"));
    assert_eq!(
        marker(body, "nightly-first"),
        Some("20261120-0100Z aaaaaaaaaa")
    );
    assert_eq!(
        marker(body, "nightly-last"),
        Some("20261126-0100Z 0123456789")
    );
}

#[test]
fn the_same_night_twice_changes_nothing() {
    let n = night(vec![result("PirCon001", Outcome::Fail, &[Kind::Stuck])]);
    let ex = vec![existing(
        5,
        "OPEN",
        "PirCon001",
        Kind::Stuck,
        "20261126-0100Z",
        3,
    )];
    assert_eq!(plan(&n, &ex, 698), vec![]);
}

#[test]
fn a_closed_break_that_comes_back_is_reopened() {
    let n = night(vec![result("PirCon001", Outcome::Fail, &[Kind::Stuck])]);
    let ex = vec![existing(
        5,
        "CLOSED",
        "PirCon001",
        Kind::Stuck,
        "20261121-0100Z",
        2,
    )];
    let actions = plan(&n, &ex, 698);
    let Action::Reopen {
        number, comment, ..
    } = &actions[0]
    else {
        panic!("{actions:?}")
    };
    assert_eq!(*number, 5);
    assert!(comment.contains("20261126-0100Z"));
    // An open one wins over an older closed one of the same key.
    let ex = vec![
        existing(5, "CLOSED", "PirCon001", Kind::Stuck, "20261121-0100Z", 2),
        existing(9, "OPEN", "PirCon001", Kind::Stuck, "20261122-0100Z", 1),
    ];
    assert!(matches!(
        plan(&n, &ex, 698)[0],
        Action::Edit { number: 9, .. }
    ));
}

#[test]
fn the_index_lists_open_issues_and_new_ones() {
    let n = night(vec![result("AllCon001", Outcome::Fail, &[Kind::Panic])]);
    let ex = vec![
        existing(5, "OPEN", "PirCon001", Kind::Stuck, "20261121-0100Z", 2),
        existing(6, "CLOSED", "ChiCon001", Kind::Hang, "20261121-0100Z", 1),
    ];
    let actions = plan(&n, &ex, 698);
    let open = open_after(&ex, &actions, &[("AllCon001/panic".into(), 12)], &n.run_id);
    let s = index_body(&open);
    assert!(s.starts_with(INDEX_MARKER));
    assert!(
        s.contains("- #12 AllCon001 `panic` (since 20261126-0100Z)"),
        "{s}"
    );
    assert!(
        s.contains("- #5 PirCon001 `stuck` (since 20261120-0100Z)"),
        "{s}"
    );
    assert!(!s.contains("ChiCon001"), "{s}");
    // Sorted by contract: AllCon before PirCon.
    assert!(s.find("AllCon001").unwrap() < s.find("PirCon001").unwrap());
}

#[test]
fn dry_run_posts_nothing_and_says_what_it_would() {
    let n = night(vec![result("AllCon001", Outcome::Fail, &[Kind::Panic])]);
    let tmp = std::env::temp_dir();
    let log = crate::issues::apply("o/r", 698, &n, &[], None, true, &tmp).unwrap();
    assert!(
        log[0].starts_with("create \"Nightly autoplay: AllCon001 panic\""),
        "{log:?}"
    );
    assert!(
        log.iter().any(|l| l == "add the index comment on #698"),
        "{log:?}"
    );
    // With the index already saying the same, it is left alone.
    let open = open_after(
        &[],
        &plan(&n, &[], 698),
        &[("AllCon001/panic".into(), 0)],
        &n.run_id,
    );
    let log = crate::issues::apply(
        "o/r",
        698,
        &n,
        &[],
        Some((1, index_body(&open))),
        true,
        &tmp,
    )
    .unwrap();
    assert!(
        log.iter().any(|l| l == "index on #698: unchanged"),
        "{log:?}"
    );
}

// ---- cancels and Retries -----------------------------------------------------

/// A race cancelled by its timer, Retry taken, and the contract put back
/// on offer instead of restarted (the shape of the nightly trial's run).
fn cancelled_log(retry_restarts: bool) -> String {
    let mut s = String::from("autoplay 4.6 s: on contract Some((\"PirCon001\", 1))\n");
    s += "autoplay 15.8 s: objectives [\"A\", \"Race (0/30)\"]\n";
    for (i, t) in [18.2, 20.9, 25.1, 38.0, 46.6, 61.7, 66.6]
        .iter()
        .enumerate()
    {
        s += &format!(
            "autoplay {t:.1} s: objectives [\"A\", \"Race ({}/30)\"]\n",
            i + 1
        );
        s += &format!("autoplay {t:.1} s: racing to checkpoint{i}\n");
    }
    s += "autoplay 79.8 s: racing, something solid ahead: round it\n";
    s += "mission: contract cancelled (1 so far)\n";
    s += "autoplay: contract cancelled (1 so far) after 79.8 s of play\n";
    s += "autoplay 79.8 s: objectives [\"A\"]\n";
    s += "autoplay 82.6 s: at [1.0, -36.0, 1.0], in a boat\n";
    s += "autoplay: she takes Retry on the failed contract's fanfare (HUD call 734, retry 1)\n";
    if retry_restarts {
        s += "autoplay 85.0 s: objectives [\"A\", \"Race (0/30)\"]\n";
        s += "autoplay 90.0 s: objectives [\"A\", \"Race (1/30)\"]\n";
        s += "autoplay: contract 1 completed after 400.0 s of play\n";
        s += "autoplay: 401.0 s played, 0 hops;\n";
    } else {
        s += "autoplay 82.8 s: on contract None\n";
        for t in (90..710).step_by(15) {
            s += &format!("autoplay {t}.0 s: at [5.0, -36.0, 5.0], on foot\n");
        }
    }
    s
}

#[test]
fn a_cancel_and_a_retry_that_put_it_back_on_offer_are_named() {
    let a = analyse(
        &cancelled_log(false),
        &Exit {
            code: None,
            killed: Some(Killed::Stuck),
            wall_seconds: 600.0,
        },
        &th(),
    );
    let kinds: Vec<Kind> = a.failures.iter().map(|f| f.kind).collect();
    assert_eq!(
        kinds,
        [
            Kind::ContractCancelled,
            Kind::RetryFailed,
            Kind::Stuck,
            Kind::NotCompleted
        ]
    );
    let c = &a.failures[0];
    assert_eq!(c.at, Some(79.8));
    assert!(
        c.detail
            .contains("1 time(s), at 79.8 s of play, the first while racing"),
        "{}",
        c.detail
    );
    assert!(c.detail.contains("1 Retry and 0 Continue"), "{}", c.detail);
    let r = &a.failures[1];
    assert_eq!(r.at, Some(82.6));
    assert!(
        r.detail.contains("on no contract from 82.8 s"),
        "{}",
        r.detail
    );
    // The stuck stretch after it is the cancel's doing: no issue of its own.
    assert_eq!(
        issue_kinds(&a.failures),
        vec![Kind::RetryFailed, Kind::ContractCancelled]
    );
    assert_eq!(Kind::from_name("retry-failed"), Some(Kind::RetryFailed));
    assert_eq!(
        Kind::from_name("contract-cancelled"),
        Some(Kind::ContractCancelled)
    );
}

#[test]
fn a_retry_that_restarts_and_completes_is_a_pass() {
    let a = analyse(&cancelled_log(true), &clean(), &th());
    assert!(a.completed);
    assert_eq!(a.failures, vec![]);
}

#[test]
fn stuck_before_a_cancel_still_files() {
    let fs = vec![
        Failure {
            kind: Kind::Stuck,
            at: Some(100.0),
            detail: String::new(),
        },
        Failure {
            kind: Kind::ContractCancelled,
            at: Some(900.0),
            detail: String::new(),
        },
        Failure {
            kind: Kind::NotCompleted,
            at: None,
            detail: String::new(),
        },
    ];
    assert_eq!(issue_kinds(&fs), vec![Kind::ContractCancelled, Kind::Stuck]);
}

/// The nightly trial2's shape: Retry, autoplay on no contract while the
/// retry save reloads behind the loading screen, then the contract back
/// 5.4 s later and the race again. Not a failed Retry.
#[test]
fn a_retry_that_reloads_behind_the_loading_screen_is_not_failed() {
    let mut s = String::from("autoplay 4.6 s: on contract Some((\"PirCon001\", 45))\n");
    s += "autoplay 15.8 s: objectives [\"A\", \"Race (0/30)\"]\n";
    s += "autoplay 47.0 s: racing to checkpoint5\n";
    s += "autoplay: contract cancelled (1 so far) after 47.4 s of play\n";
    s += "autoplay 47.4 s: objectives [\"A\"]\n";
    s += "autoplay: she takes Retry on the failed contract's fanfare (HUD call 642, retry 1)\n";
    s += "autoplay 50.4 s: on contract None\n";
    s += "loading screen: Start { covered: false } at 50.5 s\n";
    s += "loading screen: FadeOut at 54.6 s\n";
    s += "autoplay 55.8 s: on contract Some((\"PirCon001\", 45))\n";
    s += "autoplay 56.0 s: objectives [\"A\", \"Race (0/30)\"]\n";
    for t in (60..200).step_by(20) {
        s += &format!(
            "autoplay {t}.0 s: objectives [\"A\", \"Race ({}/30)\"]\n",
            t / 20
        );
    }
    let wall = Exit {
        code: None,
        killed: Some(Killed::Wall),
        wall_seconds: 2700.0,
    };
    let a = analyse(&s, &wall, &th());
    let kinds: Vec<Kind> = a.failures.iter().map(|f| f.kind).collect();
    assert!(!kinds.contains(&Kind::RetryFailed), "{:?}", a.failures);
    assert!(kinds.contains(&Kind::ContractCancelled));
    // Cut off before the grace is up (the window closing): not judged.
    let cut: String = s.lines().take(7).map(|l| format!("{l}\n")).collect();
    let deadline = Exit {
        code: None,
        killed: Some(Killed::Deadline),
        wall_seconds: 60.0,
    };
    let a = analyse(&cut, &deadline, &th());
    assert!(
        a.failures.iter().all(|f| f.kind != Kind::RetryFailed),
        "{:?}",
        a.failures
    );
}
