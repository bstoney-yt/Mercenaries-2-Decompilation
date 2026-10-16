//! Rides every motorbike template on a level plane, as the alpha's
//! physics carries it, and prints how it goes beside what its
//! template's `_CarPhysicsV2` says (gap round 18): its top speed flat
//! out (`MaxSpeed`, km/h), how far it leans in a full-lock turn beside
//! the lean that balances that turn, how far a sideways shove at
//! speed rolls it and how long it takes to stand back up, and whether
//! it ever tips over (standing, turning, shoved or weaving).
//!
//! ```text
//! cargo run --release -p m2-engine --example biketest -- "<install dir>"
//! ```
//!
//! Tipped: its up axis more than 60 degrees off the vertical at any
//! step. Balance lean: atan(v w / g) over the turn's last 3 s, its speed
//! v and turning rate w. Templates sharing a model and physics record
//! ride alike, so each pair is ridden once. Prints numbers and template
//! names only.

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
const GRAVITY: f32 = 9.81;
/// How long it goes flat out, seconds.
const AHEAD: f32 = 30.0;
/// Up axis this far off the vertical, cosine: tipped over.
const TIPPED: f32 = 0.5;
/// The sideways shove, m/s of its whole mass, 1 m above its origin.
const SHOVE: f32 = 1.5;
/// Back up: within this lean, degrees.
const STOOD: f32 = 3.0;

type Driven = (u32, Vec<u8>);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: biketest <install dir>");
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
        at("MaxSpeed"),
        at("BikeRollStabilityStrength"),
        at("BikeExtraDampRoll"),
        at("BikeExtraDampPitch"),
        at("BikeExtraDampYaw"),
    ];
    let mut groups: BTreeMap<Driven, Vec<u32>> = BTreeMap::new();
    for (id, r) in c.records().unwrap_or_default() {
        if let Some(&model) = models.get(&id) {
            groups.entry((model, r.to_vec())).or_default().push(id);
        }
    }
    println!(
        "template\tcount\tdata km/h\tkm/h\ttop share\troll k\troll c\tpitch c\tyaw c\t\
         turn km/h\tlean deg\tbalance deg\tshove deg\tstood s\tweave deg\ttipped"
    );
    let (mut ridden, mut tipped_all) = (0, 0);
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
            continue;
        };
        let spec = spec.tuned(tuning.get(&id).copied());
        if !spec.is_bike() {
            continue;
        }
        ridden += ids.len();
        let f = |at: Option<usize>| {
            at.and_then(|a| record.get(a..a + 4))
                .map_or(f32::NAN, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let [top, k, cr, cp, cy] = fields.map(f);
        let mut tipped = Vec::new();
        if stand(&spec) {
            tipped.push("stand");
        }
        let (fastest, t) = ahead(&spec);
        if t {
            tipped.push("ahead");
        }
        let (turn, lean, balance, t) = corner(&spec);
        if t {
            tipped.push("turn");
        }
        let (shoved, stood, t) = shove(&spec);
        if t {
            tipped.push("shove");
        }
        let (weave, t) = slalom(&spec);
        if t {
            tipped.push("weave");
        }
        if !tipped.is_empty() {
            tipped_all += ids.len();
        }
        println!(
            "{name}\t{}\t{top}\t{:.1}\t{:.2}\t{k}\t{cr}\t{cp}\t{cy}\t{:.1}\t{lean:.1}\t\
             {balance:.1}\t{shoved:.1}\t{stood:.2}\t{weave:.1}\t{}",
            ids.len(),
            fastest * 3.6,
            fastest * 3.6 / top,
            turn * 3.6,
            if tipped.is_empty() {
                "-".to_owned()
            } else {
                tipped.join(",")
            },
        );
    }
    println!("ridden {ridden}, tipped {tipped_all}");
    Ok(())
}

/// A level plane and the bike settled on it, facing +z.
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
        p.drive(id, parked());
        p.step(DT);
    }
    (p, id)
}

fn parked() -> VehicleControls {
    VehicleControls {
        parked: true,
        ..VehicleControls::default()
    }
}

/// Its lean, radians (positive to its right), its up axis's cosine off
/// the vertical, and which way it heads (radians about +y).
fn attitude(p: &Physics, id: VehicleId) -> (f32, f32, f32) {
    let Some(m) = p.vehicle_transform(id) else {
        return (0.0, 1.0, 0.0);
    };
    let up = m.y_axis.truncate().normalize_or(Vec3::Y);
    let ahead = m.z_axis.truncate();
    let flat = ahead.with_y(0.0).normalize_or(Vec3::Z);
    let right = flat.cross(Vec3::Y);
    (
        up.dot(right).clamp(-1.0, 1.0).asin(),
        up.y,
        flat.x.atan2(flat.z),
    )
}

/// Drives `controls` for `seconds`, calling `each` after every step;
/// whether it tipped.
fn ride(
    p: &mut Physics,
    id: VehicleId,
    seconds: f32,
    mut controls: impl FnMut(f32) -> VehicleControls,
    mut each: impl FnMut(&Physics, f32),
) -> bool {
    let mut tipped = false;
    for i in 0..(seconds / DT) as usize {
        let t = i as f32 * DT;
        p.drive(id, controls(t));
        p.step(DT);
        tipped |= attitude(p, id).1 < TIPPED;
        each(p, t + DT);
    }
    tipped
}

fn throttle(steer: f32) -> VehicleControls {
    VehicleControls {
        throttle: 1.0,
        steer,
        ..VehicleControls::default()
    }
}

/// Parked 10 s: whether it tipped.
fn stand(spec: &VehicleSpec) -> bool {
    let (mut p, id) = start(spec);
    ride(&mut p, id, 10.0, |_| parked(), |_, _| {})
}

/// Flat out [`AHEAD`] s: its fastest, m/s, and whether it tipped.
fn ahead(spec: &VehicleSpec) -> (f32, bool) {
    let (mut p, id) = start(spec);
    let mut fastest = 0.0f32;
    let t = ride(
        &mut p,
        id,
        AHEAD,
        |_| throttle(0.0),
        |p, _| fastest = fastest.max(p.vehicle_speed(id)),
    );
    (fastest, t)
}

/// 8 s flat out, then full lock left with the throttle open 6 s: its
/// speed, mean lean and the lean that balances its turn over the last
/// 3 s (degrees, positive into the turn), and whether it tipped.
fn corner(spec: &VehicleSpec) -> (f32, f32, f32, bool) {
    let (mut p, id) = start(spec);
    let mut tipped = ride(&mut p, id, 8.0, |_| throttle(0.0), |_, _| {});
    let (mut n, mut speed, mut lean, mut rate) = (0.0, 0.0, 0.0, 0.0);
    let mut heading = attitude(&p, id).2;
    tipped |= ride(
        &mut p,
        id,
        6.0,
        |_| throttle(1.0),
        |p, t| {
            let (l, _, h) = attitude(p, id);
            let turned = (h - heading + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
            heading = h;
            if t > 3.0 {
                n += 1.0;
                speed += p.vehicle_speed(id);
                // Turning left (steer +1) leans it left: negative.
                lean -= l;
                rate += turned.abs() / DT;
            }
        },
    );
    let n = f32::max(n, 1.0);
    let (speed, lean, rate) = (speed / n, lean / n, rate / n);
    let balance = (speed * rate / GRAVITY).atan();
    (speed, lean.to_degrees(), balance.to_degrees(), tipped)
}

/// 8 s flat out, then shoved sideways ([`SHOVE`]) and ridden on 6 s
/// straight: the most it rolls (degrees), how long until it stays
/// within [`STOOD`] degrees (s), and whether it tipped.
fn shove(spec: &VehicleSpec) -> (f32, f32, bool) {
    let (mut p, id) = start(spec);
    let mut tipped = ride(&mut p, id, 8.0, |_| throttle(0.0), |_, _| {});
    if let Some(m) = p.vehicle_transform(id) {
        let at = m.w_axis.truncate() + m.y_axis.truncate();
        let right = m.x_axis.truncate();
        p.push_vehicle(id, -right * spec.mass * SHOVE, at);
    }
    let (mut most, mut out) = (0.0f32, 0.0f32);
    tipped |= ride(
        &mut p,
        id,
        6.0,
        |_| throttle(0.0),
        |p, t| {
            let l = attitude(p, id).0.to_degrees().abs();
            most = most.max(l);
            if l > STOOD {
                out = t;
            }
        },
    );
    (most, out, tipped)
}

/// Flat out 12 s weaving full lock one way then the other every 1.5 s
/// after the first 4: the most it leans (degrees) and whether it
/// tipped.
fn slalom(spec: &VehicleSpec) -> (f32, bool) {
    let (mut p, id) = start(spec);
    let mut most = 0.0f32;
    let t = ride(
        &mut p,
        id,
        12.0,
        |t| {
            if t < 4.0 {
                throttle(0.0)
            } else if (((t - 4.0) / 1.5) as u32).is_multiple_of(2) {
                throttle(1.0)
            } else {
                throttle(-1.0)
            }
        },
        |p, _| most = most.max(attitude(p, id).0.to_degrees().abs()),
    );
    (most, t)
}
