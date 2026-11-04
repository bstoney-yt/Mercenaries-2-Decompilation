//! An AI driver's hands and feet: how the 360's car AI steers, throttles
//! and brakes a car toward a point (0x822AC008), how it stops one
//! (0x822AA568, 0x822AA630), and how a pursuing driver with no gun of
//! his own runs her down (`CarPursue`, 0x822ADE08).
//!
//! From the 360 code (0x822AC008), toward a point `(x, z)` with a speed
//! cap:
//! - the bearing to it off the car's heading (yaw `atan2(x, z)`,
//!   0x82855048, wrapped to +-pi, 0x82854068), and the car's speed along
//!   its own heading `s`;
//! - a turning reach `15 x (tan(|bearing| / 2 - g) + tan(g))`, `g` from
//!   its speed (`(1 - f) / 2` with `f = 1 - 4 / (|s| + 4)`, negated when
//!   going ahead); a point nearer than that is out of its turn: it backs
//!   toward it (its speed asked negated), on full lock (30 degrees) unless
//!   the bearing is within 2 degrees; when ramming (`mode 1`) a point
//!   within 16 m and more than 150 degrees off is taken as dead astern
//!   (the bearing less pi);
//! - backing (`s` under -0.05 m/s) turns the other way;
//! - the speed asked: the cap, but no more than
//!   `0.1309 / (|bearing| + 0.1309) x 38 + 2` (slower the more it must
//!   turn), nor 40 m/s (ramming, scaled by `1 - 100 / (d^2 + 100)`);
//! - the pedal: `(asked - s) x 0.14`; where that is against its motion by
//!   more than a fifth, the brake (and no throttle), else the throttle
//!   kept to -1..1;
//! - the wheel: the bearing over 30 degrees, kept to -1..1; above 3 m/s
//!   eased toward that by `10 x dt` a think and squared (sign kept).
//!
//! It sends the car the 360's messages, in the 360's order ([`AiInput`]):
//! throttle 0x460C5913, brake 0x55B8E0A1, handbrake 0x37086E0A, steering
//! 0x3483DBF1. The car holds what each last set ([`CarInputs`],
//! 0x82189AC8): the throttle sets its pedal and brake by its gear (ahead
//! in its forward gear, backing in reverse; the other way, the brake),
//! the brake and handbrake their own; a message not sent leaves its
//! value as it was.
//!
//! Unknown: the mind's +0x320 (0x822AC008 caps the car at 5 m/s while it
//! is under 0.5; who writes it is not found): taken as at least 0.5. Its
//! +0x344 (a reach from the car's width, 1.5 x 3..6.5 of it) is not used
//! here. The car's gear (+0x98 of its input) is its gearbox's: the
//! alpha's, which goes into reverse once all but still with the brake held
//! (as 0x8219C490 has it; see [`crate::physics::Physics::drive`]).

use m2_math::glam::Vec3;

/// The wheel's full lock, radians (0x822AC008's 30 degrees).
pub const FULL_LOCK: f32 = 30.0 * std::f32::consts::PI / 180.0;

/// A message to a car's input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Msg {
    /// 0x460C5913: -1 back to 1 ahead.
    Throttle,
    /// 0x55B8E0A1.
    Brake,
    /// 0x37086E0A.
    Handbrake,
    /// 0x3483DBF1: -1 to 1, +1 toward +x of its heading (the bearing over
    /// full lock).
    Steer,
}

/// What the AI sends a car this think, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiInput(pub Vec<(Msg, f32)>);

impl AiInput {
    fn send(mut self, m: Msg, v: f32) -> Self {
        self.0.push((m, v));
        self
    }

    /// The last value sent of `m`, if any.
    pub fn last(&self, m: Msg) -> Option<f32> {
        self.0.iter().rev().find(|(k, _)| *k == m).map(|&(_, v)| v)
    }
}

/// What a car's input holds (0x82189AC8's: its pedal +0x2C, brake +0x30,
/// handbrake +0x34, wheel +0x28 and forward gear +0x98).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarInputs {
    pub pedal: f32,
    pub brake: f32,
    pub handbrake: f32,
    pub steer: f32,
    pub forward: bool,
}

impl Default for CarInputs {
    fn default() -> Self {
        Self {
            pedal: 0.0,
            brake: 0.0,
            handbrake: 0.0,
            steer: 0.0,
            forward: true,
        }
    }
}

impl CarInputs {
    /// Takes `input`'s messages in order, the car's gearbox in its
    /// `forward` gears or reverse.
    pub fn take(&mut self, input: &AiInput, forward: bool) {
        self.forward = forward;
        for &(m, v) in &input.0 {
            match m {
                Msg::Throttle => {
                    (self.pedal, self.brake) = match (v >= -0.01, self.forward) {
                        (true, true) => (v, 0.0),
                        (true, false) | (false, true) => (0.0, 1.0),
                        (false, false) => (-v, 0.0),
                    };
                }
                Msg::Brake => self.brake = v,
                Msg::Handbrake => self.handbrake = v,
                Msg::Steer => self.steer = v.clamp(-1.0, 1.0),
            }
        }
    }

    /// The alpha's controls for what it holds: the alpha's car takes its
    /// stick as the 360's pedal (+0x2C) and brake (+0x30) by its gear
    /// (ahead the stick forward is the pedal and back the brake; in
    /// reverse they swap), so the brake is the stick against its gear
    /// (taking the place of the pedal while held), the pedal the stick
    /// with it; the wheel as it is (the alpha's +1 turns the car the way
    /// the 360's +1 does, toward +x of its heading: checked with the
    /// `aidrive` example).
    pub fn controls(&self) -> crate::physics::VehicleControls {
        let way = if self.forward { 1.0 } else { -1.0 };
        let throttle = if self.brake > 0.0 {
            -way * self.brake.min(1.0)
        } else {
            way * self.pedal
        };
        crate::physics::VehicleControls {
            throttle,
            steer: self.steer,
            brake: self.handbrake > 0.0,
            parked: false,
        }
    }
}

/// A car's state as the AI reads it: where it is, its heading as a
/// direction and its velocity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarState {
    pub at: Vec3,
    pub forward: Vec3,
    pub velocity: Vec3,
}

impl CarState {
    /// Its speed along its heading, flat (0x822AC008's `s`).
    pub fn speed(&self) -> f32 {
        self.velocity.x * self.forward.x + self.velocity.z * self.forward.z
    }
}

/// The yaw of a direction (0x82855048: `atan2(x, z)`).
pub fn yaw(v: Vec3) -> f32 {
    v.x.atan2(v.z)
}

/// An angle wrapped to -pi..pi (0x82854068).
pub fn wrap(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let a = (a + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI;
    if a < -std::f32::consts::PI {
        a + tau
    } else {
        a
    }
}

/// One AI driver's memory between thinks: its eased wheel (+0x8F8).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CarDriver {
    pub wheel: f32,
}

/// What a pursuing driver chases (the mind's +0x7F4 target): where it is,
/// its velocity, and whether it is a vehicle (its kind 4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quarry {
    pub at: Vec3,
    pub velocity: Vec3,
    pub vehicle: bool,
}

impl CarDriver {
    /// 0x822AC008: drives `car` toward `to` (its height unused) at no
    /// more than `cap` m/s; `ram` its mode 1; `dt` the think's length
    /// (its +0x508).
    pub fn drive_to(&mut self, car: &CarState, to: Vec3, cap: f32, ram: bool, dt: f32) -> AiInput {
        let d = Vec3::new(to.x - car.at.x, 0.0, to.z - car.at.z);
        let dist2 = d.x * d.x + d.z * d.z;
        let s = car.speed();
        let bearing = wrap(yaw(d) - yaw(car.forward));
        let mut f = 1.0 - 4.0 / (s.abs() + 4.0);
        if s > 0.0 {
            f = -f;
        }
        let g = (f + 1.0) * 0.5;
        let reach = ((bearing.abs() * 0.5 - g).tan() + g.tan()) * 15.0;
        let inside = dist2 < reach * reach;
        let mut turn = bearing;
        if inside {
            let astern = ram && dist2 < 256.0 && bearing.abs() > 150f32.to_radians();
            if astern {
                turn = if bearing <= 0.0 {
                    bearing + std::f32::consts::PI
                } else {
                    bearing - std::f32::consts::PI
                };
            } else if bearing < -2f32.to_radians() {
                turn = -FULL_LOCK;
            } else if bearing > 2f32.to_radians() {
                turn = FULL_LOCK;
            }
        }
        if s < -0.05 {
            turn = -turn;
        }
        let slow = if ram {
            1.0 - 100.0 / (dist2 + 100.0)
        } else {
            1.0
        };
        let turning = 0.130_899_7 / (turn.abs() + 0.130_899_7) * 38.0 + 2.0;
        let mut asked = cap.min(turning);
        if asked >= slow * 40.0 {
            asked = slow * 40.0;
        }
        if inside {
            asked = -asked;
        }
        let pedal = (asked - s) * 0.14;
        // The brake branch: 0x822D3148, then 0x822D3058 (handbrake off,
        // throttle 0); else 0x822D3058 with the pedal; then the wheel.
        let input = if pedal * s <= pedal.abs() * -0.2 {
            AiInput::default()
                .send(Msg::Brake, 1.0)
                .send(Msg::Handbrake, 0.0)
                .send(Msg::Throttle, 0.0)
        } else {
            AiInput::default()
                .send(Msg::Handbrake, 0.0)
                .send(Msg::Throttle, pedal.clamp(-1.0, 1.0))
        };
        let wheel = (turn / FULL_LOCK).clamp(-1.0, 1.0);
        let steer = if s <= 3.0 {
            self.wheel = wheel;
            wheel
        } else {
            let k = (dt * 10.0).min(1.0);
            self.wheel += k * (wheel - self.wheel);
            self.wheel * self.wheel.abs()
        };
        input.send(Msg::Steer, steer)
    }

    /// One think of `CarPursue` (0x822ADE08) for a driver with no weapon
    /// of his own to use (his mind's +0x4E8 at -1: his seat has none,
    /// 0x822EE320): `car` of entity `entity`, `right` its right
    /// (0x82855ED8), `turning` its body's turn rate about the up axis.
    /// - Unless she comes at it: going away from it, under 5 m/s, or more
    ///   than 60 m off, it rams through her (toward her plus twice the
    ///   way from it to her, 0x822AC008, at up to 200 m/s at a vehicle,
    ///   15 at anyone on foot); within 20 m and within 5 m/s of her
    ///   speed, full lock into her (one way or the other by its entity
    ///   being odd or even) and the brake.
    /// - She comes at it within 60 m at 5 m/s or more: under 2 m/s it
    ///   stops (brake, handbrake off, throttle 0); otherwise a handbrake
    ///   turn across her way: barely sliding (under 0.1 m/s across), full
    ///   lock -1 with full throttle while its turn rate is at least
    ///   -0.15, else +1 off the throttle; sliding, +1 off the throttle,
    ///   the brake too when she is square to its side (more than 0.8 of
    ///   the way to her across it).
    ///
    /// The 360 reads the turn rate from its body's vtable+0x50 (vtable+0x4C
    /// is its velocity: the pursuit tick takes her velocity so): taken as
    /// its turning about up (inferred). The component 0x9CDBD62D and
    /// 0x822ECE70 (which send a driver with either to `CarAttack` instead)
    /// are not read: none is taken as there.
    pub fn pursue(
        &mut self,
        car: &CarState,
        right: Vec3,
        turning: f32,
        entity: u32,
        quarry: &Quarry,
        dt: f32,
    ) -> AiInput {
        let cap = if quarry.vehicle { 200.0 } else { 15.0 };
        let d = quarry.at - car.at;
        let dist2 = d.length_squared();
        let n = d.normalize_or_zero();
        let away = n.dot(quarry.velocity) >= 0.0;
        let quick = quarry.velocity.length();
        let closing = (quarry.velocity - car.velocity).length();
        let input = AiInput::default();
        if away || quick < 5.0 || dist2 > 3600.0 {
            if dist2 < 400.0 && closing < 5.0 {
                let lock = if entity & 1 != 0 { -1.0 } else { 1.0 };
                return input.send(Msg::Steer, lock).send(Msg::Brake, 1.0);
            }
            return self.drive_to(car, quarry.at + d * 2.0, cap, false, dt);
        }
        if car.speed() < 2.0 {
            return input
                .send(Msg::Brake, 1.0)
                .send(Msg::Handbrake, 0.0)
                .send(Msg::Throttle, 0.0);
        }
        let across = car.velocity.dot(right);
        let side = n.dot(right);
        if across.abs() < 0.1 {
            let (lock, pedal) = if turning >= -0.15 {
                (-1.0, 1.0)
            } else {
                (1.0, 0.0)
            };
            return input
                .send(Msg::Steer, lock)
                .send(Msg::Throttle, pedal)
                .send(Msg::Handbrake, 1.0);
        }
        let input = input
            .send(Msg::Steer, 1.0)
            .send(Msg::Throttle, 0.0)
            .send(Msg::Handbrake, 1.0);
        if !(-0.8..=0.8).contains(&side) {
            input.send(Msg::Brake, 1.0)
        } else {
            input
        }
    }
}

/// 0x822AA568: easing a car to a stop: the brake, until its speed along
/// its heading is within 0.3 m/s (then `None`: stopped).
pub fn slow_to_stop(car: &CarState) -> Option<AiInput> {
    (car.speed().abs() > 0.3).then(|| AiInput::default().send(Msg::Brake, 1.0))
}

/// 0x822AA630: stopping now: the wheel held, handbrake and brake on, then
/// 0x822D3058's handbrake off and throttle 0; once it moves under 1 m/s
/// the wheel straight too, and `true` (stopped). The 360 holds the wheel
/// where it points ahead of the car (0x822C2008); the alpha holds it
/// straight.
pub fn stop_now(car: &CarState) -> (AiInput, bool) {
    let input = AiInput::default()
        .send(Msg::Steer, 0.0)
        .send(Msg::Handbrake, 1.0)
        .send(Msg::Brake, 1.0)
        .send(Msg::Handbrake, 0.0)
        .send(Msg::Throttle, 0.0);
    if car.velocity.length_squared() >= 1.0 {
        (input, false)
    } else {
        (input.send(Msg::Steer, 0.0), true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn car(at: Vec3, forward: Vec3, speed: f32) -> CarState {
        CarState {
            at,
            forward,
            velocity: forward * speed,
        }
    }

    // A point dead ahead: no wheel, full throttle from rest toward the
    // cap.
    #[test]
    fn ahead_it_drives_straight_at_full_throttle() {
        let mut d = CarDriver::default();
        let c = car(Vec3::ZERO, Vec3::Z, 0.0);
        let i = d.drive_to(&c, Vec3::Z * 100.0, 20.0, false, 0.1);
        assert_eq!(i.last(Msg::Steer), Some(0.0));
        assert_eq!(i.last(Msg::Throttle), Some(1.0));
        assert_eq!(i.last(Msg::Brake), None);
        // At 20 m/s already: (20 - 20) x 0.14, nothing.
        let c = car(Vec3::ZERO, Vec3::Z, 20.0);
        let i = d.drive_to(&c, Vec3::Z * 100.0, 20.0, false, 0.1);
        assert!(i.last(Msg::Throttle).unwrap().abs() < 1e-6);
    }

    // Toward +x of its heading it steers +; past 30 degrees, full lock.
    #[test]
    fn it_steers_toward_the_point() {
        let mut d = CarDriver::default();
        let c = car(Vec3::ZERO, Vec3::Z, 0.0);
        let i = d.drive_to(&c, Vec3::new(100.0, 0.0, 100.0), 40.0, false, 0.1);
        assert_eq!(i.last(Msg::Steer), Some(1.0));
        let i = d.drive_to(&c, Vec3::new(-10.0, 0.0, 100.0), 40.0, false, 0.1);
        let s = i.last(Msg::Steer).unwrap();
        assert!(s < 0.0 && s > -1.0, "{s}");
    }

    // Going fast at a point behind it: the brake, then the throttle's 0,
    // which in its forward gear lets the brake go again (the car's input
    // takes them in order).
    #[test]
    fn a_point_behind_at_speed_sends_the_brake_then_lets_go() {
        let mut d = CarDriver::default();
        let c = car(Vec3::ZERO, Vec3::Z, 20.0);
        let i = d.drive_to(&c, Vec3::NEG_Z * 10.0, 40.0, false, 0.1);
        assert_eq!(i.last(Msg::Brake), Some(1.0));
        let mut held = CarInputs::default();
        held.take(&i, true);
        assert_eq!((held.pedal, held.brake), (0.0, 0.0));
    }

    // The car's input: a throttle against its gear brakes (the alpha's
    // stick against the gear); in reverse it backs (the stick back).
    #[test]
    fn the_cars_input_brakes_against_its_gear_then_backs() {
        let back = AiInput::default().send(Msg::Throttle, -0.5);
        let mut held = CarInputs::default();
        held.take(&back, true);
        assert_eq!((held.pedal, held.brake), (0.0, 1.0));
        assert_eq!(held.controls().throttle, -1.0);
        held.take(&back, false);
        assert_eq!((held.pedal, held.brake), (0.5, 0.0));
        assert_eq!(held.controls().throttle, -0.5);
        // Ahead in reverse: the brake, the stick forward.
        held.take(&AiInput::default().send(Msg::Throttle, 0.3), false);
        assert_eq!(held.controls().throttle, 1.0);
    }

    #[test]
    fn stopping() {
        assert!(slow_to_stop(&car(Vec3::ZERO, Vec3::Z, 0.2)).is_none());
        let i = slow_to_stop(&car(Vec3::ZERO, Vec3::Z, 5.0)).unwrap();
        assert_eq!(i.last(Msg::Brake), Some(1.0));
        assert!(stop_now(&car(Vec3::ZERO, Vec3::Z, 0.5)).1);
        assert!(!stop_now(&car(Vec3::ZERO, Vec3::Z, 3.0)).1);
    }

    // CarPursue: she drives away from it far off: it rams through her;
    // she comes at it close and fast: a handbrake turn.
    #[test]
    fn a_pursuer_rams_or_turns_across() {
        let mut d = CarDriver::default();
        let c = car(Vec3::ZERO, Vec3::Z, 10.0);
        let her = Quarry {
            at: Vec3::new(0.0, 0.0, 100.0),
            velocity: Vec3::Z * 20.0,
            vehicle: true,
        };
        let i = d.pursue(&c, Vec3::NEG_X, 0.0, 7, &her, 0.1);
        assert!(i.last(Msg::Throttle).unwrap() > 0.0);
        let her = Quarry {
            at: Vec3::new(0.0, 0.0, 40.0),
            velocity: Vec3::NEG_Z * 20.0,
            vehicle: true,
        };
        let i = d.pursue(&c, Vec3::NEG_X, 0.0, 7, &her, 0.1);
        assert_eq!(i.last(Msg::Handbrake), Some(1.0));
        assert_eq!(i.last(Msg::Steer), Some(-1.0));
    }
}
