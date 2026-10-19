//! Models that stand in for ones the templates name but the archives
//! don't hold: cut content brought back.
//!
//! The cut content survey (2026-10-18) found templates whose `ModelName`
//! is in none of the PC archives. Two of them come back here:
//!
//! - `DLC_M1A1` names `0x7ee94b4e`; it drives as the M1A2
//!   (`al_veh_tank_m1a2`, `0xce77d4cb`). From samples: its `LocalizedName`
//!   key is `vehicle.m1a2`, it mounts the M1A2's weapons on the M1A2's
//!   hardpoints (`Mounted Minigun (No model) (M1A2)`, `hijack_m1a2`), and
//!   the textures named `al_veh_tank_m1a1_*` sit in the M1A2 model's own
//!   block. That it was meant to look like the M1A2 is inferred.
//! - The V-22 Osprey's three templates name `global_veh_v22osprey`
//!   (`0x2e7cb0b5`), which nothing holds; [`crate::osprey`] builds one.
//! - `DLC_50cal`, a hand weapon (`Equipment` class Rifle, firing
//!   `Autocannon Shell (AL)` 400 a minute), names `0x9221ac1c`; it is
//!   carried as the DShK (`global_weapon_dshk`), the heavy machine gun
//!   the island's `Human Heavy MG` already carries. That stand-in is
//!   invented.
//!
//! The cut hand weapons (`Coilgun`, `Riot Gun`, `Cheat RPG`, `DLC_50cal`)
//! keep their `Equipment` and `Pickup` records, but no script, layer or
//! shop entry hands them out. [`add_supply_drops`] makes a supply crate
//! for each, shaped as one of the game's own crates and leaving the
//! weapon when broken open, for the support catalogue to drop (the crates
//! are invented; the game's own crates hold the weapons they hold).

use std::collections::HashSet;

use m2_assets::name_hash;

use crate::osprey;
use crate::weapons::{Armament, Blast, Mount};
use crate::world::Templates;

/// The Osprey's templates.
pub const OSPREY_TEMPLATES: [&str; 3] = ["V22 (Delivery)", "V22 (Driver)", "V22 - DO NOT USE"];

/// Whose damage the Osprey takes as: the Ka-29, the island's other
/// armed troop transport (`Health` 300, burning below 60, light armour),
/// in place of the V22 templates' own 100 and no damage key. Brendan's
/// call (2026-10-18): a Mercs 2 aircraft's, not another game's.
const DAMAGE_DONOR: &str = "Ka29b (Driver)";

/// The weapons its pilot fires, the game's own Allied helicopter weapons
/// (the Coanda Gunship's): homing AT missiles on Fire, the remote
/// miniguns on Aim. Which mounts and where are invented.
const MISSILE: &str = "Mounted AT Missile (1) (hp_barreltip_missileL_a)";
const GUN: &str = "Mounted Minigun (Remote) (AL)";

/// What its bomb bay drops: the bombing run's own bomb.
const BOMB: &str = "Dumb Bomb Projectile";

/// Arms the Osprey's templates and has them take damage as
/// [`DAMAGE_DONOR`] does, keeping their own flight handling.
pub fn arm_the_osprey(templates: &mut Templates) {
    let donor = templates.id(DAMAGE_DONOR);
    let armament = osprey_armament(templates);
    for name in OSPREY_TEMPLATES {
        for id in templates.ids(name) {
            if let Some(a) = &armament {
                templates.arms.armaments.insert(id, a.clone());
            }
            let Some(d) = donor else { continue };
            if let Some(dt) = templates.tuning.get(&d).copied() {
                let t = templates.tuning.entry(id).or_default();
                t.health = dt.health;
                t.armour = dt.armour;
                t.blast_scale = dt.blast_scale;
                t.burns_below = dt.burns_below;
                t.rounds = dt.rounds;
                t.key = dt.key;
            }
            if let Some(&h) = templates.health.get(&d) {
                templates.health.insert(id, h);
            }
            if let Some(&m) = templates.modifiers.get(&d) {
                templates.modifiers.insert(id, m);
            }
            if let Some(dur) = templates.durability.get(&d).cloned() {
                templates.durability.insert(id, dur);
            }
        }
    }
}

/// The pilot's armament: missiles alternating between the rails, guns
/// between the chin mounts; `None` if the database lacks the weapons.
fn osprey_armament(templates: &Templates) -> Option<Armament> {
    let (missile, gun) = (templates.id(MISSILE)?, templates.id(GUN)?);
    let [ml, mr, gl, gr] = osprey::hardpoints();
    let link = |n: &str| name_hash(n);
    let mounts = vec![
        Mount {
            link: link("v22_missile_l"),
            weapon: missile,
            hardpoint: ml,
        },
        Mount {
            link: link("v22_missile_r"),
            weapon: missile,
            hardpoint: mr,
        },
        Mount {
            link: link("v22_gun_l"),
            weapon: gun,
            hardpoint: gl,
        },
        Mount {
            link: link("v22_gun_r"),
            weapon: gun,
            hardpoint: gr,
        },
    ];
    Some(Armament {
        mounts,
        couplings: vec![
            (link("v22_missile_l"), link("v22_missile_r")),
            (link("v22_gun_l"), link("v22_gun_r")),
        ],
        seats: vec![(
            name_hash("seat_driver"),
            vec![link("v22_missile_l"), link("v22_gun_l")],
        )],
    })
}

/// What the Osprey's bombs do where they land: the blast the database's
/// [`BOMB`] leaves (its reach, damage and push). Its `DamageKey` is not
/// kept with the ordnance, so the blast is taken in full by all it
/// reaches.
pub fn bomb_blast(templates: &Templates) -> Option<Blast> {
    let bomb = templates.id(BOMB)?;
    let o = &templates.ordnance;
    let blast = *o.blasts_of(bomb).first()?;
    let b = o.blasts.get(&blast)?;
    Some(Blast {
        template: blast,
        radius: b.radius,
        damage: b.damage,
        min_damage: b.min_damage,
        max_force: b.force,
        key: None,
    })
}

/// Template models no archive holds, and the archive model drawn in their
/// place.
const SUBSTITUTES: [(u32, u32); 2] = [
    // DLC_M1A1 -> al_veh_tank_m1a2.
    (0x7ee9_4b4e, 0xce77_d4cb),
    // DLC_50cal -> global_weapon_dshk.
    (0x9221_ac1c, 0xbb03_4506),
];

/// The supply crates that bring the cut hand weapons back: the crate's
/// name, the game's crate it is shaped as, and the weapon template it
/// leaves.
pub const SUPPLY_DROPS: [(&str, &str, &str); 4] = [
    (
        "Supply Drop (Coilgun)",
        "Supply Drop (Sniper RU)",
        "Coilgun",
    ),
    ("Supply Drop (Riot Gun)", "Supply Drop (CQB)", "Riot Gun"),
    ("Supply Drop (Cheat RPG)", "Supply Drop (RPG)", "Cheat RPG"),
    (
        "Supply Drop (50 Cal)",
        "Supply Drop (Light MG)",
        "DLC_50cal",
    ),
];

/// The first id the supply crates take: above every template id the
/// database uses (they run from `0x80004000` to about `0x8000b000`).
const FIRST_DROP: u32 = 0xfee0_0001;

/// Gives the cut weapons their stand-in models and adds the
/// [`SUPPLY_DROPS`] crates to `templates`, each as the crate it is
/// shaped as but leaving its weapon. A crate whose weapon or model
/// crate the database lacks is left out.
pub fn add_supply_drops(templates: &mut Templates) {
    for w in templates.arms.weapons.values_mut() {
        if let Some(&(_, standin)) = SUBSTITUTES.iter().find(|(cut, _)| Some(*cut) == w.model) {
            w.model = Some(standin);
        }
    }
    for (k, (name, shape, weapon)) in SUPPLY_DROPS.into_iter().enumerate() {
        let id = FIRST_DROP + k as u32;
        let (Some(shape_id), Some(weapon_id)) = (templates.id(shape), templates.id(weapon)) else {
            continue;
        };
        if templates.names.contains_key(&id) || templates.id(name).is_some() {
            continue;
        }
        let Some(info) = templates.by_id(shape_id).map(|(_, i)| *i) else {
            continue;
        };
        templates.names.insert(id, name.to_owned());
        templates.infos.insert(id, info);
        templates.by_name.insert(name.to_ascii_lowercase(), info);
        templates.leavings.spawns.insert(id, vec![weapon_id]);
        if let Some(&v) = templates.sound_keys.get(&shape_id) {
            templates.sound_keys.insert(id, v);
        }
        if let Some(d) = templates.durability.get(&shape_id).cloned() {
            templates.durability.insert(id, d);
        }
        if let Some(&m) = templates.piece_masses.get(&shape_id) {
            templates.piece_masses.insert(id, m);
        }
    }
}

/// The model a template naming `model` draws with, given the models the
/// archive holds: its own, else its stand-in from the archive, else a
/// model built in code ([`is_built`]); `None` when there is none.
pub fn resolve(model: u32, held: &HashSet<u32>) -> Option<u32> {
    if held.contains(&model) || is_built(model) {
        return Some(model);
    }
    SUBSTITUTES
        .iter()
        .find(|&&(cut, _)| cut == model)
        .map(|&(_, standin)| standin)
        .filter(|s| held.contains(s))
}

/// Whether `model` is one built in code rather than read from an archive.
pub fn is_built(model: u32) -> bool {
    osprey::is_osprey(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_models_stand_for_themselves() {
        let held = HashSet::from([1, 0xce77_d4cb]);
        assert_eq!(resolve(1, &held), Some(1));
        assert_eq!(resolve(2, &held), None);
    }

    #[test]
    fn the_dlc_m1a1_draws_as_the_m1a2_when_the_archive_has_it() {
        let held = HashSet::from([0xce77_d4cb]);
        assert_eq!(resolve(0x7ee9_4b4e, &held), Some(0xce77_d4cb));
        assert_eq!(resolve(0x7ee9_4b4e, &HashSet::new()), None);
    }

    #[test]
    fn supply_drops_are_shaped_as_game_crates_and_leave_the_cut_weapons() {
        use crate::world::TemplateInfo;
        let mut t = Templates::default();
        let crate_id = 0x8000_0010;
        t.names.insert(crate_id, "Supply Drop (Sniper RU)".into());
        let info = TemplateInfo {
            model: Some(0x1234),
            vehicle: None,
            engine: None,
            person: false,
            human: false,
        };
        t.infos.insert(crate_id, info);
        t.by_name.insert("supply drop (sniper ru)".into(), info);
        t.names.insert(0x8000_8448, "Coilgun".into());
        add_supply_drops(&mut t);
        let id = t.id("Supply Drop (Coilgun)").expect("the coilgun's crate");
        assert_eq!(
            t.get("Supply Drop (Coilgun)").and_then(|i| i.model),
            Some(0x1234)
        );
        assert_eq!(t.leavings.spawns[&id], [0x8000_8448]);
        // The crates whose weapon or shape the database lacks are left out.
        assert!(t.id("Supply Drop (Riot Gun)").is_none());
        // Run twice, nothing doubles.
        add_supply_drops(&mut t);
        assert_eq!(t.ids("Supply Drop (Coilgun)").len(), 1);
    }

    #[test]
    fn the_osprey_fires_missiles_and_guns_and_takes_damage_as_the_ka29() {
        use crate::tuning::VehicleTuning;
        let mut t = Templates::default();
        let (v22, ka29, missile, gun) = (0x8000_75e9, 0x8000_81ff, 0x8000_9523, 0x8000_38e6);
        for (id, name) in [
            (v22, "V22 (Driver)"),
            (ka29, DAMAGE_DONOR),
            (missile, MISSILE),
            (gun, GUN),
        ] {
            t.names.insert(id, name.into());
        }
        t.tuning.insert(
            ka29,
            VehicleTuning {
                health: Some(300.0),
                burns_below: Some(60.0),
                armour: Some(2),
                ..VehicleTuning::default()
            },
        );
        t.tuning.insert(
            v22,
            VehicleTuning {
                health: Some(100.0),
                mass: Some(9.0),
                ..VehicleTuning::default()
            },
        );
        t.health.insert(ka29, 300.0);
        arm_the_osprey(&mut t);
        // The Ka-29's damage, its own handling.
        let tuned = t.tuning[&v22];
        assert_eq!(tuned.health, Some(300.0));
        assert_eq!(tuned.burns_below, Some(60.0));
        assert_eq!(tuned.armour, Some(2));
        assert_eq!(tuned.mass, Some(9.0));
        assert_eq!(t.health[&v22], 300.0);
        // Missiles on Fire, guns on Aim, each pair fired in turn from the
        // Osprey's own hardpoints.
        let a = &t.arms.armaments[&v22];
        let fired = a.of_seat(name_hash("seat_driver"));
        assert_eq!(fired.len(), 2);
        let first = a.group(fired[0]);
        assert_eq!(first.len(), 2);
        assert!(first.iter().all(|m| m.weapon == missile));
        assert!(a.group(fired[1]).iter().all(|m| m.weapon == gun));
        let points: Vec<u32> = a.mounts.iter().map(|m| m.hardpoint).collect();
        assert_eq!(points, osprey::hardpoints());
    }

    #[test]
    fn the_bombs_burst_as_the_bombing_runs_do() {
        use crate::ordnance::{BlastInfo, ProjectileInfo};
        let mut t = Templates::default();
        let (bomb, boom) = (0x8000_0100, 0x8000_0101);
        t.names.insert(bomb, BOMB.into());
        t.ordnance.projectiles.insert(
            bomb,
            ProjectileInfo {
                leaves: vec![boom],
                ..ProjectileInfo::default()
            },
        );
        t.ordnance.blasts.insert(
            boom,
            BlastInfo {
                radius: 12.0,
                damage: 400.0,
                min_damage: 100.0,
                force: 5.0,
                ..BlastInfo::default()
            },
        );
        let b = bomb_blast(&t).expect("a blast");
        assert_eq!(
            (b.template, b.radius, b.damage, b.min_damage),
            (boom, 12.0, 400.0, 100.0)
        );
        assert!(bomb_blast(&Templates::default()).is_none());
    }

    #[test]
    fn the_osprey_is_built() {
        assert_eq!(resolve(osprey::MODEL, &HashSet::new()), Some(osprey::MODEL));
        assert!(is_built(osprey::MODEL));
    }
}
