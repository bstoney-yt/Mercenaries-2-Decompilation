//! `driverless`: a car and a helicopter after their driver gets out, as
//! the alpha's physics carries them with the inputs the 360 leaves them
//! (all cleared: message 0x6C5F1491, see `m2-alpha`'s
//! `driverless_controls`), beside the alpha's former rule (`parked`).
//!
//! ```text
//! cargo run --release -p m2-engine --example driverless -- "<install dir>"
//! ```
//!
//! The car (an M151): driven flat out on a level plane to 20 m/s, then let
//! go with every input 0 for 10 s, its speed logged each second. The 360
//! then applies no brake and no handbrake: it coasts, slowed by what its
//! physics does with no throttle (its tyres' rolling friction,
//! [`m2_engine::tuning::Tyres::slowing`], and the alpha's engine and
//! gearbox model), checked against the 360 code's coast worked out as a
//! point mass (`expected_coast`): from 1 s to 2 s it slows within 0.03
//! m/s of it, after 10 s it is within 0.2 m/s of it, and it stays in 5th
//! (its engine turning at twice its wheels' rpm over the template's wheel
//! radius). A `coast t ...` line every half second logs its gear, rpm,
//! slowing, engine drive, footing and wheels down. The helicopter (an MD-500): lifted to 50 m,
//! then every input 0 and its lift 0 for 10 s, its height logged.
//!
//! Prints `driverless: PASS ...` or `FAIL ...` lines, `driverless: INFO
//! ...` lines for what the 360 numbers don't settle, and `driverless:
//! RESULT PASS` (or `FAIL`); exits 1 on a failure. Offscreen: no window,
//! no world but a plane; it reads only the game's templates and models.

use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::name_hash;
use m2_engine::tuning::{DAMAGE_TABLE, DAMAGE_TABLE_TYPE, DamageTable};
use m2_engine::{
    CatalogueEntry, CollisionShape, Install, Physics, StaticKey, VehicleControls, VehicleId,
    VehicleSpec,
};
use m2_math::glam::{Quat, Vec3};

const DT: f32 = 1.0 / 60.0;
/// The speed the car is let go at, m/s (about 72 km/h).
const LET_GO: f32 = 20.0;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: driverless <install dir>");
        std::process::exit(2);
    };
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
        .asset(name_hash(DAMAGE_TABLE), DAMAGE_TABLE_TYPE)
        .ok()
        .and_then(|d| DamageTable::parse(&d));
    let tuning = m2_engine::tuning::read(&db, damage.as_ref());
    let models = db.hashes("ModelName");
    let mut spec_of = |component: &str, part: &str, kind: &str| -> Option<(String, VehicleSpec)> {
        let c = db.component(component)?;
        let mut ids: Vec<u32> = c
            .records()
            .unwrap_or_default()
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| names.get(id).is_some_and(|n| n.contains(part)))
            .collect();
        ids.sort_unstable();
        ids.into_iter().find_map(|id| {
            let name = names.get(&id)?.to_string();
            let model = *models.get(&id)?;
            let d = source.model(model).ok()?;
            let m = d.parse().ok()?;
            let entry = CatalogueEntry {
                model,
                kind: kind.to_owned(),
                label: name.clone(),
            };
            let spec = VehicleSpec::from_model(&entry, &m)?
                .with_chassis(&m, &d.base)
                .tuned(tuning.get(&id).copied());
            Some((name, spec))
        })
    };
    let mut failed = 0;
    let mut line = |pass: bool, what: String| {
        if !pass {
            failed += 1;
        }
        eprintln!("driverless: {} {what}", if pass { "PASS" } else { "FAIL" });
    };

    match spec_of("_CarPhysicsV2", "M151", "car") {
        Some((name, spec)) => {
            let tyres = spec
                .tuning
                .tyres
                .unwrap_or(m2_engine::tuning::Tyres::DEFAULT);
            let rolling = (tyres.rolling.0 + tyres.rolling.1) / 2.0;
            let (cleared, gears) = coast(&spec, VehicleControls::default());
            let (parked, _) = coast(
                &spec,
                VehicleControls {
                    parked: true,
                    ..VehicleControls::default()
                },
            );
            eprintln!(
                "driverless: INFO {name}: speeds each second, inputs cleared: {}",
                list(&cleared)
            );
            eprintln!(
                "driverless: INFO {name}: speeds each second, the former parked rule: {}",
                list(&parked)
            );
            let first = cleared[0] - cleared[1];
            let brake = (tyres.brake.0 + tyres.brake.1) / 2.0;
            // No brake or handbrake: well under its brake's slowing.
            line(
                first < brake * 0.5,
                format!(
                    "{name}: let go at {:.1} m/s with every input 0, it slows {first:.2} m/s in the first second, under half its brake's {brake:.2} m/s² (360: no brake, no handbrake)",
                    cleared[0]
                ),
            );
            // The 360's coast, worked out as a point mass from the code
            // (0x821A0528's drag at throttle 0 through 0x8219CAE8, the
            // engine's speed by 0x8219E678 over the template radius,
            // each axle's rolling push 0x8219FB08): `expected_coast`.
            let want = expected_coast(&spec);
            eprintln!(
                "driverless: INFO {name}: the 360 code's coast as a point mass: {} (its tyres' rolling coefficient {rolling:.2} each axle)",
                list(&want)
            );
            let slowing = |v: &[f32]| v[1] - v[2];
            line(
                (slowing(&cleared) - slowing(&want)).abs() < 0.03,
                format!(
                    "{name}: from 1 s to 2 s it slows {:.3} m/s, the 360's {:.3} (within 0.03)",
                    slowing(&cleared),
                    slowing(&want)
                ),
            );
            line(
                (cleared[10] - want[10]).abs() < 0.2,
                format!(
                    "{name}: after 10 s it is at {:.2} m/s, the 360's {:.2} (within 0.2)",
                    cleared[10], want[10]
                ),
            );
            line(
                gears.iter().all(|&g| g == 5),
                format!(
                    "{name}: it coasts in 5th throughout, as the 360's engine speed (twice its wheels' rpm) holds it: {gears:?}"
                ),
            );
            line(
                cleared[1] > parked[1] + 1.0,
                format!(
                    "{name}: it coasts, not brakes: {:.1} m/s after 1 s against the former rule's {:.1}",
                    cleared[1], parked[1]
                ),
            );
            line(
                cleared.windows(2).all(|w| w[1] <= w[0] + 0.05),
                format!("{name}: its speed only falls over 10 s (no throttle, no push)"),
            );
        }
        None => line(false, "no M151 car template".to_owned()),
    }

    match spec_of(
        "_HelicopterPhysics",
        "MD-500",
        m2_engine::vehicle::HELICOPTER,
    ) {
        Some((name, spec)) => {
            let heights = let_go_aloft(&spec);
            eprintln!(
                "driverless: INFO {name}: height each second after its lift and inputs are cleared at about 50 m: {}",
                list(&heights)
            );
            line(
                heights.iter().all(|h| h.is_finite()),
                format!("{name}: flies on with every input 0 (lift 0), height logged"),
            );
        }
        None => line(false, "no MD-500 helicopter template".to_owned()),
    }

    eprintln!(
        "driverless: RESULT {}",
        if failed == 0 { "PASS" } else { "FAIL" }
    );
    std::process::exit(if failed == 0 { 0 } else { 1 });
}

fn list(v: &[f32]) -> String {
    v.iter()
        .map(|x| format!("{x:.1}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A level plane, and `spec` settled on it facing +z.
fn start(spec: &VehicleSpec, lift: f32) -> (Physics, VehicleId) {
    let mut p = Physics::new();
    let half = 8000.0;
    let v = [
        Vec3::new(-half, 0.0, -half),
        Vec3::new(half, 0.0, -half),
        Vec3::new(half, 0.0, half),
        Vec3::new(-half, 0.0, half),
    ];
    let shape = CollisionShape::mesh(&v, &[[0, 1, 2], [0, 2, 3]]).expect("plane");
    p.add_static(StaticKey::Cell(1), &shape, Vec3::ZERO, Quat::IDENTITY);
    p.step(DT);
    let floor = spec.hull.0.y;
    let id = p.add_vehicle_turned(
        spec,
        Vec3::new(0.0, 0.2 - floor + lift, -7000.0),
        Quat::IDENTITY,
    );
    for _ in 0..(2.0 / DT) as usize {
        p.drive(id, VehicleControls::default());
        p.step(DT);
    }
    (p, id)
}

/// Driven flat out to [`LET_GO`] m/s, then `after` for 10 s: its speed at
/// the let-go and each second after, and its gear each second.
fn coast(spec: &VehicleSpec, after: VehicleControls) -> (Vec<f32>, Vec<usize>) {
    let (mut p, id) = start(spec, 0.0);
    let mut t = 0.0;
    while p.vehicle_speed(id) < LET_GO && t < 30.0 {
        p.drive(
            id,
            VehicleControls {
                throttle: 1.0,
                ..VehicleControls::default()
            },
        );
        p.step(DT);
        t += DT;
    }
    let mut out = vec![p.vehicle_speed(id)];
    let mut gears = Vec::new();
    for sec in 0..10 {
        for k in 0..60 {
            p.drive(id, after);
            let before = p.vehicle_speed(id);
            p.step(DT);
            // The coast's working, each half second: gear, engine rpm, the
            // slowing this tick and what the engine's drive force is.
            if k % 30 == 0 {
                let (gear, rpm) = p.vehicle_gear(id).unwrap_or((9, 0.0));
                let (drive, footing, down) = p.vehicle_drive_force(id);
                eprintln!(
                    "coast t {:.1}: {:.2} m/s, gear {gear}, {rpm:.0} rpm, slowing {:.3} m/s2, engine drive {drive:.0} N, footing {footing:.2}, {down} wheels down",
                    sec as f32 + k as f32 / 60.0,
                    p.vehicle_speed(id),
                    (before - p.vehicle_speed(id)) / DT,
                );
            }
        }
        out.push(p.vehicle_speed(id));
        gears.push(p.vehicle_gear(id).map_or(9, |(g, _)| g));
    }
    (out, gears)
}

/// The 360 code's coast from [`LET_GO`] m/s on the flat with every input
/// 0, as a point mass of the template's `Mass`: the engine's drag at
/// throttle 0 (0x821A0528, [`m2_engine::tuning::EngineTuning::step`]) in
/// its gear, through the drive (0x8219CAE8: twice the drive over the
/// rear radius, the shares summing to 2), the engine turning at twice
/// its wheels' rpm (0x8219E678, shares summing to 2, over the template
/// radius), with each axle's rolling push, mass x coefficient x speed
/// up to 1 m/s, 3x at a standstill easing to 1x at `LowSpeedLimit`
/// ([`m2_engine::tuning::Tyres::slowing`]). Its speed each second.
fn expected_coast(spec: &VehicleSpec) -> Vec<f32> {
    let e = spec
        .tuning
        .engine
        .unwrap_or(m2_engine::tuning::EngineTuning::DEFAULT);
    let tyres = spec
        .tuning
        .tyres
        .unwrap_or(m2_engine::tuning::Tyres::DEFAULT);
    let mass = spec.mass;
    let mut gb = m2_engine::tuning::Gearbox {
        gear: 5,
        ..Default::default()
    };
    let (mut v, dt) = (LET_GO, 1.0 / 600.0);
    let mut out = vec![v];
    for i in 1..=6000 {
        let rpm = 2.0 * v / e.radius * 60.0 / std::f32::consts::TAU;
        let torque = e.step(&mut gb, dt, 0.0, rpm, false);
        let drive = e.ratio(gb.gear).abs() * torque;
        let engine = 2.0 * drive / e.radius;
        let rolling: f32 = [true, false]
            .into_iter()
            .map(|front| tyres.slowing(front, v, 0.0, 0.0, 0.0, 0.0) * mass * v.min(1.0))
            .sum();
        v -= (rolling - engine) / mass * dt;
        if i % 600 == 0 {
            out.push(v);
        }
    }
    out
}

/// Climbed to about 50 m, then every input 0 and lift 0 for 10 s: its
/// height at the let-go and each second after.
fn let_go_aloft(spec: &VehicleSpec) -> Vec<f32> {
    let (mut p, id) = start(spec, 0.0);
    let height = |p: &Physics| p.vehicle_transform(id).map_or(f32::NAN, |m| m.w_axis.y);
    let mut t = 0.0;
    while height(&p) < 50.0 && t < 30.0 {
        p.drive(id, VehicleControls::default());
        p.fly(id, 1.0);
        p.step(DT);
        t += DT;
    }
    let mut out = vec![height(&p)];
    for _ in 0..10 {
        for _ in 0..60 {
            p.drive(id, VehicleControls::default());
            p.fly(id, 0.0);
            p.step(DT);
        }
        out.push(height(&p));
    }
    out
}
