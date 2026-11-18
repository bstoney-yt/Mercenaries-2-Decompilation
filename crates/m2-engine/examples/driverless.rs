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
//! gearbox model). The helicopter (an MD-500): lifted to 50 m,
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
            let cleared = coast(&spec, VehicleControls::default());
            let parked = coast(
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
            eprintln!(
                "driverless: INFO {name}: its tyres' rolling friction alone is {rolling:.2} m/s²; the rest of the {first:.2} is the alpha physics' own no-throttle drag (not traced here)"
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
/// the let-go and each second after.
fn coast(spec: &VehicleSpec, after: VehicleControls) -> Vec<f32> {
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
    for _ in 0..10 {
        for _ in 0..60 {
            p.drive(id, after);
            p.step(DT);
        }
        out.push(p.vehicle_speed(id));
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
