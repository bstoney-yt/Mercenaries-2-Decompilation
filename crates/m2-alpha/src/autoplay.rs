//! The mission played by itself (`--autoplay`, for testing): she heads for
//! the markers the scripts put up, shoots the soldiers and blows up the
//! things they mark, gets into the vehicles they bring and drives on, all
//! with a player's moves: an [`Intent`] a tick, as the keyboard and mouse
//! give. She goes the way planned round walls, the sea and ground too
//! steep ([`Route`]). What she does is logged, so a run shows whether the
//! mission can be played through and where she got stuck. Where she makes
//! no headway for a while she hops ahead (logged too): the run goes on to
//! the later objectives, and the hops show where the way is blocked.

use m2_engine::physics::FLOAT_DEPTH;
use m2_engine::{Physics, Templates, VehicleControls, VehicleId, VehicleSpec};
use m2_math::glam::{Mat4, Quat, Vec2, Vec3};

use crate::combat::{Combat, Trigger, Weapon};
use crate::game::{Game, can_get_out, get_out, place};
use crate::mission::{Mark, WATERLINE};
use crate::player::{Intent, Player};
use crate::route::{Mover, Route};
use crate::support::{Aim, PDA_CLOSE, PDA_OPEN, Phase};

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
/// With the grenade launcher, whose rounds reach about 40 m.
const LOB_RANGE: (f32, f32) = (12.0, 32.0);
/// What only a support strike brings down (the first contract's gates)
/// she calls the bomb on from between these (metres): out of its frag's
/// reach, and near enough that the satellite's view (5 m/s) is soon over
/// it; she confirms the view within this of the target (metres).
const CALL_FROM: (f32, f32) = (crate::combat::BOMB_FRAG.0 + 8.0, 90.0);
const ON_SPOT: f32 = 0.5;
/// A tank's cannon reaches this far, metres.
const CANNON_RANGE: f32 = 90.0;
/// She fires once aimed this long (seconds) and this close (radians); a
/// turret must be this close to the camera's heading too.
const STEADY: f32 = 0.35;
const ON_TARGET: f32 = 0.03;
const TURRET_ON: f32 = 0.05;
/// Soldiers fighting her within this far are dealt with first, metres.
const DEFEND: f32 = 30.0;
/// Aimed this long (seconds) with something in the way, she goes where she
/// can see her target from: on rings this far out (metres), no higher or
/// lower than she stands by more than this if she can (metres).
const BLOCKED: f32 = 4.0;
const VANTAGE_RINGS: [f32; 5] = [6.0, 10.0, 15.0, 20.0, 26.0];
const VANTAGE_CLIMB: f32 = 3.0;
/// Driving: she eases off within this far of a place (metres) above this
/// speed (m/s); stuck this long (seconds), she backs out for this long.
/// How far ahead a driver looks for a clear way: this much, plus this
/// many seconds at its speed, at most this (metres); how far either side
/// of the heading it looks (radians); and how flat a surface must be
/// (its normal's height) not to be a wall.
const WAY_AHEAD: f32 = 6.0;
const WAY_TIME: f32 = 1.5;
const WAY_MOST: f32 = 25.0;
const WAY_FAN: f32 = 1.4;
const WALL: f32 = 0.8;
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
/// Coming closer counts towards the hop only once she has also gone this
/// far (metres) from where she last did: the way's length wobbles by a
/// few metres as it is planned again while she stands still (round 98's
/// 1 to 6 m put her hop off by up to a minute twice).
const HEADWAY_MOVE: f32 = 4.0;
/// A hop along her way goes at least this far, metres; a way ending
/// nearer than that is passed over for the place itself.
const HOP_LEAST: f32 = 8.0;
/// A hop lands on the ground found from this far above the way's height
/// there, down to this far below it, metres; else on the highest there.
const HOP_ABOVE: f32 = 3.0;
const HOP_BELOW: f32 = 10.0;
/// A ride she has had is taken again for places further than this when it
/// stands within the second distance, metres.
const RIDE_FOR: f32 = 120.0;
const RIDE_NEAR: f32 = 40.0;
/// One of the scripts' vehicles she hasn't driven is taken from this far,
/// metres: they brought it for her.
const RIDE_FRESH: f32 = 150.0;
/// Out of a vehicle without a cannon this far from a target, metres.
const GET_OUT: f32 = 40.0;
/// Within this far of a vehicle's door she presses to get in, metres;
/// presses are this far apart, seconds (a press gets her in or out).
const DOOR: f32 = 2.5;
const PRESS_GAP: f32 = 1.5;
/// Fallen this far under the ground (metres: a hole where the island
/// hadn't streamed in yet), she is put back on it.
const UNDER: f32 = 5.0;
/// From further than this from a place (metres), she goes the way planned
/// to it ([`Route`]): in cells this size on foot and driving (metres),
/// over ground up to this steep on foot, on wheels and on tracks; planned
/// again every this long (seconds: the island streams in as she goes),
/// when the place moves this far or she strays this far off it (metres).
/// She heads for a point this far on along it, plus this many seconds at
/// her speed, at most this (metres); taken to run at this speed (m/s).
const PLAN_FROM: f32 = 25.0;
const FOOT_CELL: f32 = 2.0;
const DRIVE_CELL: f32 = 4.0;
const ON_FOOT: f32 = 40f32.to_radians();
const ON_WHEELS: f32 = 22f32.to_radians();
const ON_TRACKS: f32 = 27f32.to_radians();
const REPLAN: f32 = 20.0;
const REPLAN_MOVED: f32 = 5.0;
const STRAYED: f32 = 15.0;
const LOOK_AHEAD: f32 = 5.0;
const LOOK_TIME: f32 = 0.6;
const LOOK_MOST: f32 = 20.0;
const RUN: f32 = 6.0;
/// A boat's way keeps this much water under its keel, metres. Stuck, a
/// boat is put afloat over water that deep: on its way, back towards it
/// in steps this long, or this far to either side of it (metres).
const UNDER_KEEL: f32 = 0.6;
const AFLOAT_STEP: f32 = 5.0;
const AFLOAT_SIDES: [f32; 5] = [0.0, -8.0, 8.0, -16.0, 16.0];
/// Rays looking for open water start this far over the sea, metres (over
/// rocks standing out of it).
const OVER_WATER: f32 = 30.0;
/// A way planned this much longer than the straight one is logged, at
/// most every this long (seconds).
const ROUNDABOUT: f32 = 1.15;
const LOG_WAY: f32 = 30.0;
/// A supply crate down within this far (metres) is opened before going on,
/// standing this far from its side (with a bash's 1.8 m reach), turned to
/// it within this much (radians); one not open after this many seconds is
/// left. The crates are the deliveries' and land where the scripts call
/// her: in the real game's first contract she smashes the weapon crate
/// (1:18) before the beach guards (1:26); here the guards can draw her
/// 100 m up the beach before it lands (from the recordings; she still
/// fights back first when they're within [`DEFEND`]).
const OPEN_NEAR: f32 = 250.0;
const OPEN_FROM: f32 = 0.9;
const OPEN_FACING: f32 = 0.2;
const OPEN_FOR: f32 = 45.0;
/// A vehicle being brought to a drop point this near (metres) is waited
/// for this far from it (metres).
const AWAIT_REACH: f32 = 300.0;
const AWAIT_FROM: f32 = 15.0;
/// She looks at her PDA at least this long before shutting it, at most
/// this long (seconds).
const PDA_LOOK: f32 = 2.0;
const PDA_MOST: f32 = 15.0;
/// She opens it this long after the scripts begin waiting for that
/// (seconds): they put its prompt up two seconds after taking the last
/// line down (round 96), and she reads it first.
const PDA_PROMPT: f32 = 3.0;
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
    /// Break open the supply crate `target` (the scripts' object) with a
    /// bash.
    Open(u32),
    /// Take one thing a supply crate left (walking up to it), by its
    /// [`Pickup::key`](crate::flights::Pickup::key).
    Take((u32, u32)),
}

impl Goal {
    /// The same goal, though its target may have moved.
    fn same(&self, other: &Goal) -> bool {
        match (self, other) {
            (Goal::Reach { target: a, .. }, Goal::Reach { target: b, .. })
            | (Goal::Blast { target: a, .. }, Goal::Blast { target: b, .. }) => a == b,
            (Goal::Shoot(a), Goal::Shoot(b)) | (Goal::Board(a), Goal::Board(b)) => a == b,
            (Goal::Open(a), Goal::Open(b)) => a == b,
            (Goal::Take(a), Goal::Take(b)) => a == b,
            _ => false,
        }
    }
}

/// The way she is going ([`Route`]): when it was planned, and whether
/// for driving.
#[derive(Debug)]
struct Way {
    route: Route,
    planned: f32,
    driving: bool,
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
    /// Where she was when she last came closer having gone
    /// [`HEADWAY_MOVE`] from the time before, and when: the hop waits on
    /// this.
    moved: Option<(Vec3, f32)>,
    /// When she got to the place she is after, if she has.
    arrived: Option<f32>,
    /// The place she waits at (the scripts' object), and since when:
    /// kept while she goes for a vehicle standing by.
    waiting: Option<(u32, f32)>,
    /// Seconds she has been aiming steadily.
    steady: f32,
    /// Seconds she has been aimed with something in the way.
    blocked: f32,
    /// Where she goes to see the person she is shooting (`npcs[i]`, the
    /// spot), and the spots that turned out no better.
    vantage: Option<(usize, Vec3)>,
    tried: Vec<Vec3>,
    /// Sidestepping (-1 left, 1 right), seconds left.
    sidestep: Option<(f32, f32)>,
    /// Which way the next sidestep goes.
    side: f32,
    /// Backing a vehicle out: steering, seconds left.
    reversing: Option<(f32, f32)>,
    /// Seconds a vehicle has been stuck.
    stalled: f32,
    /// The vehicle she drove last, and all she has driven (indices into
    /// the scene's).
    ride: Option<usize>,
    ridden: Vec<usize>,
    /// Seconds since she last pressed to get in or out.
    pressed: f32,
    /// Getting out of the vehicle she is stuck in, to go on on foot.
    leave: bool,
    /// The way she is going, and when a roundabout one was last logged.
    way: Option<Way>,
    way_logged: f32,
    /// Hops: from, to.
    pub hops: Vec<(Vec3, Vec3)>,
    /// The objectives last logged.
    objectives: Vec<String>,
    next_report: f32,
    /// The supply crate she is opening and since when, and those she
    /// gave up on.
    opening: Option<(u32, f32)>,
    left_shut: Vec<u32>,
    /// The crate whose pickup she is going for and since when, and those
    /// whose pickups she gave up on.
    taking: Option<((u32, u32), f32)>,
    left_lying: Vec<(u32, u32)>,
    /// Fire as last pressed at the support (it takes presses, not holds),
    /// and the support's step last logged.
    tapped: bool,
    support_logged: &'static str,
    /// When she opened her PDA, while it is open; seconds since she last
    /// pressed its key; when the scripts began waiting for it to open,
    /// while they wait.
    pda_since: Option<f32>,
    pda_pressed: f32,
    pda_asked: Option<f32>,
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
        awaited: &[String],
        dt: f32,
    ) -> Intent {
        self.clock += dt;
        self.pressed += dt;
        self.pda_pressed += dt;
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
            Some(i) => {
                // Shot to a wreck under her: out, on foot or into another.
                if !self.leave && game.scene.vehicles.get(i).is_some_and(|v| v.wrecked) {
                    eprintln!(
                        "autoplay {:.1} s: the {} is wrecked: getting out",
                        self.clock, game.scene.vehicles[i].spec.label
                    );
                    self.leave = true;
                    self.fresh_headway();
                }
                self.ride = Some(i);
                if !self.ridden.contains(&i) {
                    self.ridden.push(i);
                }
            }
            None => self.leave = false,
        }
        self.ground(game);
        // The scripts wait for her to look at her PDA (the first contract,
        // at the first gate): she opens it, and shuts it again.
        let wants = |name: &str| awaited.iter().any(|a| a == name);
        let asked = wants(PDA_OPEN)
            && game.support.phase == Phase::Idle
            && game
                .scene
                .player
                .as_ref()
                .is_some_and(|p| p.driving.is_none());
        self.pda_asked = asked.then(|| self.pda_asked.unwrap_or(self.clock));
        if !game.support.pda
            && self
                .pda_asked
                .is_some_and(|since| self.clock - since < PDA_PROMPT)
        {
            // Its prompt isn't up yet: she waits for it.
            return Intent::default();
        }
        if game.support.pda || asked {
            return self.pda(game.support.pda, wants(PDA_CLOSE));
        }
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
            self.fresh_headway();
            self.arrived = None;
            self.steady = 0.0;
            self.sidestep = None;
            self.reversing = None;
            self.stalled = 0.0;
            self.way = None;
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
        let sturdy = |target: &u32| game.scene.props.sturdy.contains(target);
        let striking = matches!(&self.goal, Some(Goal::Blast { target, .. }) if sturdy(target));
        // A call she is no longer after is put away: the designator goes
        // with a weapon taken up (twice from the satellite's view).
        if !striking && (game.support.busy() || game.support.holding()) {
            return Intent {
                trigger: Trigger {
                    select: Some(Weapon::Rifle),
                    ..Trigger::default()
                },
                ..Intent::default()
            };
        }
        match self.goal.clone() {
            None => self.idle(game),
            Some(Goal::Reach { at, .. }) => self.travel(game, at, ARRIVE, dt),
            Some(Goal::Shoot(i)) => self.shoot(game, i, dt),
            Some(Goal::Blast { at, .. }) if striking => self.strike(game, at, dt),
            Some(Goal::Blast { at, .. }) => self.blast(game, at, dt),
            Some(Goal::Board(i)) => self.board(game, i, dt),
            Some(Goal::Open(target)) => self.open(game, target, dt),
            Some(Goal::Take(key)) => self.take(game, key, dt),
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
        // A supply crate down near her or marked (or the one she set out
        // to open): broken open first, for what is inside.
        if driving.is_none()
            && let Some((target, d)) = game
                .scene
                .flights
                .crates_standing()
                .filter(|g| !self.left_shut.contains(g))
                .filter_map(|g| {
                    let (at, _) = game.scene.flights.crate_standing(g)?;
                    let d = at.distance(here);
                    let wanted = live.iter().any(|m| m.target == g)
                        || self.opening.is_some_and(|o| o.0 == g)
                        || d < OPEN_NEAR;
                    wanted.then_some((g, d))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
        {
            return Some((
                Goal::Open(target),
                format!("opening the supply crate {target:#x}, {d:.0} m off"),
            ));
        }
        // What a crate she broke open left for her: taken before going on.
        let none = Templates::default();
        let templates = game
            .scene
            .world
            .as_ref()
            .map_or(&none, |w| &w.index.template_db);
        if driving.is_none()
            && let Some((from, d)) = game
                .scene
                .flights
                .pickups()
                .iter()
                // What the layers place she leaves to the player: going out
                // of her way for each would keep her from the contract.
                // (Cheapest first: there are hundreds placed.)
                .filter(|p| {
                    !game.placed.is_pickup(p.from)
                        && p.at.distance(here) < OPEN_NEAR
                        && !self.left_lying.contains(&p.key())
                        && wanted(templates, &game.combat, p)
                })
                .map(|p| (p.key(), p.at.distance(here)))
                .filter(|&(_, d)| d < OPEN_NEAR)
                .min_by(|a, b| a.1.total_cmp(&b.1))
        {
            return Some((
                Goal::Take(from),
                format!(
                    "taking the {:#x} left by {:#x}, {d:.1} m off",
                    from.1, from.0
                ),
            ));
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
            let how = if game.scene.props.sturdy.contains(&m.target) {
                "calling a satellite bomb on"
            } else {
                "blowing up"
            };
            return Some((
                Goal::Blast {
                    target: m.target,
                    at,
                },
                format!(
                    "{how} {} ({:#x}) at {at:.1}, {:.0} m off",
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
        // A vehicle on its way to her (hanging from a delivery's
        // helicopter): on foot she waits near where it is to be put down,
        // out from under it, and takes it once it is (the tank brought for
        // the last leg; a ride of hers would carry her past it).
        if driving.is_none()
            && let Some((load, drop)) = game
                .scene
                .flights
                .bringing()
                .into_iter()
                .filter(|(_, to)| to.distance(here) < AWAIT_REACH)
                .min_by(|a, b| a.1.distance(here).total_cmp(&b.1.distance(here)))
        {
            let at = drop + (here - drop).with_y(0.0).normalize_or(Vec3::X) * AWAIT_FROM;
            return Some((
                Goal::Reach { target: load, at },
                format!(
                    "waiting for the vehicle {load:#x} being brought to {drop:.0}, {:.0} m off",
                    drop.distance(here)
                ),
            ));
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
            // Far, with a ride close: the scripts' vehicle they brought
            // last that she hasn't driven (the tank delivered for the next
            // leg), else hers from before or another of theirs.
            let near = |i: usize, within: f32| {
                vehicles
                    .get(i)
                    .is_some_and(|v| !v.wrecked && !v.spec.is_boat())
                    && vehicle_at(game, i)
                        .is_some_and(|t| t.w_axis.truncate().distance(here) < within)
            };
            let fresh = (0..vehicles.len())
                .rev()
                .filter(|&i| vehicles[i].guid.is_some() && !self.ridden.contains(&i))
                .find(|&i| near(i, RIDE_FRESH));
            let known = || {
                self.ride
                    .into_iter()
                    .chain((0..vehicles.len()).filter(|&i| vehicles[i].guid.is_some()))
                    .find(|&i| near(i, RIDE_NEAR))
            };
            if m.at.distance(here) > RIDE_FOR
                && let Some(i) = fresh.or_else(known)
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
                    .filter(|(_, v)| v.guid.is_some() && !v.wrecked && !v.spec.is_boat())
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

    /// Opening supply crate `target`: to its side, turned to it, and a
    /// bash (low, at a crate on the ground); left shut if not open after
    /// [`OPEN_FOR`] seconds.
    fn open(&mut self, game: &mut Game, target: u32, dt: f32) -> Intent {
        let since = match self.opening {
            Some((g, t)) if g == target => t,
            _ => {
                self.opening = Some((target, self.clock));
                self.clock
            }
        };
        let (Some(p), Some((at, half))) = (
            game.scene.player.as_ref(),
            game.scene.flights.crate_standing(target),
        ) else {
            return Intent::default();
        };
        if self.clock - since > OPEN_FOR {
            eprintln!(
                "autoplay {:.1} s: the supply crate {target:#x} at {at:.1} would not open: left shut",
                self.clock
            );
            self.left_shut.push(target);
            return Intent::default();
        }
        let here = p.character.position;
        let to = (at - here).with_y(0.0);
        if to.length() > half + OPEN_FROM {
            return self.walk(game, at, half + OPEN_FROM, dt);
        }
        // Standing, she turns only to aim: so she aims round to it, and
        // bashes once she faces it.
        let yaw = to.x.atan2(to.z);
        let facing = wrap(yaw - p.facing).abs() < OPEN_FACING;
        Intent {
            look: turn_to(p, yaw, CRUISE_PITCH, dt),
            trigger: Trigger {
                aim: true,
                melee: facing,
                ..Trigger::default()
            },
            ..Intent::default()
        }
    }

    /// Taking what a supply crate left (`key`, [`Pickup::key`]): walking
    /// onto it and, within its reach, pressing the use key; left lying if
    /// not taken after [`OPEN_FOR`] seconds.
    ///
    /// [`Pickup::key`]: crate::flights::Pickup::key
    fn take(&mut self, game: &mut Game, key: (u32, u32), dt: f32) -> Intent {
        let since = match self.taking {
            Some((k, t)) if k == key => t,
            _ => {
                self.taking = Some((key, self.clock));
                self.clock
            }
        };
        let Some(p) = game
            .scene
            .flights
            .pickups()
            .iter()
            .find(|p| p.key() == key)
            .copied()
        else {
            return Intent::default();
        };
        if self.clock - since > OPEN_FOR {
            eprintln!(
                "autoplay {:.1} s: the {:#x} the supply crate {:#x} left at {:.1} was not taken: left lying",
                self.clock, key.1, key.0, p.at
            );
            self.left_lying.push(key);
            return Intent::default();
        }
        let here = game
            .scene
            .player
            .as_ref()
            .map_or(p.at, |pl| pl.character.position);
        if (p.at - here).with_y(0.0).length() <= p.what.reach * 0.8 {
            let press = self.pressed > PRESS_GAP;
            if press {
                self.pressed = 0.0;
            }
            // A rifle lying there is taken for its rounds ("Take Ammo"),
            // not taken up in place of hers ([`wanted`]).
            let templates = game.scene.world.as_ref().map(|w| &w.index.template_db);
            let swap = p.what.weapon == Some(Weapon::Rifle)
                && templates.is_none_or(|t| {
                    crate::mission::offer(t, &game.combat, &p.what) == crate::mission::Offer::Equip
                });
            return Intent {
                use_pressed: press && !swap,
                ..Intent::default()
            };
        }
        self.walk(game, p.at, p.what.reach * 0.5, dt)
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
        let mut target = self.waypoint(game, None, here, at, RUN);
        if self.headway(game, at, self.left_to_go(here, at)) {
            return Intent::default();
        }
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
        let toward = self.waypoint(game, Some(i), pos, at, speed);
        if self.headway(game, at, self.left_to_go(pos, at)) {
            return Intent::default();
        }
        if d < arrive.max(6.0) {
            self.arrived.get_or_insert(self.clock);
            return Intent {
                hold_jump: speed.abs() > 0.3,
                ..Intent::default()
            };
        }
        let (angle, _) = heading_to(m, toward);
        // Round what stands in the way.
        let angle = match (
            game.scene.world.as_ref(),
            game.scene
                .vehicles
                .get(i)
                .and_then(|v| v.id.zip(Some(v.spec.hull))),
        ) {
            (Some(w), Some((id, hull))) => clear_way(&w.physics, id, m, hull, angle, speed),
            _ => angle,
        };
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
            // Each stall logged, with what holds it (round 7 counts the
            // tank's over its jungle drive).
            if let (Some(w), Some(v)) = (game.scene.world.as_ref(), game.scene.vehicles.get(i))
                && let Some(id) = v.id
            {
                eprintln!(
                    "autoplay {:.1} s: stalled driving the {} at {:.1}: {}",
                    self.clock,
                    v.spec.label,
                    game.focus(),
                    stuck_report(w, id, v.spec.hull, false)
                );
            }
            // A boat run aground near the place: out, into the shallows
            // (further off, it backs off the rocks).
            if d < RIDE_FOR && self.can_leave(game, i) {
                eprintln!(
                    "autoplay {:.1} s: the {} is aground {d:.0} m off: getting out",
                    self.clock, game.scene.vehicles[i].spec.label
                );
                self.leave = true;
                return self.press(speed);
            }
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
        if clear {
            self.blocked = 0.0;
            self.vantage = None;
            self.tried.clear();
        } else if error < ON_TARGET {
            self.blocked += dt;
        }
        // Aimed a while with a wall in the way: from where she can see
        // the target; another spot if that one was no better.
        let here = p.character.position;
        if self.blocked > BLOCKED {
            let at_spot = self
                .vantage
                .is_some_and(|(t, s)| t == i && s.with_y(0.0).distance(here.with_y(0.0)) < ARRIVE);
            if self.vantage.is_none_or(|(t, _)| t != i) || at_spot {
                if let Some((_, s)) = self.vantage {
                    self.tried.push(s);
                }
                self.blocked = 0.0;
                self.fresh_headway();
                self.vantage = game
                    .scene
                    .world
                    .as_ref()
                    .and_then(|w| vantage(&w.physics, target, here, &self.tried))
                    .map(|s| (i, s));
                eprintln!(
                    "autoplay {:.1} s: no clear shot at {} from {here:.1}: {}",
                    self.clock,
                    self.what,
                    self.vantage
                        .map_or("nowhere better to see it from".to_owned(), |(_, s)| {
                            format!("going to {s:.1}")
                        })
                );
            }
        }
        if let Some((t, spot)) = self.vantage
            && t == i
            && spot.with_y(0.0).distance(here.with_y(0.0)) >= ARRIVE
        {
            self.arrived = None;
            return self.walk(game, spot, 1.0, dt);
        }
        // Aimed, but something is in the way: closer.
        let movement = if !clear && self.steady > 1.0 && self.vantage.is_none() {
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

    /// Blowing up what stands at `at` with what she carries in the
    /// launcher's place: rockets from between [`BLAST_RANGE`], grenades
    /// from between [`LOB_RANGE`]; from a tank, with its cannon.
    fn blast(&mut self, game: &mut Game, at: Vec3, dt: f32) -> Intent {
        if let Some(v) = game.scene.player.as_ref().and_then(|p| p.driving) {
            return self.at_target_driving(game, v, at, dt);
        }
        let Some(p) = game.scene.player.as_ref() else {
            return Intent::default();
        };
        let here = p.character.position;
        let d = at.distance(here);
        let lobbing = game.combat.second == Some(Weapon::GrenadeLauncher);
        let (near, far) = if lobbing { LOB_RANGE } else { BLAST_RANGE };
        if d > far {
            return self.walk(game, at, far - 5.0, dt);
        }
        if d < near {
            // Too close for a rocket or a round: back off.
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
                // A launcher fires once a pull: pulled and let go.
                fire: clear
                    && self.steady > STEADY
                    && Some(game.combat.weapon) == game.combat.second
                    && self.tap(),
                select: (Some(game.combat.weapon) != game.combat.second).then_some(Weapon::Rpg),
                ..Trigger::default()
            },
            ..Intent::default()
        }
    }

    /// Brings down `at`, which only a support strike fells: from between
    /// [`CALL_FROM`] off she calls the bomb as a player would (the support
    /// key, the use key, the view steered over it and confirmed, the
    /// wheel pressed in each wedge), then waits out of its reach.
    /// At her PDA: opened, looked at for [`PDA_LOOK`] seconds and shut
    /// once the scripts wait for that (else after [`PDA_MOST`]).
    fn pda(&mut self, open: bool, close_awaited: bool) -> Intent {
        let press = match open {
            false => true,
            true => {
                let looked = self.clock - *self.pda_since.get_or_insert(self.clock);
                (close_awaited && looked > PDA_LOOK) || looked > PDA_MOST
            }
        } && self.pda_pressed > PRESS_GAP;
        if press {
            self.pda_pressed = 0.0;
            if open {
                self.pda_since = None;
            }
            eprintln!(
                "autoplay {:.1} s: {} her PDA",
                self.clock,
                if open { "shutting" } else { "opening" }
            );
        }
        Intent {
            trigger: Trigger {
                pda: press,
                ..Trigger::default()
            },
            ..Intent::default()
        }
    }

    fn strike(&mut self, game: &mut Game, at: Vec3, dt: f32) -> Intent {
        let flat = |a: Vec3| a.with_y(0.0).distance(at.with_y(0.0));
        if let Some(i) = game.scene.player.as_ref().and_then(|p| p.driving) {
            let speed = vehicle_state(game, i).map_or(0.0, |s| s.1);
            let d = vehicle_at(game, i).map_or(0.0, |m| flat(m.w_axis.truncate()));
            if d < CALL_FROM.1 {
                return self.press(speed);
            }
            return self.drive(game, i, at, CALL_FROM.1 - 15.0, dt);
        }
        let Some(p) = game.scene.player.as_ref() else {
            return Intent::default();
        };
        let here = p.character.position;
        let d = flat(here);
        let support = &game.support;
        let step = match support.phase {
            Phase::Idle => "idle",
            Phase::Menu => "the support menu",
            Phase::Holding { .. } => "the designator",
            Phase::Viewing { .. } => "the satellite's view",
            Phase::Timing { .. } => "the timing wheel",
            Phase::Confirmed { .. } => "confirmed",
            Phase::Designating { .. } => "aiming a designator",
        };
        if step != self.support_logged {
            self.support_logged = step;
            eprintln!(
                "autoplay {:.1} s: support: {step}, {d:.0} m from the target{}",
                self.clock,
                support
                    .inbound()
                    .map_or_else(String::new, |t| format!(", a bomb lands in {t:.1} s"))
            );
        }
        let away = at + (here - at).with_y(0.0).normalize_or(Vec3::X) * (CALL_FROM.0 + 10.0);
        let waiting = support.phase == Phase::Idle
            && (support.inbound().is_some() || support.ready().is_none());
        if waiting || support.phase == Phase::Idle {
            if d < CALL_FROM.0 {
                return self.walk(game, away, 2.0, dt);
            }
            if waiting {
                // Watching where it is to come down.
                let look = game
                    .scene
                    .player
                    .as_ref()
                    .map_or(Vec2::ZERO, |p| aim(p, at, dt).0);
                return Intent {
                    look,
                    ..Intent::default()
                };
            }
            if d > CALL_FROM.1 {
                return self.walk(game, at, CALL_FROM.1 - 15.0, dt);
            }
        }
        let mut intent = Intent::default();
        match support.phase {
            Phase::Idle => intent.trigger.support = true,
            Phase::Menu => intent.use_pressed = true,
            // The view opens by itself.
            Phase::Holding { .. } => {}
            Phase::Viewing { at: view, .. }
            | Phase::Designating {
                aim: Aim::Satellite,
                at: view,
                ..
            } => {
                let to = Vec2::new(at.x, at.z) - view;
                if to.length() < ON_SPOT {
                    intent.trigger.fire = self.tap();
                } else {
                    // Screen right is the world's -x, looking down.
                    intent.movement = Vec2::new(-to.x, to.y).clamp_length_max(1.0);
                }
            }
            Phase::Timing { .. } => intent.trigger.fire = support.wheel_ready() && self.tap(),
            Phase::Confirmed { .. } => {}
            // The laser held on the target.
            Phase::Designating {
                aim: Aim::Laser, ..
            } => {
                if let Some(p) = game.scene.player.as_ref() {
                    intent.look = aim(p, at, dt).0;
                }
                intent.trigger.fire = true;
            }
        }
        intent
    }

    /// Fire pressed and let go on alternate ticks: presses, not a hold.
    fn tap(&mut self) -> bool {
        self.tapped = !self.tapped;
        self.tapped
    }

    /// At a target from vehicle `i`: a cannon (a tank's, the Stryker's)
    /// fires at it from within range; from anything else she gets out
    /// close to it.
    fn at_target_driving(&mut self, game: &mut Game, i: usize, target: Vec3, dt: f32) -> Intent {
        let Some((m, speed)) = vehicle_state(game, i) else {
            return Intent::default();
        };
        let Some(v) = game.scene.vehicles.get(i) else {
            return Intent::default();
        };
        if self.leave {
            return self.press(speed);
        }
        let cannon = v.spec.cannon();
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
        // The turret follows the camera at its own pace, and its gun
        // tilts after it (as far as it goes).
        let f = m.transform_vector3(Vec3::Z);
        let turret = f.x.atan2(f.z) + v.turret;
        let off = wrap(p.yaw + look.x - turret).abs();
        let laid = v.spec.muzzle_pose(v.turret, v.gun).is_none_or(|(at, dir)| {
            let (at, dir) = (m.transform_point3(at), m.transform_vector3(dir));
            let (down, up) = v.spec.elevation;
            let stopped = v.gun <= down + 0.01 || v.gun >= up - 0.01;
            stopped || (target - at).normalize_or(dir).angle_between(dir) < TURRET_ON
        });
        self.steady = if error < ON_TARGET && off < TURRET_ON && laid {
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

    /// Where to head for on her way to `at` from `pos` (driving vehicle
    /// `driving`, or on foot), going `speed` (m/s): along the way planned
    /// to it ([`Route`]), planned again as said at [`PLAN_FROM`]; straight
    /// at it from within that.
    fn waypoint(
        &mut self,
        game: &Game,
        driving: Option<usize>,
        pos: Vec3,
        at: Vec3,
        speed: f32,
    ) -> Vec3 {
        let straight = (at - pos).with_y(0.0).length();
        if straight < PLAN_FROM {
            self.way = None;
            return at;
        }
        let stale = self.way.as_ref().is_none_or(|w| {
            w.driving != driving.is_some()
                || w.route.goal.distance(at) > REPLAN_MOVED
                || self.clock - w.planned > REPLAN
        });
        if stale && let Some(world) = game.scene.world.as_ref() {
            let mover = match driving.and_then(|i| game.scene.vehicles.get(i)) {
                Some(v) => Mover {
                    cell: DRIVE_CELL,
                    steepest: if v.spec.tracked { ON_TRACKS } else { ON_WHEELS },
                    wades: false,
                    skip: v.id,
                    afloat: v.spec.is_boat().then(|| keel_depth(&v.spec) + UNDER_KEEL),
                },
                None => Mover {
                    cell: FOOT_CELL,
                    steepest: ON_FOOT,
                    wades: true,
                    skip: None,
                    afloat: None,
                },
            };
            let sea = world.settings.sea_level.max(WATERLINE);
            let route = Route::plan(&world.physics, mover, pos, at, sea);
            if (!route.reaches || route.length() > straight * ROUNDABOUT)
                && self.clock - self.way_logged > LOG_WAY
            {
                self.way_logged = self.clock;
                let end = route.points.last().copied().unwrap_or(pos);
                let short = (route.goal - end).with_y(0.0).length();
                let how = if route.reaches {
                    format!(
                        "goes round: {:.0} m, not {straight:.0} m straight",
                        route.length()
                    )
                } else if short < 0.5 {
                    "is not found: no step brings her nearer, so straight at it".to_owned()
                } else {
                    format!(
                        "is not found: {:.0} m of way ends {short:.0} m short",
                        route.length()
                    )
                };
                eprintln!(
                    "autoplay {:.1} s: the way to {} from {pos:.0} {how}, by {} points",
                    self.clock,
                    self.what,
                    route.points.len()
                );
            }
            // Headway is measured along the new way from here on.
            let left = route.remaining(pos);
            if left > self.best.0 + 1.0 {
                self.best.0 = left;
            }
            self.way = Some(Way {
                route,
                planned: self.clock,
                driving: driving.is_some(),
            });
        }
        let Some(way) = &mut self.way else {
            return at;
        };
        let ahead = (LOOK_AHEAD + speed.abs() * LOOK_TIME).min(LOOK_MOST);
        let (toward, off) = way.route.steer(pos, ahead);
        if off > STRAYED {
            self.way = None;
        }
        toward
    }

    /// How far she has still to go from `pos` to `at`, metres: along her
    /// way, else straight.
    fn left_to_go(&self, pos: Vec3, at: Vec3) -> f32 {
        self.way.as_ref().map_or_else(
            || (at - pos).with_y(0.0).length(),
            |w| w.route.remaining(pos),
        )
    }

    /// Measures headway afresh: a new goal, after a hop.
    fn fresh_headway(&mut self) {
        self.best = (f32::INFINITY, self.clock);
        self.moved = None;
    }

    /// Seconds since she last made headway that counts towards the hop.
    fn since_moved(&self) -> f32 {
        self.clock - self.moved.map_or(self.best.1, |(_, at)| at)
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

    /// Whether vehicle `i` is a boat she can get out of where it is (not
    /// over deep water).
    fn can_leave(&self, game: &Game, i: usize) -> bool {
        let (Some(world), Some(v)) = (&game.scene.world, game.scene.vehicles.get(i)) else {
            return false;
        };
        v.spec.is_boat() && can_get_out(&world.physics, v)
    }

    /// Keeps track of how close she has come to `at` (`d` metres off, on
    /// her way): short of headway she sidesteps and jumps, long short of it
    /// she hops ahead (true then: nothing more this tick).
    fn headway(&mut self, game: &mut Game, at: Vec3, d: f32) -> bool {
        if d < self.best.0 - 1.0 {
            self.best = (d, self.clock);
            let here = game.focus();
            if self
                .moved
                .is_none_or(|(was, _)| was.distance(here) > HEADWAY_MOVE)
            {
                self.moved = Some((here, self.clock));
            }
            return false;
        }
        let stuck = self.clock - self.best.1;
        if self.arrived.is_some() {
            return false;
        }
        let still = self.since_moved();
        if still > HOP_AFTER {
            // Stuck driving close to the place: on foot from here.
            let driving = game.scene.player.as_ref().and_then(|p| p.driving);
            let near = (at - game.focus()).with_y(0.0).length() < RIDE_FOR;
            if let Some(i) = driving
                && near
                && game
                    .scene
                    .world
                    .as_ref()
                    .is_none_or(|w| can_get_out(&w.physics, &game.scene.vehicles[i]))
            {
                eprintln!(
                    "autoplay {:.1} s: no headway driving for {still:.0} s, {d:.0} m off: getting out",
                    self.clock
                );
                self.leave = true;
                self.fresh_headway();
                return false;
            }
            // Ahead along her way, else (a way that ends where she is:
            // round 86 hopped her 2 m twice by gate 1, to the floor she
            // was on) straight at the place.
            let from = game.focus();
            let to = self
                .way
                .as_mut()
                .map(|w| w.route.steer(from, HOP).0)
                .filter(|to| (*to - from).with_y(0.0).length() >= HOP_LEAST)
                .unwrap_or(at);
            self.hop(game, to, still);
            return true;
        }
        if stuck > STUCK && self.sidestep.is_none() && (stuck % STUCK) < 0.05 {
            self.side = if self.side > 0.0 { -1.0 } else { 1.0 };
            self.sidestep = Some((self.side, SIDESTEP));
        }
        false
    }

    /// Moves her (and what she drives) up to [`HOP`] metres towards `at`,
    /// onto the ground there; a boat, onto water deep enough near there
    /// (else she gets out of it, onto the ground).
    fn hop(&mut self, game: &mut Game, at: Vec3, stuck: f32) {
        let from = game.focus();
        let flat = (at - from).with_y(0.0);
        let mut to = from + flat.normalize_or_zero() * flat.length().min(HOP);
        let (Some(world), Some(p)) = (&mut game.scene.world, &mut game.scene.player) else {
            return;
        };
        // Onto the ground by the way's height there (the floor under a
        // roof), else the highest there is.
        let near = Vec3::new(to.x, at.y + HOP_ABOVE, to.z);
        let high = Vec3::new(to.x, to.y.max(at.y) + 80.0, to.z);
        if let Some(y) = world
            .physics
            .cast_ray(near, Vec3::NEG_Y, HOP_ABOVE + HOP_BELOW)
            .map(|(t, _)| near.y - t)
            .or_else(|| {
                world
                    .physics
                    .cast_ray(high, Vec3::NEG_Y, 400.0)
                    .map(|(t, _)| high.y - t)
            })
        {
            to.y = y;
        }
        let yaw = flat.x.atan2(flat.z);
        // A boat she drives, and its body.
        let boat = p
            .driving
            .filter(|&i| game.scene.vehicles[i].spec.is_boat())
            .and_then(|i| game.scene.vehicles[i].id.map(|id| (i, id)));
        match (p.driving, boat) {
            // Round 88 hopped the start boat 35 m up onto the rocks it
            // was stuck on.
            (_, Some((i, id))) => {
                let v = &mut game.scene.vehicles[i];
                eprintln!(
                    "autoplay {:.1} s: stuck driving the {}: {}",
                    self.clock,
                    v.spec.label,
                    stuck_report(world, id, v.spec.hull, self.reversing.is_some())
                );
                match afloat_near(&world.physics, &v.spec, id, from, to, yaw) {
                    Some(spot) => {
                        world.physics.remove_vehicle(id);
                        v.id = None;
                        if !place(&mut world.physics, v, spot, yaw) {
                            eprintln!(
                                "autoplay: no water to put the {} on at {spot:.1}",
                                v.spec.label
                            );
                        }
                        to = spot;
                    }
                    None => {
                        eprintln!(
                            "autoplay: no water deep enough for the {} near {to:.1}: out of it",
                            v.spec.label
                        );
                        world.physics.drive(
                            id,
                            VehicleControls {
                                parked: true,
                                ..VehicleControls::default()
                            },
                        );
                        get_out(&world.physics, v, p);
                        p.character.position = to + Vec3::Y * 0.1;
                        p.character.velocity = Vec3::ZERO;
                    }
                }
            }
            (Some(i), None) => {
                let v = &mut game.scene.vehicles[i];
                if let Some(id) = v.id {
                    eprintln!(
                        "autoplay {:.1} s: stuck driving the {}: {}",
                        self.clock,
                        v.spec.label,
                        stuck_report(world, id, v.spec.hull, self.reversing.is_some())
                    );
                }
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
            (None, _) => {
                p.character.position = to + Vec3::Y * 0.1;
                p.character.velocity = Vec3::ZERO;
            }
        }
        eprintln!(
            "autoplay {:.1} s: no headway for {stuck:.0} s at {from:.1} ({}): hopped to {to:.1}",
            self.clock, self.what
        );
        self.hops.push((from, to));
        self.fresh_headway();
        self.sidestep = None;
        self.reversing = None;
        self.way = None;
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
        // What may hold her still (round 97's tank stood 125 s).
        let mut holding = String::new();
        if let Some(since) = self.arrived {
            holding += &format!("; there {:.0} s", self.clock - since);
        }
        if game.support.busy() {
            holding += &format!("; support {:?}", game.support.phase);
        }
        eprintln!(
            "autoplay {:.1} s: at {:.1}, {}, health {:.0}, {}; closest {:.0} m, {:.0} s ago; {} hops{holding}; {}",
            self.clock,
            game.focus(),
            riding.unwrap_or_else(|| "on foot".to_owned()),
            game.combat.hp,
            self.what,
            self.best.0,
            self.since_moved(),
            self.hops.len(),
            game.scene.people.fight_summary()
        );
        eprintln!(
            "autoplay tally {:.1} s: {}; {}; {}",
            self.clock,
            game.combat.tally.line(game.combat.fired.rifle),
            game.scene.people.idle.line(),
            game.combat.shots_line()
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

/// Whether she wants what lies at `p`: a weapon she doesn't carry (one
/// she left herself only once what took its place is spent), or
/// ammunition or health she would take walking over it (its template's
/// `Pickup` records for the weapons she carries, with room for them). A
/// rifle lying there (a dead soldier's `Assault Rifle`) she wants only for
/// the rounds she would take walking over it: she keeps her own.
fn wanted(templates: &Templates, combat: &Combat, p: &crate::flights::Pickup) -> bool {
    match p.what.weapon {
        Some(w) if p.from == 0 => combat.second != Some(w) && !combat.second_loaded(),
        Some(w) if w != Weapon::Rifle => combat.second != Some(w),
        // A rifle she doesn't carry gives nothing short of taking it up.
        Some(_)
            if crate::mission::offer(templates, combat, &p.what)
                == crate::mission::Offer::Equip =>
        {
            false
        }
        _ => {
            let refill = crate::mission::refill(templates, combat, &p.what);
            combat.can_use(refill, p.what.health)
        }
    }
}

/// Vehicle `vehicles[i]`'s model to world, if placed.
/// A spot she can stand on and see `target` from: on rings round it, with
/// ground under it and nothing between eye height there and the target,
/// not within a few metres of one already `tried`; of the nearest ring
/// with any, the one closest to `from`, preferring ground about as high
/// as hers.
fn vantage(physics: &Physics, target: Vec3, from: Vec3, tried: &[Vec3]) -> Option<Vec3> {
    let mut best: Option<(f32, Vec3)> = None;
    for r in VANTAGE_RINGS {
        for k in 0..24 {
            let a = k as f32 * std::f32::consts::TAU / 24.0;
            let (x, z) = (target.x + r * a.sin(), target.z + r * a.cos());
            let top = Vec3::new(x, target.y.max(from.y) + 30.0, z);
            let Some((d, n)) = physics.cast_ray(top, Vec3::NEG_Y, 120.0) else {
                continue;
            };
            let spot = Vec3::new(x, top.y - d, z);
            if n.y < 0.7 || tried.iter().any(|t| t.distance(spot) < 4.0) {
                continue;
            }
            let eye = spot + Vec3::Y * 1.6;
            let (dir, len) = ((target - eye).normalize_or_zero(), target.distance(eye));
            // Short of the target's own body.
            if physics.cast_ray(eye, dir, (len - 1.0).max(0.0)).is_some() {
                continue;
            }
            let climb = (spot.y - from.y).abs();
            let cost = from.distance(spot) + if climb > VANTAGE_CLIMB { 100.0 } else { 0.0 };
            if best.is_none_or(|(c, _)| cost < c) {
                best = Some((cost, spot));
            }
        }
        if best.is_some_and(|(c, _)| c < 100.0) {
            break;
        }
    }
    best.map(|(_, s)| s)
}

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

/// The way to steer (radians off the heading, positive to the right)
/// nearest `want` that is clear for a little way, further the faster it
/// goes: rays from both front corners at bumper height, fanned out from
/// `want`. Walls, other vehicles and slopes too steep to drive up are in
/// the way; ground it can drive up isn't.
fn clear_way(
    physics: &Physics,
    id: VehicleId,
    m: Mat4,
    hull: (Vec3, Vec3),
    want: f32,
    speed: f32,
) -> f32 {
    let (lo, hi) = hull;
    let reach = (WAY_AHEAD + speed.abs() * WAY_TIME).min(WAY_MOST);
    let up = m.transform_vector3(Vec3::Y);
    let forward = m.transform_vector3(Vec3::Z);
    let clearance = |a: f32| {
        // Positive is to the right: clockwise from above.
        let dir = Quat::from_axis_angle(up, -a) * forward;
        [lo.x + 0.1, hi.x - 0.1]
            .iter()
            .map(|&x| {
                let from = m.transform_point3(Vec3::new(x, lo.y + 0.8, hi.z));
                match physics.cast_ray_past(from, dir, reach, id) {
                    Some((d, n)) if n.y < WALL => d,
                    _ => reach,
                }
            })
            .fold(reach, f32::min)
    };
    let want = want.clamp(-WAY_FAN, WAY_FAN);
    if clearance(want) >= reach {
        return want;
    }
    let mut best = (f32::NEG_INFINITY, want);
    for k in 1..=10 {
        for side in [-1.0, 1.0] {
            let a = (want + side * k as f32 * 0.15).clamp(-WAY_FAN, WAY_FAN);
            let score = clearance(a) - (a - want).abs() * 4.0;
            if score > best.0 {
                best = (score, a);
            }
        }
    }
    best.1
}

/// Where boat `spec` (body `id`), stuck at `from`, can be put afloat
/// facing `yaw` on its way to `to`: the furthest point along it, or to
/// either side of it, with water [`UNDER_KEEL`] under its keel all over its
/// hull's footprint, at the water's height.
fn afloat_near(
    physics: &Physics,
    spec: &VehicleSpec,
    id: VehicleId,
    from: Vec3,
    to: Vec3,
    yaw: f32,
) -> Option<Vec3> {
    let water = physics.water()?;
    let floor = water - keel_depth(spec) - UNDER_KEEL;
    let flat = (to - from).with_y(0.0);
    let length = flat.length();
    let ahead = flat.normalize_or_zero();
    let side = Vec3::new(ahead.z, 0.0, -ahead.x);
    let turn = Quat::from_rotation_y(yaw);
    let (lo, hi) = spec.hull;
    let footprint = [
        Vec3::ZERO,
        Vec3::new(lo.x, 0.0, lo.z),
        Vec3::new(hi.x, 0.0, lo.z),
        Vec3::new(lo.x, 0.0, hi.z),
        Vec3::new(hi.x, 0.0, hi.z),
    ];
    let open = |p: Vec3| {
        footprint.iter().all(|&q| {
            let at = p + turn * q;
            let top = Vec3::new(at.x, water + OVER_WATER, at.z);
            physics
                .cast_ray_past(top, Vec3::NEG_Y, top.y - floor, id)
                .is_none()
        })
    };
    let steps = (length / AFLOAT_STEP).ceil() as usize;
    (1..=steps)
        .rev()
        .flat_map(|k| {
            let along = from.with_y(water) + ahead * (length * k as f32 / steps as f32);
            AFLOAT_SIDES.map(|s| along + side * s)
        })
        .find(|&p| open(p))
}

/// How deep a boat's keel floats under the water, metres.
fn keel_depth(spec: &VehicleSpec) -> f32 {
    FLOAT_DEPTH - spec.hull.0.y
}

/// Whether something stands within `within` metres of a vehicle's
/// bumper at its height (another parked close ahead, a wall): rays from
/// both front corners and the middle, a little inside the bumper.
pub(crate) fn blocked_ahead(
    physics: &Physics,
    id: VehicleId,
    hull: (Vec3, Vec3),
    within: f32,
) -> bool {
    let Some(m) = physics.vehicle_transform(id) else {
        return false;
    };
    let (lo, hi) = hull;
    let forward = m.transform_vector3(Vec3::Z);
    [lo.x + 0.1, 0.0, hi.x - 0.1].iter().any(|&x| {
        [0.8, 1.5].iter().any(|&h| {
            let from = m.transform_point3(Vec3::new(x, lo.y + h, hi.z - 0.2));
            physics
                .cast_ray_past(from, forward, within + 0.2, id)
                .is_some_and(|(_, n)| n.y < WALL)
        })
    })
}

/// How to steer a vehicle (-1 left to 1 right) to keep on its way round
/// what stands ahead ([`clear_way`]).
pub(crate) fn steer_round(physics: &Physics, id: VehicleId, hull: (Vec3, Vec3)) -> f32 {
    physics.vehicle_transform(id).map_or(0.0, |m| {
        let way = clear_way(physics, id, m, hull, 0.0, physics.vehicle_speed(id));
        (way * 2.0).clamp(-1.0, 1.0)
    })
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
    // Shots follow the crosshair's line from the camera.
    let dir = (target - p.eye()).normalize_or(Vec3::Z);
    let error = p.aim_forward().angle_between(dir);
    // The crosshair sits above the camera's line.
    let lift = p.aim_forward().y.asin() - p.camera_forward().y.asin();
    (
        turn_to(
            p,
            dir.x.atan2(dir.z),
            dir.y.clamp(-1.0, 1.0).asin() - lift,
            dt,
        ),
        error,
    )
}

/// Why a vehicle may be stuck: its speed, how it sits (nose up, rolled),
/// how many wheels touch, what is just ahead of its bumper, how the
/// ground rises ahead and what its body touches (where on the model).
pub(crate) fn stuck_report(
    world: &m2_engine::World,
    id: m2_engine::VehicleId,
    hull: (Vec3, Vec3),
    reversing: bool,
) -> String {
    let physics = &world.physics;
    let Some(m) = physics.vehicle_transform(id) else {
        return "not placed".to_owned();
    };
    let (lo, hi) = hull;
    let forward = m.transform_vector3(Vec3::Z);
    let right = m.transform_vector3(Vec3::X);
    let (down, wheels) = physics.vehicle_contacts(id);
    // Rays from the bumper at three heights, 6 m on.
    let ahead: Vec<String> = [0.3, 0.8, 1.5]
        .iter()
        .map(|&h| {
            let from = m.transform_point3(Vec3::new(0.0, lo.y + h, hi.z + 0.1));
            match physics.cast_ray_keyed(from, forward, 6.0) {
                Some((d, n, key)) => format!(
                    "{h} m up: {d:.1} m to {} (face {:.0} deg from flat)",
                    crate::static_name(world, key),
                    n.y.clamp(-1.0, 1.0).acos().to_degrees()
                ),
                None => format!("{h} m up: clear"),
            }
        })
        .collect();
    // The ground under the middle and 3, 6 and 10 m ahead.
    let ground = |p: Vec3| {
        let top = p + Vec3::Y * 30.0;
        physics
            .cast_ray(top, Vec3::NEG_Y, 80.0)
            .map(|(d, _)| top.y - d)
    };
    let centre = m.transform_point3((lo + hi) * 0.5);
    let flat = forward.with_y(0.0).normalize_or_zero();
    let rises: Vec<String> = [3.0, 6.0, 10.0]
        .iter()
        .map(|&k| match (ground(centre), ground(centre + flat * k)) {
            (Some(a), Some(b)) => format!("{:+.1} m at {k} m", b - a),
            _ => format!("? at {k} m"),
        })
        .collect();
    // What its body touches, where on the model, and the box what it
    // touches fills.
    let to_model = m.inverse();
    let touching: Vec<String> = physics
        .vehicle_touching(id)
        .iter()
        .map(|&(key, at, (lo, hi))| {
            format!(
                "{} at {:.1}, filling {:.1} to {:.1}",
                crate::static_name(world, key),
                to_model.transform_point3(at),
                lo,
                hi
            )
        })
        .collect();
    format!(
        "{:.1} m/s{}, nose {:+.0} deg, rolled {:+.0} deg, {down} of {wheels} wheels down; ahead [{}]; ground [{}]; body touching [{}]",
        physics.vehicle_speed(id),
        if reversing { " backing out" } else { "" },
        forward.y.clamp(-1.0, 1.0).asin().to_degrees(),
        right.y.clamp(-1.0, 1.0).asin().to_degrees(),
        ahead.join("; "),
        rises.join(", "),
        touching.join("; ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // A box of the carbine's rounds is wanted while it has room for them,
    // as walking over it takes it.
    #[test]
    fn rifle_ammunition_is_wanted_while_there_is_room() {
        use crate::flights::{Leaves, Pickup};
        use m2_engine::TemplateInfo;
        use m2_engine::pickup::{Leavings, PickupInfo};
        use std::collections::HashMap;
        let (gun, round, small) = (0x8000_0010, 0x8000_0011, 0x8000_0012);
        let templates = Templates {
            by_name: HashMap::from([(
                "carbine".to_owned(),
                TemplateInfo {
                    model: Some(0xabc),
                    vehicle: None,
                    engine: None,
                    person: false,
                    human: false,
                },
            )]),
            names: HashMap::from([
                (gun, "Carbine".to_owned()),
                (round, "Carbine Round".to_owned()),
                (small, "Ammo Pickup (Small)".to_owned()),
            ]),
            leavings: Leavings {
                pickups: HashMap::from([(
                    small,
                    PickupInfo {
                        kind: 1,
                        radius: 1.0,
                        ammo: Some(round),
                        rounds: 3,
                        gives: vec![(round, 3)],
                        walked: vec![(round, 3)],
                        heals: 0,
                        cue: None,
                        effect: None,
                    },
                )]),
                // The carbine fires the round, by its model.
                ammo: HashMap::from([(gun, round)]),
                ..Leavings::default()
            },
            ..Templates::default()
        };
        let mut combat = Combat::default();
        combat.carbine.template = Some(gun);
        combat.carbine_only();
        let p = Pickup {
            from: 0x4000_0001,
            what: Leaves {
                template: small,
                ..Leaves::default()
            },
            at: Vec3::ZERO,
            yaw: 0.0,
        };
        assert!(combat.rifle_rounds.stored < combat.rifle.spare);
        assert!(wanted(&templates, &combat, &p));
        combat.rifle_rounds.stored = combat.rifle.spare;
        assert!(!wanted(&templates, &combat, &p));
        // Nothing of hers takes it: not wanted.
        assert!(!wanted(&Templates::default(), &Combat::default(), &p));
    }

    #[test]
    fn a_target_behind_a_wall_is_seen_from_round_it() {
        use m2_engine::{CollisionShape, StaticKey};
        use m2_math::glam::Quat;
        let mut p = Physics::new();
        let quad = |a: Vec3, b: Vec3, c: Vec3, d: Vec3| {
            CollisionShape::mesh(&[a, b, c, d], &[[0, 1, 2], [0, 2, 3]]).unwrap()
        };
        let ground = quad(
            Vec3::new(-50.0, 0.0, -50.0),
            Vec3::new(50.0, 0.0, -50.0),
            Vec3::new(50.0, 0.0, 50.0),
            Vec3::new(-50.0, 0.0, 50.0),
        );
        // A wall 3 m high across her way, 2 m ahead.
        let wall = quad(
            Vec3::new(-10.0, 0.0, 2.0),
            Vec3::new(10.0, 0.0, 2.0),
            Vec3::new(10.0, 3.0, 2.0),
            Vec3::new(-10.0, 3.0, 2.0),
        );
        p.add_static(StaticKey::Cell(1), &ground, Vec3::ZERO, Quat::IDENTITY);
        p.add_static(StaticKey::Cell(2), &wall, Vec3::ZERO, Quat::IDENTITY);
        p.step(1.0 / 60.0);
        let target = Vec3::new(0.0, 1.3, 4.0);
        let spot = vantage(&p, target, Vec3::ZERO, &[]).expect("somewhere to see it from");
        assert!(spot.z > 2.0, "still behind the wall at {spot}");
        assert!(spot.y.abs() < 0.1, "not on the ground: {spot}");
        // Not the same spot twice.
        let next = vantage(&p, target, Vec3::ZERO, &[spot]).expect("another");
        assert!(next.distance(spot) >= 4.0, "{next} is {spot} again");
    }

    // A wall across the way with a gap to its left: the driver steers
    // for the gap, and straight on once the way is clear.
    #[test]
    fn drivers_steer_round_a_wall() {
        use m2_engine::vehicle::WheelSpec;
        use m2_engine::{CollisionShape, StaticKey, VehicleSpec};
        let mut p = Physics::new();
        let quad = |a: Vec3, b: Vec3, c: Vec3, d: Vec3| {
            CollisionShape::mesh(&[a, b, c, d], &[[0, 1, 2], [0, 2, 3]]).unwrap()
        };
        let ground = quad(
            Vec3::new(-50.0, 0.0, -50.0),
            Vec3::new(50.0, 0.0, -50.0),
            Vec3::new(50.0, 0.0, 50.0),
            Vec3::new(-50.0, 0.0, 50.0),
        );
        // Facing +z, left is +x: the wall ends 4 m to the left.
        let wall = quad(
            Vec3::new(-30.0, 0.0, 10.0),
            Vec3::new(4.0, 0.0, 10.0),
            Vec3::new(4.0, 3.0, 10.0),
            Vec3::new(-30.0, 3.0, 10.0),
        );
        p.add_static(StaticKey::Cell(1), &ground, Vec3::ZERO, Quat::IDENTITY);
        p.add_static(StaticKey::Cell(2), &wall, Vec3::ZERO, Quat::IDENTITY);
        p.step(1.0 / 60.0);
        let wheel = |x: f32, z: f32| WheelSpec {
            centre: Vec3::new(x, 0.35, z),
            radius: 0.35,
            width: 0.25,
            steers: z > 0.0,
        };
        let hull = (Vec3::new(-1.0, 0.0, -2.2), Vec3::new(1.0, 1.5, 2.2));
        let spec = VehicleSpec {
            model: 1,
            kind: "car".into(),
            label: "test".into(),
            node_names: Vec::new(),
            armed: Vec::new(),
            wheels: vec![
                wheel(-0.9, 1.4),
                wheel(0.9, 1.4),
                wheel(-0.9, -1.3),
                wheel(0.9, -1.3),
            ],
            bounds: hull,
            seat: None,
            door: None,
            tracked: false,
            road_wheels: Vec::new(),
            rest: Vec::new(),
            node_wheel: Vec::new(),
            mass: 1400.0,
            turret: Vec::new(),
            pivot: None,
            muzzle: None,
            gun: Vec::new(),
            gun_pivot: None,
            muzzle_on_gun: false,
            elevation: (0.0, 0.0),
            hull,
            turret_box: None,
            rotors: Vec::new(),
            damage: None,
            tuning: Default::default(),
        };
        let id = p.add_vehicle(&spec, Vec3::new(0.0, 0.1, 0.0), 0.0);
        p.step(1.0 / 60.0);
        let m = p.vehicle_transform(id).unwrap();
        let target = Vec3::new(0.0, 0.0, 40.0);
        let (want, _) = heading_to(m, target);
        let way = clear_way(&p, id, m, hull, want, 5.0);
        assert!(way < -0.2, "steers {way} (left is negative)");
        // Past the wall's end: straight on.
        let m = Mat4::from_translation(Vec3::new(8.0, 0.0, 0.0));
        p.remove_vehicle(id);
        let id = p.add_vehicle(&spec, Vec3::new(8.0, 0.1, 0.0), 0.0);
        p.step(1.0 / 60.0);
        let (want, _) = heading_to(m, Vec3::new(8.0, 0.0, 40.0));
        assert_eq!(clear_way(&p, id, m, hull, want, 5.0), want);
    }

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
