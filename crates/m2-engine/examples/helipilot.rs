//! Flies every helicopter template with the 360's AI pilot
//! ([`m2_engine::heli_pilot`]) over a level plane, its stick wobbling as
//! an AI's does: from the ground it hovers 30 m up (how long until it is
//! within a metre, the most it overshoots, and its height error and drift
//! over the last 5 s of 30), then flies to a point 400 m ahead at that
//! height (how long until it is within 5 m, its fastest, and where it
//! stops).
//!
//! ```text
//! cargo run --release -p m2-engine --example helipilot -- "<install dir>"
//! ```
//!
//! Prints numbers and template names only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::name_hash;
use m2_engine::heli_pilot::HeliPilot;
use m2_engine::tuning::{DAMAGE_TABLE, DAMAGE_TABLE_TYPE, DamageTable, PilotTuning};
use m2_engine::{
    CatalogueEntry, CollisionShape, Install, Physics, StaticKey, VehicleId, VehicleSpec,
};
use m2_math::glam::{Quat, Vec3};

const DT: f32 = 1.0 / 60.0;
const HOVER: f32 = 30.0;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(root) = std::env::args().nth(1) else {
        eprintln!("usage: helipilot <install dir>");
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
    let damage = source
        .asset(name_hash(DAMAGE_TABLE), DAMAGE_TABLE_TYPE)
        .ok()
        .and_then(|d| DamageTable::parse(&d));
    let tuning = m2_engine::tuning::read(&db, damage.as_ref());
    let models = db.hashes("ModelName");
    let c = db
        .component("_HelicopterPhysicsAi")
        .ok_or("no _HelicopterPhysicsAi")?;
    let mut groups: BTreeMap<(u32, Vec<u8>), Vec<u32>> = BTreeMap::new();
    for (id, r) in c.records().unwrap_or_default() {
        if let Some(&model) = models.get(&id) {
            groups.entry((model, r.to_vec())).or_default().push(id);
        }
    }
    println!(
        "template\tcount\tfwd m/s\tmin alt\tto 30 m s\tover m\tlast 5 s err m\tdrift m\tto 400 m s\tfastest m/s\tends off m"
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
        let Some(flight) = spec.tuning.flight_ai.or(spec.tuning.flight) else {
            continue;
        };
        let pilot_tuning = spec.tuning.pilot.unwrap_or(PilotTuning::DEFAULT);
        let (mut p, vid) = start(&spec);
        let mut pilot = HeliPilot::new(pilot_tuning, &flight);
        // Up to 30 m and held there for 30 s.
        let (mut reached, mut top, mut err, mut n, mut from) =
            (f32::NAN, 0.0f32, 0.0f32, 0.0f32, Vec3::ZERO);
        for i in 0..(30.0 / DT) as usize {
            think(&mut p, vid, &mut pilot);
            let stick = pilot.hover(HOVER, true);
            p.pilot_helicopter(vid, stick);
            p.step(DT);
            let at = place(&p, vid);
            let t = (i + 1) as f32 * DT;
            if reached.is_nan() && (at.y - HOVER).abs() < 1.0 {
                reached = t;
            }
            if !reached.is_nan() {
                top = top.max(at.y - HOVER);
            }
            if t > 25.0 {
                if n == 0.0 {
                    from = at;
                }
                err += (at.y - HOVER).abs();
                n += 1.0;
            }
        }
        let at = place(&p, vid);
        let drift = (at - from).with_y(0.0).length();
        // Then 400 m ahead, at 30 m.
        let goal = at + Vec3::new(0.0, 0.0, 400.0);
        let goal = goal.with_y(HOVER);
        let (mut arrived, mut fastest) = (f32::NAN, 0.0f32);
        for i in 0..(90.0 / DT) as usize {
            think(&mut p, vid, &mut pilot);
            let stick = pilot.fly_to(goal, 1.0, true);
            p.pilot_helicopter(vid, stick);
            p.step(DT);
            let at = place(&p, vid);
            fastest = fastest.max(p.vehicle_velocity(vid).length());
            if arrived.is_nan() && (at - goal).length() < 5.0 {
                arrived = (i + 1) as f32 * DT;
            }
        }
        let off = (place(&p, vid) - goal).length();
        println!(
            "{name}\t{}\t{:.0}\t{:.0}\t{reached:.2}\t{top:.2}\t{:.2}\t{drift:.2}\t{arrived:.1}\t{fastest:.1}\t{off:.2}",
            ids.len(),
            flight.forward,
            pilot_tuning.min_altitude,
            err / n.max(1.0),
        );
    }
    Ok(())
}

fn place(p: &Physics, id: VehicleId) -> Vec3 {
    p.vehicle_transform(id)
        .map_or(Vec3::ZERO, |m| m.w_axis.truncate())
}

/// One think of its pilot: where it is, its turn and velocity, and the
/// ground under it.
fn think(p: &mut Physics, id: VehicleId, pilot: &mut HeliPilot) {
    let Some(m) = p.vehicle_transform(id) else {
        return;
    };
    let (_, turn, at) = m.to_scale_rotation_translation();
    let ground = p.ground_under(id, at);
    pilot.sense(at, turn, p.vehicle_velocity(id), ground);
}

/// A level plane and the helicopter settled on it, facing +z, its stick
/// an AI's.
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
    p.set_ai_pilot(id, true);
    for _ in 0..120 {
        p.step(DT);
    }
    (p, id)
}
