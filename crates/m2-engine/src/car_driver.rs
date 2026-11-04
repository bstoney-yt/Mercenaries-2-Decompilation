//! An AI driver's hands and feet: how the 360's car AI steers, throttles
//! and brakes a car toward a point (0x822AC008), and how it stops one
//! (0x822AA568, 0x822AA630).
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
//! It sends the car the 360's messages: throttle 0x460C5913, brake
//! 0x55B8E0A1, handbrake 0x37086E0A, steering 0x3483DBF1 (taken by the
//! car's input, 0x82189AC8). [`AiInput`] keeps them so.
//!
//! Unknown: the mind's +0x320 (0x822AC008 caps the car at 5 m/s while it
//! is under 0.5; who writes it is not found): taken as at least 0.5
//! (`careful` false). Its +0x344 (a reach from the car's width, 1.5 x
//! 3..6.5 of it) is not used here.

use m2_math::glam::Vec3;

/// The wheel's full lock, radians (0x822AC008's 30 degrees).
pub const FULL_LOCK: f32 = 30.0 * std::f32::consts::PI / 180.0;

/// What the AI asks of a car this think, as the 360 sends it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AiInput {
    /// -1 back to 1 ahead (0x460C5913).
    pub throttle: f32,
    /// 0 or 1 (0x55B8E0A1).
    pub brake: f32,
    /// 0 or 1 (0x37086E0A).
    pub handbrake: f32,
    /// -1 to 1, +1 toward +x of its heading (0x3483DBF1; the bearing
    /// over full lock).
    pub steer: f32,
}

impl AiInput {
    /// The alpha's controls for it, the car going `speed` m/s along its
    /// heading. The 360 car takes its throttle message last (0x82189AC8):
    /// in its forward gear a throttle of 0 or more lets go of the brake,
    /// so a think's brake holds only while it backs (the alpha's gear is
    /// the way it rolls: inferred); a brake is a throttle against the
    /// motion; the wheel as it is (the alpha's +1 turns the car the way
    /// the 360's +1 does, toward +x of its heading: checked with the
    /// `aidrive` example).
    pub fn controls(&self, speed: f32) -> crate::physics::VehicleControls {
        let backing = speed < 0.0;
        let throttle = if self.brake > 0.0 && backing {
            1.0
        } else {
            self.throttle
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
        let mut input = AiInput::default();
        if pedal * s <= pedal.abs() * -0.2 {
            input.brake = 1.0;
            input.throttle = 0.0;
        } else {
            input.throttle = pedal.clamp(-1.0, 1.0);
        }
        let wheel = (turn / FULL_LOCK).clamp(-1.0, 1.0);
        input.steer = if s <= 3.0 {
            self.wheel = wheel;
            wheel
        } else {
            let k = (dt * 10.0).min(1.0);
            self.wheel += k * (wheel - self.wheel);
            self.wheel * self.wheel.abs()
        };
        input
    }
}

/// 0x822AA568: easing a car to a stop: the brake, until its speed along
/// its heading is within 0.3 m/s (then `None`: stopped).
pub fn slow_to_stop(car: &CarState) -> Option<AiInput> {
    let s = car.speed();
    (s.abs() > 0.3).then_some(AiInput {
        brake: 1.0,
        ..AiInput::default()
    })
}

/// 0x822AA630: stopping now: the wheel held, handbrake and brake on, no
/// throttle; `None` once it moves under 1 m/s. The 360 holds the wheel
/// where it points ahead of the car (0x822C2008); the alpha holds it
/// straight.
pub fn stop_now(car: &CarState) -> Option<AiInput> {
    (car.velocity.length_squared() >= 1.0).then_some(AiInput {
        brake: 1.0,
        handbrake: 1.0,
        ..AiInput::default()
    })
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
        let i = d.drive_to(
            &car(Vec3::ZERO, Vec3::Z, 0.0),
            Vec3::Z * 100.0,
            20.0,
            false,
            0.1,
        );
        assert_eq!(i.steer, 0.0);
        assert_eq!(i.throttle, 1.0);
        assert_eq!(i.brake, 0.0);
        // At 20 m/s already: (20 - 20) x 0.14, nothing.
        let i = d.drive_to(
            &car(Vec3::ZERO, Vec3::Z, 20.0),
            Vec3::Z * 100.0,
            20.0,
            false,
            0.1,
        );
        assert!(i.throttle.abs() < 1e-6);
    }

    // Toward +x of its heading it steers +; past 30 degrees, full lock.
    #[test]
    fn it_steers_toward_the_point() {
        let mut d = CarDriver::default();
        let to = Vec3::new(100.0, 0.0, 100.0);
        let i = d.drive_to(&car(Vec3::ZERO, Vec3::Z, 0.0), to, 40.0, false, 0.1);
        assert_eq!(i.steer, 1.0);
        let to = Vec3::new(-10.0, 0.0, 100.0);
        let i = d.drive_to(&car(Vec3::ZERO, Vec3::Z, 0.0), to, 40.0, false, 0.1);
        assert!(i.steer < 0.0 && i.steer > -1.0, "{}", i.steer);
    }

    // Going fast at a point behind it: the brake.
    #[test]
    fn a_point_behind_at_speed_brakes() {
        let mut d = CarDriver::default();
        let i = d.drive_to(
            &car(Vec3::ZERO, Vec3::Z, 20.0),
            Vec3::NEG_Z * 10.0,
            40.0,
            false,
            0.1,
        );
        assert_eq!(i.brake, 1.0);
        assert_eq!(i.throttle, 0.0);
    }

    #[test]
    fn stopping() {
        assert!(slow_to_stop(&car(Vec3::ZERO, Vec3::Z, 0.2)).is_none());
        assert_eq!(
            slow_to_stop(&car(Vec3::ZERO, Vec3::Z, 5.0)).unwrap().brake,
            1.0
        );
        assert!(stop_now(&car(Vec3::ZERO, Vec3::Z, 0.5)).is_none());
        assert_eq!(
            stop_now(&car(Vec3::ZERO, Vec3::Z, 3.0)).unwrap().handbrake,
            1.0
        );
    }
}
