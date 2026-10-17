//! Measures how often her rounds hit a soldier at range (playtest 8's
//! harness): for every gun she can hold (a hand gun template with a
//! model and a `WeaponScatter`), the share of rounds aimed at a standing
//! soldier's chest that pass within the alpha's body of him (0.3 m of the
//! line from 0.25 m to 1.7 m up) at 25, 50, 100 and 150 m, with her cone
//! at rest (`ScatterMin`) and at its most (`ScatterMax`, held), under the
//! old stray (sideways and up apart, in a square as wide as the cone) and
//! [`m2_engine::weapons::stray_in_cone`]. Pellet guns count each pellet.
//!
//! ```text
//! cargo run --release -p m2-engine --example playerspread -- "<install dir>" [guns]
//! ```
//!
//! `guns` adds one line per gun. Prints numbers and template names only.

use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::Install;
use m2_engine::weapons::stray_in_cone;
use m2_math::glam::Vec3;

const SHOTS: u32 = 20_000;
const RANGES: [f32; 4] = [25.0, 50.0, 100.0, 150.0];
/// The alpha's body (`combat::BODY_RADIUS` and its capsule).
const BODY_RADIUS: f32 = 0.3;
const SHINS: f32 = 0.25;
const HEAD: f32 = 1.7;
/// Where she aims on him.
const CHEST: f32 = 1.2;

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

/// The alpha's stray before playtest 8 (`Combat::player_shoot`).
fn old_stray(aim: Vec3, cone: f32, a: f32, b: f32) -> Vec3 {
    let spread = cone.to_radians();
    if spread <= 0.0 {
        return aim;
    }
    let side = aim.cross(Vec3::Y).normalize_or(Vec3::X);
    let up = side.cross(aim).normalize_or(Vec3::Y);
    (aim + side * (a * spread) + up * (b * spread)).normalize_or(aim)
}

/// Whether a round from the origin along `dir` passes through a soldier
/// standing at `distance` straight ahead (his feet at -CHEST).
fn hits(distance: f32, dir: Vec3) -> bool {
    // Where it crosses his plane, as near enough for a thin body.
    if dir.z <= 0.0 {
        return false;
    }
    let at = dir * (distance / dir.z);
    let height = at.y + CHEST;
    let along = height.clamp(SHINS, HEAD);
    Vec3::new(at.x, height - along, 0.0).length() <= BODY_RADIUS
}

fn share(stray: fn(Vec3, f32, f32, f32) -> Vec3, cone: f32, d: f32, rng: &mut Rng) -> f32 {
    let aim = Vec3::Z;
    let n = (0..SHOTS)
        .filter(|_| hits(d, stray(aim, cone, rng.next(), rng.next())))
        .count();
    n as f32 / SHOTS as f32
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: playerspread <install dir> [guns]");
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
    let hand = m2_engine::tuning::hand_guns(&db);
    let mut guns: Vec<_> = arms
        .weapons
        .values()
        .filter(|w| w.model.is_some() && hand.contains_key(&w.template))
        .filter(|w| w.feel.scatter_max() > 0.0)
        .collect();
    guns.sort_by(|a, b| a.name.cmp(&b.name));
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let each = args.iter().any(|a| a == "guns");
    if each {
        println!("gun\tmin\tmax\told rest 25/50/100/150\tnew rest\told held\tnew held");
    }
    // Sums: [rest, held] x [old, new] x range.
    let mut sum = [[[0.0f32; 4]; 2]; 2];
    for w in &guns {
        let (lo, hi) = (w.feel.scatter_min(), w.feel.scatter_max());
        let mut row = [[[0.0f32; 4]; 2]; 2];
        for (c, cone) in [lo, hi].into_iter().enumerate() {
            for (k, &d) in RANGES.iter().enumerate() {
                row[c][0][k] = share(old_stray, cone, d, &mut rng);
                row[c][1][k] = share(stray_in_cone, cone, d, &mut rng);
                sum[c][0][k] += row[c][0][k];
                sum[c][1][k] += row[c][1][k];
            }
        }
        if each {
            let f = |v: [f32; 4]| format!("{:.2} {:.2} {:.2} {:.2}", v[0], v[1], v[2], v[3]);
            println!(
                "{}\t{lo}\t{hi}\t{}\t{}\t{}\t{}",
                w.name,
                f(row[0][0]),
                f(row[0][1]),
                f(row[1][0]),
                f(row[1][1])
            );
        }
    }
    let n = guns.len().max(1) as f32;
    println!("{} guns she can hold with a cone", guns.len());
    for (c, what) in ["at rest (ScatterMin)", "held (ScatterMax)"]
        .iter()
        .enumerate()
    {
        for (k, d) in RANGES.iter().enumerate() {
            println!(
                "mean hit share {what} at {d} m: old {:.3}, new {:.3}",
                sum[c][0][k] / n,
                sum[c][1][k] / n
            );
        }
    }
    Ok(())
}
