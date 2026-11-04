//! A fighter's close-quarters cycle: the 360's `HumanCombat` branch for a
//! target within reach (0x822B2D30), as the research thread traced it
//! (360 notes s20) and as read here.
//!
//! A timer runs every think. Further than 2 m away, or once the timer is
//! past 3.5 s, the fighter closes in at his `CombatSpeed` (the timer held
//! at 4); between 1 and 3 s he backs off, the slower the further he is;
//! within 1 m with the timer past 2 s he strikes (the melee message the
//! player's button posts, 0x822D2348) and the timer starts again; within
//! 0.5 m he stands. Always facing the target. There is no strafing here.

/// What the cycle does this think.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Close {
    /// Move towards the target at this share of the full speed (negative:
    /// away).
    Move(f32),
    /// Strike.
    Strike,
    /// Stand.
    Stand,
}

/// The cycle's timer (the AI object's +0x728).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CloseQuarters {
    pub timer: f32,
}

impl CloseQuarters {
    /// Thinks on `dt` seconds with the target `distance` metres away, his
    /// `CombatSpeed` `combat` (a share of his full speed).
    pub fn think(&mut self, distance: f32, combat: f32, dt: f32) -> Close {
        self.timer += dt;
        let d2 = distance * distance;
        let t = self.timer;
        let k = if t > 3.5 || d2 > 4.0 {
            self.timer = 4.0;
            1.0
        } else if t > 1.0 && t < 3.0 {
            -1.0 / (d2 + 1.0)
        } else {
            0.0
        };
        if d2 < 1.0 {
            if self.timer > 2.0 {
                self.timer = 0.0;
                return Close::Strike;
            }
            if d2 < 0.25 {
                return Close::Stand;
            }
        }
        Close::Move(k.clamp(-combat, combat))
    }
}

/// How near a target must be for the close-quarters branch, metres beyond
/// the two bodies' radii (0x822B34A8: the squared distance under
/// (r_a + r_b + 1.5)^2, with nothing between them).
pub const REACH: f32 = 1.5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn he_closes_strikes_backs_off_and_closes_again() {
        let mut c = CloseQuarters::default();
        let dt = 0.1;
        // Out at 2.5 m: in at his combat speed.
        assert_eq!(c.think(2.5, 0.3, dt), Close::Move(0.3));
        assert_eq!(c.timer, 4.0);
        // Within 1 m, the timer past 2 s: the blow.
        assert_eq!(c.think(0.8, 0.3, dt), Close::Strike);
        assert_eq!(c.timer, 0.0);
        // Then a moment standing, then backing off.
        for _ in 0..9 {
            c.think(0.8, 0.3, dt);
        }
        match c.think(0.8, 0.3, dt) {
            Close::Move(k) => assert!(k < 0.0, "{k}"),
            other => panic!("{other:?}"),
        }
        // Right on top of him, before the next blow: he stands.
        let mut c = CloseQuarters { timer: 0.5 };
        assert_eq!(c.think(0.4, 0.3, dt), Close::Stand);
    }
}
