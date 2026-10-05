//! The game's own HUD: its layout and handler scripts (health, ammo and
//! weapon, reticle, hit arcs, cash and fuel, messages) run in a Lua state
//! of their own on a thread, as [`m2_game::gui_runtime`] runs them, and
//! what they build is drawn with the game's textures and bitmap fonts.
//!
//! Each frame the play's state goes to the thread ([`HudState`]); the
//! thread fires the game's GUI events for what changed, steps the scripts
//! and sends back the screen as textured quads, with any texture they
//! need for the first time decoded. Drawing lags play by a frame.
//!
//! The minimap is drawn here, as the engine draws its own: the island's
//! sea round her (see [`MAP_TEXTURE`]), turned with the camera, on the
//! radar's backdrop, with the island's roads over it ([`road_strokes`]),
//! the zones its Flash movie would outline (see [`Zone`]), the mission's
//! markers and her white square. Not drawn yet: the other Flash
//! pieces (pop-ups), animated sprites and clock wipes. With no aim point
//! the reticle's widgets are left out ([`reticle_widgets`], by the names
//! the layout gives them). The ammo the counters show is our own count (see
//! [`RIFLE_CLIP`]): the play has no ammo of its own yet.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, BufWriter, Write as _};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use egui::{Color32, Mesh, TextureHandle, TextureOptions, pos2};
use m2_assets::font::FONT_TYPE;
use m2_assets::layer::LAYER_TYPE;
use m2_assets::minimap::{self, Heights, Road, RoadRecord};
use m2_assets::path::PATH_TYPE;
use m2_assets::terrain::LOW_RES_TYPE;
use m2_assets::{BlockContents, Font, Layer, LowResTile, Path, Placement, WadArchive, name_hash};
use m2_engine::world::{ISLAND, WorldIndex};
use m2_engine::{AssetSource, Install, WorldSettings};
use m2_game::api::Object;
use m2_game::events::{as_guid, guid_value};
use m2_game::gui::Anchor;
use m2_game::gui_runtime::{self, DrawItem, TextSpan};
use m2_game::hud::ObjectiveMessage;
use m2_game::mission::{GameData, Mission};
use m2_math::glam::Vec3;
use mlua::{IntoLuaMulti, Lua, Table, Value};

use crate::combat::{Combat, Fired, Picked, Rounds, Weapon};
use crate::config::Config;
use crate::game::Game;
use crate::support::{self, SupportHud};

/// The fonts the HUD's text widgets use.
const FONTS: [&str; 2] = ["english_18", "english_20"];
/// The player's guid as the scripts know her (`Sys.StringToGuid("0x1")`).
const PLAYER: u32 = 1;
/// Guids standing for her guns (`uNewCurrentGunGuid`): objects of the
/// scripts' world, for the gun box shows `Object.GetLocalizedName` of it.
/// Past the guids the scripts' world gives out (layer entities, templates,
/// spawns, markers), so they name no other object.
const RIFLE_GUID: u32 = 0x6000_0000;
const RPG_GUID: u32 = 0x6000_0001;
const LAUNCHER_GUID: u32 = 0x6000_0002;
/// The guns' names, as the game's English strings write them. Her rifle
/// is the first contract's carbine (template 80007212), as the footage's
/// gun box reads.
const RIFLE_NAME: &str = "Carbine";
const RPG_NAME: &str = "RPG";
const LAUNCHER_NAME: &str = "Grenade Launcher";
/// The ammo the counters show. The play never runs out: the rifle's clip,
/// the rockets stored and the grenades count down and start again full;
/// the rifle's store stays as it is.
const RIFLE_CLIP: u32 = 30;
const RIFLE_STORED: u32 = 240;
const RPG_STORED: u32 = 4;
const GRENADES: u32 = 4;
/// The weapons' icons (`uNewCurrentGun`): textures of their own, as the
/// game names them (`weapons_*`).
const RIFLE_ICON: &str = "weapons_m4";
const RPG_ICON: &str = "weapons_rpg";
const LAUNCHER_ICON: &str = "weapons_grenade_launcher_2";
/// Their reticles (`uReticleTexture`), `reticle_*` by the kind of round.
const RIFLE_RETICLE: &str = "reticle_bullets";
const RPG_RETICLE: &str = "reticle_rockets";
const LAUNCHER_RETICLE: &str = "reticle_grenade";
/// Seconds an objective message stays in the message buffer, as
/// `DisplayObjectiveMessage` asks.
const MESSAGE_SECONDS: f64 = 5.0;
/// An objective message's words and priority from its kind, icon index and
/// text id, as `MrxGuiInterface.DisplayObjectiveMessage` builds them: the
/// kind's colour, the icon's tag, `[objective.<prefix><status>]`, the text.
/// Nothing for a kind the scripts don't know.
const WORD_MESSAGE: &str = r#"
local kind, icon, text = ...
local c = MrxGuiInterface.tObjectiveMessageConfigs[kind]
if not c then return nil end
local tag = MrxUtil.GetInlineIconNameByIndex(icon) or ""
return c.sColor .. tag .. " [objective." .. c.sPrefix .. c.sStatus .. "]" .. " " .. text,
    c.nPriority or 1
"#;
/// A region onto the radar by its name, as `WifVzBoundary` puts its
/// boundary there: the scripts' own `Hud.Radar:AddLineRegion` (colour and
/// alpha 0..=255, nil for the radar's defaults).
const ADD_REGION: &str = r#"
local name, r, g, b, a, invert = ...
local guid = Pg.GetGuidByName(name)
if guid then
    Hud.Radar:AddLineRegion({ vPlayer = Sys.StringToGuid("0x1"), uGuid = guid,
        nRed = r, nGreen = g, nBlue = b, nAlpha = a, bInvert = invert })
end
"#;
const REMOVE_REGION: &str = r#"
local guid = Pg.GetGuidByName(...)
if guid then
    Hud.Radar:RemoveLineRegion({ vPlayer = Sys.StringToGuid("0x1"), uGuid = guid })
end
"#;
/// The regions the radar's script holds for the movie: the
/// `MinimapFlash` widget object's `CustomData.tRegions` (each `uGuid`,
/// `sColor` `"0xRRGGBB"`, `nAlpha` 0..=100, `bInvert`).
const RADAR_ZONES: &str = r#"
local w = MrxGuiBase.WidgetIdIndex[...]
local held = w and w.CustomData and w.CustomData.tRegions
local out = {}
for _, r in pairs(held or {}) do out[#out + 1] = r end
return out
"#;
/// The satellite designator in her hands, for the gun box: a guid of the
/// scripts' world named as the template is (800052f3 "Satellite
/// Designator"). That template has no `WeaponUI` (the guns' icon and
/// reticle); its model is `global_weapon_detonator` and its equipment the
/// "C4 Detonator", so the gun box shows the detonator's HUD icon. Guessed:
/// no footage shows the gun box while she holds it.
const DESIGNATOR_GUID: u32 = 0x6000_0003;
const DESIGNATOR_ICON: &str = "explosives_c4_detonator";
/// The first contract's free Satellite Guided Bomb, as the scripts know it
/// (`MrxSupportData`'s freebie the contract adds).
const SATELLITE_FREEBIE: &str = "VzaCon01_SatBomb";
/// Before the scripts run: `Pg.FindPointFromCamera` answers the satellite
/// view's middle while it is open (`__m2_hud_view`), for the view's rulers
/// (`MrxGuiSatellite.HandleXUpdate` reads the point under the screen's
/// middle); otherwise as before.
const VIEW_POINT: &str = r#"
local find = Pg.FindPointFromCamera
Pg.FindPointFromCamera = function(...)
    local v = __m2_hud_view
    if v then return v[1], v[2], v[3] end
    return find(...)
end
"#;
/// A free support item onto the support menu, as the contract gives it
/// (`MrxSupportData.AddFreebie(key, n, nil, nil, 1)`): the menu's own
/// `AddItem`, with its icon, name, count and "FREE". First two values the
/// scripts' world gets wrong: the satellite designator's AA level comes
/// from the global `bil` (a slip for nil in its script: nil in the game,
/// a stub here), and the item's faction from a field never set (nil in
/// the game, "" here); either would deny the item.
const ADD_FREEBIE: &str = r#"
local key, n = ...
local d = MrxSupportData.tFreebieData[key]
if not d then return end
if d.oSupport.oDesignator then d.oSupport.oDesignator.sAATestLevel = nil end
d.oSupport.sFactionId = false
if d.bActive then
    MrxPmc.SetFreebieQty(d.sName, n)
else
    MrxSupportData.AddFreebie(key, n, nil, nil, 1)
end
"#;
const REMOVE_FREEBIE: &str = r#"
local d = MrxSupportData.tFreebieData[...]
if d and d.bActive then MrxSupportData.RemoveFreebie(...) end
"#;
/// The support menu opened (true) or closed: its own `Open` and `Close`.
const SUPPORT_MENU: &str = r#"
local open = ...
local menus = MrxGuiInterface._GetWidgetsForPlayers(Sys.StringToGuid("0x1"), "Support Menu")
for _, w in pairs(menus or {}) do
    if open then w:Open() else w:Close() end
end
"#;
/// Before the satellite view opens: the timing wheel's wedges (degrees
/// clockwise from the top) and the cost, as the satellite designator
/// gives them (`MrxGuiManager.SetSatelliteMinigameData`, `SetSatelliteCost`).
const SATELLITE_SETUP: &str = r#"
local sectors, cost = ...
local p = Sys.StringToGuid("0x1")
MrxGuiManager.SetSatelliteMinigameData(p, sectors)
MrxGuiManager.SetSatelliteCost(p, cost)
"#;
/// Each frame the view is open, before the scripts step: the time and
/// cost the view's boxes count up (its `_TimeUpdate` adds the step to
/// `nTime`; the cost is in dollars).
const SATELLITE_FEED: &str = r#"
local time, cost = ...
local w = MrxGui.GetWidgetByNameAndOwner("Satellite overlay", Sys.StringToGuid("0x1"))
local m = w and w.CustomData.oMinigame
if not m then return end
m.CustomData.nTime = time
m.CustomData.nCost = cost
m.CustomData.nCashPerSecond = 0
"#;
/// The HUD's tutorial box (`MrxGuiTutorial`: the line typed out under its
/// header, right of the middle) says this, or is put away (nil).
const TUTORIAL: &str = r#"
local text = ...
local w = MrxGui.GetWidgetByNameAndOwner("tutorial", Sys.StringToGuid("0x1"))
if w and w.SetText then w:SetText(text) end
"#;
/// After the scripts step, while the timing wheel shows: the wheel's own
/// widgets posed as the play has it (the minigame's own clock and hit
/// test are the play's): its size (0..=1 of the layout's), the marker's
/// angle (degrees clockwise from the top), which wedges the marker is in,
/// and the last press's mark (true a check, false a cross). As the
/// minigame draws them: a wedge lights while the marker passes through it,
/// the disc flashes green on a hit and red on a miss, under the check or
/// cross. With no size the wheel is put away.
const SATELLITE_WHEEL: &str = r#"
local size, angle, inside, mark = ...
local w = MrxGui.GetWidgetByNameAndOwner("Satellite overlay", Sys.StringToGuid("0x1"))
local m = w and w.CustomData.oMinigame
if not m then return end
local c = m.CustomData
if not size or size <= 0 then
    m:SetVisible(false)
    return
end
m:SetVisible(true)
w.CustomData.oReticle:SetVisible(false)
local r = 128 * size
local function place(x)
    x:SetLocation(c.nCenterX - r, c.nCenterY - r, c.nCenterX + r, c.nCenterY + r)
    x:SetVisible(true)
end
place(c.oBg)
if mark == true then
    c.oBg:SetColor(0, 255, 0, 255)
elseif mark == false then
    c.oBg:SetColor(200, 0, 0, 255)
else
    c.oBg:SetColor(MrxGuiSatellite.nBaseBgR, MrxGuiSatellite.nBaseBgG, MrxGuiSatellite.nBaseBgB, 255)
end
for i, s in ipairs(c.tSectorWidgets) do
    if s.CustomData.bInUse then
        place(s)
        if inside == i then
            s:SetColor(255, 255, 255, 255)
        else
            s:SetColor(MrxGuiSatellite.nSliceR, MrxGuiSatellite.nSliceG, MrxGuiSatellite.nSliceB, 255)
        end
    end
end
place(c.oCursor)
c.oCursor:SetRotation(angle)
-- The fire button's picture in the middle of the wedge the marker is in,
-- 80 of the wheel's 128 out, as the minigame's own update puts it; gone
-- after a press.
local d = inside and c.tSectorData and c.tSectorData[inside]
if d and mark == nil then
    local a = math.rad((d[1] + d[2]) / 2)
    local bx, by = c.nCenterX + math.sin(a) * 80 * size, c.nCenterY - math.cos(a) * 80 * size
    c.oButton:SetLocation(bx - 16 * size, by - 16 * size, bx + 16 * size, by + 16 * size)
    c.oButton:SetVisible(true)
    c.oButton:SetTranslucency(255)
else
    c.oButton:SetVisible(false)
end
if mark == nil then
    c.oFeedback:SetVisible(false)
else
    place(c.oFeedback)
    c.oFeedback:SetTexture(mark and "icon_success" or "icon_fail")
    c.oFeedback:SetTranslucency(255)
end
"#;
/// The satellite view's help, in the tutorial box as the PC footage has
/// it (its own `Satellite Help` text stays blank): steering the view and
/// confirming the target, by the game's own two lines (`[move] Move view`,
/// `[confirm] Confirm target`, by their keys' hashes); then the PC's
/// prompt for the timing wheel (`Press [pfire] at the right time.`).
const VIEW_HELP: &str = "[0x23fb33d9][n][0x3bfec0b6]";
const WHEEL_HELP: &str = "[SHELL.PCShell.Tutorial_SatMinigame_PC]";
/// Seconds of play the scripts are given at most per frame.
const MAX_STEP: f64 = 0.25;
/// Largest texture size decoded, pixels.
const MAX_TEXTURE: u32 = 2048;

/// The minimap's art: the engine draws the HUD's "VZ" map itself; it is
/// made the size of the island-wide map the distant terrain wears, the
/// island (`ISLAND` metres square, centred on the origin) seen from above,
/// `+x` and `+z` towards its left and top edges: its sea (see
/// [`radar_map`]), and the grey of its paved roads tells them from tracks.
const MAP_TEXTURE: &str = "vz_lrterrain";
/// The radar's backdrop (its `sBorderTexture`), with the north pointer on
/// its rim, `BORDER` virtual units across round a minimap 100 across.
const BORDER_TEXTURE: &str = "global_gui_hud_radar";
const BORDER: f32 = 154.0 / 100.0;
/// What the radar strokes its dashed roads with: two dashes, white in a
/// black rim, along 32 × 8 pixels.
const PATH_TEXTURE: &str = "global_radar_path";
/// The texture's across (u) through the middle of a dash: a paved road is
/// stroked with that column alone, solid with the dashes' rim.
const PATH_SOLID_U: f32 = 3.5 / 32.0;
/// The texture's across (u) of its first dash, rim to rim.
const PATH_DASH_U: f32 = 8.0 / 32.0;
/// Metres from the minimap's centre to its rim when the scripts set none
/// (`MinimapDataUpdateHandler` sets 150 at a walk, up to 400 at speed).
const DEFAULT_RANGE: f32 = 150.0;

/// What the HUD is told each frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudState {
    /// Screen size in points.
    pub size: [f32; 2],
    /// Seconds since the last state.
    pub dt: f64,
    /// Health, 0..=100: hers, or the vehicle's she is in.
    pub hp: f32,
    pub in_vehicle: bool,
    pub weapon: Weapon,
    pub fired: Fired,
    /// Rounds picked up (a supply crate's rockets).
    pub picked: Picked,
    /// The grenade launcher's rounds, while she carries one.
    pub launcher: Option<Rounds>,
    /// The label of the vehicle she is in.
    pub vehicle: Option<String>,
    /// Its cannon's shells, when they run out: left, and how many it
    /// carries (the AMX-30's 40, counted down in the gun box).
    pub cannon: Option<(u32, u32)>,
    /// Which way the camera looks, degrees, growing as it turns left
    /// (`Player.GetCameraXZHeading`).
    pub heading: f32,
    /// Hurts since the last state: damage and the bearing it came from
    /// (see [`direction`]).
    pub hurts: Vec<(f32, f32)>,
    /// Messages for the message buffer since the last state.
    pub messages: Vec<ObjectiveMessage>,
    /// Who is under the reticle, as `GuiReticleUpdate` takes it: -1 an
    /// enemy, 0 anyone else; and their health (of 100).
    pub target: Option<(i32, i32)>,
    /// Where she is (world), and which way she faces: degrees, growing
    /// turning left, as the heading.
    pub position: Option<[f32; 3]>,
    pub facing: f32,
    /// Objective markers for the minimap: where, icon (`deliverable`...),
    /// colour (RGBA).
    pub blips: Vec<([f32; 3], String, [u8; 4])>,
    /// Where the reticle goes: the screen point her weapon (or her
    /// vehicle's gun) aims at, -1..=1 across and up the screen as
    /// `Gui.GetReticlePosition` answers. None hides it (down, or in a
    /// vehicle without a gun).
    pub reticle: Option<[f32; 2]>,
    /// The sea's height, metres: the radar's map is water below it.
    pub sea_level: Option<f32>,
    /// The regions the mission has on the radar.
    pub radar_regions: Vec<RadarRegion>,
    /// Her support: the item ready, the menu, the designator, the
    /// satellite's view and its wheel.
    pub support: SupportHud,
    /// The tutorial line the mission has up (`Net.SetTutorialMessage`), for
    /// the HUD's own tutorial box.
    pub tutorial: Option<String>,
}

impl HudState {
    /// What the play shows now (the screen size and time step are the
    /// HUD's to fill in). Takes her hurts from `game`.
    pub fn of(game: &mut Game) -> Self {
        let player = game.scene.player.as_ref();
        let vehicle = player
            .and_then(|p| p.driving)
            .and_then(|i| game.scene.vehicles.get(i));
        let at = player.map(|p| p.character.position);
        // On foot, the person a shot fired now would hit: soldiers who
        // have turned on her are enemies.
        let people = &game.scene.people;
        let target = player
            .filter(|p| p.driving.is_none())
            .zip(game.scene.world.as_ref())
            .and_then(|(p, w)| Combat::aim_probe(&w.physics, people, p))
            .and_then(|(_, person)| people.npcs.get(person?))
            .filter(|n| n.alive)
            .map(|n| {
                let enemy = n.soldier && n.alert;
                (-i32::from(enemy), n.hp.round().clamp(0.0, 100.0) as i32)
            });
        let hurts = game
            .combat
            .take_hurts()
            .into_iter()
            .map(|(damage, from)| (damage, direction(at, from)))
            .collect();
        Self {
            hp: match vehicle {
                Some(v) if v.wrecked => 0.0,
                Some(_) => 100.0,
                None => game.combat.hp,
            },
            in_vehicle: vehicle.is_some(),
            weapon: game.combat.weapon,
            fired: game.combat.fired,
            picked: game.combat.picked,
            launcher: game.combat.launcher_rounds,
            vehicle: vehicle.map(|v| v.spec.label.clone()),
            cannon: vehicle.and_then(|v| Some((v.rounds()?, v.spec.tuning.gun?.rounds))),
            heading: game.camera.yaw.to_degrees(),
            target,
            position: at.map(|p| p.to_array()),
            facing: player.map_or(0.0, |p| p.facing.to_degrees()),
            hurts,
            sea_level: game.scene.world.as_ref().map(|w| w.settings.sea_level),
            ..Self::default()
        }
    }

    /// Folds a later state into this one: events add up, the rest is the
    /// later's.
    fn merge(&mut self, later: Self) {
        let dt = self.dt + later.dt;
        let mut hurts = std::mem::take(&mut self.hurts);
        let mut messages = std::mem::take(&mut self.messages);
        hurts.extend(later.hurts.iter().copied());
        messages.extend(later.messages.iter().cloned());
        *self = Self {
            dt,
            hurts,
            messages,
            ..later
        };
    }
}

/// The bearing of `from` seen from `at`, as `GuiPlayerReceiveDamage`
/// takes it: degrees that, added to the camera's heading
/// ([`HudState::heading`]), give how far clockwise on screen from straight
/// ahead the hit arc points (`MrxGuiHudDamageIndicator` turns the arc to
/// direction + heading each frame; its art points up unturned). With
/// headings growing as the camera turns left, the bearing grows the other
/// way: 0 towards +z, 90 towards -x. 0 when either is unknown.
fn direction(at: Option<Vec3>, from: Option<Vec3>) -> f32 {
    let (Some(at), Some(from)) = (at, from) else {
        return 0.0;
    };
    let d = from - at;
    if d.x.abs() + d.z.abs() < 1e-3 {
        return 0.0;
    }
    // Heading grows turning left: forward is (sin h, 0, cos h).
    (-d.x.atan2(d.z)).to_degrees().rem_euclid(360.0)
}

/// What the vehicle box shows for a vehicle label: its name, and its
/// faction as the game's guid for it (`SetVehicleName(name, faction)`
/// picks the faction's icon; none shows the PMC's). Labels start with the
/// faction (`"vz_veh_truck_m151"` is the VZ's `"truck m151"`).
fn vehicle_name(label: &str) -> (String, Option<u32>) {
    let (faction, name) = label.split_once("_veh_").unwrap_or(("", label));
    // `MrxGuiHudVehicleDisguise.Init`'s faction guids.
    let guid = match faction {
        "al" => Some(0xbbc3_4ef4),
        "ch" => Some(0x4135_9cce),
        "oc" => Some(0xe947_b797),
        "gr" => Some(0xb10d_73ce),
        "pr" => Some(0xc182_15fe),
        "pmc" => Some(0x30e4_a26f),
        "civ" => Some(0xdcc8_b14d),
        "vz" => Some(0xb442_0059),
        _ => None,
    };
    (name.replace('_', " "), guid)
}

/// A textured (or, without texture, plain) quad in points: corners
/// clockwise from the top left, their UVs, and a colour (RGBA 0..=1) that
/// multiplies the texture.
#[derive(Clone, Debug, PartialEq)]
pub struct Prim {
    pub texture: Option<u32>,
    pub corners: [[f32; 2]; 4],
    pub uv: [[f32; 2]; 4],
    pub colour: [f32; 4],
}

/// RGBA8 pixels.
struct Image {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// What the thread sends back.
enum Output {
    Frame {
        prims: Vec<Prim>,
        images: Vec<(u32, Option<Image>)>,
        log: Vec<String>,
    },
    Failed(String),
}

enum Status {
    Loading,
    Ready,
    Failed,
}

/// The game's HUD, on the main thread: sends the play's state and paints
/// what comes back.
pub struct GameHud {
    tx: Sender<HudState>,
    rx: Receiver<Output>,
    status: Status,
    prims: Vec<Prim>,
    /// Decoded and waiting for an egui context to upload them.
    pending: Vec<(u32, Option<Image>)>,
    /// Textures by asset hash (`None`: missing in the archive).
    textures: HashMap<u32, Option<TextureHandle>>,
    /// The last objective message seen, to tell new ones.
    last_message: Option<ObjectiveMessage>,
    /// The time of the last frame (egui's input time).
    last_time: Option<f64>,
    log: Option<BufWriter<File>>,
}

impl std::fmt::Debug for GameHud {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameHud")
            .field("prims", &self.prims.len())
            .field("textures", &self.textures.len())
            .finish_non_exhaustive()
    }
}

impl GameHud {
    /// Starts the HUD's scripts reading `install` in the background.
    pub fn start(install: &Install) -> Self {
        let (tx, thread_rx) = channel();
        let (thread_tx, rx) = channel();
        let data = install.data.clone();
        std::thread::Builder::new()
            .name("game hud".into())
            .spawn(move || run(&data, thread_rx, thread_tx))
            .expect("could not start the HUD thread");
        let path = Config::file_beside("m2-alpha-hud.log");
        let log = File::create(&path).ok().map(BufWriter::new);
        Self {
            tx,
            rx,
            status: Status::Loading,
            prims: Vec::new(),
            pending: Vec::new(),
            textures: HashMap::new(),
            last_message: None,
            last_time: None,
            log,
        }
    }

    /// The scripts are running and have drawn.
    pub fn ready(&self) -> bool {
        matches!(self.status, Status::Ready)
    }

    /// The scripts could not start: the old HUD stands in.
    pub fn failed(&self) -> bool {
        matches!(self.status, Status::Failed)
    }

    /// Seconds since the last frame, from egui's input `time`.
    pub fn frame_time(&mut self, time: Option<f64>) -> f64 {
        let dt = match (time, self.last_time) {
            (Some(now), Some(then)) => (now - then).clamp(0.0, MAX_STEP),
            _ => 0.0,
        };
        self.last_time = time.or(self.last_time);
        dt
    }

    /// An objective message for the message buffer, if new.
    pub fn message(&mut self, message: Option<&ObjectiveMessage>) -> Option<ObjectiveMessage> {
        if message == self.last_message.as_ref() {
            return None;
        }
        self.last_message = message.cloned();
        message.cloned()
    }

    /// Sends this frame's state and takes in what the thread has drawn.
    pub fn update(&mut self, ctx: &egui::Context, state: HudState) {
        if self.failed() {
            return;
        }
        let _ = self.tx.send(state);
        loop {
            match self.rx.try_recv() {
                Ok(out) => self.take(out),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if !self.failed() {
                        self.fail("the HUD thread stopped".into());
                    }
                    break;
                }
            }
        }
        self.upload(ctx);
    }

    /// Like [`update`](Self::update), but waits (up to `timeout`) for the
    /// thread to draw this state: for screenshots.
    pub fn update_and_wait(&mut self, ctx: &egui::Context, state: HudState, timeout: Duration) {
        if self.failed() {
            return;
        }
        let _ = self.tx.send(state);
        let until = Instant::now() + timeout;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(out) => {
                    let frame = matches!(out, Output::Frame { .. });
                    self.take(out);
                    if frame || self.failed() {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    self.fail("the HUD thread stopped".into());
                    break;
                }
            }
        }
        self.upload(ctx);
    }

    fn take(&mut self, out: Output) {
        match out {
            Output::Frame { prims, images, log } => {
                self.status = Status::Ready;
                self.prims = prims;
                self.pending.extend(images);
                self.write(log);
            }
            Output::Failed(why) => self.fail(why),
        }
    }

    fn fail(&mut self, why: String) {
        eprintln!("game HUD: {why}");
        self.write([why]);
        self.status = Status::Failed;
    }

    fn write(&mut self, lines: impl IntoIterator<Item = String>) {
        if let Some(log) = &mut self.log {
            for line in lines {
                let _ = writeln!(log, "{line}");
            }
            let _ = log.flush();
        }
    }

    fn upload(&mut self, ctx: &egui::Context) {
        for (hash, image) in self.pending.drain(..) {
            let handle = image.map(|i| {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [i.width as usize, i.height as usize],
                    &i.rgba,
                );
                // Tiled widgets repeat their texture.
                ctx.load_texture(
                    format!("hud {hash:08x}"),
                    image,
                    TextureOptions::LINEAR_REPEAT,
                )
            });
            self.textures.insert(hash, handle);
        }
    }

    /// Paints the HUD as last drawn.
    pub fn paint(&self, painter: &egui::Painter) {
        let mut mesh = Mesh::default();
        for p in &self.prims {
            let texture = match p.texture {
                Some(hash) => match self.textures.get(&hash) {
                    Some(Some(t)) => t.id(),
                    // Missing, or not uploaded yet.
                    _ => continue,
                },
                None => egui::TextureId::default(),
            };
            if mesh.texture_id != texture && !mesh.is_empty() {
                painter.add(std::mem::take(&mut mesh));
            }
            mesh.texture_id = texture;
            let [r, g, b, a] = p.colour.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
            let colour = Color32::from_rgba_unmultiplied(r, g, b, a);
            let base = mesh.vertices.len() as u32;
            for (c, uv) in p.corners.iter().zip(&p.uv) {
                let uv = match p.texture {
                    Some(_) => pos2(uv[0], uv[1]),
                    // The font atlas's white texel.
                    None => egui::epaint::WHITE_UV,
                };
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: pos2(c[0], c[1]),
                    uv,
                    color: colour,
                });
            }
            mesh.indices.extend([0, 1, 2, 0, 2, 3].map(|i| base + i));
        }
        if !mesh.is_empty() {
            painter.add(mesh);
        }
    }
}

/// The thread: reads the scripts, fonts and strings, starts the HUD, then
/// draws each state sent until the sender goes.
fn run(data: &std::path::Path, rx: Receiver<HudState>, tx: Sender<Output>) {
    let mut hud = match Thread::start(data) {
        Ok(h) => h,
        Err(e) => {
            let _ = tx.send(Output::Failed(e));
            return;
        }
    };
    while let Ok(mut state) = rx.recv() {
        // Behind: fold everything waiting into one step.
        while let Ok(later) = rx.try_recv() {
            state.merge(later);
        }
        let out = hud.frame(&state);
        if tx.send(out).is_err() {
            return;
        }
    }
}

/// What the HUD has been told, to fire only what changed.
#[derive(Default)]
struct Told {
    hp: Option<(i32, bool)>,
    /// The gun in her hands, and whether she holds the designator.
    weapon: Option<(Weapon, bool)>,
    ammo: Option<[u32; 5]>,
    vehicle: Option<Option<String>>,
    target: Option<Option<(i32, i32)>>,
    reticle: Option<Option<[f32; 2]>>,
    regions: Option<Vec<RadarRegion>>,
    /// The support item ready (how many, whether free), and the menu.
    support_item: Option<Option<(u32, bool)>>,
    menu: Option<bool>,
    /// The satellite's view is open.
    satellite: bool,
    /// What the tutorial box says (None: never anything).
    tutorial: Option<Option<String>>,
}

/// How far (in -1..=1 screen units, across plus up) the aim point moves
/// before the reticle is moved.
const RETICLE_STEP: f32 = 0.002;

struct Thread {
    mission: Mission,
    assets: AssetSource<BufReader<File>>,
    fonts: BTreeMap<String, Font>,
    /// Textures tried (sent, or found missing).
    tried: HashSet<u32>,
    told: Told,
    /// The reticle's widgets were listed in the log.
    reticle_logged: bool,
    /// The zones the radar's script holds, as last read.
    zones: Vec<Zone>,
    /// The island's roads, read with the radar's map.
    roads: Vec<Road>,
}

impl Thread {
    fn start(data: &std::path::Path) -> Result<Self, String> {
        let wad = data.join("vz.wad");
        let open = || {
            File::open(&wad)
                .map(BufReader::new)
                .map_err(|e| format!("{}: {e}", wad.display()))
        };
        let game_data = WadArchive::open(open()?)
            .map_err(|e| e.to_string())
            .and_then(|mut w| GameData::read(&mut w).map_err(|e| e.to_string()))?;
        let mut assets = AssetSource::new(open()?).map_err(|e| e.to_string())?;
        let mut fonts = BTreeMap::new();
        for name in FONTS {
            match assets
                .asset(name_hash(name), FONT_TYPE)
                .map_err(|e| e.to_string())
                .and_then(|a| Font::parse(&a).map_err(|e| e.to_string()))
            {
                Ok(f) => {
                    fonts.insert(name.to_owned(), f);
                }
                Err(e) => eprintln!("game HUD: font {name}: {e}"),
            }
        }
        let strings = game_data.strings.clone();
        let mut mission =
            Mission::new(game_data, "Mattias", [0.0; 3]).map_err(|e| e.to_string())?;
        let screen = gui_runtime::screen(mission.lua()).ok_or("no GUI runtime")?;
        {
            let mut s = screen.borrow_mut();
            s.set_strings(strings);
            for (name, f) in &fonts {
                s.set_font(name, f.clone());
            }
        }
        // Her guns, for the gun box to name.
        {
            let mut world = mission.world().borrow_mut();
            for (guid, name) in [
                (RIFLE_GUID, RIFLE_NAME),
                (RPG_GUID, RPG_NAME),
                (LAUNCHER_GUID, LAUNCHER_NAME),
                (DESIGNATOR_GUID, support::Kind::SatelliteBomb.designator()),
            ] {
                world.insert(guid, Object::new(name, [0.0; 3]));
            }
        }
        // As the player-character module does; her guid is userdata, or
        // the scripts never own the widgets and no event reaches them.
        mission
            .run(
                "MrxGuiManager.CreateGui(Sys.StringToGuid(\"0x1\"))",
                "create gui",
            )
            .map_err(|e| e.to_string())?;
        mission
            .run(VIEW_POINT, "m2-hud view point")
            .map_err(|e| e.to_string())?;
        // What the game finishes while it loads, before play: the
        // movies' load callbacks (the PDA's closes it, `_FinishLoadAndClose`;
        // left open, the support menu won't open over it).
        gui_runtime::tick(mission.lua(), 0.0);
        Ok(Self {
            mission,
            assets,
            fonts,
            tried: HashSet::new(),
            told: Told::default(),
            reticle_logged: false,
            zones: Vec::new(),
            roads: Vec::new(),
        })
    }

    fn frame(&mut self, state: &HudState) -> Output {
        let mut log = Vec::new();
        if let Err(e) = self.feed(state) {
            log.push(format!("event: {e}"));
        }
        let dt = state.dt.min(MAX_STEP);
        if let Err(e) = self.feed_support(state, dt) {
            log.push(format!("support: {e}"));
        }
        if dt > 0.0 {
            self.mission.tick(dt);
            gui_runtime::tick(self.mission.lua(), dt);
        }
        if let Err(e) = self.pose_wheel(state) {
            log.push(format!("wheel: {e}"));
        }
        let Some(screen) = gui_runtime::screen(self.mission.lua()) else {
            return Output::Failed("no GUI runtime".into());
        };
        let [w, h] = state.size;
        let items = screen.borrow().draw_list(w, h);
        log.extend(screen.borrow_mut().take_log());
        log.extend(self.mission.take_log());
        // The scripts' own commands (sounds, camera) are not the HUD's.
        let _ = self.mission.take_commands();
        let names = gui_runtime::script_names(self.mission.lua());
        let zones = self.zones(&names);
        if zones.len() != self.zones.len() {
            log.push(format!("radar zones: {}", zones.len()));
        }
        self.zones = zones;
        // Looking through the satellite, only its view shows: the
        // scripts put the HUD to sleep (`MrxGuiManager.ToggleHud`), which
        // leaves some of it awake here.
        // Its help is in the tutorial box (as the PC footage has it), which
        // stays awake.
        let only = state.support.satellite.map(|_| {
            widgets_under(&screen.borrow(), &names, |n| {
                n == "Satellite overlay" || n == "tutorial"
            })
        });
        // No aim point: the reticle's widgets are left out (not the
        // satellite's own, its crosshair).
        let hidden = match state.reticle.or(state.support.satellite.map(|_| [0.0; 2])) {
            Some(_) => HashSet::new(),
            None => {
                let hidden = reticle_widgets(&screen.borrow(), &names);
                if !self.reticle_logged {
                    self.reticle_logged = true;
                    let mut named: Vec<&str> = hidden
                        .iter()
                        .filter_map(|id| names.get(id).map(String::as_str))
                        .collect();
                    named.sort_unstable();
                    log.push(format!(
                        "reticle hidden: {} widgets, named {}",
                        hidden.len(),
                        named.join(", ")
                    ));
                }
                hidden
            }
        };
        let mut prims = Vec::new();
        let mut want = Vec::new();
        let shown = |i: &&DrawItem| {
            let w = widget_of(i);
            !hidden.contains(&w) && only.as_ref().is_none_or(|o| o.contains(&w))
        };
        for item in items.iter().filter(shown) {
            self.prims(item, state, &mut prims, &mut want);
        }
        let mut images = Vec::new();
        for hash in want {
            if self.tried.insert(hash) {
                if hash == name_hash(MAP_TEXTURE) {
                    let started = Instant::now();
                    let sea = state
                        .sea_level
                        .unwrap_or(WorldSettings::default().sea_level);
                    let image = match radar_map(&mut self.assets, sea) {
                        Ok((i, roads)) => {
                            let paved = roads.iter().filter(|r| r.paved).count();
                            log.push(format!(
                                "radar map: {}x{}, sea at {sea} m, {} roads ({paved} paved), in {:.0?}",
                                i.width,
                                i.height,
                                roads.len(),
                                started.elapsed()
                            ));
                            self.roads = roads;
                            Some(i)
                        }
                        Err(e) => {
                            log.push(format!("radar map: {e}"));
                            None
                        }
                    };
                    images.push((hash, image));
                    continue;
                }
                let image = match self.assets.texture(hash, MAX_TEXTURE) {
                    Ok(t) => t.mips.into_iter().next().map(|rgba| Image {
                        width: t.width,
                        height: t.height,
                        rgba,
                    }),
                    Err(e) => {
                        log.push(format!("texture {hash:08x}: {e}"));
                        None
                    }
                };
                images.push((hash, image));
            }
        }
        Output::Frame { prims, images, log }
    }

    /// Fires the GUI events for what changed since the last state.
    fn feed(&mut self, s: &HudState) -> mlua::Result<()> {
        let lua = self.mission.lua();
        let first = self.told.hp.is_none();
        if first {
            for event in ["PlayerCashChanged", "PlayerFuelChanged"] {
                let t = table(
                    lua,
                    [("uGuid", guid_value(PLAYER)), ("nValue", Value::Integer(0))],
                )?;
                fire(lua, event, t)?;
            }
            let t = table(lua, [("nX", Value::Integer(0)), ("nY", Value::Integer(0))])?;
            fire(lua, "GuiReticlePositionChange", t)?;
        }
        // The satellite looks down with north up: its compass reads the
        // camera's heading as the view opens.
        let heading = if s.support.satellite.is_some() {
            0.0
        } else {
            s.heading
        };
        if let Some(screen) = gui_runtime::screen(lua) {
            screen.borrow_mut().set_camera_heading(heading);
        }
        // Where she is for the minimap, each frame as the engine does: its
        // handler hands it on to `MinimapUpdate`, with the heading as the
        // rotation (the map turns with the camera).
        if let Some([x, y, z]) = s.position {
            fire(lua, "GuiMinimapUpdate", (x, y, z, s.heading))?;
        }
        let hp = (s.hp.round() as i32, s.in_vehicle);
        if self.told.hp != Some(hp) {
            self.told.hp = Some(hp);
            fire(lua, "GuiHealthUpdate", (hp.0, 100, hp.1))?;
        }
        if self.told.vehicle.as_ref() != Some(&s.vehicle) {
            self.told.vehicle = Some(s.vehicle.clone());
            let (name, faction) = match &s.vehicle {
                Some(label) => {
                    let (name, faction) = vehicle_name(label);
                    (
                        Value::String(lua.create_string(name)?),
                        faction.map_or(Value::Nil, guid_value),
                    )
                }
                None => (Value::Boolean(false), Value::Nil),
            };
            fire(lua, "GuiVehicleNameUpdate", (name, faction))?;
        }
        let holding = s.support.designator.is_some();
        if self.told.weapon != Some((s.weapon, holding)) {
            self.told.weapon = Some((s.weapon, holding));
            let (icon, guid, reticle) = match s.weapon {
                Weapon::Rifle => (RIFLE_ICON, RIFLE_GUID, RIFLE_RETICLE),
                Weapon::Rpg => (RPG_ICON, RPG_GUID, RPG_RETICLE),
                Weapon::GrenadeLauncher => (LAUNCHER_ICON, LAUNCHER_GUID, LAUNCHER_RETICLE),
            };
            // The designator up in her hands: the gun box names it; the
            // reticle stays the gun's.
            let (icon, guid) = if holding {
                (DESIGNATOR_ICON, DESIGNATOR_GUID)
            } else {
                (icon, guid)
            };
            // What `MrxGuiHudReticle.HandleReticleGunSwitchEvent` reads:
            // without a type it leaves the reticle as it was. No target
            // health type: that bar shows only on a target.
            let t = table(
                lua,
                [
                    ("uNewCurrentGun", Value::String(lua.create_string(icon)?)),
                    ("uNewCurrentGunGuid", guid_value(guid)),
                    ("sReticleType", Value::String(lua.create_string("Normal")?)),
                    (
                        "uReticleTexture",
                        Value::String(lua.create_string(reticle)?),
                    ),
                    ("bReticleCrosshair", Value::Boolean(true)),
                ],
            )?;
            fire(lua, "GuiWeaponEquippedUpdate", t)?;
        }
        let ammo = ammo(s.weapon, s.fired, s.picked, s.launcher, s.cannon);
        if self.told.ammo != Some(ammo) {
            self.told.ammo = Some(ammo);
            let names = [
                "PrimaryClipSize",
                "PrimaryCurrentAmmo",
                "PrimaryStoredAmmo",
                "ExplosivesCurrentAmmo",
                "ExplosivesStoredAmmo",
            ];
            let t = lua.create_table()?;
            for (k, v) in names.iter().zip(ammo) {
                t.raw_set(*k, v)?;
            }
            fire(lua, "GuiAmmoUpdate", t)?;
        }
        // The reticle sits where she aims, as the game's camera tells it.
        let moved = match (self.told.reticle, s.reticle) {
            (Some(Some(a)), Some(b)) => (a[0] - b[0]).abs() + (a[1] - b[1]).abs() > RETICLE_STEP,
            (Some(a), b) => a.is_some() != b.is_some(),
            (None, _) => true,
        };
        if moved {
            self.told.reticle = Some(s.reticle);
            if let Some([x, y]) = s.reticle {
                if let Some(screen) = gui_runtime::screen(lua) {
                    screen.borrow_mut().set_reticle(x, y);
                }
                let t = table(
                    lua,
                    [
                        ("nX", Value::Number(x.into())),
                        ("nY", Value::Number(y.into())),
                    ],
                )?;
                fire(lua, "GuiReticlePositionChange", t)?;
            }
        }
        if self.told.target != Some(s.target) {
            self.told.target = Some(s.target);
            // (target, centre x, y, spread x, y, health, maximum): no
            // health fades the target-health arc (opaque as laid out);
            // a health shows that much of it.
            match s.target {
                Some((kind, hp)) => fire(
                    lua,
                    "GuiReticleUpdate",
                    (kind, Value::Nil, Value::Nil, 0, 0, hp, 100),
                )?,
                None => fire(lua, "GuiReticleUpdate", 0)?,
            }
        }
        for &(damage, direction) in &s.hurts {
            fire(lua, "GuiPlayerReceiveDamage", (direction, damage))?;
        }
        for m in &s.messages {
            // Worded as `MrxGuiInterface.DisplayObjectiveMessage` words it,
            // from the scripts' own tables: colour, icon, "[objective.
            // Objectiveadded]" and the objective's text id.
            let (text, priority): (Option<String>, Option<mlua::Integer>) = lua
                .load(WORD_MESSAGE)
                .set_name("=m2-hud message")
                .call((m.kind.as_str(), m.icon, m.text.as_str()))?;
            let Some(text) = text else {
                continue;
            };
            let priority = priority.unwrap_or(1);
            let t = table(
                lua,
                [
                    ("sMessage", Value::String(lua.create_string(text)?)),
                    ("nPriority", Value::Integer(priority)),
                    ("nDuration", Value::Number(MESSAGE_SECONDS)),
                    ("bClearBuffer", Value::Boolean(true)),
                    ("bAllowsAppends", Value::Boolean(false)),
                ],
            )?;
            fire(lua, "TextBufferAddMessage", t)?;
        }
        // The mission's radar regions, put on this radar as the mission
        // put them on its own (`Hud.Radar:AddLineRegion`): the radar's
        // script keeps them (`tRegions`), and they are drawn from there.
        let told = self.told.regions.take().unwrap_or_default();
        for r in told.iter().filter(|r| !s.radar_regions.contains(r)) {
            lua.load(REMOVE_REGION)
                .set_name("=m2-hud region")
                .call::<()>(r.name.as_str())?;
        }
        for r in s.radar_regions.iter().filter(|r| !told.contains(r)) {
            let [red, green, blue, alpha] = r.colour;
            lua.load(ADD_REGION)
                .set_name("=m2-hud region")
                .call::<()>((r.name.as_str(), red, green, blue, alpha, r.invert))?;
        }
        self.told.regions = Some(s.radar_regions.clone());
        Ok(())
    }

    /// Feeds the support: the item onto the support menu (as the contract
    /// gives its free bomb), the menu opened and closed, and the
    /// satellite's view opened, kept in step (time, cost, help, where it
    /// looks) and closed, as the satellite designator drives it. `step` is
    /// the time the scripts are about to be given.
    fn feed_support(&mut self, s: &HudState, step: f64) -> mlua::Result<()> {
        let lua = self.mission.lua();
        let sup = &s.support;
        // Only the Satellite Guided Bomb is called in yet.
        let item = sup
            .item
            .as_ref()
            .filter(|i| i.0 == support::Kind::SatelliteBomb.name())
            .map(|i| (i.2, i.3));
        if self.told.support_item != Some(item) {
            self.told.support_item = Some(item);
            match item {
                Some((n, _)) => lua
                    .load(ADD_FREEBIE)
                    .set_name("=m2-hud support")
                    .call::<()>((SATELLITE_FREEBIE, n))?,
                None => lua
                    .load(REMOVE_FREEBIE)
                    .set_name("=m2-hud support")
                    .call::<()>(SATELLITE_FREEBIE)?,
            }
        }
        if self.told.menu != Some(sup.menu) {
            // Closed as it starts: no need to close it.
            if self.told.menu.is_some() || sup.menu {
                lua.load(SUPPORT_MENU)
                    .set_name("=m2-hud support menu")
                    .call::<()>(sup.menu)?;
            }
            self.told.menu = Some(sup.menu);
        }
        let open = sup.satellite.is_some();
        if open && !self.told.satellite {
            // The wedges as the play has them, degrees clockwise from
            // the top; free: no cost.
            let sectors = lua.create_table()?;
            for mid in support::SECTORS {
                let (mid, half) = (mid.to_degrees(), support::WEDGE.to_degrees());
                sectors.push(lua.create_sequence_from([mid - half, mid + half])?)?;
            }
            lua.load(SATELLITE_SETUP)
                .set_name("=m2-hud satellite")
                .call::<()>((sectors, 0))?;
        }
        if open != self.told.satellite {
            self.told.satellite = open;
            // As `MrxGuiManager.ToggleSatellite(player, open, "pmc")`
            // sends it: the minigame on, not the advanced view.
            let t = table(
                lua,
                [
                    (
                        "EventType",
                        Value::String(lua.create_string("SatelliteStateChange")?),
                    ),
                    ("uPlayerGuid", guid_value(PLAYER)),
                    ("bActivate", Value::Boolean(open)),
                    ("bAdvanced", Value::Boolean(false)),
                    ("bMinigame", Value::Boolean(true)),
                ],
            )?;
            fire(lua, "SatelliteStateChange", t)?;
            if !open {
                lua.globals().raw_set("__m2_hud_view", Value::Nil)?;
                lua.load(SATELLITE_WHEEL)
                    .set_name("=m2-hud wheel")
                    .call::<()>(())?;
            }
        }
        if let Some((middle, seconds, cost)) = sup.satellite {
            lua.globals().raw_set(
                "__m2_hud_view",
                lua.create_sequence_from(middle.to_array())?,
            )?;
            // The view's clock adds the step as the scripts run.
            lua.load(SATELLITE_FEED)
                .set_name("=m2-hud satellite")
                .call::<()>((f64::from(seconds) - step, f64::from(cost) * 1000.0))?;
            // What is under the crosshair, as the engine tells the view
            // each frame: nothing yet (no name, no faction), which blanks
            // the readout and puts the faction's badge away.
            fire(lua, "SatelliteTargetUpdate", lua.create_table()?)?;
        }
        // The tutorial box: the satellite's help while its view is open,
        // else the mission's line.
        let tutorial = match (sup.satellite, &sup.wheel) {
            (Some(_), Some(_)) => Some(WHEEL_HELP.to_owned()),
            (Some(_), None) => Some(VIEW_HELP.to_owned()),
            (None, _) => s.tutorial.clone(),
        };
        // Never up yet: nothing to put away (putting it away starts its
        // closing, whose end would fold up the next line's header).
        if self.told.tutorial.clone().flatten() != tutorial {
            lua.load(TUTORIAL)
                .set_name("=m2-hud tutorial")
                .call::<()>(tutorial.as_deref())?;
            self.told.tutorial = Some(tutorial);
        }
        Ok(())
    }

    /// Poses the timing wheel's widgets as the play has the wheel, after
    /// the scripts have stepped (see [`SATELLITE_WHEEL`]).
    fn pose_wheel(&self, s: &HudState) -> mlua::Result<()> {
        if s.support.satellite.is_none() {
            return Ok(());
        }
        let call = self
            .mission
            .lua()
            .load(SATELLITE_WHEEL)
            .set_name("=m2-hud wheel");
        match &s.support.wheel {
            Some(w) => {
                let inside = support::sector(w.angle).map(|i| i + 1);
                call.call::<()>((w.size, w.angle.to_degrees(), inside, w.mark))
            }
            None => call.call::<()>(()),
        }
    }

    /// The zones the radar's script holds (`tRegions` of the
    /// `MinimapFlash` widget's object), with their outlines from the
    /// world's regions.
    fn zones(&self, names: &BTreeMap<u32, String>) -> Vec<Zone> {
        let Some((&flash, _)) = names.iter().find(|(_, n)| *n == "MinimapFlash") else {
            return Vec::new();
        };
        let held: Vec<(Value, String, f64, bool)> = self
            .mission
            .lua()
            .load(RADAR_ZONES)
            .set_name("=m2-hud zones")
            .call::<Vec<Table>>(flash)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|t| {
                Some((
                    t.get("uGuid").ok()?,
                    t.get("sColor").ok()?,
                    t.get("nAlpha").ok()?,
                    t.get("bInvert").unwrap_or(false),
                ))
            })
            .collect();
        let world = self.mission.world().borrow();
        held.into_iter()
            .filter_map(|(guid, colour, alpha, invert)| {
                let guid = as_guid(&guid)?;
                let region = world.regions.get(&guid)?;
                let at = world.objects.get(&guid)?;
                let rgb = u32::from_str_radix(colour.trim_start_matches("0x"), 16).ok()?;
                Some(Zone::new(
                    region,
                    at.position,
                    at.yaw,
                    rgb,
                    alpha as f32,
                    invert,
                ))
            })
            .collect()
    }

    /// The quads of one draw item; textures they use go in `want`.
    fn prims(&self, item: &DrawItem, state: &HudState, out: &mut Vec<Prim>, want: &mut Vec<u32>) {
        match item {
            DrawItem::Quad {
                texture,
                uv,
                colour,
                rect,
                rotation,
                pie,
                ..
            } => {
                let hash = name_hash(texture);
                want.push(hash);
                match pie {
                    Some(pie) => out.extend(wedge(hash, *rect, *uv, *colour, *rotation, *pie)),
                    None => out.push(quad(Some(hash), *rect, *uv, *colour, *rotation)),
                }
            }
            DrawItem::Fill { colour, rect, .. } => {
                out.push(quad(None, *rect, [0.0, 0.0, 1.0, 1.0], *colour, 0.0));
            }
            DrawItem::Text {
                font,
                spans,
                rect,
                size,
                colour,
                justification,
                ..
            } => {
                if let Some(f) = self.fonts.get(&font.to_ascii_lowercase()) {
                    text(f, spans, *rect, *size, *colour, *justification, out, want);
                }
            }
            DrawItem::Minimap {
                rect,
                player,
                rotation,
                range,
                ..
            } => {
                if let Some(player) = player {
                    let view = MapView {
                        rect: *rect,
                        player: *player,
                        heading: *rotation,
                        range: range.unwrap_or(DEFAULT_RANGE),
                    };
                    minimap(&view, state, &self.zones, &self.roads, out, want);
                }
            }
            // Not drawn yet (see the module docs).
            DrawItem::Flash { .. } | DrawItem::Movie { .. } => {}
        }
    }
}

/// The widget a draw item belongs to.
fn widget_of(item: &DrawItem) -> u32 {
    match item {
        DrawItem::Quad { widget, .. }
        | DrawItem::Fill { widget, .. }
        | DrawItem::Text { widget, .. }
        | DrawItem::Flash { widget, .. }
        | DrawItem::Minimap { widget, .. }
        | DrawItem::Movie { widget, .. } => *widget,
    }
}

/// The reticle's widgets: those named for it, and everything under them.
/// `names` are the scripts' names ([`gui_runtime::script_names`]): the
/// reticle module's layout names its root "reticle", and the stinger's and
/// laser's "Stinger Reticle" and "Laser Reticle".
fn reticle_widgets(screen: &gui_runtime::Screen, names: &BTreeMap<u32, String>) -> HashSet<u32> {
    widgets_under(screen, names, |n| {
        n.to_ascii_lowercase().contains("reticle")
    })
}

/// The widgets with a name `pick` picks, and everything under them.
/// `names` are the scripts' names ([`gui_runtime::script_names`]),
/// before the natives' own.
fn widgets_under(
    screen: &gui_runtime::Screen,
    names: &BTreeMap<u32, String>,
    pick: impl Fn(&str) -> bool,
) -> HashSet<u32> {
    let named = |id: u32| {
        let name = names
            .get(&id)
            .map(String::as_str)
            .or_else(|| screen.widget(id).map(|w| w.name.as_str()))
            .unwrap_or_default();
        pick(name)
    };
    screen
        .widgets()
        .map(|(id, _)| id)
        .filter(|&id| {
            let mut at = Some(id);
            // Up the parents (bounded, should a parent point back down).
            for _ in 0..64 {
                match at {
                    Some(a) if named(a) => return true,
                    Some(a) => at = screen.widget(a).and_then(|w| w.parent),
                    None => return false,
                }
            }
            false
        })
        .collect()
}

/// Clip, in the clip, stored, grenades, grenades stored: as
/// `GuiAmmoUpdate` takes them. Rounds picked up add to the stores; the
/// rockets count down from that and, run out, start again full. The
/// grenade launcher's are its own (`launcher`: in the clip, stored). In
/// a vehicle whose cannon runs out, its shells (`cannon`: left, carried)
/// count down in one clip with none stored.
fn ammo(
    weapon: Weapon,
    fired: Fired,
    picked: Picked,
    launcher: Option<Rounds>,
    cannon: Option<(u32, u32)>,
) -> [u32; 5] {
    let grenades = GRENADES - fired.grenades % GRENADES;
    if let Some((left, carried)) = cannon {
        return [carried, left, 0, grenades, 0];
    }
    match weapon {
        Weapon::Rifle => [
            RIFLE_CLIP,
            RIFLE_CLIP - fired.rifle % RIFLE_CLIP,
            RIFLE_STORED + picked.rifle,
            grenades,
            0,
        ],
        Weapon::Rpg => {
            let stock = RPG_STORED + picked.rockets;
            [1, 1, stock - fired.rockets % stock, grenades, 0]
        }
        Weapon::GrenadeLauncher => {
            let r = launcher.unwrap_or_default();
            [
                Weapon::GrenadeLauncher.clip(),
                r.clip,
                r.stored,
                grenades,
                0,
            ]
        }
    }
}

fn fire(lua: &Lua, event: &str, args: impl IntoLuaMulti) -> mlua::Result<()> {
    gui_runtime::fire(lua, event, args).map(|_| ())
}

fn table<const N: usize>(lua: &Lua, fields: [(&str, Value); N]) -> mlua::Result<Value> {
    let t = lua.create_table()?;
    for (k, v) in fields {
        t.raw_set(k, v)?;
    }
    Ok(Value::Table(t))
}

/// A rectangle's quad, turned `degrees` clockwise about its centre.
fn quad(texture: Option<u32>, r: [f32; 4], uv: [f32; 4], colour: [f32; 4], degrees: f32) -> Prim {
    let corners = [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]];
    let corners = if degrees == 0.0 {
        corners
    } else {
        let (s, c) = degrees.to_radians().sin_cos();
        let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
        corners.map(|[x, y]| {
            let (dx, dy) = (x - cx, y - cy);
            // y grows down the screen: this turns clockwise.
            [cx + dx * c - dy * s, cy + dx * s + dy * c]
        })
    };
    Prim {
        texture,
        corners,
        uv: [
            [uv[0], uv[1]],
            [uv[2], uv[1]],
            [uv[2], uv[3]],
            [uv[0], uv[3]],
        ],
        colour,
    }
}

/// The minimap as the scripts set it up: its circle on screen, where she
/// is, the camera's heading (the map turns with it: ahead is up) and the
/// metres to the rim.
struct MapView {
    rect: [f32; 4],
    player: [f32; 3],
    heading: f32,
    range: f32,
}

impl MapView {
    fn centre(&self) -> [f32; 2] {
        [
            (self.rect[0] + self.rect[2]) / 2.0,
            (self.rect[1] + self.rect[3]) / 2.0,
        ]
    }

    fn radius(&self) -> f32 {
        (self.rect[2] - self.rect[0]) / 2.0
    }

    /// Where world `(x, z)` shows: on screen, and whether inside the rim.
    fn screen(&self, x: f32, z: f32) -> ([f32; 2], bool) {
        let (s, c) = self.heading.to_radians().sin_cos();
        // Heading grows turning left: ahead is (sin h, cos h), right is
        // (-cos h, sin h).
        let (dx, dz) = (x - self.player[0], z - self.player[2]);
        let ahead = dx * s + dz * c;
        let right = -dx * c + dz * s;
        let k = self.radius() / self.range;
        let [cx, cy] = self.centre();
        (
            [cx + right * k, cy - ahead * k],
            ahead.hypot(right) <= self.range,
        )
    }

    /// The world `(x, z)` that shows at `angle` degrees clockwise from
    /// straight up, `along` (0..=1) of the way to the rim.
    fn world(&self, angle: f32, along: f32) -> [f32; 2] {
        let (hs, hc) = self.heading.to_radians().sin_cos();
        let (s, c) = angle.to_radians().sin_cos();
        let (ahead, right) = (c * along * self.range, s * along * self.range);
        [
            self.player[0] + ahead * hs - right * hc,
            self.player[2] + ahead * hc + right * hs,
        ]
    }
}

/// The radar's map, island-wide as [`MAP_TEXTURE`] is laid (and as many
/// pixels across): the sea where the low-resolution terrain lies under
/// `sea_level`, the land clear (see [`m2_assets::minimap`]). With it the
/// island's roads, those [`MAP_TEXTURE`] shows grey paved.
fn radar_map(
    assets: &mut AssetSource<BufReader<File>>,
    sea_level: f32,
) -> Result<(Image, Vec<Road>), String> {
    let texture = assets
        .texture(name_hash(MAP_TEXTURE), MAX_TEXTURE)
        .map_err(|e| e.to_string())?;
    let rgba = texture.mips.first().ok_or("no mips")?;
    let (width, height) = (texture.width as usize, texture.height as usize);
    let index = WorldIndex::build(assets).map_err(|e| e.to_string())?;
    let mut heights = Heights::new(width, ISLAND);
    for (&hash, centre) in &index.tiles {
        let data = assets
            .asset(hash, LOW_RES_TYPE)
            .map_err(|e| e.to_string())?;
        let tile = LowResTile::parse(&data).map_err(|e| format!("tile {hash:08x}: {e}"))?;
        heights.add_tile(&tile, centre.to_array());
    }
    let pixel = |p: [f32; 2]| {
        let [u, v] = map_uv(p);
        let (x, y) = ((u * width as f32).floor(), (v * height as f32).floor());
        (x >= 0.0 && y >= 0.0 && (x as usize) < width && (y as usize) < height).then(|| {
            let i = (y as usize * width + x as usize) * 4;
            [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
        })
    };
    let roads = road_network(assets)?
        .into_iter()
        .map(|points| {
            let paved = minimap::paved_share(&points, pixel) >= minimap::PAVED_SHARE;
            Road::new(points, paved)
        })
        .collect();
    Ok((
        Image {
            width: texture.width,
            height: width as u32,
            rgba: minimap::radar_map(&heights, sea_level),
        },
        roads,
    ))
}

/// The island's roads: each `Road` entity's centreline (see
/// [`minimap::centreline`]), from every layer but the hidden ones and the
/// missions' (`vz_hidden`, `vz_state_...`).
fn road_network(assets: &mut AssetSource<BufReader<File>>) -> Result<Vec<Vec<[f32; 2]>>, String> {
    let index = assets.archive().index();
    let mut blocks: Vec<usize> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == LAYER_TYPE)
        .map(|a| usize::from(a.block))
        .filter(|&b| {
            let path = &index.blocks()[b].path;
            !path.contains("vz_hidden") && !path.contains("vz_state_")
        })
        .collect();
    blocks.sort_unstable();
    blocks.dedup();
    // Roads name the intersections at their ends, which may be placed in
    // another layer: gather every layer's first.
    let mut placed: HashMap<u32, Placement> = HashMap::new();
    let mut records: Vec<(RoadRecord, Placement)> = Vec::new();
    for b in blocks {
        let decoded = assets.block(b).map_err(|e| e.to_string())?;
        let contents = BlockContents::parse(&decoded).map_err(|e| e.to_string())?;
        for (e, data) in contents.iter() {
            if e.type_hash != LAYER_TYPE {
                continue;
            }
            let Ok(layer) = Layer::parse(data) else {
                continue;
            };
            let places = layer.placements();
            if let Some(c) = layer.component("Road") {
                for (id, rec) in c.records().unwrap_or_default() {
                    if let (Some(r), Some(p)) = (RoadRecord::parse(rec), places.get(&id)) {
                        records.push((r, *p));
                    }
                }
            }
            placed.extend(places);
        }
    }
    let mut paths: HashMap<u32, Option<Path>> = HashMap::new();
    let mut roads = Vec::with_capacity(records.len());
    for (r, p) in records {
        let path = paths
            .entry(r.path)
            .or_insert_with(|| {
                assets
                    .asset(r.path, PATH_TYPE)
                    .ok()
                    .and_then(|d| Path::parse(&d).ok())
            })
            .as_ref();
        let Some(path) = path.filter(|p| !p.points.is_empty()) else {
            continue;
        };
        let ends = r.ends.map(|id| {
            (id != 0)
                .then(|| placed.get(&id))
                .flatten()
                .map(|q| [q.position[0], q.position[2]])
        });
        let line = minimap::centreline(&path.points, p.position, p.rotation, ends);
        roads.push(line);
    }
    Ok(roads)
}

/// The map texture's coordinates of world `(x, z)`.
fn map_uv([x, z]: [f32; 2]) -> [f32; 2] {
    [(ISLAND / 2.0 - x) / ISLAND, (ISLAND / 2.0 - z) / ISLAND]
}

/// The minimap's quads: the radar's backdrop with north on its rim, the
/// island's map round her in the circle, the zones of the radar's movie,
/// the roads over it, the objective markers (at the rim when farther) and
/// her marker.
fn minimap(
    view: &MapView,
    state: &HudState,
    zones: &[Zone],
    roads: &[Road],
    out: &mut Vec<Prim>,
    want: &mut Vec<u32>,
) {
    let [cx, cy] = view.centre();
    let r = view.radius();
    let white = [1.0; 4];
    let full = [0.0, 0.0, 1.0, 1.0];
    let border = name_hash(BORDER_TEXTURE);
    want.push(border);
    let b = r * BORDER;
    // North (+z) is `heading` degrees clockwise from ahead.
    out.push(quad(
        Some(border),
        [cx - b, cy - b, cx + b, cy + b],
        full,
        white,
        view.heading,
    ));
    let map = name_hash(MAP_TEXTURE);
    want.push(map);
    const SIDES: usize = 48;
    let rim = |i: usize| i as f32 * 360.0 / SIDES as f32;
    let centre_uv = map_uv([view.player[0], view.player[2]]);
    for i in 0..SIDES {
        let point = |a: f32| {
            let (s, c) = a.to_radians().sin_cos();
            [cx + s * r, cy - c * r]
        };
        let (a, b) = (rim(i), rim(i + 1));
        let (ua, ub) = (map_uv(view.world(a, 1.0)), map_uv(view.world(b, 1.0)));
        out.push(Prim {
            texture: Some(map),
            corners: [[cx, cy], point(a), point(b), point(b)],
            uv: [centre_uv, ua, ub, ub],
            colour: white,
        });
    }
    road_strokes(view, roads, out, want);
    // The movie's zones over the map (it is laid over the minimap, the
    // same size): its lines one of its 256 pixels across.
    let disc: Vec<[f32; 2]> = (0..SIDES)
        .map(|i| {
            let (s, c) = rim(i).to_radians().sin_cos();
            [cx + s * r, cy - c * r]
        })
        .collect();
    let line = (2.0 * r / MOVIE_SIZE).max(1.0);
    for zone in zones {
        zone.draw(view, &disc, line, out);
    }
    for (at, icon, colour) in &state.blips {
        let ([x, y], inside) = view.screen(at[0], at[2]);
        // Farther ones sit on the rim, towards them.
        let [x, y] = if inside {
            [x, y]
        } else {
            let (dx, dy) = (x - cx, y - cy);
            let k = r / dx.hypot(dy).max(1e-3);
            [cx + dx * k, cy + dy * k]
        };
        let hash = name_hash(&format!("HUD_objective_{}", icon.replace(' ', "_")));
        want.push(hash);
        let s = r * 0.16;
        let c = colour.map(|v| f32::from(v) / 255.0);
        out.push(quad(Some(hash), [x - s, y - s, x + s, y + s], full, c, 0.0));
    }
    // Her: a white square, upright whichever way she faces (the map turns
    // with the camera, not with her).
    let s = r * minimap::PLAYER_SIZE / 2.0;
    out.push(quad(
        None,
        [cx - s, cy - s, cx + s, cy + s],
        full,
        white,
        0.0,
    ));
}

/// The roads on the minimap, inside its rim: the paved solid strokes
/// [`PAVED_WIDTH`] of its radius across, the tracks [`ROAD_WIDTH`] and dashed, a [`DASH`] of
/// road and a [`DASH_GAP`] by turns, counted along each road from its
/// start.
///
/// [`PAVED_WIDTH`]: minimap::PAVED_WIDTH
/// [`ROAD_WIDTH`]: minimap::ROAD_WIDTH
/// [`DASH`]: minimap::DASH
/// [`DASH_GAP`]: minimap::DASH_GAP
fn road_strokes(view: &MapView, roads: &[Road], out: &mut Vec<Prim>, want: &mut Vec<u32>) {
    let centre = view.centre();
    let r = view.radius();
    let colour = minimap::ROAD.map(|v| f32::from(v) / 255.0);
    let texture = name_hash(PATH_TEXTURE);
    want.push(texture);
    let period = minimap::DASH + minimap::DASH_GAP;
    let at = [view.player[0], view.player[2]];
    for road in roads.iter().filter(|road| road.near(at, view.range)) {
        let width = if road.paved {
            minimap::PAVED_WIDTH
        } else {
            minimap::ROAD_WIDTH
        };
        let half = r * width / 2.0;
        // Metres along the road, for its dashes to run on unbroken.
        let mut along = 0.0;
        for pair in road.points.windows(2) {
            let ([ax, az], [bx, bz]) = (pair[0], pair[1]);
            let metres = (bx - ax).hypot(bz - az);
            let start = along;
            along += metres;
            let (a, _) = view.screen(ax, az);
            let (b, _) = view.screen(bx, bz);
            let Some((t0, t1)) = clip_to_disc(a, b, centre, r) else {
                continue;
            };
            let length = (b[0] - a[0]).hypot(b[1] - a[1]);
            if length < 1e-3 || metres < 1e-3 {
                continue;
            }
            let (nx, ny) = (
                -(b[1] - a[1]) / length * half,
                (b[0] - a[0]) / length * half,
            );
            // A stroke from `t` to `t'` of the way along, the texture
            // across from `u` to `u'`.
            let mut stroke = |(t, u): (f32, f32), (t2, u2): (f32, f32)| {
                let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                let q = [a[0] + (b[0] - a[0]) * t2, a[1] + (b[1] - a[1]) * t2];
                out.push(Prim {
                    texture: Some(texture),
                    corners: [
                        [p[0] + nx, p[1] + ny],
                        [q[0] + nx, q[1] + ny],
                        [q[0] - nx, q[1] - ny],
                        [p[0] - nx, p[1] - ny],
                    ],
                    uv: [[u, 0.0], [u2, 0.0], [u2, 1.0], [u, 1.0]],
                    colour,
                });
            };
            if road.paved {
                stroke((t0, PATH_SOLID_U), (t1, PATH_SOLID_U));
                continue;
            }
            // Each dash takes the texture's first, rounded end to end.
            let (s0, s1) = (start + metres * t0, start + metres * t1);
            let mut k = (s0 / period).floor();
            while k * period < s1 {
                let (d0, d1) = (k * period, k * period + minimap::DASH);
                let (from, to) = (d0.max(s0), d1.min(s1));
                if from < to {
                    let t = |s: f32| (s - start) / metres;
                    let u = |s: f32| (s - d0) / minimap::DASH * PATH_DASH_U;
                    stroke((t(from), u(from)), (t(to), u(to)));
                }
                k += 1.0;
            }
        }
    }
}

/// The part of segment `a b` inside the disc round `centre` of radius
/// `r`: from and to (0..=1 along it), or None if it misses.
fn clip_to_disc(a: [f32; 2], b: [f32; 2], centre: [f32; 2], r: f32) -> Option<(f32, f32)> {
    // |a + t (b - a) - centre|² = r², solved for t.
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let (fx, fy) = (a[0] - centre[0], a[1] - centre[1]);
    let qa = dx * dx + dy * dy;
    let qb = 2.0 * (fx * dx + fy * dy);
    let qc = fx * fx + fy * fy - r * r;
    if qa < 1e-12 {
        return (qc <= 0.0).then_some((0.0, 1.0));
    }
    let disc = qb * qb - 4.0 * qa * qc;
    if disc < 0.0 {
        return None;
    }
    let root = disc.sqrt();
    let t0 = ((-qb - root) / (2.0 * qa)).max(0.0);
    let t1 = ((-qb + root) / (2.0 * qa)).min(1.0);
    (t0 < t1).then_some((t0, t1))
}

/// A region on the radar, as the mission's scripts put it there
/// (`Hud.Radar:AddLineRegion`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RadarRegion {
    /// The region object's name (`BoundaryIntro`).
    pub name: String,
    /// `nRed`, `nGreen`, `nBlue` and `nAlpha` as the scripts gave them
    /// (0..=255; None takes the radar's default).
    pub colour: [Option<f64>; 4],
    /// Shaded outside the outline rather than inside.
    pub invert: bool,
}

/// The radar's movie (`minimap.gfx`) is this many pixels square.
const MOVIE_SIZE: f32 = 256.0;

/// A zone as the radar's movie draws it (`DrawZone`): the outline filled
/// with `beginFill(sColor, nAlpha)` and stroked with `lineStyle(1, sColor,
/// nAlpha * 2)` (alphas in percent; the stroke's is opaque past 100). An
/// inverted zone (`bInvert`, the world's boundary) shades what lies
/// outside the outline instead.
#[derive(Clone, Debug, PartialEq)]
struct Zone {
    /// Corners, world (x, z).
    outline: Vec<[f32; 2]>,
    fill: [f32; 4],
    line: [f32; 4],
    invert: bool,
}

impl Zone {
    /// Region `region` placed at `position` turned `yaw` radians (as
    /// [`m2_assets::Region::contains`] places it), in colour `0xRRGGBB`
    /// at `alpha` percent.
    fn new(
        region: &m2_assets::Region,
        position: [f32; 3],
        yaw: f32,
        rgb: u32,
        alpha: f32,
        invert: bool,
    ) -> Self {
        let (s, c) = yaw.sin_cos();
        let outline = region
            .corners
            .iter()
            .map(|&[x, z]| [position[0] + x * c + z * s, position[2] - x * s + z * c])
            .collect();
        let [_, red, green, blue] = rgb.to_be_bytes().map(|v| f32::from(v) / 255.0);
        Self {
            outline,
            fill: [red, green, blue, (alpha / 100.0).clamp(0.0, 1.0)],
            line: [red, green, blue, (alpha * 2.0 / 100.0).clamp(0.0, 1.0)],
            invert,
        }
    }

    /// The zone's quads on the minimap `view`, inside `disc` (its rim, a
    /// convex polygon round the centre), its stroke `line` pixels wide.
    fn draw(&self, view: &MapView, disc: &[[f32; 2]], line: f32, out: &mut Vec<Prim>) {
        let outline: Vec<[f32; 2]> = self
            .outline
            .iter()
            .map(|&[x, z]| view.screen(x, z).0)
            .collect();
        if outline.len() < 3 {
            return;
        }
        // Even-odd: an inverted zone is the disc (a little larger, so its
        // own edge is never drawn) less the outline.
        let [cx, cy] = view.centre();
        let mut contours = vec![outline.clone()];
        if self.invert {
            contours.push(
                disc.iter()
                    .map(|&[x, y]| [cx + (x - cx) * 1.01, cy + (y - cy) * 1.01])
                    .collect(),
            );
        }
        let r = view.radius();
        for trapezoid in even_odd(&contours, cy - r, cy + r) {
            fan(&clip_convex(&trapezoid, disc), self.fill, out);
        }
        let n = outline.len();
        for i in 0..n {
            if let Some((a, b)) = segment_in_circle(outline[i], outline[(i + 1) % n], [cx, cy], r) {
                out.push(stroke(a, b, line, self.line));
            }
        }
    }
}

/// The region inside `contours` (by the even-odd rule) between heights
/// `top` and `bottom`, as trapezoids (corners clockwise on the screen):
/// slabs between every corner's height (and every crossing of two edges),
/// each cut where edges cross its middle and filled between every other
/// pair.
fn even_odd(contours: &[Vec<[f32; 2]>], top: f32, bottom: f32) -> Vec<[[f32; 2]; 4]> {
    let edges: Vec<([f32; 2], [f32; 2])> = contours
        .iter()
        .flat_map(|c| (0..c.len()).map(move |i| (c[i], c[(i + 1) % c.len()])))
        .filter(|(a, b)| a[1] != b[1])
        .collect();
    let mut ys: Vec<f32> = vec![top, bottom];
    ys.extend(edges.iter().map(|(a, _)| a[1]));
    for (i, &(a, b)) in edges.iter().enumerate() {
        for &(c, d) in &edges[i + 1..] {
            if let Some(y) = crossing_height(a, b, c, d) {
                ys.push(y);
            }
        }
    }
    ys.retain(|y| (top..=bottom).contains(y));
    ys.sort_by(f32::total_cmp);
    ys.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
    let x_at =
        |(a, b): ([f32; 2], [f32; 2]), y: f32| a[0] + (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]);
    let mut out = Vec::new();
    for slab in ys.windows(2) {
        let (y0, y1) = (slab[0], slab[1]);
        let mid = (y0 + y1) / 2.0;
        let mut across: Vec<(f32, f32, f32)> = edges
            .iter()
            .filter(|(a, b)| (a[1].min(b[1])..b[1].max(a[1])).contains(&mid))
            .map(|&e| (x_at(e, mid), x_at(e, y0), x_at(e, y1)))
            .collect();
        across.sort_by(|p, q| p.0.total_cmp(&q.0));
        for pair in across.chunks_exact(2) {
            let ((_, l0, l1), (_, r0, r1)) = (pair[0], pair[1]);
            out.push([[l0, y0], [r0, y0], [r1, y1], [l1, y1]]);
        }
    }
    out
}

/// The height where segments `a b` and `c d` cross, if they do.
fn crossing_height(a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2]) -> Option<f32> {
    let (r, s) = ([b[0] - a[0], b[1] - a[1]], [d[0] - c[0], d[1] - c[1]]);
    let denom = r[0] * s[1] - r[1] * s[0];
    if denom.abs() < 1e-9 {
        return None;
    }
    let (qx, qy) = (c[0] - a[0], c[1] - a[1]);
    let t = (qx * s[1] - qy * s[0]) / denom;
    let u = (qx * r[1] - qy * r[0]) / denom;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then(|| a[1] + t * r[1])
}

/// Convex polygon `poly` cut to convex polygon `clip` (both wound the
/// same way round; Sutherland-Hodgman).
fn clip_convex(poly: &[[f32; 2]], clip: &[[f32; 2]]) -> Vec<[f32; 2]> {
    // The clip's winding: inside is on this side of each edge.
    let n = clip.len();
    let area: f32 = (0..n)
        .map(|i| {
            let (p, q) = (clip[i], clip[(i + 1) % n]);
            p[0] * q[1] - q[0] * p[1]
        })
        .sum();
    let side = area.signum();
    let mut out = poly.to_vec();
    for i in 0..n {
        if out.is_empty() {
            break;
        }
        let (p, q) = (clip[i], clip[(i + 1) % n]);
        let inside = |v: [f32; 2]| {
            ((q[0] - p[0]) * (v[1] - p[1]) - (q[1] - p[1]) * (v[0] - p[0])) * side >= 0.0
        };
        let cut = |a: [f32; 2], b: [f32; 2]| {
            let (da, db) = (
                (q[0] - p[0]) * (a[1] - p[1]) - (q[1] - p[1]) * (a[0] - p[0]),
                (q[0] - p[0]) * (b[1] - p[1]) - (q[1] - p[1]) * (b[0] - p[0]),
            );
            let t = da / (da - db);
            [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
        };
        let input = std::mem::take(&mut out);
        for j in 0..input.len() {
            let (a, b) = (input[j], input[(j + 1) % input.len()]);
            match (inside(a), inside(b)) {
                (true, true) => out.push(b),
                (true, false) => out.push(cut(a, b)),
                (false, true) => {
                    out.push(cut(a, b));
                    out.push(b);
                }
                (false, false) => {}
            }
        }
    }
    out
}

/// A convex polygon as triangles from its first corner, untextured.
fn fan(poly: &[[f32; 2]], colour: [f32; 4], out: &mut Vec<Prim>) {
    for i in 1..poly.len().saturating_sub(1) {
        let (a, b, c) = (poly[0], poly[i], poly[i + 1]);
        out.push(Prim {
            texture: None,
            corners: [a, b, c, c],
            uv: [[0.0; 2]; 4],
            colour,
        });
    }
}

/// The part of segment `a b` inside the circle round `centre`.
fn segment_in_circle(
    a: [f32; 2],
    b: [f32; 2],
    centre: [f32; 2],
    r: f32,
) -> Option<([f32; 2], [f32; 2])> {
    let (d, f) = (
        [b[0] - a[0], b[1] - a[1]],
        [a[0] - centre[0], a[1] - centre[1]],
    );
    let qa = d[0] * d[0] + d[1] * d[1];
    if qa < 1e-9 {
        return None;
    }
    let qb = 2.0 * (f[0] * d[0] + f[1] * d[1]);
    let qc = f[0] * f[0] + f[1] * f[1] - r * r;
    let disc = qb * qb - 4.0 * qa * qc;
    if disc <= 0.0 {
        return None;
    }
    let root = disc.sqrt();
    let t0 = ((-qb - root) / (2.0 * qa)).max(0.0);
    let t1 = ((-qb + root) / (2.0 * qa)).min(1.0);
    (t0 < t1).then(|| {
        let at = |t: f32| [a[0] + d[0] * t, a[1] + d[1] * t];
        (at(t0), at(t1))
    })
}

/// A line from `a` to `b`, `width` across, untextured.
fn stroke(a: [f32; 2], b: [f32; 2], width: f32, colour: [f32; 4]) -> Prim {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let k = width / 2.0 / dx.hypot(dy).max(1e-6);
    let (nx, ny) = (-dy * k, dx * k);
    Prim {
        texture: None,
        corners: [
            [a[0] + nx, a[1] + ny],
            [b[0] + nx, b[1] + ny],
            [b[0] - nx, b[1] - ny],
            [a[0] - nx, a[1] - ny],
        ],
        uv: [[0.0; 2]; 4],
        colour,
    }
}

/// The part of a rectangle's quad in the wedge from its centre between
/// angles `pie` (degrees clockwise from straight up), turned `degrees`
/// clockwise: triangles, each a prim with its last corner doubled.
fn wedge(
    texture: u32,
    r: [f32; 4],
    uv: [f32; 4],
    colour: [f32; 4],
    degrees: f32,
    pie: [f32; 2],
) -> Vec<Prim> {
    let [from, to] = pie;
    let (w, h) = (r[2] - r[0], r[3] - r[1]);
    if to <= from || w <= 0.0 || h <= 0.0 {
        return Vec::new();
    }
    let centre = [(r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0];
    // Where a ray from the centre at angle `a` leaves the rectangle.
    let edge = |a: f32| {
        let (dx, dy) = {
            let (s, c) = a.to_radians().sin_cos();
            (s, -c)
        };
        let t = (w / 2.0 / dx.abs().max(1e-6)).min(h / 2.0 / dy.abs().max(1e-6));
        [centre[0] + dx * t, centre[1] + dy * t]
    };
    // The ends, the corners between them (where the edge bends) and every
    // 15 degrees between.
    let corner = w.atan2(h).to_degrees();
    let mut angles = vec![from, to];
    for turn in [-360.0, 0.0, 360.0] {
        for c in [corner, 180.0 - corner, 180.0 + corner, 360.0 - corner] {
            if turn + c > from && turn + c < to {
                angles.push(turn + c);
            }
        }
    }
    let mut a = from + 15.0;
    while a < to {
        angles.push(a);
        a += 15.0;
    }
    angles.sort_by(f32::total_cmp);
    let uv_at = |p: [f32; 2]| {
        [
            uv[0] + (p[0] - r[0]) / w * (uv[2] - uv[0]),
            uv[1] + (p[1] - r[1]) / h * (uv[3] - uv[1]),
        ]
    };
    let (s, c) = degrees.to_radians().sin_cos();
    // y grows down the screen: this turns clockwise.
    let turned = |[x, y]: [f32; 2]| {
        let (dx, dy) = (x - centre[0], y - centre[1]);
        [centre[0] + dx * c - dy * s, centre[1] + dx * s + dy * c]
    };
    angles
        .windows(2)
        .map(|pair| {
            let (p, q) = (edge(pair[0]), edge(pair[1]));
            let points = [centre, p, q, q];
            Prim {
                texture: Some(texture),
                corners: points.map(turned),
                uv: points.map(uv_at),
                colour,
            }
        })
        .collect()
}

/// Glyph quads for coloured `spans` in box `rect`: lines from its top,
/// each justified, `size` points per font pixel.
#[allow(clippy::too_many_arguments)]
fn text(
    font: &Font,
    spans: &[TextSpan],
    rect: [f32; 4],
    size: f32,
    colour: [f32; 4],
    justify: Anchor,
    out: &mut Vec<Prim>,
    want: &mut Vec<u32>,
) {
    // Each line's parts: text, or an inline icon a line high and square;
    // and the colour.
    type Part<'a> = (&'a str, Option<&'a str>, [f32; 4]);
    let mut lines: Vec<Vec<Part>> = vec![Vec::new()];
    for s in spans {
        let c = s.colour.map_or(colour, |c| [c[0], c[1], c[2], colour[3]]);
        if let Some(icon) = &s.icon {
            if let Some(line) = lines.last_mut() {
                line.push(("", Some(icon), c));
            }
            continue;
        }
        for (i, part) in s.text.split('\n').enumerate() {
            if i > 0 {
                lines.push(Vec::new());
            }
            if let Some(line) = lines.last_mut() {
                line.push((part, None, c));
            }
        }
    }
    let line_height = f32::from(font.line_height) * size;
    for (n, line) in lines.iter().enumerate() {
        let width: f32 = line
            .iter()
            .map(|p| match p.1 {
                Some(_) => line_height,
                None => font.text_width(p.0) * size,
            })
            .sum();
        let mut x = match justify {
            Anchor::Start => rect[0],
            Anchor::Centre => (rect[0] + rect[2] - width) / 2.0,
            Anchor::End => rect[2] - width,
        };
        let top = rect[1] + n as f32 * line_height;
        for (part, icon, c) in line {
            if let Some(icon) = icon {
                let hash = name_hash(icon);
                want.push(hash);
                let r = [x, top, x + line_height, top + line_height];
                out.push(quad(Some(hash), r, [0.0, 0.0, 1.0, 1.0], *c, 0.0));
                x += line_height;
                continue;
            }
            for code in part.encode_utf16() {
                if let Some(g) = font.glyph(code)
                    && let Some(m) = font.materials.get(usize::from(g.material))
                {
                    let y = top + f32::from(g.y_offset) * size;
                    let r = [
                        x,
                        y,
                        x + f32::from(g.width) * size,
                        y + f32::from(g.height) * size,
                    ];
                    want.push(m.texture);
                    out.push(quad(Some(m.texture), r, g.uv, *c, 0.0));
                }
                x += font.advance(code) * size;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where on screen the hit arc shows, clockwise from straight ahead:
    /// `HandleUpdateEvent` turns it by direction + heading, from the top
    /// where its art points.
    fn arc(heading: f32, from: Vec3) -> f32 {
        (direction(Some(Vec3::ZERO), Some(from)) + heading).rem_euclid(360.0)
    }

    #[test]
    fn hurts_come_from_the_side_they_came_from() {
        let near = |a: f32, b: f32| {
            let d = (a - b).rem_euclid(360.0);
            !(0.01..=359.99).contains(&d)
        };
        // Looking down +z (heading 0): +x is to the left (heading grows
        // turning left, forward is (sin h, 0, cos h), right is -x).
        assert!(near(arc(0.0, Vec3::Z), 0.0));
        assert!(near(arc(0.0, -Vec3::X), 90.0), "{}", arc(0.0, -Vec3::X));
        assert!(near(arc(0.0, -Vec3::Z), 180.0));
        assert!(near(arc(0.0, Vec3::X), 270.0));
        // Turned left to look down +x, what was on the left is ahead and
        // what was ahead is on the right.
        assert!(near(arc(90.0, Vec3::X), 0.0), "{}", arc(90.0, Vec3::X));
        assert!(near(arc(90.0, Vec3::Z), 90.0), "{}", arc(90.0, Vec3::Z));
        assert_eq!(direction(None, Some(Vec3::X)), 0.0);
    }

    #[test]
    fn counters_count_down_and_refill() {
        let fired = |rifle, rockets, grenades| Fired {
            rifle,
            rockets,
            grenades,
        };
        let none = Picked::default();
        assert_eq!(ammo(Weapon::Rifle, fired(0, 0, 0), none, None, None)[1], 30);
        assert_eq!(ammo(Weapon::Rifle, fired(29, 0, 0), none, None, None)[1], 1);
        assert_eq!(
            ammo(Weapon::Rifle, fired(30, 0, 0), none, None, None)[1],
            30
        );
        assert_eq!(
            ammo(Weapon::Rpg, fired(0, 0, 0), none, None, None)[..3],
            [1, 1, RPG_STORED]
        );
        assert_eq!(
            ammo(Weapon::Rpg, fired(0, 1, 0), none, None, None)[2],
            RPG_STORED - 1
        );
        assert_eq!(
            ammo(Weapon::Rpg, fired(0, 4, 0), none, None, None)[2],
            RPG_STORED
        );
        assert_eq!(
            ammo(Weapon::Rifle, fired(0, 0, 1), none, None, None)[3],
            GRENADES - 1
        );
        // A crate's 20 rockets after three fired: 21 left.
        let crate_ = Picked {
            rockets: 20,
            ..Picked::default()
        };
        assert_eq!(
            ammo(Weapon::Rpg, fired(0, 3, 0), crate_, None, None)[2],
            RPG_STORED + 17
        );
        // The launcher's own rounds: "6 / 12" as the crate gives it.
        let launcher = Some(Rounds {
            clip: 6,
            stored: 12,
        });
        assert_eq!(
            ammo(
                Weapon::GrenadeLauncher,
                fired(0, 0, 0),
                none,
                launcher,
                None
            )[..3],
            [6, 6, 12]
        );
        // The AMX-30's 40 shells, three fired.
        assert_eq!(
            ammo(Weapon::Rifle, fired(0, 0, 0), none, None, Some((37, 40)))[..3],
            [40, 37, 0]
        );
    }

    #[test]
    fn vehicles_are_named_with_their_faction() {
        assert_eq!(
            vehicle_name("vz_veh_truck_m151"),
            ("truck m151".to_owned(), Some(0xb442_0059))
        );
        assert_eq!(
            vehicle_name("global_veh_68valiant"),
            ("68valiant".to_owned(), None)
        );
    }

    #[test]
    fn the_minimap_turns_with_the_camera() {
        // Looking down +x (heading 90): +x is up the map and +z, which
        // was ahead, is to the right.
        let view = MapView {
            rect: [0.0, 0.0, 100.0, 100.0],
            player: [10.0, 0.0, 20.0],
            heading: 90.0,
            range: 100.0,
        };
        let ([x, y], inside) = view.screen(60.0, 20.0);
        assert!(
            inside && (x - 50.0).abs() < 1e-3 && (y - 25.0).abs() < 1e-3,
            "{x} {y}"
        );
        let ([x, y], _) = view.screen(10.0, 70.0);
        assert!(
            (x - 75.0).abs() < 1e-3 && (y - 50.0).abs() < 1e-3,
            "{x} {y}"
        );
        assert!(!view.screen(500.0, 20.0).1);
        // And back: what shows a quarter turn round, halfway out.
        let [wx, wz] = view.world(90.0, 0.5);
        let ([x, y], _) = view.screen(wx, wz);
        assert!(
            (x - 75.0).abs() < 1e-3 && (y - 50.0).abs() < 1e-3,
            "{x} {y}"
        );
        // The map's corners: +x and +z at its left and top.
        assert_eq!(map_uv([ISLAND / 2.0, ISLAND / 2.0]), [0.0, 0.0]);
    }

    #[test]
    fn roads_are_cut_at_the_minimap_rim() {
        // Across the middle of a disc of radius 10 from well outside it.
        let (t0, t1) = clip_to_disc([-20.0, 0.0], [20.0, 0.0], [0.0, 0.0], 10.0).unwrap();
        assert!((t0 - 0.25).abs() < 1e-5 && (t1 - 0.75).abs() < 1e-5);
        // Wholly inside, and wholly outside.
        assert_eq!(
            clip_to_disc([-1.0, 0.0], [1.0, 0.0], [0.0, 0.0], 10.0),
            Some((0.0, 1.0))
        );
        assert_eq!(
            clip_to_disc([-20.0, 15.0], [20.0, 15.0], [0.0, 0.0], 10.0),
            None
        );
    }

    #[test]
    fn pie_slices_keep_their_wedge() {
        // A quarter from straight up to the right: the top right quarter,
        // bent at the corner.
        let tris = wedge(
            1,
            [0.0, 0.0, 2.0, 2.0],
            [0.0, 0.0, 1.0, 1.0],
            [1.0; 4],
            0.0,
            [0.0, 90.0],
        );
        assert!(!tris.is_empty());
        for t in &tris {
            for [x, y] in t.corners {
                assert!(x > 1.0 - 1e-4 && y < 1.0 + 1e-4, "{x} {y}");
            }
        }
        let near = |c: [f32; 2]| (c[0] - 2.0).abs() + c[1].abs() < 1e-4;
        assert!(tris.iter().any(|t| t.corners.iter().any(|&c| near(c))));
        // The target-health arc's 130..230 lies under the centre.
        let arc = wedge(
            1,
            [0.0, 0.0, 2.0, 2.0],
            [0.0, 0.0, 1.0, 1.0],
            [1.0; 4],
            0.0,
            [130.0, 230.0],
        );
        assert!(
            arc.iter()
                .all(|t| t.corners.iter().all(|c| c[1] > 1.0 - 1e-4))
        );
        assert!(
            wedge(
                1,
                [0.0, 0.0, 2.0, 2.0],
                [0.0; 4],
                [1.0; 4],
                0.0,
                [90.0, 90.0]
            )
            .is_empty()
        );
    }

    #[test]
    fn quads_turn_clockwise() {
        let p = quad(
            None,
            [0.0, 0.0, 2.0, 2.0],
            [0.0, 0.0, 1.0, 1.0],
            [1.0; 4],
            90.0,
        );
        // The top left corner goes to the top right.
        let [x, y] = p.corners[0];
        assert!((x - 2.0).abs() < 1e-5 && y.abs() < 1e-5, "{x} {y}");
    }

    #[test]
    fn later_states_add_their_events() {
        let mut a = HudState {
            dt: 0.1,
            hurts: vec![(10.0, 0.0)],
            hp: 90.0,
            ..HudState::default()
        };
        a.merge(HudState {
            dt: 0.2,
            hurts: vec![(5.0, 90.0)],
            messages: vec![ObjectiveMessage {
                kind: "add".into(),
                icon: 5,
                text: "hi".into(),
            }],
            hp: 85.0,
            ..HudState::default()
        });
        assert!((a.dt - 0.3).abs() < 1e-9);
        assert_eq!(a.hurts.len(), 2);
        assert_eq!(a.messages.len(), 1);
        assert_eq!(a.hp, 85.0);
    }

    /// The area of the prims drawn in `alpha` (quads; a triangle's last
    /// corner doubled).
    fn area(prims: &[Prim], alpha: f32) -> f32 {
        prims
            .iter()
            .filter(|p| p.colour[3] == alpha)
            .map(|p| {
                let c = p.corners;
                (0..4)
                    .map(|i| c[i][0] * c[(i + 1) % 4][1] - c[(i + 1) % 4][0] * c[i][1])
                    .sum::<f32>()
                    .abs()
                    / 2.0
            })
            .sum()
    }

    #[test]
    fn a_zone_shades_its_inside_or_inverted_the_rest_of_the_disc() {
        // A minimap 100 pixels across showing 50 m round her: a pixel a
        // metre. The region is 20 m square round her.
        let view = MapView {
            rect: [0.0, 0.0, 100.0, 100.0],
            player: [0.0; 3],
            heading: 0.0,
            range: 50.0,
        };
        let disc: Vec<[f32; 2]> = (0..48)
            .map(|i| {
                let (s, c) = (i as f32 * 7.5).to_radians().sin_cos();
                [50.0 + s * 50.0, 50.0 - c * 50.0]
            })
            .collect();
        let disc_area = 24.0 * 2500.0 * 7.5f32.to_radians().sin();
        let region = m2_assets::Region {
            flags: 0,
            unk_0x04: 0,
            corners: vec![[-10.0, -10.0], [10.0, -10.0], [10.0, 10.0], [-10.0, 10.0]],
        };
        // Black at 40%: filled at 0.4, stroked at 0.8.
        for (invert, want) in [(false, 400.0), (true, disc_area - 400.0)] {
            let zone = Zone::new(&region, [0.0; 3], 0.3, 0x000000, 40.0, invert);
            let mut out = Vec::new();
            zone.draw(&view, &disc, 1.0, &mut out);
            let filled = area(&out, 0.4);
            assert!(
                (filled - want).abs() < 2.0,
                "invert {invert}: {filled} not {want}"
            );
            // The stroke: four sides of 20, a pixel wide.
            assert!((area(&out, 0.8) - 80.0).abs() < 1.0);
        }
    }

    #[test]
    fn a_zone_reaching_past_the_rim_is_cut_to_it() {
        let view = MapView {
            rect: [0.0, 0.0, 100.0, 100.0],
            player: [0.0; 3],
            heading: 0.0,
            range: 50.0,
        };
        let disc: Vec<[f32; 2]> = (0..48)
            .map(|i| {
                let (s, c) = (i as f32 * 7.5).to_radians().sin_cos();
                [50.0 + s * 50.0, 50.0 - c * 50.0]
            })
            .collect();
        let disc_area = 24.0 * 2500.0 * 7.5f32.to_radians().sin();
        // Half the island, past her on one side: half the disc.
        let region = m2_assets::Region {
            flags: 0,
            unk_0x04: 0,
            corners: vec![[0.0, -500.0], [500.0, -500.0], [500.0, 500.0], [0.0, 500.0]],
        };
        let zone = Zone::new(&region, [0.0; 3], 0.0, 0x400000, 60.0, false);
        let mut out = Vec::new();
        zone.draw(&view, &disc, 1.0, &mut out);
        let filled = area(&out, 0.6);
        assert!((filled - disc_area / 2.0).abs() < 2.0, "{filled}");
        // Only the edge through her is stroked: a diameter.
        assert!((area(&out, 1.0) - 100.0).abs() < 1.0);
    }
}
