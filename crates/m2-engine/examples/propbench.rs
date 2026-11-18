//! Times the physics step with a town's worth of props as fixed collision
//! (the port today) against the same props as sleeping loose bodies (as
//! the 360 keeps them, gap P7), headless, on a flat ground:
//!
//! ```text
//! cargo run --release -p m2-engine --example propbench -- "<install dir>" [props]
//! ```
//!
//! 3,000 crates (1 m, 50 kg) by default, on a 600 m square, with a row of
//! 40 across the car's way. Each mode runs three phases: still (5 s), the
//! Phoenix driven at 15 m/s through the row (6 s), and a 20 m blast in
//! the thick of them (10 s; the loose ones get the 360's rigid push as
//! [`m2_engine::durability::BlastPush::throw`] sums it, the fixed
//! ones nothing). Prints, per mode and phase, the step's median and worst
//! milliseconds, the most bodies awake at once and, for the blast, how
//! long until every crate sleeps again. Prints numbers only.

use std::path::Path;
use std::time::Instant;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::durability::BlastPush;
use m2_engine::physics::LoosePiece;
use m2_engine::tuning::{DAMAGE_TABLE, DAMAGE_TABLE_TYPE, DamageTable};
use m2_engine::{
    CatalogueEntry, CollisionShape, Install, Physics, StaticKey, VehicleControls, VehicleSpec,
};
use m2_math::glam::{Mat4, Quat, Vec3};

const DT: f32 = 1.0 / 60.0;
const HALF: f32 = 0.5;
const MASS: f32 = 50.0;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = args
        .first()
        .ok_or("usage: propbench <install dir> [props]")?;
    let count: usize = args.get(1).map_or(Ok(3000), |s| s.parse())?;
    let spec = phoenix(root)?;
    for loose in [false, true] {
        run(&spec, count, loose);
    }
    Ok(())
}

/// The Phoenix's spec, as the drive test builds it.
fn phoenix(root: &str) -> Result<VehicleSpec, Box<dyn std::error::Error>> {
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let index = source.archive().index();
    let db_hash = index
        .assets()
        .iter()
        .find(|a| index.type_hash(a) == TEMPLATE_DB_TYPE)
        .map(|a| a.name_hash)
        .ok_or("no template database")?;
    let data = source.asset(db_hash, TEMPLATE_DB_TYPE)?;
    let db = Layer::parse(&data)?;
    let names = db.names();
    let damage = source
        .asset(m2_assets::name_hash(DAMAGE_TABLE), DAMAGE_TABLE_TYPE)
        .ok()
        .and_then(|d| DamageTable::parse(&d));
    let tuning = m2_engine::tuning::read(&db, damage.as_ref());
    let models = db.hashes("ModelName");
    let id = names
        .iter()
        .find(|(_, n)| **n == "Phoenix")
        .map(|(i, _)| *i)
        .ok_or("no Phoenix")?;
    let model = *models.get(&id).ok_or("no model")?;
    let data = source.model(model)?;
    let m = data.parse()?;
    let entry = CatalogueEntry {
        model,
        kind: "car".to_owned(),
        label: "Phoenix".to_owned(),
    };
    let spec = VehicleSpec::from_model(&entry, &m)
        .ok_or("no spec")?
        .with_chassis(&m, &data.base)
        .tuned(tuning.get(&id).copied());
    Ok(spec)
}

/// Where the crates stand: a grid over a 600 m square, and a row of 40
/// across the car's way (x 0, z 30 to 70).
fn places(count: usize) -> Vec<Vec3> {
    let row = 40.min(count);
    let mut out: Vec<Vec3> = (0..row)
        .map(|k| Vec3::new(-0.6 + (k % 2) as f32 * 1.2, HALF, 30.0 + k as f32))
        .collect();
    let rest = count - row;
    let side = (rest as f32).sqrt().ceil() as usize;
    let step = 600.0 / side.max(1) as f32;
    for k in 0..rest {
        let (i, j) = (k % side, k / side);
        let p = Vec3::new(-300.0 + i as f32 * step, HALF, -300.0 + j as f32 * step);
        // Keep the car's lane clear but for the row.
        if p.x.abs() < 4.0 && p.z > -10.0 && p.z < 120.0 {
            continue;
        }
        out.push(p);
    }
    out
}

fn run(spec: &VehicleSpec, count: usize, loose: bool) {
    let mut p = Physics::new();
    // Every crate a body of its own (the port holds all but the newest
    // [`m2_engine::physics`]'s 160 loose pieces fixed).
    p.set_debris_cap(usize::MAX);
    let half = 1000.0;
    let v = [
        Vec3::new(-half, 0.0, -half),
        Vec3::new(half, 0.0, -half),
        Vec3::new(half, 0.0, half),
        Vec3::new(-half, 0.0, half),
    ];
    let ground = CollisionShape::mesh(&v, &[[0, 1, 2], [0, 2, 3]]).expect("plane");
    p.add_static(StaticKey::Cell(1), &ground, Vec3::ZERO, Quat::IDENTITY);
    let at = places(count);
    let crate_shape = CollisionShape::cuboid(Vec3::splat(HALF));
    let piece = LoosePiece::block(None, Vec3::ZERO, Vec3::splat(HALF), MASS);
    let mut pieces = Vec::new();
    for (k, c) in at.iter().enumerate() {
        if loose {
            pieces.push(p.add_prop_body(&piece, Mat4::from_translation(*c)));
        } else {
            p.add_static(
                StaticKey::Entity(k as u32 + 1),
                &crate_shape,
                *c,
                Quat::IDENTITY,
            );
        }
    }
    // Settled and asleep before the clock starts.
    for _ in 0..(4.0 / DT) as usize {
        p.step(DT);
    }
    let rest = spec
        .rest_on(Vec3::new(0.0, 0.0, -20.0), 0.0, |_| Some(0.0))
        .expect("rests on a plane");
    let car = p.add_vehicle_turned(spec, rest.position + Vec3::Y * 0.2, rest.rotation);
    let mode = if loose { "loose" } else { "fixed" };
    let phase = |p: &mut Physics, name: &str, seconds: f32, drive: bool| {
        let mut ms = Vec::new();
        let mut most = 0;
        let mut asleep_at = None;
        for k in 0..(seconds / DT) as usize {
            let speed = p.vehicle_speed(car);
            p.drive(
                car,
                VehicleControls {
                    throttle: if drive && speed < 15.0 { 1.0 } else { 0.0 },
                    steer: 0.0,
                    brake: !drive,
                    parked: !drive,
                },
            );
            let t = Instant::now();
            p.step(DT);
            ms.push(t.elapsed().as_secs_f64() * 1000.0);
            let awake = p.debris_awake();
            most = most.max(awake);
            if awake == 0 && asleep_at.is_none() && most > 0 {
                asleep_at = Some(k as f32 * DT);
            }
            if awake > 0 {
                asleep_at = None;
            }
        }
        ms.sort_by(f64::total_cmp);
        let car_at = p
            .vehicle_transform(car)
            .map_or(Vec3::ZERO, |m| m.w_axis.truncate());
        println!(
            "{mode} {name}: car at z {:.1}, {:.1} m/s; step median {:.2} ms, worst {:.2} ms, most awake {most}{}",
            car_at.z,
            p.vehicle_speed(car),
            ms[ms.len() / 2],
            ms[ms.len() - 1],
            match (loose, asleep_at) {
                (true, Some(t)) => format!(", all asleep again {t:.1} s in"),
                (true, None) if most > 0 => ", still awake at the end".to_owned(),
                _ => String::new(),
            }
        );
    };
    phase(&mut p, "still", 5.0, false);
    phase(&mut p, "car through the row", 6.0, true);
    // A 20 m blast among the grid's crates (300 kg-m/s-ish: `fx_Explosion
    // _Large`'s push 300), a frame's push on each crate within it.
    let blast = BlastPush {
        at: Vec3::new(100.0, 0.5, 100.0),
        radius: 20.0,
        force: 300.0,
    };
    let mut reached = 0;
    if loose {
        for (k, c) in at.iter().enumerate() {
            if c.distance(blast.at) <= blast.radius {
                let lo = *c - Vec3::splat(HALF);
                let hi = *c + Vec3::splat(HALF);
                let v = blast.throw(lo, hi, 1.0).0;
                p.push_debris(pieces[k], v);
                reached += 1;
            }
        }
    }
    println!("{mode}: the blast reaches {reached} crates");
    phase(&mut p, "after a 20 m blast", 10.0, false);
}
