//! Climbing a ladder: a building's ladder is a pair of seats (`Ladder Seat
//! (Bottom A)` and `(Top A)`) linked to each other by `SeatToSeat`, each
//! got onto through its own entrance (`bLadder`) at its seat's hardpoint.
//! Of 125 templates with ladder entrances, 537 placements on the island
//! carry 731 such pairs, every one upright (1.6 to 26.4 m long).
//!
//! Provenance:
//! - From the 360 code: the ladder runs between the two seats'
//!   hardpoints, the lower first (0x822A4510: the line between them must
//!   point within 0.85 of straight up or down, else the ladder isn't
//!   offered). Its rungs are a spacing apart taken from the climbing clips
//!   (0x822A4A38: half of the average of the ClimbUp and ClimbDown clips'
//!   rise), counted `floor(length / spacing + 0.5) + 1` from the bottom
//!   (0x82430EC0), where she starts at the rung nearest her. The climb
//!   controller (0x824313F8) waits on a rung for the stick: pushed up past
//!   0.5 she climbs two rungs (one ClimbUp cycle, 0x824320F8) unless she
//!   is within the top dismount's rungs of the top, where she gets off at
//!   the top; pulled down past 0.5 she climbs two rungs down (ClimbDown,
//!   0x82431EF8) until the lowest even rung, where she gets off at the
//!   bottom. Below the lowest rung (just on) she climbs up by herself.
//!   The jump or use button lets go (0x824313F8 sets its let-go flag,
//!   0x82431320 gets her off where she is).
//! - From samples (Jennifer's clips, `OnLadder` stance rows of the action
//!   table): ClimbUp `36784490` rises 0.8 m in 0.534 s, ClimbDown
//!   `9fcade79` falls 0.8 m in 0.534 s (spacing 0.4 m, two rungs a cycle);
//!   the top dismount `4f962bd2` rises 1.681 m in 0.801 s (4 rungs) and
//!   steps 0.418 m on; the bottom dismount `18661daa` drops 0.4 m and
//!   steps 0.44 m back in 0.767 s; the bottom mount `f5364ffe` is its
//!   reverse; the top mount `727bbe36` drops 1.607 m in 1.001 s. While
//!   climbing her root hangs 0.4 m back from the ladder's line, as the
//!   360's rung probes look (0x82430EC0, 0.4 along the seat's facing).
//! - Inferred: she faces along the bottom seat hardpoint's forward axis
//!   (towards the building, the top dismount steps that way); her feet
//!   are at the rung's height, rung 1 at the bottom hardpoint. On the tall
//!   guard tower the hardpoints stand 0.88 m over the ground and 0.59 m
//!   over the platform, so she hangs that much high; where the 360 puts
//!   the root against the seat hardpoint was not traced.
//! - Invented: the collision probes the 360 makes before each climb
//!   (0x8229A8D8) and the ground probe that ends a climb down early are
//!   not made; letting go drops her to the floor under her.

use m2_math::glam::Vec3;

/// How near straight up the line between the seats must be (0x822A4510).
pub const UPRIGHT: f32 = 0.85;
/// The stick's dead zone (0x824313F8).
pub const DEAD_ZONE: f32 = 0.5;
/// How far back from the ladder's line she hangs, metres.
pub const HANG_BACK: f32 = 0.4;

/// The action table's ladder clips, by hash.
pub mod clips {
    pub const CLIMB_UP: u32 = 0x3678_4490;
    pub const CLIMB_DOWN: u32 = 0x9fca_de79;
    pub const IDLE: u32 = 0xbb26_594d;
    pub const MOUNT_BOTTOM: u32 = 0xf536_4ffe;
    pub const MOUNT_TOP: u32 = 0x727b_be36;
    pub const DISMOUNT_BOTTOM: u32 = 0x1866_1daa;
    pub const DISMOUNT_TOP: u32 = 0x4f96_2bd2;
}

/// What the clips give the climb: their lengths and travel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moves {
    /// Rung spacing, metres.
    pub spacing: f32,
    pub up_cycle: f32,
    pub down_cycle: f32,
    pub mount_bottom: f32,
    pub mount_top: f32,
    pub dismount_bottom: f32,
    pub dismount_top: f32,
    /// Rungs the top dismount climbs (0x822A4A38's +0x48).
    pub top_rungs: i32,
    /// The lowest rung she climbs from (0x822A4A38's +0x4c).
    pub low_rung: i32,
    /// Where the top dismount leaves her from the top hardpoint: on along
    /// the facing, and up.
    pub off_top: (f32, f32),
    /// Where the bottom dismount leaves her from her hang: back, and down.
    pub off_bottom: (f32, f32),
}

impl Default for Moves {
    /// Jennifer's clips' numbers (from samples).
    fn default() -> Self {
        Self::from_clips(
            (0.8, 0.534),
            (-0.8, 0.534),
            (1.681, 0.801, 0.418),
            (-0.4, 0.767, 0.44),
            0.767,
            1.001,
        )
    }
}

impl Moves {
    /// From the clips' (rise, seconds): climbing up, down, the top
    /// dismount (and how far on it steps), the bottom dismount (and how
    /// far back), and the mounts' seconds (0x822A4A38).
    pub fn from_clips(
        up: (f32, f32),
        down: (f32, f32),
        top: (f32, f32, f32),
        bottom: (f32, f32, f32),
        mount_bottom: f32,
        mount_top: f32,
    ) -> Self {
        let spacing = ((up.0 * 0.5).abs() + (down.0 * 0.5).abs()) * 0.5;
        let spacing = if spacing > 1e-3 { spacing } else { 0.4 };
        Self {
            spacing,
            up_cycle: up.1,
            down_cycle: down.1,
            mount_bottom,
            mount_top,
            dismount_bottom: bottom.1,
            dismount_top: top.1,
            top_rungs: (top.0 / spacing + 0.5).floor() as i32,
            low_rung: ((-1.0 / spacing) * bottom.0 + 0.5).floor() as i32,
            off_top: (
                top.2,
                (top.0 - (top.0 / spacing + 0.5).floor() * spacing).max(0.0),
            ),
            off_bottom: (bottom.2, -bottom.0),
        }
    }
}

/// One ladder in the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ladder {
    pub bottom: Vec3,
    pub top: Vec3,
    /// Level, the way she faces climbing it.
    pub facing: Vec3,
}

impl Ladder {
    /// The ladder between seat hardpoints `a` and `b` (either may be the
    /// lower), facing along `forward`: `None` unless the line between them
    /// is near upright (0x822A4510). Also whether `a` is the top.
    pub fn between(a: Vec3, b: Vec3, forward: Vec3) -> Option<(Self, bool)> {
        let d = (b - a).normalize_or_zero();
        let facing = forward.with_y(0.0).normalize_or_zero();
        let make = |bottom, top| Ladder {
            bottom,
            top,
            facing,
        };
        if d.y > UPRIGHT {
            Some((make(a, b), false))
        } else if d.y < -UPRIGHT {
            Some((make(b, a), true))
        } else {
            None
        }
    }

    pub fn length(&self) -> f32 {
        self.bottom.distance(self.top)
    }

    /// How many rungs it has (0x82430EC0).
    pub fn rungs(&self, m: &Moves) -> i32 {
        (self.length() / m.spacing + 0.5).floor() as i32 + 1
    }

    /// Her feet hanging from rung `rung` (1 at the bottom).
    pub fn hang(&self, rung: i32, m: &Moves) -> Vec3 {
        let up = (self.top - self.bottom).normalize_or_zero();
        self.bottom + up * ((rung - 1) as f32 * m.spacing) - self.facing * HANG_BACK
    }

    /// The rung a climb down from the top starts on: the top mount's drop
    /// below the top, on the parity she climbs in from the lowest rung
    /// and at most the top dismount's rungs from the top (0x822A4D38).
    pub fn top_rung(&self, m: &Moves, drop: f32) -> i32 {
        let total = self.rungs(m);
        let at = ((self.length() - drop) / m.spacing + 0.5).floor() as i32 + 1;
        let low = m.low_rung.max(1);
        let even = at - (at - low).rem_euclid(2);
        even.min(total - m.top_rungs).max(low)
    }
}

/// What she is doing on a ladder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// Getting on (from the top or bottom), seconds in.
    Mounting {
        top: bool,
        t: f32,
    },
    /// Hanging on a rung.
    Still,
    /// Climbing two rungs, seconds in.
    Up(f32),
    Down(f32),
    /// Getting off (at the top or bottom), seconds in.
    Dismounting {
        top: bool,
        t: f32,
    },
}

/// How she came off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Off {
    /// Stepped off at the top or bottom: where she stands.
    Stepped(Vec3),
    /// Let go: where she was.
    LetGo(Vec3),
}

/// A climb under way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Climb {
    pub ladder: Ladder,
    pub rung: i32,
    pub phase: Phase,
}

impl Climb {
    /// Getting on at the top (`top`) or bottom.
    pub fn start(ladder: Ladder, top: bool, m: &Moves) -> Self {
        let rung = if top {
            ladder.top_rung(m, 4.0 * m.spacing)
        } else {
            m.low_rung.max(1)
        };
        Self {
            ladder,
            rung,
            phase: Phase::Mounting { top, t: 0.0 },
        }
    }

    /// One tick: `stick` up (+) or down (-), `let_go` pressed. `Some`
    /// once she is off.
    pub fn tick(&mut self, m: &Moves, stick: f32, let_go: bool, dt: f32) -> Option<Off> {
        let l = self.ladder;
        if let_go && !matches!(self.phase, Phase::Dismounting { .. }) {
            return Some(Off::LetGo(self.feet(m)));
        }
        match &mut self.phase {
            Phase::Mounting { top, t } => {
                *t += dt;
                let length = if *top { m.mount_top } else { m.mount_bottom };
                if *t >= length {
                    self.phase = Phase::Still;
                }
            }
            Phase::Up(t) => {
                *t += dt;
                if *t >= m.up_cycle {
                    self.rung += 2;
                    self.phase = Phase::Still;
                }
            }
            Phase::Down(t) => {
                *t += dt;
                if *t >= m.down_cycle {
                    self.rung -= 2;
                    self.phase = Phase::Still;
                }
            }
            Phase::Dismounting { top, t } => {
                *t += dt;
                let length = if *top {
                    m.dismount_top
                } else {
                    m.dismount_bottom
                };
                if *t >= length {
                    let at = if *top {
                        l.top + l.facing * m.off_top.0 + Vec3::Y * m.off_top.1
                    } else {
                        l.hang(self.rung, m) - l.facing * m.off_bottom.0 - Vec3::Y * m.off_bottom.1
                    };
                    return Some(Off::Stepped(at));
                }
            }
            Phase::Still => {}
        }
        if self.phase == Phase::Still {
            let total = l.rungs(m);
            let low = m.low_rung.max(1);
            self.phase = if stick > DEAD_ZONE || self.rung < low {
                if self.rung < total - m.top_rungs {
                    Phase::Up(0.0)
                } else {
                    Phase::Dismounting { top: true, t: 0.0 }
                }
            } else if stick < -DEAD_ZONE {
                if self.rung > low + (low & 1) {
                    Phase::Down(0.0)
                } else {
                    Phase::Dismounting { top: false, t: 0.0 }
                }
            } else {
                Phase::Still
            };
        }
        None
    }

    /// Where her feet are put: at the rung the cycle started from (the
    /// clip's root carries her the rest of the way), at the top hardpoint
    /// getting on or off there.
    pub fn feet(&self, m: &Moves) -> Vec3 {
        match self.phase {
            Phase::Mounting { top: true, .. } | Phase::Dismounting { top: true, .. } => {
                self.ladder.top - self.ladder.facing * HANG_BACK
            }
            _ => self.ladder.hang(self.rung, m),
        }
    }

    /// The clip she plays now.
    pub fn clip(&self) -> u32 {
        match self.phase {
            Phase::Mounting { top: true, .. } => clips::MOUNT_TOP,
            Phase::Mounting { top: false, .. } => clips::MOUNT_BOTTOM,
            Phase::Still => clips::IDLE,
            Phase::Up(_) => clips::CLIMB_UP,
            Phase::Down(_) => clips::CLIMB_DOWN,
            Phase::Dismounting { top: true, .. } => clips::DISMOUNT_TOP,
            Phase::Dismounting { top: false, .. } => clips::DISMOUNT_BOTTOM,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tower() -> Ladder {
        // The tall guard tower's: 14 m.
        Ladder::between(Vec3::ZERO, Vec3::new(0.0, 14.0, 0.0), Vec3::Z)
            .unwrap()
            .0
    }

    #[test]
    fn clips_give_the_rungs() {
        let m = Moves::default();
        assert!((m.spacing - 0.4).abs() < 1e-6);
        assert_eq!(m.top_rungs, 4);
        assert_eq!(m.low_rung, 1);
        assert_eq!(tower().rungs(&m), 36);
    }

    #[test]
    fn only_upright_lines_are_ladders() {
        let up = Ladder::between(Vec3::Y * 5.0, Vec3::ZERO, Vec3::X).unwrap();
        assert!(up.1, "the first seat given was the top");
        assert_eq!(up.0.bottom, Vec3::ZERO);
        assert!(Ladder::between(Vec3::ZERO, Vec3::new(3.0, 2.0, 0.0), Vec3::X).is_none());
    }

    fn run(c: &mut Climb, m: &Moves, stick: f32, secs: f32) -> Option<Off> {
        let dt = 1.0 / 60.0;
        let mut t = 0.0;
        while t < secs {
            if let Some(o) = c.tick(m, stick, false, dt) {
                return Some(o);
            }
            t += dt;
        }
        None
    }

    #[test]
    fn climbs_two_rungs_a_cycle_and_gets_off_at_the_top() {
        let m = Moves::default();
        let mut c = Climb::start(tower(), false, &m);
        assert_eq!(c.rung, 1);
        assert_eq!(run(&mut c, &m, 0.0, 2.0), None);
        assert_eq!(c.rung, 1, "she waits on the first rung");
        run(&mut c, &m, 1.0, 0.6);
        assert_eq!(c.rung, 3);
        // 36 rungs, off from rung 32: 16 cycles of 0.534 s, then 0.801.
        let off = run(&mut c, &m, 1.0, 60.0).expect("off the top");
        let Off::Stepped(at) = off else {
            panic!("{off:?}")
        };
        assert!((at.y - 14.0).abs() < 0.2, "{at}");
        assert!(at.z > 0.3, "stepped on over the top: {at}");
    }

    #[test]
    fn climbing_down_ends_at_the_bottom() {
        let m = Moves::default();
        let mut c = Climb::start(tower(), true, &m);
        assert_eq!(c.rung, 31, "the top mount's drop, on odd rungs");
        let off = run(&mut c, &m, -1.0, 60.0).expect("off the bottom");
        assert_eq!(c.rung, 1);
        let Off::Stepped(at) = off else {
            panic!("{off:?}")
        };
        assert!(at.y < 0.0 && at.z < -0.8, "{at}");
    }

    #[test]
    fn letting_go_leaves_her_where_she_hangs() {
        let m = Moves::default();
        let mut c = Climb::start(tower(), false, &m);
        run(&mut c, &m, 1.0, 3.0);
        let at = c.feet(&m);
        assert_eq!(c.tick(&m, 0.0, true, 0.016), Some(Off::LetGo(at)));
        assert!(at.y > 1.0 && (at.z + HANG_BACK).abs() < 1e-5);
    }
}
