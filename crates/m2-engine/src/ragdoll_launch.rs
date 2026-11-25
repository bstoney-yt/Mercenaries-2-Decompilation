//! A blast throwing a person: the 360's ragdoll controller's launch.
//!
//! From the 360 code:
//!
//! - **The push** (the Havok explosion action's person push, 0x8215CA38):
//!   [`blast_impulse`]. Its strength is `MaxForce` times the person's
//!   `ExplosionFudge` (1 without one) times the falloff `max(1 - d / R,
//!   MinForceFalloff)` times [`PUSH_SCALE`], `d` from the blast to her
//!   position. It points away from the blast across the ground and as much
//!   up (45 degrees), from her position moved by up to half a metre across
//!   and up to [`JITTER_UP`] up at random (0x824281C8).
//! - **The launch** (the controller's vfunc +0x3C, 0x824282F8, called with
//!   the controller's ragdoll flag cleared, so a limp body is launched
//!   too): [`launch_velocity`] ([`Launch`]).
//! - **Only a live ragdoll is pushed** (the explosion action's update,
//!   0x8215C560): each update of the blast's first [`BLAST_WINDOW`]
//!   seconds (its +0x238, set to 1.0 by its constructor 0x8215C110), a
//!   person not yet pushed is pushed if their ragdoll body is in the
//!   physics world (its byte +0x130: set by the body's add to the world
//!   0x821818D8, which the ragdoll's start 0x82427828 calls through vfunc
//!   +0x1C, and cleared by its removal 0x821819E8, vfunc +0x20, from the
//!   stop 0x824279F8), and a settings flag is off (byte +0xCD of the
//!   options read by 0x822193D8 from the game's settings text, key hash
//!   0xAD831DC7; 0 unless set there). A person whose ragdoll does not run
//!   within the window is not thrown; one standing is not pushed at all.
//!   The controller then keeps the launch for [`LAUNCH_FRAMES`] updates
//!   (0x82427D30); here it is taken the update the ragdoll runs.
//! - **The throw** (0x82428910, once the ragdoll runs): the bone whose
//!   body is nearest the point is the struck one and the point moves onto
//!   it; each bone takes `v * f * mass * `[`BONE_SHARE`], at most
//!   [`MASS_CAP`] times its mass, at the midpoint of the point and the
//!   bone, `f` [`bone_factor`].
//! - **The fall** (0x82428768, each update for the first
//!   [`HEAVY_SECONDS`] of the ragdoll, while the update is under 0.1 s):
//!   [`heavy_gravity`] more of gravity on every bone.

use m2_math::glam::Vec3;

/// The push's scale (0x820EBF4C), and the throw's most per kilogram.
pub const PUSH_SCALE: f32 = 150.0;
pub const MASS_CAP: f32 = 150.0;
/// The launch: the push's strength times this, at most [`LAUNCH_MAX`]
/// m/s (0x82DE4B0C, 0x82DE4B14).
pub const LAUNCH_SCALE: f32 = 1.5;
pub const LAUNCH_MAX: f32 = 120.0;
/// The least share of the launch along her up (50 x 0.01, 0x82DE4B10).
pub const UP_SHARE: f32 = 0.5;
/// Updates the controller keeps a launch for its ragdoll (+0x1D8 = 4).
pub const LAUNCH_FRAMES: u32 = 4;
/// How long a blast pushes the people whose ragdolls run, seconds (its
/// +0x238, 0x8215C110).
pub const BLAST_WINDOW: f32 = 1.0;
/// Each bone's share of the launch (0x820BB468).
pub const BONE_SHARE: f32 = 0.1;
/// The struck bone's factor (0x82DE4B18).
pub const STRUCK: f32 = 3.0;
/// How far up the push's point may move (0x820F8EF4).
pub const JITTER_UP: f32 = 1.75;
/// The ragdoll's first seconds fall harder (0x82DE4B24), from
/// [`HEAVY_START`] to [`HEAVY_END`] gravity more (0x82DE4B1C, 0x82DE4B20).
pub const HEAVY_SECONDS: f32 = 0.8;
pub const HEAVY_START: f32 = 0.2;
pub const HEAVY_END: f32 = 3.0;

/// A blast's push on a person.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Impulse {
    pub magnitude: f32,
    /// Unit.
    pub direction: Vec3,
    pub point: Vec3,
}

/// The push a blast at `centre` of `radius`, `max_force` and least
/// falloff `least` gives a person at `position` whose `ExplosionFudge`
/// is `fudge`; `random` three numbers in 0..1 moving the point.
pub fn blast_impulse(
    centre: Vec3,
    position: Vec3,
    radius: f32,
    max_force: f32,
    least: f32,
    fudge: f32,
    random: [f32; 3],
) -> Impulse {
    let off = position - centre;
    let falloff = (1.0 - off.length() / radius.max(1e-4)).max(least);
    let across = off.with_y(0.0);
    let across = if across.length() > 0.0 {
        across.normalize()
    } else {
        Vec3::ZERO
    };
    Impulse {
        magnitude: max_force * fudge * falloff * PUSH_SCALE,
        direction: (across + Vec3::Y).normalize(),
        point: position + Vec3::new(random[0] - 0.5, random[1] * JITTER_UP, random[2] - 0.5),
    }
}

/// The launch velocity of push `impulse` on a person whose up is `up`:
/// the direction times `min(1.5 x magnitude, 120)`, turned towards `up`
/// where less than [`UP_SHARE`] of it points that way (by adding `up`
/// times what it lacks, then keeping its speed).
pub fn launch_velocity(impulse: &Impulse, up: Vec3) -> Vec3 {
    let speed = (impulse.magnitude * LAUNCH_SCALE).min(LAUNCH_MAX);
    let v = impulse.direction * speed;
    let along = up.dot(v.normalize_or_zero());
    if along < UP_SHARE {
        (v.normalize_or_zero() + up * (UP_SHARE - along)).normalize_or_zero() * v.length()
    } else {
        v
    }
}

/// A bone's factor in the throw: [`STRUCK`] for the struck bone; for the
/// others `w^2`, `w = (2 - d) x 0.5` held to 0.25..1, `d` metres from the
/// bone to the struck point.
pub fn bone_factor(struck: bool, distance: f32) -> f32 {
    if struck {
        STRUCK
    } else {
        let w = ((2.0 - distance) * 0.5).clamp(0.25, 1.0);
        w * w
    }
}

/// How much more of gravity the ragdoll falls with `elapsed` seconds in:
/// from [`HEAVY_START`] easing to [`HEAVY_END`] by the square of its share
/// of [`HEAVY_SECONDS`]; none from then on.
pub fn heavy_gravity(elapsed: f32) -> f32 {
    if elapsed >= HEAVY_SECONDS {
        return 0.0;
    }
    let s = (elapsed / HEAVY_SECONDS).clamp(0.0, 1.0);
    let s2 = s * s;
    (1.0 - s2) * HEAVY_START + HEAVY_END * s2
}

/// A blast's launch waiting for the person's ragdoll to run: given it
/// within the blast's [`BLAST_WINDOW`] (0x8215C560), as the controller's
/// +0x1BC velocity and +0x1C8 point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Launch {
    pub velocity: Vec3,
    pub point: Vec3,
    /// Seconds of the blast's window left.
    pub left: f32,
}

impl Launch {
    /// Kept from push `impulse` on a person whose up is `up`.
    pub fn new(impulse: &Impulse, up: Vec3) -> Self {
        Self {
            velocity: launch_velocity(impulse, up),
            point: impulse.point,
            left: BLAST_WINDOW,
        }
    }

    /// An update of `dt` seconds without a ragdoll: false once the
    /// blast's window has passed (dropped).
    pub fn wait(&mut self, dt: f32) -> bool {
        self.left -= dt;
        self.left > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blast_pushes_away_and_up_at_45_degrees_by_its_falloff() {
        let i = blast_impulse(
            Vec3::ZERO,
            Vec3::new(3.0, 0.0, 4.0),
            10.0,
            100.0,
            0.3,
            1.0,
            [0.5, 0.0, 0.5],
        );
        assert!((i.magnitude - 100.0 * 0.5 * 150.0).abs() < 1e-2);
        assert!((i.direction - Vec3::new(0.6, 1.0, 0.8).normalize()).length() < 1e-5);
        assert_eq!(i.point, Vec3::new(3.0, 0.0, 4.0));
        // Past its reach the least falloff holds; the point moves up to
        // 1.75 m up and half a metre across.
        let far = blast_impulse(
            Vec3::ZERO,
            Vec3::new(30.0, 0.0, 0.0),
            10.0,
            100.0,
            0.3,
            2.0,
            [1.0, 1.0, 0.0],
        );
        assert!((far.magnitude - 100.0 * 2.0 * 0.3 * 150.0).abs() < 1e-2);
        assert_eq!(far.point, Vec3::new(30.5, 1.75, -0.5));
    }

    #[test]
    fn the_launch_is_capped_and_kept_up() {
        let at = |d: Vec3, m: f32| Impulse {
            magnitude: m,
            direction: d.normalize(),
            point: Vec3::ZERO,
        };
        // 45 degrees, strong: 120 m/s along it.
        let v = launch_velocity(&at(Vec3::new(1.0, 1.0, 0.0), 3.5e4), Vec3::Y);
        assert!((v.length() - 120.0).abs() < 1e-3 && (v.x - v.y).abs() < 1e-3);
        // Weak: 1.5 times it.
        let v = launch_velocity(&at(Vec3::new(1.0, 1.0, 0.0), 10.0), Vec3::Y);
        assert!((v.length() - 15.0).abs() < 1e-4);
        // Level: turned up to half its speed's share and kept as fast.
        let v = launch_velocity(&at(Vec3::X, 10.0), Vec3::Y);
        assert!((v.length() - 15.0).abs() < 1e-4);
        assert!(
            (v.normalize().y - (0.5f32).atan().sin()).abs() < 1e-4,
            "{v}"
        );
    }

    #[test]
    fn bones_take_the_throw_by_their_distance() {
        assert_eq!(bone_factor(true, 0.0), 3.0);
        assert_eq!(bone_factor(false, 0.0), 1.0);
        assert_eq!(bone_factor(false, 1.0), 0.25);
        assert_eq!(bone_factor(false, 5.0), 0.0625);
    }

    #[test]
    fn the_ragdoll_falls_harder_for_its_first_moments() {
        assert!((heavy_gravity(0.0) - 0.2).abs() < 1e-6);
        assert!((heavy_gravity(0.4) - (0.75 * 0.2 + 3.0 * 0.25)).abs() < 1e-6);
        assert_eq!(heavy_gravity(0.8), 0.0);
    }

    #[test]
    fn a_launch_waits_out_the_blasts_second() {
        let mut l = Launch::new(
            &Impulse {
                magnitude: 10.0,
                direction: Vec3::Y,
                point: Vec3::ZERO,
            },
            Vec3::Y,
        );
        // The blast's second: three quarter-second updates wait, the
        // fourth drops it.
        assert!(l.wait(0.25) && l.wait(0.25) && l.wait(0.25));
        assert!(!l.wait(0.25));
    }
}

/// The most level speed a seat exit hands a ragdoll, m/s (0x820B7B44).
pub const EXIT_LAUNCH_MOST: f32 = 15.0;
/// The share of that speed it is also thrown up with (0x820C7734).
pub const EXIT_LAUNCH_UP: f32 = 0.4;

/// The launch a seat exit carrying velocity `v` (the vehicle's, as the
/// crash ejection 0x8218A138 posts it) hands the rider's ragdoll.
///
/// From the 360 code: the ragdoll controller's message handler 0x82428658
/// takes the exit message (byte +0x21 = 1, kind 4) and keeps, at its
/// +0x1B0, `v`'s level direction times `min(|v_xz|, 15)` with `0.4` of that
/// speed upward (mode 0); 0x82428568 hands it to the ragdoll's start
/// 0x82427828 (its +0x2B8, flag 0x80 of +0x324) as the action goes FULL.
/// The toss itself (0x822A1B70) only places her. **Inferred**: whatever
/// reads +0x2B8 gives every body that velocity.
pub fn exit_launch(v: Vec3) -> Vec3 {
    let level = Vec3::new(v.x, 0.0, v.z);
    let speed = level.length().min(EXIT_LAUNCH_MOST);
    level.normalize_or_zero() * speed + Vec3::Y * speed * EXIT_LAUNCH_UP
}

#[cfg(test)]
mod exit_launch_tests {
    use super::*;

    /// A bike crashing at 19.4 m/s throws its rider 15 m/s out and 6 up;
    /// at 10 m/s, 10 out and 4 up. Its fall plays no part.
    #[test]
    fn an_exit_throws_her_level_speed_capped_at_15_and_four_tenths_up() {
        let fast = exit_launch(Vec3::new(-19.4, -3.0, 0.2));
        assert!((fast.with_y(0.0).length() - 15.0).abs() < 1e-4);
        assert!((fast.y - 6.0).abs() < 1e-4);
        assert!(fast.x < -14.9);
        let slow = exit_launch(Vec3::new(0.0, 0.0, 10.0));
        assert!((slow - Vec3::new(0.0, 4.0, 10.0)).length() < 1e-4);
        assert_eq!(exit_launch(Vec3::Y * 5.0), Vec3::ZERO);
    }
}
