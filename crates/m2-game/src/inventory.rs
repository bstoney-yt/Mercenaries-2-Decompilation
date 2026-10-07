//! What people carry, as the scripts see it: weapon objects
//! (`Human.Inventory`, `Weapon`). A contract hands its hero a loadout of
//! templates (VzaCon001: `Carbine`, `Grenade`, `C4`), each becoming a
//! weapon object with its own reserve of ammunition, empty until a script
//! fills it to its template's `MaxAmmoReserve`; saves keep each weapon's
//! template (`Object.GetParent`) and reserve.

use std::collections::{HashMap, HashSet};

use m2_assets::{Layer, name_hash};

/// A weapon someone carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Carried {
    pub owner: u32,
    /// Its template's guid.
    pub template: u32,
    /// Ammunition in reserve.
    pub reserve: u32,
}

/// Every character's weapons and what the data says of their templates.
#[derive(Clone, Debug, Default)]
pub struct Inventory {
    /// Each character's weapons, in the order its loadout gave them.
    pub weapons: HashMap<u32, Vec<u32>>,
    /// Each weapon object.
    pub carried: HashMap<u32, Carried>,
    /// Templates' `MaxAmmoReserve` by name in lower case.
    pub max_reserve: HashMap<String, u32>,
    /// Templates that are thrown (`WeaponThrown`: grenades, C4), by name
    /// in lower case: not a primary or secondary weapon.
    pub thrown: HashSet<String>,
}

/// The template database's weapons: each one's `MaxAmmoReserve` (fired
/// weapons' `WeaponProjectileBase` and thrown ones' `WeaponThrown`) and
/// which are thrown, by template name in lower case.
pub fn read(db: &Layer<'_>) -> (HashMap<String, u32>, HashSet<String>) {
    let names = db.names();
    let field = name_hash("MaxAmmoReserve");
    let mut reserves = HashMap::new();
    let mut thrown = HashSet::new();
    for component in ["WeaponProjectileBase", "WeaponThrown"] {
        let Some(c) = db.component(component) else {
            continue;
        };
        let Some(f) = c.fields.iter().find(|f| f.name_hash == field) else {
            continue;
        };
        // A field ends where the next begins (the carbine's is two bytes,
        // a flag after it).
        let at = usize::from(f.offset);
        let size = c
            .fields
            .iter()
            .map(|g| usize::from(g.offset))
            .filter(|&o| o > at)
            .min()
            .map_or(4, |next| (next - at).min(4));
        for (id, r) in c.records().unwrap_or_default() {
            let Some(name) = names.get(&id) else {
                continue;
            };
            let Some(b) = r.get(at..at + size) else {
                continue;
            };
            let mut word = [0; 4];
            word[..size].copy_from_slice(b);
            let mut v = u32::from_le_bytes(word);
            if (1..32).contains(&f.bit_width) {
                v = (v >> f.bit_shift) & ((1 << f.bit_width) - 1);
            }
            let name = name.to_ascii_lowercase();
            if component == "WeaponThrown" {
                thrown.insert(name.clone());
            }
            reserves.insert(name, v);
        }
    }
    (reserves, thrown)
}

impl Inventory {
    /// `who`'s weapons.
    pub fn of(&self, who: u32) -> &[u32] {
        self.weapons.get(&who).map_or(&[], Vec::as_slice)
    }

    /// Takes `weapon` from whoever carries it.
    pub fn drop(&mut self, weapon: u32) -> Option<Carried> {
        let c = self.carried.remove(&weapon)?;
        if let Some(list) = self.weapons.get_mut(&c.owner) {
            list.retain(|&w| w != weapon);
        }
        Some(c)
    }
}
