//! Surveys the curves of the world's mixed cues ([`m2_assets::voice::Curve`]):
//! counts by kind and parameter, each kind's range, and a cue's curves in
//! full when given.
//!
//! ```text
//! cargo run --release -p m2-engine --example enginecurves -- "<install dir>" [cue hex]...
//! ```
//!
//! Prints hashes and numbers only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_engine::Install;
use m2_engine::audio::world_cue_tables;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = args
        .first()
        .ok_or("usage: enginecurves <install dir> [cue]...")?;
    let picked: Vec<u32> = args[1..]
        .iter()
        .filter_map(|k| u32::from_str_radix(k, 16).ok())
        .collect();
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let tables = world_cue_tables(&mut source);
    let mut stats: BTreeMap<(u32, u32), (usize, f32, f32, f32)> = BTreeMap::new();
    let (mut cues, mut looping, mut rising, mut all) = (0, 0, 0, 0);
    for t in &tables {
        for m in &t.mixes {
            if m.curves.iter().all(Vec::is_empty) {
                continue;
            }
            cues += 1;
            looping += usize::from(m.seconds < 0.0);
            if picked.contains(&m.cue) {
                println!("cue {:08x}, table {:08x}, {} s:", m.cue, t.hash, m.seconds);
                for (k, (tr, cs)) in m.tracks.iter().zip(&m.curves).enumerate() {
                    println!("  track {k} record {} start {}:", tr.record, tr.start);
                    for c in cs {
                        println!("    kind {} param {:08x}: {:?}", c.kind, c.param, c.points);
                    }
                }
            }
            for c in m.curves.iter().flatten() {
                all += 1;
                rising += usize::from(c.points.windows(2).all(|w| w[1].0 >= w[0].0));
                let e = stats
                    .entry((c.kind, c.param))
                    .or_insert((0, f32::MAX, f32::MIN, 0.0));
                e.0 += 1;
                for p in &c.points {
                    e.1 = e.1.min(p.1);
                    e.2 = e.2.max(p.1);
                    e.3 = e.3.max(p.0);
                }
            }
        }
    }
    let (mut read, mut asked) = (0, 0);
    for &k in &picked {
        asked += 1;
        if let Some(c) = m2_engine::audio::EngineCue::read(&mut source, &tables, k) {
            read += 1;
            use m2_engine::audio::{PARAM_LOAD, PARAM_SPEED};
            for speed in [0.0, 0.25, 0.5, 1.0] {
                println!(
                    "{k:08x} speed {speed}: {:?}",
                    c.mix(&[(PARAM_SPEED, speed), (PARAM_LOAD, 0.5)])
                );
            }
        }
    }
    println!("{read} of {asked} cues given play as engine cues");
    println!("{cues} mixed cues with curves, {looping} looping; {rising} of {all} curves rise");
    for ((k, p), (n, lo, hi, x)) in stats {
        println!("kind {k} param {p:08x}: {n} curves, values {lo:.3} to {hi:.3}, x to {x:.3}");
    }
    Ok(())
}
