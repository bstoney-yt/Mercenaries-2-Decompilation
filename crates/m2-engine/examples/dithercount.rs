//! Grades a run's log for soldiers stuck dithering on one spot: reads the
//! `people: <id> dithers at [x, y, z]: gives the move up` lines a run
//! prints ([`m2_engine::ai_dither`]), groups each soldier's give-ups by
//! spot (within [`m2_engine::ai_dither`]'s 2 m), and prints the worst.
//!
//! ```text
//! cargo run --release -p m2-engine --example dithercount -- <run log> [limit]
//! ```
//!
//! `dithercount: PASS` when no soldier gave a move up more than `limit`
//! times (default 2) at one spot, else `dithercount: FAIL` naming him. A
//! give-up marks his spot bad for 60 s so that the planner picks again
//! (360 0x822C8370); one who gives up there again and again never got
//! away from it: the stuck soldiers of the playtest of e8749a2a (one of
//! them 33 times at one spot and 24 at the next). Invented: the limit.
//! Prints ids, places and counts only.

use std::collections::BTreeMap;

fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: dithercount <run log> [limit]");
        return std::process::ExitCode::FAILURE;
    };
    let limit: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(2);
    let Ok(text) = std::fs::read(&path) else {
        println!("dithercount: FAIL cannot read {path}");
        return std::process::ExitCode::FAILURE;
    };
    let text = String::from_utf8_lossy(&text);
    let mut spots: BTreeMap<String, Vec<([f32; 3], usize)>> = BTreeMap::new();
    let mut lines = 0;
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("people: ") else {
            continue;
        };
        let Some((id, rest)) = rest.split_once(" dithers at [") else {
            continue;
        };
        let Some((at, _)) = rest.split_once(']') else {
            continue;
        };
        let p: Vec<f32> = at
            .split(',')
            .filter_map(|v| v.trim().parse().ok())
            .collect();
        let [x, y, z] = p[..] else { continue };
        lines += 1;
        let list = spots.entry(id.to_owned()).or_default();
        let near =
            |s: &[f32; 3]| (s[0] - x).powi(2) + (s[2] - z).powi(2) < 4.0 && (s[1] - y).abs() < 2.0;
        match list.iter_mut().find(|(s, _)| near(s)) {
            Some((_, n)) => *n += 1,
            None => list.push(([x, y, z], 1)),
        }
    }
    let worst = spots
        .iter()
        .flat_map(|(id, l)| l.iter().map(move |(s, n)| (*n, id.clone(), *s)))
        .max_by_key(|w| w.0);
    let stuck = spots.values().flatten().filter(|(_, n)| *n > limit).count();
    println!(
        "dithercount: {lines} give-ups by {} soldiers; {stuck} spots past {limit}",
        spots.len()
    );
    match worst {
        Some((n, id, s)) if n > limit => {
            println!(
                "dithercount: FAIL {id} gave a move up {n} times at [{:.1}, {:.1}, {:.1}]",
                s[0], s[1], s[2]
            );
            std::process::ExitCode::FAILURE
        }
        _ => {
            println!(
                "dithercount: PASS no soldier gave a move up more than {limit} times at one spot"
            );
            std::process::ExitCode::SUCCESS
        }
    }
}
