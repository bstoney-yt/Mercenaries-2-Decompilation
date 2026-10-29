//! What the template database says is left when something is destroyed,
//! and what a pickup gives: a supply crate (`Supply Drop (GL)`) breaks
//! open to leave each of its `SpawnOnDeath`s, a `Grenade Launcher` and the
//! `Ammo Pickup (Rocket)` that gives 20 rounds of the RPG's ammunition to
//! whoever comes within a metre.
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
/// `AmmoTemplate`), how many rounds, the name hash of the icon shown over
/// it (`pickup_ammo_rockets`; from the 360 code: the record's sixth word,
/// `TextureModel`, which the HUD's pickup markers draw, 82378890) and of
/// the effect when it is taken (`global_particle_explosion_pickup_rocket`).
const AMMO: u32 = 0xc867_d0d0;
const ROUNDS: u32 = 0xa06b_3faa;
const ICON: u32 = 0xb6d6_0154;
const EFFECT: u32 = 0x6235_349a;
/// `Label`'s field: the label's name hash (`HeavySoldier`).
const LABEL: u32 = 0xfd08_4cce;

/// A pickup's `Type`: health (its amount health points), ammunition, or
/// a weapon taken up.
pub const HEALTH: u32 = 0;
pub const AMMUNITION: u32 = 1;
pub const EQUIPMENT: u32 = 2;
/// Money: a `Pickup` of `Type` 5 gives as much cash as its rounds field
/// says (`Cash (Small)` 1000, `Cash (Medium)` 5000, `Cash (Case)` 25000),
/// one of `Type` 6 as much fuel (`Fuel Pickup (Small)` 25, `(Large)` 50).
/// PC data, 2026-10-14.
pub const CASH: u32 = 5;
pub const FUEL: u32 = 6;
/// A vehicle's ammunition (`PickupTypeEnum` 4; from the 360 code, no
/// template uses it).
pub const VEHICLE_AMMUNITION: u32 = 4;

/// A pickup's template: what walking up to it gives.
#[derive(Clone, Debug, PartialEq)]
pub struct PickupInfo {
    /// Its first `Pickup`'s `Type` ([`HEALTH`], [`AMMUNITION`], [`EQUIPMENT`]: a
    /// weapon).
    pub kind: u32,
    /// How near she must come to take it, metres (`radius`).
    pub radius: f32,
    /// The ammunition it gives, by template id, and how many rounds.
    pub ammo: Option<u32>,
    pub rounds: u32,
    /// Every ammunition it gives and how much: an ammunition pickup has a
    /// `Pickup` per kind (`Ammo Pickup (Rocket)`: the RPG's rockets, the
    /// grenade launcher's rounds, missiles), the first of them as above.
    pub gives: Vec<(u32, u32)>,
    /// Those of [`gives`](Self::gives) whose own `Type` is
    /// [`AMMUNITION`]: what is taken walking over it, even where it is a
    /// weapon lying there (the `Assault Rifle`, 8000436f, a soldier's gun:
    /// a `Type` 2 equipment record of radius 1.5, and a `Type` 1 record of
    /// 8 `Assault Rifle Bullet`; PC data, 2026-10-09).
    pub walked: Vec<(u32, u32)>,
    /// The health its [`HEALTH`] records give, together: the `Health
    /// Pickup`'s one record of 50, and the 3 of the `Ammo Pickup (Small)`'s
    /// record of that `Type` among its 19 ammunition records (8000a286; PC
    /// data, 2026-10-09).
    pub heals: u32,
    /// Name hashes of the icon its marker shows (`pickup_fuel`) and of its
    /// effect on being taken.
    pub icon: Option<u32>,
    pub effect: Option<u32>,
}

impl PickupInfo {
    /// Whether its marker fades by the distance over the ground only, not
    /// counting height (from the 360 code, 82378890): ammunition, a
    /// vehicle's ammunition, cash and fuel do; health and a weapon lying
    /// there measure the whole distance.
    pub fn marker_flat(&self) -> bool {
        matches!(self.kind, AMMUNITION | VEHICLE_AMMUNITION | CASH | FUEL)
    }
}

/// What the database says of leaving and picking up, by template id.
#[derive(Clone, Debug, Default)]
pub struct Leavings {
    /// The templates each leaves when destroyed, in data order (a
    /// template may have `SpawnOnDeath` more than once).
    pub spawns: HashMap<u32, Vec<u32>>,
    pub pickups: HashMap<u32, PickupInfo>,
    /// The ammunition each weapon fires.
    pub ammo: HashMap<u32, u32>,
    /// Each template's labels' name hashes: a dead soldier's pick what
    /// it leaves (`HeavySoldier`, `RocketSoldier`).
    pub labels: HashMap<u32, Vec<u32>>,
}

pub fn read(db: &Layer<'_>) -> Leavings {
    let spawn = Table::new(db, "SpawnOnDeath");
    let pickup = Table::new(db, "Pickup");
    let weapon = Table::new(db, "WeaponProjectileBase");
    let label = Table::new(db, "Label");
    let nonzero = |v: Option<u32>| v.filter(|&v| v != 0);
    let kind = name_hash("Type");
    Leavings {
        spawns: spawn
            .templates()
            .filter_map(|id| {
                let left: Vec<u32> = spawn
                    .of_template(id)
                    .iter()
                    .filter_map(|r| nonzero(spawn.word(r, SPAWNS)))
                    .collect();
                (!left.is_empty()).then_some((id, left))
            })
            .collect(),
        pickups: pickup
            .templates()
            .filter_map(|id| {
                let records = pickup.of_template(id);
                // A gun's equipment record says what it is and how near
                // she must be, wherever it stands among its records (the
                // `Machine Pistol (Uzi)` lists its bullets first, radius
                // 1.0, then itself, 1.5; PC data: playtest 4).
                let r = deciding(records, |r| pickup.word(r, kind))?;
                Some((
                    id,
                    PickupInfo {
                        kind: pickup.word(r, kind).unwrap_or(AMMUNITION),
                        radius: pickup.float(r, name_hash("radius"))?,
                        ammo: nonzero(pickup.word(r, AMMO)),
                        rounds: pickup.word(r, ROUNDS).unwrap_or(0),
                        gives: records
                            .iter()
                            .filter_map(|r| {
                                let ammo = nonzero(pickup.word(r, AMMO))?;
                                Some((ammo, pickup.word(r, ROUNDS).unwrap_or(0)))
                            })
                            .collect(),
                        walked: records
                            .iter()
                            .filter(|r| pickup.word(r, kind).unwrap_or(AMMUNITION) == AMMUNITION)
                            .filter_map(|r| {
                                let ammo = nonzero(pickup.word(r, AMMO))?;
                                Some((ammo, pickup.word(r, ROUNDS).unwrap_or(0)))
                            })
                            .collect(),
                        heals: records
                            .iter()
                            .filter(|r| pickup.word(r, kind) == Some(HEALTH))
                            .map(|r| pickup.word(r, ROUNDS).unwrap_or(0))
                            .sum(),
                        icon: nonzero(pickup.word(r, ICON)),
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
        labels: label
            .templates()
            .map(|id| {
                let labels = label
                    .of_template(id)
                    .iter()
                    .filter_map(|r| label.word(r, LABEL))
                    .collect();
                (id, labels)
            })
            .collect(),
    }
}

/// Of a template's pickup records, the one that says what it is and its
/// reach: its [`EQUIPMENT`] record if it has one, else its first.
fn deciding<R>(records: &[R], kind_of: impl Fn(&R) -> Option<u32>) -> Option<&R> {
    records
        .iter()
        .find(|r| kind_of(r) == Some(EQUIPMENT))
        .or_else(|| records.first())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_guns_equipment_record_decides_wherever_it_stands() {
        // (Type, radius): the Uzi's bullets first, then the gun.
        let uzi = [(AMMUNITION, 1.0), (EQUIPMENT, 1.5)];
        assert_eq!(deciding(&uzi, |r| Some(r.0)), Some(&(EQUIPMENT, 1.5)));
        // A plain ammunition pickup: its first.
        let ammo = [(AMMUNITION, 1.0), (HEALTH, 1.0)];
        assert_eq!(deciding(&ammo, |r| Some(r.0)), Some(&(AMMUNITION, 1.0)));
        assert_eq!(deciding::<(u32, f32)>(&[], |r| Some(r.0)), None);
    }

    // The survey names the rocket pickup's fields `Type` and `radius`
    // by these keys, and its weapons' `AmmoTemplate`.
    #[test]
    fn the_named_fields_hash_to_their_keys() {
        assert_eq!(name_hash("radius"), 0x3546_94db);
        assert_eq!(name_hash("Type"), 0x28e1_0525);
        assert_eq!(name_hash("AmmoTemplate"), 0xc5a7_d745);
    }
}
