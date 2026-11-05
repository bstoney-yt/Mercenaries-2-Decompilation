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
//!   ([`m2_engine::car_attack`], 0x822ACB28); the crew's range is the
//!   middle of their guns' `MinFireRange` and `FireRange` (inferred, see
//!   there), her line checked by a ray (0x82166958 sweeps a box).
//!
//! - Tanks come from ahead (0x8223A410 -> 0x8223A0A8) while she is in a
//!   tank or driving, each run by `TankAttack`
//!   ([`m2_engine::tank_attack`], 0x822B6508), firing its own gun.
//!
//! Not yet: boats and men are not sent. Inferred: the gunners aim at her where she is (0x822C95D8's lead
//! and line-of-fire checks are not ported); the 360 thinks at its own
//! rate (not traced), the alpha every step. Invented: once the pursuit is
//! over a car stops (0x822AA568) and is taken out of play past
//! [`crate::gunships::VANISH`] from her, as the gunships are.

use std::collections::HashSet;

use m2_engine::Physics;
use m2_engine::car_attack::{self, AttackWorld, CarAttack, Sight};
use m2_engine::car_driver::{self, CarDriver, CarInputs, CarState, Quarry};
use m2_engine::path_search::{self, PathWorld};
use m2_engine::road_ring::{self, Arrival, Flood, RoadNet};
use m2_engine::tank_attack::{Hull, TankAttack};
use m2_math::glam::{Quat, Vec3};

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
        let d = b - a;
        let len = d.length();
        if len < 1e-3 {
            return true;
        }
        self.physics
            .cast_ray_past(a, d / len, (len - 4.0).max(0.0), self.car)
            .is_none()
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
}

impl Kind {
    fn gate(self) -> usize {
        match self {
            Kind::Car => 0,
            Kind::Tank => 1,
        }
    }

    /// Its name in the pursuit tables' `SpawnType` column.
    pub fn table_name(self) -> &'static str {
        match self {
            Kind::Car => "Car",
            Kind::Tank => "Tank",
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
    /// Its driver's way (0x822BF7A8's path), with its goal.
    route: Option<(Vec3, Option<path_search::Way>)>,
    leaving: bool,
}

/// The pursuit's cars and tanks out, and the road network they come by.
#[derive(Debug, Default)]
pub struct Pursuers {
    net: Option<RoadNet>,
    out: Vec<Pursuer>,
    next: u32,
    /// Each kind's gate, whole seconds, and the part second counted.
    gate: [u32; 2],
    second: f32,
    /// Shots their gunners ask of their guns this frame.
    pub shots: Vec<Shot>,
    gone: HashSet<u32>,
    /// Sent so far, for the log.
    pub sent: usize,
    /// Seconds to the next log line.
    log: f32,
    rng: u32,
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
            route: None,
            leaving: false,
        });
        self.gate[kind.gate()] = GATE;
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
        vanish: f32,
        labels: impl Fn(&ParkedVehicle) -> Vec<u32>,
        dt: f32,
    ) -> Vec<usize> {
        self.shots.clear();
        let mut drop = Vec::new();
        self.log -= dt;
        let log = self.log <= 0.0;
        if log {
            self.log = 5.0;
        }
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
                    let at = v
                        .spec
                        .rest_on(p.arrival.at, yaw, ground)
                        .map_or(p.arrival.at, |r| r.position);
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
            if p.kind == Kind::Tank && !p.leaving {
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
                    // The crew's range: the middle of their guns' two
                    // distances (0x822ACB28's mean of 0x822ECE70's +0x20 and
                    // +0x24), of those with a fire hint.
                    let crew_guns: Vec<(f32, f32)> = crate::combat::armed_seats(v)
                        .iter()
                        .skip(1)
                        .filter_map(|&s| v.spec.armed.iter().find(|m| m.seat == s))
                        .filter_map(|m| m.weapon.hint.map(|h| (h.min_range, h.range)))
                        .collect();
                    let crew = (!crew_guns.is_empty()).then(|| {
                        let n = crew_guns.len() as f32;
                        let (lo, hi) = crew_guns
                            .iter()
                            .fold((0.0, 0.0), |(a, b), &(l, h)| (a + l, b + h));
                        (lo / n + hi / n) * 0.5
                    });
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
                        crew,
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
                        "pursuers: {} {:#x} {:.0} m from her at {:.1} m/s, touching {}{}",
                        p.template,
                        p.entity,
                        car.at.distance(her),
                        car.speed(),
                        physics.vehicle_touching(id).len(),
                        if p.leaving { " (leaving)" } else { "" }
                    );
                }
                physics.drive(id, p.held.controls());
                if p.leaving {
                    continue;
                }
            }
            // Its gunners, and its driver when he has a gun of his own
            // (`CarAttack` and `TankAttack` fire it, 0x822C95D8).
            let seats = crate::combat::armed_seats(v);
            for (k, &seat) in seats.iter().enumerate() {
                if let Some(mount) = v.spec.armed.iter().find(|m| m.seat == seat) {
                    self.shots.push(Shot {
                        vehicle: i,
                        at: her,
                        dist: (her - at).length(),
                        run: false,
                        seat: k,
                        priority: mount.priority,
                    });
                }
            }
        }
        let gone = &self.gone;
        self.out.retain(|p| !gone.contains(&p.entity));
        drop
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
