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
//!
//! The counter runs only when the step to a destination made no progress
//! (0x822B3130 -> 0x822CB340 -> 0x822BF4D8). With no path holder the
//! step's caller (0x822B34A8) then stops the person (no velocity) and the
//! behaviour picks again; nothing else (no cooldown, no new order).
//!
//! The bad spots are a 4-slot list in the goal block (points at
//! +0xE4/+0xF4/+0x104/+0x114, timers at +0xF0/+0x100/+0x110/+0x120): a
//! new spot within 2 m on x/z (squared under 4) of an active one (its
//! timer above 0) refreshes that one's timer; else the first free slot
//! takes it; with all four active and none near, nothing is recorded.
//! The timers run down by each update's step (0x822C8530).
//!
//! Nothing in the 360 reads the list: it is written (60 s here, 10 s by
//! 0x822C7CF0 for a destination that failed its check, 20 s by
//! 0x822C6138 for a goal re-seeded at his spot), merged and counted down,
//! but no chooser passes its spots over. [`Dither::bad`] is for checking
//! only.

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

    /// Marks `at` bad for [`BAD_FOR`] seconds as 0x822C8370 does:
    /// refreshing the timer of an active spot within 2 m on x/z (the spot
    /// itself kept), else taking the first free slot; with every slot
    /// active and none near, nothing. Starts the count again.
    pub fn mark(&mut self, at: Vec3) {
        self.count = 0;
        self.last = None;
        if let Some(s) = self.spots.iter_mut().flatten().find(|(p, _)| near(*p, at)) {
            s.1 = BAD_FOR;
            return;
        }
        if let Some(slot) = self.spots.iter_mut().find(|s| s.is_none()) {
            *slot = Some((at, BAD_FOR));
        }
    }

    /// Whether `at` is within 2 m on x/z of a bad spot (for checking: the
    /// game reads the list nowhere).
    pub fn bad(&self, at: Vec3) -> bool {
        self.spots.iter().flatten().any(|(p, _)| near(*p, at))
    }
}

/// Within 2 m of each other on x/z (0x822C8370's squared distance under
/// 4.0).
fn near(a: Vec3, b: Vec3) -> bool {
    Vec2::new(a.x - b.x, a.z - b.z).length_squared() < SAME_SPOT * SAME_SPOT
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list as 0x822C8370 keeps it: a spot near an active one only
    /// refreshes it (on x/z, whatever the height), four at most, and a
    /// fifth with all four active is not recorded.
    #[test]
    fn the_bad_spots_are_four_refreshed_not_replaced() {
        let dt = 1.0;
        let mut d = Dither::default();
        d.mark(Vec3::ZERO);
        for _ in 0..30 {
            d.step(Vec2::ZERO, dt);
        }
        // 1.5 m off and 5 m up: the same spot, its minute starts again.
        d.mark(Vec3::new(1.5, 5.0, 0.0));
        assert!(!d.bad(Vec3::new(3.0, 0.0, 0.0)), "the spot itself kept");
        for _ in 0..45 {
            d.step(Vec2::ZERO, dt);
        }
        assert!(d.bad(Vec3::ZERO), "refreshed at 30 s, still bad at 75 s");
        for x in 1..4 {
            d.mark(Vec3::new(10.0 * x as f32, 0.0, 0.0));
        }
        d.mark(Vec3::new(100.0, 0.0, 0.0));
        assert!(!d.bad(Vec3::new(100.0, 0.0, 0.0)), "a fifth is not kept");
        assert!(d.bad(Vec3::new(30.0, 0.0, 0.0)));
    }

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
