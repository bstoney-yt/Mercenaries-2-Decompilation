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
    /// The way it steers when going round things (+0x50, +0x54: a
    /// heading and a climb, radians), and how long it keeps steering by
    /// them, and since something was in its way, s (+0x98, +0x9C; each
    /// think takes its time off them, 0x822BCDC8).
    pub steer_heading: f32,
    pub steer_climb: f32,
    pub steering: f32,
    pub blocked: f32,
    /// It has been off the ground half a second or more (the mind's
    /// +0x70D bit 0x80, from its physics' ground time, 0x822BCDC8).
    pub airborne: bool,
    /// What it carries slung under it, if anything ([`Slung`]).
    pub load: Option<Slung>,
    thought: bool,
}

/// A load slung under a helicopter, as its pilot's state keeps it
/// (+0xB0 its place, +0xBC the load, flag 0x04 of +0xD0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slung {
    /// Where the load is.
    pub at: Vec3,
    /// How far its underside hangs under the helicopter's origin, m (the
    /// 360: its hook's offset plus 0.75 of the load's largest side,
    /// 0x822BCDC8).
    pub below: f32,
    /// It hangs under the helicopter: within 2 m of it across, and the
    /// hook within 4 m of where it should be (0x822BCDC8's flag 0x04).
    pub under: bool,
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

/// What a pilot looks at in the world as it goes round things.
pub trait PilotWorld {
    /// The first solid thing on the line from `from` to `to`: where.
    fn ground(&mut self, from: Vec3, to: Vec3) -> Option<Vec3>;
    /// How far, of `max` metres along `way`, a box of half extents
    /// `half` turned `turn` goes from `origin` before it meets something
    /// (1 if nothing).
    fn sweep(&mut self, origin: Vec3, turn: Quat, half: Vec3, way: Vec3, max: f32) -> f32;
}

/// The alpha's physics as a pilot sees it, past its own helicopter.
pub struct Sight<'a> {
    pub physics: &'a crate::Physics,
    pub own: crate::VehicleId,
}

impl std::fmt::Debug for Sight<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sight").field("own", &self.own).finish()
    }
}

impl PilotWorld for Sight<'_> {
    fn ground(&mut self, from: Vec3, to: Vec3) -> Option<Vec3> {
        let d = to - from;
        self.physics
            .cast_ray_past(from, d, d.length(), self.own)
            .map(|(t, _)| from + d.normalize_or_zero() * t)
    }

    fn sweep(&mut self, origin: Vec3, turn: Quat, half: Vec3, way: Vec3, max: f32) -> f32 {
        self.physics
            .cast_box(origin, turn, half, way, max, Some(self.own))
            .map_or(1.0, |t| (t / max).clamp(0.0, 1.0))
    }
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
            steer_heading: 0.0,
            steer_climb: 0.0,
            steering: 0.0,
            blocked: 0.0,
            airborne: false,
            load: None,
            thought: false,
        }
    }

    /// Takes in the helicopter at a think (0x822BBAB8, 0x822BCDC8): where
    /// it is, how it is turned, its velocity (world), and the ground
    /// straight under it (the 360 casts from 1000 m down to -1000 m,
    /// 0x822ED8C0; `None` keeps the last it found).
    pub fn sense(&mut self, at: Vec3, turn: Quat, velocity: Vec3, ground: Option<f32>, dt: f32) {
        for t in [&mut self.steering, &mut self.blocked] {
            if *t > 0.0 {
                *t -= dt;
            }
        }
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
        // With a load its height is the load's underside's (+0x10).
        self.height = at.y - self.load.map_or(0.0, |l| l.below);
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

    /// Takes off (the task `HeliTakeoff`, 0x822AF2F8): the climb held
    /// until it is 10 m over the ground and off it; then the climb let go
    /// and done (`true`).
    pub fn take_off(&mut self) -> (PilotStick, bool) {
        if self.clearance < 10.0 || !self.airborne {
            self.stick.lift = 1.0;
            (self.stick, false)
        } else {
            self.stick.lift = 0.0;
            (self.stick, true)
        }
    }

    /// Delivers its load to `target` (the task `HeliDeliver`, 0x822AFA18,
    /// for the scripts' `Ai.Deliver`, request 0x1A whose goal bit 40 only
    /// it gives): over `half_wide` (0x2C0, half its wider side) away
    /// across, it flies round things to the point; within 2 m across and
    /// its load's underside within `drop` m over the ground (the order's
    /// value, 0.5 when none, 0x822DC468), it lets go (`true`; the message
    /// 0xB422F25E) and hovers its `MinAltitude` up; else it comes down
    /// onto the point. With nothing slung it flies on 1000 m ahead the
    /// way it heads (0x822AF900). The task never ends of itself. Which
    /// of the facing and floor flags the 360 hands 0x822D43A8 and
    /// 0x822D39F0 here is not traced (taken as below).
    #[allow(clippy::too_many_arguments)]
    pub fn deliver(
        &mut self,
        target: Vec3,
        drop: f32,
        half_wide: f32,
        size: (f32, f32),
        world: &mut dyn PilotWorld,
        dt: f32,
    ) -> (PilotStick, bool) {
        let Some(load) = self.load else {
            let ahead = Vec3::new(self.heading.sin(), 0.0, self.heading.cos());
            let away = self.at + ahead * 1000.0;
            let stick = self.fly_round_to(away, 1.0, true, true, size, world, dt);
            return (stick, false);
        };
        let across = (target - load.at).with_y(0.0).length();
        if across > half_wide {
            self.face = self.heading;
            let stick = self.fly_round_to(target, 1.0, true, false, size, world, dt);
            return (stick, false);
        }
        if load.under && across <= 2.0 && self.clearance <= drop {
            let stick = self.hover(self.tuning.min_altitude, true);
            return (stick, true);
        }
        (self.fly_to(target, 1.0, false), false)
    }

    /// Comes down to land (0x822D4848): holds its heading, works to no
    /// speed ahead or across, and comes down at a lift of -(0.1 + ((its
    /// clearance - 0.2) / 10)², the square at most 0.9), at most -0.5;
    /// within 0.2 m of the ground, -1.
    pub fn land(&mut self) -> PilotStick {
        self.hold_heading(self.heading);
        self.hold_across(0.0);
        self.hold_ahead(0.0);
        self.stick.lift = if self.clearance > 0.2 {
            let s = ((self.clearance - 0.2) * 0.1).powi(2).min(0.9);
            -(s + 0.1).min(0.5)
        } else {
            -1.0
        };
        self.stick
    }

    /// Flies to `target` going round what is in its way (0x822D43A8),
    /// at `haste`, facing its way if `facing`; held its `MinAltitude`
    /// over the ground there if `above` (0x822D3B60: a ray from 2 m over
    /// the point to that far under it). `size` is its box's height and
    /// its wider of its width and length (the mind's +0x2A0 box; +0x2C0
    /// is half the wider), `dt` the think's step (at most 1/30 s here).
    ///
    /// From its box's middle it heads for the point by a way it turns
    /// to: straight there unless it was steering in the last 2 s, else by
    /// its last way, turned towards the point by at most a quarter turn
    /// a second in heading and in climb. A box as wide and long as its wider side and as tall as it is
    /// (0x82166880 halves them for Havok) is swept that way, up to 50
    /// m or its distance less half its box's diagonal; anything in the
    /// way marks it blocked for 1 s and tips its way up by (1 - the share
    /// of the sweep made) x 2 turns a second. It flies to the point as
    /// far along its way as the target is ([`fly_to`](Self::fly_to)).
    /// Not ported: the turn away from what the fan of eight rays to
    /// either side of its way finds (0x822D3FC8, 0x822D40E0), whose ray
    /// results are not traced; it turns by none.
    #[allow(clippy::too_many_arguments)]
    pub fn fly_round_to(
        &mut self,
        target: Vec3,
        haste: f32,
        facing: bool,
        above: bool,
        size: (f32, f32),
        world: &mut dyn PilotWorld,
        dt: f32,
    ) -> PilotStick {
        let dt = dt.min(1.0 / 30.0);
        let (box_height, wide) = size;
        let tall = (self.at.y - self.height) + box_height;
        let lift = tall * 0.5 - (tall - box_height);
        let origin = Vec3::new(self.at.x, self.at.y + lift, self.at.z);
        let mut target = target;
        if above {
            let floor = self.tuning.min_altitude;
            if let Some(g) = world.ground(target + Vec3::Y * 2.0, target - Vec3::Y * floor) {
                target = g + Vec3::Y * floor;
            }
        }
        target.y += lift;
        let d = target - origin;
        let gap = d.length();
        let flat = d.with_y(0.0).length();
        let want_heading = heading_of(d);
        let want_climb = if flat > 0.0 { (d.y / flat).atan() } else { 0.0 };
        let way = if self.steering > 0.0 {
            let ahead = Vec3::new(self.steer_heading.sin(), 0.0, self.steer_heading.cos());
            let across = Vec3::new(-self.steer_heading.cos(), 0.0, self.steer_heading.sin());
            let way = Quat::from_axis_angle(across, self.steer_climb) * ahead;
            let most = std::f32::consts::FRAC_PI_2 * dt;
            self.steer_heading += wrap(want_heading - self.steer_heading).clamp(-most, most);
            self.steer_climb += wrap(want_climb - self.steer_climb).clamp(-most, most);
            way
        } else {
            self.steer_heading = want_heading;
            self.steer_climb = want_climb;
            d / (gap + 0.01)
        };
        self.steering = 2.0;
        let reach = gap - (tall * tall + wide * wide + wide * wide).sqrt() * 0.5;
        let sweep = reach.min(50.0);
        let made = if sweep > 0.0 {
            let made = world.sweep(
                origin,
                self.turn,
                Vec3::new(wide, tall, wide) * 0.5,
                way,
                sweep,
            );
            if made < 1.0 {
                self.blocked = 1.0;
            }
            made
        } else {
            1.0
        };
        let point = origin + way * gap;
        if made < 1.0 || self.blocked > 0.0 {
            self.steer_climb += (1.0 - made) * std::f32::consts::TAU * dt;
        }
        self.fly_to(point, haste, facing)
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
            1.0 / 60.0,
        );
        p.sense(
            Vec3::new(0.0, 20.02, 0.0),
            Quat::IDENTITY,
            Vec3::ZERO,
            Some(0.0),
            1.0 / 60.0,
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
            1.0 / 60.0,
        );
        p.sense(
            Vec3::new(0.0, 3.0, 0.0),
            Quat::IDENTITY,
            Vec3::ZERO,
            Some(0.0),
            1.0 / 60.0,
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
        p.sense(Vec3::ZERO, Quat::IDENTITY, Vec3::ZERO, None, 1.0 / 60.0);
        p.hold_heading(0.05);
        assert!((p.stick.yaw + 0.5).abs() < 1e-4, "{}", p.stick.yaw);
        let turned = Quat::from_rotation_y(0.04);
        p.sense(Vec3::ZERO, turned, Vec3::ZERO, None, 1.0 / 60.0);
        p.hold_heading(0.05);
        assert!((p.stick.yaw + (0.1 - 0.4)).abs() < 1e-3, "{}", p.stick.yaw);
    }

    // Flying a way across it, its speed across is held to its fastest
    // that way, the speed ahead with it (0x822D3638).
    #[test]
    fn its_speed_across_is_held_to_its_fastest_that_way() {
        let mut p = pilot();
        p.sense(
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::ZERO,
            Some(-50.0),
            1.0 / 60.0,
        );
        let (ahead, across, up) = p.velocity_along(1.0, Vec3::new(1.0, 0.0, 1.0));
        assert!((across - 20.0).abs() < 1e-3, "{across}");
        assert!((ahead - 20.0).abs() < 1e-3, "{ahead}");
        assert!(up.abs() < 1e-4);
        // Straight ahead: its fastest ahead.
        let (ahead, _, _) = p.velocity_along(1.0, Vec3::Z);
        assert!((ahead - 30.0).abs() < 1e-3);
    }

    // Coming down to land it eases its descent as it nears the ground
    // (0x822D4848): 20 m up -0.5, 2 m up -0.13, at 0.2 m all it can.
    #[test]
    fn it_comes_down_slower_near_the_ground() {
        let mut p = pilot();
        let at = |p: &mut HeliPilot, y: f32| {
            p.sense(
                Vec3::new(0.0, y, 0.0),
                Quat::IDENTITY,
                Vec3::ZERO,
                Some(0.0),
                1.0 / 60.0,
            );
            p.land().lift
        };
        assert_eq!(at(&mut p, 20.0), -0.5);
        assert!((at(&mut p, 2.0) + (0.1 + 0.18f32 * 0.18)).abs() < 1e-4);
        assert_eq!(at(&mut p, 0.1), -1.0);
    }

    struct Wall;

    impl PilotWorld for Wall {
        fn ground(&mut self, _: Vec3, _: Vec3) -> Option<Vec3> {
            None
        }
        fn sweep(&mut self, _: Vec3, _: Quat, _: Vec3, _: Vec3, _: f32) -> f32 {
            0.5
        }
    }

    // Something half way along its sweep marks it blocked and tips its
    // way up by half of two turns a second (0x822D43A8).
    #[test]
    fn blocked_it_tips_its_way_up() {
        let mut p = pilot();
        p.sense(
            Vec3::new(0.0, 30.0, 0.0),
            Quat::IDENTITY,
            Vec3::ZERO,
            Some(0.0),
            1.0 / 60.0,
        );
        let dt = 1.0 / 60.0;
        p.fly_round_to(
            Vec3::new(0.0, 30.0, 200.0),
            1.0,
            true,
            false,
            (3.0, 12.0),
            &mut Wall,
            dt,
        );
        assert_eq!(p.blocked, 1.0);
        assert!(
            (p.steer_climb - 0.5 * std::f32::consts::TAU * dt).abs() < 1e-5,
            "{}",
            p.steer_climb
        );
        assert_eq!(p.steering, 2.0);
    }

    // It takes off with the climb held until 10 m up and off the ground,
    // then lets go of it (0x822AF2F8).
    #[test]
    fn it_takes_off_to_10_m() {
        let mut p = pilot();
        p.sense(
            Vec3::new(0.0, 4.0, 0.0),
            Quat::IDENTITY,
            Vec3::ZERO,
            Some(0.0),
            1.0 / 60.0,
        );
        p.airborne = true;
        assert_eq!(
            p.take_off(),
            (
                PilotStick {
                    lift: 1.0,
                    ..PilotStick::default()
                },
                false
            )
        );
        p.sense(
            Vec3::new(0.0, 11.0, 0.0),
            Quat::IDENTITY,
            Vec3::ZERO,
            Some(0.0),
            1.0 / 60.0,
        );
        p.airborne = false;
        assert!(!p.take_off().1);
        p.airborne = true;
        let (stick, done) = p.take_off();
        assert!(done && stick.lift == 0.0);
    }

    struct Open;

    impl PilotWorld for Open {
        fn ground(&mut self, _: Vec3, _: Vec3) -> Option<Vec3> {
            None
        }
        fn sweep(&mut self, _: Vec3, _: Quat, _: Vec3, _: Vec3, _: f32) -> f32 {
            1.0
        }
    }

    // Delivering, it lets its load go once over the point and its load's
    // underside within the drop height, then hovers its `MinAltitude`
    // up; with nothing slung it flies on ahead (0x822AFA18, 0x822AF900).
    #[test]
    fn it_lets_its_load_go_over_the_point_and_low() {
        let mut p = pilot();
        let target = Vec3::new(0.0, 0.0, 1.0);
        p.load = Some(Slung {
            at: Vec3::new(0.0, 0.3, 0.0),
            below: 5.0,
            under: true,
        });
        p.sense(
            Vec3::new(0.0, 5.3, 0.0),
            Quat::IDENTITY,
            Vec3::ZERO,
            Some(0.0),
            1.0 / 60.0,
        );
        let (_, gone) = p.deliver(target, 0.5, 6.0, (3.0, 12.0), &mut Open, 1.0 / 60.0);
        assert!(gone);
        // Too high yet: it comes down onto the point.
        p.sense(
            Vec3::new(0.0, 9.0, 0.0),
            Quat::IDENTITY,
            Vec3::ZERO,
            Some(0.0),
            1.0 / 60.0,
        );
        let (stick, gone) = p.deliver(target, 0.5, 6.0, (3.0, 12.0), &mut Open, 1.0 / 60.0);
        assert!(!gone && stick.lift < 0.0, "{stick:?}");
        // Unladen: on ahead, the way it heads.
        p.load = None;
        p.sense(
            Vec3::new(0.0, 30.0, 0.0),
            Quat::IDENTITY,
            Vec3::ZERO,
            Some(0.0),
            1.0 / 60.0,
        );
        let (stick, gone) = p.deliver(target, 0.5, 6.0, (3.0, 12.0), &mut Open, 1.0 / 60.0);
        assert!(!gone && stick.ahead > 0.9, "{stick:?}");
    }
}
