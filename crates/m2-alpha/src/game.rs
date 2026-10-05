//! What a tick does: the player on foot (or the free camera), physics and
//! animation, and where the frame is seen from.

use std::collections::HashSet;

use m2_engine::physics::FLOAT_DEPTH;
use m2_engine::{CatalogueEntry, Physics, StaticKey, VehicleControls, VehicleSpec, World};
use m2_math::glam::{Mat4, Quat, Vec2, Vec3, Vec4Swizzles};
use m2_render::View;

use crate::camera::FlyCamera;
use crate::combat::{self, Combat, Effect, Field, Weapon};
use crate::config::CameraConfig;
use crate::input::{Action, Input};
use crate::mission::{WATERLINE, template_models};
use crate::player::{ChaseRig, Intent, Player};
use crate::props::Props;
use crate::save::{self, PlayerSave, SaveGame, VehicleSave};
use crate::scene::{ParkedVehicle, Scene};
use crate::startup::Startup;
use crate::support::{self, Call, Impact, Support};
use crate::ui::HudInfo;

/// Where a vehicle of the layers stands.
struct Spot<'a> {
    entity: u32,
    name: &'a str,
    at: Vec3,
    yaw: f32,
}

/// How fast a turret turns after the camera, and its gun tilts to the
/// crosshair, radians a second, when its template doesn't say (invented).
const TURRET_RATE: f32 = 1.2;
const GUN_RATE: f32 = 0.6;
/// How near a building the town's sounds play, metres.
const TOWN_REACH: f32 = 150.0;

/// The scripts put vehicles they don't need yet far under the island:
/// those below this height aren't placed.
const HIDDEN_BELOW: f32 = -1000.0;

/// A vehicle held where it stands, far from her, is let go once she is
/// this much nearer than the island's full-detail ground reaches and
/// ground is found within this far under it, metres.
const HOLD_SLACK: f32 = 100.0;
const HOLD_GROUND: f32 = 30.0;

/// Boats going faster than this throw spray off their sides and churn a
/// wake, the most from the second speed (m/s), puffs every this many
/// seconds, each lasting this long, the wake longer. Invented, from the
/// playthroughs' speedboat: spray wider than the boat at full speed.
const SPRAY_SPEED: f32 = 4.0;
const SPRAY_FULL: f32 = 16.0;
const SPRAY_GAP: f32 = 0.05;
const SPRAY_LIFE: f32 = 0.7;
const WAKE_LIFE: f32 = 1.2;

/// A vehicle knocks loose what it hits that comes loose (traffic lights,
/// plants, barricades, poles: their `PHY2` has bodies that move) at this
/// speed or more (m/s), pushing with its engine counting as this one, and
/// when its momentum is at least this many times the mass to shift
/// (kg m/s per kg). Invented: a car fells a lamp post or a plant, a tank
/// a parked car.
const KNOCK_SPEED: f32 = 1.0;
const PUSH_SPEED: f32 = 1.5;
const KNOCK_RATIO: f32 = 4.0;
/// What it knocks loose flies off a little faster than the two go on
/// together, so it clears the vehicle, this much up (m/s), toppling the
/// way it was hit at this many radians a second per m/s.
const FLING: f32 = 1.1;
const FLING_UP: f32 = 1.0;
const TOPPLE: f32 = 0.3;

/// A bomb lands on what it first meets falling from this far above her
/// (metres).
const BOMB_DROP: f32 = 600.0;

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
    /// The layers the game starts with, which the island's vehicles come
    /// from; `None` places them from every static layer (`--mission`,
    /// where the scripts' own layers rule).
    pub startup: Option<Startup>,
    /// The start-up's state layers have their models up.
    props_up: bool,
    /// The driver's throttle last tick, -1 (reverse) to 1.
    pub throttle: f32,
    /// Seconds to the next puffs of spray off boats going fast.
    spray_wait: f32,
    /// Her support: the items she holds and the call under way; her
    /// health when it last stepped (hurt, a call is called off).
    pub support: Support,
    support_hp: f32,
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
            startup: None,
            props_up: false,
            throttle: 0.0,
            spray_wait: 0.0,
            support: Support::default(),
            support_hp: 0.0,
        }
    }

    /// Among the town's buildings: a building placement (a town's batch
    /// or a lone building) within [`TOWN_REACH`] of the listener.
    pub fn near_town(&self) -> bool {
        let (ear, _) = self.listener();
        self.scene.world.as_ref().is_some_and(|w| {
            w.index
                .objects
                .iter()
                .any(|o| !o.named && o.position.distance(ear) < TOWN_REACH)
        })
    }

    /// Island vehicles asked for that have not been placed yet.
    pub fn island_vehicles_loading(&self) -> bool {
        if self.startup.as_ref().is_some_and(Startup::loading) {
            return true;
        }
        let placed = self
            .scene
            .vehicles
            .iter()
            .filter(|v| v.spot.is_some() && v.id.is_some())
            .count();
        placed < self.fetched.len()
    }

    /// Puts up the models of the state layers the game starts with, once
    /// the start-up has said which.
    fn startup_props(&mut self) {
        if self.props_up {
            return;
        }
        let Some(s) = &mut self.startup else { return };
        s.poll();
        let set = match s {
            Startup::Ready(set) => set,
            // Not knowing which, every state layer's people stand.
            Startup::Failed => {
                self.props_up = true;
                self.scene.people.state_layers = None;
                return;
            }
            Startup::Loading(_) => return,
        };
        self.props_up = true;
        self.scene.people.state_layers = Some(set.hashes.clone());
        for e in &set.entities {
            self.scene.props.note_layer([e.entity], &e.layer);
        }
        let shown = self.scene.show_props(set.entities.iter().map(|e| e.entity));
        eprintln!("start-up state layers: {shown} models put up");
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
        if let Some(s) = &mut self.startup {
            s.poll();
        }
        // Where they may stand: the static layers the game starts with
        // (all of them if that set isn't known) and its state layers.
        let (static_layers, state_layers) = match &self.startup {
            Some(Startup::Loading(_)) => return,
            Some(Startup::Ready(set)) => (Some(&set.hashes), set.entities.as_slice()),
            _ => (None, &[][..]),
        };

        let near = player.character.position;
        let mut wanted: Vec<(f32, Spot<'_>)> = world
            .index
            .templates
            .iter()
            .filter(|t| static_layers.is_none_or(|h| h.contains(&t.layer)))
            .map(|t| {
                let f = t.rotation * Vec3::Z;
                Spot {
                    entity: t.entity,
                    name: &t.name,
                    at: t.position,
                    yaw: f.x.atan2(f.z),
                }
            })
            .chain(state_layers.iter().map(|e| Spot {
                entity: e.entity,
                name: &e.name,
                at: e.position,
                yaw: e.yaw,
            }))
            .filter(|t| !self.fetched.contains(&t.entity) && t.at.y > HIDDEN_BELOW)
            .filter_map(|t| {
                let d = t.at.distance(near);
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
            let models = template_models(t.name, &world.index.template_db, &catalogue.vehicles);
            if models.is_empty() {
                continue;
            }
            self.fetched.insert(t.entity);
            placed += 1;
            eprintln!(
                "island vehicle {:#x}: {} as {}",
                t.entity, t.name, models[0].label
            );
            loads.push((t.entity, t.at, t.yaw, models));
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
        self.startup_props();
        self.fetch_world_vehicles(dt);
        let intent = self.call_support(intent, dt);
        // The use key at what lies within reach: taken with the fight's.
        let mut taking = false;
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
                        // Not out of a boat into deep water.
                        Some(i) if can_get_out(physics, &scene.vehicles[i]) => {
                            get_out(physics, &scene.vehicles[i], player)
                        }
                        Some(_) => {}
                        // What lies within reach before any vehicle.
                        None if scene.flights.within(player.character.position).is_some() => {
                            taking = true;
                        }
                        None => {
                            player.driving = nearest(physics, &scene.vehicles, player, REACH);
                            // Seated: she no longer waits to land (people
                            // wait on that, and she may have hopped out and
                            // back in before the ground streamed).
                            if player.driving.is_some() {
                                player.landed = true;
                            }
                        }
                    }
                }
                let id = player
                    .driving
                    .and_then(|i| scene.vehicles.get(i))
                    .and_then(|v| v.id.map(|id| (id, v)));
                self.throttle = 0.0;
                if let Some((id, v)) = id
                    && !v.wrecked
                {
                    self.throttle = intent.movement.y;
                    physics.drive(
                        id,
                        VehicleControls {
                            throttle: intent.movement.y,
                            // Wheels turned the positive way steer left.
                            steer: -intent.movement.x,
                            brake: intent.hold_jump,
                            parked: false,
                        },
                    );
                    if intent.right_vehicle {
                        physics.right_vehicle(id);
                    }
                }
                physics.step(dt);
                match id.and_then(|(id, v)| Some((physics.vehicle_transform(id)?, v, id))) {
                    Some((at, v, id)) => {
                        let speed = physics.vehicle_speed(id);
                        player.chase(at, ChaseRig::of(&v.spec), intent.look, speed, dt);
                    }
                    None => {
                        player.driving = None;
                        player.chase_target = None;
                        player.update(physics, intent, dt);
                    }
                }
                // A turret turns after the camera, at its own pace, and
                // its gun tilts to what the crosshair is on.
                if let Some(v) = player.driving.and_then(|i| scene.vehicles.get_mut(i))
                    && !v.wrecked
                    && let Some(at) = v.id.and_then(|id| physics.vehicle_transform(id))
                {
                    let f = at.transform_vector3(Vec3::Z);
                    let mut turn =
                        (player.yaw - f.x.atan2(f.z) - v.turret).rem_euclid(std::f32::consts::TAU);
                    if turn > std::f32::consts::PI {
                        turn -= std::f32::consts::TAU;
                    }
                    let turret = v.spec.tuning.turret;
                    let rate = turret.map_or(TURRET_RATE, |t| t.yaw_speed);
                    v.turret += turn.clamp(-rate * dt, rate * dt);
                    if let Some(pivot) = v.spec.gun_pivot {
                        let goal = Combat::aim_point(physics, &scene.people, player, v.id);
                        // The crosshair's point as seen from the gun with
                        // the turret straight ahead.
                        let local = (at * v.spec.turret_transform(v.turret))
                            .inverse()
                            .transform_point3(goal)
                            - pivot;
                        let (down, up) = v.spec.elevation;
                        let want = local.y.atan2(local.with_y(0.0).length()).clamp(down, up);
                        let rate = turret.map_or(GUN_RATE, |t| t.pitch_speed);
                        v.gun += (want - v.gun).clamp(-rate * dt, rate * dt);
                    }
                }
            } else {
                physics.step(dt);
            }
        }
        let focus = self.focus();
        if let Some(world) = &mut self.scene.world {
            let driven = self
                .scene
                .player
                .as_ref()
                .filter(|_| self.on_foot)
                .and_then(|p| p.driving);
            let hold = world.settings.detail_radius;
            for (i, v) in self.scene.vehicles.iter_mut().enumerate() {
                let Some(id) = v.id else { continue };
                // Far from her, the island's ground streams out from under
                // a vehicle: it is held where it stands until she comes
                // back and the ground under it is in again.
                if driven != Some(i)
                    && let Some(at) = world
                        .physics
                        .vehicle_transform(id)
                        .map(|m| m.w_axis.truncate())
                {
                    let far = at.distance(focus);
                    if far > hold {
                        world.physics.hold_vehicle(id, true);
                    } else if far < hold - HOLD_SLACK
                        && world.physics.vehicle_held(id)
                        // A boat floats on the sea, whatever is under it.
                        && ((v.spec.is_boat() && world.physics.water().is_some())
                            || world
                                .physics
                                .cast_ray_past(at + Vec3::Y, Vec3::NEG_Y, HOLD_GROUND, id)
                                .is_some())
                    {
                        world.physics.hold_vehicle(id, false);
                    }
                }
                v.rolled += world.physics.vehicle_speed(id) * dt;
                // Nobody at the wheel: it brakes to a stop and stays put,
                // on a hill too (a wreck drags on its rims, driven by
                // nobody).
                if driven != Some(i) {
                    world.physics.drive(
                        id,
                        VehicleControls {
                            parked: true,
                            ..VehicleControls::default()
                        },
                    );
                }
            }
            knock_loose(world, &mut self.scene.props, &self.scene.vehicles);
            self.scene.props.advance(dt);
            self.spray_wait -= dt;
            if self.spray_wait <= 0.0 {
                self.spray_wait = SPRAY_GAP;
                spray(world, &self.scene.vehicles, &mut self.combat);
            }
        }
        let threat = self
            .scene
            .player
            .as_ref()
            .and_then(|p| self.combat.target(p, self.on_foot));
        if let Some(world) = &mut self.scene.world
            && (!self.on_foot || self.scene.player.as_ref().is_none_or(|p| p.landed))
        {
            self.scene
                .people
                .update(&mut world.physics, &world.index.spawns, focus, threat, dt);
            let cars: Vec<_> = self
                .scene
                .vehicles
                .iter()
                .filter_map(|v| {
                    let id = v.id?;
                    let at = world.physics.vehicle_transform(id)?;
                    Some((at, world.physics.vehicle_velocity(id), v.spec.hull))
                })
                .collect();
            self.scene.people.traffic(&cars);
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
                explosives: &mut scene.explosives,
            };
            self.combat.step(&mut field, intent.trigger, dt);
            world
                .destroyed
                .extend(scene.explosives.iter().filter(|e| e.gone).map(|e| e.object));
            blast_loose(
                world,
                &mut scene.props,
                &self.combat.went_off,
                combat::REACH,
                false,
            );
            // A supply crate broken open: a puff of dust off its boards.
            let broken = scene.flights.strike(
                &mut world.physics,
                &self.combat.struck,
                &self.combat.went_off,
            );
            for (_, at) in broken {
                self.combat.show(
                    Effect::Smoke {
                        at: at + Vec3::Y * 0.6,
                        size: 1.4,
                    },
                    1.2,
                );
            }
            // What a broken crate left, taken with the use key from
            // within its reach.
            if taking
                && player.driving.is_none()
                && let Some(p) = scene.flights.take_within(player.character.position)
            {
                take_up(
                    &mut self.combat,
                    &mut scene.flights,
                    &world.index.template_db,
                    player,
                    p,
                );
            }
        }
        self.support_impacts();
        self.scene.tick(dt);
    }

    /// Her support call this tick ([`crate::support`]): on foot, the
    /// support key, the use key and fire go to it while it is under way,
    /// and the view's steering; what is left of `intent` is hers.
    fn call_support(&mut self, mut intent: Intent, dt: f32) -> Intent {
        let her = self
            .scene
            .player
            .as_ref()
            .filter(|p| self.on_foot && p.landed && p.driving.is_none() && !p.down);
        let at = her
            .map(|p| p.character.position)
            .unwrap_or(self.camera.position);
        let hp = self.combat.hp;
        let call = match her {
            Some(_) => Call {
                menu: intent.trigger.support,
                use_pressed: intent.use_pressed,
                fire: intent.trigger.fire,
                steer: intent.movement,
                look: intent.look,
                select: intent.trigger.select.is_some(),
                hurt: hp < self.support_hp,
                pda: intent.trigger.pda,
            },
            None => Call::default(),
        };
        self.support_hp = hp;
        let before = self.support.busy();
        // In a vehicle, down or flying the camera, a call is dropped (and
        // the PDA shut).
        if her.is_none() && self.support.busy() {
            self.support.phase = support::Phase::Idle;
            if self.support.pda {
                self.support.pda = false;
                self.support.posts.push(support::PDA_CLOSE);
            }
        }
        self.support.step(call, at, dt);
        self.combat.designator = self.support.holding() || self.support.looking().is_some();
        if before || self.support.busy() {
            // At the menu, the designator in her hands or looking through
            // the satellite, she stands, and fire is the satellite's.
            intent = Intent::default();
        }
        intent
    }

    /// What the support brought down this step: the bomb on what it
    /// first meets at the spot, its blast through the fight, and what
    /// comes loose thrown (the sturdy too).
    fn support_impacts(&mut self) {
        if self.support.landed.is_empty() {
            return;
        }
        let scene = &mut self.scene;
        let (Some(world), Some(player)) = (&mut scene.world, &mut scene.player) else {
            return;
        };
        let high = player.character.position.y + BOMB_DROP;
        for impact in self.support.landed.clone() {
            let Vec2 { x, y: z } = impact.at();
            let top = Vec3::new(x, high, z);
            let at = world
                .physics
                .cast_ray(top, Vec3::NEG_Y, BOMB_DROP * 2.0)
                .map_or(Vec3::new(x, player.character.position.y, z), |(d, _)| {
                    top - Vec3::Y * d
                });
            let mut field = Field {
                physics: &mut world.physics,
                player,
                people: &mut scene.people,
                vehicles: &mut scene.vehicles,
                arsenal: &scene.arsenal,
                explosives: &mut scene.explosives,
            };
            match impact {
                Impact::Bomb(_) => {
                    eprintln!("support: the bomb lands at {at:.1}");
                    self.combat.bomb(&mut field, at);
                    blast_loose(world, &mut scene.props, &[at], combat::BOMB.0, true);
                }
                Impact::Final(_) => self.combat.final_strike(&mut field, at),
            }
            world
                .destroyed
                .extend(scene.explosives.iter().filter(|e| e.gone).map(|e| e.object));
        }
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
                    Weapon::GrenadeLauncher => "grenade launcher",
                }
                .into(),
                second: match self.combat.second {
                    Some(Weapon::GrenadeLauncher) => "grenade launcher",
                    Some(_) => "rpg",
                    None => "",
                }
                .into(),
                launcher: self
                    .combat
                    .launcher_rounds
                    .filter(|_| self.combat.second == Some(Weapon::GrenadeLauncher))
                    .map(|r| [r.clip, r.stored]),
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
        player.aiming = false;
        player.facing = p.facing;
        player.yaw = p.yaw;
        player.pitch = p.pitch;
        self.combat = Combat::default();
        self.combat.hp = p.hp.clamp(1.0, 100.0);
        match (p.second.as_str(), p.launcher) {
            ("grenade launcher", Some([clip, stored])) => {
                self.combat.equip(Weapon::GrenadeLauncher, 0);
                self.combat.launcher_rounds = Some(combat::Rounds { clip, stored });
            }
            ("rpg", _) => {}
            _ => self.combat.carbine_only(),
        }
        self.combat.weapon = match (p.weapon.as_str(), self.combat.second) {
            ("rpg" | "grenade launcher", Some(w)) => w,
            _ => Weapon::Rifle,
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
        // What the crosshair is on: on foot, or in a vehicle with a gun.
        let crosshair = match (&self.scene.world, player) {
            (Some(w), Some(p)) if self.combat.down <= 0.0 => match p.driving {
                Some(i) => self
                    .scene
                    .vehicles
                    .get(i)
                    .filter(|v| v.spec.muzzle.is_some() && !v.wrecked)
                    .map(|v| Combat::aim_point(&w.physics, &self.scene.people, p, v.id)),
                None if self.on_foot && p.landed => {
                    Some(Combat::aim_point(&w.physics, &self.scene.people, p, None))
                }
                None => None,
            },
            _ => None,
        };
        let can_enter = match (&self.scene.world, player) {
            (Some(w), Some(p)) if p.driving.is_none() && p.landed => {
                nearest(&w.physics, &self.scene.vehicles, p, REACH)
                    .map(|i| self.scene.vehicles[i].spec.label.clone())
            }
            _ => None,
        };
        // What the use key takes, as the game words it.
        let prompt = match (&self.scene.world, player) {
            (Some(w), Some(p)) if p.driving.is_none() && p.landed && self.on_foot => self
                .scene
                .flights
                .within(p.character.position)
                .map(|p| match p.what.weapon {
                    Some(weapon) => format!(
                        "Equip {}",
                        w.index
                            .template_db
                            .by_id(p.what.template)
                            .map_or_else(|| format!("{weapon:?}"), |t| t.0.to_owned())
                    ),
                    None => "Take Ammo".to_owned(),
                }),
            _ => None,
        };
        HudInfo {
            playing: self.on_foot && player.is_some(),
            hp: self.combat.hp,
            down: self.combat.down > 0.0,
            weapon: self.combat.weapon,
            second: self.combat.second,
            rounds: self
                .combat
                .launcher_rounds
                .filter(|_| self.combat.weapon == Weapon::GrenadeLauncher)
                .map(|r| (r.clip, r.stored)),
            // Looking through the satellite, its own overlay is shown.
            prompt: prompt.filter(|_| !self.support.busy()),
            driving,
            crosshair: crosshair.filter(|_| self.support.looking().is_none()),
            can_enter: can_enter.filter(|_| !self.support.busy()),
            loading,
            support: self
                .support
                .hud(player.map_or(self.camera.position.y, |p| p.character.position.y)),
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
        player.landed |= player.driving.is_some();
        player.driving.is_some()
    }

    /// Loads the first of `models` that drives and parks it near her, as
    /// the vehicles she starts with are.
    pub fn park_more(&mut self, models: Vec<CatalogueEntry>) {
        self.scene.load_parked(models);
        self.park_tries = 0;
        self.park_wait = 0.0;
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

    /// The vehicle being driven's engine: its sound bank (the one its
    /// template names, else one for its kind), speed and throttle. A wreck
    /// has none.
    pub fn engine(&self) -> Option<crate::sound::EngineInput> {
        let i = self.scene.player.as_ref()?.driving?;
        let v = self.scene.vehicles.get(i).filter(|v| !v.wrecked)?;
        let (_, speed) = self.driving()?;
        let bank = self
            .scene
            .world
            .as_ref()
            .and_then(|w| w.index.template_db.engine_for_model(v.spec.model))
            .unwrap_or_else(|| {
                m2_assets::name_hash(crate::sound::engine_bank(&v.spec.kind, &v.spec.label))
            });
        Some(crate::sound::EngineInput {
            bank,
            speed,
            top: v
                .spec
                .tuning
                .max_speed
                .unwrap_or(if v.spec.tracked { 14.0 } else { 32.0 }),
            throttle: self.throttle,
        })
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

    /// Where the frame is seen from: her camera (shaken by a bomb close
    /// by), the satellite's while she looks through it, or the free one.
    pub fn view(&self, fov: f32, aspect: f32) -> View {
        let mut view = match self.walking() {
            Some(p) => match self.support.looking() {
                Some(at) => support::satellite_view(at, p.character.position.y, aspect),
                None => p.view(fov, aspect),
            },
            None => self.camera.view(fov, aspect),
        };
        if let Some((a, b)) = self.support.shake_turn()
            && self.support.looking().is_none()
        {
            view.view = Mat4::from_rotation_x(a) * Mat4::from_rotation_y(b) * view.view;
        }
        view
    }
}

/// How close she must be to a vehicle's door (or side) to get in, metres.
const REACH: f32 = 4.0;

/// A weapon she leaves for another lies this far to her right, metres.
const LEFT_BESIDE: f32 = 0.6;

/// What she takes from a pickup: a weapon is taken up in the launcher's
/// place (what she carried there left lying beside her, to be taken
/// back), ammunition goes with its weapon.
fn take_up(
    combat: &mut Combat,
    flights: &mut crate::flights::Flights,
    templates: &m2_engine::Templates,
    player: &Player,
    p: crate::flights::Pickup,
) {
    let rounds = p.what.gives.map_or(0, |(_, n)| n);
    let Some(weapon) = p.what.weapon else {
        if let Some((w, n)) = p.what.gives {
            combat.take_ammo(w, n);
        }
        eprintln!(
            "pickup: she takes {:#x} at {:.1}{}",
            p.what.template,
            p.at,
            p.what.gives.map_or(String::new(), |(w, n)| format!(
                ": {n} rounds for her {w:?}"
            ))
        );
        return;
    };
    let left = combat.equip(weapon, rounds);
    eprintln!(
        "pickup: she takes up the {weapon:?} ({:#x}) at {:.1} with {rounds} rounds{}",
        p.what.template,
        p.at,
        match left {
            Some((w, 0)) => format!(", leaving her {w:?}"),
            Some((w, n)) => format!(", leaving her {w:?} with {n} rounds"),
            None => String::new(),
        }
    );
    let Some((left, kept)) = left else {
        return;
    };
    let model = m2_assets::name_hash(left.model_name());
    // Its template: the pickup with its model (the lowest id of several).
    let template = templates
        .leavings
        .pickups
        .iter()
        .filter(|(id, _)| {
            templates
                .by_id(**id)
                .is_some_and(|(_, t)| t.model == Some(model))
        })
        .min_by_key(|(id, _)| **id);
    let (sin, cos) = player.facing.sin_cos();
    let right = Vec3::new(-cos, 0.0, sin);
    flights.leave(crate::flights::Pickup {
        from: 0,
        what: crate::flights::Leaves {
            template: template.map_or(0, |(id, _)| *id),
            model: Some(model),
            reach: template.map_or(1.0, |(_, info)| info.radius),
            weapon: Some(left),
            gives: Some((left, kept)),
        },
        at: player.character.position + right * LEFT_BESIDE,
        yaw: player.facing,
    });
}

/// Parking: the first spot looked at is this far ahead of her; spots are
/// tried on a grid this fine, out to this far.
const PARK_AHEAD: f32 = 14.0;
const PARK_GRID: f32 = 4.0;
const PARK_SPOTS: i32 = 12;
/// Looks before parking gives up on the vehicles left (cramped ground: a
/// ledge, a town).
const PARK_TRIES: u32 = 40;
/// A spot will do when the ground under the wheels slopes less than this
/// (radians, about 10°; later looks take up to twice it), nothing under
/// the hull rises more than this above them (metres: a rock, a ledge),
/// and it is near the height of her feet (not a roof or a boulder's top):
/// within [`PARK_RISE`] metres, and [`PARK_GRADE`] more for each metre
/// away, up to [`PARK_ROOF`].
const PARK_SLOPE: f32 = 0.18;
const PARK_BUMPS: f32 = 0.3;
const PARK_RISE: f32 = 1.5;
const PARK_GRADE: f32 = 0.15;
const PARK_ROOF: f32 = 8.0;
/// The ground under a spot is looked for from this far above her feet.
const PARK_LOOK: f32 = 60.0;
/// A spot whose middle is less than this above the sea is in it, metres;
/// one whose middle is further than this beyond the rise allowed (see
/// [`PARK_RISE`]) isn't looked at more closely.
const PARK_SHORE: f32 = 0.5;
const PARK_SLACK: f32 = 2.0;
/// Vehicles are let go this far above where they rest, to settle onto
/// their wheels, metres.
const DROP: f32 = 0.3;
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
    let steep = PARK_SLOPE * (1.0 + tries as f32 / 8.0).min(2.0);
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
    // The ground under each spot's middle, looked at once a look: a spot
    // in the sea, or far above or below her, is passed over before the
    // ground under each vehicle's wheels there is (a look on a coast or a
    // hillside otherwise casts a ray or two dozen for every spot).
    let mut middles: Vec<Option<Option<f32>>> = vec![None; spots.len()];
    for v in vehicles.iter_mut().filter(|v| parkable(v)) {
        let (lo, hi) = v.spec.bounds;
        let radius = footprint_radius(&v.spec);
        let middle = Vec3::new((lo.x + hi.x) * 0.5, 0.0, (lo.z + hi.z) * 0.5);
        // From high above, so a boulder or a cliff taller than her isn't
        // missed by a ray starting inside it (the spot is then too high).
        let ground = |p: Vec3| {
            let top = Vec3::new(p.x, feet.y + PARK_LOOK, p.z);
            physics
                .cast_ray(top, -Vec3::Y, PARK_LOOK + PARK_ROOF * 2.0)
                .map(|(d, _)| top.y - d)
        };
        let spot = spots.iter().zip(&mut middles).find_map(|(&centre, under)| {
            if taken
                .iter()
                .any(|&(c, r)| (c - centre).with_y(0.0).length() < r + radius + PARK_GAP)
            {
                return None;
            }
            let away = (centre - feet).with_y(0.0).length();
            let rise = (PARK_RISE + PARK_GRADE * away).min(PARK_ROOF);
            let h = (*under.get_or_insert_with(|| ground(centre)))?;
            if h < WATERLINE + PARK_SHORE || (h - feet.y).abs() > rise + PARK_SLACK {
                return None;
            }
            // Tilted to the ground under its wheels, clear of what is
            // under its hull.
            let origin = centre - (right * middle.x + forward * middle.z);
            let rest = v.spec.rest_on(origin, player.facing, ground)?;
            (rest.slope <= steep
                && rest.bumps <= PARK_BUMPS
                && (rest.position.y - feet.y).abs() < rise)
                .then_some((centre, rest))
        });
        // Nowhere yet (the ground may still be loading): try next tick.
        let Some((centre, rest)) = spot else { continue };
        let at = rest.position + Vec3::Y * DROP;
        v.id = Some(physics.add_vehicle_turned(&v.spec, at, rest.rotation));
        taken.push((centre, radius));
    }
}

/// Vehicles knock loose what they hit that comes loose, with the weight
/// and speed for it ([`KNOCK_RATIO`]): it flies off ahead, and the vehicle
/// ploughs on slowed as by the mass it shifted, rather than stopped as by
/// a wall (the step just taken stopped it).
fn knock_loose(world: &mut World, props: &mut Props, vehicles: &[ParkedVehicle]) {
    for v in vehicles {
        let Some(id) = v.id else { continue };
        let before = world.physics.vehicle_velocity_before(id);
        let mut speed = before.length();
        if world.physics.vehicle_pulling(id) {
            speed = speed.max(PUSH_SPEED);
        }
        if speed < KNOCK_SPEED {
            continue;
        }
        let mass = v.spec.mass;
        let mut shifted = 0.0;
        for key in world.physics.vehicle_statics(id) {
            let loose = match key {
                StaticKey::Object(i) => world.loose_mass(i),
                StaticKey::Entity(e) => props.loose_mass(&world.physics, e),
                _ => None,
            };
            let Some(m) = loose.filter(|&m| mass * speed >= KNOCK_RATIO * m) else {
                continue;
            };
            let ahead = before.with_y(0.0);
            let fling = before * (mass / (mass + m)) * FLING + Vec3::Y * FLING_UP;
            let topple = Vec3::Y.cross(ahead.normalize_or_zero()) * (ahead.length() * TOPPLE);
            let knocked = match key {
                StaticKey::Object(i) => world.knock(i, fling, topple),
                StaticKey::Entity(e) => props.knock(&mut world.physics, e, fling, topple, false),
                _ => false,
            };
            if knocked {
                shifted += m;
                eprintln!(
                    "knocked loose: {key:?} ({m:.0} kg) by the {} at {speed:.1} m/s",
                    v.spec.label
                );
            }
        }
        if shifted > 0.0 {
            world
                .physics
                .set_vehicle_velocity(id, before * (mass / (mass + shifted)));
        }
    }
}

/// Blasts knock loose what comes loose within `reach` (as far as they
/// hurt people), thrown out and up as people are, harder the nearer; a
/// support `strike`'s the sturdy too.
fn blast_loose(world: &mut World, props: &mut Props, blasts: &[Vec3], reach: f32, strike: bool) {
    let thrown = |blast: Vec3, at: Vec3, d: f32| {
        let near = (1.0 - d / reach).clamp(0.0, 1.0);
        let away = (at - blast).with_y(0.0).normalize_or(Vec3::X);
        let (out, up) = combat::BLAST_PUSH;
        let velocity = away * (out * near + 1.0) + Vec3::Y * (up * near + 1.0);
        (velocity, Vec3::Y.cross(away) * (TOPPLE * out * near))
    };
    for &at in blasts {
        for (i, d) in world.loose_near(at, reach) {
            let (velocity, spin) = thrown(at, world.index.objects[i].position, d);
            world.knock(i, velocity, spin);
        }
        for (e, d) in props.loose_near(&world.physics, at, reach, strike) {
            let Some(p) = props.get(e).map(|(p, _)| p.position) else {
                continue;
            };
            let (velocity, spin) = thrown(at, p, d);
            props.knock(&mut world.physics, e, velocity, spin, strike);
        }
    }
}

/// A vehicle of ours (not the mission's or the island's) still to park.
fn parkable(v: &ParkedVehicle) -> bool {
    v.id.is_none() && v.guid.is_none() && v.spot.is_none()
}

/// Spray off both sides of each boat going fast on the water, a third of
/// the way up from its stern, and churned water behind it.
fn spray(world: &World, vehicles: &[ParkedVehicle], combat: &mut Combat) {
    let Some(water) = world.physics.water() else {
        return;
    };
    for v in vehicles.iter().filter(|v| v.spec.is_boat() && !v.wrecked) {
        let Some(id) = v.id else { continue };
        let Some(m) = world.physics.vehicle_transform(id) else {
            continue;
        };
        let speed = world.physics.vehicle_speed(id);
        if speed < SPRAY_SPEED || (m.w_axis.y - water).abs() > 1.5 {
            continue;
        }
        let k = ((speed - SPRAY_SPEED) / (SPRAY_FULL - SPRAY_SPEED)).clamp(0.0, 1.0);
        let (lo, hi) = v.spec.hull;
        let back = m
            .transform_vector3(Vec3::NEG_Z)
            .with_y(0.0)
            .normalize_or_zero();
        for x in [lo.x, hi.x] {
            let out = m
                .transform_vector3(Vec3::X * x.signum())
                .with_y(0.0)
                .normalize_or_zero();
            let at = m.transform_point3(Vec3::new(x, 0.0, lo.z + (hi.z - lo.z) * 0.3));
            combat.show(
                Effect::Spray {
                    at: at.with_y(water + 0.2),
                    drift: out * (2.0 + 4.0 * k)
                        + back * (1.0 + 2.0 * k)
                        + Vec3::Y * (1.0 + 2.0 * k),
                    size: 0.6 + k,
                },
                SPRAY_LIFE,
            );
        }
        let stern = m.transform_point3(Vec3::new(0.0, 0.0, lo.z));
        combat.show(
            Effect::Wake {
                at: stern.with_y(water + 0.03),
                size: (hi.x - lo.x) * (0.6 + 0.6 * k),
            },
            WAKE_LIFE,
        );
    }
}

/// Puts a vehicle on the ground at `at` facing `yaw`, tilted to the
/// ground under its wheels; false while the ground there hasn't loaded.
/// A boat on the sea floats on it.
pub(crate) fn place(physics: &mut Physics, v: &mut ParkedVehicle, at: Vec3, yaw: f32) -> bool {
    // From a little above where the layers put it (so one under a bridge
    // or a gantry stays under it), else from well above.
    let ground = |p: Vec3| {
        [6.0, 30.0].into_iter().find_map(|up| {
            let top = Vec3::new(p.x, at.y + up, p.z);
            physics
                .cast_ray(top, Vec3::NEG_Y, up + 50.0)
                .map(|(d, _)| top.y - d)
        })
    };
    // Level, as deep as it floats, where the seabed is under its keel (or
    // hasn't loaded).
    let float = physics.water().filter(|&water| {
        let keel = water - FLOAT_DEPTH + v.spec.hull.0.y;
        v.spec.is_boat() && ground(at).is_none_or(|h| h < keel)
    });
    if let Some(water) = float {
        let at = Vec3::new(at.x, water - FLOAT_DEPTH, at.z);
        v.id = Some(physics.add_vehicle_turned(&v.spec, at, Quat::from_rotation_y(yaw)));
        return true;
    }
    // (Never where it came out NaN: round 88 lost the start boat, and her
    // with it, to a rest fitted to a boat's no wheels.)
    let rest = v
        .spec
        .rest_on(at, yaw, ground)
        .filter(|r| r.position.is_finite() && r.rotation.is_finite());
    // A wheel over nothing (a quay's edge): level, on the ground under
    // its origin.
    let (at, turn) = match rest {
        Some(r) => (r.position, r.rotation),
        None => {
            let Some(h) = ground(at) else {
                return false;
            };
            (
                Vec3::new(at.x, h + v.spec.ride_height(), at.z),
                Quat::from_rotation_y(yaw),
            )
        }
    };
    v.id = Some(physics.add_vehicle_turned(&v.spec, at + Vec3::Y * DROP, turn));
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
        let id = physics.add_vehicle(&v.spec, at, s.yaw);
        v.id = Some(id);
        v.wrecked = s.wrecked;
        if v.wrecked {
            physics.wreck_vehicle(id);
        }
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

/// Where she steps out of a vehicle standing at `at`: on the driver's
/// side, beside the hull.
fn door_out(at: Mat4, spec: &VehicleSpec) -> Vec3 {
    let (lo, hi) = spec.bounds;
    let door = spec
        .door
        .unwrap_or(Vec3::new(lo.x, 0.0, (lo.z + hi.z) * 0.5));
    let x = if door.x < (lo.x + hi.x) * 0.5 {
        lo.x - 0.8
    } else {
        hi.x + 0.8
    };
    at.transform_point3(Vec3::new(x, 0.0, door.z))
}

/// Whether she can get out of `v` where it is: not from a boat over water
/// deeper than she can stand in ([`WADE`]).
pub(crate) fn can_get_out(physics: &Physics, v: &ParkedVehicle) -> bool {
    let (Some(water), Some(id)) = (physics.water(), v.id.filter(|_| v.spec.is_boat())) else {
        return true;
    };
    let Some(at) = physics.vehicle_transform(id) else {
        return true;
    };
    let out = door_out(at, &v.spec);
    let top = Vec3::new(out.x, water + 2.0, out.z);
    physics
        .cast_ray_past(top, Vec3::NEG_Y, 2.0 + WADE, id)
        .is_some()
}

/// The deepest water she gets out of a boat into, metres: chest deep.
const WADE: f32 = 1.4;

/// Gets her out on the driver's side, beside the hull, to drop to the
/// ground there.
pub(crate) fn get_out(physics: &Physics, v: &ParkedVehicle, player: &mut Player) {
    if let Some(at) =
        v.id.and_then(|id| physics.vehicle_transform(id))
            .filter(|at| at.is_finite())
    {
        let out = door_out(at, &v.spec);
        player.character.position = Vec3::new(out.x, 0.0, out.z);
        player.character.velocity = Vec3::ZERO;
        let f = at.transform_vector3(Vec3::Z);
        player.facing = f.x.atan2(f.z);
        player.landed = false;
    }
    player.driving = None;
    player.chase_target = None;
}
