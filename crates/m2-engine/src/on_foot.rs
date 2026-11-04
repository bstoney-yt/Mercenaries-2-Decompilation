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
    /// `CrouchWalkSpeed`, m/s: the pace crouched, which is how the 360
    /// aims in (every hero 1.5).
    pub crouch_walk: f32,
    /// `ProxyCapsuleRadius` and `ProxyCapsuleHeight`, metres: the
    /// standing capsule runs from the feet to the full height (the
    /// constructor's first shape, 0x821771B0).
    pub capsule_radius: f32,
    pub capsule_height: f32,
}

impl Default for HumanMotion {
    fn default() -> Self {
        Self {
            jump_height: 1.5,
            gravity: 9.8,
            max_rotation: 180.0,
            max_air_speed: 4.0,
            paces: [1.2, 2.5, 5.0, -1.0],
            crouch_walk: 1.5,
            capsule_radius: 0.4,
            capsule_height: 1.8,
        }
    }
}

/// The field name hashes [`HumanMotion`] is read from, in its order:
/// `JumpHeight`, `Gravity`, `MaxRotationSpeed`, `MaxAirSpeed`, the four
/// `Upright` paces and `CrouchWalkSpeed`.
pub const FIELDS: [&str; 11] = [
    "JumpHeight",
    "Gravity",
    "MaxRotationSpeed",
    "MaxAirSpeed",
    "UprightWalkSpeed",
    "UprightJogSpeed",
    "UprightRunSpeed",
    "UprightSprintSpeed",
    "CrouchWalkSpeed",
    "ProxyCapsuleRadius",
    "ProxyCapsuleHeight",
];

impl HumanMotion {
    /// From the eight fields in [`FIELDS`]' order.
    pub fn from_fields(f: [f32; 11]) -> Self {
        Self {
            jump_height: f[0],
            gravity: f[1],
            max_rotation: f[2],
            max_air_speed: f[3],
            paces: [f[4], f[5], f[6], f[7]],
            crouch_walk: f[8],
            capsule_radius: f[9],
            capsule_height: f[10],
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

/// How fast the way a person moves swings round to the way the stick
/// points, rad/s (1200 degrees a second: 0x820F9808 scaled to radians by
/// the static initialiser at 0x82900138), for a person without an
/// `Ai` component, as the player is (the object's +0x418 is set when the
/// template has the AI personality record, table 0x83653598: the
/// constructor's tail at 0x821779B4). The swing is slowed where the turn
/// is small: by the turn over pi, but no less than [`SWING_MIN_SHARE`]
/// (0x8217BDE0, its first branch).
pub const SWING: f32 = 1200.0 * std::f32::consts::PI / 180.0;
const SWING_MIN_SHARE: f32 = 0.4;

/// The way a player moves after `dt` seconds, from `way` (a unit
/// vector) towards `stick` (the stick's way, any length): the new unit
/// way. The stick's length passes through untouched.
pub fn swing(way: Vec2, stick: Vec2, dt: f32) -> Vec2 {
    if stick.length_squared() < 1e-8 {
        return way;
    }
    let want = stick.normalize();
    if way.length_squared() < 1e-8 {
        return want;
    }
    let heading = |v: Vec2| v.x.atan2(v.y);
    let d = wrap(heading(want) - heading(way));
    let share = (d.abs() / std::f32::consts::PI).clamp(SWING_MIN_SHARE, 1.0);
    let step = d.clamp(-SWING * dt, SWING * dt) * share;
    let h = heading(way) + step;
    Vec2::new(h.sin(), h.cos())
}

/// Which pace the stick picks (0x82179350): the gait states it reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    Still,
    Walk,
    Jog,
    Run,
    /// Sprinting: between the run and the sprint pace.
    Sprint,
}

/// The pace and speed (m/s) a person without an `Ai` component moves
/// at with the stick pushed `stick` of the way (0 to 1, the player
/// control passes the stick's own length, no less than 0.25 once pushed:
/// 0x824333E0), sprinting or not, `align` the cosine between the way they
/// face and the way they move, crouched (aimed in) or not. As 0x82179350:
/// the paces are kept as shares of [`HumanMotion::full_speed`]
/// (0x8217D608) and the stick picks between them at the midpoints
/// (0x8217D598); crouched it is `CrouchWalkSpeed` once past the walk's
/// threshold; sprinting, the run pace plus the share of the way to the
/// sprint pace that they face where they go.
pub fn pace(m: &HumanMotion, stick: f32, sprint: bool, align: f32, crouched: bool) -> (Pace, f32) {
    let [walk, jog, run, sprint_pace] = m.paces;
    let k = m.full_speed().max(1e-4);
    let s = stick * k;
    if s <= walk * 0.5 {
        return (Pace::Still, 0.0);
    }
    if crouched {
        return (Pace::Walk, m.crouch_walk);
    }
    if sprint && sprint_pace > run {
        return (Pace::Sprint, run + align.max(0.0) * (sprint_pace - run));
    }
    if s < (walk + jog) * 0.5 {
        (Pace::Walk, walk)
    } else if s < (jog + run) * 0.5 {
        (Pace::Jog, jog)
    } else {
        (Pace::Run, run)
    }
}

/// How long a person takes to get going from a stop, seconds (0x82DE8868):
/// their speed is scaled by the square of the share of it gone. Stopping
/// for less than [`START_AGAIN`] (0x82DE886C) and setting off again needs
/// no build-up (0x82179588).
pub const START: f32 = 0.06;
const START_AGAIN: f32 = 0.25;

/// The build-up of a person's speed from a stop (0x82179588: the
/// object's +0x184 state and +0x188 timer).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Start {
    state: u8,
    time: f32,
}

impl Start {
    /// Steps on `dt` with `speed` asked: the speed to go at.
    pub fn step(&mut self, speed: f32, dt: f32) -> f32 {
        let moving = speed > 0.0;
        let next = match (self.state, moving) {
            (0, true) => 1,
            (1, true) if self.time > START => 2,
            (1, false) => 0,
            (2, false) => 3,
            (3, true) => 2,
            (3, false) if self.time > START_AGAIN => 0,
            (s, _) => s,
        };
        if next != self.state {
            self.state = next;
            self.time = 0.0;
        }
        let out = if self.state == 1 {
            let t = (self.time / START).clamp(0.0, 1.0);
            speed * t * t
        } else {
            speed
        };
        self.time += dt;
        out
    }

    /// Getting going: the turn to face builds at [`TURN_ACCEL_STARTING`]
    /// instead.
    pub fn starting(&self) -> bool {
        self.state == 1
    }
}

/// The turn's build-up while getting going, rad/s² (0x82DE8874).
pub const TURN_ACCEL_STARTING: f32 = 10.0;

/// How fast the speed of a person with an `Ai` component builds towards
/// what they ask, m/s² (0x82DE87F8): the people, not the player (the
/// object's +0x418 is set from the AI personality record, table
/// 0x83653598; see [`SWING`]).
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
const AI_SWING: f32 = std::f32::consts::PI;

/// The velocity on the ground of a person with an `Ai` component after
/// `dt` seconds from `current` (m/s, horizontal) when they ask for
/// `wanted`: that branch of 0x8217BDE0 (the player takes [`swing`] and
/// [`pace`] instead). Speed builds at [`MOVE_ACCEL`] and drops at
/// [`MOVE_DECEL`]; the people's run stop lasts 0.4 s (0x82DE8850).
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
                way = wrap(heading(current) + d.clamp(-AI_SWING * dt, AI_SWING * dt));
            }
        }
        s
    };
    if have <= 1e-4 && want <= 1e-4 {
        return Vec2::ZERO;
    }
    Vec2::new(way.sin(), way.cos()) * speed
}

/// Aiming in, a player drops into the `Crouched` stance (the pad's aim
/// button, bit 0x20000, sends the `Crouch` action and the low-mode
/// message from the player control at 0x824333E0; the camera record's
/// `CrouchFov` 0.75 and `CrouchYawSpeed` 0.5 are the aimed view). For
/// this long after (0x82DE8858), and for as long as the slide to a stop
/// that sets off lasts, what the stick asks is held at nothing
/// (the same function, its flag +0x26 and timer +0x28): running, she slides into the crouch on her run's speed
/// ([`RunStop`]); standing, she holds still a moment.
pub const CROUCH_HOLD: f32 = 0.25;

/// How long the slide to a stop after a run lasts, seconds, for a person
/// without an `Ai` component (the people with one take 0.4 s,
/// 0x82DE8850): 0.25 s (0x82DE8840, 0x82DE8844), crouched 0.666 s
/// (0x82DE8848), crouched from a sprint 1.3 s (0x82DE884C) (0x8217CD20).
pub fn run_stop_length(crouched: bool, sprinting: bool) -> f32 {
    match (crouched, sprinting) {
        (false, _) => 0.25,
        (true, false) => 0.666,
        (true, true) => 1.3,
    }
}
/// The slide ends this many times faster once hurried (0x82DE8854: the
/// player control's message 3, sent while she drops into the crouch with
/// the stick not pushed the way she slides).
pub const RUN_STOP_HURRY: f32 = 3.0;
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
    /// began (the object's +0x1b4 and +0x1bc timers), and whether they
    /// sprinted then (+0x1b1).
    since_run: f32,
    since_start: f32,
    sprinted: bool,
    /// Sliding: the speed it started at (m/s), how far through it is (0
    /// to 1), how long it lasts and whether it was hurried (+0x1b0).
    pub sliding: Option<(f32, f32)>,
    length: f32,
    hurried: bool,
    /// A person with an `Ai` component: their slide lasts [`AI_RUN_STOP`].
    ai: bool,
}

/// How long the slide to a stop lasts for a person with an `Ai`
/// component, seconds (0x82DE8850).
pub const AI_RUN_STOP: f32 = 0.4;

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
    /// One for a person with an `Ai` component (the people).
    pub fn people() -> Self {
        Self {
            ai: true,
            ..Self::default()
        }
    }

    /// Steps on `dt` seconds with `speed` m/s asked (after the pace pick
    /// and [`Start`]); `m` gives the run and sprint paces, `sprinting`
    /// whether the sprint is held, `crouched` whether aimed in. The
    /// speed to move at instead while sliding, and what happened.
    pub fn step(
        &mut self,
        m: &HumanMotion,
        speed: f32,
        sprinting: bool,
        crouched: bool,
        dt: f32,
    ) -> (f32, StopEvent) {
        let [_, _, run, sprint] = m.paces;
        self.since_run += dt;
        self.since_start += dt;
        if let Some((from, t)) = self.sliding {
            if speed > 0.0 {
                self.sliding = None;
                return (speed, StopEvent::Cancelled);
            }
            let hurry = if self.hurried { RUN_STOP_HURRY } else { 1.0 };
            let t = t + hurry * dt / (self.length + 1e-4);
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
                self.sprinted = sprinting;
            }
            return (speed, StopEvent::None);
        }
        if self.since_run < RUN_STOP_WINDOW
            && self.since_start - self.since_run > RUN_STOP_RUNNING
            && self.since_run >= RUN_STOP_WAIT
        {
            let from = if self.sprinted && sprint > run {
                sprint
            } else {
                run
            };
            self.length = if self.ai {
                AI_RUN_STOP
            } else {
                run_stop_length(crouched, self.sprinted)
            };
            self.hurried = false;
            self.sliding = Some((from, 0.0));
            return (from, StopEvent::Began);
        }
        (0.0, StopEvent::None)
    }

    /// Hurries a slide under way to its end (the player control's message
    /// 3, handled at 0x82178948).
    pub fn hurry(&mut self) {
        if self.sliding.is_some() {
            self.hurried = true;
        }
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
            crouch_walk: 1.5,
            capsule_radius: 0.4,
            capsule_height: 1.8,
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
    fn the_stick_picks_a_pace_and_sprinting_runs_up_to_the_sprint() {
        let m = jennifer();
        // Shares of her full 12 m/s: walk past 1/12, jog past 7/24, run
        // past 1/2.
        assert_eq!(pace(&m, 0.05, false, 1.0, false), (Pace::Still, 0.0));
        assert_eq!(pace(&m, 0.25, false, 1.0, false), (Pace::Walk, 2.0));
        assert_eq!(pace(&m, 0.4, false, 1.0, false), (Pace::Jog, 5.0));
        assert_eq!(pace(&m, 1.0, false, 1.0, false), (Pace::Run, 7.0));
        assert_eq!(pace(&m, 1.0, true, 1.0, false), (Pace::Sprint, 12.0));
        assert_eq!(pace(&m, 1.0, true, 0.5, false), (Pace::Sprint, 9.5));
        assert_eq!(pace(&m, 1.0, true, 1.0, true), (Pace::Walk, 1.5));
    }

    #[test]
    fn getting_going_takes_six_hundredths_and_the_way_swings_fast() {
        let mut s = Start::default();
        let first = s.step(7.0, DT);
        assert_eq!(first, 0.0);
        let mut v = 0.0;
        for _ in 0..5 {
            v = s.step(7.0, DT);
        }
        assert_eq!(v, 7.0);
        // A quarter turn of the stick: swung at up to 1200 degrees a
        // second, slowed to half for a quarter of pi.
        let w = swing(Vec2::Y, Vec2::X, DT);
        let turned = w.x.atan2(w.y).to_degrees();
        assert!((turned - 10.0).abs() < 0.1, "{turned}");
    }

    #[test]
    fn letting_go_after_a_run_slides_to_a_stop() {
        let m = jennifer();
        let slide = |sprint: bool, crouch: bool| {
            let mut s = RunStop::default();
            let pace = if sprint { 12.0 } else { 7.0 };
            for _ in 0..30 {
                s.step(&m, pace, sprint, false, DT);
            }
            let mut began = None;
            for _ in 0..10 {
                if let (v, StopEvent::Began) = s.step(&m, 0.0, sprint, crouch, DT) {
                    began = Some(v);
                    break;
                }
            }
            let from = began.expect("the slide began");
            let (mut moved, mut secs) = (0.0, 0.0);
            loop {
                let (v, e) = s.step(&m, 0.0, sprint, crouch, DT);
                moved += v * DT;
                secs += DT;
                if e == StopEvent::Ended {
                    break;
                }
            }
            (from, secs, moved)
        };
        let (from, secs, moved) = slide(false, false);
        assert_eq!(from, 7.0);
        assert!((secs - 0.25).abs() < 0.03 && (moved - 0.875).abs() < 0.15);
        // Aimed in from a sprint: a long low slide from the sprint pace.
        let (from, secs, moved) = slide(true, true);
        assert_eq!(from, 12.0);
        assert!((secs - 1.3).abs() < 0.03, "{secs}");
        assert!((moved - 7.8).abs() < 0.4, "{moved}");
        // A walk lets go without a slide.
        let mut s = RunStop::default();
        for _ in 0..30 {
            s.step(&m, 2.0, false, false, DT);
        }
        for _ in 0..30 {
            assert_eq!(s.step(&m, 0.0, false, false, DT).1, StopEvent::None);
        }
    }
}

/// Which way an aiming person's legs carry them (the locomotion node's
/// state, +0x30 of the object 0x828071A8 steps).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AimLegs {
    /// Standing (state 2).
    #[default]
    Still,
    /// Walking forwards the way they go, the upper body turned to the
    /// aim (state 0).
    Forward,
    /// Walking backwards, facing away from the way they go (state 1).
    Backward,
}

impl AimLegs {
    /// The legs' state for moving at `rel` radians from the aim's heading
    /// (positive to the left), from `self`, as 0x828071A8 picks it while
    /// aiming: forward until past 90 degrees to the left or 155 to the
    /// right; back to forward inside 80 to the left and 145 to the right;
    /// from standing, backwards past 90 left or 150 right. `None` for
    /// standing still.
    pub fn step(self, rel: Option<f32>) -> Self {
        let Some(rel) = rel.map(wrap) else {
            return Self::Still;
        };
        let d = rel.to_degrees();
        match self {
            Self::Forward if !(-155.0..=90.0).contains(&d) => Self::Backward,
            Self::Forward => Self::Forward,
            Self::Backward if d < 80.0 && d > -145.0 => Self::Forward,
            Self::Backward => Self::Backward,
            Self::Still if !(-150.0..=90.0).contains(&d) => Self::Backward,
            Self::Still => Self::Forward,
        }
    }

    /// The heading the legs face, moving along `heading`.
    pub fn facing(self, heading: f32) -> f32 {
        match self {
            Self::Backward => wrap(heading + std::f32::consts::PI),
            _ => heading,
        }
    }
}

/// How fast an aiming person's legs swing round to the way they face,
/// rad/s (720 degrees a second, 0x820F939C).
pub const LEG_SWING: f32 = 4.0 * std::f32::consts::PI;

#[cfg(test)]
mod aim_tests {
    use super::*;

    #[test]
    fn aimed_legs_switch_with_hysteresis_and_handedness() {
        let r = |d: f32| Some(d.to_radians());
        let f = AimLegs::Still.step(r(0.0));
        assert_eq!(f, AimLegs::Forward);
        // Strafing right keeps forward legs, far round to the right too.
        assert_eq!(f.step(r(-150.0)), AimLegs::Forward);
        assert_eq!(f.step(r(-160.0)), AimLegs::Backward);
        // To the left, past 90 degrees goes backwards...
        let b = f.step(r(95.0));
        assert_eq!(b, AimLegs::Backward);
        // ...and comes back only inside 80.
        assert_eq!(b.step(r(85.0)), AimLegs::Backward);
        assert_eq!(b.step(r(75.0)), AimLegs::Forward);
        assert_eq!(b.step(None), AimLegs::Still);
        assert!((b.facing(0.5) - (0.5 - std::f32::consts::PI)).abs() < 1e-5);
    }
}

/// Standing aimed, how far the aim may get from the way her feet point
/// before she shuffles round, radians (60 degrees, the constants at
/// 0x82070290 and 0x8205F1CC in the standing locomotion, 0x82809D60).
pub const SHUFFLE_AT: f32 = std::f32::consts::FRAC_PI_3;
/// How far a shuffle to the left and to the right turns her feet over
/// its clip, radians (110 and 90 degrees: 0x82070298 and 0x820F8D08,
/// scaled to radians by the static initialisers at 0x82A2A380 and
/// 0x82A2A3A0).
pub const SHUFFLE_LEFT: f32 = 110.0 * std::f32::consts::PI / 180.0;
pub const SHUFFLE_RIGHT: f32 = std::f32::consts::FRAC_PI_2;
/// Swinging the aim round faster than the shuffle turns, the feet turn at
/// this much of the aim's rate instead (0x82B4DB14).
pub const SHUFFLE_CATCH_UP: f32 = 1.1;

/// A shuffle under way: which way, and seconds into its clip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shuffle {
    pub left: bool,
    pub time: f32,
}

impl Shuffle {
    /// Starts one when the feet (`feet`) are further than [`SHUFFLE_AT`]
    /// from the aim (`aim`), both headings: to the left when the aim is to
    /// their left.
    pub fn start(feet: f32, aim: f32) -> Option<Self> {
        let off = wrap(feet - aim);
        (off.abs() > SHUFFLE_AT).then_some(Self {
            left: off < 0.0,
            time: 0.0,
        })
    }

    /// Steps on `dt` seconds of a clip `length` seconds long with the aim
    /// turning at `aim_rate` rad/s: the feet's turn this step (radians,
    /// positive to the left) and the clip's play rate, or `None` once the
    /// clip has played out (0x82809D60, states 1 and 2).
    pub fn step(&mut self, length: f32, aim_rate: f32, dt: f32) -> Option<(f32, f32)> {
        if self.time >= length {
            return None;
        }
        let base = if self.left {
            SHUFFLE_LEFT
        } else {
            SHUFFLE_RIGHT
        } / length.max(1e-4);
        let rate = if aim_rate.abs() > base {
            aim_rate.abs() * SHUFFLE_CATCH_UP
        } else {
            base
        };
        let play = rate / base;
        self.time += dt * play;
        let sign = if self.left { 1.0 } else { -1.0 };
        Some((sign * rate * dt, play))
    }
}

#[cfg(test)]
mod shuffle_tests {
    use super::*;

    #[test]
    fn standing_aimed_her_feet_stay_until_sixty_degrees_then_shuffle() {
        assert_eq!(Shuffle::start(0.0, 0.9), None);
        let s = Shuffle::start(0.0, 1.2).unwrap();
        assert!(s.left);
        assert!(!Shuffle::start(0.0, -1.2).unwrap().left);
        // A 0.5 s clip turns her 110 degrees left over its length.
        let mut s = s;
        let mut turned = 0.0;
        while let Some((d, _)) = s.step(0.5, 0.0, 1.0 / 60.0) {
            turned += d;
        }
        assert!(
            (turned.to_degrees() - 110.0).abs() < 4.0,
            "{}",
            turned.to_degrees()
        );
    }
}

#[cfg(test)]
mod people_tests {
    use super::*;

    #[test]
    fn the_peoples_speed_builds_slowly_and_drops_at_once() {
        let dt = 1.0 / 60.0;
        let mut v = Vec2::ZERO;
        let want = Vec2::new(0.0, 5.0);
        let mut steps = 0;
        while v.length() < 4.999 {
            v = steer(v, want, dt);
            steps += 1;
        }
        // 5 m/s at 7.5 m/s²: two thirds of a second.
        assert!((steps as f32 * dt - 0.667).abs() < 0.02, "{steps}");
        assert_eq!(steer(v, Vec2::ZERO, dt), Vec2::ZERO);
        // Asked straight back, they stop before going the other way.
        assert_eq!(steer(v, -want, dt).length(), 0.0);
    }
}

/// Havok's character on-ground state, as the 360 builds it (0x8268D398):
/// each step the velocity over the ground closes this share of its gap to
/// what is wanted (0x82053618)...
pub const GROUND_GAIN: f32 = 0.95;
/// ...changing by at most this much a second, m/s² (0x820516B4).
pub const GROUND_MAX_ACCEL: f32 = 200.0;
/// Off the ground for less than this, seconds (0x82DE88B0), a person
/// still steers as on it ("ledge grace", 0x82171330), the change capped at
/// [`LEDGE_MAX_CHANGE`] m/s a step whatever the step, falling meanwhile.
pub const LEDGE_GRACE: f32 = 0.25;
pub const LEDGE_MAX_CHANGE: f32 = 100.0;

/// The horizontal velocity after one Havok step of `dt` from `current`
/// towards `wanted` (0x8268DD08): the gap times [`GROUND_GAIN`], its
/// length capped at `max_change` ([`GROUND_MAX_ACCEL`] x `dt` on the
/// ground, [`LEDGE_MAX_CHANGE`] in the grace). Per step, so it depends on
/// the step's length as the 360's does.
pub fn ground_step(current: Vec2, wanted: Vec2, max_change: f32) -> Vec2 {
    let d = wanted - current;
    let change = d * GROUND_GAIN;
    let len = change.length();
    if len > max_change && len > 0.0 {
        current + change * (max_change / len)
    } else {
        current + change
    }
}

#[cfg(test)]
mod ground_tests {
    use super::*;

    #[test]
    fn havok_closes_ninety_five_percent_of_the_gap_a_step() {
        let dt = 1.0 / 60.0;
        // 0 to 5 m/s: 4.75 asked of the first step, capped at 200 x dt.
        let v = ground_step(Vec2::ZERO, Vec2::new(0.0, 5.0), GROUND_MAX_ACCEL * dt);
        assert!((v.y - 3.333).abs() < 1e-3, "{v}");
        let v = ground_step(v, Vec2::new(0.0, 5.0), GROUND_MAX_ACCEL * dt);
        assert!((v.y - (3.333 + 0.95 * 1.667)).abs() < 1e-2, "{v}");
        // In the ledge grace the cap is 100 m/s a step: the 95% stands.
        let v = ground_step(Vec2::ZERO, Vec2::new(0.0, 5.0), LEDGE_MAX_CHANGE);
        assert!((v.y - 4.75).abs() < 1e-4);
    }
}
