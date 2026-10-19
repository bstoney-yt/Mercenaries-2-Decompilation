//! Measures the clips the faction HQs' animated briefings play by name
//! (`Human.PlayRawAnimation` from `WifHq._NextCinematicFrame`): whether
//! each is in the archive, its length, and the events it carries (the
//! scripts wait on `CinematicStart` and `CinematicEnd`).
//!
//! ```text
//! cargo run --release -p m2-engine --example briefclips -- "<install dir>" <names.txt>
//! ```
//!
//! `names.txt` holds one clip name per line (taken from the briefing
//! tables of your own copy's scripts). Prints names, hashes and numbers
//! only: one row per clip, then the totals and each event name's count.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::animation::{ANIMATION_TYPE, Clip};
use m2_assets::name_hash;
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [root, list] = &args[..] else {
        eprintln!("usage: briefclips <install dir> <names.txt>");
        std::process::exit(2);
    };
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let names = std::fs::read_to_string(list)?;
    let (mut found, mut missing, mut ended) = (0, 0, 0);
    let mut events: BTreeMap<String, usize> = BTreeMap::new();
    for name in names.lines().map(str::trim).filter(|n| !n.is_empty()) {
        let hash = name_hash(name);
        let clip = source
            .asset(hash, ANIMATION_TYPE)
            .ok()
            .and_then(|b| Clip::parse(&b).ok());
        let Some(clip) = clip else {
            missing += 1;
            println!("{hash:08x}\t{name}\tmissing");
            continue;
        };
        found += 1;
        let names: Vec<String> = clip
            .events
            .iter()
            .map(|e| format!("{}@{:.2}", e.name, e.time))
            .collect();
        if clip.events.iter().any(|e| e.name == "CinematicEnd") {
            ended += 1;
        }
        for e in &clip.events {
            *events
                .entry(format!(
                    "[{}] {}",
                    e.category,
                    if e.category.is_empty() || e.category == "camera" {
                        e.name.as_str()
                    } else {
                        "(line)"
                    }
                ))
                .or_default() += 1;
        }
        println!(
            "{hash:08x}\t{name}\t{:.2} s\t{} tracks\t{}",
            clip.header.duration,
            clip.track_nodes.len(),
            names.join(" ")
        );
    }
    println!("clips {found} found, {missing} missing; {ended} carry CinematicEnd");
    for (name, n) in events {
        println!("event {name}: {n}");
    }
    Ok(())
}
