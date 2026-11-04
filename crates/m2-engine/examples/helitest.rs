//! Flies every helicopter template (`_HelicopterPhysics`) over a level
//! plane, as the alpha's physics carries it, and prints how it goes
//! beside its template's top speeds (`MaxFwdSpeed`, `MaxElevationSpeed`,
//! m/s): how long it takes to lift 2 m off the ground with the climb
//! held, how fast it climbs after 10 s of it, its fastest flat out
//! holding its height and how far its nose dips then, its turning rate
//! hovering and at speed and how far it banks then.
//!
//! ```text
//! cargo run --release -p m2-engine --example helitest -- "<install dir>"
//! ```
//!
//! Templates sharing a model and physics record fly alike, so each pair
//! is flown once. Prints numbers and template names only.

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

type Flown = (u32, Vec<u8>);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: helitest <install dir>");
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
    let c = db
        .component("_HelicopterPhysics")
        .ok_or("no _HelicopterPhysics")?;
    let mut groups: BTreeMap<Flown, Vec<u32>> = BTreeMap::new();
    for (id, r) in c.records().unwrap_or_default() {
        if let Some(&model) = models.get(&id) {
            groups.entry((model, r.to_vec())).or_default().push(id);
        }
    }
    println!(
        "template\tcount\tfwd m/s\tclimb m/s\tlift-off s\tclimb@10s m/s\tfastest m/s\tnose deg\thover turn deg/s\tturn m/s\tturn deg/s\tbank deg\thover drift m\thover sway m/s"
    );
    for ((model, _), ids) in &groups {
        let mut ids = ids.clone();
        ids.sort_by_key(|id| names.get(id).map(|n| n.to_ascii_lowercase()));
        let id = ids[0];
        let name = names.get(&id).copied().unwrap_or("?");
        let Some(spec) = source.model(*model).ok().and_then(|d| {
            let m = d.parse().ok()?;
            let entry = CatalogueEntry {
                model: *model,
                kind: m2_engine::vehicle::HELICOPTER.to_owned(),
                label: name.to_owned(),
            };
            VehicleSpec::from_model(&entry, &m).map(|s| s.with_chassis(&m, &d.base))
        }) else {
            continue;
        };
        let spec = spec.tuned(tuning.get(&id).copied());
        let Some(f) = spec.tuning.flight else {
            println!("{name}\t{}\tno flight tuning", ids.len());
            continue;
        };
        eprintln!("{name}: mass {} {f:?}", spec.mass);
        let (lift_off, climb) = climb(&spec);
        let (fastest, nose) = ahead(&spec);
        let hover = hover_turn(&spec);
        let (turn_speed, turn, bank) = corner(&spec);
        let (drift, sway) = hands_off(&spec);
        println!(
            "{name}\t{}\t{:.0}\t{:.0}\t{lift_off:.2}\t{climb:.1}\t{fastest:.1}\t{nose:.1}\t{hover:.1}\t{turn_speed:.1}\t{turn:.1}\t{bank:.1}\t{drift:.2}\t{sway:.2}",
            ids.len(),
            f.forward,
            f.climb,
        );
    }
    Ok(())
}

/// A level plane and the helicopter settled on it, facing +z.
fn start(spec: &VehicleSpec) -> (Physics, VehicleId) {
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
    let id = p.add_vehicle_turned(spec, Vec3::new(0.0, 0.2 - floor, -7000.0), Quat::IDENTITY);
    ride(&mut p, id, 2.0, controls(0.0, 0.0, true), 0.0, |_, _| {});
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

fn height(p: &Physics, id: VehicleId) -> f32 {
    p.vehicle_transform(id).map_or(f32::NAN, |m| m.w_axis.y)
}

/// Which way it heads, radians about +y.
fn heading(p: &Physics, id: VehicleId) -> f32 {
    let ahead = p
        .vehicle_transform(id)
        .map_or(Vec3::Z, |m| m.z_axis.truncate());
    ahead.x.atan2(ahead.z)
}

/// Flies `c` with the climb at `lift` for `seconds`, calling `each`
/// after every step.
fn ride(
    p: &mut Physics,
    id: VehicleId,
    seconds: f32,
    c: VehicleControls,
    lift: f32,
    mut each: impl FnMut(&Physics, f32),
) {
    for i in 0..(seconds / DT) as usize {
        p.drive(id, c);
        p.fly(id, lift);
        p.step(DT);
        each(p, (i + 1) as f32 * DT);
    }
}

/// The climb held 10 s from the ground: when it first stood 2 m above
/// where it rested, s, and how fast it climbs at the end, m/s.
fn climb(spec: &VehicleSpec) -> (f32, f32) {
    let (mut p, id) = start(spec);
    let from = height(&p, id);
    let mut at = f32::NAN;
    ride(&mut p, id, 10.0, controls(0.0, 0.0, false), 1.0, |p, t| {
        if at.is_nan() && height(p, id) > from + 2.0 {
            at = t;
        }
    });
    (at, p.vehicle_velocity(id).y)
}

/// Up 12 s, then flat out holding its height 25 s: its speed, m/s, and
/// how far its nose dips, degrees.
fn ahead(spec: &VehicleSpec) -> (f32, f32) {
    let (mut p, id) = start(spec);
    ride(&mut p, id, 12.0, controls(0.0, 0.0, false), 1.0, |_, _| {});
    ride(&mut p, id, 25.0, controls(1.0, 0.0, false), 0.0, |_, _| {});
    let nose = p
        .vehicle_transform(id)
        .map_or(f32::NAN, |m| m.z_axis.y.asin().to_degrees());
    (p.vehicle_velocity(id).with_y(0.0).length(), -nose)
}

/// Mean turning rate over the last half of `seconds` of `c`, deg/s, its
/// mean speed then, m/s, and how far it banks at the end, degrees.
fn turning(p: &mut Physics, id: VehicleId, seconds: f32, c: VehicleControls) -> (f32, f32, f32) {
    let mut last = heading(p, id);
    let (mut n, mut turned, mut speed) = (0.0, 0.0, 0.0);
    ride(p, id, seconds, c, 0.0, |p, t| {
        let h = heading(p, id);
        let d = (h - last + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        last = h;
        if t > seconds * 0.5 {
            n += 1.0;
            turned += d.abs();
            speed += p.vehicle_velocity(id).with_y(0.0).length();
        }
    });
    let bank = p
        .vehicle_transform(id)
        .map_or(f32::NAN, |m| m.x_axis.y.asin().to_degrees());
    let n = f32::max(n, 1.0);
    ((turned / n / DT).to_degrees(), speed / n, bank.abs())
}

/// Up 12 s, then full stick to one side hovering 6 s: its turning
/// rate, deg/s.
fn hover_turn(spec: &VehicleSpec) -> f32 {
    let (mut p, id) = start(spec);
    ride(&mut p, id, 12.0, controls(0.0, 0.0, false), 1.0, |_, _| {});
    ride(&mut p, id, 4.0, controls(0.0, 0.0, false), 0.0, |_, _| {});
    turning(&mut p, id, 6.0, controls(0.0, 1.0, false)).0
}

/// Up 12 s, 15 s flat out, then full stick to one side flat out 6 s:
/// its speed, m/s, turning rate, deg/s, and bank, degrees.
fn corner(spec: &VehicleSpec) -> (f32, f32, f32) {
    let (mut p, id) = start(spec);
    ride(&mut p, id, 12.0, controls(0.0, 0.0, false), 1.0, |_, _| {});
    ride(&mut p, id, 15.0, controls(1.0, 0.0, false), 0.0, |_, _| {});
    let (rate, speed, bank) = turning(&mut p, id, 6.0, controls(1.0, 1.0, false));
    (speed, rate, bank)
}

/// Up 12 s, then hands off 20 s: how far it wanders across the ground
/// over the last 15, m, and its mean speed across the ground then, m/s.
fn hands_off(spec: &VehicleSpec) -> (f32, f32) {
    let (mut p, id) = start(spec);
    ride(&mut p, id, 12.0, controls(0.0, 0.0, false), 1.0, |_, _| {});
    ride(&mut p, id, 5.0, controls(0.0, 0.0, false), 0.0, |_, _| {});
    let at = |p: &Physics| {
        p.vehicle_transform(id)
            .map_or(Vec3::ZERO, |m| m.w_axis.truncate())
    };
    let from = at(&p);
    let (mut most, mut speed, mut n) = (0.0f32, 0.0, 0.0);
    ride(&mut p, id, 15.0, controls(0.0, 0.0, false), 0.0, |p, _| {
        most = most.max((at(p) - from).with_y(0.0).length());
        speed += p.vehicle_velocity(id).with_y(0.0).length();
        n += 1.0;
    });
    (most, speed / n)
}
