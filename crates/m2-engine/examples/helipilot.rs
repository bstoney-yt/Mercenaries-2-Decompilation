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
        "template\tcount\tfwd m/s\tmin alt\tto 30 m s\tover m\tlast 5 s err m\tdrift m\tto 400 m s\tfastest m/s\tends off m	land s	round wall s	wall clear m	land-near s	land-near off m"
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
        // Down from where it stopped.
        let mut landed = f32::NAN;
        for i in 0..(60.0 / DT) as usize {
            think(&mut p, vid, &mut pilot);
            let stick = pilot.land();
            p.pilot_helicopter(vid, stick);
            p.step(DT);
            if landed.is_nan() && pilot.clearance < 0.3 {
                landed = (i + 1) as f32 * DT;
            }
        }
        let (round, clear) = round_the_wall(&spec, &flight, pilot_tuning);
        let (down, spot) = land_by_a_shed(&spec, &flight, pilot_tuning);
        println!(
            "{name}\t{}\t{:.0}\t{:.0}\t{reached:.2}\t{top:.2}\t{:.2}\t{drift:.2}\t{arrived:.1}\t{fastest:.1}\t{off:.2}\t{landed:.1}\t{round:.1}\t{clear:.1}	{down:.1}	{spot:.1}",
            ids.len(),
            flight.forward,
            pilot_tuning.min_altitude,
            err / n.max(1.0),
        );
    }
    Ok(())
}

/// From a hover 30 m up, flown round things ([`HeliPilot::fly_round_to`])
/// to 300 m ahead at 30 m past a wall 60 m high and 40 m wide across its
/// way halfway: how long until it is within 5 m, and the least its
/// underside cleared the wall's top by passing it (negative: under it).
fn round_the_wall(
    spec: &VehicleSpec,
    flight: &m2_engine::tuning::FlightTuning,
    tuning: PilotTuning,
) -> (f32, f32) {
    let (mut p, vid) = start(spec);
    let at = place(&p, vid);
    let wall = CollisionShape::cuboid(Vec3::new(20.0, 30.0, 2.0));
    p.add_static(
        StaticKey::Cell(2),
        &wall,
        Vec3::new(at.x, 30.0, at.z + 150.0),
        Quat::IDENTITY,
    );
    let mut pilot = HeliPilot::new(tuning, flight);
    for _ in 0..(20.0 / DT) as usize {
        think(&mut p, vid, &mut pilot);
        let stick = pilot.hover(HOVER, true);
        p.pilot_helicopter(vid, stick);
        p.step(DT);
    }
    let size = {
        let (lo, hi) = spec.hull;
        (hi.y - lo.y, (hi.x - lo.x).max(hi.z - lo.z))
    };
    let goal = Vec3::new(at.x, HOVER, at.z + 300.0);
    let (mut arrived, mut clear) = (f32::NAN, f32::INFINITY);
    for i in 0..(120.0 / DT) as usize {
        think(&mut p, vid, &mut pilot);
        let stick = {
            let mut sight = m2_engine::heli_pilot::Sight {
                physics: &p,
                own: vid,
            };
            pilot.fly_round_to(goal, 1.0, true, true, size, &mut sight, DT)
        };
        p.pilot_helicopter(vid, stick);
        p.step(DT);
        let now = place(&p, vid);
        if (now.z - (at.z + 150.0)).abs() < 3.0 {
            clear = clear.min(now.y + spec.hull.0.y - 60.0);
        }
        if arrived.is_nan() && (now - goal).length() < 5.0 {
            arrived = (i + 1) as f32 * DT;
        }
    }
    (arrived, clear)
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
    pilot.sense(at, turn, p.vehicle_velocity(id), ground, DT);
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

/// From a hover 30 m up 300 m off, a landing (`HeliLand`,
/// [`HeliPilot::land_near`]) at a point half a metre from a shed 8 m square and 4 m high: how
/// long until it is down, and how far from the point it set down (off
/// the shed: the patch under it level).
fn land_by_a_shed(
    spec: &VehicleSpec,
    flight: &m2_engine::tuning::FlightTuning,
    tuning: PilotTuning,
) -> (f32, f32) {
    let (mut p, vid) = start(spec);
    let at = place(&p, vid);
    let point = Vec3::new(at.x, 0.0, at.z + 300.0);
    let shed = CollisionShape::cuboid(Vec3::new(4.0, 2.0, 4.0));
    p.add_static(
        StaticKey::Object(3),
        &shed,
        point + Vec3::new(4.5, 2.0, 0.0),
        Quat::IDENTITY,
    );
    let mut pilot = HeliPilot::new(tuning, flight);
    for _ in 0..(20.0 / DT) as usize {
        think(&mut p, vid, &mut pilot);
        let stick = pilot.hover(HOVER, true);
        p.pilot_helicopter(vid, stick);
        p.step(DT);
    }
    let size = {
        let (lo, hi) = spec.hull;
        (hi.y - lo.y, (hi.x - lo.x).max(hi.z - lo.z))
    };
    let mut search = None;
    let mut down = f32::NAN;
    for i in 0..(120.0 / DT) as usize {
        think(&mut p, vid, &mut pilot);
        pilot.airborne = p.helicopter_airborne(vid).unwrap_or(true);
        let (stick, how) = {
            let mut sight = m2_engine::heli_pilot::Sight {
                physics: &p,
                own: vid,
            };
            pilot.land_near(point, &mut search, spec.hull, size, &mut sight, DT)
        };
        p.pilot_helicopter(vid, stick);
        p.step(DT);
        if how == m2_engine::heli_pilot::Landing::Landed {
            down = (i + 1) as f32 * DT;
            break;
        }
    }
    (down, (place(&p, vid) - point).with_y(0.0).length())
}
