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
//! Not from the original: the beta's clips are not all driven by human
//! states. Locomotion within `Idle` (idle, walk, run, sprint) is blended by
//! a separate original locomotion node not traced yet; the beta crossfades
//! those changes with [`DEFAULT`] (Invented, the action blend standing in).

/// The blend into most actions, seconds.
pub const DEFAULT: f32 = 0.2;

/// The quick blend into staggers, the animated-ragdoll death and from a
/// landing to `Idle`, seconds: the program's float 0.06667 (about 1/15).
pub const QUICK: f32 = 0.066_67;

/// Fades shorter than this snap.
pub const INSTANT: f32 = 1e-4;

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
