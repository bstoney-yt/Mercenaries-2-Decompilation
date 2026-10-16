//! Drives every wheeled and tracked vehicle template flat out on a level
//! plane, as the alpha's physics drives it, and prints how fast it gets:
//! the time to 50 km/h and to 90% of its template's `MaxSpeed`, its
//! speed after 5, 10 and 20 s and the fastest it went in 40 s, beside
//! what its template's `MaxSpeed` asks for. Then it brakes to a stop,
//! and for a vehicle with an engine and gearbox it prints its changes of
//! gear: how many up and down, how many undone within a second (the
//! gearbox hunting), and how far from its template's `GearUpshiftRPM`
//! and `GearDownshiftRPM` the engine turned when it changed (mean of
//! |rpm - data| / data).
//!
//! ```text
//! cargo run --release -p m2-engine --example drivetest -- "<install dir>"
//! ```
//!
//! Templates sharing a model and the same physics record drive alike, so
//! each such pair is driven once and printed with its template count.
//! Prints numbers and template names only.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_engine::VehicleSpec;
use m2_engine::tuning::{DAMAGE_TABLE, DAMAGE_TABLE_TYPE, DamageTable};
use m2_engine::{CatalogueEntry, CollisionShape, Install, Physics, StaticKey, VehicleControls};
use m2_math::glam::{Quat, Vec3};

const DT: f32 = 1.0 / 60.0;
const SECONDS: f32 = 40.0;
const BRAKING: f32 = 20.0;

/// A model and its physics record: what drives alike.
type Driven = (u32, Vec<u8>);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: drivetest <install dir>");
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

    // (model, physics record) -> (kind, templates)
    let mut groups: BTreeMap<Driven, (&str, Vec<u32>)> = BTreeMap::new();
    let mut templates = 0;
    for (component, kind) in [("_CarPhysicsV2", "car"), ("_TankPhysics", "tank")] {
        let Some(c) = db.component(component) else {
            continue;
        };
        for (id, r) in c.records().unwrap_or_default() {
            templates += 1;
            let Some(&model) = models.get(&id) else {
                continue;
            };
            groups
                .entry((model, r.to_vec()))
                .or_insert((kind, Vec::new()))
                .1
                .push(id);
        }
    }
    println!(
        "{templates} templates, {} with a model, {} kinds driven",
        groups.values().map(|g| g.1.len()).sum::<usize>(),
        groups.len()
    );
    println!(
        "template\tcount\tkind\tmass\tdata km/h\tt50 s\tt90 s\tv5\tv10\tv20\ttop km/h\tof data\tups\tdowns\thunts\tup err\tdown err"
    );
    let (mut driven, mut no_spec, mut no_model) = (0, 0, 0);
    let mut rows = Vec::new();
    for ((model, _), (kind, ids)) in &groups {
        let mut ids = ids.clone();
        ids.sort_by_key(|id| names.get(id).map(|n| n.to_ascii_lowercase()));
        let id = ids[0];
        let name = names.get(&id).copied().unwrap_or("?");
        let Ok(data) = source.model(*model) else {
            no_model += ids.len();
            continue;
        };
        let Ok(m) = data.parse() else {
            no_model += ids.len();
            continue;
        };
        let entry = CatalogueEntry {
            model: *model,
            kind: (*kind).to_owned(),
            label: name.to_owned(),
        };
        let Some(spec) = VehicleSpec::from_model(&entry, &m) else {
            no_spec += ids.len();
            continue;
        };
        let spec = spec.tuned(tuning.get(&id).copied());
        let want = spec.tuning.max_speed;
        let run = drive(&spec);
        driven += ids.len();
        let kmh = |v: f32| format!("{:.0}", v * 3.6);
        let shifts = spec.tuning.engine.map_or("-\t-\t-\t-\t-".to_owned(), |e| {
            run.shifts(e.upshift, e.downshift)
        });
        let at = |t: Option<f32>| t.map_or("-".to_owned(), |t| format!("{t:.1}"));
        let t90 = want.and_then(|w| run.time_to(0.9 * w));
        rows.push(format!(
            "{name}\t{}\t{kind}\t{:.0}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{shifts}",
            ids.len(),
            spec.mass,
            want.map_or("-".to_owned(), kmh),
            at(run.time_to(50.0 / 3.6)),
            at(t90),
            kmh(run.at(5.0)),
            kmh(run.at(10.0)),
            kmh(run.at(20.0)),
            kmh(run.top()),
            want.map_or("-".to_owned(), |w| format!("{:.2}", run.top() / w)),
        ));
    }
    for r in rows {
        println!("{r}");
    }
    println!("driven {driven}, no wheels found {no_spec}, model unread {no_model}");
    Ok(())
}

/// Forward speed, m/s, every step of a full-throttle run; and its gear
/// and engine rpm every step of that run and the stop after it.
struct Run(Vec<f32>, Vec<Option<(usize, f32)>>);

impl Run {
    fn at(&self, t: f32) -> f32 {
        self.0
            .get((t / DT) as usize)
            .or(self.0.last())
            .copied()
            .unwrap_or(0.0)
    }
    fn time_to(&self, v: f32) -> Option<f32> {
        self.0.iter().position(|&s| s >= v).map(|i| i as f32 * DT)
    }
    fn top(&self) -> f32 {
        self.0.iter().copied().fold(0.0, f32::max)
    }
    /// Its changes of gear: up, down, undone within a second, and the
    /// mean error of the rpm it changed up and down at against `up` and
    /// `down`.
    fn shifts(&self, up: f32, down: f32) -> String {
        let mut changes = Vec::new();
        for (i, w) in self.1.windows(2).enumerate() {
            if let [Some((a, rpm)), Some((b, _))] = w
                && a != b
            {
                changes.push((i, *b > *a, *rpm));
            }
        }
        let second = (1.0 / DT) as usize;
        let hunts = changes
            .windows(2)
            .filter(|w| w[0].1 != w[1].1 && w[1].0 - w[0].0 <= second)
            .count();
        let err = |rising: bool, want: f32| {
            let e: Vec<f32> = changes
                .iter()
                .filter(|c| c.1 == rising)
                .map(|c| (c.2 - want).abs() / want.max(1.0))
                .collect();
            if e.is_empty() {
                "-".to_owned()
            } else {
                format!("{:.3}", e.iter().sum::<f32>() / e.len() as f32)
            }
        };
        let ups = changes.iter().filter(|c| c.1).count();
        format!(
            "{ups}\t{}\t{hunts}\t{}\t{}",
            changes.len() - ups,
            err(true, up),
            err(false, down)
        )
    }
}

fn drive(spec: &VehicleSpec) -> Run {
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
    let (mut out, mut gears) = (Vec::new(), Vec::new());
    for _ in 0..(SECONDS / DT) as usize {
        p.drive(
            id,
            VehicleControls {
                throttle: 1.0,
                ..Default::default()
            },
        );
        p.step(DT);
        out.push(p.vehicle_speed(id));
        gears.push(p.vehicle_gear(id));
    }
    // Brakes to a stop: throttle against the way it rolls.
    for _ in 0..(BRAKING / DT) as usize {
        let rolling = p.vehicle_speed(id) > 0.5;
        p.drive(
            id,
            VehicleControls {
                throttle: if rolling { -1.0 } else { 0.0 },
                ..Default::default()
            },
        );
        p.step(DT);
        gears.push(p.vehicle_gear(id));
    }
    Run(out, gears)
}
