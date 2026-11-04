//! An AI helicopter pilot as the 360 code flies one: its mind keeps a
//! small state of the helicopter (0x822BCDC8, each think) and its tasks
//! work the four stick messages a player's pad sends (lift 0xE6FA6B1B,
//! yaw 0x30FBBF64, ahead 0x330850A1, sideways 0x11C78A3C) through a
//! handful of controllers: hold a height (0x822D3288), a heading
//! (0x822D3370), a speed ahead (0x822D3438) and sideways (0x822D34D8);
//! hover (0x822D35A0) and fly to a point (0x822D39F0, 0x822D3798,
//! 0x822D3638). A message holds until the next sets it again, so each
//! controller here leaves the parts of the stick it does not send.
//!
//! Provenance: all from the 360 code, apart from where it says
//! otherwise. The 360's mind thinks each game frame and its height and
//! heading terms are differences since its last think, so a pilot
//! thinking at another rate reacts by those as its rate says; the
//! alpha's thinks once each of its frames (as the 360's, the rate
//! itself not traced).

use m2_math::glam::{Quat, Vec3};

use crate::tuning::{FlightTuning, PilotTuning};

/// The four stick messages as the 360 code sends them, each -1 to 1:
/// climb (+), turn right (+), ahead (+) and slide right (+).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PilotStick {
    pub lift: f32,
    pub yaw: f32,
    pub ahead: f32,
    pub right: f32,
}

/// The helicopter's state an AI pilot's mind keeps (its 0xD4 bytes at
/// mind+0x960), as far as flying needs it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeliPilot {
    pub tuning: PilotTuning,
    /// Its fastest ahead, sideways and climb, m/s (`_HelicopterPhysicsAi`'s
    /// `MaxFwdSpeed`, `MaxRightSpeed`, `MaxElevationSpeed`: +0x44, +0x48,
    /// +0x4C, from 0x822BCC08).
    pub fastest: (f32, f32, f32),
    /// Where it is and which way it heads, at this think and the last
    /// (the mind's +0x50C and +0x518; its height +0x10 and +0x14).
    pub at: Vec3,
    pub turn: Quat,
    pub heading: f32,
    pub last_heading: f32,
    pub height: f32,
    pub last_height: f32,
    /// Its velocity along its own z (ahead), x and y, m/s (+0x38, +0x3C,
    /// +0x40).
    pub along: Vec3,
    /// The ground under it (+0x8; +0xC is that less 10), and how far it
    /// is over it (+0x4), m.
    pub ground: f32,
    pub clearance: f32,
    /// The heading it flies to face (+0xC8).
    pub face: f32,
    /// The stick as it was last sent.
    pub stick: PilotStick,
    thought: bool,
}

/// `x` held to -1..1, as each controller holds its message.
fn unit(x: f32) -> f32 {
    x.clamp(-1.0, 1.0)
}

/// An angle wrapped to -pi..pi (0x82854068).
fn wrap(a: f32) -> f32 {
    let t = std::f32::consts::TAU;
    a - t * ((a + std::f32::consts::PI) / t).floor()
}

/// The heading of a direction, radians (0x82855048: from its x and z).
pub fn heading_of(v: Vec3) -> f32 {
    v.x.atan2(v.z)
}

impl HeliPilot {
    /// A pilot for a helicopter with its `AiHelicopter` record and its AI
    /// handling (its fastest speeds).
    pub fn new(tuning: PilotTuning, flight: &FlightTuning) -> Self {
        Self {
            tuning,
            fastest: (flight.forward, flight.sideways, flight.climb),
            at: Vec3::ZERO,
            turn: Quat::IDENTITY,
            heading: 0.0,
            last_heading: 0.0,
            height: 0.0,
            last_height: 0.0,
            along: Vec3::ZERO,
            ground: 0.0,
            clearance: 0.0,
            face: 0.0,
            stick: PilotStick::default(),
            thought: false,
        }
    }

    /// Takes in the helicopter at a think (0x822BBAB8, 0x822BCDC8): where
    /// it is, how it is turned, its velocity (world), and the ground
    /// straight under it (the 360 casts from 1000 m down to -1000 m,
    /// 0x822ED8C0; `None` keeps the last it found).
    pub fn sense(&mut self, at: Vec3, turn: Quat, velocity: Vec3, ground: Option<f32>) {
        let heading = heading_of(turn * Vec3::Z);
        if self.thought {
            self.last_heading = self.heading;
            self.last_height = self.height;
        } else {
            self.last_heading = heading;
            self.last_height = at.y;
            self.thought = true;
        }
        self.at = at;
        self.turn = turn;
        self.heading = heading;
        self.height = at.y;
        self.along = Vec3::new(
            velocity.dot(turn * Vec3::X),
            velocity.dot(turn * Vec3::Y),
            velocity.dot(turn * Vec3::Z),
        );
        if let Some(g) = ground {
            self.ground = g;
        }
        // The higher of +0x8 and +0xC (10 below it): the ground itself.
        self.clearance = (at.y - self.ground).max(0.0) - (at.y - self.height);
    }

    /// Climbs or comes down towards `height` (0x822D3288): unless
    /// `forced`, no lower than its `MinAltitude` over the ground while
    /// under it. Its lift: 0.5 x the height still to go less 25 x how far
    /// it rose since its last think.
    pub fn hold_height(&mut self, height: f32, forced: bool) {
        let mut want = height;
        if !forced && self.clearance < self.tuning.min_altitude {
            want = self.ground + self.tuning.min_altitude;
        }
        let e = want - self.height;
        let rose = (want - self.last_height) - e;
        self.stick.lift = unit(-rose * 25.0 + e * 0.5);
    }

    /// Turns towards `heading` (0x822D3370): 10 x the turn still to go
    /// less 10 x how far it turned since its last think, as a turn right.
    pub fn hold_heading(&mut self, heading: f32) {
        let e = wrap(heading - self.heading);
        let turned = wrap(e - (heading - self.last_heading));
        self.stick.yaw = -unit(turned * 10.0 + e * 10.0);
    }

    /// Works towards `speed` ahead, m/s (0x822D3438): 2 x the speed still
    /// to go.
    pub fn hold_ahead(&mut self, speed: f32) {
        self.stick.ahead = unit((speed - self.along.z) * 2.0);
    }

    /// Works towards `speed` along its own +x (its left), m/s
    /// (0x822D34D8): a slide right of -5 x the speed still to go (its smoothing, flag 0x02 of +0xD0, left off as
    /// a fresh pilot has it).
    pub fn hold_across(&mut self, speed: f32) {
        self.stick.right = unit((speed - self.along.x) * -5.0);
    }

    /// Hovers at `height` (0x822D35A0), over the ground if `over_ground`,
    /// else a world height: holds it (forced), holds its heading as it
    /// is, and works to no speed ahead or across.
    pub fn hover(&mut self, height: f32, over_ground: bool) -> PilotStick {
        let height = if over_ground {
            height + self.ground
        } else {
            height
        };
        self.hold_height(height, true);
        self.hold_heading(self.heading);
        self.hold_across(0.0);
        self.hold_ahead(0.0);
        self.stick
    }

    /// Flies to `target` (0x822D39F0) at `haste` (0 to 1) of its fastest,
    /// slowing over its last `MaxFwdSpeed` metres, turning to face the
    /// way if `facing`; within half a metre it hovers at its height.
    pub fn fly_to(&mut self, target: Vec3, haste: f32, facing: bool) -> PilotStick {
        let d = Vec3::new(
            target.x - self.at.x,
            target.y - self.height,
            target.z - self.at.z,
        );
        self.face = heading_of(d);
        let gap = d.length();
        if gap < 0.5 {
            return self.hover(target.y, false);
        }
        let reach = self.fastest.0;
        let haste = if gap >= reach {
            haste
        } else {
            gap / reach * haste
        };
        self.fly_along(haste, d, facing)
    }

    /// Flies along `d` at `haste` of its fastest (0x822D3798): faces its
    /// way if `facing`, then works to the velocity 0x822D3638 asks along
    /// its own axes, its climb by 2 x the climb still to go.
    fn fly_along(&mut self, haste: f32, d: Vec3, facing: bool) -> PilotStick {
        if facing {
            self.hold_heading(self.face);
        }
        let (ahead, across, up) = self.velocity_along(haste, d);
        self.hold_ahead(ahead);
        self.hold_across(across);
        self.stick.lift = unit((up - self.along.y) * 2.0);
        self.stick
    }

    /// The velocity along its own z, x and y it flies `d` at `haste`
    /// with (0x822D3638): the way `d` points on its own axes, at `haste`
    /// x its fastest ahead, slowed so neither the part across nor the
    /// climb passes its fastest those ways.
    fn velocity_along(&self, haste: f32, d: Vec3) -> (f32, f32, f32) {
        let (fwd, right, climb) = self.fastest;
        let dir = d.normalize_or_zero();
        let (x, y, z) = (
            dir.dot(self.turn * Vec3::X),
            dir.dot(self.turn * Vec3::Y),
            dir.dot(self.turn * Vec3::Z),
        );
        let mut s = haste;
        if x != 0.0 {
            s = s.min((right / fwd / x).abs());
        }
        if y != 0.0 {
            s = s.min((climb / fwd / y).abs());
        }
        (fwd * s * z, fwd * s * x, fwd * s * y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pilot() -> HeliPilot {
        HeliPilot::new(PilotTuning::DEFAULT, &FlightTuning::DEFAULT)
    }

    // Its lift is 0.5 x the height to go less 25 x the rise since its
    // last think (0x822D3288); under its `MinAltitude` over the ground
    // it aims for that instead, unless forced.
    #[test]
    fn its_lift_works_on_the_height_to_go_and_the_rise() {
        let mut p = pilot();
        p.sense(
            Vec3::new(0.0, 20.0, 0.0),
            Quat::IDENTITY,
            Vec3::ZERO,
            Some(0.0),
        );
        p.sense(
            Vec3::new(0.0, 20.02, 0.0),
            Quat::IDENTITY,
            Vec3::ZERO,
            Some(0.0),
        );
        p.hold_height(21.0, true);
        let want = (0.98 * 0.5 - 0.02 * 25.0f32).clamp(-1.0, 1.0);
        assert!((p.stick.lift - want).abs() < 1e-4, "{}", p.stick.lift);
        // 3 m up, asked down to 2 m: held up to its 10 m.
        p.sense(
            Vec3::new(0.0, 3.0, 0.0),
            Quat::IDENTITY,
            Vec3::ZERO,
            Some(0.0),
        );
        p.sense(
            Vec3::new(0.0, 3.0, 0.0),
            Quat::IDENTITY,
            Vec3::ZERO,
            Some(0.0),
        );
        p.hold_height(2.0, false);
        assert_eq!(p.stick.lift, 1.0);
        p.hold_height(2.0, true);
        assert_eq!(p.stick.lift, -0.5);
    }

    // Asked to face left of where it heads it turns left (a negative
    // turn right), and its turn since the last think holds it back
    // (0x822D3370).
    #[test]
    fn it_turns_to_the_heading_asked() {
        let mut p = pilot();
        p.sense(Vec3::ZERO, Quat::IDENTITY, Vec3::ZERO, None);
        p.hold_heading(0.05);
        assert!((p.stick.yaw + 0.5).abs() < 1e-4, "{}", p.stick.yaw);
        let turned = Quat::from_rotation_y(0.04);
        p.sense(Vec3::ZERO, turned, Vec3::ZERO, None);
        p.hold_heading(0.05);
        assert!((p.stick.yaw + (0.1 - 0.4)).abs() < 1e-3, "{}", p.stick.yaw);
    }

    // Flying a way across it, its speed across is held to its fastest
    // that way, the speed ahead with it (0x822D3638).
    #[test]
    fn its_speed_across_is_held_to_its_fastest_that_way() {
        let mut p = pilot();
        p.sense(Vec3::ZERO, Quat::IDENTITY, Vec3::ZERO, Some(-50.0));
        let (ahead, across, up) = p.velocity_along(1.0, Vec3::new(1.0, 0.0, 1.0));
        assert!((across - 20.0).abs() < 1e-3, "{across}");
        assert!((ahead - 20.0).abs() < 1e-3, "{ahead}");
        assert!(up.abs() < 1e-4);
        // Straight ahead: its fastest ahead.
        let (ahead, _, _) = p.velocity_along(1.0, Vec3::Z);
        assert!((ahead - 30.0).abs() < 1e-3);
    }
}
