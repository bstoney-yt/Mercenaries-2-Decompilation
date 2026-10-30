//! Brakes and steers every wheeled vehicle template on a level plane, as
//! the alpha's physics drives it, and prints how it stops and turns:
//! from 50 km/h (or 90% of its top speed, if lower), braking with the
//! throttle against the motion, the time and distance to a stop and the
//! mean slowing; then, held near 20 km/h at full lock, the radius it
//! turns on and whether it rolls over.
//!
//! ```text
//! cargo run --release -p m2-engine --example handlingtest -- "<install dir>"
//! ```
//!
//! Templates sharing a model and the same physics record drive alike, so
//! each such pair is driven once and printed with its template count.
//! Prints numbers and template names only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::tuning::{DAMAGE_TABLE, DAMAGE_TABLE_TYPE, DamageTable};
use m2_engine::{
    CatalogueEntry, CollisionShape, Install, Physics, StaticKey, VehicleControls, VehicleId,
    VehicleSpec,
};
use m2_math::glam::{Quat, Vec3};

const DT: f32 = 1.0 / 60.0;
/// The speed it brakes from and turns at, m/s.
const BRAKE_FROM: f32 = 50.0 / 3.6;
const TURN_AT: f32 = 20.0 / 3.6;

type Driven = (u32, Vec<u8>);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: handlingtest <install dir>");
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
        .asset(m2_assets::name_hash(DAMAGE_TABLE), DAMAGE_TABLE_TYPE)
        .ok()
        .and_then(|d| DamageTable::parse(&d));
    let tuning = m2_engine::tuning::read(&db, damage.as_ref());
    let models = db.hashes("ModelName");

    let mut groups: BTreeMap<Driven, Vec<u32>> = BTreeMap::new();
    let c = db.component("_CarPhysicsV2").ok_or("no _CarPhysicsV2")?;
    for (id, r) in c.records().unwrap_or_default() {
        if let Some(&model) = models.get(&id) {
            groups.entry((model, r.to_vec())).or_default().push(id);
        }
    }
    println!(
        "template\tcount\tmass\tlock rad\tbrake m/s2 (data)\tfrom km/h\tstop s\tstop m\tmean m/s2\tturn km/h\tradius m\trolled"
    );
    let (mut driven, mut skipped) = (0, 0);
    for ((model, _), ids) in &groups {
        let mut ids = ids.clone();
        ids.sort_by_key(|id| names.get(id).map(|n| n.to_ascii_lowercase()));
        let id = ids[0];
        let name = names.get(&id).copied().unwrap_or("?");
        let Some(spec) = source.model(*model).ok().and_then(|d| {
            let m = d.parse().ok()?;
            let entry = CatalogueEntry {
                model: *model,
                kind: "car".to_owned(),
                label: name.to_owned(),
            };
            VehicleSpec::from_model(&entry, &m)
        }) else {
            skipped += ids.len();
            continue;
        };
        let spec = spec.tuned(tuning.get(&id).copied());
        if spec.tracked {
            skipped += ids.len();
            continue;
        }
        driven += ids.len();
        let handling = spec.tuning.handling;
        let lock = handling.map_or("-".to_owned(), |h| format!("{:.2}", h.steer_lock));
        let brakes = spec.tuning.brakes.map_or("-".to_owned(), |b| {
            format!("{:.1}/{:.1}", b.brake.0, b.brake.1)
        });
        let from = spec
            .tuning
            .max_speed
            .map_or(BRAKE_FROM, |top| BRAKE_FROM.min(0.9 * top));
        let stop = brake(&spec, from);
        let turn = turn(&spec);
        let f = |v: Option<f32>, p: usize| v.map_or("-".to_owned(), |v| format!("{v:.p$}"));
        println!(
            "{name}\t{}\t{:.0}\t{lock}\t{brakes}\t{:.0}\t{}\t{}\t{}\t{}\t{}\t{}",
            ids.len(),
            spec.mass,
            from * 3.6,
            f(stop.map(|s| s.0), 2),
            f(stop.map(|s| s.1), 1),
            f(stop.map(|s| s.2), 2),
            f(turn.map(|t| t.0 * 3.6), 0),
            f(turn.map(|t| t.1), 1),
            turn.is_some_and(|t| t.2),
        );
    }
    println!("driven {driven}, skipped {skipped}");
    Ok(())
}

/// A level plane and the vehicle settled on it, facing +z.
fn start(spec: &VehicleSpec) -> (Physics, VehicleId) {
    let mut p = Physics::new();
    let half = 3000.0;
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
        .rest_on(Vec3::new(0.0, 0.0, -2500.0), 0.0, |_| Some(0.0))
        .expect("rests on a plane");
    let id = p.add_vehicle_turned(spec, rest.position + Vec3::Y * 0.2, rest.rotation);
    for _ in 0..(2.0 / DT) as usize {
        p.drive(id, VehicleControls::default());
        p.step(DT);
    }
    (p, id)
}

fn position(p: &Physics, id: VehicleId) -> Vec3 {
    p.vehicle_transform(id)
        .map_or(Vec3::ZERO, |m| m.w_axis.truncate())
}

/// Flat out to `from` m/s, then the throttle full against the motion:
/// seconds and metres to a stop (below 0.3 m/s) and the mean slowing.
fn brake(spec: &VehicleSpec, from: f32) -> Option<(f32, f32, f32)> {
    let (mut p, id) = start(spec);
    let full = |throttle| VehicleControls {
        throttle,
        ..Default::default()
    };
    let mut t = 0.0;
    while p.vehicle_speed(id) < from {
        p.drive(id, full(1.0));
        p.step(DT);
        t += DT;
        if t > 60.0 {
            return None;
        }
    }
    let v0 = p.vehicle_speed(id);
    let at = position(&p, id);
    let mut t = 0.0;
    while p.vehicle_speed(id) > 0.3 {
        p.drive(id, full(-1.0));
        p.step(DT);
        t += DT;
        if t > 30.0 {
            return None;
        }
    }
    let d = (position(&p, id) - at).with_y(0.0).length();
    Some((t, d, v0 / t.max(DT)))
}

/// Held near [`TURN_AT`] at full left lock for 8 s: the speed it holds,
/// the radius it turns on over the last 4 s (speed over yaw rate), and
/// whether it rolled over (its roof below level).
fn turn(spec: &VehicleSpec) -> Option<(f32, f32, bool)> {
    let (mut p, id) = start(spec);
    let mut heading = Vec::new();
    let mut speeds = Vec::new();
    let mut rolled = false;
    for i in 0..(10.0 / DT) as usize {
        let v = p.vehicle_speed(id);
        let throttle = ((TURN_AT - v) * 0.5).clamp(-1.0, 1.0);
        // Straight until it is up to speed, then full lock.
        let steer = if i as f32 * DT < 2.0 { 0.0 } else { 1.0 };
        p.drive(
            id,
            VehicleControls {
                throttle: if throttle < 0.0 { 0.0 } else { throttle },
                steer,
                ..Default::default()
            },
        );
        p.step(DT);
        let m = p.vehicle_transform(id)?;
        rolled |= m.y_axis.y < 0.0;
        if i as f32 * DT >= 6.0 {
            let ahead = m.z_axis.truncate();
            heading.push(ahead.x.atan2(ahead.z));
            speeds.push(v);
        }
    }
    let mut yaw = 0.0;
    for w in heading.windows(2) {
        let mut d = w[1] - w[0];
        if d > std::f32::consts::PI {
            d -= std::f32::consts::TAU;
        } else if d < -std::f32::consts::PI {
            d += std::f32::consts::TAU;
        }
        yaw += d;
    }
    let secs = (heading.len().max(2) - 1) as f32 * DT;
    let rate = (yaw / secs).abs();
    let speed = speeds.iter().sum::<f32>() / speeds.len().max(1) as f32;
    (rate > 1e-3).then_some((speed, speed / rate, rolled))
}
