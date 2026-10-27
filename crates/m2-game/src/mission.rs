//! A mission session: the game's scripts, read from the player's install,
//! running over a world built from its entity layers, as the headless
//! `mission` example and m2-alpha both run them.
//!
//! [`GameData::read`] collects the scripts, the layers' named entities,
//! the region shapes and the English strings from `vz.wad`. [`Mission::new`] loads every module
//! and entry script and sets the modules up; [`Mission::start`] then starts
//! play as the engine does (the bootstrap's `Init`, which starts a new game,
//! then its "GUI loaded" step), and [`Mission::tick`] advances it. Layer
//! requests and AI deliveries finish at once; everything the scripts ask
//! the game to do is also kept for the game in [`Mission::take_commands`],
//! and a readable account of the session in [`Mission::take_log`].

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek};
use std::rc::Rc;

use m2_assets::layer::{Component, LAYER_TYPE, Placement, TEMPLATE_DB_TYPE};
use m2_assets::lua::{Chunk, SCRIPT_TYPE};
use m2_assets::path::PATH_TYPE;
use m2_assets::region::REGION_TYPE;
use m2_assets::strings::STRINGS_TYPE;
use m2_assets::{BlockContents, Layer, Path, Region, StringTable, WadArchive, WadError, name_hash};
use m2_engine::population::{riders, seat_name};
use m2_math::glam::{Quat, Vec3};
use mlua::{Function, Table, Value};

use crate::api::{self, Command, Object, ScriptWorld, Shared};
use crate::events::Events;
use crate::hud::render;
use crate::layers::{LayerOp, LayerState, finished, layer_name};
use crate::library::{KNOWN_MODULES, LOADERS, Library};
use crate::script::{ScriptError, ScriptHost};

/// The script that starts the game (`Init`, then `LoadGame`).
pub const BOOTSTRAP: u32 = 0xb442_0059;

/// The goal a support helicopter is given to land at her mark
/// (`MrxSupportCopterDelivery._HeliReady`).
pub const LAND: &str = "HeliLand";

/// The base block's layer (`vz_base_P000_Q3.block`), always resident.
const BASE_LAYER: &str = "vz_base";
/// Activates the player's character as the engine does its object
/// scripts: the hero module's `OnActivate` (healing, the survival
/// system's one rescue, her inventory), which goes on once she is awake.
const HERO_ACTIVATE: &str = "Hero.OnActivate(Player.GetCharacter(1))";
/// The pending save's script data (`__m2_saved`, Lua text from
/// [`crate::savedata::generate`]) read back into a table.
const LOAD_SAVED: &str = "assert(loadstring('return ' .. __m2_saved, '=m2-saved'))()";

/// The scripts' free support items (`MrxSupportData.AddFreebie`, as the
/// first contract gives its Satellite Guided Bomb) reach the game too:
/// each call goes on as it would, and the game hears of it
/// ([`Command::Freebie`], [`Command::RemoveFreebie`]).
const FOLLOW_FREEBIES: &str = r#"
local data, ears = MrxSupportData, __m2_support
if type(data) ~= "table" or type(data.AddFreebie) ~= "function" then
    return false
end
local add, remove = data.AddFreebie, data.RemoveFreebie
data.AddFreebie = function(...)
    ears.AddFreebie(...)
    return add(...)
end
data.RemoveFreebie = function(...)
    ears.RemoveFreebie(...)
    if type(remove) == "function" then return remove(...) end
end
return true
"#;

/// The PDA map's mission tracking: `MrxMissionFlow.Reset` hands the map
/// `_TrackMission` (`Pda.Map:SetMissionTrackCallback{ fCallback = ... }`,
/// which `MrxGuiInterface` passes on to the PDA widget). The widget calls
/// it with the mission the player picks on the map
/// (`MrxGuiPda._HandleTrackEvent`: the callback's data, then the mission
/// id); a job marks its targets only while it is tracked. The PDA here is
/// a stand-in, so the callback is kept in `__m2_track` for
/// [`Mission::track_mission`] as well.
const FOLLOW_TRACKING: &str = r#"
local pda = rawget(_G, "Pda")
local map = type(pda) == "table" and rawget(pda, "Map")
if type(map) ~= "table" or type(map.SetMissionTrackCallback) ~= "function" then
    return
end
local set = map.SetMissionTrackCallback
map.SetMissionTrackCallback = function(self, config, ...)
    if type(config) == "table" then
        __m2_track = { config.fCallback, config.tCallbackData }
    end
    return set(self, config, ...)
end
"#;

/// The flow's full-screen movies (`_PlayMovie{sMovie = "01_AOA_J",
/// fCallback = ...}`, as `WifMissionFlow` plays the contracts' intro and
/// closing movies) wait for the game instead of ending at once: each
/// request is queued in `__m2_movies` for [`Mission::take_movie`], and
/// [`Mission::movie_done`] then runs its callback as the script's own
/// skip path does (`MrxUtil.CallWithOptionalArgs(fCallback,
/// tCallbackData)`).
const HOLD_MOVIES: &str = r#"
__m2_movies = {}
local held = 0
local function hold(owner)
    local play = rawget(owner, "_PlayMovie")
    if type(play) ~= "function" then
        return
    end
    rawset(owner, "_PlayMovie", function(t)
        if type(t) == "table" and type(t.sMovie) == "string" then
            table.insert(__m2_movies, t)
        else
            return play(t)
        end
    end)
    held = held + 1
end
-- A module's `_PlayMovie` lives in its own table (`WifMissionFlow`),
-- which its functions read it from.
hold(_G)
for _, v in pairs(_G) do
    if type(v) == "table" and v ~= _G then
        hold(v)
    end
end
return held
"#;

/// The screen fade `MrxGui` runs through its own Flash widget
/// (`GlobalFadeToBlack(fCallback, data)`: the `loadingscreen_standalone`
/// movie wipes to black, its `wipeComplete` command runs
/// `_FinishFadeToBlack`, and the next GUI update the callbacks queued, as
/// the teleport after the first contract's closing movie;
/// `GlobalFadeFromBlack` asks the movie to `fadeOut`, whose `close` command
/// runs `_FinishFadeFromBlack`). There is no Flash in the mission's state,
/// so this stands in for the movie: it loads at once and each wipe takes
/// [`FADE_WIPE`] seconds. Returns how black the screen is, 0 to 1.
const FADE: &str = r#"
local dt, wipe = ...
local m = __m2_fade_module
if m == nil then
    for _, t in pairs(_G) do
        if type(t) == "table" and type(rawget(t, "_CompleteFadeFlashLoad")) == "function"
            and type(rawget(t, "_FinishFadeToBlack")) == "function" then
            m = t
            break
        end
    end
    if m == nil then return 0 end
    __m2_fade_module = m
    local from = rawget(m, "GlobalFadeFromBlack")
    if type(from) == "function" then
        rawset(m, "GlobalFadeFromBlack", function(...)
            local was = rawget(m, "_bCurrentlyFadedIn") or rawget(m, "_bCurrentlyFadingIn")
            local r = from(...)
            if was and not rawget(m, "_bCurrentlyFadedIn") then __m2_fade.out = 0 end
            return r
        end)
    end
end
__m2_fade = __m2_fade or {}
local s = __m2_fade
local w = rawget(m, "_oFadeFlash")
if type(w) ~= "table" or type(w.CustomData) ~= "table" then return 0 end
local c = w.CustomData
if c.bLoading and not c.bLoaded then m._CompleteFadeFlashLoad() end
if s.update then
    s.update = nil
    m._FadeUpdate(w)
end
if rawget(m, "_bCurrentlyFadingIn") then
    -- A new fade to black takes over from a fade back that hadn't ended
    -- (the movie restarts its wipe).
    s.out = nil
    s.t = (s.t or 0) + dt
    if s.t >= wipe then
        s.t = nil
        m._FinishFadeToBlack(w)
        s.update = true
        return 1
    end
    return s.t / wipe
end
s.t = nil
if s.out then
    s.out = s.out + dt
    if s.out >= wipe then
        s.out = nil
        if not rawget(m, "_bCurrentlyFadedIn") then m._FinishFadeFromBlack(w) end
        return 0
    end
    return 1 - s.out / wipe
end
if rawget(m, "_bCurrentlyFadedIn") then return 1 end
return 0
"#;

/// Seconds the fade's wipe takes, each way: the movie's 60 frames at
/// 30 fps (`_fadeOn` to `wipeComplete`, `_fadeOff` to `close`; from
/// `Loading.wad`).
const FADE_WIPE: f64 = 2.0;

/// Skip mode (which [`Mission::start`] turns on so the intro movies end
/// at once) also has `MrxUtil.TeleportHeroesToLocations` skip the
/// teleport and only call back: with the movies held for the game, the
/// teleports happen as without it (after the first contract's closing
/// movie, to `PmcCon001_Start1`).
const TELEPORTS: &str = r#"
local util = rawget(_G, "MrxUtil")
local cheat = rawget(_G, "MrxCheatBootstrap")
if type(util) ~= "table" or type(cheat) ~= "table" then return false end
local tele = rawget(util, "TeleportHeroesToLocations")
if type(tele) ~= "function" then return false end
rawset(util, "TeleportHeroesToLocations", function(...)
    local skip = cheat.IsSkipModeEnabled
    cheat.IsSkipModeEnabled = function() return false end
    local ok, err = pcall(tele, ...)
    cheat.IsSkipModeEnabled = skip
    if not ok then error(err, 0) end
end)
return true
"#;

/// Puts `MrxCheatBootstrap.IsSkipModeEnabled` back as the scripts had it
/// before [`Mission::start`] turned skip mode on, if that answers no, or
/// yes while a skip to a mission is under way
/// ([`Mission::set_skip_mission`]: the scripts end it themselves once the
/// contract is reached); else (none, an error, or yes) one that answers
/// no. Returns what the scripts' own answered.
const END_SKIP_MODE: &str = r#"
local cheat = rawget(_G, "MrxCheatBootstrap")
if type(cheat) ~= "table" then return "no MrxCheatBootstrap" end
local own = __m2_own_skip_mode
local said = "none"
if type(own) == "function" then
    local ok, on = pcall(own)
    said = ok and tostring(on) or ("error: " .. tostring(on))
    local skipping = type(cheat.GetMissionSkipData) == "function"
        and cheat.GetMissionSkipData() ~= nil
    if ok and (not on or skipping) then
        cheat.IsSkipModeEnabled = own
        return said
    end
end
cheat.IsSkipModeEnabled = function() return false end
return said
"#;

#[derive(Debug, thiserror::Error)]
pub enum MissionError {
    #[error("archive: {0}")]
    Wad(#[from] WadError),
    #[error("script: {0}")]
    Script(#[from] ScriptError),
    #[error("script: {0}")]
    Lua(#[from] mlua::Error),
}

/// A named entity of a layer.
#[derive(Debug, Clone)]
pub struct Entity {
    /// The layer's entity id, which scripts know it by.
    pub guid: u32,
    pub name: String,
    pub at: [f32; 3],
    pub yaw: f32,
    /// The outline of a region entity (`LineRegion`).
    pub region: Option<Region>,
    /// Its labels' name hashes: its template's and its own (`Label`).
    pub labels: Vec<u32>,
    /// The script the engine runs on it (`ObjectScript`), by module hash.
    pub script: Option<u32>,
    /// The faction a faction zone belongs to (`FactionZone`'s faction
    /// template), as its guid: the name hash of the template's name.
    pub faction: Option<u32>,
    /// A path entity's points (`Path`, [`m2_assets::path`]) in the world:
    /// the route an `Ai.Goal` `PathMove` sends a driver along (PmcCon001's
    /// `Path_PMC001_GarageSmash`). Empty for other entities.
    pub path: Vec<[f32; 3]>,
    /// Who rides in it as it comes into play: (seat, rider template), as
    /// its own `Rider` records say, else its template's (PmcCon001's
    /// villa tank, `Scorpion90 (Driver) (PMC001)`, has a `VZ Tank
    /// Commander` in `seat_driver`). Empty for most.
    pub crew: Vec<(String, String)>,
    /// The template it was placed as (`Mi35 (Solano)` for PmcCon004's
    /// `Mi35 (Solano Hijack)`), by name, when the database names it.
    pub template: Option<String>,
}

impl Entity {
    /// Whether it is a marker: a region, or a place marked with no
    /// template or a location's (`location`, `Paradrop Location (CH)`,
    /// `Landing Zone (player 1)`), not an object with a body. A marker of
    /// a layer that isn't in is still found by name (the HQs' landing
    /// zones a delivery helicopter flies home to); an object is not (the
    /// PMC's `HqInterior`, of `vz_state_pmcinterior`, once that layer is
    /// out). From the data: 1280 of the layers' unlabelled entities are
    /// placed as `location` and 1179 with no template.
    pub fn is_marker(&self) -> bool {
        self.region.is_some()
            || self.template.as_deref().is_none_or(|t| {
                let t = t.to_ascii_lowercase();
                t.contains("location") || t.starts_with("landing zone")
            })
    }
}

/// An object of the layers with a crew of its own aboard
/// ([`Mission::crews_aboard`]): its guid, and each rider's seat, guid and
/// template.
pub type CrewAboard = (u32, Vec<(String, u32, String)>);

/// A rider of a layer's crew out of its seat ([`Mission::crews_on_foot`]):
/// the rider's guid, what it came in on, its template, where it is and
/// its yaw.
pub type OnFoot = (u32, u32, String, [f32; 3], f32);

/// What a mission session needs from the archives.
#[derive(Debug, Default)]
pub struct GameData {
    pub library: Library,
    /// Named entities by layer (lower case).
    pub layers: BTreeMap<String, Vec<Entity>>,
    /// Region shapes read.
    pub shapes: usize,
    /// The game's English text by id, for the HUD's texts.
    pub strings: StringTable,
    /// Templates' labels (name hashes) by the template's name in lower
    /// case, for objects the scripts spawn.
    pub template_labels: HashMap<String, Vec<u32>>,
    /// Every template's name in the template database, in lower case:
    /// the names scripts look templates up by (`Pg.GetGuidByName("OC")`).
    pub template_names: std::collections::HashSet<String>,
    /// Who drives a template's vehicle as its `Rider` in `seat_driver`
    /// names them (`"AH1Z (Ewan)"`: `"Recruit Ewen Garret"`), by the
    /// template's name in lower case, for vehicles the scripts spawn.
    pub template_drivers: HashMap<String, String>,
    /// Who else rides a template's vehicle, as its other `Rider` records
    /// name them (`"Ka29b (Full)"`: the soldiers a soldier delivery
    /// brings), by the template's name in lower case.
    pub template_passengers: HashMap<String, Vec<String>>,
    /// Weapon templates' `MaxAmmoReserve` and which are thrown, by name in
    /// lower case ([`crate::inventory::read`]).
    pub max_reserve: HashMap<String, u32>,
    pub thrown: std::collections::HashSet<String>,
}

/// `Label`'s field: the label's name hash.
const LABEL_FIELD: u32 = 0xfd08_4cce;

/// Every `Label` record's name hash, by entity (or template).
fn labels_of(c: Option<&Component<'_>>) -> HashMap<u32, Vec<u32>> {
    let mut out: HashMap<u32, Vec<u32>> = HashMap::new();
    let Some(c) = c else {
        return out;
    };
    let at = c
        .fields
        .iter()
        .find(|f| f.name_hash == LABEL_FIELD)
        .map_or(0, |f| usize::from(f.offset));
    for (id, r) in c.records().unwrap_or_default() {
        if let Some(b) = r.get(at..at + 4) {
            out.entry(id)
                .or_default()
                .push(u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        }
    }
    out
}

/// `Rider` records: (seat name hash, person template id).
type Riders = Vec<(u32, u32)>;

/// The top bit of a template id: the template database's `UNIQ` ids have
/// it, and entities' parents and riders name templates with or without
/// it (as `m2_engine`'s crews read them).
const TEMPLATE_BIT: u32 = 0x8000_0000;

/// The string table the HUD's texts come from.
pub const LANGUAGE: &str = "english";

impl GameData {
    /// Reads the scripts, layers, region shapes and the English string
    /// table of an archive (`vz.wad`). Blocks that don't parse are
    /// skipped.
    pub fn read<R: Read + Seek>(wad: &mut WadArchive<R>) -> Result<Self, MissionError> {
        let paths: Vec<String> = wad
            .index()
            .blocks()
            .iter()
            .map(|b| b.path.clone())
            .collect();
        let mut data = Self::default();
        let mut shapes: HashMap<u32, Region> = HashMap::new();
        // Entities with the shape hash their LineRegion names.
        let mut regions: Vec<(String, usize, u32)> = Vec::new();
        // Entities' templates, to give them their templates' labels.
        let mut templated: Vec<(String, usize, u32)> = Vec::new();
        let mut template_labels: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut template_scripts: HashMap<u32, u32> = HashMap::new();
        // Faction zones' faction templates, and every template's name.
        let mut zone_factions: Vec<(String, usize, u32)> = Vec::new();
        let mut template_names: HashMap<u32, String> = HashMap::new();
        // Path assets' points, and the path entities naming them.
        let mut path_points: HashMap<u32, Vec<[f32; 3]>> = HashMap::new();
        let mut routes: Vec<(String, usize, u32, Placement)> = Vec::new();
        // `Rider` records (seat hash, person template id): the template
        // database's by template id, and entities' own.
        let mut template_riders: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
        let mut own_riders: Vec<(String, usize, Riders)> = Vec::new();
        let language = name_hash(LANGUAGE);
        for (block, path) in paths.iter().enumerate() {
            let decoded = wad.read_block(block)?;
            let Ok(contents) = BlockContents::parse(&decoded) else {
                continue;
            };
            for (entry, asset) in contents.iter() {
                if entry.type_hash == SCRIPT_TYPE {
                    if let Ok(c) = Chunk::from_asset(asset) {
                        data.library.add(entry.name_hash, c);
                    }
                } else if entry.type_hash == PATH_TYPE {
                    if let Ok(p) = Path::parse(asset) {
                        path_points.insert(entry.name_hash, p.points);
                    }
                } else if entry.type_hash == REGION_TYPE {
                    if let Ok(r) = Region::parse(asset) {
                        shapes.insert(entry.name_hash, r);
                    }
                } else if entry.type_hash == STRINGS_TYPE && entry.name_hash == language {
                    if let Ok(t) = StringTable::parse(asset) {
                        data.strings = t;
                    }
                } else if entry.type_hash == TEMPLATE_DB_TYPE
                    && let Ok(db) = Layer::parse(asset)
                {
                    template_labels = labels_of(db.component("Label"));
                    template_scripts = db.hashes("ObjectScript");
                    template_riders = db.component("Rider").map(|c| riders(c)).unwrap_or_default();
                    (data.max_reserve, data.thrown) = crate::inventory::read(&db);
                    template_names = db
                        .names()
                        .into_iter()
                        .map(|(id, n)| (id, n.to_owned()))
                        .collect();
                    data.template_names = template_names
                        .values()
                        .map(|n| n.to_ascii_lowercase())
                        .collect();
                    for (id, name) in db.names() {
                        if let Some(l) = template_labels.get(&id) {
                            data.template_labels
                                .insert(name.to_ascii_lowercase(), l.clone());
                        }
                    }
                } else if entry.type_hash == LAYER_TYPE
                    && let Ok(layer) = Layer::parse(asset)
                {
                    let name = layer_name(path);
                    let list = data.layers.entry(name.clone()).or_default();
                    let lines = layer.hashes("LineRegion");
                    let scripts = layer.hashes("ObjectScript");
                    let zones = layer.hashes("FactionZone");
                    let path_of = layer.hashes("Path");
                    let mut rides = layer
                        .component("Rider")
                        .map(|c| riders(c))
                        .unwrap_or_default();
                    let placements = layer.placements();
                    let mut own = labels_of(layer.component("Label"));
                    for (guid, entity) in layer.names() {
                        let p = placements.get(&guid);
                        if let Some(&h) = lines.get(&guid) {
                            regions.push((name.clone(), list.len(), h));
                        }
                        if let Some(p) = p {
                            templated.push((name.clone(), list.len(), p.parent));
                        }
                        if let Some(&t) = zones.get(&guid) {
                            zone_factions.push((name.clone(), list.len(), t));
                        }
                        list.push(Entity {
                            guid,
                            name: entity.to_owned(),
                            at: p.map_or([0.0; 3], |p| p.position),
                            yaw: p.map_or(0.0, |p| yaw(p.rotation)),
                            region: None,
                            labels: own.remove(&guid).unwrap_or_default(),
                            script: scripts.get(&guid).copied(),
                            faction: None,
                            path: Vec::new(),
                            template: None,
                            crew: Vec::new(),
                        });
                        if let Some(r) = rides.remove(&guid) {
                            own_riders.push((name.clone(), list.len() - 1, r));
                        }
                        if let (Some(&asset), Some(p)) = (path_of.get(&guid), p) {
                            routes.push((name.clone(), list.len() - 1, asset, *p));
                        }
                    }
                }
            }
        }
        for (layer, i, template) in &templated {
            let e = &mut data.layers.get_mut(layer).expect("indexed above")[*i];
            if let Some(l) = template_labels.get(template) {
                e.labels.extend(l.iter().copied());
            }
            // An entity without a script of its own runs its template's:
            // the faction gates' (`_ocoutpost_wallgate`, `friendlygate`).
            if e.script.is_none() {
                e.script = template_scripts.get(template).copied();
            }
        }
        for (layer, i, template) in zone_factions {
            if let Some(n) = template_names.get(&template) {
                data.layers.get_mut(&layer).expect("indexed above")[i].faction = Some(name_hash(n));
            }
        }
        for (layer, i, hash) in regions {
            if let Some(r) = shapes.get(&hash) {
                data.layers.get_mut(&layer).expect("indexed above")[i].region = Some(r.clone());
            }
        }
        // Crews: an entity's own riders, else its template's; each rider
        // named by its template. Template ids are looked up with and
        // without their top bit, as the engine's crews are.
        let named = |id: u32| {
            template_names
                .get(&id)
                .or_else(|| template_names.get(&(id | TEMPLATE_BIT)))
                .cloned()
        };
        let crew_of = |records: &[(u32, u32)]| -> Vec<(String, String)> {
            records
                .iter()
                .filter_map(|&(seat, person)| Some((seat_name(seat), named(person)?)))
                .collect()
        };
        for (layer, i, template) in &templated {
            let records = template_riders
                .get(template)
                .or_else(|| template_riders.get(&(template & !TEMPLATE_BIT)));
            let e = &mut data.layers.get_mut(layer).expect("indexed above")[*i];
            if let Some(r) = records {
                e.crew = crew_of(r);
            }
            e.template = named(*template);
        }
        for (&template, records) in &template_riders {
            let driver = crew_of(records)
                .into_iter()
                .find(|(seat, _)| seat == "seat_driver");
            if let (Some(name), Some((_, driver))) = (named(template), driver) {
                data.template_drivers
                    .insert(name.to_ascii_lowercase(), driver);
            }
            let others: Vec<String> = crew_of(records)
                .into_iter()
                .filter(|(seat, _)| seat != "seat_driver")
                .map(|(_, rider)| rider)
                .collect();
            if let (Some(name), false) = (named(template), others.is_empty()) {
                data.template_passengers
                    .insert(name.to_ascii_lowercase(), others);
            }
        }
        for (layer, i, records) in own_riders {
            data.layers.get_mut(&layer).expect("indexed above")[i].crew = crew_of(&records);
        }
        for (layer, i, asset, placed) in routes {
            if let Some(points) = path_points.get(&asset) {
                data.layers.get_mut(&layer).expect("indexed above")[i].path =
                    place_points(points, &placed);
            }
        }
        data.shapes = shapes.len();
        Ok(data)
    }

    /// The transit landing zones, by player (1, 2) and then zone number.
    pub fn landing_zones(&self) -> [BTreeMap<i64, u32>; 2] {
        let mut out: [BTreeMap<i64, u32>; 2] = Default::default();
        for e in self.layers.values().flatten() {
            if let Some((n, player)) = landing_zone(&e.name) {
                out[player - 1].insert(n, e.guid);
            }
        }
        out
    }
}

/// A transit landing zone's number and player (1 or 2), from its name:
/// `06_gua_upperclass_lz_playerone` is zone 6 for player 1 (one is
/// spelled `_llz_`).
pub fn landing_zone(name: &str) -> Option<(i64, usize)> {
    let lower = name.to_ascii_lowercase();
    let (rest, player) = if let Some(r) = lower.strip_suffix("z_playerone") {
        (r, 1)
    } else {
        (lower.strip_suffix("z_playertwo")?, 2)
    };
    let rest = rest
        .strip_suffix('l')?
        .trim_end_matches('l')
        .strip_suffix('_')?;
    let n = rest.get(..2)?.parse().ok()?;
    rest[2..].starts_with('_').then_some((n, player))
}

/// A path's points, in the space of the entity that names it, placed in
/// the world by that entity's `Transform` (as [`m2_assets::path`] says;
/// a rotation that isn't one counts as none).
fn place_points(points: &[[f32; 3]], placed: &Placement) -> Vec<[f32; 3]> {
    let q = Quat::from_array(placed.rotation);
    let q = if q.is_finite() && q.length() > 0.5 {
        q.normalize()
    } else {
        Quat::IDENTITY
    };
    let at = Vec3::from(placed.position);
    points
        .iter()
        .map(|&p| (at + q * Vec3::from(p)).to_array())
        .collect()
}

/// The turn about the vertical axis of an (x, y, z, w) rotation.
fn yaw([x, y, z, w]: [f32; 4]) -> f32 {
    (2.0 * (w * y + x * z)).atan2(1.0 - 2.0 * (x * x + y * y))
}

/// A running mission session.
#[derive(Debug)]
pub struct Mission {
    host: ScriptHost,
    events: Events,
    world: Shared,
    layers: BTreeMap<String, Vec<Entity>>,
    strings: StringTable,
    /// Modules loaded before play started.
    started: usize,
    /// Game time, seconds.
    now: f64,
    /// When the current `WaitFor...` game state was asked for.
    waiting_since: Option<f64>,
    log: Vec<String>,
    outbox: Vec<Command>,
    /// The game flies deliveries ([`fly_deliveries`](Self::fly_deliveries)).
    flown: bool,
    /// A held movie was handed to the game and has not ended yet.
    movie_out: bool,
    /// How black `MrxGui`'s fade has the screen ([`FADE`]), 0 to 1.
    black: f32,
    /// [`FADE`], compiled once (on the first tick) rather than every tick.
    fade: Option<mlua::Function>,
}

impl Mission {
    /// Loads every module and entry script, with the player `hero`
    /// (`"Mattias"`, `"Jennifer"` or `"Chris"`) standing at `at`, and sets
    /// the modules up. Failures to load or set up are logged.
    pub fn new(data: GameData, hero: &str, at: [f32; 3]) -> Result<Self, MissionError> {
        let world: Shared = Rc::new(RefCell::new(ScriptWorld::default()));
        let [one, two] = data.landing_zones();
        let GameData {
            mut library,
            layers,
            shapes,
            strings,
            template_labels,
            template_names,
            template_drivers,
            template_passengers,
            max_reserve,
            thrown,
        } = data;
        let regions = layers
            .values()
            .flatten()
            .filter(|e| e.region.is_some())
            .count();
        let mut log = vec![format!(
            "{} scripts; {} layers; {shapes} region shapes; {regions} regions; {} landing zones; {} strings",
            library.scripts().len(),
            layers.len(),
            one.len(),
            strings.strings.len()
        )];
        {
            let mut w = world.borrow_mut();
            w.layers.known = layers.keys().cloned().collect();
            w.template_labels = template_labels;
            w.template_names = template_names;
            w.template_drivers = template_drivers;
            w.template_passengers = template_passengers;
            w.inventory.max_reserve = max_reserve;
            w.inventory.thrown = thrown;
            for e in layers.values().flatten() {
                w.entities.insert(e.guid, e.at);
                if e.is_marker() {
                    w.entity_names
                        .entry(e.name.to_ascii_lowercase())
                        .or_insert(e.guid);
                }
            }
            w.landing_zones.insert(1, one);
            w.landing_zones.insert(2, two);
            let mut player = Object::new(hero, at);
            player.player_controlled = true;
            // Scripts tell the heroes apart by these labels.
            player.labels.push(hero.to_ascii_lowercase());
            w.insert(PLAYER, player);
            w.player = Some(PLAYER);
        }
        // The static layers are always in, and so is the base block's
        // (`vz_base`, from samples: never loaded or unloaded by name). Its
        // named locators are objects the scripts act on: PmcCon003's "Get
        // inside the PMC!" prompts at `PlayerLocation_PMC` (round 26: with
        // no object there its target filter came out empty).
        for (name, entities) in &layers {
            if LayerState::is_static(name) || name.eq_ignore_ascii_case(BASE_LAYER) {
                place(&world, entities, true);
                world.borrow_mut().layers.loaded.insert(name.clone());
            }
        }
        for (name, hash) in KNOWN_MODULES.into_iter().chain(OBJECT_SCRIPTS) {
            library.pin(name, hash);
        }
        let library = Rc::new(library);

        let host = ScriptHost::new()?;
        host.stub_missing_globals()?;
        api::install(host.lua(), &world)?;
        host.complete_libraries(&api::LIBRARIES)?;
        let events = Events::default();
        events.install(host.lua())?;
        crate::hijack::install(&host, &world)?;
        host.install_library(Rc::clone(&library), true)?;
        // The contracts' own globals read nil while unset, as in the game,
        // not stubs: every global a contract's or job's script assigns.
        let contracts: std::collections::BTreeSet<&str> = library
            .scripts()
            .iter()
            .flat_map(|s| s.chunk.main.strings())
            .filter(|n| crate::script::is_contract_module(n))
            .collect();
        let own: std::collections::BTreeSet<String> = contracts
            .iter()
            .filter_map(|&n| library.resolve(n).map(|(s, _, _)| s))
            .flat_map(|s| s.chunk.main.names(s.chunk.version))
            .filter(|(u, _)| *u == m2_assets::lua::NameUse::SetGlobal)
            .map(|(_, n)| n)
            .collect();
        host.nil_globals(own.iter().map(String::as_str))?;

        // Every module, by the name scripts spell it with, so code run
        // later can use any module as a global.
        let mut spelled: BTreeMap<String, String> = BTreeMap::new();
        for script in library.scripts() {
            for call in script.chunk.main.calls() {
                if let Some(name) = call
                    .first
                    .filter(|_| LOADERS.contains(&call.callee.as_str()))
                {
                    spelled.entry(name.to_ascii_lowercase()).or_insert(name);
                }
            }
        }
        // The object scripts the port runs: no script imports them by name
        // (the engine loads them for their objects), but the faction
        // manager's `Init` calls `FactionZone.Init` as a global.
        for (name, _) in OBJECT_SCRIPTS {
            spelled
                .entry(name.to_ascii_lowercase())
                .or_insert_with(|| name.to_owned());
        }
        let globals = host.lua().globals();
        let import: Function = globals.get("import")?;
        let mut failures = Vec::new();
        for name in spelled.values() {
            match import.call::<mlua::Value>(name.as_str()) {
                Ok(module) => globals.raw_set(name.as_str(), module)?,
                Err(e) => failures.push(format!("module {name}: {e}")),
            }
        }
        // Entry scripts' environments, as `Scripts.<hash>`.
        let modules: Vec<u32> = host.modules().iter().filter_map(|m| m.1).collect();
        let entries = host.lua().create_table()?;
        for script in library.scripts() {
            if modules.contains(&script.hash) {
                continue;
            }
            match host.run_script(&library, script.hash) {
                Ok(env) => entries.raw_set(format!("{:08x}", script.hash), env)?,
                Err(e) => failures.push(format!("script {:08x}: {e}", script.hash)),
            }
        }
        globals.raw_set("Scripts", entries)?;
        for f in host.init_modules()? {
            failures.push(format!("Init of {f}"));
        }
        let started = host.modules().len();
        log.push(format!(
            "started: {started} modules, {} failures",
            failures.len()
        ));
        log.extend(failures.iter().map(|f| format!("  {}", first_lines(f))));
        let mut mission = Self {
            host,
            events,
            world,
            layers,
            strings,
            started,
            now: 0.0,
            waiting_since: None,
            log,
            outbox: Vec::new(),
            flown: false,
            movie_out: false,
            black: 0.0,
            fade: None,
        };
        mission.settle("set-up");
        Ok(mission)
    }

    /// Puts the layers whose names contain `part` (any case) in the world
    /// now, as if the scripts had loaded them; returns their names.
    pub fn preload(&mut self, part: &str) -> Vec<String> {
        let part = part.to_ascii_lowercase();
        let names: Vec<String> = self
            .layers
            .keys()
            .filter(|n| n.contains(&part))
            .cloned()
            .collect();
        for name in &names {
            place(&self.world, &self.layers[name], true);
            self.world.borrow_mut().layers.loaded.insert(name.clone());
            let failed = activate(self.host.lua(), &self.layers[name], true);
            self.log
                .extend(failed.into_iter().map(|f| format!("  {f}")));
        }
        names
    }

    /// Puts the entity called `name` (any case) of any layer in the world,
    /// with its region, without running its script: for a second view of
    /// the world (the game HUD's) to find what the mission's scripts put
    /// on the radar. Its guid, if one is called that.
    pub fn adopt(&self, name: &str) -> Option<u32> {
        let e = self
            .layers
            .values()
            .flatten()
            .find(|e| e.name.eq_ignore_ascii_case(name))?;
        if !self.world.borrow().objects.contains_key(&e.guid) {
            place(&self.world, std::slice::from_ref(e), true);
        }
        Some(e.guid)
    }

    /// The layers in the world now, lower case: blocks of their own
    /// (`vz_state_vzacon001`) and the layers inside `layers_static`
    /// (`vz_mar_village`), which the game loads by name.
    pub fn loaded_layers(&self) -> Vec<String> {
        self.world.borrow().layers.loaded.iter().cloned().collect()
    }

    /// The entities of a layer that is a block of its own (lower case).
    pub fn layer(&self, name: &str) -> Option<&[Entity]> {
        self.layers.get(name).map(Vec::as_slice)
    }

    /// The world as the scripts see it.
    pub fn world(&self) -> &Shared {
        &self.world
    }

    pub fn lua(&self) -> &mlua::Lua {
        self.host.lua()
    }

    /// Runs Lua (a test driver, say) with every module as a global, then
    /// carries out what it asked for.
    pub fn run(&mut self, code: &str, name: &str) -> Result<(), MissionError> {
        let result = self
            .host
            .lua()
            .load(code)
            .set_name(format!("={name}"))
            .exec();
        self.settle(name);
        Ok(result?)
    }

    /// Has [`start`](Self::start) load a save: `text` is the scripts' save
    /// data as [`save_data`](Self::save_data) wrote it. The scripts then
    /// carry on from it (the contracts done and on offer, her PMC, the
    /// factions...) instead of starting a new game.
    pub fn load_save(&mut self, text: String) {
        self.world.borrow_mut().saved = Some(text);
    }

    /// Starts play as the engine does: intro movies skipped, the
    /// bootstrap's `Init` (which starts a new game when there is no save,
    /// and is otherwise handed the save's data: [`load_save`](Self::load_save)),
    /// then the step the engine runs once the GUI has loaded. The first
    /// contract's mission, or the save's next, is then waiting.
    pub fn start(&mut self) {
        self.follow_freebies();
        if let Err(e) = self.run(FOLLOW_TRACKING, "follow tracking") {
            self.log.push(format!(
                "  mission tracking: {}",
                first_lines(&e.to_string())
            ));
        }
        let saved = self.world.borrow().saved.clone();
        if let Some(text) = &saved {
            if let Err(e) = self.host.lua().globals().set("__m2_saved", text.as_str()) {
                self.log.push(format!("  load save: {e}"));
            }
            self.log.push(format!(
                "save: loading the scripts' save data, {} bytes",
                text.len()
            ));
        }
        let steps = [
            (
                "skip mode",
                "__m2_own_skip_mode = MrxCheatBootstrap.IsSkipModeEnabled\n\
                 MrxCheatBootstrap.IsSkipModeEnabled = function() return true end"
                    .to_owned(),
            ),
            ("bootstrap", format!("Scripts[\"{BOOTSTRAP:08x}\"].Init()")),
            // What the engine does when `Pg.LoadGame` said a save loads.
            (
                "load save",
                if saved.is_some() {
                    format!("Scripts[\"{BOOTSTRAP:08x}\"].LoadSingleton({LOAD_SAVED})")
                } else {
                    String::new()
                },
            ),
            // The callback MrxGuiBootstrap.SetOnGuiLoadedFunc registers.
            ("gui loaded", "MrxBootstrap._GuiLoaded()".to_owned()),
            ("hero activated", HERO_ACTIVATE.to_owned()),
        ];
        for (name, code) in steps {
            if code.is_empty() {
                continue;
            }
            if let Err(e) = self.run(&code, name) {
                self.log
                    .push(format!("  {name}: {}", first_lines(&e.to_string())));
            }
        }
        if saved.is_some() {
            self.world.borrow_mut().saved = None;
            let _ = self.host.lua().globals().set("__m2_saved", Value::Nil);
            // A save has no intro movies to skip, and in skip mode the
            // scripts don't move her to the save's place
            // (`MrxUtil.TeleportHeroesToLocations` only calls back).
            self.end_skip_mode();
        }
    }

    /// Turns the scripts' developer skip mode, which [`start`](Self::start)
    /// turns on so the intro movies end at once, off again once the movies
    /// wait for the game ([`hold_movies`](Self::hold_movies)): for the
    /// rest of play the scripts that ask (`MrxCheatBootstrap.
    /// IsSkipModeEnabled`) are told no. The scripts' own answer is logged,
    /// and their own function put back when it answers no.
    pub fn end_skip_mode(&mut self) {
        let ended = self
            .host
            .lua()
            .load(END_SKIP_MODE)
            .set_name("=end skip mode")
            .eval::<String>();
        self.log.push(match ended {
            Ok(own) => format!("skip mode: off after the start (the scripts' own: {own})"),
            Err(e) => format!("skip mode: {}", first_lines(&e.to_string())),
        });
    }

    /// Has the scripts' free support items reach the game
    /// ([`FOLLOW_FREEBIES`]); logs whether they can.
    pub fn follow_freebies(&mut self) {
        let followed = self
            .host
            .lua()
            .load(FOLLOW_FREEBIES)
            .set_name("=follow freebies")
            .eval::<bool>();
        self.log.push(match followed {
            Ok(true) => "free support items: followed (MrxSupportData.AddFreebie)".to_owned(),
            Ok(false) => "free support items: no MrxSupportData.AddFreebie to follow".to_owned(),
            Err(e) => format!("free support items: {}", first_lines(&e.to_string())),
        });
    }

    /// Advances game time by `dt` seconds: fires the events that are due
    /// and carries out what their callbacks asked for.
    pub fn tick(&mut self, dt: f64) {
        self.now += dt;
        let now = self.now;
        {
            let mut w = self.world.borrow_mut();
            w.honk_heard = (w.honk_heard - dt as f32).max(0.0);
        }
        // Streaming is instant here: a waiting state ends a second after
        // it was asked for, and the scripts hear that it did.
        {
            let mut w = self.world.borrow_mut();
            if w.game_state.starts_with("WaitFor") {
                let since = *self.waiting_since.get_or_insert(now);
                if now - since >= 1.0 {
                    self.log
                        .push(format!("t={now:.1} game state {} done", w.game_state));
                    w.game_state = "Game".into();
                    self.waiting_since = None;
                }
            }
        }
        self.world.borrow_mut().update_hibernation();
        for t in self.events.take_trace() {
            self.log.push(format!("  t={:.1} event {t}", self.now));
        }
        for e in self.events.update_shared(dt, &self.world) {
            self.log.push(format!(
                "  t={now:.1} {} callback: {}",
                e.kind,
                first_lines(&e.error.to_string())
            ));
        }
        for kind in self.events.take_unhandled() {
            self.log.push(format!(
                "  t={now:.1} events: unhandled kind {kind} (its events never fire)"
            ));
        }
        self.check_boundary(now);
        for e in crate::api::answer_asset_loads(&self.world) {
            self.log.push(format!(
                "  t={now:.1} asset load callback: {}",
                first_lines(&e.to_string())
            ));
        }
        for e in crate::api::answer_drop_tests(&self.world) {
            self.log.push(format!(
                "  t={now:.1} drop zone callback: {}",
                first_lines(&e.to_string())
            ));
        }
        for (cue, error) in crate::vo::show_subtitles(&self.world, self.host.lua(), now) {
            self.log.push(match error {
                None => format!("  t={now:.1} subtitle {cue}"),
                Some(e) => format!(
                    "  t={now:.1} subtitle {cue}: {}",
                    first_lines(&e.to_string())
                ),
            });
        }
        let ended = crate::vo::update(&self.world, self.host.lua(), now);
        for (cue, error) in ended {
            self.log.push(match error {
                None => format!("  t={now:.1} line {cue} ended"),
                Some(e) => format!(
                    "  t={now:.1} line {cue} ended; callback: {}",
                    first_lines(&e.to_string())
                ),
            });
        }
        if self.fade.is_none() {
            match self
                .host
                .lua()
                .load(FADE)
                .set_name("=m2 fade")
                .into_function()
            {
                Ok(f) => self.fade = Some(f),
                Err(e) => self.log.push(format!(
                    "  t={now:.1} fade: {}",
                    first_lines(&e.to_string())
                )),
            }
        }
        let faded = self.fade.as_ref().map(|f| f.call::<f32>((dt, FADE_WIPE)));
        match faded.unwrap_or(Ok(self.black)) {
            Ok(b) => {
                if (b >= 1.0) != (self.black >= 1.0) {
                    self.log.push(format!(
                        "  t={now:.1} screen {}",
                        if b >= 1.0 { "black" } else { "fading in" }
                    ));
                }
                self.black = b;
            }
            Err(e) => self.log.push(format!(
                "  t={now:.1} fade: {}",
                first_lines(&e.to_string())
            )),
        }
        self.settle(&format!("t={now:.1}"));
    }

    /// How black the scripts' fade has the screen, 0 to 1.
    pub fn black(&self) -> f32 {
        self.black
    }

    /// Tells the boundary callback when the player leaves the play area
    /// (zone `"out"`, `"enter"`) or comes back (`"exit"`). The player is
    /// out when inside none of the boundaries the game can place.
    fn check_boundary(&mut self, now: f64) {
        let call = {
            let mut w = self.world.borrow_mut();
            let Some(f) = w.boundary_callback.clone() else {
                return;
            };
            // While a state change holds her (the black screen as a
            // contract loads, `MrxState`), she is still at the last spot
            // and about to be placed: not a real exit.
            if w.invincible
                .get(&PLAYER)
                .is_some_and(|r| r.contains("MrxState"))
            {
                return;
            }
            // An exclusion region (`WifVzBoundary.EnableExclusionBoundary`:
            // `Boundary_ALHQ_Exclusion`, the Allied HQ while it is shut to
            // her) is out of bounds inside it, the others outside them.
            let (mut found, mut excluded) = (Vec::new(), false);
            for &b in &w.boundaries {
                let Some(inside) = crate::events::World::inside(&*w, PLAYER, b) else {
                    continue;
                };
                if w.objects.get(&b).is_some_and(|o| is_exclusion(&o.name)) {
                    excluded |= inside;
                } else {
                    found.push(inside);
                }
            }
            let out = excluded || (!found.is_empty() && !found.contains(&true));
            if out == w.outside {
                return;
            }
            w.outside = out;
            (f, if out { "enter" } else { "exit" })
        };
        let (f, event) = call;
        self.log
            .push(format!("  t={now:.1} play area: out {event}"));
        if let Err(e) = f.call::<()>((mlua::Value::Nil, "out", event)) {
            self.log.push(format!(
                "  t={now:.1} boundary callback: {}",
                first_lines(&e.to_string())
            ));
        }
    }

    /// The game is playing scripted line `id` (a [`Command::Say`]), which
    /// lasts `seconds`.
    pub fn line_length(&mut self, id: u32, seconds: f64) {
        self.world.borrow_mut().vo.set_length(id, seconds);
    }

    /// Starts a hijack as the engine does when the player takes a
    /// vehicle at its hijack entrance: script `script` (`hijack_m1a2`'s
    /// name hash; see `m2_engine::hijack`) loads its clips (`Init`) and
    /// plays `StartHijack(hijacker, crewman, "d", vehicle)`. False if the
    /// script refused or failed (the failure is logged).
    pub fn start_hijack(&mut self, hijacker: u32, crewman: u32, vehicle: u32, script: u32) -> bool {
        use crate::events::guid_value;
        let run = || -> mlua::Result<bool> {
            let scripts: mlua::Table = self.host.lua().globals().get("Scripts")?;
            let Some(s) = scripts.get::<Option<mlua::Table>>(format!("{script:08x}"))? else {
                return Ok(false);
            };
            let Some(start) = s.get::<Option<mlua::Function>>("StartHijack")? else {
                return Ok(false);
            };
            if let Some(init) = s.get::<Option<mlua::Function>>("Init")? {
                init.call::<()>(())?;
            }
            let started: mlua::Value = start.call((
                guid_value(hijacker),
                guid_value(crewman),
                "d",
                guid_value(vehicle),
            ))?;
            Ok(matches!(started, mlua::Value::Boolean(true)))
        };
        let started = run();
        self.settle("hijack");
        match started {
            Ok(b) => b,
            Err(e) => {
                self.log.push(format!(
                    "  hijack {script:08x}: {}",
                    first_lines(&e.to_string())
                ));
                false
            }
        }
    }

    /// Spawns a template (`"M1A2"`) into the scripts' world as the
    /// scripts' `Pg.Spawn` does, facing north; its guid.
    pub fn spawn(&mut self, template: &str, at: [f32; 3]) -> Option<u32> {
        let v: mlua::Value = self
            .host
            .lua()
            .load(format!(
                "return Pg.Spawn({template:?}, {}, {}, {}, 0)",
                at[0], at[1], at[2]
            ))
            .eval()
            .ok()?;
        crate::events::as_guid(&v)
    }

    /// The minigames waiting on the player's pad (a hijack's prompts).
    pub fn minigames(&self) -> Vec<crate::hijack::Minigame> {
        self.events.minigames()
    }

    /// The player pressed a pad button, or started or stopped holding it
    /// (`crate::hijack::BUTTONS` numbers).
    pub fn press(&mut self, button: u32) {
        let now = self.now;
        self.world.borrow_mut().hijack.press(button, now);
    }

    /// She did an infraction of `kind` (a [`crate::report::KINDS`] key) to
    /// the faction labelled `faction` (`OC`, `Allied`): its next reporter
    /// reports it.
    pub fn record_infraction(&mut self, faction: &str, kind: &'static str) {
        self.world.borrow_mut().reports.record(faction, kind);
    }

    /// Offers witness `reporter` of the faction labelled `faction` to the
    /// faction manager as its reporter (with what the faction has against
    /// her so far): true if he reports, false if it turned him down or no
    /// manager listens.
    pub fn begin_report(&mut self, reporter: u32, faction: &str) -> bool {
        {
            let mut w = self.world.borrow_mut();
            if w.reports.callback.is_none() {
                return false;
            }
            w.reports.offer(reporter, faction);
        }
        if let Err(e) = crate::report::tell(&self.world, reporter, 0) {
            self.world.borrow_mut().printed.push(format!("report: {e}"));
        }
        self.world
            .borrow()
            .reports
            .reporting
            .contains_key(&reporter)
    }

    /// Reporter `reporter` got through and is reporting (state 1): the
    /// faction manager starts its reporting display, whose timer ends it
    /// (`FinishedReporting`, then `Report.Completed`).
    pub fn report_through(&mut self, reporter: u32) {
        if let Err(e) = crate::report::tell(&self.world, reporter, 1) {
            self.world.borrow_mut().printed.push(format!("report: {e}"));
        }
    }

    /// Reporter `reporter` was stopped (killed) before the manager was
    /// done with him (state 2): his report is lost.
    pub fn stop_report(&mut self, reporter: u32) {
        if let Err(e) = crate::report::tell(&self.world, reporter, 2) {
            self.world.borrow_mut().printed.push(format!("report: {e}"));
        }
        let mut w = self.world.borrow_mut();
        if w.reports.reporting.remove(&reporter).is_some() {
            w.reports.ended.push((reporter, false));
        }
    }

    /// Reporters the manager finished with since the last call, each with
    /// whether he reported.
    pub fn take_report_ends(&mut self) -> Vec<(u32, bool)> {
        self.world.borrow_mut().reports.take_ended()
    }

    /// The cues of the radio playlist with name hash `hash`, as the
    /// scripts filled it ([`crate::music::Music::playlist`]).
    pub fn playlist(&self, hash: u32) -> Vec<String> {
        self.world.borrow().music.playlist(hash).to_vec()
    }

    /// The pursuit the scripts set ran out: `Pg.GetPursuitState` reads
    /// level 0 again.
    pub fn pursuit_over(&mut self) {
        let mut w = self.world.borrow_mut();
        w.pursuit.level = 0;
        w.pursuit.faction = None;
    }

    /// Whether character `guid` swims (`Human.IsSwimming`).
    pub fn set_swimming(&mut self, guid: u32, on: bool) {
        let mut w = self.world.borrow_mut();
        if on {
            w.swimming.insert(guid);
        } else {
            w.swimming.remove(&guid);
        }
    }

    /// Whether aircraft `guid` is in the air (`Vehicle.IsFlying`).
    pub fn set_flying(&mut self, guid: u32, flying: bool) {
        self.world.borrow_mut().hijack.set_flying(guid, flying);
    }

    /// Where `guid`'s hardpoint (`hp_seat_lt`, a node of its model, by
    /// name hash) is in the world now; hijacks set off their explosions
    /// there.
    pub fn set_hardpoint(&mut self, guid: u32, name: u32, at: [f32; 3]) {
        self.world.borrow_mut().hijack.set_hardpoint(guid, name, at);
    }

    pub fn hold(&mut self, button: u32, down: bool) {
        let at = down.then_some(self.now);
        self.world.borrow_mut().hijack.hold(button, at);
    }

    /// The prompt (`[ContextAction.Subdue]`) of the context action offered
    /// beside `guid`, if one is.
    pub fn context_text(&self, guid: u32) -> Option<String> {
        self.world
            .borrow()
            .context_actions
            .get(&guid)
            .map(|a| a.text.clone())
    }

    /// Character `guid` is now in human state `state` (`Stance.Action`:
    /// `KnockedDown.Idle`), for the scripts' `HumanStateTransition` events.
    pub fn set_human_state(&mut self, guid: u32, state: &str) {
        let mut w = self.world.borrow_mut();
        let from = w
            .human_states
            .insert(guid, state.to_owned())
            .unwrap_or_else(|| "Upright.Idle".to_owned());
        if from != state {
            w.state_changes
                .push((self.now, guid, from, state.to_owned()));
            let extra = w.state_changes.len().saturating_sub(32);
            w.state_changes.drain(..extra);
        }
    }

    /// Character `carrier` now carries `carried` (or nobody), for
    /// `Human.IsCarrying`.
    pub fn set_carrying(&mut self, carrier: u32, carried: Option<u32>) {
        let mut w = self.world.borrow_mut();
        match carried {
            Some(c) => w.carrying.insert(carrier, c),
            None => w.carrying.remove(&carrier),
        };
    }

    /// A character's action (a [`crate::hijack::HijackCommand::State`]
    /// key) played to its end.
    pub fn action_finished(&mut self, guid: u32) {
        self.world.borrow_mut().hijack.action_finished(guid);
    }

    /// Game time so far.
    pub fn now(&self) -> f64 {
        self.now
    }

    /// Moves the player (and anything it rides in).
    pub fn put_player(&mut self, at: [f32; 3]) {
        let mut w = self.world.borrow_mut();
        let vehicle = w.objects.get(&PLAYER).and_then(|o| o.vehicle);
        for g in [Some(PLAYER), vehicle].into_iter().flatten() {
            if let Some(o) = w.objects.get_mut(&g) {
                o.position = at;
            }
        }
    }

    /// Puts the player in a vehicle (or out of one, with `None`): seat
    /// events fire on the next tick.
    pub fn ride(&mut self, vehicle: Option<u32>) {
        if let Some(o) = self.world.borrow_mut().objects.get_mut(&PLAYER)
            && o.vehicle != vehicle
        {
            o.vehicle = vehicle;
            o.seated_instantly = false;
        }
    }

    /// Character `guid` is put in vehicle `vehicle` (a prisoner loaded
    /// into the extraction helicopter): seat events fire on the next tick.
    pub fn seat_in(&mut self, guid: u32, vehicle: u32) {
        if let Some(o) = self.world.borrow_mut().objects.get_mut(&guid) {
            o.vehicle = Some(vehicle);
            o.seated_instantly = false;
        }
    }

    /// Character `guid` gets out of whatever they ride (a follower out of
    /// her vehicle).
    pub fn unseat(&mut self, guid: u32) {
        if let Some(o) = self.world.borrow_mut().objects.get_mut(&guid) {
            o.vehicle = None;
            o.seated_instantly = false;
        }
    }

    /// The objects carrying label `label` (`ExtractionHelicopter`), and
    /// where each is.
    pub fn labelled(&self, label: &str) -> Vec<(u32, [f32; 3])> {
        use crate::events::World;
        let w = self.world.borrow();
        w.labelled(label)
            .into_iter()
            .filter_map(|g| Some((g, w.objects.get(&g)?.position)))
            .collect()
    }

    /// The points of path entity `guid` in the world ([`Entity::path`]),
    /// of whichever layer names it; empty if none does.
    pub fn path(&self, guid: u32) -> &[[f32; 3]] {
        self.layers
            .values()
            .flatten()
            .find(|e| e.guid == guid && !e.path.is_empty())
            .map_or(&[], |e| e.path.as_slice())
    }

    /// She honked: the friendly faction gates hear it for
    /// [`api::HONK_HEARD`] seconds.
    pub fn honk(&mut self) {
        self.world.borrow_mut().honk_heard = api::HONK_HEARD;
    }

    /// Reports where an object is in play (a vehicle that was driven or
    /// pushed about).
    pub fn place(&mut self, guid: u32, at: [f32; 3], yaw: f32) {
        let mut w = self.world.borrow_mut();
        if let Some(o) = w.objects.get_mut(&guid) {
            o.position = at;
            o.yaw = yaw;
        } else if let Some(e) = w.entities.get_mut(&guid) {
            // A layer's entity no script has made an object of yet (a
            // truck's rum jugs riding on it): where the scripts' area
            // searches find it.
            *e = at;
        }
    }

    /// Leaves deliveries to the game: an `Ai.Deliver` order, and a
    /// support helicopter's landing (an `Ai.Goal` [`LAND`]), stay a
    /// [`Command::Goal`] for it to fly, and it reports the arrival with
    /// [`arrived`](Self::arrived). Without this they arrive as soon as
    /// they are ordered.
    pub fn fly_deliveries(&mut self) {
        self.flown = true;
    }

    /// Reports that `guid` (a delivery's pilot, a person sent somewhere,
    /// the player) reached `at`, its AI goal's place: any cargo is let go
    /// there and the scripts hear of it ([`api::arrived`]); what they do
    /// about it is carried out at once.
    pub fn arrived(&mut self, guid: u32, at: [f32; 3]) {
        self.log
            .push(format!("t={:.1} {guid:#x} arrived at {at:?}", self.now));
        for e in api::arrived(&self.world, guid, at) {
            self.log
                .push(format!("  goal callback: {}", first_lines(&e.to_string())));
        }
        self.settle(&format!("t={:.1}, delivered", self.now));
    }

    /// The support catalogue as the scripts hold it now, with her stock
    /// of each item ([`crate::support`]).
    pub fn support_items(&self) -> Vec<crate::support::SupportItem> {
        crate::support::items(self.host.lua()).unwrap_or_default()
    }

    /// Her cash and fuel as the scripts hold them (`MrxPmc.GetCashQty`,
    /// `GetFuelQty`; the cash is the engine's, `Player.GetCash`).
    pub fn resources(&self) -> (f64, f64) {
        let lua = self.host.lua();
        let get = |f: &str| {
            lua.globals()
                .get::<Table>("MrxPmc")
                .and_then(|m| m.get::<Function>(f))
                .and_then(|f| f.call::<Option<f64>>(()))
                .ok()
                .flatten()
        };
        let cash = get("GetCashQty").unwrap_or(self.world.borrow().cash);
        (cash, get("GetFuelQty").unwrap_or(0.0))
    }

    /// Gives her `cash` (taken, if less than 0) and `fuel` as the scripts
    /// do (`MrxPmc.AddCashQty`, which clamps it and tells the HUD and the
    /// stats, with `reason`, a string key such as `"[Generic.Pickups]"`;
    /// `AddFuelQty`, which stops at her tanks' capacity).
    pub fn give_resources(&mut self, cash: f64, fuel: f64, reason: &str) {
        let code = format!(
            "if {cash} ~= 0 then MrxPmc.AddCashQty({cash}, nil, {reason:?}) end \
             if {fuel} ~= 0 then MrxPmc.AddFuelQty({fuel}) end"
        );
        if let Err(e) = self.run(&code, "give resources") {
            self.log
                .push(format!("  give resources: {}", first_lines(&e.to_string())));
        }
    }

    /// The scripts' own save data now, as Lua text
    /// ([`crate::savedata::generate`]).
    pub fn save_data(&self) -> Result<String, MissionError> {
        Ok(crate::savedata::generate(self.host.lua(), BOOTSTRAP)?)
    }

    /// Where the story stands, as the scripts say
    /// ([`crate::savedata::CONTRACTS`]): the contracts done, the one
    /// active, and those on offer.
    pub fn contracts(&self) -> Result<crate::savedata::Contracts, MissionError> {
        Ok(crate::savedata::contracts(self.host.lua())?)
    }

    /// Opens faction `faction`'s shop (`MrxFactionManager`'s keys: `"All"`,
    /// `"Chi"`, `"Gur"`, `"Pir"`, `"Oil"`, `"Pmc"`) as its
    /// contact's menu does ([`crate::shop::OPEN`]); read it with
    /// [`shop`](Self::shop). Whether it came up.
    pub fn open_shop(&mut self, faction: &str) -> bool {
        let lua = self.host.lua();
        let opened = lua.globals().set("__m2_faction", faction).and_then(|()| {
            lua.load(crate::shop::OPEN)
                .set_name("=open shop")
                .eval::<bool>()
        });
        let opened = match opened {
            Ok(o) => o,
            Err(e) => {
                self.log
                    .push(format!("  open shop: {}", first_lines(&e.to_string())));
                false
            }
        };
        self.settle(&format!("t={:.1}, shop {faction}", self.now));
        opened
    }

    /// The shop the scripts have up, as they filled it.
    pub fn shop(&self) -> crate::shop::Shop {
        crate::shop::Shop::read(self.host.lua()).unwrap_or_default()
    }

    /// Buys `quantity` of shop item `id` as the shop's own selection does
    /// (the scripts' callback, `MrxShop._ShopSelection`): whether they
    /// sold it.
    pub fn buy(&mut self, id: &str, quantity: u32) -> bool {
        let lua = self.host.lua();
        let bought = lua
            .globals()
            .get::<Table>("__m2_shop")
            .and_then(|s| s.get::<Function>("fCallback"))
            .and_then(|f| f.call::<Value>((id, quantity)));
        let bought = match bought {
            Ok(v) => !matches!(v, Value::Nil | Value::Boolean(false)),
            Err(e) => {
                self.log
                    .push(format!("  buy {id}: {}", first_lines(&e.to_string())));
                false
            }
        };
        self.log.push(format!(
            "t={:.1} shop: {quantity} {id} bought: {bought}",
            self.now
        ));
        self.settle(&format!("t={:.1}, bought {id}", self.now));
        bought
    }

    /// Closes the shop as the player leaving it does: the scripts' close
    /// callback, then `MrxShop.Close`.
    pub fn close_shop(&mut self) {
        let code = "local s = __m2_shop \
                    if s and s.open and type(s.fClose) == 'function' then s.fClose() end \
                    if __m2_shop and __m2_shop.open then MrxShop.Close() end";
        if let Err(e) = self.run(code, "close shop") {
            self.log
                .push(format!("  close shop: {}", first_lines(&e.to_string())));
        }
    }

    /// Adds `count` of support item `key` to her stockpile, and cash and
    /// fuel, as a purchase or a find would (`MrxPmc.AddSupportQty`,
    /// `AddCashQty`, `AddFuelQty`).
    pub fn stock_support(&mut self, key: &str, count: i32, cash: f64, fuel: f64) {
        let code = format!(
            "MrxPmc.AddSupportQty({key:?}, {count}) MrxPmc.AddCashQty({cash}) MrxPmc.AddFuelQty({fuel})"
        );
        if let Err(e) = self.run(&code, "stock support") {
            self.log
                .push(format!("  stock support: {}", first_lines(&e.to_string())));
        }
    }

    /// Calls support item `key` in for the player, as the PDA and the
    /// support menu do ([`crate::support::CALL_IN`]): her designator for
    /// it comes as [`Command::EquipDesignator`]. Whether the scripts took
    /// the call.
    pub fn use_support(&mut self, key: &str) -> bool {
        let lua = self.host.lua();
        let owner = self.world.borrow().player.map(crate::events::guid_value);
        let set = lua.globals().set("__m2_key", key).and_then(|()| {
            lua.globals()
                .set("__m2_owner", owner.unwrap_or(mlua::Value::Nil))
        });
        let took = set.and_then(|()| {
            lua.load(crate::support::CALL_IN)
                .set_name("=call in support")
                .eval::<bool>()
        });
        let took = match took {
            Ok(t) => t,
            Err(e) => {
                self.log
                    .push(format!("  call in {key}: {}", first_lines(&e.to_string())));
                false
            }
        };
        self.log
            .push(format!("t={:.1} support {key} called in: {took}", self.now));
        self.settle(&format!("t={:.1}, support {key}", self.now));
        took
    }

    /// `owner` threw her designator, now at `at`: it becomes an object of
    /// `what` (`"Smoke Grenade Projectile"`) whose script, the support
    /// manager, takes up the call waiting for her
    /// (`MrxSupportManager.OnActivate`). Its guid; tell
    /// [`Mission::designator_settled`] when its timer runs out where it
    /// lies.
    pub fn designator_thrown(&mut self, owner: u32, what: &str, at: [f32; 3]) -> u32 {
        let guid = {
            let mut w = self.world.borrow_mut();
            let g = w.spawn_guid();
            w.insert(g, api::Object::new(what, at));
            w.strikes.designators.insert(g, owner);
            g
        };
        self.call_manager("OnActivate", guid, owner);
        guid
    }

    /// The designator `guid` has lain at `at` its template's time
    /// (`TriggerOnTimer`: 4 s for the smoke grenade): the support manager
    /// marks the spot there (`OnTimer`), and the support's own script
    /// takes over.
    pub fn designator_settled(&mut self, guid: u32, at: [f32; 3]) {
        let owner = {
            let mut w = self.world.borrow_mut();
            if let Some(o) = w.objects.get_mut(&guid) {
                o.position = at;
            }
            w.strikes.designators.get(&guid).copied()
        };
        if let Some(owner) = owner {
            self.call_manager("OnTimer", guid, owner);
        }
    }

    fn call_manager(&mut self, what: &str, guid: u32, owner: u32) {
        let lua = self.host.lua();
        let call = lua
            .globals()
            .get::<mlua::Table>("MrxSupportManager")
            .and_then(|m| m.get::<Function>(what))
            .and_then(|f| {
                f.call::<()>((
                    crate::events::guid_value(guid),
                    crate::events::guid_value(owner),
                ))
            });
        if let Err(e) = call {
            self.log.push(format!(
                "  support manager {what}: {}",
                first_lines(&e.to_string())
            ));
        }
        self.settle(&format!("t={:.1}, designator {what}", self.now));
    }

    /// Aircraft `guid` (an [`Command::Flyby`]) has reached the end of its
    /// run at `at`: its script lets its ordnance go.
    pub fn flyby_arrived(&mut self, guid: u32, at: [f32; 3]) {
        self.strike_callbacks(guid, at, true, "flyby arrived");
    }

    /// Ordnance `guid` (an [`Command::Ordnance`]) went off at `at`.
    pub fn ordnance_burst(&mut self, guid: u32, at: [f32; 3]) {
        self.strike_callbacks(guid, at, false, "ordnance burst");
    }

    fn strike_callbacks(&mut self, guid: u32, at: [f32; 3], arrival: bool, what: &str) {
        let callbacks = {
            let mut w = self.world.borrow_mut();
            if let Some(o) = w.objects.get_mut(&guid) {
                o.position = at;
            }
            w.strikes.take(guid, arrival)
        };
        for (f, data) in callbacks {
            let args: Vec<mlua::Value> = match data {
                Some(t) => t.sequence_values().filter_map(Result::ok).collect(),
                None => Vec::new(),
            };
            if let Err(e) = f.call::<()>(mlua::MultiValue::from_vec(args)) {
                self.log
                    .push(format!("  {what}: {}", first_lines(&e.to_string())));
            }
        }
        self.settle(&format!("t={:.1}, {what}", self.now));
    }

    /// Object `guid`'s health when whole (the hero template's `Health`:
    /// Mattias's 120), and its health with it when it was whole.
    pub fn set_max_health(&mut self, guid: u32, max: f32) {
        if let Some(o) = self.world.borrow_mut().objects.get_mut(&guid) {
            if o.health >= o.max_health {
                o.health = max;
            }
            o.max_health = max;
        }
    }

    /// Makes the player `hero` from now on (named as [`Mission::new`]
    /// takes it), whose template is `template` (`mattias`, `Jen`, `Chris`),
    /// whole at `max` health (as the game makes her, hurt or not): the
    /// scripts find her by the new name, tell her apart by the new
    /// hero's label, and see her template's labels as the data gives them
    /// (none if it has none for `template`). What they already chose by
    /// the old hero (lines cued, a contract's movies) stays as it was.
    /// Returns how many template labels she has.
    pub fn set_hero(&mut self, hero: &str, template: &str, max: f32) -> usize {
        let labels = {
            let mut w = self.world.borrow_mut();
            let labels = w
                .template_labels
                .get(&template.to_ascii_lowercase())
                .cloned()
                .unwrap_or_default();
            let n = labels.len();
            if let Some(p) = w.objects.get_mut(&PLAYER) {
                let old = std::mem::replace(&mut p.name, hero.to_owned());
                p.labels.retain(|l| !l.eq_ignore_ascii_case(&old));
                p.labels.push(hero.to_ascii_lowercase());
                p.label_hashes = labels;
                p.max_health = max;
                p.health = max;
            }
            n
        };
        self.log.push(format!(
            "  t={:.1} the player is {hero} (template {template}, {labels} labels), {max} health",
            self.now
        ));
        labels
    }

    /// Reports that an object died in play (a soldier the player shot).
    /// The game's reading of an object's health (the player's, as she is
    /// hurt and healed).
    pub fn set_health(&mut self, guid: u32, health: f32) {
        let was = self
            .world
            .borrow_mut()
            .objects
            .get_mut(&guid)
            .map(|o| std::mem::replace(&mut o.health, health));
        // A drop the game made (not the scripts' own healing): what the
        // scripts are watching for, while the health system is checked.
        if guid == PLAYER && was.is_some_and(|w| health < w - 5.0) && crate::script::stub_args() {
            self.log.push(format!(
                "  t={:.1} her health {:.1} -> {health:.1}; pending: {}",
                self.now,
                was.unwrap_or(0.0),
                self.events.describe().join(" ")
            ));
        }
    }

    /// The player died and is back on her feet: the hero module's
    /// `OnDeath` sets its survival system up again once she is healthy.
    pub fn player_died(&mut self) {
        self.log.push(format!("  t={:.1} player died", self.now));
        if let Err(e) = self.run("Hero.OnDeath(Player.GetCharacter(1))", "hero died") {
            self.log
                .push(format!("  hero died: {}", first_lines(&e.to_string())));
        }
    }

    pub fn kill(&mut self, guid: u32) {
        if let Some(o) = self.world.borrow_mut().objects.get_mut(&guid) {
            o.alive = false;
            o.health = 0.0;
        }
    }

    /// As [`kill`](Self::kill), by `killer` (her character, for what her
    /// shots, blasts and rams kill): the death event passes the killer
    /// to the scripts, which pay a hero-only bounty for it.
    pub fn kill_by(&mut self, guid: u32, killer: u32) {
        if let Some(o) = self.world.borrow_mut().objects.get_mut(&guid) {
            if o.alive {
                o.killer = Some(killer);
            }
            o.alive = false;
            o.health = 0.0;
        }
    }

    /// The player picks mission `id` (`"AllJob002"`) to track on the PDA's
    /// map: the callback the scripts gave the map
    /// (`MrxMissionFlow._TrackMission`, [`FOLLOW_TRACKING`]) is called
    /// with its data and the id, as the PDA widget calls it. False if the
    /// scripts gave the map no callback.
    pub fn track_mission(&mut self, id: &str) -> bool {
        let code = r#"
local id = ...
local t = rawget(_G, "__m2_track")
if type(t) ~= "table" or type(t[1]) ~= "function" then return false end
local args = {}
if type(t[2]) == "table" then
    for _, v in ipairs(t[2]) do args[#args + 1] = v end
end
args[#args + 1] = id
t[1](unpack(args))
return true
"#;
        let tracked = self
            .host
            .lua()
            .load(code)
            .set_name("=m2 track mission")
            .call::<bool>(id);
        match tracked {
            Ok(t) => t,
            Err(e) => {
                self.log
                    .push(format!("  tracking {id}: {}", first_lines(&e.to_string())));
                false
            }
        }
    }

    /// The player is in the music region naming faction `faction` (name
    /// hash; `None`: in none): the free-play score follows it
    /// ([`crate::music::Music::enter_region`]).
    pub fn music_region(&mut self, faction: Option<u32>) {
        let mut w = self.world.borrow_mut();
        if let Some(c) = w.music.enter_region(faction) {
            w.commands.push(c);
        }
    }

    /// The score changed under the HUD's scripts (the pause menu's
    /// music): see [`crate::music::Music::heard`].
    pub fn music_heard(&mut self, cue: Option<String>) {
        self.world.borrow_mut().music.heard(cue);
    }

    /// The faction the score plays now (`freeplay_city`, `pmc`, ...).
    pub fn music_faction(&self) -> String {
        self.world.borrow().music.faction().to_owned()
    }

    /// The game spent or picked up ammunition: the player's weapon of
    /// template `weapon` now has `rounds` in reserve, as scripts read it
    /// (`Weapon.GetReserveAmmo`).
    pub fn set_player_reserve(&mut self, weapon: &str, rounds: u32) {
        let mut w = self.world.borrow_mut();
        if let Some(who) = w.player {
            w.set_reserve(who, weapon, rounds);
        }
    }

    /// Has the flow's movies wait for the game ([`HOLD_MOVIES`]): call
    /// before [`start`](Self::start). Without it they end at once.
    pub fn hold_movies(&mut self) {
        let held = self
            .host
            .lua()
            .load(HOLD_MOVIES)
            .set_name("=hold movies")
            .eval::<u32>();
        match self
            .host
            .lua()
            .load(TELEPORTS)
            .set_name("=teleports")
            .eval::<bool>()
        {
            Ok(true) => {}
            Ok(false) => self
                .log
                .push("teleports: no MrxUtil.TeleportHeroesToLocations".into()),
            Err(e) => self
                .log
                .push(format!("teleports: {}", first_lines(&e.to_string()))),
        }
        self.log.push(match held {
            Ok(0) => "movies: no _PlayMovie to hold".to_owned(),
            Ok(n) => format!("movies: held for the game ({n} _PlayMovie)"),
            Err(e) => format!("movies: {}", first_lines(&e.to_string())),
        });
    }

    /// The next movie the flow asked for (`01_AOA_J`), if any: the game
    /// plays it and then calls [`movie_done`](Self::movie_done).
    pub fn take_movie(&mut self) -> Option<String> {
        if self.movie_out {
            return None;
        }
        let movies = self
            .host
            .lua()
            .globals()
            .raw_get::<mlua::Table>("__m2_movies")
            .ok()?;
        let first = movies.raw_get::<mlua::Table>(1).ok()?;
        let name = first.raw_get::<String>("sMovie").ok()?;
        self.log.push(format!("movie: {name}"));
        self.movie_out = true;
        Some(name)
    }

    /// Whether the player has subtitles on (`Sys.SubtitlesEnabled`; on
    /// unless told).
    pub fn set_subtitles(&mut self, on: bool) {
        self.world.borrow_mut().subtitles_off = !on;
    }

    /// Whether the player has the tutorial tips on
    /// (`Sys.TutorialsEnabled`; on unless told).
    pub fn set_tutorials(&mut self, on: bool) {
        self.world.borrow_mut().tutorials_off = !on;
    }

    /// Skips the story to a contract the developer's way (`Sys.
    /// GetSkipMission`): set before [`start`](Self::start), the new game
    /// turns on the scripts' skip mode with it, and the flow completes
    /// each story contract it unlocks until it unlocks this one, which
    /// then starts as in play (at its briefing with `briefing`).
    pub fn set_skip_mission(&mut self, mission: &str, briefing: bool) {
        let mut w = self.world.borrow_mut();
        w.skip_mission = Some(mission.to_owned());
        w.skip_briefing = briefing;
    }

    /// The subtitles of the movie [`take_movie`](Self::take_movie) gave,
    /// if the scripts asked for them (`bSubtitles`) and it has any: its
    /// `Subtitles_<movie>` script's `SubtitleData`, loaded as
    /// `MrxGuiCinematic.ShowMovie` loads it.
    pub fn movie_subtitles(&mut self) -> Option<Vec<crate::subtitles::Line>> {
        let first = self
            .host
            .lua()
            .globals()
            .raw_get::<mlua::Table>("__m2_movies")
            .ok()?
            .raw_get::<mlua::Table>(1)
            .ok()?;
        if !first.raw_get::<bool>("bSubtitles").unwrap_or(false) {
            return None;
        }
        let movie = first.raw_get::<String>("sMovie").ok()?;
        let script = crate::subtitles::script_name(&movie);
        let code = r#"
local name = ...
local m = __m2_module(name)
local data = type(m) == "table" and rawget(m, "SubtitleData")
if type(data) ~= "table" then return nil end
local out = {}
for i, e in ipairs(data) do
    out[i] = { tonumber(e[1]) or 0, tostring(e[2] or ""), tonumber(e[3]), e[4] and true or false }
end
return out
"#;
        let lines = self
            .host
            .lua()
            .load(code)
            .set_name("=m2 subtitles")
            .call::<Option<mlua::Table>>(script.as_str());
        let lines = match lines {
            Ok(Some(t)) => t,
            Ok(None) => {
                self.log.push(format!("subtitles: {script} has none"));
                return None;
            }
            Err(e) => {
                self.log.push(format!(
                    "subtitles: {script}: {}",
                    first_lines(&e.to_string())
                ));
                return None;
            }
        };
        let lines: Vec<_> = lines
            .sequence_values::<mlua::Table>()
            .flatten()
            .map(|e| crate::subtitles::Line {
                time: e.raw_get(1).unwrap_or(0.0),
                text: e.raw_get(2).unwrap_or_default(),
                seconds: e.raw_get(3).ok().flatten(),
                top: e.raw_get(4).unwrap_or(false),
            })
            .collect();
        self.log
            .push(format!("subtitles: {script}, {} lines", lines.len()));
        Some(lines)
    }

    /// The movie [`take_movie`](Self::take_movie) gave has ended (or was
    /// skipped): its callback runs.
    pub fn movie_done(&mut self) {
        self.movie_out = false;
        let code = r#"
local t = table.remove(__m2_movies, 1)
if t and t.fCallback then
    MrxUtil.CallWithOptionalArgs(t.fCallback, t.tCallbackData)
end
"#;
        if let Err(e) = self.run(code, "movie done") {
            self.log
                .push(format!("  movie done: {}", first_lines(&e.to_string())));
        }
    }

    /// The player pressed the GUI's button `name` (`"selection"`), for
    /// the scripts' `Button` events.
    pub fn press_button(&mut self, name: &str) {
        let mut w = self.world.borrow_mut();
        w.buttons.push((self.now, name.to_ascii_lowercase()));
        let extra = w.buttons.len().saturating_sub(16);
        w.buttons.drain(..extra);
    }

    /// The context actions on offer (`Pg.AddContextAction`): each one's
    /// object, where it stands (if known), its prompt's text id and reach.
    pub fn context_actions(&self) -> Vec<(u32, Option<[f32; 3]>, String, f32)> {
        let w = self.world.borrow();
        w.context_actions
            .iter()
            .map(|(&g, a)| {
                // A placed object's place, else its entity's, loaded or not
                // (PmcCon003's `PlayerLocation_PMC`, a `vz_base` locator
                // the action objective prompts at).
                let at = w
                    .objects
                    .get(&g)
                    .map(|o| o.position)
                    .or_else(|| w.entities.get(&g).copied());
                (g, at, a.text.clone(), a.reach)
            })
            .collect()
    }

    /// The template the layers' object `guid` was placed as, by name.
    pub fn template_of(&self, guid: u32) -> Option<String> {
        self.world.borrow().placed_as.get(&guid).cloned()
    }

    /// Who a layer's vehicle `guid` came into play with: (seat, rider
    /// template), as its entity's own `Rider` records say, else its
    /// template's; empty for none.
    pub fn crew_of(&self, guid: u32) -> Vec<(String, String)> {
        self.world
            .borrow()
            .crews
            .get(&guid)
            .cloned()
            .unwrap_or_default()
    }

    /// The layers' objects with a crew of their own still aboard, and who:
    /// `(object, [(seat, rider, rider's template)])`, by object. Each
    /// rider is matched to the first free seat of its `Rider` records
    /// naming its template; those the scripts have taken out
    /// (`Vehicle.Exit`) or killed are left out. The HQ interior is such
    /// an object (`HqInterior`, a building): its starter, Fiona, rides
    /// it, and `WifPmcInterior` finds her as `Vehicle.GetRiders(it)[1]`.
    pub fn crews_aboard(&self) -> Vec<CrewAboard> {
        let w = self.world.borrow();
        // Who rides what, in one pass over the objects: once the story
        // has moved on (`--skip-to`) the layers hold thousands of crews,
        // and a pass over every object for each crew stalled each tick
        // for minutes.
        let mut by_vehicle: HashMap<u32, Vec<(u32, &String)>> = HashMap::new();
        for (&r, o) in &w.objects {
            if let Some(v) = o.vehicle
                && o.alive
                && Some(r) != w.player
                && w.crews.contains_key(&v)
            {
                by_vehicle.entry(v).or_default().push((r, &o.name));
            }
        }
        let mut out: Vec<CrewAboard> = w
            .crews
            .iter()
            .filter_map(|(&g, crew)| {
                let mut riders = by_vehicle.remove(&g).unwrap_or_default();
                riders.sort_unstable();
                let mut free: Vec<&(String, String)> = crew.iter().collect();
                let aboard: Vec<(String, u32, String)> = riders
                    .into_iter()
                    .filter_map(|(r, name)| {
                        let i = free
                            .iter()
                            .position(|(_, t)| t.eq_ignore_ascii_case(name))?;
                        let (seat, template) = free.remove(i);
                        Some((seat.clone(), r, template.clone()))
                    })
                    .collect();
                (!aboard.is_empty()).then_some((g, aboard))
            })
            .collect();
        out.sort_unstable_by_key(|(g, _)| *g);
        out
    }

    /// Riders of the layers' crews the scripts have out of their seats and
    /// alive: `(rider, what it came in on, template, place, yaw)`. The HQ's
    /// briefing takes its starter out of her seat (`Vehicle.Exit`) and puts
    /// her at its briefing spot (`Object.SetTransformToObject`).
    pub fn crews_on_foot(&self) -> Vec<OnFoot> {
        let w = self.world.borrow();
        let mut out: Vec<OnFoot> = w
            .crew_riders
            .iter()
            .filter_map(|(&r, &from)| {
                let o = w.objects.get(&r)?;
                (o.alive && o.vehicle.is_none())
                    .then(|| (r, from, o.name.clone(), o.position, o.yaw))
            })
            .collect();
        out.sort_unstable_by_key(|e| e.0);
        out
    }

    /// The player took the context action offered beside `guid`, for the
    /// scripts' `ContextAction` events.
    pub fn take_context_action(&mut self, guid: u32) {
        let mut w = self.world.borrow_mut();
        w.context_presses.push((self.now, guid));
        let extra = w.context_presses.len().saturating_sub(16);
        w.context_presses.drain(..extra);
    }

    /// A clip the scripts played on `guid` by name met `event`, for the
    /// scripts' `AnimationEvent` events (the latest 32 are kept).
    pub fn animation_event(&mut self, guid: u32, event: &str) {
        let mut w = self.world.borrow_mut();
        w.animation_events.push((self.now, guid, event.to_owned()));
        let extra = w.animation_events.len().saturating_sub(32);
        w.animation_events.drain(..extra);
    }

    /// Whether the scripts let the player grapple
    /// (`Player.SetGrappleEnabled`).
    pub fn grapple_enabled(&self) -> bool {
        self.world.borrow().grapple_enabled
    }

    /// The player is (no longer) on her grappling hook's line, for the
    /// scripts' `Human.IsGrappling`.
    pub fn set_grappling(&mut self, on: bool) {
        self.world.borrow_mut().grappling = on;
    }

    /// A clip the scripts played on `speaker` by name met a voice line
    /// (its `vo` events: the faction HQs' briefings are spoken by their
    /// clips, 806 lines over 258 clips, from samples): said as a scripted
    /// line with no callback, its subtitle up if the player has them on.
    pub fn clip_line(&mut self, speaker: u32, cue: &str) {
        let mut w = self.world.borrow_mut();
        let subtitled = !w.subtitles_off;
        let say = w.vo.cue(Some(speaker), cue.to_owned(), None, subtitled);
        w.commands.push(say);
    }

    /// Everything the scripts asked the game to do since the last call.
    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.outbox)
    }

    /// A readable account of the session since the last call.
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }

    /// A HUD text with its ids replaced by the game's English strings
    /// (`[VzaCon001.Objectives.001]` as `Secure the beach`).
    pub fn text(&self, text: &str) -> String {
        render(text, &self.strings)
    }

    /// The game's English strings.
    pub fn strings(&self) -> &StringTable {
        &self.strings
    }

    /// Modules loaded since play started: name and script.
    pub fn late_modules(&self) -> Vec<(String, Option<u32>)> {
        self.host.modules().into_iter().skip(self.started).collect()
    }

    /// Pending events by kind.
    pub fn pending_events(&self) -> Vec<String> {
        self.events.kinds()
    }

    /// The names the scripts' pending `ScriptEvent` events wait for.
    pub fn awaited_posts(&self) -> Vec<String> {
        self.events.awaited()
    }

    /// Where the player's camera looks from and how wide it sees, as the
    /// game draws it this tick: what `Event.ObjectIsVisible` checks
    /// against.
    pub fn set_camera(&mut self, camera: api::Camera) {
        self.world.borrow_mut().camera = Some(camera);
    }

    /// The objects pending `ObjectIsVisible` events wait to see, for the
    /// game to look whether anything solid stands between each and the
    /// camera ([`set_unseen`](Self::set_unseen)).
    pub fn sight_watched(&self) -> Vec<u32> {
        self.events.sight_watched(&*self.world.borrow())
    }

    /// The regions pending `Boundary` events wait for something to leave
    /// ([`crate::events::Events::exits_watched`]).
    pub fn exits_watched(&self) -> Vec<u32> {
        self.events.exits_watched(&*self.world.borrow())
    }

    /// The objects the game found behind something solid from the camera
    /// (of those [`sight_watched`](Self::sight_watched) named): not on
    /// screen, though in view.
    pub fn set_unseen(&mut self, unseen: std::collections::HashSet<u32>) {
        self.world.borrow_mut().unseen = unseen;
    }

    /// Posts a script event as the engine does (`Event.Post(name)`): the
    /// scripts waiting for it hear of it at the next tick.
    pub fn post(&mut self, name: &str) {
        self.events.post(name);
    }

    /// Posts a script event as the resident script's network-event
    /// callback does for the PDA: `"PDA Open"` with `{ uPlayer = 1 }` (the
    /// watch filters on whose PDA it was), `"PDA Close"` with an empty
    /// table (`for_player` false).
    pub fn post_net_event(&mut self, name: &str, for_player: bool) {
        let data = self.host.lua().create_table().and_then(|t| {
            if for_player {
                t.set("uPlayer", 1)?;
            }
            Ok(t)
        });
        match data {
            Ok(t) => self.events.post_data(name, mlua::Value::Table(t)),
            Err(_) => self.events.post(name),
        }
    }

    /// Logs what happened, then carries out layer requests and deliveries
    /// (at once) and logs what their callbacks did, until nothing is left.
    fn settle(&mut self, when: &str) {
        let mut commands = self.report(when);
        for _ in 0..50 {
            // Modules a script loaded "in the background" are ready.
            let ran = match self.host.run_deferred() {
                Ok((ran, errors)) => {
                    for e in errors {
                        self.log
                            .push(format!("  dynamic_import callback: {}", first_lines(&e)));
                    }
                    ran
                }
                Err(e) => {
                    self.log.push(format!("  dynamic_import callbacks: {e}"));
                    0
                }
            };
            if ran > 0 {
                commands.extend(self.report(&format!("{when}, {ran} modules loaded")));
            }
            let mut delivered = 0;
            for c in &commands {
                if let Command::Goal {
                    guid,
                    goal,
                    location: Some(at),
                    ..
                } = c
                    && (goal == "Deliver" || goal == LAND)
                    && !self.flown
                {
                    self.log
                        .push(format!("  delivery by {guid} arrived at {at:?}"));
                    for e in api::arrived(&self.world, *guid, *at) {
                        self.log
                            .push(format!("  goal callback: {}", first_lines(&e.to_string())));
                    }
                    delivered += 1;
                }
            }
            let requests: Vec<(LayerOp, String)> = commands
                .into_iter()
                .filter_map(|c| match c {
                    Command::Layer { op, name } => Some((op, name)),
                    _ => None,
                })
                .collect();
            if requests.is_empty() {
                if delivered > 0 {
                    commands = self.report(&format!("{when}, deliveries done"));
                    continue;
                }
                if ran == 0 {
                    return;
                }
                commands = Vec::new();
                continue;
            }
            for (op, name) in requests {
                let found = self.layers.get(&name);
                if let Some(entities) = found {
                    let lua = self.host.lua();
                    let mut failed = Vec::new();
                    if op != LayerOp::Load {
                        failed.extend(activate(lua, entities, false));
                        place(&self.world, entities, false);
                    }
                    if op != LayerOp::Unload {
                        place(&self.world, entities, true);
                        failed.extend(activate(lua, entities, true));
                    }
                    self.log
                        .extend(failed.into_iter().map(|f| format!("  {f}")));
                }
                let what = found.map_or("part of layers_static".to_owned(), |e| {
                    format!("{} entities", e.len())
                });
                self.log.push(format!("  layer {op:?} {name}: {what}"));
                for e in finished(&self.world, &name, true) {
                    self.log
                        .push(format!("  layer callback: {}", first_lines(&e.to_string())));
                }
            }
            commands = self.report(&format!("{when}, layers done"));
        }
        self.log
            .push("  requests still coming after 50 rounds; carrying on".into());
    }

    /// Logs what happened since the last report, if anything; returns the
    /// commands (also kept for the game).
    fn report(&mut self, when: &str) -> Vec<Command> {
        let calls = self.host.take_calls();
        let (commands, printed) = {
            let mut w = self.world.borrow_mut();
            (w.take_commands(), std::mem::take(&mut w.printed))
        };
        self.outbox.extend(commands.iter().cloned());
        if calls.is_empty() && commands.is_empty() && printed.is_empty() {
            return commands;
        }
        let log = &mut self.log;
        log.push(String::new());
        log.push(format!("{when} ({} events pending):", self.events.len()));
        log.extend(printed.iter().take(60).map(|p| format!("  print {p}")));
        log.extend(commands.iter().take(60).map(|c| format!("  command {c:?}")));
        // Repeated calls in a row collapse to one line with a count.
        let mut runs: Vec<(&str, usize)> = Vec::new();
        for c in &calls {
            match runs.last_mut() {
                Some((l, n)) if *l == c.as_str() => *n += 1,
                _ => runs.push((c.as_str(), 1)),
            }
        }
        // With arguments shown every call differs: keep them all.
        let cap = if crate::script::stub_args() {
            usize::MAX
        } else {
            120
        };
        for (c, n) in runs.iter().take(cap) {
            log.push(match n {
                1 => format!("  stub {c}"),
                _ => format!("  stub {c} x{n}"),
            });
        }
        if runs.len() > cap {
            log.push(format!("  ... {} stub calls in all", calls.len()));
        }
        commands
    }
}

/// The player's guid.
pub const PLAYER: u32 = 1;

/// Puts a layer's entities in the world as objects scripts can find by
/// name, or takes them out.
fn place(world: &Shared, entities: &[Entity], load: bool) {
    let mut w = world.borrow_mut();
    for e in entities {
        if load {
            let mut o = Object::new(&e.name, e.at);
            o.yaw = e.yaw;
            o.label_hashes.clone_from(&e.labels);
            o.faction = e.faction;
            w.insert(e.guid, o);
            if e.script == Some(FRIENDLY_GATE) {
                w.gates.insert(e.guid);
            }
            if let Some(r) = &e.region {
                w.regions.insert(e.guid, r.clone());
            }
            for r in seat_crew(&mut w, e.guid, e.at, &e.crew) {
                w.crew_riders.insert(r, e.guid);
            }
            if !e.crew.is_empty() {
                w.crews.insert(e.guid, e.crew.clone());
            }
            if let Some(t) = &e.template {
                w.placed_as.insert(e.guid, t.clone());
            }
        } else {
            w.crews.remove(&e.guid);
            w.placed_as.remove(&e.guid);
            w.gates.remove(&e.guid);
            w.crew_riders.retain(|_, v| *v != e.guid);
            w.objects.remove(&e.guid);
            w.regions.remove(&e.guid);
            // Its crew goes with it.
            w.objects
                .retain(|&g, o| g < api::FIRST_SPAWNED || o.vehicle != Some(e.guid));
        }
    }
}

/// Seats a vehicle's crew ([`Entity::crew`]) as it comes into play, as
/// the engine does: an object per rider, named by its template and
/// labelled as it, in `vehicle` from the start, the driver's seat
/// (`seat_driver`) first so `Vehicle.GetDriver` answers it (PmcCon001's
/// `GarageSmash` orders the villa tank's driver along its path the moment
/// the tank's layer is in). Returns the riders' guids.
fn seat_crew(
    w: &mut ScriptWorld,
    vehicle: u32,
    at: [f32; 3],
    crew: &[(String, String)],
) -> Vec<u32> {
    let mut seats: Vec<&(String, String)> = crew.iter().collect();
    seats.sort_by_key(|(seat, _)| !seat.eq_ignore_ascii_case("seat_driver"));
    let mut out = Vec::new();
    for (_, template) in seats {
        let guid = w.spawn_guid();
        let mut o = Object::new(template, at);
        o.vehicle = Some(vehicle);
        o.seated_instantly = true;
        if let Some(l) = w.template_labels.get(&template.to_ascii_lowercase()) {
            o.label_hashes.clone_from(l);
        }
        w.insert(guid, o);
        out.push(guid);
    }
    out
}

/// The object scripts the engine runs on layer entities that the port runs
/// too, by module name and hash: as an entity's layer comes in the engine
/// calls the module's `OnActivate(guid, param)`, and as it goes,
/// `OnDeactivate(guid)` (`Inheritable`'s, which deletes the instance).
/// `FactionZone` makes each zone an instance that enables itself: the zone
/// on the radar and PDA map, and a watch on the player crossing it that
/// tells the HUD (`TrespassStateChange`).
/// `friendlygate` makes each faction base gate open and shut by itself:
/// within 20 m of it, the hero (taken for the faction of the vehicle
/// she is disguised in, else the PMC's) or one of its faction's
/// vehicles; open while one is there whom the gate's faction is at
/// least `Friendly` to (the PMC only while the faction's attitude to it
/// can change, at least `Neutral`), unless a script locked it (its
/// `_TestAttitude`, `_EvaluateCandidates`, `LockGate`), and shut as the
/// last goes 40 m off. Opening, a soldier of its faction within 25 m
/// says his `GateYes01` line. The engine tells it of her only after she
/// honks ([`crate::api::ScriptWorld::honk_heard`]).
pub const OBJECT_SCRIPTS: [(&str, u32); 2] = [
    ("FactionZone", 0x6726_7cc1),
    ("friendlygate", FRIENDLY_GATE),
];

/// `friendlygate`'s module hash.
pub const FRIENDLY_GATE: u32 = 0x6d4f_9301;

/// Runs the object scripts of `entities` (see [`OBJECT_SCRIPTS`]) as their
/// layer comes in (`load`) or goes. Returns what failed.
fn activate(lua: &mlua::Lua, entities: &[Entity], load: bool) -> Vec<String> {
    let mut failed = Vec::new();
    for e in entities {
        let Some((module, _)) = e
            .script
            .and_then(|h| OBJECT_SCRIPTS.iter().find(|(_, s)| *s == h))
        else {
            continue;
        };
        let call = if load { "OnActivate" } else { "OnDeactivate" };
        let result = lua
            .globals()
            .get::<mlua::Table>(*module)
            .and_then(|m| m.get::<mlua::Function>(call))
            .and_then(|f| f.call::<()>(crate::events::guid_value(e.guid)));
        if let Err(err) = result {
            failed.push(format!(
                "{module}.{call} {}: {}",
                e.name,
                first_lines(&err.to_string())
            ));
        }
    }
    failed
}

/// The first lines of a (Lua) error, on one line.
pub fn first_lines(s: &str) -> String {
    s.lines().take(4).collect::<Vec<_>>().join(" | ")
}

/// Whether a boundary region is one she must keep out of: from samples,
/// the two regions with a `BoundaryData` record are the HQs' exclusions
/// (`Boundary_ALHQ_Exclusion`, `Boundary_CHHQ_Exclusion`), the only ones
/// so named; the record itself isn't decoded.
fn is_exclusion(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with("_exclusion")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_object_scripts_are_named_by_their_hash() {
        for (name, hash) in OBJECT_SCRIPTS {
            assert_eq!(name_hash(name), hash, "{name}");
        }
    }

    #[test]
    fn landing_zones_come_from_names() {
        assert_eq!(landing_zone("01_pmc_hq_lz_playerone"), Some((1, 1)));
        assert_eq!(landing_zone("30_chi_hqb_lz_PlayerTwo"), Some((30, 2)));
        assert_eq!(landing_zone("03_mar_airport_llz_playerone"), Some((3, 1)));
        assert_eq!(landing_zone("lz_helipad_playerone"), None);
        assert_eq!(landing_zone("06_gua_upperclass_playerone"), None);
    }

    #[test]
    fn the_players_subtitles_and_tutorials_reach_the_scripts() {
        let mut m = Mission::new(GameData::default(), "Mattias", [0.0; 3]).unwrap();
        let ask = |m: &Mission| {
            m.lua()
                .load("return { Sys.SubtitlesEnabled(), Sys.TutorialsEnabled() }")
                .eval::<Vec<bool>>()
                .unwrap()
        };
        assert_eq!(ask(&m), [true, true]);
        m.set_tutorials(false);
        assert_eq!(ask(&m), [true, false]);
        m.set_subtitles(false);
        m.set_tutorials(true);
        assert_eq!(ask(&m), [false, true]);
    }

    #[test]
    fn a_layer_entity_is_placed_where_the_game_says() {
        let mut m = Mission::new(GameData::default(), "Mattias", [0.0; 3]).unwrap();
        m.world.borrow_mut().entities.insert(5, [0.0; 3]);
        m.place(5, [1.0, 2.0, 3.0], 0.0);
        assert_eq!(m.world.borrow().entities[&5], [1.0, 2.0, 3.0]);
    }

    #[test]
    fn the_skip_to_mission_switch_reaches_the_bootstrap_once() {
        let mut m = Mission::new(GameData::default(), "Mattias", [0.0; 3]).unwrap();
        let ask = |m: &Mission| {
            m.lua()
                .load("return Sys.GetSkipMission(), Sys.GetINIBriefing()")
                .eval::<(Option<String>, bool)>()
                .unwrap()
        };
        assert_eq!(ask(&m), (None, false));
        m.set_skip_mission("GurCon053", true);
        assert_eq!(ask(&m), (Some("GurCon053".into()), true));
        // The bootstrap clears it once taken, as the engine's INI switch.
        m.lua().load("Sys.SetSkipMission('')").exec().unwrap();
        assert_eq!(ask(&m).0, None);
    }

    #[test]
    fn skip_mode_ends_after_the_start() {
        let mut m = Mission::new(GameData::default(), "Mattias", [0.0; 3]).unwrap();
        m.run(
            "MrxCheatBootstrap = { IsSkipModeEnabled = function() return false end }",
            "cheats",
        )
        .unwrap();
        m.start();
        let on = |m: &Mission| {
            m.lua()
                .load("return MrxCheatBootstrap.IsSkipModeEnabled()")
                .eval::<bool>()
                .unwrap()
        };
        assert!(on(&m), "the start turns it on");
        m.end_skip_mode();
        assert!(!on(&m));
        assert!(
            m.take_log()
                .iter()
                .any(|l| l == "skip mode: off after the start (the scripts' own: false)")
        );
        // Scripts whose own says yes are told no all the same.
        m.run(
            "__m2_own_skip_mode = function() return true end",
            "own says yes",
        )
        .unwrap();
        m.end_skip_mode();
        assert!(!on(&m));
    }

    #[test]
    fn the_scripts_free_support_items_reach_the_game() {
        let mut m = Mission::new(GameData::default(), "Mattias", [0.0; 3]).unwrap();
        m.follow_freebies();
        assert!(m.take_log().iter().any(|l| l.contains("no MrxSupportData")));
        m.run(
            r#"
            MrxSupportData = { tGiven = {} }
            function MrxSupportData.AddFreebie(sKey, nQty, uPlayer, bFlag, nLimit)
                local t = MrxSupportData.tGiven
                t[sKey] = math.min((t[sKey] or 0) + nQty, nLimit or 99)
            end
            function MrxSupportData.RemoveFreebie(sKey)
                MrxSupportData.tGiven[sKey] = nil
            end
            "#,
            "support data",
        )
        .unwrap();
        m.follow_freebies();
        m.run(
            r#"
            MrxSupportData.AddFreebie("VzaCon01_SatBomb", 1, nil, nil, 1)
            MrxSupportData.AddFreebie("VzaCon01_SatBomb", 1, nil, nil, 1)
            assert(MrxSupportData.tGiven.VzaCon01_SatBomb == 1)
            MrxSupportData.RemoveFreebie("VzaCon01_SatBomb")
            assert(MrxSupportData.tGiven.VzaCon01_SatBomb == nil)
            MrxSupportData.AddFreebie("Flare", 2)
            "#,
            "give",
        )
        .unwrap();
        let bomb = || Command::Freebie {
            key: "VzaCon01_SatBomb".into(),
            quantity: 1,
            limit: Some(1),
        };
        assert_eq!(
            m.take_commands(),
            [
                bomb(),
                bomb(),
                Command::RemoveFreebie("VzaCon01_SatBomb".into()),
                Command::Freebie {
                    key: "Flare".into(),
                    quantity: 2,
                    limit: None
                }
            ]
        );
    }

    #[test]
    fn a_conversation_moves_on_as_each_line_ends() {
        let mut m = Mission::new(GameData::default(), "Jennifer", [0.0; 3]).unwrap();
        m.run(
            r#"
            local function second(state) VO.Cue(0, "B", function() heard = state end) end
            VO.Cue(0, "A", second, {}, 1, true)
            VO.Cue(0, "C", function() cancelled = true end)
            VO.Cancel(0, "C", true)
            ordered = VO.PRIORITY_CINEMATIC < VO.PRIORITY_SCRIPTED_CONTRACT
            "#,
            "test",
        )
        .unwrap();
        let say = |id, cue: &str| Command::Say {
            id,
            speaker: None,
            cue: cue.into(),
        };
        assert_eq!(
            m.take_commands(),
            [say(1, "A"), say(2, "C"), Command::Hush(2)]
        );
        // The call as the scripts made it, for the log.
        assert!(
            m.take_log()
                .iter()
                .any(|l| l == r#"  print VO.Cancel(0, "C"): speaker None, stops lines [2]"#)
        );
        m.tick(0.1);
        m.line_length(1, 1.0);
        m.tick(0.5);
        assert!(m.take_commands().is_empty());
        m.tick(0.6);
        assert_eq!(m.take_commands(), [say(3, "B")]);
        for _ in 0..50 {
            m.tick(0.1);
        }
        let g = m.host.lua().globals();
        assert_eq!(g.raw_get::<String>("heard").unwrap(), "complete");
        assert_eq!(g.raw_get::<Option<bool>>("cancelled").unwrap(), None);
        assert!(g.raw_get::<bool>("ordered").unwrap());
    }

    #[test]
    fn invincible_while_any_reason_holds() {
        let mut m = Mission::new(GameData::default(), "Jennifer", [0.0; 3]).unwrap();
        m.run(
            r#"
            local hero = Player.GetCharacter(1)
            Object.SetInvincible(hero, true, "MrxState")
            Object.SetInvincible(hero, true, "Fanfare")
            Object.SetInvincible(hero, false, "MrxState")
            Object.SetInvincible(hero, false, "Fanfare")
            "#,
            "test",
        )
        .unwrap();
        let on = |on| Command::Invincible { guid: PLAYER, on };
        assert_eq!(m.take_commands(), [on(true), on(false)]);
    }

    #[test]
    fn health_watches_fire_as_she_is_hurt() {
        let mut m = Mission::new(GameData::default(), "Jennifer", [0.0; 3]).unwrap();
        m.run(
            r#"
            dropped = 0
            local hero = Player.GetCharacter(1)
            Player.SetHealthClamp(1, true)
            Event.Create(Event.ObjectHealth, { hero, "<", Object.GetHealth(hero) },
                function() dropped = dropped + 1 end)
            Sound.StopSound(0, "sfx_survival_lp")
            "#,
            "test",
        )
        .unwrap();
        assert_eq!(
            m.take_commands(),
            [
                Command::HealthClamp(true),
                Command::StopSound {
                    at: None,
                    cue: "sfx_survival_lp".into()
                }
            ]
        );
        m.tick(0.1);
        m.set_health(PLAYER, 60.0);
        m.tick(0.1);
        m.tick(0.1);
        let dropped: i64 = m.lua().globals().raw_get("dropped").unwrap();
        assert_eq!(dropped, 1);
    }

    #[test]
    fn the_player_becomes_another_hero() {
        let data = GameData {
            template_labels: HashMap::from([("jen".to_owned(), vec![name_hash("HeroLabel")])]),
            ..GameData::default()
        };
        let mut m = Mission::new(data, "Mattias", [0.0; 3]).unwrap();
        m.set_max_health(PLAYER, 120.0);
        // Hurt: the new hero is whole all the same.
        m.set_health(PLAYER, 60.0);
        m.run(
            r#"
            local me = Player.GetCharacter(1)
            assert(Object.GetName(me) == "Mattias")
            assert(Object.HasLabel(me, "mattias"))
            Object.AddLabel(me, "Scripted")
            "#,
            "before",
        )
        .unwrap();
        assert_eq!(m.set_hero("Jennifer", "Jen", 100.0), 1);
        m.run(
            r#"
            local me = Player.GetCharacter(1)
            assert(Object.GetName(me) == "Jennifer")
            assert(Pg.GetGuidByName("Jennifer") == me)
            assert(Object.HasLabel(me, "jennifer"))
            assert(not Object.HasLabel(me, "mattias"))
            assert(Object.HasLabel(me, "HeroLabel"))
            assert(Object.HasLabel(me, "Scripted"))
            assert(Object.GetMaxHealth(me) == 100)
            assert(Object.GetHealth(me) == 100)
            "#,
            "after",
        )
        .unwrap();
        // A template the data has no labels for leaves her none of it.
        assert_eq!(m.set_hero("Chris", "Chris", 100.0), 0);
        let w = m.world().borrow();
        let p = &w.objects[&PLAYER];
        assert_eq!(p.name, "Chris");
        assert!(p.label_hashes.is_empty());
        assert_eq!(p.labels, ["Scripted", "chris"]);
    }

    #[test]
    fn play_reports_rides_and_places() {
        let mut m = Mission::new(GameData::default(), "Mattias", [0.0; 3]).unwrap();
        m.ride(Some(77));
        m.place(PLAYER, [1.0, 2.0, 3.0], 0.5);
        let w = m.world().borrow();
        let p = &w.objects[&PLAYER];
        assert_eq!(
            (p.vehicle, p.position, p.yaw),
            (Some(77), [1.0, 2.0, 3.0], 0.5)
        );
    }

    #[test]
    fn the_scripts_fade_calls_back_once_black_and_clears_after_a_fade_back() {
        let mut m = Mission::new(GameData::default(), "Mattias", [0.0; 3]).unwrap();
        // MrxGui's fade as its script runs it, cut down: a queued callback
        // runs on the GUI update after the wipe; a fade back asks the
        // movie to fade out.
        m.run(
            r#"
            Fade = { _nCount = 0, _tCallbacks = {} }
            local F = Fade
            F._oFadeFlash = { CustomData = { bLoaded = false } }
            function F.GlobalFadeToBlack(f)
                table.insert(F._tCallbacks, f)
                F._nCount = F._nCount + 1
                if not F._oFadeFlash.CustomData.bLoaded then
                    F._oFadeFlash.CustomData.bLoading = true
                else
                    F._bCurrentlyFadingIn = true
                end
            end
            function F._CompleteFadeFlashLoad()
                F._oFadeFlash.CustomData.bLoaded = true
                F._oFadeFlash.CustomData.bLoading = false
                F._bCurrentlyFadingIn = true
            end
            function F._FinishFadeToBlack(w)
                F._bCurrentlyFadingIn = false
                F._bCurrentlyFadedIn = true
            end
            function F._FadeUpdate(w)
                for _, f in ipairs(F._tCallbacks) do f() end
                F._tCallbacks = {}
            end
            function F.GlobalFadeFromBlack()
                F._nCount = F._nCount - 1
                F._bCurrentlyFadedIn = false
                F._bCurrentlyFadingIn = false
            end
            function F._FinishFadeFromBlack(w) F.closed = (F.closed or 0) + 1 end
            "#,
            "fade",
        )
        .unwrap();
        let lua = m.host.lua();
        lua.load("Fade.GlobalFadeToBlack(function() called = true end)")
            .exec()
            .unwrap();
        let mut blacks = Vec::new();
        for _ in 0..30 {
            m.tick(0.1);
            blacks.push(m.black());
        }
        let lua = m.host.lua();
        assert!(lua.globals().get::<bool>("called").unwrap());
        assert_eq!(m.black(), 1.0);
        // The wipe takes the movie's two seconds.
        assert!(blacks[17] < 1.0 && blacks[21] >= 1.0, "{blacks:?}");
        // Back, and at once to black again: the fade back gives way.
        lua.load("Fade.GlobalFadeFromBlack() Fade.GlobalFadeToBlack(function() end)")
            .exec()
            .unwrap();
        for _ in 0..30 {
            m.tick(0.1);
        }
        assert_eq!(m.black(), 1.0);
        let lua = m.host.lua();
        assert_eq!(
            lua.load("return Fade.closed")
                .eval::<Option<u32>>()
                .unwrap(),
            None
        );
        lua.load("Fade.GlobalFadeFromBlack()").exec().unwrap();
        for _ in 0..30 {
            m.tick(0.1);
        }
        assert_eq!(m.black(), 0.0);
        let lua = m.host.lua();
        assert_eq!(lua.load("return Fade.closed").eval::<u32>().unwrap(), 1);
        assert!(blacks.iter().any(|&b| b > 0.0 && b < 1.0));
    }

    #[test]
    fn flown_deliveries_wait_for_the_game_to_say_they_arrived() {
        for flown in [false, true] {
            let mut m = Mission::new(GameData::default(), "Mattias", [0.0; 3]).unwrap();
            if flown {
                m.fly_deliveries();
            }
            {
                let mut w = m.world().borrow_mut();
                w.insert(80, Object::new("copter", [0.0, 100.0, 0.0]));
                let mut pilot = Object::new("pilot", [0.0, 100.0, 0.0]);
                pilot.vehicle = Some(80);
                w.insert(81, pilot);
                w.insert(82, Object::new("crate", [0.0, 95.0, 0.0]));
            }
            m.run(
                r#"
                local copter, crate = Sys.StringToGuid("80"), Sys.StringToGuid("82")
                Object.AttachCargoToWinch(crate, copter)
                Ai.Deliver(Sys.StringToGuid("81"), 50, 0, 60, 0.5)
                "#,
                "deliver",
            )
            .unwrap();
            let at = [50.0, 0.0, 60.0];
            let moved = |m: &mut Mission| {
                m.take_commands()
                    .iter()
                    .any(|c| matches!(c, Command::Move { guid: 82, position } if *position == at))
            };
            let there = |m: &Mission| m.world().borrow().objects[&82].position == at;
            // At once, unless the game flies it.
            assert_eq!(moved(&mut m), !flown);
            assert_eq!(there(&m), !flown);
            if flown {
                m.arrived(81, at);
                assert!(moved(&mut m));
                assert!(there(&m));
            }
        }
    }

    #[test]
    fn path_points_are_placed_by_their_entity() {
        // A quarter turn about up (x goes to -z) and a move.
        let half = std::f32::consts::FRAC_PI_4;
        let placed = Placement {
            position: [10.0, 1.0, 20.0],
            rotation: [0.0, half.sin(), 0.0, half.cos()],
            ..placement()
        };
        let out = place_points(&[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]], &placed);
        assert_eq!(out[0], [10.0, 1.0, 20.0]);
        let [x, y, z] = out[1];
        assert!((x - 10.0).abs() < 1e-5 && (y - 1.0).abs() < 1e-5 && (z - 18.0).abs() < 1e-5);
        // A rotation that isn't one counts as none.
        let bad = Placement {
            rotation: [0.0; 4],
            ..placed
        };
        assert_eq!(place_points(&[[2.0, 0.0, 0.0]], &bad), [[12.0, 1.0, 20.0]]);
    }

    fn placement() -> Placement {
        Placement {
            position: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            parent: 0,
            flags: 0,
        }
    }

    fn villa_tank() -> Entity {
        Entity {
            guid: 0xfaa53,
            name: "PMC001_EntourageScorpion".into(),
            at: [5.0, 0.0, 5.0],
            yaw: 0.0,
            region: None,
            labels: Vec::new(),
            script: None,
            faction: None,
            path: Vec::new(),
            template: None,
            crew: vec![
                ("seat_gunner".into(), "VZ Soldier".into()),
                ("seat_driver".into(), "VZ Tank Commander".into()),
            ],
        }
    }

    #[test]
    fn a_crewed_vehicle_comes_in_with_its_driver_and_goes_with_it() {
        let world: Shared = Rc::new(RefCell::new(ScriptWorld::default()));
        let tank = villa_tank();
        place(&world, std::slice::from_ref(&tank), true);
        {
            let w = world.borrow();
            let mut riders: Vec<(u32, &str)> = w
                .objects
                .iter()
                .filter(|(_, o)| o.vehicle == Some(tank.guid))
                .map(|(&g, o)| (g, o.name.as_str()))
                .collect();
            riders.sort_unstable();
            // The driver first, as `Vehicle.GetDriver` takes the first.
            assert_eq!(riders.len(), 2);
            assert_eq!(riders[0].1, "VZ Tank Commander");
            assert_eq!(riders[1].1, "VZ Soldier");
            assert!(w.objects[&riders[0].0].seated_instantly);
            assert_eq!(w.objects[&riders[0].0].position, [5.0, 0.0, 5.0]);
        }
        place(&world, std::slice::from_ref(&tank), false);
        assert!(world.borrow().objects.values().all(|o| o.vehicle.is_none()));
    }

    #[test]
    fn the_hq_interiors_starter_is_aboard_until_the_scripts_take_her_out() {
        // `HqInterior` (vz_state_pmcinterior 0xc73ad) is a building with
        // one `Rider` record of its own: Fiona, in seat a651ff78.
        let hq = Entity {
            guid: 0xc73ad,
            name: "HqInterior".into(),
            at: [3750.0, 450.0, -3840.0],
            template: None,
            crew: vec![("a651ff78".into(), "Fiona Taylor".into())],
            ..villa_tank()
        };
        let mut data = GameData::default();
        data.layers
            .insert("vz_state_pmcinterior".into(), vec![hq.clone()]);
        let mut m = Mission::new(data, "Mattias", [0.0; 3]).unwrap();
        place(m.world(), std::slice::from_ref(&hq), true);
        let aboard = m.crews_aboard();
        assert_eq!(aboard.len(), 1);
        let (g, crew) = &aboard[0];
        assert_eq!(*g, 0xc73ad);
        assert_eq!(crew.len(), 1);
        assert_eq!(crew[0].0, "a651ff78");
        assert_eq!(crew[0].2, "Fiona Taylor");
        let fiona = crew[0].1;
        // The scripts find her as the building's first rider.
        m.run(
            r#"__fiona = Vehicle.GetRiders(Pg.GetGuidByName("HqInterior"))[1]"#,
            "riders",
        )
        .unwrap();
        let found: Value = m.lua().globals().get("__fiona").unwrap();
        assert_eq!(found, crate::events::guid_value(fiona));
        m.run(
            r#"Vehicle.Exit(Pg.GetGuidByName("HqInterior"), __fiona, true)"#,
            "exit",
        )
        .unwrap();
        assert!(m.crews_aboard().is_empty());
    }

    /// The PMC's `HqInterior` as its layer places it (vz_state_pmcinterior
    /// 0xc73ad, template `PmcHq_Interior`) and a landing zone marker of a
    /// layer that isn't in either.
    fn pmc_interior_and_a_marker() -> GameData {
        let hq = Entity {
            guid: 0xc73ad,
            name: "HqInterior".into(),
            at: [3750.0, 450.0, -3840.0],
            template: Some("PmcHq_Interior".into()),
            crew: vec![("a651ff78".into(), "Fiona Taylor".into())],
            ..villa_tank()
        };
        let lz = Entity {
            guid: 0xd78ea,
            name: "01_pmc_hq_lz_playerone".into(),
            at: [3700.0, 450.0, -3800.0],
            template: Some("Landing Zone (player 1)".into()),
            crew: Vec::new(),
            ..villa_tank()
        };
        let mut data = GameData::default();
        data.layers.insert("vz_state_pmcinterior".into(), vec![hq]);
        data.layers.insert("vz_pmc_markers".into(), vec![lz]);
        data
    }

    #[test]
    fn an_object_of_a_layer_not_in_has_no_guid_by_name_but_a_marker_has() {
        let mut m = Mission::new(pmc_interior_and_a_marker(), "Mattias", [0.0; 3]).unwrap();
        m.run(
            r#"__hq = Pg.GetGuidByName("HqInterior")
            __lz = Pg.GetGuidByName("01_pmc_hq_lz_playerone")"#,
            "names",
        )
        .unwrap();
        let g = m.lua().globals();
        assert_eq!(g.get::<Value>("__hq").unwrap(), Value::Nil);
        assert_eq!(
            g.get::<Value>("__lz").unwrap(),
            crate::events::guid_value(0xd78ea)
        );
    }

    #[test]
    fn each_crew_aboard_is_its_own_objects_riders_by_object() {
        // Two crewed objects at once (the HQ's starter and the villa's
        // tank): each lists only its own riders, the objects in guid
        // order, with others (the island's) about that ride nothing.
        let hq = Entity {
            guid: 0xc73ad,
            name: "HqInterior".into(),
            template: None,
            crew: vec![("a651ff78".into(), "Fiona Taylor".into())],
            ..villa_tank()
        };
        let tank = villa_tank();
        let mut data = GameData::default();
        data.layers
            .insert("layers".into(), vec![tank.clone(), hq.clone()]);
        let m = Mission::new(data, "Mattias", [0.0; 3]).unwrap();
        for g in 0x1000..0x1400 {
            m.world()
                .borrow_mut()
                .insert(g, crate::api::Object::new("VZ Soldier", [0.0; 3]));
        }
        place(m.world(), &[tank, hq], true);
        let aboard = m.crews_aboard();
        let objects: Vec<u32> = aboard.iter().map(|a| a.0).collect();
        assert_eq!(objects, [0xc73ad, 0xfaa53]);
        let seats = |i: usize| -> Vec<(String, String)> {
            let mut s: Vec<_> = aboard[i]
                .1
                .iter()
                .map(|(seat, _, t)| (seat.clone(), t.clone()))
                .collect();
            s.sort();
            s
        };
        assert_eq!(seats(0), [("a651ff78".into(), "Fiona Taylor".into())]);
        assert_eq!(
            seats(1),
            [
                ("seat_driver".into(), "VZ Tank Commander".into()),
                ("seat_gunner".into(), "VZ Soldier".into()),
            ]
        );
    }

    #[test]
    fn a_targets_state_changes_and_actions_reach_the_verify_objectives_events() {
        let mut m = Mission::new(GameData::default(), "Mattias", [0.0; 3]).unwrap();
        m.world
            .borrow_mut()
            .insert(9, crate::api::Object::new("Blanco", [5.0, 0.0, 0.0]));
        // As `MrxTaskObjectiveVerify` waits: knocked down, then subdued,
        // and up again; and the player's photo action ending.
        m.run(
            r#"__heard = {}
            local function note(s) return function(g) __heard[#__heard + 1] = s end end
            local t = Sys.StringToGuid('0x9')
            Event.CreatePersistent(Event.HumanStateTransition, { t, "*", "KnockedDown.Idle" }, note("down"), {})
            Event.CreatePersistent(Event.HumanStateTransition, { t, "*", "Subdued.Idle" }, note("subdued"), {})
            Event.CreatePersistent(Event.HumanStateTransition, { t, "KnockedDown.*", "Upright.*" }, note("up"), {})
            Human.DoAction(Player.GetAnyCharacter(), "VerifyCamera")
            Event.Create(Event.HumanActionComplete, { Player.GetAnyCharacter() }, note("photo"), {})"#,
            "events",
        )
        .unwrap();
        let heard = |m: &Mission| -> Vec<String> {
            m.lua()
                .globals()
                .get::<Table>("__heard")
                .unwrap()
                .sequence_values()
                .map(|v| v.unwrap())
                .collect()
        };
        m.tick(0.1);
        assert!(heard(&m).is_empty());
        m.set_human_state(9, "KnockedDown.Idle");
        m.tick(0.1);
        m.set_human_state(9, "KnockedDown.Getup");
        m.tick(0.1);
        m.set_human_state(9, "Upright.Idle");
        m.tick(0.1);
        m.set_human_state(9, "Subdued.Idle");
        m.action_finished(PLAYER);
        m.tick(0.1);
        assert_eq!(heard(&m), ["down", "up", "subdued", "photo"]);
        // And carrying, as `Human.IsCarrying` reads it.
        m.set_carrying(PLAYER, Some(9));
        m.run("__c = Human.IsCarrying(Player.GetAnyCharacter())", "carry")
            .unwrap();
        assert!(m.lua().globals().get::<bool>("__c").unwrap());
    }
    #[test]
    fn leaving_the_play_area_or_entering_an_exclusion_tells_the_boundary_callback() {
        let mut m = Mission::new(GameData::default(), "Mattias", [0.0; 3]).unwrap();
        let square = |half: f32| m2_assets::Region {
            flags: 0,
            unk_0x04: 0,
            corners: vec![[-half, -half], [half, -half], [half, half], [-half, half]],
        };
        {
            let mut w = m.world.borrow_mut();
            w.insert(100, crate::api::Object::new("Boundary00", [0.0; 3]));
            w.regions.insert(100, square(100.0));
            w.insert(
                101,
                crate::api::Object::new("Boundary_ALHQ_Exclusion", [50.0, 0.0, 50.0]),
            );
            w.regions.insert(101, square(10.0));
        }
        m.run(
            r#"__heard = {}
            Player.SetBoundaryCallback(nil, function(_, zone, event)
              __heard[#__heard + 1] = zone .. ' ' .. event
            end)
            Player.AddBoundary(nil, Sys.StringToGuid('0x64'))
            Player.AddBoundary(nil, Sys.StringToGuid('0x65'))"#,
            "boundaries",
        )
        .unwrap();
        let at = |m: &mut Mission, p: [f32; 3]| {
            m.world
                .borrow_mut()
                .objects
                .get_mut(&PLAYER)
                .unwrap()
                .position = p;
            m.tick(0.1);
        };
        at(&mut m, [10.0, 0.0, 10.0]);
        // Out past the play area's edge, and back.
        at(&mut m, [150.0, 0.0, 0.0]);
        at(&mut m, [10.0, 0.0, 10.0]);
        // Into the exclusion, inside the play area, and out of it again.
        at(&mut m, [52.0, 0.0, 48.0]);
        at(&mut m, [10.0, 0.0, 10.0]);
        // The HQ's interior mode takes the boundaries away and clears her
        // being out: no "exit" when she comes back.
        at(&mut m, [150.0, 0.0, 0.0]);
        m.run(
            "Player.RemoveAllBoundary(nil)\nPlayer.SetOutBoundary(nil, false)",
            "interior",
        )
        .unwrap();
        at(&mut m, [10.0, 0.0, 10.0]);
        let heard: Vec<String> = m
            .lua()
            .globals()
            .get::<Table>("__heard")
            .unwrap()
            .sequence_values()
            .map(|v| v.unwrap())
            .collect();
        assert_eq!(
            heard,
            [
                "out enter",
                "out exit",
                "out enter",
                "out exit",
                "out enter"
            ]
        );
    }
    #[test]
    fn a_witness_reports_what_his_faction_saw_and_the_manager_settles_it() {
        let mut m = Mission::new(GameData::default(), "Mattias", [0.0; 3]).unwrap();
        // No manager yet: nobody reports.
        assert!(!m.begin_report(7, "OC"));
        // As `MrxFactionManager` does: turn down reporter 8; for one that
        // gets through (state 1) start a timer whose end (`__finish` here,
        // the HUD's `FactionDisplay` timer there) settles his report.
        m.run(
            r#"__heard = {}
            Report.Init({ Callback = function(g, state)
              __heard[#__heard + 1] = state
              if state == 0 and g == Sys.StringToGuid('0x8') then Report.Failed(g) end
              if state == 1 then
                __finish = function()
                  local t = Report.GetInfractions(g)
                  __hurt, __killed = t.DamagePerson[2], t.DestroyPerson[2]
                  Report.Completed(g)
                end
              end
            end })"#,
            "init",
        )
        .unwrap();
        m.record_infraction("OC", "DamagePerson");
        m.record_infraction("OC", "DamagePerson");
        m.record_infraction("OC", "DestroyPerson");
        m.record_infraction("Allied", "DamagePerson");
        assert!(!m.begin_report(8, "OC"));
        assert_eq!(m.take_report_ends(), vec![(8, false)]);
        // 8 took OC's tally with him; it starts again for 7.
        m.record_infraction("OC", "DamagePerson");
        assert!(m.begin_report(7, "OC"));
        m.report_through(7);
        assert!(m.take_report_ends().is_empty());
        m.run("__finish()", "finish").unwrap();
        assert_eq!(m.take_report_ends(), vec![(7, true)]);
        // One stopped before he is done loses his report.
        m.record_infraction("OC", "Hijack");
        assert!(m.begin_report(9, "OC"));
        m.stop_report(9);
        assert_eq!(m.take_report_ends(), vec![(9, false)]);
        let g = m.lua().globals();
        assert_eq!(g.get::<i64>("__hurt").unwrap(), 1);
        assert_eq!(g.get::<i64>("__killed").unwrap(), 0);
        let heard: Vec<i64> = g
            .get::<Table>("__heard")
            .unwrap()
            .sequence_values()
            .map(|v| v.unwrap())
            .collect();
        assert_eq!(heard, vec![0, 0, 1, 0, 2]);
        // Allied's tally waits for an Allied reporter.
        assert_eq!(m.world.borrow().reports.seen["Allied"]["DamagePerson"], 1);
    }

    #[test]
    fn a_factions_infraction_multiplier_scales_what_its_witnesses_report() {
        let data = GameData {
            template_names: ["oc", "guerilla"].map(String::from).into(),
            ..GameData::default()
        };
        let mut m = Mission::new(data, "Mattias", [0.0; 3]).unwrap();
        // As a faction contract's start does (2f0b1589), and a mission
        // that lets the Guerillas' witnesses see nothing (08dcd9af).
        m.run(
            r#"Report.Init({ Callback = function() end })
            Ai.SetInfractionMultiplier(Pg.GetGuidByName("OC"), 0.2)
            Ai.SetInfractionMultiplier(Pg.GetGuidByName("Guerilla"), 0)"#,
            "init",
        )
        .unwrap();
        for _ in 0..5 {
            m.record_infraction("OC", "DamagePerson");
        }
        m.record_infraction("Guerilla", "DestroyPerson");
        m.record_infraction("Allied", "DamagePerson");
        assert!(m.begin_report(7, "OC"));
        assert!(m.begin_report(8, "Guerilla"));
        assert!(m.begin_report(9, "Allied"));
        m.run(
            r#"__oc = Report.GetInfractions(Sys.StringToGuid('0x7')).DamagePerson[2]
            __gur = Report.GetInfractions(Sys.StringToGuid('0x8')).DestroyPerson[2]
            __all = Report.GetInfractions(Sys.StringToGuid('0x9')).DamagePerson[2]"#,
            "ask",
        )
        .unwrap();
        let g = m.lua().globals();
        let oc: f64 = g.get("__oc").unwrap();
        assert!((oc - 1.0).abs() < 1e-6, "5 x 0.2, got {oc}");
        assert_eq!(g.get::<f64>("__gur").unwrap(), 0.0);
        // Never set: as seen.
        assert_eq!(g.get::<i64>("__all").unwrap(), 1);
    }

    #[test]
    fn the_base_blocks_locators_are_objects_from_the_start() {
        // PmcCon003's "Get inside the PMC!" acts on `PlayerLocation_PMC`,
        // a locator of the base block's layer; a state layer's stays out.
        let at = |guid: u32, name: &str| Entity {
            guid,
            name: name.into(),
            at: [7.0, 1.0, 9.0],
            template: None,
            crew: Vec::new(),
            ..villa_tank()
        };
        let mut data = GameData::default();
        data.layers
            .insert("vz_base".into(), vec![at(0x9f305, "PlayerLocation_PMC")]);
        data.layers.insert(
            "vz_state_pmccon002".into(),
            vec![at(0xb639b, "PMC002_Office")],
        );
        let mut m = Mission::new(data, "Mattias", [0.0; 3]).unwrap();
        m.run(
            r#"local x, y, z = Object.GetPosition(Pg.GetGuidByName('PlayerLocation_PMC'))
            __base = x
            __state = Object.GetPosition(Pg.GetGuidByName('PMC002_Office'))"#,
            "ask",
        )
        .unwrap();
        let g = m.lua().globals();
        assert_eq!(g.get::<f64>("__base").unwrap(), 7.0);
        assert!(g.get::<Option<f64>>("__state").unwrap().is_none());
    }

    #[test]
    fn the_villa_tanks_driver_is_sent_along_its_path() {
        let path = Entity {
            guid: 0x11a600,
            name: "Path_PMC001_GarageSmash".into(),
            template: None,
            crew: Vec::new(),
            path: vec![[0.0, 0.0, 0.0], [0.0, 0.0, 30.0]],
            ..villa_tank()
        };
        let mut data = GameData::default();
        data.layers.insert(
            "vz_state_pmccon001_actionhijacktutorial".into(),
            vec![villa_tank(), path],
        );
        let mut m = Mission::new(data, "Mattias", [0.0; 3]).unwrap();
        assert_eq!(m.path(0x11a600), [[0.0, 0.0, 0.0], [0.0, 0.0, 30.0]]);
        assert!(m.path(0xfaa53).is_empty());
        let layer = m
            .layer("vz_state_pmccon001_actionhijacktutorial")
            .unwrap()
            .to_vec();
        place(m.world(), &layer, true);
        m.run(
            r#"Ai.Goal({ AIGuid = Vehicle.GetDriver(Pg.GetGuidByName("PMC001_EntourageScorpion")),
                Goal = "PathMove", Target = Pg.GetGuidByName("Path_PMC001_GarageSmash"),
                Priority = "HiPri", Haste = 1 })"#,
            "garage smash",
        )
        .unwrap();
        let goal = m.take_commands().into_iter().find_map(|c| match c {
            Command::Goal {
                guid,
                goal,
                target,
                haste,
                ..
            } => Some((guid, goal, target, haste)),
            _ => None,
        });
        let (driver, goal, target, haste) = goal.expect("a goal for the tank's driver");
        assert_eq!(goal, "PathMove");
        assert_eq!(target, Some(0x11a600));
        assert_eq!(haste, Some(1.0));
        assert_eq!(m.world().borrow().objects[&driver].vehicle, Some(0xfaa53));
    }

    #[test]
    fn yaw_reads_turns_about_up() {
        let half = std::f32::consts::FRAC_PI_4;
        let q = [0.0, half.sin(), 0.0, half.cos()];
        assert!((yaw(q) - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    }

    #[test]
    fn a_button_event_fires_on_its_press_after_it_was_made() {
        let mut m = Mission::new(GameData::default(), "Jennifer", [0.0; 3]).unwrap();
        m.press_button("selection");
        m.tick(0.1);
        m.run(
            r#"
            skipped = 0
            Event.Create(Event.Button, { Player.GetLocalPlayer(), "selection", "press", true },
                function() skipped = skipped + 1 end)
            "#,
            "test",
        )
        .unwrap();
        m.tick(0.1);
        m.press_button("cancel");
        m.tick(0.1);
        let skipped = |m: &Mission| m.host.lua().globals().get::<i64>("skipped").unwrap();
        assert_eq!(skipped(&m), 0);
        m.press_button("Selection");
        m.tick(0.1);
        m.tick(0.1);
        assert_eq!(skipped(&m), 1);
    }
}
