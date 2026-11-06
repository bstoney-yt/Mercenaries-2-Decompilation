//! A pursuit's cars: the ones its faction's pursuit lists send after her
//! by road ([`m2_engine::road_ring`]), driven at her by the 360's AI
//! driver ([`m2_engine::car_driver`]) with their gunners firing on her.
//!
//! From the 360 code:
//! - The pursuit tick (0x822371F8) sends a kind of unit while its count
//!   for her state and the level (the `e480240b` table) is above those
//!   of that kind out, and the kind's gate is shut: after each one sent
//!   the kind waits 5 s (0x82239AD0 sets 5; 0x82238020 counts it down a
//!   whole second at a time).
//! - Cars come in only while she is in a tank, by a road or driving one
//!   (0x822387C0, her states 1, 4 and 5): from behind her (60 m back by
//!   road) with a chance of 0.75 in a tank, 0.5 by a road and 0.25
//!   driving, else from ahead (130 m on); none where her road reaches no
//!   such point.
//! - A car's driver with no gun of his own pursues her (`CarPursue`,
//!   0x822ADE08); its gunners fire their seats' guns at her by the AI's
//!   fire control (`TurretAttack` 0x822B7C10 -> 0x822C95D8 ->
//!   0x822F0098, as the gunships' [`crate::combat::Combat::pilots_fire`]).
//!
//! - A driver with a gun of his own, or the driver of a car labelled
//!   `Apc` with a crew (its gunners here), runs `CarAttack` instead
//!   ([`m2_engine::car_attack`], 0x822ACB28), moving and turning by what
//!   each gunner's fire control reports
//!   ([`m2_engine::gunner_aim::Report`]), her line checked by a ray
//!   (0x82166958 sweeps a box). A gunner too far, too near or unable to
//!   bear does not fire (0x822C95D8 ends short of it).
//!
//! - Tanks come from ahead (0x8223A410 -> 0x8223A0A8) while she is in a
//!   tank or driving, each run by `TankAttack`
//!   ([`m2_engine::tank_attack`], 0x822B6508), firing its own gun.
//!
//! - Boats come over the water (0x82239FC8 -> 0x82238E88) while she is in
//!   a boat, off the roads or stopped, each run by `BoatAttack`
//!   ([`m2_engine::boat_attack`], 0x822A9750).
//!
//! Not yet: men are not sent (no faction's table asks for any). Inferred: how long a gunner has
//! gone without seeing her is the time since his sight lines were last
//! clear (the 360 keeps it in her record, +0x8C). Inferred: the gunners aim at her where she is (0x822C95D8's lead
//! and line-of-fire checks are not ported); the 360 thinks at its own
//! rate (not traced), the alpha every step. Invented: once the pursuit is
//! over a car stops (0x822AA568) and is taken out of play past
//! [`crate::gunships::VANISH`] from her, as the gunships are.

use std::collections::HashSet;

use m2_engine::Physics;
use m2_engine::boat_attack::{self, BoatAttack};
use m2_engine::car_attack::{self, AttackWorld, CarAttack, Rider, Sight};
use m2_engine::car_driver::{self, CarDriver, CarInputs, CarState, Quarry};
use m2_engine::gunner_aim::{self, Report};
use m2_engine::path_search::{self, PathWorld};
use m2_engine::road_ring::{self, Arrival, Flood, RoadNet};
use m2_engine::tank_attack::{Hull, TankAttack};
use m2_math::glam::{Mat4, Quat, Vec3};

use crate::gunships::Shot;
use crate::reinforce::Standing;
use crate::scene::ParkedVehicle;

/// The entities the pursuit's cars are placed as (their own range).
pub const FIRST_ENTITY: u32 = 0xfc00_0000;

/// Seconds a kind waits after one is sent (0x82239AD0's 5).
const GATE: u32 = 5;

/// What `CarAttack` asks of the world, from the alpha's physics.
struct Look<'a> {
    physics: &'a Physics,
    rng: &'a mut u32,
    car: m2_engine::physics::VehicleId,
    /// Its nose, metres ahead of its origin.
    nose: f32,
    /// Its tail, metres behind its origin.
    tail: f32,
    /// Its size for a path search, its way found (with the goal it was
    /// found for), and the lookahead it follows it by.
    size: path_search::Size,
    route: &'a mut Option<(Vec3, Option<path_search::Way>)>,
    ahead: f32,
}

/// A path search's world: the alpha's physics, passing through the car
/// searching.
struct Ground<'a> {
    physics: &'a Physics,
    car: m2_engine::physics::VehicleId,
    sweeps: usize,
    ground_hits: usize,
    body_hits: usize,
}

impl PathWorld for Ground<'_> {
    fn sweep(&mut self, from: Vec3, to: Vec3, half: Vec3) -> Option<path_search::Obstacle> {
        self.sweeps += 1;
        let o = self.physics.sweep_obstacle(from, to, half, Some(self.car));
        match o {
            Some(o) if o.ground => self.ground_hits += 1,
            Some(o) => {
                self.body_hits += 1;
                if self.body_hits <= 4 && std::env::var_os("M2_PATH_DEBUG").is_some() {
                    eprintln!(
                        "pursuers: path sweep {:.0} -> {:.0} met body {:#x} at {:.0}, box {:.1} at {:.0}",
                        from,
                        to,
                        o.id,
                        o.point,
                        o.max - o.min,
                        o.pose.0
                    );
                }
            }
            None => {}
        }
        o
    }
    /// Past the car searching (the 360's ray, 0x822E4120, would stand its
    /// first node on its own roof; inferred that it doesn't).
    fn ground(&mut self, at: Vec3) -> Option<(Vec3, Option<u64>)> {
        self.physics.path_ground(at, Some(self.car))
    }
    /// The alpha does not yet place `AiHintNode` hardpoints: none.
    fn hints(&mut self, _: u64) -> Vec<(u32, Vec3)> {
        Vec::new()
    }
    fn body(&mut self, id: u64) -> Option<path_search::Obstacle> {
        self.physics.obstacle(id)
    }
}

/// What a boat keeps clear of (0x822ED248): the boats; not buildings.
pub fn afloat(physics: &Physics, vehicles: &[ParkedVehicle]) -> Vec<boat_attack::Neighbour> {
    vehicles
        .iter()
        .filter(|v| v.spec.is_boat() && !v.wrecked)
        .filter_map(|v| {
            let id = v.id?;
            let turn = physics.vehicle_transform(id)?;
            let (lo, hi) = v.spec.bounds;
            Some(boat_attack::Neighbour {
                at: turn.transform_point3((lo + hi) * 0.5),
                radius: (hi - lo).length() * 0.5,
                velocity: physics.vehicle_velocity(id),
            })
        })
        .collect()
}

/// What the boats want to know of her.
#[derive(Clone, Copy, Debug)]
pub struct Her {
    pub forward: Vec3,
    /// Her vehicle's larger side (0x822A94E8's ring).
    pub size: f32,
    pub vehicle: Option<m2_engine::physics::VehicleId>,
    pub swimming: bool,
}

/// What a boat asks of the world ([`boat_attack::BoatWorld`]).
struct Afloat<'a> {
    physics: &'a Physics,
    id: m2_engine::physics::VehicleId,
    from: Vec3,
    face: Vec3,
    length: f32,
    her: Option<m2_engine::physics::VehicleId>,
}

impl boat_attack::BoatWorld for Afloat<'_> {
    fn sweep(&mut self, to: Vec3) -> boat_attack::Sweep {
        let to = to.with_y(self.from.y);
        match self.physics.sweep_afloat(self.from, to, self.face, self.id) {
            // Its own length short of it.
            Some(met) => boat_attack::Sweep {
                blocked: true,
                met: met - (to - self.from).normalize_or_zero() * self.length,
            },
            None => boat_attack::Sweep {
                blocked: false,
                met: to,
            },
        }
    }

    fn open_under(&mut self, her: Vec3, depth: f32) -> bool {
        self.physics
            .path_ground(her, self.her)
            .is_none_or(|(g, _)| g.y < her.y - depth)
    }
}

impl AttackWorld for Look<'_> {
    fn rand(&mut self) -> f32 {
        *self.rng = self.rng.wrapping_mul(0x0019_660d).wrapping_add(0x3c6e_f35f);
        (*self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Its way to `goal`, searched once for each goal (0x822BF5E0 keeps a
    /// way while its goal stays put), then followed a lookahead on; straight
    /// at the goal when the search found none.
    fn toward(&mut self, from: Vec3, goal: Vec3) -> Vec3 {
        if self
            .route
            .as_ref()
            .is_none_or(|(g, _)| g.distance(goal) > 0.01)
        {
            let mut world = Ground {
                physics: self.physics,
                car: self.car,
                sweeps: 0,
                ground_hits: 0,
                body_hits: 0,
            };
            let way = path_search::find(&mut world, self.size, from, goal);
            eprintln!(
                "pursuers: way to {:.0} from {:.0}: {} ({} sweeps, {} met the ground, {} a body)",
                goal,
                from,
                way.as_ref()
                    .map_or("none".to_owned(), |w| format!("{} points", w.points.len())),
                world.sweeps,
                world.ground_hits,
                world.body_hits
            );
            *self.route = Some((goal, way));
        }
        match self.route.as_ref() {
            Some((_, Some(w))) => w.ahead_of(from, self.ahead),
            _ => goal,
        }
    }

    /// A ray 1 m up from its middle to 1 m up at her, past the car's own
    /// body, stopping 4 m short of her (the 360 sweeps a box, 0x82166958,
    /// past her: a ray here).
    /// 0x822C8CB8: its box's face (its width by its height, 0.02 m thin),
    /// its middle `lift` m above the middle of its height, swept `reach` m
    /// on from its nose (or back from its tail). Here past its own bodies
    /// and meeting no terrain, as the path search's sweeps
    /// ([`Physics::sweep_obstacle`]; the 360's mask 0x6228651E is layers
    /// 1-4, 8, 10, 13, 14, 19, 21, 25, 29 and 30, not mapped to the alpha's
    /// bodies); the middle of its height taken as half its height up.
    fn room(&mut self, forward: bool, lift: f32, reach: f32) -> bool {
        let Some(m) = self.physics.vehicle_transform(self.car) else {
            return true;
        };
        let way = if forward { 1.0 } else { -1.0 };
        let ahead = m
            .transform_vector3(Vec3::Z)
            .with_y(0.0)
            .normalize_or(Vec3::Z)
            * way;
        let end = if forward { self.nose } else { -self.tail };
        let face = m.transform_point3(Vec3::Z * end) + Vec3::Y * (self.size.height * 0.5 + lift);
        let half = Vec3::new(self.size.width * 0.5, self.size.height * 0.5, 0.01);
        self.physics
            .sweep_obstacle(face, face + ahead * reach, half, Some(self.car))
            .is_none()
    }

    fn line_clear(&mut self, from: Vec3, to: Vec3) -> bool {
        let (a, b) = (from + Vec3::Y, to + Vec3::Y);
        let end = b - (b - a).normalize_or_zero() * 4.0;
        self.physics.line_clear(a, end, Some(self.car))
    }
}

/// The kinds of pursuit unit sent by road (the pursuit tick's kinds 1 and
/// 2, 0x822371F8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// 0x822387C0.
    Car,
    /// 0x8223A410: from ahead (0x8223A0A8), while she is in a tank or
    /// driving.
    Tank,
    /// 0x82239FC8: over the water near her (0x82238E88), while she is in
    /// a boat, off the roads or stopped.
    Boat,
}

impl Kind {
    fn gate(self) -> usize {
        match self {
            Kind::Car => 0,
            Kind::Tank => 1,
            Kind::Boat => 2,
        }
    }

    /// Its name in the pursuit tables' `SpawnType` column.
    pub fn table_name(self) -> &'static str {
        match self {
            Kind::Car => "Car",
            Kind::Tank => "Tank",
            Kind::Boat => "Boat",
        }
    }
}

/// One unit out.
#[derive(Debug)]
struct Pursuer {
    entity: u32,
    kind: Kind,
    faction: &'static str,
    template: String,
    arrival: Arrival,
    driver: CarDriver,
    held: CarInputs,
    /// Its driver runs `CarAttack` (0x822ADE08's choice: a gun of his
    /// own, or an `Apc` with a crew).
    attacks: bool,
    attack: CarAttack,
    /// A tank's driver: `TankAttack`.
    tank: TankAttack,
    /// A boat's driver: `BoatAttack`.
    boat: BoatAttack,
    /// Its driver's way (0x822BF7A8's path), with its goal.
    route: Option<(Vec3, Option<path_search::Way>)>,
    /// Seconds to its next look at her, whether it saw her, and seconds
    /// since it last did.
    sight_wait: f32,
    sees: bool,
    unseen: f32,
    /// What each armed seat's fire control last reported
    /// ([`m2_engine::gunner_aim::Report`]), by [`armed_seats`](crate::combat::armed_seats).
    reports: Vec<Report>,
    leaving: bool,
}

/// The pursuit's cars and tanks out, and the road network they come by.
#[derive(Debug, Default)]
pub struct Pursuers {
    net: Option<RoadNet>,
    out: Vec<Pursuer>,
    next: u32,
    /// Each kind's gate, whole seconds, and the part second counted.
    gate: [u32; 3],
    /// The boats' two placements: the one next, and how far round each
    /// has come (0x8380EC88, 0x83134160, 0x8313415C).
    boat_side: bool,
    boat_sweep: [f32; 2],
    /// Which boat has the run at her (0x822A86B0's claim).
    boat_run: Option<u32>,
    second: f32,
    /// Shots their gunners ask of their guns this frame.
    pub shots: Vec<Shot>,
    gone: HashSet<u32>,
    /// Sent so far, for the log.
    pub sent: usize,
    /// Seconds to the next log line.
    log: f32,
    rng: u32,
    /// What their gunners have seen of her motion ([`m2_engine::gunner_aim::Watch`]).
    watch: m2_engine::gunner_aim::Watch,
}

impl Pursuers {
    /// Whether the scene's vehicle placed for `entity` is one of them.
    pub fn owns(&self, entity: u32) -> bool {
        self.out.iter().any(|p| p.entity == entity)
    }

    /// How many of `kind` `faction` has out.
    pub fn out(&self, faction: &str, kind: Kind) -> usize {
        self.out
            .iter()
            .filter(|p| p.faction == faction && p.kind == kind && !p.leaving)
            .count()
    }

    /// A random number, 0..1 (the alpha's own: the 360's is 0x8215E260).
    pub fn random(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(0x0019_660d).wrapping_add(0x3c6e_f35f);
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// The road network, built from the world's roads the first time.
    pub fn net(
        &mut self,
        roads: &[m2_engine::world::Road],
        junctions: &[m2_engine::world::Junction],
    ) -> &RoadNet {
        self.net
            .get_or_insert_with(|| RoadNet::new(roads, junctions))
    }

    /// Counts the gates down by `dt` (0x82238020).
    pub fn tick(&mut self, dt: f32) {
        self.second += dt;
        while self.second >= 1.0 {
            self.second -= 1.0;
            for g in &mut self.gate {
                *g = g.saturating_sub(1);
            }
        }
    }

    /// Whether one of `kind` may be sent now: her state is one that kind
    /// comes in for (cars 1, 4, 5; tanks 1 and 5) and its gate is open.
    pub fn may_send(&self, kind: Kind, standing: Standing) -> bool {
        self.gate[kind.gate()] == 0
            && match kind {
                Kind::Car => behind_chance(standing).is_some(),
                Kind::Tank => matches!(standing, Standing::Tank | Standing::Driving),
                Kind::Boat => matches!(
                    standing,
                    Standing::Boat | Standing::OffRoad | Standing::Stopped
                ),
            }
    }

    /// Where one of `kind` comes in for her at `her`, facing `forward`, in
    /// `standing`, by the road network: a car from behind or ahead by
    /// 0x822387C0's chance (`rand`, 0..1), a tank from ahead
    /// (0x8223A0A8, `rand` its offset; it is taken as not joining the
    /// traffic).
    pub fn arrival(
        &self,
        kind: Kind,
        her: Vec3,
        forward: Vec3,
        standing: Standing,
        rand: f32,
    ) -> Option<Arrival> {
        let net = self.net.as_ref()?;
        let (road, frac, _) = net.nearest(her)?;
        let frac = frac.clamp(0.001, 0.999);
        let r = &net.roads[road];
        let dir = road_ring::facing_along(r, frac, her, forward, road_ring::NEAR);
        let flood = Flood::new(net, road, frac, dir, road_ring::REACH)?;
        if kind == Kind::Tank {
            return road_ring::tank_ahead(net, &flood, false, rand);
        }
        let p = behind_chance(standing)?;
        if rand <= p {
            road_ring::car_behind(net, &flood)
        } else {
            road_ring::car_ahead(net, &flood)
        }
    }

    /// Where a boat comes in for her at `her`, facing `forward`, moving
    /// `velocity` (0x82238E88): by turns 200 m off at a heading that swings
    /// with each try, making for where she will be in 1.5 s at 25 m/s, and
    /// 125 m off, making for where she will be in 8 s at ten times her
    /// speed (25 to 65 m/s). It must be over water at least 5 m deep.
    /// Not checked: the box of 20 m round the point (0x82153B30).
    pub fn boat_arrival(
        &mut self,
        physics: &Physics,
        her: Vec3,
        forward: Vec3,
        velocity: Vec3,
    ) -> Option<Arrival> {
        let pi = std::f32::consts::PI;
        let second = self.boat_side;
        let t = &mut self.boat_sweep[usize::from(second)];
        *t = if *t < pi { *t + 0.1 } else { 0.0 };
        let (reach, angle) = if second {
            (125.0, *t * 1.4 + 0.942_477_9)
        } else {
            (200.0, 0.628_318_55 - *t * 0.4)
        };
        let (s, c) = angle.sin_cos();
        let f = forward.with_y(0.0).normalize_or(Vec3::Z) * reach;
        let at = her + Vec3::new(f.x * c - f.z * s, 0.0, f.z * c + f.x * s);
        let surface = physics.water_at(at.x, at.z)?;
        let top = Vec3::new(at.x, surface, at.z);
        if physics
            .cast_ray(top + Vec3::Y * 0.1, Vec3::NEG_Y, 5.0)
            .is_some()
        {
            return None;
        }
        let (goal, speed) = if second {
            (
                her + velocity * 8.0,
                (velocity.length() * 10.0).clamp(25.0, 65.0),
            )
        } else {
            (her + velocity * 1.5, 25.0)
        };
        let way = (goal - top).with_y(0.0).normalize_or(Vec3::Z);
        Some(Arrival {
            at: top + Vec3::Y,
            facing: way,
            velocity: way * speed + Vec3::Y,
        })
    }

    /// Sends one: `template` of `faction`, a `kind`, coming in by
    /// `arrival`; returns the entity it is to be placed as. Shuts its gate.
    pub fn send(
        &mut self,
        kind: Kind,
        faction: &'static str,
        template: String,
        arrival: Arrival,
    ) -> u32 {
        let entity = FIRST_ENTITY + self.next;
        self.next += 1;
        self.out.push(Pursuer {
            entity,
            kind,
            faction,
            template,
            arrival,
            driver: CarDriver::default(),
            held: CarInputs::default(),
            attacks: false,
            attack: CarAttack::default(),
            tank: TankAttack::default(),
            boat: BoatAttack::default(),
            route: None,
            sight_wait: 0.0,
            sees: false,
            unseen: 0.0,
            reports: Vec::new(),
            leaving: false,
        });
        // 0x82239AD0: 5 s, a boat's 2.
        self.gate[kind.gate()] = if kind == Kind::Boat { 2 } else { GATE };
        if kind == Kind::Boat {
            self.boat_side = !self.boat_side;
        }
        self.sent += 1;
        entity
    }

    /// The pursuit of `faction` is over (or there is none): its cars stop.
    pub fn call_off(&mut self, faction: Option<&str>) {
        for p in &mut self.out {
            if faction.is_none_or(|f| f != p.faction) {
                p.leaving = true;
            }
        }
    }

    /// Drives the cars for a step of `dt`: each placed where it comes in
    /// as soon as its vehicle has loaded, moving as it comes; then its
    /// driver at her (at `her`, moving `her_velocity`, `in_vehicle` or on
    /// foot) and its gunners asking to fire ([`shots`](Self::shots)).
    /// Returns the scene vehicles to drop (wrecked, or stopped past
    /// `vanish` from her).
    #[allow(clippy::too_many_arguments)]
    pub fn drive(
        &mut self,
        physics: &mut Physics,
        vehicles: &mut [ParkedVehicle],
        her: Vec3,
        her_velocity: Vec3,
        in_vehicle: bool,
        she: Her,
        vanish: f32,
        labels: impl Fn(&ParkedVehicle) -> Vec<u32>,
        dt: f32,
    ) -> Vec<usize> {
        self.shots.clear();
        // Their gunners' watch on her motion (0x822C95D8, every 0.25 s).
        self.watch.see(her, dt);
        let mut drop = Vec::new();
        self.log -= dt;
        let log = self.log <= 0.0;
        if log {
            self.log = 5.0;
        }
        let mut boats = 0;
        let afloat = afloat(physics, vehicles);
        for p in &mut self.out {
            let Some(i) = vehicles
                .iter()
                .position(|v| v.spot.is_some_and(|(e, _, _)| e == p.entity))
            else {
                continue;
            };
            let v = &mut vehicles[i];
            if v.wrecked {
                self.gone.insert(p.entity);
                drop.push(i);
                continue;
            }
            let id = match v.id {
                Some(id) => id,
                None => {
                    let yaw = car_driver::yaw(p.arrival.facing);
                    // On the ground under the road's point (the alpha's road
                    // paths lie off its drawn surface: inferred).
                    let top = p.arrival.at + Vec3::Y * 5.0;
                    let ground = |q: Vec3| {
                        physics
                            .cast_ray(Vec3::new(q.x, top.y, q.z), Vec3::NEG_Y, 20.0)
                            .map(|(d, _)| top.y - d)
                    };
                    let surface = ground(p.arrival.at);
                    let at = if p.kind == Kind::Boat {
                        p.arrival.at
                    } else {
                        v.spec
                            .rest_on(p.arrival.at, yaw, ground)
                            .map_or(p.arrival.at, |r| r.position)
                    };
                    eprintln!(
                        "pursuers: the road point at {:.2} m, the ground under it at {:?}",
                        p.arrival.at.y, surface
                    );
                    let id = physics.add_vehicle_turned(&v.spec, at, Quat::from_rotation_y(yaw));
                    physics.set_vehicle_velocity(id, p.arrival.velocity);
                    v.id = Some(id);
                    let driver = crate::combat::armed_seats(v)[0];
                    let armed = v.spec.armed.iter().any(|m| m.seat == driver);
                    let crew = crate::combat::armed_seats(v).len() - 1;
                    p.attacks = car_attack::runs_car_attack(armed, &labels(v), crew);
                    eprintln!(
                        "pursuers: {}'s {} in at {:.0}, {:.0} m from her, {:.1} m/s, {}",
                        p.faction,
                        p.template,
                        p.arrival.at,
                        p.arrival.at.distance(her),
                        p.arrival.velocity.length(),
                        match (p.kind, p.attacks) {
                            (Kind::Tank, _) => "TankAttack",
                            (Kind::Boat, _) => "BoatAttack",
                            (Kind::Car, true) => "CarAttack",
                            (Kind::Car, false) => "CarPursue",
                        }
                    );
                    id
                }
            };
            let Some(m) = physics.vehicle_transform(id) else {
                continue;
            };
            let at = m.transform_point3(Vec3::ZERO);
            let forward = m
                .transform_vector3(Vec3::Z)
                .with_y(0.0)
                .normalize_or(Vec3::Z);
            let car = CarState {
                at,
                forward,
                velocity: physics.vehicle_velocity(id),
            };
            if p.leaving && car.at.distance(her) > vanish {
                self.gone.insert(p.entity);
                drop.push(i);
                continue;
            }
            if p.kind == Kind::Boat && !p.leaving {
                // A boat's driver: `BoatAttack` (0x822A9750).
                let (lo, hi) = v.spec.bounds;
                let seats = v.spec.seating.as_deref();
                let mut bears = boat_attack::Bears::default();
                for &seat in crate::combat::armed_seats(v).iter().skip(1) {
                    let Some(m) = v.spec.armed.iter().find(|m| m.seat == seat) else {
                        continue;
                    };
                    let facing = m
                        .nodes
                        .iter()
                        .flatten()
                        .next()
                        .and_then(|&n| v.spec.rest.get(n))
                        .map_or(Vec3::Z, |t| t.z_axis.truncate().normalize_or(Vec3::Z));
                    let yaw = seats
                        .and_then(|s| s.turret_of(s.seat(seat)?))
                        .map_or((0.0, 0.0), |t| t.yaw);
                    bears.add(facing, yaw);
                }
                let hull = boat_attack::Hull {
                    id: p.entity,
                    car,
                    length: hi.z - lo.z,
                    width: hi.x - lo.x,
                    height: hi.y - lo.y,
                    stern: lo.z,
                    range: v
                        .spec
                        .armed
                        .iter()
                        .find_map(|m| m.weapon.hint.map(|h| h.range)),
                    rank: boats,
                    bears,
                };
                boats += 1;
                let target = boat_attack::Target {
                    sight: Sight {
                        at: her,
                        unseen: 0.0,
                        vehicle: in_vehicle,
                    },
                    velocity: her_velocity,
                    forward: she.forward,
                    size: she.size,
                    swimming: she.swimming,
                };
                let state = p.boat.state;
                let mut world = Afloat {
                    physics: &*physics,
                    id,
                    from: at + (lo + hi).y * 0.5 * Vec3::Y,
                    face: Vec3::new(hull.width, hull.height, 0.02) * 0.5,
                    length: hull.length,
                    her: she.vehicle,
                };
                self.rng = self.rng.wrapping_mul(0x0019_660d).wrapping_add(0x3c6e_f35f);
                let rand = (self.rng >> 8) as f32 / (1u32 << 24) as f32;
                let b = p.boat.think(
                    &hull,
                    &target,
                    &mut world,
                    &afloat,
                    &mut self.boat_run,
                    rand,
                    dt,
                );
                if p.boat.state != state {
                    eprintln!(
                        "pursuers: {} {:#x} BoatAttack {:?} -> {:?}, {:.0} m from her",
                        p.template,
                        p.entity,
                        state,
                        p.boat.state,
                        car.at.distance(her)
                    );
                }
                physics.drive(
                    id,
                    m2_engine::physics::VehicleControls {
                        throttle: b.throttle,
                        steer: b.steer,
                        brake: false,
                        parked: false,
                    },
                );
                if log {
                    eprintln!(
                        "pursuers: {} {:#x} {:.0} m from her at {:.1} m/s, {:?}",
                        p.template,
                        p.entity,
                        car.at.distance(her),
                        car.speed(),
                        p.boat.state
                    );
                }
            } else if p.kind == Kind::Tank && !p.leaving {
                // A tank's driver: `TankAttack` (0x822B6508), another
                // gunner aboard who can fire (0x822ECA20) taken as any
                // armed seat but his.
                let (lo, hi) = v.spec.bounds;
                let hull = Hull {
                    car,
                    length: hi.z - lo.z,
                    width: hi.x - lo.x,
                    gunner: crate::combat::armed_seats(v).len() > 1,
                };
                let sight = Sight {
                    at: her,
                    unseen: 0.0,
                    vehicle: in_vehicle,
                };
                // What she has hit it for since the last think (0x822FA988).
                p.tank.feel(std::mem::take(&mut v.hit), dt);
                let state = p.tank.state;
                let mut world = Look {
                    physics,
                    rng: &mut self.rng,
                    car: id,
                    nose: v.spec.bounds.1.z,
                    tail: -v.spec.bounds.0.z,
                    size: path_search::Size {
                        width: v.spec.bounds.1.x - v.spec.bounds.0.x,
                        height: v.spec.bounds.1.y,
                    },
                    route: &mut p.route,
                    ahead: path_search::lookahead(
                        (v.spec.bounds.1 - v.spec.bounds.0).max_element(),
                        car.speed().abs(),
                        dt,
                    ),
                };
                let t = p.tank.think(&hull, &sight, &mut world, dt);
                if p.tank.state != state {
                    eprintln!(
                        "pursuers: {} {:#x} TankAttack {:?} -> {:?}, {:.0} m from her",
                        p.template,
                        p.entity,
                        state,
                        p.tank.state,
                        car.at.distance(her)
                    );
                }
                physics.drive(
                    id,
                    m2_engine::physics::VehicleControls {
                        throttle: t.throttle,
                        steer: t.steer,
                        brake: false,
                        parked: false,
                    },
                );
                if log {
                    eprintln!(
                        "pursuers: {} {:#x} {:.0} m from her at {:.1} m/s, {:?}",
                        p.template,
                        p.entity,
                        car.at.distance(her),
                        car.speed(),
                        p.tank.state
                    );
                }
            } else {
                let input = if p.leaving {
                    car_driver::slow_to_stop(&car).unwrap_or_default()
                } else if p.attacks {
                    // The crew as 0x822ECE70 lists them: the riders of the
                    // seats with a gun, each with his fire control's last
                    // report and his weapon slot's distances.
                    let crew = rider_list(v, &p.reports, car.at, car.forward);
                    let (lo, hi) = v.spec.bounds;
                    let sight = Sight {
                        at: her,
                        unseen: 0.0,
                        vehicle: in_vehicle,
                    };
                    let mut world = Look {
                        physics,
                        rng: &mut self.rng,
                        car: id,
                        nose: v.spec.bounds.1.z,
                        tail: -v.spec.bounds.0.z,
                        size: path_search::Size {
                            width: v.spec.bounds.1.x - v.spec.bounds.0.x,
                            height: v.spec.bounds.1.y,
                        },
                        route: &mut p.route,
                        ahead: path_search::lookahead(
                            (v.spec.bounds.1 - v.spec.bounds.0).max_element(),
                            car.speed().abs(),
                            dt,
                        ),
                    };
                    p.attack.feel(std::mem::take(&mut v.hit), dt);
                    let state = p.attack.state;
                    let input = p.attack.think(
                        &mut p.driver,
                        &car,
                        hi.z - lo.z,
                        &crew,
                        &sight,
                        &mut world,
                        dt,
                    );
                    if p.attack.state != state {
                        eprintln!(
                            "pursuers: {} {:#x} CarAttack {:?} -> {:?}, {:.0} m from her",
                            p.template,
                            p.entity,
                            state,
                            p.attack.state,
                            car.at.distance(her)
                        );
                    }
                    input
                } else {
                    let right = m.transform_vector3(Vec3::X).normalize_or(Vec3::X);
                    let turning = physics.vehicle_spin(id).y;
                    let quarry = Quarry {
                        at: her,
                        velocity: her_velocity,
                        vehicle: in_vehicle,
                    };
                    p.driver.pursue(&car, right, turning, p.entity, &quarry, dt)
                };
                let forward = physics.vehicle_gear(id).is_none_or(|(g, _)| g != 0);
                p.held.take(&input, forward);
                if log {
                    eprintln!(
                        "pursuers: {} {:#x} {:.0} m from her at {:.1} m/s, touching {}, reports {:?}{}",
                        p.template,
                        p.entity,
                        car.at.distance(her),
                        car.speed(),
                        physics.vehicle_touching(id).len(),
                        p.reports,
                        if p.leaving { " (leaving)" } else { "" }
                    );
                }
                physics.drive(id, p.held.controls());
                if p.leaving {
                    continue;
                }
            }
            // Its line of sight to her, tried once a second (0x822C95D8,
            // 0x822C9250): with no line clear it does not fire.
            p.sight_wait -= dt;
            if p.sight_wait <= 0.0 {
                p.sight_wait = 1.0;
                let (lo, hi) = v.spec.bounds;
                let eye = at + Vec3::Y * ((lo.y + hi.y) * 0.5);
                let aim_at = her + Vec3::Y;
                let half = (hi.x - lo.x).min(hi.z - lo.z) * 0.5;
                p.sees = m2_engine::gunner_aim::sight_lines(eye, aim_at, half, 0.25)
                    .iter()
                    .any(|&(a, b)| {
                        // Stopping 1 m short: she or her vehicle is no block.
                        let end = b - (b - a).normalize_or_zero();
                        physics.line_clear(a, end, Some(id))
                    });
            }
            p.unseen = if p.sees { 0.0 } else { p.unseen + dt };
            // Its gunners, and its driver when he has a gun of his own
            // (`CarAttack` and `TankAttack` fire it, 0x822C95D8): each
            // think ends in a report, and short of firing unless ready.
            let seats = crate::combat::armed_seats(v);
            p.reports.resize(seats.len(), Report::default());
            for (k, &seat) in seats.iter().enumerate() {
                if let Some(mount) = v.spec.armed.iter().find(|m| m.seat == seat) {
                    let (near, far) = mount
                        .weapon
                        .hint
                        .map_or((0.0, gunner_aim::NO_HINT_RANGE), |h| (h.min_range, h.range));
                    p.reports[k] = p.reports[k].next(&gunner_aim::FireView {
                        unseen: p.unseen,
                        flat: (her - at).with_y(0.0).length(),
                        near,
                        far,
                        sees: p.sees,
                        reaches: bears(v, mount, k == 0, m, at, her),
                    });
                    if !p.sees || !p.reports[k].fires() {
                        continue;
                    }
                    self.shots.push(Shot {
                        vehicle: i,
                        at: her,
                        dist: (her - at).length(),
                        run: false,
                        seat: k,
                        priority: mount.priority,
                        lead: Some(crate::gunships::Lead {
                            velocity: her_velocity,
                            watched: self.watch.velocity,
                            speed: mount.weapon.ammo.velocity,
                            gravity: mount.weapon.ammo.gravity,
                        }),
                    });
                }
            }
        }
        let gone = &self.gone;
        self.out.retain(|p| !gone.contains(&p.entity));
        drop
    }
}

/// The riders of `v`'s seats with a gun, its driver's left out (he asks
/// 0x822ECE70), as `CarAttack` reads them: each with the report in
/// `reports`, at the vehicle's middle and heading (inferred: nothing here
/// turns a pursuit gunner's turret).
fn rider_list(v: &ParkedVehicle, reports: &[Report], at: Vec3, forward: Vec3) -> Vec<Rider> {
    crate::combat::armed_seats(v)
        .iter()
        .enumerate()
        .skip(1)
        .filter_map(|(k, &seat)| {
            let m = v.spec.armed.iter().find(|m| m.seat == seat)?;
            let (near, far) = m
                .weapon
                .hint
                .map_or((0.0, gunner_aim::NO_HINT_RANGE), |h| (h.min_range, h.range));
            Some(Rider {
                seat,
                report: reports.get(k).copied().unwrap_or_default(),
                near: gunner_aim::slot_near(near, m.weapon.ammo.blast.as_ref().map(|b| b.radius)),
                far,
                at,
                facing: car_driver::yaw(forward),
            })
        })
        .collect()
}

/// Whether the rider of a seat with gun `mount` can bring it to bear on
/// `her`, `v` at `body` with its middle `at`: from the 360 code (0x822C95D8,
/// by what his mind's +0x14 top byte says he rides, 0x822BB608): a seat
/// with a turret (7), inside its arc and tilt (0x822EF8C8), a driver of
/// a fixed gun (1..4), inside its weapon's cone round the nose; a seat
/// with neither (6) is not asked. A driver who turns the main turret
/// has it go all the way round. Inferred: out of its arc a turret is not
/// also within its weapon's cone (nothing turns a pursuit gunner's turret
/// to be).
fn bears(
    v: &ParkedVehicle,
    mount: &m2_engine::weapons::Mounted,
    driver: bool,
    body: Mat4,
    at: Vec3,
    her: Vec3,
) -> bool {
    match v.spec.aimer_of_seat(mount.seat) {
        Some(a) => {
            let want = v.spec.aim_toward(a, body, v.turret, her);
            let margin = m2_engine::weapons::ai_turret_margin(mount.weapon.aim_ai);
            v.spec.aimers[a].record.reaches(want, margin)
        }
        None if driver && !v.spec.turns_main_turret(None) => {
            let nose = body.transform_vector3(Vec3::Z).normalize_or(Vec3::Z);
            (her - at).normalize_or(nose).dot(nose)
                > m2_engine::weapons::ai_aim_cosine(mount.weapon.aim_ai)
        }
        None => true,
    }
}

/// 0x822387C0's chance of a car coming from behind her, by her state:
/// none for the states it sends no car in.
fn behind_chance(standing: Standing) -> Option<f32> {
    match standing {
        Standing::Tank => Some(0.75),
        Standing::Stopped => Some(0.5),
        Standing::Driving => Some(0.25),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Cars come in only for her in a tank, by a road or driving, tanks
    // only for her in a tank or driving; each kind's gate shuts for 5
    // whole seconds after one of it is sent.
    #[test]
    fn units_come_for_some_states_and_wait_between() {
        use Kind::{Car, Tank};
        let mut p = Pursuers::default();
        assert!(p.may_send(Car, Standing::Driving));
        assert!(p.may_send(Car, Standing::Stopped));
        assert!(!p.may_send(Car, Standing::OffRoad));
        assert!(!p.may_send(Car, Standing::Heli));
        assert!(p.may_send(Tank, Standing::Tank));
        assert!(!p.may_send(Tank, Standing::Stopped));
        let a = Arrival {
            at: Vec3::ZERO,
            facing: Vec3::Z,
            velocity: Vec3::ZERO,
        };
        p.send(Car, "Vza", "M151".into(), a);
        assert!(!p.may_send(Car, Standing::Driving));
        assert!(p.may_send(Tank, Standing::Driving));
        p.tick(4.5);
        assert!(!p.may_send(Car, Standing::Driving));
        p.tick(0.5);
        assert!(p.may_send(Car, Standing::Driving));
        assert_eq!(p.out("Vza", Car), 1);
        assert_eq!(p.out("Vza", Tank), 0);
        p.call_off(None);
        assert_eq!(p.out("Vza", Car), 0);
    }
}
