//! Per-object node health: the table the engine builds from a template's
//! `NodeHealth` records when an object is placed, how a node takes
//! damage, and the script natives' view of it (`Object.GetNodeHealth`,
//! `ObjectState.SendDamage`).
//!
//! # Provenance
//!
//! From the 360 code:
//!
//! - `NodeHealth`'s schema (0x823F41F0) is `Health` (100 by default),
//!   `DamagePassThrough` (0.2), `DamageThreshhold` (0), `TickThreshold`
//!   (0) and `TickDPS` (1). Their name hashes are the keys the template
//!   database gives the records (`27c50f84`, `ea813967`, `0ec0aee7`,
//!   `261b2109`); the record's `Name` is the node.
//! - On placing (0x8222D850) each record with a node becomes a 12-byte
//!   entry (0x82233568, added by 0x82233480, at most 64): the node's
//!   hash, its most health and its health (both the record's `Health`,
//!   kept between 1 and 65535 and rounded down, as 16-bit counts), and
//!   two shares as bytes of 255: `DamagePassThrough` (kept to 0..1) and
//!   `DamageThreshhold` over the most health (likewise). The
//!   bytes are read back times 0.004 (0x820F8C54), not over 255.
//! - `Object.GetNodeHealth(object, name)` (0x8232E898) hashes the name
//!   (0x82308518) and answers that node's health, a whole number; 0 for
//!   an object without the node (0x820BAF14) or without a table.
//! - `ObjectState.SendDamage(object, node hash, share)` (0x82333F80,
//!   0x8222F7C0) deals the share times the node's most health
//!   (0x823F7F50), or with node 0 times the object's own health; nothing
//!   if that is not above 0 or the model has no such node. The damage
//!   goes out as a damage event of kind 0 at that node (0x82155680).
//! - A node taking damage (0x82289020): its health less the damage, not
//!   below 0, rounded down, kept to its most (0x822333C8). If it is left
//!   above 0.01 (or was at or under 0.01 already), the node takes
//!   `DamageMsg` (`c6507ee1`) when it falls to its damaged-at health
//!   (its threshold share times its most health), from above it, with
//!   the threshold above 0.01. If it falls to 0.01 or under from above,
//!   it takes `DestroyMsg` (`1ed7ad78`) for damage kinds 0 and 3, and for
//!   kind 1 but from two weapons; otherwise the third message
//!   ([`DESTROY_OTHER_MSG`]).
//! - Then the whole object's check (same routine): it walks the table and
//!   looks for one node that is a piece of the object's model (found in
//!   the model's name index, 0x821C1EC0), whose piece is still shown (bit
//!   set in the first of the model instance's five 1024-bit sets, at
//!   +0x14, which 0x82426738 fills with ones; the instance is the
//!   component at 0x8380F1D8 that 0x8220A9A8 makes for the object's
//!   model) and whose health is above 0.01. With none, the object takes
//!   damage of all its health left (0x822889F8 given `Health`'s current
//!   value): it dies ([`NodeTable::any_left`]).
//! - Then the object takes the pass-through share of the damage, when
//!   that share is above 0, the hit was not flagged to keep it (event
//!   flag bits 29 and 30), and the node had health above 0.01 before the
//!   hit or the object is not labelled `Building` (`e6124e8d`, looked up
//!   in the `Label` component 0x83653C4C by 0x82289650).
//! - A hit whose node the model has no part for goes to the object in
//!   full (0x82288868 finds none: 0x822889F8 with the whole damage).
//! - The object's `Health` taking damage (0x822889F8; [`ObjectHealth`]):
//!   nothing at 0.01 or under; otherwise its health less the damage,
//!   kept to its most and not below 0. Under 0.01 it dies: the death
//!   event (0x8217D8A0, queue 0x831AD128) is posted and the health is not
//!   stored there. Otherwise the health is kept.
//! - The death (0x82289F40, which also calls the object script's
//!   `OnDeath`): `Health` goes to 0 and every node with health above 0.01
//!   takes `DestroyMsg` (the float 2.2835755e-20 in the listing is its
//!   bits) and is set to none ([`NodeTable::destroy_all`]).
//! - Reviving the object (`Object.Revive`, `Vehicle.RestoreHealth`,
//!   0x8222C9B8) puts every node's health back to its most.
//!
//! (`5b724250`, which an earlier round took for a list of the nodes that
//! keep an object alive, is the key 0x82846620 looks a model resource up
//! by, as 0x8220A9A8 and 0x8222D850 do; it chooses no nodes.)
//!
//! Not done here: the node `765cd254` kept at 1 health under a flag of
//! the object's (component 0x83657EF4); the object's own `DamageMsg` when
//! its health falls past its damaged-at share (0x822889F8, through
//! 0x8228D138); difficulty and invulnerability adjustments in 0x822889F8;
//! the "can't die" test 0x822885B0 that leaves it at 1.

use std::collections::HashMap;

use m2_assets::model::{DAMAGE_MSG, DESTROY_MSG};
use m2_assets::{Layer, name_hash};

use crate::tuning::Table;

/// The message a node at no health takes for damage of other kinds
/// (`3d0d4c99`, name unsolved). From the 360 code (0x82289020).
pub const DESTROY_OTHER_MSG: u32 = 0x3d0d_4c99;
/// The most nodes an object's table holds (0x82233480).
pub const MAX_NODES: usize = 64;
/// Health at or under this is none (0x820B7AE0).
const NONE_LEFT: f32 = 0.01;
/// A share byte's worth (0x820F8C54), and what a share is stored times
/// (0x820F8D80).
const SHARE_UNIT: f32 = 0.004;
const SHARE_STORE: f32 = 255.0;
/// The most a node's health can be (0x820F8EA8).
const MOST_HEALTH: f32 = 65535.0;

/// `NodeHealth`'s field keys.
const DAMAGE_PASS_THROUGH: u32 = 0x27c5_0f84;
const DAMAGE_THRESHHOLD: u32 = 0xea81_3967;

/// One `NodeHealth` record as the template gives it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeRecord {
    /// The node's name hash (`Name`).
    pub node: u32,
    /// `Health`.
    pub health: f32,
    /// `DamagePassThrough`: the share of a node's damage its object
    /// takes too.
    pub pass_through: f32,
    /// `DamageThreshhold`: the health at which the node is damaged.
    pub threshold: f32,
}

impl NodeRecord {
    /// A record with the schema's defaults but its node.
    pub fn new(node: u32) -> Self {
        Self {
            node,
            health: 100.0,
            pass_through: 0.2,
            threshold: 0.0,
        }
    }
}

/// Every template's `NodeHealth` records that name a node, by template id.
pub fn read(db: &Layer<'_>) -> HashMap<u32, Vec<NodeRecord>> {
    let t = Table::new(db, "NodeHealth");
    let (name, health) = (name_hash("Name"), name_hash("Health"));
    t.templates()
        .filter_map(|id| {
            let recs: Vec<NodeRecord> = t
                .of_template(id)
                .iter()
                .filter_map(|r| {
                    let d = NodeRecord::new(t.word(r, name).filter(|&n| n != 0)?);
                    Some(NodeRecord {
                        health: t.float(r, health).unwrap_or(d.health),
                        pass_through: t.float(r, DAMAGE_PASS_THROUGH).unwrap_or(d.pass_through),
                        threshold: t.float(r, DAMAGE_THRESHHOLD).unwrap_or(d.threshold),
                        ..d
                    })
                })
                .collect();
            (!recs.is_empty()).then_some((id, recs))
        })
        .collect()
}

/// Round down, as 0x8285D5F0.
fn floor(x: f32) -> f32 {
    x.floor()
}

/// A share kept between 0 and 1, stored as a byte of 255.
fn share_byte(x: f32) -> u8 {
    floor(x.clamp(0.0, 1.0) * SHARE_STORE) as u8
}

/// One node's entry in an object's table (12 bytes in the engine).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeEntry {
    pub node: u32,
    pub most: u16,
    pub health: u16,
    /// `DamagePassThrough`, in 255ths.
    pub pass_through: u8,
    /// `DamageThreshhold` over the most health, in 255ths.
    pub damaged_at: u8,
}

impl NodeEntry {
    /// From a record, as 0x82233568 builds it.
    pub fn of(r: &NodeRecord) -> Self {
        let most = floor(r.health.clamp(1.0, MOST_HEALTH));
        Self {
            node: r.node,
            most: most as u16,
            health: most as u16,
            pass_through: share_byte(r.pass_through),
            damaged_at: share_byte(r.threshold / most),
        }
    }

    /// The health at which it is damaged.
    pub fn damaged_health(&self) -> f32 {
        f32::from(self.damaged_at) * f32::from(self.most) * SHARE_UNIT
    }

    /// Of a node's damage, the share its object takes.
    pub fn pass_share(&self) -> f32 {
        f32::from(self.pass_through) * SHARE_UNIT
    }
}

/// What a node's damage did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeHit {
    /// The node's health before it.
    pub before: u16,
    /// The node's health after it.
    pub health: u16,
    /// The message the node takes, if any ([`DAMAGE_MSG`],
    /// [`DESTROY_MSG`], [`DESTROY_OTHER_MSG`]).
    pub message: Option<u32>,
    /// The damage its object takes too.
    pub pass_through: f32,
}

/// An object's node table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeTable {
    entries: Vec<NodeEntry>,
}

impl NodeTable {
    /// The table placing builds from a template's records: those with a
    /// node, the first [`MAX_NODES`].
    pub fn new(records: &[NodeRecord]) -> Self {
        Self {
            entries: records
                .iter()
                .filter(|r| r.node != 0)
                .take(MAX_NODES)
                .map(NodeEntry::of)
                .collect(),
        }
    }

    pub fn entries(&self) -> &[NodeEntry] {
        &self.entries
    }

    /// The entry for `node`. A node given twice is found as its later
    /// record (the lookup's key is set again; inferred).
    pub fn get(&self, node: u32) -> Option<&NodeEntry> {
        self.entries.iter().rev().find(|e| e.node == node)
    }

    fn get_mut(&mut self, node: u32) -> Option<&mut NodeEntry> {
        self.entries.iter_mut().rev().find(|e| e.node == node)
    }

    /// `Object.GetNodeHealth`: the node's health, 0 without it.
    pub fn health(&self, node: u32) -> f32 {
        self.get(node).map_or(0.0, |e| f32::from(e.health))
    }

    /// `ObjectState.SendDamage`'s damage for a node: `share` of its most
    /// health; `None` when that is not above 0 or there is no such node.
    pub fn send_damage(&self, node: u32, share: f32) -> Option<f32> {
        let d = self.get(node).map_or(0.0, |e| f32::from(e.most)) * share;
        (d > 0.0).then_some(d)
    }

    /// The node takes `damage` of damage kind `kind`; `spared` is a kind
    /// 1 hit from one of the two weapons that don't destroy (0x82289020).
    /// `building`: the object is labelled `Building`, so a node already
    /// at none passes nothing on.
    pub fn damage(
        &mut self,
        node: u32,
        damage: f32,
        kind: u32,
        spared: bool,
        building: bool,
    ) -> Option<NodeHit> {
        let e = self.get_mut(node)?;
        let was = f32::from(e.health);
        let damaged_at = e.damaged_health();
        let left = floor((was - damage).max(0.0));
        let message = if left > NONE_LEFT || was <= NONE_LEFT {
            (damaged_at > NONE_LEFT && left <= damaged_at && damaged_at < was).then_some(DAMAGE_MSG)
        } else if kind == 0 || kind == 3 || (kind == 1 && !spared) {
            Some(DESTROY_MSG)
        } else {
            Some(DESTROY_OTHER_MSG)
        };
        e.health = left.min(f32::from(e.most)) as u16;
        let share = e.pass_share();
        let passes = share > 0.0 && (was > NONE_LEFT || !building);
        Some(NodeHit {
            before: was as u16,
            health: e.health,
            message,
            pass_through: if passes { share * damage } else { 0.0 },
        })
    }

    /// Whether the object lives on after a node's damage: some node that
    /// `shown` says is a shown piece of its model has health above 0.01
    /// (0x82289020). False for a table with no such node: the object
    /// dies.
    pub fn any_left(&self, shown: impl Fn(u32) -> bool) -> bool {
        self.entries
            .iter()
            .any(|e| shown(e.node) && f32::from(e.health) > NONE_LEFT)
    }

    /// The object died (0x82289F40): every node with health left is set
    /// to none. The nodes that had some, in table order: each takes
    /// [`DESTROY_MSG`].
    pub fn destroy_all(&mut self) -> Vec<u32> {
        let mut gone = Vec::new();
        for e in &mut self.entries {
            if f32::from(e.health) > NONE_LEFT {
                gone.push(e.node);
            }
            e.health = 0;
        }
        gone
    }

    /// Every node's health back to its most (`Object.Revive`).
    pub fn restore(&mut self) {
        for e in &mut self.entries {
            e.health = e.most;
        }
    }
}

/// What the object's `Health` did with some damage (0x822889F8).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ObjectHit {
    /// It had none left to lose (0.01 or under): nothing happens.
    Untouched,
    /// It lives, with this health.
    Left(f32),
    /// It fell under 0.01: it dies (the death handler sets it to 0).
    Died,
}

/// An object's `Health`: its most and current health.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjectHealth {
    pub most: f32,
    pub health: f32,
}

impl ObjectHealth {
    /// The object takes `damage` (0x822889F8): its health less the
    /// damage, kept to its most and not below 0; under 0.01 it dies.
    pub fn take(&mut self, damage: f32) -> ObjectHit {
        if self.health <= NONE_LEFT {
            return ObjectHit::Untouched;
        }
        let left = (self.health - damage).min(self.most).max(0.0);
        if left < NONE_LEFT {
            return ObjectHit::Died;
        }
        self.health = left;
        ObjectHit::Left(left)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(node: u32, health: f32, pass_through: f32, threshold: f32) -> NodeRecord {
        NodeRecord {
            node,
            health,
            pass_through,
            threshold,
        }
    }

    #[test]
    fn the_field_keys_are_the_schemas_names() {
        assert_eq!(name_hash("DamagePassThrough"), DAMAGE_PASS_THROUGH);
        assert_eq!(name_hash("DamageThreshhold"), DAMAGE_THRESHHOLD);
        assert_eq!(name_hash("TickThreshold"), 0x0ec0_aee7);
        assert_eq!(name_hash("TickDPS"), 0x261b_2109);
    }

    #[test]
    fn an_entry_keeps_whole_health_and_shares_in_255ths() {
        // A door: 30 health, damaged at 15, all its damage passed on.
        let e = NodeEntry::of(&rec(7, 30.7, 1.0, 15.0));
        assert_eq!((e.most, e.health), (30, 30));
        assert_eq!(e.pass_through, 255);
        assert_eq!(e.damaged_at, 127); // 15/30 * 255 = 127.5, down.
        assert!((e.damaged_health() - 15.24).abs() < 1e-3);
        assert!((e.pass_share() - 1.02).abs() < 1e-6);
        // Kept to 1..65535, the shares to 0..1.
        let e = NodeEntry::of(&rec(7, 0.0, -1.0, 9.0));
        assert_eq!((e.most, e.pass_through, e.damaged_at), (1, 0, 255));
        assert_eq!(NodeEntry::of(&rec(7, 1e9, 0.2, 0.0)).most, 65535);
        assert_eq!(NodeEntry::of(&NodeRecord::new(7)).pass_through, 51);
    }

    #[test]
    fn the_table_holds_named_nodes_up_to_64() {
        let recs: Vec<_> = (0..70).map(|n| rec(n, 10.0, 0.0, 0.0)).collect();
        let t = NodeTable::new(&recs);
        assert_eq!(t.entries().len(), MAX_NODES);
        assert_eq!(t.entries()[0].node, 1);
        assert_eq!(t.health(1), 10.0);
        assert_eq!(t.health(0), 0.0);
        assert_eq!(t.health(999), 0.0);
    }

    #[test]
    fn send_damage_is_a_share_of_the_nodes_most_health() {
        let t = NodeTable::new(&[rec(5, 400.0, 0.2, 0.0)]);
        assert_eq!(t.send_damage(5, 1.0), Some(400.0));
        assert_eq!(t.send_damage(5, 0.25), Some(100.0));
        assert_eq!(t.send_damage(5, 0.0), None);
        assert_eq!(t.send_damage(6, 1.0), None);
    }

    #[test]
    fn a_node_is_damaged_at_its_threshold_and_destroyed_at_none() {
        let mut t = NodeTable::new(&[rec(5, 30.0, 0.5, 15.0)]);
        let h = t.damage(5, 10.5, 0, false, false).unwrap();
        assert_eq!((h.health, h.message), (19, None));
        assert!((h.pass_through - 127.0 * 0.004 * 10.5).abs() < 1e-4);
        // Down past 15 (stored as 127/255 of 30: 15.24).
        let h = t.damage(5, 4.0, 0, false, false).unwrap();
        assert_eq!((h.health, h.message), (15, Some(DAMAGE_MSG)));
        // Not again while above none.
        assert_eq!(t.damage(5, 1.0, 0, false, false).unwrap().message, None);
        let h = t.damage(5, 100.0, 0, false, false).unwrap();
        assert_eq!((h.health, h.message), (0, Some(DESTROY_MSG)));
        // Already none: nothing more.
        assert_eq!(t.damage(5, 1.0, 0, false, false).unwrap().message, None);
        t.restore();
        assert_eq!(t.health(5), 30.0);
    }

    #[test]
    fn a_node_destroyed_by_other_kinds_takes_the_third_message() {
        let destroy = |kind, spared| {
            let mut t = NodeTable::new(&[rec(5, 10.0, 0.0, 0.0)]);
            t.damage(5, 10.0, kind, spared, false).unwrap().message
        };
        assert_eq!(destroy(0, false), Some(DESTROY_MSG));
        assert_eq!(destroy(3, false), Some(DESTROY_MSG));
        assert_eq!(destroy(1, false), Some(DESTROY_MSG));
        assert_eq!(destroy(1, true), Some(DESTROY_OTHER_MSG));
        assert_eq!(destroy(2, false), Some(DESTROY_OTHER_MSG));
    }

    #[test]
    fn send_damage_of_one_destroys_the_node() {
        // The island fortress's script: SendDamage(o, hash, 1) on a slice
        // with health left.
        let mut t = NodeTable::new(&[rec(5, 250.0, 0.0, 100.0)]);
        let d = t.send_damage(5, 1.0).unwrap();
        let h = t.damage(5, d, 0, false, false).unwrap();
        assert_eq!((h.health, h.message), (0, Some(DESTROY_MSG)));
        assert_eq!(t.health(5), 0.0);
    }

    #[test]
    fn a_building_passes_nothing_on_from_a_node_at_none() {
        // A fortress slice: 100 health, all its damage passed on.
        let mut t = NodeTable::new(&[rec(5, 100.0, 1.0, 0.0)]);
        let h = t.damage(5, 100.0, 0, false, true).unwrap();
        assert_eq!((h.before, h.health), (100, 0));
        assert!((h.pass_through - 102.0).abs() < 1e-3); // 255 * 0.004.
        // At none already: a building takes nothing more, anything else
        // does.
        assert_eq!(t.damage(5, 50.0, 0, false, true).unwrap().pass_through, 0.0);
        assert!(t.damage(5, 50.0, 0, false, false).unwrap().pass_through > 0.0);
    }

    #[test]
    fn the_object_dies_once_no_shown_node_has_health() {
        let mut t = NodeTable::new(&[rec(1, 10.0, 0.0, 0.0), rec(2, 10.0, 0.0, 0.0)]);
        assert!(t.any_left(|_| true));
        t.damage(1, 10.0, 0, false, true);
        assert!(t.any_left(|_| true));
        // Node 2 alive but not a shown piece: it doesn't count.
        assert!(!t.any_left(|n| n == 1));
        t.damage(2, 10.0, 0, false, true);
        assert!(!t.any_left(|_| true));
        // A table with no node on the model keeps nothing alive.
        let t = NodeTable::new(&[rec(3, 10.0, 0.0, 0.0)]);
        assert!(!t.any_left(|_| false));
    }

    #[test]
    fn the_death_destroys_every_node_with_health_left() {
        let mut t = NodeTable::new(&[
            rec(1, 10.0, 0.0, 0.0),
            rec(2, 10.0, 0.0, 0.0),
            rec(3, 10.0, 0.0, 0.0),
        ]);
        t.damage(2, 10.0, 0, false, true);
        assert_eq!(t.destroy_all(), [1, 3]);
        assert!(t.entries().iter().all(|e| e.health == 0));
        assert!(t.destroy_all().is_empty());
    }

    #[test]
    fn the_objects_health_falls_to_its_death() {
        let mut o = ObjectHealth {
            most: 4000.0,
            health: 4000.0,
        };
        assert_eq!(o.take(900.0), ObjectHit::Left(3100.0));
        // Healing is kept to the most.
        assert_eq!(o.take(-5000.0), ObjectHit::Left(4000.0));
        assert_eq!(o.take(3999.995), ObjectHit::Died);
        // The death doesn't store the health; the death handler zeroes it.
        assert_eq!(o.health, 4000.0);
        o.health = 0.0;
        assert_eq!(o.take(10.0), ObjectHit::Untouched);
    }
}
