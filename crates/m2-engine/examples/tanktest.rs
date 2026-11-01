//! Drives every tracked vehicle template (`_TankPhysics`) on a level
//! plane, as the alpha's physics carries it, and prints how it goes
//! beside its template's top speed (`MaxSpeed`, km/h): how long it takes
//! to reach 20 km/h and its fastest flat out, its turning rate spinning
//! on the spot and turning at speed, how far it rolls to a stop with the
//! throttle against it, and whether its model has the six track
//! hardpoints the 360 code drives it on (`hp_wheel_fl` .. `hp_wheel_mr`,
//! 0x82900338).
//!
//! ```text
//! cargo run --release -p m2-engine --example tanktest -- "<install dir>"
//! ```
//!
//! Templates sharing a model and physics record drive alike, so each
//! pair is driven once. Prints numbers and template names only.

use std::collections::BTreeMap;
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
/// The six track hardpoints, in the 360 code's order.
const HARDPOINTS: [&str; 6] = [
    "hp_wheel_fl",
    "hp_wheel_fr",
    "hp_wheel_rl",
    "hp_wheel_rr",
    "hp_wheel_ml",
    "hp_wheel_mr",
];

type Driven = (u32, Vec<u8>);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: tanktest <install dir>");
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
    let c = db.component("_TankPhysics").ok_or("no _TankPhysics")?;
    let mut groups: BTreeMap<Driven, Vec<u32>> = BTreeMap::new();
    for (id, r) in c.records().unwrap_or_default() {
        if let Some(&model) = models.get(&id) {
            groups.entry((model, r.to_vec())).or_default().push(id);
        }
    }
    println!(
        "template\tcount\tdata km/h\tkm/h\tto 20 s\tspin deg/s\tturn km/h\tturn deg/s\tstop m\thardpoints"
    );
    for ((model, _), ids) in &groups {
        let mut ids = ids.clone();
        ids.sort_by_key(|id| names.get(id).map(|n| n.to_ascii_lowercase()));
        let id = ids[0];
        let name = names.get(&id).copied().unwrap_or("?");
        let Some((spec, hardpoints)) = source.model(*model).ok().and_then(|d| {
            let m = d.parse().ok()?;
            let entry = CatalogueEntry {
                model: *model,
                kind: "tank".to_owned(),
                label: name.to_owned(),
            };
            let found = HARDPOINTS
                .iter()
                .filter(|h| m2_assets::vehicle::marker(&m, name_hash(h)).is_some())
                .count();
            if let Some(h) = m2_engine::vehicle::track_hardpoints(&m) {
                let lo = m.info.bounds.min[1];
                eprintln!(
                    "{name}: hull floor {lo:.2} hardpoints {:?}",
                    h.map(|p| (p.x * 100.0).round() / 100.0)
                );
                eprintln!(
                    "  y {:?} z {:?}",
                    h.map(|p| (p.y * 100.0).round() / 100.0),
                    h.map(|p| (p.z * 100.0).round() / 100.0)
                );
            }
            Some((VehicleSpec::from_model(&entry, &m)?, found))
        }) else {
            continue;
        };
        let spec = spec.tuned(tuning.get(&id).copied());
        if !spec.tracked {
            continue;
        }
        let top = spec.tuning.max_speed.unwrap_or(f32::NAN) * 3.6;
        let (fastest, to20) = ahead(&spec);
        let spin = spin(&spec);
        let (turn_speed, turn) = corner(&spec);
        let stop = stop(&spec);
        println!(
            "{name}\t{}\t{top:.0}\t{:.1}\t{to20:.2}\t{spin:.1}\t{:.1}\t{turn:.1}\t{stop:.1}\t{hardpoints}",
            ids.len(),
            fastest * 3.6,
            turn_speed * 3.6,
        );
    }
    Ok(())
}

/// A level plane and the vehicle settled on it, facing +z.
fn start(spec: &VehicleSpec) -> (Physics, VehicleId) {
    let mut p = Physics::new();
    let half = 4000.0;
    let v = [
        Vec3::new(-half, 0.0, -half),
        Vec3::new(half, 0.0, -half),
        Vec3::new(half, 0.0, half),
        Vec3::new(-half, 0.0, half),
    ];
    let shape = CollisionShape::mesh(&v, &[[0, 1, 2], [0, 2, 3]]).expect("plane");
    p.add_static(StaticKey::Cell(1), &shape, Vec3::ZERO, Quat::IDENTITY);
    p.step(DT);
    let rest = spec
        .rest_on(Vec3::new(0.0, 0.0, -3500.0), 0.0, |_| Some(0.0))
        .expect("rests on a plane");
    let id = p.add_vehicle_turned(spec, rest.position + Vec3::Y * 0.2, rest.rotation);
    for _ in 0..(2.0 / DT) as usize {
        p.drive(id, controls(0.0, 0.0, true));
        p.step(DT);
    }
    (p, id)
}

fn controls(throttle: f32, steer: f32, parked: bool) -> VehicleControls {
    VehicleControls {
        throttle,
        steer,
        parked,
        ..VehicleControls::default()
    }
}

/// Which way it heads, radians about +y.
fn heading(p: &Physics, id: VehicleId) -> f32 {
    let ahead = p
        .vehicle_transform(id)
        .map_or(Vec3::Z, |m| m.z_axis.truncate());
    ahead.x.atan2(ahead.z)
}

/// Drives `c` for `seconds`, calling `each` after every step.
fn ride(
    p: &mut Physics,
    id: VehicleId,
    seconds: f32,
    c: VehicleControls,
    mut each: impl FnMut(&Physics, f32),
) {
    for i in 0..(seconds / DT) as usize {
        p.drive(id, c);
        p.step(DT);
        each(p, (i + 1) as f32 * DT);
    }
}

/// Mean turning rate over the last half of `seconds` of `c`, deg/s, and
/// its mean speed then, m/s.
fn turning(p: &mut Physics, id: VehicleId, seconds: f32, c: VehicleControls) -> (f32, f32) {
    let mut last = heading(p, id);
    let (mut n, mut turned, mut speed) = (0.0, 0.0, 0.0);
    ride(p, id, seconds, c, |p, t| {
        let h = heading(p, id);
        let d = (h - last + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        last = h;
        if t > seconds * 0.5 {
            n += 1.0;
            turned += d.abs();
            speed += p.vehicle_speed(id);
        }
    });
    let n = f32::max(n, 1.0);
    ((turned / n / DT).to_degrees(), speed / n)
}

/// Flat out 30 s: its fastest, m/s, and when it first reached 20 km/h.
fn ahead(spec: &VehicleSpec) -> (f32, f32) {
    let (mut p, id) = start(spec);
    let (mut fastest, mut at) = (0.0f32, f32::NAN);
    ride(&mut p, id, 30.0, controls(1.0, 0.0, false), |p, t| {
        let v = p.vehicle_speed(id);
        fastest = fastest.max(v);
        if at.is_nan() && v * 3.6 >= 20.0 {
            at = t;
        }
    });
    (fastest, at)
}

/// Full lock with no throttle 6 s: its turning rate, deg/s.
fn spin(spec: &VehicleSpec) -> f32 {
    let (mut p, id) = start(spec);
    turning(&mut p, id, 6.0, controls(0.0, 1.0, false)).0
}

/// 10 s flat out, then full lock with the throttle open 6 s: its speed,
/// m/s, and turning rate, deg/s.
fn corner(spec: &VehicleSpec) -> (f32, f32) {
    let (mut p, id) = start(spec);
    ride(&mut p, id, 10.0, controls(1.0, 0.0, false), |_, _| {});
    let (rate, speed) = turning(&mut p, id, 6.0, controls(1.0, 1.0, false));
    (speed, rate)
}

/// 15 s flat out, then the throttle against the motion: how far it goes
/// before it stops, metres.
fn stop(spec: &VehicleSpec) -> f32 {
    let (mut p, id) = start(spec);
    ride(&mut p, id, 15.0, controls(1.0, 0.0, false), |_, _| {});
    let from = p
        .vehicle_transform(id)
        .map_or(Vec3::ZERO, |m| m.w_axis.truncate());
    let mut went = 0.0;
    let mut stopped = false;
    ride(&mut p, id, 15.0, controls(-1.0, 0.0, false), |p, _| {
        if !stopped {
            let at = p
                .vehicle_transform(id)
                .map_or(Vec3::ZERO, |m| m.w_axis.truncate());
            went = (at - from).with_y(0.0).length();
            stopped = p.vehicle_speed(id) < 0.3;
        }
    });
    went
}
