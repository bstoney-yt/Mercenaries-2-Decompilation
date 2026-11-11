//! Prints the human state table of your own installation
//! ([`m2_assets::human_states`]): every stance and action with its
//! settings, and where each message leads. With a stance (and an action)
//! only those; with `walk` and a list of messages, the states a person
//! goes through from a stance and action.
//!
//! ```text
//! cargo run --release -p m2-engine --example humanstates -- "<install dir>" [stance [action]]
//! cargo run --release -p m2-engine --example humanstates -- "<install dir>" walk Upright Idle JumpMoving JumpInAir Complete MovingOnGround Complete
//! ```
//!
//! Prints names and numbers only.

use std::path::Path;

use m2_assets::human_states::{HUMAN_STATES, HUMAN_STATES_TYPE, HumanStates};
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = args.first().ok_or(
        "usage: humanstates <install dir> [stance [action]] | walk <stance> <action> <message>...",
    )?;
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let table = HumanStates::parse(&source.asset(HUMAN_STATES, HUMAN_STATES_TYPE)?)?;
    if args.get(1).map(String::as_str) == Some("walk") {
        let (Some(stance), Some(action)) = (args.get(2), args.get(3)) else {
            return Err("walk needs a stance and an action".into());
        };
        let mut at = (stance.clone(), action.clone());
        println!("{}.{}", at.0, at.1);
        for message in &args[4..] {
            match table.next(&at.0, &at.1, message) {
                Some(t) => {
                    at = (t.stance.clone(), t.action.clone());
                    println!("  {message} -> {}.{}", at.0, at.1);
                }
                None => println!("  {message} -> (ignored)"),
            }
        }
        return Ok(());
    }
    let actions: usize = table.stances.iter().map(|s| s.actions.len()).sum();
    let transitions: usize = table
        .stances
        .iter()
        .flat_map(|s| &s.actions)
        .map(|a| a.transitions.len())
        .sum();
    eprintln!(
        "{} stances, {actions} actions, {transitions} transitions (first word {})",
        table.stances.len(),
        table.unk_0x00
    );
    let same = |a: &str, b: Option<&String>| b.is_none_or(|b| a.eq_ignore_ascii_case(b));
    for stance in &table.stances {
        if !same(&stance.name, args.get(1)) {
            continue;
        }
        for action in &stance.actions {
            if !same(&action.name, args.get(2)) {
                continue;
            }
            println!(
                "{}.{}\t{}\t{} transitions",
                stance.name,
                action.name,
                action.words.join(" "),
                action.transitions.len()
            );
            for t in &action.transitions {
                println!("\t{} -> {}.{}", t.message, t.stance, t.action);
            }
        }
    }
    Ok(())
}
