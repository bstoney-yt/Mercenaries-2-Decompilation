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
//! - **What it reports:** each think it leaves its verdict in its +0xCC
//!   byte (the rider's mind +0x128, which 0x822ECE70 hands the driver):
//!   [`Report`], from which `CarAttack` moves and turns for its crew.
//!
//! Inferred: the gravity factor is the round's `GravityScale`, the world's
//! gravity the alpha's own ([`crate::physics::GRAVITY`], as its rounds fall
//! by it); the round's speed its `Velocity`. Not ported: how often it
//! takes a new aim point (+0x3C counting down from its +0x40), and what
//! it does about what its line of sight meets (0x822C9530); its rays
//! (mask 0x223F6FDA) are here all that is solid but its own bodies and hers.

use m2_math::glam::Vec3;

/// What a gunner's fire control last made of its target, its +0xCC byte
/// (0x822C95D8 writes it, 0x822C8EA8 clears it; 0x822ECE70 reads it as the
/// rider's mind +0x128, the fire control being the mind's +0x5C).
///
/// From the 360 code. The byte is left as it is unless a think has
/// something to say, so a gunner goes on reporting what it last did:
/// [`next`](Report::next). Not told apart here: 8 (its burst running) and
/// 9 (two kinds of mind, or asked for a special attack), both of which
/// `CarAttack` counts as it does 3, and which only follow a 3. 7 is read
/// by `CarAttack` and written by nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Report {
    /// 0: nothing yet.
    #[default]
    Nothing,
    /// 2: not seen for too long.
    Lost,
    /// 3: it has had a clear line and may fire.
    Ready,
    /// 4: past its weapon's reach.
    TooFar,
    /// 5: inside its weapon's least range.
    TooNear,
    /// 6: its turret or gun can't be brought to bear.
    NoReach,
}

/// How long a target goes unseen before a gunner reports it lost, seconds
/// (0x822C95D8: 5; 30 after it took its last aim from a hit, not ported).
pub const LOST_AFTER: f32 = 5.0;

/// A gunner's weapon's reach with no `WeaponHint`, metres (the float 80 at
/// 0x820BAF98, used by 0x822C95D8 and 0x822EDBC0 alike).
pub const NO_HINT_RANGE: f32 = 80.0;

/// What one think of the fire control looks at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FireView {
    /// Seconds since its target was seen (its record's +0x8C).
    pub unseen: f32,
    /// Its flat distance to its aim point (0x822C95D8 leaves out height).
    pub flat: f32,
    /// Its weapon's `MinFireRange` and `FireRange` (0 and
    /// [`NO_HINT_RANGE`] with no hint).
    pub near: f32,
    pub far: f32,
    /// One of its sight lines is clear (+0x70 bits 30, 31).
    pub sees: bool,
    /// It can be brought to bear (0x822EF8C8, or its gun's cone).
    pub reaches: bool,
}

impl Report {
    /// One think of 0x822C95D8. In its order: unseen too long it reports
    /// lost; a clear line makes it ready; then inside its least range, past
    /// its reach and not able to bear each report and end the think short
    /// of firing (0x822CAA64..0x822CABFC). With nothing to say the last
    /// report stands.
    pub fn next(self, v: &FireView) -> Self {
        if v.unseen >= LOST_AFTER {
            return Self::Lost;
        }
        let was = if v.sees { Self::Ready } else { self };
        if v.flat <= v.near {
            Self::TooNear
        } else if v.far <= v.flat {
            Self::TooFar
        } else if !v.reaches {
            Self::NoReach
        } else {
            was
        }
    }

    /// Whether this think may fire: it did not end short of firing, and
    /// it has been ready.
    pub fn fires(self) -> bool {
        self == Self::Ready
    }
}

/// The least range a gunner's weapon slot keeps (0x822EDBC0, its +0x20,
/// which `CarAttack` averages): its `MinFireRange` (0 with no hint), but
/// at least 1 m past its round's blast (1 m with none).
pub fn slot_near(min_range: f32, blast_radius: Option<f32>) -> f32 {
    min_range.max(blast_radius.unwrap_or(0.0) + 1.0)
}

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

    fn view(flat: f32) -> FireView {
        FireView {
            unseen: 0.0,
            flat,
            near: 5.0,
            far: 100.0,
            sees: true,
            reaches: true,
        }
    }

    // What 0x822C95D8 leaves in its +0xCC byte, in its own order.
    #[test]
    fn a_gunner_reports_as_the_360s_fire_control() {
        let r = Report::default();
        assert_eq!(r.next(&view(50.0)), Report::Ready);
        assert_eq!(r.next(&view(100.0)), Report::TooFar);
        assert_eq!(r.next(&view(5.0)), Report::TooNear);
        let v = FireView {
            reaches: false,
            ..view(50.0)
        };
        assert_eq!(r.next(&v), Report::NoReach);
        // Lost comes first, however near.
        let v = FireView {
            unseen: 5.0,
            ..view(1.0)
        };
        assert_eq!(r.next(&v), Report::Lost);
    }

    // With no clear line and nothing else to say, the last report stands.
    #[test]
    fn a_report_stands_until_a_think_has_something_to_say() {
        let blind = FireView {
            sees: false,
            ..view(50.0)
        };
        assert_eq!(Report::Nothing.next(&blind), Report::Nothing);
        assert_eq!(Report::TooFar.next(&blind), Report::TooFar);
        assert_eq!(Report::Ready.next(&blind), Report::Ready);
        assert!(Report::Ready.next(&blind).fires());
        assert!(!Report::Nothing.next(&blind).fires());
        // A clear line back in reach makes it ready again.
        assert_eq!(Report::TooFar.next(&view(50.0)), Report::Ready);
    }

    // 0x822EDBC0: at least 1 m past its blast.
    #[test]
    fn a_slot_keeps_its_gunner_out_of_its_own_blast() {
        assert_eq!(slot_near(0.0, None), 1.0);
        assert_eq!(slot_near(20.0, None), 20.0);
        assert_eq!(slot_near(4.0, Some(8.0)), 9.0);
    }

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
