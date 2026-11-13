//! What an AI mind knows of its target: the 360's target record (the
//! mind's +0x8D4; built by 0x822D51E8, kept by 0x822D5468 and
//! 0x822D6030, ticked by 0x822DE240).
//!
//! From the 360 code:
//! - **Sensed** (0x822D5468, 0x822D6030): the record takes where she is,
//!   her velocity (+0x28, copied from the sensing by 0x822D5048; the
//!   lead of 0x822D5620 reads it as her velocity) and sets the time
//!   since (+0x8C) to 0. Where she was sensed is kept at +0x80
//!   (0x822D51E8).
//! - **Each think** (0x822DE240) adds its length to +0x8C.
//! - **Unsensed** (0x822D5468), the record's point (+0x08, which every
//!   driver task reads through 0x8225E998) is moved on from where she
//!   was sensed: by her velocity x the time, or, for a target with a
//!   mind of its own (+0x0 bit 26, set by 0x822F4178 when 0x822F8900
//!   finds one), x `1 - 1 / (t + 1)`, never more than a second's worth.
//! - **Forgotten** (0x822DE240): once the time passes its memory (+0x90,
//!   from +0x94: 30 s, 3 s for a record of kind bit 0, 0x822D51E8) the
//!   record is freed (0x822D5CD8).
//!
//! Inferred: she, the player, has no AI mind, so her point moves on in a
//! straight line. Not ported: the forgetting (the alpha's pursuits keep
//! their target; what a task does with its record gone, return 4, is
//! not traced past `BoatIdle`).

use m2_math::glam::Vec3;

/// Seconds a record is kept unsensed (0x822D51E8's +0x94, 30.0).
pub const MEMORY: f32 = 30.0;

/// One mind's record of its target.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TargetRecord {
    /// Where she was sensed (+0x80) and her velocity then (+0x28).
    sensed_at: Vec3,
    velocity: Vec3,
    /// Seconds since (+0x8C).
    pub unseen: f32,
    /// Its target has a mind of its own (+0x0 bit 26).
    pub minded: bool,
}

impl TargetRecord {
    /// A sensing of her (0x822D5468, 0x822D6030).
    pub fn sensed(&mut self, at: Vec3, velocity: Vec3) {
        self.sensed_at = at;
        self.velocity = velocity;
        self.unseen = 0.0;
    }

    /// One think (0x822DE240).
    pub fn tick(&mut self, dt: f32) {
        self.unseen += dt;
    }

    /// Where the record has her (+0x08, 0x822D5468).
    pub fn at(&self) -> Vec3 {
        let t = if self.minded {
            1.0 - 1.0 / (self.unseen + 1.0)
        } else {
            self.unseen
        };
        self.sensed_at + self.velocity * t
    }

    /// Past its memory: the 360 frees it (0x822DE240).
    pub fn forgotten(&self) -> bool {
        self.unseen > MEMORY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsensed_her_point_moves_on_by_her_velocity() {
        let mut r = TargetRecord::default();
        r.sensed(Vec3::new(10.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 5.0));
        assert_eq!(r.at(), Vec3::new(10.0, 0.0, 0.0));
        for _ in 0..20 {
            r.tick(0.1);
        }
        assert!((r.at() - Vec3::new(10.0, 0.0, 10.0)).length() < 1e-4);
        // A target with a mind: never more than a second's worth.
        r.minded = true;
        assert!((r.at().z - 5.0 * (1.0 - 1.0 / 3.0)).abs() < 1e-4);
        r.unseen = 1000.0;
        assert!(r.at().z < 5.0);
        // Sensed again: where she is.
        r.sensed(Vec3::ONE, Vec3::ZERO);
        assert_eq!((r.at(), r.unseen), (Vec3::ONE, 0.0));
    }

    #[test]
    fn it_is_forgotten_past_thirty_seconds() {
        let mut r = TargetRecord::default();
        r.tick(30.0);
        assert!(!r.forgotten());
        r.tick(0.1);
        assert!(r.forgotten());
    }
}
