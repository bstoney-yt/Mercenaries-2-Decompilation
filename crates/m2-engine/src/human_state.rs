//! The stance and action a person is in, moved by the messages the
//! original's code posts and the human state table
//! ([`m2_assets::human_states`]).
//!
//! # Provenance
//!
//! From the 360 code:
//!
//! - A person's animation driver (0x82420A00) reads their stance and
//!   action from their human-state record and asks the stance and action
//!   table for the handle. The record changes only by a message: code
//!   posts `{person, message}` (0x821752B8 to the queue at 0x8323F3C8)
//!   and the state table gives the stance and action the message leads to
//!   from where they are. A message the state has no transition for does
//!   nothing.
//! - **The jump's messages.** Jump pressed, the player control
//!   (0x824333E0) posts `JumpMoving` (0x4BB71A5F) when the squared length
//!   of the stick's move is over [`JUMP_MOVING_STICK_SQ`] (the float
//!   0.5625 at 0x820F91FC: a push past three quarters), else
//!   `JumpStationary` (0x5CBC3867). The human physics, when its Havok
//!   state changes (0x8217C9F0), posts `JumpInAir` (0xEDBFCB78) for the
//!   jumping state, `InAir` and `Falling` for the two in-air states, and
//!   on the ground `MovingOnGround` (0x190F4575) when a dot product is
//!   over [`MOVING_ON_GROUND`] (the float 2.5 at 0x820D9EE0), else
//!   `OnGround` (0xAAF68AD3).
//! - **The launch.** The take-off's clip carries a `jump` event (0.10 s
//!   into its 0.17 to 0.20 s, from samples); the driver's handler
//!   (0x82420208) answers that hash by sending the physics its jump. So
//!   the body leaves the ground a tenth of a second after the press.
//!
//! From samples (the state table): `Upright` `Idle` on `JumpMoving` goes
//! to `Upright` `JumpTakeoffMoving` (`JumpStationary`:
//! `JumpTakeoffStationary`); those on `JumpInAir` to `Upright` `Jump`;
//! that on `Complete` (its clip ended) to `InAir` `Jump`; either on
//! `OnGround` to `Upright` `JumpLandStationary` and on `MovingOnGround`
//! to `JumpLandMoving`; those on `Complete` to `Upright` `Idle`.
//!
//! **Inferred**: the dot product's vectors. They read as the character
//! proxy's velocity and the way the person faces, so [`ground_message`]
//! takes the speed along the facing.

use m2_assets::human_states::HumanStates;
use m2_assets::name_hash;

/// The messages a jump goes by.
pub mod message {
    pub const JUMP_MOVING: &str = "JumpMoving";
    pub const JUMP_STATIONARY: &str = "JumpStationary";
    pub const JUMP_IN_AIR: &str = "JumpInAir";
    pub const ON_GROUND: &str = "OnGround";
    pub const MOVING_ON_GROUND: &str = "MovingOnGround";
    pub const FALLING: &str = "Falling";
    /// The action's clip ended.
    pub const COMPLETE: &str = "Complete";
}

/// The event of a take-off clip that launches the jump.
pub const JUMP_EVENT: &str = "jump";

/// Over this squared length of the stick's move, a jump is `JumpMoving`.
pub const JUMP_MOVING_STICK_SQ: f32 = 0.5625;

/// Over this, touching down is `MovingOnGround`, m/s.
pub const MOVING_ON_GROUND: f32 = 2.5;

/// The message the jump button posts, for the stick's move (its length 1
/// at a full push).
pub fn jump_message(stick_sq: f32) -> &'static str {
    if stick_sq > JUMP_MOVING_STICK_SQ {
        message::JUMP_MOVING
    } else {
        message::JUMP_STATIONARY
    }
}

/// The message the physics posts on touching the ground, moving
/// `forward_speed` m/s the way the person faces.
pub fn ground_message(forward_speed: f32) -> &'static str {
    if forward_speed > MOVING_ON_GROUND {
        message::MOVING_ON_GROUND
    } else {
        message::ON_GROUND
    }
}

/// The stance and action a person is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HumanState {
    pub stance: String,
    pub action: String,
}

impl Default for HumanState {
    /// `Upright` `Idle`.
    fn default() -> Self {
        Self {
            stance: "Upright".to_owned(),
            action: "Idle".to_owned(),
        }
    }
}

impl HumanState {
    /// Whether this is `stance` and `action`, by hash as the game
    /// compares them.
    pub fn is(&self, stance: &str, action: &str) -> bool {
        name_hash(&self.stance) == name_hash(stance) && name_hash(&self.action) == name_hash(action)
    }

    /// Posts `message`: moves to the stance and action `table` gives it
    /// from here. `false`, and no change, when the state has no
    /// transition for it.
    pub fn post(&mut self, table: &HumanStates, message: &str) -> bool {
        let Some(t) = table.next(&self.stance, &self.action, message) else {
            return false;
        };
        self.stance = t.stance.clone();
        self.action = t.action.clone();
        true
    }
}

#[cfg(test)]
mod tests {
    use m2_assets::human_states::{Action, Stance, Transition};

    use super::*;

    fn action(name: &str, to: &[[&str; 3]]) -> Action {
        Action {
            name: name.to_owned(),
            transitions: to
                .iter()
                .map(|t| Transition {
                    message: t[0].to_owned(),
                    stance: t[1].to_owned(),
                    action: t[2].to_owned(),
                    unk_0x0c: 0,
                })
                .collect(),
            ..Action::default()
        }
    }

    /// The jump's rows, in the game's own words.
    fn table() -> HumanStates {
        let land = [
            ["OnGround", "Upright", "JumpLandStationary"],
            ["MovingOnGround", "Upright", "JumpLandMoving"],
        ];
        HumanStates {
            unk_0x00: 5,
            stances: vec![
                Stance {
                    name: "Upright".to_owned(),
                    actions: vec![
                        action(
                            "Idle",
                            &[
                                ["JumpMoving", "Upright", "JumpTakeoffMoving"],
                                ["JumpStationary", "Upright", "JumpTakeoffStationary"],
                                ["JumpInAir", "Upright", "Jump"],
                            ],
                        ),
                        action(
                            "JumpTakeoffMoving",
                            &[
                                ["Complete", "Upright", "Idle"],
                                ["JumpInAir", "Upright", "Jump"],
                            ],
                        ),
                        action(
                            "JumpTakeoffStationary",
                            &[
                                ["Complete", "Upright", "Idle"],
                                ["JumpInAir", "Upright", "Jump"],
                            ],
                        ),
                        action("Jump", &[["Complete", "InAir", "Jump"], land[0], land[1]]),
                        action("JumpLandMoving", &[["Complete", "Upright", "Idle"]]),
                        action("JumpLandStationary", &[["Complete", "Upright", "Idle"]]),
                    ],
                },
                Stance {
                    name: "InAir".to_owned(),
                    actions: vec![action("Jump", &[land[0], land[1]])],
                },
            ],
        }
    }

    #[test]
    fn the_messages_hash_to_the_programs_words() {
        for (name, hash) in [
            (message::JUMP_MOVING, 0x4bb7_1a5f),
            (message::JUMP_STATIONARY, 0x5cbc_3867),
            (message::JUMP_IN_AIR, 0xedbf_cb78),
            (message::ON_GROUND, 0xaaf6_8ad3),
            (message::MOVING_ON_GROUND, 0x190f_4575),
            (message::FALLING, 0x1f8c_36e6),
            (message::COMPLETE, 0xb674_1f78),
            (JUMP_EVENT, 0x6fbe_1165),
        ] {
            assert_eq!(name_hash(name), hash, "{name}");
        }
    }

    #[test]
    fn the_stick_and_the_landing_pick_the_message() {
        assert_eq!(jump_message(0.0), message::JUMP_STATIONARY);
        assert_eq!(jump_message(0.5625), message::JUMP_STATIONARY);
        assert_eq!(jump_message(0.76 * 0.76), message::JUMP_MOVING);
        assert_eq!(ground_message(2.5), message::ON_GROUND);
        assert_eq!(ground_message(2.6), message::MOVING_ON_GROUND);
        assert_eq!(ground_message(-4.0), message::ON_GROUND);
    }

    #[test]
    fn a_running_jump_goes_through_its_states() {
        let t = table();
        let mut s = HumanState::default();
        let mut been = Vec::new();
        for m in [
            message::JUMP_MOVING,
            message::JUMP_IN_AIR,
            message::COMPLETE,
            message::MOVING_ON_GROUND,
            message::COMPLETE,
        ] {
            assert!(s.post(&t, m), "{m}");
            been.push(format!("{}.{}", s.stance, s.action));
        }
        assert_eq!(
            been,
            [
                "Upright.JumpTakeoffMoving",
                "Upright.Jump",
                "InAir.Jump",
                "Upright.JumpLandMoving",
                "Upright.Idle"
            ]
        );
    }

    #[test]
    fn a_message_the_state_has_no_row_for_changes_nothing() {
        let t = table();
        let mut s = HumanState::default();
        assert!(!s.post(&t, message::ON_GROUND));
        assert!(s.is("upright", "IDLE"));
        // A take-off whose clip ends before she leaves the ground goes
        // back to the idle.
        assert!(s.post(&t, message::JUMP_STATIONARY));
        assert!(s.is("Upright", "JumpTakeoffStationary"));
        assert!(s.post(&t, message::COMPLETE));
        assert!(s.is("Upright", "Idle"));
        // A landing before the rise's clip ends, from `Upright` `Jump`.
        assert!(s.post(&t, message::JUMP_IN_AIR));
        assert!(s.post(&t, message::ON_GROUND));
        assert!(s.is("Upright", "JumpLandStationary"));
    }
}
