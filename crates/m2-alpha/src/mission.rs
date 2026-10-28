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
use crate::ridehijack::{self, RideHijack};
use crate::scene::NOT_DRIVABLE;
use crate::support::{self, FALLBACK_AFTER, Impact, Kind};

/// The scripts run ten times a second.
const STEP: f32 = 0.1;
/// Seconds after the scripts start from a save that where the story
/// stands is logged: by then they have taken the save in (invented).
const LOAD_REPORT_SECONDS: f32 = 15.0;
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
    /// Discs on the ground (`Marker.AddDisc`): centre, radius, colour.
    pub discs: Vec<(Vec3, f32, [u8; 3])>,
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
    /// Its fade, metres: opaque within `near`, gone beyond `far` (see
    /// [`MarkerView::alpha_from`]).
    pub near: f32,
    pub far: f32,
    /// Its fade measures the distance over the ground only (the call's
    /// "2D only").
    pub flat: bool,
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

    /// How opaque it shows to someone at `from`, 0 to 1 (from the 360
    /// code, the marker renderer): 1 within `near`, 0 beyond `far`, and in
    /// between `(far² - d²) / (far² - near²)`, linear in the squared
    /// distance `d²`. A `flat` marker leaves height out of `d`.
    ///
    /// The game measures from the camera (from the 360 code: the
    /// renderer reads its view's camera-to-world translation), so `from`
    /// is the camera's position.
    pub fn alpha_from(&self, from: Vec3) -> f32 {
        let mut off = self.at - from;
        if self.flat {
            off.y = 0.0;
        }
        let d2 = off.length_squared();
        let (near2, far2) = (self.near * self.near, self.far * self.far);
        if d2 > far2 {
            0.0
        } else if d2 <= near2 {
            1.0
        } else {
            (far2 - d2) / (far2 - near2)
        }
    }
}

/// What `FactionZone` last told the HUD (`TrespassStateChange`): whether
/// she is trespassing, and the zone's faction as the HUD's radar names it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trespass {
    pub inside: bool,
    pub faction: Option<String>,
}

/// The radios ([`m2_engine::music_source`]): within a radio's reach (one
/// whose layer is in and whose playlist the scripts filled) its playlist
/// plays over the score, a song per visit in the playlist's order; out of
/// reach the score comes back. Invented: the song per visit (the engine's
/// own playlist order and its blend are not read) and the cut from the
/// score to it.
#[derive(Debug, Default)]
struct Radio {
    /// The score's cue as the scripts last set it.
    score: Option<String>,
    /// The radio playing over it (its entity, cue, place and reach).
    playing: Option<(u32, String, Vec3, f32)>,
    /// How many radio songs started.
    turn: usize,
}

impl Radio {
    /// What the music changes to, when it does.
    fn update(&mut self, game: &Game, mission: &Mission) -> Option<Option<String>> {
        let her = game.focus();
        let near = game.scene.world.as_ref().and_then(|w| {
            w.index
                .music_sources
                .iter()
                .filter(|s| game.scene.people.layer_in(Some(s.layer)))
                .map(|s| (s, s.at.distance(her)))
                .filter(|(s, d)| *d <= s.reach)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(s, _)| *s)
        });
        let playing = self.playing.as_ref().map(|r| r.0);
        match near {
            Some(s) if playing != Some(s.entity) => {
                let list = mission.playlist(s.playlist);
                if list.is_empty() {
                    return None;
                }
                let cue = list[self.turn % list.len()].clone();
                self.turn += 1;
                eprintln!(
                    "mission: radio {:#x} at {:.0} plays {cue} ({} in its playlist)",
                    s.entity,
                    s.at,
                    list.len()
                );
                self.playing = Some((s.entity, cue.clone(), s.at, s.reach));
                Some(Some(cue))
            }
            None if playing.is_some() => {
                eprintln!("mission: out of the radio's reach, the score again");
                self.playing = None;
                Some(self.score.clone())
            }
            _ => None,
        }
    }
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
    -- The engine's `Pda` library goes to the HUD's `PdaInterface` the
    -- same way: its database is the PDA's Info pages (the factions'
    -- attitudes, the log, the people, the score).
    ["Pda.Database"] = { "SetFactionAttitude", "AddLogEntry", "AddHelpEntry",
        "AddDossierEntry", "AddStatisticCategory", "AddStatisticEntry" },
}
for group, names in pairs(methods) do
    local g
    if group:sub(1, 4) == "Pda." then
        g = Pda and Pda[group:sub(5)]
    else
        g = Hud and Hud[group]
    end
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

/// The scripts' dialog boxes (`MrxGuiDialogBox.DisplayDialogBox(owner,
/// title, options, default, callback, callback args, ...)`, which
/// `MrxGui.DisplayDialogBox` names too): a box the game's HUD builds from
/// its widgets and works with the pad, so here each is a [`HUD_CALLS`]
/// call of its own group, `DialogBox` `Display` (title, option texts,
/// default and the plain arguments after), with a callback; the HUD's
/// answer is the option picked, and the callback runs as the box's own
/// `_CloseAndCallCallback` runs it, `callback(unpack(args), picked)`
/// ([`MissionLink::hud_callback`]). What the scripts get back stands for
/// the box: its `Close` (`MrxGuiDialogBox.Close`) is a `DialogBox`
/// `Close` call naming it. Run every tick: the scripts import the module
/// afresh as they go, over what was wrapped.
///
/// From the scripts (no executable read): the HQ's briefings put up their
/// menus this way (`MrxBriefing`'s `[Briefing.Bribe]`, `[Generic.Exit]`
/// list after `LTILibName.ChangeShellState(true)`), and the contract's
/// description with its Accept and Decline
/// (`MrxGuiDialogBox.DisplayScrollingDialogBox`, method `Scroll`), and a
/// recruit's wager its digit box (`MrxGui.DisplayNumericBox`, method
/// `Numeric`, with an accept and a cancel callback).
const DIALOG_BOXES: &str = r##"
__m2_dialog_wrapped = __m2_dialog_wrapped or setmetatable({}, { __mode = "k" })
local function plain(v)
    local t = type(v)
    if t == "string" or t == "number" or t == "boolean" then return v end
    return nil
end
local function texts(options)
    local out = {}
    if type(options) == "table" then
        for i, o in ipairs(options) do out[i] = plain(o) or "" end
    end
    return out
end
-- A box the HUD puts up for the call `seq`: what the scripts get back.
local function open(method, data, callback, args)
    __m2_hud_call_count = __m2_hud_call_count + 1
    local seq = __m2_hud_call_count
    __m2_hud_callbacks[seq] = { fCallback = callback, tCallbackData = args, dialog = true,
        method = method, data = data }
    table.insert(__m2_hud_calls, { seq = seq, group = "DialogBox", method = method,
        data = data, callback = type(callback) == "function" })
    if #__m2_hud_calls > 64 then table.remove(__m2_hud_calls, 1) end
    local box = { CustomData = {}, __m2_dialog = seq }
    box.Close = function()
        if __m2_hud_callbacks[seq] then __m2_hud_callbacks[seq] = nil end
        __m2_hud_call_count = __m2_hud_call_count + 1
        table.insert(__m2_hud_calls, { seq = __m2_hud_call_count, group = "DialogBox",
            method = "Close", data = { nBox = seq }, callback = false })
    end
    -- Whatever else the scripts ask of a box does nothing here.
    return setmetatable(box, { __index = function() return function() end end })
end
local function rest(...)
    local out, n = {}, select("#", ...)
    for i = 1, n do out[i] = plain((select(i, ...))) end
    return out
end
-- DisplayDialogBox(owner, title, options, default, callback, args, ...).
local function display(owner, title, options, default, callback, args, ...)
    return open("Display", { sTitle = plain(title) or "", tOptions = texts(options),
        nDefault = plain(default), tRest = rest(...) }, callback, args)
end
-- DisplayScrollingDialogBox(owner, text, callback, args, options, ...): the
-- contract's description with its Accept and Decline (a cancel picks 2).
local function scrolling(owner, text, callback, args, options, ...)
    return open("Scroll", { sText = plain(text) or "", tOptions = texts(options),
        tRest = rest(...) }, callback, args)
end
-- DisplayNumericBox(owner, title, text, prefix, suffix, start, min, max,
-- _, _, _, accept, accept args, cancel, cancel args, ...): the HQ's wager
-- (`[Briefing.ChangeWagerQuery]`). The HUD answers the value accepted, or
-- -1 - the value for a cancel; each callback gets its args and the value.
local function numeric(owner, ...)
    local a, n = { ... }, select("#", ...)
    local accept, aargs, cancel, cargs = a[11], a[12], a[13], a[14]
    local p = {}
    for i = 1, 10 do p[i] = plain(a[i]) end
    local tail = {}
    for i = 15, n do tail[i - 14] = plain(a[i]) end
    local function answer(v)
        local f, args = accept, aargs
        if v < 0 then f, args, v = cancel, cargs, -1 - v end
        local call = {}
        if type(args) == "table" then
            for i = 1, table.getn(args) do call[i] = args[i] end
        end
        table.insert(call, v)
        if type(f) == "function" then f(unpack(call)) end
    end
    return open("Numeric", { tArgs = p, nArgs = 10, tRest = tail, nRest = n - 14 }, answer, {})
end
__m2_dialog_wrapped[display] = true
__m2_dialog_wrapped[scrolling] = true
__m2_dialog_wrapped[numeric] = true
local function close(box)
    if type(box) == "table" and type(box.Close) == "function" and box.__m2_dialog then
        box.Close(box)
    end
end
__m2_dialog_wrapped[close] = true
-- The transit map: `MrxTransit.OpenInterface` asks the PDA, a widget of the
-- HUD's, to `OpenTransitInterface(zones, callback, data)`. Here it is a box
-- of method `Transit` with the zones offered by number; the answer is the
-- zone picked (0: shut), and the callback runs as the PDA's
-- `_InvokeCallback` runs it: callback(zone, picked, unpack(data)).
local function transit(pda, zones, callback, data)
    -- One map at a time, as the PDA's own `bActive` has it: OilCon002's
    -- seat event asks again each time it fires.
    if __m2_transit_box and __m2_hud_callbacks[__m2_transit_box] then return end
    local offered, list = {}, {}
    if type(zones) == "table" then
        for k, z in pairs(zones) do
            if type(z) == "table" and type(k) == "number" then
                offered[k] = { sName = plain(z.sName), nX = plain(z.nX), nY = plain(z.nY),
                    nSortOrder = plain(z.nSortOrder) }
                table.insert(list, k)
            end
        end
    end
    table.sort(list)
    local function answer(v)
        local call, n = { v, v ~= 0 }, 2
        if type(data) == "table" then
            for i = 1, table.maxn(data) do call[i + 2] = data[i] end
            n = n + table.maxn(data)
        end
        if type(callback) == "function" then callback(unpack(call, 1, n)) end
    end
    local box = open("Transit", { tZones = offered, tOffered = list }, answer, {})
    __m2_hud_callbacks[box.__m2_dialog].transit = list
    __m2_transit_box = box.__m2_dialog
end
-- The PDA as the scripts find it (`MrxGui.GetWidgetByNameAndOwner("PDA",
-- player)`): the HUD's, in its own Lua state, so a stand-in that opens the
-- transit map and does nothing else.
local pda = setmetatable({ OpenTransitInterface = transit },
    { __index = function() return function() end end })
local function lookup(original)
    local f = function(name, ...)
        local w = original(name, ...)
        if w == nil and name == "PDA" then return pda end
        return w
    end
    __m2_dialog_wrapped[f] = true
    return f
end
local function wrap(m, name, f)
    local d = rawget(m, name)
    if d ~= nil and not __m2_dialog_wrapped[d] then rawset(m, name, f) end
end
function __m2_wrap_dialogs()
    local m = rawget(_G, "MrxGuiDialogBox")
    if type(m) == "table" then
        wrap(m, "DisplayDialogBox", display)
        wrap(m, "DisplayScrollingDialogBox", scrolling)
        wrap(m, "Close", close)
    end
    local g = rawget(_G, "MrxGui")
    if type(g) == "table" then
        wrap(g, "DisplayDialogBox", display)
        wrap(g, "CloseDialogBox", close)
        wrap(g, "DisplayNumericBox", numeric)
        local o = rawget(g, "GetWidgetByNameAndOwner")
        if type(o) == "function" and not __m2_dialog_wrapped[o] then
            rawset(g, "GetWidgetByNameAndOwner", lookup(o))
        end
    end
    local nb = rawget(_G, "MrxGuiNumericBox")
    if type(nb) == "table" then
        wrap(nb, "DisplayNumericBox", numeric)
        wrap(nb, "Close", close)
    end
end
__m2_wrap_dialogs()
"##;

/// The factions whose attitude to the PMC the scripts have at
/// `Hostile` (`MrxFactionManager.GetAttitudeLabel(faction, "Pmc")`:
/// relations below -33 of -100..100, from the scripts' `_tAttitudes`),
/// by the manager's abbreviations; `None` before the manager runs.
fn hostile_factions(m: &Mission) -> Option<HashSet<&'static str>> {
    let hostile = factions_at(m, "hostile")?
        .into_iter()
        .chain(["Vza"])
        .collect();
    // The Venezuelans always fight her (the faction manager leaves them
    // out of reporting: "VZ, civilian, or reporting is disabled").
    Some(hostile)
}

/// The factions whose attitude to the PMC the scripts have at `label`
/// (`hostile`, `friendly`: `MrxFactionManager.GetAttitudeLabel(faction,
/// "Pmc")`), by the manager's abbreviations; `None` before the manager
/// runs.
fn factions_at(m: &Mission, label: &str) -> Option<HashSet<&'static str>> {
    let labels: Option<mlua::Table> = m
        .lua()
        .load(
            "local m = rawget(_G, 'MrxFactionManager') \
             if type(m) ~= 'table' or type(m.GetAttitudeLabel) ~= 'function' then return nil end \
             local out = {} \
             for _, f in ipairs({ 'All', 'Chi', 'Gur', 'Oil', 'Pir', 'Vza' }) do \
                 local ok, a = pcall(m.GetAttitudeLabel, f, 'Pmc') \
                 if ok then out[f] = type(a) == 'table' and a.sLabel or a end \
             end \
             return out",
        )
        .eval()
        .ok()
        .flatten();
    let labels = labels?;
    Some(
        ["All", "Chi", "Gur", "Oil", "Pir", "Vza"]
            .into_iter()
            .filter(|f| {
                labels
                    .get::<Option<String>>(*f)
                    .ok()
                    .flatten()
                    .is_some_and(|l| l.eq_ignore_ascii_case(label))
            })
            .collect(),
    )
}

/// How near her a friendly soldier hears her horn, metres (invented).
const ALLIES_HEAR: f32 = 20.0;

/// She honked: soldiers of the factions friendly to her (their attitude
/// to the PMC `Friendly`, `MrxFactionManager.GetAttitudeLabel`) within
/// [`ALLIES_HEAR`] jump into her vehicle's free gunner seats, nearest
/// first, and ride with her (out of play, as followers do: getting out
/// with her, [`ride_along`]). From the game's text ("Making a faction
/// happy has benefits: Their faction units will jump in your vehicle if
/// you honk your horn"; `Tutorial.VehicleHorn.Attract`: "Press [cancel]
/// to call allies into your vehicle"); that they take the seats with a
/// gun ([`crate::combat::armed_seats`] but the driver's) is inferred, the
/// reach invented. Riding, they man the guns ([`crate::combat`]'s gunners).
fn call_allies(game: &mut Game, mission: &mut Mission, riding: Option<u32>) {
    let Some(p) = game.scene.player.as_ref() else {
        return;
    };
    let her = p.character.position;
    let Some(v) = p.driving.and_then(|i| game.scene.vehicles.get(i)) else {
        return;
    };
    let free =
        (crate::combat::armed_seats(v).len() - 1).saturating_sub(game.scene.people.gunners());
    if free == 0 {
        return;
    }
    let friends = factions_at(mission, "friendly").unwrap_or_default();
    if friends.is_empty() {
        return;
    }
    let mut near: Vec<(f32, u32)> = game
        .scene
        .people
        .npcs
        .iter()
        .filter(|n| n.alive && !n.soldier && n.faction.is_some_and(|f| friends.contains(f)))
        .filter_map(|n| {
            let d = n.character.position.distance(her);
            (d <= ALLIES_HEAR).then_some((d, n.spawn?))
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    let Some(world) = &mut game.scene.world else {
        return;
    };
    for (_, g) in near.into_iter().take(free) {
        if game.scene.people.board(&mut world.physics, g, true, her) {
            if let Some(v) = riding {
                mission.seat_in(g, v);
            }
            game.scene.people.man_guns(g);
            eprintln!("people: {g:08x} hears her horn and jumps in her vehicle");
        }
    }
}

/// The faction of the vehicle she rides, by the faction manager's
/// abbreviation (`MrxFactionManager.GetFactionAbbrev` of
/// `MrxUtil.GetFaction(Vehicle.GetFromRider(her))`, as
/// `GetPerceivedFaction` takes a disguised rider's): `None` on foot, in a
/// civilian's or a PMC vehicle, or before the scripts run.
fn vehicle_faction(m: &Mission) -> Option<&'static str> {
    let abbrev: Option<String> = m
        .lua()
        .load(
            "local u, f = rawget(_G, 'MrxUtil'), rawget(_G, 'MrxFactionManager') \
             if type(u) ~= 'table' or type(f) ~= 'table' then return nil end \
             local p = Player.GetLocalCharacter() \
             local v = p and Vehicle.GetFromRider(p) \
             if v == nil then return nil end \
             local ok, faction = pcall(u.GetFaction, v) \
             if not ok or faction == nil then return nil end \
             local ok2, a = pcall(f.GetFactionAbbrev, faction) \
             if ok2 and type(a) == 'string' then return a end \
             return nil",
        )
        .eval()
        .ok()
        .flatten();
    let abbrev = abbrev?;
    ["All", "Chi", "Gur", "Oil", "Pir", "Vza"]
        .into_iter()
        .find(|f| f.eq_ignore_ascii_case(&abbrev))
}

/// Runs the callback of dialog box `seq` ([`DIALOG_BOXES`]) for option
/// `picked`: `callback(unpack(args), picked)`, as the game's box's own
/// `_CloseAndCallCallback` calls it.
fn choice_code(seq: u64, picked: i64) -> String {
    format!(
        "local t = __m2_hud_callbacks[{seq}]\n\
         __m2_hud_callbacks[{seq}] = nil\n\
         if t and type(t.fCallback) == 'function' then\n\
             local a = {{}}\n\
             if type(t.tCallbackData) == 'table' then\n\
                 for i = 1, table.getn(t.tCallbackData) do a[i] = t.tCallbackData[i] end\n\
             end\n\
             table.insert(a, {picked})\n\
             t.fCallback(unpack(a))\n\
         end"
    )
}

/// People this near a marker's target are listed with it, metres; story
/// characters' spawns this near the player.
const MARK_NEAR: f32 = 6.0;
/// A witness this near what she did reports it, metres; he gets through
/// this many seconds after he is offered (his call-in line, measured at
/// 3.7 s for `OCSoldier01.Reporting.CallIn03`), and is given up on after
/// this many if the faction manager hasn't finished with him (invented:
/// the game's own choice of reporter and his call aren't read).
const REPORT_WITNESS: f32 = 80.0;
const REPORT_CALL_IN: f32 = 4.0;
const REPORT_GIVE_UP: f32 = 30.0;
const STORY_REACH: f32 = 450.0;
/// The player's verify photo clips, by hero ([`Command::DoAction`]), and
/// how long an action without a clip lasts (invented).
const VERIFY_CLIPS: [u32; 3] = [0x5ae2_cef2, 0x0053_81c4, 0xb448_382e];
const ACTION_SECONDS: f32 = 2.0;
/// How near the extraction helicopter she loads a prisoner, metres
/// (invented).
const LOAD_REACH: f32 = 8.0;

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
    /// The mission it is for, when the scripts say: the marker's
    /// (`"GurCon053"`), else that of an objective on the PDA about the
    /// same object.
    pub mission: Option<MarkFor>,
    /// Its marker is drawn in the secondary objectives' colour
    /// (`MrxUtil.GetSecondaryObjectiveRgb`: the jobs' and bounties'),
    /// not the primary ones' (a contract's).
    pub secondary: bool,
}

/// The mission a [`Mark`] is for, as the scripts name it.
#[derive(Clone, Debug, PartialEq)]
pub enum MarkFor {
    /// A marker's mission id (`"GurCon053"`).
    Id(String),
    /// A PDA objective's index in the mission data.
    Index(i64),
}

enum State {
    Loading(Receiver<Result<GameData, String>>),
    Running(Box<Mission>),
    Failed(String),
}

/// The mission scripts and what they want shown.
pub struct MissionLink {
    state: State,
    /// The contract's unmarked action targets last logged
    /// ([`contract_marks`](Self::contract_marks)).
    acts_logged: std::cell::RefCell<Vec<u32>>,
    /// The flow's movies wait for the game to play them
    /// ([`Mission::hold_movies`]); otherwise they end at once.
    movies: bool,
    /// With [`movies`](Self::movies), the scripts' skip mode ends once the
    /// start has run ([`Mission::end_skip_mode`]; `--skip-mode-on` keeps
    /// it). Surveyed on the game's data (round 25): 7 scripts ask, 36
    /// times; the mission flow (27 of them) skips its fades and voice
    /// sequences with it, the PMC HQ its interior. Both contracts' flows
    /// were seen to finish with it off.
    end_skip: bool,
    /// The player has subtitles on.
    subtitles: bool,
    /// The player has the tutorial tips on.
    tutorials: bool,
    /// Time not yet given to the scripts.
    lag: f32,
    log: Option<BufWriter<File>>,
    /// The newest objective message, its text readable, and its age.
    message: Option<(ObjectiveMessage, String, f32)>,
    drivers: Vec<(String, String)>,
    /// The story skipped to this contract (`--skip-to`), and whether to
    /// its briefing ([`Mission::set_skip_mission`]).
    skip_to: Option<(String, bool)>,
    /// The job `--skip-to` names, followed as she takes it on.
    job: Option<crate::jobs::JobRun>,
    /// Skipped there: ticks left to find where the scripts' new game
    /// spawns her ([`spawn_point`]) and put her there.
    spawn_tries: u32,
    /// A save to start from ([`with_save`](Self::with_save)), until the
    /// scripts start.
    saved: Option<Box<crate::save::SaveGame>>,
    /// Seconds since the scripts started from a save, until where the
    /// story stands is logged ([`LOAD_REPORT_SECONDS`]).
    since_load: Option<f32>,
    /// The scripts put the player somewhere the island has no way to hold
    /// her (in the start's boat, at sea): she goes ashore once the ground
    /// there has loaded. Tries left.
    ashore: u32,
    /// The action hijack of a scripts' vehicle under way in the scripts
    /// ([`crate::ridehijack`]).
    hijack: Option<RideHijack>,
    /// The tutorial line up, with its key tokens.
    tutorial: Option<String>,
    /// Blasts since the scripts last ran, and support strikes' bombs.
    blasts: Vec<Vec3>,
    bombs: Vec<Vec3>,
    /// The contract's destroy objectives' targets, marked or not, and when
    /// they are next read again (seconds).
    unmarked: Vec<u32>,
    unmarked_for: f32,
    vehicles: Fleet,
    /// Seconds she has been out of her vehicle with followers aboard
    /// ([`ride_along`]).
    out_for: f32,
    /// The armies' pursuit helicopters ([`crate::reinforce`]), read from
    /// the data once, and those under way.
    squads: Option<HashMap<&'static str, crate::reinforce::Squad>>,
    reinforcements: crate::reinforce::Reinforcements,
    /// Seconds since the scripts said the contract was completed.
    completed: Option<f32>,
    /// How many times the scripts have said a contract was completed.
    contracts: u32,
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
    /// The radio it plays from, if a radio's ([`Self::music_at`]).
    music_at: Option<(Vec3, f32)>,
    /// The radios near her ([`Radio`]).
    radio: Radio,
    /// The human state each placed person was last told to the scripts
    /// in ([`crate::npc::Down`]), by guid.
    states: HashMap<u32, &'static str>,
    /// The reverb to switch to, if the scripts changed it.
    reverb: Option<Option<m2_engine::reverb::ReverbPreset>>,
    /// The music region she was last in, by entity (`Some(None)`: none).
    music_region: Option<Option<u32>>,
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
    /// The place the scripts sent her to (`Ai.Goal` `MoveTo` on the
    /// player: an outpost counts her among its rushers), and where it is:
    /// her goal is done once she stands within [`GOAL_REACH`] of it.
    player_goal: Option<(u32, Vec3)>,
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
            acts_logged: Default::default(),
            end_skip: false,
            subtitles: true,
            tutorials: true,
            lag: 0.0,
            log,
            message: None,
            drivers,
            skip_to: None,
            job: None,
            spawn_tries: 0,
            saved: None,
            since_load: None,
            ashore: 0,
            hijack: None,
            blasts: Vec::new(),
            tutorial: None,
            bombs: Vec::new(),
            unmarked: Vec::new(),
            unmarked_for: 0.0,
            vehicles: Fleet::default(),
            out_for: 0.0,
            squads: None,
            reinforcements: crate::reinforce::Reinforcements::default(),
            completed: None,
            contracts: 0,
            speed: 0.0,
            keys: Default::default(),
            lines: Vec::new(),
            hushed: Vec::new(),
            sounds: Vec::new(),
            fades: Vec::new(),
            music: None,
            music_at: None,
            radio: Radio::default(),
            states: HashMap::new(),
            reverb: None,
            music_region: None,
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

    /// Every item of the scripts' support catalogue, readable, for the
    /// cheat menu ([`crate::cheats`]).
    pub fn cheat_support(&self) -> Vec<crate::cheats::SupportLine> {
        let State::Running(m) = &self.state else {
            return Vec::new();
        };
        let mut out: Vec<crate::cheats::SupportLine> = m
            .support_items()
            .into_iter()
            .map(|i| crate::cheats::SupportLine {
                name: m.text(&i.name),
                key: i.key,
                kind: i.kind,
            })
            .collect();
        out.sort_by(|a, b| (&a.kind, &a.name).cmp(&(&b.kind, &b.name)));
        out
    }

    /// The cheat menu's support: one of item `key` stocked free into her
    /// support menu, and called in at once if `call`, as `--support`
    /// does. The recruits that bring it are freed first, so a second call
    /// isn't kept waiting. Whether the scripts took the call (true when
    /// only stocked).
    pub fn cheat_support_item(&mut self, key: &str, call: bool) -> bool {
        self.free_recruits();
        let State::Running(m) = &mut self.state else {
            return false;
        };
        m.stock_support(key, 1, 0.0, 0.0);
        let took = !call || m.use_support(key);
        let lines = m.take_log();
        self.write(lines);
        self.stock_look = 0;
        took
    }

    /// The cheat menu's money: `cash` and `fuel` given the scripts' way.
    pub fn cheat_money(&mut self, cash: f64, fuel: f64) {
        if let State::Running(m) = &mut self.state {
            m.give_resources(cash, fuel, "[Generic.Pickups]");
            let lines = m.take_log();
            self.write(lines);
            self.stock_look = 0;
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

    /// Where the story stands, once the scripts run
    /// ([`Mission::contracts`]).
    pub fn contracts(&self) -> Option<m2_game::savedata::Contracts> {
        let State::Running(m) = &self.state else {
            return None;
        };
        m.contracts()
            .map_err(|e| eprintln!("mission: the contracts' states: {e}"))
            .ok()
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

    /// The player's subtitles and tutorial tips, from now on (the pause
    /// menu's options).
    pub fn set_options(&mut self, subtitles: bool, tutorials: bool) {
        self.subtitles = subtitles;
        self.tutorials = tutorials;
        if let State::Running(m) = &mut self.state {
            m.set_subtitles(subtitles);
            m.set_tutorials(tutorials);
        }
    }

    /// Skips the story to this contract as the game's developer switch
    /// does (`--skip-to`): every story contract before it is completed as
    /// the flow unlocks it.
    pub fn with_skip_to(self, skip_to: Option<(String, bool)>) -> Self {
        let job = skip_to
            .as_ref()
            .and_then(|(id, _)| crate::jobs::JobRun::for_skip(id));
        Self {
            skip_to,
            job,
            ..self
        }
    }

    /// The job she was skipped to, and where it stands.
    pub fn job(&self) -> Option<&crate::jobs::JobRun> {
        self.job.as_ref()
    }

    /// With a job to follow: she is put this far from its nearest target
    /// once it is tracked (`--job-near`).
    pub fn with_job_near(mut self, near: Option<f32>) -> Self {
        if let Some(job) = &mut self.job {
            job.near = near;
        }
        self
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

    /// The HUD's dialog box for the mission's call `seq` ([`DIALOG_BOXES`])
    /// closed on option `picked`: its callback runs with its arguments and
    /// then the option, as the game's box calls it.
    pub fn hud_choice(&mut self, seq: u64, picked: i64) {
        let State::Running(m) = &mut self.state else {
            return;
        };
        let code = choice_code(seq, picked);
        let line = match m.run(&code, "hud choice") {
            Ok(_) => format!("hud choice {seq}: option {picked}"),
            Err(e) => format!("hud choice {seq}: {e}"),
        };
        eprintln!("mission: dialog box {seq}: option {picked} picked");
        self.write([line]);
    }

    /// What the pause menu's Medevac reads (`MrxPlayer.CanMedEvac` and
    /// `GetMedEvacCost`), as the mission's scripts answer now: whether she
    /// can be flown out (alive, not in transit, a hijack, the HQ or a
    /// locked state) and its cost. `None` before they run.
    pub fn medevac_state(&self) -> Option<(bool, f64)> {
        let State::Running(m) = &self.state else {
            return None;
        };
        m.lua()
            .load(
                "local p = rawget(_G, 'MrxPlayer') \
                 if type(p) ~= 'table' then return nil end \
                 local ok, can = pcall(p.CanMedEvac) \
                 local okc, cost = pcall(p.GetMedEvacCost) \
                 return ok and can == true, okc and tonumber(cost) or 0",
            )
            .eval::<(Option<bool>, Option<f64>)>()
            .ok()
            .and_then(|(can, cost)| Some((can?, cost.unwrap_or(0.0))))
    }

    /// The pause menu's Medevac was confirmed: the scripts' own
    /// `MrxPlayer.MedEvac` runs (a contract under way is cancelled, else
    /// she goes to the sickbay).
    pub fn medevac(&mut self) {
        let State::Running(m) = &mut self.state else {
            return;
        };
        let line = match m.run("MrxPlayer.MedEvac()", "medevac") {
            Ok(_) => "medevac: MrxPlayer.MedEvac() ran".to_owned(),
            Err(e) => format!("medevac: {e}"),
        };
        eprintln!("mission: {line}");
        self.write([line]);
    }

    /// The pause menu's scripts changed the score (its `mu_shell_01`, then
    /// silence on leaving): the mission's scripts start their state's
    /// piece again at their next transition, and a radio going quiet
    /// comes back to this.
    pub fn music_heard(&mut self, cue: Option<String>) {
        self.radio.score.clone_from(&cue);
        if let State::Running(m) = &mut self.state {
            m.music_heard(cue);
        }
    }

    /// Whether a dialog box the scripts put up ([`DIALOG_BOXES`]) is still
    /// waiting for a pick: the menu keys go to it, not to her.
    pub fn dialog_open(&self) -> bool {
        let State::Running(m) = &self.state else {
            return false;
        };
        m.lua()
            .load(
                "for _, t in pairs(__m2_hud_callbacks or {}) do \
                 if type(t) == 'table' and t.dialog then return true end end return false",
            )
            .eval::<bool>()
            .unwrap_or(false)
    }

    /// The dialog box the scripts have up and waiting for a pick
    /// ([`DIALOG_BOXES`]): its call's number, its kind (`Display` for a
    /// menu, `Scroll` for a contract's terms with Accept), its title or
    /// text, and its options as text ids.
    pub fn open_dialog(&self) -> Option<OpenDialog> {
        let State::Running(m) = &self.state else {
            return None;
        };
        let (seq, method, text, options) = m
            .lua()
            .load(
                "for seq, t in pairs(__m2_hud_callbacks or {}) do \
                   if type(t) == 'table' and t.dialog and type(t.data) == 'table' then \
                     local o = {} \
                     for i, v in ipairs(t.data.tOptions or {}) do o[i] = tostring(v) end \
                     return seq, t.method or '', t.data.sTitle or t.data.sText or '', o \
                   end \
                 end",
            )
            .eval::<(
                Option<u64>,
                Option<String>,
                Option<String>,
                Option<Vec<String>>,
            )>()
            .ok()?;
        Some(OpenDialog {
            seq: seq?,
            method: method.unwrap_or_default(),
            text: text.unwrap_or_default(),
            options: options.unwrap_or_default(),
        })
    }

    /// The starters' talks on offer (the context actions whose prompt is
    /// [`TALK_PROMPT`]): each one's object and where it stands, a person
    /// where one is that object, else its place.
    pub fn talk_offers(&self, game: &Game) -> Vec<crate::autoplay::ActionAt> {
        let State::Running(mission) = &self.state else {
            return Vec::new();
        };
        mission
            .context_actions()
            .into_iter()
            .filter(|(_, _, text, _)| text.eq_ignore_ascii_case(TALK_PROMPT))
            // A starter riding a prop (the HQ's, at her desk) is where the
            // scripts last put her until the game seats her in play
            // (`crate::hijack`, which loads ~50 s after a save in headless
            // runs); till then her talk is at the building's origin, over
            // no floor (story run B3: she walked there and fell).
            .filter(|(g, _, _, _)| {
                let rides_prop = mission
                    .world()
                    .borrow()
                    .objects
                    .get(g)
                    .and_then(|o| o.vehicle)
                    .is_some_and(|v| game.scene.props.get(v).is_some());
                let seated_in_play = game.scene.riders.iter().any(|r| r.guid == Some(*g));
                talk_ready(rides_prop, seated_in_play)
            })
            .filter_map(|(g, at, _, reach)| {
                let person = game
                    .scene
                    .people
                    .npcs
                    .iter()
                    .find(|n| n.spawn == Some(g))
                    .map(|n| n.character.position);
                Some((g, person.or_else(|| at.map(Vec3::from))?, reach))
            })
            .collect()
    }

    /// Every context action on offer, for the log: its object, its
    /// prompt's text id and that text as shown, and where it stands (if
    /// known).
    pub fn context_list(&self) -> Vec<(u32, String, String, Option<Vec3>)> {
        let State::Running(mission) = &self.state else {
            return Vec::new();
        };
        mission
            .context_actions()
            .into_iter()
            .map(|(g, at, id, _)| (g, mission.text(&id), id, at.map(Vec3::from)))
            .collect()
    }

    /// The flow's contracts on offer now (not its jobs), in the scripts'
    /// order, while none is active: what a player off a contract goes to a
    /// starter for.
    pub fn contracts_on_offer(&self) -> Vec<String> {
        match self.contracts() {
            Some(c) if c.active.is_empty() => c
                .offered
                .into_iter()
                .filter(|id| is_flow_contract(id))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The transit map the scripts have open ([`DIALOG_BOXES`]' `Transit`
    /// box), if any: its call's number and the zones it offers, each by
    /// number with where its landing zone is (`MrxTransit`'s
    /// `_tLandingZones`, its `uLocation1`).
    pub fn transit_open(&self) -> Option<(u64, Vec<(i64, Vec3)>)> {
        let State::Running(m) = &self.state else {
            return None;
        };
        let (seq, zones) = m
            .lua()
            .load(
                "for seq, t in pairs(__m2_hud_callbacks or {}) do \
                   if type(t) == 'table' and type(t.transit) == 'table' then \
                     local m = rawget(_G, 'MrxTransit') \
                     local all = type(m) == 'table' and type(m._tLandingZones) == 'table' \
                       and m._tLandingZones or {} \
                     local out = {} \
                     for _, k in ipairs(t.transit) do \
                       local z, x, y, w = all[k], 0, 0, 0 \
                       if type(z) == 'table' and z.uLocation1 then \
                         x, y, w = Object.GetPosition(z.uLocation1) \
                       end \
                       table.insert(out, { k, x or 0, y or 0, w or 0 }) \
                     end \
                     return seq, out \
                   end \
                 end",
            )
            .eval::<(Option<u64>, Option<Vec<Vec<f32>>>)>()
            .ok()?;
        Some((
            seq?,
            zones?
                .into_iter()
                .filter_map(|z| {
                    Some((
                        *z.first()? as i64,
                        Vec3::new(*z.get(1)?, *z.get(2)?, *z.get(3)?),
                    ))
                })
                .collect(),
        ))
    }

    /// Where the scripts' markers stand: on their targets.
    pub fn marked_places(&self) -> Vec<Vec3> {
        let State::Running(m) = &self.state else {
            return Vec::new();
        };
        let w = m.world().borrow();
        w.hud
            .markers
            .values()
            .filter_map(|m| m.target)
            .filter_map(|g| w.objects.get(&g))
            .map(|o| Vec3::from(o.position))
            .collect()
    }

    /// Has the scripts' movies (`01_AOA_J`, `02_AOB_J`) wait for the game:
    /// [`take_movie`](Self::take_movie) hands them out.
    pub fn with_movies(self, movies: bool) -> Self {
        Self { movies, ..self }
    }

    /// Starts the scripts from `save` rather than a new game, as loading
    /// it does: her place, weapons and the world's wrecks and dead are put
    /// back first, then the scripts are handed their own save data
    /// ([`Mission::load_save`]): the contracts done and on offer, her PMC,
    /// the factions. A save without script data (from before it was kept)
    /// starts a new game with her cash and stock made up to the save's.
    pub fn with_save(mut self, save: Option<crate::save::SaveGame>) -> Self {
        if let Some(save) = save {
            self.set_save(save);
        }
        self
    }

    /// As [`with_save`](Self::with_save), for a link already made whose
    /// scripts have not started (a front end holding play until a game is
    /// chosen). Whether it was taken: once the scripts run it is too late,
    /// and the save is not taken.
    pub fn set_save(&mut self, save: crate::save::SaveGame) -> bool {
        if !matches!(self.state, State::Loading(_)) {
            return false;
        }
        self.saved = Some(Box::new(save));
        true
    }

    /// Ends the scripts' skip mode once the start has run, when the movies
    /// wait for the game (unless `--skip-mode-on`).
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

    /// The prompts of the hijack the scripts play on a vehicle of theirs
    /// ([`crate::ridehijack`]), while one is under way.
    pub fn hijack_prompts(&self) -> Option<Vec<m2_game::hijack::Minigame>> {
        let (Some(_), State::Running(m)) = (&self.hijack, &self.state) else {
            return None;
        };
        Some(RideHijack::prompts(m))
    }

    /// The pad during that hijack: the buttons ([`m2_game::hijack::BUTTONS`]
    /// numbers) pressed since the last time, and those held.
    pub fn hijack_pad(&mut self, pressed: &[u32], held: &[u32]) {
        let (Some(h), State::Running(m)) = (&mut self.hijack, &mut self.state) else {
            return;
        };
        for &b in pressed {
            RideHijack::press(m, b);
        }
        h.hold(m, held);
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
        // The scripts take a save's data in as the world streams in: where
        // the story stands, and where she is, is logged once they have.
        if let Some(t) = &mut self.since_load {
            *t += dt;
            if *t >= LOAD_REPORT_SECONDS {
                self.since_load = None;
                if let Some(c) = self.contracts() {
                    let at = game.focus();
                    eprintln!(
                        "load: {LOAD_REPORT_SECONDS} s on, the scripts hold {c}; she is at ({:.0}, {:.0}, {:.0})",
                        at.x, at.y, at.z
                    );
                }
            }
        }
        let State::Running(mission) = &mut self.state else {
            return;
        };
        let focus = game.focus();
        let mut lap = crate::spans::Lap::new();
        // The music region she is in: the free-play score follows it.
        if let Some(world) = &game.scene.world {
            let region =
                m2_engine::sound_regions::music_at(&world.index.sound_regions, focus.x, focus.z);
            let entity = region.map(|r| r.entity);
            if self.music_region != Some(entity) {
                self.music_region = Some(entity);
                mission.music_region(region.and_then(|r| r.faction));
                eprintln!(
                    "music: region {} at ({:.0}, {:.0}), score {}",
                    region.map_or("none", |r| r.name.as_str()),
                    focus.x,
                    focus.z,
                    mission.music_faction()
                );
            }
        }
        lap.mark("scripts: music");
        // Deliveries fly on; the scripts hear of each load let go (but the
        // pursuits' helicopters', [`crate::reinforce`]).
        let scene = &mut game.scene;
        if let Some(world) = &mut scene.world {
            let releases = scene
                .flights
                .step(dt, &mut world.physics, &mut scene.vehicles, focus);
            for r in self.reinforcements.take_releases(releases) {
                for &g in &r.dropped {
                    self.vehicles.landing.insert(g, mission.now() + LANDING);
                }
                self.vehicles.dropped.extend(r.dropped);
                mission.arrived(r.pilot, r.to.to_array());
                self.counts.arrived += 1;
            }
            for (pilot, home) in scene.flights.take_home() {
                mission.arrived(pilot, home.to_array());
            }
        }
        lap.mark("scripts: deliveries");
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
        lap.mark("scripts: crates");
        // People the scripts sent somewhere got there, and she got where
        // they sent her: their goals are done.
        for (spawn, at) in game.scene.people.take_route_ends() {
            mission.arrived(spawn, at.to_array());
        }
        if let Some((to, at)) = self.vehicles.player_goal
            && game.scene.player.as_ref().is_some_and(|p| {
                p.driving.is_none() && p.character.position.distance(at) < GOAL_REACH
            })
        {
            self.vehicles.player_goal = None;
            eprintln!("mission: she got to {to:#x}, where the scripts sent her");
            mission.arrived(PLAYER, at.to_array());
        }
        // Skipped to a later contract: the world opened at the first
        // contract's start, but the scripts' new game spawns her where
        // they name (the contract's start, or by the PMC's offices); there
        // once that point has loaded.
        if self.spawn_tries > 0 {
            self.spawn_tries -= 1;
            if let Some((name, at)) = spawn_point(mission)
                && let Some(p) = game.scene.player.as_mut()
            {
                self.spawn_tries = 0;
                if p.character.position.distance(at) > SPAWN_NEAR {
                    p.drop_at(at);
                    p.driving = None;
                    p.chase_target = None;
                    eprintln!("mission: she starts at the scripts' spawn point {name} ({at:.1})");
                }
            }
        }
        // The job she follows, tracked: put near its nearest target once
        // (`--job-near`, a test shortcut past the drive there).
        if let Some(job) = &mut self.job
            && let Some((name, at)) = job.start_near(mission, game.focus())
            && let Some(p) = game.scene.player.as_mut()
        {
            p.drop_at(at);
            p.driving = None;
            p.chase_target = None;
            eprintln!("jobs: test shortcut: she starts at {at:.1}, near {name}");
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
        lap.mark("scripts: blasts");
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
        lap.mark("scripts: posts");
        // The scripts' context actions (the HQ's exits, a starter's talk):
        // the one she stands at is offered, and taken with the use key.
        if let Some(g) = game.context_taken.take() {
            if game.aboard == Some(g) {
                leave_ride(game, mission, g);
            } else if rides(game, mission).iter().any(|r| r.0 == g) {
                // One with an action hijack in the data is hijacked
                // ([`crate::ridehijack`]), else boarded.
                match ridehijack::entrance(game, mission, g)
                    .and_then(|(t, e)| RideHijack::start(mission, g, &t, &e))
                {
                    Some(h) => {
                        game.input_locked = true;
                        self.hijack = Some(h);
                    }
                    None => board_ride(game, mission, g),
                }
            } else {
                let text = mission.context_text(g).unwrap_or_default();
                mission.take_context_action(g);
                // The verify objective's prompts no script answers: the engine
                // subdues a knocked-down target, and picks a subdued one up
                // (from the scripts: `MrxTaskObjectiveVerify` offers them and
                // waits on the states; who acts on them is inferred).
                if let Some(n) = game
                    .scene
                    .people
                    .npcs
                    .iter_mut()
                    .find(|n| n.spawn == Some(g))
                {
                    if text.eq_ignore_ascii_case("[ContextAction.Subdue]") && n.subdue() {
                        eprintln!("mission: she subdues {g:#x}");
                    } else if text.eq_ignore_ascii_case("[ContextAction.Carry]") && n.carry(true) {
                        eprintln!("mission: she picks {g:#x} up");
                    }
                }
            }
        }
        lap.mark("scripts: context taken");
        // Whom she carries rides her shoulder (invented: over her right
        // shoulder, facing behind her).
        if let Some(p) = game.scene.player.as_ref() {
            let (at, facing) = (p.character.position, p.facing);
            for n in game
                .scene
                .people
                .npcs
                .iter_mut()
                .filter(|n| n.down == Some(crate::npc::Down::Carried))
            {
                let right = Vec3::new(facing.cos(), 0.0, -facing.sin());
                n.character.position = at + right * 0.25 + Vec3::Y * 1.3;
                n.facing = facing + std::f32::consts::PI;
            }
        }
        // Carrying a prisoner to the extraction helicopter (the verify
        // objective's `ExtractionHelicopter`, landed for her): within
        // [`LOAD_REACH`] of it she loads them aboard, the seat event the
        // objective waits on (`ObjectInSeat { target, "ExtractionHelicopter",
        // "a", "e" }`), and they leave with it. Inferred: no script seats
        // the prisoner, so the engine does it; the reach is invented.
        if let Some(her) = game.scene.player.as_ref().map(|p| p.character.position)
            && let Some(k) = game
                .scene
                .people
                .npcs
                .iter()
                .position(|n| n.down == Some(crate::npc::Down::Carried))
            && let Some(g) = game.scene.people.npcs[k].spawn
            && let Some((heli, _)) = mission
                .labelled("ExtractionHelicopter")
                .into_iter()
                .find(|(_, at)| Vec3::from(*at).distance(her) <= LOAD_REACH)
        {
            eprintln!("mission: she loads {g:#x} into the extraction helicopter {heli:#x}");
            game.scene.people.npcs.remove(k);
            // Gone with it: their spawn places nobody again.
            game.scene.people.killed.insert(g);
            mission.set_human_state(g, "InVehicle.Subdued");
            self.states.insert(g, "InVehicle.Subdued");
            mission.seat_in(g, heli);
        }
        lap.mark("scripts: carry");
        // Keyed actions run out: `HumanActionComplete` fires.
        for a in &mut game.actions {
            a.2 -= dt;
        }
        let (done, left): (Vec<_>, Vec<_>) = std::mem::take(&mut game.actions)
            .into_iter()
            .partition(|a| a.2 <= 0.0);
        game.actions = left;
        for (g, action, _) in done {
            eprintln!("mission: {g:#x}'s {action} ends");
            mission.action_finished(g);
        }
        // Whom she carries, for `Human.IsCarrying`.
        let carried = game
            .scene
            .people
            .npcs
            .iter()
            .find(|n| n.down == Some(crate::npc::Down::Carried))
            .and_then(|n| n.spawn);
        mission.set_carrying(PLAYER, carried);
        lap.mark("scripts: actions");
        // The placed people's human states, as they change, for the
        // scripts' `HumanStateTransition` events.
        for n in &game.scene.people.npcs {
            let Some(g) = n.spawn else { continue };
            let state = n.human_state();
            if self.states.get(&g).copied().unwrap_or("Upright.Idle") != state {
                self.states.insert(g, state);
                mission.set_human_state(g, state);
            }
        }
        // What the clips the scripts hold people in met: their events
        // (a briefing frame's start and end) and their voice lines.
        for (g, met) in game.raw.take_met() {
            match met {
                crate::raw::Met::Event(e) => {
                    eprintln!("raw-clip: {g:#x} meets {e}");
                    mission.animation_event(g, &e);
                }
                crate::raw::Met::Line(cue) => mission.clip_line(g, &cue),
            }
        }
        lap.mark("scripts: states");
        // Her infractions against the factions, for their witnesses to
        // report: the first one of a faction not already reporting, a
        // witness of theirs near it (not the victim) is offered to the
        // faction manager ([`m2_game::report`]).
        let infractions = game.scene.people.take_infractions();
        let hurt_someone = !infractions.is_empty();
        for inf in infractions {
            let label = crate::npc::faction_label(inf.faction);
            mission.record_infraction(label, inf.kind);
            if inf.faction == "Vza" || game.reporters.iter().any(|r| r.1 == inf.faction) {
                continue;
            }
            let witness = game
                .scene
                .people
                .npcs
                .iter()
                .filter(|n| n.alive && n.faction == Some(inf.faction) && n.spawn != inf.victim)
                .filter_map(|n| {
                    let d = n.character.position.distance(inf.at);
                    (d <= REPORT_WITNESS).then_some((d, n.spawn?))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, g)| g);
            if let Some(g) = witness {
                if mission.begin_report(g, label) {
                    eprintln!(
                        "mission: {g:#x} of {label} goes to report her ({})",
                        inf.kind
                    );
                    game.reporters.push((g, inf.faction, 0.0));
                } else {
                    eprintln!("mission: {g:#x} of {label} was turned down as a reporter");
                }
            }
        }
        lap.mark("scripts: infractions");
        // Her vehicle disguise ([`crate::disguise`]): the faction of the
        // vehicle she rides, as the scripts have it; the faction's
        // soldiers take her for one of theirs while it holds, and the
        // scripts and the HUD's gauge hear how it stands.
        let honks = game.take_honks();
        if honks > 0 {
            mission.honk();
            eprintln!(
                "horn: she honks; the faction gates hear her for {} s",
                m2_game::api::HONK_HEARD
            );
            call_allies(game, mission, self.vehicles.riding);
        }
        let riding = game.scene.player.as_ref().and_then(|p| p.driving);
        let faction = riding.and_then(|_| vehicle_faction(mission));
        let her = game.scene.player.as_ref().map(|p| p.character.position);
        let near = match (faction, her) {
            (Some(f), Some(at)) => game
                .scene
                .people
                .npcs
                .iter()
                .filter(|n| n.alive && n.faction == Some(f))
                .filter(|n| n.character.position.distance(at) <= crate::disguise::NEAR)
                .count(),
            _ => 0,
        };
        let enabled = mission.world().borrow().vehicle_disguise;
        let state = game.disguise.update(
            crate::disguise::Tick {
                enabled,
                vehicle: riding,
                faction,
                honks,
                near,
                hurt_someone: hurt_someone && riding.is_some(),
            },
            dt,
        );
        mission.world().borrow_mut().disguise = state;
        game.scene.people.set_disguise(game.disguise.holds());
        // Reporters: each calls in, then gets through (state 1) and the
        // faction manager's own timer finishes his report; one killed
        // first, or that it never finishes with, is stopped (state 2).
        let mut through = Vec::new();
        let mut stopped = Vec::new();
        for (g, _, t) in &mut game.reporters {
            let before = *t;
            *t += dt;
            let alive = game
                .scene
                .people
                .npcs
                .iter()
                .any(|n| n.spawn == Some(*g) && n.alive);
            if !alive || *t >= REPORT_GIVE_UP {
                stopped.push(*g);
            } else if before < REPORT_CALL_IN && *t >= REPORT_CALL_IN {
                through.push(*g);
            }
        }
        for g in through {
            eprintln!("mission: reporter {g:#x} got through");
            mission.report_through(g);
        }
        for g in stopped {
            eprintln!("mission: reporter {g:#x} was stopped before he reported");
            mission.stop_report(g);
            game.reporters.retain(|r| r.0 != g);
        }
        for (g, done) in mission.take_report_ends() {
            game.reporters.retain(|r| r.0 != g);
            eprintln!(
                "mission: the faction manager is done with reporter {g:#x} (reported: {done})"
            );
        }
        // The radios near her play over the score, from where they stand.
        if let Some(cue) = self.radio.update(game, mission) {
            self.music = Some(cue);
            self.music_at = self.radio.playing.as_ref().map(|r| (r.2, r.3));
        }
        // A pursuit brings its army's helicopter with soldiers.
        if self.squads.is_none()
            && let Some(world) = &game.scene.world
        {
            let w = mission.world().borrow();
            let squads = crate::reinforce::squads(
                &w.template_passengers,
                &w.template_drivers,
                &world.index.template_db,
            );
            let mut names: Vec<String> = squads
                .iter()
                .map(|(f, s)| format!("{f} {} ({} soldiers)", s.template, s.soldiers.len()))
                .collect();
            names.sort();
            eprintln!("reinforcements: pursuit helicopters {names:?}");
            self.squads = Some(squads);
        }
        if let (Some(squads), Some(her)) = (
            &self.squads,
            game.scene.player.as_ref().map(|p| p.character.position),
        ) {
            let mut pursuing: Vec<&'static str> =
                game.scene.people.pursuit.keys().copied().collect();
            pursuing.sort_unstable();
            let due = self
                .reinforcements
                .step(&pursuing, squads, &mut game.scene.flights, her, dt);
            for (g, template, at) in due {
                let model = game
                    .scene
                    .world
                    .as_ref()
                    .and_then(|w| w.index.template_db.get(&template))
                    .and_then(|t| t.model);
                let at = ground_under(game, at);
                if game.scene.people.bring_scripted(g, &template, model, at) {
                    self.reinforcements.brought(g);
                }
            }
        }
        // A pursuit run out: the scripts read level 0 again.
        let pursued = !game.scene.people.pursuit.is_empty();
        if game.pursued && !pursued {
            mission.pursuit_over();
        }
        game.pursued = pursued;
        lap.mark("scripts: reporters");
        // The factions' attitudes, once a second: whose soldiers fight her.
        game.attitude_wait -= dt;
        if game.attitude_wait <= 0.0 {
            game.attitude_wait = 1.0;
            if let Some(hostile) = hostile_factions(mission) {
                game.scene.people.set_hostile(hostile);
            }
        }
        // Whether she swims, for `Human.IsSwimming`.
        let swimming = game
            .scene
            .player
            .as_ref()
            .is_some_and(|p| p.driving.is_none() && p.swimming());
        mission.set_swimming(PLAYER, swimming);
        // A scene's camera shot, from where its two stand now.
        if let Some(shot) = game.shot.as_mut() {
            let w = mission.world().borrow();
            let at = |g: &u32| w.objects.get(g).map(|o| Vec3::from(o.position));
            if let (Some(b), Some(t)) = (at(&shot.base), at(&shot.target)) {
                shot.place(b, t);
            }
        }
        lap.mark("scripts: attitudes");
        // The dialog boxes, as the scripts may have imported their module
        // afresh ([`DIALOG_BOXES`]).
        if let Ok(Some(wrap)) = mission
            .lua()
            .globals()
            .get::<Option<mlua::Function>>("__m2_wrap_dialogs")
            && let Err(e) = wrap.call::<()>(())
        {
            eprintln!("mission: dialog boxes: {e}");
        }
        lap.mark("scripts: dialog wrap");
        // Riders of the layers' props (the HQ interior's starter): drawn in
        // their seats by `crate::hijack`, which says where those are; the
        // scripts hear it, so what is offered by her (her `Talk`) is where
        // she sits, not at the building's origin.
        for (g, at, yaw) in std::mem::take(&mut game.scene.prop_rider_spots) {
            // Taken out of the seat since the spot was found (a briefing's
            // `Vehicle.Exit` and `SetTransformToObject`): the scripts' place
            // stands, or Fiona went back to her desk for the talk.
            let seated = mission
                .world()
                .borrow()
                .objects
                .get(&g)
                .is_some_and(|o| o.vehicle.is_some());
            if seated {
                mission.place(g, at.to_array(), yaw);
            }
        }
        let props = &game.scene.props;
        game.scene.prop_crews = mission
            .crews_aboard()
            .into_iter()
            .filter(|(g, _)| props.get(*g).is_some())
            .collect();
        lap.mark("scripts: crews aboard");
        // Out of their seats (a starter at her briefing spot), they stand
        // where the scripts have them; back in, they sit again.
        let scene = &mut game.scene;
        if let Some(world) = scene.world.as_mut() {
            for (_, crew) in &scene.prop_crews {
                for (_, rider, _) in crew {
                    scene.people.unstand_scripted(&mut world.physics, *rider);
                }
            }
            let here = scene.player.as_ref().map(|p| p.character.position);
            for (rider, from, who, at, yaw) in mission.crews_on_foot() {
                let at = Vec3::from(at);
                let near = here.is_some_and(|h| h.distance(at) <= crate::npc::DRAW_DISTANCE);
                if scene.props.get(from).is_none() || !near {
                    continue;
                }
                let model = world.index.template_db.get(&who).and_then(|t| t.model);
                scene
                    .people
                    .stand_scripted(rider, &who, model, None, at, yaw);
            }
        }
        lap.mark("scripts: crews on foot");
        let on_foot = game
            .scene
            .player
            .as_ref()
            .filter(|p| p.driving.is_none())
            .map(|p| p.character.position);
        // Riding in a scripts' helicopter its pilot flies: she is where
        // it is, and the use key takes her out again.
        if let Some(g) = game.aboard {
            let at = mission
                .world()
                .borrow()
                .objects
                .get(&g)
                .filter(|o| o.alive)
                .map(|o| Vec3::from(o.position));
            match (at, game.scene.player.as_mut()) {
                (Some(at), Some(p)) => {
                    p.character.position = at;
                    p.character.velocity = Vec3::ZERO;
                }
                _ => leave_ride(game, mission, g),
            }
        }
        // A person's action (a verified HVT's) is where they lie in play,
        // not where the scripts last placed them.
        let actions: Vec<_> = mission
            .context_actions()
            .into_iter()
            .map(|(g, at, text, reach)| {
                let person = game
                    .scene
                    .people
                    .npcs
                    .iter()
                    .find(|n| n.spawn == Some(g))
                    .map(|n| n.character.position.to_array());
                (g, person.or(at), text, reach)
            })
            .collect();
        let offered = match game.aboard {
            Some(g) => Some((g, RIDE_OUT.to_owned())),
            None => on_foot
                .and_then(|at| nearest_context(&actions, at))
                .map(|(g, id)| (g, mission.text(&id)))
                .or_else(|| {
                    let at = on_foot?;
                    rides(game, mission)
                        .into_iter()
                        .find(|(_, p)| {
                            p.with_y(0.0).distance(at.with_y(0.0)) <= RIDE_REACH
                                && (p.y - at.y).abs() <= CONTEXT_RISE
                        })
                        .map(|(g, _)| (g, RIDE_IN.to_owned()))
                }),
        };
        if offered != game.context {
            match &offered {
                Some((g, text)) => eprintln!("context: {text:?} offered at {g:#x}"),
                None => eprintln!("context: none offered"),
            }
            game.context = offered;
        }
        lap.mark("scripts: context offer");
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
            // The scripts' vehicles' load beds, where they count the goods
            // on them (`Object.GetHardpointPosition(truck, "HP_Truckbed")`).
            if let Some(world) = game.scene.world.as_ref() {
                for v in &game.scene.vehicles {
                    if let (Some(g), Some(id)) = (v.guid, v.id)
                        && let Some(bed) = game.bed(v.spec.model)
                        && let Some(t) = world.physics.vehicle_transform(id)
                    {
                        mission.set_hardpoint(
                            g,
                            m2_assets::name_hash(crate::game::BED_NODE),
                            t.transform_point3(bed).to_array(),
                        );
                    }
                }
            }
            // What rides on a vehicle is where it rides, for the scripts
            // that count it there (`Pg.GetObjectsInArea`).
            for (g, at, yaw) in game.scene.props.cargo_places() {
                mission.place(g, at.to_array(), yaw);
            }
            for (g, at, yaw) in game.scene.flights.places(&game.scene.vehicles) {
                mission.place(g, at.to_array(), yaw);
            }
            self.vehicles.sync(
                game,
                mission,
                self.job.as_ref().map_or(&[][..], |j| j.bounty_quarry()),
            );
            ride_along(game, mission, self.vehicles.riding, &mut self.out_for, STEP);
            // Those she killed die by her hand: a job's hero-only bounty
            // counts them.
            let hero = mission.world().borrow().player;
            let hers = game.scene.people.her_kills();
            for guid in game.scene.people.all_killed() {
                match hero.filter(|_| hers.contains(&guid)) {
                    Some(h) => mission.kill_by(guid, h),
                    None => mission.kill(guid),
                }
            }
            // The scripts' people are where they stand in play: the dead where
            // their bodies lie (a verify prompt is offered over the object; a
            // job's targets run from their spawns before they fall: jobs round
            // 3 found the prompt left at the spawn), the living where they went
            // (OilCon001's executive, following her to his drop-off, stayed at
            // his spawn for the scripts' delivery check: round 24).
            for n in &game.scene.people.npcs {
                // (The living by their own object only, not a spawner's place.)
                if let Some(g) = n.spawn
                    && (!n.alive || mission.world().borrow().objects.contains_key(&g))
                {
                    mission.place(g, n.character.position.to_array(), n.facing);
                }
            }
            // Targets to destroy: soldiers stand at those that are spawns,
            // and blasts close to the rest (gates, buildings) destroy them.
            // Those the contract's destroy objectives name with no marker
            // over them count too (PmcCon031's 50 columns), checked a few
            // times a second: the query walks the scripts' tasks.
            self.unmarked_for -= STEP;
            if self.unmarked_for <= 0.0 {
                self.unmarked_for = UNMARKED_EVERY;
                let now = contract_destroy_targets(mission);
                if now.len() != self.unmarked.len() {
                    let w = mission.world().borrow();
                    let alive = now
                        .iter()
                        .filter(|g| w.objects.get(g).is_some_and(|o| o.alive))
                        .count();
                    eprintln!(
                        "mission: the contract's destroy targets now {} ({alive} standing)",
                        now.len()
                    );
                }
                self.unmarked = now;
            }
            let targets: Vec<(u32, Vec3)> = {
                let w = mission.world().borrow();
                let marked = w
                    .hud
                    .markers
                    .values()
                    .filter(|m| m.icon.contains("destroy"))
                    .filter_map(|m| m.target);
                let mut seen = HashSet::new();
                marked
                    .chain(self.unmarked.iter().copied())
                    .filter(|g| seen.insert(*g))
                    .filter_map(|g| {
                        let o = w.objects.get(&g).filter(|o| o.alive)?;
                        Some((g, Vec3::from(o.position)))
                    })
                    // And a bounty's quarry, marked for her as these are.
                    .chain(
                        self.job
                            .iter()
                            .flat_map(|j| j.bounty_quarry())
                            .filter_map(|&g| {
                                let o = w.objects.get(&g).filter(|o| o.alive)?;
                                Some((g, Vec3::from(o.position)))
                            }),
                    )
                    .collect()
            };
            game.scene.people.mission_soldiers = targets.iter().map(|t| t.0).collect();
            // Those standing as models (the gates) fall only to a support
            // strike, which the scripts give her; should they not, one is
            // given her anyway after a while.
            // Not those whose template's damage row takes rounds or blasts
            // (the ranges' statues, targets and columns, PmcCon031-034):
            // they wear down as the data says, and breaking destroys them.
            let fragile = |g: u32| {
                let (Some((p, _)), Some(w)) = (game.scene.props.get(g), game.scene.world.as_ref())
                else {
                    return false;
                };
                let db = &w.index.template_db;
                let makeup = crate::breakables::Makeup::of(db, p.template);
                db.durability
                    .get(&p.template)
                    .is_some_and(|d| !d.invincible && d.health > 0.0)
                    && (makeup.bullet > 0.0 || makeup.explosion > 0.0)
            };
            let sturdy: HashSet<u32> = targets
                .iter()
                .map(|t| t.0)
                .filter(|&g| game.scene.props.get(g).is_some() && !fragile(g))
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
                // Broken to its wreck (by rounds or a blast's wear).
                let wrecked = props.get(*g).is_some_and(|(p, _)| p.wrecked);
                if bombed || blasted || wrecked {
                    if bombed {
                        eprintln!("mission: {g:#x} falls to the bomb");
                    }
                    // Her blast, ram or the bomb she called in.
                    match hero {
                        Some(h) => mission.kill_by(*g, h),
                        None => mission.kill(*g),
                    }
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
                game.support.stock = crate::spans::time("scripts: stock", || stocked(mission));
                self.resources = Some(mission.resources());
                self.stock_look = STOCK_LOOK;
            }
            self.stock_look -= 1;
            let (flown, burst) =
                crate::spans::time("scripts: strikes", || report_strikes(game, mission));
            self.counts.flown += flown;
            self.counts.burst += burst;
            mission.set_player_reserve("C4", game.combat.charges);
            let spare = game.combat.rifle_rounds.stored;
            mission.set_player_reserve(CARBINE, spare.div_ceil(combat::RIFLE_CLIP));
            crate::spans::time("scripts: look", || look(game, mission, &mut self.sighted));
            crate::spans::time("scripts: lua", || mission.tick(f64::from(STEP)));
            // The scripts' own interface runs too: its animations finish
            // and their callbacks run (`MrxGuiCinematic.ShowMovie` plays
            // a cutscene's movie only once its fade-in animation is done,
            // and the contract waits on the movie's end: PmcCon002's
            // 10_BRV_J and PmcCon003's 12_CAR_J never ended, rounds 23
            // and 28).
            m2_game::gui_runtime::tick(mission.lua(), f64::from(STEP));
            lines.extend(mission.take_log());
            let commands = mission.take_commands();
            if let Some(h) = &mut self.hijack
                && !h.follow(game, mission, &commands)
            {
                self.hijack = None;
                game.input_locked = false;
            }
            for c in commands {
                // Seated by the scripts in the driver's seat of one of theirs
                // with an action hijack (PmcCon004's `DockAttack`:
                // `Vehicle.Enter(mi35, hero, "d")`, 50 m from it): the
                // hijack plays ([`crate::ridehijack`]; inferred, the
                // engine's own seating is not read).
                if let Command::EnterVehicle {
                    guid,
                    vehicle,
                    seat,
                    ..
                } = &c
                    && *guid == PLAYER
                    && seat.eq_ignore_ascii_case("d")
                    && self.hijack.is_none()
                    && let Some((t, e)) = ridehijack::entrance(game, mission, *vehicle)
                    && let Some(h) = RideHijack::start(mission, *vehicle, &t, &e)
                {
                    game.input_locked = true;
                    self.hijack = Some(h);
                    continue;
                }
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
                    Command::CueSound { at: None, cue } => {
                        // The scripts' countdown ticks each minute (`MrxTimer`'s cue):
                        // autoplay hurries while it runs.
                        if cue.starts_with("ui_HUD_Timer_") && cue != "ui_HUD_Timer_End" {
                            game.countdown_tick = true;
                        }
                        self.sounds.push((cue.clone(), false));
                    }
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
                    // The score, unless a radio near her plays over it.
                    Command::Music(cue) => {
                        self.radio.score = cue.clone();
                        if self.radio.playing.is_none() {
                            self.music = Some(cue.clone());
                            self.music_at = None;
                        }
                    }
                    Command::Reverb(preset) => self.reverb = Some(preset.clone()),
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
                        // The first weapon named is the gun in the
                        // rifle's place (the first contract's `Carbine`,
                        // PmcCon001's `Assault Rifle`), as its template
                        // says; else the carbine.
                        let gun = weapons
                            .first()
                            .and_then(|w| game.scene.arms()?.named(w))
                            .map(|spec| game.combat.stock(spec));
                        match gun {
                            Some(rifle) => {
                                eprintln!(
                                    "mission: her gun is the {} ({} a clip)",
                                    rifle.name, rifle.clip
                                );
                                game.combat.scripted_rifle(rifle);
                            }
                            None => game.combat.carbine_only(),
                        }
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
                        } else if weapon.eq_ignore_ascii_case(&game.combat.rifle.name)
                            && !weapon.eq_ignore_ascii_case(CARBINE)
                        {
                            // Clips of the gun she was handed (`Weapon.
                            // SetReserveAmmo` of `GetMaxReserveAmmo`: the
                            // Assault Rifle's 7), as for the carbine.
                            game.combat.rifle_rounds.stored = *rounds * game.combat.rifle.clip;
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
                    self.contracts += 1;
                    // The contract's locked pursuits end with it, and so do
                    // the helicopters they sent (invented: they landed in
                    // the villa she had just taken).
                    game.scene.people.clear_scripted_pursuit();
                    self.reinforcements.call_off();
                    eprintln!(
                        "mission: contract done: its pursuits end, their helicopters turn back"
                    );
                }
            }
            if let Some(line) = tutorial(mission) {
                self.tutorial = line;
            }
            crate::spans::time("scripts: markers", || {
                follow_markers(game, mission, &mut self.marked, &mut self.markers_logged);
            });
            if let Some(job) = &mut self.job {
                for line in job.update(mission, game.focus(), STEP) {
                    eprintln!("{line}");
                    lines.push(line);
                }
            }
            crate::spans::time("scripts: props", || break_props(game, mission));
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
                if let Err(e) = mission.run(DIALOG_BOXES, "dialog boxes") {
                    self.write([format!("dialog boxes: {e}")]);
                }
                if let Err(e) = mission.run(TRESPASS, "trespass") {
                    self.write([format!("trespass: {e}")]);
                }
                if let Err(e) = mission.run(RESTORED_SUPPORT, "restored support") {
                    self.write([format!("restored support: {e}")]);
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
                mission.set_tutorials(self.tutorials);
                if let Some((to, briefing)) = &self.skip_to {
                    mission.set_skip_mission(to, *briefing);
                    self.write([format!("skip to: {to} (briefing {briefing})")]);
                    self.spawn_tries = ASHORE_TRIES;
                }
                let loaded = self.saved.is_some();
                let pmc = self.saved.take().and_then(|save| {
                    let mut save = *save;
                    let pmc = save.pmc.take();
                    let scripts = save.scripts.take();
                    game.load(save);
                    match scripts {
                        Some(text) => {
                            mission.load_save(text);
                            None
                        }
                        None => pmc,
                    }
                });
                mission.start();
                if self.movies && self.end_skip {
                    // The movies wait for the game, so the scripts' skip
                    // mode has done its part.
                    mission.end_skip_mode();
                } else if self.movies {
                    eprintln!("mission: the scripts' skip mode stays on (--skip-mode-on)");
                }
                let lines = mission.take_log();
                for c in mission.take_commands() {
                    if obey(game, &mission, &mut self.vehicles, &c) {
                        self.ashore = ASHORE_TRIES;
                    }
                }
                self.write(lines);
                self.state = State::Running(Box::new(mission));
                if let Some(pmc) = pmc {
                    self.restore(&pmc);
                }
                if loaded {
                    self.since_load = Some(0.0);
                }
            }
            Err(e) => self.fail(format!("could not start the scripts: {e}")),
        }
    }

    fn fail(&mut self, why: String) {
        eprintln!("mission: {why}");
        self.write([why.clone()]);
        self.state = State::Failed(why);
    }

    /// The context actions on offer: on what, and where it stands (a
    /// person where they are in play, else where the scripts have them).
    pub fn offers(&self, game: &Game) -> Vec<crate::autoplay::ActionAt> {
        let State::Running(mission) = &self.state else {
            return Vec::new();
        };
        mission
            .context_actions()
            .into_iter()
            .filter_map(|(g, at, _, reach)| {
                let person = game
                    .scene
                    .people
                    .npcs
                    .iter()
                    .find(|n| n.spawn == Some(g))
                    .map(|n| n.character.position);
                Some((g, person.or_else(|| at.map(Vec3::from))?, reach))
            })
            .collect()
    }

    /// The first context action on offer whose prompt reads `text` (as
    /// shown, any case): its object and where that stands. For
    /// `--context`.
    pub fn context_spot(&self, text: &str) -> Option<(u32, Vec3)> {
        let State::Running(mission) = &self.state else {
            return None;
        };
        let text = text.to_ascii_lowercase();
        mission
            .context_actions()
            .into_iter()
            .filter(|(_, _, id, _)| mission.text(id).to_ascii_lowercase().contains(&text))
            .find_map(|(g, at, _, _)| Some((g, Vec3::from(at?))))
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

    /// Where the music last switched to plays from, and how far it
    /// carries: a radio's place and reach; `None` for the score.
    pub fn music_at(&self) -> Option<(Vec3, f32)> {
        self.music_at
    }

    /// The reverb to switch to, if the scripts changed it.
    pub fn take_reverb(&mut self) -> Option<Option<m2_engine::reverb::ReverbPreset>> {
        self.reverb.take()
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

    /// Whether the scripts let her grapple (`Player.SetGrappleEnabled`).
    pub fn grapple_enabled(&self) -> bool {
        matches!(&self.state, State::Running(m) if m.grapple_enabled())
    }

    /// Tells the scripts whether she is on her grappling hook's line.
    pub fn set_grappling(&mut self, on: bool) {
        if let State::Running(m) = &mut self.state {
            m.set_grappling(on);
        }
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

    /// The scripts' objects carrying label `label`, and where each is.
    pub fn labelled(&self, label: &str) -> Vec<(u32, Vec3)> {
        let State::Running(m) = &self.state else {
            return Vec::new();
        };
        m.labelled(label)
            .into_iter()
            .map(|(g, at)| (g, Vec3::from(at)))
            .collect()
    }

    /// Runs `code` in the mission's scripts (a test harness's line).
    pub fn run_lua(&mut self, code: &str) {
        if let State::Running(m) = &mut self.state
            && let Err(e) = m.run(code, "harness")
        {
            eprintln!("mission: harness code failed: {e}");
        }
    }

    /// A guid the scripts keep in global `name` (a test driver's
    /// `__m2_test_target`), if the mission runs and it is one.
    pub fn global_guid(&self, name: &str) -> Option<u32> {
        let State::Running(m) = &self.state else {
            return None;
        };
        let v: mlua::Value = m.lua().globals().raw_get(name).ok()?;
        m2_game::events::as_guid(&v)
    }

    /// The markers the scripts have up, with what they are over; then the
    /// targets of the objectives on the map (`"objective"`) and of the
    /// radar's blips (`"radar"`) no marker is over.
    pub fn marks(&self) -> Vec<Mark> {
        let State::Running(mission) = &self.state else {
            return Vec::new();
        };
        let w = mission.world().borrow();
        let on_pda = |g: u32| {
            w.hud
                .objectives
                .values()
                .find(|o| o.target == Some(g))
                .map(|o| MarkFor::Index(o.mission))
        };
        let secondary = mission
            .lua()
            .load(
                "local u = rawget(_G, 'MrxUtil')                  if type(u) ~= 'table' or type(u.GetSecondaryObjectiveRgb) ~= 'function' then return nil end                  return u.GetSecondaryObjectiveRgb()",
            )
            .eval::<(Option<f64>, Option<f64>, Option<f64>)>()
            .ok()
            .and_then(|(r, g, b)| Some([r? as u8, g? as u8, b? as u8]));
        let mark = |g: u32, kind: &str, for_mission: Option<MarkFor>, colour: Option<[u8; 3]>| {
            let o = w.objects.get(&g)?;
            Some(Mark {
                target: g,
                kind: kind.to_owned(),
                name: o.name.clone(),
                at: Vec3::from(o.position),
                alive: o.alive,
                mission: for_mission.or_else(|| on_pda(g)),
                secondary: colour.is_some() && colour == secondary,
            })
        };
        let mut out: Vec<Mark> = w
            .hud
            .markers
            .values()
            .filter_map(|m| {
                mark(
                    m.target?,
                    m.icon.trim_start_matches("HUD_objective_"),
                    (m.mission != "nil").then(|| MarkFor::Id(m.mission.clone())),
                    Some([m.colour[0], m.colour[1], m.colour[2]]),
                )
            })
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
                && let Some(m) = mark(g, kind, None, None)
            {
                out.push(m);
            }
        }
        out
    }

    /// The contract she is on (`MrxPlayState.GetCurrentMission`, its id
    /// without `Mission`) and its index in the mission data, if any.
    pub fn contract(&self) -> Option<(String, i64)> {
        let State::Running(mission) = &self.state else {
            return None;
        };
        mission
            .lua()
            .load(CURRENT_CONTRACT)
            .set_name("=current contract")
            .eval::<(Option<String>, Option<i64>)>()
            .ok()
            .and_then(|(id, index)| Some((id?, index?)))
    }

    /// The marks ([`marks`](Self::marks)) a player on the contract goes
    /// for: the objectives' (not the map's icons over the faction HQs'
    /// doors, `HUD_Outpost_GR`); of those, the contract's own while any
    /// is still to be done. (The jobs the flow unlocks put up markers of
    /// their own all over the island, which a player on a contract
    /// passes by.)
    pub fn contract_marks(&self) -> Vec<Mark> {
        let marks: Vec<Mark> = self
            .marks()
            .into_iter()
            .filter(|m| !m.kind.starts_with("HUD_"))
            .collect();
        let Some((id, index)) = self.contract() else {
            // Off a contract, on the job she was skipped to: its marks.
            if let (Some(job), State::Running(mission)) = (&self.job, &self.state)
                && job.view.active
            {
                return job.marks(marks, mission);
            }
            return marks;
        };
        let ours = |m: &Mark| match &m.mission {
            Some(MarkFor::Id(i)) => i.eq_ignore_ascii_case(&id),
            Some(MarkFor::Index(i)) => *i == index,
            None => false,
        };
        // The markers put up for the contract by name come first. The
        // objectives on the PDA count by the contract's index, which the
        // jobs' objectives shown during it share (round 20: PmcCon003's
        // "Get inside the PMC!" passed over for OilJob008's targets).
        let named =
            |m: &Mark| matches!(&m.mission, Some(MarkFor::Id(i)) if i.eq_ignore_ascii_case(&id));
        // A verify marker stays to do once its person is dead: the body is
        // verified (round 13: General Peng shot, then left for the jobs).
        let to_do = |m: &Mark| m.alive || m.kind == "verify";
        // A race running for the contract: its checkpoint now up, before
        // the markers named for the contract (PmcCon016 round 1: she went
        // for the pylons that only add time, and the race ran out at its
        // first checkpoint).
        if let Some(g) = self.contract_race() {
            let gate = marks.iter().find(|m| m.target == g).cloned().or_else(|| {
                let State::Running(mission) = &self.state else {
                    return None;
                };
                let w = mission.world().borrow();
                let o = w.objects.get(&g)?;
                Some(Mark {
                    target: g,
                    kind: "deliverable".to_owned(),
                    name: o.name.clone(),
                    at: Vec3::from(o.position),
                    alive: true,
                    mission: Some(MarkFor::Index(index)),
                    secondary: false,
                })
            });
            if let Some(gate) = gate {
                // The contract's timer targets near the gate are blown up
                // on the way: each adds time, which the race's own start
                // time doesn't cover (PmcCon016: 75 s for 27 gates, 15 s a
                // pylon; from the scripts). The reach is invented.
                let mut out: Vec<Mark> = marks
                    .iter()
                    .filter(|m| {
                        m.kind == "timer"
                            && m.alive
                            && named(m)
                            && m.at.distance(gate.at) < RACE_TIMER_NEAR
                    })
                    .map(|m| Mark {
                        kind: "destroy".to_owned(),
                        ..m.clone()
                    })
                    .collect();
                out.push(gate);
                return out;
            }
        }
        // The contract's active destroy objectives' targets the scripts
        // put no marker over, while what is named for it is only places
        // (PmcCon031 round 1: she stood 880 s at `MGStatueLoc` while its
        // 50 columns to bring down had no marker).
        if !marks
            .iter()
            .any(|m| m.alive && named(m) && m.kind != "objective")
        {
            let found = self.unmarked_targets(&marks, index);
            if !found.is_empty() {
                return found;
            }
        }
        let acts = self.contract_actions();
        if *self.acts_logged.borrow() != acts {
            eprintln!("mission: {id}'s action targets now {acts:x?}");
            *self.acts_logged.borrow_mut() = acts.clone();
        }
        if marks.iter().any(|m| to_do(m) && named(m)) {
            return marks.into_iter().filter(named).collect();
        }
        // None up for the contract: its active action objectives' targets,
        // which the scripts put no marker or blip over (PmcCon003's "Get
        // inside the PMC!" on `PlayerLocation_PMC`, round 16: she went
        // after the jobs' markers instead).
        let State::Running(mission) = &self.state else {
            return marks;
        };
        let w = mission.world().borrow();
        let found: Vec<Mark> = acts
            .iter()
            .filter_map(|&g| {
                // An entity the layers name but don't place as an object
                // (`PlayerLocation_PMC` in `vz_base`): at its entity's place.
                let (name, at, alive) = match w.objects.get(&g) {
                    Some(o) => (o.name.clone(), o.position, o.alive),
                    None => (format!("{g:#x}"), *w.entities.get(&g)?, true),
                };
                Some(Mark {
                    target: g,
                    kind: "action".to_owned(),
                    name,
                    at: Vec3::from(at),
                    alive,
                    mission: Some(MarkFor::Index(index)),
                    secondary: false,
                })
            })
            .collect();
        drop(w);
        if !found.is_empty() {
            return found;
        }
        if marks.iter().any(|m| to_do(m) && ours(m)) {
            return marks.into_iter().filter(ours).collect();
        }
        // None for it at all (a race's jetski, whose marker's mission the
        // scripts give as a list): the primary objectives' marks. Not
        // those the scripts put up for no mission (the jobs' and the
        // island's treasures): between a contract's stages she went off
        // after `_global_treasures 97`, 230 m away (PmcCon031/032 round
        // 16), and the contract's timer ran out.
        let primary: Vec<Mark> = marks
            .iter()
            .filter(|m| !m.secondary && m.mission.is_some())
            .cloned()
            .collect();
        if primary.iter().any(|m| m.alive) {
            primary
        } else {
            marks.into_iter().filter(|m| m.mission.is_some()).collect()
        }
    }

    /// The standing targets of the current contract's active destroy
    /// objectives ([`CONTRACT_TARGETS`]) that no mark is over, as marks
    /// to destroy.
    fn unmarked_targets(&self, marks: &[Mark], index: i64) -> Vec<Mark> {
        let State::Running(mission) = &self.state else {
            return Vec::new();
        };
        let found = contract_destroy_targets(mission);
        let w = mission.world().borrow();
        found
            .into_iter()
            .filter(|g| !marks.iter().any(|m| m.target == *g))
            .filter_map(|g| {
                let o = w.objects.get(&g).filter(|o| o.alive)?;
                Some(Mark {
                    target: g,
                    kind: "destroy".to_owned(),
                    name: o.name.clone(),
                    at: Vec3::from(o.position),
                    alive: true,
                    mission: Some(MarkFor::Index(index)),
                    secondary: false,
                })
            })
            .collect()
    }

    /// The checkpoint of the current contract's running race
    /// ([`CONTRACT_RACE`]), if one is running.
    fn contract_race(&self) -> Option<u32> {
        let State::Running(mission) = &self.state else {
            return None;
        };
        let g = mission
            .lua()
            .load(CONTRACT_RACE)
            .set_name("=contract race")
            .eval::<mlua::Value>()
            .ok()?;
        m2_game::events::as_guid(&g)
    }

    /// The targets of the current contract's active action objectives
    /// (`MrxTaskObjectiveAction`, their `_uTgtObjFilter`), from the
    /// scripts' own tasks.
    fn contract_actions(&self) -> Vec<u32> {
        let State::Running(mission) = &self.state else {
            return Vec::new();
        };
        let found = match mission
            .lua()
            .load(CONTRACT_ACTIONS)
            .set_name("=contract actions")
            .eval::<Option<mlua::Table>>()
        {
            Ok(found) => found,
            Err(e) => {
                eprintln!(
                    "mission: contract actions: {}",
                    m2_game::mission::first_lines(&e.to_string())
                );
                return Vec::new();
            }
        };
        found
            .into_iter()
            .flat_map(|t| {
                t.sequence_values::<mlua::Value>()
                    .flatten()
                    .collect::<Vec<_>>()
            })
            .filter_map(|v| m2_game::events::as_guid(&v))
            .collect()
    }

    /// Where the things are that the scripts wait for her to see
    /// (`Event.ObjectIsVisible`), as the game looks at them.
    pub fn sight_targets(&self, game: &Game) -> Vec<Vec3> {
        let State::Running(m) = &self.state else {
            return Vec::new();
        };
        let w = m.world().borrow();
        m.sight_watched()
            .into_iter()
            .filter_map(|g| {
                let o = w.objects.get(&g)?;
                Some(
                    game.scene
                        .props
                        .centre(g)
                        .unwrap_or(Vec3::from(o.position) + Vec3::Y * SIGHT_HEIGHT),
                )
            })
            .collect()
    }

    /// The regions she must not leave, as the scripts watch them
    /// (`Event.Boundary` on "exit": PmcCon034's out-of-bounds, which
    /// cancels it): each outline with where it is placed and its turn.
    pub fn kept_within(&self) -> Vec<crate::autoplay::Keep> {
        let State::Running(m) = &self.state else {
            return Vec::new();
        };
        let w = m.world().borrow();
        m.exits_watched()
            .into_iter()
            .filter_map(|g| {
                let (r, o) = (w.regions.get(&g)?, w.objects.get(&g)?);
                Some(crate::autoplay::Keep {
                    region: r.clone(),
                    centre: [o.position[0], o.position[2]],
                    yaw: o.yaw,
                })
            })
            .collect()
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

    /// How many contracts the scripts have said are completed.
    pub fn contracts_completed(&self) -> u32 {
        self.contracts
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
        let discs = hud
            .discs
            .values()
            .filter_map(|d| {
                let at = match (d.at, d.target.and_then(|g| w.objects.get(&g))) {
                    (Some(p), _) => Vec3::from(p),
                    (None, Some(o)) => Vec3::from(o.position),
                    (None, None) => return None,
                };
                Some((at + Vec3::Y * d.lift, d.radius, d.colour))
            })
            .collect();
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
                    flat: m.flat,
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
            discs,
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
/// How far above or below a context action's object she may stand and
/// still be offered it (metres; invented: a floor's height, so the HQ's
/// upper landing doesn't offer what stands below it).
const CONTEXT_RISE: f32 = 2.5;

/// The context action (`(object, position, text id, reach)`, as
/// [`m2_game::mission::Mission::context_actions`] lists them) nearest `at`
/// within its reach, measured flat, and within [`CONTEXT_RISE`] of it up
/// or down: its object and text id.
fn nearest_context(
    actions: &[(u32, Option<[f32; 3]>, String, f32)],
    at: Vec3,
) -> Option<(u32, String)> {
    actions
        .iter()
        .filter_map(|(g, pos, text, reach)| {
            let p = Vec3::from(pos.as_ref()?.to_owned());
            let flat = p.with_y(0.0).distance(at.with_y(0.0));
            (flat <= *reach && (p.y - at.y).abs() <= CONTEXT_RISE).then_some((flat, g, text))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, &g, text)| (g, text.clone()))
}

/// A scripts' helicopter with its own pilot that an action objective's
/// marker is over (PmcCon003's "Board the Lucky Lady",
/// `MrxTaskObjectiveEnterVehicle` with `bUseAnySeat`; from the scripts) is
/// boarded within this far of it, flat, metres, with this prompt; aboard,
/// the use key gets her out with the second. The reach and both prompts
/// are invented: the game's own entering is not known.
const RIDE_REACH: f32 = 4.0;
const RIDE_IN: &str = "Get in";
const RIDE_OUT: &str = "Get out";

/// The scripts' helicopters with their own pilot ([`ai_flown`]) that an
/// action objective's marker is over, and where they are: hers to board
/// as a passenger ([`board_ride`]).
fn rides(game: &Game, mission: &Mission) -> Vec<(u32, Vec3)> {
    let Some(catalogue) = &game.scene.catalogue else {
        return Vec::new();
    };
    let island = game.scene.world.as_ref();
    let enter = contract_targets(mission, "MrxTaskObjectiveEnterVehicle");
    let w = mission.world().borrow();
    w.hud
        .markers
        .values()
        .filter(|m| m.icon == "HUD_objective_action")
        .filter_map(|m| m.target)
        .filter_map(|g| {
            let o = w.objects.get(&g).filter(|o| o.alive)?;
            let models = object_models(g, &o.name, island, &catalogue.vehicles);
            // Or one nobody is in that the alpha doesn't have as hers
            // (PmcCon004's "Get Solano", the `Mi35 (Solano Hijack)` an
            // `MrxTaskObjectiveEnterVehicle` names: round 7 stood 1400 s by
            // it with nothing offered). Boarded the same way (inferred).
            // (The Mi35's template matches no catalogue model, so it is
            // known by the objective that names it.)
            let heli = models
                .iter()
                .any(|m| m.kind == m2_engine::vehicle::HELICOPTER)
                || enter.contains(&g);
            let hers = game.scene.vehicles.iter().any(|v| v.guid == Some(g));
            (ai_flown(&w, g, &models) || (heli && !hers)).then(|| (g, Vec3::from(o.position)))
        })
        .collect()
}

/// She gets into the scripts' helicopter `g` its pilot flies, as a
/// passenger: the scripts are told (their seat events fire), and she is
/// kept where it is until she gets out ([`leave_ride`]). It is not one of
/// hers: the alpha shows it only while it flies a delivery.
fn board_ride(game: &mut Game, mission: &mut Mission, g: u32) {
    eprintln!("mission: she boards {g:#x} as a passenger");
    mission.ride(Some(g));
    game.aboard = Some(g);
}

/// She gets out of the helicopter `g` she rode in: onto the floor beside
/// it (3 m off, invented), and the scripts are told.
fn leave_ride(game: &mut Game, mission: &mut Mission, g: u32) {
    eprintln!("mission: she gets out of {g:#x}");
    game.aboard = None;
    mission.ride(None);
    if let Some(p) = game.scene.player.as_mut() {
        let at = p.character.position + Vec3::new(3.0, 0.0, 0.0);
        p.drop_at(at);
    }
}

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
    fn sync(&mut self, game: &mut Game, mission: &mut Mission, quarry: &[u32]) {
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
        let mut seated = false;
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
                    seated = true;
                    eprintln!(
                        "mission: she is seated in {} ({g:#x}){}",
                        v.spec.label,
                        if v.spec.is_boat() { ", afloat" } else { "" }
                    );
                }
            }
        }
        if seated {
            game.scripts_seated = true;
        }
        let now = mission.now();
        self.landing.retain(|_, until| *until > now);
        for v in &game.scene.vehicles {
            let (Some(g), Some(id)) = (v.guid, v.id) else {
                continue;
            };
            // A ruin stands wrecked from the start: nothing died.
            if v.wrecked && !game.scene.ruins.contains(&g) {
                // Her bounty's quarry wrecked: by her, who went for it
                // (inferred: the wreck doesn't say who did it).
                let hers = quarry.contains(&g);
                let hero = mission.world().borrow().player.filter(|_| hers);
                match hero {
                    Some(h) => mission.kill_by(g, h),
                    None => mission.kill(g),
                }
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
        // Those the scripts still have in play near her count against the
        // most: the first contract's boats and jeeps, kilometres behind
        // her, mustn't keep the villa's tank from loading (playtest 4).
        let live = in_play_near(
            self.requested
                .iter()
                .filter_map(|g| w.objects.get(g))
                .filter(|o| o.alive)
                .map(|o| Vec3::from(o.position)),
            near,
        );
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
            let crew = mission.crew_of(g);
            if !crew.is_empty() {
                game.scene.mission_crews.insert(g, crew);
            }
            game.scene.load_vehicle(g, models);
        }
    }
}

/// How many of the loaded vehicles at `positions` count against
/// [`MAX_VEHICLES`]: those within [`VEHICLE_REACH`] of `near`.
fn in_play_near(positions: impl IntoIterator<Item = Vec3>, near: Vec3) -> usize {
    positions
        .into_iter()
        .filter(|p| p.distance(near) <= VEHICLE_REACH)
        .count()
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

/// The contract she is on and its index in the mission data, from the
/// scripts' own modules (`MrxPlayState`, `WifMissionData`).
const CURRENT_CONTRACT: &str = r#"
local ps, data = rawget(_G, "MrxPlayState"), rawget(_G, "WifMissionData")
if type(ps) ~= "table" or type(data) ~= "table" then return nil end
local m = ps.GetCurrentMission and ps.GetCurrentMission()
if type(m) ~= "table" or type(m.GetName) ~= "function" then return nil end
local id = string.gsub(m:GetName(), "Mission$", "")
local index = data.GetMissionIndexFromId and data.GetMissionIndexFromId(id)
if type(index) ~= "number" then return nil end
return id, index
"#;

/// The targets of the current contract's active action objectives: its
/// task's children, depth first, that are active and act on a target
/// filter with an action prompt (`sActionLabel`).
const CONTRACT_ACTIONS: &str = r#"
local ps = rawget(_G, "MrxPlayState")
if type(ps) ~= "table" or not ps.GetCurrentMission then return nil end
local m = ps.GetCurrentMission()
if type(m) ~= "table" then return nil end
local out = {}
local function walk(t, depth)
    if depth > 6 or type(t) ~= "table" then return end
    local ok, kids = pcall(function() return t:GetChildren() end)
    if not ok or type(kids) ~= "table" then return end
    for _, c in pairs(kids) do
        local okActive, active = pcall(function() return c:IsActive() end)
        active = okActive and active
        local cfg = type(c.GetConfig) == "function" and c:GetConfig() or {}
        if active and c._uTgtObjFilter and type(cfg) == "table" and cfg.sActionLabel then
            for _, g in ipairs(ObjectFilter.GetObjects(c._uTgtObjFilter)) do
                out[#out + 1] = g
            end
        end
        walk(c, depth + 1)
    end
end
walk(m, 0)
return out
"#;

/// The checkpoint the current contract's running race waits for: under
/// an active `MrxTaskRace` (it alone has `_tCourseLocs`), the active
/// checkpoint objective's `vDestLoc` (from the scripts).
const CONTRACT_RACE: &str = r#"
local ps = rawget(_G, "MrxPlayState")
if type(ps) ~= "table" or not ps.GetCurrentMission then return nil end
local m = ps.GetCurrentMission()
if type(m) ~= "table" then return nil end
local function walk(t, depth)
    if depth > 6 or type(t) ~= "table" then return nil end
    local ok, kids = pcall(function() return t:GetChildren() end)
    if not ok or type(kids) ~= "table" then return nil end
    for _, c in pairs(kids) do
        local okActive, active = pcall(function() return c:IsActive() end)
        if okActive and active and type(rawget(c, "_tCourseLocs")) == "table" then
            -- The gate up is its active checkpoint objective's place
            -- (`iCurLoc` runs one ahead of it: round 2 went for gate 3).
            local okGates, gates = pcall(function() return c:GetChildren() end)
            for _, gate in pairs(okGates and gates or {}) do
                local okUp, up = pcall(function() return gate:IsActive() end)
                local gc = type(gate.GetConfig) == "function" and gate:GetConfig() or {}
                if okUp and up and type(gc) == "table" and gc.vDestLoc then
                    local d = gc.vDestLoc
                    if type(d) == "string" then d = Pg.GetGuidByName(d) end
                    return d
                end
            end
        end
        local g = walk(c, depth + 1)
        if g then return g end
    end
    return nil
end
return walk(m, 0)
"#;

/// How often the contract's destroy targets are read again (seconds;
/// invented).
const UNMARKED_EVERY: f32 = 0.5;

/// A race's timer targets this near its gate now up (metres) are blown
/// up on the way to it (invented).
const RACE_TIMER_NEAR: f32 = 120.0;

/// The targets of the current contract's active objectives of module
/// `sModule` (their `_uTgtObjFilter`; from the scripts), read with
/// `sModule` set: `MrxTaskObjectiveDestroy`, `MrxTaskObjectiveEnterVehicle`.
const CONTRACT_TARGETS: &str = r#"
local sModule = ...
local ps = rawget(_G, "MrxPlayState")
if type(ps) ~= "table" or not ps.GetCurrentMission then return nil end
local m = ps.GetCurrentMission()
if type(m) ~= "table" then return nil end
local out = {}
local function walk(t, depth)
    if depth > 6 or type(t) ~= "table" then return end
    local ok, kids = pcall(function() return t:GetChildren() end)
    if not ok or type(kids) ~= "table" then return end
    for _, c in pairs(kids) do
        local okActive, active = pcall(function() return c:IsActive() end)
        local cfg = type(c.GetConfig) == "function" and c:GetConfig() or {}
        if okActive and active and type(cfg) == "table"
            and cfg.sModuleName == sModule and c._uTgtObjFilter then
            for _, g in ipairs(ObjectFilter.GetObjects(c._uTgtObjFilter)) do
                out[#out + 1] = g
            end
        end
        walk(c, depth + 1)
    end
end
walk(m, 0)
return out
"#;

/// The targets of the current contract's active destroy objectives
/// ([`CONTRACT_TARGETS`]), marked or not.
fn contract_destroy_targets(mission: &Mission) -> Vec<u32> {
    contract_targets(mission, "MrxTaskObjectiveDestroy")
}

/// The targets of the current contract's active objectives of `module`
/// ([`CONTRACT_TARGETS`]).
fn contract_targets(mission: &Mission, module: &str) -> Vec<u32> {
    let found = match mission
        .lua()
        .load(CONTRACT_TARGETS)
        .set_name("=contract targets")
        .call::<Option<mlua::Table>>(module)
    {
        Ok(found) => found,
        Err(e) => {
            eprintln!(
                "mission: contract targets ({module}): {}",
                m2_game::mission::first_lines(&e.to_string())
            );
            return Vec::new();
        }
    };
    found
        .into_iter()
        .flat_map(|t| {
            t.sequence_values::<mlua::Value>()
                .flatten()
                .collect::<Vec<_>>()
        })
        .filter_map(|v| m2_game::events::as_guid(&v))
        .collect()
}

/// She has got where the scripts sent her within this, metres (invented:
/// the autoplay's and the people's own arrival distance).
const GOAL_REACH: f32 = 2.5;

/// Further than this from the scripts' spawn point (metres), she is put
/// there ([`spawn_point`]).
const SPAWN_NEAR: f32 = 20.0;

/// Where the scripts' new game spawns the player: the first of
/// `MrxPlayer.SetSpawnLocations`' points (from the scripts: the
/// bootstrap names the contract skipped to's start, else
/// `Starter_Pmc_Start1`), once a layer has placed it.
fn spawn_point(mission: &Mission) -> Option<(String, Vec3)> {
    let name: String = mission
        .lua()
        .load(
            "local p = rawget(_G, 'MrxPlayer')              local t = type(p) == 'table' and rawget(p, '_tSpawnLocations')              return type(t) == 'table' and t[1] or nil",
        )
        .eval::<Option<String>>()
        .ok()??;
    let w = mission.world().borrow();
    let at = w.objects.get(&w.by_name(&name)?)?.position;
    Some((name, Vec3::from(at)))
}

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

/// Those following her ride with her (inferred: the engine's `Follow`
/// role boards the leader's vehicle; the scripts board a follower only
/// for the transit helicopter, `MrxFollow` on `transitStart`): one within
/// [`BOARD_REACH`] gets in as she drives, rides out of play where she is
/// (the scripts' delivery checks see them there), and gets out with her.
///
/// Out of a vehicle, they stay aboard [`OUT_AFTER`] seconds first, and go
/// on with her if she gets into another meanwhile (invented: the autoplay
/// swaps a stuck vehicle for another, and ChiCon001's VIP, out in a fight
/// meanwhile, was shot).
fn ride_along(
    game: &mut Game,
    mission: &mut Mission,
    riding: Option<u32>,
    out_for: &mut f32,
    dt: f32,
) {
    const BOARD_REACH: f32 = 6.0;
    const OUT_AFTER: f32 = 15.0;
    let Some(p) = game.scene.player.as_ref() else {
        return;
    };
    let (her, driving) = (p.character.position, p.driving.is_some());
    let Some(world) = &mut game.scene.world else {
        return;
    };
    let people = &mut game.scene.people;
    if driving {
        for g in people.followers_within(her, BOARD_REACH) {
            if people.board(&mut world.physics, g, true, her) {
                if let Some(v) = riding {
                    mission.seat_in(g, v);
                }
                eprintln!("people: {g:08x} gets in her vehicle with her");
            }
        }
        *out_for = 0.0;
    } else if !people.aboard().is_empty() {
        *out_for += dt;
        if *out_for >= OUT_AFTER {
            for g in people.aboard() {
                people.board(&mut world.physics, g, false, her);
                mission.unseat(g);
                eprintln!("people: {g:08x} gets out with her at {her:.1}");
            }
        }
    }
    for g in people.aboard() {
        mission.place(g, her.to_array(), 0.0);
    }
}

/// One of the scripts' own people on foot (spawned by them, not placed by
/// a layer: a soldier delivery's soldiers): their template's name, and
/// its model. People by their `Human` label, as `Pg.FastCollectHumans`
/// finds them.
fn scripted_person(
    mission: &Mission,
    templates: &Templates,
    guid: u32,
) -> Option<(String, Option<u32>)> {
    let w = mission.world().borrow();
    let o = w.objects.get(&guid)?;
    let human = [m2_assets::name_hash("human"), m2_assets::name_hash("Human")];
    if o.vehicle.is_some() || !o.all_labels().iter().any(|l| human.contains(l)) {
        return None;
    }
    let model = templates.get(&o.name).and_then(|t| t.model);
    Some((o.name.clone(), model))
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
                "mission: marker {} on {:?} ({g:#x}) for {}: colour {:?}, {} m up, fades {}..{} m, sizes {:?}, 2D only {}{}",
                m.icon,
                w.objects.get(&g).map_or("?", |o| o.name.as_str()),
                m.mission,
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

/// Lua that adds cut content back to the support catalogue (see
/// `m2_engine::standin`), each item built as the game's own of its kind:
///
/// - the V-22 Osprey, flown in as the helicopters are
///   (`mrxsupportcopterdelivery` with its `V22 (Delivery)` template, which
///   the game keeps), under its own PDA icon `vehicles_plane_v22`;
/// - `DLC_M1A1`, slung under the Mi-26 as the M1A2 is, under the M1A2's
///   icon;
/// - the cut hand weapons, each in its supply crate
///   (`m2_engine::standin::SUPPLY_DROPS`), dropped as the game's crates
///   are, under the icon of the crate each is shaped as.
///
/// The game has no strings for them, so names and descriptions are
/// written here; the prices are invented, near the game's for their
/// kind (the Endriago's 45,000 and 140 fuel, the M1A2's 425,000 and 240,
/// a weapon crate's 10,000 and 40).
const RESTORED_SUPPORT: &str = r#"
local data = MrxSupportData and MrxSupportData.tSupportData
local function lookup(name)
    return (MrxSupportData and MrxSupportData[name]) or rawget(_G, name)
end
local copter, crates = lookup("mrxsupportcopterdelivery"), lookup("mrxcratedelivery")
local function add(key, item, support)
    if data and support and not data[key] then
        item.nMaxStock = 99
        item.oSupport = support
        data[key] = item
    end
end
if copter then
    local o = copter:Create()
    o:SetDeliveryVehicle("V22 (Delivery)")
    add("v22osprey", {
        sName = "V-22 Osprey",
        sDescription = "Tiltrotor transport, cut from the game and flown back in.",
        sIcon = "vehicles_plane_v22", nCashCost = 150000, nFuelCost = 200, sType = "Heli",
    }, o)
end
if crates then
    local o = crates:Create()
    o:SetCargo("DLC_M1A1")
    o:SetDeliveryVehicle("Mi26 (PMC) (Driver)")
    add("dlcm1a1", {
        sName = "M1A1 Abrams",
        sDescription = "The cut M1A1, slung in under a Mi-26.",
        sIcon = "vehicles_tank_m1a2", nCashCost = 400000, nFuelCost = 240, sType = "Heavy",
    }, o)
    for _, w in ipairs({
        { "coilgun", "Supply Drop (Coilgun)", "Coilgun", "supplies_sniper_kit", 25000 },
        { "riotgun", "Supply Drop (Riot Gun)", "Riot Gun", "supplies_cqb", 10000 },
        { "cheatrpg", "Supply Drop (Cheat RPG)", "Cheat RPG", "supplies_rpg", 50000 },
        { "dlc50cal", "Supply Drop (50 Cal)", ".50 Cal", "supplies_light_mg", 15000 },
    }) do
        local o = crates:Create()
        o:SetCargo(w[2])
        o:SetCareless(true)
        add(w[1], {
            sName = w[3] .. " crate",
            sDescription = "A cut weapon, dropped in its own crate.",
            sIcon = w[4], nCashCost = w[5], nFuelCost = 40, sType = "Supply",
        }, o)
    end
end
-- Who sells them: the Osprey Universal Petroleum alone (Brendan's call,
-- 2026-10-18), the M1A1 the Allies as they sell the M1A2, the weapons'
-- crates your PMC (invented). A shop lists what `MrxRewardData`'s rewards
-- name for its faction (`GetAllPotentialShopItems`, cached in
-- `gtAllSupport`); each is unlocked there from the start. The Cheat RPG's
-- crate is sold nowhere: the cheat menu and `--support` call it.
local rewards = MrxRewardData and MrxRewardData._tRewards
if data and rewards and not rewards.__m2_restored then
    local sold = { { "v22osprey", "Oil" }, { "dlcm1a1", "All" },
        { "coilgun", "Pmc" }, { "riotgun", "Pmc" }, { "dlc50cal", "Pmc" } }
    local listed = {}
    for _, s in ipairs(sold) do
        local item = data[s[1]]
        if item then
            item.tUnlockStatus = { [s[2]] = 2 }
            table.insert(listed, s)
        end
    end
    rewards.__m2_restored = { tSupport = listed }
    MrxRewardData.gtAllSupport = nil
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
        .filter(|i| i.stock > 0 || i.free > 0 || i.active)
        // The free Satellite Guided Bomb is her own item ([`Kind`]).
        .filter(|i| Kind::from_freebie(&i.key).is_none())
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
            count: u32::try_from(i.stock.max(0) + i.free.max(0))
                .unwrap_or(0)
                .max(u32::from(i.active)),
            free: i.free > 0 || i.active,
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
/// the same way, with nothing on the winch, down to the ground under its
/// goal's location: the copter delivery's is 50 m over the mark (its
/// script adds 50 to the mark's height), a soldier delivery's is the
/// mark itself (`MrxSoldierDelivery`; landed 50 m under it, its soldiers
/// came down underground). Once down its pilot gets out and walks off, and
/// the helicopter stays: its pilot's `MoveTo` doesn't send it home
/// (`landed`). It can't be flown in this game yet.
fn fly(game: &mut Game, mission: &mut Mission, landed: &mut HashSet<u32>, c: &Command) {
    match c {
        Command::Goal {
            guid,
            goal,
            location: Some(to),
            ..
        } if goal == "Deliver" || goal == m2_game::mission::LAND => {
            let to = &if goal == m2_game::mission::LAND {
                landed.insert(*guid);
                ground_under(game, Vec3::from(*to)).to_array()
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
                    if goal == m2_game::mission::LAND {
                        game.scene.flights.land(*guid);
                    }
                }
                None => mission.arrived(*guid, *to),
            }
        }
        Command::Goal {
            guid,
            goal,
            location: Some(to),
            ..
        } if goal == "MoveTo"
            && game.scene.flights.flying(*guid)
            // Landed, it stays once its pilot is out (a copter delivery's);
            // with the pilot still aboard (a soldier delivery's, once its
            // soldiers are down: `MrxSupport.GoHome`) it goes, freeing the
            // support's recruit for the next call.
            && (!landed.contains(guid) || mission.world().borrow().vehicle_of(*guid).is_some()) =>
        {
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
    // A person the scripts make (`Pg.Spawn` of a person's template) stands
    // as a layer's spawn would; one they take away goes.
    match c {
        Command::Spawn {
            guid,
            what,
            position,
            yaw,
        } => {
            if let Some(s) = game.scene.world.as_ref().and_then(|w| {
                w.index
                    .template_db
                    .scripted_spawn(*guid, what, Vec3::from(*position), *yaw)
            }) {
                game.scene.people.scripted_spawn(s);
            }
            // A prop of theirs (PirCon004's organ boxes) stands as a
            // layer's would.
            if let (Some(w), Some(install)) =
                (game.scene.world.as_ref(), game.scene.install.as_ref())
                && let Some(o) =
                    w.index
                        .template_db
                        .scripted_prop(*guid, what, Vec3::from(*position), *yaw)
                && game
                    .scene
                    .props
                    .show_spawned(install, &w.index.template_db, &o)
            {
                eprintln!("mission: {what} put up as {guid:#x} at {:.1?}", position);
            }
        }
        Command::Remove(guid) => {
            game.scene.people.scripted_gone(*guid);
            if let Some(w) = game.scene.world.as_mut() {
                game.scene.props.hide(&mut w.physics, [*guid]);
            }
        }
        _ => {}
    }
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
                game.scripts_seated = true;
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
            let models = w
                .objects
                .get(vehicle)
                .map(|o| object_models(*vehicle, &o.name, game.scene.world.as_ref(), catalogue));
            // A helicopter its pilot flies (the scripts' transit seats her
            // in the Lucky Lady again, "p"): she rides in it.
            if models.as_ref().is_some_and(|m| ai_flown(&w, *vehicle, m)) {
                drop(w);
                if game.aboard != Some(*vehicle) {
                    eprintln!("mission: the scripts seat her in {vehicle:#x} as a passenger");
                    game.aboard = Some(*vehicle);
                }
                return false;
            }
            if let Some(o) = w.objects.get(vehicle)
                && models.is_some_and(|m| !m.is_empty())
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
        // Turned by the scripts (`Object.SetYaw`, or put at a locator with
        // `Object.SetTransformToObject`: the HQ's briefing faces her to
        // its starter). Their yaw is taken as her facing, both about
        // +y from +z (inferred: the briefing's two spots face each other
        // by it, 0.58 and -2.54).
        Command::Turn { guid, yaw } if *guid == PLAYER => {
            if let Some(p) = game.scene.player.as_mut()
                && p.driving.is_none()
            {
                p.facing = *yaw;
            }
            return false;
        }
        // The briefings' camera shots: held until the scene ends.
        Command::CameraShot { shot, base, target } => {
            eprintln!("mission: camera shot {shot} from {base:#x} to {target:#x}");
            game.shot = Some(crate::game::Shot {
                name: shot.clone(),
                base: *base,
                target: *target,
                view: None,
            });
            return false;
        }
        Command::Cinematic(on) => {
            if !on && game.shot.take().is_some() {
                eprintln!("mission: the scene ends; the camera is hers again");
            }
            if !on && game.raw.held() > 0 {
                eprintln!(
                    "raw-clip: the scene ends; {} let go of their clips",
                    game.raw.held()
                );
                game.raw.release(&mut game.scene);
            }
            return false;
        }
        // A briefing frame's clip for one of its actors, by name.
        Command::RawClip { guid, clip } => {
            match game.raw.start(&mut game.scene, *guid, clip) {
                Ok(length) => eprintln!("raw-clip: {guid:#x} plays {clip} ({length:.2} s)"),
                Err(why) => eprintln!("raw-clip: {guid:#x} cannot play {clip}: {why}"),
            }
            return false;
        }
        // The wardrobe's pick: put on her before the next frame.
        Command::Outfit { guid, model } if *guid == PLAYER => {
            game.outfit_wanted = Some(model.clone());
            return false;
        }
        Command::ExitVehicle(guid) if *guid == PLAYER => {
            fleet.reenter = None;
            if let Some(g) = game.aboard.take() {
                eprintln!("mission: the scripts take her out of {g:#x}");
                if let Some(p) = game.scene.player.as_mut() {
                    p.drop_at(p.character.position + Vec3::new(3.0, 0.0, 0.0));
                }
                return false;
            }
            let (Some(world), Some(p)) = (&game.scene.world, game.scene.player.as_mut()) else {
                return false;
            };
            if let Some(v) = p.driving.and_then(|i| game.scene.vehicles.get(i)) {
                get_out(&world.physics, v, p);
            }
            return false;
        }
        // Someone following her (`Ai.Role` `Follow`: a VIP to deliver).
        Command::Follow { guid, target, near } => {
            game.scene
                .people
                .follow(*guid, target.is_some().then_some(*near));
            return false;
        }
        // A follower gets into her vehicle (`MrxFollow`: `Vehicle.Enter(v,
        // actor, "p")`) and rides it, out of play, or gets out beside her.
        Command::EnterVehicle { guid, .. } | Command::ExitVehicle(guid) if *guid != PLAYER => {
            let entering = matches!(c, Command::EnterVehicle { .. });
            let at = game.focus();
            if let Some(world) = &mut game.scene.world
                && game
                    .scene
                    .people
                    .board(&mut world.physics, *guid, entering, at)
            {
                eprintln!(
                    "people: {guid:08x} {} her vehicle",
                    if entering { "rides" } else { "gets out of" }
                );
                if !entering
                    && !is_spawn(world, *guid)
                    && let Some((template, model)) =
                        scripted_person(mission, &world.index.template_db, *guid)
                {
                    game.scene
                        .people
                        .bring_scripted(*guid, &template, model, at);
                }
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
                        // there they stand. One of the scripts' own on
                        // foot (a soldier delivery's, set down): brought
                        // into play there.
                        if is_spawn(world, *guid) {
                            scene.people.put(*guid, at);
                        } else if let Some((template, model)) =
                            scripted_person(mission, &world.index.template_db, *guid)
                        {
                            scene.people.bring_scripted(*guid, &template, model, at);
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
        // Sent somewhere herself (an outpost's rushers, those near its flag,
        // include the player): her goal is done when she gets there.
        Command::Goal {
            guid,
            goal,
            target: Some(to),
            ..
        } if goal == "MoveTo" && *guid == PLAYER => {
            let at = mission
                .world()
                .borrow()
                .objects
                .get(to)
                .map(|o| Vec3::from(o.position));
            if let Some(at) = at
                && fleet.player_goal.is_none_or(|g| g.0 != *to)
            {
                eprintln!("mission: the scripts send her to {to:#x} at {at:.1}");
                fleet.player_goal = Some((*to, at));
            }
            return false;
        }
        // A person on foot sent to a place (PmcCon001's beaten soldiers
        // running for `loc_EntourageFleePoint`, an outpost's rushers for its
        // flag, the scripts' own soldiers too): they run there.
        Command::Goal {
            guid,
            goal,
            target: Some(to),
            ..
        } if goal == "MoveTo"
            && game.scene.world.as_ref().is_some_and(|w| {
                is_spawn(w, *guid)
                    || scripted_person(mission, &w.index.template_db, *guid).is_some()
            }) =>
        {
            let at = mission
                .world()
                .borrow()
                .objects
                .get(to)
                .map(|o| Vec3::from(o.position));
            if let Some(at) = at {
                eprintln!("mission: {guid:#x} runs to {to:#x} at {at:.1}");
                game.scene.people.walk(*guid, vec![at]);
            }
            return false;
        }
        // A driver sent along a path (PmcCon001's villa tank, through the
        // garage wall the scripts have just killed): its vehicle drives it
        // ([`crate::paths`]). A person on foot runs it ([`People::walk`]:
        // the villa's wave soldiers).
        //
        // [`People::walk`]: crate::npc::People::walk
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
                // A person on foot (the villa's wave soldiers, coming in
                // through the garage and over the walls): they run it.
                None if !route.is_empty()
                    && game
                        .scene
                        .world
                        .as_ref()
                        .is_some_and(|w| is_spawn(w, *guid)) =>
                {
                    eprintln!(
                        "mission: {guid:#x} runs along path {path:#x} ({} points) to {:.1?}",
                        route.len(),
                        route[route.len() - 1]
                    );
                    game.scene.people.walk(*guid, route);
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
        // The scripts' pursuit: that army's soldiers fight her for a while,
        // or until it is cleared.
        Command::Pursuit { faction, seconds } => match crate::npc::faction_by_label(faction) {
            Some(f) => game.scene.people.scripted_pursuit(f, *seconds),
            None => eprintln!("mission: a pursuit by {faction}, not an army here"),
        },
        Command::PursuitClear => game.scene.people.clear_scripted_pursuit(),
        // A keyed action: the player's verify photo plays her hero's
        // `VerifyCamera` clip (the on-foot tables' handle fe0ead5a: rifle
        // 5ae2cef2 Jennifer's, 005381c4 Mattias's, b448382e Chris's, from
        // samples) and ends with it; anyone else's ends at once (no clip
        // is looked up for theirs yet).
        Command::DoAction { guid, action } if *guid == PLAYER => {
            let mut seconds = ACTION_SECONDS;
            if action.eq_ignore_ascii_case("VerifyCamera")
                && let Some(p) = game.scene.player.as_mut()
                && let Some((_, rig)) = p.model.as_mut()
                && let Some(i) = VERIFY_CLIPS.iter().find_map(|&h| {
                    rig.clips
                        .iter()
                        .position(|c| c.hash == h && !rig.foreign.contains(&h))
                })
            {
                seconds = rig.clips[i].info.duration;
                rig.animator.play_once(i);
            }
            eprintln!("mission: she plays {action} ({seconds:.1} s)");
            game.actions.push((*guid, action.clone(), seconds));
        }
        Command::DoAction { guid, action } => {
            eprintln!("mission: {guid:#x} plays {action}");
            game.actions.push((*guid, action.clone(), 0.0));
        }
        Command::Drop(guid) => {
            for n in &mut game.scene.people.npcs {
                if n.carry(false) {
                    eprintln!("mission: {guid:#x} puts down {:?}", n.spawn);
                }
            }
        }
        // Shut again: its model and collision go back up where its layer
        // has it (as OpenGate took them down).
        Command::CloseGate(guid) => {
            let put = game.scene.show_props([*guid]);
            eprintln!(
                "mission: Object.CloseGate {guid:#x}: {}",
                if put > 0 {
                    "shut, its model and collision back up"
                } else {
                    "already shut, or not a layer's model here"
                }
            );
            return false;
        }
        Command::Weapons { guid, on } if *guid == PLAYER => {
            if let Some(p) = game.scene.player.as_mut()
                && p.holstered == *on
            {
                p.holstered = !on;
                eprintln!(
                    "mission: her weapons {}",
                    if *on { "back in her hands" } else { "put away" }
                );
            }
            return false;
        }
        Command::Usable { guid, on } => {
            let changed = if *on {
                game.unusable.remove(guid)
            } else {
                game.unusable.insert(*guid)
            };
            if changed {
                eprintln!(
                    "mission: vehicle {guid:#x} {} to her",
                    if *on { "usable again" } else { "kept from" }
                );
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
                let before = scene
                    .player
                    .as_ref()
                    .and_then(|p| p.floor_under(&world.physics));
                scene
                    .props
                    .hide(&mut world.physics, entities.iter().copied());
                // The floor she stood on, or was put down on, went with it
                // (playtest 7: the villa garage's, as PmcCon001 ended and
                // put her out of the tank; she fell to y -55.7 and stayed):
                // she waits there for what the next layers put in its
                // place, and lands on it ([`Player::layer_went`]).
                if let Some(p) = scene.player.as_mut()
                    && p.driving.is_none()
                {
                    let after = p.floor_under(&world.physics);
                    if p.layer_went(before, after) {
                        eprintln!(
                            "layer {name}: the floor under her at {:.1?} went with it: she \
                             waits for what comes in its place",
                            p.character.position.to_array()
                        );
                    } else if let Some(b) = before
                        && p.character.position.y - b > 0.3
                    {
                        eprintln!(
                            "layer {name}: she is {:.2} m over her floor, which stays: she \
                             comes down on it",
                            p.character.position.y - b
                        );
                    }
                }
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
        // A prop the layers place by its model's name (a leading `_`)
        // or a ruin's or wreck's name: drawn as itself, no vehicle
        // (playtest 4: `_global_veh_ruin_68valiant`, by the porch hut,
        // stood a second, drivable Valiant).
        _ if placed_prop_or_ruin(name) => Vec::new(),
        _ => vehicle_models(name, catalogue),
    }
}

/// Whether an object's `name` is a prop placed by its model's name or a
/// ruin's or wreck's (the words `m2_assets::vehicle` keeps out of its
/// shared models).
fn placed_prop_or_ruin(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with('_') || n.contains("ruin") || n.contains("wreck")
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
    // The pickup itself, when it is a gun with that model (the restored
    // crates' `Cheat RPG`, which shares its model with the `Fuel-Air
    // RPG`); else a gun of the templates' with its model (of several,
    // the lowest).
    let own = templates
        .arms
        .weapons
        .get(&id)
        .filter(|w| w.class.is_some() && model.is_some() && w.model == model);
    let gun = own.or_else(|| {
        model.and_then(|m| {
            templates
                .arms
                .weapons
                .values()
                .filter(|w| w.class.is_some() && w.model == Some(m))
                .min_by_key(|w| w.template)
        })
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
///
/// The 360 code differs: there every pickup without a `Weapon` record
/// (`Type` 2) is taken walking over it, ammunition boxes too, and only a
/// weapon lying there is a context action (8228F5E8, 8228F450, 82432F68).
/// That looks like a platform difference; the PC footage's use key is kept
/// until PC evidence says otherwise (Brendan, 2026-10-27).
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

/// A starter's talk, as the scripts' HQs offer it (`[ContextAction.Talk]`,
/// `WifPmcInterior._EnableStarter`; from the scripts).
pub const TALK_PROMPT: &str = "[ContextAction.Talk]";

/// A dialog box the scripts have up ([`MissionLink::open_dialog`]).
#[derive(Clone, Debug, PartialEq)]
pub struct OpenDialog {
    pub seq: u64,
    pub method: String,
    pub text: String,
    pub options: Vec<String>,
}

/// Whether a starter's talk can be gone to: one riding a prop only once
/// the game has seated her in play ([`MissionLink::talk_offers`]).
pub fn talk_ready(rides_prop: bool, seated_in_play: bool) -> bool {
    !rides_prop || seated_in_play
}

/// Whether `id` names one of the flow's contracts (`OilCon020`), not a
/// job (`PmcJob001`).
pub fn is_flow_contract(id: &str) -> bool {
    m2_game::script::is_contract_module(id) && id.get(3..6) == Some("Con")
}

/// What a player taking a contract picks in a starter's box: in her menu
/// (`Display`), the first option naming the title of a contract on
/// `offered` (the HQ's reads `[yellow][shop.new] [white]"[OilCon020.Title]"`);
/// in a contract's terms (`Scroll`, naming `[OilCon020.Terms.Summary]`)
/// for one on offer, `[Generic.Accept]` (the first, when the box gives no
/// options). Contracts are found by their localization tokens inside the
/// shown text, whatever colour tags and quotes surround them. Options count from 1, as the scripts'
/// callbacks get them. None: the box isn't one for taking a contract on
/// offer.
pub fn contract_pick(dialog: &OpenDialog, offered: &[String]) -> Option<(i64, String)> {
    let number = |i: usize| i64::try_from(i + 1).ok();
    let names = |text: &str, tail: &str| {
        offered
            .iter()
            .find(|c| text.contains(&format!("[{c}{tail}")))
            .cloned()
    };
    match dialog.method.as_str() {
        "Display" => dialog
            .options
            .iter()
            .enumerate()
            .find_map(|(i, o)| Some((number(i)?, names(o, ".Title]")?))),
        // The terms box names the contract by its terms' token; with no
        // options of its own it is the game's Accept and Decline, Accept
        // first (a cancel picks 2: see [`DIALOG_BOXES`]).
        "Scroll" => {
            let id = names(&dialog.text, ".Terms")?;
            let n = match dialog
                .options
                .iter()
                .position(|o| o.contains("[Generic.Accept]"))
            {
                Some(i) => number(i)?,
                None if dialog.options.is_empty() => 1,
                None => return None,
            };
            Some((n, id))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    // A starter's menu: the first contract on offer among its options is
    // picked (counting from 1); a job or a contract not on offer isn't.
    // Its terms: Accept, for a contract on offer only. Other boxes: none.
    #[test]
    fn a_contract_on_offer_is_picked_then_accepted() {
        let offered = ["OilCon020".to_owned(), "PmcCon031".to_owned()];
        let box_of = |method: &str, text: &str, options: &[&str]| OpenDialog {
            seq: 4,
            method: method.into(),
            text: text.into(),
            options: options.iter().map(|o| (*o).to_owned()).collect(),
        };
        let menu = box_of(
            "Display",
            "[Briefing.RootMenuDialog]",
            // As the PMC HQ's menu offered them in the story run (B3).
            &[
                "[PmcJob001.Title]",
                "[GurCon002.Title]",
                "[yellow][shop.new] [white]\"[OilCon020.Title]\"",
                "[yellow][shop.new] [white]\"[PmcCon031.Title]\" ([Generic.Level] 1)",
                "[Briefing.Hint]",
                "[Generic.Exit]",
            ],
        );
        assert_eq!(
            contract_pick(&menu, &offered),
            Some((3, "OilCon020".into()))
        );
        let terms = box_of(
            "Scroll",
            "[OilCon020.Terms.Summary]",
            &["[Generic.Accept]", "[Generic.Decline]"],
        );
        assert_eq!(
            contract_pick(&terms, &offered),
            Some((1, "OilCon020".into()))
        );
        // The terms box as the PMC HQ put it up in the story run (B3): no
        // options of its own, so Accept is the first.
        let real_terms = box_of(
            "Scroll",
            "\"[OilCon020.Title]\"\n\n[Briefing.Faction]: [flagpmc] [Generic.Factions.Pmc.Short]\n\n\
             [OilCon020.Terms.Summary]\n\n[Generic.Rewards]:\n[OilCon020.Objectives.CustomReward]\n",
            &[],
        );
        assert_eq!(
            contract_pick(&real_terms, &offered),
            Some((1, "OilCon020".into()))
        );
        let other = box_of("Scroll", "[GurCon002.Terms.Summary]", &["[Generic.Accept]"]);
        assert_eq!(contract_pick(&other, &offered), None);
        let exit_only = box_of("Display", "[Briefing.RootMenuDialog]", &["[Generic.Exit]"]);
        assert_eq!(contract_pick(&exit_only, &offered), None);
        assert_eq!(contract_pick(&menu, &[]), None);
    }

    // The box's call keeps what it shows: its kind, title and options,
    // which autoplay reads to pick a contract on offer.
    #[test]
    fn an_open_box_keeps_its_kind_title_and_options() {
        let lua = mlua::Lua::new();
        lua.load(
            r#"
            Hud = {}
            MrxGuiDialogBox = { DisplayDialogBox = function() end,
                DisplayScrollingDialogBox = function() end, Close = function() end }
            MrxGui = { DisplayDialogBox = MrxGuiDialogBox.DisplayDialogBox }
            "#,
        )
        .exec()
        .unwrap();
        lua.load(HUD_CALLS).exec().unwrap();
        lua.load(DIALOG_BOXES).exec().unwrap();
        lua.load(
            r#"MrxGui.DisplayDialogBox(1, "[Briefing.RootMenuDialog]",
                { "[OilCon020.Title]", "[Generic.Exit]" }, 1, function() end, {})"#,
        )
        .exec()
        .unwrap();
        let (method, title, first): (String, String, String) = lua
            .load(
                "for _, t in pairs(__m2_hud_callbacks) do \
                   if type(t) == 'table' and t.dialog then \
                     return t.method, t.data.sTitle, t.data.tOptions[1] end end",
            )
            .eval()
            .unwrap();
        assert_eq!(
            (method.as_str(), title.as_str(), first.as_str()),
            ("Display", "[Briefing.RootMenuDialog]", "[OilCon020.Title]")
        );
    }

    // A starter on a prop is gone to only once she is seated in play;
    // one on foot at once.
    #[test]
    fn a_seated_starter_is_gone_to_once_seated_in_play() {
        assert!(talk_ready(false, false));
        assert!(!talk_ready(true, false));
        assert!(talk_ready(true, true));
    }

    // The flow's contracts are told from its jobs by their ids.
    #[test]
    fn flow_contracts_are_told_from_jobs() {
        assert!(is_flow_contract("OilCon020"));
        assert!(!is_flow_contract("PmcJob001"));
        assert!(!is_flow_contract("OilCon20"));
    }

    #[test]
    fn the_nearest_context_action_in_reach_is_offered() {
        // The HQ's exits stand at its doors, a floor up in the sky
        // (y 450); its starter's talk reaches 4 m.
        let actions = vec![
            (
                1,
                Some([0.0, 450.0, 0.0]),
                "[ContextAction.Exit]".into(),
                2.0,
            ),
            (
                2,
                Some([3.0, 450.0, 0.0]),
                "[ContextAction.Talk]".into(),
                4.0,
            ),
            (3, None, "[ContextAction.Exit]".into(), 2.0),
            (
                4,
                Some([1.0, 446.0, 0.0]),
                "[ContextAction.Exit]".into(),
                2.0,
            ),
        ];
        let at = |x: f32, y: f32| Vec3::new(x, y, 0.0);
        let near = |p| nearest_context(&actions, p).map(|(g, _)| g);
        // Beside the exit: it, though the talk reaches too.
        assert_eq!(near(at(0.5, 450.0)), Some(1));
        // Nearer the starter.
        assert_eq!(near(at(2.5, 450.0)), Some(2));
        // Out of reach of all; one a floor below never offered.
        assert_eq!(near(at(10.0, 450.0)), None);
        assert_eq!(near(at(1.0, 446.0 - CONTEXT_RISE - 0.5)), None);
        assert_eq!(
            nearest_context(&actions, at(0.0, 450.0)),
            Some((1, "[ContextAction.Exit]".to_owned()))
        );
    }

    use super::*;

    fn entry(model: u32, kind: &str, label: &str) -> CatalogueEntry {
        CatalogueEntry {
            model,
            kind: kind.into(),
            label: label.into(),
        }
    }

    #[test]
    fn a_ruin_prop_placed_by_its_models_name_is_no_vehicle() {
        let catalogue = [entry(1, "car", "global_veh_68valiant")];
        let none = Templates::default();
        assert!(
            template_models("_global_veh_ruin_68valiant 0x000d8eac", &none, &catalogue).is_empty()
        );
        assert!(template_models("68Valiant Wreck", &none, &catalogue).is_empty());
        // A script's car by that name still finds its model.
        assert_eq!(
            template_models("68Valiant 0x000fab16", &none, &catalogue).len(),
            1
        );
    }

    #[test]
    fn only_the_vehicles_near_her_count_against_the_most() {
        // Playtest 4: 37 loaded from the first contract, 4 km behind her
        // at the villa, left no room for the tank.
        let villa = Vec3::new(2625.0, -13.0, -953.0);
        let island = Vec3::new(-1500.0, 0.0, 600.0);
        let far = std::iter::repeat_n(island, 37);
        assert_eq!(in_play_near(far.clone(), villa), 0);
        let near = std::iter::repeat_n(villa + Vec3::X * 50.0, 3);
        assert_eq!(in_play_near(far.chain(near), villa), 3);
        assert_eq!(
            in_play_near([villa + Vec3::Z * (VEHICLE_REACH + 1.0)], villa),
            0
        );
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
    fn the_pdas_database_reaches_the_hud_as_pda_calls() {
        let lua = mlua::Lua::new();
        lua.load("Hud = {} Pda = { Database = {} }").exec().unwrap();
        lua.load(HUD_CALLS).exec().unwrap();
        lua.load(
            r#"Pda.Database:SetFactionAttitude({ sName = "China", sTexture = "t", nAttitude = 60 })
            Pda.Database:AddLogEntry({ sText = "x" })"#,
        )
        .exec()
        .unwrap();
        let calls = hud_calls(&lua);
        let names: Vec<(&str, &str)> = calls
            .iter()
            .map(|c| (c.group.as_str(), c.method.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("Pda.Database", "SetFactionAttitude"),
                ("Pda.Database", "AddLogEntry")
            ]
        );
    }

    #[test]
    fn the_scripts_dialog_boxes_go_to_the_hud_and_the_pick_comes_back() {
        // The HQ's briefing menu: `MrxGui.DisplayDialogBox(owner, title,
        // options, default, callback, args, ...)`; the box's own module
        // stands in the mission's state until it is wrapped.
        let lua = mlua::Lua::new();
        lua.load(
            r#"
            Hud = {}
            MrxGuiDialogBox = { DisplayDialogBox = function() error("drawn here") end,
                DisplayScrollingDialogBox = function() error("drawn here") end,
                Close = function() end }
            MrxGui = { DisplayDialogBox = MrxGuiDialogBox.DisplayDialogBox }
            "#,
        )
        .exec()
        .unwrap();
        lua.load(HUD_CALLS).exec().unwrap();
        lua.load(DIALOG_BOXES).exec().unwrap();
        lua.load(
            r#"
            picked = nil
            box = MrxGui.DisplayDialogBox(1, "[Briefing.RootMenuDialog]",
                { "[OilCon020.Title]", "[Generic.Exit]" }, 1,
                function(who, n) picked = { who, n } end, { "Fiona" }, 48, 36, "left")
            "#,
        )
        .exec()
        .unwrap();
        let calls = hud_calls(&lua);
        assert_eq!(calls.len(), 1);
        assert_eq!(
            (calls[0].group.as_str(), calls[0].method.as_str()),
            ("DialogBox", "Display")
        );
        assert!(calls[0].callback);
        let Plain::Table(data) = &calls[0].data else {
            panic!("{:?}", calls[0].data);
        };
        assert!(data.contains(&(
            Plain::Text("sTitle".into()),
            Plain::Text("[Briefing.RootMenuDialog]".into())
        )));
        // The HUD's pick: the callback, with its args and then the option.
        lua.load(choice_code(calls[0].seq, 2)).exec().unwrap();
        let (who, n): (String, i64) = lua.load("return picked[1], picked[2]").eval().unwrap();
        assert_eq!((who.as_str(), n), ("Fiona", 2));
        // The scrolling box (the contract's Accept) goes the same way, and
        // closing a box tells the HUD.
        lua.load(
            r#"s = MrxGuiDialogBox.DisplayScrollingDialogBox(1, "[OilCon020.Terms.Summary]",
                function(n) picked = n end, {}, { "[Generic.Accept]" })
            s:Close()"#,
        )
        .exec()
        .unwrap();
        let calls = hud_calls(&lua);
        let methods: Vec<&str> = calls.iter().map(|c| c.method.as_str()).collect();
        assert_eq!(methods, ["Display", "Scroll", "Close"]);
        // The wager's digit box, as the HQ's `_ChangeWager` calls it: the
        // HUD answers the value, or -1 - the value for a cancel.
        lua.load(
            r#"MrxGui.DisplayNumericBox = function() error("drawn here") end
            __m2_wrap_dialogs()
            local function n(...) MrxGui.DisplayNumericBox(1, "[Briefing.ChangeWagerQuery]", "",
                "$", ".0 K", 20000, 10000, 50000, 5, 3, 5,
                function(tag, v) picked = { tag, v } end, { "accept" },
                function(tag, v) picked = { tag, v } end, { "cancel" },
                48, 36, "right", "bottom", false) end
            n() n()"#,
        )
        .exec()
        .unwrap();
        let calls = hud_calls(&lua);
        let numeric: Vec<u64> = calls
            .iter()
            .filter(|c| c.method == "Numeric")
            .map(|c| c.seq)
            .collect();
        assert_eq!(numeric.len(), 2);
        let answer = |seq, v| {
            lua.load(choice_code(seq, v)).exec().unwrap();
            lua.load("return picked[1], picked[2]")
                .eval::<(String, i64)>()
                .unwrap()
        };
        assert_eq!(answer(numeric[0], 30000), ("accept".into(), 30000));
        assert_eq!(answer(numeric[1], -1 - 20000), ("cancel".into(), 20000));
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
                dropoff: 1.0,
                hero: 1.0,
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
            feel: Default::default(),
            hint: None,
            ui: None,
            assist: None,
            camera: None,
            scope: None,
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
    fn markers_fade_from_near_to_far() {
        // The first contract's: 2 m over its target, fading 5 m to 175 m.
        let mut m = MarkerView {
            at: Vec3::new(0.0, 2.0, 0.0),
            icon: "HUD_objective_deliverable".into(),
            colour: [255, 200, 0, 255],
            near: 5.0,
            far: 175.0,
            flat: false,
            size: 32.0,
        };
        assert_eq!(m.label(), "deliverable");
        // Close by it shows in full: the beta hid it within `near`.
        assert_eq!(m.alpha_from(Vec3::new(3.0, 2.0, 0.0)), 1.0);
        assert_eq!(m.alpha_from(Vec3::new(0.0, 2.0, 0.0)), 1.0);
        assert_eq!(m.alpha_from(Vec3::new(0.0, 2.0, 5.0)), 1.0);
        // Between, linear in the squared distance: at 100 m,
        // (175² - 100²) / (175² - 5²).
        let mid = m.alpha_from(Vec3::new(0.0, 2.0, 100.0));
        assert!((mid - 20_625.0 / 30_600.0).abs() < 1e-6, "{mid}");
        // Not linear in the distance itself (that would be 75/170).
        assert!((mid - 75.0 / 170.0).abs() > 0.1);
        // It thins further out, and is gone beyond `far`.
        assert!(m.alpha_from(Vec3::new(0.0, 2.0, 150.0)) < mid);
        assert_eq!(m.alpha_from(Vec3::new(0.0, 2.0, 175.0)), 0.0);
        assert_eq!(m.alpha_from(Vec3::new(0.0, 2.0, 175.5)), 0.0);
        assert_eq!(m.alpha_from(Vec3::new(0.0, 0.0, 200.0)), 0.0);
        // Height counts, unless it is "2D only".
        let above = Vec3::new(0.0, 202.0, 3.0);
        assert_eq!(m.alpha_from(above), 0.0);
        m.flat = true;
        assert_eq!(m.alpha_from(above), 1.0);
    }

    #[test]
    fn markers_without_a_fade_show_to_200_m_and_fade_out_by_300() {
        let m = MarkerView {
            at: Vec3::ZERO,
            icon: "HUD_objective_destroy".into(),
            colour: [255, 0, 0, 255],
            near: m2_game::hud::MARKER_NEAR,
            far: m2_game::hud::MARKER_FAR,
            flat: false,
            size: 32.0,
        };
        // 5 m off: shown (the beta's 5 m default hid it there).
        assert_eq!(m.alpha_from(Vec3::new(3.0, 0.0, 0.0)), 1.0);
        assert_eq!(m.alpha_from(Vec3::new(200.0, 0.0, 0.0)), 1.0);
        let a = m.alpha_from(Vec3::new(250.0, 0.0, 0.0));
        assert!((a - 27_500.0 / 50_000.0).abs() < 1e-6, "{a}");
        // Shown at 200 to 300 m, where the beta's 175 m hid it.
        assert!(m.alpha_from(Vec3::new(290.0, 0.0, 0.0)) > 0.0);
        assert_eq!(m.alpha_from(Vec3::new(301.0, 0.0, 0.0)), 0.0);
    }
}
