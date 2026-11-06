//! How an AI boat fights her: the 360's boat driving helpers and its
//! `BoatAttack` task (0x822A9750).
//!
//! From the 360 code:
//! - **Drive to a point** (0x822A7C30), at up to `cap` m/s: the bearing to
//!   it off its heading. Its reference speed is the larger of `cap` and
//!   25; the speed asked no more than `cap` nor
//!   `0.1309 / (|bearing| + 0.1309)` x the reference, and when asked to
//!   slow for the point nor `1 - 64 / (d^2 + 64)` x the reference.
//!   Steering `bearing / pi x 10`, throttle `(asked - speed) x 0.1`, each
//!   kept to -1..1.
//! - **Keeping clear** (0x822A7858, inside the drive): what stands within
//!   50 m of the point 50 m ahead (0x822ED248: object kinds 6, buildings,
//!   `_BuildingPhysics`, and 10, boats, the class that reads the boat
//!   record store), at most 32 of them. Each that is not behind it and
//!   that it will pass within `its radius + half the boat's larger side`
//!   of, at the moment they come closest (0x822FDE50, level, on their
//!   velocities; a boat barely moving counts as moving its own length a
//!   second), pushes the course away from where it will be then. Each
//!   push is at most 1, they are averaged, added to the way to the point
//!   and made unit: the boat then steers for 100 m along that.
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
//!   when the ring is at or past its gun's `FireRange`, or something
//!   stands in the way there (0x822A8220: a box as wide and tall as the
//!   boat, 2 cm long, swept from its centre to the point at its own height;
//!   what it meets counts by object kind, buildings and boats among them).
//!
//! Inferred: the water asked of the point (open, at least 5 m deep): the
//! 360's sweep does not say what it makes of land, and the alpha's leaves
//! the terrain out. The timer counts down by the think. A neighbour's
//! sphere is taken as its bounds' (0x82855640 not read). Not ported:
//! buildings among the neighbours (the alpha does not list them apart),
//! the point on
//! the target itself for her on foot by the water (the claim, kind 2 of the
//! list at +0x11400, and state 3's run and state 4's evasion), the
//! candidates either side by the guns that bear (+0x1C bits 0x8000000 to
//! 0x40000000), aiming at her when stopped (0x822A7628), the leash.

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

/// Something a boat keeps clear of (0x822ED248's record).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Neighbour {
    pub at: Vec3,
    pub radius: f32,
    pub velocity: Vec3,
}

/// 0x822A7858: the push off `others`, for a hull `length` long whose
/// larger side is `size`. Zero when nothing is in its way.
pub fn keep_clear(car: &CarState, length: f32, size: f32, others: &[Neighbour]) -> Vec3 {
    let v = if car.velocity.length_squared() <= 1.0 {
        car.forward * length
    } else {
        car.velocity
    };
    let half = size * 0.5;
    let ahead = car.at + car.forward * 50.0;
    let (mut push, mut count) = (Vec3::ZERO, 0);
    let near = others
        .iter()
        .filter(|n| (n.at - ahead).with_y(0.0).length_squared() <= 2500.0)
        .take(32);
    for n in near {
        let there = n.at.with_y(car.at.y);
        if car.forward.dot(there - car.at) < 0.0 {
            continue;
        }
        let closing = v - n.velocity;
        let c2 = closing.length_squared();
        if c2 < 1e-4 {
            continue;
        }
        let t = -(car.at - there).dot(closing) / c2;
        if t <= 0.0 {
            continue;
        }
        let reach = n.radius + half;
        let mut miss = car.at + v * t - (there + n.velocity * t);
        if miss.with_y(0.0).length_squared() > reach * reach {
            continue;
        }
        let d = miss.length();
        if d > 1e-4 {
            miss /= d;
        }
        count += 1;
        push += miss * ((reach * 2.0 - d).abs() / reach).clamp(0.0, 1.0);
    }
    if count > 0 { push / count as f32 } else { push }
}

/// 0x822A7C30: toward `to` at up to `cap` m/s, `slowing` for it, turned
/// by `push` (from [`keep_clear`]).
pub fn drive_to(car: &CarState, to: Vec3, cap: f32, slowing: bool, push: Vec3) -> BoatInput {
    let to = if push != Vec3::ZERO {
        let way = (to - car.at).normalize_or_zero();
        car.at + (way + push).normalize_or_zero() * 100.0
    } else {
        to
    };
    let b = bearing(car, to);
    let reference = cap.max(25.0);
    let share = if slowing {
        let d2 = (to - car.at).length_squared();
        1.0 - 64.0 / (d2 + 64.0)
    } else {
        1.0
    };
    let asked = cap
        .min(0.130_899_7 / (b.abs() + 0.130_899_7) * reference)
        .min(share * reference);
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
    pub height: f32,
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
        others: &[Neighbour],
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
        let push = keep_clear(car, h.length, h.length.max(h.width), others);
        drive_to(car, p, (velocity.length() * 1.2).max(70.0), false, push)
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
            height: 3.0,
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
        let i = a.think(
            &hull(0),
            &her(100.0),
            Vec3::ZERO,
            0.0,
            &mut water,
            &[],
            0.5,
            0.1,
        );
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
        let i = a.think(
            &hull(0),
            &her(100.0),
            Vec3::ZERO,
            0.0,
            &mut dry,
            &[],
            0.5,
            0.1,
        );
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
        assert!(drive_to(&h.car, Vec3::X * 50.0, 30.0, false, Vec3::ZERO).steer > 0.0);
        assert!(stop(&h.car).1);
    }

    #[test]
    fn slowing_it_eases_off_near_the_point() {
        let mut car = hull(0).car;
        car.velocity = Vec3::Z * 20.0;
        // 8 m off, half the reference speed of 30: 15 m/s.
        let i = drive_to(&car, Vec3::Z * 8.0, 30.0, true, Vec3::ZERO);
        assert!((i.throttle + 0.5).abs() < 1e-3, "{}", i.throttle);
    }

    fn moored(at: Vec3) -> Neighbour {
        Neighbour {
            at,
            radius: 6.0,
            velocity: Vec3::ZERO,
        }
    }

    #[test]
    fn it_turns_off_what_lies_in_its_way() {
        let mut car = hull(0).car;
        car.velocity = Vec3::Z * 20.0;
        let push = keep_clear(&car, 12.0, 12.0, &[moored(Vec3::new(2.0, 0.0, 60.0))]);
        assert!(push.x < -0.9, "{push}");
        let i = drive_to(&car, Vec3::Z * 200.0, 30.0, false, push);
        assert!(i.steer < 0.0, "{}", i.steer);
    }

    #[test]
    fn it_leaves_alone_what_it_will_miss_or_has_passed() {
        let mut car = hull(0).car;
        car.velocity = Vec3::Z * 20.0;
        // Past the reach of 6 + 6.
        let wide = moored(Vec3::new(13.0, 0.0, 60.0));
        // Astern of it.
        let astern = moored(Vec3::new(2.0, 0.0, -10.0));
        // Further than 50 m from the point ahead.
        let far = moored(Vec3::new(2.0, 0.0, 120.0));
        for n in [wide, astern, far] {
            assert_eq!(keep_clear(&car, 12.0, 12.0, &[n]), Vec3::ZERO);
        }
    }

    #[test]
    fn stopped_it_looks_its_length_ahead_and_ignores_itself() {
        let car = hull(0).car;
        let me = Neighbour {
            at: car.at,
            radius: 6.0,
            velocity: Vec3::ZERO,
        };
        assert_eq!(keep_clear(&car, 12.0, 12.0, &[me]), Vec3::ZERO);
        let n = moored(Vec3::new(2.0, 0.0, 40.0));
        assert!(keep_clear(&car, 12.0, 12.0, &[n]).x < 0.0);
    }
}
