//! Measures what every weapon's firing sounds like in the alpha (round 12
//! of the gap fills): over every weapon template, whether its `SoundSet`
//! reaches a sound the archive holds (a spinning gun's loop, else its
//! shot, as the alpha plays one); then over every vehicle template the
//! database gives physics, a model and a muzzle, each seat the player can
//! take and both fire buttons, what the press fires and whether it is
//! heard with its weapon's own sound, the alpha's stand-in, or not at all.
//!
//! ```text
//! cargo run --release -p m2-engine --example firesounds -- "<install dir>" [before]
//! ```
//!
//! `before` counts as the alpha did before round 12: only bullet guns
//! played their own sound; launchers, lobbers, missiles and cannons the
//! alpha's own; a coaxial gun its template doesn't mount the rifle's.
//! The dispatch mirrors the alpha's `Combat::mounted_fire` and
//! `vehicle_fire`. Prints numbers, hashes and names only.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use m2_assets::name_hash;
use m2_engine::audio::{KeySounds, world_cue_tables};
use m2_engine::gun_sounds::GunCues;
use m2_engine::weapons::{Flight, MachineGun, WeaponSpec, machine_gun};
use m2_engine::world::{VehicleKind, WorldIndex};
use m2_engine::{CatalogueEntry, Install, VehicleSpec};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(root) = args.first() else {
        eprintln!("usage: firesounds <install dir> [before]");
        std::process::exit(2);
    };
    let before = args.get(1).is_some_and(|a| a == "before");
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let world = WorldIndex::build(&mut source)?;
    let db = &world.template_db;
    let cues = GunCues::read(&install)?;
    let tables = world_cue_tables(&mut source);
    let mut resolved: HashMap<u32, bool> = HashMap::new();
    // Whether a weapon's sound set reaches a sound: a spinning gun's loop,
    // else its shot (the alpha's `play_shot`).
    let mut heard = |set: Option<u32>, source: &mut _| -> bool {
        let Some(cue) = set.and_then(|s| cues.spin(s, false).or_else(|| cues.shot(s, false)))
        else {
            return false;
        };
        *resolved
            .entry(cue)
            .or_insert_with(|| KeySounds::resolve(source, &tables, cue).is_some())
    };

    let mut weapons: Vec<&WeaponSpec> = db.arms.weapons.values().collect();
    weapons.sort_by(|a, b| a.name.cmp(&b.name).then(a.template.cmp(&b.template)));
    // By flight: templates, with a sound set, heard as their own.
    let mut by_flight: BTreeMap<String, [usize; 3]> = BTreeMap::new();
    println!("weapon\tflight\tsound set\town sound");
    for w in &weapons {
        let reaches = heard(w.feel.sound_set, &mut source);
        let own = reaches && (!before || w.flight() == Flight::Round);
        let row = by_flight.entry(format!("{:?}", w.flight())).or_default();
        row[0] += 1;
        row[1] += usize::from(w.feel.sound_set.is_some());
        row[2] += usize::from(own);
        println!(
            "{}\t{:?}\t{}\t{own}",
            w.name,
            w.flight(),
            w.feel
                .sound_set
                .map_or("-".to_owned(), |s| format!("{s:08x}"))
        );
    }
    let total: usize = by_flight.values().map(|v| v[2]).sum();
    println!(
        "weapons: {total}/{} templates fire with their own sound; by flight (templates, with a \
         sound set, own sound): {}",
        weapons.len(),
        by_flight
            .iter()
            .map(|(k, v)| format!("{k} {}/{}/{}", v[0], v[1], v[2]))
            .collect::<Vec<_>>()
            .join(", ")
    );

    let driver = name_hash("seat_driver");
    let mut ids: Vec<(&u32, &String)> = db.names.iter().collect();
    ids.sort_by_key(|a| a.1.to_ascii_lowercase());
    let mut models = HashMap::new();
    let mut presses: BTreeMap<&str, usize> = BTreeMap::new();
    let mut vehicles = 0;
    println!("template\tseat\tbutton\tfires\tweapon\theard as");
    for (&id, name) in ids {
        let Some(info) = db.infos.get(&id) else {
            continue;
        };
        let (Some(kind), Some(model)) = (info.vehicle, info.model) else {
            continue;
        };
        let kind = match kind {
            VehicleKind::Tank => "tank",
            VehicleKind::Boat => m2_engine::vehicle::BOAT,
            VehicleKind::Helicopter => m2_engine::vehicle::HELICOPTER,
            VehicleKind::Car => "car",
            VehicleKind::Unknown => continue,
        };
        let data = models
            .entry(model)
            .or_insert_with(|| source.model(model).ok());
        let Some(m) = data.as_ref().and_then(|d| d.parse().ok()) else {
            continue;
        };
        let entry = CatalogueEntry {
            model,
            kind: kind.to_owned(),
            label: name.clone(),
        };
        let Some(spec) = VehicleSpec::from_model(&entry, &m) else {
            continue;
        };
        let spec = spec
            .tuned(db.tuning_for(name, model))
            .armed_with(db.armament_for(name, model), &db.arms);
        if spec.muzzle.is_none() && spec.armed.is_empty() {
            continue;
        }
        vehicles += 1;
        let cannon = spec.cannon();
        // The seats she can take: the driver's, then each that fires.
        let mut seats = vec![driver];
        for m in &spec.armed {
            if !seats.contains(&m.seat) {
                seats.push(m.seat);
            }
        }
        for (k, &seat) in seats.iter().enumerate() {
            for priority in 0..2 {
                let mounted = (!(k == 0 && cannon))
                    .then(|| {
                        spec.armed
                            .iter()
                            .find(|m| m.seat == seat && m.priority == priority)
                    })
                    .flatten();
                let coax = priority == 1;
                let (fires, weapon, how): (&str, String, &str) = if let Some(m) = mounted {
                    let w = &m.weapon;
                    let reaches = heard(w.feel.sound_set, &mut source);
                    let own = reaches && (!before || w.flight() == Flight::Round);
                    (
                        "mounted",
                        w.name.clone(),
                        if own { "own" } else { "stand-in" },
                    )
                } else if spec.muzzle.is_none() || (coax && !cannon) {
                    ("nothing", "-".to_owned(), "none")
                } else if cannon && !coax {
                    let w = spec.tuning.gun.and_then(|g| db.arms.weapons.get(&g.weapon));
                    let reaches = w.is_some_and(|w| heard(w.feel.sound_set, &mut source));
                    (
                        "cannon",
                        w.map_or("-".to_owned(), |w| w.name.clone()),
                        if reaches && !before {
                            "own"
                        } else {
                            "stand-in"
                        },
                    )
                } else {
                    match machine_gun(&spec.armed) {
                        MachineGun::Mounted(w) => {
                            let own = heard(w.feel.sound_set, &mut source);
                            (
                                "machine gun",
                                w.name.clone(),
                                if own { "own" } else { "stand-in" },
                            )
                        }
                        MachineGun::Unmounted if !before => ("nothing", "-".to_owned(), "none"),
                        _ => ("machine gun", "stand-in".to_owned(), "stand-in"),
                    }
                };
                *presses.entry(how).or_default() += 1;
                println!(
                    "{name}\t{seat:08x}\t{}\t{fires}\t{weapon}\t{how}",
                    if coax { "aim" } else { "fire" }
                );
            }
        }
    }
    let all: usize = presses.values().sum();
    println!(
        "vehicles: {vehicles} armed vehicle templates, {all} seat-button presses: {}",
        presses
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(())
}
