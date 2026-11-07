//! Whom a fighter goes for: the 360's target choice among all they
//! perceive, the player being one of those. From the 360 code unless a
//! line says otherwise.
//!
//! Each subject a mind perceives has an attitude, its faction's relation
//! (-100 to 100, 0x822F0600) read at 0x822D5948. A table built into the
//! program (0x822EB658, read by 0x82163CE0) turns it into what the mind
//! may do: only below [`ATTACK_BELOW`] may it attack (bit 2, 0x822E0D68).
//! Each subject it may attack is scored (0x822E0988) and offered for the
//! mind's attack slot (0x822C0318): the one in it keeps its place with
//! its own new weight, and a challenger needs a weight above that.
//!
//! The score's base is two fields of the perception record not traced
//! (+0x68, +0xC8), taken here as 1 and 0: every subject starts at
//! [`BASE`]. What's left is the distance: within [`NEAR`] (or [`HOLD`] for
//! the one in the slot) a subject counts double, and one nearer than the
//! one in the slot gains [`CLOSER`].

/// A relation below this may be attacked (the program's first band,
/// -100 to -33, rule bits 7).
pub const ATTACK_BELOW: f32 = -33.0;
/// A subject's score before distance: 0.1 (0x820BB468) with the two
/// untraced fields at 1 and 0.
const BASE: f32 = 0.1;
/// Within this a subject counts double, metres (0x820BB1F8).
const NEAR: f32 = 30.0;
/// As [`NEAR`], for the one already in the slot (0x820B7FB8).
const HOLD: f32 = 50.0;
/// The most a score comes to (0x820D3254).
const MOST: f32 = 0.9;
/// What being nearer than the one in the slot adds (0x820B7AE0).
const CLOSER: f32 = 0.01;
/// A score's weight in the slot (0x820BAF84).
const WEIGHT: f32 = 5.0;

/// Whether a fighter whose faction has this relation to a subject's may
/// attack them.
pub fn attacks(relation: f32) -> bool {
    relation < ATTACK_BELOW
}

/// Score of a subject `distance` metres away; `current` is the distance
/// to the one in the slot, if there is one, and `held` that this is it.
pub fn score(distance: f32, current: Option<f32>, held: bool) -> f32 {
    let near = if held { HOLD } else { NEAR };
    let doubled = if distance < near { BASE * 2.0 } else { BASE };
    let closer = current.is_some_and(|c| distance < c);
    doubled.min(MOST) + if closer { CLOSER } else { 0.0 }
}

/// The mind's attack slot: who is in it and at what weight.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Slot {
    /// Who, by the caller's number.
    pub who: Option<u32>,
    weight: f32,
}

impl Slot {
    /// Offers `who` at `score` ([`score`]): the one in the slot takes the
    /// new weight, another takes the slot only with a greater one.
    pub fn offer(&mut self, who: u32, score: f32) {
        let weight = score * WEIGHT;
        if self.who == Some(who) || weight > self.weight {
            self.who = Some(who);
            self.weight = weight;
        }
    }

    /// Empties the slot.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_hostile_band_may_be_attacked() {
        assert!(attacks(-100.0));
        assert!(attacks(-33.1));
        assert!(!attacks(-33.0));
        assert!(!attacks(0.0));
    }

    #[test]
    fn near_counts_double_and_the_held_one_longer() {
        assert_eq!(score(20.0, None, false), 0.2);
        assert_eq!(score(40.0, None, false), 0.1);
        assert_eq!(score(40.0, Some(40.0), true), 0.2);
        assert!(score(35.0, Some(40.0), false) > score(40.0, None, false));
    }

    #[test]
    fn a_challenger_needs_more_weight_than_the_one_held() {
        let mut slot = Slot::default();
        slot.offer(1, score(40.0, None, false));
        slot.offer(2, score(45.0, Some(40.0), false));
        assert_eq!(slot.who, Some(1));
        slot.offer(3, score(35.0, Some(40.0), false));
        assert_eq!(slot.who, Some(3));
        slot.offer(1, score(10.0, Some(35.0), false));
        assert_eq!(slot.who, Some(1));
        slot.clear();
        assert_eq!(slot.who, None);
    }
}
