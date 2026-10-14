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
use m2_engine::{
    CatalogueEntry, Install, Physics, StaticKey, TemplateInfo, Templates, VehicleKind,
};
use m2_game::api::{Camera, Command, VISIBLE_RANGE};
use m2_game::events::World as _;
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
/// The scripts' steps between looks at her support stock, cash and fuel
/// (and at once after anything that changes them).
const STOCK_LOOK: u32 = 10;
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
    /// The keys bound to those tokens (`("pda", "P")`), for the game's own
    /// HUD to name those its fonts have no picture for.
    pub key_names: Vec<(String, String)>,
    /// Markers over targets.
    pub markers: Vec<MarkerView>,
    /// The newest objective message: completed (or else added), and text.
    pub message: Option<(bool, String)>,
    /// The same message as the scripts sent it, for the game's own HUD to
    /// word.
    pub game_message: Option<ObjectiveMessage>,
    /// The contract was just completed (for [`BANNER_TIME`] seconds).
    pub contract_complete: bool,
    /// Her cash, as the scripts hold it.
    pub cash: Option<f64>,
    /// The contract is completed and its fanfare waits for Enter to
    /// close it ([`MissionLink::waiting_fanfare`]).
    pub fanfare_waiting: bool,
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

/// A marker the scripts put over one of their objects (`Marker.AddBlip`),
/// as the HUD shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkerView {
    /// Where: its height over its target; over the soldier a spawn target
    /// placed, where he is now.
    pub at: Vec3,
    /// Its texture (`HUD_objective_destroy`).
    pub icon: String,
    /// RGBA.
    pub colour: [u8; 4],
    /// Shown from `near` to `far` metres away.
    pub near: f32,
    pub far: f32,
    /// Its size as the scripts ask (the call's first size, 32 in the first
    /// contract's).
    pub size: f32,
}

impl MarkerView {
    /// What the marker asks to be done there (`destroy`, `deliverable`):
    /// its texture's name without the prefix.
    pub fn label(&self) -> String {
        self.icon
            .trim_start_matches("HUD_objective_")
            .replace('_', " ")
    }

    /// Whether it shows to someone at `from`: `near` to `far` metres away.
    pub fn shown_from(&self, from: Vec3) -> bool {
        (self.near..=self.far).contains(&self.at.distance(from))
    }
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
                    -- The contract's fanfare, which the flow waits on.
                    if group == "Fanfare" then __m2_fanfare = __m2_hud_call_count end
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
    /// With [`movies`](Self::movies), the scripts' skip mode ends once the
    /// start has run ([`Mission::end_skip_mode`], `--skip-mode-off`).
    /// Off by default until the scripts that ask about it are surveyed on
    /// the game's data: the flow was only seen to finish with it on.
    end_skip: bool,
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
    /// Where the soldiers stand that markers' spawn targets placed, by
    /// target: the markers go over them, not over their spawns.
    marked: HashMap<u32, Vec3>,
    /// The markers logged, by handle.
    markers_logged: HashSet<u32>,
    /// Whether each object `ObjectIsVisible` events wait for was on
    /// screen at the last look, for the log.
    sighted: HashMap<u32, bool>,
    /// Saves the scripts asked for (`Pg.SaveGame`, `Sys.RequestAutosave`),
    /// by slot, for the game to write.
    saves: Vec<String>,
    /// Steps to the next look at her support stock.
    stock_look: u32,
    /// Aimed designators' marks (guid, where) waiting to settle, and the
    /// scripts' time they do.
    aimed: Vec<(u32, Vec3, f64)>,
    /// The pilots of support helicopters that have landed: they get out
    /// and walk off, and the helicopter stays for her.
    landed: HashSet<u32>,
    /// Her cash and fuel as the scripts last said, and the cash and fuel
    /// she has picked up in all.
    resources: Option<(f64, f64)>,
    pub earned: (u64, u64),
    /// What the support brought so far, for checks.
    pub counts: SupportCounts,
}

/// What the support brought in play: helicopters at their drop points
/// (deliveries and landings), aircraft over their marks, ordnance gone
/// off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SupportCounts {
    pub arrived: u32,
    pub flown: u32,
    pub burst: u32,
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
            end_skip: false,
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
            marked: HashMap::new(),
            markers_logged: HashSet::new(),
            sighted: HashMap::new(),
            saves: Vec::new(),
            stock_look: 0,
            aimed: Vec::new(),
            landed: HashSet::new(),
            resources: None,
            earned: (0, 0),
            counts: SupportCounts::default(),
        }
    }

    /// Every item of the scripts' support catalogue, by key.
    /// Every item of the scripts' support catalogue: its key, and whether
    /// it is a strike (`sType` "Airstrike") rather than a delivery.
    pub fn support_keys(&self) -> Vec<(String, bool)> {
        match &self.state {
            State::Running(m) => m
                .support_items()
                .into_iter()
                .map(|i| (i.key, i.kind == "Airstrike"))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Frees every recruit of the support's (the helicopter pilot, the jet
    /// pilot...) at once, their cooldowns and errands cut short: for a
    /// check calling one item after another.
    pub fn free_recruits(&mut self) {
        if let State::Running(m) = &mut self.state {
            let code = "local s = MrxSupportManager._tRecruitStates \
                        for k in pairs(s or {}) do s[k] = true end";
            if let Err(e) = m.run(code, "free recruits") {
                eprintln!("mission: could not free the recruits: {e}");
            }
        }
    }

    /// The saves the scripts asked for since the last call, by slot
    /// ([`Command::Save`]).
    pub fn take_saves(&mut self) -> Vec<String> {
        std::mem::take(&mut self.saves)
    }

    /// Her cash and fuel as the scripts hold them, once they run.
    pub fn resources(&self) -> Option<(f64, f64)> {
        self.resources
    }

    /// Puts her cash, fuel and support stock back as a save had them:
    /// what she has now is made up to it the scripts' way
    /// ([`Mission::give_resources`], `MrxPmc.SetSupportQty`).
    pub fn restore(&mut self, pmc: &crate::save::PmcSave) {
        let State::Running(m) = &mut self.state else {
            return;
        };
        let (cash, fuel) = m.resources();
        m.give_resources(pmc.cash - cash, pmc.fuel - fuel, "[Generic.Pickups]");
        for (key, count) in &pmc.stock {
            let code = format!("MrxPmc.SetSupportQty({key:?}, {count})");
            if let Err(e) = m.run(&code, "restore stock") {
                eprintln!("mission: could not restore {key}: {e}");
            }
        }
        self.resources = Some(m.resources());
        self.stock_look = 0;
    }

    /// Her cash, fuel and the support items she holds, to save.
    pub fn pmc_save(&self) -> Option<crate::save::PmcSave> {
        let State::Running(m) = &self.state else {
            return None;
        };
        let (cash, fuel) = m.resources();
        Some(crate::save::PmcSave {
            cash,
            fuel,
            stock: m
                .support_items()
                .into_iter()
                .filter(|i| i.stock > 0)
                .map(|i| (i.key, i.stock))
                .collect(),
        })
    }

    /// The scripts' own save data, to keep in a save
    /// ([`Mission::save_data`]).
    pub fn script_save(&self) -> Option<String> {
        let State::Running(m) = &self.state else {
            return None;
        };
        match m.save_data() {
            Ok(text) => Some(text),
            Err(e) => {
                eprintln!("mission: the scripts' save data: {e}");
                None
            }
        }
    }

    /// Opens faction `faction`'s shop as its contact does
    /// ([`Mission::open_shop`]); with `unlock_all`, everything it sells is
    /// unlocked first (a cheat: the scripts unlock items as contract
    /// rewards). What it sells, if it came up.
    pub fn open_shop(&mut self, faction: &str, unlock_all: bool) -> Option<ShopView> {
        let State::Running(m) = &mut self.state else {
            return None;
        };
        if unlock_all {
            let code = format!("{UNLOCK_ALL}\n__m2_unlock_all({faction:?})");
            if let Err(e) = m.run(&code, "unlock all") {
                eprintln!("mission: could not unlock the shop: {e}");
            }
        }
        let opened = m.open_shop(faction);
        let lines = m.take_log();
        self.write(lines);
        let State::Running(m) = &mut self.state else {
            return None;
        };
        opened.then(|| shop_view(m))
    }

    /// Buys one of shop item `id`; the shop as it is now, and whether it
    /// was sold.
    pub fn buy(&mut self, id: &str) -> Option<(ShopView, bool)> {
        let State::Running(m) = &mut self.state else {
            return None;
        };
        let bought = m.buy(id, 1);
        self.resources = Some(m.resources());
        self.stock_look = 0;
        eprintln!(
            "shop: {id} bought: {bought}; cash {:.0}",
            self.resources.map_or(0.0, |r| r.0)
        );
        let lines = m.take_log();
        self.write(lines);
        let State::Running(m) = &mut self.state else {
            return None;
        };
        Some((shop_view(m), bought))
    }

    /// Leaves the shop.
    pub fn close_shop(&mut self) {
        if let State::Running(m) = &mut self.state {
            m.close_shop();
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

    /// Ends the scripts' skip mode once the start has run, when the movies
    /// wait for the game (`--skip-mode-off`).
    pub fn with_skip_mode_off(self, end_skip: bool) -> Self {
        Self { end_skip, ..self }
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

    /// How black the scripts' fade has the screen, 0 to 1 (0 until they
    /// run).
    pub fn black(&self) -> f32 {
        match &self.state {
            State::Running(m) => m.black(),
            _ => 0.0,
        }
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
                self.counts.arrived += 1;
            }
        }
        // A crate she broke open is destroyed for the scripts too (what
        // it held, it leaves of itself: no script spawns it).
        for (g, at) in scene.flights.take_broken() {
            let none = Templates::default();
            let templates = scene.world.as_ref().map_or(&none, |w| &w.index.template_db);
            let left: Vec<String> = scene
                .flights
                .pickups()
                .iter()
                .filter(|p| p.from == g)
                .map(|p| {
                    // A weapon comes with its rounds; ammunition gives by
                    // its records, to what she carries when she comes.
                    let giving = match p.what.gives {
                        Some((w, n)) => format!("{n} rounds for her {w:?}"),
                        None => format!(
                            "{} (for her now {:?})",
                            records(templates, p.what.template),
                            refill(templates, &game.combat, &p.what)
                        ),
                    };
                    format!(
                        "{} ({:#x}), model {}, giving {giving}",
                        templates.by_id(p.what.template).map_or("?", |t| t.0),
                        p.what.template,
                        p.what.model.map_or("none".into(), |m| format!("{m:08x}")),
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
            // The money she picked up is the scripts' to count.
            for (cash, fuel) in std::mem::take(&mut game.earned) {
                self.earned.0 += u64::from(cash);
                self.earned.1 += u64::from(fuel);
                mission.give_resources(f64::from(cash), f64::from(fuel), "[Generic.Pickups]");
                self.stock_look = 0;
            }
            // What she chose in the support menu, the scripts call in;
            // they hand her its designator.
            if let Some(key) = game.support.call_in.take() {
                let took = mission.use_support(&key);
                eprintln!("mission: she calls in {key}: {took}");
                self.stock_look = 0;
            }
            // An aimed designator came down: the support manager takes the
            // spot as it takes a thrown marker's (the laser's own callback,
            // `LaserFinished`, isn't called).
            if let Some((what, at)) = game.support.designated.take() {
                let at = ground_under(game, at);
                let g = mission.designator_thrown(PLAYER, what, at.to_array());
                eprintln!("mission: her {what} marks {at:.1} ({g:#x})");
                self.aimed
                    .push((g, at, mission.now() + f64::from(MARKER_TIMER)));
                self.stock_look = 0;
            }
            // ... and settles there after a thrown marker's time: the
            // manager checks the spot in between (its validation queue),
            // and a mark settled at once is never taken up.
            let now = mission.now();
            for (g, at, _) in self.aimed.extract_if(.., |a| a.2 <= now) {
                mission.designator_settled(g, at.to_array());
            }
            // Her stock, cash and fuel as the scripts hold them.
            if self.stock_look == 0 {
                game.support.stock = stocked(mission);
                self.resources = Some(mission.resources());
                self.stock_look = STOCK_LOOK;
            }
            self.stock_look -= 1;
            let (flown, burst) = report_strikes(game, mission);
            self.counts.flown += flown;
            self.counts.burst += burst;
            mission.set_player_reserve("C4", game.combat.charges);
            let spare = game.combat.rifle_rounds.stored;
            mission.set_player_reserve(CARBINE, spare.div_ceil(combat::RIFLE_CLIP));
            look(game, mission, &mut self.sighted);
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
                        eprintln!(
                            "mission: her loadout is now {weapons:?}; her grenades, C4 and mines \
                             start at 0 until the scripts give them a reserve (the explosives \
                             box shows 0 till then)"
                        );
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
                    Command::Save { slot } => {
                        eprintln!("mission: the scripts ask for a save ({slot})");
                        self.saves.push(slot.clone());
                    }
                    _ => {}
                }
                support_item(game, &c);
                strike(game, &c);
                fly(game, mission, &mut self.landed, &c);
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
            follow_markers(game, mission, &mut self.marked, &mut self.markers_logged);
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
                if self.movies && self.end_skip {
                    // The movies wait for the game, so the scripts' skip
                    // mode has done its part.
                    mission.end_skip_mode();
                } else if self.movies {
                    eprintln!("mission: the scripts' skip mode stays on (--skip-mode-off)");
                }
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
        for n in people.npcs.iter().filter(|n| n.held) {
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

    /// The newest of the scripts' fanfare calls (`Hud.Fanfare`) whose
    /// callback is still to be called: the contract's, which the flow
    /// waits on until the fanfare's Enter closes it.
    pub fn waiting_fanfare(&self) -> Option<u64> {
        match &self.state {
            State::Running(m) => waiting_fanfare(m.lua()),
            _ => None,
        }
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
                let g = m.target?;
                let o = w.objects.get(&g)?;
                let at = self
                    .marked
                    .get(&g)
                    .copied()
                    .unwrap_or_else(|| Vec3::from(o.position));
                Some(MarkerView {
                    at: at + Vec3::Y * m.height,
                    icon: m.icon.clone(),
                    colour: m.colour,
                    near: m.near,
                    far: m.far,
                    size: m.sizes[0],
                })
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
            key_names: crate::input::token_keys(&self.keys),
            objectives,
            markers,
            message,
            game_message,
            contract_complete: self.completed.is_some_and(|age| age < BANNER_TIME),
            cash: self.resources.map(|r| r.0),
            fanfare_waiting: self.completed.is_some() && waiting_fanfare(mission.lua()).is_some(),
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

/// The fanfare call [`HUD_CALLS`] kept last with a callback, while that
/// callback has not been called.
fn waiting_fanfare(lua: &mlua::Lua) -> Option<u64> {
    let g = lua.globals();
    let seq = g.get::<Option<u64>>("__m2_fanfare").ok().flatten()?;
    let callbacks = g.get::<mlua::Table>("__m2_hud_callbacks").ok()?;
    callbacks.contains_key(seq).ok()?.then_some(seq)
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
            // A ruin stands wrecked from the start: nothing died.
            if v.wrecked && !game.scene.ruins.contains(&g) {
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
            .filter(|(_, g, models)| !ai_flown(&w, *g, models))
            .collect();
        // Those the scripts still have in play count against the most.
        let live = self
            .requested
            .iter()
            .filter(|g| w.objects.get(g).is_some_and(|o| o.alive))
            .count();
        drop(w);
        wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
        let w = mission.world().borrow();
        for (_, g, models) in wanted.into_iter().take(MAX_VEHICLES.saturating_sub(live)) {
            self.requested.insert(g);
            // A vehicle's ruin (`M151_Ruin`) stands wrecked.
            if let (Some(o), Some(island)) = (w.objects.get(&g), game.scene.world.as_ref())
                && is_ruin(&island.index.template_db, &o.name)
            {
                game.scene.ruins.insert(g);
            }
            game.scene.load_vehicle(g, models);
        }
    }
}

/// A ray from the camera that stops this far short of an object (metres)
/// still reaches it: what it hits there is the object itself or stands
/// right by it (a tank's hull, the wall it is parked against).
const SIGHT_PAD: f32 = 4.0;
/// How high above its place an object is looked at (metres): about a
/// vehicle's middle (inferred).
const SIGHT_HEIGHT: f32 = 1.5;
/// A ray from the camera passes through at most this many vehicles and
/// people (her own body first): they don't hide what is behind them.
const SIGHT_SKIPS: usize = 8;

/// Tells the scripts where her camera is and how wide it sees, and which
/// of the objects their `ObjectIsVisible` events wait for stand behind
/// something solid from it: the island's ground and buildings, the state
/// layers' models (vehicles and people don't hide what is behind them).
/// Logs each as it comes on screen or leaves it.
fn look(game: &Game, mission: &mut Mission, sighted: &mut HashMap<u32, bool>) {
    let (eye, forward) = game.listener();
    let (fov, aspect) = game.lens;
    let camera = Camera {
        eye: eye.to_array(),
        forward: forward.to_array(),
        fov_y: fov.to_radians(),
        aspect,
        range: VISIBLE_RANGE,
    };
    mission.set_camera(camera);
    let watched = mission.sight_watched();
    sighted.retain(|g, _| watched.contains(g));
    if watched.is_empty() {
        return;
    }
    // Looked at: a prop's middle, else a little above its place.
    let targets: Vec<(u32, Vec3, String)> = {
        let w = mission.world().borrow();
        watched
            .iter()
            .filter_map(|&g| {
                let Some(o) = w.objects.get(&g) else {
                    if sighted.insert(g, false).is_none() {
                        eprintln!(
                            "mission: ObjectIsVisible waits for {g:#x}, not in play (its layer \
                             isn't loaded): never on screen"
                        );
                    }
                    return None;
                };
                let at = game
                    .scene
                    .props
                    .centre(g)
                    .unwrap_or(Vec3::from(o.position) + Vec3::Y * SIGHT_HEIGHT);
                Some((g, at, o.name.clone()))
            })
            .collect()
    };
    let mut unseen = HashSet::new();
    let mut viewed = Vec::new();
    for (g, at, name) in targets {
        let in_view = camera.sees(at.to_array());
        let behind = in_view
            && game
                .scene
                .world
                .as_ref()
                .is_some_and(|world| behind_something(&world.physics, eye, at, g));
        if behind {
            unseen.insert(g);
        }
        viewed.push((g, name, at, in_view, behind));
    }
    mission.set_unseen(unseen);
    let w = mission.world().borrow();
    for (g, name, at, in_view, behind) in viewed {
        let seen = w.is_visible(g) == Some(true);
        if sighted.insert(g, seen) != Some(seen) {
            eprintln!(
                "mission: {name} ({g:#x}) at {at:.1} {} for the scripts (in view {in_view}, \
                 behind something {behind}, {:.0} m from the camera)",
                if seen { "on screen" } else { "not on screen" },
                at.distance(eye)
            );
        }
    }
}

/// Whether something solid stands between `eye` and `at`, short of
/// [`SIGHT_PAD`]: the island's or a state layer's model other than
/// `target`'s own, passing through vehicles and people.
fn behind_something(physics: &Physics, eye: Vec3, at: Vec3, target: u32) -> bool {
    let way = at - eye;
    let reach = way.length() - SIGHT_PAD;
    let dir = way.normalize_or_zero();
    let (mut from, mut gone) = (eye, 0.0);
    for _ in 0..SIGHT_SKIPS {
        if reach - gone <= 0.0 {
            return false;
        }
        match physics.cast_ray_keyed(from, dir, reach - gone) {
            None => return false,
            Some((_, _, Some(StaticKey::Entity(e)))) if e == target => return false,
            Some((_, _, Some(_))) => return true,
            Some((t, _, None)) => {
                from += dir * (t + 0.1);
                gone += t + 0.1;
            }
        }
    }
    false
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

/// Where the soldiers stand whom the markers' spawn targets placed (the
/// first contract's "destroy" markers are on spawns, and the scripts'
/// world keeps the spawn's place while he moves), by target; and each new
/// marker logged once, with what the scripts asked of it.
fn follow_markers(
    game: &Game,
    mission: &Mission,
    marked: &mut HashMap<u32, Vec3>,
    logged: &mut HashSet<u32>,
) {
    let w = mission.world().borrow();
    marked.clear();
    for (&handle, m) in &w.hud.markers {
        let Some(g) = m.target else { continue };
        if let Some(n) = game.scene.people.npcs.iter().find(|n| n.spawn == Some(g)) {
            marked.insert(g, n.character.position);
        }
        if logged.insert(handle) {
            eprintln!(
                "mission: marker {} on {:?} ({g:#x}): colour {:?}, {} m up, shown {}..{} m, sizes {:?}, 2D only {}{}",
                m.icon,
                w.objects.get(&g).map_or("?", |o| o.name.as_str()),
                m.colour,
                m.height,
                m.near,
                m.far,
                m.sizes,
                m.flat,
                if marked.contains_key(&g) {
                    ", over the soldier it placed"
                } else {
                    ""
                }
            );
        }
    }
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
        // laser designators are aimed ([`support::Aim`]).
        Command::EquipDesignator { who, kind, .. } if *who == PLAYER => {
            let thrown = ["Smoke", "Flare", "Beacon"]
                .iter()
                .any(|k| kind.contains(k));
            let aimed = !thrown && game.support.equip(kind);
            eprintln!("mission: the scripts hand her a {kind} (thrown: {thrown}, aimed: {aimed})");
            game.combat.to_throw = thrown;
        }
        Command::RemoveDesignator { who } if *who == PLAYER => {
            game.combat.to_throw = false;
            game.support.unequip();
        }
        Command::DirectedEffect { what, at, .. } => {
            eprintln!("mission: effect {what} at {at:?} (not drawn yet)");
        }
        _ => {}
    }
}

/// Tells the scripts of her designators thrown and come to rest, the
/// aircraft over their marks and the ordnance gone off; how many of the
/// last two there were.
fn report_strikes(game: &mut Game, mission: &mut Mission) -> (u32, u32) {
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
    let counts = (strikes.arrived.len() as u32, strikes.burst.len() as u32);
    for (g, at) in strikes.arrived.drain(..) {
        mission.flyby_arrived(g, at.to_array());
    }
    for (g, _, at) in strikes.burst.drain(..) {
        mission.ordnance_burst(g, at.to_array());
    }
    counts
}

/// The ground under `at` (a satellite's mark is on the ground she stands
/// on); `at` itself where nothing is found.
fn ground_under(game: &Game, at: Vec3) -> Vec3 {
    const ABOVE: f32 = 300.0;
    let top = at + Vec3::Y * ABOVE;
    game.scene
        .world
        .as_ref()
        .and_then(|w| w.physics.cast_ray(top, Vec3::NEG_Y, ABOVE * 2.0))
        .map_or(at, |(d, _)| top - Vec3::Y * d)
}

/// The factions whose contacts keep a shop, as the scripts name them
/// (`MrxFactionManager`'s keys), and as the shop's tabs name them.
pub const SHOPS: [(&str, &str); 6] = [
    ("All", "Allied Nations"),
    ("Chi", "China"),
    ("Gur", "PLAV"),
    ("Pir", "Pirates"),
    ("Oil", "Universal Petroleum"),
    ("Pmc", "Your PMC"),
];

/// Lua that unlocks everything faction `f`'s shop could ever sell, as
/// the contracts' rewards would one by one (`MrxRewardData`'s potential
/// shop items; a support item is unlocked by its `tUnlockStatus` for the
/// faction, equipment by `WifEquipmentData.UnlockItem`). The recruits the
/// support needs (the helicopter and jet pilots, the mechanic: the story
/// brings them one by one, and without them a strike is refused, "Pilot
/// unavailable") join too, as the scripts' own setters have them.
const UNLOCK_ALL: &str = r#"
function __m2_unlock_all(f)
    MrxSupportData.SetHeliPilotRecruited(true)
    MrxSupportData.SetJetPilotRecruited(true)
    MrxSupportData.SetMechanicRecruited(true)
    local support, equipment = MrxRewardData.GetAllPotentialShopItems(f)
    local seen = MrxSupportData._knUnlockStatusViewed or 2
    for _, key in ipairs(support or {}) do
        local item = MrxSupportData.tSupportData[key]
        if item then
            item.tUnlockStatus = item.tUnlockStatus or {}
            if item.tUnlockStatus[f] == nil then item.tUnlockStatus[f] = seen end
        end
    end
    for _, key in ipairs(equipment or {}) do
        WifEquipmentData.UnlockItem(key, f)
    end
end
"#;

/// A shop as the player sees it: her cash, and each thing on sale.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShopView {
    pub cash: f64,
    pub fuel: f64,
    pub items: Vec<ShopLine>,
}

/// A thing on sale, readable.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShopLine {
    /// What buying it names.
    pub id: String,
    pub name: String,
    pub description: String,
    pub price: f64,
    /// How many she holds, and the most she may.
    pub stock: i64,
    pub most: i64,
    pub unlocked: bool,
}

/// The shop the scripts have up, its texts read.
fn shop_view(m: &Mission) -> ShopView {
    let (cash, fuel) = m.resources();
    let items = m
        .shop()
        .items
        .into_iter()
        .map(|i| ShopLine {
            name: m.text(&i.name),
            description: m.text(&i.description),
            id: i.id,
            price: i.cash,
            stock: i.stock,
            most: i.max_stock,
            unlocked: i.unlocked,
        })
        .collect();
    ShopView { cash, fuel, items }
}

/// The scripts' support items she holds (bought or free), readable, for
/// the support menu.
fn stocked(m: &Mission) -> Vec<support::Stocked> {
    m.support_items()
        .into_iter()
        .filter(|i| i.stock > 0 || i.free > 0)
        .map(|i| support::Stocked {
            name: {
                let shown = m.text(&i.name);
                if shown.is_empty() {
                    i.key.clone()
                } else {
                    shown
                }
            },
            method: i.designator.trim_end_matches(" Designator").to_owned(),
            count: u32::try_from(i.stock.max(0) + i.free.max(0)).unwrap_or(0),
            free: i.free > 0,
            key: i.key,
        })
        .collect()
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
///
/// A support helicopter's landing ([`m2_game::mission::LAND`]) is flown
/// the same way, with nothing on the winch, down to the mark: its goal's
/// location is 50 m over the mark (the copter delivery's script adds 50
/// to the mark's height). Once down its pilot gets out and walks off, and
/// the helicopter stays: its pilot's `MoveTo` doesn't send it home
/// (`landed`). It can't be flown in this game yet.
fn fly(game: &mut Game, mission: &mut Mission, landed: &mut HashSet<u32>, c: &Command) {
    const LAND_ABOVE: f32 = 50.0;
    match c {
        Command::Goal {
            guid,
            goal,
            location: Some(to),
            ..
        } if goal == "Deliver" || goal == m2_game::mission::LAND => {
            let to = &if goal == m2_game::mission::LAND {
                landed.insert(*guid);
                [to[0], to[1] - LAND_ABOVE, to[2]]
            } else {
                *to
            };
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
        } if goal == "MoveTo" && game.scene.flights.flying(*guid) && !landed.contains(guid) => {
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
        // or person just moves.
        Command::Move { guid, .. } | Command::Remove(guid) if *guid != PLAYER => {
            if let (Some(i), Some(world)) = (index(guid), &mut game.scene.world)
                && let Some(id) = game.scene.vehicles[i].id.take()
            {
                world.physics.remove_vehicle(id);
                let driving = game.scene.player.as_ref().and_then(|p| p.driving);
                // She goes with it, as when they seat her in one: there,
                // so the ground loads around it and it is placed, with
                // the same time to be. Not where they hide it under the
                // island; there she stays on foot.
                if let Command::Move { position, .. } = c
                    && driving == Some(i)
                    && position[1] > HIDDEN_BELOW
                    && let Some(p) = game.scene.player.as_mut()
                {
                    fleet.reenter = Some(*guid);
                    fleet.boarding = BOARDING;
                    p.drop_at(Vec3::from(*position));
                    p.driving = None;
                    p.chase_target = None;
                    eprintln!(
                        "mission: the scripts moved {guid:#x} with her in it: she waits at \
                         {:.1?} to be seated again",
                        position
                    );
                }
            }
            // Gone, or hidden, while she waited to get in: the wait ends
            // on the next step as if it had run out, so she stays out (and
            // goes ashore if she waits in the sea, as for the start's boat)
            // and the scripts hear where she is again.
            if fleet.reenter == Some(*guid)
                && match c {
                    Command::Move { position, .. } => position[1] <= HIDDEN_BELOW,
                    _ => true,
                }
            {
                fleet.boarding = 1;
            }
            let scene = &mut game.scene;
            if let Some(world) = &mut scene.world {
                match c {
                    Command::Move { position, .. } => {
                        let at = Vec3::from(*position);
                        scene.props.put(&mut world.physics, *guid, at);
                        // A person the layers place (a story character):
                        // there they stand.
                        if is_spawn(world, *guid) {
                            scene.people.put(*guid, at);
                        }
                    }
                    _ if is_spawn(world, *guid) => {
                        scene.people.script_kill(&mut world.physics, *guid, true);
                    }
                    _ => {}
                }
            }
            return false;
        }
        // A driver sent along a path (PmcCon001's villa tank, through the
        // garage wall the scripts have just killed): its vehicle drives it
        // ([`crate::paths`]). A person on foot sent along one isn't walked
        // yet (the villa's wave soldiers).
        Command::Goal {
            guid,
            goal,
            target: Some(path),
            haste,
            ..
        } if goal == "PathMove" => {
            let vehicle = mission.world().borrow().vehicle_of(*guid);
            let route: Vec<Vec3> = mission.path(*path).iter().map(|&p| Vec3::from(p)).collect();
            match vehicle {
                Some(v) if !route.is_empty() => {
                    eprintln!(
                        "mission: {guid:#x} drives {v:#x} along path {path:#x} ({} points, haste \
                         {haste:?}): {:.1?} to {:.1?}",
                        route.len(),
                        route[0],
                        route[route.len() - 1]
                    );
                    game.paths.start(v, route, haste.unwrap_or(1.0));
                }
                _ => eprintln!(
                    "mission: PathMove for {guid:#x} along {path:#x} not driven (vehicle \
                     {vehicle:x?}, {} points)",
                    route.len()
                ),
            }
            return false;
        }
        Command::Kill(guid) => {
            if let Some(i) = index(guid) {
                game.scene.vehicles[i].wrecked = true;
            }
            // A prop the scripts kill (PmcCon001's garage entrance, the
            // villa hall's wall the tank comes through) breaks: its wreck,
            // else it goes.
            let scene = &mut game.scene;
            if let Some(world) = &mut scene.world
                && scene.props.get(*guid).is_some()
            {
                let centre = scene.props.centre(*guid);
                let broke = scene.props.wreck(&mut world.physics, *guid, centre);
                eprintln!(
                    "mission: the scripts killed prop {guid:#x}: {}",
                    if broke { "broken" } else { "already broken" }
                );
            }
            // Wrecked while she waited to get in: the wait ends, as above.
            if fleet.reenter == Some(*guid) {
                fleet.boarding = 1;
            }
            let scene = &mut game.scene;
            if let Some(world) = &mut scene.world
                && is_spawn(world, *guid)
            {
                scene.people.script_kill(&mut world.physics, *guid, false);
            }
            return false;
        }
        // A gate or door opened (PmcCon001's villa doors): its model is
        // taken down with its collision, so she walks through (inferred:
        // the door's own opening animation isn't played).
        Command::OpenGate(guid) => {
            let name = mission
                .world()
                .borrow()
                .objects
                .get(guid)
                .map_or_else(|| "?".to_owned(), |o| o.name.clone());
            let scene = &mut game.scene;
            match (&mut scene.world, scene.props.get(*guid)) {
                (Some(world), Some(_)) => {
                    scene.props.hide(&mut world.physics, [*guid]);
                    eprintln!(
                        "mission: Object.OpenGate {name} ({guid:#x}): opened, its model and \
                         collision taken down (inferred: no opening animation)"
                    );
                }
                _ => eprintln!(
                    "mission: Object.OpenGate {name} ({guid:#x}): no model of it up to open"
                ),
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
        // Up until the place has loaded (the engine's to take down).
        Command::LoadingScreen(on) => {
            game.arrival = on.then_some(0.0);
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
    if let Some(from) = game.scene.player.as_ref().map(|p| p.character.position) {
        game.teleported(from, Vec3::from(to));
    }
    if let Some(p) = game.scene.player.as_mut() {
        // At the height they give: onto the floor or ground there, not a
        // roof over it.
        p.drop_at(Vec3::from(to));
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
        // A template the database knows with no vehicle physics is no
        // vehicle, whatever its name's words: `68Valiant_Ruin`, a burnt
        // car its own model draws, isn't a Valiant to drive (round 8:
        // the two ruins by VzaCon001's huts stood twice, intact too).
        Some(&TemplateInfo { vehicle: None, .. }) => Vec::new(),
        _ => vehicle_models(name, catalogue),
    }
}

/// What the scripts' object `name` (a supply crate) leaves once destroyed:
/// the pickups its template's `SpawnOnDeath`s name, in data order (the
/// first contract's: a grenade launcher, then rockets). One with the
/// model of a weapon of hers is that weapon, loaded; any other gives what
/// [`refill`] reads off its records when she comes to it. A weapon's
/// `rounds` count clips: the grenade launcher's 3 are the HUD's "6 2" in
/// the footage, 6 loaded and 2 clips more. What isn't a pickup it doesn't
/// leave for her.
pub(crate) fn leaves(templates: &Templates, name: &str) -> Vec<Leaves> {
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

/// What pickup template `id` is, as [`leaves`] reads it: a weapon taken
/// up, else ammunition and health (`Health Pickup`: 50, points of her
/// 100; `Ammo Pickup (Small)`: 3, beside its rounds).
pub(crate) fn pickup(templates: &Templates, id: u32) -> Option<Leaves> {
    let info = templates.leavings.pickups.get(&id)?;
    let model = templates.by_id(id).and_then(|(_, t)| t.model);
    // Money: cash or fuel, as much as its rounds field says.
    if matches!(info.kind, m2_engine::pickup::CASH | m2_engine::pickup::FUEL) {
        let cash = info.kind == m2_engine::pickup::CASH;
        return Some(Leaves {
            template: id,
            model,
            reach: info.radius,
            cash: if cash { info.rounds } else { 0 },
            fuel: if cash { 0 } else { info.rounds },
            ..Leaves::default()
        });
    }
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
        // Its clips: its first record's amount (the crate launcher's 3);
        // else, where that is none (the soldiers' `Assault Rifle`, whose
        // equipment record gives no amount), what its ammunition records
        // give that the gun takes (its 8 `Assault Rifle Bullet`); else one.
        let fires = Some(spec.ammo.template);
        let ammo: u32 = info
            .walked
            .iter()
            .filter(|&&(a, _)| takes(templates, Some(spec.template), fires, a))
            .map(|&(_, n)| n)
            .sum();
        let clips = match info.rounds {
            0 if ammo > 0 => ammo,
            0 => 1,
            n => n,
        };
        return Some(Leaves {
            template: id,
            model,
            reach: info.radius,
            weapon: Some(slot),
            gun: Some(spec.template),
            gives: Some((slot, clips * spec.clip.max(1))),
            ..Leaves::default()
        });
    }
    let weapon = Weapon::ALL
        .into_iter()
        .find(|w| model == Some(name_hash(w.model_name())));
    Some(Leaves {
        template: id,
        model,
        reach: info.radius,
        weapon,
        gun: None,
        // Its own rounds, whatever its record names: the launcher's names
        // "Grenade Launcher Projectile", not the "GL Projectile" it fires
        // (round 94: taken up empty).
        gives: weapon.map(|w| (w, info.rounds.max(1) * w.clip())),
        // Every health record's, not only a health pickup's: the small
        // ammunition box heals 3 as well.
        health: info.heals as f32,
        ..Leaves::default()
    })
}

/// How she takes a pickup lying within reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Offer {
    /// As she walks over it, unworded.
    Walk,
    /// With the use key, worded "Take Ammo": its ammunition only.
    TakeAmmo,
    /// With the use key, worded "Equip <its name>": the weapon taken up
    /// in place of one of hers.
    Equip,
}

/// How she takes pickup `what`, as the real game's footage shows it
/// (playtest 3; inferred, no `Pickup` field found to say it): what has a
/// health record (`Health Pickup`, and `Ammo Pickup (Small)`, which heals
/// 3) is walked over (3:32, health 62 to 100); a weapon she doesn't carry
/// is "Equip Assault Rifle" (3:14, 14:20, 15:24); one she carries, and an
/// ammunition box without health (`Ammo Pickup (Bullet)`, the rockets),
/// "Take Ammo" (9:00, 9:42, 15:38, 16:36).
pub(crate) fn offer(templates: &Templates, combat: &combat::Combat, what: &Leaves) -> Offer {
    // Money is walked over, whatever she carries (inferred: its record
    // names nothing to choose, and no prompt for it was seen).
    if what.is_money() {
        return Offer::Walk;
    }
    if what.weapon.is_some() {
        let is = |t: Option<u32>| t.is_some() && (t == what.gun || t == Some(what.template));
        let carries =
            is(combat.rifle.template) || (combat.second.is_some() && is(combat.launcher.template));
        return if carries {
            Offer::TakeAmmo
        } else {
            Offer::Equip
        };
    }
    let heals = templates
        .leavings
        .pickups
        .get(&what.template)
        .is_some_and(|p| p.heals > 0);
    if heals || what.health > 0.0 {
        Offer::Walk
    } else {
        Offer::TakeAmmo
    }
}

/// Whether a `Pickup` record's ammunition `ammo` is for weapon template
/// `weapon`, which fires `fires` (its `AmmoTemplate`), by the links the
/// templates give: it is that ammunition, or the weapon itself (or one of
/// the same name); it is what the weapon's own `Pickup`s give, lying as a
/// pickup (the grenade launcher's `Grenade Launcher Projectile`, not the
/// `GL Projectile` it fires); or a gun of the weapon's model fires it.
pub(crate) fn takes(
    templates: &Templates,
    weapon: Option<u32>,
    fires: Option<u32>,
    ammo: u32,
) -> bool {
    let name = |id: u32| templates.names.get(&id);
    let same = |a: u32| {
        a == ammo
            || name(a)
                .zip(name(ammo))
                .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
    };
    if fires.is_some_and(same) {
        return true;
    }
    let Some(weapon) = weapon else {
        return false;
    };
    if same(weapon) {
        return true;
    }
    let own = templates
        .leavings
        .pickups
        .get(&weapon)
        .is_some_and(|p| p.gives.iter().any(|&(a, _)| same(a)));
    let model = templates.by_id(weapon).and_then(|(_, t)| t.model);
    own || model.is_some_and(|m| {
        templates
            .leavings
            .ammo
            .iter()
            .any(|(&w, &a)| same(a) && templates.by_id(w).is_some_and(|(_, t)| t.model == Some(m)))
    })
}

/// What pickup `what` gives her now, as ammunition: every `Pickup`
/// record of its template, each to the weapon of hers that [`takes`] it.
/// A weapon lying there is taken up; as she walks over it, it gives only
/// what its ammunition records (`Type` 1) give a weapon of hers (a dead
/// soldier's `Assault Rifle`: its 8 `Assault Rifle Bullet` to a rifle
/// that fires them), none for most.
pub(crate) fn refill(
    templates: &Templates,
    combat: &combat::Combat,
    what: &Leaves,
) -> combat::Refill {
    let records = given(templates, what);
    combat.refill(records, |weapon, fires, ammo| {
        takes(templates, weapon, fires, ammo)
    })
}

/// The `Pickup` records of `what`'s template that count walking over it:
/// every one, or a weapon lying there's ammunition records alone.
fn given<'a>(templates: &'a Templates, what: &Leaves) -> &'a [(u32, u32)] {
    templates
        .leavings
        .pickups
        .get(&what.template)
        .map_or(&[][..], |p| {
            if what.weapon.is_some() {
                p.walked.as_slice()
            } else {
                p.gives.as_slice()
            }
        })
}

/// Why pickup `what` gives her nothing now, for the log: the records
/// [`refill`] reads, each to the weapon of hers that [`takes`] it and that
/// weapon's room, or to none ([`combat::Combat::refusal`]).
pub(crate) fn refusal(templates: &Templates, combat: &combat::Combat, what: &Leaves) -> String {
    let records = given(templates, what);
    combat.refusal(
        records,
        what.health,
        |weapon, fires, ammo| takes(templates, weapon, fires, ammo),
        |id| {
            templates
                .names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| format!("{id:#x}"))
        },
    )
}

/// Pickup template `id`'s `Pickup` records, for the log: each
/// ammunition's name and amount.
pub(crate) fn records(templates: &Templates, id: u32) -> String {
    let records = templates
        .leavings
        .pickups
        .get(&id)
        .map_or(&[][..], |p| p.gives.as_slice());
    let named: Vec<String> = records
        .iter()
        .map(|&(ammo, n)| match templates.names.get(&ammo) {
            Some(name) => format!("{name} {n}"),
            None => format!("{ammo:#x} {n}"),
        })
        .collect();
    format!("[{}]", named.join(", "))
}

/// A helicopter of the scripts' with someone other than her aboard (the
/// pilot a `(Driver)` spawn comes with): it is in the air, flown by them,
/// and only shown while it flies a delivery ([`fly`]). Parked as a vehicle
/// of hers it stood on the ground where it was made, rotors turning, with
/// nothing to bump into (playtest 3, BUGLIST #3, alpha recording 7:42).
/// One she is in herself is still hers (inferred).
fn ai_flown(w: &m2_game::api::ScriptWorld, guid: u32, models: &[CatalogueEntry]) -> bool {
    models
        .iter()
        .any(|m| m.kind == m2_engine::vehicle::HELICOPTER)
        && w.objects
            .iter()
            .any(|(&g, o)| o.vehicle == Some(guid) && Some(g) != w.player)
}

/// The models one of the scripts' objects drives as
/// ([`template_models`]): by its name, else (a state layer's renamed
/// entity, the first contract's `VzaCon001_StartingBoat`) by the template
/// it is an instance of.
/// Whether the template an object or entity named `name` is an instance
/// of stands destroyed ([`Templates::ruined`]: its `Health` is 0): a
/// vehicle's ruin (`M151_Ruin`, `AMX30_RUIN`) has its intact twin's
/// model and physics, to stand wrecked.
pub(crate) fn is_ruin(templates: &Templates, name: &str) -> bool {
    templates
        .ids(name)
        .iter()
        .any(|id| templates.ruined.contains(id))
}

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
    fn the_contract_fanfare_waits_until_called_back() {
        let lua = mlua::Lua::new();
        lua.load("Hud = { Fanfare = {}, SubtitleBuffer = {} }")
            .exec()
            .unwrap();
        lua.load(HUD_CALLS).exec().unwrap();
        assert_eq!(waiting_fanfare(&lua), None);
        lua.load(
            r#"
            Hud.Fanfare:Create({ sType = "contract", fCallback = function() end })
            Hud.Fanfare:Commence({})
            Hud.SubtitleBuffer:AddMessage({ fCallback = function() end })
            "#,
        )
        .exec()
        .unwrap();
        // The fanfare's, not the subtitle's after it.
        assert_eq!(waiting_fanfare(&lua), Some(1));
        // Called back (as `MissionLink::hud_callback` does): no longer.
        lua.load("__m2_hud_callbacks[1] = nil").exec().unwrap();
        assert_eq!(waiting_fanfare(&lua), None);
    }

    #[test]
    fn health_and_small_boxes_are_walked_over_and_the_rest_worded() {
        use m2_engine::pickup::{AMMUNITION, HEALTH, Leavings, PickupInfo};
        let info = |kind, heals, gives: Vec<(u32, u32)>| PickupInfo {
            kind,
            radius: 1.0,
            ammo: None,
            rounds: 0,
            walked: gives.clone(),
            gives,
            heals,
            cue: None,
            effect: None,
        };
        // The PC data's `Health Pickup` (50), `Ammo Pickup (Small)` (3
        // rounds and 3 health), `Ammo Pickup (Bullet)` (20 rounds).
        let (health, small, bullet) = (0x8000_6ea9, 0x8000_a286, 0x8000_233e);
        let db = Templates {
            leavings: Leavings {
                pickups: HashMap::from([
                    (health, info(HEALTH, 50, Vec::new())),
                    (small, info(AMMUNITION, 3, vec![(0x8000_4375, 3)])),
                    (bullet, info(AMMUNITION, 0, vec![(0x8000_4375, 20)])),
                ]),
                ..Leavings::default()
            },
            ..Templates::default()
        };
        let mut combat = combat::Combat::default();
        let lying = |template, weapon: Option<Weapon>| Leaves {
            template,
            weapon,
            ..Leaves::default()
        };
        assert_eq!(offer(&db, &combat, &lying(health, None)), Offer::Walk);
        assert_eq!(offer(&db, &combat, &lying(small, None)), Offer::Walk);
        assert_eq!(offer(&db, &combat, &lying(bullet, None)), Offer::TakeAmmo);
        // A soldier's `Assault Rifle`: "Equip" over her carbine, "Take
        // Ammo" once she carries one.
        let ak = lying(0x8000_436f, Some(Weapon::Rifle));
        combat.rifle.template = Some(0x8000_af91);
        assert_eq!(offer(&db, &combat, &ak), Offer::Equip);
        combat.rifle.template = Some(0x8000_436f);
        assert_eq!(offer(&db, &combat, &ak), Offer::TakeAmmo);
    }

    #[test]
    fn a_soldiers_rifle_lies_loaded_and_gives_its_bullets_walked_over() {
        use m2_engine::pickup::{EQUIPMENT, Leavings, PickupInfo};
        use m2_engine::weapons::{Ammo, Arms, WeaponSpec};
        // The PC data's `Assault Rifle` (8000436f): an equipment record of
        // radius 1.5 and a record of 8 `Assault Rifle Bullet`, the ak47's
        // model. Its clip here is made up for the test.
        let (ak, bullet) = (0x8000_436f, 0x8000_0b01);
        let model = name_hash("global_weapon_ak47");
        let spec = WeaponSpec {
            template: ak,
            name: "Assault Rifle".into(),
            model: Some(model),
            class: Some(name_hash("Rifle")),
            semi: false,
            rate_of_fire: 600.0,
            clip: 30,
            reserve_clips: 8,
            pellets: 1,
            rounds_per_reload: 0,
            scatter: 1.0,
            ammo: Ammo {
                template: bullet,
                velocity: 300.0,
                max_velocity: 0.0,
                human_boost: 1.0,
                gravity: 0.0,
                damage: 10.0,
                damage_min: 10.0,
                falloff: (0.0, 0.0),
                key: None,
                blast: None,
                timer: None,
                timer_blast: None,
                homing: None,
                model: None,
                sticky: false,
            },
            lock_on: None,
            pickup: Some(8),
        };
        let db = Templates {
            infos: HashMap::from([(
                ak,
                TemplateInfo {
                    model: Some(model),
                    vehicle: None,
                    engine: None,
                    person: false,
                    human: false,
                },
            )]),
            names: HashMap::from([
                (ak, "Assault Rifle".to_owned()),
                (bullet, "Assault Rifle Bullet".to_owned()),
            ]),
            leavings: Leavings {
                pickups: HashMap::from([(
                    ak,
                    PickupInfo {
                        kind: EQUIPMENT,
                        radius: 1.5,
                        ammo: None,
                        rounds: 0,
                        gives: vec![(bullet, 8)],
                        walked: vec![(bullet, 8)],
                        heals: 0,
                        cue: None,
                        effect: None,
                    },
                )]),
                ammo: HashMap::from([(ak, bullet)]),
                ..Leavings::default()
            },
            arms: Arms {
                weapons: HashMap::from([(ak, spec.clone())]),
                ..Arms::default()
            },
            ..Templates::default()
        };
        // Taken up: the rifle, with its 8 clips of bullets.
        let gun = pickup(&db, ak).unwrap();
        assert_eq!(
            gun,
            Leaves {
                template: ak,
                model: Some(model),
                reach: 1.5,
                weapon: Some(Weapon::Rifle),
                gun: Some(ak),
                gives: Some((Weapon::Rifle, 240)),
                ..Leaves::default()
            }
        );
        // Walked over with a rifle that fires its bullets: they are hers.
        let mut combat = combat::Combat::default();
        combat.rifle = combat::Rifle::from_spec(&spec);
        assert_eq!(refill(&db, &combat, &gun).rifle, 240);
        // With a rifle that fires another round: nothing; it is taken up.
        combat.rifle.template = Some(0x8000_0c01);
        combat.rifle.ammo = None;
        combat.rifle.model_hash = None;
        assert_eq!(refill(&db, &combat, &gun), combat::Refill::default());
        // The PC data's `Ammo Pickup (Small)` (8000a286): a record for each
        // of 18 rounds (the shotgun's first, the rifles' 3 clips) and one
        // of 3 health. With the rifle back, walked over, its bullets go to
        // the rifle and it heals; once the rifle is full the log says why
        // it lies.
        let (small, shotgun) = (0x8000_a286, 0x8000_56a0);
        let mut db = db;
        let records = vec![(shotgun, 3), (bullet, 3)];
        db.leavings.pickups.insert(
            small,
            PickupInfo {
                kind: 1,
                radius: 1.0,
                ammo: Some(shotgun),
                rounds: 3,
                gives: records.clone(),
                walked: records,
                heals: 3,
                cue: None,
                effect: None,
            },
        );
        combat.rifle = combat::Rifle::from_spec(&spec);
        let small = pickup(&db, small).unwrap();
        assert_eq!((small.weapon, small.health), (None, 3.0));
        assert_eq!(refill(&db, &combat, &small).rifle, 90);
        combat.rifle_rounds.stored = combat.rifle.spare;
        let why = refusal(&db, &combat, &small);
        assert!(
            why.contains("Assault Rifle Bullet 3 to her Assault Rifle, full at 30+240 of 240"),
            "{why}"
        );
        assert!(
            why.contains("1 of 2 records for no weapon of hers"),
            "{why}"
        );
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
            walked: vec![(ammo, rounds)],
            heals: 0,
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
                (missile, "Rocket".to_owned()),
                (0x8000_0003, "rocket".to_owned()),
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
        // The launcher, loaded with its 3 clips; the rockets, whose rounds
        // go by the weapons she carries when she comes to them.
        let left = leaves(&db, "Supply Drop (GL) 0x40000001");
        assert_eq!(
            left,
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
                    ..Leaves::default()
                },
            ]
        );
        // Carrying the RPG (its template, the round it fires unread): 20
        // rockets, one a clip, by its model's link. The launcher is taken
        // up, not walked over.
        let mut combat = combat::Combat::default();
        combat.launcher.template = Some(rpg);
        assert_eq!(refill(&db, &combat, &left[1]).launcher, 20);
        assert_eq!(refill(&db, &combat, &left[0]), combat::Refill::default());
        // The launcher's own record names what refills it, whatever it
        // fires; a round of the same name as hers is hers, and a record
        // naming the weapon itself is for it.
        assert!(takes(&db, Some(launcher), Some(0x8000_0002), grenade));
        assert!(takes(&db, None, Some(0x8000_0003), missile));
        assert!(takes(&db, Some(rpg), None, rpg));
        assert!(!takes(&db, None, Some(0x8000_0002), missile));
        assert_eq!(records(&db, rockets), "[Rocket 20]");
        // Ammunition for no weapon of hers gives her nothing, but a weapon
        // comes loaded whatever its record names; a template that leaves
        // nothing, nothing at all.
        let mut db = db;
        db.leavings.ammo.clear();
        let left = leaves(&db, "Supply Drop (GL)");
        assert_eq!(left[0].gives, Some((Weapon::GrenadeLauncher, 18)));
        assert_eq!(refill(&db, &combat, &left[1]), combat::Refill::default());
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
    fn a_burnt_car_is_no_car_to_drive_and_a_vehicle_ruin_stands_wrecked() {
        let catalogue = [
            entry(1, "car", "global_veh_68valiant"),
            entry(2, "truck", "vz_veh_truck_m151"),
        ];
        let mut db = Templates::default();
        // `68Valiant_Ruin`: a burnt car of its own model, no vehicle
        // physics; `M151_Ruin`: the jeep's model and physics, `Health` 0.
        for (id, name, vehicle) in [
            (0x8000_6a30, "68Valiant_Ruin", None),
            (0x9000_01f6, "M151_Ruin", Some(VehicleKind::Car)),
        ] {
            let info = TemplateInfo {
                model: Some(id),
                vehicle,
                engine: None,
                person: false,
                human: false,
            };
            db.by_name.insert(name.to_ascii_lowercase(), info);
            db.infos.insert(id, info);
            db.names.insert(id, name.to_owned());
            db.ruined.insert(id);
        }
        // Its name's words (`68valiant`) found the Valiant to drive.
        assert_eq!(
            vehicle_models("68Valiant_Ruin 0x00133154", &catalogue).len(),
            1
        );
        assert!(template_models("68Valiant_Ruin 0x00133154", &db, &catalogue).is_empty());
        assert_eq!(
            template_models("M151_Ruin 0x000aef56", &db, &catalogue).len(),
            1
        );
        assert!(is_ruin(&db, "M151_Ruin 0x000aef56"));
        assert!(!is_ruin(&db, "M151 Softtop (VZ) 0x0012abcd"));
    }

    #[test]
    fn helicopters_their_pilots_fly_are_not_parked_for_her() {
        use m2_game::api::{Object, ScriptWorld};
        let heli = [entry(4, "helicopter", "vz_veh_helicopter_alouetteiii")];
        let tank = [entry(3, "tank", "vz_veh_tank_amx30")];
        let mut w = ScriptWorld::default();
        w.player = Some(1);
        w.insert(1, Object::new("Jennifer", [0.0; 3]));
        w.insert(
            10,
            Object::new("Alouette3 Transport (VZA Intro) (Driver)", [0.0; 3]),
        );
        w.insert(20, Object::new("AMX30", [0.0; 3]));
        // Empty, it is hers to take.
        assert!(!ai_flown(&w, 10, &heli));
        let mut pilot = Object::new("Pilot", [0.0; 3]);
        pilot.vehicle = Some(10);
        w.insert(11, pilot);
        assert!(ai_flown(&w, 10, &heli));
        // A crewed ground vehicle is still parked; one she flies is hers.
        let mut driver = Object::new("Pilot", [0.0; 3]);
        driver.vehicle = Some(20);
        w.insert(21, driver);
        assert!(!ai_flown(&w, 20, &tank));
        w.objects.remove(&11);
        w.objects.get_mut(&1).unwrap().vehicle = Some(10);
        assert!(!ai_flown(&w, 10, &heli));
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

    #[test]
    fn markers_show_from_near_to_far() {
        // The first contract's: 2 m over its target, from 5 m to 175 m.
        let m = MarkerView {
            at: Vec3::new(0.0, 2.0, 0.0),
            icon: "HUD_objective_deliverable".into(),
            colour: [255, 200, 0, 255],
            near: 5.0,
            far: 175.0,
            size: 32.0,
        };
        assert_eq!(m.label(), "deliverable");
        assert!(!m.shown_from(Vec3::new(3.0, 0.0, 0.0)));
        assert!(m.shown_from(Vec3::new(0.0, 0.0, 20.0)));
        assert!(m.shown_from(Vec3::new(0.0, 2.0, 175.0)));
        assert!(!m.shown_from(Vec3::new(0.0, 0.0, 200.0)));
    }
}
