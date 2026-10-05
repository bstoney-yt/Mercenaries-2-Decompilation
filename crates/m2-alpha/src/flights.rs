//! The mission scripts' deliveries by helicopter (`Ai.Deliver`): the first
//! contract's weapon crate, jeep and tank come slung under a helicopter
//! that flies in from where the scripts made it, lowers its load to a few
//! metres over the drop point, lets go and flies off, as the playthroughs
//! show (the jeep is dropped about 3 m and bounces on its wheels).
//!
//! The scripts only hear of the arrival
//! ([`Mission::arrived`](m2_game::mission::Mission::arrived)); the flight
//! itself (speed, heights, the cable, how long it hovers) is invented.

use std::collections::HashMap;

use m2_assets::vehicle::Rotor;
use m2_engine::{CollisionShape, GpuModel, Physics, StaticKey};
use m2_math::glam::{Mat4, Quat, Vec3};
use m2_render::DrawQueue;

use crate::combat::Strike;
use crate::scene::ParkedVehicle;

/// Cruising speed, m/s, and how quickly it speeds up and slows down,
/// m/s².
const CRUISE: f32 = 25.0;
const ACCEL: f32 = 4.0;
const BRAKE: f32 = 3.0;
/// Fastest it climbs or comes down, m/s.
const CLIMB: f32 = 6.0;
/// It comes down from its cruising height over this last stretch to the
/// drop point, metres.
const DESCENT: f32 = 150.0;
/// Its load's underside clears the ground ahead by at least this much on
/// the way (but over the last [`FINAL`] metres), metres.
const CLEARANCE: f32 = 12.0;
const FINAL: f32 = 40.0;
/// The ground is looked at this far ahead on its way (and where it will
/// be in two seconds), metres.
const LOOK_AHEAD: [f32; 4] = [0.0, 20.0, 40.0, 60.0];
/// The cable from the hook under it to the top of its load, and how far
/// above the ground the load is let go, metres.
const CABLE: f32 = 5.0;
const DROP: f32 = 3.0;
/// Seconds it hovers over the drop point before letting go, and after.
const HOVER: f32 = 1.5;
const LINGER: f32 = 2.0;
/// One not hovering over its drop point after this many seconds on its
/// way is put there, so the scripts never wait on it for ever.
const INBOUND_LONGEST: f32 = 180.0;
/// Leaving, it climbs to this far over where it hovered (or its cruising
/// height, if higher), metres, and is gone this far from the player or
/// after this many seconds.
const LEAVE_RISE: f32 = 60.0;
const VANISH: f32 = 900.0;
const LEAVE_TIME: f32 = 60.0;
/// Turning to face where it goes, radians a second; it faces its way when
/// faster than this, m/s.
const TURN: f32 = 1.0;
const HEADING: f32 = 3.0;
/// It leans into its acceleration as a helicopter does (radians per m/s²,
/// and per m/s of speed against the air), at most this far, radians.
const LEAN: f32 = 0.06;
const LEAN_DRAG: f32 = 0.006;
const LEAN_MOST: f32 = 0.35;
/// Rotor speeds, radians a second: main and tail.
const MAIN_ROTOR: f32 = 30.0;
const TAIL_ROTOR: f32 = 90.0;
/// A load whose model hasn't loaded is taken to be this tall, metres.
const LOAD_HEIGHT: f32 = 2.0;
/// The ground found under the drop point counts if within this far of
/// the scripts' height for it, metres.
const GROUND_SLACK: f32 = 15.0;
/// Downward acceleration of a crate let go, m/s².
const GRAVITY: f32 = 9.81;
/// A delivered crate's health (the first contract's supply drop's
/// template gives it 100): blows and rounds that take it to nothing break
/// it open, as does a blast this near, metres.
const CRATE_HP: f32 = 100.0;
const CRATE_BLAST: f32 = 6.0;

/// A helicopter or crate model, once loaded: drawn with its rotors turning.
pub struct FlightModel {
    pub gpu: GpuModel,
    /// Each node's place in the model at rest.
    pub rest: Vec<Mat4>,
    pub rotors: Vec<Rotor>,
}

/// What hangs from a helicopter's winch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Load {
    /// The scripts' object.
    pub guid: u32,
    /// The model of one that isn't a vehicle (a supply crate); a vehicle
    /// is one of the scene's ([`ParkedVehicle::guid`]).
    pub model: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Inbound,
    Hovering,
    Leaving,
}

/// What hangs from the helicopters this frame: the cables (hook to load)
/// and the vehicles (index in the scene's, model-to-world).
#[derive(Debug, Default)]
pub struct Hanging {
    pub cables: Vec<(Vec3, Vec3)>,
    pub hung: Vec<(usize, Mat4)>,
}

/// A delivery's helicopter: the scripts' object and its pilot, its model
/// and the sound bank its template names, if known.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Copter {
    pub guid: u32,
    pub pilot: u32,
    pub model: Option<u32>,
    pub bank: Option<u32>,
}

/// A helicopter on a delivery.
#[derive(Debug)]
struct Flight {
    copter: u32,
    pilot: u32,
    model: Option<u32>,
    bank: Option<u32>,
    /// Hanging from its winch; let go at the drop point.
    loads: Vec<Load>,
    /// The drop point: the ground there.
    to: Vec3,
    at: Vec3,
    yaw: f32,
    /// Horizontal, m/s.
    velocity: Vec3,
    climb: f32,
    /// Leaning into its acceleration: its up axis's tilt.
    lean: Vec3,
    /// Its cruising height.
    cruise: f32,
    /// Where it goes once it has let go (the scripts' `MoveTo`).
    home: Option<Vec3>,
    phase: Phase,
    /// Seconds in the phase.
    clock: f32,
    rotor: f32,
    released: bool,
}

/// A crate let go: falling, then lying where it landed, until broken
/// open.
#[derive(Debug)]
struct Crate {
    guid: u32,
    model: u32,
    at: Vec3,
    yaw: f32,
    falling: Option<f32>,
    hp: f32,
    /// Broken open: drawn wrecked (if its model has a wreck), no longer
    /// in the way.
    broken: bool,
}

/// A load let go: the pilot whose delivery it was and where (for the
/// scripts), and the vehicles put down in play there.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub pilot: u32,
    pub to: Vec3,
    pub dropped: Vec<u32>,
}

/// The deliveries in flight, and the crates they brought.
#[derive(Default)]
pub struct Flights {
    flights: Vec<Flight>,
    crates: Vec<Crate>,
    /// By hash: `None` while it loads (or if it failed to).
    models: HashMap<u32, Option<FlightModel>>,
    /// Wanted and not yet asked for.
    wanted: Vec<u32>,
    /// Crates broken open since [`take_broken`](Self::take_broken).
    broken: Vec<(u32, Vec3)>,
}

impl std::fmt::Debug for Flights {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Flights")
            .field("flights", &self.flights)
            .field("crates", &self.crates)
            .field("models", &self.models.len())
            .finish()
    }
}

impl Flights {
    /// Starts helicopter `copter` from `from` to the drop point `to` (on
    /// the ground) with `loads` on its winch.
    pub fn start(&mut self, copter: Copter, loads: Vec<Load>, from: Vec3, to: Vec3) {
        let Copter {
            guid: copter,
            pilot,
            model,
            bank,
        } = copter;
        for m in model
            .into_iter()
            .chain(loads.iter().filter_map(|l| l.model))
        {
            if let std::collections::hash_map::Entry::Vacant(e) = self.models.entry(m) {
                e.insert(None);
                self.wanted.push(m);
            }
        }
        let flat = (to - from).with_y(0.0);
        self.flights.retain(|f| f.copter != copter);
        self.flights.push(Flight {
            copter,
            pilot,
            model,
            bank,
            loads,
            to,
            at: from,
            yaw: flat.x.atan2(flat.z),
            velocity: Vec3::ZERO,
            climb: 0.0,
            lean: Vec3::ZERO,
            cruise: from.y,
            home: None,
            phase: Phase::Inbound,
            clock: 0.0,
            rotor: 0.0,
            released: false,
        });
    }

    /// Models to load ([`loaded`](Self::loaded) takes them).
    pub fn take_wanted(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.wanted)
    }

    /// A model asked for has loaded.
    pub fn loaded(&mut self, hash: u32, model: FlightModel) {
        self.models.insert(hash, Some(model));
    }

    /// The pilot of a delivery that has let go is sent `to` (home).
    pub fn send_home(&mut self, pilot: u32, to: Vec3) {
        for f in self.flights.iter_mut().filter(|f| f.pilot == pilot) {
            f.home = Some(to);
        }
    }

    /// The scripts no longer have `guid`: a crate of theirs goes, and what
    /// stood in the way with it (a helicopter flies off out of sight
    /// first).
    pub fn removed(&mut self, guid: u32, physics: Option<&mut Physics>) {
        if self.crates.iter().any(|c| c.guid == guid)
            && let Some(physics) = physics
        {
            physics.remove_static(StaticKey::Entity(guid));
        }
        self.crates.retain(|c| c.guid != guid);
    }

    /// Blows and rounds (`struck`) and blasts reach the crates down: one
    /// taken to no health or caught in a blast breaks open and no longer
    /// stands in the way. Returns those broken now (guid, where).
    pub fn strike(
        &mut self,
        physics: &mut Physics,
        struck: &[Strike],
        blasts: &[Vec3],
    ) -> Vec<(u32, Vec3)> {
        let mut now = Vec::new();
        for c in self
            .crates
            .iter_mut()
            .filter(|c| c.falling.is_none() && !c.broken)
        {
            let key = StaticKey::Entity(c.guid);
            c.hp -= struck
                .iter()
                .filter(|s| s.key == Some(key))
                .map(|s| s.damage)
                .sum::<f32>();
            if blasts.iter().any(|b| b.distance(c.at) < CRATE_BLAST) {
                c.hp = 0.0;
            }
            if c.hp <= 0.0 {
                c.broken = true;
                physics.remove_static(key);
                now.push((c.guid, c.at));
            }
        }
        self.broken.extend(&now);
        now
    }

    /// The crates broken open since the last call: each one's guid and
    /// where it stood.
    pub fn take_broken(&mut self) -> Vec<(u32, Vec3)> {
        std::mem::take(&mut self.broken)
    }

    /// Where crate `guid` stands, down and whole: its box's middle and
    /// how far its sides are from it across the ground (the larger),
    /// metres.
    pub fn crate_standing(&self, guid: u32) -> Option<(Vec3, f32)> {
        let c = self
            .crates
            .iter()
            .find(|c| c.guid == guid && c.falling.is_none() && !c.broken)?;
        let (lo, hi) = self
            .models
            .get(&c.model)
            .and_then(Option::as_ref)
            .map_or((Vec3::new(-0.5, 0.0, -0.5), Vec3::splat(0.5)), |m| {
                m.gpu.bounds
            });
        let turn = Quat::from_rotation_y(c.yaw);
        let half = (hi - lo) * 0.5;
        Some((c.at + turn * ((lo + hi) * 0.5), half.x.max(half.z)))
    }

    /// The crates down and whole: each one's guid.
    pub fn crates_standing(&self) -> impl Iterator<Item = u32> + '_ {
        self.crates
            .iter()
            .filter(|c| c.falling.is_none() && !c.broken)
            .map(|c| c.guid)
    }

    /// Whether `pilot` is flying a delivery.
    pub fn flying(&self, pilot: u32) -> bool {
        self.flights.iter().any(|f| f.pilot == pilot)
    }

    /// Whether `guid` hangs from a helicopter, not yet let go.
    pub fn carrying(&self, guid: u32) -> bool {
        self.flights
            .iter()
            .any(|f| !f.released && f.loads.iter().any(|l| l.guid == guid))
    }

    /// The helicopters hovering over their drop points: each one's guid,
    /// where it is and the drop point.
    pub fn hovering(&self) -> Vec<(u32, Vec3, Vec3)> {
        self.flights
            .iter()
            .filter(|f| f.phase == Phase::Hovering)
            .map(|f| (f.copter, f.at, f.to))
            .collect()
    }

    /// What hangs from helicopter `copter`: each load, where it hangs
    /// (its origin) and, for a vehicle, which of the scene's it is (none
    /// while its model loads, when only its cable is drawn).
    pub fn hung_from(
        &self,
        copter: u32,
        vehicles: &[ParkedVehicle],
    ) -> Vec<(Load, Vec3, Option<usize>)> {
        self.flights
            .iter()
            .filter(|f| f.copter == copter && !f.released)
            .flat_map(|f| {
                f.loads.iter().map(move |l| {
                    let (at, _) = self.hanging(f, l, vehicles);
                    let found = vehicles.iter().position(|v| v.guid == Some(l.guid));
                    (*l, at, found)
                })
            })
            .collect()
    }

    /// The helicopters in the air, to be heard: each one's guid, where it
    /// is and its sound bank, if its template names one.
    pub fn sounding(&self) -> Vec<(u32, Vec3, Option<u32>)> {
        self.flights
            .iter()
            .map(|f| (f.copter, f.at, f.bank))
            .collect()
    }

    /// Where the scripts' objects in flight are: each helicopter and its
    /// pilot, what still hangs from it, and the crates let go (guid,
    /// position, yaw).
    pub fn places(&self, vehicles: &[ParkedVehicle]) -> Vec<(u32, Vec3, f32)> {
        let mut out = Vec::new();
        for f in self.flights.iter().filter(|f| f.phase != Phase::Leaving) {
            out.push((f.copter, f.at, f.yaw));
            out.push((f.pilot, f.at, f.yaw));
            if !f.released {
                for l in &f.loads {
                    let (at, _) = self.hanging(f, l, vehicles);
                    out.push((l.guid, at, f.yaw));
                }
            }
        }
        out.extend(self.crates.iter().map(|c| (c.guid, c.at, c.yaw)));
        out
    }

    /// Flies them on by `dt` seconds (`focus`: where the player is), and
    /// lets go of loads over their drop points: vehicles of the scene's
    /// are put down in `physics` where they hang. Returns what was let go.
    pub fn step(
        &mut self,
        dt: f32,
        physics: &mut Physics,
        vehicles: &mut [ParkedVehicle],
        focus: Vec3,
    ) -> Vec<Release> {
        let ground = |physics: &Physics, p: Vec3| {
            let top = Vec3::new(p.x, p.y + 300.0, p.z);
            physics
                .cast_ray(top, Vec3::NEG_Y, 900.0)
                .map(|(t, _)| top.y - t)
        };
        let mut releases = Vec::new();
        for i in 0..self.flights.len() {
            // The drop point on the ground as it has loaded (the scripts'
            // height may be a little off), not a roof or a tree over it.
            if self.flights[i].phase == Phase::Inbound {
                let to = self.flights[i].to;
                if let Some(g) = ground(physics, to).filter(|g| (g - to.y).abs() < GROUND_SLACK) {
                    self.flights[i].to.y = g;
                }
            }
            let hover = self.hover_height(&self.flights[i], vehicles);
            let f = &mut self.flights[i];
            f.clock += dt;
            f.rotor = (f.rotor + dt) % 1000.0;
            let before = f.velocity;
            match f.phase {
                Phase::Inbound => {
                    let flat = (f.to - f.at).with_y(0.0);
                    let d = flat.length();
                    let speed = CRUISE.min((2.0 * BRAKE * (d - 0.3).max(0.0)).sqrt());
                    let want = flat.normalize_or_zero() * speed;
                    f.velocity += (want - f.velocity).clamp_length_max(ACCEL * dt);
                    // Down from its cruising height over the last stretch,
                    // its load clear of the ground ahead on the way.
                    let k = (d / DESCENT).clamp(0.0, 1.0);
                    let mut height =
                        hover + (f.cruise.max(hover) - hover) * (k * k * (3.0 - 2.0 * k));
                    if d > FINAL {
                        let way = flat.normalize_or_zero();
                        let highest = LOOK_AHEAD
                            .iter()
                            .map(|&m| f.at + way * m.min(d - FINAL))
                            .chain([f.at + f.velocity * 2.0])
                            .filter_map(|p| ground(physics, p))
                            .reduce(f32::max);
                        if let Some(g) = highest {
                            let hang = hover - f.to.y - DROP;
                            height = height.max(g + CLEARANCE + hang);
                        }
                    }
                    f.climb = (height - f.at.y).clamp(-CLIMB, CLIMB);
                    f.at += (f.velocity + Vec3::Y * f.climb) * dt;
                    let over = d < 0.6 && f.velocity.length() < 0.6 && (f.at.y - hover).abs() < 0.4;
                    if over || f.clock > INBOUND_LONGEST {
                        f.phase = Phase::Hovering;
                        f.clock = 0.0;
                    }
                }
                Phase::Hovering => {
                    f.velocity = Vec3::ZERO;
                    let bob = (f.clock * 1.7).sin() * 0.12;
                    f.at = Vec3::new(f.to.x, hover + bob, f.to.z);
                    if !f.released && f.clock >= HOVER {
                        f.released = true;
                        releases.push(Release {
                            pilot: f.pilot,
                            to: f.to,
                            dropped: Vec::new(),
                        });
                    }
                    if f.clock >= HOVER + LINGER {
                        f.phase = Phase::Leaving;
                        f.clock = 0.0;
                    }
                }
                Phase::Leaving => {
                    let ahead = Vec3::new(f.yaw.sin(), 0.0, f.yaw.cos());
                    let way = f
                        .home
                        .map(|h| (h - f.at).with_y(0.0))
                        .filter(|w| w.length() > 30.0)
                        .map_or(ahead, Vec3::normalize);
                    let want = way * CRUISE * 1.2;
                    f.velocity += (want - f.velocity).clamp_length_max(ACCEL * dt);
                    let height = (hover + LEAVE_RISE).max(f.cruise);
                    f.climb = (height - f.at.y).clamp(-CLIMB, CLIMB);
                    f.at += (f.velocity + Vec3::Y * f.climb) * dt;
                }
            }
            // Facing its way; leaning into its acceleration and against
            // the air.
            if f.velocity.length() > HEADING {
                let want = f.velocity.x.atan2(f.velocity.z);
                let turn = wrap(want - f.yaw).clamp(-TURN * dt, TURN * dt);
                f.yaw = wrap(f.yaw + turn);
            }
            let accel = (f.velocity - before) / dt.max(1e-3);
            let lean = (accel * LEAN + f.velocity * LEAN_DRAG).clamp_length_max(LEAN_MOST);
            f.lean += (lean - f.lean) * (3.0 * dt).min(1.0);
        }
        // Let go: they fall from where they hang.
        for r in &mut releases {
            let Some(f) = self.flights.iter().find(|f| f.pilot == r.pilot) else {
                continue;
            };
            let hung: Vec<(Load, Vec3, Quat, f32)> = f
                .loads
                .iter()
                .map(|l| {
                    let (at, turn) = self.hanging(f, l, vehicles);
                    (*l, at, turn, f.yaw)
                })
                .collect();
            for (l, at, turn, yaw) in hung {
                eprintln!(
                    "delivery: {} lets go of {} ({}) at {at:.1}, {:.1} m over the drop point {:.1}",
                    r.pilot,
                    l.guid,
                    if l.model.is_some() {
                        "a crate"
                    } else {
                        "a vehicle"
                    },
                    at.y - r.to.y,
                    r.to
                );
                match l.model {
                    Some(model) => self.crates.push(Crate {
                        guid: l.guid,
                        model,
                        at,
                        yaw,
                        falling: Some(0.0),
                        hp: CRATE_HP,
                        broken: false,
                    }),
                    None => {
                        if let Some(v) = vehicles
                            .iter_mut()
                            .find(|v| v.guid == Some(l.guid) && v.id.is_none())
                        {
                            v.id = Some(physics.add_vehicle_turned(&v.spec, at, turn));
                            r.dropped.push(l.guid);
                        }
                    }
                }
            }
        }
        for c in &mut self.crates {
            let Some(speed) = &mut c.falling else {
                continue;
            };
            *speed += GRAVITY * dt;
            c.at.y -= *speed * dt;
            let floor = ground(physics, c.at + Vec3::Y * 2.0);
            if let Some(g) = floor
                && c.at.y <= g
            {
                c.at.y = g;
                c.falling = None;
                // Down, it stands in the way: a box round its model.
                if let Some(m) = self.models.get(&c.model).and_then(Option::as_ref) {
                    let (lo, hi) = m.gpu.bounds;
                    let turn = Quat::from_rotation_y(c.yaw);
                    physics.add_static(
                        StaticKey::Entity(c.guid),
                        &CollisionShape::cuboid((hi - lo) * 0.5),
                        c.at + turn * ((lo + hi) * 0.5),
                        turn,
                    );
                }
            }
        }
        self.flights.retain(|f| {
            f.phase != Phase::Leaving
                || (f.clock < LEAVE_TIME && f.at.with_y(0.0).distance(focus.with_y(0.0)) < VANISH)
        });
        releases
    }

    /// Queues the helicopters, the crates hanging from them and the crates
    /// let go; returns the cables to draw and the vehicles hanging, for the
    /// scene to draw posed.
    pub fn draw(&self, queue: &mut DrawQueue, vehicles: &[ParkedVehicle]) -> Hanging {
        let mut cables = Vec::new();
        let mut hung = Vec::new();
        for f in &self.flights {
            let body = self.body(f);
            if let Some(m) = f.model.and_then(|m| self.models.get(&m)?.as_ref()) {
                let mut nodes = m.rest.clone();
                for r in &m.rotors {
                    let main = r.axis[1] > 0.5;
                    let angle = f.rotor * if main { MAIN_ROTOR } else { TAIL_ROTOR };
                    let hub = Vec3::from(r.centre);
                    let spin = Mat4::from_translation(hub)
                        * Mat4::from_axis_angle(Vec3::from(r.axis), angle)
                        * Mat4::from_translation(-hub);
                    for &n in &r.nodes {
                        if let Some(node) = nodes.get_mut(usize::from(n)) {
                            *node = spin * *node;
                        }
                    }
                }
                m.gpu.draw_moved(queue, body, &nodes, false, &[]);
            }
            if f.released {
                continue;
            }
            let hook = body.transform_point3(self.hook(f));
            for l in &f.loads {
                let (at, turn) = self.hanging(f, l, vehicles);
                let place = Mat4::from_rotation_translation(turn, at);
                let top = match l.model {
                    Some(model) => {
                        let Some(m) = self.models.get(&model).and_then(Option::as_ref) else {
                            continue;
                        };
                        m.gpu.draw(queue, place);
                        m.gpu.bounds.1.y
                    }
                    None => {
                        let Some(i) = vehicles.iter().position(|v| v.guid == Some(l.guid)) else {
                            continue;
                        };
                        hung.push((i, place));
                        vehicles[i].model.bounds.1.y
                    }
                };
                cables.push((hook, at + Vec3::Y * top));
            }
        }
        for c in &self.crates {
            if let Some(m) = self.models.get(&c.model).and_then(Option::as_ref)
                && (!c.broken || m.gpu.has_wreck())
            {
                m.gpu.draw_as(
                    queue,
                    Mat4::from_rotation_translation(Quat::from_rotation_y(c.yaw), c.at),
                    c.broken,
                );
            }
        }
        Hanging { cables, hung }
    }

    /// The helicopter's model-to-world transform: placed, turned to its
    /// heading, leaning.
    fn body(&self, f: &Flight) -> Mat4 {
        let up = (Vec3::Y + f.lean).normalize();
        Mat4::from_rotation_translation(
            Quat::from_rotation_arc(Vec3::Y, up) * Quat::from_rotation_y(f.yaw),
            f.at,
        )
    }

    /// Where the cable hangs from, model space: under its middle.
    fn hook(&self, f: &Flight) -> Vec3 {
        let low = f
            .model
            .and_then(|m| self.models.get(&m)?.as_ref())
            .map_or(0.0, |m| m.gpu.bounds.0.y);
        Vec3::new(0.0, low, 0.0)
    }

    /// A load's box (model space), if its model has loaded.
    fn load_box(&self, l: &Load, vehicles: &[ParkedVehicle]) -> Option<(Vec3, Vec3)> {
        match l.model {
            Some(m) => self.models.get(&m)?.as_ref().map(|m| m.gpu.bounds),
            None => vehicles
                .iter()
                .find(|v| v.guid == Some(l.guid))
                .map(|v| v.model.bounds),
        }
    }

    /// Where load `l` hangs (its origin) and how it is turned: level, as
    /// the helicopter heads, its top [`CABLE`] below the hook.
    fn hanging(&self, f: &Flight, l: &Load, vehicles: &[ParkedVehicle]) -> (Vec3, Quat) {
        let turn = Quat::from_rotation_y(f.yaw);
        let hook = self.body(f).transform_point3(self.hook(f));
        let (lo, hi) = self
            .load_box(l, vehicles)
            .unwrap_or((Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, LOAD_HEIGHT, 1.0)));
        let top = Vec3::new((lo.x + hi.x) * 0.5, hi.y, (lo.z + hi.z) * 0.5);
        (hook - Vec3::Y * CABLE - turn * top, turn)
    }

    /// The height it hovers at over its drop point: its loads' undersides
    /// [`DROP`] metres over the ground.
    fn hover_height(&self, f: &Flight, vehicles: &[ParkedVehicle]) -> f32 {
        let tallest = f
            .loads
            .iter()
            .map(|l| {
                self.load_box(l, vehicles)
                    .map_or(LOAD_HEIGHT, |(lo, hi)| hi.y - lo.y)
            })
            .fold(0.0, f32::max);
        f.to.y + DROP + tallest + CABLE - self.hook(f).y
    }
}

/// An angle in -π..π.
fn wrap(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    (a + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI
}

#[cfg(test)]
mod tests {
    use super::*;

    const COPTER: Copter = Copter {
        guid: 1,
        pilot: 2,
        model: None,
        bank: None,
    };

    #[test]
    fn a_delivery_flies_in_lets_go_over_the_drop_point_and_leaves() {
        let mut physics = Physics::new();
        let mut flights = Flights::default();
        let to = Vec3::new(0.0, 0.0, 120.0);
        let load = Load {
            guid: 3,
            model: None,
        };
        flights.start(COPTER, vec![load], Vec3::new(0.0, 60.0, 0.0), to);
        assert!(flights.flying(2));
        let dt = 1.0 / 60.0;
        let mut released = None;
        let mut lowest = f32::MAX;
        for k in 0..60 * 60 {
            let places = flights.places(&[]);
            if let (Some(c), Some(l)) = (
                places.iter().find(|p| p.0 == 1),
                places.iter().find(|p| p.0 == 3),
            ) {
                // The load hangs under it on the cable.
                assert!(
                    (c.1.y - l.1.y - CABLE - LOAD_HEIGHT).abs() < 0.5,
                    "{c:?} {l:?}"
                );
                lowest = lowest.min(l.1.y);
            }
            let r = flights.step(dt, &mut physics, &mut [], Vec3::ZERO);
            if let Some(r) = r.first() {
                released = Some((k as f32 * dt, r.clone()));
                break;
            }
        }
        let (when, r) = released.expect("let go");
        assert_eq!(r.pilot, 2);
        assert_eq!(r.to, to);
        // About 120 m at up to 25 m/s, slowing to a hover: well under a
        // minute, not at once.
        assert!((5.0..40.0).contains(&when), "let go after {when} s");
        // Its load let go 3 m over the drop point.
        assert!((lowest - DROP).abs() < 0.5, "lowest {lowest}");
        let copter = flights.places(&[])[0].1;
        assert!(copter.with_y(0.0).distance(to) < 1.0, "over {copter}");
        // Then it flies off and is gone.
        for _ in 0..(LEAVE_TIME as usize + 5) * 60 {
            flights.step(dt, &mut physics, &mut [], Vec3::ZERO);
        }
        assert!(!flights.flying(2));
    }

    #[test]
    fn a_crate_down_breaks_open_to_a_blow_or_a_blast() {
        let mut physics = Physics::new();
        let floor = CollisionShape::cuboid(Vec3::new(500.0, 1.0, 500.0));
        physics.add_static(
            StaticKey::Cell(1),
            &floor,
            Vec3::new(0.0, -1.0, 0.0),
            Quat::IDENTITY,
        );
        physics.step(1.0 / 60.0);
        let mut flights = Flights::default();
        let crates = [3, 4].map(|guid| Load {
            guid,
            model: Some(99),
        });
        flights.start(
            COPTER,
            crates.to_vec(),
            Vec3::new(0.0, 30.0, 0.0),
            Vec3::new(0.0, 0.0, 60.0),
        );
        for _ in 0..60 * 60 {
            flights.step(1.0 / 60.0, &mut physics, &mut [], Vec3::ZERO);
            if flights.crates_standing().count() == 2 {
                break;
            }
        }
        assert_eq!(flights.crates_standing().count(), 2, "both down");
        let hit = |damage| Strike {
            at: Vec3::ZERO,
            damage,
            key: Some(StaticKey::Entity(3)),
        };
        // A round chips it; a bash breaks it open.
        assert!(flights.strike(&mut physics, &[hit(34.0)], &[]).is_empty());
        let broke = flights.strike(&mut physics, &[hit(100.0)], &[]);
        assert_eq!(broke.iter().map(|b| b.0).collect::<Vec<_>>(), [3]);
        assert!(flights.crate_standing(3).is_none());
        // A blast beside the other.
        let (at, _) = flights.crate_standing(4).expect("still whole");
        let broke = flights.strike(&mut physics, &[], &[at + Vec3::X * 3.0]);
        assert_eq!(broke.iter().map(|b| b.0).collect::<Vec<_>>(), [4]);
        assert_eq!(flights.take_broken().len(), 2);
        assert!(flights.take_broken().is_empty());
    }

    #[test]
    fn it_flies_over_the_ground_on_its_way() {
        let mut physics = Physics::new();
        // A 40 m hill halfway, under its way in.
        let hill = m2_engine::CollisionShape::mesh(
            &[
                Vec3::new(-50.0, 40.0, 40.0),
                Vec3::new(50.0, 40.0, 40.0),
                Vec3::new(50.0, 40.0, 80.0),
                Vec3::new(-50.0, 40.0, 80.0),
            ],
            &[[0, 2, 1], [0, 3, 2]],
        )
        .unwrap();
        physics.add_static(
            m2_engine::StaticKey::Cell(1),
            &hill,
            Vec3::ZERO,
            Quat::IDENTITY,
        );
        physics.step(1.0 / 60.0);
        let mut flights = Flights::default();
        let load = Load {
            guid: 3,
            model: None,
        };
        flights.start(
            COPTER,
            vec![load],
            Vec3::new(0.0, 45.0, 0.0),
            Vec3::new(0.0, 0.0, 200.0),
        );
        let mut lowest_over_hill = f32::MAX;
        for _ in 0..60 * 60 {
            if let Some(l) = flights.places(&[]).iter().find(|p| p.0 == 3)
                && (40.0..80.0).contains(&l.1.z)
            {
                lowest_over_hill = lowest_over_hill.min(l.1.y);
            }
            if !flights
                .step(1.0 / 60.0, &mut physics, &mut [], Vec3::ZERO)
                .is_empty()
            {
                break;
            }
        }
        assert!(
            lowest_over_hill > 40.0 + 5.0,
            "load at {lowest_over_hill} over the hill"
        );
    }
}
