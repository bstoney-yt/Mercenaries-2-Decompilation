//! The game's mission scripts running alongside play (`--mission`): the
//! first contract starts as in the game, the scripts follow the player
//! about, and what they want shown (objectives, markers, messages) is
//! handed to the HUD.
//!
//! The scripts' world is [`m2_game::mission`]'s: the layers' entities by
//! name, with mission layers streamed in and out as the scripts ask.
//! Objective texts are the game's own English strings (read from `vz.wad`
//! by [`m2_game::mission::GameData`]). Soldiers of the island killed in
//! play die in the scripts' world too.
//! Ground vehicles of theirs near the player (dropped jeeps, a tank) are
//! loaded and placed where they say, to be driven; which model a vehicle
//! template is (`"M151 Softtop (VZ)"`) comes from the template database,
//! or is guessed from the models' texture names (`vz_veh_truck_m151`).
//! The models of the state layers they load (a mission's gates) stand
//! while the layer is in, and break when the scripts' object dies: a gate
//! to destroy breaks to a blast by it or a vehicle driven into it. Their
//! helicopters, boats and supply drops are not shown yet.
//!
//! The scripts' log goes to `m2-alpha-mission.log` beside the settings.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write as _};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use m2_assets::{WadArchive, name_hash};
use m2_engine::vehicle::DRIVABLE;
use m2_engine::world::member_stem;
use m2_engine::{CatalogueEntry, Install, TemplateInfo, Templates, VehicleKind};
use m2_game::api::Command;
use m2_game::layers::LayerOp;
use m2_game::mission::{GameData, Mission, PLAYER};
use m2_math::glam::Vec3;

use crate::config::Config;
use crate::game::{Game, get_out, place};
use crate::scene::NOT_DRIVABLE;

/// The scripts run ten times a second.
const STEP: f32 = 0.1;
/// The water's surface as the layers' boats and docks show it (8 patrol
/// boats and 4 air boats at exactly -36 m, docks at -36.0 to -36.4).
const WATERLINE: f32 = -36.0;
/// How close a blast destroys a target the scripts named (a gate), metres:
/// from its model's box, or from its place when it shows no model.
const BLAST_PAD: f32 = 4.0;
const BLAST_REACH: f32 = 10.0;
/// A target is rammed by driving into its box (padded this much, metres,
/// for the vehicle's front to its middle) at this speed or more (m/s), as
/// fast as it went in the last half second or so.
const RAM_PAD: f32 = 4.0;
const RAM_SPEED: f32 = 6.0;
const SPEED_FADE: f32 = 12.0;
/// Seconds an objective message stays up.
const MESSAGE_TIME: f32 = 6.0;
/// The scripts' vehicles are loaded within this far of the player, metres,
/// and at most this many.
const VEHICLE_REACH: f32 = 300.0;
const MAX_VEHICLES: usize = 16;
/// The scripts put vehicles they don't need yet far under the island
/// (about 40 km down): those below this height aren't placed.
const HIDDEN_BELOW: f32 = -1000.0;
/// Steps between looks for the scripts' vehicles near the player.
const VEHICLE_LOOK: u32 = 10;

/// What the HUD shows for the mission.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MissionView {
    /// A line about the scripts themselves (loading, failed), if any.
    pub status: Option<String>,
    /// The objectives to do, readable.
    pub objectives: Vec<String>,
    /// Markers over targets: where, what, colour (RGBA).
    pub markers: Vec<(Vec3, String, [u8; 4])>,
    /// The newest objective message: completed (or else added), and text.
    pub message: Option<(bool, String)>,
    /// The contract was just completed (for [`BANNER_TIME`] seconds).
    pub contract_complete: bool,
}

/// Seconds the contract's completion stays up.
const BANNER_TIME: f32 = 10.0;

enum State {
    Loading(Receiver<Result<GameData, String>>),
    Running(Box<Mission>),
    Failed(String),
}

/// The mission scripts and what they want shown.
pub struct MissionLink {
    state: State,
    /// Time not yet given to the scripts.
    lag: f32,
    log: Option<BufWriter<File>>,
    message: Option<(bool, String, f32)>,
    drivers: Vec<(String, String)>,
    /// The scripts put the player somewhere the island has no way to hold
    /// her (in the start's boat, at sea): she goes ashore once the ground
    /// there has loaded. Tries left.
    ashore: u32,
    /// Blasts since the scripts last ran.
    blasts: Vec<Vec3>,
    vehicles: Fleet,
    /// Seconds since the scripts said the contract was completed.
    completed: Option<f32>,
    /// The speed of the vehicle she drives, held a little (m/s).
    speed: f32,
}

/// The scripts' vehicles in play.
#[derive(Default)]
struct Fleet {
    /// Those asked for (loaded or loading).
    requested: HashSet<u32>,
    /// The one the player is in, as last told the scripts.
    riding: Option<u32>,
    /// Steps to the next look for vehicles near her.
    look: u32,
    /// The one she was driving when the scripts moved it: she gets back in
    /// once it is placed again.
    reenter: Option<u32>,
}

impl std::fmt::Debug for MissionLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = match &self.state {
            State::Loading(_) => "loading",
            State::Running(_) => "running",
            State::Failed(_) => "failed",
        };
        f.debug_struct("MissionLink")
            .field("state", &state)
            .finish_non_exhaustive()
    }
}

impl MissionLink {
    /// Starts reading the scripts and layers from `install` in the
    /// background; play goes on meanwhile. `drivers` (a name for errors,
    /// and Lua) run before play starts, as the `mission` example's
    /// `--driver` files do: for testing.
    pub fn start(install: &Install, drivers: Vec<(String, String)>) -> Self {
        let wad = install.data.join("vz.wad");
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let data = File::open(&wad)
                .map_err(|e| format!("{}: {e}", wad.display()))
                .and_then(|f| WadArchive::open(BufReader::new(f)).map_err(|e| e.to_string()))
                .and_then(|mut w| GameData::read(&mut w).map_err(|e| e.to_string()));
            // The game may have quit meanwhile.
            let _ = tx.send(data);
        });
        let path = Config::file_beside("m2-alpha-mission.log");
        let log = match File::create(&path) {
            Ok(f) => {
                eprintln!("mission log: {}", path.display());
                Some(BufWriter::new(f))
            }
            Err(e) => {
                eprintln!("no mission log ({}): {e}", path.display());
                None
            }
        };
        Self {
            state: State::Loading(rx),
            lag: 0.0,
            log,
            message: None,
            drivers,
            ashore: 0,
            blasts: Vec::new(),
            vehicles: Fleet::default(),
            completed: None,
            speed: 0.0,
        }
    }

    fn write(&mut self, lines: impl IntoIterator<Item = String>) {
        if let Some(log) = &mut self.log {
            for line in lines {
                let _ = writeln!(log, "{line}");
            }
            let _ = log.flush();
        }
    }

    /// Advances the scripts by `dt` seconds of play: tells them where the
    /// player is and who died, and carries out what they ask of the
    /// player.
    pub fn tick(&mut self, game: &mut Game, dt: f32) {
        if let Some((_, _, age)) = &mut self.message {
            *age += dt;
        }
        if let Some(age) = &mut self.completed {
            *age += dt;
        }
        if let State::Loading(rx) = &self.state {
            match rx.try_recv() {
                Ok(Ok(data)) => self.begin(data, game),
                Ok(Err(e)) => self.fail(format!("could not read the scripts: {e}")),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => self.fail("script reader stopped".into()),
            }
            // Without the scripts, every state layer's people stand.
            if matches!(self.state, State::Failed(_)) {
                game.scene.people.state_layers = None;
            }
        }
        let State::Running(mission) = &mut self.state else {
            return;
        };
        if self.ashore > 0 && game.scene.player.as_ref().is_some_and(|p| p.landed) {
            self.ashore -= 1;
            // Thousands of rays a look: twice a second is plenty.
            if self.ashore.is_multiple_of(30) && go_ashore(game) {
                self.ashore = 0;
            }
        }
        self.blasts.extend(game.combat.explosions());
        self.lag += dt;
        let mut lines = Vec::new();
        while self.lag >= STEP {
            self.lag -= STEP;
            // Driving, she is where her vehicle is.
            let at = game
                .driving()
                .map(|(at, _)| at)
                .or_else(|| game.scene.player.as_ref().map(|p| p.character.position));
            if let Some(at) = at
                && self.vehicles.reenter.is_none()
            {
                mission.put_player(at.to_array());
            }
            self.vehicles.sync(game, mission);
            for guid in game.scene.people.all_killed() {
                mission.kill(guid);
            }
            // Targets to destroy: soldiers stand at those that are spawns,
            // and blasts close to the rest (gates, buildings) destroy them.
            let targets: Vec<(u32, Vec3)> = {
                let w = mission.world().borrow();
                w.hud
                    .markers
                    .values()
                    .filter(|m| m.icon.contains("destroy"))
                    .filter_map(|m| {
                        let g = m.target?;
                        let o = w.objects.get(&g).filter(|o| o.alive)?;
                        Some((g, Vec3::from(o.position)))
                    })
                    .collect()
            };
            game.scene.people.mission_soldiers = targets.iter().map(|t| t.0).collect();
            // A vehicle driven hard into a target (a gate) breaks it too:
            // by its speed of late, as the gate stops it.
            let driving = game.driving();
            let speed = driving.map_or(0.0, |(_, v)| v.abs());
            self.speed = speed.max(self.speed - SPEED_FADE * STEP);
            let ram = driving
                .filter(|_| self.speed >= RAM_SPEED)
                .map(|(at, _)| at);
            for (g, at) in &targets {
                let props = &game.scene.props;
                let hit = |p: Vec3, pad: f32| {
                    props
                        .touches(*g, p, pad)
                        .unwrap_or_else(|| p.distance(*at) <= BLAST_REACH)
                };
                if self.blasts.iter().any(|&b| hit(b, BLAST_PAD))
                    || ram.is_some_and(|r| props.touches(*g, r, RAM_PAD) == Some(true))
                {
                    mission.kill(*g);
                }
            }
            self.blasts.clear();
            mission.tick(f64::from(STEP));
            lines.extend(mission.take_log());
            for c in mission.take_commands() {
                if obey(game, mission, &mut self.vehicles, &c) {
                    self.ashore = ASHORE_TRIES;
                }
                if c == Command::Notify("Pg.ContractCompleted".into()) {
                    self.completed = Some(0.0);
                }
            }
            break_props(game, mission);
            let w = mission.world().borrow();
            if let Some((done, text)) = w.hud.messages.last() {
                self.message = Some((*done, mission.text(text), 0.0));
            }
            drop(w);
            mission.world().borrow_mut().hud.messages.clear();
        }
        self.write(lines);
    }

    fn begin(&mut self, data: GameData, game: &mut Game) {
        let at = game
            .scene
            .player
            .as_ref()
            .map_or([0.0; 3], |p| p.character.position.to_array());
        match Mission::new(data, "Mattias", at) {
            Ok(mut mission) => {
                for (name, code) in std::mem::take(&mut self.drivers) {
                    if let Err(e) = mission.run(&code, &name) {
                        self.write([format!("{name}: {e}")]);
                    }
                }
                mission.start();
                let lines = mission.take_log();
                for c in mission.take_commands() {
                    if obey(game, &mission, &mut self.vehicles, &c) {
                        self.ashore = ASHORE_TRIES;
                    }
                }
                self.write(lines);
                self.state = State::Running(Box::new(mission));
            }
            Err(e) => self.fail(format!("could not start the scripts: {e}")),
        }
    }

    fn fail(&mut self, why: String) {
        eprintln!("mission: {why}");
        self.write([why.clone()]);
        self.state = State::Failed(why);
    }

    /// The scripts are running (not still loading, not failed).
    pub fn running(&self) -> bool {
        matches!(self.state, State::Running(_))
    }

    /// The scripts' vehicles in play: label, and where it stands if
    /// placed.
    pub fn vehicles(game: &Game) -> Vec<(String, Option<Vec3>)> {
        let physics = game.scene.world.as_ref().map(|w| &w.physics);
        game.scene
            .vehicles
            .iter()
            .filter(|v| v.guid.is_some())
            .map(|v| {
                let at =
                    v.id.zip(physics)
                        .and_then(|(id, p)| p.vehicle_transform(id))
                        .map(|m| m.transform_point3(Vec3::ZERO));
                (v.spec.label.clone(), at)
            })
            .collect()
    }

    /// A readable account of where the mission stands, for screenshot
    /// runs: the state layers loaded, and each marker with what stands at
    /// it (a soldier of the island, one of the scripts' vehicles).
    pub fn report(&self, game: &Game) -> Vec<String> {
        let State::Running(mission) = &self.state else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let layers: Vec<String> = mission
            .loaded_layers()
            .into_iter()
            .filter(|n| !m2_game::layers::LayerState::is_static(n))
            .collect();
        out.push(format!("state layers loaded: {}", layers.join(", ")));
        let at = game.focus();
        let w = mission.world().borrow();
        let spawns = game
            .scene
            .world
            .as_ref()
            .map(|w| w.index.spawns.as_slice())
            .unwrap_or_default();
        for m in w.hud.markers.values() {
            let Some(g) = m.target else { continue };
            let Some(o) = w.objects.get(&g) else {
                out.push(format!("marker {} on {g:#x}: no such object", m.icon));
                continue;
            };
            let p = Vec3::from(o.position);
            let mut what = Vec::new();
            if let Some(s) = spawns.iter().find(|s| s.entity == g) {
                what.push(format!(
                    "spawn {:?} {:.1} m off",
                    s.name,
                    s.position.distance(p)
                ));
            }
            if let Some(n) = game.scene.people.npcs.iter().find(|n| n.spawn == Some(g)) {
                what.push(format!(
                    "soldier {} at {:.1}",
                    if n.alive { "alive" } else { "dead" },
                    n.character.position
                ));
            }
            if game.scene.people.all_killed().contains(&g) {
                what.push("killed".into());
            }
            if let Some((prop, loaded)) = game.scene.props.get(g) {
                what.push(format!(
                    "model {:08x} at {:.1}{}{}",
                    prop.model,
                    prop.position,
                    if loaded { "" } else { " (loading)" },
                    if prop.wrecked { ", wrecked" } else { "" }
                ));
            }
            if let Some(v) = game.scene.vehicles.iter().find(|v| v.guid == Some(g)) {
                what.push(format!(
                    "vehicle {} {}",
                    v.spec.label,
                    if v.id.is_some() {
                        "placed"
                    } else {
                        "not placed"
                    }
                ));
            }
            out.push(format!(
                "marker {} on {:?} ({g:#x}) at {p:.1}, {:.0} m away, {}: {}",
                m.icon.trim_start_matches("HUD_objective_"),
                o.name,
                p.distance(at),
                if o.alive { "alive" } else { "dead" },
                if what.is_empty() {
                    "nothing of the island's there".to_owned()
                } else {
                    what.join("; ")
                }
            ));
        }
        out
    }

    /// What the HUD shows.
    pub fn view(&self) -> MissionView {
        let message = self
            .message
            .as_ref()
            .filter(|m| m.2 < MESSAGE_TIME)
            .map(|(done, text, _)| (*done, text.clone()));
        let mission = match &self.state {
            State::Loading(_) => {
                return MissionView {
                    status: Some("Reading the mission scripts...".into()),
                    ..MissionView::default()
                };
            }
            State::Failed(why) => {
                return MissionView {
                    status: Some(format!("Mission scripts stopped: {why}")),
                    ..MissionView::default()
                };
            }
            State::Running(m) => m,
        };
        let w = mission.world().borrow();
        let hud = &w.hud;
        // The objectives on the map, else the mission's list.
        let mut objectives: Vec<String> = hud
            .objectives
            .iter()
            .filter(|(name, _)| !name.ends_with("_PreMission"))
            .map(|(_, o)| mission.text(&o.text))
            .collect();
        if objectives.is_empty() {
            objectives = hud.mission.iter().map(|(t, _)| mission.text(t)).collect();
        }
        objectives.dedup();
        let markers = hud
            .markers
            .values()
            .filter_map(|m| {
                let at = w.objects.get(&m.target?)?.position;
                let label = m
                    .icon
                    .trim_start_matches("HUD_objective_")
                    .replace('_', " ");
                Some((Vec3::from(at) + Vec3::Y * m.height, label, m.colour))
            })
            .collect();
        MissionView {
            status: None,
            objectives,
            markers,
            message,
            contract_complete: self.completed.is_some_and(|age| age < BANNER_TIME),
        }
    }
}

impl Fleet {
    /// Keeps the scripts' vehicles and the island's in step: loads those
    /// near the player, places them where the scripts say, tells the
    /// scripts where they went, which one she is in, and which are wrecked.
    fn sync(&mut self, game: &mut Game, mission: &mut Mission) {
        let Some(world) = &mut game.scene.world else {
            return;
        };
        let physics = &mut world.physics;
        let riding = game
            .scene
            .player
            .as_ref()
            .and_then(|p| p.driving)
            .and_then(|i| game.scene.vehicles.get(i)?.guid);
        if riding != self.riding && self.reenter.is_none() {
            mission.ride(riding);
            self.riding = riding;
        }
        {
            let w = mission.world().borrow();
            for (i, v) in game.scene.vehicles.iter_mut().enumerate() {
                let Some(g) = v.guid else { continue };
                // Placed once the scripts have let go of it (a winch) and
                // the ground under it has loaded.
                if v.id.is_none()
                    && let Some(o) = w
                        .objects
                        .get(&g)
                        .filter(|o| o.alive && o.position[1] > HIDDEN_BELOW)
                    && !w.winch.get(&g).is_some_and(Option::is_some)
                    && place(physics, v, Vec3::from(o.position), o.yaw)
                    && self.reenter == Some(g)
                    && let Some(p) = game.scene.player.as_mut()
                {
                    p.driving = Some(i);
                    self.reenter = None;
                }
            }
        }
        for v in &game.scene.vehicles {
            let (Some(g), Some(id)) = (v.guid, v.id) else {
                continue;
            };
            if v.wrecked {
                mission.kill(g);
            }
            if let Some(m) = physics.vehicle_transform(id) {
                let f = m.transform_vector3(Vec3::Z);
                mission.place(g, m.transform_point3(Vec3::ZERO).to_array(), f.x.atan2(f.z));
            }
        }
        if self.look > 0 {
            self.look -= 1;
            return;
        }
        self.look = VEHICLE_LOOK;
        let (Some(catalogue), Some(player)) = (&game.scene.catalogue, &game.scene.player) else {
            return;
        };
        let near = player.character.position;
        let none = Templates::default();
        let templates = game
            .scene
            .world
            .as_ref()
            .map_or(&none, |w| &w.index.template_db);
        let w = mission.world().borrow();
        let mut wanted: Vec<(f32, u32, Vec<CatalogueEntry>)> = w
            .objects
            .iter()
            .filter(|(g, o)| o.alive && !self.requested.contains(g))
            .filter_map(|(&g, o)| {
                let d = Vec3::from(o.position).distance(near);
                (d <= VEHICLE_REACH).then_some((d, g, o))
            })
            .filter_map(|(d, g, o)| {
                let models = template_models(&o.name, templates, &catalogue.vehicles);
                (!models.is_empty()).then_some((d, g, models))
            })
            .collect();
        drop(w);
        wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, g, models) in wanted {
            if self.requested.len() >= MAX_VEHICLES {
                break;
            }
            self.requested.insert(g);
            game.scene.load_vehicle(g, models);
        }
    }
}

/// Ticks the player gets to go ashore: about ten seconds for the ground
/// around her to load.
const ASHORE_TRIES: u32 = 600;

/// Moves a player standing under the sea to the nearest dry ground within
/// 1.5 km. True once she stands on dry ground (moved there, or there
/// already); false while none around her has loaded.
fn go_ashore(game: &mut Game) -> bool {
    let (Some(world), Some(player)) = (&game.scene.world, &mut game.scene.player) else {
        return true;
    };
    // Boats and docks the layers place float at -36 m: dry ground is
    // above that, whatever height the sea is drawn at.
    let sea = world.settings.sea_level.max(WATERLINE);
    let from = player.character.position;
    if from.y > sea {
        return true;
    }
    let ground = |x: f32, z: f32| {
        let top = 600.0;
        let (t, normal) = world
            .physics
            .cast_ray(Vec3::new(x, top, z), Vec3::NEG_Y, 1200.0)?;
        let y = top - t;
        (y > sea + 0.5 && normal.y > 0.7).then_some(Vec3::new(x, y + 1.0, z))
    };
    for r in (1..=60).map(|i| i as f32 * 25.0) {
        let n = ((r / 10.0) as usize).max(8);
        for k in 0..n {
            let a = k as f32 / n as f32 * std::f32::consts::TAU;
            if let Some(to) = ground(from.x + r * a.cos(), from.z + r * a.sin()) {
                player.character.position = to;
                player.character.velocity = Vec3::ZERO;
                player.landed = false;
                return true;
            }
        }
    }
    false
}

/// Whether `guid` is a layer entity that places a person.
fn is_spawn(world: &m2_engine::World, guid: u32) -> bool {
    world.index.spawns.iter().any(|s| s.entity == guid)
}

/// Breaks the state layers' models whose objects the scripts' world has
/// dead (a gate blown up), and takes down those it no longer has.
fn break_props(game: &mut Game, mission: &Mission) {
    let scene = &mut game.scene;
    let Some(world) = &mut scene.world else {
        return;
    };
    let w = mission.world().borrow();
    let (dead, gone): (Vec<u32>, Vec<u32>) = {
        let mut dead = Vec::new();
        let mut gone = Vec::new();
        for (e, p) in scene.props.shown() {
            match w.objects.get(&e) {
                Some(o) if !o.alive && !p.wrecked => dead.push(e),
                None => gone.push(e),
                _ => {}
            }
        }
        (dead, gone)
    };
    drop(w);
    for e in dead {
        scene.props.wreck(&mut world.physics, e);
    }
    scene.props.hide(&mut world.physics, gone);
}

/// Carries out a script command about the player, the scripts' vehicles
/// or the layers they stream; true when it seated her in a vehicle the
/// island doesn't show, so she stands where it is.
fn obey(game: &mut Game, mission: &Mission, fleet: &mut Fleet, c: &Command) -> bool {
    let index = |guid: &u32| {
        game.scene
            .vehicles
            .iter()
            .position(|v| v.guid == Some(*guid))
    };
    match c {
        // Seated in one of the scripts' vehicles that is placed: drive it.
        Command::EnterVehicle { guid, vehicle, .. } if *guid == PLAYER => {
            if let Some(i) = index(vehicle).filter(|&i| game.scene.vehicles[i].id.is_some())
                && let Some(p) = game.scene.player.as_mut()
            {
                p.driving = Some(i);
                return false;
            }
        }
        Command::ExitVehicle(guid) if *guid == PLAYER => {
            fleet.reenter = None;
            let (Some(world), Some(p)) = (&game.scene.world, game.scene.player.as_mut()) else {
                return false;
            };
            if let Some(v) = p.driving.and_then(|i| game.scene.vehicles.get(i)) {
                get_out(&world.physics, v, p);
            }
            return false;
        }
        // Moved by the scripts: a vehicle is placed afresh where they now
        // say, with her back in it if she was driving it; a layer's model
        // just moves.
        Command::Move { guid, .. } | Command::Remove(guid) if *guid != PLAYER => {
            if let (Some(i), Some(world)) = (index(guid), &mut game.scene.world)
                && let Some(id) = game.scene.vehicles[i].id.take()
            {
                world.physics.remove_vehicle(id);
                let driving = game.scene.player.as_ref().and_then(|p| p.driving);
                if matches!(c, Command::Move { .. }) && driving == Some(i) {
                    fleet.reenter = Some(*guid);
                }
            }
            let scene = &mut game.scene;
            if let Some(world) = &mut scene.world {
                match c {
                    Command::Move { position, .. } => {
                        scene
                            .props
                            .put(&mut world.physics, *guid, Vec3::from(*position));
                    }
                    _ if is_spawn(world, *guid) => {
                        scene.people.script_kill(&mut world.physics, *guid, true);
                    }
                    _ => {}
                }
            }
            return false;
        }
        Command::Kill(guid) => {
            if let Some(i) = index(guid) {
                game.scene.vehicles[i].wrecked = true;
            }
            let scene = &mut game.scene;
            if let Some(world) = &mut scene.world
                && is_spawn(world, *guid)
            {
                scene.people.script_kill(&mut world.physics, *guid, false);
            }
            return false;
        }
        // A state layer in or out: its models and people with it.
        Command::Layer { op, name } => {
            let entities: Vec<u32> = mission
                .layer(name)
                .map(|es| es.iter().map(|e| e.guid).collect())
                .unwrap_or_default();
            let scene = &mut game.scene;
            if let Some(set) = &mut scene.people.state_layers {
                if *op == LayerOp::Unload {
                    set.remove(&name_hash(name));
                } else {
                    set.insert(name_hash(name));
                }
            }
            if let Some(world) = &mut scene.world
                && *op != LayerOp::Load
            {
                scene
                    .props
                    .hide(&mut world.physics, entities.iter().copied());
            }
            if *op != LayerOp::Unload {
                let shown = scene.show_props(entities);
                if shown > 0 {
                    eprintln!("layer {name}: {shown} models put up");
                }
            }
            return false;
        }
        _ => {}
    }
    let to = match c {
        Command::Move { guid, position } if *guid == PLAYER => Some(*position),
        // Seated in a vehicle the island doesn't show (the start's boat):
        // stand where it is.
        Command::EnterVehicle { guid, vehicle, .. } if *guid == PLAYER => mission
            .world()
            .borrow()
            .objects
            .get(vehicle)
            .map(|o| o.position),
        _ => None,
    };
    let Some(to) = to else { return false };
    if let Some(p) = game.scene.player.as_mut() {
        p.character.position = Vec3::from(to);
        p.character.velocity = Vec3::ZERO;
        p.landed = false;
        p.driving = None;
        p.chase_target = None;
    }
    matches!(c, Command::EnterVehicle { .. })
}

/// The model a vehicle template of the scripts' drives as: the template
/// database's, when it has the template and it drives on the ground
/// (boats and helicopters don't drive yet); else the catalogue models
/// whose name fits the template's words ([`vehicle_models`]).
pub(crate) fn template_models(
    name: &str,
    templates: &Templates,
    catalogue: &[CatalogueEntry],
) -> Vec<CatalogueEntry> {
    match templates.get(name) {
        Some(&TemplateInfo {
            model: Some(model),
            vehicle: Some(kind @ (VehicleKind::Car | VehicleKind::Tank)),
            ..
        }) => vec![CatalogueEntry {
            model,
            kind: if kind == VehicleKind::Tank {
                "tank"
            } else {
                "car"
            }
            .to_owned(),
            label: member_stem(name).to_owned(),
        }],
        Some(&TemplateInfo {
            vehicle: Some(VehicleKind::Boat | VehicleKind::Helicopter),
            ..
        }) => Vec::new(),
        _ => vehicle_models(name, catalogue),
    }
}

/// Catalogue models that could be a vehicle template of the scripts', best
/// first: those whose model name (the texture name's last part,
/// `m151` of `vz_veh_truck_m151`) is a word of the template's name (or
/// starts like it, `alouette` of `Alouette3`), its faction's first. Only
/// ground vehicles to drive. `name` is an object's (`"M151 Softtop (VZ)
/// 0x0012abcd"`) or a template's.
pub(crate) fn vehicle_models(name: &str, catalogue: &[CatalogueEntry]) -> Vec<CatalogueEntry> {
    let template = name
        .rfind(" 0x")
        .map_or(name, |i| &name[..i])
        .to_ascii_lowercase();
    let faction = [
        ("(vz)", "vz_"),
        ("(gl)", "gr_"),
        ("(gr)", "gr_"),
        ("(ch)", "ch_"),
        ("(al)", "al_"),
        ("(pmc)", "pmc_"),
        ("(civ)", "civ_"),
    ]
    .iter()
    .find(|(tag, _)| template.contains(tag))
    .map_or("vz_", |(_, prefix)| prefix);
    // The name without its (tags).
    let mut plain = String::new();
    let mut depth = 0;
    for c in template.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ if depth == 0 => plain.push(c),
            _ => {}
        }
    }
    let words: Vec<&str> = plain
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() >= 3)
        .collect();
    let fits = |part: &str| {
        words.iter().any(|w| {
            let stem = w.trim_end_matches(|c: char| c.is_ascii_digit());
            part == *w || (stem.len() >= 4 && stem.len() < w.len() && part.starts_with(stem))
        })
    };
    let mut out: Vec<CatalogueEntry> = catalogue
        .iter()
        .filter(|e| {
            DRIVABLE.contains(&e.kind.as_str())
                || (e.label.starts_with("global_") && !NOT_DRIVABLE.contains(&e.kind.as_str()))
        })
        .filter(|e| e.label.rsplit('_').next().is_some_and(fits))
        .cloned()
        .collect();
    out.sort_by_key(|e| !e.label.starts_with(faction));
    out
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn entry(model: u32, kind: &str, label: &str) -> CatalogueEntry {
        CatalogueEntry {
            model,
            kind: kind.into(),
            label: label.into(),
        }
    }

    #[test]
    fn templates_find_their_vehicles() {
        let catalogue = [
            entry(1, "truck", "gr_veh_truck_m151"),
            entry(2, "truck", "vz_veh_truck_m151"),
            entry(3, "tank", "vz_veh_tank_amx30"),
            entry(4, "helicopter", "vz_veh_helicopter_alouetteiii"),
            entry(5, "transport", "global_veh_transport"),
            entry(6, "car", "civ_veh_car_van"),
        ];
        let models = |n: &str| -> Vec<u32> {
            vehicle_models(n, &catalogue)
                .iter()
                .map(|e| e.model)
                .collect()
        };
        assert_eq!(models("M151 Softtop (VZ) 0x0012abcd"), [2, 1]);
        assert_eq!(models("M151 Softtop (GL)"), [1, 2]);
        assert_eq!(models("AMX30"), [3]);
        // Not ground vehicles to drive.
        assert!(models("Alouette3 Transport (VZA Intro) (Driver)").is_empty());
        assert!(models("VZ Soldier 0x00000001").is_empty());
        assert!(models("Supply Drop (GL)").is_empty());
    }

    #[test]
    fn the_template_database_names_the_model_first() {
        let catalogue = [entry(1, "truck", "gr_veh_truck_m151")];
        let info = |model, vehicle| TemplateInfo {
            model,
            vehicle,
            engine: None,
            person: false,
            human: false,
        };
        let db = Templates {
            by_name: HashMap::from([
                (
                    "m151 softtop (vz)".to_owned(),
                    info(Some(0xabc), Some(VehicleKind::Car)),
                ),
                (
                    "amx30 (full)".to_owned(),
                    info(Some(0xdef), Some(VehicleKind::Tank)),
                ),
                (
                    "patrol boat (vz)".to_owned(),
                    info(Some(0x123), Some(VehicleKind::Boat)),
                ),
                (
                    "m151 softtop (gl)".to_owned(),
                    info(None, Some(VehicleKind::Car)),
                ),
            ]),
            names: HashMap::new(),
        };
        let models = |n: &str| -> Vec<(u32, String)> {
            template_models(n, &db, &catalogue)
                .into_iter()
                .map(|e| (e.model, e.kind))
                .collect()
        };
        // The database's model, whatever the case and the entity's id.
        assert_eq!(
            models("M151 Softtop (VZ) 0x0012abcd"),
            [(0xabc, "car".to_owned())]
        );
        assert_eq!(models("AMX30 (Full)"), [(0xdef, "tank".to_owned())]);
        // A boat doesn't drive yet.
        assert!(models("Patrol Boat (VZ)").is_empty());
        // Without a model in the database, the words find the catalogue's.
        assert_eq!(models("M151 Softtop (GL)"), [(1, "truck".to_owned())]);
    }
}
