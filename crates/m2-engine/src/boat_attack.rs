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
//!   (0x822C95D8).
//!   - State 0: while its timer runs it does nothing; then it picks a
//!     point (0x822A86B0): state 1, or 3 for a run at her, timer `2r + 3`;
//!     or none: it stops.
//!   - State 1: it picks again each think and goes to the point at
//!     `max(1.2 x her speed, 70)` until within half its larger side, then
//!     stops, turning to her (0x822A7628) if the pick said so.
//!   - State 3: it goes at 30 m/s while the pick is still a run. If not it
//!     looks 35 either way of its heading (the 360 adds 35 to radians, kept
//!     here), as far as it would go in 3 s, takes the first clear way and
//!     is in state 4: steering full that way at half throttle for 3 s,
//!     then state 0.
//! - **The point** (0x822A86B0). The ring is its base + spacing; with boats
//!   on the same target it is one `0.75 x (their spacing + its own)` past
//!   the ring next in. No point when the ring is at or past its gun's
//!   `FireRange`.
//!   - With her out of a vehicle and no other boat holding her (the list
//!     at +0x11400, kind 2): a run at her, to the point 2 lengths past her.
//!     A boat holding the run, within 3 lengths of that goes for 3 lengths
//!     ahead of itself. It takes the run if that point is not astern of it,
//!     a sweep to it is clear, she swims, and it is within 45 degrees of
//!     her facing.
//!   - Otherwise her place a second on (0x822A7540), and from there the way
//!     back to the boat. Once a second it asks whether anything solid is
//!     within 1.5 m under her (0x822A8540). If so, she is ashore: the
//!     point that way at its length + its ring, or where the sweep to that
//!     met something, slowing, then turning to her.
//!   - Over open water, the guns that bear decide (0x822EEA58): an eighth
//!     of a turn either side of the way back, at the ring, the side she is
//!     on first, for guns on that side; her length + ring behind her for a
//!     gun ahead (then turning to her), before her for one astern. These
//!     are led 2 s by her velocity; the first clear one is taken. Then
//!     before her, then behind her, for an end with no gun, then the
//!     ring's point, slowing. With no gun at all: behind her, slowing.
//! - **The sweep** (0x822A8220): a box as wide and tall as the boat, 2 cm
//!   long, from its centre to the point at its own height, over the ground
//!   too (object kind 0xD).
//! - **Her object kinds**: 6 `_BuildingPhysics`, 10 boats (the class that
//!   reads the boat record store 0x83651C60), 0xD the ground.
//!
//! Inferred: the timer counts down by the think; a neighbour's sphere is
//! its bounds' (0x82855640 not read); +0x3F8 bit 0x10, her swimming (the
//! human's +0x170 of 4 or 5); +0x58 is left 0 (no writer found); the run
//! is let go when it fails (the 360 keeps the claim 2 s). A sweep's kinds
//! are not told apart: all it meets counts as blocking, where the 360 lets
//! a candidate past some (buildings, boats). The cone looks along her
//! facing (0x822EC800 reads her 0x83657FA8 record).
//! Not ported: buildings among the neighbours (the alpha does not list
//! them apart), the leash, a target in a vehicle of sub-kinds 0x180 and
//! 0x300 (no point: not known which those are), the flip of the aim
//! (+0x3F8 bit 0x40, no writer found).

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

/// 0x822A7628: stopped, turning to `to`.
pub fn aim(car: &CarState, to: Vec3) -> f32 {
    bearing(car, to).clamp(-1.0, 1.0)
}

/// Which way a boat's gunners' guns bear (0x822EEA58: mind +0x1C, bits
/// 0x40000000, 0x20000000, 0x8000000, 0x10000000).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bears {
    pub ahead: bool,
    pub astern: bool,
    /// Toward its +x and -x.
    pub plus: bool,
    pub minus: bool,
}

impl Bears {
    /// Adds a gunner's turret facing `facing` in the boat's own frame,
    /// turning `yaw` either way of that, radians: a side counts when the
    /// gun looks within 0.9 of it, or a gun ahead reaches a quarter turn
    /// that way.
    pub fn add(&mut self, facing: Vec3, yaw: (f32, f32)) {
        let quarter = std::f32::consts::FRAC_PI_2;
        if facing.z > 0.9 {
            self.ahead = true;
            self.minus |= yaw.0 <= -quarter;
            self.plus |= yaw.1 >= quarter;
        }
        self.astern |= -facing.z > 0.9;
        self.minus |= -facing.x > 0.9;
        self.plus |= facing.x > 0.9;
    }
}

/// The boat and how it fights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hull {
    /// Whose it is: who has claimed a run at her.
    pub id: u32,
    pub car: CarState,
    pub length: f32,
    pub width: f32,
    pub height: f32,
    /// Its bounds' stern, along its heading from its place (negative).
    pub stern: f32,
    /// Its gun's `FireRange`, if it has one.
    pub range: Option<f32>,
    /// Which boat it is of those after her, 0 the first.
    pub rank: usize,
    pub bears: Bears,
}

/// Who it is after.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub sight: Sight,
    pub velocity: Vec3,
    /// The way she faces.
    pub forward: Vec3,
    /// Her vehicle's larger side, if she is in one.
    pub size: f32,
    pub swimming: bool,
}

/// What 0x822A8220's sweep to a point met.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sweep {
    pub blocked: bool,
    /// Where it stopped.
    pub met: Vec3,
}

/// What `BoatAttack` asks of the world.
pub trait BoatWorld {
    /// 0x822A8220: the boat's face swept to `to` at its own height.
    fn sweep(&mut self, to: Vec3) -> Sweep;
    /// 0x822A8540: nothing solid under her down to `depth` below her.
    fn open_under(&mut self, her: Vec3, depth: f32) -> bool;
}

/// `BoatAttack`'s state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Choose,
    Going,
    /// 3: the run at her.
    Ram,
    /// 4: turning off, after one.
    Evade,
}

/// One boat driver's `BoatAttack` memory.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoatAttack {
    pub state: State,
    pub timer: f32,
    /// +0x3F8's top bit, and how long until it is asked again (+0x3F4).
    open: bool,
    open_timer: f32,
    /// The way it turns off, and for how long (+0x3F0, +0x3EC).
    turn: f32,
    hold: f32,
}

/// 0x822A86B0's ring for a boat `width` wide, `rank` out, at a target of
/// larger side `target` if a vehicle.
pub fn ring(width: f32, target: f32, rank: usize) -> f32 {
    let spacing = width * 3.0;
    (width + target) + spacing + rank as f32 * 0.75 * (spacing + spacing)
}

/// What 0x822A86B0 chose.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pick {
    pub at: Vec3,
    /// Its `slowing` for [`drive_to`].
    pub slowing: bool,
    /// A run at her.
    pub ram: bool,
    /// Once there, turn to her.
    pub aim: bool,
}

impl Pick {
    fn at(at: Vec3) -> Self {
        Self {
            at,
            ..Self::default()
        }
    }
}

/// `v` turned level by `a` radians (0x822A86B0's sine table, 8192 to the
/// turn: `x cos - z sin`, `z cos + x sin`).
fn turned(v: Vec3, a: f32) -> Vec3 {
    let (s, c) = a.sin_cos();
    Vec3::new(c * v.x - s * v.z, v.y, c * v.z + s * v.x)
}

impl BoatAttack {
    /// 0x822A86B0: where it goes for her. `claim` is who has the run at
    /// her.
    pub fn point(
        &mut self,
        h: &Hull,
        her: &Target,
        world: &mut impl BoatWorld,
        claim: &mut Option<u32>,
    ) -> Option<Pick> {
        let car = &h.car;
        let at_her = her.sight.at;
        if !her.sight.vehicle && claim.is_none_or(|c| c == h.id) {
            let way = (at_her - car.at).normalize_or_zero();
            let mut p = at_her + way * h.length * 2.0;
            let stern = car.at + car.forward * h.stern;
            let behind = (p - stern).dot(-car.forward) > 0.0;
            let ram = Pick {
                ram: true,
                ..Pick::at(p)
            };
            if *claim == Some(h.id) {
                let three = h.length * 3.0;
                if (p - car.at).length_squared() < three * three {
                    p = car.at + car.forward * three;
                }
                return (!behind && !world.sweep(p).blocked).then_some(Pick { at: p, ..ram });
            }
            let cone =
                her.forward.dot((car.at - at_her).normalize_or_zero()) >= 0.785_249_95_f32.cos();
            if her.swimming && cone && !behind && !world.sweep(p).blocked {
                *claim = Some(h.id);
                return Some(ram);
            }
        }
        if *claim == Some(h.id) {
            *claim = None;
        }
        let r = ring(h.width, her.size, h.rank);
        if h.range.is_some_and(|f| f <= r) {
            return None;
        }
        let ahead = at_her + her.velocity;
        let to_her = ahead - car.at;
        let back = -to_her.with_y(0.0).normalize_or_zero();
        let far = h.length + r;
        let plain = ahead + back * far;
        if self.open_timer <= 0.0 {
            self.open_timer = 1.0;
            self.open = world.open_under(at_her, 1.5);
        }
        if !self.open {
            let s = world.sweep(plain);
            return Some(Pick {
                at: if s.blocked { s.met } else { plain },
                slowing: true,
                aim: true,
                ram: false,
            });
        }
        // Either side of the way to her, the side she is on first, as its
        // guns bear; then behind and before her, the way she faces.
        let side = Vec3::Y.cross(car.forward).dot(to_her);
        let eighth = 0.785_249_95;
        let plus = (h.bears.plus, ahead + turned(back * r, eighth));
        let minus = (h.bears.minus, ahead + turned(back * r, -eighth));
        let sides = if side < 0.0 {
            [plus, minus]
        } else {
            [minus, plus]
        };
        let f = her.forward * far;
        let (front, rear) = (ahead + f, ahead - f);
        let mut tries: Vec<(Vec3, bool)> =
            sides.iter().filter(|s| s.0).map(|s| (s.1, false)).collect();
        if h.bears.ahead {
            tries.push((rear, true));
        }
        if h.bears.astern {
            tries.push((front, false));
        }
        if tries.is_empty() {
            let s = world.sweep(rear);
            return (!s.blocked).then_some(Pick {
                slowing: true,
                ..Pick::at(rear)
            });
        }
        for (p, aim) in tries {
            let p = p + her.velocity * 2.0;
            if !world.sweep(p).blocked {
                return Some(Pick { aim, ..Pick::at(p) });
            }
        }
        if !h.bears.ahead && !world.sweep(front).blocked {
            return Some(Pick {
                aim: true,
                ..Pick::at(front)
            });
        }
        if !h.bears.astern && !world.sweep(rear).blocked {
            return Some(Pick::at(rear));
        }
        (!world.sweep(plain).blocked).then_some(Pick {
            slowing: true,
            ..Pick::at(plain)
        })
    }

    /// One think at her.
    #[allow(clippy::too_many_arguments)]
    pub fn think(
        &mut self,
        h: &Hull,
        her: &Target,
        world: &mut impl BoatWorld,
        others: &[Neighbour],
        claim: &mut Option<u32>,
        rand: f32,
        dt: f32,
    ) -> BoatInput {
        self.timer -= dt;
        self.open_timer -= dt;
        self.hold -= dt;
        let car = &h.car;
        if her.sight.unseen > 10.0 {
            return stop(car).0;
        }
        if self.state == State::Choose {
            if self.timer > 0.0 {
                return BoatInput::default();
            }
            let pick = self.point(h, her, world, claim);
            self.timer = rand * 2.0 + 3.0;
            return match pick {
                None => stop(car).0,
                Some(p) => {
                    self.state = if p.ram { State::Ram } else { State::Going };
                    BoatInput::default()
                }
            };
        }
        let push = keep_clear(car, h.length, h.length.max(h.width), others);
        if self.state == State::Going {
            let Some(p) = self.point(h, her, world, claim) else {
                return stop(car).0;
            };
            if p.ram {
                self.state = State::Ram;
            }
            let half = h.length.max(h.width) * 0.5;
            let cap = (her.velocity.length() * 1.2).max(70.0);
            if (p.at - car.at).length_squared() >= half * half {
                return drive_to(car, p.at, cap, p.slowing, push);
            }
            let mut i = stop(car).0;
            if p.aim {
                i.steer = aim(car, her.sight.at);
            }
            return i;
        }
        if self.state == State::Ram {
            if let Some(p) = self.point(h, her, world, claim).filter(|p| p.ram) {
                return drive_to(car, p.at, 30.0, false, push);
            }
            // Off to one side, then the other, as far as it would go in
            // 3 s; 35 is added to a heading in radians, as the 360's is.
            self.hold = 3.0;
            let far = car.speed().abs() * 3.0;
            let heading = yaw(car.forward);
            for (turn, by) in [(1.0, 35.0), (-1.0, -35.0)] {
                let (s, c) = (heading + by).sin_cos();
                if !world.sweep(car.at + Vec3::new(s, 0.0, c) * far).blocked {
                    self.turn = turn;
                    self.state = State::Evade;
                    return BoatInput::default();
                }
            }
            self.state = State::Evade;
        }
        if self.hold <= 0.0 {
            self.state = State::Choose;
            return BoatInput::default();
        }
        BoatInput {
            throttle: 0.5,
            steer: self.turn,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hull(rank: usize) -> Hull {
        Hull {
            id: 1,
            car: CarState {
                at: Vec3::ZERO,
                forward: Vec3::Z,
                velocity: Vec3::ZERO,
            },
            length: 12.0,
            width: 4.0,
            height: 3.0,
            stern: -6.0,
            range: Some(120.0),
            rank,
            bears: Bears::default(),
        }
    }

    fn her(z: f32) -> Target {
        Target {
            sight: Sight {
                at: Vec3::new(0.0, 0.0, z),
                unseen: 0.0,
                vehicle: true,
            },
            velocity: Vec3::ZERO,
            forward: Vec3::Z,
            size: 0.0,
            swimming: false,
        }
    }

    /// Open sea, or with `open` false a shore under her; points past
    /// `wall` along z are blocked.
    struct Sea {
        open: bool,
        wall: f32,
    }

    impl BoatWorld for Sea {
        fn sweep(&mut self, to: Vec3) -> Sweep {
            let blocked = to.z > self.wall;
            Sweep {
                blocked,
                met: if blocked { to.with_z(self.wall) } else { to },
            }
        }
        fn open_under(&mut self, _: Vec3, _: f32) -> bool {
            self.open
        }
    }

    fn sea() -> Sea {
        Sea {
            open: true,
            wall: f32::MAX,
        }
    }

    fn shore(wall: f32) -> Sea {
        Sea { open: false, wall }
    }

    #[test]
    fn its_ring_opens_with_rank() {
        assert_eq!(ring(4.0, 0.0, 0), 16.0);
        assert_eq!(ring(4.0, 0.0, 1), 34.0);
        assert_eq!(ring(4.0, 10.0, 0), 26.0);
    }

    #[test]
    fn over_a_shore_it_comes_to_its_ring_slowing_and_aims() {
        let mut a = BoatAttack::default();
        let mut w = shore(f32::MAX);
        let p = a.point(&hull(0), &her(100.0), &mut w, &mut None).unwrap();
        assert!((p.at.z - 72.0).abs() < 1e-4, "{}", p.at);
        assert!(p.slowing && p.aim && !p.ram);
    }

    #[test]
    fn over_a_shore_something_in_the_way_only_stops_it_short() {
        let mut a = BoatAttack::default();
        let mut w = shore(40.0);
        let p = a.point(&hull(0), &her(100.0), &mut w, &mut None).unwrap();
        assert_eq!(p.at.z, 40.0);
    }

    #[test]
    fn with_no_gun_that_bears_it_goes_behind_her() {
        let mut a = BoatAttack::default();
        let p = a
            .point(&hull(0), &her(100.0), &mut sea(), &mut None)
            .unwrap();
        assert!((p.at.z - 72.0).abs() < 1e-4, "{}", p.at);
        assert!(p.slowing && !p.aim);
    }

    #[test]
    fn with_side_guns_it_stands_off_an_eighth_turn_round() {
        let mut a = BoatAttack::default();
        let mut h = hull(0);
        h.bears.plus = true;
        let p = a.point(&h, &her(100.0), &mut sea(), &mut None).unwrap();
        // 16 m back from her and an eighth round, toward +x.
        assert!((p.at.x - 11.3).abs() < 0.1, "{}", p.at);
        assert!((p.at.z - 88.7).abs() < 0.1, "{}", p.at);
        assert!(!p.slowing);
    }

    #[test]
    fn with_a_gun_ahead_it_lies_behind_her_and_aims() {
        let mut a = BoatAttack::default();
        let mut h = hull(0);
        h.bears.ahead = true;
        let mut t = her(100.0);
        t.forward = Vec3::X;
        let p = a.point(&h, &t, &mut sea(), &mut None).unwrap();
        assert!((p.at.x + 28.0).abs() < 1e-4, "{}", p.at);
        assert!(p.aim);
    }

    #[test]
    fn past_its_guns_range_it_gives_up() {
        let mut a = BoatAttack::default();
        let mut h = hull(0);
        h.range = Some(10.0);
        assert!(a.point(&h, &her(100.0), &mut sea(), &mut None).is_none());
    }

    #[test]
    fn it_runs_at_her_swimming_toward_it_and_claims_her() {
        let mut a = BoatAttack::default();
        let mut t = her(100.0);
        t.sight.vehicle = false;
        t.swimming = true;
        t.forward = Vec3::NEG_Z;
        let mut claim = None;
        let p = a.point(&hull(0), &t, &mut sea(), &mut claim).unwrap();
        assert!(p.ram);
        assert_eq!(p.at.z, 124.0);
        assert_eq!(claim, Some(1));
        // Another boat keeps to its ring.
        let mut other = hull(1);
        other.id = 2;
        let p = a.point(&other, &t, &mut sea(), &mut claim).unwrap();
        assert!(!p.ram);
        assert_eq!(claim, Some(1));
    }

    #[test]
    fn a_failed_run_turns_it_off_for_three_seconds() {
        let mut a = BoatAttack {
            state: State::Ram,
            ..BoatAttack::default()
        };
        let mut h = hull(0);
        h.car.velocity = Vec3::Z * 10.0;
        let mut t = her(100.0);
        t.sight.vehicle = false;
        let mut w = sea();
        let i = a.think(&h, &t, &mut w, &[], &mut None, 0.5, 0.1);
        assert_eq!(a.state, State::Evade);
        assert_eq!(i, BoatInput::default());
        let i = a.think(&h, &t, &mut w, &[], &mut None, 0.5, 1.0);
        assert_eq!((i.throttle, i.steer), (0.5, 1.0));
        a.think(&h, &t, &mut w, &[], &mut None, 0.5, 2.5);
        assert_eq!(a.state, State::Choose);
    }

    #[test]
    fn it_chooses_then_goes() {
        let mut a = BoatAttack::default();
        let mut w = sea();
        a.think(&hull(0), &her(100.0), &mut w, &[], &mut None, 0.5, 0.1);
        assert_eq!(a.state, State::Going);
        assert!((a.timer - 4.0).abs() < 1e-6);
        let i = a.think(&hull(0), &her(100.0), &mut w, &[], &mut None, 0.5, 0.1);
        assert_eq!(i.steer, 0.0);
        assert_eq!(i.throttle, 1.0);
    }

    #[test]
    fn its_guns_bear_by_their_mounts() {
        let mut b = Bears::default();
        b.add(Vec3::Z, (-1.0, 1.0));
        assert_eq!(
            b,
            Bears {
                ahead: true,
                ..Bears::default()
            }
        );
        let q = std::f32::consts::FRAC_PI_2;
        let mut b = Bears::default();
        b.add(Vec3::Z, (-q, q));
        assert!(b.ahead && b.plus && b.minus && !b.astern);
        let mut b = Bears::default();
        b.add(Vec3::NEG_Z, (0.0, 0.0));
        b.add(Vec3::NEG_X, (0.0, 0.0));
        assert!(b.astern && b.minus && !b.plus);
    }

    #[test]
    fn its_helpers() {
        let h = hull(0);
        assert!(drive_to(&h.car, Vec3::X * 50.0, 30.0, false, Vec3::ZERO).steer > 0.0);
        assert!(stop(&h.car).1);
        assert!(aim(&h.car, Vec3::X * 50.0) > 0.0);
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
