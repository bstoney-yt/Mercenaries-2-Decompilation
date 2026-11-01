//! Tests on small made-up log snippets in m2-alpha's line formats.

use super::*;

fn run(log: &str) -> Summary {
    summarize(log, Config::default())
}

#[test]
fn timed_prefixes() {
    assert_eq!(
        timed("autoplay 12.5 s: at [1, 2, 3]"),
        Some((12.5, "at [1, 2, 3]"))
    );
    assert_eq!(
        timed("accept: 3.0 s: context action").map(|t| t.0),
        Some(3.0)
    );
    assert_eq!(
        timed("autoplay tally 40.0 s: 1 rounds").map(|t| t.0),
        Some(40.0)
    );
    assert_eq!(
        timed("after contract: 50 s, she is at [0, 0, 0]").map(|t| t.0),
        Some(50.0)
    );
    assert_eq!(timed("autoplay: contract 1 completed"), None);
    assert_eq!(timed("mission: controls on"), None);
    assert_eq!(
        after_play("autoplay: contract 1 completed after 99.5 s of play (40 s here)"),
        Some(99.5)
    );
}

#[test]
fn triples() {
    assert_eq!(triple("1.5, -2, 3]"), Some([1.5, -2.0, 3.0]));
    assert_eq!(triple("1, 2]"), None);
    assert_eq!(
        triple_after_last("at [1, 1, 1] (x): hopped to [2, 2, 2]", "hopped to ["),
        Some([2.0, 2.0, 2.0])
    );
}

#[test]
fn contract_flow() {
    let log = "\
load: 15 s on, the scripts hold 1 done [TstCon000], active [], on offer [TstCon001]; she is at (1, 2, 3)
autoplay 0.0 s: on contract None
autoplay 5.0 s: on contract None
accept: TstCon001 is active, taken from its starter after 20.5 s of play
autoplay 20.5 s: on contract Some((\"TstCon001\", 12))
autoplay 21.0 s: objectives [\"Reach the tower\"]
autoplay 22.0 s: objectives [\"Reach the tower\"]
autoplay 40.0 s: objectives [\"Hold the tower (1/3)\"]
game saved with 1 done [TstCon000], active [TstCon001], on offer []
game saved with 1 done [TstCon000], active [TstCon001], on offer []
autoplay: contract 1 completed after 60.0 s of play (30 s here)
game saved with 2 done [TstCon000 TstCon001], active [], on offer []
autoplay: 61.0 s played, 0 hops
wrote C:\\x\\run.ppm
";
    let s = run(log);
    let states: Vec<_> = s
        .contract_states
        .iter()
        .map(|e| (e.t, e.what.as_str()))
        .collect();
    assert_eq!(states, [(0.0, "none"), (20.5, "TstCon001")]);
    assert_eq!(s.accepted.len(), 1);
    assert_eq!(s.accepted[0].what, "TstCon001");
    assert_eq!(s.completed[0].t, 60.0);
    assert_eq!(s.objectives.len(), 2);
    assert_eq!(s.saves.len(), 2);
    assert_eq!(s.saves[1].t, 60.0);
    assert_eq!(s.loads.len(), 1);
    assert_eq!(s.end.verdict, "clean");
    assert_eq!(s.end.played, Some(61.0));
    assert!(s.end.screenshot_written);
    assert!(!s.end.not_completed);
    let text = to_text(&s, "t.err");
    assert!(text.contains("completed: 60.0 contract 1"), "{text}");
}

#[test]
fn ends() {
    let s = run("autoplay 5.0 s: at [0, 0, 0], on foot\n");
    assert_eq!(s.end.verdict, "truncated");
    let s = run("autoplay 5.0 s: at [0, 0, 0]\nthread 'main' panicked at src/x.rs:1:2:\n");
    assert_eq!(s.end.verdict, "panic");
    assert!(s.end.message.as_deref().unwrap().contains("panicked"));
    let s = run("Error: no install\n");
    assert_eq!(s.end.verdict, "error");
    let log = "autoplay 99.6 s: at [0, 0, 0]\nautoplay: 100.0 s played, 2 hops; [0, 0, 0] to [1, 1, 1]; the contract was not completed\n";
    let s = summarize(
        log,
        Config {
            mission_seconds: Some(100.0),
            ..Config::default()
        },
    );
    assert_eq!(s.end.verdict, "clean");
    assert!(s.end.not_completed && s.end.time_limit);
    assert_eq!(s.end.hops, Some(2));
    assert!(!s.end.screenshot_written);
}

#[test]
fn locks_and_hijacks() {
    let log = "\
autoplay 10.0 s: no headway for 20 s at [5, 0, 5] (getting into a tank): hopped to [9, 0, 9]
hijack: Tank01 started
hijack: started: 3 modules, 0 failures
autoplay 10.0 s: controls off: waiting
mission: controls off
autoplay tally 15.0 s: 0 rounds
autoplay 20.0 s: controls on
hijack: done
autoplay 25.0 s: controls off: waiting
autoplay 26.0 s: taking the 0x1 left by 0x2, 3 m off, from [9, 0, 9]
autoplay 27.0 s: controls on
autoplay 30.0 s: controls off: waiting
";
    let s = run(log);
    assert_eq!(s.locks.len(), 3);
    assert_eq!((s.locks[0].start, s.locks[0].end), (10.0, Some(20.0)));
    assert_eq!(s.locks[0].action_lines, 0);
    assert_eq!(s.locks[1].action_lines, 1);
    assert_eq!(s.locks[2].end, None);
    assert_eq!(s.hijacks.len(), 1);
    assert_eq!(s.hijacks[0].label, "Tank01");
    // It ends at the first timed line after `hijack: done`.
    assert_eq!((s.hijacks[0].start, s.hijacks[0].end), (10.0, Some(25.0)));
    assert_eq!(s.hops.len(), 1);
    assert_eq!(s.hops[0].what, "no headway 20 s");
}

#[test]
fn markers_and_idle() {
    let log = "\
autoplay 1.0 s: at [0, 0, 0], on foot
mission: marker HUD_objective_action on \"Crate A\" (0x10) for TstCon001: colour [1, 2, 3, 4]
autoplay 9.0 s: at [0, 0, 0], on foot
mission: marker HUD_objective_action on \"Crate A\" (0x10) for TstCon001: colour [1, 2, 3, 4]
mission: marker HUD_exit on \"Door\" (0x20) for nil: colour [1, 2, 3, 4]
autoplay 12.0 s: nothing to do, from [0, 0, 0]
autoplay 13.0 s: at [0, 0, 0], on foot, health 100, nothing to do; closest inf m
";
    let s = run(log);
    assert_eq!(s.markers.len(), 2);
    let crate_a = &s.markers[0];
    assert_eq!((crate_a.first, crate_a.last, crate_a.lines), (1.0, 9.0, 2));
    assert_eq!(crate_a.id, "0x10");
    assert_eq!(s.markers[1].kind, "HUD_exit");
    assert_eq!(s.idle, [12.0, 13.0]);
}

#[test]
fn still_windows_and_threshold() {
    let mut log = String::new();
    // Walks for 30 s, then stands (with 1 m of jitter) from 40 to 130 s.
    for i in 0..4 {
        let _ = writeln!(
            log,
            "autoplay {}.0 s: at [{}, 0, 0], on foot",
            i * 10,
            i * 20
        );
    }
    for i in 4..14 {
        let _ = writeln!(
            log,
            "autoplay {}.0 s: at [100, 0, {}], on foot",
            i * 10,
            i % 2
        );
    }
    let _ = writeln!(log, "autoplay 140.0 s: at [200, 0, 0], on foot");
    let s = run(&log);
    let w = s.longest_still.clone().unwrap();
    assert_eq!((w.start, w.end, w.samples), (40.0, 130.0, 10));
    assert_eq!(s.stalls.len(), 1);
    let s = summarize(
        &log,
        Config {
            stall_seconds: 100.0,
            ..Config::default()
        },
    );
    assert!(s.stalls.is_empty());
    let s = summarize(
        &log,
        Config {
            still_radius: 0.5,
            ..Config::default()
        },
    );
    // 1 m of jitter breaks every stretch at 0.5 m.
    assert!(s.stalls.is_empty());
}

#[test]
fn aba_loops() {
    let mut log = String::new();
    // A, B, A, B, A, then off somewhere else: three returns.
    let path = [0.0, 30.0, 1.0, 31.0, 0.5, 100.0, 200.0];
    for (i, x) in path.iter().enumerate() {
        let _ = writeln!(
            log,
            "autoplay {}.0 s: fighting back a soldier 8 m off, from [{x}, 0, 0]",
            i * 10
        );
    }
    let s = run(&log);
    assert_eq!(s.loops.len(), 1, "{:?}", s.loops);
    let l = &s.loops[0];
    assert_eq!((l.start, l.end, l.returns), (0.0, 40.0, 3));
    // A straight walk has none.
    let mut log = String::new();
    for i in 0..10 {
        let _ = writeln!(log, "autoplay {i}.0 s: at [{}, 0, 0], on foot", i * 10);
    }
    assert!(run(&log).loops.is_empty());
}

#[test]
fn lua_errors_and_memory() {
    let log = "\
mission: the scripts' state: script: runtime error: TstFlow:10: bad argument #1 to 'pairs' (table expected, got nil)
\tstack traceback: in main chunk
mission: the scripts' state: script: runtime error: TstFlow:10: bad argument #1 to 'pairs' (table expected, got nil)
mission: TstScript:4: attempt to index a nil value
lua memory: 2048 KB
lua memory: 3 MB
pace: 60 fps over 30 s, slowest frame 45 ms, 0 over 50 ms, 1280x720
pace: 41 fps over 30 s, slowest frame 120 ms, 3 over 50 ms, 1280x720
";
    let s = run(log);
    assert_eq!(s.lua_errors.len(), 2);
    assert_eq!(s.lua_errors[0].count, 2);
    assert!(s.lua_errors[0].message.starts_with("TstFlow:10"));
    assert!(s.lua_errors[1].message.contains("attempt to index"));
    let m = s.lua_memory_kb.unwrap();
    assert_eq!((m.min, m.max, m.count), (2048.0, 3072.0, 2));
    let f = s.fps.unwrap();
    assert_eq!((f.min, f.max), (41.0, 60.0));
    assert_eq!(s.slowest_frame_ms.unwrap().max, 120.0);
}

#[test]
fn json_is_well_formed_enough() {
    let s = run("autoplay 1.0 s: objectives [\"Say \\\"hi\\\"\tnow\"]\n");
    let j = to_json(&s, "C:\\runs\\a.err");
    assert!(j.contains("\"log\":\"C:\\\\runs\\\\a.err\""), "{j}");
    assert!(j.contains("\\t"), "{j}");
    assert!(j.trim_end().ends_with('}'));
    let opens = j.matches('{').count() + j.matches('[').count();
    let closes = j.matches('}').count() + j.matches(']').count();
    assert_eq!(opens, closes);
}

#[test]
fn objective_items_come_and_go() {
    assert_eq!(
        objective_names(r#"["Hold the tower (2/3)", "Say \"hi\"", "Count (a/b)"]"#),
        ["Hold the tower", "Say \"hi\"", "Count (a/b)"]
    );
    let log = "\
autoplay 1.0 s: objectives [\"Reach the tower\"]
autoplay 5.0 s: objectives [\"Hold the tower (0/3)\", \"Find the car\"]
autoplay 6.0 s: objectives [\"Hold the tower (1/3)\"]
autoplay 7.0 s: objectives [\"Hold the tower (2/3)\", \"Find the car\"]
autoplay 9.0 s: objectives []
";
    let s = run(log);
    let items: Vec<_> = s
        .objective_items
        .iter()
        .map(|i| (i.name.as_str(), i.first, i.gone, i.appearances))
        .collect();
    assert_eq!(
        items,
        [
            ("Reach the tower", 1.0, Some(5.0), 1),
            ("Hold the tower", 5.0, Some(9.0), 1),
            ("Find the car", 5.0, Some(9.0), 2),
        ]
    );
}

#[test]
fn status_lines_are_not_actions_in_locks() {
    let log = "\
autoplay 1.0 s: controls off: waiting
autoplay 2.0 s: HQ state: inside, exits offered [], at [1, 2, 3]
autoplay 2.0 s: objectives [\"Reach the tower\"]
autoplay 2.0 s: on contract None
autoplay 2.0 s: at [1, 2, 3], on foot, health 100
autoplay 3.0 s: fighting back a soldier 8 m off, from [1, 2, 3]
autoplay 4.0 s: controls on
";
    let s = run(log);
    assert_eq!(s.locks[0].action_lines, 1);
}
