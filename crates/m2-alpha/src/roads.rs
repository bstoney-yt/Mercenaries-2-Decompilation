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
//! - From the 360 code ([`m2_engine::road_driver`], 0x822AAD80): each
//!   drives by 0x822AC008 toward its lane's next point at 20 m/s, 10 near
//!   its road's end, stopping short of an end that leads nowhere; it eases off and
//!   stops behind a car (or her) ahead in its lane, stops while a box
//!   just ahead of it meets something, and brakes and holds under
//!   0.5 m/s.
//! - From the 360 code ([`m2_engine::road_population`], 0x822639A0):
//!   how many are out (the zone at her, 2 outside every zone), where new
//!   ones come out (on the upkeep's timed rings along the road network,
//!   ahead of her or behind, each with what may be seen there, by her
//!   camera's real frustum) and when they go (beyond 200 m out of sight,
//!   beyond 250 m always). Nothing takes one away for being stuck.
//! - Each one put out takes its template afresh from the vehicle list of
//!   the zone where it comes out (0x8226C618); one taken away comes again
//!   under its entity with its new model.
//! - One comes out only where its box (0x8226BD10,
//!   [`road_population::spawn_box`]) meets no vehicle or prop, fixed or
//!   loose: 0x82153B30 lets buildings, the ground, debris, winches and AI
//!   occluders by ([`m2_engine::Physics::box_clear`]). Invented: off the roads the
//!   rings are laid from the road she was last on.
//!   Not ported: junctions' stop lines and flow control (no layer read
//!   holds an `RtFlowControl`); the 360's density grid for rings that
//!   ask it; the upkeep of states 7 and 8 (0x8226AE98).

use std::collections::{HashMap, HashSet};

use m2_engine::car_driver::{CarDriver, CarInputs, CarState};
use m2_engine::physics::StaticKey;
use m2_engine::population::{DensityZone, zone_at};
use m2_engine::road_population::{self, Mode, Spawner, Viewer};
use m2_engine::road_ring::{self, Flood, RingPoint, RoadNet};
use m2_engine::world::{Junction, Road};
use m2_engine::{Physics, VehicleControls, road_driver};
use m2_math::glam::{Quat, Vec2, Vec3};

use crate::scene::{ParkedVehicle, Scene};

/// A ring point's road must be longer than this, and the point is kept
/// this far off its ends, metres (0x8226A438).
const SHORTEST: f32 = 12.0;
const END_GAP: f32 = 6.0;
/// A point of the way is reached this near, metres.
const ARRIVE: f32 = 6.0;
/// A road's end meets a junction's port this near, metres.
const PORT: f32 = 8.0;
/// Top speed when the template gives none, m/s.
const DEFAULT_TOP: f32 = 20.0;
/// The scripts' path movers ([`crate::paths`]) brake for her or a vehicle
/// this near ahead, metres (invented; road traffic stops by the 360's
/// rules, [`road_driver`]).
const CLEAR_AHEAD: f32 = 12.0;

/// Whether any of `places` is in the way of a vehicle at `here` facing
/// `forward` (flat) going `speed`: ahead of it within [`CLEAR_AHEAD`], or
/// as far as it goes in [`STOP_SECONDS`], and within [`CLEAR_SIDE`] of its
/// line. The scripts' path movers stop for it (invented).
pub(crate) fn in_the_way(
    here: Vec3,
    forward: Vec2,
    speed: f32,
    places: impl IntoIterator<Item = Vec3>,
) -> bool {
    let reach = CLEAR_AHEAD.max(speed * STOP_SECONDS);
    places.into_iter().any(|p| {
        let d = flat(p - here);
        let ahead = d.dot(forward);
        ahead > 1.0 && ahead < reach && d.perp_dot(forward).abs() < CLEAR_SIDE
    })
}
/// And as far ahead as it goes in this long, seconds, within this of its
/// line, metres (invented: room to stop, and a truck's half width more
/// than its own).
const STOP_SECONDS: f32 = 1.8;
const CLEAR_SIDE: f32 = 3.5;
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
}

/// A road vehicle's look just ahead of it (0x822B8C68): seconds to the
/// next, whether the last met something, and for how long it has.
#[derive(Clone, Copy, Debug, Default)]
struct Look {
    next: f32,
    blocked: bool,
    blocked_for: f32,
}

/// Where a road vehicle is, by its road, way and lane, for those behind
/// it (the 360's road registry, store 0x8365739C).
#[derive(Clone, Copy, Debug)]
struct OnRoad {
    entity: u32,
    road: usize,
    forward: bool,
    lane: usize,
    at: Vec3,
    heading: Vec3,
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
    /// Each one's look ahead ([`Look`]) and its driver and what its
    /// input holds ([`m2_engine::car_driver`]).
    looks: HashMap<u32, Look>,
    drivers: HashMap<u32, (CarDriver, CarInputs)>,
    /// Ones asked for, with the seconds they have taken to load.
    loading: HashMap<u32, f32>,
    /// Ones she has driven: hers now.
    taken: HashSet<u32>,
    /// Templates that won't load, left out.
    refused: HashSet<String>,
    made: u32,
    rng: u32,
    /// The network as the 360's traffic manager floods it, and each of
    /// its roads' index among the roads.
    net: Option<RoadNet>,
    net_roads: Vec<usize>,
    /// The road she was last on or by, how far along and which way she
    /// faced along it.
    on_road: Option<(usize, f32, i32)>,
    spawner: Spawner,
    /// Where the rotating look at those out starts next.
    cursor: usize,
    /// Those taken away, to come again, and those of them coming again
    /// with where (their new model loading).
    resting: HashSet<u32>,
    reloading: HashMap<u32, Vec3>,
    /// Her place last frame, for her speed.
    last_focus: Option<Vec3>,
}

/// A place for one to come out ([`RoadTraffic::places_on`]): the road,
/// its way, metres along, the zone's vehicle list, the lane, the place
/// and the way it faces.
type Place = (usize, bool, f32, u32, usize, Vec3, Vec3);

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
        self.net = Some(RoadNet::new(roads, junctions));
        self.net_roads = roads
            .iter()
            .enumerate()
            .filter(|(_, r)| r.drivable() && r.points.len() >= 2)
            .map(|(i, _)| i)
            .collect();
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

    /// Keeps the road traffic round her as the 360's upkeep does
    /// ([`road_population`]): takes away those the band and her view say
    /// go, then puts out the rings due while fewer are out than the zone
    /// at `focus` wants (of the zones whose layer is in: `layer_in`).
    /// `viewer` is her camera this frame; without it nothing comes or
    /// goes.
    pub fn spawn(
        &mut self,
        scene: &mut Scene,
        focus: Vec3,
        viewer: Option<Viewer>,
        layer_in: impl Fn(u32) -> bool,
        dt: f32,
    ) {
        let (Some(world), Some(catalogue)) = (&mut scene.world, &scene.catalogue) else {
            return;
        };
        let index = &world.index;
        self.index(&index.roads, &index.junctions);
        for t in self.loading.values_mut() {
            *t += dt;
        }
        self.loading.retain(|_, t| *t < GIVE_UP);
        // One coming again is out once its new model stands where it was
        // put; one whose model never came stays taken away.
        let vehicles = &scene.vehicles;
        let loading = &self.loading;
        let mut back = Vec::new();
        self.reloading.retain(|&e, &mut at| {
            let here = vehicles
                .iter()
                .any(|v| v.spot.is_some_and(|(x, p, _)| x == e && p == at));
            if here || !loading.contains_key(&e) {
                back.push((e, here));
                false
            } else {
                true
            }
        });
        for (e, here) in back {
            if here {
                self.resting.remove(&e);
            } else {
                self.cars.remove(&e);
            }
        }
        let her_speed = self
            .last_focus
            .filter(|_| dt > 0.0)
            .map_or(0.0, |p| flat(focus - p).length() / dt);
        self.last_focus = Some(focus);
        let Some(viewer) = viewer else {
            return;
        };
        let her_car = scene
            .player
            .as_ref()
            .and_then(|p| p.driving)
            .and_then(|i| scene.vehicles.get(i));
        let in_vehicle = her_car.is_some();
        let afloat_or_flying = her_car.is_some_and(|v| v.spec.is_boat() || v.spec.is_helicopter());
        let height = world
            .physics
            .cast_ray(focus + Vec3::Y * 0.5, Vec3::NEG_Y, 100.0)
            .map_or(100.0, |(d, _)| (d - 0.5).max(0.0));
        let zones: Vec<&DensityZone> = index.zones.iter().filter(|z| layer_in(z.layer)).collect();
        let zone = zone_at(zones.iter().copied(), focus.x, focus.z);
        let (zv, zp) = zone.map_or(
            (
                road_population::DEFAULT_WANTED,
                road_population::DEFAULT_WANTED,
            ),
            |z| (z.vehicles, z.people),
        );
        let mode = Mode::of(zv, zp, height, afloat_or_flying);
        let wanted = road_population::wanted(zone.map(|z| z.vehicles), mode, in_vehicle);

        // Those that go (0x82263590): a rotating few looked at a frame,
        // at most two taken.
        let ours: Vec<usize> = scene
            .vehicles
            .iter()
            .enumerate()
            .filter(|(_, v)| {
                v.id.is_some()
                    && v.spot.is_some_and(|(e, _, _)| {
                        self.cars.contains_key(&e) && !self.taken.contains(&e)
                    })
            })
            .map(|(i, _)| i)
            .collect();
        let mut taken = 0;
        for k in 0..ours.len().min(road_population::LOOK_AT) {
            if taken >= road_population::MOST_TAKEN {
                break;
            }
            let i = ours[(self.cursor + k) % ours.len()];
            let v = &mut scene.vehicles[i];
            let Some((e, at, _)) = v.spot else {
                continue;
            };
            let here =
                v.id.and_then(|id| world.physics.vehicle_transform(id))
                    .map_or(at, |m| m.w_axis.truncate());
            let d = viewer.distance(here, in_vehicle);
            if road_population::despawns(d, viewer.sees(here)) {
                if let Some(id) = v.id.take() {
                    world.physics.remove_vehicle(id);
                }
                self.cars.remove(&e);
                self.looks.remove(&e);
                self.drivers.remove(&e);
                self.resting.insert(e);
                taken += 1;
            }
        }
        self.cursor = if ours.is_empty() {
            0
        } else {
            (self.cursor + road_population::LOOK_AT) % ours.len()
        };

        // Those that come (0x8226ADB0).
        let live = self.cars.keys().filter(|e| !self.taken.contains(e)).count();
        let room = road_population::MOST_A_FRAME.min(wanted.saturating_sub(live));
        let Some(flood) = self.flood(focus, viewer.look) else {
            return;
        };
        let coin = self.roll();
        // Those put out and not yet in the world (loading, or coming
        // again), by their boxes: the world's box test can't see them.
        let mut stands: Vec<(Vec3, Vec3)> = scene
            .vehicles
            .iter()
            .filter(|v| v.id.is_none())
            .filter_map(|v| v.spot)
            .filter(|(e, _, _)| self.cars.contains_key(e))
            .map(|(_, at, yaw)| {
                road_population::spawn_box(at, Vec3::new(yaw.sin(), 0.0, yaw.cos()))
            })
            .collect();
        // Which statics keep one from coming out: the props
        // (`_PropPhysics`, fixed or loose), not the buildings.
        let props = &scene.props;
        let db = &index.template_db;
        let blocks = |k: StaticKey| {
            let template = match k {
                StaticKey::Object(i) => index.objects.get(i).map(|o| o.template),
                StaticKey::Entity(e) => props.get(e).map(|(p, _)| p.template),
                StaticKey::Cell(_) | StaticKey::Member(..) => None,
            };
            template.is_some_and(|t| db.prop_physics.contains_key(&t))
        };
        let mut places = Vec::new();
        let mut spawner = self.spawner;
        spawner.step(dt, mode, !in_vehicle, her_speed, coin, room, &mut |ask| {
            let found = self.places_on(
                &index.roads,
                &zones,
                &flood,
                ask,
                mode,
                &viewer,
                in_vehicle,
                &world.physics,
                &blocks,
                &mut stands,
            );
            let n = found.len();
            places.extend(found);
            n
        });
        self.spawner = spawner;
        let mut loads = Vec::new();
        for (road, forward, s, list, pick, at, dir) in places {
            let r = &index.roads[road];
            let yaw = dir.x.atan2(dir.z);
            let car = Car {
                road,
                forward,
                next: next_point(&r.points, s, forward),
                lane: pick,
            };
            let roll = self.roll();
            let list = self.list(list, &index.template_db).to_vec();
            let weights: Vec<(u32, f32)> = list
                .iter()
                .enumerate()
                .filter(|(_, (n, _))| !self.refused.contains(n))
                .map(|(i, (_, w))| (i as u32, *w))
                .collect();
            let Some(pick) = m2_engine::population::pick(&weights, roll) else {
                continue;
            };
            let name = list[pick as usize].0.clone();
            let models =
                crate::mission::template_models(&name, &index.template_db, &catalogue.vehicles);
            if models.is_empty() {
                eprintln!("road traffic: no model for {name}; left out");
                self.refused.insert(name);
                continue;
            }
            // Each one out takes its template afresh from the zone's list
            // where it comes out (0x8226C618 through 0x82233A30 and
            // 0x82249B90), one taken away coming again under its entity.
            let rest = self
                .resting
                .iter()
                .copied()
                .find(|e| !self.reloading.contains_key(e));
            let entity = match rest {
                Some(e) => {
                    self.reloading.insert(e, at);
                    e
                }
                None => {
                    self.made += 1;
                    BASE + self.made - 1
                }
            };
            let crew = index.template_db.crew(&name);
            eprintln!(
                "road traffic {entity:#x}: {name} as {} at ({:.0}, {:.0}, {:.0}) on road {:#x} ({mode:?}, wants {wanted}), crew {crew:?}",
                models[0].label, at.x, at.y, at.z, r.entity,
            );
            loads.push((entity, at, yaw, crew, models));
            self.cars.insert(entity, car);
            self.loading.insert(entity, 0.0);
        }
        for (entity, at, yaw, crew, models) in loads {
            scene.load_world_vehicle(entity, at, yaw, crew, models);
        }
    }

    /// Whether `entity` is a road vehicle taken away, waiting to come
    /// again (not to be put back where it stood).
    pub fn resting(&self, entity: u32) -> bool {
        self.resting.contains(&entity)
    }

    /// The road network flooded out from her at `focus`, looking `look`
    /// (0x822644E0): from the road she is on or by, else the last she was
    /// (inferred: what the 360 does off the roads isn't traced).
    fn flood(&mut self, focus: Vec3, look: Vec3) -> Option<Flood> {
        let net = self.net.as_ref()?;
        if let Some((road, frac, _)) = net.nearest(focus) {
            let frac = frac.clamp(0.001, 0.999);
            let dir = road_ring::facing_along(&net.roads[road], frac, focus, look, road_ring::NEAR);
            self.on_road = Some((road, frac, dir));
        }
        let (road, frac, dir) = self.on_road?;
        Flood::new(net, road, frac, dir, road_ring::REACH)
    }

    /// Up to `ask.count` places on its ring (0x8226A438): on her side the
    /// ask says, on a road over [`SHORTEST`] long, kept [`END_GAP`] off
    /// its ends; far enough from her and no farther than
    /// [`road_population::FARTHEST`], seen or not as the ask says, passing
    /// the vehicles' keep test, clear of the others, and in a zone that
    /// sends vehicles.
    #[allow(clippy::too_many_arguments)]
    fn places_on(
        &mut self,
        roads: &[Road],
        zones: &[&DensityZone],
        flood: &Flood,
        ask: road_population::Ask,
        mode: Mode,
        viewer: &Viewer,
        in_vehicle: bool,
        physics: &Physics,
        blocks: &dyn Fn(StaticKey) -> bool,
        stands: &mut Vec<(Vec3, Vec3)>,
    ) -> Vec<Place> {
        let mut out = Vec::new();
        let side = if ask.behind { -1 } else { 1 };
        let points: Vec<RingPoint> = flood
            .ring(ask.radius)
            .into_iter()
            .filter(|p| p.side == side)
            .collect();
        for p in points {
            if out.len() >= ask.count {
                break;
            }
            let Some(length) = self.net.as_ref().map(|n| n.roads[p.road].length) else {
                break;
            };
            if length <= SHORTEST {
                continue;
            }
            let gap = END_GAP / length;
            let s = p.frac.clamp(gap, 1.0 - gap) * length;
            let road = self.net_roads[p.road];
            let r = &roads[road];
            let forward = (p.dir == 1) != ask.toward;
            let lanes = r.lane_offsets().len().max(1);
            let pick = ((self.roll() * lanes as f32) as usize).min(lanes - 1);
            let (at, dir) = lane(r, s, forward, pick);
            let d = viewer.distance(at, in_vehicle);
            if d < ask.least || d > road_population::FARTHEST {
                continue;
            }
            let seen = viewer.sees(at);
            if !ask.seen.allows(seen) || !road_population::spawn_keeps(d, seen, mode) {
                continue;
            }
            // Its box clear of vehicles and props (0x8226BD10, 0x82153B30), and
            // of those put out this frame, not yet in it.
            let (lo, hi) = road_population::spawn_box(at, dir);
            let meets = |o: &(Vec3, Vec3)| lo.cmple(o.1).all() && hi.cmpge(o.0).all();
            if !physics.box_clear(lo, hi, blocks) || stands.iter().any(meets) {
                continue;
            }
            let Some(zone) = zone_at(zones.iter().copied(), at.x, at.z)
                .filter(|z| z.vehicles > 0 && z.vehicle_list != 0)
            else {
                continue;
            };
            stands.push((lo, hi));
            out.push((road, forward, s, zone.vehicle_list, pick, at, dir));
        }
        out
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
        // Each road vehicle's road, way and lane, and her vehicle (the
        // 360's road registry, 0x82265540).
        let mine: Vec<OnRoad> = vehicles
            .iter()
            .filter_map(|v| {
                let (e, _, _) = v.spot?;
                let car = self.cars.get(&e)?;
                let m = physics.vehicle_transform(v.id?)?;
                Some(OnRoad {
                    entity: e,
                    road: car.road,
                    forward: car.forward,
                    lane: car.lane,
                    at: m.w_axis.truncate(),
                    heading: m.transform_vector3(Vec3::Z).with_y(0.0),
                })
            })
            .collect();
        let her_car = driven
            .and_then(|i| vehicles.get(i)?.id)
            .and_then(|id| physics.vehicle_transform(id))
            .map(|m| {
                (
                    m.w_axis.truncate(),
                    Some(m.transform_vector3(Vec3::Z).with_y(0.0)),
                )
            })
            .or(player.map(|p| (p, None)));
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
                    // A road that leads nowhere: it stays, stopped short
                    // of its end (0x822B9440), not turned back.
                    _ if self.ways_out(car, roads).is_empty() => break,
                    _ => car = self.turn_off(car, roads),
                }
            }
            let r = &roads[car.road];
            let target = lane_point(r, car.next, car.forward, car.lane);
            let heading = at
                .transform_vector3(Vec3::Z)
                .with_y(0.0)
                .normalize_or(Vec3::Z);
            let forward = flat(heading).normalize_or_zero();
            let top = v.spec.tuning.max_speed.unwrap_or(DEFAULT_TOP);
            let speed = physics.vehicle_speed(id);
            self.cars.insert(entity, car);
            // The 360's road driver (0x822AAD80, [`road_driver`]): 20 m/s,
            // 10 near its road's end, none short of an end that leads
            // nowhere, less for a car ahead in its lane, and none while a
            // box just ahead of it meets something. (No faster than its
            // template's top speed.)
            let (remaining, length) = along(r, car, here);
            let mut want =
                road_driver::base_speed(remaining, length, off_ends(&r.points, here)).min(top);
            if road_driver::near_end(remaining, length, road_driver::DEAD_END)
                && self.ways_out(car, roads).is_empty()
            {
                want = 0.0;
            }
            let ahead = mine
                .iter()
                .filter(|o| o.entity != entity && o.road == car.road && o.forward == car.forward)
                .filter(|o| o.lane == car.lane)
                .map(|o| (o.at, o.heading))
                .chain(
                    her_car
                        .map(|(p, h)| (p, h.unwrap_or(heading)))
                        // Her, on foot or in her vehicle, in its lane: the
                        // 360 puts her on a lane of the road she is on when
                        // how far she is from its line falls in that lane's
                        // strip (0x822644E0, 0x82263C98, the strips laid
                        // out by 0x8223E508).
                        .filter(|&(p, _)| in_lane(r, car, p)),
                )
                .filter(|&(p, _)| flat(p - here).dot(forward) > 0.0)
                .min_by(|a, b| {
                    here.distance_squared(a.0)
                        .total_cmp(&here.distance_squared(b.0))
                });
            if let Some((p, h)) = ahead {
                let aligned = flat(h).normalize_or_zero().dot(forward) >= road_driver::AHEAD_COS;
                if let Some(s) =
                    road_driver::follow(here.distance(p), speed.max(0.0), want, aligned)
                {
                    want = want.min(s);
                }
            }
            let look = self.looks.entry(entity).or_default();
            look.next -= dt;
            if look.next <= 0.0 {
                let last = r.points.len().saturating_sub(1).max(1);
                let t = car.next as f32 / last as f32;
                look.next = road_driver::probe_every(t, self.rng as f32 / u32::MAX as f32);
                // Ahead, or behind when its point is behind it.
                let way = if flat(target - here).dot(forward) > 0.1 {
                    heading
                } else {
                    -heading
                };
                let up = at.transform_vector3(Vec3::Y).normalize_or(Vec3::Y);
                let from = here + up * road_driver::PROBE_UP + way * road_driver::PROBE_OUT;
                let [w, h] = road_driver::PROBE_HALF;
                look.blocked = physics
                    .cast_box(
                        from + Vec3::Y * h,
                        Quat::from_rotation_y(way.x.atan2(way.z)),
                        Vec3::new(w, h, 0.05),
                        way,
                        road_driver::PROBE_LENGTH,
                        Some(id),
                    )
                    .is_some();
            }
            look.blocked_for = if look.blocked {
                look.blocked_for + dt
            } else {
                0.0
            };
            if look.blocked {
                want = 0.0;
            }
            let controls = if want < road_driver::HOLD {
                VehicleControls {
                    brake: true,
                    ..VehicleControls::default()
                }
            } else {
                let state = CarState {
                    at: here,
                    forward: heading,
                    velocity: physics.vehicle_velocity(id),
                };
                let (driver, held) = self.drivers.entry(entity).or_default();
                let input = driver.drive_to(&state, target, want, false, dt);
                let ahead_gear = physics.vehicle_gear(id).is_none_or(|(g, _)| g != 0);
                held.take(&input, ahead_gear);
                held.controls()
            };
            physics.drive(id, controls);
        }
    }
    /// At the end of its road: onto another road that meets the junction
    /// there (at random), else back the way it came.
    /// The roads out of the junction at the end of its road ahead, and
    /// which way each goes from there (0x8223D270 finds none at a road
    /// that leads nowhere).
    fn ways_out(&self, car: Car, roads: &[Road]) -> Vec<(usize, bool)> {
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
        ways
    }

    fn turn_off(&mut self, car: Car, roads: &[Road]) -> Car {
        let r = &roads[car.road];
        let ways = self.ways_out(car, roads);
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
/// Whether `p` is in lane `car.lane` of `road` the way `car` goes: how
/// far right of the road's line it is (on the stretch nearest it) falls in
/// that lane's strip ([`Road::lane_strips`]; the outermost where it has
/// fewer).
fn in_lane(road: &Road, car: Car, p: Vec3) -> bool {
    let strips = road.lane_strips();
    let Some(&(from, to)) = strips.get(car.lane).or(strips.last()) else {
        return false;
    };
    let at = flat(p);
    let best = road
        .points
        .windows(2)
        .map(|w| {
            let (a, b) = (flat(w[0]), flat(w[1]));
            let ab = b - a;
            let t = ((at - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            let near = a + ab * t;
            (at.distance_squared(near), near, ab.normalize_or_zero())
        })
        .min_by(|x, y| x.0.total_cmp(&y.0));
    let Some((_, near, dir)) = best else {
        return false;
    };
    let dir = if car.forward { dir } else { -dir };
    // Right of the way it goes (+x of a vehicle facing +z is its left).
    let right = Vec2::new(-dir.y, dir.x);
    let off = (at - near).dot(right);
    (from..=to).contains(&off)
}

/// Whether `at` lies off a road's ends: its nearest place on the road's
/// line, as a fraction along it, would be below 0 or above 1 were it not
/// clamped (the 360's follower fraction at exactly 0 or 1).
fn off_ends(points: &[Vec3], at: Vec3) -> bool {
    let n = points.len();
    if n < 2 {
        return false;
    }
    let p = flat(at);
    let (mut best, mut off) = (f32::MAX, false);
    for (k, w) in points.windows(2).enumerate() {
        let (a, b) = (flat(w[0]), flat(w[1]));
        let ab = b - a;
        let raw = (p - a).dot(ab) / ab.length_squared().max(1e-6);
        let t = raw.clamp(0.0, 1.0);
        let d = (p - (a + ab * t)).length_squared();
        if d < best {
            best = d;
            off = (k == 0 && raw < 0.0) || (k == n - 2 && raw > 1.0);
        }
    }
    off
}

/// How much of its road is left ahead of a vehicle at `here` heading for
/// point `car.next` the way it goes, and the road's length, metres.
fn along(road: &Road, car: Car, here: Vec3) -> (f32, f32) {
    let p = &road.points;
    let length: f32 = p.windows(2).map(|w| flat(w[1] - w[0]).length()).sum();
    let next = car.next.min(p.len() - 1);
    let mut left = flat(p[next] - here).length();
    if car.forward {
        left += p[next..]
            .windows(2)
            .map(|w| flat(w[1] - w[0]).length())
            .sum::<f32>();
    } else {
        left += p[..=next]
            .windows(2)
            .map(|w| flat(w[1] - w[0]).length())
            .sum::<f32>();
    }
    (left, length)
}

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

    // A boat on its path at 17 m/s sees her ahead of it in time to stop
    // (GurCon003 round 82), not one beside its line or behind it; slow,
    // only what is near.
    #[test]
    fn what_is_in_the_way_reaches_further_the_faster() {
        let f = Vec2::new(0.0, 1.0);
        let ahead = |z: f32, x: f32| [Vec3::new(x, 0.0, z)];
        assert!(in_the_way(Vec3::ZERO, f, 17.0, ahead(28.0, 1.0)));
        assert!(!in_the_way(Vec3::ZERO, f, 17.0, ahead(28.0, 5.0)));
        assert!(!in_the_way(Vec3::ZERO, f, 17.0, ahead(-5.0, 0.0)));
        assert!(!in_the_way(Vec3::ZERO, f, 2.0, ahead(28.0, 0.0)));
        assert!(in_the_way(Vec3::ZERO, f, 2.0, ahead(10.0, 0.0)));
    }

    fn road(entity: u32, points: &[(f32, f32)]) -> Road {
        Road {
            entity,
            layer: 0,
            points: points.iter().map(|&(x, z)| Vec3::new(x, 0.0, z)).collect(),
            width: 8.0,
            kind: 0,
            lanes: None,
            named_road: true,
            ends: [0, 0],
        }
    }

    // What is left of its road ahead, the way it goes (for 0x822B8138's
    // "near the end").
    #[test]
    fn it_knows_how_much_road_is_left() {
        let r = road(1, &[(0.0, 0.0), (0.0, 10.0), (0.0, 30.0)]);
        let car = |forward, next| Car {
            road: 0,
            forward,
            next,
            lane: 0,
        };
        assert_eq!(
            along(&r, car(true, 1), Vec3::new(0.0, 0.0, 5.0)),
            (25.0, 30.0)
        );
        assert_eq!(
            along(&r, car(false, 1), Vec3::new(0.0, 0.0, 20.0)),
            (20.0, 30.0)
        );
    }

    // She is in a road vehicle's lane when how far right of the line she
    // is falls in its strip (0x82263C98): on the near half going +z, the
    // far half going back; not off the road.
    #[test]
    fn she_is_in_its_lane_by_the_lanes_strip() {
        let r = road(1, &[(0.0, 0.0), (0.0, 100.0)]);
        let car = |forward| Car {
            road: 0,
            forward,
            next: 1,
            lane: 0,
        };
        assert!(in_lane(&r, car(true), Vec3::new(-2.0, 0.0, 50.0)));
        assert!(!in_lane(&r, car(true), Vec3::new(2.0, 0.0, 50.0)));
        assert!(in_lane(&r, car(false), Vec3::new(2.0, 0.0, 50.0)));
        assert!(!in_lane(&r, car(true), Vec3::new(-6.0, 0.0, 50.0)));
    }

    // 0x8226A438 on her road: the 110 m ring's point ahead of her, in
    // view (flag 8), heading away (no flag 2); none behind her for it; one
    // nearer her than the ring's least distance is refused.
    #[test]
    fn they_come_out_on_the_rings_ahead_in_view() {
        use m2_math::glam::Mat4;
        let mut r0 = road(10, &[(0.0, 0.0), (0.0, 400.0)]);
        r0.ends = [0, 0];
        let roads = [r0];
        let mut t = RoadTraffic::default();
        t.index(&roads, &[]);
        let her = Vec3::new(0.0, 0.0, 20.0);
        let viewer = Viewer {
            at: her,
            look: Vec3::Z,
            view_proj: Mat4::perspective_rh(1.0, 1.5, 0.1, 1000.0)
                * Mat4::look_at_rh(her, her + Vec3::Z, Vec3::Y),
        };
        let zone = DensityZone {
            entity: 1,
            name: String::new(),
            layer: 0,
            static_layer: true,
            outline: vec![
                Vec2::new(-50.0, -50.0),
                Vec2::new(50.0, -50.0),
                Vec2::new(50.0, 450.0),
                Vec2::new(-50.0, 450.0),
            ],
            vehicles: 12,
            people: 8,
            vehicle_list: 7,
            people_list: 0,
            priority: 0,
            faction: 0,
        };
        let flood = t.flood(her, Vec3::Z).unwrap();
        let ask = |radius, flags| {
            let mut got = None;
            let mut s = road_population::Spawner::default();
            // Find the ask by running the dense timers until it is due.
            s.step(3.0, Mode::Dense, true, 0.0, 0.0, 10, &mut |a| {
                if a.radius == radius
                    && got.is_none()
                    && (flags & 8 != 0) == (a.seen == road_population::Seen::InView)
                {
                    got = Some(a);
                }
                0
            });
            got.unwrap()
        };
        let in_view = ask(110.0, 0xa);
        let physics = Physics::new();
        let mut stands = Vec::new();
        let got = t.places_on(
            &roads,
            &[&zone],
            &flood,
            in_view,
            Mode::Dense,
            &viewer,
            false,
            &physics,
            &|_| false,
            &mut stands,
        );
        assert_eq!(got.len(), 1);
        let (road, forward, s, list, _, at, _) = got[0];
        assert_eq!((road, list), (0, 7));
        // Flag 2: it heads toward her, back down the road.
        assert!(!forward);
        // On her own road, her fraction plus 110 m over what is left of
        // it (0x8223F3F8, kept as the 360 works it).
        let want = 400.0 * (0.05 + 110.0 / 380.0);
        assert!((s - want).abs() < 1e-2, "{s}");
        assert!(viewer.sees(at));
        // That place is now taken by its box: none comes out on it again.
        assert_eq!(stands.len(), 1);
        let again = t.places_on(
            &roads,
            &[&zone],
            &flood,
            in_view,
            Mode::Dense,
            &viewer,
            false,
            &physics,
            &|_| false,
            &mut stands,
        );
        assert!(again.is_empty());
        // Turned round, she doesn't see it: an in-view ring puts none out.
        let away = Viewer {
            look: Vec3::NEG_Z,
            view_proj: Mat4::perspective_rh(1.0, 1.5, 0.1, 1000.0)
                * Mat4::look_at_rh(her, her - Vec3::Z, Vec3::Y),
            ..viewer
        };
        let mut stands = Vec::new();
        let got = t.places_on(
            &roads,
            &[&zone],
            &flood,
            in_view,
            Mode::Dense,
            &away,
            false,
            &physics,
            &|_| false,
            &mut stands,
        );
        assert!(got.is_empty());
    }

    // 0x822B9A30's clamp: before the first point or past the last.
    #[test]
    fn it_knows_when_it_is_off_its_roads_ends() {
        let p: Vec<Vec3> = [(0.0, 0.0), (0.0, 50.0), (10.0, 100.0)]
            .iter()
            .map(|&(x, z)| Vec3::new(x, 0.0, z))
            .collect();
        assert!(off_ends(&p, Vec3::new(0.0, 0.0, -3.0)));
        assert!(!off_ends(&p, Vec3::new(1.0, 0.0, 20.0)));
        assert!(off_ends(&p, Vec3::new(12.0, 0.0, 105.0)));
    }

    // A road with no junction at its end leads nowhere: no way out.
    #[test]
    fn a_road_without_a_junction_leads_nowhere() {
        let roads = [road(1, &[(0.0, 0.0), (0.0, 30.0)])];
        let t = RoadTraffic::default();
        let car = Car {
            road: 0,
            forward: true,
            next: 1,
            lane: 0,
        };
        assert!(t.ways_out(car, &roads).is_empty());
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
        };
        let back = t.turn_off(dead, &roads);
        assert_eq!((back.road, back.forward, back.next), (1, false, 0));
    }
}
