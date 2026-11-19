//! The camera that looks at her while she lies limp (knocked down or
//! dead), as the 360's.
//!
//! From the 360 code:
//! - Her `Human` player state (0x82443718) makes two cameras: the on-foot
//!   chase camera (class 0x820F0290) and this one (class 0x820EC978,
//!   0x620 bytes). Each frame it hands on the second while her animation
//!   state (+0x1C, its +0x24) is `FULL` (0x4294EBA2), the state that has
//!   her body go limp (0x824439A0).
//! - Taking over (0x824C6EC0), it starts from the camera before it:
//!   its yaw (+0x614) that camera's heading, its pitch (+0x610) 45°
//!   (0.0174533 × 45) down, its inputs and their eased values zero.
//! - Its handler (0x824C7118) keeps the look-yaw input (message
//!   0x4BD465C7, the stick's x) negated at +0x600 and the look-pitch
//!   (0x574220AC, its y) at +0x604.
//! - Each update (0x824C6F78) eases each input (to +0x608, +0x60C) with
//!   its own approach (0x824C7168: 0.025 of the gap a sixtieth of a
//!   second, [`ease`]). The view's yaw is the start's plus 25° times the
//!   eased yaw, its pitch the start's less 25° times the eased pitch,
//!   held between 5° and 85° down. It looks at her body's place (the
//!   entity's transform, 0x823F8370) from 4 m back along that view.
//!   So the stick swings the view at most 25° either way, and it drifts
//!   back once the stick is let go.
//!
//! Inferred: the yaw is this crate's heading (radians, `atan2(x, z)` of
//! the forward); the stick's +x turns the heading down, as the other
//! cameras' (the negation at +0x600). The point looked at is her body's
//! first node (the 360 reads the entity's transform, which the limp body
//! carries).

use m2_math::glam::Vec3;

/// The engine's sixtieth of a second (0x820EB668).
const TICK: f32 = 1.0 / 60.0;
/// The share of the gap closed each sixtieth (0x820F8BC0).
const GAIN: f32 = 0.025;
/// Over this long a step jumps straight to the target (0x820F8BB4).
const SNAP: f32 = 16.666_668;
/// Below this squared gap it is there (0x820F8BBC).
const THERE: f32 = 9.506_25e-11;

/// The first pitch, radians down (45°, 0x82B7C5EC).
pub const START_PITCH: f32 = 45.0 * DEG;
/// How far the stick swings the view, radians (25°, 0x820F02E4).
pub const SWING: f32 = 25.0 * DEG;
/// The pitch's limits, radians down (5° and 85°, 0x820BAF84, 0x820F9744).
pub const PITCH_LIMITS: (f32, f32) = (5.0 * DEG, 85.0 * DEG);
/// How far back from her body it looks, metres (4.0, 0x820B7B6C).
pub const DISTANCE: f32 = 4.0;

/// The 360's degree (0x82DE6E64).
const DEG: f32 = 0.017_453_292;

/// `x` moved toward `target` for `dt` seconds (0x824C7168): a step of
/// [`GAIN`] of the gap each sixtieth, the last part of a sixtieth by its
/// share; a step longer than [`SNAP`] seconds lands on the target.
pub fn ease(mut x: f32, target: f32, mut dt: f32) -> f32 {
    if dt > SNAP {
        return target;
    }
    while dt > TICK {
        let gap = target - x;
        if gap * gap < THERE {
            return target;
        }
        x += gap * GAIN;
        dt -= TICK;
    }
    let part = dt * 60.0;
    let stepped = (target - x) * GAIN + x;
    stepped * part + (1.0 - part) * x
}

/// The limp body's camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RagdollCamera {
    /// Its start: the heading (radians) and the pitch (radians down).
    pub yaw: f32,
    pub pitch: f32,
    /// The stick's inputs as the handler keeps them (yaw negated).
    pub input: [f32; 2],
    /// Their eased values.
    pub eased: [f32; 2],
}

impl RagdollCamera {
    /// Taking over from a camera facing `heading` (0x824C6EC0).
    pub fn start(heading: f32) -> Self {
        Self {
            yaw: heading,
            pitch: START_PITCH,
            input: [0.0; 2],
            eased: [0.0; 2],
        }
    }

    /// The stick (-1..1 each way, after its dead zone).
    pub fn look(&mut self, stick: [f32; 2]) {
        self.input = [-stick[0], stick[1]];
    }

    /// A step of `dt` seconds: its heading and pitch down now.
    pub fn step(&mut self, dt: f32) -> (f32, f32) {
        self.eased[0] = ease(self.eased[0], self.input[0], dt);
        self.eased[1] = ease(self.eased[1], self.input[1], dt);
        self.angles()
    }

    /// Its heading and pitch down now.
    pub fn angles(&self) -> (f32, f32) {
        let pitch = (self.pitch - self.eased[1] * SWING).clamp(PITCH_LIMITS.0, PITCH_LIMITS.1);
        (self.yaw + self.eased[0] * SWING, pitch)
    }

    /// Where it is and which way it looks, at a body at `at`.
    pub fn view(&self, at: Vec3) -> (Vec3, Vec3) {
        let (yaw, pitch) = self.angles();
        let forward = Vec3::new(
            yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        );
        (at - forward * DISTANCE, forward)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn its_ease_closes_a_fortieth_of_the_gap_each_sixtieth() {
        assert!((ease(0.0, 1.0, TICK) - GAIN).abs() < 1e-6);
        // Half a sixtieth: half a step.
        assert!((ease(0.0, 1.0, TICK / 2.0) - GAIN / 2.0).abs() < 1e-6);
        // A second: 1 - 0.975^60 = 0.78 of the way.
        let s = ease(0.0, 1.0, 1.0);
        assert!((s - (1.0 - 0.975f32.powi(60))).abs() < 0.03, "{s}");
        // A very long step lands on it.
        assert_eq!(ease(0.0, 1.0, 20.0), 1.0);
    }

    #[test]
    fn it_looks_down_at_her_from_four_metres_and_the_stick_swings_it_25_degrees() {
        let mut c = RagdollCamera::start(0.0);
        let (yaw, pitch) = c.step(TICK);
        assert_eq!(yaw, 0.0);
        assert!((pitch - 45f32.to_radians()).abs() < 1e-5);
        let (eye, f) = c.view(Vec3::ZERO);
        assert!((eye.length() - DISTANCE).abs() < 1e-5);
        assert!(eye.y > 0.0 && eye.z < 0.0, "{eye}");
        assert!(f.y < 0.0);
        // Full stick right for long: the heading 25° down (the input
        // negated); full up: the pitch 25° less, to 20°.
        c.look([1.0, 1.0]);
        for _ in 0..600 {
            c.step(TICK);
        }
        let (yaw, pitch) = c.angles();
        assert!((yaw + 25f32.to_radians()).abs() < 1e-3, "{yaw}");
        assert!((pitch - 20f32.to_radians()).abs() < 1e-3, "{pitch}");
        // Let go: it drifts back.
        c.look([0.0, 0.0]);
        for _ in 0..600 {
            c.step(TICK);
        }
        assert!(c.angles().0.abs() < 1e-3);
        // Held to 5°..85° down: from a start of 82°, full down stops at 85°.
        let mut low = RagdollCamera {
            pitch: 82f32.to_radians(),
            ..RagdollCamera::start(0.0)
        };
        low.look([0.0, -1.0]);
        for _ in 0..600 {
            low.step(TICK);
        }
        assert!((low.angles().1 - 85f32.to_radians()).abs() < 1e-4);
    }
}
