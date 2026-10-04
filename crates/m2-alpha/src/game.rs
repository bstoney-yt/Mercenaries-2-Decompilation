//! What a tick does: the player on foot (or the free camera), physics and
//! animation, and where the frame is seen from.

use std::collections::HashSet;

use m2_engine::{Physics, TemplatePlacement, VehicleControls};
use m2_math::glam::{Vec3, Vec4Swizzles};
use m2_render::View;

use crate::camera::FlyCamera;
use crate::combat::{Combat, Field, Weapon};
use crate::config::CameraConfig;
use crate::input::{Action, Input};
use crate::mission::vehicle_models;
use crate::player::{Intent, Player};
use crate::save::{self, PlayerSave, SaveGame, VehicleSave};
use crate::scene::{ParkedVehicle, Scene};
use crate::ui::HudInfo;

#[derive(Debug)]
pub struct Game {
    pub scene: Scene,
    pub camera: FlyCamera,
    /// Playing as Jennifer rather than flying the camera.
    pub on_foot: bool,
    /// Seconds to the next look for parking spots.
    park_wait: f32,
    /// Looks for parking so far; each widens the search and allows
    /// steeper ground, until [`PARK_TRIES`] give up.
    park_tries: u32,
    /// Vehicles from a loaded save, placed as their models load.
    restore: Vec<VehicleSave>,
    pub combat: Combat,
    /// Layer entities of the island's vehicles asked for so far.
    fetched: HashSet<u32>,
    /// Seconds to the next look for the island's vehicles near the player.
    vehicle_wait: f32,
}

impl Game {
    pub fn new(scene: Scene, camera: FlyCamera) -> Self {
        let on_foot = scene.player.is_some();
        Self {
            scene,
            camera,
            on_foot,
            park_wait: 0.0,
            park_tries: 0,
            restore: Vec::new(),
            combat: Combat::default(),
            fetched: HashSet::new(),
            vehicle_wait: 0.0,
        }
    }

    /// Island vehicles asked for that have not been placed yet.
    pub fn island_vehicles_loading(&self) -> bool {
        let placed = self
            .scene
            .vehicles
            .iter()
            .filter(|v| v.spot.is_some() && v.id.is_some())
            .count();
        placed < self.fetched.len()
    }

    /// Asks for the island's own vehicles near the player, once the
    /// vehicle catalogue is known, and places the ones that have loaded
    /// where the layers park them.
    fn fetch_world_vehicles(&mut self, dt: f32) {
        let scene = &mut self.scene;
        let (Some(world), Some(player)) = (&mut scene.world, &scene.player) else {
            return;
        };
        for v in scene.vehicles.iter_mut().filter(|v| v.id.is_none()) {
            if let Some((entity, at, yaw)) = v.spot
                && place(&mut world.physics, v, at, yaw)
            {
                eprintln!(
                    "island vehicle {entity:#x}: {} parked at ({:.0}, {:.0}, {:.0})",
                    v.spec.label, at.x, at.y, at.z
                );
            }
        }
        self.vehicle_wait -= dt;
        if self.vehicle_wait > 0.0 || !player.landed {
            return;
        }
        self.vehicle_wait = VEHICLE_LOOK;
        let Some(catalogue) = &scene.catalogue else {
            return;
        };
        let near = player.character.position;
        let mut wanted: Vec<(f32, &TemplatePlacement)> = world
            .index
            .templates
            .iter()
            .filter(|t| !self.fetched.contains(&t.entity))
            .filter_map(|t| {
                let d = t.position.distance(near);
                (d <= VEHICLE_REACH).then_some((d, t))
            })
            .collect();
        wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut placed = scene.vehicles.iter().filter(|v| v.spot.is_some()).count();
        let mut loads = Vec::new();
        for (_, t) in wanted {
            if placed >= WORLD_VEHICLES {
                break;
            }
            let models = vehicle_models(&t.name, &catalogue.vehicles);
            if models.is_empty() {
                continue;
            }
            self.fetched.insert(t.entity);
            placed += 1;
            eprintln!(
                "island vehicle {:#x}: {} as {}",
                t.entity, t.name, models[0].label
            );
            let f = t.rotation * Vec3::Z;
            loads.push((t.entity, t.position, f.x.atan2(f.z), models));
        }
        for (entity, at, yaw, models) in loads {
            scene.load_world_vehicle(entity, at, yaw, models);
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
        self.fetch_world_vehicles(dt);
        let scene = &mut self.scene;
        if let Some(world) = &mut scene.world {
            let streaming = world.stats().pending > 0;
            let physics = &mut world.physics;
            let player = scene.player.as_mut().filter(|_| self.on_foot);
            if let Some(player) = player {
                player.streaming = streaming;
                self.park_wait -= dt;
                if !self.restore.is_empty() {
                    restore(physics, &mut scene.vehicles, &mut self.restore);
                }
                let waiting = scene.vehicles.iter().any(parkable);
                if player.landed && waiting && self.park_wait <= 0.0 && self.park_tries < PARK_TRIES
                {
                    // Each look can cast thousands of rays: not every tick.
                    self.park_wait = 0.5;
                    self.park_tries += 1;
                    park(physics, &mut scene.vehicles, player, self.park_tries);
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
                if let Some((id, v)) = id
                    && !v.wrecked
                {
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
        let threat = self
            .scene
            .player
            .as_ref()
            .and_then(|p| self.combat.target(p, self.on_foot));
        if let Some(world) = &self.scene.world
            && (!self.on_foot || self.scene.player.as_ref().is_none_or(|p| p.landed))
        {
            self.scene
                .people
                .update(&world.physics, &world.index.spawns, focus, threat, dt);
        }
        let scene = &mut self.scene;
        if self.on_foot
            && let (Some(world), Some(player)) = (&mut scene.world, &mut scene.player)
        {
            let mut field = Field {
                physics: &mut world.physics,
                player,
                people: &mut scene.people,
                vehicles: &mut scene.vehicles,
                arsenal: &scene.arsenal,
            };
            self.combat.step(&mut field, intent.trigger, dt);
        }
        self.scene.tick(dt);
    }

    /// The game as it stands, to save; `None` without an island or player.
    pub fn save(&self) -> Option<SaveGame> {
        let world = self.scene.world.as_ref()?;
        let player = self.scene.player.as_ref()?;
        let mut at = player.character.position;
        if let Some(v) = player.driving.and_then(|i| self.scene.vehicles.get(i))
            && let Some(m) = v.id.and_then(|id| world.physics.vehicle_transform(id))
        {
            // Saved beside the vehicle, not in it.
            at = m.transform_point3(Vec3::new(v.spec.bounds.0.x - 1.0, 0.0, 0.0));
        }
        let vehicles = self
            .scene
            .vehicles
            .iter()
            // The mission's vehicles are the scripts' to place, the
            // island's the layers'.
            .filter(|v| v.guid.is_none() && v.spot.is_none())
            .filter_map(|v| {
                let m = world.physics.vehicle_transform(v.id?)?;
                let f = m.transform_vector3(Vec3::Z);
                Some(VehicleSave {
                    model: v.spec.model,
                    position: m.transform_point3(Vec3::ZERO).to_array(),
                    yaw: f.x.atan2(f.z),
                    wrecked: v.wrecked,
                })
            })
            // Vehicles a save placed whose models haven't loaded yet.
            .chain(self.restore.iter().cloned())
            .collect();
        Some(SaveGame {
            version: save::VERSION,
            player: PlayerSave {
                position: at.to_array(),
                facing: player.facing,
                yaw: player.yaw,
                pitch: player.pitch,
                hp: self.combat.hp,
                weapon: match self.combat.weapon {
                    Weapon::Rifle => "rifle",
                    Weapon::Rpg => "rpg",
                }
                .into(),
            },
            vehicles,
            killed: self.scene.people.all_killed(),
        })
    }

    /// Puts the game back as `save` left it: the player drops to the ground
    /// where she was, vehicles go back as their models load, and the dead
    /// stay dead.
    pub fn load(&mut self, save: SaveGame) {
        let (Some(world), Some(player)) = (&mut self.scene.world, &mut self.scene.player) else {
            return;
        };
        for v in &mut self.scene.vehicles {
            if let Some(id) = v.id.take() {
                world.physics.remove_vehicle(id);
            }
            v.wrecked = false;
        }
        let p = &save.player;
        player.character.position = Vec3::from(p.position);
        player.character.velocity = Vec3::ZERO;
        player.landed = false;
        player.driving = None;
        player.chase_target = None;
        player.down = false;
        player.hold = None;
        player.facing = p.facing;
        player.yaw = p.yaw;
        player.pitch = p.pitch;
        self.combat = Combat::default();
        self.combat.hp = p.hp.clamp(1.0, 100.0);
        self.combat.weapon = if p.weapon == "rpg" {
            Weapon::Rpg
        } else {
            Weapon::Rifle
        };
        self.scene.people.reset(save.killed);
        self.restore = save.vehicles;
        // Vehicles not in the save park afresh.
        self.park_tries = 0;
        self.on_foot = true;
    }

    /// Parking is finished: every vehicle loaded is parked, or the search
    /// gave up.
    pub fn parking_done(&self) -> bool {
        self.park_tries >= PARK_TRIES || !self.scene.vehicles.iter().any(parkable)
    }

    /// What the HUD shows.
    pub fn hud_info(&self) -> HudInfo {
        let player = self.scene.player.as_ref();
        let loading = match &self.scene.world {
            // The island-wide tiles; nearby detail streams in quietly.
            Some(w) if w.stats().tiles + w.stats().failed < w.index.tiles.len() => {
                Some("Loading the island...".to_owned())
            }
            Some(_) if self.scene.vehicles_loading() => {
                Some("Finding vehicles and people...".to_owned())
            }
            _ => None,
        };
        let driving = player.and_then(|p| p.driving).and_then(|i| {
            let v = self.scene.vehicles.get(i)?;
            let speed = self.driving().map_or(0.0, |d| d.1);
            Some((v.spec.label.clone(), speed, v.wrecked))
        });
        let can_enter = match (&self.scene.world, player) {
            (Some(w), Some(p)) if p.driving.is_none() && p.landed => {
                nearest(&w.physics, &self.scene.vehicles, p, REACH)
                    .map(|i| self.scene.vehicles[i].spec.label.clone())
            }
            _ => None,
        };
        HudInfo {
            playing: self.on_foot && player.is_some(),
            hp: self.combat.hp,
            down: self.combat.down > 0.0,
            weapon: self.combat.weapon,
            driving,
            can_enter,
            loading,
            ..HudInfo::default()
        }
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
/// Looks before parking gives up on the vehicles left (cramped ground: a
/// ledge, a town).
const PARK_TRIES: u32 = 40;
/// A spot is flat enough when the ground under the footprint's corners
/// varies by less than this fraction of its length (about 10°) plus a
/// little, near the height of her feet (not a roof).
const PARK_FLAT: f32 = 0.18;
const PARK_ROOF: f32 = 8.0;
/// Room left between parked vehicles, metres.
const PARK_GAP: f32 = 1.5;

/// Parks vehicles that have loaded but have no body yet on flat, clear
/// ground near the spot ahead of the player, facing the way she faces.
fn park(physics: &mut Physics, vehicles: &mut [ParkedVehicle], player: &Player, tries: u32) {
    if !vehicles.iter().any(parkable) {
        return;
    }
    let (s, c) = player.facing.sin_cos();
    let forward = Vec3::new(s, 0.0, c);
    let right = Vec3::new(c, 0.0, -s);
    let feet = player.character.position;
    let ahead = feet + forward * PARK_AHEAD;
    // Spots nearest the one ahead first.
    // Later looks reach further and take steeper ground.
    let reach = PARK_SPOTS + (tries / 2) as i32;
    let flat = PARK_FLAT * (1.0 + tries as f32 / 8.0).min(3.0);
    let mut spots: Vec<Vec3> = (-reach..=reach)
        .flat_map(|i| (-reach..=reach).map(move |j| (i, j)))
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
    for v in vehicles.iter_mut().filter(|v| parkable(v)) {
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
            (max - min <= length * flat + 0.2 && (max - feet.y).abs() < PARK_ROOF)
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

/// A vehicle of ours (not the mission's or the island's) still to park.
fn parkable(v: &ParkedVehicle) -> bool {
    v.id.is_none() && v.guid.is_none() && v.spot.is_none()
}

/// Puts a vehicle on the ground at `at` facing `yaw`; false while the
/// ground there hasn't loaded.
pub(crate) fn place(physics: &mut Physics, v: &mut ParkedVehicle, at: Vec3, yaw: f32) -> bool {
    let top = at + Vec3::Y * 30.0;
    let Some((d, _)) = physics.cast_ray(top, Vec3::NEG_Y, 80.0) else {
        return false;
    };
    let ground = top.y - d;
    let at = Vec3::new(at.x, ground + v.spec.ride_height() + 0.3, at.z);
    v.id = Some(physics.add_vehicle(&v.spec, at, yaw));
    true
}

/// The game's own parked vehicles within this many metres of the player
/// are loaded and placed where the layers put them, nearest first, up to
/// [`WORLD_VEHICLES`] at a time; ones she drives away stay.
const VEHICLE_REACH: f32 = 400.0;
const WORLD_VEHICLES: usize = 12;
/// Seconds between looks for vehicles to load.
const VEHICLE_LOOK: f32 = 1.0;

/// Puts vehicles from a save back where they were, as their models load.
fn restore(physics: &mut Physics, vehicles: &mut [ParkedVehicle], saved: &mut Vec<VehicleSave>) {
    for v in vehicles.iter_mut().filter(|v| parkable(v)) {
        let Some(k) = saved.iter().position(|s| s.model == v.spec.model) else {
            continue;
        };
        let s = saved.swap_remove(k);
        // A little high, to settle onto its wheels.
        let at = s.position() + Vec3::Y * 0.3;
        v.id = Some(physics.add_vehicle(&v.spec, at, s.yaw));
        v.wrecked = s.wrecked;
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
        .filter(|(_, v)| !v.wrecked)
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
pub(crate) fn get_out(physics: &Physics, v: &ParkedVehicle, player: &mut Player) {
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
