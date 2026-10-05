//! The mission played by itself (`--autoplay`, for testing): she heads for
//! the markers the scripts put up, shoots the soldiers and blows up the
//! things they mark, gets into the vehicles they bring and drives on, all
//! with a player's moves: an [`Intent`] a tick, as the keyboard and mouse
//! give. What she does is logged, so a run shows whether the mission can
//! be played through and where she got stuck. Where she makes no headway
//! for a while she hops ahead (logged too): the run goes on to the later
//! objectives, and the hops show where the way is blocked.

use m2_math::glam::{Mat4, Vec2, Vec3};

use crate::combat::{Combat, Trigger, Weapon};
use crate::game::{Game, place};
use crate::mission::Mark;
use crate::player::{Intent, Player};

/// How fast she turns the camera, radians a second (a quick hand on the
/// mouse).
const TURN: f32 = 5.0;
/// The camera's pitch while she runs or drives, radians.
const CRUISE_PITCH: f32 = -0.2;
/// Close enough to a place to be there, metres. Waiting there longer than
/// this (seconds), she walks round it this far out: the scripts' region
/// may lie to a side of their marker.
const ARRIVE: f32 = 2.5;
const CIRCLE_AFTER: f32 = 4.0;
const CIRCLE: f32 = 6.0;
/// At a place this long (seconds) with nothing happening, she takes one
/// of the scripts' vehicles standing within this far of it (metres): a
/// vehicle they brought her.
const TAKE_AFTER: f32 = 8.0;
const TAKE_NEAR: f32 = 40.0;
/// She shoots soldiers from within this far (metres), and blows things up
/// from between these.
const SHOOT_RANGE: f32 = 35.0;
const BLAST_RANGE: (f32, f32) = (15.0, 45.0);
/// A tank's cannon reaches this far, metres.
const CANNON_RANGE: f32 = 90.0;
/// She fires once aimed this long (seconds) and this close (radians); a
/// turret must be this close to the camera's heading too.
const STEADY: f32 = 0.35;
const ON_TARGET: f32 = 0.03;
const TURRET_ON: f32 = 0.05;
/// Soldiers fighting her within this far are dealt with first, metres.
const DEFEND: f32 = 30.0;
/// Driving: she eases off within this far of a place (metres) above this
/// speed (m/s); stuck this long (seconds), she backs out for this long.
const SLOW_NEAR: f32 = 30.0;
const SLOW: f32 = 7.0;
const STALL: f32 = 2.5;
const REVERSE: f32 = 1.5;
/// No headway (a metre closer) for this long, seconds: she jumps and
/// sidesteps; for this long, she hops this far ahead (metres).
const STUCK: f32 = 3.0;
const SIDESTEP: f32 = 1.2;
const HOP_AFTER: f32 = 25.0;
const HOP: f32 = 25.0;
/// A ride she has had is taken again for places further than this when it
/// stands within the second distance, metres.
const RIDE_FOR: f32 = 120.0;
const RIDE_NEAR: f32 = 40.0;
/// Out of a vehicle without a cannon this far from a target, metres.
const GET_OUT: f32 = 40.0;
/// Within this far of a vehicle's door she presses to get in, metres;
/// presses are this far apart, seconds (a press gets her in or out).
const DOOR: f32 = 2.5;
const PRESS_GAP: f32 = 1.5;
/// Fallen this far under the ground (metres: a hole where the island
/// hadn't streamed in yet), she is put back on it.
const UNDER: f32 = 5.0;
/// Seconds between lines saying how she is getting on.
const REPORT: f32 = 15.0;

/// What she is after.
#[derive(Clone, Debug, PartialEq)]
enum Goal {
    /// Get to the scripts' object `target`, at `at`.
    Reach { target: u32, at: Vec3 },
    /// Shoot person `npcs[i]`.
    Shoot(usize),
    /// Blow up the scripts' object `target`, aiming at `at`.
    Blast { target: u32, at: Vec3 },
    /// Get into vehicle `vehicles[i]`.
    Board(usize),
}

impl Goal {
    /// The same goal, though its target may have moved.
    fn same(&self, other: &Goal) -> bool {
        match (self, other) {
            (Goal::Reach { target: a, .. }, Goal::Reach { target: b, .. })
            | (Goal::Blast { target: a, .. }, Goal::Blast { target: b, .. }) => a == b,
            (Goal::Shoot(a), Goal::Shoot(b)) | (Goal::Board(a), Goal::Board(b)) => a == b,
            _ => false,
        }
    }
}

/// The player playing herself.
#[derive(Debug, Default)]
pub struct Autoplay {
    /// Seconds played.
    clock: f32,
    goal: Option<Goal>,
    /// What the goal is, for the log.
    what: String,
    /// The closest she has come to the goal (metres), and when.
    best: (f32, f32),
    /// When she got to the place she is after, if she has.
    arrived: Option<f32>,
    /// The place she waits at (the scripts' object), and since when:
    /// kept while she goes for a vehicle standing by.
    waiting: Option<(u32, f32)>,
    /// Seconds she has been aiming steadily.
    steady: f32,
    /// Sidestepping (-1 left, 1 right), seconds left.
    sidestep: Option<(f32, f32)>,
    /// Which way the next sidestep goes.
    side: f32,
    /// Backing a vehicle out: steering, seconds left.
    reversing: Option<(f32, f32)>,
    /// Seconds a vehicle has been stuck.
    stalled: f32,
    /// The vehicle she drove last (index into the scene's).
    ride: Option<usize>,
    /// Seconds since she last pressed to get in or out.
    pressed: f32,
    /// Getting out of the vehicle she is stuck in, to go on on foot.
    leave: bool,
    /// Hops: from, to.
    pub hops: Vec<(Vec3, Vec3)>,
    /// The objectives last logged.
    objectives: Vec<String>,
    next_report: f32,
}

impl Autoplay {
    /// One tick: how she plays it, from the scripts' markers. `objectives`
    /// (as shown) are logged as they change. May hop her ahead, or put
    /// her back on the ground.
    pub fn step(
        &mut self,
        game: &mut Game,
        marks: &[Mark],
        objectives: &[String],
        dt: f32,
    ) -> Intent {
        self.clock += dt;
        self.pressed += dt;
        if objectives != self.objectives.as_slice() {
            eprintln!("autoplay {:.1} s: objectives {objectives:?}", self.clock);
            self.objectives = objectives.to_vec();
        }
        let Some(p) = &game.scene.player else {
            return Intent::default();
        };
        if !game.on_foot || !p.landed || game.combat.down > 0.0 {
            return Intent::default();
        }
        match p.driving {
            Some(i) => self.ride = Some(i),
            None => self.leave = false,
        }
        self.ground(game);
        let chosen = self.choose(game, marks);
        let changed = match (&chosen, &self.goal) {
            (Some((a, _)), Some(b)) => !a.same(b),
            (None, None) => false,
            _ => true,
        };
        if changed {
            self.what = chosen
                .as_ref()
                .map_or_else(|| "nothing to do".to_owned(), |c| c.1.clone());
            eprintln!(
                "autoplay {:.1} s: {}, from {:.1}",
                self.clock,
                self.what,
                game.focus()
            );
            self.best = (f32::INFINITY, self.clock);
            self.arrived = None;
            self.steady = 0.0;
            self.sidestep = None;
            self.reversing = None;
            self.stalled = 0.0;
        }
        self.goal = chosen.map(|c| c.0);
        if let (Some(Goal::Reach { target, .. }), Some(t)) = (&self.goal, self.arrived)
            && self.waiting.is_none_or(|w| w.0 != *target)
        {
            self.waiting = Some((*target, t));
        }
        if self.clock >= self.next_report {
            self.next_report = self.clock + REPORT;
            self.report(game);
        }
        match self.goal.clone() {
            None => self.idle(game),
            Some(Goal::Reach { at, .. }) => self.travel(game, at, ARRIVE, dt),
            Some(Goal::Shoot(i)) => self.shoot(game, i, dt),
            Some(Goal::Blast { at, .. }) => self.blast(game, at, dt),
            Some(Goal::Board(i)) => self.board(game, i, dt),
        }
    }

    /// What to do next, and words for the log.
    fn choose(&self, game: &Game, marks: &[Mark]) -> Option<(Goal, String)> {
        let p = game.scene.player.as_ref()?;
        let here = game.focus();
        let npcs = &game.scene.people.npcs;
        let vehicles = &game.scene.vehicles;
        let driving = p.driving;
        // Soldiers fighting her close by, while she's on foot.
        if driving.is_none()
            && let Some((i, d)) = npcs
                .iter()
                .enumerate()
                .filter(|(_, n)| n.alive && n.soldier && n.alert)
                .map(|(i, n)| (i, n.character.position.distance(here)))
                .filter(|&(_, d)| d < DEFEND)
                .min_by(|a, b| a.1.total_cmp(&b.1))
        {
            return Some((
                Goal::Shoot(i),
                format!("fighting back a soldier {d:.0} m off"),
            ));
        }
        let live: Vec<&Mark> = marks.iter().filter(|m| m.alive).collect();
        fn nearest(ms: Vec<&Mark>, here: Vec3) -> Option<&Mark> {
            ms.into_iter()
                .min_by(|a, b| a.at.distance(here).total_cmp(&b.at.distance(here)))
        }
        // What to destroy, nearest first: soldiers are shot, things blown up.
        let destroy: Vec<&Mark> = live
            .iter()
            .copied()
            .filter(|m| m.kind == "destroy")
            .collect();
        if let Some(m) = nearest(destroy, here) {
            if let Some(i) = npcs
                .iter()
                .position(|n| n.spawn == Some(m.target) && n.alive)
            {
                return Some((
                    Goal::Shoot(i),
                    format!("shooting {} ({:#x})", m.name, m.target),
                ));
            }
            let spawn = game
                .scene
                .world
                .as_ref()
                .is_some_and(|w| w.index.spawns.iter().any(|s| s.entity == m.target));
            if spawn {
                return Some((
                    Goal::Reach {
                        target: m.target,
                        at: m.at,
                    },
                    format!(
                        "heading for {} ({:#x}), {:.0} m off, nobody standing there yet",
                        m.name,
                        m.target,
                        m.at.distance(here)
                    ),
                ));
            }
            let at = vehicles
                .iter()
                .position(|v| v.guid == Some(m.target))
                .and_then(|i| vehicle_at(game, i))
                .map(|t| t.transform_point3(Vec3::Y))
                .or_else(|| game.scene.props.centre(m.target))
                .unwrap_or(m.at + Vec3::Y);
            return Some((
                Goal::Blast {
                    target: m.target,
                    at,
                },
                format!(
                    "blowing up {} ({:#x}) at {at:.1}, {:.0} m off",
                    m.name,
                    m.target,
                    at.distance(here)
                ),
            ));
        }
        // The scripts' vehicles they mark: in, or by where it will be.
        for m in &live {
            let Some(i) = vehicles
                .iter()
                .position(|v| v.guid == Some(m.target) && !v.wrecked)
            else {
                continue;
            };
            if driving == Some(i) {
                continue;
            }
            return Some(if vehicles[i].id.is_some() {
                (
                    Goal::Board(i),
                    format!("getting into {} ({})", m.name, vehicles[i].spec.label),
                )
            } else {
                (
                    Goal::Reach {
                        target: m.target,
                        at: m.at,
                    },
                    format!("waiting by {} for it to come", m.name),
                )
            });
        }
        // Places to get to (not the vehicle she is in): those marked, else
        // the objectives' on the map or radar.
        let riding = driving.and_then(|i| vehicles.get(i)?.guid);
        let unmarked = |m: &Mark| m.kind == "objective" || m.kind == "radar";
        let marked = live
            .iter()
            .any(|m| !unmarked(m) && Some(m.target) != riding);
        let places: Vec<&Mark> = live
            .iter()
            .copied()
            .filter(|m| Some(m.target) != riding && (!marked || !unmarked(m)))
            .collect();
        let m = nearest(places, here)?;
        if driving.is_none() {
            // Far, with a ride close (hers from before, or one of the
            // scripts' vehicles): take it.
            let near = |i: usize| {
                vehicles.get(i).is_some_and(|v| !v.wrecked)
                    && vehicle_at(game, i)
                        .is_some_and(|t| t.w_axis.truncate().distance(here) < RIDE_NEAR)
            };
            if m.at.distance(here) > RIDE_FOR
                && let Some(i) = self
                    .ride
                    .into_iter()
                    .chain((0..vehicles.len()).filter(|&i| vehicles[i].guid.is_some()))
                    .find(|&i| near(i))
            {
                return Some((
                    Goal::Board(i),
                    format!(
                        "taking the {} for {} ({:.0} m off)",
                        vehicles[i].spec.label,
                        m.name,
                        m.at.distance(here)
                    ),
                ));
            }
            // There a while and nothing happening: the scripts' vehicle
            // standing by is hers.
            if let Some((_, since)) = self.waiting.filter(|w| w.0 == m.target)
                && self.clock - since > TAKE_AFTER
                && let Some(i) = vehicles
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.guid.is_some() && !v.wrecked)
                    .filter_map(|(i, _)| {
                        let d = vehicle_at(game, i)?.w_axis.truncate().distance(m.at);
                        (d < TAKE_NEAR).then_some((i, d))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(i, _)| i)
            {
                return Some((
                    Goal::Board(i),
                    format!(
                        "nothing at {}: getting into the {} standing by",
                        m.name, vehicles[i].spec.label
                    ),
                ));
            }
        }
        Some((
            Goal::Reach {
                target: m.target,
                at: m.at,
            },
            format!(
                "going to {} ({:#x}, {}), {:.0} m off",
                m.name,
                m.target,
                m.kind,
                m.at.distance(here)
            ),
        ))
    }

    /// Nothing to do: stand, or stop the vehicle.
    fn idle(&mut self, game: &Game) -> Intent {
        let driving = game
            .scene
            .player
            .as_ref()
            .is_some_and(|p| p.driving.is_some());
        Intent {
            hold_jump: driving,
            ..Intent::default()
        }
    }

    /// Gets to `at`, on foot or in the vehicle she is in.
    fn travel(&mut self, game: &mut Game, at: Vec3, arrive: f32, dt: f32) -> Intent {
        match game.scene.player.as_ref().and_then(|p| p.driving) {
            Some(i) => self.drive(game, i, at, arrive, dt),
            None => self.walk(game, at, arrive, dt),
        }
    }

    /// On foot to within `arrive` of `at` (then round it, as said at
    /// [`CIRCLE_AFTER`]).
    fn walk(&mut self, game: &mut Game, at: Vec3, arrive: f32, dt: f32) -> Intent {
        let Some(here) = game.scene.player.as_ref().map(|p| p.character.position) else {
            return Intent::default();
        };
        let d = (at - here).with_y(0.0).length();
        if self.headway(game, at, d) {
            return Intent::default();
        }
        let mut target = at;
        if d < arrive || self.arrived.is_some() {
            let since = *self.arrived.get_or_insert(self.clock);
            if self.clock - since < CIRCLE_AFTER {
                return Intent::default();
            }
            let a = (self.clock - since) * 0.6;
            target = at + Vec3::new(a.sin(), 0.0, a.cos()) * CIRCLE;
        }
        let Some(p) = game.scene.player.as_ref() else {
            return Intent::default();
        };
        let to = (target - here).with_y(0.0);
        let look = turn_to(p, to.x.atan2(to.z), CRUISE_PITCH, dt);
        let mut movement = Vec2::Y;
        let mut jump = false;
        if let Some((side, left)) = &mut self.sidestep {
            jump = *left == SIDESTEP;
            *left -= dt;
            movement = Vec2::new(*side, 0.5).normalize();
            if *left <= 0.0 {
                self.sidestep = None;
            }
        }
        Intent {
            movement,
            sprint: d > 12.0,
            walk: d < 3.0,
            jump,
            hold_jump: jump,
            look,
            ..Intent::default()
        }
    }

    /// Driving vehicle `i` to within `arrive` of `at`.
    fn drive(&mut self, game: &mut Game, i: usize, at: Vec3, arrive: f32, dt: f32) -> Intent {
        let Some((m, speed)) = vehicle_state(game, i) else {
            return Intent::default();
        };
        if self.leave {
            return self.press(speed);
        }
        let pos = m.w_axis.truncate();
        let d = (at - pos).with_y(0.0).length();
        if self.headway(game, at, d) {
            return Intent::default();
        }
        if d < arrive.max(6.0) {
            self.arrived.get_or_insert(self.clock);
            return Intent {
                hold_jump: speed.abs() > 0.3,
                ..Intent::default()
            };
        }
        let (angle, _) = heading_to(m, at);
        let mut steer = (angle * 2.0).clamp(-1.0, 1.0);
        let mut throttle = 1.0;
        if (d < SLOW_NEAR && speed > SLOW) || (angle.abs() > 0.8 && speed > SLOW) {
            throttle = 0.2;
        }
        // Stuck against something: back out, turning towards the way on.
        if throttle > 0.5 && speed.abs() < 1.0 && self.reversing.is_none() {
            self.stalled += dt;
        } else {
            self.stalled = 0.0;
        }
        if self.stalled > STALL {
            self.stalled = 0.0;
            self.reversing = Some((-angle.signum(), REVERSE));
        }
        if let Some((s, left)) = &mut self.reversing {
            steer = *s;
            throttle = -1.0;
            *left -= dt;
            if *left <= 0.0 {
                self.reversing = None;
            }
        }
        Intent {
            movement: Vec2::new(steer, throttle),
            ..Intent::default()
        }
    }

    /// Shooting person `npcs[i]`: closer first if out of range; from a
    /// tank, with its cannon.
    fn shoot(&mut self, game: &mut Game, i: usize, dt: f32) -> Intent {
        let Some(target) = game.scene.people.npcs.get(i).map(|n| n.chest()) else {
            return Intent::default();
        };
        if let Some(v) = game.scene.player.as_ref().and_then(|p| p.driving) {
            return self.at_target_driving(game, v, target, dt);
        }
        let Some(p) = game.scene.player.as_ref() else {
            return Intent::default();
        };
        let d = target.distance(p.character.position);
        if d > SHOOT_RANGE {
            return self.walk(game, target, SHOOT_RANGE - 5.0, dt);
        }
        let (look, error) = aim(p, target, dt);
        let clear = game.scene.world.as_ref().is_some_and(|w| {
            Combat::aim_probe(&w.physics, &game.scene.people, p)
                .is_some_and(|(_, who)| who == Some(i))
        });
        self.steady = if error < ON_TARGET {
            self.steady + dt
        } else {
            0.0
        };
        // Aimed, but something is in the way: closer.
        let movement = if !clear && self.steady > 1.0 {
            Vec2::Y
        } else {
            Vec2::ZERO
        };
        Intent {
            look,
            movement,
            walk: true,
            trigger: Trigger {
                aim: true,
                fire: clear && self.steady > STEADY,
                select: (game.combat.weapon != Weapon::Rifle).then_some(Weapon::Rifle),
                ..Trigger::default()
            },
            ..Intent::default()
        }
    }

    /// Blowing up what stands at `at` with rockets: from between
    /// [`BLAST_RANGE`]; from a tank, with its cannon.
    fn blast(&mut self, game: &mut Game, at: Vec3, dt: f32) -> Intent {
        if let Some(v) = game.scene.player.as_ref().and_then(|p| p.driving) {
            return self.at_target_driving(game, v, at, dt);
        }
        let Some(p) = game.scene.player.as_ref() else {
            return Intent::default();
        };
        let here = p.character.position;
        let d = at.distance(here);
        if d > BLAST_RANGE.1 {
            return self.walk(game, at, BLAST_RANGE.1 - 5.0, dt);
        }
        if d < BLAST_RANGE.0 {
            // Too close for a rocket: back off.
            let away = here + (here - at).with_y(0.0).normalize_or(Vec3::X) * 10.0;
            return self.walk(game, away, 1.0, dt);
        }
        let (look, error) = aim(p, at, dt);
        let reach = at.distance(p.pivot());
        let clear = game.scene.world.as_ref().is_some_and(|w| {
            Combat::aim_probe(&w.physics, &game.scene.people, p)
                .is_none_or(|(t, _)| t > reach - 4.0)
        });
        self.steady = if error < ON_TARGET {
            self.steady + dt
        } else {
            0.0
        };
        let movement = if !clear && self.steady > 1.0 {
            Vec2::Y
        } else {
            Vec2::ZERO
        };
        Intent {
            look,
            movement,
            walk: true,
            trigger: Trigger {
                aim: true,
                fire: clear && self.steady > STEADY && game.combat.weapon == Weapon::Rpg,
                select: (game.combat.weapon != Weapon::Rpg).then_some(Weapon::Rpg),
                ..Trigger::default()
            },
            ..Intent::default()
        }
    }

    /// At a target from vehicle `i`: a tank's cannon fires at it from
    /// within range; from anything else she gets out close to it.
    fn at_target_driving(&mut self, game: &mut Game, i: usize, target: Vec3, dt: f32) -> Intent {
        let Some((m, speed)) = vehicle_state(game, i) else {
            return Intent::default();
        };
        let Some(v) = game.scene.vehicles.get(i) else {
            return Intent::default();
        };
        let cannon = v.spec.kind == "tank" && v.spec.muzzle.is_some();
        let d = target.distance(m.w_axis.truncate());
        if !cannon {
            if d < GET_OUT {
                return self.press(speed);
            }
            return self.drive(game, i, target, GET_OUT - 5.0, dt);
        }
        if d > CANNON_RANGE {
            return self.drive(game, i, target, CANNON_RANGE - 10.0, dt);
        }
        let Some(p) = game.scene.player.as_ref() else {
            return Intent::default();
        };
        let (look, error) = aim(p, target, dt);
        // The turret follows the camera at its own pace.
        let f = m.transform_vector3(Vec3::Z);
        let turret = f.x.atan2(f.z) + v.turret;
        let off = wrap(p.yaw + look.x - turret).abs();
        self.steady = if error < ON_TARGET && off < TURRET_ON {
            self.steady + dt
        } else {
            0.0
        };
        Intent {
            look,
            hold_jump: speed.abs() > 0.3,
            trigger: Trigger {
                fire: self.steady > STEADY,
                ..Trigger::default()
            },
            ..Intent::default()
        }
    }

    /// Into vehicle `vehicles[i]`: to its door, then the use key; out of
    /// any other first.
    fn board(&mut self, game: &mut Game, i: usize, dt: f32) -> Intent {
        let Some(p) = game.scene.player.as_ref() else {
            return Intent::default();
        };
        if let Some(other) = p.driving {
            let speed = vehicle_state(game, other).map_or(0.0, |s| s.1);
            return self.press(speed);
        }
        let Some(m) = vehicle_at(game, i) else {
            return Intent::default();
        };
        let spec = &game.scene.vehicles[i].spec;
        // Its door, else beside it on the left.
        let door = spec.door.map_or_else(
            || {
                let (lo, hi) = spec.bounds;
                m.transform_point3(Vec3::new(hi.x + 0.8, 0.0, (lo.z + hi.z) * 0.5))
            },
            |d| m.transform_point3(d),
        );
        let here = p.character.position;
        if (door - here).with_y(0.0).length() < DOOR {
            return self.press(0.0);
        }
        self.walk(game, door, 1.0, dt)
    }

    /// The use key, once a [`PRESS_GAP`]: stopped first when driving.
    fn press(&mut self, speed: f32) -> Intent {
        if speed.abs() > 1.0 {
            return Intent {
                hold_jump: true,
                ..Intent::default()
            };
        }
        let press = self.pressed > PRESS_GAP;
        if press {
            self.pressed = 0.0;
        }
        Intent {
            use_pressed: press,
            hold_jump: true,
            ..Intent::default()
        }
    }

    /// Keeps track of how close she has come to `at` (`d` metres off):
    /// short of headway she sidesteps and jumps, long short of it she
    /// hops ahead (true then: nothing more this tick).
    fn headway(&mut self, game: &mut Game, at: Vec3, d: f32) -> bool {
        if d < self.best.0 - 1.0 {
            self.best = (d, self.clock);
            return false;
        }
        let stuck = self.clock - self.best.1;
        if self.arrived.is_some() {
            return false;
        }
        if stuck > HOP_AFTER {
            // Stuck driving close to the place: on foot from here.
            let driving = game
                .scene
                .player
                .as_ref()
                .is_some_and(|p| p.driving.is_some());
            if driving && d < RIDE_FOR {
                eprintln!(
                    "autoplay {:.1} s: no headway driving for {stuck:.0} s, {d:.0} m off: getting out",
                    self.clock
                );
                self.leave = true;
                self.best = (f32::INFINITY, self.clock);
                return false;
            }
            self.hop(game, at, stuck);
            return true;
        }
        if stuck > STUCK && self.sidestep.is_none() && (stuck % STUCK) < 0.05 {
            self.side = if self.side > 0.0 { -1.0 } else { 1.0 };
            self.sidestep = Some((self.side, SIDESTEP));
        }
        false
    }

    /// Moves her (and what she drives) up to [`HOP`] metres towards `at`,
    /// onto the ground there.
    fn hop(&mut self, game: &mut Game, at: Vec3, stuck: f32) {
        let from = game.focus();
        let flat = (at - from).with_y(0.0);
        let mut to = from + flat.normalize_or_zero() * flat.length().min(HOP);
        let (Some(world), Some(p)) = (&mut game.scene.world, &mut game.scene.player) else {
            return;
        };
        let top = Vec3::new(to.x, to.y.max(at.y) + 80.0, to.z);
        if let Some((t, _)) = world.physics.cast_ray(top, Vec3::NEG_Y, 400.0) {
            to.y = top.y - t;
        }
        match p.driving {
            Some(i) => {
                let v = &mut game.scene.vehicles[i];
                let yaw = flat.x.atan2(flat.z);
                if let Some(id) = v.id.take() {
                    world.physics.remove_vehicle(id);
                }
                if !place(&mut world.physics, v, to, yaw) {
                    eprintln!(
                        "autoplay: no ground to put the {} on at {to:.1}",
                        v.spec.label
                    );
                }
            }
            None => {
                p.character.position = to + Vec3::Y * 0.1;
                p.character.velocity = Vec3::ZERO;
            }
        }
        eprintln!(
            "autoplay {:.1} s: no headway for {stuck:.0} s at {from:.1} ({}): hopped to {to:.1}",
            self.clock, self.what
        );
        self.hops.push((from, to));
        self.best = (f32::INFINITY, self.clock);
        self.sidestep = None;
        self.reversing = None;
    }

    /// Fallen through a hole in the island (it hadn't streamed in under
    /// her): back up on the ground.
    fn ground(&mut self, game: &mut Game) {
        let (Some(world), Some(p)) = (&game.scene.world, &mut game.scene.player) else {
            return;
        };
        if p.driving.is_some() {
            return;
        }
        let at = p.character.position;
        // Something under her: not fallen through (a roof over her head
        // doesn't count).
        if world
            .physics
            .cast_ray(at + Vec3::Y, Vec3::NEG_Y, 300.0)
            .is_some()
        {
            return;
        }
        let top = Vec3::new(at.x, at.y + 300.0, at.z);
        let Some((t, _)) = world.physics.cast_ray(top, Vec3::NEG_Y, 600.0) else {
            return;
        };
        let ground = top.y - t;
        if at.y < ground - UNDER {
            eprintln!(
                "autoplay {:.1} s: fell through at {at:.1}; back up on the ground at {ground:.1}",
                self.clock
            );
            p.character.position.y = ground + 0.1;
            p.character.velocity = Vec3::ZERO;
        }
    }

    /// How she is getting on, for the log.
    fn report(&self, game: &Game) {
        let Some(p) = game.scene.player.as_ref() else {
            return;
        };
        let riding = p
            .driving
            .and_then(|i| game.scene.vehicles.get(i))
            .map(|v| format!("driving the {}", v.spec.label));
        eprintln!(
            "autoplay {:.1} s: at {:.1}, {}, health {:.0}, {}; closest {:.0} m, {:.0} s ago; {} hops",
            self.clock,
            game.focus(),
            riding.unwrap_or_else(|| "on foot".to_owned()),
            game.combat.hp,
            self.what,
            self.best.0,
            self.clock - self.best.1,
            self.hops.len()
        );
    }

    /// A summary for the end of the run.
    pub fn summary(&self) -> String {
        format!(
            "{:.1} s played, {} hops{}",
            self.clock,
            self.hops.len(),
            self.hops
                .iter()
                .map(|(a, b)| format!("; {a:.0} to {b:.0}"))
                .collect::<String>()
        )
    }
}

/// Vehicle `vehicles[i]`'s model to world, if placed.
fn vehicle_at(game: &Game, i: usize) -> Option<Mat4> {
    let id = game.scene.vehicles.get(i)?.id?;
    game.scene.world.as_ref()?.physics.vehicle_transform(id)
}

/// And its forward speed (m/s).
fn vehicle_state(game: &Game, i: usize) -> Option<(Mat4, f32)> {
    let id = game.scene.vehicles.get(i)?.id?;
    let physics = &game.scene.world.as_ref()?.physics;
    Some((physics.vehicle_transform(id)?, physics.vehicle_speed(id)))
}

/// How far `at` lies off the heading of a vehicle at `m` (radians,
/// positive to its right), and how far away (metres).
fn heading_to(m: Mat4, at: Vec3) -> (f32, f32) {
    let f = m
        .transform_vector3(Vec3::Z)
        .with_y(0.0)
        .normalize_or(Vec3::Z);
    // Right-handed, +y up: right is forward turned clockwise from above.
    let right = Vec3::new(-f.z, 0.0, f.x);
    let to = (at - m.w_axis.truncate()).with_y(0.0);
    let d = to.length();
    let dir = to / d.max(1e-3);
    (dir.dot(right).atan2(dir.dot(f)), d)
}

/// An angle wrapped to -π..π.
fn wrap(a: f32) -> f32 {
    let mut a = a.rem_euclid(std::f32::consts::TAU);
    if a > std::f32::consts::PI {
        a -= std::f32::consts::TAU;
    }
    a
}

/// The camera turn (yaw, pitch) towards heading `yaw` and `pitch`, as a
/// quick hand on the mouse turns it in `dt` seconds.
fn turn_to(p: &Player, yaw: f32, pitch: f32, dt: f32) -> Vec2 {
    let most = TURN * dt;
    Vec2::new(
        wrap(yaw - p.yaw).clamp(-most, most),
        (pitch - p.pitch).clamp(-most, most),
    )
}

/// The camera turn putting the crosshair on `target`, and how far off it
/// is now (radians).
fn aim(p: &Player, target: Vec3, dt: f32) -> (Vec2, f32) {
    let dir = (target - p.pivot()).normalize_or(Vec3::Z);
    let error = p.camera_forward().angle_between(dir);
    (
        turn_to(p, dir.x.atan2(dir.z), dir.y.clamp(-1.0, 1.0).asin(), dt),
        error,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_place_to_the_right_steers_right() {
        // Facing +z, right is -x.
        let (angle, d) = heading_to(Mat4::IDENTITY, Vec3::new(-10.0, 0.0, 10.0));
        assert!(
            (angle - std::f32::consts::FRAC_PI_4).abs() < 1e-5,
            "{angle}"
        );
        assert!((d - 200f32.sqrt()).abs() < 1e-4);
        let (angle, _) = heading_to(Mat4::IDENTITY, Vec3::new(10.0, 0.0, 0.0));
        assert!(
            (angle + std::f32::consts::FRAC_PI_2).abs() < 1e-5,
            "{angle}"
        );
    }

    #[test]
    fn turns_take_the_short_way_round() {
        assert!(
            (wrap(3.0 * std::f32::consts::FRAC_PI_2) + std::f32::consts::FRAC_PI_2).abs() < 1e-5
        );
        assert!((wrap(-0.5) + 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_crosshair_comes_onto_a_target() {
        let mut p = Player::new(0.0, 0.0, 0.0, None);
        p.pitch = 0.0;
        let target = p.pivot() + Vec3::new(5.0, 0.0, 5.0);
        let mut error = f32::MAX;
        for _ in 0..120 {
            let (look, e) = aim(&p, target, 1.0 / 60.0);
            p.yaw += look.x;
            p.pitch += look.y;
            error = e;
        }
        assert!(error < 1e-3, "{error}");
        assert!(
            (p.yaw - std::f32::consts::FRAC_PI_4).abs() < 1e-3,
            "{}",
            p.yaw
        );
    }
}
