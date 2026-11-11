//! What a mind senses of each subject: the 360's sensing records. From
//! the 360 code unless a line says otherwise.
//!
//! A mind's sense query (0x822E89B0) measures a subject from the mind's
//! sense origin (its body, or its task's vehicle's, 0x822D9C10) and keeps
//! the result only if it sees or hears it well enough ([`sense`]). A kept
//! result is merged into the mind's record of that subject (0x822D6030),
//! one of at most [`RECORDS`] ([`SenseTable`]), which the mind rates at
//! each think (0x822DDF50).
//!
//! One of those per-think steps (0x822DF988, handler bit 2 of the table at
//! 0x82B77DE8) is where the disguise meter is fed: a mind fresh on a
//! player in sight and within her range ([`feeds_disguise`]) offers itself
//! to her watchers (0x82445EC8, ported in `vehicle_disguise`),
//! and if it was taken raises one of two events ([`SeenOutcome`]).
//!
//! When a mind is reset or goes, both players' watchers drop it
//! (0x822F8A10 calling 0x82446008): [`SenseTable::forget_all`] and the
//! caller's "mind dropped" signal stand for that.

use m2_math::glam::Vec3;

/// The most records a mind keeps (its +0x820 table, 0x80 entries).
pub const RECORDS: usize = 0x80;

/// Within this distance a subject counts as in sight whatever way the
/// mind faces, metres.
pub const NEAR_SIGHT: f32 = 2.0;

/// Share of the sight reach in the record's likelihood (+0x68).
pub const LIKELY_SHARE: f32 = 0.75;

/// Reach multiplier for an urgent query (the subject in the mind's slot).
pub const URGENT_REACH: f32 = 1.5;

/// A record is fresh enough to feed the disguise meter below this age,
/// seconds (its +0x8C, 0x822DF988).
pub const FRESH: f32 = 0.5;

/// Strength of the two disguise events.
pub const EVENT_STRENGTH: f32 = 50.0;

/// The event a mind raises on seeing through her disguise.
pub const SEEN_THROUGH_DISGUISE: u32 = 0x4E1A_2D0D;

/// The event a mind raises on seeing her undisguised and near.
pub const SEEN_UNDISGUISED: u32 = 0x2E16_ACCD;

/// Below this value of the player's +0x3B0 (not traced) an undisguised
/// sighting raises [`SEEN_UNDISGUISED`].
pub const UNDISGUISED_BELOW: f32 = 5.0;

/// A mind's senses (its +0x7E0 block).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SenseBlock {
    /// Where it senses from.
    pub origin: Vec3,
    /// Which way it faces (unit).
    pub facing: Vec3,
    /// Its `Hearing` and `Vision` (block +0x18, +0x1C).
    pub hearing: f32,
    pub vision: f32,
    /// Cosine of half its view cone (+0x20).
    pub cone_cos: f32,
    /// Scale on both reaches (+0x24).
    pub scale: f32,
    /// The least sight plus hearing a result needs to be kept (+0x28).
    pub threshold: f32,
}

/// What is sensed of: where, and how visible and audible it is (its
/// `Visibility` and `Audibility`: the subject's +0x58 and +0x5C).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Subject {
    pub position: Vec3,
    pub visibility: f32,
    pub audibility: f32,
    /// A person (record kind 4), as against a vehicle or other object.
    pub person: bool,
    /// Being forgotten (its flags bit 36): kept at full strength.
    pub forgotten: bool,
}

/// A mind's record of one subject.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SenseRecord {
    /// From the mind to it (+0x50..+0x58).
    pub offset: Vec3,
    /// How far, metres (+0x5C), and how far across the ground (+0x60).
    pub distance: f32,
    pub flat_distance: f32,
    /// How likely it is seen (+0x68).
    pub likelihood: f32,
    /// How well heard and how well seen, 0 to 1 (+0x6C, +0x70).
    pub hearing: f32,
    pub sight: f32,
    /// Perceived on this query (+0x74 bit 31).
    pub perceived: bool,
    /// Seconds since last merged (+0x8C).
    pub since_update: f32,
    /// A person (kind 4).
    pub person: bool,
}

/// One sense query of `subject` (0x822E89B0): `None` when neither seen
/// nor heard well enough to keep. `urgent` reaches [`URGENT_REACH`]
/// further; `perceive` is what the perceived bit takes where it is set.
pub fn sense(
    block: &SenseBlock,
    subject: &Subject,
    urgent: bool,
    perceive: bool,
) -> Option<SenseRecord> {
    let k = if urgent { URGENT_REACH } else { 1.0 };
    let offset = subject.position - block.origin;
    let distance = offset.length();
    let flat_distance = offset.with_y(0.0).length();
    let mut rec = SenseRecord {
        offset,
        distance,
        flat_distance,
        likelihood: 0.0,
        hearing: 0.0,
        sight: 0.0,
        perceived: false,
        since_update: 0.0,
        person: subject.person,
    };
    if subject.audibility > 0.0 {
        let reach = block.hearing * block.scale * k * subject.audibility;
        if distance < reach {
            rec.perceived = perceive;
            rec.hearing = 1.0 - distance / reach;
        }
    }
    if subject.visibility > 0.0 {
        let reach = block.vision * block.scale * k * subject.visibility;
        let r = reach * LIKELY_SHARE;
        rec.likelihood = r * r / (r * r + distance * distance);
        let in_cone = block.cone_cos * distance < block.facing.dot(offset);
        if distance < NEAR_SIGHT || (in_cone && distance < reach) {
            rec.perceived = perceive;
            rec.sight = 1.0 - distance / reach;
        }
    }
    if subject.forgotten {
        rec.sight = 1.0;
        rec.hearing = 1.0;
        rec.perceived = false;
        return Some(rec);
    }
    (rec.hearing + rec.sight > block.threshold).then_some(rec)
}

/// A mind's records, by the caller's subject number (its +0x820 table).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SenseTable {
    records: Vec<(u32, SenseRecord)>,
}

impl SenseTable {
    /// Its record of `who`.
    pub fn get(&self, who: u32) -> Option<&SenseRecord> {
        self.records.iter().find(|r| r.0 == who).map(|r| &r.1)
    }

    /// Whether it has a record of `who`.
    pub fn knows(&self, who: u32) -> bool {
        self.get(who).is_some()
    }

    /// Merges a query's result into its record of `who`, new or not
    /// (0x822D6030): its age starts again. A full table takes no new one.
    pub fn merge(&mut self, who: u32, rec: SenseRecord) {
        let rec = SenseRecord {
            since_update: 0.0,
            ..rec
        };
        if let Some(r) = self.records.iter_mut().find(|r| r.0 == who) {
            r.1 = rec;
        } else if self.records.len() < RECORDS {
            self.records.push((who, rec));
        }
    }

    /// Ages every record by `dt`.
    pub fn tick(&mut self, dt: f32) {
        for r in &mut self.records {
            r.1.since_update += dt;
        }
    }

    /// Drops its record of `who` (invented: when a fresh look finds it no
    /// longer; how long the 360 keeps a record is not traced).
    pub fn forget(&mut self, who: u32) {
        self.records.retain(|r| r.0 != who);
    }

    /// Keeps only its records of `who` (invented, as [`forget`](Self::forget)).
    pub fn keep_only(&mut self, who: &[u32]) {
        self.records.retain(|r| who.contains(&r.0));
    }

    /// Drops them all, as when the mind is reset (0x822F8A10).
    pub fn forget_all(&mut self) {
        self.records.clear();
    }

    /// Whom it has records of.
    pub fn subjects(&self) -> impl Iterator<Item = u32> + '_ {
        self.records.iter().map(|r| r.0)
    }
}

/// What keeps a mind from feeding the disguise meter (0x822DF988).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DisguiseGate {
    /// A civilian: its entity has the label `Civ` (0xDCC8B14D), the
    /// mind's +0x17DC bit 0, set at its start (0x822D7D50).
    pub civilian: bool,
    /// Its task has an object (+0xFF4: inferred, its vehicle) and it is
    /// aboard it (+0xFF0: inferred).
    pub task_object: bool,
    pub aboard: bool,
}

impl DisguiseGate {
    /// Whether the mind may feed the meter.
    pub fn passes(&self) -> bool {
        !self.civilian && !(self.task_object && self.aboard)
    }
}

/// Whether a mind's record `rec` of a player feeds her disguise meter
/// (0x822DF988): a person, fresh, in sight, the gate open and within her
/// range (the player's +0x39C).
pub fn feeds_disguise(rec: &SenseRecord, gate: &DisguiseGate, her_range: f32) -> bool {
    rec.person
        && rec.since_update < FRESH
        && rec.sight > 0.0
        && gate.passes()
        && rec.distance <= her_range
}

/// What a mind does once it offered itself to her watchers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeenOutcome {
    /// It saw through her disguise ([`SEEN_THROUGH_DISGUISE`]).
    SeenThroughDisguise,
    /// It saw her undisguised and near ([`SEEN_UNDISGUISED`]).
    SeenUndisguised,
}

/// The event after an offer `taken` by her watchers: with vehicle
/// disguises on and her disguised, [`SeenOutcome::SeenThroughDisguise`];
/// undisguised with her +0x3B0 (`her_3b0`, not traced) under
/// [`UNDISGUISED_BELOW`], [`SeenOutcome::SeenUndisguised`].
pub fn outcome(
    taken: bool,
    disguises_on: bool,
    disguised: bool,
    her_3b0: f32,
) -> Option<SeenOutcome> {
    if !taken {
        return None;
    }
    if disguises_on && disguised {
        Some(SeenOutcome::SeenThroughDisguise)
    } else if (!disguises_on || !disguised) && her_3b0 < UNDISGUISED_BELOW {
        Some(SeenOutcome::SeenUndisguised)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block() -> SenseBlock {
        SenseBlock {
            origin: Vec3::ZERO,
            facing: Vec3::Z,
            hearing: 20.0,
            vision: 100.0,
            cone_cos: 0.5,
            scale: 1.0,
            threshold: 0.0,
        }
    }

    fn her(at: Vec3) -> Subject {
        Subject {
            position: at,
            visibility: 1.0,
            audibility: 1.0,
            person: true,
            forgotten: false,
        }
    }

    // Seen ahead as 1 - d/reach; behind only heard, or within 2 m; the
    // slot's subject half as far again.
    #[test]
    fn a_query_sees_ahead_hears_around_and_keeps_what_counts() {
        let b = block();
        let ahead = sense(&b, &her(Vec3::new(0.0, 0.0, 50.0)), false, true).unwrap();
        assert!((ahead.sight - 0.5).abs() < 1e-5);
        assert_eq!(ahead.hearing, 0.0);
        assert_eq!(ahead.distance, 50.0);
        let r = 75.0f32;
        assert!((ahead.likelihood - r * r / (r * r + 2500.0)).abs() < 1e-5);
        let behind = sense(&b, &her(Vec3::new(0.0, 3.0, -10.0)), false, true).unwrap();
        assert_eq!(behind.sight, 0.0);
        assert!((behind.hearing - (1.0 - behind.distance / 20.0)).abs() < 1e-5);
        assert!((behind.flat_distance - 10.0).abs() < 1e-5);
        let close = sense(&b, &her(Vec3::new(0.0, 0.0, -1.5)), false, true).unwrap();
        assert!(close.sight > 0.0, "within 2 m whatever way");
        assert!(sense(&b, &her(Vec3::new(0.0, 0.0, -50.0)), false, true).is_none());
        let far = her(Vec3::new(0.0, 0.0, 120.0));
        assert!(sense(&b, &far, false, true).is_none());
        assert!(sense(&b, &far, true, true).is_some(), "urgent reaches 150");
    }

    #[test]
    fn the_table_merges_ages_and_is_bounded() {
        let b = block();
        let rec = sense(&b, &her(Vec3::new(0.0, 0.0, 10.0)), false, true).unwrap();
        let mut t = SenseTable::default();
        t.merge(7, rec);
        t.tick(0.4);
        assert!(t.get(7).unwrap().since_update > 0.39);
        t.merge(7, rec);
        assert_eq!(t.get(7).unwrap().since_update, 0.0);
        for k in 0..200 {
            t.merge(100 + k, rec);
        }
        assert_eq!(t.subjects().count(), RECORDS);
        t.forget(7);
        assert!(!t.knows(7));
        t.forget_all();
        assert_eq!(t.subjects().count(), 0);
    }

    #[test]
    fn only_a_fresh_sighting_within_her_range_by_a_soldier_feeds_the_meter() {
        let b = block();
        let rec = sense(&b, &her(Vec3::new(0.0, 0.0, 30.0)), false, true).unwrap();
        let soldier = DisguiseGate::default();
        assert!(feeds_disguise(&rec, &soldier, 40.0));
        assert!(!feeds_disguise(&rec, &soldier, 20.0), "out of her range");
        let civ = DisguiseGate {
            civilian: true,
            ..soldier
        };
        assert!(!feeds_disguise(&rec, &civ, 40.0));
        let aboard = DisguiseGate {
            task_object: true,
            aboard: true,
            ..soldier
        };
        assert!(!feeds_disguise(&rec, &aboard, 40.0));
        let going = DisguiseGate {
            task_object: true,
            ..soldier
        };
        assert!(feeds_disguise(&rec, &going, 40.0));
        let stale = SenseRecord {
            since_update: 0.6,
            ..rec
        };
        assert!(!feeds_disguise(&stale, &soldier, 40.0));
        let heard = sense(&b, &her(Vec3::new(0.0, 0.0, -10.0)), false, true).unwrap();
        assert!(!feeds_disguise(&heard, &soldier, 40.0), "heard, not seen");
        assert_eq!(
            outcome(true, true, true, 9.0),
            Some(SeenOutcome::SeenThroughDisguise)
        );
        assert_eq!(
            outcome(true, true, false, 1.0),
            Some(SeenOutcome::SeenUndisguised)
        );
        assert_eq!(outcome(true, false, true, 9.0), None);
        assert_eq!(outcome(false, true, true, 1.0), None);
    }
}
