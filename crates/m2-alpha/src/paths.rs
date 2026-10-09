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
//! Inferred: the speed is `Haste` times the template's top speed (what
//! `Haste` scales isn't known, as for traffic); slower into turns.

use std::collections::HashMap;

use m2_engine::{Physics, VehicleControls};
use m2_math::glam::{Vec2, Vec3};

use crate::scene::ParkedVehicle;

/// A point is reached this near, metres (flat).
const ARRIVE: f32 = 3.0;
/// Steering: full lock at this many radians off the way (as traffic's).
const FULL_LOCK: f32 = 0.6;
/// Top speed when the template gives none, m/s (as traffic's).
const DEFAULT_TOP: f32 = 20.0;
/// Where it is is logged every this many steps.
const LOG_EVERY: u32 = 30;

/// A vehicle driving a path: its points, the one it heads for, its haste,
/// and the steps it has driven.
#[derive(Debug)]
struct PathMove {
    route: Vec<Vec3>,
    next: usize,
    haste: f32,
    steps: u32,
}

/// The vehicles driving the scripts' paths, by the scripts' guid.
#[derive(Debug, Default)]
pub struct PathMoves {
    moves: HashMap<u32, PathMove>,
}

impl PathMoves {
    /// Sends `vehicle` along `route` at `haste` (0..1) of its top speed,
    /// from the route's first point.
    pub fn start(&mut self, vehicle: u32, route: Vec<Vec3>, haste: f32) {
        if route.is_empty() {
            return;
        }
        self.moves.insert(
            vehicle,
            PathMove {
                route,
                next: 0,
                haste: haste.clamp(0.05, 1.0),
                steps: 0,
            },
        );
    }

    /// Drives each vehicle on a path one step; `driven` is the one she is
    /// in, which leaves its path.
    pub fn drive(
        &mut self,
        physics: &mut Physics,
        vehicles: &[ParkedVehicle],
        driven: Option<usize>,
    ) {
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
            let (controls, want, off) = steer(
                at.transform_vector3(Vec3::Z),
                here,
                m.route[m.next],
                v.spec.tuning.max_speed.unwrap_or(DEFAULT_TOP) * m.haste,
                physics.vehicle_speed(id),
            );
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
        p.start(7, Vec::new(), 1.0);
        assert!(p.moves.is_empty());
        p.start(7, vec![Vec3::ZERO], 1.0);
        assert!(p.moves.contains_key(&7));
    }
}
