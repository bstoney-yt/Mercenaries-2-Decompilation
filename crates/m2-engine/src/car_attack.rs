//! How an AI car that fights from where it stands goes about it: the
//! 360's `CarAttack` task (0x822ACB28), run by a driver with a gun of his
//! own, or by the driver of a car labelled `Apc` that has a crew
//! (`CarPursue` 0x822ADE08 hands over to it then; see [`runs_car_attack`]).
//!
//! From the 360 code (0x822ACB28), the task's states (its mind's +0x14,
//! bits 19..23) and its timer (+0x2C):
//! - **0, hold:** easing to a stop (0x822AA568). When the timer runs out:
//!   with her behind the car (0x822C1C90), or one time in ten (the double
//!   0.1 at 0x820F9248), it turns to face her (state 2, timer
//!   `(r + 1) x 1.5`); otherwise, unless the crew are in range of her,
//!   it sets off for a point one car length short of her on its own side
//!   (state 1, timer `(r + 1) x 3`), noting whether it set off out of
//!   range.
//! - **1, moving:** with her far ahead (more than five car lengths, within
//!   30 degrees of its heading, past its nose, not in a vehicle), and its
//!   line-check wait (+0x30) out, one think in five (one in 2.5 with no
//!   crew) it sweeps the line to her (0x82166958): clear, it stops and
//!   charges (state 3, timer `(r + 1) x 3`); blocked, it waits 20 s
//!   before the next. It stops (to state 2 as in state 0, else 0) once it
//!   set off out of range and is now in range, or its way is done.
//! - **2, turning to face her:** while she is more than 45 degrees off
//!   its heading it manoeuvres round ([`Manoeuvre`]); then, or when it
//!   has no room, holds (state 0,
//!   timer `(r + 1) x 3`).
//! - **3, charging:** while she was seen within 2 s, the timer runs and
//!   she is within `sqrt(1.5)` x five car lengths (the double 1.5 at
//!   0x820F9960): at her at up to 25 m/s (0x822AC008), or, with her
//!   behind it, a stop now (state 5, 0x822AA630). Otherwise it eases to a
//!   stop and then holds (state 0, line checks off 40 s, timer
//!   `(r + 1) x 1.5`).
//! - **5, stopping:** stops now; stopped, it moves on (state 1, line
//!   checks off 40 s, timer `(r + 1) x 1.5`).
//! - Moving, it follows its way at up to 10 m/s (0x822AC008); within 4 m
//!   of its end the way is done (timer `(r + 1) x 3`).
//!
//! The crew's range: the mean of the crew's two weapon distances (the
//! weapon slots' +0x20 and +0x24 of those 0x822ECE70 lists), halved
//! together; a car with no crew so counted always moves up.
//!
//! Inferred: the crew's two distances are their weapons' `MinFireRange`
//! and `FireRange` (the slots' +0x20/+0x24); her record's +0x5C and +0x60
//! are both her distance; the timer and line-check wait count down by
//! the think's length (nothing in 0x822ACB28 counts them); "behind" is
//! her on the far side of the car's middle across its heading, "past its
//! nose" past its front. Not ported (taken as never): the stuck check
//! (+0x34), the hit memory (+0x6BC, +0x6CC, state 6), riders asking for
//! a turn (kind 6), 0x822AA348's ordered point. Its way is
//! 0x822BF7A8's path ([`AttackWorld::toward`], [`crate::path_search`]).
//! The manoeuvre is [`Manoeuvre`] (0x822C8770); inferred there: its room
//! check is only 0x822C8CB8's face sweep (the 360 first also sweeps the
//! whole box shifted toward the turn, 0x82166880 with mask 0x6228651E).

use m2_math::glam::Vec3;

use crate::car_driver::{self, AiInput, CarDriver, CarState};

/// The 360's label of an armoured personnel carrier (`name_hash("Apc")`,
/// 0x82255118's class 3).
pub const APC_LABEL: u32 = 0x9cdb_d62d;

/// Whether a car's driver runs `CarAttack` rather than `CarPursue`
/// (0x822ADE08): he has a gun of his own, or the car is labelled `Apc`
/// and has a crew (0x822ECE70 lists someone).
pub fn runs_car_attack(driver_armed: bool, labels: &[u32], crew: usize) -> bool {
    driver_armed || (labels.contains(&APC_LABEL) && crew > 0)
}

/// The task's state (its mind's +0x14 bits 19..23).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Hold,
    Moving,
    Turning,
    Charging,
    Stopping,
}

/// What the car sees of her this think.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sight {
    pub at: Vec3,
    /// Seconds since she was last seen (her record's +0x8C).
    pub unseen: f32,
    /// She is in a vehicle (her record's kind 4).
    pub vehicle: bool,
}

/// What `CarAttack` asks of the world.
pub trait AttackWorld {
    /// A random number, 0..1 (0x8215E260).
    fn rand(&mut self) -> f32;
    /// The line from `from` to `to` is clear (0x82166958's sweep).
    fn line_clear(&mut self, from: Vec3, to: Vec3) -> bool;
    /// Where to steer for, going from `from` to `goal` by its way
    /// (0x822BF7A8 asks for one, [`crate::path_search`]); straight at it
    /// unless the world finds one.
    fn toward(&mut self, from: Vec3, goal: Vec3) -> Vec3 {
        let _ = from;
        goal
    }
    /// Room to move ahead (`forward`) or back: 0x822C8CB8's face of its
    /// box, `lift` m up, swept `reach` m on from its nose or tail; clear
    /// unless the world says otherwise.
    fn room(&mut self, forward: bool, lift: f32, reach: f32) -> bool {
        let _ = (forward, lift, reach);
        true
    }
}

/// The turn-round manoeuvre's state (its record's +8 top bits, 0x822C8770).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Manoeuvre {
    #[default]
    Idle,
    /// Under way, ahead or backing (the record's +0xC bit 3).
    Turning { forward: bool },
    /// Over: no room either way, or none left the way it went.
    Done,
}

impl Manoeuvre {
    /// One think of 0x822C8770: turning `car` (of `length`) round toward a
    /// point `bearing` off its heading, `dist` away. Starting, it goes ahead
    /// when the point is more than 3 lengths off, else backs; with no room
    /// that way (0x822C8CB8: 0.5 m up, 0.2 m on) it tries the other, four
    /// tries in all, and with none it is over. Under way: when the way it
    /// goes has no room it is over; going ahead within 0.75 x 3 lengths it
    /// backs instead; full lock toward the point (the other way backing,
    /// kept to -1..1 of the bearing in radians), the handbrake off and the
    /// throttle 0.6 ahead or -0.6 back (0x822D3058). `None` when over.
    pub fn think(
        &mut self,
        car: &CarState,
        length: f32,
        bearing: f32,
        dist: f32,
        world: &mut impl AttackWorld,
    ) -> Option<AiInput> {
        let _ = car;
        if *self == Manoeuvre::Idle {
            let mut forward = dist > length * 3.0;
            *self = Manoeuvre::Done;
            for _ in 0..4 {
                if world.room(forward, 0.5, 0.2) {
                    *self = Manoeuvre::Turning { forward };
                    break;
                }
                forward = !forward;
            }
        }
        let Manoeuvre::Turning { mut forward } = *self else {
            return None;
        };
        if !world.room(forward, 0.5, 0.2) {
            *self = Manoeuvre::Done;
            return None;
        }
        if forward && dist < length * 3.0 * 0.75 {
            forward = false;
            *self = Manoeuvre::Turning { forward };
        }
        let steer = if forward { bearing } else { -bearing }.clamp(-1.0, 1.0);
        let pedal = if forward { 0.6 } else { -0.6 };
        Some(
            AiInput::default()
                .send(car_driver::Msg::Steer, steer)
                .send(car_driver::Msg::Handbrake, 0.0)
                .send(car_driver::Msg::Throttle, pedal),
        )
    }
}

/// One driver's `CarAttack` memory.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CarAttack {
    pub state: State,
    /// +0x2C.
    pub timer: f32,
    /// +0x30: no line check while above 0.
    pub check_wait: f32,
    /// +0x1C bit 25: set off out of the crew's range.
    pub set_off_out: bool,
    /// Its way's end (+0x2DC, +0x360 bit 6).
    pub way: Option<Vec3>,
    /// Its turn-round manoeuvre (+0x4C record, 0x822C8770).
    pub manoeuvre: Manoeuvre,
}

/// The cone she must be in for a line check, radians (0x822ACB28 asks
/// 0x822EC720 with 0.5235, the 360's own literal, not quite pi / 6).
#[allow(clippy::approx_constant)]
const AHEAD_CONE: f32 = 0.523_5;

/// Whether `p` is behind the car (0x822C1C90).
fn behind(car: &CarState, p: Vec3) -> bool {
    (p - car.at).with_y(0.0).dot(car.forward) <= 0.0
}

/// Whether `p` is within `angle` of the car's heading (0x822EC720).
fn within(car: &CarState, p: Vec3, angle: f32) -> bool {
    let d = (p - car.at).with_y(0.0).normalize_or_zero();
    angle.cos() < d.dot(car.forward)
}

impl CarAttack {
    /// One think: `car` of `length` (its box along its heading), `crew`
    /// its crew's range if any are counted, her as `her`; `dt` the
    /// think's length.
    #[allow(clippy::too_many_arguments)]
    pub fn think(
        &mut self,
        driver: &mut CarDriver,
        car: &CarState,
        length: f32,
        crew: Option<f32>,
        her: &Sight,
        world: &mut impl AttackWorld,
        dt: f32,
    ) -> AiInput {
        self.timer -= dt;
        self.check_wait -= dt;
        if her.unseen > 20.0 {
            return car_driver::slow_to_stop(car).unwrap_or_default();
        }
        let dist2 = (her.at - car.at).length_squared();
        let near2 = (length * 5.0) * (length * 5.0);
        let out_of_range = crew.is_none_or(|m| m * m < dist2);
        let mut expired = self.timer <= 0.0;
        let far_ahead = self.check_wait <= 0.0
            && near2 < dist2
            && within(car, her.at, AHEAD_CONE)
            && (her.at - (car.at + car.forward * length * 0.5))
                .with_y(0.0)
                .dot(car.forward)
                > 0.0
            && !her.vehicle;
        let mut out = None;
        match self.state {
            State::Hold => {
                if expired {
                    if behind(car, her.at) || world.rand() < 0.1 {
                        self.state = State::Turning;
                        self.timer = (world.rand() + 1.0) * 1.5;
                    } else if crew.is_none() || her.unseen > 5.0 || out_of_range {
                        let back = (car.at - her.at).normalize_or_zero();
                        self.way = Some(her.at + back * length);
                        self.state = State::Moving;
                        self.set_off_out = out_of_range;
                        self.timer = (world.rand() + 1.0) * 3.0;
                    }
                }
            }
            State::Moving => {
                expired = true;
                let chance = if crew.is_some() { 0.2 } else { 0.4 };
                if far_ahead && world.rand() < chance {
                    if world.line_clear(car.at, her.at) {
                        self.way = None;
                        self.state = State::Charging;
                        self.timer = (world.rand() + 1.0) * 3.0;
                        return AiInput::default().send(car_driver::Msg::Throttle, 0.0);
                    }
                    self.check_wait = 20.0;
                } else if (!out_of_range && self.set_off_out) || self.way.is_none() {
                    self.way = None;
                    if behind(car, her.at) || world.rand() < 0.1 {
                        self.state = State::Turning;
                        self.timer = (world.rand() + 1.0) * 1.5;
                    } else {
                        self.state = State::Hold;
                    }
                    return car_driver::slow_to_stop(car).unwrap_or_default();
                }
            }
            State::Turning => {
                expired = false;
                let bearing = car_driver::wrap(
                    car_driver::yaw(her.at - car.at) - car_driver::yaw(car.forward),
                );
                let dist = (her.at - car.at).with_y(0.0).length();
                let turning = bearing.abs() > 0.785_25
                    && self
                        .manoeuvre
                        .think(car, length, bearing, dist, world)
                        .inspect(|i| out = Some(i.clone()))
                        .is_some();
                if !turning {
                    self.manoeuvre = Manoeuvre::Idle;
                    self.state = State::Hold;
                    self.timer = (world.rand() + 1.0) * 3.0;
                }
            }
            State::Charging => {
                if her.unseen <= 2.0 && self.timer > 0.0 && dist2 <= near2 * 1.5 {
                    if behind(car, her.at) {
                        self.state = State::Stopping;
                        return car_driver::stop_now(car).0;
                    }
                    return driver.drive_to(car, her.at, 25.0, false, dt);
                }
                self.way = None;
                match car_driver::slow_to_stop(car) {
                    Some(i) => return i,
                    None => {
                        self.state = State::Hold;
                        self.check_wait = 40.0;
                        self.timer = (world.rand() + 1.0) * 1.5;
                        return AiInput::default();
                    }
                }
            }
            State::Stopping => {
                let (i, stopped) = car_driver::stop_now(car);
                if stopped {
                    self.state = State::Moving;
                    self.check_wait = 40.0;
                    self.set_off_out = false;
                    self.timer = (world.rand() + 1.0) * 1.5;
                }
                return i;
            }
        }
        if expired && let Some(end) = self.way {
            let flat = (end - car.at).with_y(0.0);
            if flat.length_squared() <= 16.0 {
                self.way = None;
                self.timer = (world.rand() + 1.0) * 3.0;
            } else {
                let to = world.toward(car.at, end);
                return driver.drive_to(car, to, 10.0, false, dt);
            }
        }
        out.unwrap_or_else(|| car_driver::slow_to_stop(car).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct W {
        r: f32,
        clear: bool,
    }
    impl AttackWorld for W {
        fn rand(&mut self) -> f32 {
            self.r
        }
        fn line_clear(&mut self, _: Vec3, _: Vec3) -> bool {
            self.clear
        }
    }

    fn car(speed: f32) -> CarState {
        CarState {
            at: Vec3::ZERO,
            forward: Vec3::Z,
            velocity: Vec3::Z * speed,
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
    fn who_runs_it() {
        assert!(runs_car_attack(true, &[], 0));
        assert!(runs_car_attack(false, &[APC_LABEL], 1));
        assert!(!runs_car_attack(false, &[APC_LABEL], 0));
        assert!(!runs_car_attack(false, &[0x2b8d_1fdf], 3));
        assert_eq!(m2_assets::name_hash("Apc"), APC_LABEL);
    }

    // Out of its crew's range it sets off toward her, then, far ahead
    // with a clear line, charges; within range it holds.
    #[test]
    fn it_moves_up_then_charges() {
        let mut a = CarAttack::default();
        let mut d = CarDriver::default();
        let mut w = W {
            r: 0.5,
            clear: true,
        };
        let i = a.think(&mut d, &car(0.0), 6.0, Some(50.0), &her(150.0), &mut w, 0.1);
        assert_eq!(a.state, State::Moving);
        assert!(a.set_off_out);
        assert_eq!(a.way, Some(Vec3::new(0.0, 0.0, 144.0)));
        assert!(i.last(car_driver::Msg::Throttle).unwrap() > 0.0);
        // One think in five checks the line: r 0.1 < 0.2.
        w.r = 0.1;
        a.think(&mut d, &car(5.0), 6.0, Some(50.0), &her(150.0), &mut w, 0.1);
        assert_eq!(a.state, State::Charging);
        // Charging needs her within sqrt(1.5) x 30 m.
        let i = a.think(&mut d, &car(5.0), 6.0, Some(50.0), &her(30.0), &mut w, 0.1);
        assert_eq!(a.state, State::Charging);
        assert!(i.last(car_driver::Msg::Throttle).unwrap() > 0.0);
        // In range from the start: it holds.
        let mut a = CarAttack::default();
        w.r = 0.5;
        a.think(&mut d, &car(0.0), 6.0, Some(80.0), &her(60.0), &mut w, 0.1);
        assert_eq!(a.state, State::Hold);
    }

    // With her behind it, it turns to face her.
    #[test]
    fn it_turns_to_face_her_behind() {
        let mut a = CarAttack::default();
        let mut d = CarDriver::default();
        let mut w = W {
            r: 0.5,
            clear: true,
        };
        a.think(&mut d, &car(0.0), 6.0, Some(50.0), &her(-60.0), &mut w, 0.1);
        assert_eq!(a.state, State::Turning);
        assert!((a.timer - 2.25).abs() < 1e-6);
    }

    // The manoeuvre: far off it goes ahead on full lock toward her; near,
    // it backs the other way; with no room either way it is over.
    #[test]
    fn the_manoeuvre_goes_ahead_or_backs_or_gives_up() {
        struct Room(bool, bool);
        impl AttackWorld for Room {
            fn rand(&mut self) -> f32 {
                0.5
            }
            fn line_clear(&mut self, _: Vec3, _: Vec3) -> bool {
                true
            }
            fn room(&mut self, forward: bool, _: f32, _: f32) -> bool {
                if forward { self.0 } else { self.1 }
            }
        }
        let c = car(0.0);
        let mut m = Manoeuvre::Idle;
        let i = m.think(&c, 5.0, 2.0, 40.0, &mut Room(true, true)).unwrap();
        assert_eq!(m, Manoeuvre::Turning { forward: true });
        assert_eq!(i.last(car_driver::Msg::Steer), Some(1.0));
        assert_eq!(i.last(car_driver::Msg::Throttle), Some(0.6));
        // Within 0.75 x 3 lengths: it backs, steering the other way.
        let i = m.think(&c, 5.0, 2.0, 10.0, &mut Room(true, true)).unwrap();
        assert_eq!(m, Manoeuvre::Turning { forward: false });
        assert_eq!(i.last(car_driver::Msg::Steer), Some(-1.0));
        assert_eq!(i.last(car_driver::Msg::Throttle), Some(-0.6));
        // No room backing: over.
        assert!(
            m.think(&c, 5.0, 2.0, 10.0, &mut Room(true, false))
                .is_none()
        );
        assert_eq!(m, Manoeuvre::Done);
        // Near with no room behind: it starts ahead, but near as it is it
        // backs at once (0x822C8770 checks the distance after choosing).
        let mut m = Manoeuvre::Idle;
        m.think(&c, 5.0, -0.5, 10.0, &mut Room(true, false));
        assert_eq!(m, Manoeuvre::Turning { forward: false });
        let mut m = Manoeuvre::Idle;
        assert!(
            m.think(&c, 5.0, -0.5, 10.0, &mut Room(false, false))
                .is_none()
        );
    }
}
