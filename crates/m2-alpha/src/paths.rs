//! Vehicles the mission scripts send along a path: an `Ai.Goal` with
//! `Goal = "PathMove"` for a vehicle's driver and a path entity as its
//! `Target` (PmcCon001's `GarageSmash` sends the villa tank's driver along
//! `Path_PMC001_GarageSmash`, through the hall wall the script has just
//! killed). The vehicle drives the path's points in turn, as the island's
//! traffic drives its patrols ([`crate::traffic`]), and parks at the last.
//! One she gets into is hers: its path ends.
//!
//! Driven from [`Game::step`](crate::game::Game::step) right after the
//! traffic, so the step's "nobody at the wheel" brake doesn't hold it.
//!
//! Cars and tanks drive by the 360's own drivers: a car by 0x822AC008
//! ([`CarDriver::drive_to`], as `CarMove` 0x822AC458), a tank by 0x822B5B98
//! ([`tank_attack::drive_to`], as `TankMove` 0x822B6150), a boat by
//! 0x822A7C30 ([`boat_attack::drive_to`], as 0x822A8028), keeping clear of
//! other boats.
//!
//! From the 360 code: the speed asked is `MindPathMove`'s
//! ([`m2_engine::path_move`], 0x822B4538): 50 m/s a unit of the speed
//! factor for a car, 25 for a tank, 40 for a boat, the factor 0.4 unless
//! the request gives one; nothing in it stops for what is ahead (a boat
//! still keeps clear of other boats by its own driver).
//!
//! The goal's `Haste` is that factor: the scripts' `PathMove` reader
//! (0x8230EB48) puts it in the request's +0x24, 0 when the goal has none
//! (so the mind's 0.4). Not ported: the search for a way between the
//! points (0x822BF7A8); it goes straight at each.

use std::collections::HashMap;

use m2_engine::car_driver::{CarDriver, CarInputs, CarState};
use m2_engine::path_move::Mover;
use m2_engine::{Physics, VehicleControls, boat_attack, tank_attack};
use m2_game::path_order::PathMode;
use m2_math::glam::{Vec2, Vec3};

use crate::scene::ParkedVehicle;

/// A point is reached this near, metres (flat).
const ARRIVE: f32 = 3.0;
/// Steering: full lock at this many radians off the way (as traffic's).
const FULL_LOCK: f32 = 0.6;
/// Where it is is logged every this many steps.
const LOG_EVERY: u32 = 30;

/// A vehicle driving a path: its points, the one it heads for, its haste,
/// and the steps it has driven.
#[derive(Debug)]
struct PathMove {
    route: Vec<Vec3>,
    /// The whole path as it is travelled, for another time along it, and
    /// what it does at its end (`Mode`, [`m2_game::path_order`]).
    lap: Vec<Vec3>,
    mode: PathMode,
    next: usize,
    haste: Option<f32>,
    steps: u32,
    /// A car's driver and what its input holds.
    driver: CarDriver,
    held: CarInputs,
}

impl PathMove {
    /// At its route's end, sets out along its path again if its mode has
    /// it do so (and the path has a second point to go to): whether it
    /// did.
    fn again(&mut self) -> bool {
        if self.lap.len() < 2 {
            return false;
        }
        match self.mode {
            PathMode::Oneway => return false,
            PathMode::Loop => self.next = 0,
            // Back the way it came: it stands at what is now the first.
            PathMode::Bounce => {
                self.lap.reverse();
                self.next = 1;
            }
        }
        self.route = self.lap.clone();
        true
    }
}

/// The vehicles driving the scripts' paths, by the scripts' guid.
#[derive(Debug, Default)]
pub struct PathMoves {
    moves: HashMap<u32, PathMove>,
}

impl PathMoves {
    /// Sends `vehicle` along `route` with the goal's `Haste` as its speed
    /// factor ([`Mover::speed`]; none: the mind's own 0.4), from the
    /// route's first point.
    /// Along `route` first and then, by `mode`, `lap` (the whole path as
    /// it is travelled) again: round it from its first point (`Loop`) or
    /// back and forth (`Bounce`).
    pub fn start(
        &mut self,
        vehicle: u32,
        route: Vec<Vec3>,
        lap: Vec<Vec3>,
        mode: PathMode,
        haste: Option<f32>,
    ) {
        if route.is_empty() {
            return;
        }
        self.moves.insert(
            vehicle,
            PathMove {
                route,
                lap,
                mode,
                next: 0,
                haste,
                steps: 0,
                driver: CarDriver::default(),
                held: CarInputs::default(),
            },
        );
    }

    /// The scripts cleared the goals of `vehicle`'s driver (`Ai.RemoveGoal`
    /// with `Handle` 0, which empties the AI's goal list in the 360 code,
    /// 0x822CEE48): it drives on no further and, like a path driven to its
    /// end, parks where it is at its next step. Parking is a stand-in
    /// (invented): with no goals the 360's AI falls back on its own
    /// behaviour, which isn't traced here. Whether it was on a path.
    pub fn stop(&mut self, vehicle: u32) -> bool {
        let Some(m) = self.moves.get_mut(&vehicle) else {
            return false;
        };
        m.route.clear();
        m.lap.clear();
        m.next = 0;
        true
    }

    /// Drives each vehicle on a path one step; `driven` is the one she is
    /// in, which leaves its path.
    pub fn drive(
        &mut self,
        physics: &mut Physics,
        vehicles: &[ParkedVehicle],
        driven: Option<usize>,
        dt: f32,
    ) {
        let afloat = crate::pursuers::afloat(physics, vehicles);
        self.moves.retain(|&g, m| {
            let Some((i, v)) = vehicles.iter().enumerate().find(|(_, v)| v.guid == Some(g)) else {
                return true;
            };
            // Not placed yet: it waits.
            let Some(id) = v.id else { return true };
            if v.wrecked || driven == Some(i) {
                eprintln!(
                    "paths: {g:#x} leaves its path ({})",
                    if v.wrecked { "wrecked" } else { "she took it" }
                );
                return false;
            }
            let Some(at) = physics.vehicle_transform(id) else {
                return true;
            };
            let here = at.w_axis.truncate();
            while m.next < m.route.len() && flat(m.route[m.next] - here).length() < ARRIVE {
                m.next += 1;
                eprintln!(
                    "paths: {g:#x} at point {} of {}, {here:.1}",
                    m.next,
                    m.route.len()
                );
            }
            if m.next >= m.route.len() && m.again() {
                eprintln!(
                    "paths: {g:#x} at its path's end, {here:.1}: along it again ({:?})",
                    m.mode
                );
                return true;
            }
            if m.next >= m.route.len() {
                physics.drive(
                    id,
                    VehicleControls {
                        parked: true,
                        ..VehicleControls::default()
                    },
                );
                eprintln!("paths: {g:#x} drove its path to the end, at {here:.1}");
                return false;
            }
            let mover = if v.spec.tracked {
                Mover::Tank
            } else if v.spec.is_boat() {
                Mover::Boat
            } else {
                Mover::Car
            };
            let top = mover.speed(m.haste);
            let (_, want, off) = steer(
                at.transform_vector3(Vec3::Z),
                here,
                m.route[m.next],
                top,
                physics.vehicle_speed(id),
            );
            let car = CarState {
                at: here,
                forward: at
                    .transform_vector3(Vec3::Z)
                    .with_y(0.0)
                    .normalize_or(Vec3::Z),
                velocity: physics.vehicle_velocity(id),
            };
            let controls = if v.spec.tracked {
                let t = tank_attack::drive_to(&car, m.route[m.next], top, false);
                VehicleControls {
                    throttle: t.throttle,
                    steer: t.steer,
                    ..VehicleControls::default()
                }
            } else if v.spec.is_boat() {
                let (lo, hi) = v.spec.bounds;
                let length = hi.z - lo.z;
                let size = length.max(hi.x - lo.x);
                let near = crate::pursuers::around(physics, &afloat, &car, (lo, hi));
                let push = boat_attack::keep_clear(&car, length, size, &near);
                let b = boat_attack::drive_to(&car, m.route[m.next], top, false, push);
                VehicleControls {
                    throttle: b.throttle,
                    steer: b.steer,
                    ..VehicleControls::default()
                }
            } else {
                let input = m.driver.drive_to(&car, m.route[m.next], top, false, dt);
                let forward = physics.vehicle_gear(id).is_none_or(|(g, _)| g != 0);
                m.held.take(&input, forward);
                m.held.controls()
            };
            if m.steps % LOG_EVERY == 0 {
                eprintln!(
                    "paths: {g:#x} at {here:.1}, {:.1} m/s (wants {want:.1}), for point {} \
                     {:.1}, {:.0} degrees off",
                    physics.vehicle_speed(id),
                    m.next + 1,
                    m.route[m.next],
                    off.to_degrees()
                );
            }
            m.steps += 1;
            physics.drive(id, controls);
            true
        });
    }
}

/// The controls that take a vehicle facing `forward` at `here`, going
/// `speed`, towards `to` at up to `top` m/s (slower into a turn), with the
/// speed it wants and how far off its way it points (radians, positive:
/// the point is to its left). As traffic's.
fn steer(forward: Vec3, here: Vec3, to: Vec3, top: f32, speed: f32) -> (VehicleControls, f32, f32) {
    let forward = flat(forward).normalize_or_zero();
    let way = flat(to - here).normalize_or_zero();
    // Positive: the point is to the left (+x of a vehicle facing +z), and
    // wheels turned the positive way steer left.
    let off = -forward.perp_dot(way).atan2(forward.dot(way));
    let want = top * (1.0 - (off.abs() / std::f32::consts::PI).min(0.6));
    let controls = VehicleControls {
        throttle: ((want - speed) / want.max(1.0)).clamp(-1.0, 1.0),
        steer: (off / FULL_LOCK).clamp(-1.0, 1.0),
        brake: speed > want * 1.3,
        parked: false,
    };
    (controls, want, off)
}

fn flat(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_straight_ahead_is_driven_at_full_haste() {
        let (c, want, off) = steer(Vec3::Z, Vec3::ZERO, Vec3::new(0.0, 0.0, 30.0), 10.0, 0.0);
        assert!(off.abs() < 1e-5);
        assert!((want - 10.0).abs() < 1e-5);
        assert!((c.throttle - 1.0).abs() < 1e-5);
        assert!(c.steer.abs() < 1e-5 && !c.brake && !c.parked);
    }

    #[test]
    fn a_point_to_the_left_steers_left_and_slows() {
        // Facing +z, +x is to the left.
        let (c, want, off) = steer(Vec3::Z, Vec3::ZERO, Vec3::new(10.0, 0.0, 10.0), 10.0, 0.0);
        assert!(off > 0.0);
        assert!(c.steer > 0.0);
        assert!(want < 10.0);
    }

    #[test]
    fn far_too_fast_brakes() {
        let (c, ..) = steer(Vec3::Z, Vec3::ZERO, Vec3::new(0.0, 0.0, 30.0), 5.0, 9.0);
        assert!(c.brake);
        assert!(c.throttle < 0.0);
    }

    #[test]
    fn an_empty_route_is_not_driven() {
        let mut p = PathMoves::default();
        p.start(7, Vec::new(), Vec::new(), PathMode::Oneway, None);
        assert!(p.moves.is_empty());
        p.start(
            7,
            vec![Vec3::ZERO],
            vec![Vec3::ZERO],
            PathMode::Oneway,
            None,
        );
        assert!(p.moves.contains_key(&7));
    }

    #[test]
    fn a_cleared_driver_goes_no_further_and_does_not_go_round_again() {
        let lap = vec![Vec3::ZERO, Vec3::X * 50.0, Vec3::new(50.0, 0.0, 50.0)];
        let mut p = PathMoves::default();
        assert!(!p.stop(7), "not on a path");
        p.start(7, lap.clone(), lap, PathMode::Loop, None);
        assert!(p.stop(7));
        let m = p.moves.get_mut(&7).unwrap();
        // At its end at once (parked at the next step), and no lap to loop.
        assert!(m.next >= m.route.len());
        assert!(!m.again());
    }

    #[test]
    fn at_its_end_a_path_is_parked_on_looped_or_bounced_by_its_mode() {
        let lap = vec![Vec3::ZERO, Vec3::X * 50.0, Vec3::new(50.0, 0.0, 50.0)];
        let at_end = |mode| {
            let mut p = PathMoves::default();
            // Joined at its second point (a `Nearest` start).
            p.start(7, lap[1..].to_vec(), lap.clone(), mode, None);
            let mut m = p.moves.remove(&7).unwrap();
            m.next = m.route.len();
            let again = m.again();
            (again, m.route, m.next)
        };
        assert!(!at_end(PathMode::Oneway).0);
        // Round again: the whole path from its first point.
        assert_eq!(at_end(PathMode::Loop), (true, lap.clone(), 0));
        // Back: the path reversed, heading for the point before the end.
        let back: Vec<Vec3> = lap.iter().rev().copied().collect();
        assert_eq!(at_end(PathMode::Bounce), (true, back, 1));
        // One point is nowhere to go again.
        let mut p = PathMoves::default();
        p.start(7, vec![Vec3::ZERO], vec![Vec3::ZERO], PathMode::Loop, None);
        assert!(!p.moves.get_mut(&7).unwrap().again());
    }
}
