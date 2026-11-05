//! How an AI boat fights her: the 360's boat driving helpers and its
//! `BoatAttack` task (0x822A9750).
//!
//! From the 360 code:
//! - **Drive to a point** (0x822A7C30), at up to `cap` m/s: the bearing to
//!   it off its heading. Its reference speed is the larger of `cap` and
//!   25; the speed asked no more than `cap` nor
//!   `0.1309 / (|bearing| + 0.1309)` x the reference. Steering
//!   `bearing / pi x 10`, throttle `(asked - speed) x 0.1`, each kept to
//!   -1..1.
//! - **Stop** (0x822A7730): steering 0, throttle the speed against it, kept
//!   to -1..1; stopped under 0.1 m/s.
//! - `BoatAttack`: on entering (0x822A94E8) its ring's base is its width
//!   plus a vehicle target's larger side, its spacing 3 x its width. It
//!   gives up on her unseen over 10 s. Its gun's riders fire as ever
//!   (0x822C95D8). State 0: while its timer runs it waits; then it picks a
//!   point (0x822A86B0): state 1, timer `2r + 3`, or none: it stops.
//!   State 1: it picks again each think and goes to the point at
//!   `max(1.2 x her speed, 70)` until within half its larger side, then
//!   stops.
//! - **The point** (0x822A86B0): her place a second on (0x822A7540), plus
//!   the way from there to the boat x (its length + its ring). The ring is
//!   its base + spacing; with boats on the same target it is one
//!   `0.75 x (their spacing + its own)` past the ring next in. No point
//!   when the ring is at or past its gun's `FireRange`, or the water
//!   there will not take it (0x822A8220).
//!
//! Inferred: 0x822A8220 is taken as open water at the point at least
//! 5 m deep; the timer counts down by the think. Not ported: the point on
//! the target itself for her on foot by the water (the claim, kind 2 of the
//! list at +0x11400, and state 3's run and state 4's evasion), the
//! candidates either side by the guns that bear (+0x1C bits 0x8000000 to
//! 0x40000000), aiming at her when stopped (0x822A7628), the avoidance
//! nudge (0x822A7858), the leash.

use m2_math::glam::Vec3;

use crate::car_attack::Sight;
use crate::car_driver::{CarState, wrap, yaw};

/// The boat's input as the alpha takes it: throttle (+ ahead), steering
/// (+ toward +x of its heading).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoatInput {
    pub throttle: f32,
    pub steer: f32,
}

fn bearing(car: &CarState, to: Vec3) -> f32 {
    wrap(yaw((to - car.at).with_y(0.0)) - yaw(car.forward))
}

/// 0x822A7C30: toward `to` at up to `cap` m/s.
pub fn drive_to(car: &CarState, to: Vec3, cap: f32) -> BoatInput {
    let b = bearing(car, to);
    let reference = cap.max(25.0);
    let asked = cap.min(0.130_899_7 / (b.abs() + 0.130_899_7) * reference);
    BoatInput {
        steer: (b / std::f32::consts::PI * 10.0).clamp(-1.0, 1.0),
        throttle: ((asked - car.speed()) * 0.1).clamp(-1.0, 1.0),
    }
}

/// 0x822A7730: stopping; `true` once stopped.
pub fn stop(car: &CarState) -> (BoatInput, bool) {
    let s = car.speed();
    (
        BoatInput {
            throttle: (-s).clamp(-1.0, 1.0),
            steer: 0.0,
        },
        s.abs() < 0.1,
    )
}

/// The boat and how it fights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hull {
    pub car: CarState,
    pub length: f32,
    pub width: f32,
    /// Its gun's `FireRange`, if it has one.
    pub range: Option<f32>,
    /// Which boat it is of those after her, 0 the first.
    pub rank: usize,
}

/// `BoatAttack`'s state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Choose,
    Going,
}

/// One boat driver's `BoatAttack` memory.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoatAttack {
    pub state: State,
    pub timer: f32,
}

/// 0x822A86B0's ring for a boat `width` wide, `rank` out, at a target of
/// larger side `target` if a vehicle.
pub fn ring(width: f32, target: f32, rank: usize) -> f32 {
    let spacing = width * 3.0;
    (width + target) + spacing + rank as f32 * 0.75 * (spacing + spacing)
}

impl BoatAttack {
    /// 0x822A86B0: where it goes for her, moving at `velocity`, whose
    /// larger side is `target` if a vehicle; `water` whether a point will
    /// take it.
    pub fn point(
        h: &Hull,
        her: &Sight,
        velocity: Vec3,
        target: f32,
        water: &mut impl FnMut(Vec3) -> bool,
    ) -> Option<Vec3> {
        let r = ring(h.width, target, h.rank);
        if h.range.is_some_and(|f| f <= r) {
            return None;
        }
        let ahead = her.at + velocity;
        let back = (h.car.at - ahead).with_y(0.0).normalize_or_zero();
        let p = ahead + back * (h.length + r);
        water(p).then_some(p)
    }

    /// One think at her.
    #[allow(clippy::too_many_arguments)]
    pub fn think(
        &mut self,
        h: &Hull,
        her: &Sight,
        velocity: Vec3,
        target: f32,
        water: &mut impl FnMut(Vec3) -> bool,
        rand: f32,
        dt: f32,
    ) -> BoatInput {
        self.timer -= dt;
        let car = &h.car;
        if her.unseen > 10.0 {
            return stop(car).0;
        }
        let point = Self::point(h, her, velocity, target, water);
        if self.state == State::Choose {
            if self.timer > 0.0 {
                return stop(car).0;
            }
            self.timer = rand * 2.0 + 3.0;
            if point.is_none() {
                return stop(car).0;
            }
            self.state = State::Going;
        }
        let Some(p) = point else {
            return stop(car).0;
        };
        let half = h.length.max(h.width) * 0.5;
        if (p - car.at).with_y(0.0).length_squared() <= half * half {
            return stop(car).0;
        }
        drive_to(car, p, (velocity.length() * 1.2).max(70.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hull(rank: usize) -> Hull {
        Hull {
            car: CarState {
                at: Vec3::ZERO,
                forward: Vec3::Z,
                velocity: Vec3::ZERO,
            },
            length: 12.0,
            width: 4.0,
            range: Some(120.0),
            rank,
        }
    }

    fn her(z: f32) -> Sight {
        Sight {
            at: Vec3::new(0.0, 0.0, z),
            unseen: 0.0,
            vehicle: false,
        }
    }

    #[test]
    fn its_ring_opens_with_rank() {
        assert_eq!(ring(4.0, 0.0, 0), 16.0);
        assert_eq!(ring(4.0, 0.0, 1), 34.0);
        assert_eq!(ring(4.0, 10.0, 0), 26.0);
    }

    #[test]
    fn it_goes_to_its_ring_short_of_her() {
        let mut a = BoatAttack::default();
        let mut water = |_: Vec3| true;
        let i = a.think(&hull(0), &her(100.0), Vec3::ZERO, 0.0, &mut water, 0.5, 0.1);
        assert_eq!(a.state, State::Going);
        assert_eq!(i.steer, 0.0);
        assert_eq!(i.throttle, 1.0);
        // 12 + 16 short of her.
        let p = BoatAttack::point(&hull(0), &her(100.0), Vec3::ZERO, 0.0, &mut water).unwrap();
        assert!((p.z - 72.0).abs() < 1e-4, "{p}");
    }

    #[test]
    fn with_no_water_or_out_of_range_it_stops() {
        let mut a = BoatAttack::default();
        let mut dry = |_: Vec3| false;
        let i = a.think(&hull(0), &her(100.0), Vec3::ZERO, 0.0, &mut dry, 0.5, 0.1);
        assert_eq!(a.state, State::Choose);
        assert_eq!(i, BoatInput::default());
        let mut h = hull(0);
        h.range = Some(10.0);
        let mut water = |_: Vec3| true;
        assert!(BoatAttack::point(&h, &her(100.0), Vec3::ZERO, 0.0, &mut water).is_none());
    }

    #[test]
    fn its_helpers() {
        let h = hull(0);
        assert!(drive_to(&h.car, Vec3::X * 50.0, 30.0).steer > 0.0);
        assert!(stop(&h.car).1);
    }
}
