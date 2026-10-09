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
//! while the layer is in, and break when the scripts' object dies. A
//! target to destroy that is such a model (the first contract's gates)
//! falls only to a support strike: the videos' gates stood through
//! grenade-launcher hits and fell to the Satellite Guided Bomb, one of
//! which is given her free while such a target is marked
//! ([`crate::support`]); others to a blast by them or a vehicle driven
//! into them. Their
//! deliveries come by helicopter ([`crate::flights`]): the crate, the jeep
//! and the tank hang under it until it lets go over the drop point. Their
//! boats are not shown yet.
//!
//! The scripts' log goes to `m2-alpha-mission.log` beside the settings.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, BufWriter, Write as _};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use m2_assets::{WadArchive, name_hash};
use m2_engine::vehicle::DRIVABLE;
use m2_engine::world::member_stem;
use m2_engine::{CatalogueEntry, Install, TemplateInfo, Templates, VehicleKind};
use m2_game::api::Command;
use m2_game::hud::ObjectiveMessage;
use m2_game::layers::LayerOp;
use m2_game::mission::{GameData, Mission, PLAYER};
use m2_math::glam::Vec3;

use crate::combat::{self, Weapon};
use crate::config::Config;
use crate::flights::{Copter, Leaves, Load};
use crate::game::{Game, get_out, place};
use crate::gamehud::{HudCall, Plain, RadarRegion};
use crate::hero::Hero;
use crate::scene::NOT_DRIVABLE;
use crate::support::{self, FALLBACK_AFTER, Impact, Kind};

/// The scripts run ten times a second.
const STEP: f32 = 0.1;
/// A scripted voice line to play: the mission's id for it, its cue, and
/// where it is said (`None`: flat).
pub type ScriptLine = (u32, String, Option<Vec3>);

/// The grenades' weapon template.
const GRENADE: &str = "Grenade";
/// What she throws to mark a support call: the scripts' support manager
/// runs on it (its `ObjectScript`), and it marks the spot when its timer
/// runs out (`TriggerOnTimer`; 4 s if the template can't be read).
const MARKER: &str = "Smoke Grenade Projectile";
const MARKER_TIMER: f32 = 4.0;
/// `--support` calls come this long into the scripts' run (seconds), once
/// she is on foot on land, with this much cash and fuel: enough for any
/// item.
const SUPPORT_AFTER: f64 = 2.0;
const SUPPORT_CASH: f64 = 1.0e7;
const SUPPORT_FUEL: f64 = 1000.0;
/// She is ashore this far above the water (metres): not seated in the
/// start's boat, which the scripts don't count as driving.
const ASHORE_HEIGHT: f32 = 2.0;
/// ... for this long without a break (seconds): she stands ashore a moment
/// at the start, before the scripts seat her in the boat.
const SUPPORT_ASHORE: f32 = 10.0;
/// Her rifle's template in the first contract.
const CARBINE: &str = "Carbine";

/// The water's surface as the layers' boats and docks show it (8 patrol
/// boats and 4 air boats at exactly -36 m, docks at -36.0 to -36.4).
pub(crate) const WATERLINE: f32 = -36.0;
/// How close a blast destroys a target the scripts named, metres: from
/// its model's box, or from its place when it shows no model; a support
/// strike's bomb, its blast's reach.
const BLAST_PAD: f32 = 4.0;
const BLAST_REACH: f32 = 10.0;
const BOMB_PAD: f32 = crate::combat::BOMB.0;
/// A target is rammed by driving into its box (padded this much, metres,
/// for the vehicle's front to its middle) at this speed or more (m/s), as
/// fast as it went in the last half second or so.
const RAM_PAD: f32 = 4.0;
const RAM_SPEED: f32 = 6.0;
const SPEED_FADE: f32 = 12.0;
/// Seconds an objective message stays up. A tutorial line stays until
/// the scripts take it down or put up another (their tutorial manager
/// shows one with no time of its own: round 96).
const MESSAGE_TIME: f32 = 6.0;
/// The scripts' vehicles are loaded within this far of the player, metres,
/// and at most this many in play at once (wrecks and those they removed
/// don't count).
const VEHICLE_REACH: f32 = 300.0;
const MAX_VEHICLES: usize = 24;
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
    /// How black the scripts' screen fade has it, 0 to 1 (as she is
    /// moved to the next contract).
    pub black: f32,
    /// The objectives to do, readable.
    pub objectives: Vec<String>,
    /// The tutorial line up, readable.
    pub tutorial: Option<String>,
    /// The same line with its key tokens (`[pda]`) as the scripts' text
    /// has them, for the game's own tutorial box to draw as glyphs.
    pub game_tutorial: Option<String>,
    /// Markers over targets: where, what, colour (RGBA).
    pub markers: Vec<(Vec3, String, [u8; 4])>,
    /// The newest objective message: completed (or else added), and text.
    pub message: Option<(bool, String)>,
    /// The same message as the scripts sent it, for the game's own HUD to
    /// word.
    pub game_message: Option<ObjectiveMessage>,
    /// The contract was just completed (for [`BANNER_TIME`] seconds).
    pub contract_complete: bool,
    /// The regions the scripts have on the radar (see [`RADAR_REGIONS`]).
    pub radar_regions: Vec<RadarRegion>,
    /// The scripts' latest calls to the HUD's interface (see [`HUD_CALLS`]).
    pub hud_calls: Vec<HudCall>,
    /// The contract's objective list for the PDA as the scripts send it
    /// (`Net.SendEvent_AddPDAMission`): text id and icon tag each.
    pub pda_objectives: Vec<(String, String)>,
    /// Whether she is trespassing in a faction's zone, and whose (`"Vz"`),
    /// as the zone's script last told the HUD (see [`TRESPASS`]).
    pub trespass: Trespass,
    /// Whether vehicle disguises work, as the scripts set it.
    pub vehicle_disguise: bool,
    /// Her disguise while one holds: how much is left and whether it holds.
    pub disguise: Option<(f32, bool)>,
}

/// What `FactionZone` last told the HUD (`TrespassStateChange`): whether
/// she is trespassing, and the zone's faction as the HUD's radar names it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trespass {
    pub inside: bool,
    pub faction: Option<String>,
}

/// Seconds the contract's completion stays up.
const BANNER_TIME: f32 = 10.0;

/// The regions the scripts put on the radar (`Hud.Radar:AddLineRegion`:
/// the world's boundary, shaded outside, as `WifVzBoundary` draws it),
/// kept by the region's name in `__m2_radar_regions` for the game HUD,
/// which runs the radar in a Lua state of its own. Here the call reaches
/// no radar (there are no widgets in this state) and goes on as it would.
const RADAR_REGIONS: &str = r#"
__m2_radar_regions = {}
local radar = Hud and Hud.Radar
if radar then
    local add, remove = radar.AddLineRegion, radar.RemoveLineRegion
    radar.AddLineRegion = function(self, t)
        local name = t and t.uGuid and Object.GetName(t.uGuid)
        if name then
            __m2_radar_regions[name] = {
                nRed = t.nRed, nGreen = t.nGreen, nBlue = t.nBlue,
                nAlpha = t.nAlpha, bInvert = t.bInvert and true or false,
            }
        end
        if add then return add(self, t) end
    end
    radar.RemoveLineRegion = function(self, t)
        local name = t and t.uGuid and Object.GetName(t.uGuid)
        if name then __m2_radar_regions[name] = nil end
        if remove then return remove(self, t) end
    end
end
"#;

/// The faction zones' word to the HUD (`MrxGui.SendEvent{ EventType =
/// "TrespassStateChange", bTrespassing, sFaction }`): the game HUD runs in
/// a Lua state of its own, so the latest is kept in `__m2_trespass` for it.
/// The event then goes on as it would.
const TRESPASS: &str = r#"
__m2_trespass = { bTrespassing = false }
local gui = rawget(_G, "MrxGui")
if type(gui) == "table" and type(gui.SendEvent) == "function" then
    local send = gui.SendEvent
    gui.SendEvent = function(t, ...)
        if type(t) == "table" and t.EventType == "TrespassStateChange" then
            __m2_trespass = { bTrespassing = t.bTrespassing and true or false,
                sFaction = t.sFaction }
        end
        return send(t, ...)
    end
end
"#;

/// The scripts' calls to the HUD's interface through the engine's `Hud`
/// library (`Hud.Fanfare:Create`, `:AddItem`, `:Commence` as a contract
/// ends...), which the engine hands to the HUD's `HudInterface`: kept,
/// with their data copied as it was, in `__m2_hud_calls` (the latest 64)
/// for the game HUD, which runs in a Lua state of its
/// own. Each call then goes on as it would. The groups and methods are
/// those `MrxGuiInterface` gives `HudInterface`.
const HUD_CALLS: &str = r#"
__m2_hud_calls = {}
__m2_hud_call_count = 0
-- Calls with a callback, by number, for the HUD to call back.
__m2_hud_callbacks = {}
local function copy(v, depth)
    if type(v) ~= "table" then
        -- Guids (userdata) go across; the HUD's state knows them.
        if type(v) == "function" then return nil end
        return v
    end
    if depth > 3 then return nil end
    local out = {}
    for k, x in pairs(v) do out[k] = copy(x, depth + 1) end
    return out
end
local methods = {
    Fanfare = { "Create", "AddItem", "Commence" },
    SupportFanfare = { "Create", "AddItem", "Commence" },
    ContactFanfare = { "Commence" },
    CardFanfare = { "Commence" },
    JobFanfare = { "Complete", "Failed" },
    EventFanfare = { "Commence" },
    TextFanfare = { "Commence" },
    Announcement = { "Show" },
    MapLabel = { "Show" },
    MessageBox = { "AddMessage", "ModifyPendingMessage", "RemovePendingMessage", "Clear" },
    SubtitleBuffer = { "AddMessage", "RemovePendingMessage", "Clear" },
    ObjectiveTray = { "SetSlotToText", "SetSlotToImage", "SetSlotToWidget", "ClearSlot" },
    ResourceCounter = { "SetCash", "SetFuel", "SetSuppressed", "Show", "Hide" },
    FactionDisplay = { "Show", "SetValue", "SetInsideFactionZone", "ConfigureThresholds",
        "AddMeter", "StartTimer", "StartPursuit", "HideMeter", "RemoveMeter",
        "RemoveAllMeters" },
    Tutorial = { "SetText", "ShowTutorialForObject", "ShowTutorialOnscreen" },
}
for group, names in pairs(methods) do
    local g = Hud and Hud[group]
    if type(g) == "table" then
        for _, name in ipairs(names) do
            local f = g[name]
            g[name] = function(self, t, ...)
                __m2_hud_call_count = __m2_hud_call_count + 1
                local callback = type(t) == "table" and type(t.fCallback) == "function"
                if callback then
                    __m2_hud_callbacks[__m2_hud_call_count] = t
                end
                table.insert(__m2_hud_calls, { seq = __m2_hud_call_count,
                    group = group, method = name, data = copy(t, 0),
                    callback = callback })
                if #__m2_hud_calls > 64 then table.remove(__m2_hud_calls, 1) end
                local r
                if type(f) == "function" then r = f(self, t, ...) end
                -- A message added here gets the call's number as its id,
                -- for the HUD to tell its own message by: what the
                -- scripts keep to take it down again (`MrxSubtitle`'s
                -- `_tMsgIds`, passed back as `tMessageIds`). What the
                -- engine's own `Hud` answers here (a stub) means nothing
                -- to the HUD.
                if name == "AddMessage" then
                    r = { __m2_seq = __m2_hud_call_count }
                end
                return r
            end
        end
    end
end
"#;

/// People this near a marker's target are listed with it, metres; story
/// characters' spawns this near the player.
const MARK_NEAR: f32 = 6.0;
const STORY_REACH: f32 = 450.0;

/// A marker the scripts put up, over one of their objects.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub target: u32,
    /// What to do there: `"destroy"`, `"deliverable"` (get there),
    /// `"action"`; or `"objective"`, `"radar"` for an objective's target
    /// on the map or radar without a marker.
    pub kind: String,
    /// The object's name (`vza_beachguard_1`, `loc_vza_beach`).
    pub name: String,
    pub at: Vec3,
    /// The object is alive (not yet destroyed).
    pub alive: bool,
}

enum State {
    Loading(Receiver<Result<GameData, String>>),
    Running(Box<Mission>),
    Failed(String),
}

/// The mission scripts and what they want shown.
pub struct MissionLink {
    state: State,
    /// The flow's movies wait for the game to play them
    /// ([`Mission::hold_movies`]); otherwise they end at once.
    movies: bool,
    /// The player has subtitles on.
    subtitles: bool,
    /// Time not yet given to the scripts.
    lag: f32,
    log: Option<BufWriter<File>>,
    /// The newest objective message, its text readable, and its age.
    message: Option<(ObjectiveMessage, String, f32)>,
    drivers: Vec<(String, String)>,
    /// The scripts put the player somewhere the island has no way to hold
    /// her (in the start's boat, at sea): she goes ashore once the ground
    /// there has loaded. Tries left.
    ashore: u32,
    /// The tutorial line up, with its key tokens.
    tutorial: Option<String>,
    /// Blasts since the scripts last ran, and support strikes' bombs.
    blasts: Vec<Vec3>,
    bombs: Vec<Vec3>,
    vehicles: Fleet,
    /// Seconds since the scripts said the contract was completed.
    completed: Option<f32>,
    /// The speed of the vehicle she drives, held a little (m/s).
    speed: f32,
    /// The keys tutorial lines name.
    keys: crate::input::Bindings,
    /// Scripted voice lines to play, and lines to stop.
    lines: Vec<ScriptLine>,
    hushed: Vec<u32>,
    /// Interface sounds the scripts cued (`ui_HUD_Objective_New`), and
    /// whether each is to stop instead.
    sounds: Vec<(String, bool)>,
    /// Sound category fades and pitches the scripts asked for.
    fades: Vec<crate::sound::CategoryChange>,
    /// The music cue the scripts last switched to (`Some(None)`: none).
    music: Option<Option<String>>,
    /// Support items to stock and call in once the scripts run
    /// (`--support`).
    support: Vec<String>,
    /// Seconds she has been on foot ashore without a break.
    ashore_for: f32,
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
    /// Let go by a delivery's helicopter, falling from where they hung:
    /// not placed afresh when the scripts move them to the drop point.
    dropped: HashSet<u32>,
    /// Those let go, by the scripts' time until which the scripts keep
    /// them at the drop point as they fall and land.
    landing: HashMap<u32, f64>,
    /// Ticks left for the vehicle the scripts seated her in (the start's
    /// boat) to load and be placed, she waiting where it is ([`reenter`]
    /// names it); then she goes ashore instead.
    ///
    /// [`reenter`]: Self::reenter
    boarding: u32,
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
            movies: false,
            subtitles: true,
            lag: 0.0,
            log,
            message: None,
            drivers,
            ashore: 0,
            blasts: Vec::new(),
            tutorial: None,
            bombs: Vec::new(),
            vehicles: Fleet::default(),
            completed: None,
            speed: 0.0,
            keys: Default::default(),
            lines: Vec::new(),
            hushed: Vec::new(),
            sounds: Vec::new(),
            fades: Vec::new(),
            music: None,
            support: Vec::new(),
            ashore_for: 0.0,
        }
    }

    /// Stocks and calls in these support items once the scripts run.
    pub fn with_support(self, support: Vec<String>) -> Self {
        Self { support, ..self }
    }

    /// The HUD called back the mission's HUD call number `seq` (the
    /// contract fanfare closing): its `fCallback` runs, as the HUD's own
    /// interface would run it, with its `tCallbackData`.
    pub fn hud_callback(&mut self, seq: u64) {
        let State::Running(m) = &mut self.state else {
            return;
        };
        let code = format!(
            "local t = __m2_hud_callbacks[{seq}]\n\
             __m2_hud_callbacks[{seq}] = nil\n\
             if t then MrxUtil.CallWithOptionalArgs(t.fCallback, t.tCallbackData) end"
        );
        if let Err(e) = m.run(&code, "hud callback") {
            self.write([format!("hud callback {seq}: {e}")]);
        } else {
            self.write([format!("hud callback {seq}")]);
        }
    }

    /// Has the scripts' movies (`01_AOA_J`, `02_AOB_J`) wait for the game:
    /// [`take_movie`](Self::take_movie) hands them out.
    pub fn with_movies(self, movies: bool) -> Self {
        Self { movies, ..self }
    }

    /// Whether the player has subtitles on (the game's option).
    pub fn with_subtitles(self, subtitles: bool) -> Self {
        Self { subtitles, ..self }
    }

    /// The next movie the scripts are waiting on, if any, with its
    /// subtitles if it has them: once it has played,
    /// [`movie_done`](Self::movie_done).
    pub fn take_movie(&mut self) -> Option<(String, Option<Vec<m2_game::subtitles::Line>>)> {
        let State::Running(m) = &mut self.state else {
            return None;
        };
        let name = m.take_movie();
        let subtitles = name.as_ref().and_then(|_| m.movie_subtitles());
        let lines = m.take_log();
        self.write(lines);
        name.map(|n| (n, subtitles))
    }

    /// The game's English strings, once the scripts run.
    pub fn strings(&self) -> Option<&m2_assets::StringTable> {
        match &self.state {
            State::Running(m) => Some(m.strings()),
            _ => None,
        }
    }

    /// The movie [`take_movie`](Self::take_movie) gave is over.
    pub fn movie_done(&mut self) {
        if let State::Running(m) = &mut self.state {
            m.movie_done();
            let lines = m.take_log();
            self.write(lines);
        }
    }

    /// Tutorial lines name these keys (else the usual ones).
    pub fn with_keys(self, keys: crate::input::Bindings) -> Self {
        Self { keys, ..self }
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
        // Deliveries fly on; the scripts hear of each load let go.
        let focus = game.focus();
        let scene = &mut game.scene;
        if let Some(world) = &mut scene.world {
            for r in scene
                .flights
                .step(dt, &mut world.physics, &mut scene.vehicles, focus)
            {
                for &g in &r.dropped {
                    self.vehicles.landing.insert(g, mission.now() + LANDING);
                }
                self.vehicles.dropped.extend(r.dropped);
                mission.arrived(r.pilot, r.to.to_array());
            }
        }
        // A crate she broke open is destroyed for the scripts too (what
        // it held, it leaves of itself: no script spawns it).
        for (g, at) in scene.flights.take_broken() {
            let left: Vec<String> = scene
                .flights
                .pickups()
                .iter()
                .filter(|p| p.from == g)
                .map(|p| {
                    format!(
                        "{} ({:#x}), model {}, giving {}",
                        scene
                            .world
                            .as_ref()
                            .and_then(|w| w.index.template_db.by_id(p.what.template))
                            .map_or("?", |t| t.0),
                        p.what.template,
                        p.what.model.map_or("none".into(), |m| format!("{m:08x}")),
                        p.what.gives.map_or("her nothing".into(), |(w, n)| format!(
                            "{n} rounds for her {w:?}"
                        )),
                    )
                })
                .collect();
            eprintln!(
                "mission: crate {g:#x} broken open at {at:.1}: it leaves {}",
                if left.is_empty() {
                    "nothing".to_owned()
                } else {
                    left.join("; ")
                }
            );
            mission.kill(g);
        }
        // The vehicle she was seated in never came: ashore instead.
        if self.vehicles.boarding > 0 {
            self.vehicles.boarding -= 1;
            if self.vehicles.boarding == 0
                && let Some(g) = self.vehicles.reenter.take()
            {
                eprintln!("mission: {g:#x} to seat her in did not load: she goes ashore");
                self.ashore = ASHORE_TRIES;
            }
        }
        if self.ashore > 0 && game.scene.player.as_ref().is_some_and(|p| p.landed) {
            self.ashore -= 1;
            // Thousands of rays a look: twice a second is plenty.
            if self.ashore.is_multiple_of(30) && go_ashore(game) {
                self.ashore = 0;
            }
        }
        self.blasts.extend(game.combat.explosions());
        self.bombs.extend(
            game.support
                .landed
                .iter()
                .filter(|l| matches!(l, Impact::Bomb(_)))
                .map(|l| {
                    let at = l.at();
                    Vec3::new(at.x, game.focus().y, at.y)
                }),
        );
        // The support's steps (the satellite's, the PDA's), posted as the
        // game's interface posts them, for the scripts waiting on them
        // (the first contract's).
        for &name in &game.support.posts {
            let waiting = mission
                .awaited_posts()
                .iter()
                .filter(|n| n.as_str() == name)
                .count();
            eprintln!("mission: posted {name:?}, {waiting} script events waiting for it");
            match name {
                support::PDA_OPEN => mission.post_net_event(name, true),
                support::PDA_CLOSE => mission.post_net_event(name, false),
                _ => mission.post(name),
            }
        }
        // And the sounds the interface cues with them (the timing wheel's
        // beeps, its correct and failed presses).
        self.sounds
            .extend(game.support.cues.iter().map(|&c| (c.to_owned(), false)));
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
            for (g, at, yaw) in game.scene.flights.places(&game.scene.vehicles) {
                mission.place(g, at.to_array(), yaw);
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
            // Those standing as models (the gates) fall only to a support
            // strike, which the scripts give her; should they not, one is
            // given her anyway after a while.
            let sturdy: HashSet<u32> = targets
                .iter()
                .map(|t| t.0)
                .filter(|&g| game.scene.props.get(g).is_some())
                .collect();
            if game
                .support
                .fallback(Kind::SatelliteBomb, !sturdy.is_empty(), STEP)
            {
                eprintln!(
                    "mission: no bomb from the scripts in {FALLBACK_AFTER:.0} s with a gate to bring down: one is given her"
                );
            }
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
                // A bomb's spot is on the ground under where it was
                // called: measured flat.
                let bombed = self.bombs.iter().any(|b| {
                    let c = props.centre(*g).unwrap_or(*at);
                    b.with_y(0.0).distance(c.with_y(0.0)) <= BOMB_PAD
                });
                let hit = |p: Vec3, pad: f32| {
                    props
                        .touches(*g, p, pad)
                        .unwrap_or_else(|| p.distance(*at) <= BLAST_REACH)
                };
                let blasted = !sturdy.contains(g)
                    && (self.blasts.iter().any(|&b| hit(b, BLAST_PAD))
                        || ram.is_some_and(|r| props.touches(*g, r, RAM_PAD) == Some(true)));
                if bombed || blasted {
                    if bombed {
                        eprintln!("mission: {g:#x} falls to the bomb");
                    }
                    mission.kill(*g);
                }
            }
            game.scene.props.sturdy = sturdy;
            // A target knocked loose (shoved over by a vehicle, thrown by a
            // blast) is destroyed too.
            for e in game.scene.props.take_knocked() {
                if targets.iter().any(|t| t.0 == e) {
                    mission.kill(e);
                }
            }
            self.blasts.clear();
            self.bombs.clear();
            // The scripts heal her and watch her health.
            game.combat.scripted_health = true;
            if std::mem::take(&mut game.combat.revived) {
                mission.player_died();
            }
            mission.set_health(PLAYER, game.combat.hp);
            mission.set_player_reserve(GRENADE, game.combat.grenades);
            // Called on foot, on land, as the support menu is.
            let on_land = game.on_foot
                && game.scene.player.as_ref().is_some_and(|p| {
                    p.landed
                        && p.driving.is_none()
                        && !p.down
                        && p.character.position.y > WATERLINE + ASHORE_HEIGHT
                })
                && self.vehicles.riding.is_none();
            self.ashore_for = if on_land { self.ashore_for + STEP } else { 0.0 };
            if mission.now() >= SUPPORT_AFTER && self.ashore_for >= SUPPORT_ASHORE {
                for key in std::mem::take(&mut self.support) {
                    mission.stock_support(&key, 1, SUPPORT_CASH, SUPPORT_FUEL);
                    let took = mission.use_support(&key);
                    eprintln!("mission: support {key} called in: {took}");
                }
            }
            report_strikes(game, mission);
            mission.set_player_reserve("C4", game.combat.charges);
            let spare = game.combat.rifle_rounds.stored;
            mission.set_player_reserve(CARBINE, spare.div_ceil(combat::RIFLE_CLIP));
            mission.tick(f64::from(STEP));
            lines.extend(mission.take_log());
            for c in mission.take_commands() {
                match &c {
                    Command::Say { id, speaker, cue } => {
                        // The player's own lines (and the radio) are flat.
                        let at = speaker.filter(|&g| g != PLAYER).and_then(|g| {
                            let w = mission.world().borrow();
                            w.objects.get(&g).map(|o| Vec3::from_array(o.position))
                        });
                        self.lines.push((*id, cue.clone(), at));
                    }
                    Command::Hush(id) => self.hushed.push(*id),
                    Command::CueSound { at: None, cue } => self.sounds.push((cue.clone(), false)),
                    Command::StopSound { at: None, cue } => self.sounds.push((cue.clone(), true)),
                    Command::FadeSound {
                        category,
                        level,
                        seconds,
                    } => match m2_engine::Category::named(category) {
                        Some(category) => self.fades.push(crate::sound::CategoryChange::Fade {
                            category,
                            level: *level,
                            seconds: *seconds,
                        }),
                        None => eprintln!("mission: no sounds of category {category} to fade"),
                    },
                    Command::PitchSound {
                        category,
                        pitch,
                        seconds,
                    } => match m2_engine::Category::named(category) {
                        Some(category) => self.fades.push(crate::sound::CategoryChange::Pitch {
                            category,
                            pitch: *pitch,
                            seconds: *seconds,
                        }),
                        None => eprintln!("mission: no sounds of category {category} to pitch"),
                    },
                    Command::InputEnabled(on) => {
                        eprintln!("mission: controls {}", if *on { "on" } else { "off" });
                        game.input_locked = !on;
                    }
                    Command::HealthClamp(on) => {
                        eprintln!("mission: health clamp {on}");
                        game.combat.health_clamp = *on;
                    }
                    Command::SetHealth { guid, health } if *guid == PLAYER => {
                        eprintln!(
                            "mission: scripts set her health {:.1} -> {health:.1}",
                            game.combat.hp
                        );
                        game.combat.hp = health.clamp(0.0, game.combat.max_hp);
                    }
                    Command::Music(cue) => self.music = Some(cue.clone()),
                    Command::Invincible { guid, on } if *guid == PLAYER => {
                        game.combat.invincible = *on;
                    }
                    // Her weapons, as the contract's scripts hand them out:
                    // fresh ones come with nothing in reserve.
                    Command::Loadout { who, weapons } if *who == PLAYER => {
                        eprintln!("mission: her loadout is now {weapons:?}");
                        game.combat.carbine_only();
                        game.combat.scripted_loadout = true;
                    }
                    Command::Reserve {
                        who,
                        weapon,
                        rounds,
                    } if *who == PLAYER => {
                        eprintln!("mission: scripts give her {weapon} {rounds} in reserve");
                        game.combat.scripted_loadout = true;
                        if weapon.eq_ignore_ascii_case(GRENADE) {
                            game.combat.grenades = (*rounds).min(combat::MAX_GRENADES);
                        } else if weapon.eq_ignore_ascii_case("C4") {
                            game.combat.charges = *rounds;
                        } else if weapon.eq_ignore_ascii_case(CARBINE) {
                            game.combat.rifle_rounds.stored =
                                (*rounds * combat::RIFLE_CLIP).min(combat::RIFLE_SPARE);
                        }
                    }
                    Command::DropWeapon { who, weapon } => {
                        eprintln!("mission: {who:#x} lets go of their {weapon}");
                    }
                    _ => {}
                }
                support_item(game, &c);
                strike(game, &c);
                fly(game, mission, &c);
                if obey(game, mission, &mut self.vehicles, &c) {
                    self.ashore = ASHORE_TRIES;
                }
                if c == Command::Notify("Pg.ContractCompleted".into()) {
                    self.completed = Some(0.0);
                }
            }
            if let Some(line) = tutorial(mission) {
                self.tutorial = line;
            }
            break_props(game, mission);
            let w = mission.world().borrow();
            if let Some(m) = w.hud.messages.last() {
                self.message = Some((m.clone(), mission.text(&m.text), 0.0));
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
        match Mission::new(data, game.scene.hero.name(), at) {
            Ok(mut mission) => {
                set_hero(&mut mission, game.scene.hero, game.combat.max_hp, "mission");
                mission.fly_deliveries();
                if let Err(e) = mission.run(RADAR_REGIONS, "radar regions") {
                    self.write([format!("radar regions: {e}")]);
                }
                if let Err(e) = mission.run(HUD_CALLS, "hud calls") {
                    self.write([format!("hud calls: {e}")]);
                }
                if let Err(e) = mission.run(TRESPASS, "trespass") {
                    self.write([format!("trespass: {e}")]);
                }
                for (name, code) in std::mem::take(&mut self.drivers) {
                    if let Err(e) = mission.run(&code, &name) {
                        self.write([format!("{name}: {e}")]);
                    }
                }
                if self.movies {
                    mission.hold_movies();
                }
                mission.set_subtitles(self.subtitles);
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

    /// Scripted voice lines to play (id, cue, where said) and to stop,
    /// since the last call.
    pub fn take_lines(&mut self) -> (Vec<ScriptLine>, Vec<u32>) {
        (
            std::mem::take(&mut self.lines),
            std::mem::take(&mut self.hushed),
        )
    }

    /// Sound category fades and pitches the scripts asked for since the
    /// last call.
    pub fn take_fades(&mut self) -> Vec<crate::sound::CategoryChange> {
        std::mem::take(&mut self.fades)
    }

    /// Interface sounds the scripts cued since the last call.
    pub fn take_sounds(&mut self) -> Vec<(String, bool)> {
        std::mem::take(&mut self.sounds)
    }

    /// The music cue to switch to, if the scripts changed it.
    pub fn take_music(&mut self) -> Option<Option<String>> {
        self.music.take()
    }

    /// Scripted line `id` lasts `seconds`.
    pub fn line_length(&mut self, id: u32, seconds: f32) {
        if let State::Running(m) = &mut self.state {
            m.line_length(id, f64::from(seconds));
        }
    }

    /// She is `hero` from now on, whole at `max` health: the running
    /// scripts' player takes their name, labels and health (see
    /// [`set_hero`]). Scripts still loading start as whoever she is then.
    pub fn set_hero(&mut self, hero: Hero, max: f32) {
        if let State::Running(m) = &mut self.state {
            set_hero(m, hero, max, "mission");
            let lines = m.take_log();
            self.write(lines);
        }
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
                    "soldier {} at {:.1} ({}, {}, {})",
                    if n.alive { "alive" } else { "dead" },
                    n.character.position,
                    game.scene.people.kinds[n.kind].role,
                    if n.landed { "landed" } else { "not landed" },
                    n.doing()
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
            // Who else stands within a few metres of it.
            let near: Vec<String> = game
                .scene
                .people
                .npcs
                .iter()
                .filter(|n| n.spawn != Some(g) && n.character.position.distance(p) < MARK_NEAR)
                .map(|n| {
                    format!(
                        "{} {:.1} m off{}",
                        game.scene.people.kinds[n.kind].role,
                        n.character.position.distance(p),
                        if n.alive { "" } else { " (dead)" }
                    )
                })
                .collect();
            if !near.is_empty() {
                what.push(format!("near it: {}", near.join(", ")));
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
        // Story characters: who stands, and the spawns that should.
        let people = &game.scene.people;
        for n in people.npcs.iter().filter(|n| people.kinds[n.kind].named) {
            out.push(format!(
                "story character {} at {:.1}, {}, {}, {}",
                people.kinds[n.kind].role,
                n.character.position,
                if n.landed { "landed" } else { "not landed" },
                if n.alive { "alive" } else { "dead" },
                n.doing()
            ));
        }
        // A story character's template is one word (Carmona), a soldier's
        // several; those within reach.
        let story = |s: &&m2_engine::world::SpawnPoint| {
            s.model.is_some()
                && s.template.as_deref().is_some_and(|t| !t.contains(' '))
                && s.position.distance(at) < STORY_REACH
        };
        for s in spawns.iter().filter(story) {
            if !people.npcs.iter().any(|n| n.spawn == Some(s.entity)) {
                out.push(format!(
                    "story spawn {:?} ({:08x}) at {:.1}, {:.0} m away: not standing{}",
                    s.name,
                    s.entity,
                    s.position,
                    s.position.distance(at),
                    if people.layer_in(s.layer) {
                        ""
                    } else {
                        " (its layer is out)"
                    }
                ));
            }
        }
        out.push(format!("props: {}", game.scene.props.counts()));
        out
    }

    /// The markers the scripts have up, with what they are over; then the
    /// targets of the objectives on the map (`"objective"`) and of the
    /// radar's blips (`"radar"`) no marker is over.
    pub fn marks(&self) -> Vec<Mark> {
        let State::Running(mission) = &self.state else {
            return Vec::new();
        };
        let w = mission.world().borrow();
        let mark = |g: u32, kind: &str| {
            let o = w.objects.get(&g)?;
            Some(Mark {
                target: g,
                kind: kind.to_owned(),
                name: o.name.clone(),
                at: Vec3::from(o.position),
                alive: o.alive,
            })
        };
        let mut out: Vec<Mark> = w
            .hud
            .markers
            .values()
            .filter_map(|m| mark(m.target?, m.icon.trim_start_matches("HUD_objective_")))
            .collect();
        let others = w
            .hud
            .objectives
            .iter()
            .filter(|(name, _)| !name.ends_with("_PreMission"))
            .filter_map(|(_, o)| Some((o.target?, "objective")))
            .chain(
                w.hud
                    .radar
                    .values()
                    .filter_map(|b| Some((b.target?, "radar"))),
            );
        for (g, kind) in others {
            if !out.iter().any(|m| m.target == g)
                && let Some(m) = mark(g, kind)
            {
                out.push(m);
            }
        }
        out
    }

    /// The names of the events the scripts wait for the game to post
    /// (`"PDA Open"`).
    pub fn awaited(&self) -> Vec<String> {
        match &self.state {
            State::Running(m) => m.awaited_posts(),
            _ => Vec::new(),
        }
    }

    /// The scripts have said the contract is completed.
    pub fn completed(&self) -> bool {
        self.completed.is_some()
    }

    /// What the HUD shows.
    pub fn view(&self) -> MissionView {
        let recent = self.message.as_ref().filter(|m| m.2 < MESSAGE_TIME);
        let message = recent.map(|(m, text, _)| (m.done(), text.clone()));
        let game_message = recent.map(|(m, ..)| m.clone());
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
            black: mission.black(),
            tutorial: self
                .tutorial
                .as_ref()
                .map(|t| crate::input::name_keys(t, &self.keys)),
            game_tutorial: self.tutorial.clone(),
            objectives,
            markers,
            message,
            game_message,
            contract_complete: self.completed.is_some_and(|age| age < BANNER_TIME),
            radar_regions: radar_regions(mission.lua()),
            hud_calls: hud_calls(mission.lua()),
            pda_objectives: hud.mission.clone(),
            trespass: trespass(mission.lua()),
            vehicle_disguise: w.vehicle_disguise,
            disguise: w.disguise,
        }
    }
}

/// Makes the player of `mission` (the mission's scripts, the hijack
/// scripts' or the HUD's) `hero`, whole at `max` health: their name, the
/// labels their template has in the data, and their health
/// ([`Mission::set_hero`]). Says so on stderr after `who`.
pub fn set_hero(mission: &mut Mission, hero: Hero, max: f32, who: &str) {
    let labels = mission.set_hero(hero.name(), hero.template(), max);
    eprintln!(
        "{who}: the scripts' player is {hero}, {max} health, {labels} labels from template {}",
        hero.template()
    );
}

/// The calls [`HUD_CALLS`] keeps, oldest first.
fn hud_calls(lua: &mlua::Lua) -> Vec<HudCall> {
    let Ok(Some(calls)) = lua.globals().get::<Option<mlua::Table>>("__m2_hud_calls") else {
        return Vec::new();
    };
    calls
        .sequence_values::<mlua::Table>()
        .flatten()
        .map(|t| HudCall {
            seq: t.get("seq").unwrap_or(0),
            group: t.get("group").unwrap_or_default(),
            method: t.get("method").unwrap_or_default(),
            data: Plain::of(&t.get("data").unwrap_or(mlua::Value::Nil)),
            callback: t.get("callback").unwrap_or(false),
        })
        .collect()
}

/// What [`TRESPASS`] keeps.
fn trespass(lua: &mlua::Lua) -> Trespass {
    let Ok(Some(t)) = lua.globals().get::<Option<mlua::Table>>("__m2_trespass") else {
        return Trespass::default();
    };
    Trespass {
        inside: t.get("bTrespassing").unwrap_or(false),
        faction: t.get("sFaction").ok().flatten(),
    }
}

/// The regions [`RADAR_REGIONS`] keeps, by name.
fn radar_regions(lua: &mlua::Lua) -> Vec<RadarRegion> {
    let Ok(Some(regions)) = lua
        .globals()
        .get::<Option<mlua::Table>>("__m2_radar_regions")
    else {
        return Vec::new();
    };
    let mut out: Vec<RadarRegion> = regions
        .pairs::<String, mlua::Table>()
        .flatten()
        .map(|(name, t)| RadarRegion {
            colour: ["nRed", "nGreen", "nBlue", "nAlpha"]
                .map(|k| t.get::<Option<f64>>(k).ok().flatten()),
            invert: t.get("bInvert").unwrap_or(false),
            name,
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
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
                    && !game.scene.flights.carrying(g)
                    && place(physics, v, Vec3::from(o.position), o.yaw)
                    && self.reenter == Some(g)
                    && let Some(p) = game.scene.player.as_mut()
                {
                    p.driving = Some(i);
                    // Seated, she no longer waits to land.
                    p.landed = true;
                    self.reenter = None;
                    self.boarding = 0;
                    eprintln!(
                        "mission: she is seated in {} ({g:#x}){}",
                        v.spec.label,
                        if v.spec.is_boat() { ", afloat" } else { "" }
                    );
                }
            }
        }
        let now = mission.now();
        self.landing.retain(|_, until| *until > now);
        for v in &game.scene.vehicles {
            let (Some(g), Some(id)) = (v.guid, v.id) else {
                continue;
            };
            if v.wrecked {
                mission.kill(g);
            }
            if self.landing.contains_key(&g) {
                continue;
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
        let island = game.scene.world.as_ref();
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
                let models = object_models(g, &o.name, island, &catalogue.vehicles);
                (!models.is_empty()).then_some((d, g, models))
            })
            .collect();
        // Those the scripts still have in play count against the most.
        let live = self
            .requested
            .iter()
            .filter(|g| w.objects.get(g).is_some_and(|o| o.alive))
            .count();
        drop(w);
        wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, g, models) in wanted.into_iter().take(MAX_VEHICLES.saturating_sub(live)) {
            self.requested.insert(g);
            game.scene.load_vehicle(g, models);
        }
    }
}

/// Seconds the scripts keep a vehicle a helicopter let go at its drop
/// point (where they moved it) while it falls the last few metres and
/// settles, so they see it delivered there.
const LANDING: f64 = 3.0;

/// Ticks the player gets to go ashore: about ten seconds for the ground
/// around her to load.
const ASHORE_TRIES: u32 = 600;

/// Ticks a vehicle the scripts seat her in gets to load and be placed:
/// about 30 seconds.
const BOARDING: u32 = 1800;

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

/// The scripts' newest tutorial line since the last call, if they asked
/// for one: its text, its key tokens kept (`[pda] Open PDA`), or none if
/// they cleared it. Each call is logged with its arguments.
fn tutorial(mission: &Mission) -> Option<Option<String>> {
    let calls = std::mem::take(&mut mission.world().borrow_mut().hud.tutorials);
    for args in &calls {
        eprintln!("mission: tutorial line {args:?}");
    }
    let args = calls.last()?;
    Some(m2_game::hud::tutorial_text(args).map(|t| mission.text(t)))
}

/// What the scripts' strike modules fly (`Airstrike`): the aircraft and
/// ordnance, and the designator she is handed to throw.
fn strike(game: &mut Game, c: &Command) {
    let none = Templates::default();
    let templates = game
        .scene
        .world
        .as_ref()
        .map_or(&none, |w| &w.index.template_db);
    game.scene.strikes.start(c, templates);
    match c {
        // A smoke grenade, flare or beacon is thrown; the satellite and
        // laser designators are aimed, which this game doesn't do for the
        // scripts' strikes yet.
        Command::EquipDesignator { who, kind, .. } if *who == PLAYER => {
            let thrown = ["Smoke", "Flare", "Beacon"]
                .iter()
                .any(|k| kind.contains(k));
            eprintln!("mission: the scripts hand her a {kind} (thrown: {thrown})");
            game.combat.to_throw = thrown;
        }
        Command::RemoveDesignator { who } if *who == PLAYER => game.combat.to_throw = false,
        Command::DirectedEffect { what, at, .. } => {
            eprintln!("mission: effect {what} at {at:?} (not drawn yet)");
        }
        _ => {}
    }
}

/// Tells the scripts of her designators thrown and come to rest, the
/// aircraft over their marks and the ordnance gone off.
fn report_strikes(game: &mut Game, mission: &mut Mission) {
    let settle = game
        .scene
        .world
        .as_ref()
        .and_then(|w| {
            let t = &w.index.template_db;
            let id = t.id(MARKER)?;
            t.ordnance.projectiles.get(&id)?.timer.map(|t| t.0)
        })
        .unwrap_or(MARKER_TIMER);
    for t in &mut game.combat.thrown {
        match t.guid {
            None => {
                let g = mission.designator_thrown(PLAYER, MARKER, t.at.to_array());
                eprintln!("mission: she throws her marker ({g:#x})");
                t.guid = Some(g);
            }
            Some(g) if !t.settled && t.age >= settle => {
                eprintln!("mission: her marker lies at {:.1}", t.at);
                mission.designator_settled(g, t.at.to_array());
                t.settled = true;
            }
            _ => {}
        }
    }
    let strikes = &mut game.scene.strikes;
    for (g, at) in strikes.arrived.drain(..) {
        mission.flyby_arrived(g, at.to_array());
    }
    for (g, _, at) in strikes.burst.drain(..) {
        mission.ordnance_burst(g, at.to_array());
    }
}

/// The scripts' free support items, given her or taken back.
fn support_item(game: &mut Game, c: &Command) {
    match c {
        Command::Freebie {
            key,
            quantity,
            limit,
        } => match Kind::from_freebie(key) {
            Some(kind) => {
                game.support.grant(kind, *quantity, *limit);
                eprintln!(
                    "mission: the scripts give her {quantity} {} free ({key}, at most {limit:?}): she has {}",
                    kind.name(),
                    game.support.ready().map_or(0, |i| i.count)
                );
            }
            None => eprintln!(
                "mission: the scripts give her {quantity} {key:?}, a support item not known here"
            ),
        },
        Command::RemoveFreebie(key) => {
            if let Some(kind) = Kind::from_freebie(key) {
                game.support.take_back(kind);
            }
            eprintln!("mission: the scripts take back her free {key:?}");
        }
        _ => {}
    }
}

/// Breaks the state layers' models whose objects the scripts' world has
/// dead (a gate blown up), and takes down those it no longer has.
fn break_props(game: &mut Game, mission: &Mission) {
    let blast = game.combat.last_blast;
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
                // Not the scripts' (one of the island's, brought down).
                None if crate::destruct::adopted(e) => {}
                None => gone.push(e),
                _ => {}
            }
        }
        (dead, gone)
    };
    drop(w);
    let (mut fires, mut falling) = (Vec::new(), Vec::new());
    for e in dead {
        scene.props.wreck(&mut world.physics, e, blast);
        fires.extend(scene.props.breakup_points(e));
        if scene.props.broke_apart(e) {
            falling.extend(scene.props.centre(e));
        }
    }
    scene.props.hide(&mut world.physics, gone);
    // What broke apart burns where its break-up says, and is heard
    // coming down.
    for at in fires {
        game.combat.blaze(&world.physics, at);
    }
    for at in falling {
        game.combat.collapse(at);
    }
}

/// Starts the helicopter a delivery's pilot flies (an `Ai.Deliver` order)
/// with what hangs from its winch, sends it home once it has let go (the
/// scripts' `MoveTo`), and takes away crates the scripts remove. A
/// delivery whose helicopter the scripts don't have arrives at once.
fn fly(game: &mut Game, mission: &mut Mission, c: &Command) {
    match c {
        Command::Goal {
            guid,
            goal,
            location: Some(to),
            ..
        } if goal == "Deliver" => {
            let none = Templates::default();
            let templates = game
                .scene
                .world
                .as_ref()
                .map_or(&none, |w| &w.index.template_db);
            let catalogue = game
                .scene
                .catalogue
                .as_ref()
                .map_or(&[][..], |c| c.vehicles.as_slice());
            let flight = {
                let w = mission.world().borrow();
                let copter = w.objects.get(guid).and_then(|o| o.vehicle).unwrap_or(*guid);
                w.objects.get(&copter).map(|o| {
                    let template = templates.get(&o.name);
                    let model = template.and_then(|t| t.model);
                    let bank = template.and_then(|t| t.engine);
                    let mut loads: Vec<Load> = w
                        .winch
                        .iter()
                        .filter(|(_, c)| **c == Some(copter))
                        .filter_map(|(&g, _)| {
                            let name = &w.objects.get(&g)?.name;
                            // A vehicle of theirs is the scene's; anything
                            // else hangs as its template's model.
                            let vehicle = !template_models(name, templates, catalogue).is_empty();
                            Some(Load {
                                guid: g,
                                model: (!vehicle)
                                    .then(|| templates.get(name).and_then(|t| t.model))
                                    .flatten(),
                                leaves: if vehicle {
                                    Vec::new()
                                } else {
                                    leaves(templates, name)
                                },
                            })
                        })
                        .collect();
                    loads.sort_by_key(|l| l.guid);
                    let copter = Copter {
                        guid: copter,
                        pilot: *guid,
                        model,
                        bank,
                    };
                    (copter, o.name.clone(), Vec3::from(o.position), loads)
                })
            };
            match flight {
                Some((copter, name, from, loads)) => {
                    // A vehicle to carry that is already down (it loaded
                    // before the scripts hung it on the winch) is taken up,
                    // with her got out of it first.
                    for l in loads.iter().filter(|l| l.model.is_none()) {
                        let (Some(i), Some(world)) = (
                            game.scene
                                .vehicles
                                .iter()
                                .position(|v| v.guid == Some(l.guid)),
                            &mut game.scene.world,
                        ) else {
                            continue;
                        };
                        if let Some(p) = game.scene.player.as_mut().filter(|p| p.driving == Some(i))
                        {
                            get_out(&world.physics, &game.scene.vehicles[i], p);
                        }
                        if let Some(id) = game.scene.vehicles[i].id.take() {
                            world.physics.remove_vehicle(id);
                            eprintln!("mission: {:#x} taken up to be carried", l.guid);
                        }
                    }
                    eprintln!(
                        "mission: {name} flies a delivery from {from:.1} to {:.1}, model {}, \
                         sound bank {}, carrying {:?}",
                        Vec3::from(*to),
                        copter
                            .model
                            .map_or("unknown".to_owned(), |m| format!("{m:08x}")),
                        copter.bank.map_or("none".to_owned(), |b| format!(
                            "{b:08x} {}",
                            crate::sound::bank_name(b).unwrap_or("?")
                        )),
                        loads
                    );
                    game.scene
                        .flights
                        .start(copter, loads, from, Vec3::from(*to));
                }
                None => mission.arrived(*guid, *to),
            }
        }
        Command::Goal {
            guid,
            goal,
            location: Some(to),
            ..
        } if goal == "MoveTo" && game.scene.flights.flying(*guid) => {
            game.scene.flights.send_home(*guid, Vec3::from(*to));
        }
        Command::Remove(guid) => game
            .scene
            .flights
            .removed(*guid, game.scene.world.as_mut().map(|w| &mut w.physics)),
        _ => {}
    }
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
            // One still to load (the start's boat): she waits where it is,
            // to be seated once it is placed.
            let w = mission.world().borrow();
            let catalogue = game
                .scene
                .catalogue
                .as_ref()
                .map_or(&[][..], |c| c.vehicles.as_slice());
            if let Some(o) = w.objects.get(vehicle)
                && !object_models(*vehicle, &o.name, game.scene.world.as_ref(), catalogue)
                    .is_empty()
            {
                let at = Vec3::from(o.position);
                drop(w);
                fleet.reenter = Some(*vehicle);
                fleet.boarding = BOARDING;
                fleet.look = 0;
                if let Some(p) = game.scene.player.as_mut() {
                    p.character.position = at;
                    p.character.velocity = Vec3::ZERO;
                    p.driving = None;
                    p.chase_target = None;
                }
                eprintln!("mission: she waits at {at:.1} for {vehicle:#x} to load, to be seated");
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
        // A delivery's load let go: falling from where it hung.
        Command::Move { guid, .. } if fleet.dropped.remove(guid) => return false,
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
        Command::Atmosphere { region, setting } => {
            game.atmosphere_setting(*region, setting);
            return false;
        }
        Command::AtmosphereImmediate(on) => {
            game.sky.immediate = *on;
            return false;
        }
        // A state layer in or out: its models and people with it.
        Command::Layer { op, name } => {
            let entities: Vec<u32> = mission
                .layer(name)
                .into_iter()
                .flatten()
                .map(|e| e.guid)
                .collect();
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
                scene.props.note_layer(entities.iter().copied(), name);
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
/// database's, when it has the template and it drives on the ground or
/// floats (helicopters don't fly yet); else the catalogue models whose
/// name fits the template's words ([`vehicle_models`]).
pub(crate) fn template_models(
    name: &str,
    templates: &Templates,
    catalogue: &[CatalogueEntry],
) -> Vec<CatalogueEntry> {
    match templates.get(name) {
        Some(&TemplateInfo {
            model: Some(model),
            vehicle:
                Some(
                    kind @ (VehicleKind::Car
                    | VehicleKind::Tank
                    | VehicleKind::Boat
                    | VehicleKind::Helicopter),
                ),
            ..
        }) => vec![CatalogueEntry {
            model,
            kind: match kind {
                VehicleKind::Tank => "tank",
                VehicleKind::Boat => m2_engine::vehicle::BOAT,
                VehicleKind::Helicopter => m2_engine::vehicle::HELICOPTER,
                _ => "car",
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

/// What the scripts' object `name` (a supply crate) leaves once destroyed:
/// the pickups its template's `SpawnOnDeath`s name, in data order (the
/// first contract's: a grenade launcher, then rockets). One with the
/// model of a weapon of hers is that weapon, loaded; any other gives the
/// rounds of one of her weapons when its ammunition is one of theirs (a
/// template with that weapon's model fires it). A pickup's `rounds` count
/// clips: the grenade launcher's 3 are the HUD's "6 2" in the footage, 6
/// loaded and 2 clips more. What isn't a pickup it doesn't leave for her.
fn leaves(templates: &Templates, name: &str) -> Vec<Leaves> {
    let Some(left) = templates
        .id(name)
        .and_then(|id| templates.leavings.spawns.get(&id))
    else {
        return Vec::new();
    };
    left.iter()
        .filter_map(|&left| pickup(templates, left))
        .collect()
}

/// What pickup template `id` gives her, as [`leaves`] reads it; besides,
/// the grenades of its `Grenade Projectile` record and a health pickup's
/// health (`Health Pickup`: 50, points of her 100).
pub(crate) fn pickup(templates: &Templates, id: u32) -> Option<Leaves> {
    let fires = |weapon: &str, ammo: u32| {
        let model = name_hash(weapon);
        templates.leavings.ammo.iter().any(|(&w, &a)| {
            a == ammo
                && templates
                    .by_id(w)
                    .is_some_and(|(_, t)| t.model == Some(model))
        })
    };
    let info = templates.leavings.pickups.get(&id)?;
    let model = templates.by_id(id).and_then(|(_, t)| t.model);
    // A gun of the templates' with its model (of several, the lowest).
    let gun = model.and_then(|m| {
        templates
            .arms
            .weapons
            .values()
            .filter(|w| w.class.is_some() && w.model == Some(m))
            .min_by_key(|w| w.template)
    });
    if let Some(spec) = gun {
        let slot = combat::Rifle::from_spec(spec).slot();
        return Some(Leaves {
            template: id,
            model,
            reach: info.radius,
            weapon: Some(slot),
            gun: Some(spec.template),
            gives: Some((slot, info.rounds.max(1) * spec.clip.max(1))),
            ..Leaves::default()
        });
    }
    let weapon = Weapon::ALL
        .into_iter()
        .find(|w| model == Some(name_hash(w.model_name())));
    let gives = match weapon {
        // Its own rounds, whatever its record names: the launcher's names
        // "Grenade Launcher Projectile", not the "GL Projectile" it fires
        // (round 94: taken up empty).
        Some(w) => Some((w, info.rounds.max(1) * w.clip())),
        None => info.ammo.and_then(|ammo| {
            Weapon::ALL
                .into_iter()
                .find(|w| fires(w.model_name(), ammo))
                .map(|w| (w, info.rounds * w.clip()))
        }),
    };
    // The launcher's rounds of an ammunition pickup: clips, as the
    // launcher's own 3 are.
    let launcher = match weapon {
        Some(_) => 0,
        None => info
            .gives
            .iter()
            .filter(|&&(ammo, _)| fires(Weapon::GrenadeLauncher.model_name(), ammo))
            .map(|(_, n)| n * Weapon::GrenadeLauncher.clip())
            .sum(),
    };
    let grenade = templates.id("Grenade Projectile");
    let grenades = info
        .gives
        .iter()
        .filter(|(ammo, _)| Some(*ammo) == grenade)
        .map(|(_, n)| n)
        .sum();
    Some(Leaves {
        template: id,
        model,
        reach: info.radius,
        weapon,
        gun: None,
        gives,
        launcher,
        grenades,
        health: if info.kind == m2_engine::pickup::HEALTH {
            info.rounds as f32
        } else {
            0.0
        },
    })
}

/// The models one of the scripts' objects drives as
/// ([`template_models`]): by its name, else (a state layer's renamed
/// entity, the first contract's `VzaCon001_StartingBoat`) by the template
/// it is an instance of.
fn object_models(
    guid: u32,
    name: &str,
    world: Option<&m2_engine::World>,
    catalogue: &[CatalogueEntry],
) -> Vec<CatalogueEntry> {
    let none = Templates::default();
    let templates = world.map_or(&none, |w| &w.index.template_db);
    let models = template_models(name, templates, catalogue);
    match world.and_then(|w| w.index.state_vehicles.get(&guid)) {
        Some(t) if models.is_empty() => template_models(&t.name, templates, catalogue),
        _ => models,
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
    fn cash_and_the_objective_tray_reach_the_hud_with_their_guids() {
        let lua = mlua::Lua::new();
        lua.load("Hud = { ResourceCounter = {}, ObjectiveTray = {}, Radar = function() end }")
            .exec()
            .unwrap();
        lua.globals()
            .set("me", m2_game::events::guid_value(1))
            .unwrap();
        lua.load(HUD_CALLS).exec().unwrap();
        lua.load(
            r#"Hud.ResourceCounter:SetCash({ vPlayer = me, nValue = 25000 })
            Hud.ObjectiveTray:SetSlotToText({ vPlayer = me, nSlot = 1, sText = "4" })"#,
        )
        .exec()
        .unwrap();
        let calls = hud_calls(&lua);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].group, "ResourceCounter");
        let Plain::Table(cash) = &calls[0].data else {
            panic!("{:?}", calls[0].data);
        };
        assert!(cash.contains(&(Plain::Text("vPlayer".into()), Plain::Guid(1))));
        assert_eq!(calls[1].method, "SetSlotToText");
    }

    #[test]
    fn the_scripts_hud_calls_are_kept_as_they_were_made() {
        let lua = mlua::Lua::new();
        lua.load(
            r#"
            Hud = { Fanfare = {} }
            got = 0
            function Hud.Fanfare:AddItem(t) got = got + 1 end
            "#,
        )
        .exec()
        .unwrap();
        lua.load(HUD_CALLS).exec().unwrap();
        lua.load(
            r#"
            Hud.Fanfare:Create({ sType = "contract", fCallback = function() end })
            local item = { sDescription = "[Fanfare.Completion.Total]", nValue = 5 }
            Hud.Fanfare:AddItem(item)
            item.nValue = 7
            "#,
        )
        .exec()
        .unwrap();
        let calls = hud_calls(&lua);
        assert_eq!(calls.len(), 2);
        assert_eq!((calls[0].seq, calls[0].method.as_str()), (1, "Create"));
        // The callback is left out; the item is as it was when added.
        assert_eq!(
            calls[0].data,
            Plain::Table(vec![(
                Plain::Text("sType".into()),
                Plain::Text("contract".into())
            )])
        );
        let Plain::Table(item) = &calls[1].data else {
            panic!("{:?}", calls[1].data);
        };
        assert!(item.contains(&(Plain::Text("nValue".into()), Plain::Number(5.0))));
        // The call still reaches what was there.
        assert_eq!(lua.globals().get::<i64>("got").unwrap(), 1);
    }

    #[test]
    fn a_broken_supply_crate_leaves_a_grenade_launcher_and_rockets() {
        use m2_engine::pickup::{Leavings, PickupInfo};
        let info = |model| TemplateInfo {
            model,
            vehicle: None,
            engine: None,
            person: false,
            human: false,
        };
        let (crate_id, launcher, grenade, rockets, rpg, missile) = (
            0x8000_9ae9,
            0x8000_6ea4,
            0x8000_6ea6,
            0x8000_56dc,
            0x8000_437b,
            0x8000_4379,
        );
        let pickup = |ammo, rounds| PickupInfo {
            kind: 1,
            radius: 1.0,
            ammo: Some(ammo),
            rounds,
            gives: vec![(ammo, rounds)],
            cue: None,
            effect: None,
        };
        let db = Templates {
            by_name: HashMap::from([
                ("supply drop (gl)".to_owned(), info(Some(0x81c7_1c96))),
                ("ammo pickup (rocket)".to_owned(), info(Some(0x8e77_a04d))),
                ("rpg".to_owned(), info(Some(name_hash("global_weapon_rpg")))),
                (
                    "grenade launcher".to_owned(),
                    info(Some(name_hash("global_weapon_mgl"))),
                ),
            ]),
            names: HashMap::from([
                (crate_id, "Supply Drop (GL)".to_owned()),
                (rockets, "Ammo Pickup (Rocket)".to_owned()),
                (rpg, "RPG".to_owned()),
                (launcher, "Grenade Launcher".to_owned()),
            ]),
            leavings: Leavings {
                // Not a pickup, so not left for her.
                spawns: HashMap::from([(crate_id, vec![launcher, rockets, 0x8000_0001])]),
                pickups: HashMap::from([
                    (launcher, pickup(grenade, 3)),
                    (rockets, pickup(missile, 20)),
                ]),
                ammo: HashMap::from([(rpg, missile), (launcher, grenade)]),
                labels: HashMap::new(),
            },
            ..Templates::default()
        };
        // The launcher, loaded with its 3 clips; 20 rockets (one a clip).
        assert_eq!(
            leaves(&db, "Supply Drop (GL) 0x40000001"),
            [
                Leaves {
                    template: launcher,
                    model: Some(name_hash("global_weapon_mgl")),
                    reach: 1.0,
                    weapon: Some(Weapon::GrenadeLauncher),
                    gives: Some((Weapon::GrenadeLauncher, 18)),
                    ..Leaves::default()
                },
                Leaves {
                    template: rockets,
                    model: Some(0x8e77_a04d),
                    reach: 1.0,
                    weapon: None,
                    gives: Some((Weapon::Rpg, 20)),
                    ..Leaves::default()
                },
            ]
        );
        // Ammunition for no weapon of hers gives her nothing, but a weapon
        // comes loaded whatever its record names; a template that leaves
        // nothing, nothing at all.
        let mut db = db;
        db.leavings.ammo.clear();
        let left = leaves(&db, "Supply Drop (GL)");
        assert_eq!(left[0].gives, Some((Weapon::GrenadeLauncher, 18)));
        assert_eq!(left[1].gives, None);
        assert!(leaves(&db, "RPG").is_empty());
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
            ..Templates::default()
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
        // A boat floats.
        assert_eq!(models("Patrol Boat (VZ)"), [(0x123, "boat".to_owned())]);
        // Without a model in the database, the words find the catalogue's.
        assert_eq!(models("M151 Softtop (GL)"), [(1, "truck".to_owned())]);
    }
}
