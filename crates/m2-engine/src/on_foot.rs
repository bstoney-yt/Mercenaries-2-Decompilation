//! How a person moves on foot: the jump, turning to face the way they go,
//! the build-up and drop of their speed, and the slide to a stop after a
//! run (`RunStop`). From the 360 code, unless a line says otherwise.
//!
//! The 360's human physics object (constructed at 0x82176F38, stepped by
//! 0x8217A2D0) drives Havok's character proxy. Its tuning is the
//! template's `_HumanPhysics` record, whose fields and defaults the program
//! registers at 0x823EAE98 ([`HumanMotion`]). The constructor's tail
//! (0x82177500 on, past the VMX the decompiler stops at) hands the record
//! to the Havok states: `JumpHeight` (+0x1c) to the jumping state's height
//! (setter 0x82691DC8), `MaxAirSpeed` (+0x4c) to the in-air state,
//! `Gravity` (+0x58) to the object's gravity (+0x254), and the larger of
//! `UprightRunSpeed` and `UprightSprintSpeed` to the on-ground state as its
//! full speed (+0x258), against which every pace is kept as a fraction
//! (0x8217D608).

use m2_math::glam::Vec2;

/// A person template's `_HumanPhysics` fields that shape how they move.
/// Defaults are the program's registered ones (0x823EAE98); Jennifer's
/// template gives a jump of 1.0 m, gravity 16.5, a 700 degree a second turn
/// and paces 2, 5, 7 and 12 m/s (round 1's survey of the template data).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HumanMotion {
    /// `JumpHeight`, metres: the height a jump rises to.
    pub jump_height: f32,
    /// `Gravity`, m/s² (positive down).
    pub gravity: f32,
    /// `MaxRotationSpeed`, degrees a second.
    pub max_rotation: f32,
    /// `MaxAirSpeed`, m/s.
    pub max_air_speed: f32,
    /// `UprightWalkSpeed`, `UprightJogSpeed`, `UprightRunSpeed` and
    /// `UprightSprintSpeed`, m/s (the sprint is -1, none, by default).
    pub paces: [f32; 4],
}

impl Default for HumanMotion {
    fn default() -> Self {
        Self {
            jump_height: 1.5,
            gravity: 9.8,
            max_rotation: 180.0,
            max_air_speed: 4.0,
            paces: [1.2, 2.5, 5.0, -1.0],
        }
    }
}

/// The field name hashes [`HumanMotion`] is read from, in its order:
/// `JumpHeight`, `Gravity`, `MaxRotationSpeed`, `MaxAirSpeed`, then the
/// four `Upright` paces.
pub const FIELDS: [&str; 8] = [
    "JumpHeight",
    "Gravity",
    "MaxRotationSpeed",
    "MaxAirSpeed",
    "UprightWalkSpeed",
    "UprightJogSpeed",
    "UprightRunSpeed",
    "UprightSprintSpeed",
];

impl HumanMotion {
    /// From the eight fields in [`FIELDS`]' order.
    pub fn from_fields(f: [f32; 8]) -> Self {
        Self {
            jump_height: f[0],
            gravity: f[1],
            max_rotation: f[2],
            max_air_speed: f[3],
            paces: [f[4], f[5], f[6], f[7]],
        }
    }

    /// The upward speed a jump leaves the ground at, m/s: Havok's jumping
    /// state's first step (0x82691CA8, VMX read by hand) gives the body
    /// `sqrt(2 |g| h)` up, `h` its height (+8) and `g` the character's
    /// gravity, unless it already rises faster.
    pub fn jump_speed(&self) -> f32 {
        (2.0 * self.gravity.abs() * self.jump_height.abs()).sqrt()
    }

    /// The on-ground state's full speed, m/s: the larger of the run and
    /// sprint paces (the constructor's `fsel` at 0x8217771C).
    pub fn full_speed(&self) -> f32 {
        self.paces[2].max(self.paces[3])
    }
}

/// How fast a person's turn to face where they go builds up, rad/s²
/// (0x8217BB60: the constant at 0x82DE8870).
pub const TURN_ACCEL: f32 = 50.0;
/// The same while aiming: the turn is all but instant (0x82DE8878). The
/// program uses it while its animation state's top two flag bits read
/// `10` without bit 0x400; that those bits are the aim is inferred from
/// the message that sets them (type 3 at 0x82178630).
pub const TURN_ACCEL_AIMED: f32 = 10_000.0;
/// The turn's top speed in the air, rad/s (0x820B8034), whatever the
/// template's `MaxRotationSpeed`: Havok's character state 2 is in the air.
pub const AIR_TURN: f32 = 0.8;
/// Facings closer than this (the cosine between them) count as faced,
/// and the turn's speed drops back to 0 (0x820F8BB0).
const FACED: f32 = 0.9999;

/// Turning to face a heading: the turn's speed carries over between steps
/// (+0x16c in the 360's object).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Turn {
    /// Radians a second, signed as the heading turns.
    pub rate: f32,
}

/// Wraps an angle into (-pi, pi].
fn wrap(a: f32) -> f32 {
    let t = std::f32::consts::TAU;
    let mut a = a.rem_euclid(t);
    if a > std::f32::consts::PI {
        a -= t;
    }
    a
}

impl Turn {
    /// Turns `facing` (radians) towards `target` for `dt` seconds, building
    /// the turn's speed up at `accel` rad/s² to at most `max` rad/s; the
    /// new facing. As 0x8217BB60: faced already, it snaps and the speed is
    /// 0; turning back the other way, the speed starts again from 0; the
    /// step never passes the target.
    pub fn step(&mut self, facing: f32, target: f32, accel: f32, max: f32, dt: f32) -> f32 {
        let diff = wrap(target - facing);
        if diff.cos() > FACED {
            self.rate = 0.0;
            return target;
        }
        if diff * self.rate < 0.0 {
            self.rate = 0.0;
        }
        let sign = if diff < 0.0 { -1.0 } else { 1.0 };
        self.rate = (self.rate + sign * accel * dt).clamp(-max, max);
        let by = self.rate * dt;
        let by = if diff.abs() < by.abs() { diff } else { by };
        wrap(facing + by)
    }
}

/// How fast a player's speed builds towards what the stick asks, m/s²
/// (0x82DE87F8).
pub const MOVE_ACCEL: f32 = 7.5;
/// How fast it drops, m/s² (0x82DE87FC): at once, in practice.
pub const MOVE_DECEL: f32 = 1000.0;
/// Asked to go further round than this from the way they move (the
/// cosine, 0x82DE880C: 150 degrees) at over [`REVERSE_MIN`] m/s, they stop
/// first.
const REVERSE_COS: f32 = -0.866;
const REVERSE_MIN: f32 = 0.5;
/// Above this speed, m/s (0x82DE8804), the way they move swings round to
/// the asked one at no more than [`SWING`] rad/s; at or below it, at once.
const SWING_MIN: f32 = 1.75;
/// 180 degrees a second (0x82DE8800 = 180 x pi/180, set by the static
/// initialiser at 0x829064C8).
const SWING: f32 = std::f32::consts::PI;

/// A player's velocity on the ground after `dt` seconds from `current`
/// (m/s, horizontal) when the stick asks for `wanted`: the player's branch
/// of 0x8217BDE0 (the object's +0x418 set; the people's branch turns the
/// asked way at 1200 degrees a second instead). Speed builds at
/// [`MOVE_ACCEL`] and drops at [`MOVE_DECEL`].
pub fn steer(current: Vec2, wanted: Vec2, dt: f32) -> Vec2 {
    let (have, want) = (current.length(), wanted.length());
    let heading = |v: Vec2| v.x.atan2(v.y);
    let reversing =
        have > 1e-4 && want > REVERSE_MIN && current.dot(wanted) / (have * want) < REVERSE_COS;
    let mut way = heading(current);
    let speed = if reversing {
        (have - MOVE_DECEL * dt).max(0.0)
    } else {
        let s = if have > want {
            (have - MOVE_DECEL * dt).max(want)
        } else {
            (have + MOVE_ACCEL * dt).min(want)
        };
        if want > 1e-4 {
            way = heading(wanted);
            if have > SWING_MIN {
                let d = wrap(heading(wanted) - heading(current));
                way = wrap(heading(current) + d.clamp(-SWING * dt, SWING * dt));
            }
        }
        s
    };
    if have <= 1e-4 && want <= 1e-4 {
        return Vec2::ZERO;
    }
    Vec2::new(way.sin(), way.cos()) * speed
}

/// How long the slide to a stop after a run lasts for a player, seconds
/// (0x82DE8850; the people's are 0.25 s, 0.666 s crouched).
pub const RUN_STOP: f32 = 0.4;
/// Let go within this long of last being at a run, and after running at
/// least [`RUN_STOP_RUNNING`], they slide (0x82DE8860, 0x82DE8864), but
/// not until [`RUN_STOP_WAIT`] after (0x82DE885C).
const RUN_STOP_WINDOW: f32 = 0.25;
const RUN_STOP_RUNNING: f32 = 0.25;
const RUN_STOP_WAIT: f32 = 0.05;
/// The action the slide plays, and the one that ends it when they set
/// off again (`RunStop`, `CancelStop`: their name hashes are the ones
/// 0x8217CD20 sends).
pub const RUN_STOP_ACTION: &str = "RunStop";
pub const CANCEL_STOP_ACTION: &str = "CancelStop";

/// The slide to a stop after a run (0x8217CD20).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RunStop {
    /// Seconds since they last ran, and since the run they were last in
    /// began (the object's +0x1b4 and +0x1bc timers).
    since_run: f32,
    since_start: f32,
    /// Sliding: the speed it started at (m/s) and how far through it is,
    /// 0 to 1.
    pub sliding: Option<(f32, f32)>,
}

/// What [`RunStop::step`] did this step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StopEvent {
    None,
    /// The slide began (play `RunStop`).
    Began,
    /// They set off again during it (play `CancelStop`).
    Cancelled,
    /// It ran out.
    Ended,
}

impl RunStop {
    /// Steps on `dt` seconds with the player asking for `speed` m/s
    /// (after [`steer`]); `run` is their run pace. The speed to move at
    /// instead while sliding, and what happened.
    pub fn step(&mut self, speed: f32, run: f32, dt: f32) -> (f32, StopEvent) {
        self.since_run += dt;
        self.since_start += dt;
        if let Some((from, t)) = self.sliding {
            if speed > 0.0 {
                self.sliding = None;
                return (speed, StopEvent::Cancelled);
            }
            let t = t + dt / (RUN_STOP + 1e-4);
            if t >= 1.0 {
                self.sliding = None;
                return (0.0, StopEvent::Ended);
            }
            self.sliding = Some((from, t));
            return ((1.0 - t) * from, StopEvent::None);
        }
        if speed > 0.0 {
            if speed > run - 1e-4 {
                if self.since_run > dt + 1e-4 {
                    self.since_start = 0.0;
                }
                self.since_run = 0.0;
            }
            return (speed, StopEvent::None);
        }
        if self.since_run < RUN_STOP_WINDOW
            && self.since_start - self.since_run > RUN_STOP_RUNNING
            && self.since_run >= RUN_STOP_WAIT
        {
            self.sliding = Some((run, 0.0));
            return (run, StopEvent::Began);
        }
        (0.0, StopEvent::None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn jennifer() -> HumanMotion {
        HumanMotion {
            jump_height: 1.0,
            gravity: 16.5,
            max_rotation: 700.0,
            max_air_speed: 12.0,
            paces: [2.0, 5.0, 7.0, 12.0],
        }
    }

    #[test]
    fn a_jump_rises_to_its_height_under_its_gravity() {
        let m = jennifer();
        let v = m.jump_speed();
        assert!((v - 5.745).abs() < 1e-3, "{v}");
        // Up and down again in 2v/g.
        assert!((2.0 * v / m.gravity - 0.696).abs() < 1e-3);
        assert_eq!(m.full_speed(), 12.0);
        assert_eq!(HumanMotion::default().full_speed(), 5.0);
    }

    #[test]
    fn turning_builds_up_and_never_overshoots() {
        let m = jennifer();
        let max = m.max_rotation.to_radians();
        let mut t = Turn::default();
        let mut f = 0.0;
        let mut steps = 0;
        let target = std::f32::consts::PI - 1e-3;
        while (f - target).abs() > 1e-5 && steps < 600 {
            f = t.step(f, target, TURN_ACCEL, max, DT);
            steps += 1;
        }
        // 180 degrees: sqrt(2 pi / 50) = 0.35 s at 50 rad/s², under the cap.
        let secs = steps as f32 * DT;
        assert!((0.33..0.40).contains(&secs), "{secs}");
        // Aimed, the turn is at its top speed from the first step: a
        // quarter turn at 700 degrees a second takes 0.13 s.
        let mut t = Turn::default();
        let mut f = t.step(0.0, 1.5, TURN_ACCEL_AIMED, max, DT);
        assert!((t.rate - max).abs() < 1e-4);
        let mut steps = 1;
        while (f - 1.5).abs() > 1e-5 {
            f = t.step(f, 1.5, TURN_ACCEL_AIMED, max, DT);
            steps += 1;
        }
        assert_eq!(steps, 8);
        // In the air, 0.8 rad/s at most.
        let mut t = Turn::default();
        let mut f = 0.0;
        for _ in 0..60 {
            f = t.step(f, 3.0, TURN_ACCEL, AIR_TURN, DT);
        }
        assert!(f < 0.81, "{f}");
    }

    #[test]
    fn speed_builds_slowly_and_drops_at_once() {
        let mut v = Vec2::ZERO;
        let want = Vec2::new(0.0, 5.0);
        let mut steps = 0;
        while v.length() < 4.999 {
            v = steer(v, want, DT);
            steps += 1;
        }
        // 5 m/s at 7.5 m/s²: two thirds of a second.
        assert!((steps as f32 * DT - 0.667).abs() < 0.02, "{steps}");
        assert_eq!(steer(v, Vec2::ZERO, DT), Vec2::ZERO);
        // Asked straight back, she stops before going the other way.
        let back = steer(v, -want, DT);
        assert_eq!(back.length(), 0.0);
        // Above 1.75 m/s the way she goes swings at 180 degrees a second.
        let side = steer(v, Vec2::new(5.0, 0.0), DT);
        let turned = side.x.atan2(side.y);
        assert!(
            (turned - std::f32::consts::PI / 60.0).abs() < 1e-4,
            "{turned}"
        );
    }

    #[test]
    fn letting_go_after_a_run_slides_to_a_stop() {
        let mut s = RunStop::default();
        for _ in 0..30 {
            assert_eq!(s.step(7.0, 7.0, DT), (7.0, StopEvent::None));
        }
        // Let go: a moment's wait, then the slide from her run pace.
        let mut began = None;
        for i in 0..10 {
            if let (v, StopEvent::Began) = s.step(0.0, 7.0, DT) {
                began = Some((i, v));
                break;
            }
        }
        let (i, v) = began.expect("the slide began");
        assert!((2..=4).contains(&i), "{i}");
        assert_eq!(v, 7.0);
        let mut moved = 0.0;
        let mut steps = 0;
        loop {
            let (v, e) = s.step(0.0, 7.0, DT);
            moved += v * DT;
            steps += 1;
            if e == StopEvent::Ended {
                break;
            }
        }
        assert!((steps as f32 * DT - RUN_STOP).abs() < 0.03, "{steps}");
        // About half of 7 m/s over 0.4 s.
        assert!((moved - 1.4).abs() < 0.15, "{moved}");
        // A walk lets go without a slide.
        let mut s = RunStop::default();
        for _ in 0..30 {
            s.step(2.0, 7.0, DT);
        }
        for _ in 0..30 {
            assert_eq!(s.step(0.0, 7.0, DT).1, StopEvent::None);
        }
    }
}
