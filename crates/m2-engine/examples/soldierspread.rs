//! Measures how often soldiers' rounds hit her (gap round 15's harness):
//! for every person template with an `AiAttributes` `Accuracy`, with the
//! gun its `EquipmentLink`s hand it (else the Assault Rifle, as in play),
//! the share of rounds that pass within the alpha's 0.45 m of her chest at
//! 10, 20 and 40 m, under the old invented spread (the same for every gun)
//! and under [`m2_engine::weapons::SoldierSpread`] (the gun's
//! `LowSkillScatter` plus a miss by `Accuracy`).
//!
//! ```text
//! cargo run --release -p m2-engine --example soldierspread -- "<install dir>" [guns]
//! ```
//!
//! `guns` adds one line per gun for a VZ soldier (`Accuracy` 0.4). Prints
//! numbers and template names only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::Install;
use m2_engine::weapons::SoldierSpread;
use m2_math::glam::Vec3;

/// A hit: the round passes this close to where he aimed (the alpha's).
const HIT: f32 = 0.45;
const SHOTS: u32 = 20_000;
const RANGES: [f32; 3] = [10.0, 20.0, 40.0];

/// The alpha's random number in [-0.5, 0.5) (`Combat::random`'s xorshift).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32 - 0.5
    }
}

/// The old rule (rounds 1-14): a spread growing with range, scaled by
/// what he lacks of a perfect `Accuracy`, added along the world's x, y
/// and z to his aim (sideways twice as wide as up), whatever the gun.
fn old_hits(distance: f32, accuracy: f32, rng: &mut Rng) -> f32 {
    let s = (0.02 + distance * 0.004) * ((1.0 - accuracy) / 0.6).clamp(0.2, 2.0);
    let mut hits = 0;
    for k in 0..SHOTS {
        // Any heading: the old offsets were in world axes.
        let a = k as f32 / SHOTS as f32 * std::f32::consts::TAU;
        let aim = Vec3::new(a.sin(), 0.0, a.cos());
        let dir = (aim + Vec3::new(rng.next() * s * 2.0, rng.next() * s, rng.next() * s * 2.0))
            .normalize();
        hits += u32::from(miss_by(aim * distance, dir) < HIT);
    }
    hits as f32 / SHOTS as f32
}

fn new_hits(spread: SoldierSpread, distance: f32, rng: &mut Rng) -> f32 {
    let aim = Vec3::Z;
    let mut hits = 0;
    for _ in 0..SHOTS {
        let dir = spread.stray(aim, rng.next(), rng.next());
        hits += u32::from(miss_by(aim * distance, dir) < HIT);
    }
    hits as f32 / SHOTS as f32
}

/// How far from `at` a round from the origin along `dir` passes.
fn miss_by(at: Vec3, dir: Vec3) -> f32 {
    let closest = dir * at.dot(dir).max(0.0);
    closest.distance(at)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: soldierspread <install dir> [guns]");
        std::process::exit(2);
    };
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let db_hash = {
        let index = source.archive().index();
        index
            .assets()
            .iter()
            .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
            .map(|a| a.name_hash)
            .ok_or("no template database")?
    };
    let data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&data)?;
    let arms = m2_engine::weapons::read(&db);
    let guns = m2_engine::tuning::hand_guns(&db);
    let equipment = m2_engine::tuning::equipment(&db);
    let ai = m2_engine::ai::read(&db);
    let rifle = arms
        .weapons
        .values()
        .find(|w| w.name == "Assault Rifle")
        .ok_or("no Assault Rifle")?;
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);

    // The anchor: a VZ soldier with the Assault Rifle at 20 m hits as
    // often as under the old rule.
    let want = old_hits(20.0, SoldierSpread::VZ_ACCURACY, &mut rng);
    let got = new_hits(
        SoldierSpread::new(20.0, SoldierSpread::VZ_ACCURACY, rifle.feel.scatter[0]),
        20.0,
        &mut rng,
    );
    // The scale that makes it so (what [`SoldierSpread::MISS`] holds).
    let s20 = (0.02 + 20.0 * 0.004) * 1.0;
    let cone = rifle.feel.scatter[0].to_radians();
    let (mut lo, mut hi) = (0.0f32, 2.0f32);
    for _ in 0..30 {
        let k = 0.5 * (lo + hi);
        let spread = SoldierSpread {
            side: cone + 2.0 * k * s20,
            up: cone + k * s20,
        };
        if new_hits(spread, 20.0, &mut Rng(7)) > want {
            lo = k;
        } else {
            hi = k;
        }
    }
    println!(
        "fitted miss scale {:.3} (SoldierSpread::MISS {})",
        0.5 * (lo + hi),
        SoldierSpread::MISS
    );
    println!(
        "anchor: VZ soldier, Assault Rifle (LowSkillScatter {}), 20 m: old {want:.3}, new {got:.3}",
        rifle.feel.scatter[0]
    );

    // Every person template with an `Accuracy`, with his gun.
    let mut by_gun: BTreeMap<String, (u32, f32)> = BTreeMap::new();
    let (mut n, mut old_sum, mut new_sum) = (0u32, [0.0f32; 3], [0.0f32; 3]);
    let mut ids: Vec<_> = ai.keys().copied().collect();
    ids.sort_unstable();
    for id in ids {
        let accuracy = ai[&id].accuracy;
        let own = equipment
            .get(&id)
            .and_then(|l| l.iter().find_map(|t| guns.get(t)))
            .and_then(|g| arms.weapons.get(&g.template));
        let gun = own.unwrap_or(rifle);
        let low = gun.feel.scatter[0];
        let name = own.map_or_else(|| "(none: Assault Rifle)".to_owned(), |g| g.name.clone());
        let e = by_gun.entry(name).or_insert((0, low));
        e.0 += 1;
        n += 1;
        for (k, &d) in RANGES.iter().enumerate() {
            old_sum[k] += old_hits(d, accuracy, &mut rng);
            new_sum[k] += new_hits(SoldierSpread::new(d, accuracy, low), d, &mut rng);
        }
    }
    let nf = n.max(1) as f32;
    println!("{n} person templates with an Accuracy; guns they hold (templates, LowSkillScatter):");
    for (name, (count, low)) in &by_gun {
        println!("  {name}\t{count}\t{low}");
    }
    for (k, d) in RANGES.iter().enumerate() {
        println!(
            "mean hit share at {d} m: old {:.3}, new {:.3}",
            old_sum[k] / nf,
            new_sum[k] / nf
        );
    }

    if args.iter().any(|a| a == "guns") {
        let mut all: Vec<_> = arms
            .weapons
            .values()
            .filter(|w| w.model.is_some() && guns.contains_key(&w.template))
            .collect();
        all.sort_by(|a, b| a.name.cmp(&b.name));
        println!("gun\tLowSkillScatter\told 10/20/40 m\tnew 10/20/40 m (VZ soldier)");
        for w in all {
            let low = w.feel.scatter[0];
            let acc = SoldierSpread::VZ_ACCURACY;
            let old: Vec<_> = RANGES.iter().map(|&d| old_hits(d, acc, &mut rng)).collect();
            let new: Vec<_> = RANGES
                .iter()
                .map(|&d| new_hits(SoldierSpread::new(d, acc, low), d, &mut rng))
                .collect();
            println!(
                "{}\t{low}\t{:.2} {:.2} {:.2}\t{:.2} {:.2} {:.2}",
                w.name, old[0], old[1], old[2], new[0], new[1], new[2]
            );
        }
    }
    Ok(())
}
