//! Falling debris crushing what it lands on: the `Crusher` component's
//! `DamageModifier`, as the 360's collision damage applies it.
//!
//! From the 360 code (0x8228B5F0, once for each contact event that
//! 0x82282530 takes off the queue at 0x8320C6F0):
//!
//! - A contact smaller than [`crate::durability::CRASH_FLOOR`] (5) does
//!   nothing.
//! - Of the two sides, the one whose template's `Crusher` `DamageModifier`
//!   (table 0x83655CC8, field 0x1CB6F279, read by 0x823F77E0, default 1)
//!   is the larger, and over 1, is the crusher.
//! - Its modifier stands only when the crusher comes down onto the other
//!   side: the contact normal, seen from the crusher's side, points up at
//!   least [`DOWNWARD`] (0x820D9EDC); the contact's closing speed is at
//!   least [`CRUSH_SPEED`] (0x820BAF84); and the crusher itself moves at
//!   [`CRUSH_SPEED`] or more (its speed squared against 25, 0x820F02E4).
//!   Otherwise it counts as 1.
//! - The damage passed on (0x82287CF8, with the crusher as the other side,
//!   so its attacker) is the contact's size times the modifier times a
//!   hundredth (0x820B7AE0); 0x82287CF8 then applies each side's cell of
//!   the damage table and its "over 1" rule
//!   ([`crate::durability::crash_damage`]).
//! - A hit whose attacker carries `Crusher` goes past the player's health
//!   clamp (0x822889F8; `m2_game::crusher`).
//!
//! Which pieces crush (from samples): an object's template names the
//! templates its broken pieces become in its `OSMParameter` records
//! ([`piece_crushers`]); 751 records (on 750 templates, 748 of them the
//! `DebrisTemplate` parameter) name a `Crusher` template, such as a
//! smokestack's `CrushingDebrisMedTemplate`, an electrical tower's
//! `TowerDestructTemplate`, a stone bridge's `debris_stone_lrg`.
//!
//! Inferred: the normal's sense. The 360 reads the record's normal as it
//! is for side 0 and negated for side 1; here it is taken to point from
//! side `b` to side `a` (Havok's contact normal, toward the first body),
//! so a crusher on side `a` lying on top of `b` sees it point up.

use std::collections::HashMap;

use m2_assets::{Layer, name_hash};
use m2_math::glam::Vec3;

use crate::durability::{CRASH_FLOOR, crash_base};
use crate::tuning::Table;

/// The least the normal must point up from the crusher's side for it to
/// count as landing on the other (0x820D9EDC).
pub const DOWNWARD: f32 = 0.5;

/// The least closing speed, and the least speed of the crusher itself,
/// m/s, for its modifier to stand (0x820BAF84; 0x820F02E4 = 25 = 5^2).
pub const CRUSH_SPEED: f32 = 5.0;

/// `Crusher`'s one field, `DamageModifier` (0x823F77E0).
pub const DAMAGE_MODIFIER: &str = "DamageModifier";

/// One contact between two bodies, as the crush step reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    /// The contact's size ([`crate::durability::crash_size`]).
    pub size: f32,
    /// Each side's `Crusher` `DamageModifier` (1 without one).
    pub modifier: [f32; 2],
    /// The contact normal, pointing from side `b` (1) to side `a` (0).
    pub normal: Vec3,
    /// How fast the two closed along the normal, m/s.
    pub closing: f32,
    /// Each side's own velocity, m/s.
    pub velocity: [Vec3; 2],
}

/// What a contact does once the crush step has read it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crush {
    /// The damage before the damage table's cells: size x modifier x a
    /// hundredth.
    pub base: f32,
    /// The side that crushed (0 or 1), the attacker of the other, where
    /// a `Crusher` landed on it with its modifier standing.
    pub crusher: Option<usize>,
    /// The modifier that stood (1 where none did).
    pub modifier: f32,
}

/// The crush step of a contact (0x8228B5F0): `None` under the floor.
pub fn crush(c: &Contact) -> Option<Crush> {
    if c.size < CRASH_FLOOR {
        return None;
    }
    let side = if c.modifier[1] > c.modifier[0] { 1 } else { 0 };
    let mut modifier = c.modifier[side].max(1.0);
    if modifier > 1.0 {
        let n = c.normal.normalize_or_zero();
        let up = if side == 0 { n.y } else { -n.y };
        let falling = c.velocity[side].length_squared() >= CRUSH_SPEED * CRUSH_SPEED;
        if c.closing < CRUSH_SPEED || up < DOWNWARD || !falling {
            modifier = 1.0;
        }
    }
    let crusher = (modifier > 1.0).then_some(side);
    Some(Crush {
        base: crash_base(c.size) * modifier,
        crusher,
        modifier,
    })
}

/// The crush step of a loose piece coming onto a ragdoll's bone
/// ([`crate::Physics::ragdoll_debris_hits`]): the piece side `a` with its
/// template's `modifier` (1 without `Crusher`), the bone side `b`.
pub fn crush_hit(hit: &crate::physics::DebrisHit, modifier: f32) -> Option<Crush> {
    crush(&Contact {
        size: hit.size,
        modifier: [modifier, 1.0],
        normal: hit.normal,
        closing: hit.closing,
        velocity: hit.velocity,
    })
}

/// What a crush does to the struck side, by its cell of the damage table
/// (its `ModifierKey` row, the crusher's `DamageKey` column): the base
/// times the cell, nothing unless over 1 (0x82287CF8).
pub fn crush_damage(c: &Crush, cell: f32) -> f32 {
    let damage = c.base * cell;
    if damage > 1.0 { damage } else { 0.0 }
}

/// Each template's `Crusher` `DamageModifier`, by id.
pub fn crusher_modifiers(db: &Layer<'_>) -> HashMap<u32, f32> {
    let t = Table::new(db, "Crusher");
    let field = name_hash(DAMAGE_MODIFIER);
    t.templates()
        .filter_map(|id| Some((id, t.float(t.of_template(id).first()?, field)?)))
        .collect()
}

/// What each object template's broken pieces crush with, by its id: for
/// each of its `OSMParameter` records whose value names a template that
/// carries `Crusher`, (the parameter's `Name` hash, that template's id,
/// its `DamageModifier`). From samples: 748 of the 751 are its
/// `DebrisTemplate` (what its model's state machine makes its debris
/// as); 3 have parameter names not known (hashes 0x56C61F52,
/// 0x6729C128); none is a `PropTemplate`.
pub fn piece_crushers(
    db: &Layer<'_>,
    names: &HashMap<u32, &str>,
) -> HashMap<u32, Vec<(u32, u32, f32)>> {
    let crushers = crusher_modifiers(db);
    let by_hash: HashMap<u32, u32> = names
        .iter()
        .filter(|(id, _)| crushers.contains_key(id))
        .map(|(&id, n)| (name_hash(n), id))
        .collect();
    let osm = Table::new(db, "OSMParameter");
    let name = name_hash("Name");
    let value = crate::durability::OSM_VALUE;
    osm.templates()
        .filter_map(|id| {
            let pieces: Vec<(u32, u32, f32)> = osm
                .of_template(id)
                .iter()
                .filter_map(|r| {
                    let t = *by_hash.get(&osm.word(r, value)?)?;
                    Some((osm.word(r, name).unwrap_or(0), t, crushers[&t]))
                })
                .collect();
            (!pieces.is_empty()).then_some((id, pieces))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn landing(modifier: f32) -> Contact {
        Contact {
            size: 40.0,
            modifier: [modifier, 1.0],
            normal: Vec3::Y,
            closing: 8.0,
            velocity: [Vec3::new(0.0, -8.0, 0.0), Vec3::ZERO],
        }
    }

    #[test]
    fn the_crushers_field_is_damage_modifier() {
        assert_eq!(name_hash(DAMAGE_MODIFIER), 0x1cb6_f279);
    }

    #[test]
    fn a_crusher_landing_on_someone_multiplies_the_damage() {
        // 40 x 1000 x 0.01: 400, the crusher on side 0 the attacker.
        let c = crush(&landing(1000.0)).unwrap();
        assert_eq!(c.crusher, Some(0));
        assert!((c.base - 400.0).abs() < 1e-3, "{c:?}");
        // Under the floor nothing at all.
        assert_eq!(
            crush(&Contact {
                size: 4.9,
                ..landing(1000.0)
            }),
            None
        );
        // No Crusher: the plain hundredth.
        let plain = crush(&landing(1.0)).unwrap();
        assert_eq!(plain.crusher, None);
        assert!((plain.base - 0.4).abs() < 1e-6);
    }

    #[test]
    fn a_crusher_counts_only_coming_down_fast_onto_it() {
        // From the side: the normal level.
        let side = crush(&Contact {
            normal: Vec3::X,
            ..landing(1000.0)
        })
        .unwrap();
        assert_eq!((side.crusher, side.modifier), (None, 1.0));
        // Slow closing.
        let slow = crush(&Contact {
            closing: 4.0,
            ..landing(1000.0)
        })
        .unwrap();
        assert_eq!(slow.crusher, None);
        // The crusher itself near still (a body walking into resting debris).
        let resting = crush(&Contact {
            velocity: [Vec3::ZERO, Vec3::new(0.0, 8.0, 0.0)],
            ..landing(1000.0)
        })
        .unwrap();
        assert_eq!(resting.crusher, None);
        // On side 1, the normal is seen the other way: it must point down.
        let other = crush(&Contact {
            modifier: [1.0, 2000.0],
            normal: -Vec3::Y,
            velocity: [Vec3::ZERO, Vec3::new(0.0, -6.0, 0.0)],
            ..landing(1.0)
        })
        .unwrap();
        assert_eq!(other.crusher, Some(1));
        assert!((other.base - 800.0).abs() < 1e-3);
    }
}
