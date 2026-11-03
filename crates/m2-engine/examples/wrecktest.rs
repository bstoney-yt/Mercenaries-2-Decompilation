//! Wrecks every wheeled and tracked vehicle template at speed on a level
//! plane, as the alpha's physics carries it, and prints how its wreck
//! goes: how far it slides after it is wrecked, how long until it stops,
//! and how high its model's origin and how tilted it rests.
//!
//! ```text
//! cargo run --release -p m2-engine --example wrecktest -- "<install dir>"
//! ```
//!
//! Templates sharing a model drive alike, so each model is wrecked once.
//! Prints numbers and template names only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::{
    CatalogueEntry, CollisionShape, Install, Physics, StaticKey, VehicleControls, VehicleSpec,
};
use m2_math::glam::{Quat, Vec3};

const DT: f32 = 1.0 / 60.0;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(root) = std::env::args().nth(1) else {
        eprintln!("usage: wrecktest <install dir>");
        std::process::exit(2);
    };
    let install = Install::find(Path::new(&root))?;
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
    let models = db.hashes("ModelName");
    let tuning = m2_engine::tuning::read(&db, None);
    let mut seen: BTreeMap<u32, (u32, &str)> = BTreeMap::new();
    for (comp, kind) in [("_CarPhysicsV2", "car"), ("_TankPhysics", "tank")] {
        let Some(c) = db.component(comp) else {
            continue;
        };
        for (id, _) in c.records().unwrap_or_default() {
            if let Some(&model) = models.get(&id) {
                seen.entry(model).or_insert((id, kind));
            }
        }
    }
    println!("template\tslid m\tstopped s\trest height m\ttilt deg");
    for (model, (id, kind)) in seen {
        let name = names.get(&id).copied().unwrap_or("?");
        let Ok(d) = source.model(model) else {
            continue;
        };
        let Ok(m) = d.parse() else {
            continue;
        };
        let entry = CatalogueEntry {
            model,
            kind: kind.to_owned(),
            label: name.to_owned(),
        };
        let Some(spec) = VehicleSpec::from_model(&entry, &m).map(|s| s.with_chassis(&m, &d.base))
        else {
            continue;
        };
        let spec = spec.tuned(tuning.get(&id).copied());
        if spec.wheels.is_empty() || spec.is_bike() {
            continue;
        }
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
        let Some(rest) = spec.rest_on(Vec3::new(0.0, 0.0, -3500.0), 0.0, |_| Some(0.0)) else {
            continue;
        };
        let id = p.add_vehicle_turned(&spec, rest.position + Vec3::Y * 0.2, rest.rotation);
        let flat_out = VehicleControls {
            throttle: 1.0,
            ..VehicleControls::default()
        };
        for _ in 0..(8.0 / DT) as usize {
            p.drive(id, flat_out);
            p.step(DT);
        }
        let at = |p: &Physics| {
            p.vehicle_transform(id)
                .map_or(Vec3::ZERO, |m| m.w_axis.truncate())
        };
        let from = at(&p);
        p.wreck_vehicle(id);
        let mut stopped = f32::NAN;
        for i in 0..(20.0 / DT) as usize {
            p.step(DT);
            if stopped.is_nan() && p.vehicle_speed(id) < 0.1 {
                stopped = (i + 1) as f32 * DT;
            }
        }
        let m = p.vehicle_transform(id).unwrap_or_default();
        let tilt = m.y_axis.truncate().normalize().y.clamp(-1.0, 1.0).acos();
        println!(
            "{name}\t{:.1}\t{stopped:.1}\t{:.2}\t{:.0}",
            (at(&p) - from).with_y(0.0).length(),
            m.w_axis.y,
            tilt.to_degrees()
        );
    }
    Ok(())
}
