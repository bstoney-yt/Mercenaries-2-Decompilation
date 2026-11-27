//! A person looking into something he heard but has not seen: the 360's
//! `HumanInvestigate` task (step 0x822B49C8, start 0x822B4C88). From the
//! 360 code; the map is `docs/fidelity/combat-mover.md`.
//!
//! On start, moving faster than 1.45 m/s, he stops. With a point to look
//! at (his orders' +0x174C object) he turns to it (phase 0, 0x822B28B0:
//! until within 0.05 rad); without one he looks about at once (phase 1).
//! Once turned and 2 s on, he looks about (0x822B25D0: one of two looks,
//! 0x53AC2C09 or 0x3A054530, by which side of him the point is). 2 s
//! after that:
//!
//! - without an area to keep to, he is done;
//! - with one and his group's alarm high (its +0x1774 over
//!   `1 - 0.01 x` its +0x1730), he takes cover (`HumanTakeCover`);
//! - else he walks to where his record has the target (phase 2) at full
//!   push (0x822B1900, haste 1), until 10 s have passed or he is beyond
//!   0.6 of his area's radius from its centre (0x822C0648). Then a
//!   callout (0x5FB7650A, heard 100 m), and done.
//!
//! The task's timer is the mind's +0x720 (inferred: counted up by the
//! frame's step; the counter is not in these functions). Not traced: the
//! group's alarm values; taken as not high.

use m2_math::glam::Vec3;

/// He stops on start when faster than this, m/s (0x820F9248).
pub const STOP_ABOVE: f32 = 1.45;
/// Turned once within this of the point, radians (0x820B8170).
pub const TURNED: f32 = 0.05;
/// Seconds from turned to the look, and from the look to the walk
/// (0x820C77A4).
pub const PAUSE: f32 = 2.0;
/// The walk's longest, seconds (0x820BAEEC), and the share of his area's
/// radius it stops past (0x82B7D82C).
pub const WALK: f32 = 10.0;
pub const AREA_SHARE: f32 = 0.6;
/// The callout at the end.
pub const CALLOUT: u32 = 0x5fb7_650a;
/// The two looks about.
pub const LOOKS: [u32; 2] = [0x53ac_2c09, 0x3a05_4530];

/// What the task is told each step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sense {
    pub dt: f32,
    /// How far his facing is off the point, radians.
    pub off: f32,
    /// His area keeps him (an anchor that holds, [`crate::ai_anchor`]).
    pub has_area: bool,
    /// He is beyond [`AREA_SHARE`] of its radius from its centre.
    pub beyond_share: bool,
    /// Where his record has the target.
    pub target: Vec3,
}

/// What he does this step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Do {
    /// Stand (turning to the point, or waiting).
    Stand,
    /// Look about (once).
    Look,
    /// Walk there at full push.
    Walk(Vec3),
    /// Done: say the callout if `callout`.
    Done { callout: bool },
}

/// The task's state: its phase (+0x4F4) and timer (+0x720).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Investigate {
    phase: u8,
    timer: f32,
    looked: bool,
}

impl Investigate {
    /// The start: to turn to a point, or (`point` false) look about now.
    pub fn start(point: bool) -> Self {
        Self {
            phase: if point { 0 } else { 1 },
            timer: 0.0,
            looked: false,
        }
    }

    /// The phase, for checking.
    pub fn phase(&self) -> u8 {
        self.phase
    }

    pub fn step(&mut self, s: &Sense) -> Do {
        self.timer += s.dt;
        match self.phase {
            0 => {
                if s.off <= TURNED && self.timer >= PAUSE {
                    self.timer = 0.0;
                    self.phase = 1;
                    self.looked = true;
                    return Do::Look;
                }
                Do::Stand
            }
            1 => {
                if !self.looked {
                    self.looked = true;
                    return Do::Look;
                }
                if self.timer < PAUSE {
                    return Do::Stand;
                }
                self.timer = 0.0;
                if !s.has_area {
                    self.phase = 3;
                    return Do::Done { callout: false };
                }
                self.phase = 2;
                Do::Walk(s.target)
            }
            2 => {
                if self.timer > WALK || s.beyond_share {
                    self.phase = 3;
                    Do::Done { callout: true }
                } else {
                    Do::Walk(s.target)
                }
            }
            // Done stays done.
            _ => Do::Done { callout: false },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sense() -> Sense {
        Sense {
            dt: 0.1,
            off: 0.0,
            has_area: true,
            beyond_share: false,
            target: Vec3::new(0.0, 0.0, 20.0),
        }
    }

    fn run(t: &mut Investigate, s: &Sense, seconds: f32) -> Vec<Do> {
        (0..(seconds / s.dt).round() as usize)
            .map(|_| t.step(s))
            .collect()
    }

    #[test]
    fn he_turns_looks_then_walks_ten_seconds_and_calls_out() {
        let mut t = Investigate::start(true);
        // Not yet turned: he stands however long.
        let turning = Sense {
            off: 0.3,
            ..sense()
        };
        assert!(run(&mut t, &turning, 3.0).iter().all(|d| *d == Do::Stand));
        // Turned (and past 2 s): the look, then 2 s, then the walk.
        assert_eq!(t.step(&sense()), Do::Look);
        let wait = run(&mut t, &sense(), 1.9);
        assert!(wait.iter().all(|d| *d == Do::Stand));
        let walk = run(&mut t, &sense(), 0.2);
        assert_eq!(walk.last(), Some(&Do::Walk(sense().target)));
        assert_eq!(t.phase(), 2);
        let walked = run(&mut t, &sense(), 9.8);
        assert!(walked.iter().all(|d| matches!(d, Do::Walk(_))));
        let end = run(&mut t, &sense(), 0.3);
        assert!(end.contains(&Do::Done { callout: true }));
        assert_eq!(t.phase(), 3);
    }

    #[test]
    fn the_walk_stops_past_six_tenths_of_his_area() {
        let mut t = Investigate::start(false);
        run(&mut t, &sense(), 2.2);
        let out = Sense {
            beyond_share: true,
            ..sense()
        };
        assert_eq!(t.step(&out), Do::Done { callout: true });
    }

    #[test]
    fn without_an_area_he_only_looks() {
        let mut t = Investigate::start(false);
        let free = Sense {
            has_area: false,
            ..sense()
        };
        assert_eq!(t.step(&free), Do::Look);
        let rest = run(&mut t, &free, 2.2);
        assert_eq!(rest.last(), Some(&Do::Done { callout: false }));
        assert!(t.phase() == 3);
    }
}
