//! Finds each vehicle sound bank's horn as the alpha picks it
//! ([`EngineSounds::horn_of`]): per `veh_` bank, the horn's hash, length
//! and level, or none; then how many sounds of every bank of the archive
//! are a horn's length ([`HORN_FRAMES`]) and how many of those are held
//! notes.
//!
//! ```text
//! cargo run --release -p m2-engine --example horns -- "<install dir>"
//! ```
//!
//! Prints hashes, lengths and counts only.

use std::collections::BTreeSet;
use std::path::Path;

use m2_engine::Install;
use m2_engine::audio::{EngineSounds, HORN_FRAMES, SoundLibrary};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: horns <install dir>")?;
    let install = Install::find(Path::new(&root))?;
    let mut source = install.open_world()?;
    let stems: BTreeSet<String> = source
        .archive()
        .index()
        .blocks()
        .iter()
        .map(|b| m2_engine::world::layer_name(&b.path))
        .collect();
    let all: Vec<&str> = stems.iter().map(String::as_str).collect();
    let library = SoundLibrary::load(&mut source, &all)?;
    let (mut banks, mut with, mut sounds, mut long, mut held) = (0, 0, 0, 0, 0);
    for stem in &all {
        let bank = library.bank(stem);
        if bank.is_empty() {
            continue;
        }
        sounds += bank.len();
        for s in bank.iter().filter(|s| HORN_FRAMES.contains(&s.frames())) {
            long += 1;
            held += usize::from(s.is_tone());
            if !stem.starts_with("veh_") {
                println!(
                    "{stem}: a horn-length sound {:08x}, {} frames (held note: {})",
                    s.hash,
                    s.frames(),
                    s.is_tone()
                );
            }
        }
        if !stem.starts_with("veh_") {
            continue;
        }
        banks += 1;
        match EngineSounds::horn_of(bank) {
            Some(h) => {
                with += 1;
                println!(
                    "{stem}: horn {:08x}, {} frames, level {:.3}",
                    h.hash,
                    h.frames(),
                    h.loudness()
                );
            }
            None => println!("{stem}: no horn"),
        }
    }
    println!("{with} of {banks} vehicle banks hold a horn");
    println!(
        "{long} of {sounds} sounds in all banks are a horn's length, {held} of them held notes"
    );
    Ok(())
}
