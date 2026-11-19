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
    /// The falls that hurt ([`fall_impact`]).
    pub falls: FallLimits,
}

/// A person template's `_HumanPhysics` fall fields: how far a fall is
/// light, heavy and deadly, metres (`FallDistanceLight`,
/// `FallDistanceHeavy`, `FallDistanceDeath`, at +0x60, +0x64 and +0x68 of
/// the record), and what the light and the heavy one cost
/// (`FallDamageLight`, `FallDamageHeavy`, +0x6c and +0x70). The defaults
/// are the program's registered ones (0x823EAE98), which every soldier's
/// template keeps; the heroes' give 5, 15 and 30 m (from samples).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FallLimits {
    pub light: f32,
    pub heavy: f32,
    pub death: f32,
    pub damage_light: f32,
    pub damage_heavy: f32,
}

impl Default for FallLimits {
    fn default() -> Self {
        Self {
            light: 2.0,
            heavy: 3.0,
            death: 4.0,
            damage_light: 10.0,
            damage_heavy: 50.0,
        }
    }
}

/// The heroes' fall fields (templates `Jen`, `mattias`, `Chris`, `Hero`;
/// from samples).
pub const HERO_FALLS: FallLimits = FallLimits {
    light: 5.0,
    heavy: 15.0,
    death: 30.0,
    damage_light: 10.0,
    damage_heavy: 50.0,
};

/// The message a fall posts the person, which the human state table
/// turns into the `Upright` action of the same name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FallImpact {
    Light,
    Heavy,
}

impl FallImpact {
    /// The message's name, and the action's.
    pub fn name(self) -> &'static str {
        match self {
            Self::Light => "FallImpactLight",
            Self::Heavy => "FallImpactHeavy",
        }
    }
}

/// What a fall does: the damage, and the message if the person lives to
/// play it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fell {
    pub damage: f32,
    pub impact: Option<FallImpact>,
}

/// What a deadly fall costs: the program's float 1,000,000 (0x820F9604).
pub const FALL_DEATH_DAMAGE: f32 = 1_000_000.0;

/// What a fall of `distance` metres does to a person with `limits`.
///
/// From the 360 code (0x82205F20): nothing under `FallDistanceLight`;
/// under `FallDistanceHeavy`, `FallDamageLight` and the message
/// `FallImpactLight` (0x9E1B18F4); under `FallDistanceDeath`,
/// `FallDamageHeavy` and `FallImpactHeavy` (0x56482A61); from there on,
/// [`FALL_DEATH_DAMAGE`] and no message. (The function also skips the
/// message for a person whose action is already a ragdoll's, and skips
/// everything by a record of the person's that was not traced.)
pub fn fall_impact(limits: &FallLimits, distance: f32) -> Option<Fell> {
    if distance < limits.light {
        None
    } else if distance >= limits.death {
        Some(Fell {
            damage: FALL_DEATH_DAMAGE,
            impact: None,
        })
    } else if distance < limits.heavy {
        Some(Fell {
            damage: limits.damage_light,
            impact: Some(FallImpact::Light),
        })
    } else {
        Some(Fell {
            damage: limits.damage_heavy,
            impact: Some(FallImpact::Heavy),
        })
    }
}

/// The bounds the program keeps a template's `PhysicsPropertyGravityScaler`
/// `Scale` within: -10 and 10 (0x82DE8C30, 0x820BAEEC).
pub const GRAVITY_SCALE_LIMITS: (f32, f32) = (-10.0, 10.0);

/// The downward pull, m/s², on a person whose template scales gravity by
/// `scale` (the paratroopers' 0.2) when it would otherwise be `gravity`.
///
/// From the 360 code: the object physics setup (0x821831B0) looks the
/// object up in the `PhysicsPropertyGravityScaler` store (0x83651B64) and
/// gives its rigid bodies an action (vtable 0x820C8284) holding the scale,
/// clamped to [`GRAVITY_SCALE_LIMITS`]. That the walking capsule a person
/// falls as (`InAir.Fall`) is among those bodies is inferred, not read.
pub fn scaled_gravity(gravity: f32, scale: f32) -> f32 {
    gravity * scale.clamp(GRAVITY_SCALE_LIMITS.0, GRAVITY_SCALE_LIMITS.1)
}

/// How far a person fell: the height they left the ground at less the
/// height they came down at.
///
/// From the 360 code: the human physics' state step (0x8217C4A8) keeps
/// whether the character is in the air (its Havok state jumping or in
/// air). On leaving the ground it records its height (+0x180); on coming
/// back down it sends (0x8217C410) the recorded height less its height
/// now as the fall's distance (a physics event of kind 7, which
/// 0x82206128 hands to the fall impact). So a jump on level ground falls
/// no distance, whatever height it reaches.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct FallWatch {
    airborne: bool,
    left_at: f32,
}

impl FallWatch {
    /// One step: `airborne` now, at height `y`. Returns the distance
    /// fallen on the step the person comes down.
    pub fn step(&mut self, airborne: bool, y: f32) -> Option<f32> {
        if airborne == self.airborne {
            return None;
        }
        self.airborne = airborne;
        if airborne {
            self.left_at = y;
            None
        } else {
            Some(self.left_at - y)
        }
    }
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
            falls: FallLimits::default(),
        }
    }
}

/// The field name hashes [`HumanMotion`] is read from, in its order:
/// `JumpHeight`, `Gravity`, `MaxRotationSpeed`, `MaxAirSpeed`, the four
/// `Upright` paces, `CrouchWalkSpeed`, the capsule and the five fall
/// fields.
pub const FIELDS: [&str; 16] = [
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
    "FallDistanceLight",
    "FallDistanceHeavy",
    "FallDistanceDeath",
    "FallDamageLight",
    "FallDamageHeavy",
];

impl HumanMotion {
    /// From the fields in [`FIELDS`]' order.
    pub fn from_fields(f: [f32; 16]) -> Self {
        Self {
            jump_height: f[0],
            gravity: f[1],
            max_rotation: f[2],
            max_air_speed: f[3],
            paces: [f[4], f[5], f[6], f[7]],
            crouch_walk: f[8],
            capsule_radius: f[9],
            capsule_height: f[10],
            falls: FallLimits {
                light: f[11],
                heavy: f[12],
                death: f[13],
                damage_light: f[14],
                damage_heavy: f[15],
            },
        }
    }

    /// The fields in [`FIELDS`]' order.
    pub fn to_fields(&self) -> [f32; 16] {
        [
            self.jump_height,
            self.gravity,
            self.max_rotation,
            self.max_air_speed,
            self.paces[0],
            self.paces[1],
            self.paces[2],
            self.paces[3],
            self.crouch_walk,
            self.capsule_radius,
            self.capsule_height,
            self.falls.light,
            self.falls.heavy,
            self.falls.death,
            self.falls.damage_light,
            self.falls.damage_heavy,
        ]
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
            falls: HERO_FALLS,
        }
    }

    #[test]
    fn a_fall_hurts_by_how_far_it_was() {
        let hero = HERO_FALLS;
        assert_eq!(fall_impact(&hero, 4.9), None);
        let light = Fell {
            damage: 10.0,
            impact: Some(FallImpact::Light),
        };
        assert_eq!(fall_impact(&hero, 5.0), Some(light));
        assert_eq!(fall_impact(&hero, 14.9), Some(light));
        let heavy = Fell {
            damage: 50.0,
            impact: Some(FallImpact::Heavy),
        };
        assert_eq!(fall_impact(&hero, 15.0), Some(heavy));
        assert_eq!(fall_impact(&hero, 29.9), Some(heavy));
        let dead = Fell {
            damage: FALL_DEATH_DAMAGE,
            impact: None,
        };
        assert_eq!(fall_impact(&hero, 30.0), Some(dead));
        // A soldier's, the program's defaults: 2, 3 and 4 m.
        let soldier = FallLimits::default();
        assert_eq!(fall_impact(&soldier, 1.9), None);
        assert_eq!(fall_impact(&soldier, 2.5).map(|f| f.damage), Some(10.0));
        assert_eq!(fall_impact(&soldier, 3.5).map(|f| f.damage), Some(50.0));
        assert_eq!(fall_impact(&soldier, 4.0), Some(dead));
        // The messages' names hash to the program's words.
        assert_eq!(m2_assets::name_hash(FallImpact::Light.name()), 0x9e1b_18f4);
        assert_eq!(m2_assets::name_hash(FallImpact::Heavy.name()), 0x5648_2a61);
    }

    #[test]
    fn a_gravity_scale_is_kept_within_ten_either_way() {
        // The paratroopers' 0.2 of the program's 9.8.
        assert!((scaled_gravity(9.8, 0.2) - 1.96).abs() < 1e-5);
        assert_eq!(scaled_gravity(9.8, 1.0), 9.8);
        assert_eq!(scaled_gravity(1.0, 25.0), 10.0);
        assert_eq!(scaled_gravity(1.0, -25.0), -10.0);
    }

    #[test]
    fn a_paratrooper_dropped_high_takes_the_deadly_falls_damage() {
        // Dropped 100 m over the ground (`paradroplocation` flies its plane
        // that high), a soldier's limits make the fall deadly whatever the
        // pull: the distance is the height left less the height landed.
        let mut w = FallWatch::default();
        assert_eq!(w.step(true, 140.0), None);
        let fell = w.step(false, 40.0).unwrap();
        assert_eq!(
            fall_impact(&FallLimits::default(), fell).map(|f| f.damage),
            Some(FALL_DEATH_DAMAGE)
        );
    }

    #[test]
    fn a_fall_is_measured_from_where_the_ground_was_left() {
        let mut w = FallWatch::default();
        // On the ground: nothing.
        assert_eq!(w.step(false, 10.0), None);
        // A jump on the level: up and back to the same height falls 0.
        assert_eq!(w.step(true, 10.0), None);
        assert_eq!(w.step(true, 11.0), None);
        assert_eq!(w.step(false, 10.0), Some(0.0));
        // Off a ledge: the height left less the height reached.
        assert_eq!(w.step(true, 10.0), None);
        assert_eq!(w.step(true, 4.0), None);
        assert_eq!(w.step(false, 3.5), Some(6.5));
        // Up onto something higher: a negative distance, no fall.
        assert_eq!(w.step(true, 3.5), None);
        assert_eq!(w.step(false, 4.5), Some(-1.0));
        assert_eq!(fall_impact(&HERO_FALLS, -1.0), None);
    }

    #[test]
    fn the_fields_go_round() {
        let m = jennifer();
        assert_eq!(HumanMotion::from_fields(m.to_fields()), m);
        assert_eq!(FIELDS.len(), m.to_fields().len());
        assert_eq!(FIELDS[11], "FallDistanceLight");
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
/// Turning faster than the shuffle turns, the feet turn at this much of
/// the body's rate instead (0x82B4DB14).
pub const SHUFFLE_CATCH_UP: f32 = 1.1;
/// The wanted heading counts as still turning at under this much of the
/// shuffle's own rate (0x82003BCC); then the feet catch up only once far
/// behind the body: shuffling left, past 0.8 (0x82B4DB10) of 155 degrees
/// (0x82070294, scaled at 0x82A2A308) to its right; shuffling right,
/// past 0.8 of 90 degrees (the -90 at 0x820F8D04, scaled at 0x82A2A328)
/// to its left.
pub const SHUFFLE_CATCH_STILL: f32 = 0.25;
pub const SHUFFLE_BEHIND_LEFT: f32 = 0.8 * 155.0 * std::f32::consts::PI / 180.0;
pub const SHUFFLE_BEHIND_RIGHT: f32 = 0.8 * std::f32::consts::FRAC_PI_2;

/// A shuffle under way: which way, and seconds into its clip.
///
/// From the 360 code: the standing locomotion's step (0x82809D60) keeps
/// its own heading for the feet (+0x1584, the body's when the stance
/// starts, 0x828071A8) and their offset from the body's heading
/// (+0x1580). Aiming (state 0), past [`SHUFFLE_AT`] it plays a shuffle
/// clip (state 1 to the left, 2 to the right) that turns the feet by
/// [`SHUFFLE_LEFT`] or [`SHUFFLE_RIGHT`] over its length, and goes back
/// to state 0 when the clip ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shuffle {
    pub left: bool,
    pub time: f32,
}

impl Shuffle {
    /// Starts one when the feet (`feet`) are further than [`SHUFFLE_AT`]
    /// from the aim (`aim`), both headings: to the left when the aim is to
    /// their left. The player's: her aim is both the body's heading and
    /// the wanted one ([`begin`](Self::begin)).
    pub fn start(feet: f32, aim: f32) -> Option<Self> {
        Self::begin(wrap(feet - aim), 0.0)
    }

    /// The 360's test (0x82809D60, state 0): `feet` the feet's offset
    /// from the body's heading, `toward` the wanted heading's offset from
    /// it, radians, positive to the left. To the left when the feet are
    /// past [`SHUFFLE_AT`] to the body's right or the wanted heading past
    /// it to the body's left; to the right the other way round.
    pub fn begin(feet: f32, toward: f32) -> Option<Self> {
        let left = feet < -SHUFFLE_AT || toward > SHUFFLE_AT;
        let right = feet > SHUFFLE_AT || toward < -SHUFFLE_AT;
        (left || right).then_some(Self { left, time: 0.0 })
    }

    /// Steps on `dt` seconds of a clip `length` seconds long, the
    /// player's: her aim, turning at `aim_rate` rad/s, is both the body's
    /// heading and the wanted one, her feet `feet` off it ([`turn`](Self::turn)).
    pub fn step(&mut self, length: f32, aim_rate: f32, feet: f32, dt: f32) -> Option<(f32, f32)> {
        self.turn(length, aim_rate, aim_rate, feet, dt)
    }

    /// Steps on `dt` seconds of a clip `length` seconds long: the feet's
    /// turn this step (radians, positive to the left) and the clip's play
    /// rate, or `None` once the clip has played out (0x82809D60, states 1
    /// and 2). `body` and `toward` are how fast the body's heading and the
    /// wanted one turn (rad/s, positive to the left), `feet` the feet's
    /// offset from the body. With the body turning the shuffle's way
    /// faster than the clip turns, the feet take [`SHUFFLE_CATCH_UP`] of
    /// its rate, the clip played faster to match; with the wanted heading
    /// still ([`SHUFFLE_CATCH_STILL`]), only when the feet are far behind.
    pub fn turn(
        &mut self,
        length: f32,
        body: f32,
        toward: f32,
        feet: f32,
        dt: f32,
    ) -> Option<(f32, f32)> {
        if self.time >= length {
            return None;
        }
        let length = length.max(1e-4);
        let (base, faster, still, behind) = if self.left {
            let base = SHUFFLE_LEFT / length;
            (
                base,
                body > base,
                toward <= base * SHUFFLE_CATCH_STILL,
                feet < -SHUFFLE_BEHIND_LEFT,
            )
        } else {
            let base = -SHUFFLE_RIGHT / length;
            (
                base,
                body < base,
                base * SHUFFLE_CATCH_STILL <= toward,
                feet > SHUFFLE_BEHIND_RIGHT,
            )
        };
        let rate = if faster && (!still || behind) {
            body * SHUFFLE_CATCH_UP
        } else {
            base
        };
        let play = rate / base;
        self.time += dt * play;
        Some((rate * dt, play))
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
        while let Some((d, _)) = s.step(0.5, 0.0, 0.0, 1.0 / 60.0) {
            turned += d;
        }
        assert!(
            (turned.to_degrees() - 110.0).abs() < 4.0,
            "{}",
            turned.to_degrees()
        );
    }

    #[test]
    fn the_wanted_heading_past_sixty_degrees_starts_one_too() {
        let r = f32::to_radians;
        // Feet with the body, the wanted heading 70 degrees to its left.
        assert!(Shuffle::begin(0.0, r(70.0)).unwrap().left);
        assert!(!Shuffle::begin(0.0, r(-70.0)).unwrap().left);
        // Feet 65 degrees to the body's right: round to the left.
        assert!(Shuffle::begin(r(-65.0), 0.0).unwrap().left);
        assert_eq!(Shuffle::begin(r(50.0), r(-50.0)), None);
    }

    #[test]
    fn the_feet_catch_a_fast_body_unless_the_wanted_heading_is_still() {
        let dt = 1.0 / 60.0;
        // A 1 s clip turns 110 degrees a second to the left.
        let base = SHUFFLE_LEFT;
        let fast = base * 2.0;
        let mut s = Shuffle {
            left: true,
            time: 0.0,
        };
        // Body and wanted heading both turning fast: 1.1 of the body's.
        let (by, play) = s.turn(1.0, fast, fast, 0.0, dt).unwrap();
        assert!((by / dt - fast * SHUFFLE_CATCH_UP).abs() < 1e-3);
        assert!((play - 2.2).abs() < 1e-4);
        // The wanted heading still, feet near: the clip's own rate...
        let (by, _) = s.turn(1.0, fast, 0.0, 0.0, dt).unwrap();
        assert!((by / dt - base).abs() < 1e-4);
        // ...unless the feet are past 124 degrees behind.
        let (by, _) = s
            .turn(1.0, fast, 0.0, (-130.0f32).to_radians(), dt)
            .unwrap();
        assert!((by / dt - fast * SHUFFLE_CATCH_UP).abs() < 1e-3);
        // To the right, the body turning left doesn't speed it up.
        let mut s = Shuffle {
            left: false,
            time: 0.0,
        };
        let (by, play) = s.turn(1.0, fast, fast, 0.0, dt).unwrap();
        assert!((by / dt + SHUFFLE_RIGHT).abs() < 1e-4);
        assert!((play - 1.0).abs() < 1e-5);
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
/// In the air (past the grace, and from the start of a jump), Havok's
/// in-air state steers the same way towards the stick times the
/// template's `MaxAirSpeed`, with this gain a step and change capped at
/// [`AIR_MAX_ACCEL`] m/s² (research thread, 360 notes s16); the vertical
/// speed is left to gravity.
pub const AIR_GAIN: f32 = 0.07;
pub const AIR_MAX_ACCEL: f32 = 50.0;

/// The horizontal velocity after one Havok step of `dt` from `current`
/// towards `wanted` (0x8268DD08): the gap times [`GROUND_GAIN`], its
/// length capped at `max_change` ([`GROUND_MAX_ACCEL`] x `dt` on the
/// ground, [`LEDGE_MAX_CHANGE`] in the grace). Per step, so it depends on
/// the step's length as the 360's does.
pub fn ground_step(current: Vec2, wanted: Vec2, max_change: f32) -> Vec2 {
    gain_step(current, wanted, GROUND_GAIN, max_change)
}

/// [`ground_step`] with gain `gain`.
pub fn gain_step(current: Vec2, wanted: Vec2, gain: f32, max_change: f32) -> Vec2 {
    let d = wanted - current;
    let change = d * gain;
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
