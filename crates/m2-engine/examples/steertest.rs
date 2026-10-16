//! Steers every wheeled vehicle template on a level plane, as the
//! alpha's physics drives it, and prints how quickly its front wheels
//! swing to full lock, slow (15 km/h) and fast (70 km/h, or 80% of its
//! top speed if lower), beside its template's `SteerSpeed`,
//! `LowSpeedSteerSpeed` and `LowSpeedLimit` (gap round 14).
//!
//! ```text
//! cargo run --release -p m2-engine --example steertest -- "<install dir>"
//! ```
//!
//! For each speed: held straight up to it, then full left steering for
//! 1.5 s; the angle the front wheels reach, the swing rate (90% of that
//! angle over the time to reach it, rad/s), the rate its template's
//! fields give at that speed (rad/s, `SteerSpeed` at or above
//! `LowSpeedLimit` km/h, `LowSpeedSteerSpeed` below), and the heading
//! turned in the first 0.5 s (degrees). Templates sharing a model and
//! physics record drive alike, so each pair is driven once. Prints
//! numbers and template names only.

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
const SLOW: f32 = 15.0 / 3.6;
const FAST: f32 = 70.0 / 3.6;
const HOLD: f32 = 1.5;

type Driven = (u32, Vec<u8>);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: steertest <install dir>");
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

    let c = db.component("_CarPhysicsV2").ok_or("no _CarPhysicsV2")?;
    let at = |name: &str| {
        c.fields
            .iter()
            .find(|f| f.name_hash == name_hash(name))
            .map(|f| usize::from(f.offset))
    };
    let fields = [
        at("SteerMaxAngle"),
        at("SteerSpeed"),
        at("LowSpeedSteerSpeed"),
        at("LowSpeedLimit"),
    ];
    let mut groups: BTreeMap<Driven, Vec<u32>> = BTreeMap::new();
    for (id, r) in c.records().unwrap_or_default() {
        if let Some(&model) = models.get(&id) {
            groups.entry((model, r.to_vec())).or_default().push(id);
        }
    }
    println!(
        "template\tcount\tlock\tSteerSpeed\tLowSpeedSteerSpeed\tLowSpeedLimit km/h\t\
         slow km/h\tslow angle\tslow rate\tslow data\tslow deg 0.5s\t\
         fast km/h\tfast angle\tfast rate\tfast data\tfast deg 0.5s"
    );
    let (mut driven, mut skipped) = (0, 0);
    for ((model, record), ids) in &groups {
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
        if spec.tracked || !spec.wheels.iter().any(|w| w.steers) {
            skipped += ids.len();
            continue;
        }
        driven += ids.len();
        let f = |at: Option<usize>| {
            at.and_then(|a| record.get(a..a + 4))
                .map_or(f32::NAN, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let [lock, steer, low_steer, low_limit] = fields.map(f);
        let data = |speed: f32| {
            if speed * 3.6 < low_limit {
                low_steer
            } else {
                steer
            }
        };
        let top = spec.tuning.max_speed.unwrap_or(FAST / 0.8);
        let mut row = format!(
            "{name}\t{}\t{lock:.2}\t{steer:.2}\t{low_steer:.2}\t{low_limit:.0}",
            ids.len()
        );
        for want in [SLOW, FAST.min(0.8 * top)] {
            match swing(&spec, want) {
                Some((v, angle, rate, turned)) => {
                    row += &format!(
                        "\t{:.0}\t{angle:.3}\t{rate:.2}\t{:.2}\t{turned:.1}",
                        v * 3.6,
                        data(v)
                    );
                }
                None => row += "\t-\t-\t-\t-\t-",
            }
        }
        println!("{row}");
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

fn heading(p: &Physics, id: VehicleId) -> f32 {
    p.vehicle_transform(id).map_or(0.0, |m| {
        let a = m.z_axis.truncate();
        a.x.atan2(a.z)
    })
}

/// The most any wheel is steered, radians.
fn wheel_angle(p: &Physics, id: VehicleId) -> f32 {
    p.vehicle_wheels(id)
        .iter()
        .map(|w| w.0.abs())
        .fold(0.0, f32::max)
}

/// Straight up to `want` m/s, then full left steering for [`HOLD`] s
/// holding the speed: the speed it held, the angle its wheels reached,
/// the swing rate to 90% of it (rad/s) and the heading turned in the
/// first 0.5 s (degrees).
fn swing(spec: &VehicleSpec, want: f32) -> Option<(f32, f32, f32, f32)> {
    let (mut p, id) = start(spec);
    let throttle = |p: &Physics| ((want - p.vehicle_speed(id)) * 0.5).clamp(0.0, 1.0);
    let mut t = 0.0;
    while p.vehicle_speed(id) < want * 0.97 {
        let c = VehicleControls {
            throttle: throttle(&p),
            ..Default::default()
        };
        p.drive(id, c);
        p.step(DT);
        t += DT;
        if t > 60.0 {
            return None;
        }
    }
    let v = p.vehicle_speed(id);
    let h0 = heading(&p, id);
    let mut angles = Vec::new();
    let mut turned = 0.0;
    for i in 0..(HOLD / DT) as usize {
        let c = VehicleControls {
            throttle: throttle(&p),
            steer: 1.0,
            ..Default::default()
        };
        p.drive(id, c);
        p.step(DT);
        angles.push(wheel_angle(&p, id));
        if i + 1 == (0.5 / DT) as usize {
            let mut d = heading(&p, id) - h0;
            if d > std::f32::consts::PI {
                d -= std::f32::consts::TAU;
            } else if d < -std::f32::consts::PI {
                d += std::f32::consts::TAU;
            }
            turned = d.abs().to_degrees();
        }
    }
    let end = *angles.last()?;
    let reach = angles.iter().position(|a| *a >= 0.9 * end)? + 1;
    Some((v, end, 0.9 * end / (reach as f32 * DT), turned))
}
