//! What the template database says is left when something is destroyed,
//! and what a pickup gives: a supply crate (`Supply Drop (GL)`) breaks
//! open to leave its `SpawnOnDeath`, the `Ammo Pickup (Rocket)` that gives
//! 20 rounds of the RPG's ammunition to whoever comes within a metre.
//!
//! Field keys without a known name are used as they are; the names in
//! the comments say what each holds, read off the crates' and pickups'
//! values.

use std::collections::HashMap;

use m2_assets::{Layer, name_hash};

use crate::tuning::Table;

/// `SpawnOnDeath`'s field naming the template left (name unknown).
const SPAWNS: u32 = 0x7461_c0cd;
/// `Pickup`'s fields (names unknown): the ammunition it gives (a weapon's
/// `AmmoTemplate`), how many rounds, the name hash of what is heard or
/// shown when it is taken (`pickup_ammo_rockets`) and of the effect then
/// (`global_particle_explosion_pickup_rocket`).
const AMMO: u32 = 0xc867_d0d0;
const ROUNDS: u32 = 0xa06b_3faa;
const CUE: u32 = 0xb6d6_0154;
const EFFECT: u32 = 0x6235_349a;

/// A pickup's template: what walking up to it gives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PickupInfo {
    /// How near she must come to take it, metres (`radius`).
    pub radius: f32,
    /// The ammunition it gives, by template id, and how many rounds.
    pub ammo: Option<u32>,
    pub rounds: u32,
    /// Name hashes of its sound or message on being taken and of its
    /// effect.
    pub cue: Option<u32>,
    pub effect: Option<u32>,
}

/// What the database says of leaving and picking up, by template id.
#[derive(Clone, Debug, Default)]
pub struct Leavings {
    /// The template each leaves when destroyed.
    pub spawns: HashMap<u32, u32>,
    pub pickups: HashMap<u32, PickupInfo>,
    /// The ammunition each weapon fires.
    pub ammo: HashMap<u32, u32>,
}

pub fn read(db: &Layer<'_>) -> Leavings {
    let spawn = Table::new(db, "SpawnOnDeath");
    let pickup = Table::new(db, "Pickup");
    let weapon = Table::new(db, "WeaponProjectileBase");
    let nonzero = |v: Option<u32>| v.filter(|&v| v != 0);
    Leavings {
        spawns: spawn
            .templates()
            .filter_map(|id| {
                let r = spawn.of_template(id).first()?;
                Some((id, nonzero(spawn.word(r, SPAWNS))?))
            })
            .collect(),
        pickups: pickup
            .templates()
            .filter_map(|id| {
                let r = pickup.of_template(id).first()?;
                Some((
                    id,
                    PickupInfo {
                        radius: pickup.float(r, name_hash("radius"))?,
                        ammo: nonzero(pickup.word(r, AMMO)),
                        rounds: pickup.word(r, ROUNDS).unwrap_or(0),
                        cue: nonzero(pickup.word(r, CUE)),
                        effect: nonzero(pickup.word(r, EFFECT)),
                    },
                ))
            })
            .collect(),
        ammo: weapon
            .templates()
            .filter_map(|id| {
                let r = weapon.of_template(id).first()?;
                Some((id, nonzero(weapon.word(r, name_hash("AmmoTemplate")))?))
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The survey names the rocket pickup's fields `Type` and `radius`
    // by these keys, and its weapons' `AmmoTemplate`.
    #[test]
    fn the_named_fields_hash_to_their_keys() {
        assert_eq!(name_hash("radius"), 0x3546_94db);
        assert_eq!(name_hash("Type"), 0x28e1_0525);
        assert_eq!(name_hash("AmmoTemplate"), 0xc5a7_d745);
    }
}
