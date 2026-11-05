//! How an AI tank fights her: the 360's tank driving helpers and its
//! `TankAttack` task (0x822B6508).
//!
//! From the 360 code, the helpers (each sends the tank's input its
//! throttle 0x7D3B632C and steering 0x3483DBF1; the tank's input,
//! 0x82194298, keeps the throttle as it comes, +0x28, and the steering
//! negated, +0x2C; the alpha's tank takes both the other way round from
//! the 360's, [`crate::physics`], so here both are in the alpha's sense:
//! throttle + ahead, steering + toward +x of its heading):
//! - **Drive to a point** (0x822B5B98), at up to `cap` m/s: the bearing
//!   to it off its heading; within 20 m and more than 1.3 x 90 degrees
//!   off, it backs to it (the bearing taken from its tail, the speed
//!   negated). The speed asked: no more than
//!   `0.1309 / (|bearing| + 0.1309) x 25`, nor 25 (ramming: scaled by
//!   `1 - 16 / (d^2 + 16)`). Steering `bearing / pi x 10`, throttle
//!   `(asked - speed) x 0.1`, each kept to -1..1.
//! - **Stop** (0x822B5918): steering 0; over 0.3 m/s along its heading,
//!   throttle `-speed x 0.2` (kept to -1..1); else 0, stopped.
//! - **Turn to face a point** (0x822B5A88): the stop, then steering the
//!   bearing itself (radians, kept to -1..1); done within 0.26175 rad.
//!
//! `TankAttack` (0x822B6508), its states (the mind's +0x14 bits 19..23),
//! timer (+0x2C) and line-check wait (+0x30):
//! - **0, hold:** stopping. When the timer runs out: with her behind, or
//!   at random under 0.15 (the double at 0x820F9970), it turns to face
//!   her (state 2, timer `r + 2.25`); otherwise it picks a point beside
//!   her `length + standoff` away (13 m standoff at a man, 20 at a
//!   vehicle): on its own side one time in five, else at a random
//!   bearing round her, and goes there (state 1, timer `2r + 4.5`).
//! - **1, moving:** with her far ahead (over five lengths, within 30
//!   degrees, past its nose, not in a vehicle), the line-check wait out,
//!   and at random under 0.2 (0.4 with no other gunner aboard able to
//!   fire, 0x822ECA20) and nothing just ahead (0x822C8CB8): it stops and
//!   charges (state 3, timer `2r + 4.5`). Otherwise it goes on its way
//!   unless the way is done or it is driving away from her at over
//!   2 m/s (0x822C1B68); then it stops and holds.
//! - **2, turning to face her** (0x822B5A88) until the timer runs out or
//!   she has gone unseen 5 s (state 0, timer `r + 2.25`).
//! - **3, charging** at her at up to 25 m/s (0x822B5B98) while she was
//!   seen within 2 s, the timer runs, she is within `sqrt(1.5)` x five
//!   lengths (the double at 0x820F9960), not behind and nothing is just
//!   ahead; else it stops (state 1, timer `r + 2.25`, line checks off
//!   40 s).
//! - **4, beside her on foot** (she is within its length and beside its
//!   hull, 0x822C1F18): throttle 0, full lock toward her side; back to 0
//!   once she is not.
//! - Moving, it follows its way at up to 10 m/s; within 4 m of the end
//!   the way is done (timer `2r + 4.5`).
//! - Its own gun fires at her every think (0x822C95D8).
//!
//! Inferred: the timer and wait count down by the think's length; "just
//! ahead" is a ray 0.7 m out from its nose (0x822C8CB8 sweeps a box);
//! her record's +0x5C is her distance. Not ported (never): the hit
//! memory (state 6), the stuck check, the leash (+0x7BC), 0x822B5EC0's
//! ordered point. Stand-in: its way (0x822BF7A8's path) is the straight
//! line to the point.

use m2_math::glam::Vec3;

use crate::car_attack::{AttackWorld, Sight};
use crate::car_driver::{CarState, wrap, yaw};

/// The tank's input as the alpha takes it: throttle (+ ahead) and
/// steering (+ toward +x of its heading).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TankInput {
    pub throttle: f32,
    pub steer: f32,
}

fn bearing(car: &CarState, to: Vec3) -> f32 {
    wrap(yaw((to - car.at).with_y(0.0)) - yaw(car.forward))
}

/// 0x822B5B98: toward `to` at up to `cap` m/s, `ram` slowing near it.
pub fn drive_to(car: &CarState, to: Vec3, cap: f32, ram: bool) -> TankInput {
    let pi = std::f32::consts::PI;
    let d = (to - car.at).with_y(0.0);
    let dist2 = d.length_squared();
    let s = car.speed();
    let mut b = bearing(car, to);
    let back = dist2 < 400.0 && b.abs() > pi * 0.5 * 1.3;
    if back {
        let flip = if yaw(d) > 0.0 { -pi } else { pi };
        b = wrap(flip + yaw(d) - yaw(car.forward));
    }
    let slow = if ram {
        1.0 - 16.0 / (dist2 + 16.0)
    } else {
        1.0
    };
    let turning = 0.130_899_7 / (b.abs() + 0.130_899_7) * 25.0;
    let mut asked = cap.min(turning).min(slow * 25.0);
    if back {
        asked = -asked;
    }
    TankInput {
        steer: (b / pi * 10.0).clamp(-1.0, 1.0),
        throttle: ((asked - s) * 0.1).clamp(-1.0, 1.0),
    }
}

/// 0x822B5918: stopping; `true` once stopped.
pub fn stop(car: &CarState) -> (TankInput, bool) {
    let s = car.speed();
    if s.abs() > 0.3 {
        (
            TankInput {
                throttle: (-s * 0.2).clamp(-1.0, 1.0),
                steer: 0.0,
            },
            false,
        )
    } else {
        (TankInput::default(), true)
    }
}

/// 0x822B5A88: turning on the spot to face `to`; `true` once within
/// 0.26175 rad.
pub fn turn_to(car: &CarState, to: Vec3) -> (TankInput, bool) {
    let (mut i, _) = stop(car);
    let b = bearing(car, to);
    i.steer = b.clamp(-1.0, 1.0);
    (
        i,
        0.261_75_f32.cos()
            < (to - car.at)
                .with_y(0.0)
                .normalize_or_zero()
                .dot(car.forward),
    )
}

/// What `TankAttack` asks of the world beyond [`AttackWorld`].
pub trait TankWorld: AttackWorld {
    /// Nothing within `reach` m ahead of its nose (0x822C8CB8).
    fn ahead_clear(&mut self, reach: f32) -> bool;
}

/// `TankAttack`'s state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Hold,
    Moving,
    Turning,
    Charging,
    Beside,
}

/// One tank driver's `TankAttack` memory.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TankAttack {
    pub state: State,
    pub timer: f32,
    pub check_wait: f32,
    pub way: Option<Vec3>,
}

/// The tank: where and how it moves, its box (`length` along its heading,
/// `width` across) and whether another gunner aboard can fire.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hull {
    pub car: CarState,
    pub length: f32,
    pub width: f32,
    pub gunner: bool,
}

/// 0x822C1F18: which way it slews for her (+1 toward +x, -1 toward -x, the
/// other way when she is by its back half) when she is alongside it (within its length along, between half its width and
/// half its length across); 0 otherwise.
pub fn beside(h: &Hull, at: Vec3) -> f32 {
    let d = (at - h.car.at).with_y(0.0);
    let along = d.dot(h.car.forward);
    let right = Vec3::new(h.car.forward.z, 0.0, -h.car.forward.x);
    let across = d.dot(right);
    let half = h.length * 0.5;
    if along.abs() < half && h.width * 0.5 <= across.abs() && across.abs() <= half {
        if along >= 0.0 {
            across.signum()
        } else {
            -across.signum()
        }
    } else {
        0.0
    }
}

impl TankAttack {
    /// One think at her (`her`), `dt` its length.
    pub fn think(&mut self, h: &Hull, her: &Sight, w: &mut impl TankWorld, dt: f32) -> TankInput {
        self.timer -= dt;
        self.check_wait -= dt;
        let car = &h.car;
        let dist2 = (her.at - car.at).with_y(0.0).length_squared();
        let near2 = (h.length * 5.0) * (h.length * 5.0);
        let behind = (her.at - car.at).with_y(0.0).dot(car.forward) <= 0.0;
        let side = beside(h, her.at);
        let expired = self.timer <= 0.0;
        let far_ahead = self.check_wait <= 0.0
            && !her.vehicle
            && near2 < dist2
            && 0.523_499_97_f32.cos()
                < (her.at - car.at)
                    .with_y(0.0)
                    .normalize_or_zero()
                    .dot(car.forward)
            && (her.at - (car.at + car.forward * h.length * 0.5))
                .with_y(0.0)
                .dot(car.forward)
                > 0.0;
        let follow;
        match self.state {
            State::Hold => {
                if side != 0.0 && !her.vehicle {
                    self.state = State::Beside;
                    return TankInput::default();
                }
                follow = expired;
                if expired {
                    if behind || w.rand() < 0.15 {
                        self.state = State::Turning;
                        self.timer = w.rand() + 2.25;
                    } else {
                        let standoff = if her.vehicle { 20.0 } else { 13.0 };
                        let r = h.length + standoff;
                        let p = if w.rand() <= 0.2 {
                            her.at + (car.at - her.at).normalize_or_zero() * r
                        } else {
                            let a = w.rand() * std::f32::consts::TAU;
                            her.at + Vec3::new(a.sin(), 0.0, a.cos()) * r
                        };
                        self.way = Some(p);
                        self.timer = w.rand() * 2.0 + 4.5;
                        self.state = State::Moving;
                    }
                }
            }
            State::Moving => {
                if side != 0.0 && !her.vehicle {
                    self.state = State::Beside;
                    return TankInput::default();
                }
                let chance = if h.gunner { 0.2 } else { 0.4 };
                if far_ahead && w.rand() < chance && w.ahead_clear(0.7) {
                    self.way = None;
                    self.state = State::Charging;
                    self.timer = w.rand() * 2.0 + 4.5;
                    return TankInput::default();
                }
                follow = true;
                let away = car.velocity.dot((her.at - car.at).normalize_or_zero()) <= -2.0;
                if self.way.is_none() || away {
                    self.way = None;
                    self.state = State::Hold;
                    return stop(car).0;
                }
            }
            State::Turning => {
                if expired || her.unseen > 5.0 {
                    self.timer = w.rand() + 2.25;
                    self.state = State::Hold;
                }
                return turn_to(car, her.at).0;
            }
            State::Charging => {
                if side != 0.0 && !her.vehicle {
                    self.check_wait = 40.0;
                    self.state = State::Beside;
                    return TankInput::default();
                }
                if her.unseen > 2.0
                    || expired
                    || near2 * 1.5 < dist2
                    || behind
                    || !w.ahead_clear(0.7)
                {
                    self.way = None;
                    self.state = State::Moving;
                    self.timer = w.rand() + 2.25;
                    self.check_wait = 40.0;
                    return stop(car).0;
                }
                return drive_to(car, her.at, 25.0, false);
            }
            State::Beside => {
                if side == 0.0 {
                    self.state = State::Hold;
                }
                return TankInput {
                    throttle: 0.0,
                    steer: side,
                };
            }
        }
        if follow && let Some(end) = self.way {
            if (end - car.at).with_y(0.0).length_squared() <= 16.0 {
                self.way = None;
                self.timer = w.rand() * 2.0 + 4.5;
            } else {
                return drive_to(car, end, 10.0, false);
            }
        }
        stop(car).0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct W(f32);
    impl AttackWorld for W {
        fn rand(&mut self) -> f32 {
            self.0
        }
        fn line_clear(&mut self, _: Vec3, _: Vec3) -> bool {
            true
        }
    }
    impl TankWorld for W {
        fn ahead_clear(&mut self, _: f32) -> bool {
            true
        }
    }

    fn hull(speed: f32) -> Hull {
        Hull {
            car: CarState {
                at: Vec3::ZERO,
                forward: Vec3::Z,
                velocity: Vec3::Z * speed,
            },
            length: 8.0,
            width: 3.5,
            gunner: false,
        }
    }

    fn her(x: f32, z: f32) -> Sight {
        Sight {
            at: Vec3::new(x, 0.0, z),
            unseen: 0.0,
            vehicle: false,
        }
    }

    #[test]
    fn the_helpers() {
        let h = hull(0.0);
        // Dead ahead from rest: full lock none, throttle up.
        let i = drive_to(&h.car, Vec3::Z * 100.0, 25.0, false);
        assert_eq!(i.steer, 0.0);
        assert!((i.throttle - 1.0).abs() < 1e-6);
        // Close behind: it backs.
        let i = drive_to(&h.car, Vec3::NEG_Z * 10.0, 25.0, false);
        assert!(i.throttle < 0.0, "{i:?}");
        // Stopping from 5 m/s: throttle -1 (5 x 0.2), then stopped.
        assert_eq!(stop(&hull(5.0).car).0.throttle, -1.0);
        assert!(stop(&hull(0.2).car).1);
        // Turning to +x: steering +.
        let (i, done) = turn_to(&h.car, Vec3::X * 10.0);
        assert!(i.steer > 0.0 && !done);
        assert!(turn_to(&h.car, Vec3::Z * 10.0).1);
    }

    #[test]
    fn it_goes_round_her_then_charges() {
        let mut t = TankAttack::default();
        let mut w = W(0.5);
        t.think(&hull(0.0), &her(0.0, 150.0), &mut w, 0.1);
        assert_eq!(t.state, State::Moving);
        // r 0.5: a random bearing round her, 21 m off.
        let p = t.way.unwrap();
        assert!((p.distance(Vec3::new(0.0, 0.0, 150.0)) - 21.0).abs() < 1e-3);
        w.0 = 0.1;
        t.think(&hull(3.0), &her(0.0, 150.0), &mut w, 0.1);
        assert_eq!(t.state, State::Charging);
        // Out of reach (past sqrt(1.5) x 40 m): it stops and moves on.
        t.think(&hull(3.0), &her(0.0, 150.0), &mut w, 0.1);
        assert_eq!(t.state, State::Moving);
    }

    #[test]
    fn beside_its_hull_it_slews() {
        let mut t = TankAttack::default();
        let mut w = W(0.5);
        let i = t.think(&hull(0.0), &her(3.0, 0.0), &mut w, 0.1);
        assert_eq!(t.state, State::Beside);
        assert_eq!(i, TankInput::default());
        let i = t.think(&hull(0.0), &her(3.0, 0.0), &mut w, 0.1);
        assert_eq!(i.steer, 1.0);
        // Behind it: it turns to face her.
        let mut t = TankAttack::default();
        t.think(&hull(0.0), &her(0.0, -60.0), &mut w, 0.1);
        assert_eq!(t.state, State::Turning);
    }
}
