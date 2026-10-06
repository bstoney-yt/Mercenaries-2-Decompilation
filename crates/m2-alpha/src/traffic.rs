//! The island's traffic, as its layers place it: each vehicle spawner
//! ([`TrafficPoint`]) in a layer that is in spawns a vehicle from its list
//! while the player is within its `ActiveRadius`, again `SecondsPerCycle`
//! after the last one is wrecked. A vehicle with a patrol path drives it at
//! its `Haste`; one without stands where its spawner is (the roadblocks'
//! gun trucks). The player can take any of them over.
//!
//! What `Haste` scales is not known: it is taken as the part of the
//! template's top speed to drive at.

use std::collections::{HashMap, HashSet};

use m2_engine::world::TrafficPoint;
use m2_engine::{Catalogue, Physics, VehicleControls, WorldIndex};
use m2_math::glam::{Vec2, Vec3};

use crate::scene::{ParkedVehicle, Scene};

/// Traffic vehicles out at once, at most, of those that drive a path and
/// of those that stand (the alpha's limits, for the frame rate; the
/// game's own aren't known). Apart, so a place full of standing vehicles
/// (the guerrilla HQ) doesn't keep its driving traffic in.
const MOST_OUT: usize = 8;
const MOST_STANDING: usize = 8;
/// A waypoint is reached this near, metres.
const ARRIVE: f32 = 7.0;
/// A path whose ends are this near is driven round and round.
const CLOSED: f32 = 20.0;
/// Steering: full lock at this many radians off the way.
const FULL_LOCK: f32 = 0.6;
/// Top speed when the template gives none, m/s.
const DEFAULT_TOP: f32 = 20.0;
/// Haste when the patrol gives none (the install's spawners give 0.175 to
/// 0.35).
const DEFAULT_HASTE: f32 = 0.3;

#[derive(Debug, Default)]
pub struct Traffic {
    /// Spawners by entity, once the island is indexed.
    points: HashMap<u32, TrafficPoint>,
    indexed: bool,
    /// Per spawner: seconds before it may spawn again.
    wait: HashMap<u32, f32>,
    /// Spawners whose vehicle is loading.
    loading: HashSet<u32>,
    /// Per spawner: the waypoint its vehicle drives to, and whether it
    /// has finished an open path.
    next: HashMap<u32, (usize, bool)>,
    /// Spawners whose vehicle the player has driven: it is hers now.
    taken: HashSet<u32>,
    rng: u32,
}

impl Traffic {
    /// Whether a vehicle placed for `entity` is traffic.
    pub fn owns(&self, entity: u32) -> bool {
        self.points.contains_key(&entity)
    }

    fn index(&mut self, index: &WorldIndex) {
        if self.indexed {
            return;
        }
        self.indexed = true;
        self.points = index
            .traffic
            .iter()
            .map(|t| (t.entity, t.clone()))
            .collect();
        self.rng = 0x9e37_79b9;
        eprintln!(
            "traffic: {} spawners ({} with a path)",
            self.points.len(),
            self.points.values().filter(|t| t.route.len() >= 2).count()
        );
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

    /// Spawns what is due near `focus`; `layer_in` says whether a state
    /// layer is in.
    pub fn spawn(
        &mut self,
        scene: &Scene,
        focus: Vec3,
        layer_in: impl Fn(Option<u32>) -> bool,
        dt: f32,
    ) {
        let (Some(world), Some(catalogue)) = (&scene.world, &scene.catalogue) else {
            return;
        };
        self.index(&world.index);
        for w in self.wait.values_mut() {
            *w -= dt;
        }
        // A vehicle that loaded is out; a wreck frees its spawner.
        let mut out: HashSet<u32> = HashSet::new();
        for v in &scene.vehicles {
            if let Some((e, _, _)) = v.spot
                && self.points.contains_key(&e)
            {
                self.loading.remove(&e);
                if !v.wrecked && !self.taken.contains(&e) {
                    out.insert(e);
                } else if v.wrecked && !self.wait.contains_key(&e) {
                    let cycle = self.points[&e].spawner.seconds_per_cycle;
                    self.wait.insert(e, cycle);
                }
            }
        }
        let moves = |e: &u32| self.points[e].route.len() >= 2;
        let busy = out.iter().chain(&self.loading);
        let moving = busy.clone().filter(|e| moves(e)).count();
        let standing = busy.count() - moving;
        let mut room = [
            MOST_STANDING.saturating_sub(standing),
            MOST_OUT.saturating_sub(moving),
        ];
        if room == [0, 0] {
            return;
        }
        let mut due: Vec<(f32, u32)> = self
            .points
            .values()
            .filter(|t| {
                !out.contains(&t.entity)
                    && !self.loading.contains(&t.entity)
                    && self.wait.get(&t.entity).is_none_or(|&w| w <= 0.0)
                    && layer_in(t.layer)
            })
            .filter_map(|t| {
                let d = flat(t.position - focus).length();
                (d <= t.spawner.active_radius).then_some((d, t.entity))
            })
            .collect();
        due.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, entity) in due {
            if room == [0, 0] {
                break;
            }
            let share = usize::from(self.points[&entity].route.len() >= 2);
            if room[share] == 0 {
                continue;
            }
            let roll = self.roll();
            let t = &self.points[&entity];
            let Some(name) = pick(&t.vehicles, roll) else {
                continue;
            };
            let models = models(name, world, catalogue);
            if models.is_empty() {
                continue;
            }
            let forward = t.rotation * Vec3::Z;
            eprintln!(
                "traffic {entity:#x} ({}): {name} as {} at ({:.0}, {:.0}, {:.0})",
                t.name, models[0].label, t.position.x, t.position.y, t.position.z
            );
            scene.load_world_vehicle(entity, t.position, forward.x.atan2(forward.z), models);
            self.loading.insert(entity);
            self.wait.remove(&entity);
            self.next.remove(&entity);
            room[share] -= 1;
        }
    }

    /// Drives the traffic that has a path; `driven` is the vehicle the
    /// player is in.
    pub fn drive(
        &mut self,
        physics: &mut Physics,
        vehicles: &[ParkedVehicle],
        driven: Option<usize>,
    ) {
        for (i, v) in vehicles.iter().enumerate() {
            let (Some((entity, _, _)), Some(id)) = (v.spot, v.id) else {
                continue;
            };
            let Some(point) = self.points.get(&entity) else {
                continue;
            };
            if driven == Some(i) {
                self.taken.insert(entity);
            }
            if v.wrecked || self.taken.contains(&entity) || point.route.len() < 2 {
                continue;
            }
            let Some(at) = physics.vehicle_transform(id) else {
                continue;
            };
            let here = at.w_axis.truncate();
            let route = &point.route;
            let closed = flat(route[0] - route[route.len() - 1]).length() < CLOSED;
            let (mut next, mut done) = *self
                .next
                .entry(entity)
                .or_insert_with(|| (nearest_ahead(route, here), false));
            while !done && flat(route[next] - here).length() < ARRIVE {
                if next + 1 < route.len() {
                    next += 1;
                } else if closed {
                    next = 0;
                } else {
                    done = true;
                }
            }
            self.next.insert(entity, (next, done));
            if done {
                physics.drive(
                    id,
                    VehicleControls {
                        parked: true,
                        ..VehicleControls::default()
                    },
                );
                continue;
            }
            let forward = flat(at.transform_vector3(Vec3::Z)).normalize_or_zero();
            let to = flat(route[next] - here).normalize_or_zero();
            // Positive: the waypoint is to the left (+x of a vehicle facing
            // +z), and wheels turned the positive way steer left.
            let off = -forward.perp_dot(to).atan2(forward.dot(to));
            let top = v.spec.tuning.max_speed.unwrap_or(DEFAULT_TOP);
            let haste = point.haste.unwrap_or(DEFAULT_HASTE);
            // Slower into a turn.
            let want = top * haste * (1.0 - (off.abs() / std::f32::consts::PI).min(0.6));
            let speed = physics.vehicle_speed(id);
            physics.drive(
                id,
                VehicleControls {
                    throttle: ((want - speed) / want.max(1.0)).clamp(-1.0, 1.0),
                    steer: (off / FULL_LOCK).clamp(-1.0, 1.0),
                    brake: speed > want * 1.3,
                    parked: false,
                },
            );
        }
    }
}

impl Traffic {
    /// A line per traffic vehicle out: what, where, how fast, which
    /// waypoint; and the spawners nearest `focus`, for checking.
    pub fn report(
        &self,
        physics: &Physics,
        vehicles: &[ParkedVehicle],
        focus: Vec3,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        for v in vehicles {
            let (Some((e, _, _)), Some(id)) = (v.spot, v.id) else {
                continue;
            };
            let Some(point) = self.points.get(&e) else {
                continue;
            };
            let at = physics
                .vehicle_transform(id)
                .map_or(Vec3::NAN, |m| m.w_axis.truncate());
            lines.push(format!(
                "  {} from {e:#x} {}: at ({:.0}, {:.0}, {:.0}), {:.1} m/s, waypoint {:?} of {}{}{}",
                v.spec.label,
                point.name,
                at.x,
                at.y,
                at.z,
                physics.vehicle_speed(id),
                self.next.get(&e),
                point.route.len(),
                if v.wrecked { ", wrecked" } else { "" },
                if self.taken.contains(&e) {
                    ", hers"
                } else {
                    ""
                },
            ));
        }
        let mut near: Vec<&TrafficPoint> = self.points.values().collect();
        near.sort_by(|a, b| {
            flat(a.position - focus)
                .length()
                .total_cmp(&flat(b.position - focus).length())
        });
        for t in near.into_iter().take(5) {
            lines.push(format!(
                "  spawner {:#x} {} {:.0} m away at ({:.0}, {:.0}, {:.0}), layer {}, radius {:.0}, every {:.0} s, {} route points, haste {:?}, {:?}",
                t.entity,
                t.name,
                flat(t.position - focus).length(),
                t.position.x,
                t.position.y,
                t.position.z,
                t.layer.map_or("static".to_owned(), |l| format!("{l:08x}")),
                t.spawner.active_radius,
                t.spawner.seconds_per_cycle,
                t.route.len(),
                t.haste,
                t.vehicles,
            ));
        }
        lines
    }
}

fn flat(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}

/// The waypoint to head for first: the nearest, or the one after it when
/// that is behind on the way.
fn nearest_ahead(route: &[Vec3], here: Vec3) -> usize {
    let near = (0..route.len())
        .min_by(|&a, &b| {
            flat(route[a] - here)
                .length()
                .total_cmp(&flat(route[b] - here).length())
        })
        .unwrap_or(0);
    if near + 1 < route.len() {
        let along = flat(route[near + 1] - route[near]);
        if along.dot(flat(here - route[near])) > 0.0 {
            return near + 1;
        }
    }
    near
}

fn pick(entries: &[(String, f32)], roll: f32) -> Option<&str> {
    let weights: Vec<(u32, f32)> = entries
        .iter()
        .enumerate()
        .map(|(i, (_, w))| (i as u32, *w))
        .collect();
    m2_engine::population::pick(&weights, roll).map(|i| entries[i as usize].0.as_str())
}

fn models(
    name: &str,
    world: &m2_engine::World,
    catalogue: &Catalogue,
) -> Vec<m2_engine::CatalogueEntry> {
    crate::mission::template_models(name, &world.index.template_db, &catalogue.vehicles)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heads_for_the_waypoint_ahead() {
        let route = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 50.0),
            Vec3::new(0.0, 0.0, 100.0),
        ];
        assert_eq!(nearest_ahead(&route, Vec3::new(1.0, 0.0, 55.0)), 2);
        assert_eq!(nearest_ahead(&route, Vec3::new(1.0, 0.0, 45.0)), 1);
        assert_eq!(nearest_ahead(&route, Vec3::new(0.0, 0.0, -10.0)), 0);
    }
}
