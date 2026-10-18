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

use crate::osprey;
use crate::world::Templates;

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
    fn the_osprey_is_built() {
        assert_eq!(resolve(osprey::MODEL, &HashSet::new()), Some(osprey::MODEL));
        assert!(is_built(osprey::MODEL));
    }
}
