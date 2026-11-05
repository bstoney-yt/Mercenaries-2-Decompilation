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
//! Not yet: tanks (`TankAttack`), boats and men are not sent. Inferred: the gunners aim at her where she is (0x822C95D8's lead
//! and line-of-fire checks are not ported); the 360 thinks at its own
//! rate (not traced), the alpha every step. Invented: once the pursuit is
//! over a car stops (0x822AA568) and is taken out of play past
//! [`crate::gunships::VANISH`] from her, as the gunships are.

use std::collections::HashSet;

use m2_engine::Physics;
use m2_engine::car_attack::{self, AttackWorld, CarAttack, Sight};
use m2_engine::car_driver::{self, CarDriver, CarInputs, CarState, Quarry};
use m2_engine::road_ring::{self, Arrival, Flood, RoadNet};
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
}

impl AttackWorld for Look<'_> {
    fn rand(&mut self) -> f32 {
        *self.rng = self.rng.wrapping_mul(0x0019_660d).wrapping_add(0x3c6e_f35f);
        (*self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// A ray 1 m up from its middle to 1 m up at her, past the car's own
    /// body, stopping 4 m short of her (the 360 sweeps a box, 0x82166958,
    /// past her: a ray here).
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

/// One car out.
#[derive(Debug)]
struct Pursuer {
    entity: u32,
    faction: &'static str,
    template: String,
    arrival: Arrival,
    driver: CarDriver,
    held: CarInputs,
    /// Its driver runs `CarAttack` (0x822ADE08's choice: a gun of his
    /// own, or an `Apc` with a crew).
    attacks: bool,
    attack: CarAttack,
    leaving: bool,
}

/// The pursuit's cars out, and the road network they come by.
#[derive(Debug, Default)]
pub struct Pursuers {
    net: Option<RoadNet>,
    out: Vec<Pursuer>,
    next: u32,
    /// The car kind's gate, whole seconds, and the part second counted.
    gate: u32,
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

    /// How many cars `faction` has out.
    pub fn out(&self, faction: &str) -> usize {
        self.out
            .iter()
            .filter(|p| p.faction == faction && !p.leaving)
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

    /// Counts the gate down by `dt` (0x82238020).
    pub fn tick(&mut self, dt: f32) {
        self.second += dt;
        while self.second >= 1.0 {
            self.second -= 1.0;
            self.gate = self.gate.saturating_sub(1);
        }
    }

    /// Whether a car may be sent now: her state is one cars come in for
    /// and the gate is open.
    pub fn may_send(&self, standing: Standing) -> bool {
        self.gate == 0 && behind_chance(standing).is_some()
    }

    /// Where a car comes in for her at `her`, facing `forward`, in
    /// `standing`, by the road network: from behind or ahead by
    /// 0x822387C0's chance (`rand`, 0..1).
    pub fn arrival(
        &self,
        her: Vec3,
        forward: Vec3,
        standing: Standing,
        rand: f32,
    ) -> Option<Arrival> {
        let net = self.net.as_ref()?;
        let p = behind_chance(standing)?;
        let (road, frac, _) = net.nearest(her)?;
        let frac = frac.clamp(0.001, 0.999);
        let r = &net.roads[road];
        let dir = road_ring::facing_along(r, frac, her, forward, road_ring::NEAR);
        let flood = Flood::new(net, road, frac, dir, road_ring::REACH)?;
        if rand <= p {
            road_ring::car_behind(net, &flood)
        } else {
            road_ring::car_ahead(net, &flood)
        }
    }

    /// Sends one: `template` of `faction` coming in by `arrival`; returns
    /// the entity it is to be placed as. Shuts the gate.
    pub fn send(&mut self, faction: &'static str, template: String, arrival: Arrival) -> u32 {
        let entity = FIRST_ENTITY + self.next;
        self.next += 1;
        self.out.push(Pursuer {
            entity,
            faction,
            template,
            arrival,
            driver: CarDriver::default(),
            held: CarInputs::default(),
            attacks: false,
            attack: CarAttack::default(),
            leaving: false,
        });
        self.gate = GATE;
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
                        if p.attacks { "CarAttack" } else { "CarPursue" }
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
                };
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
            // Its gunners: every armed seat but the driver's.
            let seats = crate::combat::armed_seats(v);
            for (k, &seat) in seats.iter().enumerate().skip(1) {
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

    // Cars come in only for her in a tank, by a road or driving; the
    // gate shuts for 5 whole seconds after one is sent.
    #[test]
    fn cars_come_for_some_states_and_wait_between() {
        let mut p = Pursuers::default();
        assert!(p.may_send(Standing::Driving));
        assert!(!p.may_send(Standing::OffRoad));
        assert!(!p.may_send(Standing::Heli));
        let a = Arrival {
            at: Vec3::ZERO,
            facing: Vec3::Z,
            velocity: Vec3::ZERO,
        };
        p.send("Vza", "M151".into(), a);
        assert!(!p.may_send(Standing::Driving));
        p.tick(4.5);
        assert!(!p.may_send(Standing::Driving));
        p.tick(0.5);
        assert!(p.may_send(Standing::Driving));
        assert_eq!(p.out("Vza"), 1);
        p.call_off(None);
        assert_eq!(p.out("Vza"), 0);
    }
}
