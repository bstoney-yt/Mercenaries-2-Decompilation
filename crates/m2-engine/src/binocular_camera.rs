//! The camera she looks through her binoculars (and a gun's scope) with,
//! as the 360's (class 0x820EC8D8, 0x690 bytes).
//!
//! From the 360 code:
//! - **Up and down.** On foot, R3 (input bit 0x400000; the on-foot input
//!   0x824333E0) pushes the `Binoculars` player state (0x8243AC38, state
//!   hash 0xA0332C77). The state makes this camera (0x824429F8, which
//!   becomes the active one at 0x83457050), and R3 again leaves it
//!   (0x82442D00). The same state looks through a held gun's scope, with
//!   the scope's record (store 0x83652890) in place of the defaults.
//! - **Zoom** (+0x600). It is clamped to `min`..`max` (+0x638, +0x63C) and
//!   starts at `start`. The binoculars' values are fixed in the code:
//!   2× to 7×, starting at 2×, continuous, rate 1 ([`BINOCULARS`],
//!   0x82436700). A continuous zoom (+0x648) takes an axis (message
//!   0x816EA2AC, stick axes 0xB/0xC: the left stick's up-down, inferred
//!   from the on-foot input's use of them for moving) and each update
//!   multiplies the zoom by `rate × axis × dt + 1` (0x824BF3E0). A
//!   stepped zoom (message 0x4EADCF67, input bits 0x800 in and 0x1000
//!   out) sets a target `zoom × rate` (in) or `zoom ÷ rate` (out) and
//!   moves there at 4× the step a second (so in 0.25 s).
//! - **Lens.** The view's field is the camera's own (0.95975 rad,
//!   [`crate::projection::FOV`]) divided by the zoom (0x824BF700, +0x5D8).
//! - **Look.** The handler (0x824BF888) keeps the look-yaw input (message
//!   0x4BD465C7, the stick's x) at +0x608 and the look-pitch (0x574220AC)
//!   at +0x60C. Each update eases them through the three-keep inertia
//!   (0x824B9BD8; [`Inertia3`]) with keeps 0.55 (pushing on), 0 (a
//!   reversal starts from rest) and 0.05 (let go), set when it is made
//!   (0x824BEFC8). The yaw turns by −1/zoom × eased × dt and the pitch by
//!   +1/zoom × eased × dt (radians): at most 1/zoom rad/s.
//! - **Limits.** The pitch is held to −0.69575..0.90425 rad (up positive;
//!   +0x61C, +0x618). The yaw is held to 1.5 rad either side of the
//!   heading it came up at (+0x628: +0x62C is the offset turned so far,
//!   +0x620/+0x624 the two edges).
//! - **Where it looks from.** The body's place plus 2 m up (1 m when her
//!   controller's +0x414 is 1, inferred to be crouched) plus 0.2 m along
//!   the horizontal look it came up with (0x824BEFC8, +0x67C..+0x684).
//!
//! Not ported: the depth of field it sets from the distance to what is in
//! front of it (+0x5E0..+0x5EC), the external turn offsets (messages
//! 0x997CAD30 and 0xE34ED4B5, from the scope's sway, into +0x614 and
//! +0x610), and the turn added through the base camera's +0x400/+0x404.

use m2_math::glam::Vec3;

/// The engine's sixtieth of a second (0x820EB668).
const TICK: f32 = 1.0 / 60.0;

/// The view's pitch limits, radians up positive (0x820F9748, 0x820F974C).
pub const PITCH_LIMITS: (f32, f32) = (-0.695_75, 0.904_25);
/// How far the yaw may turn either side of where it came up, radians
/// (0x820BB6AC).
pub const YAW_LIMIT: f32 = 1.5;
/// The look's keeps: pushing on, reversing, let go (0x820F8E30, 0,
/// 0x820B8170).
pub const LOOK_KEEP: [f32; 3] = [0.55, 0.0, 0.05];
/// How high above her feet it looks from, metres: standing, crouched
/// (0x820C77A4 and 0x820BAEE8).
pub const EYE_HEIGHT: [f32; 2] = [2.0, 1.0];
/// How far along the look it starts, metres (0x820B8054).
pub const EYE_AHEAD: f32 = 0.2;
/// A stepped zoom covers its step in a quarter second (0x820B7B6C, 4).
pub const STEP_SPEED: f32 = 4.0;

/// A zoom's range and how it moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomParams {
    pub min: f32,
    pub max: f32,
    pub rate: f32,
    pub start: f32,
    pub continuous: bool,
}

/// The binoculars' (fixed in the code, 0x824366F8).
pub const BINOCULARS: ZoomParams = ZoomParams {
    min: 2.0,
    max: 7.0,
    rate: 1.0,
    start: 2.0,
    continuous: true,
};

/// `x` moved toward `target` for `dt` seconds, keeping `keep` of the gap
/// each sixtieth (0x824BB050).
fn approach(mut x: f32, target: f32, keep: f32, mut dt: f32) -> f32 {
    if dt > 16.666_668 {
        return if keep <= 0.9999 { target } else { x };
    }
    let a = 1.0 - keep;
    while dt > TICK {
        if (target - x) * (target - x) < (keep * 1e-5) * (keep * 1e-5) {
            return target;
        }
        dt -= TICK;
        x += (target - x) * a;
    }
    let f = dt * 59.999_996;
    ((target - x) * a + x) * f + (1.0 - f) * x
}

/// An axis's input inertia with three keeps (0x824B9BD8): pushing the
/// way it already goes the first, from rest or reversing the second
/// (0: the value restarts from 0 with the first), let go after let go
/// the third.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Inertia3 {
    pub value: f32,
    idle: bool,
}

impl Inertia3 {
    pub fn step(&mut self, input: f32, keep: &[f32; 3], dt: f32) {
        let none = input == 0.0;
        let k = if none && self.idle {
            keep[2]
        } else if self.value * input <= 0.0 {
            if keep[1] == 0.0 {
                self.value = 0.0;
                keep[0]
            } else {
                keep[1]
            }
        } else {
            keep[0]
        };
        self.value = approach(self.value, input, k, dt);
        self.idle = none;
    }
}

/// The binocular (or scope) camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BinocularCamera {
    pub params: ZoomParams,
    pub zoom: f32,
    /// A stepped zoom's part still to go, and its speed.
    pending: f32,
    step_speed: f32,
    /// The continuous zoom's axis.
    pub zoom_axis: f32,
    /// The view's heading (this crate's) and pitch (up positive), radians.
    pub yaw: f32,
    pub pitch: f32,
    /// The heading it came up at, and how far it has turned from it.
    pub entry_yaw: f32,
    turned: f32,
    /// The stick's look, and its eased values.
    pub input: [f32; 2],
    pub eased: [Inertia3; 2],
}

impl BinocularCamera {
    /// Up at `heading` and `pitch` (radians, up positive) with `params`.
    pub fn start(params: ZoomParams, heading: f32, pitch: f32) -> Self {
        Self {
            params,
            zoom: params.start.clamp(params.min, params.max),
            pending: 0.0,
            step_speed: 0.0,
            zoom_axis: 0.0,
            yaw: heading,
            pitch: pitch.clamp(PITCH_LIMITS.0, PITCH_LIMITS.1),
            entry_yaw: heading,
            turned: 0.0,
            input: [0.0; 2],
            eased: [Inertia3::default(); 2],
        }
    }

    /// A stepped zoom, in (`dir` > 0) or out, unless one is under way or
    /// the zoom is continuous (0x824BF888).
    pub fn zoom_step(&mut self, dir: f32) {
        if self.params.continuous || self.pending != 0.0 || dir == 0.0 {
            return;
        }
        let to = if dir > 0.0 {
            self.zoom * self.params.rate
        } else {
            self.zoom / self.params.rate
        };
        self.pending = to - self.zoom;
        self.step_speed = self.pending * STEP_SPEED;
    }

    /// A step of `dt` seconds (0x824BF3E0).
    pub fn step(&mut self, dt: f32) {
        self.eased[0].step(self.input[0], &LOOK_KEEP, dt);
        self.eased[1].step(self.input[1], &LOOK_KEEP, dt);
        let pitch_turn = self.eased[1].value / self.zoom * dt;
        let yaw_turn = -1.0 / self.zoom * self.eased[0].value * dt;
        if pitch_turn != 0.0 {
            self.pitch = (self.pitch + pitch_turn).clamp(PITCH_LIMITS.0, PITCH_LIMITS.1);
        }
        if yaw_turn != 0.0 {
            self.turned += yaw_turn;
            if self.turned.abs() <= YAW_LIMIT {
                self.yaw += yaw_turn;
            } else {
                self.turned = YAW_LIMIT.copysign(self.turned);
                self.yaw = wrap(self.entry_yaw + self.turned);
            }
        }
        if self.params.continuous {
            self.zoom *= self.params.rate * self.zoom_axis * dt + 1.0;
        } else if self.pending != 0.0 {
            let by = self.step_speed * dt;
            let by = if by.abs() < self.pending.abs() {
                self.pending -= by;
                by
            } else {
                std::mem::take(&mut self.pending)
            };
            self.zoom += by;
        }
        self.zoom = self.zoom.clamp(self.params.min, self.params.max);
    }

    /// The lens: the camera's field of view `fov` (radians, the game's)
    /// over the zoom.
    pub fn fov(&self, fov: f32) -> f32 {
        fov / self.zoom.max(1e-4)
    }

    /// Which way it looks.
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, cy * cp)
    }

    /// Where it looks from, for her feet at `feet`, crouched or not.
    pub fn eye(&self, feet: Vec3, crouched: bool) -> Vec3 {
        let (s, c) = self.entry_yaw.sin_cos();
        feet + Vec3::Y * EYE_HEIGHT[usize::from(crouched)] + Vec3::new(s, 0.0, c) * EYE_AHEAD
    }
}

fn wrap(a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    (a + PI).rem_euclid(TAU) - PI
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_binoculars_zoom_two_to_seven_continuously() {
        let mut c = BinocularCamera::start(BINOCULARS, 0.0, 0.0);
        assert_eq!(c.zoom, 2.0);
        // Lens: the camera's 0.95975 rad over 2.
        assert!((c.fov(crate::projection::FOV) - 0.479_875).abs() < 1e-5);
        // Full in for a second: each sixtieth × (1 + 1/60), about e.
        c.zoom_axis = 1.0;
        for _ in 0..60 {
            c.step(TICK);
        }
        assert!(
            (c.zoom - 2.0 * (1.0f32 + TICK).powi(60)).abs() < 1e-3,
            "{}",
            c.zoom
        );
        for _ in 0..600 {
            c.step(TICK);
        }
        assert_eq!(c.zoom, 7.0);
        c.zoom_axis = -1.0;
        for _ in 0..600 {
            c.step(TICK);
        }
        assert_eq!(c.zoom, 2.0);
        // A step does nothing to a continuous zoom.
        c.zoom_step(1.0);
        c.zoom_axis = 0.0;
        c.step(TICK);
        assert_eq!(c.zoom, 2.0);
    }

    #[test]
    fn a_stepped_zoom_gets_there_in_a_quarter_second() {
        let scope = ZoomParams {
            min: 2.0,
            max: 8.0,
            rate: 2.0,
            start: 2.0,
            continuous: false,
        };
        let mut c = BinocularCamera::start(scope, 0.0, 0.0);
        c.zoom_step(1.0);
        // A quarter second, and the last sliver of the step.
        for _ in 0..16 {
            c.step(TICK);
        }
        assert!((c.zoom - 4.0).abs() < 1e-4, "{}", c.zoom);
        // Another step waits for this one to end; this one has.
        c.zoom_step(-1.0);
        for _ in 0..30 {
            c.step(TICK);
        }
        assert!((c.zoom - 2.0).abs() < 1e-4);
    }

    #[test]
    fn the_look_is_slowed_by_the_zoom_and_held_to_its_limits() {
        let mut c = BinocularCamera::start(BINOCULARS, 0.0, 0.0);
        // Full right for long: 1/2 rad/s down the heading at 2x.
        c.input = [1.0, 0.0];
        for _ in 0..120 {
            c.step(TICK);
        }
        let before = c.yaw;
        c.step(TICK);
        assert!(((before - c.yaw) / TICK - 0.5).abs() < 1e-3);
        // On to its edge, 1.5 rad round, and held there.
        for _ in 0..600 {
            c.step(TICK);
        }
        assert!((c.yaw + YAW_LIMIT).abs() < 1e-5, "{}", c.yaw);
        // Up: held at 0.90425.
        c.input = [0.0, 1.0];
        for _ in 0..600 {
            c.step(TICK);
        }
        assert!((c.pitch - PITCH_LIMITS.1).abs() < 1e-6);
        // Let go: the eased look dies away fast (keep 0.05 a sixtieth).
        c.input = [0.0; 2];
        c.step(TICK);
        c.step(TICK);
        assert!(c.eased[1].value < 0.01);
    }

    #[test]
    fn it_looks_from_two_metres_up_and_a_little_ahead() {
        let c = BinocularCamera::start(BINOCULARS, 0.0, 0.0);
        assert_eq!(c.eye(Vec3::ZERO, false), Vec3::new(0.0, 2.0, 0.2));
        assert_eq!(c.eye(Vec3::ZERO, true), Vec3::new(0.0, 1.0, 0.2));
    }
}
