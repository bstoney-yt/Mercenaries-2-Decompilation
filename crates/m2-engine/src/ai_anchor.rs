//! An AI's anchor: a point and a radius it is held within.
//!
//! From the 360 code. The scripted AI record (a mind reaches it at
//! mind+0x274) keeps the anchor's point at +0x7A8, +0x7AC, +0x7B0, an
//! anchor object at +0x7B8 and its radius at +0x7BC; `Ai.Anchor`
//! (82310EB8) writes them, and a spawner of `SpawnerType` 3 gives the one
//! it sends its own place with a radius of [`SPAWNER_RADIUS`] (82243320).
//!
//! - It holds only with a radius of 0 or more ([`Anchor::holds`]; the
//!   record's flag byte at +0x7C4, bit 0x40, switches it off too, which
//!   nothing here sets).
//! - A point is outside it when it is further than the radius from the
//!   anchor's point over the ground, x and z only (822C0740; 822C0648 is
//!   the same test of where the AI stands).
//! - A point the AI wants to go to that is outside is brought onto the
//!   anchor's circle, on the line from the anchor's point to it
//!   (822C0800, called from the movement routines 822ACB28 and 822B6508).
//!   With a radius of 0 that is the anchor's point itself: hold the spot
//!   (OilCon001 anchors its executive with `AnchorRadius = 0`).
//!
//! Inferred: the height of a clamped point is the anchor's (the
//! decompiler does not show the middle component of the offset clearly;
//! the test is over the ground). [`Anchor::hold`], which applies the
//! clamp to a velocity for a mover that steers by one, is the port's.
//!
//! Not ported: the mind's state that walks an AI back once it is beyond
//! 0.6 of the radius (822B49C8), and the goal a `SpawnerType` 3 spawner
//! gives the one it sends (to its place, at haste 0.5).

use m2_math::glam::{Vec2, Vec3};

/// The radius a `SpawnerType` 3 spawner anchors the one it sends with,
/// metres (from the 360 code: 50.0 at 820B7FB8, written in 82243320).
pub const SPAWNER_RADIUS: f32 = 50.0;

/// The `SpawnerType` whose spawners anchor whom they send.
pub const ANCHORING_SPAWNER: u32 = 3;

/// A point and the radius an AI is held within, metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub at: Vec3,
    pub radius: f32,
}

impl Anchor {
    /// Whether it holds the AI at all.
    pub fn holds(&self) -> bool {
        self.radius >= 0.0
    }

    /// Whether `point` is outside it (822C0740).
    pub fn outside(&self, point: Vec3) -> bool {
        self.holds()
            && Vec2::new(point.x - self.at.x, point.z - self.at.z).length_squared()
                > self.radius * self.radius
    }

    /// Where the AI may go when it wants to go to `wanted` (822C0800).
    pub fn clamp(&self, wanted: Vec3) -> Vec3 {
        if !self.outside(wanted) {
            return wanted;
        }
        let out = Vec2::new(wanted.x - self.at.x, wanted.z - self.at.z).normalize_or_zero();
        Vec3::new(
            self.at.x + out.x * self.radius,
            self.at.y,
            self.at.z + out.y * self.radius,
        )
    }

    /// The velocity over the ground an AI standing at `at` may take for a
    /// step of `dt` seconds when it wants `velocity`: where the step
    /// would take it is clamped, and it heads for that at the speed it
    /// wanted, or stands if it is there.
    pub fn hold(&self, at: Vec3, velocity: Vec3, dt: f32) -> Vec3 {
        let speed = Vec2::new(velocity.x, velocity.z).length();
        if !self.holds() || speed <= 0.0 || dt <= 0.0 {
            return velocity;
        }
        let wanted = at + velocity * dt;
        if !self.outside(wanted) {
            return velocity;
        }
        let to = self.clamp(wanted) - at;
        let flat = Vec2::new(to.x, to.z);
        let reach = flat.length();
        if reach <= speed * dt {
            // There within the step: no further.
            return Vec3::new(to.x / dt, velocity.y, to.z / dt);
        }
        let heading = flat / reach;
        Vec3::new(heading.x * speed, velocity.y, heading.y * speed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: Anchor = Anchor {
        at: Vec3::new(100.0, 5.0, 100.0),
        radius: 50.0,
    };

    #[test]
    fn outside_is_measured_over_the_ground() {
        assert!(!HOME.outside(Vec3::new(140.0, 5.0, 100.0)));
        assert!(HOME.outside(Vec3::new(151.0, 5.0, 100.0)));
        // Height does not count.
        assert!(!HOME.outside(Vec3::new(100.0, 900.0, 100.0)));
        // On the circle is not outside.
        assert!(!HOME.outside(Vec3::new(150.0, 5.0, 100.0)));
        // A radius below 0 holds nothing.
        let none = Anchor {
            radius: -1.0,
            ..HOME
        };
        assert!(!none.holds() && !none.outside(Vec3::new(9000.0, 0.0, 0.0)));
    }

    #[test]
    fn a_point_outside_is_brought_onto_the_circle() {
        let inside = Vec3::new(110.0, 7.0, 90.0);
        assert_eq!(HOME.clamp(inside), inside);
        // 200 m east: 50 m east of the anchor, at the anchor's height.
        assert_eq!(
            HOME.clamp(Vec3::new(300.0, 40.0, 100.0)),
            Vec3::new(150.0, 5.0, 100.0)
        );
        // Radius 0: the anchor's own point, wherever it wanted to go.
        let spot = Anchor {
            radius: 0.0,
            ..HOME
        };
        assert_eq!(spot.clamp(Vec3::new(300.0, 0.0, -40.0)), spot.at);
    }

    #[test]
    fn a_step_that_would_leave_is_turned_along_or_stopped() {
        let dt = 0.1;
        // Well inside, going anywhere: as wanted.
        let v = Vec3::new(4.0, 0.0, 0.0);
        assert_eq!(HOME.hold(Vec3::new(100.0, 5.0, 100.0), v, dt), v);
        // At the edge, heading straight out: it stands.
        let edge = Vec3::new(150.0, 5.0, 100.0);
        let held = HOME.hold(edge, v, dt);
        assert!(held.x.abs() < 1e-3 && held.z.abs() < 1e-3);
        // At the edge, heading along it and a little out: it goes on,
        // no faster than it wanted, and ends no further out.
        let along = Vec3::new(1.0, 0.0, 4.0);
        let held = HOME.hold(edge, along, dt);
        assert!(held.length() <= along.length() + 1e-3);
        assert!(!HOME.outside(edge + held * dt - Vec3::new(1e-3, 0.0, 0.0)));
        // Far outside (put there), wanting to go further: back towards
        // the circle at the speed it wanted.
        let far = Vec3::new(300.0, 5.0, 100.0);
        let back = HOME.hold(far, v, dt);
        assert!((back.x + 4.0).abs() < 1e-3 && back.z.abs() < 1e-3);
        // Holding a spot: it stands on it.
        let spot = Anchor {
            radius: 0.0,
            ..HOME
        };
        assert_eq!(spot.hold(spot.at, v, dt).with_y(0.0), Vec3::ZERO);
    }
}
