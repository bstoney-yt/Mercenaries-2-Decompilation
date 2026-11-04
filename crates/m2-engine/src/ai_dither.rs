//! The 360's check for a fighter dithering on the spot, and the spots it
//! marks bad. From the 360 code, as the research thread traced it (360
//! notes s12) and as read here.
//!
//! Only the `HumanCombat` path step (0x822B3130) checks: after its path
//! move it calls 0x822BF4D8, which compares the way the person heads with
//! the way they headed the step before. A swing of more than about 47
//! degrees (the squared distance between the two unit headings over 0.64,
//! 0x820F8D0C) counts one; a second without such a swing clears the count.
//! Past 8 the step marks where the person stands as a bad spot for 60
//! seconds (0x822C8370, 0x82B7D884) and fails, so the planner picks again.

use m2_math::glam::{Vec2, Vec3};

/// A swing counts when the squared distance between the unit headings is
/// over this (0x820F8D0C).
const SWING: f32 = 0.64;
/// A count lapses after this long without a swing, seconds.
const LAPSE: f32 = 1.0;
/// Past this many swings the combat step gives up.
pub const GIVE_UP: u32 = 8;
/// How long a spot stays bad, seconds (0x82B7D884), how many are kept, and
/// how near (metres) a new one must be to an old one to refresh it.
pub const BAD_FOR: f32 = 60.0;
const SLOTS: usize = 4;
const SAME_SPOT: f32 = 2.0;

/// One person's dithering count and bad spots (the AI object's +0x9b4..
/// and its position chooser's slots at +0x130).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dither {
    last: Option<Vec2>,
    pub count: u32,
    since_swing: f32,
    spots: [Option<(Vec3, f32)>; SLOTS],
}

impl Dither {
    /// Steps on `dt` seconds heading `way` (any length; nothing when still).
    /// True once the count is past [`GIVE_UP`].
    pub fn step(&mut self, way: Vec2, dt: f32) -> bool {
        for s in self.spots.iter_mut() {
            if let Some((_, t)) = s {
                *t -= dt;
                if *t <= 0.0 {
                    *s = None;
                }
            }
        }
        self.since_swing += dt;
        if way.length_squared() > 1e-8 {
            let d = way.normalize();
            if let Some(prev) = self.last
                && (d - prev).length_squared() > SWING
            {
                self.count += 1;
                self.since_swing = 0.0;
            }
            self.last = Some(d);
        }
        if self.since_swing > LAPSE {
            self.count = 0;
        }
        self.count > GIVE_UP
    }

    /// Marks `at` bad for [`BAD_FOR`] seconds: refreshing a spot within
    /// 2 m, else taking a free slot (else the one nearest its end), and
    /// starts the count again.
    pub fn mark(&mut self, at: Vec3) {
        self.count = 0;
        self.last = None;
        if let Some(s) = self
            .spots
            .iter_mut()
            .flatten()
            .find(|(p, _)| p.distance_squared(at) < SAME_SPOT * SAME_SPOT)
        {
            *s = (at, BAD_FOR);
            return;
        }
        let slot = self
            .spots
            .iter()
            .position(Option::is_none)
            .unwrap_or_else(|| {
                (0..SLOTS)
                    .min_by(|&a, &b| {
                        let t = |i: usize| self.spots[i].map_or(0.0, |(_, t)| t);
                        t(a).total_cmp(&t(b))
                    })
                    .unwrap_or(0)
            });
        self.spots[slot] = Some((at, BAD_FOR));
    }

    /// Whether `at` is within 2 m of a bad spot.
    pub fn bad(&self, at: Vec3) -> bool {
        self.spots
            .iter()
            .flatten()
            .any(|(p, _)| p.distance_squared(at) < SAME_SPOT * SAME_SPOT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swinging_back_and_forth_gives_up_and_marks_the_spot() {
        let mut d = Dither::default();
        let dt = 1.0 / 60.0;
        // Steady heading: never.
        for _ in 0..120 {
            assert!(!d.step(Vec2::Y, dt));
        }
        // Flipping every step: past 8 swings.
        let mut gave_up = 0;
        for i in 0..20 {
            let w = if i % 2 == 0 { Vec2::X } else { -Vec2::X };
            if d.step(w, dt) {
                gave_up = i;
                break;
            }
        }
        assert_eq!(gave_up, 8);
        d.mark(Vec3::new(1.0, 0.0, 1.0));
        assert!(d.bad(Vec3::new(2.0, 0.0, 1.5)));
        assert!(!d.bad(Vec3::new(5.0, 0.0, 1.0)));
        // A second without a swing clears a count.
        let mut d = Dither::default();
        for i in 0..5 {
            d.step(if i % 2 == 0 { Vec2::X } else { -Vec2::X }, dt);
        }
        for _ in 0..70 {
            d.step(Vec2::Y, dt);
        }
        assert_eq!(d.count, 0);
        // Bad for a minute.
        let mut d = Dither::default();
        d.mark(Vec3::ZERO);
        for _ in 0..(60 * 61) {
            d.step(Vec2::Y, dt);
        }
        assert!(!d.bad(Vec3::ZERO));
    }
}
