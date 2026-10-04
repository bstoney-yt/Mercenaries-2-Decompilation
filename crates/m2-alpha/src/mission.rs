//! The game's mission scripts running alongside play (`--mission`): the
//! first contract starts as in the game, the scripts follow the player
//! about, and what they want shown (objectives, markers, messages) is
//! handed to the HUD.
//!
//! The scripts' world is [`m2_game::mission`]'s: the layers' entities by
//! name, with mission layers streamed in and out as the scripts ask. The
//! island the player sees doesn't show those layers' people and vehicles
//! yet, so objectives about them (guards to kill, dropped vehicles) can't
//! be done in play; places to reach can. Soldiers of the island killed in
//! play die in the scripts' world too.
//!
//! The scripts' log goes to `m2-alpha-mission.log` beside the settings.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write as _};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use m2_assets::WadArchive;
use m2_engine::Install;
use m2_game::api::Command;
use m2_game::hud::readable;
use m2_game::mission::{GameData, Mission, PLAYER};
use m2_math::glam::Vec3;

use crate::config::Config;
use crate::game::Game;

/// The scripts run ten times a second.
const STEP: f32 = 0.1;
/// How close a blast destroys a target the scripts named (a gate), metres.
const BLAST_REACH: f32 = 10.0;
/// Seconds an objective message stays up.
const MESSAGE_TIME: f32 = 6.0;

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
}

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
    /// The scripts put the player somewhere the island has no way to hold
    /// her (in the start's boat, at sea): she goes ashore once the ground
    /// there has loaded. Tries left.
    ashore: u32,
    /// Blasts since the scripts last ran.
    blasts: Vec<Vec3>,
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
    /// background; play goes on meanwhile.
    pub fn start(install: &Install) -> Self {
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
            ashore: 0,
            blasts: Vec::new(),
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
        if let State::Loading(rx) = &self.state {
            match rx.try_recv() {
                Ok(Ok(data)) => self.begin(data, game),
                Ok(Err(e)) => self.fail(format!("could not read the scripts: {e}")),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => self.fail("script reader stopped".into()),
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
            if let Some(p) = game.scene.player.as_ref() {
                mission.put_player(p.character.position.to_array());
            }
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
            for (g, at) in &targets {
                if self.blasts.iter().any(|b| b.distance(*at) <= BLAST_REACH) {
                    mission.kill(*g);
                }
            }
            self.blasts.clear();
            mission.tick(f64::from(STEP));
            lines.extend(mission.take_log());
            for c in mission.take_commands() {
                if obey(game, mission, &c) {
                    self.ashore = ASHORE_TRIES;
                }
            }
            let w = mission.world().borrow();
            if let Some((done, text)) = w.hud.messages.last() {
                self.message = Some((*done, readable(text), 0.0));
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
                mission.start();
                let lines = mission.take_log();
                for c in mission.take_commands() {
                    if obey(game, &mission, &c) {
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
            .map(|(_, o)| readable(&o.text))
            .collect();
        if objectives.is_empty() {
            objectives = hud.mission.iter().map(|(t, _)| readable(t)).collect();
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
    let sea = world.settings.sea_level;
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

/// Carries out a script command that is about the player; true when it
/// seated her in a vehicle the island doesn't show, so she stands where
/// it is.
fn obey(game: &mut Game, mission: &Mission, c: &Command) -> bool {
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
