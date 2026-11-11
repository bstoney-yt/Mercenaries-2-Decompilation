//! The actions the original asks a person's animation for by name, and
//! how a grenade throw picks its action: the request side of the two
//! animation tables.
//!
//! A person's clip comes of two steps
//! ([`m2_assets::animation_table`]): the stance and action table turns a
//! stance and an action into an animation handle, and the on-foot table
//! turns the handle, the character and what they hold into a clip. The
//! tables say nothing of which action is asked for when; that is the
//! program's. The beta's fight clips were first found the other way
//! round, from a clip that looked right to the first row listing it, and
//! a clip is listed under several handles: that way gave the bash the
//! `Crouched` stance's handle and the standing take-off
//! `DieAnimatedRagdoll`'s. [`SLOT_ACTIONS`] are the slots whose action the
//! 360 code names, to be looked up forward.
//!
//! # Provenance
//!
//! From the 360 code:
//!
//! - **Bash**: the bash handler (0x8222A828) sets action 0x29EFE0B5 for a
//!   high bash and 0x47BC7419 for a low one ([`crate::melee`]); they are
//!   the hashes of `MeleeBash` and `MeleeBashLow` (tested below).
//! - **Landing**: the locomotion node's step (0x828071A8) and the blend
//!   time function (0x8241FCA8) compare the action with 0xADCED6B8 and
//!   0xC9592BE4, the hashes of `JumpLandMoving` and `JumpLandStationary`.
//!   Where the action is set was not found (no code builds the two as
//!   constants to store): which one a landing takes, moving or standing,
//!   is inferred from the names.
//! - **Throw** (0x8227E560): the action is `ThrowGrenadeRun`
//!   (0xDFD8FAC6) when the thrower is moving (0x82282CC8: the pace bits of
//!   its movement record above 1) and `ThrowGrenadeIdle` (0xE65AF5D5)
//!   otherwise, and its `ActionDirection` is set from the angle between
//!   the way the thrower faces and the way the throw goes, wrapped to half
//!   a turn either way ([`throw_way`]): `Front` under 60 degrees, `Back`
//!   over 120, else `Right` for an angle of 0 or more and `Left` below
//!   (the degrees are the doubles at 0x820F9840 and 0x820F9838, times the
//!   float pi/180 at 0x82DE2F08). The function's flag argument picks two
//!   other actions (0x5B79336E, 0x79D780AF) that the stance and action
//!   table has no row for; not named.
//!
//! From samples: the stance and action table's rows for these actions
//! under `Upright`, and the on-foot table's rows for their handles (a
//! throw's name `Grenade` in use and the gender `Hero`, with no row of a
//! hero's own; a high bash's are upper body, a low bash's whole body).
//!
//! **Inferred**: which sign of the angle is the thrower's right. The
//! program takes the facing's heading less the throw's; the beta's
//! headings are taken to turn the same way.
//!
//! **Not traced**: the take-off and the air. `JumpTakeoffMoving`,
//! `JumpTakeoffStationary` and `Jump` are compared in the player's input
//! code (0x82436A48) but the order the jump goes through them, and what
//! plays in the `InAir` stance, was not read, so the beta's take-off and
//! air clips stay as they were found.

use m2_assets::name_hash;

/// A fight clip slot whose action the 360 code names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotAction {
    /// The slot's name in the beta's logs.
    pub slot: &'static str,
    pub stance: &'static str,
    pub action: &'static str,
}

/// The slots looked up forward, by stance and action.
pub const SLOT_ACTIONS: [SlotAction; 4] = [
    SlotAction {
        slot: "land",
        stance: "Upright",
        action: "JumpLandMoving",
    },
    SlotAction {
        slot: "standing land",
        stance: "Upright",
        action: "JumpLandStationary",
    },
    SlotAction {
        slot: "bash",
        stance: "Upright",
        action: "MeleeBash",
    },
    SlotAction {
        slot: "low bash",
        stance: "Upright",
        action: "MeleeBashLow",
    },
];

/// The action of `slot`, if the 360 code names one.
pub fn slot_action(slot: &str) -> Option<SlotAction> {
    SLOT_ACTIONS.iter().copied().find(|a| a.slot == slot)
}

/// Which way a throw goes from the way the thrower faces: the stance and
/// action table's `ActionDirection`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ThrowWay {
    Front,
    Back,
    Left,
    Right,
}

impl ThrowWay {
    pub const ALL: [Self; 4] = [Self::Front, Self::Back, Self::Left, Self::Right];

    /// The table's word for it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Front => "Front",
            Self::Back => "Back",
            Self::Left => "Left",
            Self::Right => "Right",
        }
    }

    /// The word's hash, as the table's cells hold it.
    pub fn hash(self) -> u32 {
        name_hash(self.name())
    }
}

/// Under this the throw goes to the front, radians (60 degrees).
pub const THROW_FRONT: f32 = 60.0 * std::f32::consts::PI / 180.0;
/// Over this it goes to the back, radians (120 degrees).
pub const THROW_BACK: f32 = 120.0 * std::f32::consts::PI / 180.0;

/// The way a throw goes for `angle`, the thrower's heading less the
/// throw's, radians (any size: wrapped to half a turn either way here).
pub fn throw_way(angle: f32) -> ThrowWay {
    use std::f32::consts::{PI, TAU};
    let a = (angle + PI).rem_euclid(TAU) - PI;
    if a.abs() < THROW_FRONT {
        ThrowWay::Front
    } else if a.abs() > THROW_BACK {
        ThrowWay::Back
    } else if a >= 0.0 {
        ThrowWay::Right
    } else {
        ThrowWay::Left
    }
}

/// The throw's action: on the move or standing.
pub fn throw_action(moving: bool) -> &'static str {
    if moving {
        "ThrowGrenadeRun"
    } else {
        "ThrowGrenadeIdle"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names hash to the words the program compares and stores.
    #[test]
    fn names_hash_to_the_programs_words() {
        for (name, hash) in [
            ("MeleeBash", 0x29ef_e0b5),
            ("MeleeBashLow", 0x47bc_7419),
            ("JumpLandMoving", 0xadce_d6b8),
            ("JumpLandStationary", 0xc959_2be4),
            ("ThrowGrenadeIdle", 0xe65a_f5d5),
            ("ThrowGrenadeRun", 0xdfd8_fac6),
            ("Front", 0xe4d5_3df6),
            ("Back", 0xe490_c918),
            ("Right", 0x1ca1_1cdd),
            ("Left", 0x25f2_11ae),
        ] {
            assert_eq!(name_hash(name), hash, "{name}");
        }
        assert_eq!(throw_action(false), "ThrowGrenadeIdle");
        assert_eq!(throw_action(true), "ThrowGrenadeRun");
        assert_eq!(ThrowWay::Right.hash(), 0x1ca1_1cdd);
    }

    #[test]
    fn slots_have_one_action_each() {
        assert_eq!(slot_action("bash").map(|a| a.action), Some("MeleeBash"));
        assert_eq!(
            slot_action("low bash").map(|a| a.action),
            Some("MeleeBashLow")
        );
        assert_eq!(slot_action("take-off"), None);
        for (i, a) in SLOT_ACTIONS.iter().enumerate() {
            assert!(SLOT_ACTIONS[..i].iter().all(|b| b.slot != a.slot));
        }
    }

    #[test]
    fn throw_way_follows_the_angle() {
        let deg = |d: f32| d.to_radians();
        assert_eq!(throw_way(0.0), ThrowWay::Front);
        assert_eq!(throw_way(deg(59.0)), ThrowWay::Front);
        assert_eq!(throw_way(deg(-59.0)), ThrowWay::Front);
        assert_eq!(throw_way(deg(61.0)), ThrowWay::Right);
        assert_eq!(throw_way(deg(119.0)), ThrowWay::Right);
        assert_eq!(throw_way(deg(-61.0)), ThrowWay::Left);
        assert_eq!(throw_way(deg(-119.0)), ThrowWay::Left);
        assert_eq!(throw_way(deg(121.0)), ThrowWay::Back);
        assert_eq!(throw_way(deg(-121.0)), ThrowWay::Back);
        assert_eq!(throw_way(deg(180.0)), ThrowWay::Back);
        // Wrapped: 350 degrees is 10 the other way.
        assert_eq!(throw_way(deg(350.0)), ThrowWay::Front);
        assert_eq!(throw_way(deg(270.0)), ThrowWay::Left);
        assert_eq!(throw_way(deg(-270.0)), ThrowWay::Right);
    }
}
