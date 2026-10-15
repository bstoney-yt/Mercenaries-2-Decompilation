//! Lists every vehicle physics component in the template database with
//! its fields (named where a guessed name hashes to the key) and each
//! template's values, so the alpha's handling can be checked against
//! the data over all vehicle templates.
//!
//! ```text
//! cargo run --release -p m2-engine --example vehiclefeel -- "<install dir>" [component-word..]
//! ```
//!
//! Without words it lists the components whose names hold `Physics`,
//! `Car`, `Tank`, `Boat`, `Bike`, `Wheel`, `Engine`, `Gear` or `Vehicle`.
//! Prints numbers, hashes and names only.

use std::collections::HashMap;
use std::path::Path;

use m2_assets::layer::{Layer, TEMPLATE_DB_TYPE};
use m2_assets::name_hash;
use m2_engine::Install;

/// Guessed field names: whichever hash to a field are printed by name.
const GUESSES: &[&str] = &[
    "MaxSpeed",
    "MinSpeed",
    "TopSpeed",
    "MaxReverseSpeed",
    "ReverseSpeed",
    "Mass",
    "Brake",
    "BrakeForce",
    "BrakeTorque",
    "HandBrake",
    "HandbrakeForce",
    "SteerSpeed",
    "SteerInertia",
    "SteerAngle",
    "MaxSteerAngle",
    "MaxSteer",
    "SteerReturn",
    "SpringStrength",
    "SpringDamp",
    "SpringLength",
    "SideFriction",
    "Friction",
    "FrontFriction",
    "RearFriction",
    "GravityFactor",
    "EngineTorque",
    "EngineInertia",
    "EngineOffsetY",
    "Torque",
    "MaxTorque",
    "MaxRPM",
    "MinRPM",
    "IdleRPM",
    "RedlineRPM",
    "Acceleration",
    "Accel",
    "MaxAccel",
    "Deceleration",
    "Gears",
    "NumGears",
    "GearRatio",
    "FinalDrive",
    "Drag",
    "AirDrag",
    "RollingResistance",
    "Downforce",
    "WheelRadius",
    "WheelMass",
    "Suspension",
    "SuspensionLength",
    "SuspensionStrength",
    "SuspensionDamping",
    "CenterOfMass",
    "CentreOfMass",
    "COMOffset",
    "InertiaScale",
    "TurnRate",
    "TurnSpeed",
    "MaxTurnSpeed",
    "Speed",
    "Name",
    "Template",
    "Model",
    "BoostSpeed",
    "Boost",
    "MaxPitch",
    "MaxRoll",
    "Buoyancy",
    "Thrust",
    "MaxThrust",
    "LinearDamping",
    "AngularDamping",
    "Grip",
    "TractionScale",
    "SlipRatio",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: vehiclefeel <install dir> [component-word..]");
        std::process::exit(2);
    };
    let words: Vec<String> = if args.len() > 1 {
        args[1..].to_vec()
    } else {
        [
            "Physics", "Car", "Tank", "Boat", "Bike", "Wheel", "Engine", "Gear", "Vehicle",
        ]
        .map(str::to_owned)
        .to_vec()
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
    let guess: HashMap<u32, &str> = GUESSES.iter().map(|g| (name_hash(g), *g)).collect();
    let template = |t: u32| {
        names
            .get(&t)
            .map_or(format!("{t:08x}"), |n| (*n).to_owned())
    };

    for c in &db.components {
        if !words.iter().any(|w| c.name.contains(w.as_str())) {
            continue;
        }
        let records = c.records().unwrap_or_default();
        println!(
            "== {} ({} records, {} fields, {} bytes each)",
            c.name,
            records.len(),
            c.fields.len(),
            c.record_size
        );
        let mut fields = c.fields.clone();
        fields.sort_by_key(|f| f.offset);
        let label = |h: u32| {
            guess
                .get(&h)
                .map_or(format!("{h:08x}"), |g| (*g).to_owned())
        };
        println!(
            "template\t{}",
            fields
                .iter()
                .map(|f| format!("{}@{:x}:{:08x}", label(f.name_hash), f.offset, f.ty))
                .collect::<Vec<_>>()
                .join("\t")
        );
        let mut rows: Vec<(String, Vec<String>)> = records
            .iter()
            .map(|(id, r)| {
                let vals = fields
                    .iter()
                    .map(|f| {
                        let at = usize::from(f.offset);
                        let Some(b) = r.get(at..at + 4) else {
                            return "-".to_owned();
                        };
                        let w = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                        let w = if (1..32).contains(&f.bit_width) {
                            (w >> f.bit_shift) & ((1 << f.bit_width) - 1)
                        } else {
                            w
                        };
                        let x = f32::from_bits(w);
                        if w == 0 {
                            "0".to_owned()
                        } else if x.is_normal() && x.abs() < 1e7 && x.abs() > 1e-6 {
                            format!("{x}")
                        } else {
                            names.get(&w).map_or(format!("#{w:x}"), |n| format!("'{n}"))
                        }
                    })
                    .collect();
                (template(*id), vals)
            })
            .collect();
        rows.sort();
        for (t, v) in rows {
            println!("{t}\t{}", v.join("\t"));
        }
    }
    Ok(())
}
