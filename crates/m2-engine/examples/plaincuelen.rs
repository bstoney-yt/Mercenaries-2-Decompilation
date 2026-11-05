//! Checks the world's plain cues against what they play: for each plain
//! cue of the cue tables with a length, whether the longest sound its
//! record finds lasts that long (within 2%), and the same for the cues
//! given (the vehicles' engine starts and stops, whose record words carry
//! a high half).
//!
//! ```text
//! cargo run --release -p m2-engine --example plaincuelen -- "<install dir>" [cue hex]...
//! ```
//!
//! Prints hashes, lengths and counts only.

use std::path::Path;

use m2_engine::Install;
use m2_engine::audio::{KeySounds, world_cue_tables};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = args
        .first()
        .ok_or("usage: plaincuelen <install dir> [cue]...")?;
    let picked: Vec<u32> = args[1..]
        .iter()
        .filter_map(|k| u32::from_str_radix(k, 16).ok())
        .collect();
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let tables = world_cue_tables(&mut source);
    let (mut all, mut all_match, mut some, mut some_match) = (0, 0, 0, 0);
    for t in &tables {
        for c in &t.cues {
            let is_picked = picked.contains(&c.cue);
            if c.seconds <= 0.0 || !(is_picked || all < 400) {
                continue;
            }
            let Some(k) = KeySounds::resolve(&mut source, std::slice::from_ref(t), c.cue) else {
                continue;
            };
            let longest = k.sounds.iter().map(|s| s.0.seconds()).fold(0.0, f32::max);
            let ok = (longest - c.seconds).abs() <= 0.02 * c.seconds.max(0.1);
            if is_picked {
                some += 1;
                some_match += usize::from(ok);
                println!(
                    "{:08x} table {:08x} record {}: cue {:.3} s, longest sound {longest:.3} s{}",
                    c.cue,
                    t.hash,
                    c.record,
                    c.seconds,
                    if ok { "" } else { "  (differs)" }
                );
            } else {
                all += 1;
                all_match += usize::from(ok);
            }
        }
    }
    println!("other plain cues: {all_match} of {all} last as long as their longest sound");
    println!("cues given: {some_match} of {some}");
    Ok(())
}
