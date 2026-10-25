//! Measures how every effect's particles move: each effect alone at the
//! origin, stepped 2 s at 30 Hz; prints, over the effects that have
//! particles then, the mean rise and mean horizontal spread of their
//! particles, and how many rise, fall or stay level. Reads the install
//! given as the first argument (the folder holding `data\vz.wad`).
use std::collections::HashMap;
use std::io::BufReader;

use m2_engine::AssetSource;
use m2_engine::particles::{Library, Particles};
use m2_math::glam::Vec3;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: fx_motion <install folder>")?;
    let f = std::fs::File::open(std::path::Path::new(&root).join("data").join("vz.wad"))?;
    let mut source = AssetSource::new(BufReader::new(f))?;
    let lib = Library::load(&mut source, HashMap::new())?;
    let mut kinds: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for spec in lib.effects.values() {
        for f in &spec.forces {
            *kinds.entry(format!("{:?}", f.kind)).or_default() += 1;
        }
    }
    println!("forces by kind: {kinds:?}");
    let mut hashes: Vec<u32> = lib.effects.keys().copied().collect();
    hashes.sort_unstable();
    let (mut n, mut rise_sum, mut spread_sum) = (0usize, 0.0f32, 0.0f32);
    let (mut up, mut down, mut level) = (0, 0, 0);
    for h in hashes {
        let mut p = Particles::default();
        p.spawn(&lib, h, Vec3::ZERO, 0.0, 1.0);
        for _ in 0..60 {
            p.step(&lib, 1.0 / 30.0);
        }
        let at = p.positions();
        if at.is_empty() {
            continue;
        }
        let k = at.len() as f32;
        let rise = at.iter().map(|q| q.y).sum::<f32>() / k;
        let spread = at.iter().map(|q| q.with_y(0.0).length()).sum::<f32>() / k;
        n += 1;
        rise_sum += rise;
        spread_sum += spread;
        if rise > 0.5 {
            up += 1;
        } else if rise < -0.5 {
            down += 1;
        } else {
            level += 1;
        }
    }
    println!(
        "{n} effects with particles at 2 s: mean rise {:.2} m, mean spread {:.2} m; rising {up}, falling {down}, level {level}",
        rise_sum / n.max(1) as f32,
        spread_sum / n.max(1) as f32
    );
    Ok(())
}
