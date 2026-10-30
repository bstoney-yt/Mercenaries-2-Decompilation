//! Settles every wheeled vehicle template on a level plane, as the
//! alpha's physics carries it, then kicks it 0.3 m/s downwards and
//! prints how it bounces: its sag (how far its springs sit compressed,
//! mean over the wheels, m) and the heave frequency and damping ratio
//! its height swings with, beside the ones its template's
//! `FrontWheelSuspStrength`, `RearWheelSuspStrength`,
//! `FrontWheelSuspDamp` and `RearWheelSuspDamp` give, and its template's
//! `FrontWheelRadius` and `RearWheelRadius` beside the model's wheels
//! (gap round 16), and how far below the plane its tyres are drawn once
//! settled (`sunk`, deepest and mean over its wheels; `SUNK_AT_MODEL=1`
//! measures them where the model has them, as the alpha drew them before
//! they followed their springs).
//!
//! ```text
//! cargo run --release -p m2-engine --example susptest -- "<install dir>"
//! ```
//!
//! The data's heave: each wheel pushes with its strength x compression
//! x the chassis mass, less its damp x compression speed x the mass
//! (both read per unit of mass, as a Havok or Bullet ray-cast
//! suspension takes them; inferred), so the body bounces at
//! w = sqrt(sum of strengths) rad/s with damping ratio
//! (sum of damps) / (2 w), and swings at w sqrt(1 - zeta^2) (`data
//! damped Hz`). Measured: when it first crosses its settled height
//! back, and how far it swings past it (see [`bounce`]). Templates
//! sharing a model and physics record ride alike, so each pair is
//! settled once. Prints numbers and template names only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::name_hash;
use m2_engine::tuning::{DAMAGE_TABLE, DAMAGE_TABLE_TYPE, DamageTable, Springs};
use m2_engine::{
    CatalogueEntry, CollisionShape, Install, Physics, StaticKey, VehicleControls, VehicleId,
    VehicleSpec,
};
use m2_math::glam::{Quat, Vec3};

const DT: f32 = 1.0 / 60.0;
const KICK: f32 = 0.3;
const WATCH: f32 = 3.0;
const NOISE: f32 = 0.02;

type Driven = (u32, Vec<u8>);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: susptest <install dir>");
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
        at("FrontWheelSuspStrength"),
        at("RearWheelSuspStrength"),
        at("FrontWheelSuspDamp"),
        at("RearWheelSuspDamp"),
        at("FrontWheelRadius"),
        at("RearWheelRadius"),
    ];
    let mut groups: BTreeMap<Driven, Vec<u32>> = BTreeMap::new();
    for (id, r) in c.records().unwrap_or_default() {
        if let Some(&model) = models.get(&id) {
            groups.entry((model, r.to_vec())).or_default().push(id);
        }
    }
    println!(
        "template\tcount\twheels\tk front\tk rear\tc front\tc rear\t\
         sag m\tdata Hz\tdata damped Hz\tHz\tdata zeta\tzeta\t\
         data r front\tr front\tdata r rear\tr rear\tsunk max m\tsunk mean m\t\
         exp front\texp rear\tcmp front\tcmp rear\textra\tgravity\tdata sag m\tdrop m"
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
        if spec.tracked || spec.wheels.is_empty() || spec.is_boat() {
            skipped += ids.len();
            continue;
        }
        driven += ids.len();
        let f = |at: Option<usize>| {
            at.and_then(|a| record.get(a..a + 4))
                .map_or(f32::NAN, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let [kf, kr, cf, cr, rf, rr] = fields.map(f);
        let front = spec.wheels.iter().filter(|w| w.steers).count() as f32;
        let rear = spec.wheels.len() as f32 - front;
        let w = (front * kf + rear * kr).sqrt();
        let zeta = (front * cf + rear * cr) / (2.0 * w);
        let radius = |steers: bool| {
            let r: Vec<f32> = spec
                .wheels
                .iter()
                .filter(|w| w.steers == steers)
                .map(|w| w.radius)
                .collect();
            r.iter().sum::<f32>() / r.len().max(1) as f32
        };
        let mut row = format!(
            "{name}\t{}\t{}\t{kf}\t{kr}\t{cf}\t{cr}",
            ids.len(),
            spec.wheels.len()
        );
        let sunk = sunk(&spec);
        let (sag, hz, z) = bounce(&spec);
        row += &format!(
            "\t{sag:.3}\t{:.2}\t{:.2}\t{hz:.2}\t{zeta:.2}\t{z:.2}",
            w / std::f32::consts::TAU,
            w * (1.0 - zeta * zeta).max(0.0).sqrt() / std::f32::consts::TAU
        );
        row += &format!(
            "\t{rf:.2}\t{:.2}\t{rr:.2}\t{:.2}",
            radius(true),
            radius(false)
        );
        row += &format!("\t{:.3}\t{:.3}", sunk.0, sunk.1);
        // Its springs' lengths and gravity factor, the sag they give if
        // every spring squeezed alike, and how far its wheels stand
        // below where its model has them (mean).
        let s = spec.tuning.springs.unwrap_or(Springs::DEFAULT);
        row += &format!(
            "\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.3}\t{:.3}",
            s.stretch.0,
            s.stretch.1,
            s.squeeze.0,
            s.squeeze.1,
            s.extra,
            s.gravity_factor,
            s.gravity_factor * 9.81 / (front * kf + rear * kr),
            sunk.2
        );
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
    for _ in 0..(3.0 / DT) as usize {
        p.drive(id, VehicleControls::default());
        p.step(DT);
    }
    (p, id)
}

/// How far below the plane each wheel's tyre is drawn once settled, m
/// (deepest and mean over its wheels; negative: above it): the lowest
/// point of each wheel's circle as drawn, in the world; and how far its
/// wheels stand above where its model has them, mean (negative: below).
fn sunk(spec: &VehicleSpec) -> (f32, f32, f32) {
    let (p, id) = start(spec);
    let Some(m) = p.vehicle_transform(id) else {
        return (f32::NAN, f32::NAN, f32::NAN);
    };
    // `SUNK_AT_MODEL=1`: the wheels where the model has them, as the
    // alpha drew them before its wheels followed their springs.
    let lift = if std::env::var("SUNK_AT_MODEL").is_ok() {
        Vec::new()
    } else {
        p.vehicle_wheel_lift(id)
    };
    let depths: Vec<f32> = spec
        .wheels
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let c = m.transform_point3(w.centre + Vec3::Y * lift.get(i).copied().unwrap_or(0.0));
            // The circle's lowest point in the wheel's plane (axle along
            // the model's x).
            let axle = m.transform_vector3(Vec3::X).normalize();
            let down = (Vec3::NEG_Y - axle * axle.dot(Vec3::NEG_Y)).normalize_or_zero();
            -(c + down * w.radius).y
        })
        .collect();
    let deepest = depths.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    (
        deepest,
        depths.iter().sum::<f32>() / depths.len().max(1) as f32,
        p.vehicle_wheel_lift(id).iter().sum::<f32>() / spec.wheels.len().max(1) as f32,
    )
}

/// Settled, then kicked [`KICK`] m/s down and watched [`WATCH`] s: its
/// sag (m, mean over its wheels), the frequency its height swings with
/// (Hz) and its damping ratio. Kicked from rest, a damped spring's
/// height goes as -exp(-zeta w t) sin(w' t): it first crosses the
/// settled height back at t = pi / w', and the swing past it is the
/// first's times exp(-pi zeta / sqrt(1 - zeta^2)). NaN and 1 when it
/// never swings back past it.
fn bounce(spec: &VehicleSpec) -> (f32, f32, f32) {
    let (mut p, id) = start(spec);
    let sag = p.vehicle_suspension(id);
    let sag = sag.iter().sum::<f32>() / sag.len().max(1) as f32;
    let height = |p: &Physics| p.vehicle_transform(id).map_or(0.0, |m| m.w_axis.y);
    let rest = height(&p);
    let v = p.vehicle_velocity(id);
    p.set_vehicle_velocity(id, v - Vec3::Y * KICK);
    // Its height after each step, the kick at t = 0.
    let mut d = vec![0.0];
    for _ in 0..(WATCH / DT) as usize {
        p.drive(id, VehicleControls::default());
        p.step(DT);
        d.push(height(&p) - rest);
    }
    let Some(i) = (1..d.len()).find(|&i| d[i - 1] < 0.0 && d[i] >= 0.0) else {
        return (sag, f32::NAN, 1.0);
    };
    let cross = i as f32 - 1.0 + d[i - 1] / (d[i - 1] - d[i]);
    let down = d[..i].iter().copied().fold(0.0f32, f32::min).abs();
    // The swing back up: the highest within another half swing.
    let end = (2 * i + 1).min(d.len());
    let up = d[i..end].iter().copied().fold(0.0f32, f32::max);
    if up < NOISE * down {
        return (sag, 0.5 / (cross * DT), 1.0);
    }
    let fall = (down / up).ln();
    let zeta = fall / (std::f32::consts::PI.powi(2) + fall * fall).sqrt();
    (sag, 0.5 / (cross * DT), zeta)
}
