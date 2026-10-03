//! What a tick does: the player on foot (or the free camera), physics and
//! animation, and where the frame is seen from.

use m2_engine::{Physics, VehicleControls};
use m2_math::glam::{Vec3, Vec4Swizzles};
use m2_render::View;

use crate::camera::FlyCamera;
use crate::config::CameraConfig;
use crate::input::{Action, Input};
use crate::player::{Intent, Player};
use crate::scene::{ParkedVehicle, Scene};

#[derive(Debug)]
pub struct Game {
    pub scene: Scene,
    pub camera: FlyCamera,
    /// Playing as Jennifer rather than flying the camera.
    pub on_foot: bool,
    /// Seconds to the next look for parking spots.
    park_wait: f32,
}

impl Game {
    pub fn new(scene: Scene, camera: FlyCamera) -> Self {
        let on_foot = scene.player.is_some();
        Self {
            scene,
            camera,
            on_foot,
            park_wait: 0.0,
        }
    }

    /// One fixed tick from the keyboard and mouse.
    pub fn tick(&mut self, input: &mut Input, config: &CameraConfig, mouse_look: bool, dt: f32) {
        if input.take_pressed(Action::ToggleFly) {
            self.toggle_fly();
        }
        if self.on_foot {
            let intent = Intent::from_input(input, config, mouse_look);
            self.step(intent, dt);
        } else {
            self.camera.update(input, config, mouse_look, dt);
            self.step(Intent::default(), dt);
        }
    }

    /// One fixed tick with `intent` for the player (ignored while flying).
    pub fn step(&mut self, intent: Intent, dt: f32) {
        let scene = &mut self.scene;
        if let Some(world) = &mut scene.world {
            let physics = &mut world.physics;
            let player = scene.player.as_mut().filter(|_| self.on_foot);
            if let Some(player) = player {
                self.park_wait -= dt;
                if player.landed && self.park_wait <= 0.0 {
                    // Each look can cast thousands of rays: not every tick.
                    self.park_wait = 0.5;
                    park(physics, &mut scene.vehicles, player);
                }
                if intent.use_pressed {
                    match player.driving {
                        Some(i) => get_out(physics, &scene.vehicles[i], player),
                        None => player.driving = nearest(physics, &scene.vehicles, player, REACH),
                    }
                }
                let id = player
                    .driving
                    .and_then(|i| scene.vehicles.get(i))
                    .and_then(|v| v.id.map(|id| (id, v)));
                if let Some((id, _)) = id {
                    physics.drive(
                        id,
                        VehicleControls {
                            throttle: intent.movement.y,
                            steer: intent.movement.x,
                            brake: intent.hold_jump,
                        },
                    );
                    if intent.right_vehicle {
                        physics.right_vehicle(id);
                    }
                }
                physics.step(dt);
                match id.and_then(|(id, v)| Some((physics.vehicle_transform(id)?, v))) {
                    Some((at, v)) => {
                        let length = v.spec.bounds.1.z - v.spec.bounds.0.z;
                        player.chase(at, length, intent.look, dt);
                    }
                    None => {
                        player.driving = None;
                        player.chase_target = None;
                        player.update(physics, intent, dt);
                    }
                }
            } else {
                physics.step(dt);
            }
        }
        let focus = self.focus();
        if let Some(world) = &self.scene.world
            && (!self.on_foot || self.scene.player.as_ref().is_none_or(|p| p.landed))
        {
            self.scene
                .people
                .update(&world.physics, &world.index.spawns, focus, dt);
        }
        self.scene.tick(dt);
    }

    /// Puts the player in the nearest vehicle within `reach` metres (any
    /// distance for `f32::INFINITY`). False if there is none.
    pub fn enter_nearest(&mut self, reach: f32) -> bool {
        let (Some(world), Some(player)) = (&self.scene.world, &mut self.scene.player) else {
            return false;
        };
        player.driving = nearest(&world.physics, &self.scene.vehicles, player, reach);
        player.driving.is_some()
    }

    /// The vehicle being driven: its position and speed (m/s).
    pub fn driving(&self) -> Option<(Vec3, f32)> {
        let world = self.scene.world.as_ref()?;
        let i = self.scene.player.as_ref()?.driving?;
        let id = self.scene.vehicles.get(i)?.id?;
        let at = world.physics.vehicle_transform(id)?;
        Some((
            at.transform_point3(Vec3::ZERO),
            world.physics.vehicle_speed(id),
        ))
    }

    /// Switches between walking and flying. Flying starts from the player's
    /// camera; walking starts by dropping to the ground under the camera.
    pub fn toggle_fly(&mut self) {
        let Some(player) = &mut self.scene.player else {
            return;
        };
        if self.on_foot {
            self.camera.position = player.eye();
            self.camera.yaw = player.yaw;
            self.camera.pitch = player.pitch;
        } else {
            let p = self.camera.position;
            player.character.position = Vec3::new(p.x, 0.0, p.z);
            player.character.velocity = Vec3::ZERO;
            player.landed = false;
            player.yaw = self.camera.yaw;
            player.facing = self.camera.yaw;
        }
        self.on_foot = !self.on_foot;
    }

    fn walking(&self) -> Option<&crate::player::Player> {
        self.scene.player.as_ref().filter(|_| self.on_foot)
    }

    /// Where the world streams around.
    pub fn focus(&self) -> Vec3 {
        match self.walking() {
            Some(p) => p.character.position,
            None => self.camera.position,
        }
    }

    /// Where sound is heard from: the camera's position and direction.
    pub fn listener(&self) -> (Vec3, Vec3) {
        match self.walking() {
            Some(p) => (p.eye(), p.camera_forward()),
            None => (self.camera.position, self.camera.forward()),
        }
    }

    pub fn view(&self, fov: f32, aspect: f32) -> View {
        match self.walking() {
            Some(p) => p.view(fov, aspect),
            None => self.camera.view(fov, aspect),
        }
    }
}

/// How close she must be to a vehicle's door (or side) to get in, metres.
const REACH: f32 = 4.0;

/// Parking: the first spot looked at is this far ahead of her; spots are
/// tried on a grid this fine, out to this far.
const PARK_AHEAD: f32 = 14.0;
const PARK_GRID: f32 = 4.0;
const PARK_SPOTS: i32 = 12;
/// A spot is flat enough when the ground under the footprint's corners
/// varies by less than this fraction of its length (about 10°) plus a
/// little, near the height of her feet (not a roof).
const PARK_FLAT: f32 = 0.18;
const PARK_ROOF: f32 = 8.0;
/// Room left between parked vehicles, metres.
const PARK_GAP: f32 = 1.5;

/// Parks vehicles that have loaded but have no body yet on flat, clear
/// ground near the spot ahead of the player, facing the way she faces.
fn park(physics: &mut Physics, vehicles: &mut [ParkedVehicle], player: &Player) {
    if vehicles.iter().all(|v| v.id.is_some()) {
        return;
    }
    let (s, c) = player.facing.sin_cos();
    let forward = Vec3::new(s, 0.0, c);
    let right = Vec3::new(c, 0.0, -s);
    let feet = player.character.position;
    let ahead = feet + forward * PARK_AHEAD;
    // Spots nearest the one ahead first.
    let mut spots: Vec<Vec3> = (-PARK_SPOTS..=PARK_SPOTS)
        .flat_map(|i| (-PARK_SPOTS..=PARK_SPOTS).map(move |j| (i, j)))
        .map(|(i, j)| ahead + right * (i as f32 * PARK_GRID) + forward * (j as f32 * PARK_GRID))
        .filter(|p| p.distance(feet) > 6.0)
        .collect();
    spots.sort_by(|a, b| a.distance(ahead).total_cmp(&b.distance(ahead)));
    // Footprints taken: centre and radius.
    let mut taken: Vec<(Vec3, f32)> = vehicles
        .iter()
        .filter_map(|v| {
            let at = physics.vehicle_transform(v.id?)?;
            let (lo, hi) = v.spec.bounds;
            Some((
                at.transform_point3((lo + hi) * 0.5),
                footprint_radius(&v.spec),
            ))
        })
        .collect();
    for v in vehicles.iter_mut().filter(|v| v.id.is_none()) {
        let (lo, hi) = v.spec.bounds;
        let radius = footprint_radius(&v.spec);
        let middle = Vec3::new((lo.x + hi.x) * 0.5, 0.0, (lo.z + hi.z) * 0.5);
        let length = (hi.z - lo.z).max(hi.x - lo.x);
        let ground = |p: Vec3| {
            let top = Vec3::new(p.x, feet.y + PARK_ROOF, p.z);
            physics
                .cast_ray(top, -Vec3::Y, PARK_ROOF * 3.0)
                .map(|(d, _)| top.y - d)
        };
        let spot = spots.iter().find_map(|&centre| {
            if taken
                .iter()
                .any(|&(c, r)| (c - centre).with_y(0.0).length() < r + radius + PARK_GAP)
            {
                return None;
            }
            let corner = |x: f32, z: f32| centre + right * x + forward * z;
            let (hx, hz) = ((hi.x - lo.x) * 0.5, (hi.z - lo.z) * 0.5);
            let heights = [
                ground(centre)?,
                ground(corner(-hx, -hz))?,
                ground(corner(hx, -hz))?,
                ground(corner(-hx, hz))?,
                ground(corner(hx, hz))?,
            ];
            let (min, max) = heights
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), &h| (a.min(h), b.max(h)));
            (max - min <= length * PARK_FLAT + 0.2 && (max - feet.y).abs() < PARK_ROOF)
                .then_some((centre, max))
        });
        // Nowhere yet (the ground may still be loading): try next tick.
        let Some((centre, top)) = spot else { continue };
        let origin = centre - (right * middle.x + forward * middle.z);
        let at = Vec3::new(origin.x, top + v.spec.ride_height() + 0.3, origin.z);
        v.id = Some(physics.add_vehicle(&v.spec, at, player.facing));
        taken.push((centre, radius));
    }
}

/// Half the diagonal of a vehicle's footprint.
fn footprint_radius(spec: &m2_engine::VehicleSpec) -> f32 {
    let (lo, hi) = spec.bounds;
    (hi.x - lo.x).hypot(hi.z - lo.z) * 0.5
}

/// The vehicle nearest her within `reach` metres of its door marker (or its
/// hull, for those without one).
fn nearest(
    physics: &Physics,
    vehicles: &[ParkedVehicle],
    player: &Player,
    reach: f32,
) -> Option<usize> {
    let p = player.character.position;
    vehicles
        .iter()
        .enumerate()
        .filter_map(|(i, v)| {
            let at = physics.vehicle_transform(v.id?)?;
            let local = at.inverse() * p.extend(1.0);
            let (lo, hi) = v.spec.bounds;
            let d = match v.spec.door {
                Some(door) => (local.xyz() - door).length(),
                None => (local.xyz().clamp(lo, hi) - local.xyz()).length(),
            };
            (d <= reach).then_some((i, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Gets her out on the driver's side, beside the hull, to drop to the
/// ground there.
fn get_out(physics: &Physics, v: &ParkedVehicle, player: &mut Player) {
    if let Some(at) = v.id.and_then(|id| physics.vehicle_transform(id)) {
        let (lo, hi) = v.spec.bounds;
        let door = v
            .spec
            .door
            .unwrap_or(Vec3::new(lo.x, 0.0, (lo.z + hi.z) * 0.5));
        let x = if door.x < (lo.x + hi.x) * 0.5 {
            lo.x - 0.8
        } else {
            hi.x + 0.8
        };
        let out = at.transform_point3(Vec3::new(x, 0.0, door.z));
        player.character.position = Vec3::new(out.x, 0.0, out.z);
        player.character.velocity = Vec3::ZERO;
        let f = at.transform_vector3(Vec3::Z);
        player.facing = f.x.atan2(f.z);
        player.landed = false;
    }
    player.driving = None;
    player.chase_target = None;
}
