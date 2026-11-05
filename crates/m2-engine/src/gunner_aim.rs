//! Where an AI gunner aims: the 360's lead and drop for a vehicle's
//! gunner (`TurretAttack`'s fire control, 0x822C95D8).
//!
//! From the 360 code (0x822C95D8):
//! - **Its target's motion:** every 0.25 s it keeps where the target is,
//!   its last four places; the target's velocity is the newest less the
//!   oldest, over the 0.75 s between (x 1.3333, the float at 0x820AD158).
//! - **The lead:** seen within 0.1 s, it aims where the target will be
//!   when a round gets there: its place plus that velocity times the
//!   round's time there (0x822D5620: the distance over the round's speed
//!   less the target's own speed toward it, 1000 s when that is nothing).
//!   Unseen longer, at where it was last seen.
//! - **The drop:** a round its projectile's gravity pulls (the projectile
//!   record's +0x18 times the world's gravity) is aimed up by the height
//!   it falls on the way, from the arc 0x828542C0 solves through the
//!   point at the round's speed (the flatter of its two arcs).
//!
//! Inferred: the gravity factor is the round's `GravityScale`, the world's
//! gravity the alpha's own ([`crate::physics::GRAVITY`], as its rounds fall
//! by it); the round's speed its `Velocity`. Not ported: how often it
//! takes a new aim point (+0x3C counting down from its +0x40), and what
//! it does about what its line of sight meets (0x822C9530); its rays
//! (mask 0x223F6FDA) are here all that is solid but its own bodies and hers.

use m2_math::glam::Vec3;

/// A gunner's watch on its target's motion (its +0x8C..+0xC4).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Watch {
    places: [Vec3; 4],
    clock: f32,
    started: bool,
    /// The target's velocity as it has seen it.
    pub velocity: Vec3,
}

impl Watch {
    /// Over `dt`, the target at `at`: every 0.25 s its place is kept and
    /// its velocity taken from the oldest of the last four.
    pub fn see(&mut self, at: Vec3, dt: f32) {
        if !self.started {
            self.places = [at; 4];
            self.started = true;
        }
        self.clock -= dt;
        if self.clock < 0.0 {
            self.clock = 0.25;
            self.places.rotate_left(1);
            self.places[3] = at;
            self.velocity = (self.places[3] - self.places[0]) * 1.333_333_4;
        }
    }
}

/// 0x822D5620: how long a round at `speed` takes to a target `to` away
/// moving at `velocity`: the distance over the speed less the target's
/// speed toward it; 1000 s when that is nothing.
pub fn time_there(to: Vec3, velocity: Vec3, speed: f32) -> f32 {
    let dist = to.length();
    let closing = speed - velocity.dot(to) / (dist + 1e-4);
    if closing > 0.0 {
        dist / closing
    } else {
        1000.0
    }
}

/// 0x828542C0's arc: the height to aim above a point `to` away (from the
/// muzzle) for a round at `speed` pulled down by `gravity`, the flatter of
/// the two arcs; `None` when out of its reach.
pub fn drop(to: Vec3, speed: f32, gravity: f32) -> Option<f32> {
    if gravity.abs() < 1e-4 {
        return Some(0.0);
    }
    let across = (to.x * to.x + to.z * to.z).sqrt();
    let up = to.y;
    // Time t: (g^2 / 4) t^4 + (up g - v^2) t^2 + (across^2 + up^2) = 0.
    let a = gravity * gravity * 0.25;
    let b = up * gravity - speed * speed;
    let c = across * across + up * up;
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    let root = disc.sqrt();
    let q = if b < 0.0 { root } else { -root };
    let t1 = (q - b) / (2.0 * a);
    let t2 = c / (t1 * a);
    let mut t2s = t2;
    if t1 < t2 && t1 >= 0.0 {
        t2s = t1;
    }
    if t2s <= 0.0 {
        return None;
    }
    let t = t2s.sqrt();
    // The launch's rise over its run, aimed at the point's own distance.
    let rise = (up + 0.5 * gravity * t2s) / t;
    let run = across / t;
    Some(across * rise / run.max(1e-4) - up)
}

/// Where a gunner at `muzzle` aims for a target at `at`, `velocity` its
/// own (as its record has it), `watched` its velocity as the gunner has
/// seen it ([`Watch`]),
/// `seen` whether it was seen within 0.1 s, `last_seen` where it was last;
/// rounds at `speed` m/s pulled by `gravity` x the world's.
#[allow(clippy::too_many_arguments)]
pub fn aim(
    muzzle: Vec3,
    at: Vec3,
    velocity: Vec3,
    watched: Vec3,
    seen: bool,
    last_seen: Vec3,
    speed: f32,
    gravity: f32,
) -> Vec3 {
    let mut p = if seen && speed > 0.0 {
        at + watched * time_there(at - muzzle, velocity, speed)
    } else if seen {
        at
    } else {
        last_seen
    };
    if speed > 0.0
        && gravity != 0.0
        && let Some(h) = drop(p - muzzle, speed, gravity * crate::physics::GRAVITY)
    {
        p.y += h;
    }
    p
}

/// 0x822C9250's three lines of sight, tried once a second: from the middle
/// of its eye (`eye`) to the middle of its target, and crosswise from
/// `own` either side of the eye (half the smaller side of its box) to
/// `far` the other side of the target (0.25 m, or half the smaller side of
/// its box when it has one). It sees its target when any is clear
/// (0x822C95D8's +0x70 bits).
pub fn sight_lines(eye: Vec3, at: Vec3, own: f32, far: f32) -> [(Vec3, Vec3); 3] {
    let d = (at - eye).with_y(0.0).normalize_or(Vec3::Z);
    let side = Vec3::new(d.z, 0.0, -d.x);
    [
        (eye + side * own, at - side * far),
        (eye, at),
        (eye - side * own, at + side * far),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_reads_the_targets_motion_over_three_quarters_of_a_second() {
        let mut w = Watch::default();
        let mut at = Vec3::ZERO;
        for _ in 0..120 {
            at += Vec3::X * 10.0 / 60.0;
            w.see(at, 1.0 / 60.0);
        }
        // Kept on the frame after each 0.25 s (16 frames at 60 Hz): 10.67.
        assert!((w.velocity.x - 10.67).abs() < 0.05, "{:?}", w.velocity);
    }

    #[test]
    fn it_leads_a_crossing_target() {
        // 100 m off, crossing at 10 m/s, rounds at 500 m/s: 0.2 s ahead.
        let w = Watch {
            velocity: Vec3::X * 10.0,
            ..Watch::default()
        };
        let p = aim(
            Vec3::ZERO,
            Vec3::Z * 100.0,
            Vec3::X * 10.0,
            w.velocity,
            true,
            Vec3::ZERO,
            500.0,
            0.0,
        );
        assert!(
            (p.x - 2.0).abs() < 0.01 && (p.z - 100.0).abs() < 1e-4,
            "{p}"
        );
        // Unseen: where it was last.
        let p = aim(
            Vec3::ZERO,
            Vec3::Z * 100.0,
            Vec3::ZERO,
            w.velocity,
            false,
            Vec3::Y,
            500.0,
            0.0,
        );
        assert_eq!(p, Vec3::Y);
    }

    #[test]
    fn a_falling_round_is_aimed_up_by_its_drop() {
        // 200 m level at 100 m/s: about 2 s there, 0.5 g t^2 ~ 19.6 m.
        let h = drop(Vec3::Z * 200.0, 100.0, 9.81).unwrap();
        assert!((h - 19.6).abs() < 1.0, "{h}");
        assert_eq!(drop(Vec3::Z * 200.0, 100.0, 0.0), Some(0.0));
        assert!(drop(Vec3::Z * 5000.0, 100.0, 9.81).is_none());
    }

    #[test]
    fn its_lines_of_sight_cross() {
        let [a, b, c] = sight_lines(Vec3::ZERO, Vec3::Z * 50.0, 1.0, 0.25);
        assert_eq!(b, (Vec3::ZERO, Vec3::Z * 50.0));
        assert_eq!(a.0.x, 1.0);
        assert_eq!(a.1.x, -0.25);
        assert_eq!(c.0.x, -1.0);
        assert_eq!(c.1.x, 0.25);
    }
}
