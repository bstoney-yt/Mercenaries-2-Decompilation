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
//!   too): [`launch_velocity`], kept for [`LAUNCH_FRAMES`] updates
//!   ([`Launch`]). The controller's update (0x82427D30) counts it down
//!   while there is no ragdoll; at none left it is dropped, so a person who
//!   does not go limp in time is not thrown.
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
/// Updates the launch waits for the ragdoll (+0x1D8 = 4).
pub const LAUNCH_FRAMES: u32 = 4;
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

/// A launch waiting for the person's ragdoll (the controller's +0x1BC
/// velocity, +0x1C8 point, +0x1D8 updates left).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Launch {
    pub velocity: Vec3,
    pub point: Vec3,
    pub frames: u32,
}

impl Launch {
    /// Kept from push `impulse` on a person whose up is `up`.
    pub fn new(impulse: &Impulse, up: Vec3) -> Self {
        Self {
            velocity: launch_velocity(impulse, up),
            point: impulse.point,
            frames: LAUNCH_FRAMES,
        }
    }

    /// One update without a ragdoll: false once none is left (dropped).
    pub fn wait(&mut self) -> bool {
        self.frames = self.frames.saturating_sub(1);
        self.frames > 0
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
    fn a_launch_waits_four_updates() {
        let mut l = Launch::new(
            &Impulse {
                magnitude: 10.0,
                direction: Vec3::Y,
                point: Vec3::ZERO,
            },
            Vec3::Y,
        );
        assert!(l.wait() && l.wait() && l.wait());
        assert!(!l.wait());
    }
}
