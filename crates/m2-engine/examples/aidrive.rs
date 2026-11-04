//! Drives every car template to three points with the 360's AI driver
//! ([`m2_engine::car_driver`]) on a level plane, as the alpha's physics
//! drives it: one 300 m ahead, one 100 m off at 45 degrees to port, one
//! 40 m astern. For each, the time to come within 6 m (or the nearest it
//! came in 60 s) and the fastest it went.
//!
//! ```text
//! cargo run --release -p m2-engine --example aidrive -- "<install dir>"
//! ```
//!
//! The 360 mind's think rate is not traced: the driver thinks every step.
//! Prints numbers and template names only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::car_driver::{CarDriver, CarInputs, CarState};
use m2_engine::tuning::{DAMAGE_TABLE, DAMAGE_TABLE_TYPE, DamageTable};
use m2_engine::{CatalogueEntry, CollisionShape, Install, Physics, StaticKey, VehicleControls};
use m2_engine::{VehicleId, VehicleSpec};
use m2_math::glam::{Quat, Vec3};

const DT: f32 = 1.0 / 60.0;
const SECONDS: f32 = 60.0;
const NEAR: f32 = 6.0;
const CAP: f32 = 40.0;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: aidrive <install dir>");
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
    let mut groups: BTreeMap<(u32, Vec<u8>), Vec<u32>> = BTreeMap::new();
    if let Some(c) = db.component("_CarPhysicsV2") {
        for (id, r) in c.records().unwrap_or_default() {
            if let Some(&model) = models.get(&id) {
                groups.entry((model, r.to_vec())).or_default().push(id);
            }
        }
    }
    println!("template\tcount\tahead s\tport s\tastern s\ttop m/s");
    for ((model, _), ids) in &groups {
        let mut ids = ids.clone();
        ids.sort_by_key(|id| names.get(id).map(|n| n.to_ascii_lowercase()));
        let name = names.get(&ids[0]).copied().unwrap_or("?");
        let Ok(data) = source.model(*model) else {
            continue;
        };
        let Ok(m) = data.parse() else {
            continue;
        };
        let entry = CatalogueEntry {
            model: *model,
            kind: "car".to_owned(),
            label: name.to_owned(),
        };
        let Some(spec) =
            VehicleSpec::from_model(&entry, &m).map(|s| s.with_chassis(&m, &data.base))
        else {
            continue;
        };
        let spec = spec.tuned(tuning.get(&ids[0]).copied());
        let mut top = 0.0f32;
        let mut cells = Vec::new();
        for to in [
            Vec3::new(0.0, 0.0, 300.0),
            Vec3::new(70.7, 0.0, 70.7),
            Vec3::new(0.0, 0.0, -40.0),
        ] {
            let (t, nearest, fastest) = drive(&spec, to);
            top = top.max(fastest);
            cells.push(t.map_or(format!("({nearest:.0} m)"), |t| format!("{t:.1}")));
        }
        println!("{name}\t{}\t{}\t{top:.1}", ids.len(), cells.join("\t"));
    }
    Ok(())
}

fn state(p: &Physics, id: VehicleId) -> Option<CarState> {
    let m = p.vehicle_transform(id)?;
    let at = m.transform_point3(Vec3::ZERO);
    let forward = m
        .transform_vector3(Vec3::Z)
        .with_y(0.0)
        .normalize_or(Vec3::Z);
    Some(CarState {
        at,
        forward,
        velocity: p.vehicle_velocity(id),
    })
}

/// From rest at the origin facing +z: when it came within [`NEAR`] of
/// `to` (from its start, the point moved by where it started), the
/// nearest it came, and its top speed.
fn drive(spec: &VehicleSpec, to: Vec3) -> (Option<f32>, f32, f32) {
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
        .rest_on(Vec3::ZERO, 0.0, |_| Some(0.0))
        .expect("rests on a plane");
    let id = p.add_vehicle_turned(spec, rest.position + Vec3::Y * 0.2, rest.rotation);
    for _ in 0..(2.0 / DT) as usize {
        p.drive(id, VehicleControls::default());
        p.step(DT);
    }
    let start = state(&p, id).map_or(Vec3::ZERO, |s| s.at.with_y(0.0));
    let goal = start + to;
    let mut driver = CarDriver::default();
    let mut held = CarInputs::default();
    let (mut nearest, mut fastest) = (f32::INFINITY, 0.0f32);
    for i in 0..(SECONDS / DT) as usize {
        let Some(s) = state(&p, id) else {
            break;
        };
        let d = s.at.with_y(0.0).distance(goal);
        nearest = nearest.min(d);
        fastest = fastest.max(s.velocity.length());
        if d < NEAR {
            return (Some(i as f32 * DT), nearest, fastest);
        }
        let input = driver.drive_to(&s, goal, CAP, false, DT);
        let forward = p.vehicle_gear(id).is_none_or(|(g, _)| g != 0);
        held.take(&input, forward);
        p.drive(id, held.controls());
        p.step(DT);
    }
    (None, nearest, fastest)
}
