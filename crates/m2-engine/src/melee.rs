//! A melee bash, as the 360 lands one: who it turns to, whether it goes
//! high or low, who it reaches, how hard it shoves and how long it fells.
//!
//! # Provenance
//!
//! From the 360 code (checked in the animation thread's own Ghidra copy;
//! first traced by the research thread, notes s21):
//!
//! - The bash button, and an AI's close-quarters strike, post message
//!   0xBC11E4B7 {attacker, target} (0x822D2348, 0x824333E0). Its handler
//!   (0x8222A828) skips an attacker already bashing.
//! - **Turning to a target** (0x8222AE60): of the objects within
//!   [`TARGET_RADIUS`] of the attacker, the one with the lowest
//!   [`target_score`] (0x8222AC30), and the attacker's facing snaps to it.
//! - **High or low** (0x8222A828 with the box query 0x8222AFE8): two boxes
//!   ahead of the attacker, turned with them, [`HIGH`] (0x82DB5D80) and
//!   [`LOW`] (0x82DB5DA0). The bash goes low (action 0x47BC7419) when
//!   something is in the low box and nothing in the high one, otherwise
//!   high (0x29EFE0B5).
//! - **The blow** (the action's hit event, 0x8222B1A0): everyone in that
//!   bash's box is hit (0x8222B2E8). A person takes the damage table's cell
//!   for their row and `MeleeBash` (0x29EFE0B5, damage of type 3), an
//!   impulse of the `Bash` record's `BashHumanForceHeightHigh` (60) for a
//!   high bash or `...Low` (300) for a low one (0x8222B7A8), along the
//!   line from the attacker to them, at the attacker's position raised by
//!   the box's push height; and a knockdown ([`knockdown_seconds`]). The
//!   player hit takes only the damage and a 0.15 camera shake
//!   ([`SHAKE_HIT`]); a player landing a blow gets a 0.12 shake
//!   ([`SHAKE_HITTER`]).
//!
//! Read from use, not the record layout: the box words are a size (across,
//! up, ahead) then an offset of its middle from the attacker's position,
//! then the push height. Taken as full sizes, the low box runs from the
//! ground to 1.25 m and the high box on from there to 1.75 m, which is
//! what makes "low" mean "something below the chest" (inferred).
//!
//! - **The impulse's way to the body** (traced for this module): the
//!   blow's record {target, force, direction, point} goes on the impulse
//!   queue 0x8319C8B8 (0x8217D758). The physics pass (0x821544F0) hands
//!   each to its target's physics object (slot 0xF0, 0x82185320, which
//!   every physics class shares, a person's proxy class 0x820C7E48 among
//!   them). That makes it the impulse `force * direction`, kg m/s, and
//!   applies it at the point (slot 0x14, 0x82181858) to the object's
//!   rigid body of the record's index (+0x4C, +0x50). That is a Havok
//!   point impulse (0x8216DD38 calling 0x8262AC00 on the body's +0x38,
//!   skipping motion type 7), so the speed it gives is the impulse over
//!   that body's own mass. Another producer, 0x8243DFA8, multiplies its
//!   force by each ragdoll body's mass (0x8262C120), which confirms the
//!   record holds an impulse.
//! - **The knockdown's spread** (0x82423848): `r` is one draw of the
//!   engine's generator, stream 1 (`0x8215E260(1)`, state `0x82B5E588`,
//!   seeded `0x94153A94`; `m2_game::script_math::EngineRng`).
//!
//! **Inferred**: the body a standing person's impulse moves, and so its
//! mass. A person's proxy (0x82176F38 constructor) reads its
//! `_HumanPhysics` record, but the code that builds its body is VMX128
//! and decompiles cut short, so whether that body carries the record's
//! `Mass` (field 0, registered default 100, 0x823EAE98) was not seen.
//! Here the change of speed is `impulse / Mass` with that 100.

use m2_math::glam::{Vec2, Vec3};

/// A melee box: its size (across, up, ahead), its middle relative to the
/// attacker's position and facing, and the height above their position
/// the shove pushes from, metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BashBox {
    pub size: Vec3,
    pub middle: Vec3,
    pub push_height: f32,
}

/// The high bash's box (0x82DB5D80).
pub const HIGH: BashBox = BashBox {
    size: Vec3::new(0.8, 0.5, 1.2),
    middle: Vec3::new(0.0, 1.5, 0.7),
    push_height: 1.0,
};

/// The low bash's box (0x82DB5DA0).
pub const LOW: BashBox = BashBox {
    size: Vec3::new(0.8, 1.25, 1.2),
    middle: Vec3::new(0.0, 0.625, 0.7),
    push_height: 0.5,
};

/// How far around the attacker a target is looked for, metres (the
/// float at 0x820C77A4).
pub const TARGET_RADIUS: f32 = 2.0;

/// The `Bash` record's registered defaults (schema 0x823F43D8):
/// `BashKnockdownSeconds`, `BashHumanForceHeightHigh` and `...Low`.
pub const KNOCKDOWN_SECONDS: f32 = 6.0;
pub const FORCE_HIGH: f32 = 60.0;
pub const FORCE_LOW: f32 = 300.0;

/// The camera shakes: the player hit (0x820BB49C) and the player landing
/// a blow (0x820F9944).
pub const SHAKE_HIT: f32 = 0.15;
pub const SHAKE_HITTER: f32 = 0.12;

/// How a target stands to the attacker, for [`target_score`]: the
/// relation the original reads (0x822F09F0) is under -33 for an enemy,
/// over 33 for a friend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    Enemy,
    Neutral,
    Friend,
}

/// How unlikely `target` is to be turned to (lower is likelier;
/// 0x8222AC30): its flat squared distance, plus 10000 for a vehicle, 50
/// for a neutral and 100 for a friend.
pub fn target_score(flat_distance: f32, vehicle: bool, relation: Relation) -> f32 {
    let mut score = flat_distance * flat_distance;
    if vehicle {
        score += 10_000.0;
    }
    score
        + match relation {
            Relation::Enemy => 0.0,
            Relation::Neutral => 50.0,
            Relation::Friend => 100.0,
        }
}

/// Whether a body standing at `feet`, `radius` wide and `height` tall,
/// reaches into box `b` of an attacker at `from` facing `facing`
/// (radians, 0 along +z).
pub fn reaches(b: &BashBox, from: Vec3, facing: f32, feet: Vec3, radius: f32, height: f32) -> bool {
    let to = feet - from;
    let (s, c) = facing.sin_cos();
    // Into the attacker's frame: across (x), ahead (z).
    let ahead = to.x * s + to.z * c;
    let across = to.x * c - to.z * s;
    let half = b.size * 0.5;
    let low = b.middle.y - half.y;
    let high = b.middle.y + half.y;
    across.abs() <= half.x + radius
        && (ahead - b.middle.z).abs() <= half.z + radius
        && to.y <= high
        && to.y + height >= low
}

/// Whether a bash goes low: something in the low box and nothing in the
/// high one.
pub fn goes_low(in_high: bool, in_low: bool) -> bool {
    in_low && !in_high
}

/// The impulse a person takes from a bash (`low` or high) by an attacker
/// at `from`, standing at `to`, as a change of speed for a body of
/// `mass` kg (see the provenance), m/s.
pub fn shove(low: bool, from: Vec3, to: Vec3, mass: f32) -> Vec3 {
    let force = if low { FORCE_LOW } else { FORCE_HIGH };
    (to - from).normalize_or_zero() * force / mass.max(1.0)
}

/// How long a bash fells someone, seconds (0x82423848): the record's
/// `seconds` (half a second instead for a player's character), at least
/// 1e-4, times `1 + 0.4 (r - 0.5)` for `r` a random number in [0, 1].
pub fn knockdown_seconds(seconds: f32, player: bool, r: f32) -> f32 {
    let base = if player { 0.5 } else { seconds }.max(1e-4);
    (((r - 0.5) * 0.4 + 1.0) * base).max(1e-4)
}

/// The yaw from `from` to `to` (radians, 0 along +z), as the attacker
/// snaps to its target.
pub fn yaw_to(from: Vec3, to: Vec3) -> Option<f32> {
    let d = Vec2::new(to.x - from.x, to.z - from.z);
    (d.length_squared() > 1e-8).then(|| d.x.atan2(d.y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_boxes_meet_at_chest_height() {
        assert_eq!(LOW.middle.y + LOW.size.y * 0.5, 1.25);
        assert_eq!(HIGH.middle.y - HIGH.size.y * 0.5, 1.25);
        assert_eq!(HIGH.middle.y + HIGH.size.y * 0.5, 1.75);
    }

    #[test]
    fn a_man_standing_ahead_takes_a_high_bash() {
        let at = Vec3::ZERO;
        let man = Vec3::new(0.0, 0.0, 0.9);
        let high = reaches(&HIGH, at, 0.0, man, 0.4, 1.8);
        let low = reaches(&LOW, at, 0.0, man, 0.4, 1.8);
        assert!(high && low);
        assert!(!goes_low(high, low));
        // Behind her, out of both.
        let behind = Vec3::new(0.0, 0.0, -0.9);
        assert!(!reaches(&LOW, at, 0.0, behind, 0.4, 1.8));
        // Turned to face him along +x, he is to her side, out of reach.
        assert!(!reaches(
            &HIGH,
            at,
            std::f32::consts::FRAC_PI_2,
            man,
            0.4,
            1.8
        ));
    }

    #[test]
    fn something_lying_low_takes_a_low_bash() {
        // A man knocked down: 0.5 m tall.
        let lying = Vec3::new(0.0, 0.0, 1.0);
        let high = reaches(&HIGH, Vec3::ZERO, 0.0, lying, 0.4, 0.5);
        let low = reaches(&LOW, Vec3::ZERO, 0.0, lying, 0.4, 0.5);
        assert!(goes_low(high, low));
    }

    #[test]
    fn the_low_bash_shoves_five_times_as_hard() {
        let from = Vec3::ZERO;
        let to = Vec3::new(0.0, 0.0, 1.0);
        assert_eq!(shove(false, from, to, 100.0), Vec3::new(0.0, 0.0, 0.6));
        assert_eq!(shove(true, from, to, 100.0), Vec3::new(0.0, 0.0, 3.0));
    }

    #[test]
    fn a_knockdown_lasts_six_seconds_give_or_take_a_fifth() {
        assert_eq!(knockdown_seconds(KNOCKDOWN_SECONDS, false, 0.5), 6.0);
        assert!((knockdown_seconds(6.0, false, 0.0) - 4.8).abs() < 1e-5);
        assert!((knockdown_seconds(6.0, false, 1.0) - 7.2).abs() < 1e-5);
        assert_eq!(knockdown_seconds(6.0, true, 0.5), 0.5);
    }

    #[test]
    fn enemies_near_are_turned_to_first() {
        let enemy = target_score(1.5, false, Relation::Enemy);
        let neutral = target_score(0.5, false, Relation::Neutral);
        let car = target_score(0.1, true, Relation::Enemy);
        assert!(enemy < neutral && neutral < car);
        assert!(target_score(1.0, false, Relation::Friend) > neutral);
        assert_eq!(
            yaw_to(Vec3::ZERO, Vec3::X),
            Some(std::f32::consts::FRAC_PI_2)
        );
    }
}
