//! Traffic on the island's road network: as many cars, vans and bikes as
//! the area's traffic zone says ([`DensityZone`]), from its vehicle list,
//! driving the roads ([`Road`]) and turning off at the junctions onto the
//! others that meet there ([`Junction`]), the way the PC footage shows
//! them on the Maracaibo Coast Highway. The player can take any of them
//! over.
//!
//! What decides it, and how sure it is:
//! - From the data: the roads' paths and junctions, the zones' outlines,
//!   lists and priorities, the lists' weights, who rides in each vehicle
//!   (`Rider`), each road's width (its record's box) and its kind's
//!   `LaneData` strips.
//! - Inferred from samples: that a zone's first count is how many
//!   vehicles are out, and that the zone of highest priority rules; that
//!   a `LaneData` strip of kind 0 is a lane of traffic, its width a share
//!   of the road's ([`m2_engine::population::RoadLanes`]): they drive the
//!   middle of one of the lanes on the right, and only on roads with one
//!   (not the sidewalks, curbs, walls and driveways placed as roads).
//! - Invented (no field decoded says): how far off they come and go,
//!   their speed, and that they slow for what is ahead.

use std::collections::{HashMap, HashSet};

use m2_engine::population::{DensityZone, zone_at};
use m2_engine::world::{Junction, Road};
use m2_engine::{Physics, VehicleControls};
use m2_math::glam::{Vec2, Vec3};

use crate::scene::{ParkedVehicle, Scene};

/// Road vehicles made in all (those beyond are moved to new places when
/// she leaves them behind; invented: the zones ask for 12 at most).
const MOST_MADE: usize = 16;
/// They come between these distances from her, metres, and are taken
/// back past `GONE`. Invented: out of sight, inside the vehicles' reach.
const COME_NEAR: f32 = 140.0;
const COME_FAR: f32 = 320.0;
const GONE: f32 = 420.0;
/// Seconds between looks for a place to bring one.
const LOOK: f32 = 1.0;
/// None comes out nearer another vehicle than this, metres (a semi is 9
/// m long), and so many places are tried for each. Invented.
const CLEAR: f32 = 12.0;
const PLACE_TRIES: usize = 8;
/// A point of the way is reached this near, metres.
const ARRIVE: f32 = 6.0;
/// A road's end meets a junction's port this near, metres.
const PORT: f32 = 8.0;
/// The share of their template's top speed they drive at (invented: the
/// placed traffic's `Haste` runs 0.175 to 0.35; the highway's traffic in
/// the footage passes her car).
const HASTE: f32 = 0.45;
/// Top speed when the template gives none, m/s.
const DEFAULT_TOP: f32 = 20.0;
/// Full lock at this many radians off the way.
const FULL_LOCK: f32 = 0.6;
/// They brake for her or a vehicle this near ahead, metres, for at most
/// this long, seconds, then go on (round her, or into what is there).
const CLEAR_AHEAD: f32 = 12.0;
const MOST_WAIT: f32 = 3.0;
/// One stuck this long, seconds, at least this far from her, metres, is
/// taken back and comes again elsewhere (invented).
const STUCK: f32 = 20.0;
const STUCK_AWAY: f32 = 80.0;
/// A vehicle not loaded after this many seconds never will be.
const GIVE_UP: f32 = 30.0;
/// Road vehicles' entities, which no layer uses.
const BASE: u32 = 0xf200_0000;

/// One road vehicle: the road it is on, which way, and the point of the
/// road it heads for.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Car {
    road: usize,
    forward: bool,
    next: usize,
    /// Which of the road's lanes on its right it keeps to, inner first
    /// (the outermost there is, where a road has fewer).
    lane: usize,
    /// Seconds it has stood waiting for what is ahead.
    waited: f32,
    /// Seconds it has hardly moved (stuck on something).
    stuck: f32,
}

#[derive(Debug, Default)]
pub struct RoadTraffic {
    indexed: bool,
    /// The vehicle lists asked for so far, by id: the vehicles that
    /// drive on roads (template names and weights).
    lists: HashMap<u32, Vec<(String, f32)>>,
    /// Road index by entity.
    by_entity: HashMap<u32, usize>,
    /// Per road entity: the junctions it meets and the port where.
    ends: HashMap<u32, Vec<(usize, Vec3)>>,
    cars: HashMap<u32, Car>,
    /// Ones asked for, with the seconds they have taken to load.
    loading: HashMap<u32, f32>,
    /// Ones she has driven: hers now.
    taken: HashSet<u32>,
    /// Templates that won't load, left out.
    refused: HashSet<String>,
    made: u32,
    wait: f32,
    rng: u32,
}

impl RoadTraffic {
    /// Whether vehicle `v` is road traffic.
    pub fn owns(&self, v: &ParkedVehicle) -> bool {
        v.spot.is_some_and(|(e, _, _)| self.cars.contains_key(&e))
    }

    fn index(&mut self, roads: &[Road], junctions: &[Junction]) {
        if self.indexed {
            return;
        }
        self.indexed = true;
        self.rng = 0x2545_f491;
        self.by_entity = roads
            .iter()
            .enumerate()
            .map(|(i, r)| (r.entity, i))
            .collect();
        for (j, junction) in junctions.iter().enumerate() {
            for &(road, port) in &junction.ports {
                self.ends.entry(road).or_default().push((j, port));
            }
        }
        eprintln!(
            "road traffic: {} roads, {} junctions",
            roads.len(),
            junctions.len()
        );
    }

    /// What vehicle list `id` sends onto the roads: its entries that are
    /// cars, vans or bikes.
    fn list(&mut self, id: u32, templates: &m2_engine::Templates) -> &[(String, f32)] {
        self.lists.entry(id).or_insert_with(|| {
            templates
                .list_of(id)
                .into_iter()
                .filter(|(name, _)| {
                    templates
                        .get(name)
                        .is_some_and(|t| matches!(t.vehicle, Some(m2_engine::VehicleKind::Car)))
                })
                .collect()
        })
    }

    fn roll(&mut self) -> f32 {
        // xorshift32
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Brings vehicles out near `focus`, as many as the zone there asks
    /// for (of the zones whose layer is in: `layer_in`), and moves those
    /// she left far behind to new places near her.
    pub fn spawn(
        &mut self,
        scene: &mut Scene,
        focus: Vec3,
        layer_in: impl Fn(u32) -> bool,
        dt: f32,
    ) {
        let (Some(world), Some(catalogue)) = (&mut scene.world, &scene.catalogue) else {
            return;
        };
        let index = &world.index;
        self.index(&index.roads, &index.junctions);
        let zones: Vec<&DensityZone> = index.zones.iter().filter(|z| layer_in(z.layer)).collect();
        let wanted =
            zone_at(zones.iter().copied(), focus.x, focus.z).map_or(0, |z| z.vehicles as usize);
        for t in self.loading.values_mut() {
            *t += dt;
        }
        self.loading.retain(|_, t| *t < GIVE_UP);
        self.wait -= dt;
        if self.wait > 0.0 {
            return;
        }
        self.wait = LOOK;
        let mut near = 0;
        let mut far = Vec::new();
        for (i, v) in scene.vehicles.iter().enumerate() {
            let Some((e, at, _)) = v.spot.filter(|(e, _, _)| self.cars.contains_key(e)) else {
                continue;
            };
            self.loading.remove(&e);
            if self.taken.contains(&e) {
                continue;
            }
            let here =
                v.id.and_then(|id| world.physics.vehicle_transform(id))
                    .map_or(at, |m| m.w_axis.truncate());
            let away = flat(here - focus).length();
            let stuck = self.cars.get(&e).is_some_and(|c| c.stuck > STUCK);
            if away > GONE || stuck && away > STUCK_AWAY {
                far.push(i);
            } else {
                near += 1;
            }
        }
        near += self.loading.len();
        // Where vehicles stand already: none comes out on top of another
        // (playtest 5: a Sportbike and an Austin semi came out on the same
        // spot, the semi on the bike).
        let mut stands: Vec<Vec3> = scene
            .vehicles
            .iter()
            .filter_map(|v| {
                v.id.and_then(|id| world.physics.vehicle_transform(id))
                    .map(|m| m.w_axis.truncate())
                    .or(v.spot.map(|s| s.1))
            })
            .collect();
        let mut loads = Vec::new();
        'come: while near < wanted {
            let mut found = None;
            for _ in 0..PLACE_TRIES {
                let Some((road, forward, s, list)) = self.place_near(&index.roads, &zones, focus)
                else {
                    break 'come;
                };
                let r = &index.roads[road];
                let lanes = r.lane_offsets().len().max(1);
                let pick = ((self.roll() * lanes as f32) as usize).min(lanes - 1);
                let (at, dir) = lane(r, s, forward, pick);
                if stands.iter().all(|&o| flat(o - at).length() >= CLEAR) {
                    found = Some((road, forward, s, list, pick, at, dir));
                    break;
                }
            }
            let Some((road, forward, s, list, pick, at, dir)) = found else {
                break;
            };
            stands.push(at);
            let r = &index.roads[road];
            let yaw = dir.x.atan2(dir.z);
            let next = next_point(&r.points, s, forward);
            let car = Car {
                road,
                forward,
                next,
                lane: pick,
                waited: 0.0,
                stuck: 0.0,
            };
            if let Some(i) = far.pop() {
                // One left far behind comes round again here.
                let v = &mut scene.vehicles[i];
                if let Some(id) = v.id.take() {
                    world.physics.remove_vehicle(id);
                }
                let Some((e, _, _)) = v.spot else {
                    continue;
                };
                v.spot = Some((e, at, yaw));
                v.wrecked = false;
                v.damage = 0.0;
                self.cars.insert(e, car);
                near += 1;
                continue;
            }
            if self.made as usize >= MOST_MADE {
                break;
            }
            let roll = self.roll();
            let list = self.list(list, &index.template_db).to_vec();
            let weights: Vec<(u32, f32)> = list
                .iter()
                .enumerate()
                .filter(|(_, (n, _))| !self.refused.contains(n))
                .map(|(i, (_, w))| (i as u32, *w))
                .collect();
            let Some(pick) = m2_engine::population::pick(&weights, roll) else {
                break;
            };
            let name = list[pick as usize].0.clone();
            let models =
                crate::mission::template_models(&name, &index.template_db, &catalogue.vehicles);
            if models.is_empty() {
                eprintln!("road traffic: no model for {name}; left out");
                self.refused.insert(name);
                continue;
            }
            let entity = BASE + self.made;
            self.made += 1;
            let crew = index.template_db.crew(&name);
            eprintln!(
                "road traffic {entity:#x}: {name} as {} at ({:.0}, {:.0}, {:.0}) on road {:#x} (zone wants {wanted}), crew {crew:?}",
                models[0].label, at.x, at.y, at.z, r.entity,
            );
            loads.push((entity, at, yaw, crew, models));
            self.cars.insert(entity, car);
            self.loading.insert(entity, 0.0);
            near += 1;
        }
        for (entity, at, yaw, crew, models) in loads {
            scene.load_world_vehicle(entity, at, yaw, crew, models);
        }
    }

    /// A place for one to come out: a point of a road between
    /// [`COME_NEAR`] and [`COME_FAR`] from `focus` whose zone sends
    /// vehicles, and which way it goes; how far along the road that point
    /// is, and the zone's vehicle list.
    fn place_near(
        &mut self,
        roads: &[Road],
        zones: &[&DensityZone],
        focus: Vec3,
    ) -> Option<(usize, bool, f32, u32)> {
        let mut spots: Vec<(usize, f32, u32)> = Vec::new();
        for (i, r) in roads.iter().enumerate().filter(|(_, r)| r.drivable()) {
            let mut along = 0.0;
            for w in r.points.windows(2) {
                let d = flat(w[0] - focus).length();
                if (COME_NEAR..COME_FAR).contains(&d)
                    && let Some(zone) = zone_at(zones.iter().copied(), w[0].x, w[0].z)
                    && zone.vehicles > 0
                    && zone.vehicle_list != 0
                {
                    spots.push((i, along, zone.vehicle_list));
                }
                along += w[0].distance(w[1]);
            }
        }
        if spots.is_empty() {
            return None;
        }
        let k = ((self.roll() * spots.len() as f32) as usize).min(spots.len() - 1);
        let (road, s, list) = spots[k];
        Some((road, self.roll() < 0.5, s, list))
    }

    /// Drives the road traffic; `driven` is the vehicle she is in.
    pub fn drive(
        &mut self,
        physics: &mut Physics,
        vehicles: &[ParkedVehicle],
        roads: &[Road],
        driven: Option<usize>,
        player: Option<Vec3>,
        dt: f32,
    ) {
        if !self.indexed {
            return;
        }
        let places: Vec<Vec3> = vehicles
            .iter()
            .filter_map(|v| {
                physics
                    .vehicle_transform(v.id?)
                    .map(|m| m.w_axis.truncate())
            })
            .chain(player)
            .collect();
        for (i, v) in vehicles.iter().enumerate() {
            let (Some((entity, _, _)), Some(id)) = (v.spot, v.id) else {
                continue;
            };
            let Some(mut car) = self.cars.get(&entity).copied() else {
                continue;
            };
            if driven == Some(i) {
                self.taken.insert(entity);
            }
            if v.wrecked || self.taken.contains(&entity) {
                continue;
            }
            let Some(at) = physics.vehicle_transform(id) else {
                continue;
            };
            let here = at.w_axis.truncate();
            // Along the road to its end, then on at the junction there.
            for _ in 0..4 {
                let r = &roads[car.road];
                let target = lane_point(r, car.next, car.forward, car.lane);
                if flat(target - here).length() >= ARRIVE {
                    break;
                }
                let last = r.points.len() - 1;
                match (car.forward, car.next) {
                    (true, n) if n < last => car.next += 1,
                    (false, n) if n > 0 => car.next -= 1,
                    _ => car = self.turn_off(car, roads),
                }
            }
            let r = &roads[car.road];
            let mut target = lane_point(r, car.next, car.forward, car.lane);
            let forward = flat(at.transform_vector3(Vec3::Z)).normalize_or_zero();
            let top = v.spec.tuning.max_speed.unwrap_or(DEFAULT_TOP);
            let speed = physics.vehicle_speed(id);
            let mut blocked = places.iter().any(|&p| {
                let d = flat(p - here);
                let ahead = d.dot(forward);
                ahead > 1.0 && ahead < CLEAR_AHEAD && d.perp_dot(forward).abs() < 2.5
            });
            car.waited = if blocked { car.waited + dt } else { 0.0 };
            car.stuck = if speed.abs() < 0.5 {
                car.stuck + dt
            } else {
                0.0
            };
            // Waited long enough: on it goes for a while, round her or
            // whatever it is, a little way off to the right; then it
            // waits again if it must.
            if car.waited > MOST_WAIT {
                blocked = false;
                target += right_of(Vec3::new(forward.x, 0.0, forward.y)) * 2.0;
                if car.waited > 2.0 * MOST_WAIT {
                    car.waited = 0.0;
                }
            }
            self.cars.insert(entity, car);
            let to = flat(target - here).normalize_or_zero();
            // Positive: the point is to the left (+x of a vehicle facing
            // +z), and wheels turned the positive way steer left.
            let off = -forward.perp_dot(to).atan2(forward.dot(to));
            let want = if blocked {
                0.0
            } else {
                top * HASTE * (1.0 - (off.abs() / std::f32::consts::PI).min(0.6))
            };
            physics.drive(
                id,
                VehicleControls {
                    throttle: ((want - speed) / want.max(1.0)).clamp(-1.0, 1.0),
                    steer: (off / FULL_LOCK).clamp(-1.0, 1.0),
                    brake: speed > want * 1.3 + 0.5,
                    parked: false,
                },
            );
        }
    }

    /// At the end of its road: onto another road that meets the junction
    /// there (at random), else back the way it came.
    fn turn_off(&mut self, car: Car, roads: &[Road]) -> Car {
        let r = &roads[car.road];
        let end = if car.forward {
            r.points[r.points.len() - 1]
        } else {
            r.points[0]
        };
        let junction = self.ends.get(&r.entity).and_then(|ends| {
            ends.iter()
                .filter(|(_, port)| flat(*port - end).length() < PORT)
                .map(|&(j, _)| j)
                .next()
        });
        let mut ways: Vec<(usize, bool)> = Vec::new();
        if let Some(j) = junction {
            for (&road, ends) in &self.ends {
                if road == r.entity {
                    continue;
                }
                let Some(&k) = self.by_entity.get(&road) else {
                    continue;
                };
                for &(jj, port) in ends {
                    if jj != j {
                        continue;
                    }
                    let other = &roads[k];
                    if !other.drivable() {
                        continue;
                    }
                    let first = flat(other.points[0] - port).length();
                    let last = flat(other.points[other.points.len() - 1] - port).length();
                    ways.push((k, first <= last));
                }
            }
        }
        ways.sort_unstable();
        ways.dedup();
        if ways.is_empty() {
            let last = r.points.len() - 1;
            return Car {
                road: car.road,
                forward: !car.forward,
                next: if car.forward {
                    last.saturating_sub(1)
                } else {
                    1.min(last)
                },
                lane: car.lane,
                waited: 0.0,
                stuck: 0.0,
            };
        }
        let k = ((self.roll() * ways.len() as f32) as usize).min(ways.len() - 1);
        let (road, forward) = ways[k];
        let last = roads[road].points.len() - 1;
        Car {
            road,
            forward,
            next: if forward {
                1.min(last)
            } else {
                last.saturating_sub(1)
            },
            lane: car.lane,
            waited: 0.0,
            stuck: 0.0,
        }
    }

    /// A line per road vehicle out, for checking.
    pub fn report(
        &self,
        physics: &Physics,
        vehicles: &[ParkedVehicle],
        roads: &[Road],
    ) -> Vec<String> {
        vehicles
            .iter()
            .filter_map(|v| {
                let (e, _, _) = v.spot?;
                let car = self.cars.get(&e)?;
                let at =
                    v.id.and_then(|id| physics.vehicle_transform(id))
                        .map(|m| m.w_axis.truncate())?;
                let road = roads.get(car.road)?;
                Some(format!(
                    "  {} {e:#x} on road {:#x} ({:.0} m wide): at ({:.0}, {:.0}, {:.0}), {:.1} m off its line, {:.1} m/s{}{}",
                    v.spec.label,
                    road.entity,
                    road.width,
                    at.x,
                    at.y,
                    at.z,
                    off_line(&road.points, at),
                    v.id.map_or(0.0, |id| physics.vehicle_speed(id)),
                    if v.wrecked { ", wrecked" } else { "" },
                    if self.taken.contains(&e) {
                        ", hers"
                    } else {
                        ""
                    },
                ))
            })
            .collect()
    }
}

fn flat(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}

/// How far `at` is from a road's line, on the ground, metres.
fn off_line(points: &[Vec3], at: Vec3) -> f32 {
    points
        .windows(2)
        .map(|w| {
            let (a, b, p) = (flat(w[0]), flat(w[1]), flat(at));
            let ab = b - a;
            let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            (p - (a + ab * t)).length()
        })
        .fold(f32::MAX, f32::min)
}

/// The right-hand side of a road going `dir` (level, unit): for a vehicle
/// facing +z, +x is its left.
fn right_of(dir: Vec3) -> Vec3 {
    Vec3::new(-dir.z, 0.0, dir.x)
}

/// How far right of a road's middle its traffic in lane `lane` keeps
/// ([`Road::lane_offsets`]; the outermost where it has fewer).
fn keep_right(road: &Road, lane: usize) -> f32 {
    let lanes = road.lane_offsets();
    lanes
        .get(lane)
        .or(lanes.last())
        .copied()
        .unwrap_or(road.width / 4.0)
}

/// Where on road `road`, `s` metres along, a vehicle going `forward` in
/// lane `n` drives, and its way there.
fn lane(road: &Road, s: f32, forward: bool, n: usize) -> (Vec3, Vec3) {
    let (at, dir) = m2_engine::world::point_along(&road.points, s);
    let dir = if forward { dir } else { -dir };
    (at + right_of(dir) * keep_right(road, n), dir)
}

/// Point `i` of a road, moved to lane `n` of traffic going `forward`.
fn lane_point(road: &Road, i: usize, forward: bool, n: usize) -> Vec3 {
    let p = &road.points;
    let last = p.len() - 1;
    let (a, b) = if i < last {
        (p[i], p[i + 1])
    } else {
        (p[last - 1], p[last])
    };
    let mut dir = Vec3::new(b.x - a.x, 0.0, b.z - a.z).normalize_or(Vec3::Z);
    if !forward {
        dir = -dir;
    }
    p[i] + right_of(dir) * keep_right(road, n)
}

/// The first point of a road past `s` metres along it going `forward`.
fn next_point(points: &[Vec3], s: f32, forward: bool) -> usize {
    let mut along = 0.0;
    let mut passed = 0;
    for (i, w) in points.windows(2).enumerate() {
        if along <= s {
            passed = i;
        }
        along += w[0].distance(w[1]);
    }
    if forward {
        (passed + 1).min(points.len() - 1)
    } else {
        passed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn road(entity: u32, points: &[(f32, f32)]) -> Road {
        Road {
            entity,
            layer: 0,
            points: points.iter().map(|&(x, z)| Vec3::new(x, 0.0, z)).collect(),
            width: 8.0,
            kind: 0,
            lanes: None,
            named_road: true,
        }
    }

    #[test]
    fn traffic_keeps_to_the_right() {
        // Going +z, the right is -x; going back, +x.
        let r = road(1, &[(0.0, 0.0), (0.0, 100.0)]);
        let (at, dir) = lane(&r, 50.0, true, 0);
        assert_eq!((at, dir), (Vec3::new(-2.0, 0.0, 50.0), Vec3::Z));
        let (at, dir) = lane(&r, 50.0, false, 0);
        assert_eq!((at, dir), (Vec3::new(2.0, 0.0, 50.0), Vec3::NEG_Z));
        assert_eq!(lane_point(&r, 1, true, 0), Vec3::new(-2.0, 0.0, 100.0));
    }

    #[test]
    fn traffic_drives_the_lanes_the_data_gives_and_not_the_sidewalks() {
        use m2_engine::population::RoadLanes;
        // A `commercial_road20`: 22 m each side by its box; median 4,
        // two lanes of 7.5, walkway 4 (shares of the 22).
        let mut r = road(1, &[(0.0, 0.0), (0.0, 100.0)]);
        r.width = 44.0;
        r.named_road = false;
        r.lanes = Some(RoadLanes {
            strips: [(2, 4.0), (0, 7.5), (0, 7.5), (1, 4.0)],
            direction: None,
        });
        let x = |n| lane(&r, 50.0, true, n).0.x;
        assert!((x(0) + 22.0 * 7.75 / 23.0).abs() < 1e-4, "{}", x(0));
        assert!((x(1) + 22.0 * 15.25 / 23.0).abs() < 1e-4, "{}", x(1));
        // A lane it hasn't: the outermost.
        assert_eq!(x(5), x(1));
        // A sidewalk: one walkway strip, no traffic.
        r.lanes = Some(RoadLanes {
            strips: [(1, 10.0), (3, 5.0), (3, 5.0), (3, 5.0)],
            direction: None,
        });
        assert!(!r.drivable());
        assert!(r.lane_offsets().is_empty());
        assert_eq!(r.walk_offsets(), vec![11.0]);
    }

    #[test]
    fn heads_for_the_next_point_its_way() {
        let p: Vec<Vec3> = [0.0, 10.0, 20.0, 30.0]
            .iter()
            .map(|&z| Vec3::new(0.0, 0.0, z))
            .collect();
        assert_eq!(next_point(&p, 15.0, true), 2);
        assert_eq!(next_point(&p, 15.0, false), 1);
        assert_eq!(next_point(&p, 0.0, true), 1);
        assert_eq!(next_point(&p, 99.0, true), 3);
    }

    #[test]
    fn turns_off_onto_a_road_that_meets_the_junction() {
        let roads = [
            road(10, &[(0.0, 0.0), (0.0, 100.0)]),
            road(11, &[(0.0, 120.0), (0.0, 200.0)]),
            road(12, &[(80.0, 110.0), (10.0, 110.0)]),
        ];
        let junctions = [Junction {
            entity: 99,
            position: Vec3::new(0.0, 0.0, 110.0),
            ports: vec![
                (10, Vec3::new(0.0, 0.0, 100.0)),
                (11, Vec3::new(0.0, 0.0, 120.0)),
                (12, Vec3::new(10.0, 0.0, 110.0)),
            ],
        }];
        let mut t = RoadTraffic::default();
        t.index(&roads, &junctions);
        let at_end = Car {
            road: 0,
            forward: true,
            next: 1,
            lane: 0,
            waited: 0.0,
            stuck: 0.0,
        };
        let mut seen = HashSet::new();
        for _ in 0..40 {
            let c = t.turn_off(at_end, &roads);
            seen.insert((c.road, c.forward, c.next));
        }
        // Up road 11 from its start, or along road 12 from its end.
        assert_eq!(seen, HashSet::from([(1, true, 1), (2, false, 0)]));
        // A dead end turns it round.
        let dead = Car {
            road: 1,
            forward: true,
            next: 1,
            lane: 0,
            waited: 0.0,
            stuck: 0.0,
        };
        let back = t.turn_off(dead, &roads);
        assert_eq!((back.road, back.forward, back.next), (1, false, 0));
    }
}
