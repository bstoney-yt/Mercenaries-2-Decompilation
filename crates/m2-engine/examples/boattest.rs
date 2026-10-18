//! Floats every boat template on open water, as the alpha's physics
//! carries it, and prints how it goes beside what its template's
//! `_BoatPhysics` says (gap round 17): its top speed flat out ahead and
//! backing up (`MaxSpeed`, `MaxSpeedReverse`, km/h), how long it takes
//! to reach 90% of its top speed, and how quickly the water slows it
//! coasting, braking and sliding sideways, per second (`WaterDragFwd`,
//! `WaterDragFwd` + `BrakeDrag`, `WaterDragSide`).
//!
//! ```text
//! cargo run --release -p m2-engine --example boattest -- "<install dir>"
//! ```
//!
//! Each slowing is measured from 8 m/s over a second with nothing else
//! pushing: ln(v0 / v1) / t. Templates sharing a model and physics
//! record go alike, so each pair is floated once. Prints numbers and
//! template names only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::name_hash;
use m2_engine::tuning::{DAMAGE_TABLE, DAMAGE_TABLE_TYPE, DamageTable};
use m2_engine::{CatalogueEntry, Install, Physics, VehicleControls, VehicleId, VehicleSpec};
use m2_math::glam::Vec3;

const DT: f32 = 1.0 / 60.0;
/// How long it goes flat out ahead, and backing up, seconds.
const AHEAD: f32 = 90.0;
const ASTERN: f32 = 40.0;
/// The speed each slowing is measured from, m/s, and for how long, s.
const FROM: f32 = 8.0;
const OVER: f32 = 1.0;

type Floated = (u32, Vec<u8>);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: boattest <install dir>");
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

    let c = db.component("_BoatPhysics").ok_or("no _BoatPhysics")?;
    let at = |name: &str| {
        c.fields
            .iter()
            .find(|f| f.name_hash == name_hash(name))
            .map(|f| usize::from(f.offset))
    };
    let fields = [
        at("MaxSpeed"),
        at("MaxSpeedReverse"),
        at("WaterDragFwd"),
        at("BrakeDrag"),
        at("WaterDragSide"),
        at("EngineTorque"),
        at("EngineReverseTorque"),
    ];
    let mut groups: BTreeMap<Floated, Vec<u32>> = BTreeMap::new();
    for (id, r) in c.records().unwrap_or_default() {
        if let Some(&model) = models.get(&id) {
            groups.entry((model, r.to_vec())).or_default().push(id);
        }
    }
    println!(
        "template\tcount\tmass\ttorque\treverse torque\t\
         data top\ttop\tt90 s\tdata reverse\treverse\t\
         data coast\tcoast\tdata brake\tbrake\tdata side\tside"
    );
    let (mut floated, mut skipped) = (0, 0);
    for ((model, record), ids) in &groups {
        let mut ids = ids.clone();
        ids.sort_by_key(|id| names.get(id).map(|n| n.to_ascii_lowercase()));
        let id = ids[0];
        let name = names.get(&id).copied().unwrap_or("?");
        let Some(spec) = source.model(*model).ok().and_then(|d| {
            let m = d.parse().ok()?;
            let entry = CatalogueEntry {
                model: *model,
                kind: "boat".to_owned(),
                label: name.to_owned(),
            };
            VehicleSpec::from_model(&entry, &m)
        }) else {
            skipped += ids.len();
            continue;
        };
        let spec = spec.tuned(tuning.get(&id).copied());
        if !spec.is_boat() {
            skipped += ids.len();
            continue;
        }
        floated += ids.len();
        let f = |at: Option<usize>| {
            at.and_then(|a| record.get(a..a + 4))
                .map_or(f32::NAN, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let [top, reverse, fwd, brake, side, torque, back] = fields.map(f);
        let (top, reverse) = (top / 3.6, reverse / 3.6);
        let (got_top, t90) = flat_out(&spec, 1.0, AHEAD, top);
        let (got_reverse, _) = flat_out(&spec, -1.0, ASTERN, reverse);
        let coast = slowing(&spec, Vec3::Z, false);
        let braking = slowing(&spec, Vec3::Z, true);
        let sliding = slowing(&spec, Vec3::X, false);
        println!(
            "{name}\t{}\t{}\t{torque}\t{back}\t{top:.2}\t{got_top:.2}\t{t90:.1}\t\
             {reverse:.2}\t{got_reverse:.2}\t{fwd:.3}\t{coast:.3}\t{:.3}\t{braking:.3}\t\
             {side:.3}\t{sliding:.3}",
            ids.len(),
            spec.mass,
            fwd + brake,
        );
    }
    println!("floated {floated}, skipped {skipped}");
    Ok(())
}

/// Open water at 0 m and the boat settled on it, facing +z.
fn start(spec: &VehicleSpec) -> (Physics, VehicleId) {
    let mut p = Physics::new();
    p.set_water(0.0);
    let id = p.add_vehicle(spec, Vec3::new(0.0, 0.5, 0.0), 0.0);
    for _ in 0..(5.0 / DT) as usize {
        p.drive(id, VehicleControls::default());
        p.step(DT);
    }
    (p, id)
}

/// Its speed along its heading, m/s (negative backing up).
fn along(p: &Physics, id: VehicleId) -> f32 {
    let ahead = p
        .vehicle_transform(id)
        .map_or(Vec3::Z, |m| m.transform_vector3(Vec3::Z).with_y(0.0))
        .normalize_or(Vec3::Z);
    p.vehicle_velocity(id).dot(ahead)
}

/// Throttle held at `throttle` for `secs`: the fastest it went that way,
/// m/s, and when it first reached 90% of `want`, s (NaN if never).
fn flat_out(spec: &VehicleSpec, throttle: f32, secs: f32, want: f32) -> (f32, f32) {
    let (mut p, id) = start(spec);
    let (mut best, mut t90) = (0.0f32, f32::NAN);
    for i in 0..(secs / DT) as usize {
        p.drive(
            id,
            VehicleControls {
                throttle,
                ..VehicleControls::default()
            },
        );
        p.step(DT);
        let v = along(&p, id) * throttle.signum();
        best = best.max(v);
        if t90.is_nan() && v >= 0.9 * want {
            t90 = (i + 1) as f32 * DT;
        }
    }
    (best, t90)
}

/// Set going [`FROM`] m/s along `way` (its own axes) and left alone, or
/// braking, for [`OVER`] s: how quickly it slowed, per second.
fn slowing(spec: &VehicleSpec, way: Vec3, brake: bool) -> f32 {
    let (mut p, id) = start(spec);
    let world = p
        .vehicle_transform(id)
        .map_or(way, |m| m.transform_vector3(way).with_y(0.0))
        .normalize_or(way);
    let v = p.vehicle_velocity(id);
    p.set_vehicle_velocity(id, v.with_x(0.0).with_z(0.0) + world * FROM);
    for _ in 0..(OVER / DT) as usize {
        p.drive(
            id,
            VehicleControls {
                brake,
                ..VehicleControls::default()
            },
        );
        p.step(DT);
    }
    let left = p.vehicle_velocity(id).dot(world).max(1e-3);
    (FROM / left).ln() / OVER
}
