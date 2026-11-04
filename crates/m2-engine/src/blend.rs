//! How long a character crossfades into a new action, as the original
//! decides it.
//!
//! A human's animation driver changes clip when its action changes (its
//! human state: a stance and an action, such as `Upright` `Jump`). The
//! crossfade into the new action's clip lasts [`time`]`(previous, next)`
//! seconds, a linear fade whose weight rises at `1 / time` per second; a
//! time under [`INSTANT`] snaps.
//!
//! | Into | Blend |
//! |---|---|
//! | the first action (none before it) | 0 |
//! | `Jump`, `JumpLandMoving`, `JumpLandStationary`, `ExitVehicle`, `ExitVehicleOpenDoor`, `CrawlExitVehicle`, `GrabLadder`, `Getup`, `MeleeGetup`, `SubdueDefender`, `ActionHijackHijackerA`, `ActionHijackHijackerB`, `ActionHijackHijackeeA` | 0 (a snap) |
//! | `StaggerShort`, `StaggerInPlace`, `StaggerLong`, `DieAnimatedRagdoll` | [`QUICK`] |
//! | `Idle` from `JumpLandMoving` or `JumpLandStationary` | [`QUICK`] |
//! | anything else | [`DEFAULT`] |
//!
//! Actions are named by [`name_hash`](m2_assets::name_hash) of the name, as
//! the original compares them.
//!
//! # Provenance
//!
//! From the 360 code: the table is the blend-time function at
//! `0x8241FCA8`, which the human animation driver (`0x82420A00`) calls
//! with the previous and new actions when the action's row changes and
//! hands to the transition it builds (`0x82421C80`). That applies it with
//! `0x827FA6F0`, the linear fade (rate `1 / time`, snap under `1e-4`). The
//! constants are the program's floats: 0.2 (`0x820B8054`), 0.0
//! (`0x820BAF14`) and 0.06667 (`0x820F8C00`). The action hashes match
//! their names (tested below).
//!
//! The driver also lets a script override the time for one change: a
//! component (`0x83656C38`) holds a time, -1 for none; when the action
//! changes and it is 0 or more it is used instead of [`time`] and set back
//! to -1. [`Animator::override_next_blend`](crate::Animator::override_next_blend)
//! is that override; which script call sets it in the original is not
//! traced yet, so nothing in the beta calls it. The 360 program's
//! `transitions` animation table (`ab8fe34b`) has a `TransitionDuration`
//! column but no reader was found, so it plays no part.
//!
//! # Verification
//!
//! Verified in play (m2-alpha shot runs on 2026-10-27, read from the
//! `anim: blend` log lines): landings snapping after moving and standing
//! take-offs, a landing settling into `Idle` over [`QUICK`], and the
//! hijack actions (`ActionHijackHijackerA`, `ActionHijackHijackerB`)
//! snapping. Take-offs logged no line, which fits [`DEFAULT`]; their time
//! was not measured directly.
//!
//! **Not verified at runtime**, covered by unit tests only:
//! - `Getup`, the NPC get-up (needs a mission run);
//! - the air `Jump` (the beta plays it only when no take-off clip is
//!   bound);
//! - the staggers, `DieAnimatedRagdoll` and the vehicle exits (the beta
//!   has no clips for them yet);
//! - the script override (nothing calls it yet);
//! - how any of it looks (no frame-stepped comparison with PC footage).
//!
//! # Locomotion
//!
//! Within `Idle` a stance's locomotion node plays the gaits ([`gait`],
//! [`GAIT`]). From the 360 code: the driver builds the node per stance
//! (0x8241F878: `Upright` 0x82802B10, `Crouched` 0x8280EC68, `Swim`
//! 0x827F9890, all one class), blending into a new stance's node over
//! [`DEFAULT`] (0 into `InVehicle`). The node's step (0x828071A8) keeps
//! it in one of three modes: moving forward (enter 0x82808018, step
//! 0x82808468), moving backward, or standing (enter 0x82809B98), standing
//! when its speed is at most [`STILL`].
//!
//! Moving, it picks a gait from its speed (0x82807F70, [`gait`]) between
//! thresholds set halfway between the template's `_HumanPhysics` paces
//! (0x828070D0). On a change it fades the old gait's weights out and the
//! new one's in over [`GAIT`] seconds, linearly (0x827FA6F0 and
//! 0x827FA620, the same fade as the actions'); going between standing and
//! moving fades over the same 0.3 s (the float at 0x8206F940). Every gait
//! clip plays at the same point of its stride: the gait playing is moved
//! on by `dt * speed / clip speed` and the others are set to the same
//! fraction of their length (the tail of 0x82808468), so the feet stay in
//! step through a blend. A clip of no measurable speed counts as 1, 2.5,
//! 5 and 5 m/s for the four gaits.
//!
//! The four gaits' handles are the node's list at 0x82B4DA98:
//! `Upright_Walk_Forward`, `Upright_Jog_Forward`, `Upright_Run_Forward`
//! and `Upright_Sprint_Forward` (entries 10 to 7, matched by name hash).
//! The beta has no sprint clip bound, so its [`LocoGait::Sprint`] plays the
//! run's (not from the original).
//!
//! **Not ported**: the backward mode's gaits, the node's lean and the
//! turn-in-place clips of the standing mode, and the carry gaits (the
//! node's second set, picked by a flag in the human state, with its own
//! threshold halfway between `CarryWalkSpeed` and `CarryJogSpeed`).

/// The blend into most actions, seconds.
pub const DEFAULT: f32 = 0.2;

/// The quick blend into staggers, the animated-ragdoll death and from a
/// landing to `Idle`, seconds: the program's float 0.06667 (about 1/15).
pub const QUICK: f32 = 0.066_67;

/// Fades shorter than this snap.
pub const INSTANT: f32 = 1e-4;

/// The locomotion node's fade between gaits, and between standing and
/// moving, seconds: the program's floats at 0x8206F93C and 0x8206F940.
pub const GAIT: f32 = 0.3;

/// The locomotion node stands when its speed is at most this, m/s (the
/// float at 0x82052394, compared in 0x828071A8).
pub const STILL: f32 = 0.01;

/// The node's gait threshold above the run when the template has no
/// faster sprint (the float at 0x82108BFC, as 0x828070D0 stores it).
const NO_SPRINT: f32 = 2.003_358;

/// A gait the locomotion node plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LocoGait {
    Still,
    Walk,
    Jog,
    Run,
    Sprint,
}

/// The gait the locomotion node plays moving `speed` m/s with the
/// template's `Upright` paces `[walk, jog, run, sprint]` (0x82807F70
/// with the thresholds of 0x828070D0): below halfway from walk to jog it
/// walks, below halfway from jog to run it jogs, below halfway from run to
/// sprint it runs, and otherwise sprints. With no sprint faster than the
/// run, the last threshold is a constant below the others, so anything at
/// or past the jog-to-run threshold sprints.
pub fn gait(paces: [f32; 4], speed: f32) -> LocoGait {
    if speed <= STILL {
        return LocoGait::Still;
    }
    let [walk, jog, run, sprint] = paces;
    let fast = if sprint > run {
        (run + sprint) * 0.5
    } else {
        NO_SPRINT
    };
    if speed < (walk + jog) * 0.5 {
        LocoGait::Walk
    } else if speed < (jog + run) * 0.5 {
        LocoGait::Jog
    } else if speed < fast {
        LocoGait::Run
    } else {
        LocoGait::Sprint
    }
}

/// Action name hashes the blend times depend on.
pub mod action {
    /// The upright action locomotion plays in (idle, walk, run).
    pub const IDLE: u32 = 0xb4da_003b;
    /// An action with no special blend, for a clip the beta plays outside
    /// the human states (a pickup, a bash, a scripted clip).
    pub const UNNAMED: u32 = 0;
    pub const JUMP: u32 = 0x6fbe_1165;
    pub const JUMP_TAKEOFF_MOVING: u32 = 0x8a05_004d;
    pub const JUMP_TAKEOFF_STATIONARY: u32 = 0xfb4d_afb5;
    pub const JUMP_LAND_MOVING: u32 = 0xadce_d6b8;
    pub const JUMP_LAND_STATIONARY: u32 = 0xc959_2be4;
    pub const EXIT_VEHICLE: u32 = 0x57b2_5a1f;
    pub const EXIT_VEHICLE_OPEN_DOOR: u32 = 0x253f_f6bb;
    pub const CRAWL_EXIT_VEHICLE: u32 = 0x10dc_cd7e;
    pub const GRAB_LADDER: u32 = 0x8484_7ed7;
    pub const GETUP: u32 = 0xa433_86b8;
    pub const MELEE_GETUP: u32 = 0x8f2b_12ce;
    pub const SUBDUE_DEFENDER: u32 = 0xc79e_9b70;
    pub const HIJACK_HIJACKER_A: u32 = 0x779c_968f;
    pub const HIJACK_HIJACKER_B: u32 = 0xed9f_8ee8;
    pub const HIJACK_HIJACKEE_A: u32 = 0xae7e_0d1e;
    pub const STAGGER_SHORT: u32 = 0x3eb6_2604;
    pub const STAGGER_IN_PLACE: u32 = 0x03ca_a640;
    pub const STAGGER_LONG: u32 = 0x9dfc_6a70;
    pub const DIE_ANIMATED_RAGDOLL: u32 = 0xd873_91b3;
}

use action::*;

/// Actions entered without a blend.
const SNAP: [u32; 13] = [
    JUMP,
    JUMP_LAND_MOVING,
    JUMP_LAND_STATIONARY,
    EXIT_VEHICLE,
    EXIT_VEHICLE_OPEN_DOOR,
    CRAWL_EXIT_VEHICLE,
    GRAB_LADDER,
    GETUP,
    MELEE_GETUP,
    SUBDUE_DEFENDER,
    HIJACK_HIJACKER_A,
    HIJACK_HIJACKER_B,
    HIJACK_HIJACKEE_A,
];

/// Actions entered with the [`QUICK`] blend.
const QUICK_INTO: [u32; 4] = [
    STAGGER_SHORT,
    STAGGER_IN_PLACE,
    STAGGER_LONG,
    DIE_ANIMATED_RAGDOLL,
];

/// The crossfade into action `next` from `previous` (`None` for the
/// first), seconds.
pub fn time(previous: Option<u32>, next: u32) -> f32 {
    let Some(previous) = previous else {
        return 0.0;
    };
    if (previous == JUMP_LAND_MOVING || previous == JUMP_LAND_STATIONARY) && next == IDLE {
        return QUICK;
    }
    if SNAP.contains(&next) {
        0.0
    } else if QUICK_INTO.contains(&next) {
        QUICK
    } else {
        DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_assets::name_hash;

    #[test]
    fn action_hashes_are_their_names() {
        for (hash, name) in [
            (IDLE, "Idle"),
            (JUMP, "Jump"),
            (JUMP_TAKEOFF_MOVING, "JumpTakeoffMoving"),
            (JUMP_TAKEOFF_STATIONARY, "JumpTakeoffStationary"),
            (JUMP_LAND_MOVING, "JumpLandMoving"),
            (JUMP_LAND_STATIONARY, "JumpLandStationary"),
            (EXIT_VEHICLE, "ExitVehicle"),
            (EXIT_VEHICLE_OPEN_DOOR, "ExitVehicleOpenDoor"),
            (CRAWL_EXIT_VEHICLE, "CrawlExitVehicle"),
            (GRAB_LADDER, "GrabLadder"),
            (GETUP, "Getup"),
            (MELEE_GETUP, "MeleeGetup"),
            (SUBDUE_DEFENDER, "SubdueDefender"),
            (HIJACK_HIJACKER_A, "ActionHijackHijackerA"),
            (HIJACK_HIJACKER_B, "ActionHijackHijackerB"),
            (HIJACK_HIJACKEE_A, "ActionHijackHijackeeA"),
            (STAGGER_SHORT, "StaggerShort"),
            (STAGGER_IN_PLACE, "StaggerInPlace"),
            (STAGGER_LONG, "StaggerLong"),
            (DIE_ANIMATED_RAGDOLL, "DieAnimatedRagdoll"),
        ] {
            assert_eq!(hash, name_hash(name), "{name}");
        }
    }

    #[test]
    fn the_default_blend_is_a_fifth_of_a_second() {
        let some = name_hash("Pickup");
        assert_eq!(time(Some(IDLE), some), 0.2);
        assert_eq!(time(Some(some), IDLE), 0.2);
        assert_eq!(time(Some(UNNAMED), UNNAMED), 0.2);
        // Take-offs are not in the snap list, only the air `Jump`.
        assert_eq!(time(Some(IDLE), JUMP_TAKEOFF_MOVING), 0.2);
        assert_eq!(time(Some(IDLE), JUMP_TAKEOFF_STATIONARY), 0.2);
    }

    #[test]
    fn the_first_action_snaps() {
        assert_eq!(time(None, IDLE), 0.0);
        assert_eq!(time(None, STAGGER_LONG), 0.0);
    }

    #[test]
    fn jumps_landings_exits_ladders_getups_and_hijacks_snap() {
        for next in SNAP {
            assert_eq!(time(Some(IDLE), next), 0.0, "{next:08x}");
            assert_eq!(time(Some(JUMP), next), 0.0, "{next:08x}");
        }
    }

    #[test]
    fn staggers_and_the_ragdoll_death_blend_quickly() {
        for next in QUICK_INTO {
            assert_eq!(time(Some(IDLE), next), QUICK, "{next:08x}");
        }
        assert!((QUICK - 1.0 / 15.0).abs() < 1e-4);
    }

    #[test]
    fn gaits_change_halfway_between_the_paces() {
        use super::{LocoGait::*, gait};
        // Jennifer: 2, 5, 7 and 12 m/s; thresholds 3.5, 6 and 9.5.
        let jen = [2.0, 5.0, 7.0, 12.0];
        assert_eq!(gait(jen, 0.0), Still);
        assert_eq!(gait(jen, 0.01), Still);
        assert_eq!(gait(jen, 3.49), Walk);
        assert_eq!(gait(jen, 3.5), Jog);
        assert_eq!(gait(jen, 5.99), Jog);
        assert_eq!(gait(jen, 6.0), Run);
        assert_eq!(gait(jen, 9.49), Run);
        assert_eq!(gait(jen, 9.5), Sprint);
        // The program's defaults, no sprint: 1.85, then 3.75 and on.
        let people = [1.2, 2.5, 5.0, -1.0];
        assert_eq!(gait(people, 1.84), Walk);
        assert_eq!(gait(people, 1.85), Jog);
        assert_eq!(gait(people, 3.74), Jog);
        assert_eq!(gait(people, 3.75), Sprint);
        assert_eq!(gait(people, 5.0), Sprint);
        assert_eq!((super::GAIT, super::STILL), (0.3, 0.01));
    }

    #[test]
    fn a_landing_settles_into_idle_quickly() {
        assert_eq!(time(Some(JUMP_LAND_MOVING), IDLE), QUICK);
        assert_eq!(time(Some(JUMP_LAND_STATIONARY), IDLE), QUICK);
        // Only into Idle: a landing into anything else blends as usual.
        assert_eq!(time(Some(JUMP_LAND_MOVING), UNNAMED), DEFAULT);
        assert_eq!(time(Some(JUMP_LAND_STATIONARY), JUMP), 0.0);
    }

    #[test]
    fn the_beta_values_are_gone() {
        // The beta faded 0.25 s into loops and 0.1 s into one-shots
        // whatever the clips (invented); neither is an original time.
        for previous in [None, Some(IDLE), Some(JUMP_LAND_MOVING)] {
            for next in [IDLE, UNNAMED, JUMP, STAGGER_SHORT] {
                let t = time(previous, next);
                assert!(t != 0.25 && t != 0.1, "{t}");
            }
        }
    }
}
