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
//! sea round her (see [`MAP_TEXTURE`]), turned with the camera that draws
//! the frame ([`HudState::heading`]), on the radar's backdrop, with the
//! island's roads over it ([`road_strokes`]), the zones its Flash movie
//! would outline (see [`Zone`]), the mission's
//! markers and her white square. Not drawn yet: the other Flash
//! pieces (pop-ups, the PDA) and clock wipes. With no aim point
//! the reticle's widgets are left out ([`reticle_widgets`], by the names
//! the layout gives them). The rifle's and the launcher's ammo is the
//! play's; the rockets' and grenades' our own count (see [`RPG_STORED`]).

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, BufWriter, Write as _};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use egui::{Color32, Mesh, TextureHandle, TextureOptions, pos2};
use m2_assets::flash::{self, avm1};
use m2_assets::font::FONT_TYPE;
use m2_assets::gfx::{MOVIE_TYPE, movie_from_asset};
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
use m2_game::gui_runtime::{self, DrawItem, FlashArg, TextSpan};
use m2_game::hud::ObjectiveMessage;
use m2_game::mission::{GameData, Mission};
use m2_math::glam::Vec3;
use mlua::{IntoLuaMulti, Lua, Table, Value};

use crate::binoculars;
use crate::combat::{Combat, Fired, Picked, Rounds, Weapon};
use crate::config::Config;
use crate::game::Game;
use crate::hero::Hero;
use crate::hijack::GuiCall;
use crate::support::{self, SupportHud};

/// The pause menu's movie (`MrxGuiPauseScreen._Initialize`'s `sFile`).
const PAUSE_MOVIE: &str = crate::pause::MOVIE;
/// The pause layout's widget (`Pause_Layout`).
const PAUSE_LAYOUT: &str = "Pause Layout";
/// The PC's library behind the pause menu's options (`LTILibName`): its
/// calls are queued for the engine ([`crate::pause::Native::library`]).
const LTI_LIBRARY: &str = r#"
__m2_lti = {}
LTILibName = setmetatable({}, {
    __index = function(_, k)
        return function(a)
            local q = __m2_lti
            q[#q + 1] = { tostring(k), a == nil and "" or tostring(a) }
        end
    end,
})
"#;
/// The calls to the PC's library the scripts queued (`__m2_lti`, as
/// [`LTI_LIBRARY`] and the shell's queue them), each its name and
/// argument, taken.
fn take_library_calls(lua: &mlua::Lua) -> Vec<(String, String)> {
    let globals = lua.globals();
    let calls: Vec<(String, String)> = globals
        .raw_get::<Option<mlua::Table>>("__m2_lti")
        .ok()
        .flatten()
        .map(|t| {
            t.sequence_values::<mlua::Table>()
                .flatten()
                .filter_map(|c| Some((c.get(1).ok()?, c.get(2).ok()?)))
                .collect()
        })
        .unwrap_or_default();
    if !calls.is_empty() {
        let _ = globals.raw_set("__m2_lti", lua.create_table().ok());
    }
    calls
}

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
/// The ammo the counters show of what the play doesn't count yet: the
/// rockets stored count down and start again full.
const RPG_STORED: u32 = 4;
/// The weapons' icons (`uNewCurrentGun`): textures of their own, as the
/// game names them (`weapons_*`).
const RIFLE_ICON: &str = "weapons_m4";
const RPG_ICON: &str = "weapons_rpg";
const LAUNCHER_ICON: &str = "weapons_grenade_launcher_2";
/// Their reticles (`uReticleTexture`), `reticle_*` by the kind of round
/// (inferred from the textures' names: the guns' `WeaponUI` records, which
/// name them, aren't read yet).
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
/// A mission's call to the HUD's interface, as the engine hands it on:
/// `HudInterface.<group>.<method>(HudInterface.<group>, data)`. Answers
/// whether the HUD has that method.
///
/// `HudInterface` is a global of `MrxGuiInterface`'s script, which runs in
/// its own environment like every module: its globals are the module's
/// members.
const HUD_CALL: &str = r#"
local group, method, data, seq, call = ...
-- The mission's callback (`fCallback`, kept in the mission's own Lua
-- state) stands here as a note of the call's number, for the game to
-- hand back ([`HudCall::callback`]).
if seq then
    data.fCallback = function()
        __m2_hud_done = __m2_hud_done or {}
        table.insert(__m2_hud_done, seq)
    end
end
local module = rawget(_G, "MrxGuiInterface")
local hud = type(module) == "table" and module.HudInterface
local g = type(hud) == "table" and hud[group]
local f = type(g) == "table" and g[method]
if type(f) ~= "function" then return false end
-- Message ids: the mission knows a message by its call's number
-- (`{ __m2_seq = n }`), the buffers by their own ids, kept here.
__m2_message_ids = __m2_message_ids or {}
local ids = type(data) == "table" and data.tMessageIds
if type(ids) == "table" and ids.__m2_seq then
    data.tMessageIds = __m2_message_ids[ids.__m2_seq] or {}
    __m2_message_ids[ids.__m2_seq] = nil
end
local r = f(g, data)
if method == "AddMessage" and call then
    __m2_message_ids[call] = r
end
return true
"#;
/// A mission's dialog box ([`HudCall`] group `DialogBox`, as the
/// mission's `DIALOG_BOXES` records it) put up with the HUD's own
/// `MrxGuiDialogBox` (its widgets, fonts and pad handling), or taken down.
/// The option picked goes into `__m2_hud_choices` with the call's number,
/// for the mission's callback ([`GameHud::take_choices`]).
const DIALOG_CALL: &str = r##"
local method, data, seq = ...
__m2_dialog_boxes = __m2_dialog_boxes or {}
local m = rawget(_G, "MrxGuiDialogBox")
if type(m) ~= "table" or type(m.DisplayDialogBox) ~= "function" then return false end
if method == "Display" then
    local rest = data.tRest or {}
    local box = m.DisplayDialogBox(Player.GetLocalPlayer(), data.sTitle, data.tOptions or {},
        data.nDefault or 1,
        function(picked)
            __m2_hud_choices = __m2_hud_choices or {}
            table.insert(__m2_hud_choices, { seq, tonumber(picked) or 0 })
            __m2_dialog_boxes[seq] = nil
        end,
        {}, rest[1], rest[2], rest[3], rest[4], rest[5], rest[6])
    __m2_dialog_boxes[seq] = box
    return box ~= nil
elseif method == "Scroll" and type(m.DisplayScrollingDialogBox) == "function" then
    local rest = data.tRest or {}
    local box = m.DisplayScrollingDialogBox(Player.GetLocalPlayer(), data.sText,
        function(picked)
            __m2_hud_choices = __m2_hud_choices or {}
            table.insert(__m2_hud_choices, { seq, tonumber(picked) or 0 })
            __m2_dialog_boxes[seq] = nil
        end,
        {}, data.tOptions or {}, rest[1], rest[2], rest[3])
    __m2_dialog_boxes[seq] = box
    return box ~= nil
elseif method == "Numeric" then
    -- The wager's digit box: the value accepted, or -1 - the value for a
    -- cancel (each callback's last argument is the value).
    local nb = rawget(_G, "MrxGuiNumericBox")
    if type(nb) ~= "table" or type(nb.DisplayNumericBox) ~= "function" then return false end
    local p, r = data.tArgs or {}, data.tRest or {}
    local function answer(v)
        __m2_hud_choices = __m2_hud_choices or {}
        table.insert(__m2_hud_choices, { seq, v })
        __m2_dialog_boxes[seq] = nil
    end
    local function last(...)
        local n = select("#", ...)
        if n == 0 then return 0 end
        return tonumber((select(n, ...))) or 0
    end
    local box = nb.DisplayNumericBox(Player.GetLocalPlayer(), p[1], p[2], p[3], p[4], p[5],
        p[6], p[7], p[8], p[9], p[10],
        function(...) answer(last(...)) end, {},
        function(...) answer(-1 - last(...)) end, {},
        r[1], r[2], r[3], r[4], r[5])
    __m2_dialog_boxes[seq] = box
    return box ~= nil
elseif method == "Close" then
    local box = __m2_dialog_boxes[data.nBox]
    __m2_dialog_boxes[data.nBox] = nil
    if type(box) == "table" and type(box.Close) == "function" then box.Close(box) end
    return true
end
return false
"##;

/// The pause menu's Medevac (`MrxGuiPauseScreen` asks
/// `MrxPlayer.CanMedEvac` to enable it, `GetMedEvacCost` for its message,
/// and calls `MrxPlayer.MedEvac` once confirmed): here the player module
/// is the mission's, in its own Lua state, so these answer as it last did
/// (`can`, `cost`) and the Medevac goes to the engine's library
/// (`M2MedEvac`, [`crate::pause::News::medevac`]) for the mission to run.
/// Set again each frame, over the module should it be loaded afresh.
const MEDEVAC: &str = r#"
local can, cost = ...
local p = rawget(_G, "MrxPlayer")
if type(p) ~= "table" then
    p = {}
    rawset(_G, "MrxPlayer", p)
end
rawset(p, "CanMedEvac", function() return can end)
rawset(p, "GetMedEvacCost", function() return cost end)
rawset(p, "MedEvac", function() LTILibName.M2MedEvac() end)
"#;

/// A pad button by the HUD's name for it (`BUTTON_PAD1_D`) as a
/// `ControllerInput` event's table, for a dialog box's `_HandleInputEvent`:
/// `ButtonPress` is `MrxGuiBase.Joystick`'s number for it. Nil when the
/// HUD's scripts don't name it.
const CONTROLLER_PRESS: &str = r#"
local name = ...
local base = rawget(_G, "MrxGuiBase")
local j = type(base) == "table" and base.Joystick
local b = type(j) == "table" and j[name]
if b == nil then return nil end
return { ButtonPress = b }
"#;

/// A call of the hijack scripts to one of the HUD's modules
/// (`MrxGuiHudActionHijack.ShowButton(player, button, ...)`), made here
/// as they made it.
const GUI_CALL: &str = r#"
local module, method, n = ...
local m = rawget(_G, module)
local f = type(m) == "table" and m[method]
if type(f) ~= "function" then return false end
f(select(4, ...))
return true
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
/// `FactionZone`'s `TrespassStateChange`, sent to the HUD's handlers.
const TRESPASS_EVENT: &str = r#"
local inside, faction = ...
MrxGui.SendEvent({ EventType = "TrespassStateChange", bTrespassing = inside,
    sFaction = faction })
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
/// A tank's cannon, while she drives one whose shells run out: the gun
/// template's own `WeaponUI` (80005fb5 "Mounted Tank Gun (Default)", the
/// AMX-30's): its icon `support_artillery` (the footage's gun box shows a
/// cannon there) and reticle `reticle_tank`, without the crosshair. The
/// template has no localized name, so the gun box names none.
const CANNON_GUID: u32 = 0x6000_0004;
/// The vehicle she drives, as the health box looks it up
/// (`Vehicle.GetFromRider` of her): it carries its template's labels, of
/// which `ArmorVehicle`, `ArmorLight`, `ArmorMedium` and `ArmorTank` pick
/// the badge `HUD_vehicle_armor_1` to `_4` (the AMX-30's "3").
const VEHICLE_GUID: u32 = 0x6000_0005;
const CANNON_ICON: &str = "support_artillery";
const CANNON_RETICLE: &str = "reticle_tank";
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
/// Her fuel, as the engine tells the HUD: how much, and of how much (her
/// capacity, `MrxPmc.GetFuelCapacity`, 300 at the start), which the fuel
/// box shows after it ("0/300"). What `MrxGuiInterface.SetFuel` does (its
/// module is not one the HUD's scripts load): each fuel box's value, then
/// "/" and the most as its appended text; not shown (the box opens with
/// the support menu, as in the footage).
const FUEL: &str = r#"
local fuel = ...
local cap = MrxPmc and MrxPmc.GetFuelCapacity and MrxPmc.GetFuelCapacity() or 300
for _, w in pairs(MrxGuiInterface._GetWidgetsForPlayers(nil, "fuel") or {}) do
    w:SetValue(fuel)
    w:SetAppendedString("/" .. cap)
end
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
/// cross. With no size the wheel is put away. Answers what it found
/// ("no overlay", "no minigame", "put away" or "posed") and the overlay's
/// and the minigame's widget ids, for the log.
const SATELLITE_WHEEL: &str = r#"
local size, angle, inside, mark = ...
local base = rawget(_G, "MrxGuiBase")
local function id_of(x)
    for id, o in pairs(base and base.WidgetIdIndex or {}) do
        if o == x then return id end
    end
end
local w = MrxGui.GetWidgetByNameAndOwner("Satellite overlay", Sys.StringToGuid("0x1"))
if not w then return "no overlay" end
local m = w.CustomData.oMinigame
if not m then return "no minigame", id_of(w) end
local c = m.CustomData
if not size or size <= 0 then
    m:SetVisible(false)
    return "put away", id_of(w), id_of(m)
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
return "posed", id_of(w), id_of(m)
"#;
/// The satellite view's help, in the tutorial box as the PC footage has
/// it (its own `Satellite Help` text stays blank): what the first
/// contract's own tutorial lines say there (`Net.SetTutorialMessage`):
/// steering the view and confirming the target (`[move] Move view`,
/// `[pfire] Confirm target`), then the PC's prompt for the timing wheel
/// (`Press [pfire] at the right time.`). For a view opened without the
/// mission's scripts; with them, their line.
const VIEW_HELP: &str = "[Tutorial.MoveCamera][n][SHELL.PCShell.Tutorial_ConfirmTarget_PC]";
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
/// The radar's movie, whose `roads_mc` holds the island's map of sea and
/// roads ([`minimap::MovieMap`]): the radar's map is drawn from it, and
/// from the terrain and the road network only without it.
const RADAR_MOVIE: &str = "minimap";
/// The radar's backdrop (its `sBorderTexture`), with the north pointer on
/// its rim, `BORDER` virtual units across round a minimap 100 across.
const BORDER_TEXTURE: &str = "global_gui_hud_radar";
const BORDER: f32 = 154.0 / 100.0;
/// What the radar strokes its dashed roads with: two dashes, white in a
/// black rim, along 32 Ã— 8 pixels.
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
    /// The pad's (or the touch controls') left stick, x right and y up,
    /// for the PDA's map.
    pub stick: [f32; 2],
    /// Seconds since the last state.
    pub dt: f64,
    /// Health, 0..=100: hers, or the vehicle's she is in.
    pub hp: f32,
    pub in_vehicle: bool,
    /// The hero she is and her health when whole: the HUD's scripts know
    /// her player by them ([`crate::mission::set_hero`]). `None` leaves
    /// them be (the HUD starts as [`GameHud::start`]'s hero).
    pub hero: Option<(Hero, f32)>,
    pub weapon: Weapon,
    /// The gun in her rifle's place, as its template names and shows it
    /// (`WeaponUI`): the pistol, the shotgun. `None`: the carbine she
    /// starts with, as the constants have it.
    pub gun: Option<(u32, String, m2_engine::weapons::GunUi)>,
    pub fired: Fired,
    /// The rifle's rounds: in the clip, stored.
    pub rifle: Rounds,
    /// Rounds the gun in her hands holds a clip (the carbine's 30, the
    /// shotgun's own).
    pub rifle_clip: u32,
    /// Rounds picked up (a supply crate's rockets).
    pub picked: Picked,
    /// The grenade launcher's rounds, while she carries one.
    pub launcher: Option<Rounds>,
    /// What she has left of the explosive she has chosen to throw
    /// (grenades, C4 or mines): the explosives box's count.
    pub grenades: u32,
    /// The label of the vehicle she is in.
    pub vehicle: Option<String>,
    /// The templates that vehicle may be, its own first: their labels
    /// pick the health box's armour badge.
    pub vehicle_templates: Vec<String>,
    /// The string table keys of the names the game shows for those
    /// templates (their `LocalizedName`s), in the same order: the vehicle
    /// box names it by the first the strings hold.
    pub vehicle_names: Vec<u32>,
    /// Its cannon's shells, when they run out: left, and how many it
    /// carries (the AMX-30's 40, counted down in the gun box).
    pub cannon: Option<(u32, u32)>,
    /// Its machine gun, while she has it chosen (the weapon switch, as
    /// the game's tank controls have it): its `WeaponUI`, the rounds left
    /// and how many it carries.
    pub machine_gun: Option<(m2_engine::weapons::GunUi, u32, u32)>,
    /// Which way the camera that draws the frame looks, degrees, growing
    /// as it turns left (`Player.GetCameraXZHeading`): hers in play, on
    /// foot or driving ([`crate::game::Game::view_heading`]). The radar's
    /// north pointer sits this far clockwise round its rim, as the PC
    /// footage's does (from the footage: north found in every eighth of
    /// the rim over the first contracts). That it is the camera's heading
    /// and not her facing is from the scripts: `MinimapDataUpdateHandler`
    /// passes the heading it is given on as the rotation; which heading
    /// the engine gives it was not read from the 360 code.
    pub heading: f32,
    /// Hurts since the last state: damage and the bearing it came from
    /// (see [`direction`]).
    pub hurts: Vec<(f32, f32)>,
    /// Messages for the message buffer since the last state.
    pub messages: Vec<ObjectiveMessage>,
    /// Who is under the reticle, as `GuiReticleUpdate` takes it: -1 an
    /// enemy, 0 anyone else; their health, and its most (their template's
    /// `Health`: a VZ soldier's 75).
    pub target: Option<(i32, i32, i32)>,
    /// Where she is (world), and which way she faces: degrees, growing
    /// turning left, as the heading.
    pub position: Option<[f32; 3]>,
    pub facing: f32,
    /// Objective markers for the minimap: where, icon
    /// (`HUD_objective_deliverable`...), colour (RGBA). Their icons are
    /// read for the world's markers too ([`GameHud::paint_texture`]).
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
    /// The mission's calls to the HUD's interface (`Hud.Fanfare:Create`...)
    /// not yet passed on.
    pub hud_calls: Vec<HudCall>,
    /// The hijack scripts' calls to the HUD's modules (the buttons to
    /// press), not yet passed on.
    pub gui_calls: Vec<GuiCall>,
    /// Keys going down (true) or up while the PDA is open, as Flash's key
    /// codes ([`flash_key`]): the PDA's movies' own `Key` listeners take
    /// them (the map's Home shows the current contract).
    pub keys: Vec<(i32, bool)>,
    /// The GUI's named buttons pressed (`"selection"`: Enter), for the
    /// scripts' `Button` events (the contract fanfare's skip).
    pub buttons: Vec<String>,
    /// The pad's buttons pressed for a dialog box the mission put up
    /// ([`crate::mission`]'s `DIALOG_BOXES`), by the HUD's names for them
    /// (`MrxGuiBase.Joystick`'s `BUTTON_PAD1_U`...): each goes to the box
    /// as a `ControllerInput` event.
    pub dialog_keys: Vec<&'static str>,
    /// What the mission's scripts answer for the pause menu's Medevac:
    /// whether she can be flown out, and its cost
    /// ([`crate::mission::MissionLink::medevac_state`]).
    pub medevac: Option<(bool, f64)>,
    /// The game was just saved: the HUD's save icon is up
    /// ([`crate::game::SAVE_ICON_SECONDS`]).
    pub saving: bool,
    /// The contract's objectives for the PDA: text id and icon tag each
    /// (`MissionView::pda_objectives`).
    pub pda_objectives: Vec<(String, String)>,
    /// Her cash, as the scripts hold it (`PlayerCashChanged` tells the HUD;
    /// the PDA's stats page reads `Player.GetCash`).
    pub cash: Option<f64>,
    /// What the faction zones last told the HUD (`MissionView::trespass`).
    pub trespass: crate::mission::Trespass,
    /// Whether vehicle disguises work, and her disguise while one holds:
    /// how much is left (0..=100) and whether it holds.
    pub vehicle_disguise: bool,
    pub disguise: Option<(f32, bool)>,
    /// The keys bound to the texts' control words (`("support", "T")`,
    /// [`crate::input::token_keys`]), for those the fonts have no picture
    /// for ([`gui_runtime::Screen::set_key_names`]).
    pub key_names: Vec<(String, String)>,
    /// An Xbox pad plays ([`crate::pad`]): the texts' control words show
    /// its pictures and the hijack's buttons its Xbox ones
    /// ([`gui_runtime::Screen::set_pad`]).
    pub pad: bool,
    /// The pause menu is wanted open (Escape, the pad's Start): the
    /// engine's `Pause` game state, which the pause layout's
    /// `MrxGuiPauseScreen` opens its movie for.
    pub pause: bool,
    /// The player's settings and saves, for the pause menu's movie
    /// ([`crate::pause`]).
    pub pause_settings: crate::pause::Settings,
    /// Keys and mouse buttons pressed while the pause menu's keyboard
    /// table waited for one ([`crate::pause::News::capture`]).
    pub pause_captured: Vec<crate::input::Control>,
    /// The mouse pointer (screen points), while the menus show it, and the
    /// left button going down (true) and up since the last frame: for the
    /// menu movies' buttons (the front end's, the pause menu's).
    pub pointer: Option<[f32; 2]>,
    pub clicks: Vec<bool>,
    /// Her binoculars are up: what their screen shows
    /// ([`crate::binoculars`]).
    pub binoculars: Option<binoculars::View>,
}

/// The pointer (`at`, screen points) and the left button's presses (true)
/// and releases among `events`, for the menu movies
/// ([`HudState::pointer`]).
pub fn pointer_input(
    events: &[egui::Event],
    at: Option<egui::Pos2>,
) -> (Option<[f32; 2]>, Vec<bool>) {
    let clicks = events
        .iter()
        .filter_map(|e| match e {
            egui::Event::PointerButton {
                button: egui::PointerButton::Primary,
                pressed,
                ..
            } => Some(*pressed),
            _ => None,
        })
        .collect();
    (at.map(|p| [p.x, p.y]), clicks)
}

/// Flash's key code (the PC's virtual-key code) of `key`, for the keys
/// the PDA's movies listen for and their neighbours.
pub fn flash_key(key: egui::Key) -> Option<i32> {
    use egui::Key::*;
    let name = key.name();
    if name.len() == 1 && name.as_bytes()[0].is_ascii_alphanumeric() {
        return Some(i32::from(name.as_bytes()[0].to_ascii_uppercase()));
    }
    Some(match key {
        Backspace => 8,
        Tab => 9,
        Enter => 13,
        Escape => 27,
        Space => 32,
        PageUp => 33,
        PageDown => 34,
        End => 35,
        Home => 36,
        ArrowLeft => 37,
        ArrowUp => 38,
        ArrowRight => 39,
        ArrowDown => 40,
        Insert => 45,
        Delete => 46,
        OpenBracket => 219,
        CloseBracket => 221,
        _ => return None,
    })
}

/// A value passed between the mission's scripts and the HUD's, which run
/// in Lua states of their own: what a table of plain values holds, and
/// guids (functions and other userdata are left out). The states agree
/// on guids: the player is 1 in both.
#[derive(Clone, Debug, PartialEq)]
pub enum Plain {
    Nil,
    Bool(bool),
    Number(f64),
    Text(String),
    Guid(u32),
    Table(Vec<(Plain, Plain)>),
}

impl Plain {
    /// Tables this deep are copied; deeper ones are left out.
    const DEPTH: usize = 4;

    pub fn of(v: &Value) -> Self {
        Self::at(v, 0)
    }

    fn at(v: &Value, depth: usize) -> Self {
        match v {
            Value::Boolean(b) => Plain::Bool(*b),
            Value::Integer(n) => Plain::Number(*n as f64),
            Value::Number(n) => Plain::Number(*n),
            Value::String(s) => Plain::Text(s.to_string_lossy()),
            Value::LightUserData(_) => as_guid(v).map_or(Plain::Nil, Plain::Guid),
            Value::Table(t) if depth < Self::DEPTH => Plain::Table(
                t.pairs::<Value, Value>()
                    .flatten()
                    .map(|(k, v)| (Self::at(&k, depth + 1), Self::at(&v, depth + 1)))
                    .filter(|(k, v)| *k != Plain::Nil && *v != Plain::Nil)
                    .collect(),
            ),
            _ => Plain::Nil,
        }
    }

    pub fn to_lua(&self, lua: &Lua) -> mlua::Result<Value> {
        Ok(match self {
            Plain::Nil => Value::Nil,
            Plain::Bool(b) => Value::Boolean(*b),
            // Whole numbers as integers, as the scripts wrote them.
            Plain::Number(n) if n.fract() == 0.0 && n.abs() < f64::from(i32::MAX) => {
                Value::Integer(*n as mlua::Integer)
            }
            Plain::Number(n) => Value::Number(*n),
            Plain::Text(t) => Value::String(lua.create_string(t)?),
            Plain::Guid(g) => guid_value(*g),
            Plain::Table(pairs) => {
                let t = lua.create_table()?;
                for (k, v) in pairs {
                    t.raw_set(k.to_lua(lua)?, v.to_lua(lua)?)?;
                }
                Value::Table(t)
            }
        })
    }
}

/// A call the mission's scripts made to the HUD through the engine's `Hud`
/// library (`Hud.Fanfare:Create({ sType = "contract", ... })`), which the
/// engine hands to the HUD's `HudInterface.<group>.<method>`.
#[derive(Clone, Debug, PartialEq)]
pub struct HudCall {
    /// Counts the calls from the mission's start, to pass each on once.
    pub seq: u64,
    pub group: String,
    pub method: String,
    pub data: Plain,
    /// The call carried a callback (`fCallback`, as the contract's
    /// fanfare does, which the flow waits on); the HUD hands its number
    /// back when it calls it ([`GameHud::take_callbacks`]).
    pub callback: bool,
}

/// The reticle's kind and health for a person under it: soldiers who have
/// turned on her are enemies (-1), anyone else 0; their health of the most
/// they started with (a soldier's template's, a civilian's 100), which the
/// arc shows a share of.
fn target_health(n: &crate::npc::Npc) -> (i32, i32, i32) {
    let enemy = n.soldier && n.alert;
    let most = n.most_hp.round().max(1.0);
    (
        -i32::from(enemy),
        n.hp.round().clamp(0.0, most) as i32,
        most as i32,
    )
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
        // On foot, the person a shot fired now would hit.
        let people = &game.scene.people;
        let target = player
            .filter(|p| p.driving.is_none())
            .zip(game.scene.world.as_ref())
            .and_then(|(p, w)| Combat::aim_probe(&w.physics, people, p))
            .and_then(|(_, person)| people.npcs.get(person?))
            .filter(|n| n.alive)
            .map(target_health);
        let hurts = game
            .combat
            .take_hurts()
            .into_iter()
            .map(|(damage, from)| (damage, direction(at, from)))
            .collect();
        Self {
            hp: vehicle.map_or(game.combat.hp_percent(), |v| v.health()),
            in_vehicle: vehicle.is_some(),
            weapon: game.combat.weapon,
            gun: {
                let r = &game.combat.rifle;
                r.template.zip(r.ui).map(|(t, ui)| (t, r.name.clone(), ui))
            },
            fired: game.combat.fired,
            picked: game.combat.picked,
            launcher: game.combat.launcher_rounds,
            rifle: game.combat.rifle_rounds,
            rifle_clip: game.combat.rifle.clip,
            grenades: game.combat.explosive_left().1,
            vehicle: vehicle.map(|v| v.spec.label.clone()),
            vehicle_templates: vehicle
                .zip(game.scene.world.as_ref())
                .map(|(v, w)| w.index.template_db.names_for(&v.spec.label, v.spec.model))
                .unwrap_or_default(),
            vehicle_names: vehicle
                .zip(game.scene.world.as_ref())
                .map(|(v, w)| {
                    w.index
                        .template_db
                        .localized_keys(&v.spec.label, v.spec.model)
                })
                .unwrap_or_default(),
            cannon: vehicle.and_then(|v| Some((v.rounds()?, v.spec.tuning.gun?.rounds))),
            machine_gun: vehicle
                .filter(|_| game.combat.vehicle_gun == 1)
                .and_then(|v| v.machine_gun())
                .and_then(|(w, left)| Some((w.ui?, left, w.clip))),
            heading: game.view_heading().to_degrees(),
            target,
            position: at.map(|p| p.to_array()),
            facing: player.map_or(0.0, |p| p.facing.to_degrees()),
            hurts,
            sea_level: game.scene.world.as_ref().map(|w| w.settings.sea_level),
            saving: game.save_icon > 0.0,
            hero: Some((game.scene.hero, game.combat.max_hp)),
            binoculars: game.walking().and_then(|p| {
                p.binoculars.map(|b| binoculars::View {
                    zoom: b.zoom,
                    heading: binoculars::wrap_degrees(game.view_heading().to_degrees()),
                    pitch: p.pitch.to_degrees(),
                })
            }),
            ..Self::default()
        }
    }

    /// Folds a later state into this one: events add up, the rest is the
    /// later's.
    fn merge(&mut self, later: Self) {
        let dt = self.dt + later.dt;
        let mut hurts = std::mem::take(&mut self.hurts);
        let mut messages = std::mem::take(&mut self.messages);
        let mut hud_calls = std::mem::take(&mut self.hud_calls);
        let mut gui_calls = std::mem::take(&mut self.gui_calls);
        gui_calls.extend(later.gui_calls.iter().cloned());
        let mut keys = std::mem::take(&mut self.keys);
        keys.extend(later.keys.iter().copied());
        let mut buttons = std::mem::take(&mut self.buttons);
        buttons.extend(later.buttons.iter().cloned());
        let mut dialog_keys = std::mem::take(&mut self.dialog_keys);
        dialog_keys.extend(later.dialog_keys.iter().copied());
        let mut pause_captured = std::mem::take(&mut self.pause_captured);
        pause_captured.extend(later.pause_captured.iter().copied());
        let mut clicks = std::mem::take(&mut self.clicks);
        clicks.extend(later.clicks.iter().copied());
        hurts.extend(later.hurts.iter().copied());
        messages.extend(later.messages.iter().cloned());
        hud_calls.extend(later.hud_calls.iter().cloned());
        *self = Self {
            dt,
            hurts,
            messages,
            hud_calls,
            gui_calls,
            keys,
            buttons,
            dialog_keys,
            pause_captured,
            clicks,
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

/// What the vehicle box shows for a vehicle label: its name (where none
/// of its templates' shown names is found, [`HudState::vehicle_names`]),
/// and its faction as the game's guid for it (`SetVehicleName(name,
/// faction)` picks the faction's icon; none shows the PMC's). Labels
/// start with the faction (`"vz_veh_truck_m151"` is the VZ's `"truck
/// m151"`).
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
    /// Multiplies what is under it (Flash's multiply blend mode).
    pub multiply: bool,
}

/// RGBA8 pixels.
struct Image {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// Whether a HUD log line tells of a mission fanfare call that failed
/// (`hud call Fanfare.Create: ...`, as the HUD thread logs its calls).
fn fanfare_failed(line: &str) -> bool {
    line.strip_prefix("hud call Fanfare.")
        .is_some_and(|rest| rest.contains(": "))
}

/// Whether a HUD log line tells of its Lua state out of stack (mlua's
/// "out of Lua stack"), after which every call into its scripts fails. A
/// script's own error, even a runaway one ("stack overflow"), unwinds and
/// leaves the state as it was.
fn broken_lua(line: &str) -> bool {
    line.contains("out of Lua stack")
}

/// A widget texture's name hash: `#` and eight hex digits name a texture
/// by its hash itself, as the gun box's and the reticle's are (the guns'
/// `WeaponUI` records name them by hash only); any other, its name's.
fn texture_hash(name: &str) -> u32 {
    name.strip_prefix('#')
        .filter(|h| h.len() == 8)
        .and_then(|h| u32::from_str_radix(h, 16).ok())
        .unwrap_or_else(|| name_hash(name))
}

/// A texture named by its hash, for [`texture_hash`].
fn by_hash(hash: u32) -> String {
    format!("#{hash:08x}")
}

/// Whether a movie's `fscommand` plays or stops an interface sound.
fn is_sound_command(command: &str) -> bool {
    matches!(command, "Sound.Play" | "Sound.Loop" | "Sound.Stop")
}

/// What the thread sends back.
enum Output {
    Frame {
        prims: Vec<Prim>,
        images: Vec<(u32, Option<Image>)>,
        log: Vec<String>,
        /// The mission calls whose callbacks the HUD called.
        callbacks: Vec<u64>,
        /// The mission's dialog boxes closed, each with the option picked.
        choices: Vec<(u64, i64)>,
        /// The front end's news (nothing for the HUD).
        shell: Box<ShellNews>,
        /// The pause menu's (nothing for the front end).
        pause: Box<PauseOut>,
    },
    Failed(String),
}

/// What the pause menu ([`crate::pause`]) asks of play, and whether the
/// game's own is there to open.
#[derive(Clone, Debug, Default, PartialEq)]
struct PauseOut {
    news: crate::pause::News,
    available: bool,
}

/// What the front end ([`GameHud::start_shell`]) asks of the game.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShellNews {
    /// The backdrop movie its movie widget plays (`shell_mainmenu_jennifer`)
    /// and the widget's box, screen points.
    pub movie: Option<(String, [f32; 4])>,
    /// How many of the prims come before the movie widget in the draw
    /// order, and so go under its picture ([`GameHud::paint_under`]).
    pub under: usize,
    /// A new game was started, as this character (`MrxGuiShellBootstrap.
    /// GetSelectedCharacter`: `Chris`, `Jen` or `mattias`).
    pub new_game: Option<String>,
    /// What its menus asked of the game: settings kept, a save to load
    /// or delete, leaving ([`crate::shell::Native::take_news`]).
    pub news: crate::pause::News,
    /// The press-start screen is behind ([`crate::shell::Native::started`]).
    pub started: bool,
}

/// What a [`GameHud`] thread runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// The HUD, from `vz.wad`.
    Hud,
    /// The front end's menus, from `shell.wad`.
    Shell,
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
    /// Their pixels, for the triangles drawn multiplying
    /// ([`crate::multiply`]).
    pixels: HashMap<u32, Arc<crate::multiply::Pixels>>,
    /// Leave out solid prims over most of the screen (fades): for
    /// pictures with and without them (`M2_PAINT_AB`).
    pub skip_covers: bool,
    /// The last objective message seen, to tell new ones.
    last_message: Option<ObjectiveMessage>,
    /// The last of the mission's HUD calls passed on.
    last_call: u64,
    /// The time of the last frame (egui's input time).
    last_time: Option<f64>,
    /// Mission calls whose callbacks the HUD has called, to hand back.
    callbacks: Vec<u64>,
    /// The mission's dialog boxes closed and the options picked, to hand
    /// back.
    choices: Vec<(u64, i64)>,
    /// A mission fanfare call (`Hud.Fanfare`) failed in the HUD's scripts:
    /// it can't show, nor its Enter close it.
    dropped_fanfare: bool,
    /// The front end's latest news (see [`GameHud::start_shell`]).
    shell: ShellNews,
    /// What the game's pause menu asked of play, not yet taken, and
    /// whether the HUD's scripts have it ([`crate::pause`]).
    pause_news: crate::pause::News,
    pause_available: bool,
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
    /// Starts the HUD's scripts reading `install` in the background, their
    /// player `hero` whole at `max` health (play's, as the game starts;
    /// a later pick comes with [`HudState::hero`]).
    pub fn start(install: &Install, hero: Hero, max: f32) -> Self {
        Self::start_mode(install, Mode::Hud, Some((hero, max)), "m2-alpha-hud.log")
    }

    /// Starts the front end instead: the shell's own scripts and Flash
    /// menus (`MrxGuiShell` and `shell.gfx`), as the game opens them after
    /// its boot movies, drawn like the HUD. What they ask of the game
    /// comes back as [`ShellNews`].
    pub fn start_shell(install: &Install) -> Self {
        Self::start_mode(install, Mode::Shell, None, "m2-alpha-shell.log")
    }

    fn start_mode(install: &Install, mode: Mode, hero: Option<(Hero, f32)>, log: &str) -> Self {
        let (tx, thread_rx) = channel();
        let (thread_tx, rx) = channel();
        let data = install.data.clone();
        std::thread::Builder::new()
            .name(
                if mode == Mode::Shell {
                    "front end"
                } else {
                    "game hud"
                }
                .into(),
            )
            .spawn(move || run(&data, mode, hero, thread_rx, thread_tx))
            .expect("could not start the HUD thread");
        let path = Config::file_beside(log);
        let log = File::create(&path).ok().map(BufWriter::new);
        Self {
            tx,
            rx,
            status: Status::Loading,
            callbacks: Vec::new(),
            choices: Vec::new(),
            dropped_fanfare: false,
            prims: Vec::new(),
            pending: Vec::new(),
            textures: HashMap::new(),
            pixels: HashMap::new(),
            skip_covers: false,
            last_message: None,
            last_call: 0,
            last_time: None,
            shell: ShellNews::default(),
            pause_news: crate::pause::News::default(),
            pause_available: false,
            log,
        }
    }

    /// The game's own pause menu can open: its scripts run and have its
    /// layout ([`HudState::pause`]).
    pub fn pause_available(&self) -> bool {
        self.ready() && self.pause_available
    }

    /// What the game's pause menu asked of play since the last time.
    pub fn take_pause_news(&mut self) -> crate::pause::News {
        std::mem::take(&mut self.pause_news)
    }

    /// The front end's latest news.
    pub fn shell(&self) -> &ShellNews {
        &self.shell
    }

    /// The character a new game was started as in the front end, once.
    pub fn take_new_game(&mut self) -> Option<String> {
        self.shell.new_game.take()
    }

    /// What the front end's menus asked of the game since last taken.
    pub fn take_shell_news(&mut self) -> crate::pause::News {
        std::mem::take(&mut self.shell.news)
    }

    /// Logs to `path` instead of `m2-alpha-hud.log` (a second HUD in one
    /// run, kept apart from the first's).
    pub fn log_to(&mut self, path: &std::path::Path) {
        self.log = File::create(path).ok().map(BufWriter::new);
    }

    /// The scripts are running and have drawn.
    pub fn ready(&self) -> bool {
        matches!(self.status, Status::Ready)
    }

    /// The scripts could not start, or broke: the old HUD stands in.
    pub fn failed(&self) -> bool {
        matches!(self.status, Status::Failed)
    }

    /// A mission fanfare call failed in the HUD's scripts (the log says
    /// which): the contract's fanfare isn't up for Enter to close.
    pub fn dropped_fanfare(&self) -> bool {
        self.dropped_fanfare
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

    /// The mission calls ([`HudCall::seq`]) whose callbacks the HUD has
    /// called since the last time: the mission runs its own.
    pub fn take_callbacks(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.callbacks)
    }

    /// The mission's dialog boxes ([`HudCall::seq`]) the player closed on
    /// an option, each with the option, since the last call.
    pub fn take_choices(&mut self) -> Vec<(u64, i64)> {
        std::mem::take(&mut self.choices)
    }

    /// The mission's HUD calls in `calls` not passed on yet.
    pub fn hud_calls(&mut self, calls: &[HudCall]) -> Vec<HudCall> {
        let new: Vec<HudCall> = calls
            .iter()
            .filter(|c| c.seq > self.last_call)
            .cloned()
            .collect();
        if let Some(last) = new.iter().map(|c| c.seq).max() {
            self.last_call = last;
        }
        new
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
            Output::Frame {
                prims,
                images,
                log,
                callbacks,
                choices,
                shell,
                pause,
            } => {
                self.callbacks.extend(callbacks);
                self.choices.extend(choices);
                let (pause, shell) = (*pause, *shell);
                self.pause_news.merge(pause.news);
                self.pause_available = pause.available;
                // Handed over already (a frame queued before it in this
                // drain broke): a later frame does not take it back.
                if self.failed() {
                    self.write(log);
                    return;
                }
                let new_game = self.shell.new_game.take().or(shell.new_game);
                let mut news = std::mem::take(&mut self.shell.news);
                news.merge(shell.news);
                self.shell = ShellNews {
                    new_game,
                    news,
                    ..shell
                };
                self.dropped_fanfare |= log.iter().any(|l| fanfare_failed(l));
                let broken = log.iter().find(|l| broken_lua(l)).cloned();
                self.write(log);
                if let Some(line) = broken {
                    // Every call into its scripts fails from here on: the
                    // stand-in HUD shows the objectives, the support menu,
                    // the PDA and the satellite's view instead.
                    self.fail(format!(
                        "the HUD's scripts broke ({line}); the stand-in HUD takes over"
                    ));
                    return;
                }
                self.status = Status::Ready;
                self.prims = prims;
                self.pending.extend(images);
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
            if let Some(i) = &image {
                self.pixels.insert(
                    hash,
                    Arc::new(crate::multiply::Pixels {
                        width: i.width,
                        height: i.height,
                        rgba: i.rgba.clone(),
                    }),
                );
            }
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

    /// Paints the HUD as last drawn; solid prims (fills, fades) from
    /// egui's white texel. (As egui shapes, the PDA's thousands of small
    /// triangles smeared: their feathering assumes one winding.)
    pub fn paint(&self, painter: &egui::Painter) {
        self.paint_prims(painter, &self.prims);
    }

    /// Paints the HUD's texture `name` over `rect` (screen points), tinted
    /// by `colour`, once the HUD has read it (the objective markers'
    /// icons, read with the minimap's blips: [`HudState::blips`]). False
    /// until then, or if the archive has no such texture.
    pub fn paint_texture(
        &self,
        painter: &egui::Painter,
        name: &str,
        rect: egui::Rect,
        colour: Color32,
    ) -> bool {
        let Some(Some(texture)) = self.textures.get(&texture_hash(name)) else {
            return false;
        };
        let uv = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        painter.image(texture.id(), rect, uv, colour);
        true
    }

    /// Paints what the shell draws under its movie widget (the widgets
    /// before it, such as the screen's black), for the movie's picture to
    /// go over; [`paint_over`](Self::paint_over) paints the rest.
    pub fn paint_under(&self, painter: &egui::Painter) {
        let at = self.shell.under.min(self.prims.len());
        self.paint_prims(painter, &self.prims[..at]);
    }

    /// Paints what the shell draws over its movie widget (its menus).
    pub fn paint_over(&self, painter: &egui::Painter) {
        let at = self.shell.under.min(self.prims.len());
        self.paint_prims(painter, &self.prims[at..]);
    }

    fn paint_prims(&self, painter: &egui::Painter, prims: &[Prim]) {
        let mut mesh = Mesh::default();
        // Multiplying triangles in a row with one texture, drawn together.
        let mut multiply: Option<(u64, Vec<crate::multiply::Vertex>)> = None;
        let clip = painter.clip_rect();
        let flush_multiply = |m: &mut Option<(u64, Vec<crate::multiply::Vertex>)>| {
            if let Some((key, vertices)) = m.take() {
                let pixels = match self.pixels.get(&(key as u32)) {
                    Some(p) if key <= u64::from(u32::MAX) => Arc::clone(p),
                    _ => Arc::new(crate::multiply::Pixels {
                        width: 1,
                        height: 1,
                        rgba: vec![255; 4],
                    }),
                };
                painter.add(egui_wgpu::Callback::new_paint_callback(
                    clip,
                    crate::multiply::Multiply::new((key, pixels), vertices),
                ));
            }
        };
        // `M2_NO_MULTIPLY`: leave the multiplying triangles out, to see
        // what they do.
        let no_multiply = std::env::var_os("M2_NO_MULTIPLY").is_some();
        for p in prims {
            if p.multiply {
                if no_multiply {
                    continue;
                }
                if p.texture.is_some_and(|h| !self.pixels.contains_key(&h)) {
                    continue;
                }
                // Solid fills multiply by a white texel.
                let key = p.texture.map_or(u64::MAX, u64::from);
                if multiply.as_ref().is_some_and(|(k, _)| *k != key) {
                    flush_multiply(&mut multiply);
                }
                if !mesh.is_empty() {
                    painter.add(std::mem::take(&mut mesh));
                }
                let (_, vertices) = multiply.get_or_insert_with(|| (key, Vec::new()));
                let ndc = |c: [f32; 2]| {
                    [
                        (c[0] - clip.min.x) / clip.width().max(1.0) * 2.0 - 1.0,
                        1.0 - (c[1] - clip.min.y) / clip.height().max(1.0) * 2.0,
                    ]
                };
                for i in [0, 1, 2, 0, 2, 3] {
                    vertices.push(crate::multiply::Vertex {
                        pos: ndc(p.corners[i]),
                        uv: if p.texture.is_some() {
                            p.uv[i]
                        } else {
                            [0.5; 2]
                        },
                        colour: p.colour.map(|c| c.clamp(0.0, 1.0)),
                    });
                }
                continue;
            }
            flush_multiply(&mut multiply);
            let texture = match p.texture {
                Some(hash) => match self.textures.get(&hash) {
                    Some(Some(t)) => t.id(),
                    // Missing, or not uploaded yet.
                    _ => continue,
                },
                None if self.skip_covers && covers(p, painter.clip_rect()) => continue,
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
        flush_multiply(&mut multiply);
        if !mesh.is_empty() {
            painter.add(mesh);
        }
    }
}

/// The contract under way in the PDA, once its widget is up: its details
/// as `MrxMissionFlow.AddPdaMissionDetails` gives them (what the
/// contract's `RefreshPdaDisplay` calls), then selected as the contract's
/// task does when it starts (script 2f0b1589, `Activated`).
const CONTRACT_IN_PDA: &str = r#"
-- The objectives as `Net.SendEvent_AddPDAMission` has them: { text id,
-- icon tag } each, which `BuildMissionDescription` lists under the
-- objectives header.
local objectives = ...
local flow = rawget(_G, "MrxMissionFlow")
if type(flow) == "table" and type(flow.AddPdaMissionDetails) == "function" then
    flow.AddPdaMissionDetails("VzaCon001", objectives)
end
local pda = rawget(_G, "Pda")
if type(pda) == "table" and type(pda.Map) == "table" then
    pda.Map:SetSelectedMission({ sName = "VzaCon001" })
    -- On a contract the PDA's tracking is fixed: `MrxPlayState.Set`
    -- (script bef35c31) tells it so for any state but free play.
    pda.Map:SetMissionChangeAllowed({ bAllow = false })
end
"#;

/// The PDA map's left stick (x right, y up, -1..=1), as the game hands
/// the map widget its `ControllerInput` event (`MrxGuiPda._HandleInput`,
/// script 97f9c4ee: in map mode it passes `LeftAnalogX`/`Y` to the map
/// movie's `leftAnalog`, which moves the map under its cursor). Inferred
/// from the alpha's renders: the movie moves the map's picture by the pair
/// as given, so the view's way across is the stick's reversed. Answers
/// whether the PDA's map took it.
const PDA_STICK: &str = r#"
local x, y = ...
local module = rawget(_G, "MrxGuiPda")
if type(module) ~= "table" or type(module._HandleInput) ~= "function" then
    return false
end
-- The PDA's widget: the one whose CustomData holds the map's movie
-- (`oMapFlash`), found from the globals once.
local widget = rawget(_G, "__m2_pda_widget")
if widget == nil then
    local seen = {}
    local function find(t, depth)
        if depth > 6 or seen[t] then return nil end
        seen[t] = true
        local cd = rawget(t, "CustomData")
        if type(cd) == "table" and rawget(cd, "oMapFlash") ~= nil then
            return t
        end
        for _, v in pairs(t) do
            if type(v) == "table" then
                local w = find(v, depth + 1)
                if w then return w end
            end
        end
        return nil
    end
    widget = find(_G, 0)
    if widget == nil then return false end
    rawset(_G, "__m2_pda_widget", widget)
end
-- It opens on its map page; the page change that says so
-- (`_HandlePageChangeEvent`) comes only when the page is changed.
if widget.CustomData.bMapMode == nil then
    widget.CustomData.bMapMode = true
end
-- The movie moves the map's picture by the pair (right, down): up the
-- screen is north, and the view goes against x, so x is reversed.
module._HandleInput(widget, { LeftAnalogX = -x, LeftAnalogY = y })
return true
"#;

/// The keys that move the PDA's map, as Flash key codes, and the way
/// each pushes the stick (x right, y up): W A S D and the arrows.
/// Invented: the PC game's PDA moves its cursor with these (the owner's
/// play; the PC footage shows the cursor moving, not the keys).
const PDA_STICK_KEYS: [(i32, [f32; 2]); 8] = [
    (87, [0.0, 1.0]),
    (65, [-1.0, 0.0]),
    (83, [0.0, -1.0]),
    (68, [1.0, 0.0]),
    (38, [0.0, 1.0]),
    (37, [-1.0, 0.0]),
    (40, [0.0, -1.0]),
    (39, [1.0, 0.0]),
];

/// How much faster than the stick's own push the PDA's map moves under it.
/// Invented: the map crawled at the push as given (playtest on the phone).
const PDA_PAN: f32 = 2.5;

/// The stick the PDA's movement keys `held` (Flash key codes) push.
fn pda_stick(held: &HashSet<i32>) -> [f32; 2] {
    let mut stick = [0.0f32; 2];
    for (code, [x, y]) in PDA_STICK_KEYS {
        if held.contains(&code) {
            stick[0] += x;
            stick[1] += y;
        }
    }
    stick.map(|v| v.clamp(-1.0, 1.0))
}

/// Puts the contract in the PDA ([`CONTRACT_IN_PDA`]) with `objectives`.
fn contract_in_pda(lua: &Lua, objectives: &[(String, String)]) -> mlua::Result<()> {
    let list = lua.create_table()?;
    for (text, icon) in objectives {
        list.raw_push(lua.create_sequence_from([text.as_str(), icon.as_str()])?)?;
    }
    lua.load(CONTRACT_IN_PDA)
        .set_name("=m2-hud contract")
        .call(list)
}

/// What `MrxGuiPda.Open` checks, for the log: the PDA module's plain
/// values (and those of its tables one level down), MrxState's lock and
/// the control holder.
fn pda_probe(lua: &Lua) -> String {
    const PROBE: &str = r#"
        local out = {}
        local function scalars(prefix, t)
            for k, v in pairs(t) do
                local ty = type(v)
                if ty == "boolean" or ty == "number" or ty == "string" then
                    out[#out + 1] = prefix .. tostring(k) .. "=" .. tostring(v)
                end
            end
        end
        -- The PDA widget objects: those whose CustomData has bHaveFlash
        -- (Open's checks live there), found from the globals; each one's
        -- values, and its Open/Close print when called.
        local seen, found = {}, {}
        local function find(t, depth)
            if depth > 6 or seen[t] then return end
            seen[t] = true
            local cd = rawget(t, "CustomData")
            if type(cd) == "table" and rawget(cd, "bHaveFlash") ~= nil then
                found[#found + 1] = t
            end
            for _, v in pairs(t) do
                if type(v) == "table" then find(v, depth + 1) end
            end
        end
        find(_G, 0)
        out[#out + 1] = #found .. " PDA widgets"
        for i, w in ipairs(found) do
            local d = {}
            for k, v in pairs(w.CustomData) do
                local ty = type(v)
                if ty == "boolean" or ty == "number" then
                    d[#d + 1] = tostring(k) .. "=" .. tostring(v)
                end
            end
            table.sort(d)
            local n = 0
            if type(w.CustomData.tMissions) == "table" then
                for _ in pairs(w.CustomData.tMissions) do n = n + 1 end
            end
            d[#d + 1] = "tMissions=" .. n .. " sSelectedMission=" .. tostring(w.CustomData.sSelectedMission)
            local bd = rawget(w, "BasicData")
            if type(bd) == "table" then
                d[#d + 1] = "enabled=" .. tostring(bd.bEnabled) .. " owner=" .. tostring(bd.uOwnerGuid)
            end
            out[#out + 1] = "[" .. i .. " " .. tostring(w) .. ": " .. table.concat(d, " ") .. "]"
            if not rawget(w, "__m2_wrapped") then
                rawset(w, "__m2_wrapped", true)
                local tag = tostring(w)
                for _, name in ipairs({"Open", "Close"}) do
                    local f = w[name]
                    if type(f) == "function" then
                        w[name] = function(self, ...)
                            print("PDA " .. name .. " called on " .. tag)
                            return f(self, ...)
                        end
                    end
                end
            end
        end
        local st = rawget(_G, "MrxState")
        if type(st) == "table" and type(st.IsLocked) == "function" then
            local ok, v = pcall(st.IsLocked)
            out[#out + 1] = "MrxState.IsLocked=" .. tostring(ok and v)
        end
        local dlg = rawget(_G, "MrxGuiDialogBox")
        out[#out + 1] = "system dialog=" .. tostring(type(dlg) == "table" and rawget(dlg, "oSystemDialogBoxFlash"))
        local base = rawget(_G, "MrxGuiBase")
        if type(base) == "table" then
            for _, f in ipairs({"GetCurrentControlHolder", "GetControlHolder"}) do
                if type(base[f]) == "function" then
                    local ok, v = pcall(base[f])
                    out[#out + 1] = f .. "=" .. tostring(ok and v)
                end
            end
        end
        return table.concat(out, " ")
    "#;
    lua.load(PROBE)
        .eval::<String>()
        .unwrap_or_else(|e| format!("probe failed: {e}"))
}

/// HUD updates the PDA's script needs between its movie's load (and
/// each toggle) and the next toggle: its close's cooldown is 20.
const PDA_SETTLE: u32 = 30;

/// Whether a prim covers over half of `screen`.
fn covers(p: &Prim, screen: egui::Rect) -> bool {
    let xs = p.corners.map(|c| c[0]);
    let ys = p.corners.map(|c| c[1]);
    let w =
        xs.iter().cloned().fold(f32::MIN, f32::max) - xs.iter().cloned().fold(f32::MAX, f32::min);
    let h =
        ys.iter().cloned().fold(f32::MIN, f32::max) - ys.iter().cloned().fold(f32::MAX, f32::min);
    w * h > screen.area() / 2.0
}

/// The thread: reads the scripts, fonts and strings, starts the HUD (its
/// player `hero`), then draws each state sent until the sender goes.
fn run(
    data: &std::path::Path,
    mode: Mode,
    hero: Option<(Hero, f32)>,
    rx: Receiver<HudState>,
    tx: Sender<Output>,
) {
    let mut hud = match Thread::start(data, mode, hero) {
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
        let out = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| hud.frame(&state)))
        {
            Ok(out) => out,
            Err(e) => {
                let why = e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                    .unwrap_or_default();
                let _ = tx.send(Output::Failed(format!("the HUD's frame panicked: {why}")));
                return;
            }
        };
        if tx.send(out).is_err() {
            return;
        }
    }
}

/// What the HUD has been told, to fire only what changed.
#[derive(Default)]
struct Told {
    /// The save icon was asked up ([`HudState::saving`]).
    saving: bool,
    hp: Option<(i32, bool)>,
    /// The gun in her hands, whether she holds the designator, and whether
    /// she drives a vehicle with a cannon.
    weapon: Option<(Weapon, bool, bool, bool)>,
    /// The template of the gun in her rifle's place ([`HudState::gun`]).
    gun: Option<u32>,
    ammo: Option<[i64; 5]>,
    /// She was in a vehicle ([`HudState::in_vehicle`]).
    in_vehicle: Option<bool>,
    /// The gun box's icon widgets as last logged ([`Thread::gun_box`]).
    gun_box: String,
    /// Her cash as last told, in whole dollars.
    cash: Option<i64>,
    vehicle: Option<Option<String>>,
    vehicle_templates: Option<Vec<String>>,
    target: Option<Option<(i32, i32, i32)>>,
    reticle: Option<Option<[f32; 2]>>,
    regions: Option<Vec<RadarRegion>>,
    /// The support item ready (how many, whether free), and the menu.
    support_item: Option<Option<(u32, bool)>>,
    menu: Option<bool>,
    /// The satellite's view is open.
    satellite: bool,
    /// What the binocular screen was last told (None: down).
    binoculars: Option<binoculars::View>,
    /// The zoom and the handlers that answered when the meters were last
    /// logged.
    binoculars_logged: Option<(u8, usize)>,
    /// What the tutorial box says (None: never anything).
    tutorial: Option<Option<String>>,
    trespass: crate::mission::Trespass,
    disguise: Option<Option<(f32, bool)>>,
    /// The keys the texts' control words name ([`HudState::key_names`]).
    key_names: Option<Vec<(String, String)>>,
}

impl Told {
    /// She is `in_vehicle` now: got out of one, her gun and its rounds are
    /// to be told again (as for a change of gun).
    fn seat(&mut self, in_vehicle: bool) {
        if self.in_vehicle == Some(true) && !in_vehicle {
            self.weapon = None;
            self.ammo = None;
        }
        self.in_vehicle = Some(in_vehicle);
    }
}

/// How far (in -1..=1 screen units, across plus up) the aim point moves
/// before the reticle is moved.
const RETICLE_STEP: f32 = 0.002;

struct Thread {
    mission: Mission,
    /// The hero its player was last made ([`HudState::hero`]).
    hero: Option<Hero>,
    assets: AssetSource<BufReader<File>>,
    /// The language archive: the HUD's localised textures (the fanfares'
    /// titles and borders are only there, as `<name>_english`).
    language: Option<AssetSource<BufReader<File>>>,
    fonts: BTreeMap<String, Font>,
    /// Fonts the text widgets name, looked for (read, or found missing).
    fonts_tried: HashSet<String>,
    /// Textures tried (sent, or found missing).
    tried: HashSet<u32>,
    told: Told,
    /// The fullscreen widgets last logged.
    fullscreen_seen: String,
    /// The solid quads over the screen last logged.
    covers_seen: String,
    /// The reticle's widgets were listed in the log.
    reticle_logged: bool,
    /// Whether its script showed the PDA last frame (its movie drawn).
    pda_shown: bool,
    /// Keys for the movies' `Key` listeners ([`HudState::keys`]).
    keys: Vec<(i32, bool)>,
    /// Whether the scripts were told the `Pause` game state was entered.
    pause: bool,
    /// The engine's side of the pause menu's movie, and the movie it
    /// set up (widget, restarts).
    pause_native: crate::pause::Native,
    pause_movie: Option<(u32, u32)>,
    /// The player's options as last told the HUD's scripts.
    options_told: Option<(bool, bool, bool, bool)>,
    /// The PDA's movement keys held ([`PDA_STICK_KEYS`]), and the stick
    /// they last pushed its map with.
    pda_held: HashSet<i32>,
    pda_stick: [f32; 2],
    /// HUD updates since the PDA's movie loaded (`None`: not yet), and
    /// since `TogglePDA` was last sent.
    pda_ready: Option<u32>,
    since_toggle: u32,
    /// Whether its script showed it, as last logged.
    pda_scripted: Option<bool>,
    pda_hid_logged: bool,
    contract_in_pda: Option<Vec<(String, String)>>,
    /// The zones the radar's script holds, as last read.
    zones: Vec<Zone>,
    /// The island's roads, read with the radar's map.
    roads: Vec<Road>,
    /// The PDA's screens as triangles (see [`pda_triangles`]): not read yet,
    /// or read (None if they could not be).
    pda: Option<Option<PdaScreens>>,
    /// The archive's Flash movies and their fonts.
    movies: Rc<RefCell<Movies>>,
    /// The movies playing in the Flash widgets, by widget.
    flash: BTreeMap<u32, FlashMovie>,
    mode: Mode,
    /// The game state the front end is in (`Shell`), once entered.
    game_state: Option<String>,
    /// The front end's Flash widgets' state, as last logged.
    shell_seen: String,
    /// The engine's side of the front end's menu movie.
    native: crate::shell::Native,
    /// `fscommand`s no script heard, for the engine (the front end's).
    unheard: Vec<(String, String)>,
    /// The HUD's, with the widget whose movie said them.
    unheard_by: Vec<(u32, String, String)>,
    /// Seconds since `M2_SHELL_PROBE` (or `M2_HUD_PROBE`) last logged.
    probe_time: f64,
    /// How many natives the scripts called that aren't made, as last
    /// logged.
    unknown_seen: usize,
    /// What posing the timing wheel found, as last logged.
    wheel_seen: String,
}

/// The language archive beside `vz.wad` (English only for now).
const LANGUAGE_WAD: &str = "English.wad";

impl Thread {
    fn start(
        data: &std::path::Path,
        mode: Mode,
        hero: Option<(Hero, f32)>,
    ) -> Result<Self, String> {
        let wad = data.join(match mode {
            Mode::Hud => "vz.wad",
            Mode::Shell => "shell.wad",
        });
        let open = || {
            File::open(&wad)
                .map(BufReader::new)
                .map_err(|e| format!("{}: {e}", wad.display()))
        };
        let game_data = WadArchive::open(open()?)
            .map_err(|e| e.to_string())
            .and_then(|mut w| GameData::read(&mut w).map_err(|e| e.to_string()))?;
        let mut assets = AssetSource::new(open()?).map_err(|e| e.to_string())?;
        let language = File::open(data.join(LANGUAGE_WAD))
            .ok()
            .and_then(|f| AssetSource::new(BufReader::new(f)).ok());
        let movies = Movies {
            assets: AssetSource::new(open()?).map_err(|e| e.to_string())?,
            libraries: HashMap::new(),
            fonts: None,
        };
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
        // Its player is play's hero from the start, for the modules' `Init`
        // and `CreateGui` to see; a later pick renames her
        // ([`HudState::hero`]). The front end's world keeps "Mattias".
        let name = hero.map_or("Mattias", |(h, _)| h.name());
        let mut mission = Mission::new(game_data, name, [0.0; 3]).map_err(|e| e.to_string())?;
        if let Some((h, max)) = hero {
            crate::mission::set_hero(&mut mission, h, max, "game HUD");
        }
        let screen = gui_runtime::screen(mission.lua()).ok_or("no GUI runtime")?;
        {
            let mut s = screen.borrow_mut();
            s.set_strings(strings);
            for (name, f) in &fonts {
                s.set_font(name, f.clone());
            }
        }
        if mode == Mode::Shell {
            crate::shell::open(&mut mission)?;
            return Ok(Self::new(mission, assets, language, fonts, movies, mode));
        }
        // Her guns, for the gun box to name.
        {
            let mut world = mission.world().borrow_mut();
            for (guid, name) in [
                (RIFLE_GUID, RIFLE_NAME),
                (RPG_GUID, RPG_NAME),
                (LAUNCHER_GUID, LAUNCHER_NAME),
                (DESIGNATOR_GUID, support::Kind::SatelliteBomb.designator()),
                (CANNON_GUID, ""),
                (VEHICLE_GUID, ""),
            ] {
                world.insert(guid, Object::new(name, [0.0; 3]));
            }
        }
        // As the player-character module does; her guid is userdata, or
        // the scripts never own the widgets and no event reaches them.
        // The HUD's own scripts look her up as the local player too
        // (`CommenceFanfare` hides the HUD through `GetHudState` and
        // `ToggleHud` of `Player.GetLocalPlayer()`), so here that answers
        // the same userdata, not m2's number. The game has the dialog box
        // module loaded at boot; unloaded, its `oSystemDialogBoxFlash` is a
        // stub, and `MrxGuiPda.Open` takes that for a system dialog up.
        // Her cash is the scripts', as `PlayerCashChanged` tells the HUD (the PDA's
        // stats page formats `Player.GetCash()`). She is the only player
        // (`UpdateAllPlayerMarkers` puts each on the PDA's map).
        mission
            .run(
                "local me = Sys.StringToGuid(\"0x1\")
                 for _, f in ipairs({\"GetLocalPlayer\", \"GetPrimaryPlayer\"}) do
                     rawset(Player, f, function() return me end)
                 end
                 rawset(Player, \"GetCash\", function() return __m2_cash or 0 end)
                 rawset(Player, \"GetAllPlayers\", function() return { me } end)
                 import(\"MrxGuiDialogBox\")
                 MrxGuiManager.CreateGui(me)",
                "create gui",
            )
            .map_err(|e| e.to_string())?;
        mission
            .run(VIEW_POINT, "m2-hud view point")
            .map_err(|e| e.to_string())?;
        mission
            .run(LTI_LIBRARY, "m2-hud pc library")
            .map_err(|e| e.to_string())?;
        // What the game finishes while it loads, before play: the
        // movies' load callbacks (the PDA's closes it, `_FinishLoadAndClose`;
        // left open, the support menu won't open over it).
        gui_runtime::tick(mission.lua(), 0.0);
        let mut thread = Self::new(mission, assets, language, fonts, movies, mode);
        thread.hero = hero.map(|(h, _)| h);
        Ok(thread)
    }

    fn new(
        mission: Mission,
        assets: AssetSource<BufReader<File>>,
        language: Option<AssetSource<BufReader<File>>>,
        fonts: BTreeMap<String, Font>,
        movies: Movies,
        mode: Mode,
    ) -> Self {
        Self {
            mission,
            hero: None,
            assets,
            language,
            fonts,
            fonts_tried: HashSet::new(),
            tried: HashSet::new(),
            told: Told::default(),
            fullscreen_seen: String::new(),
            covers_seen: String::new(),
            reticle_logged: false,
            pda_shown: false,
            keys: Vec::new(),
            pause: false,
            pause_native: crate::pause::Native::default(),
            pause_movie: None,
            options_told: None,
            pda_held: HashSet::new(),
            pda_stick: [0.0; 2],
            pda_ready: None,
            since_toggle: u32::MAX,
            pda_scripted: None,
            pda_hid_logged: false,
            contract_in_pda: None,
            zones: Vec::new(),
            roads: Vec::new(),
            pda: None,
            movies: Rc::new(RefCell::new(movies)),
            flash: BTreeMap::new(),
            mode,
            game_state: None,
            shell_seen: String::new(),
            native: crate::shell::Native::default(),
            unheard: Vec::new(),
            unheard_by: Vec::new(),
            probe_time: 0.0,
            unknown_seen: 0,
            wheel_seen: String::new(),
        }
    }

    fn frame(&mut self, state: &HudState) -> Output {
        if self.mode == Mode::Shell {
            return self.shell_frame(state);
        }
        // Who she is, as play says: picked in the front end after this
        // thread started, or changed from the pause menu.
        if let Some((hero, max)) = state.hero
            && self.hero != Some(hero)
        {
            self.hero = Some(hero);
            crate::mission::set_hero(&mut self.mission, hero, max, "game HUD");
        }
        let mut log = Vec::new();
        if let Err(e) = self.feed(state) {
            log.push(format!("event: {e}"));
        }
        self.pass_calls(state, &mut log);
        for b in &state.buttons {
            self.mission.press_button(b);
        }
        // The PDA button: the engine's `TogglePDA` GUI event, which the PDA
        // layout's widget hands to `MrxGuiPda` (it opens the "topbar"
        // movie over a black backdrop and puts the HUD to sleep). Only
        // once its movie has loaded and the cooldown of the close that
        // follows (20 updates) has passed: before, a toggle closes it.
        if self.pda_ready.is_none() && self.flash.values().any(|m| m.swf.contains("topbar")) {
            self.pda_ready = Some(0);
        }
        if state.dt > 0.0 {
            if let Some(n) = &mut self.pda_ready {
                *n = n.saturating_add(1);
            }
            self.since_toggle = self.since_toggle.saturating_add(1);
        }
        if state.support.pda != self.pda_shown
            && self.pda_ready.is_some_and(|n| n >= PDA_SETTLE)
            && self.since_toggle >= PDA_SETTLE
        {
            self.since_toggle = 0;
            // The contract goes in (again when its objectives changed) as
            // the PDA opens.
            if state.support.pda && self.contract_in_pda.as_ref() != Some(&state.pda_objectives) {
                self.contract_in_pda = Some(state.pda_objectives.clone());
                if let Err(e) = contract_in_pda(self.mission.lua(), &state.pda_objectives) {
                    log.push(format!("contract in the PDA: {e}"));
                }
            }
            log.push(format!("PDA state: {}", pda_probe(self.mission.lua())));
            // Which Lua functions the event runs, for the log.
            let lua = self.mission.lua();
            let _ = lua
                .load(
                    r#"__m2_calls = {}
                    if debug and debug.sethook then
                        debug.sethook(function(kind, line)
                            local n = #__m2_calls
                            if n >= 80 then return end
                            local i = debug.getinfo(2, "nS")
                            if kind == "line" then
                                if i and i.short_src == "MrxGuiPda" then
                                    __m2_calls[n + 1] = "L" .. tostring(line)
                                end
                            elseif i and i.what ~= "C" then
                                __m2_calls[n + 1] = tostring(i.name) .. "@" .. tostring(i.short_src)
                                    .. ":" .. tostring(i.linedefined)
                            end
                        end, "cl")
                    else
                        __m2_calls[1] = "no debug.sethook"
                    end"#,
                )
                .exec();
            let fired = gui_runtime::fire(lua, "TogglePDA", ());
            let calls: String = lua
                .load(
                    r#"if debug and debug.sethook then debug.sethook() end
                    local pda = rawget(_G, "MrxGuiPda")
                    local env = type(pda) == "table" and type(pda.Open) == "function"
                        and getfenv(pda.Open)
                    if type(env) == "table" then
                        local d = rawget(env, "MrxGuiDialogBox")
                        __m2_calls[#__m2_calls + 1] = "| env dialog raw=" .. tostring(d)
                            .. " read=" .. tostring(env.MrxGuiDialogBox)
                            .. " global=" .. tostring(rawget(_G, "MrxGuiDialogBox"))
                            .. " flash=" .. tostring(env.MrxGuiDialogBox
                                and env.MrxGuiDialogBox.oSystemDialogBoxFlash)
                    end
                    return table.concat(__m2_calls or {}, " ")"#,
                )
                .eval()
                .unwrap_or_default();
            log.push(format!("TogglePDA ran: {calls}"));
            match fired {
                Ok(n) => log.push(format!(
                    "TogglePDA (want {}): {n} handlers",
                    state.support.pda
                )),
                Err(e) => log.push(format!("TogglePDA: {e}")),
            }
        }
        if state.pause != self.pause {
            self.pause = state.pause;
            let phase = if state.pause { "Enter" } else { "Exit" };
            match gui_runtime::fire(self.mission.lua(), "GuiGameStateChange", ("Pause", phase)) {
                Ok(n) => log.push(format!("pause: game state Pause {phase} ({n} handlers)")),
                Err(e) => log.push(format!("pause: game state Pause {phase}: {e}")),
            }
        }
        let dt = state.dt.min(MAX_STEP);
        if let Err(e) = self.feed_support(state, dt) {
            log.push(format!("support: {e}"));
        }
        if let Err(e) = self.feed_binoculars(state, &mut log) {
            log.push(format!("binoculars: {e}"));
        }
        if dt > 0.0 {
            self.mission.tick(dt);
            gui_runtime::tick(self.mission.lua(), dt);
        }
        if let Err(e) = self.pose_wheel(state, &mut log) {
            log.push(format!("wheel: {e}"));
        }
        self.gun_box(state, &mut log);
        // Keys go to the movie that has the controls: the mission's
        // fanfare while it is up (its Key listener's Enter continues, as
        // `_HandleFanfareInput` passes the select button on), else the PDA.
        self.tell_options(&state.pause_settings, &mut log);
        if self.pda_shown || self.fanfare_up() || self.pause_up() {
            self.keys.extend(state.keys.iter().copied());
        }
        // The PDA's map moves while its movement keys are held, as the
        // pad's left stick moves it.
        for &(code, down) in &state.keys {
            if down && self.pda_shown {
                self.pda_held.insert(code);
            } else {
                self.pda_held.remove(&code);
            }
        }
        if !self.pda_shown {
            self.pda_held.clear();
        }
        // The movement keys and the pad's stick together, sped up.
        let keys = pda_stick(&self.pda_held);
        let stick = if self.pda_shown {
            [0, 1].map(|i| (keys[i] + state.stick[i]).clamp(-1.0, 1.0) * PDA_PAN)
        } else {
            [0.0; 2]
        };
        if stick != [0.0; 2] || self.pda_stick != [0.0; 2] {
            let took = self
                .mission
                .lua()
                .load(PDA_STICK)
                .set_name("=m2-hud pda stick")
                .call::<bool>((stick[0], stick[1]));
            if stick != self.pda_stick {
                log.push(format!("pda: stick {stick:?} ({took:?})"));
            }
            self.pda_stick = stick;
        }
        for &c in &state.pause_captured {
            log.push(format!("pause: the keyboard table takes {c:?}"));
            self.pause_native.captured(c);
        }
        self.pause_calls(state, &mut log);
        if self.pause_up() {
            self.mouse(PAUSE_MOVIE, state, &mut log);
        }
        self.play_flash(dt, &mut log);
        self.pause_answers(&mut log);
        let Some(screen) = gui_runtime::screen(self.mission.lua()) else {
            return Output::Failed("no GUI runtime".into());
        };
        let [w, h] = state.size;
        let items = screen.borrow().draw_list(w, h);
        self.load_fonts(&items, &mut log);
        // Natives the scripts call that aren't made (they answer nothing,
        // or zeros), as more are called.
        {
            let s = screen.borrow();
            if s.unknown().len() != self.unknown_seen {
                self.unknown_seen = s.unknown().len();
                let unknown: Vec<&str> = s.unknown().iter().map(String::as_str).collect();
                log.push(format!("natives not made: {}", unknown.join(", ")));
            }
        }
        // The fullscreen widgets drawn (a fanfare and its fade), where in
        // the order and how opaque, when that changes.
        let fullscreen: Vec<String> = {
            let s = screen.borrow();
            items
                .iter()
                .enumerate()
                .filter_map(|(i, item)| {
                    let id = widget_of(item);
                    let w = s.widgets().find(|(wid, _)| *wid == id)?.1;
                    w.fullscreen.then(|| {
                        format!(
                            "{id} {:?} at {i}/{} alpha {:.0}",
                            w.kind,
                            items.len(),
                            w.colour[3]
                        )
                    })
                })
                .collect()
        };
        let fullscreen = fullscreen.join(", ");
        if fullscreen != self.fullscreen_seen {
            log.push(format!("fullscreen widgets: [{fullscreen}]"));
            self.fullscreen_seen = fullscreen;
        }
        log.extend(screen.borrow_mut().take_log());
        log.extend(self.mission.take_log());
        // Of the HUD scripts' own commands, the game states the pause
        // menu's ask for (leaving it), and their interface sounds (the
        // cash counter's `UI_hud_cashUp_*` as it counts up: playtest 4).
        for c in self.mission.take_commands() {
            use m2_game::api::Command;
            match c {
                Command::GameState(s) => {
                    log.push(format!("pause: the scripts ask for game state {s}"));
                    self.pause_native.game_state(&s);
                }
                Command::CueSound { at: None, cue } => {
                    self.pause_native.command("Sound.Play", &cue);
                }
                Command::StopSound { at: None, cue } => {
                    self.pause_native.command("Sound.Stop", &cue);
                }
                _ => {}
            }
        }
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
        // Shown by its script: its movie is up.
        let scripted = state.support.pda
            && items
                .iter()
                .any(|i| matches!(i, DrawItem::Flash { swf: Some(f), .. } if f.contains("topbar")));
        self.pda_shown = items
            .iter()
            .any(|i| matches!(i, DrawItem::Flash { swf: Some(f), .. } if f.contains("topbar")));
        if state.support.pda && self.pda_scripted != Some(scripted) {
            self.pda_scripted = Some(scripted);
            log.push(if scripted {
                "PDA: opened by its script".into()
            } else {
                "PDA: its script did not open it; drawn directly".into()
            });
        }
        // Not opened by its script: only the PDA drawn here and the
        // tutorial box.
        // Opened by its script: its movie and what hangs under it. The
        // scripts put the HUD to sleep (`ToggleHud`) as for the satellite,
        // which leaves some of it awake here.
        // By id: the natives are made nameless, and the movie's widget
        // has no layout name either. With the fade, which dims the world.
        let pda_movies: HashSet<u32> = items
            .iter()
            .filter_map(|i| match i {
                DrawItem::Flash {
                    widget,
                    swf: Some(f),
                    ..
                } if f.contains("topbar") => Some(*widget),
                _ => None,
            })
            .collect();
        let only = if state.support.pda && !scripted {
            Some(widgets_under(&screen.borrow(), &names, |n| n == "tutorial"))
        } else if scripted {
            let screen = screen.borrow();
            // The PDA widget holds the movie and, first, its backdrop
            // (`_Initialize`: a fullscreen image coloured 0,0,0,192).
            let roots: HashSet<u32> = pda_movies
                .iter()
                .map(|&id| screen.widget(id).and_then(|w| w.parent).unwrap_or(id))
                .collect();
            let mut keep = widgets_below(&screen, |id| roots.contains(&id));
            keep.extend(widgets_under(&screen, &names, |n| n == "FlashFade"));
            if !self.pda_hid_logged {
                self.pda_hid_logged = true;
                let mut hid: Vec<&str> = items
                    .iter()
                    .map(widget_of)
                    .filter(|w| !keep.contains(w))
                    .filter_map(|w| names.get(&w).map(String::as_str))
                    .collect();
                hid.sort_unstable();
                hid.dedup();
                log.push(format!(
                    "PDA keeps {} widgets; hides awake {}",
                    keep.len(),
                    hid.join(", ")
                ));
            }
            Some(keep)
        } else if state.binoculars.is_some() {
            // Through the binoculars only their screen shows: the scripts
            // put the HUD to sleep (`ToggleHud(owner, false, "scope")`),
            // which leaves some of it awake here.
            Some(widgets_under(&screen.borrow(), &names, |n| {
                n == "Binoculars"
            }))
        } else {
            state.support.satellite.map(|_| {
                widgets_under(&screen.borrow(), &names, |n| {
                    n == "Satellite overlay" || n == "tutorial"
                })
            })
        };
        // No aim point: the reticle's widgets are left out (not the
        // satellite's own, its crosshair, nor the binoculars').
        let hidden = match state
            .reticle
            .or(state.support.satellite.map(|_| [0.0; 2]))
            .or(state.binoculars.map(|_| [0.0; 2]))
        {
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
        // The tutorial box with nothing to say is left out, frame and all
        // (inferred: its line emptied, by either of the paths that write
        // it, would leave the frame drawn as a bare bar).
        let mut hidden = hidden;
        let tutorial = widgets_under(&screen.borrow(), &names, |n| n == "tutorial");
        if !says_something(&items, &tutorial) {
            hidden.extend(tutorial);
        }
        let mut prims = Vec::new();
        let mut want = Vec::new();
        let shown = |i: &&DrawItem| {
            let w = widget_of(i);
            !hidden.contains(&w) && only.as_ref().is_none_or(|o| o.contains(&w))
        };
        // `M2_HUD_PROBE=x,y` (screen points, as the HUD is drawn): once a
        // second, what is drawn over that point, and what of it is left
        // out here.
        if let Some(at) = probe_point("M2_HUD_PROBE") {
            self.probe_time += dt;
            if self.probe_time >= 1.0 {
                self.probe_time = 0.0;
                log.push(format!("probe {},{}:", at[0], at[1]));
                let s = screen.borrow();
                for item in items.iter().filter(|i| holds(i, at)) {
                    let left_out = if shown(&item) { "" } else { " (left out)" };
                    log.push(format!("  {}{left_out}", probe_line(&s, &names, item)));
                }
            }
        }
        if state.support.pda && !scripted {
            if self.pda.is_none() {
                let started = Instant::now();
                let screens = pda_triangles(&mut self.assets);
                log.push(match &screens {
                    Ok(s) => format!(
                        "PDA screens: {} triangles, {:?} px, in {:.0?}",
                        s.triangles.len(),
                        s.size,
                        started.elapsed()
                    ),
                    Err(e) => format!("PDA screens: {e}"),
                });
                self.pda = Some(screens.ok());
            }
            if let Some(Some(screens)) = &self.pda {
                pda_prims(screens, state.size, &mut prims, &mut want);
            }
        }
        for item in items.iter().filter(shown) {
            self.prims(item, state, &mut prims, &mut want);
        }
        self.log_flash_shown(&items, &shown, &mut log);
        // Solid quads over most of the screen (a fade), when they change.
        let [sw, sh] = state.size;
        let covers: Vec<String> = prims
            .iter()
            .filter(|p| p.texture.is_none())
            .filter(|p| {
                let xs = p.corners.map(|c| c[0]);
                let ys = p.corners.map(|c| c[1]);
                let w = xs.iter().cloned().fold(f32::MIN, f32::max)
                    - xs.iter().cloned().fold(f32::MAX, f32::min);
                let h = ys.iter().cloned().fold(f32::MIN, f32::max)
                    - ys.iter().cloned().fold(f32::MAX, f32::min);
                w * h > sw * sh / 2.0
            })
            .map(|p| format!("{:.2?}", p.colour))
            .collect();
        let covers = format!("{covers:?}");
        if covers != self.covers_seen {
            log.push(format!("screen covers: {covers} of {} prims", prims.len()));
            self.covers_seen = covers;
        }
        // The objective markers' icons, for the world's markers
        // ([`GameHud::paint_texture`]) whether or not the minimap is up.
        want.extend(state.blips.iter().map(|(_, icon, _)| name_hash(icon)));
        // The pad's Y, for the use prompts with a pad (`crate::ui`).
        if state.pad {
            want.push(name_hash(crate::pad::Button::Y.texture()));
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
                            let from = if roads.iter().any(|r| r.width.is_some()) {
                                format!("the radar movie's map, {} road strokes", roads.len())
                            } else {
                                format!("sea at {sea} m, {} roads ({paved} paved)", roads.len())
                            };
                            log.push(format!(
                                "radar map: {}x{}, {from}, in {:.0?}",
                                i.width,
                                i.height,
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
                let image = self.decode(hash, &mut log);
                images.push((hash, image));
            }
        }
        // Taken out, not emptied: mlua's `Table::clear` (Lua 5.1) leaves the
        // table on the Lua stack, one slot a frame, until every call into
        // the HUD's scripts fails ("out of Lua stack") after ~8000 frames.
        let globals = self.mission.lua().globals();
        let callbacks = globals
            .raw_get::<Option<mlua::Table>>("__m2_hud_done")
            .ok()
            .flatten()
            .map(|t| {
                let _ = globals.raw_set("__m2_hud_done", mlua::Value::Nil);
                t.sequence_values::<u64>().flatten().collect()
            })
            .unwrap_or_default();
        // Taken out the same way.
        let choices = globals
            .raw_get::<Option<mlua::Table>>("__m2_hud_choices")
            .ok()
            .flatten()
            .map(|t| {
                let _ = globals.raw_set("__m2_hud_choices", mlua::Value::Nil);
                t.sequence_values::<mlua::Table>()
                    .flatten()
                    .filter_map(|c| Some((c.get::<u64>(1).ok()?, c.get::<i64>(2).ok()?)))
                    .collect()
            })
            .unwrap_or_default();
        Output::Frame {
            prims,
            images,
            log,
            callbacks,
            choices,
            shell: Box::default(),
            pause: Box::new(PauseOut {
                news: self.pause_native.take_news(),
                available: names.values().any(|n| n == PAUSE_LAYOUT),
            }),
        }
    }

    /// Reads the fonts the text widgets drawn name and the HUD hasn't
    /// read yet (from the archive, else the language archive), once each:
    /// text in a font not read isn't drawn. A font read is given to the
    /// widgets' store too, for their measures. Those missing are logged.
    fn load_fonts(&mut self, items: &[DrawItem], log: &mut Vec<String>) {
        for item in items {
            let DrawItem::Text { font, .. } = item else {
                continue;
            };
            let name = font.to_ascii_lowercase();
            if name.is_empty()
                || self.fonts.contains_key(&name)
                || !self.fonts_tried.insert(name.clone())
            {
                continue;
            }
            let hash = name_hash(&name);
            let read = |a: &mut AssetSource<BufReader<File>>| {
                a.asset(hash, FONT_TYPE)
                    .map_err(|e| e.to_string())
                    .and_then(|a| Font::parse(&a).map_err(|e| e.to_string()))
            };
            let found = read(&mut self.assets)
                .or_else(|e| self.language.as_mut().and_then(|l| read(l).ok()).ok_or(e));
            match found {
                Ok(f) => {
                    log.push(format!("font {name}: read for a text widget"));
                    if let Some(screen) = gui_runtime::screen(self.mission.lua()) {
                        screen.borrow_mut().set_font(&name, f.clone());
                    }
                    self.fonts.insert(name, f);
                }
                Err(e) => {
                    eprintln!("game HUD: font {name}: {e}");
                    log.push(format!("font {name}: {e}"));
                }
            }
        }
    }

    /// Texture `hash` from the archive (or the language archive).
    fn decode(&mut self, hash: u32, log: &mut Vec<String>) -> Option<Image> {
        let found = self.assets.texture(hash, MAX_TEXTURE).or_else(|e| {
            self.language
                .as_mut()
                .and_then(|l| l.texture(hash, MAX_TEXTURE).ok())
                .ok_or(e)
        });
        match found {
            Ok(t) => t.mips.into_iter().next().map(|rgba| Image {
                width: t.width,
                height: t.height,
                rgba,
            }),
            Err(e) => {
                log.push(format!("texture {hash:08x}: {e}"));
                None
            }
        }
    }

    /// One frame of the front end ([`Mode::Shell`]): every key goes to its
    /// movies, the scripts and movies run, and everything on screen is
    /// drawn but the movie widget's video, which the game plays under it
    /// ([`ShellNews::movie`]).
    fn shell_frame(&mut self, state: &HudState) -> Output {
        let mut log = Vec::new();
        let dt = state.dt.min(MAX_STEP);
        self.keys.extend(state.keys.iter().copied());
        if dt > 0.0 {
            self.mission.tick(dt);
            gui_runtime::tick(self.mission.lua(), dt);
        }
        // The engine's calls into the menu movie: set up as it starts, and
        // the keys it waits for itself.
        let mut calls = Vec::new();
        if self.flash.values().any(|m| m.swf == crate::shell::MOVIE) {
            for &c in &state.pause_captured {
                log.push(format!("shell: the keyboard table takes {c:?}"));
                self.native.captured(c);
            }
            calls.extend(self.native.calls(&state.pause_settings));
            for &(code, down) in &state.keys {
                if down {
                    calls.extend(self.native.key(code));
                }
            }
        }
        if let Some(m) = self
            .flash
            .values_mut()
            .find(|m| m.swf == crate::shell::MOVIE)
        {
            for (name, args) in calls {
                log.push(format!("shell: the engine calls {name}"));
                m.rt.call(name, &args);
            }
        }
        self.mouse(crate::shell::MOVIE, state, &mut log);
        self.play_flash(dt, &mut log);
        // `M2_SHELL_PROBE=x,y`: once a second, what covers that stage
        // point in the menu movie.
        if let Some((x, y)) = std::env::var("M2_SHELL_PROBE")
            .ok()
            .and_then(|v| {
                v.split_once(',')
                    .map(|(a, b)| (a.parse().ok(), b.parse().ok()))
            })
            .and_then(|(a, b)| a.zip(b))
        {
            self.probe_time += dt;
            if self.probe_time >= 1.0 {
                self.probe_time = 0.0;
                let movies = self.movies.borrow();
                if let (Some(m), Some(fonts)) = (
                    self.flash.values().find(|m| m.swf == crate::shell::MOVIE),
                    movies.fonts.as_ref(),
                ) {
                    log.push(format!("probe {x},{y}:"));
                    log.extend(
                        m.rt.player
                            .covering([x, y], 4, fonts)
                            .into_iter()
                            .map(|l| format!("  {l}")),
                    );
                }
            }
        }
        for (command, arg) in std::mem::take(&mut self.unheard) {
            if self.native.command(&command, &arg) {
                log.push(format!("shell: the engine hears {command}({arg})"));
            }
        }
        let video = state.pause_settings.video;
        for (name, arg) in take_library_calls(self.mission.lua()) {
            // As the pause menu's: which Aspect Ratio is the movie's own
            // `videoWidescreen`.
            if name == "LTIVideoSwitchMode"
                && let Some(wide) = self.movie_bool(crate::shell::MOVIE, "videoWidescreen")
            {
                self.native.set_wide(wide);
            }
            let answered = self
                .native
                .library(&name, &arg, video, &state.pause_settings.modes);
            log.push(format!(
                "shell: LTILibName.{name}({arg}){}",
                if answered { "" } else { " not answered" }
            ));
        }
        let Some(screen) = gui_runtime::screen(self.mission.lua()) else {
            return Output::Failed("no GUI runtime".into());
        };
        let seen: String = screen
            .borrow()
            .widgets()
            .filter(|(_, w)| matches!(w.kind, gui_runtime::Kind::Flash | gui_runtime::Kind::Movie))
            .map(|(id, w)| {
                format!(
                    "{id} {:?} {:?}{}{}; ",
                    w.swf,
                    w.movie,
                    if w.flash_paused { " paused" } else { "" },
                    if w.movie_playing { " playing" } else { "" }
                )
            })
            .collect();
        if seen != self.shell_seen {
            log.push(format!("shell widgets: {seen}"));
            self.shell_seen = seen;
        }
        let [w, h] = state.size;
        let items = screen.borrow().draw_list(w, h);
        let mut prims = Vec::new();
        let mut want = Vec::new();
        let mut movie = None;
        let mut under = 0;
        for item in &items {
            match item {
                DrawItem::Movie {
                    file: Some(file),
                    rect,
                    playing: true,
                    ..
                } => {
                    movie = Some((file.clone(), *rect));
                    under = prims.len();
                }
                _ => self.prims(item, state, &mut prims, &mut want),
            }
        }
        self.log_flash_shown(&items, &|_| true, &mut log);
        log.extend(screen.borrow_mut().take_log());
        log.extend(self.mission.take_log());
        let commands = self.mission.take_commands();
        log.extend(crate::shell::game_states(
            self.mission.lua(),
            &commands,
            &mut self.game_state,
        ));
        let new_game = crate::shell::take_new_game(self.mission.lua());
        let mut images = Vec::new();
        for hash in want {
            if self.tried.insert(hash) {
                let image = self.decode(hash, &mut log);
                images.push((hash, image));
            }
        }
        Output::Frame {
            prims,
            images,
            log,
            callbacks: Vec::new(),
            choices: Vec::new(),
            shell: Box::new(ShellNews {
                movie,
                under,
                new_game,
                news: self.native.take_news(),
                started: self.native.started(),
            }),
            pause: Box::default(),
        }
    }

    /// Plays the Flash widgets' movies `dt` seconds: starts the movies the
    /// scripts set (`SetFlashSwfFile`, again on `RestartFlash`), hands
    /// them the scripts' calls (`CallActionScriptCallback`), steps those
    /// not paused, and hands their `fscommand`s back to the scripts.
    /// The mouse, to the menu movie `swf` (as Scaleform passes it on):
    /// the pointer from the screen to its stage, through the box it was
    /// last drawn in, then the left button's presses and releases.
    fn mouse(&mut self, swf: &str, state: &HudState, log: &mut Vec<String>) {
        let Some(m) = self.flash.values_mut().find(|m| m.swf == swf) else {
            return;
        };
        let Some(r) = m.drawn.get() else {
            return;
        };
        let [w, h] = m.rt.player.lib.size;
        let (bw, bh) = (r[2] - r[0], r[3] - r[1]);
        if bw <= 0.0 || bh <= 0.0 {
            return;
        }
        if let Some(p) = state.pointer {
            let at = [(p[0] - r[0]) * w / bw, (p[1] - r[1]) * h / bh];
            m.rt.mouse_move(at);
        }
        for &down in &state.clicks {
            let at = m.rt.avm.mouse;
            log.push(format!(
                "{swf}: mouse {} at [{:.0}, {:.0}], over {:?}",
                if down { "down" } else { "up" },
                at[0],
                at[1],
                m.rt.button_at(at)
                    .map(|b| flash::avm1::target_path(&m.rt.player, b))
            ));
            // `M2_MOUSE_TRACE`: the buttons there are, when a press finds
            // none under the pointer.
            if down && m.rt.button_at(at).is_none() && std::env::var_os("M2_MOUSE_TRACE").is_some()
            {
                for (path, shown, b) in m.rt.buttons() {
                    log.push(format!("  button {path} shown {shown} box {b:?}"));
                }
            }
            if down {
                m.rt.mouse_down();
            } else {
                m.rt.mouse_up();
            }
        }
    }

    /// The variable `name` on movie `swf`'s root, if it is a boolean.
    fn movie_bool(&mut self, swf: &str, name: &str) -> Option<bool> {
        let m = self.flash.values_mut().find(|m| m.swf == swf)?;
        let root = flash::avm1::Value::Object(m.rt.avm.clip_object(m.rt.player.root));
        match m.rt.avm.get(&mut m.rt.player, &root, name) {
            flash::avm1::Value::Bool(b) => Some(b),
            _ => None,
        }
    }

    /// A fanfare movie is playing (`fanfare_mission`...).
    fn fanfare_up(&self) -> bool {
        self.flash.values().any(|m| m.swf.contains("fanfare"))
    }

    /// The pause menu's movie is playing.
    fn pause_up(&self) -> bool {
        self.flash.values().any(|m| m.swf == PAUSE_MOVIE)
    }

    /// The engine's calls into the pause menu's movie: set up as it
    /// starts (again when the scripts load it again), then its answers.
    fn pause_calls(&mut self, state: &HudState, log: &mut Vec<String>) {
        let movie = self
            .flash
            .iter()
            .find(|(_, m)| m.swf == PAUSE_MOVIE)
            .map(|(&id, m)| (id, m.restarts));
        if movie != self.pause_movie {
            self.pause_movie = movie;
            self.pause_native.reset();
        }
        let Some((id, _)) = movie else {
            return;
        };
        let calls = self.pause_native.calls(&state.pause_settings);
        if let Some(m) = self.flash.get_mut(&id) {
            for (name, args) in calls {
                log.push(format!(
                    "pause: the engine calls {name}({} args)",
                    args.len()
                ));
                m.rt.call(name, &args);
            }
        }
    }

    /// The scripts' calls to the PC's library ([`LTI_LIBRARY`]) and the
    /// pause movie's commands no script heard, for the engine.
    fn pause_answers(&mut self, log: &mut Vec<String>) {
        for (name, arg) in take_library_calls(self.mission.lua()) {
            // Its Aspect Ratio says only that it switched (as the item is
            // clicked, and again as the changes are kept): which it is
            // now is the movie's own `videoModeVar`.
            if name == "LTIVideoSwitchMode"
                && let Some(wide) = self.movie_bool(PAUSE_MOVIE, "videoModeVar")
            {
                self.pause_native.set_wide(wide);
            }
            let answered = self.pause_native.library(&name, &arg);
            log.push(format!(
                "pause: LTILibName.{name}({arg}){}",
                if answered { "" } else { " not answered" }
            ));
        }
        let pause = self.pause_movie.map(|(id, _)| id);
        for (id, command, arg) in std::mem::take(&mut self.unheard_by) {
            // Any movie's sounds are the engine's (the PDA map's pan and
            // zoom loops: playtest 4); the rest only the pause movie's.
            let heard = (Some(id) == pause || is_sound_command(&command))
                && self.pause_native.command(&command, &arg);
            if heard {
                log.push(format!("pause: the engine hears {command}({arg})"));
            }
        }
    }

    /// The player's options the HUD's own scripts read (`Sys.*`), as they
    /// change.
    fn tell_options(&mut self, s: &crate::pause::Settings, log: &mut Vec<String>) {
        let told = (s.subtitles, s.tutorials, s.rumble, s.invert_pad);
        if self.options_told == Some(told) {
            return;
        }
        self.options_told = Some(told);
        self.mission.set_subtitles(s.subtitles);
        self.mission.set_tutorials(s.tutorials);
        let set = format!(
            "rawset(Sys, \"RumbleEnabled\", function() return {} end)
             rawset(Sys, \"YAxisInverted\", function() return {} end)",
            s.rumble, s.invert_pad
        );
        if let Err(e) = self.mission.run(&set, "m2-hud options") {
            log.push(format!("options: {e}"));
        }
    }

    fn play_flash(&mut self, dt: f64, log: &mut Vec<String>) {
        let lua = self.mission.lua();
        let Some(screen) = gui_runtime::screen(lua) else {
            return;
        };
        let names = gui_runtime::script_names(lua);
        let wanted: BTreeMap<u32, (String, u32, bool)> = screen
            .borrow()
            .widgets()
            .filter(|(_, w)| w.kind == gui_runtime::Kind::Flash)
            .filter_map(|(id, w)| {
                let swf = movie_name(w.swf.as_deref()?);
                // The radar's zones are drawn natively (see `Zone`).
                (!NATIVE_MOVIES.contains(&swf.as_str()))
                    .then_some((id, (swf, w.flash_restarts, w.flash_paused)))
            })
            .collect();
        self.flash.retain(|id, m| {
            wanted
                .get(id)
                .is_some_and(|(swf, restarts, _)| *swf == m.swf && *restarts == m.restarts)
        });
        for (&id, (swf, restarts, _)) in &wanted {
            if self.flash.contains_key(&id) {
                continue;
            }
            let lib = self.movies.borrow_mut().library(swf);
            let Some(lib) = lib else {
                log.push(format!("flash: widget {id}: no movie {swf}"));
                // Not asked again until the scripts set it again.
                continue;
            };
            let mut rt = avm1::Runtime::new(flash::Player::new(Rc::clone(&lib)));
            // `M2_AVM1_TRACE=topbar` logs what the movies named so run.
            rt.avm.trace = std::env::var("M2_AVM1_TRACE")
                .is_ok_and(|t| !t.is_empty() && swf.contains(t.as_str()));
            // Text ids the movies set themselves (`[0x129ee2f1]`) show
            // their text, as the engine's translator has them.
            let strings = Rc::clone(&screen);
            rt.player.translate = Some(flash::Translator(Rc::new(move |t: &str| {
                strings
                    .try_borrow()
                    .map_or_else(|_| t.to_owned(), |s| s.display_text(t))
            })));
            let movies = Rc::clone(&self.movies);
            rt.set_loader(Box::new(move |name: &str| {
                movies.borrow_mut().library(&movie_name(name))
            }));
            log.push(format!(
                "flash: widget {id} ({}) plays {swf}, {} images, {} not drawn {:?}",
                names.get(&id).map_or("", String::as_str),
                lib.images.len(),
                lib.missing_images.len(),
                lib.missing_images,
            ));
            self.flash.insert(
                id,
                FlashMovie {
                    swf: swf.clone(),
                    restarts: *restarts,
                    rt,
                    seen: None,
                    since_status: f64::INFINITY,
                    levels: Vec::new(),
                    drawn: std::cell::Cell::new(None),
                },
            );
        }
        if !self.flash.is_empty() && self.movies.borrow().fonts.is_none() {
            let summary = self.movies.borrow_mut().fonts().summary();
            log.push(format!("fonts: {summary}"));
        }
        let calls = screen.borrow_mut().take_flash_calls();
        for (id, name, args) in calls {
            let Some(m) = self.flash.get_mut(&id) else {
                continue;
            };
            // The engine hands the movies their text looked up
            // (`"[VzaCon001.Title]"` as its words), as it does for text
            // widgets.
            let args: Vec<avm1::Value> = args
                .into_iter()
                .map(|a| match a {
                    FlashArg::Nil => avm1::Value::Undefined,
                    FlashArg::Bool(b) => avm1::Value::Bool(b),
                    FlashArg::Number(n) => avm1::Value::Number(n),
                    FlashArg::Text(t) if t.contains('[') => {
                        avm1::Value::Str(screen.borrow().display_text(&t).into())
                    }
                    FlashArg::Text(t) => avm1::Value::Str(t.into()),
                })
                .collect();
            log.push(format!("flash {}: {name}({} args)", m.swf, args.len()));
            m.rt.call(&name, &args);
        }
        for (code, down) in std::mem::take(&mut self.keys) {
            log.push(format!(
                "flash: key {code} {}",
                if down { "down" } else { "up" }
            ));
            // The pause menu over all holds them; else a fanfare.
            let pause = self.pause_up();
            let fanfare = self.fanfare_up();
            for m in self.flash.values_mut().filter(|m| {
                if pause {
                    m.swf == PAUSE_MOVIE
                } else {
                    !fanfare || m.swf.contains("fanfare")
                }
            }) {
                if down {
                    m.rt.key_down(code);
                } else {
                    m.rt.key_up(code);
                }
            }
        }
        let mut commands = Vec::new();
        let mut status = Vec::new();
        for (&id, m) in &mut self.flash {
            let paused = wanted.get(&id).is_some_and(|w| w.2);
            if !paused {
                m.rt.step(dt as f32);
            }
            m.since_status += dt;
            if m.since_status >= 1.0 {
                m.since_status = 0.0;
                status.push((id, paused));
            }
            m.rt.sync_text();
            for line in m.rt.avm.log.drain(..) {
                log.push(format!("flash {}: {line}", m.swf));
            }
            commands.extend(m.rt.take_commands().into_iter().map(|c| (id, c)));
            let levels: Vec<_> =
                m.rt.player
                    .levels
                    .iter()
                    .filter(|(l, _)| **l != 0)
                    .map(|(&l, &r)| (l, r))
                    .collect();
            if levels != m.levels {
                // topbar's `UpdateBlip` places icons on `_level1`'s map,
                // so blips sent before it loaded go again.
                if levels.iter().any(|l| !m.levels.contains(l)) {
                    screen.borrow_mut().resend_pda_blips(id);
                }
                m.levels = levels;
            }
        }
        {
            let movies = self.movies.borrow();
            for (id, paused) in status {
                if let Some(m) = self.flash.get(&id) {
                    log.push(flash_status(m, paused, movies.fonts.as_ref()));
                }
            }
        }
        for (id, (command, arg)) in commands {
            let character = self.native.character();
            let heard = match character {
                Some(who) if self.mode == Mode::Shell && command == "newGame" => {
                    log.push(format!(
                        "shell: newGame({arg}) as the profile's character, {who}"
                    ));
                    crate::shell::start_new_game(lua, id, who)
                }
                _ => gui_runtime::flash_command(lua, id, &command, &arg),
            };
            match heard {
                Ok(true) => {}
                Ok(false) => {
                    log.push(format!(
                        "flash: widget {id}: fscommand {command}({arg}) unheard"
                    ));
                    if self.mode == Mode::Shell {
                        self.unheard.push((command, arg));
                    } else {
                        self.unheard_by.push((id, command, arg));
                    }
                }
                Err(e) => log.push(format!("flash: widget {id}: {command}: {e}")),
            }
        }
    }

    /// Logs when a movie comes on screen, leaves it, moves, or starts or
    /// stops drawing anything.
    fn log_flash_shown(
        &mut self,
        items: &[DrawItem],
        shown: &impl Fn(&&DrawItem) -> bool,
        log: &mut Vec<String>,
    ) {
        let mut rects: HashMap<u32, ([f32; 4], bool)> = HashMap::new();
        for item in items.iter().filter(shown) {
            if let DrawItem::Flash {
                widget,
                rect,
                fullscreen,
                ..
            } = item
            {
                rects.insert(*widget, (*rect, *fullscreen));
            }
        }
        let movies = self.movies.borrow();
        for (id, m) in &mut self.flash {
            let at = rects.get(id).copied();
            let rect = at.map(|(r, _)| r);
            let triangles = match at {
                Some((r, fit)) => {
                    let mut out = Vec::new();
                    let mut want = Vec::new();
                    let r = if fit {
                        show_all(r, m.rt.player.lib.size)
                    } else {
                        r
                    };
                    flash_prims(m, &movies, r, &mut out, &mut want);
                    out.len()
                }
                None => 0,
            };
            let seen = (rect.map(|r| r.map(|v| v.round() as i32)), triangles > 0);
            if m.seen.as_ref() != Some(&seen) {
                log.push(format!(
                    "flash {} (widget {id}): {}, {triangles} triangles",
                    m.swf,
                    rect.map_or("not drawn".to_owned(), |r| format!("at {r:.0?}"))
                ));
                m.seen = Some(seen);
            }
        }
    }

    /// Hands the mission's HUD calls to the HUD's interface ([`HUD_CALL`]),
    /// and the hijack scripts' to the HUD's modules ([`GUI_CALL`]).
    fn pass_calls(&mut self, s: &HudState, log: &mut Vec<String>) {
        let lua = self.mission.lua();
        for c in &s.gui_calls {
            let name = format!("{}.{}", c.module, c.method);
            let done = (|| {
                let mut args = vec![
                    Value::String(lua.create_string(&c.module)?),
                    Value::String(lua.create_string(&c.method)?),
                    Value::Integer(c.args.len().max(1) as mlua::Integer),
                ];
                // Each takes the player first: she is the HUD's only one,
                // whatever the hijack's state called her.
                args.push(guid_value(PLAYER));
                for a in c.args.iter().skip(1) {
                    args.push(a.to_lua(lua)?);
                }
                lua.load(GUI_CALL)
                    .set_name("=m2-gui call")
                    .call::<bool>(mlua::MultiValue::from_vec(args))
            })();
            log.push(match done {
                Ok(true) => format!("gui call {name}"),
                Ok(false) => format!("gui call {name}: the HUD has no such function"),
                Err(e) => format!("gui call {name}: {e}"),
            });
        }
        for c in s.hud_calls.iter().filter(|c| c.group == "DialogBox") {
            let done = c.data.to_lua(lua).and_then(|data| {
                lua.load(DIALOG_CALL)
                    .set_name("=m2-dialog call")
                    .call::<bool>((c.method.as_str(), data, c.seq))
            });
            log.push(match done {
                Ok(true) => format!("dialog box {} {}: {:?}", c.seq, c.method, c.data),
                Ok(false) => format!(
                    "dialog box {} {}: the HUD has no dialog box",
                    c.seq, c.method
                ),
                Err(e) => format!("dialog box {} {}: {e}", c.seq, c.method),
            });
        }
        for &key in &s.dialog_keys {
            let fired = lua
                .load(CONTROLLER_PRESS)
                .set_name("=m2-controller press")
                .call::<Value>(key)
                .and_then(|event| match event {
                    Value::Table(t) => gui_runtime::fire(lua, "ControllerInput", t),
                    _ => Ok(0),
                });
            // Where the box's selection is now (the wager's digit box moves
            // it with left and right): index and of how many.
            let at: (Option<i64>, Option<i64>) = lua
                .load(
                    "for _, b in pairs(__m2_dialog_boxes or {}) do \
                       local c = type(b) == 'table' and b.CustomData \
                       if type(c) == 'table' and c.nSelectedIndex then \
                         return c.nSelectedIndex, type(c.oSelectableList) == 'table' \
                           and #c.oSelectableList or nil \
                       end \
                     end",
                )
                .eval()
                .unwrap_or((None, None));
            log.push(match fired {
                Ok(n) if at.0.is_some() => {
                    format!(
                        "dialog key {key}: {n} handlers, selection {:?} of {:?}",
                        at.0, at.1
                    )
                }
                Ok(n) => format!("dialog key {key}: {n} handlers"),
                Err(e) => format!("dialog key {key}: {e}"),
            });
        }
        for c in s.hud_calls.iter().filter(|c| c.group != "DialogBox") {
            let name = format!("{}.{}", c.group, c.method);
            let done = c.data.to_lua(lua).and_then(|data| {
                lua.load(HUD_CALL).set_name("=m2-hud call").call::<bool>((
                    c.group.as_str(),
                    c.method.as_str(),
                    data,
                    c.callback.then_some(c.seq),
                    c.seq,
                ))
            });
            log.push(match done {
                // The tutorial box's: with what it says, as the box has
                // another writer too (`Net.SetTutorialMessage`, [`TUTORIAL`]).
                Ok(true) if c.group == "Tutorial" => format!("hud call {name} {:?}", c.data),
                Ok(true) => format!("hud call {name}"),
                Ok(false) => format!("hud call {name}: the HUD has no such method"),
                Err(e) => format!("hud call {name}: {e}"),
            });
        }
    }

    /// Fires the GUI events for what changed since the last state.
    fn feed(&mut self, s: &HudState) -> mlua::Result<()> {
        let lua = self.mission.lua();
        if let Some((can, cost)) = s.medevac {
            lua.load(MEDEVAC)
                .set_name("=m2-hud medevac")
                .call::<()>((can, cost))?;
        }
        // The save icon (the HUD's own skull and "Saving", which its
        // loading-screen module shows on `ShowSaveIcon` and takes down on
        // `HideSaveIcon`), as the game saves.
        if s.saving != self.told.saving {
            self.told.saving = s.saving;
            let event = if s.saving {
                "ShowSaveIcon"
            } else {
                "HideSaveIcon"
            };
            gui_runtime::fire(lua, event, ())?;
        }
        let first = self.told.hp.is_none();
        if first {
            let t = table(
                lua,
                [("uGuid", guid_value(PLAYER)), ("nValue", Value::Integer(0))],
            )?;
            fire(lua, "PlayerCashChanged", t)?;
            lua.load(FUEL).set_name("=m2-hud fuel").call::<()>(0)?;
            let t = table(lua, [("nX", Value::Integer(0)), ("nY", Value::Integer(0))])?;
            fire(lua, "GuiReticlePositionChange", t)?;
        }
        // Her cash, whenever the scripts' changes.
        let cash = s.cash.map(|c| c.round() as i64);
        if cash.is_some() && cash != self.told.cash {
            self.told.cash = cash;
            let n = cash.unwrap_or(0) as f64;
            lua.globals().set("__m2_cash", n)?;
            let t = table(
                lua,
                [("uGuid", guid_value(PLAYER)), ("nValue", Value::Number(n))],
            )?;
            fire(lua, "PlayerCashChanged", t)?;
        }
        // The satellite looks down with north up: its compass reads the
        // camera's heading as the view opens.
        let heading = if s.support.satellite.is_some() {
            0.0
        } else {
            s.heading
        };
        if let Some(screen) = gui_runtime::screen(lua) {
            let mut screen = screen.borrow_mut();
            screen.set_camera_heading(heading);
            screen.set_pad(s.pad);
            screen.set_driving(s.in_vehicle);
        }
        // The keys the texts' control words name, as the bindings are (and
        // again should they change): before any text this frame is set.
        if self.told.key_names.as_ref() != Some(&s.key_names) {
            self.told.key_names = Some(s.key_names.clone());
            // The words the fonts have no picture matched for, as their
            // keys, and what the PC must look at to match them.
            let keyed: Vec<String> = s
                .key_names
                .iter()
                .filter(|(w, _)| gui_runtime::button_glyph(w).is_none())
                .map(|(w, k)| format!("[{w}] as [{k}]"))
                .collect();
            if !keyed.is_empty() {
                eprintln!(
                    "hud text: no button picture matched for {}: on the PC, which of \
                     english_18's button page (pc\\common_18_buttons) glyphs {} (or past \
                     U+E013) is each",
                    keyed.join(", "),
                    gui_runtime::UNMATCHED_GLYPHS
                        .iter()
                        .map(|c| format!("U+{:04X}", u32::from(*c)))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            if let Some(screen) = gui_runtime::screen(lua) {
                screen
                    .borrow_mut()
                    .set_key_names(s.key_names.iter().cloned());
            }
        }
        // Where she is for the minimap, each frame as the engine does: its
        // handler hands it on to `MinimapUpdate`, with the heading as the
        // rotation (the map turns with the camera).
        if let Some([x, y, z]) = s.position {
            fire(lua, "GuiMinimapUpdate", (x, y, z, s.heading))?;
        }
        // Into the vehicle before her health is told, as the health box
        // picks its badge then.
        if self.told.vehicle_templates.as_ref() != Some(&s.vehicle_templates) {
            self.told.vehicle_templates = Some(s.vehicle_templates.clone());
            let mut world = self.mission.world().borrow_mut();
            let labels = s
                .vehicle_templates
                .iter()
                .find_map(|t| world.template_labels.get(&t.to_ascii_lowercase()))
                .cloned()
                .unwrap_or_default();
            if let Some(v) = world.objects.get_mut(&VEHICLE_GUID) {
                v.label_hashes = labels;
            }
            if let Some(me) = world.objects.get_mut(&m2_game::mission::PLAYER) {
                me.vehicle = s.in_vehicle.then_some(VEHICLE_GUID);
            }
            self.told.hp = None;
        }
        let hp = (s.hp.round() as i32, s.in_vehicle);
        if self.told.hp != Some(hp) {
            self.told.hp = Some(hp);
            fire(lua, "GuiHealthUpdate", (hp.0, 100, hp.1))?;
        }
        // Before the vehicle box hears of the vehicle: it shows the gauge
        // then if disguises work.
        self.mission.world().borrow_mut().vehicle_disguise = s.vehicle_disguise;
        if self.told.vehicle.as_ref() != Some(&s.vehicle) {
            self.told.vehicle = Some(s.vehicle.clone());
            let (name, faction) = match &s.vehicle {
                Some(label) => {
                    let (name, faction) = vehicle_name(label);
                    // The name the game shows, its template's
                    // `LocalizedName` string ("Chopper"), where the
                    // strings hold one; else the label's.
                    let strings = &self.mission.strings().strings;
                    let name = s
                        .vehicle_names
                        .iter()
                        .find_map(|k| strings.get(k).cloned())
                        .unwrap_or(name);
                    (
                        Value::String(lua.create_string(name)?),
                        faction.map_or(Value::Nil, guid_value),
                    )
                }
                None => (Value::Boolean(false), Value::Nil),
            };
            fire(lua, "GuiVehicleNameUpdate", (name, faction))?;
        }
        // The disguise gauge in the vehicle box shows only while disguises
        // work (`Player.GetVehicleDisguise`); the engine tells it how much
        // of her disguise is left and whether it holds.
        if self.told.disguise != Some(s.disguise) {
            self.told.disguise = Some(s.disguise);
            self.mission.world().borrow_mut().disguise = s.disguise;
            if let Some((level, held)) = s.disguise {
                fire(lua, "GuiVehicleDisguiseUpdate", (f64::from(level), held))?;
            }
        }
        // A faction zone's word to the HUD, sent on as the zone's script
        // sent it: the trespass icon by the radar and its map label.
        if self.told.trespass != s.trespass {
            self.told.trespass = s.trespass.clone();
            lua.load(TRESPASS_EVENT)
                .set_name("=m2-hud trespass")
                .call::<()>((s.trespass.inside, s.trespass.faction.clone()))?;
        }
        let holding = s.support.designator.is_some();
        // In a tank, the gun chosen: its machine gun, else its cannon.
        let mg = s.in_vehicle && s.machine_gun.is_some();
        let cannon = s.in_vehicle && s.cannon.is_some() && !mg;
        // Out of a vehicle, she has her gun in hand again, and the HUD is
        // told so as for a change of gun, and her rounds after it: the
        // gun box (shown by `GuiWeaponEquippedUpdate`) had been shown only
        // once, on the boat at the start, and never again on foot, as no
        // gun was changed after (PC playtest, 2026-10-08).
        self.told.seat(s.in_vehicle);
        let gun = s.gun.as_ref().map(|g| g.0);
        if self.told.weapon != Some((s.weapon, holding, cannon, mg)) || self.told.gun != gun {
            self.told.weapon = Some((s.weapon, holding, cannon, mg));
            self.told.gun = gun;
            // The gun in her rifle's place shows as its `WeaponUI` says,
            // and the gun box names it (playtest 4: her pistol showed the
            // carbine's picture and name, her shotgun the bullets'
            // reticle).
            let (rifle_icon, rifle_reticle, rifle_cross) = match &s.gun {
                Some((_, name, ui)) => {
                    if let Some(o) = self
                        .mission
                        .world()
                        .borrow_mut()
                        .objects
                        .get_mut(&RIFLE_GUID)
                    {
                        o.name.clone_from(name);
                    }
                    eprintln!(
                        "hud: the gun box shows {name} (icon {:08x}, reticle {:08x}, crosshair {})",
                        ui.icon, ui.reticle, ui.crosshair
                    );
                    (by_hash(ui.icon), by_hash(ui.reticle), ui.crosshair)
                }
                None => {
                    if let Some(o) = self
                        .mission
                        .world()
                        .borrow_mut()
                        .objects
                        .get_mut(&RIFLE_GUID)
                    {
                        RIFLE_NAME.clone_into(&mut o.name);
                    }
                    (RIFLE_ICON.to_owned(), RIFLE_RETICLE.to_owned(), true)
                }
            };
            let (icon, guid, reticle, crosshair) = match s.weapon {
                Weapon::Rifle => (rifle_icon, RIFLE_GUID, rifle_reticle, rifle_cross),
                Weapon::Rpg => (RPG_ICON.to_owned(), RPG_GUID, RPG_RETICLE.to_owned(), true),
                Weapon::GrenadeLauncher => (
                    LAUNCHER_ICON.to_owned(),
                    LAUNCHER_GUID,
                    LAUNCHER_RETICLE.to_owned(),
                    true,
                ),
            };
            // The designator up in her hands: the gun box names it; the
            // reticle stays the gun's. In a tank, its cannon.
            let (icon, guid, reticle, crosshair) =
                if let Some((ui, _, _)) = s.machine_gun.filter(|_| mg) {
                    // The tank's machine gun as its own `WeaponUI` shows it
                    // (playtest 5: chosen, the gun box still showed the
                    // cannon).
                    (
                        by_hash(ui.icon),
                        CANNON_GUID,
                        by_hash(ui.reticle),
                        ui.crosshair,
                    )
                } else if cannon {
                    (
                        CANNON_ICON.to_owned(),
                        CANNON_GUID,
                        CANNON_RETICLE.to_owned(),
                        false,
                    )
                } else if holding {
                    (
                        DESIGNATOR_ICON.to_owned(),
                        DESIGNATOR_GUID,
                        reticle,
                        crosshair,
                    )
                } else {
                    (icon, guid, reticle, crosshair)
                };
            // What `MrxGuiHudReticle.HandleReticleGunSwitchEvent` reads:
            // without a type it leaves the reticle as it was. No target
            // health type: that bar shows only on a target. The crosshair
            // for every gun but the cannon (inferred, as the textures are:
            // the tank's none is its own template's).
            let t = table(
                lua,
                [
                    ("uNewCurrentGun", Value::String(lua.create_string(&icon)?)),
                    ("uNewCurrentGunGuid", guid_value(guid)),
                    ("sReticleType", Value::String(lua.create_string("Normal")?)),
                    (
                        "uReticleTexture",
                        Value::String(lua.create_string(&reticle)?),
                    ),
                    ("bReticleCrosshair", Value::Boolean(crosshair)),
                ],
            )?;
            fire(lua, "GuiWeaponEquippedUpdate", t)?;
        }
        let ammo = ammo(
            s.weapon,
            s.fired,
            s.picked,
            (s.rifle, s.rifle_clip),
            s.launcher,
            match s.machine_gun {
                Some((_, left, carried)) if mg => Some((left, carried)),
                _ => s.cannon,
            },
            s.grenades,
        );
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
        // (target, centre x, y, spread x, y, health, maximum), again as the
        // aim moves too, not only as who is under it changes: the centre
        // is the aim point, -1..=1 as `GuiReticlePositionChange` has it
        // (inferred: the handler isn't read here), for the crosshair to sit
        // where shots go rather than at the layout's middle. No spread (0,
        // inferred: the guns' spreads aren't read). No health fades the
        // target-health arc (opaque as laid out); a health shows that share
        // of the target's own most.
        if moved || self.told.target != Some(s.target) {
            self.told.target = Some(s.target);
            let number = |v: f32| Value::Number(v.into());
            let (x, y) = s
                .reticle
                .map_or((Value::Nil, Value::Nil), |[x, y]| (number(x), number(y)));
            let (kind, hp, most) = match s.target {
                Some((kind, hp, most)) => (
                    kind,
                    Value::Integer(hp as mlua::Integer),
                    Value::Integer(most as mlua::Integer),
                ),
                None => (0, Value::Nil, Value::Nil),
            };
            fire(lua, "GuiReticleUpdate", (kind, x, y, 0, 0, hp, most))?;
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
            // A region of a layer this view hasn't loaded (a contract's
            // faction zone): taken in, outline and all.
            self.mission.adopt(&r.name);
            let [red, green, blue, alpha] = r.colour;
            lua.load(ADD_REGION)
                .set_name("=m2-hud region")
                .call::<()>((r.name.as_str(), red, green, blue, alpha, r.invert))?;
        }
        self.told.regions = Some(s.radar_regions.clone());
        Ok(())
    }

    /// Feeds the binocular screen the engine's events, as
    /// `MrxGuiBinoculars` takes them ([`crate::binoculars`]):
    /// `BinocularsEnter` as they come up, `BinocularsUpdate` with the
    /// heading, pitch and zoom while they are up, `BinocularsExit` as they
    /// go down. The faction badge is told a number (none: put away) and the
    /// name under the reticle is not told.
    fn feed_binoculars(&mut self, s: &HudState, log: &mut Vec<String>) -> mlua::Result<()> {
        let lua = self.mission.lua();
        let was = self.told.binoculars;
        if was.is_none() && s.binoculars.is_none() {
            return Ok(());
        }
        let event = |kind: &str| -> mlua::Result<Table> {
            let t = lua.create_table()?;
            t.raw_set("EventType", kind)?;
            t.raw_set("uPlayerGuid", guid_value(PLAYER))?;
            Ok(t)
        };
        match (was, s.binoculars) {
            (None, Some(_)) => {
                let t = event("BinocularsEnter")?;
                t.set("bSniper", false)?;
                let n = gui_runtime::fire(lua, "BinocularsEnter", t)?;
                log.push(format!("binoculars: up ({n} handlers)"));
            }
            (Some(_), None) => {
                let n = gui_runtime::fire(lua, "BinocularsExit", event("BinocularsExit")?)?;
                log.push(format!("binoculars: down ({n} handlers)"));
            }
            _ => {}
        }
        // Each frame while they are up, as the engine does: the meters'
        // handlers answer only once the frame has closed in
        // (`_FinishEnter` enables them).
        if let Some(v) = s.binoculars {
            let t = event("BinocularsUpdate")?;
            t.set("nCameraHeading", f64::from(v.heading))?;
            t.set("nPitch", f64::from(v.pitch))?;
            t.set("nZoomLevel", f64::from(v.zoom))?;
            t.set("FactionTexture", 0)?;
            let n = gui_runtime::fire(lua, "BinocularsUpdate", t)?;
            if self.told.binoculars_logged != Some((v.zoom, n)) {
                self.told.binoculars_logged = Some((v.zoom, n));
                // Where the handlers put the meters' pointers: the
                // pointer's middle from its meter's, a share of the
                // meter's length, against the share their sums give.
                let names = gui_runtime::script_names(lua);
                let mut at = Vec::new();
                if let Some(screen) = gui_runtime::screen(lua) {
                    let screen = screen.borrow();
                    for (id, name) in &names {
                        let (across, want) = match name.as_str() {
                            "top pointer" => (true, binoculars::heading_meter(v.heading)),
                            "left pointer" => (false, binoculars::pitch_meter(v.pitch)),
                            "right pointer" => (false, binoculars::zoom_meter(f32::from(v.zoom))),
                            _ => continue,
                        };
                        let Some(bar) = screen.widget(*id).and_then(|w| w.parent) else {
                            continue;
                        };
                        let (p, b) = (screen.rect(*id), screen.rect(bar));
                        let i = if across { 0 } else { 1 };
                        let mid = |r: [f32; 4]| (r[i] + r[i + 2]) * 0.5;
                        let share = (mid(p) - mid(b)) / (b[i + 2] - b[i]).max(1e-3);
                        at.push(format!("{name} {share:.3} (sums {want:.3})"));
                    }
                }
                log.push(format!(
                    "binoculars: zoom {}x heading {:.1} pitch {:.1} ({n} handlers): {}",
                    v.zoom,
                    v.heading,
                    v.pitch,
                    at.join(", ")
                ));
            }
        }
        self.told.binoculars = s.binoculars;
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
        // The tutorial box: the mission's line; with none, the satellite's
        // help while its view is open.
        let tutorial = match (&s.tutorial, sup.satellite, &sup.wheel) {
            (Some(line), _, _) => Some(line.clone()),
            (None, Some(_), Some(_)) => Some(WHEEL_HELP.to_owned()),
            (None, Some(_), None) => Some(VIEW_HELP.to_owned()),
            (None, None, _) => None,
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

    /// The gun box as the scripts left it, logged as it changes: each
    /// widget showing one of the guns' icons (the box's picture of the
    /// gun in hand), with whatever of it and its parents keeps it from
    /// drawing (hidden, asleep, inactive, see-through), on foot or in a
    /// vehicle. Tells a PC run what keeps the box away on foot, should it
    /// still be.
    fn gun_box(&mut self, s: &HudState, log: &mut Vec<String>) {
        let Some(screen) = gui_runtime::screen(self.mission.lua()) else {
            return;
        };
        let screen = screen.borrow();
        let icons = [
            RIFLE_ICON,
            RPG_ICON,
            LAUNCHER_ICON,
            DESIGNATOR_ICON,
            CANNON_ICON,
        ];
        let mut seen: Vec<String> = screen
            .widgets()
            .filter_map(|(id, w)| {
                let icon = w.texture.as_deref()?;
                icons.iter().any(|i| icon.eq_ignore_ascii_case(i)).then(|| {
                    let mut kept = Vec::new();
                    let mut at = Some(id);
                    // Up the parents (bounded, should one point back down).
                    for _ in 0..64 {
                        let Some(i) = at else { break };
                        let Some(x) = screen.widget(i) else {
                            kept.push(format!("{i} gone"));
                            break;
                        };
                        let flags: String = [
                            (!x.visible, " hidden"),
                            (x.sleeping, " asleep"),
                            (x.inactive, " inactive"),
                            (x.colour[3] <= 0.0, " clear"),
                        ]
                        .iter()
                        .filter(|(on, _)| *on)
                        .map(|(_, f)| *f)
                        .collect();
                        if !flags.is_empty() {
                            kept.push(format!("{i} {:?}{flags}", x.name));
                        }
                        at = x.parent;
                    }
                    format!(
                        "{id} {icon} {}",
                        if kept.is_empty() {
                            "shown".to_owned()
                        } else {
                            format!("kept by {}", kept.join(", "))
                        }
                    )
                })
            })
            .collect();
        seen.sort();
        let line = format!(
            "{}: {}",
            if s.in_vehicle {
                "in a vehicle"
            } else {
                "on foot"
            },
            if seen.is_empty() {
                "no gun icon widget".to_owned()
            } else {
                seen.join("; ")
            }
        );
        if line != self.told.gun_box {
            log.push(format!("gun box: {line}"));
            self.told.gun_box = line;
        }
    }

    /// Poses the timing wheel's widgets as the play has the wheel, after
    /// the scripts have stepped (see [`SATELLITE_WHEEL`]). What it found,
    /// with the overlay's and the minigame's flags, goes in the log as
    /// that changes: the wheel can be posed and still not drawn.
    fn pose_wheel(&mut self, s: &HudState, log: &mut Vec<String>) -> mlua::Result<()> {
        if s.support.satellite.is_none() {
            return Ok(());
        }
        let lua = self.mission.lua();
        let call = lua.load(SATELLITE_WHEEL).set_name("=m2-hud wheel");
        let (found, overlay, minigame): (Option<String>, Option<u32>, Option<u32>) =
            match &s.support.wheel {
                Some(w) => {
                    let inside = support::sector(w.angle).map(|i| i + 1);
                    call.call((w.size, w.angle.to_degrees(), inside, w.mark))?
                }
                None => call.call(())?,
            };
        let flags = |id: Option<u32>| {
            let screen = gui_runtime::screen(lua)?;
            let screen = screen.borrow();
            let w = screen.widget(id?)?;
            Some(format!(
                "{} visible {} asleep {} inactive {}",
                id?, w.visible, w.sleeping, w.inactive
            ))
        };
        let seen = format!(
            "{}; overlay {}; minigame {}",
            found.as_deref().unwrap_or("nothing"),
            flags(overlay).unwrap_or_else(|| "-".to_owned()),
            flags(minigame).unwrap_or_else(|| "-".to_owned()),
        );
        if seen != self.wheel_seen {
            log.push(format!("wheel: {seen}"));
            self.wheel_seen = seen;
        }
        Ok(())
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
                let hash = texture_hash(texture);
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
            DrawItem::Flash {
                widget,
                rect,
                fullscreen,
                ..
            } => {
                if let Some(m) = self.flash.get(widget) {
                    // The pause menu fills the screen, its stage stretched
                    // across (the PC footage: its 850 x 720 stage spans
                    // the 1280 x 720 picture, 1.5 times wider than high).
                    let fit = *fullscreen && m.swf != PAUSE_MOVIE;
                    let rect = if fit {
                        show_all(*rect, m.rt.player.lib.size)
                    } else {
                        *rect
                    };
                    m.drawn.set(Some(rect));
                    flash_prims(m, &self.movies.borrow(), rect, out, want);
                }
            }
            // Not drawn yet (see the module docs).
            DrawItem::Movie { .. } => {}
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
    widgets_below(screen, named)
}

/// The widgets `pick` picks by id, and everything under them.
fn widgets_below(screen: &gui_runtime::Screen, named: impl Fn(u32) -> bool) -> HashSet<u32> {
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

/// Whether any of the text `items` of the widgets `held` says something
/// (not only spaces).
fn says_something(items: &[DrawItem], held: &HashSet<u32>) -> bool {
    items.iter().any(|i| {
        matches!(i, DrawItem::Text { widget, text, .. }
            if held.contains(widget) && !text.trim().is_empty())
    })
}

/// The point a probe's variable (`M2_HUD_PROBE=x,y`) names, if set.
fn probe_point(var: &str) -> Option<[f32; 2]> {
    let v = std::env::var(var).ok()?;
    let (x, y) = v.split_once(',')?;
    Some([x.trim().parse().ok()?, y.trim().parse().ok()?])
}

/// A draw item's box (screen points).
fn rect_of(item: &DrawItem) -> [f32; 4] {
    match item {
        DrawItem::Quad { rect, .. }
        | DrawItem::Fill { rect, .. }
        | DrawItem::Text { rect, .. }
        | DrawItem::Flash { rect, .. }
        | DrawItem::Minimap { rect, .. }
        | DrawItem::Movie { rect, .. } => *rect,
    }
}

/// Whether `item`'s box (unturned) holds the point `at`.
fn holds(item: &DrawItem, [x, y]: [f32; 2]) -> bool {
    let r = rect_of(item);
    (r[0]..=r[2]).contains(&x) && (r[1]..=r[3]).contains(&y)
}

/// The HUD probe's line for `item`: what it draws (the texture, or the
/// font and text), its box, and its widget up through the parents, each
/// by id and name, with the flags that keep it from drawing.
fn probe_line(
    screen: &gui_runtime::Screen,
    names: &BTreeMap<u32, String>,
    item: &DrawItem,
) -> String {
    let what = match item {
        DrawItem::Quad { texture, .. } => format!("quad {texture}"),
        DrawItem::Fill { colour, .. } => format!("fill {colour:.2?}"),
        DrawItem::Text { font, text, .. } => format!("text {font} {text:?}"),
        DrawItem::Flash { swf, .. } => format!("flash {swf:?}"),
        DrawItem::Minimap { .. } => "minimap".to_owned(),
        DrawItem::Movie { file, .. } => format!("movie {file:?}"),
    };
    let mut chain = Vec::new();
    let mut at = Some(widget_of(item));
    // Up the parents (bounded, should a parent point back down).
    for _ in 0..64 {
        let Some(id) = at else { break };
        let w = screen.widget(id);
        let name = names
            .get(&id)
            .map(String::as_str)
            .or_else(|| w.map(|w| w.name.as_str()))
            .unwrap_or_default();
        let flags = w.map_or_else(
            || " gone".to_owned(),
            |w| {
                [
                    (!w.visible, " hidden"),
                    (w.sleeping, " asleep"),
                    (w.inactive, " inactive"),
                ]
                .iter()
                .filter(|(on, _)| *on)
                .map(|(_, f)| *f)
                .collect()
            },
        );
        chain.push(format!("{id} {name:?}{flags}"));
        at = w.and_then(|w| w.parent);
    }
    format!("{what} at {:.0?}: {}", rect_of(item), chain.join(" < "))
}

/// Clip, in the clip, stored, grenades, grenades stored: as
/// `GuiAmmoUpdate` takes them. The rifle's are the play's (`rifle`: in
/// the clip, stored, and the clip of the gun in her hands: the shotgun's
/// own, not the carbine's 30, or its full clip reads as low and its count
/// flashes red, playtest 5). Rockets picked up add to their store; they count
/// down from that and, run out, start again full. The grenade launcher's
/// are its own (`launcher`: in the clip, stored). In
/// a vehicle whose cannon runs out, its shells (`cannon`: left, carried)
/// count down alone: no clip (0) and no store (-1), as the engine sends a
/// gun without either, for which the gun box shows the one number (its
/// `Unreloadable Ammo`, as the footage's "40").
fn ammo(
    weapon: Weapon,
    fired: Fired,
    picked: Picked,
    (rifle, rifle_clip): (Rounds, u32),
    launcher: Option<Rounds>,
    cannon: Option<(u32, u32)>,
    grenades: u32,
) -> [i64; 5] {
    if let Some((left, _)) = cannon {
        return [0, i64::from(left), -1, i64::from(grenades), 0];
    }
    let ammo = match weapon {
        Weapon::Rifle => [rifle_clip, rifle.clip, rifle.stored, grenades, 0],
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
    };
    ammo.map(i64::from)
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
        multiply: false,
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
    if let Some(map) = movie_map(assets) {
        return Ok((
            Image {
                width: texture.width,
                height: width as u32,
                rgba: map.image(width, ISLAND),
            },
            map.roads,
        ));
    }
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

/// The radar movie's own map (see [`minimap::MovieMap`]), if it reads.
fn movie_map(assets: &mut AssetSource<BufReader<File>>) -> Option<minimap::MovieMap> {
    let data = assets.asset(name_hash(RADAR_MOVIE), MOVIE_TYPE).ok()?;
    let lib = flash::Library::new(&movie_from_asset(&data).ok()?);
    let (shape, mesh) = minimap::roads_shape(&lib)?;
    Some(minimap::MovieMap::new(shape, mesh))
}

/// How dark the scene goes behind the PDA (the black's alpha): the
/// footage's scene shows at about a quarter of its brightness round it.
const PDA_DIM: f32 = 0.85;
/// The PDA's Flash movies, in the order drawn: its frame and tabs
/// (`topbar.gfx`), then its map screen (`map.gfx`), the first screen it
/// opens on.
const PDA_MOVIES: [&str; 2] = ["topbar", "map"];

/// Flash movies the HUD draws natively instead.
const NATIVE_MOVIES: [&str; 1] = ["minimap"];

/// The movie of Scaleform's font library, which the movies' text fields
/// take their fonts from.
const FONT_LIBRARY: &str = "gfxfontlib";

/// The game's font movies (see `m2_assets::flash::text`).
const FONT_MOVIES: [&str; 4] = [
    "_normal_font",
    "_bold_font",
    "_italic_font",
    "_bold_italic_font",
];

/// The archive's Flash movies, read once each.
struct Movies {
    assets: AssetSource<BufReader<File>>,
    /// By name (see [`movie_name`]); None if missing or unreadable.
    libraries: HashMap<String, Option<Rc<flash::Library>>>,
    /// The font library's fonts, once read.
    fonts: Option<flash::FontBook>,
}

impl Movies {
    /// The movie called `name` (as [`movie_name`] gives it).
    fn library(&mut self, name: &str) -> Option<Rc<flash::Library>> {
        if let Some(l) = self.libraries.get(name) {
            return l.clone();
        }
        let lib = self
            .movie(name)
            .map(|movie| Rc::new(flash::Library::new(&movie)));
        self.libraries.insert(name.to_owned(), lib.clone());
        lib
    }

    fn movie(&mut self, name: &str) -> Option<m2_assets::gfx::Movie> {
        let data = self.assets.asset(name_hash(name), MOVIE_TYPE).ok()?;
        movie_from_asset(&data).ok()
    }

    /// The font library's fonts, read the first time.
    fn fonts(&mut self) -> &flash::FontBook {
        if self.fonts.is_none() {
            let mut book = flash::FontBook::default();
            if let Some(movie) = self.movie(FONT_LIBRARY) {
                book.add_movie(&movie);
            }
            // The font movies themselves, over the library's stand-ins
            // (its `_normal_Font` lacks most glyphs).
            for name in FONT_MOVIES {
                if let Some(movie) = self.movie(name) {
                    book.add_movie(&movie);
                }
            }
            self.fonts = Some(book);
        }
        self.fonts.get_or_insert_with(flash::FontBook::default)
    }
}

/// A movie a Flash widget plays.
struct FlashMovie {
    swf: String,
    /// The widget's restart count when started.
    restarts: u32,
    rt: avm1::Runtime,
    /// What was last logged of it on screen: its box (None: not drawn)
    /// and whether it drew anything.
    seen: Option<(Option<[i32; 4]>, bool)>,
    /// Seconds since its state was last logged ([`flash_status`]).
    since_status: f64,
    /// The movies on other levels, by their roots, as last seen: one
    /// loaded (topbar's map) needs the PDA's blips again.
    levels: Vec<(i32, flash::ClipId)>,
    /// The box its stage was last drawn in (screen points): where the
    /// mouse is on it ([`GameHud::mouse`]).
    drawn: std::cell::Cell<Option<[f32; 4]>>,
}

/// A movie's state, once a second while it plays: its root's frame,
/// whether the scripts paused it, and what it draws on its stage (bounds
/// and the most opaque alpha).
fn flash_status(m: &FlashMovie, paused: bool, fonts: Option<&flash::FontBook>) -> String {
    let player = &m.rt.player;
    let (frame, frames) = player
        .clip(player.root)
        .map_or((0, 0), |c| (c.frame + 1, c.frame_count()));
    let empty = flash::FontBook::default();
    let mut triangles = Vec::new();
    player.draw(
        &flash::Matrix::IDENTITY,
        fonts.unwrap_or(&empty),
        &mut triangles,
    );
    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    let mut alpha = 0.0f32;
    for t in &triangles {
        alpha = alpha.max(t.colour[3]);
        for [x, y] in t.corners {
            bounds = [
                bounds[0].min(x),
                bounds[1].min(y),
                bounds[2].max(x),
                bounds[3].max(y),
            ];
        }
    }
    // By texture (none: solid): triangles, their uv box and colours.
    type Fill = (usize, [f32; 4], [f32; 4], [f32; 4]);
    let mut fills: BTreeMap<&str, Fill> = BTreeMap::new();
    for t in &triangles {
        let e = fills
            .entry(t.texture.as_deref().unwrap_or("solid"))
            .or_insert((
                0,
                [
                    f32::INFINITY,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    f32::NEG_INFINITY,
                ],
                [f32::INFINITY; 4],
                [f32::NEG_INFINITY; 4],
            ));
        e.0 += 1;
        for [u, v] in t.uv {
            e.1 = [e.1[0].min(u), e.1[1].min(v), e.1[2].max(u), e.1[3].max(v)];
        }
        for i in 0..4 {
            e.2[i] = e.2[i].min(t.colour[i]);
            e.3[i] = e.3[i].max(t.colour[i]);
        }
    }
    let fills: Vec<String> = fills
        .iter()
        .map(|(name, (n, uv, lo, hi))| format!("{name} x{n} uv {uv:.2?} colour {lo:.2?}..{hi:.2?}"))
        .collect();
    // Movies loaded on other levels (the PDA's map on `_level1`).
    let mut levels = String::new();
    for (&level, &root) in player.levels.iter().filter(|(l, _)| **l != 0) {
        let mut own = Vec::new();
        player.draw_from(
            root,
            &flash::Matrix::IDENTITY,
            fonts.unwrap_or(&empty),
            &mut own,
        );
        let c = player.clip(root);
        levels.push_str(&format!(
            "; _level{level}: {} children, visible {}, colour {:?}, {} triangles",
            c.map_or(0, |c| c.children.len()),
            c.is_some_and(|c| c.visible),
            c.map(|c| c.colour),
            own.len(),
        ));
    }
    let [w, h] = player.lib.size;
    format!(
        "flash {}: frame {frame}/{frames}{}, stage {w:.0}x{h:.0}, {} triangles in {}, alpha up to {alpha:.2}; {}{levels}",
        m.swf,
        if paused { " paused" } else { "" },
        triangles.len(),
        if triangles.is_empty() {
            "nothing".to_owned()
        } else {
            format!("{bounds:.0?}")
        },
        fills.join("; "),
    )
}

/// A movie's name as the archive knows it: a file name or path
/// (`ui/fanfare_contract.swf`) without folders and extension, lower case.
fn movie_name(file: &str) -> String {
    let base = file.rsplit(['/', '\\']).next().unwrap_or(file);
    base.split('.').next().unwrap_or(base).to_ascii_lowercase()
}

/// A movie's triangles stretched over the widget's box `rect` (screen
/// points), as Scaleform fits a movie to its widget; each a quad with its
/// last corner doubled. A fullscreen widget's movie (but the pause menu's)
/// is drawn in [`show_all`]'s box, keeping its stage's shape, centred, as
/// Scaleform's default "show all" scale mode does.
fn flash_prims(
    m: &FlashMovie,
    movies: &Movies,
    rect: [f32; 4],
    out: &mut Vec<Prim>,
    want: &mut Vec<u32>,
) {
    let [mw, mh] = m.rt.player.lib.size;
    if mw <= 0.0 || mh <= 0.0 {
        return;
    }
    let matrix = flash::Matrix {
        a: (rect[2] - rect[0]) / mw,
        b: 0.0,
        c: 0.0,
        d: (rect[3] - rect[1]) / mh,
        tx: rect[0],
        ty: rect[1],
    };
    let empty = flash::FontBook::default();
    let fonts = movies.fonts.as_ref().unwrap_or(&empty);
    let mut triangles = Vec::new();
    m.rt.player.draw(&matrix, fonts, &mut triangles);
    triangle_prims(&triangles, out, want);
}

/// The largest box of the stage's shape `size` centred in `rect`.
fn show_all(rect: [f32; 4], size: [f32; 2]) -> [f32; 4] {
    let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
    let scale = (w / size[0]).min(h / size[1]);
    let (sw, sh) = (size[0] * scale, size[1] * scale);
    let x = rect[0] + (w - sw) / 2.0;
    let y = rect[1] + (h - sh) / 2.0;
    [x, y, x + sw, y + sh]
}

/// Triangles as prims (quads with the last corner doubled), asking for
/// their textures.
fn triangle_prims(triangles: &[flash::Triangle], out: &mut Vec<Prim>, want: &mut Vec<u32>) {
    let mut textures: HashMap<&str, u32> = HashMap::new();
    for t in triangles {
        let texture = t.texture.as_deref().map(|name| {
            *textures.entry(name).or_insert_with(|| {
                let hash = name_hash(name);
                want.push(hash);
                hash
            })
        });
        let [a, b, c] = t.corners;
        let [ua, ub, uc] = t.uv;
        out.push(Prim {
            texture,
            corners: [a, b, c, c],
            uv: [ua, ub, uc, uc],
            colour: t.colour,
            multiply: t.multiply,
        });
    }
}

/// The PDA's screens as triangles in the movies' pixels (all of them
/// `size`), first frames only: nothing runs their ActionScript yet, so the
/// map shows as laid out (unzoomed, no blips) and the tabs unlit.
struct PdaScreens {
    size: [f32; 2],
    triangles: Vec<flash::Triangle>,
}

/// Reads [`PDA_MOVIES`] and lays out their first frames ([`flash`]).
fn pda_triangles(assets: &mut AssetSource<BufReader<File>>) -> Result<PdaScreens, String> {
    let mut size = [0.0; 2];
    let mut triangles = Vec::new();
    for name in PDA_MOVIES {
        let data = assets
            .asset(name_hash(name), MOVIE_TYPE)
            .map_err(|e| format!("{name}: {e}"))?;
        let movie = movie_from_asset(&data).map_err(|e| format!("{name}: {e}"))?;
        let lib = std::rc::Rc::new(flash::Library::new(&movie));
        size = lib.size;
        flash::Player::new(lib).draw(
            &flash::Matrix::IDENTITY,
            &flash::FontBook::default(),
            &mut triangles,
        );
    }
    Ok(PdaScreens { size, triangles })
}

/// The PDA on a `screen` (points): its movies stretched over the 4:3
/// virtual screen at the screen's height, centred, as Scaleform fits a
/// movie to its widget (the PDA's covers the virtual screen, 640 x 480);
/// each triangle a quad with its last corner doubled.
fn pda_prims(screens: &PdaScreens, screen: [f32; 2], out: &mut Vec<Prim>, want: &mut Vec<u32>) {
    let [mw, mh] = screens.size;
    if mw <= 0.0 || mh <= 0.0 {
        return;
    }
    let h = screen[1];
    let w = h * 4.0 / 3.0;
    let left = (screen[0] - w) / 2.0;
    // The scene behind darkened, as the footage has it with the PDA up.
    out.push(quad(
        None,
        [0.0, 0.0, screen[0], screen[1]],
        [0.0, 0.0, 1.0, 1.0],
        [0.0, 0.0, 0.0, PDA_DIM],
        0.0,
    ));
    let at = |[x, y]: [f32; 2]| [left + x * w / mw, y * h / mh];
    let placed: Vec<flash::Triangle> = screens
        .triangles
        .iter()
        .map(|t| flash::Triangle {
            corners: t.corners.map(at),
            ..t.clone()
        })
        .collect();
    triangle_prims(&placed, out, want);
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
            multiply: false,
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
        let hash = name_hash(icon);
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
        // A stroke of the movie's map is so many metres across, as the
        // movie's strokes widen with its map.
        let width = match road.width {
            Some(metres) => metres / view.range.max(1.0),
            None if road.paved => minimap::PAVED_WIDTH,
            None => minimap::ROAD_WIDTH,
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
                    multiply: false,
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
    // |a + t (b - a) - centre|Â² = rÂ², solved for t.
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
            multiply: false,
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
        multiply: false,
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
                multiply: false,
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
    use crate::combat::RIFLE_CLIP;

    #[test]
    fn a_texture_named_by_hash_is_that_hash() {
        // The shotgun's reticle, by its `WeaponUI` hash.
        assert_eq!(texture_hash(&by_hash(0xe350_5568)), 0xe350_5568);
        assert_eq!(
            texture_hash("reticle_shotgun"),
            name_hash("reticle_shotgun")
        );
        // Not eight hex digits: a name like any other.
        assert_eq!(texture_hash("#beef"), name_hash("#beef"));
    }

    #[test]
    fn hud_sounds_are_the_movies_sound_commands() {
        assert!(is_sound_command("Sound.Loop"));
        assert!(is_sound_command("Sound.Stop"));
        assert!(!is_sound_command("audioSetSFXVolume"));
    }

    /// Where on screen the hit arc shows, clockwise from straight ahead:
    /// `HandleUpdateEvent` turns it by direction + heading, from the top
    /// where its art points.
    fn arc(heading: f32, from: Vec3) -> f32 {
        (direction(Some(Vec3::ZERO), Some(from)) + heading).rem_euclid(360.0)
    }

    #[test]
    fn getting_out_of_a_vehicle_tells_her_gun_again() {
        let mut told = Told::default();
        let gun = Some((Weapon::Rifle, false, false, false));
        // On the boat at the start: the gun told once.
        told.seat(true);
        told.weapon = gun;
        told.ammo = Some([30, 30, 270, 0, 0]);
        told.seat(true);
        assert_eq!(told.weapon, gun);
        // Ashore: told again, rounds and all; and not again on foot.
        told.seat(false);
        assert_eq!((told.weapon, told.ammo), (None, None));
        told.weapon = gun;
        told.seat(false);
        assert_eq!(told.weapon, gun);
    }

    #[test]
    fn plain_values_cross_between_lua_states() {
        let lua = Lua::new();
        let v: Value = lua
            .load(r#"return { sType = "contract", nPlayer = 1, nValue = 2.5, t = { true } }"#)
            .eval()
            .unwrap();
        let Value::Table(table) = &v else { panic!() };
        table.set("vPlayer", guid_value(1)).unwrap();
        let plain = Plain::of(&v);
        let other = Lua::new();
        other
            .globals()
            .set("t", plain.to_lua(&other).unwrap())
            .unwrap();
        let t: Table = other.globals().get("t").unwrap();
        assert_eq!(t.get::<String>("sType").unwrap(), "contract");
        assert!(matches!(
            t.get::<Value>("nPlayer").unwrap(),
            Value::Integer(1)
        ));
        assert_eq!(t.get::<f64>("nValue").unwrap(), 2.5);
        let inner: Table = t.get("t").unwrap();
        assert!(inner.get::<bool>(1).unwrap());
        assert_eq!(as_guid(&t.get::<Value>("vPlayer").unwrap()), Some(1));
    }

    #[test]
    fn a_fullscreen_movie_keeps_its_shape() {
        assert_eq!(
            show_all([0.0, 0.0, 1280.0, 720.0], [640.0, 480.0]),
            [160.0, 0.0, 1120.0, 720.0]
        );
        assert_eq!(
            show_all([0.0, 0.0, 1280.0, 720.0], [1280.0, 720.0]),
            [0.0, 0.0, 1280.0, 720.0]
        );
    }

    #[test]
    fn movies_are_named_without_folders_or_extensions() {
        assert_eq!(movie_name("fanfare_contract"), "fanfare_contract");
        assert_eq!(movie_name("UI/Fanfare_Mission.swf"), "fanfare_mission");
        assert_eq!(movie_name("ui\\map.gfx"), "map");
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
        // The rifle's rounds are the play's: a full clip and nine spare.
        let full = Rounds::of(RIFLE_CLIP + crate::combat::RIFLE_SPARE, RIFLE_CLIP);
        assert_eq!(
            ammo(
                Weapon::Rifle,
                fired(0, 0, 0),
                none,
                (full, RIFLE_CLIP),
                None,
                None,
                0
            )[..3],
            [30, 30, 270]
        );
        let used = Rounds {
            clip: 1,
            stored: 120,
        };
        assert_eq!(
            ammo(
                Weapon::Rifle,
                fired(29, 0, 0),
                none,
                (used, RIFLE_CLIP),
                None,
                None,
                0
            )[..3],
            [30, 1, 120]
        );
        // A shotgun reloaded: its own clip size, so its count is full,
        // not 8 of the carbine's 30.
        let shells = Rounds::of(8 + 32, 8);
        assert_eq!(
            ammo(
                Weapon::Rifle,
                fired(0, 0, 0),
                none,
                (shells, 8),
                None,
                None,
                0
            )[..3],
            [8, 8, 32]
        );
        assert_eq!(
            ammo(
                Weapon::Rpg,
                fired(0, 0, 0),
                none,
                (full, RIFLE_CLIP),
                None,
                None,
                0
            )[..3],
            [1, 1, i64::from(RPG_STORED)]
        );
        assert_eq!(
            ammo(
                Weapon::Rpg,
                fired(0, 1, 0),
                none,
                (full, RIFLE_CLIP),
                None,
                None,
                0
            )[2],
            i64::from(RPG_STORED) - 1
        );
        assert_eq!(
            ammo(
                Weapon::Rpg,
                fired(0, 4, 0),
                none,
                (full, RIFLE_CLIP),
                None,
                None,
                0
            )[2],
            i64::from(RPG_STORED)
        );
        // Her own grenades, whatever she has thrown.
        assert_eq!(
            ammo(
                Weapon::Rifle,
                fired(0, 0, 1),
                none,
                (full, RIFLE_CLIP),
                None,
                None,
                3
            )[3],
            3
        );
        // A crate's 20 rockets after three fired: 21 left.
        let crate_ = Picked {
            rockets: 20,
            ..Picked::default()
        };
        assert_eq!(
            ammo(
                Weapon::Rpg,
                fired(0, 3, 0),
                crate_,
                (full, RIFLE_CLIP),
                None,
                None,
                0
            )[2],
            i64::from(RPG_STORED) + 17
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
                (full, RIFLE_CLIP),
                launcher,
                None,
                0
            )[..3],
            [6, 6, 12]
        );
        // The AMX-30's 40 shells, three fired: alone in the gun box.
        assert_eq!(
            ammo(
                Weapon::Rifle,
                fired(0, 0, 0),
                none,
                (full, RIFLE_CLIP),
                None,
                Some((37, 40)),
                0
            )[..3],
            [0, 37, -1]
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
    fn a_stroke_of_the_movie_map_widens_with_the_map() {
        // A 12 m stroke across the middle of a minimap 200 across.
        let road = Road::stroke(vec![[-50.0, 0.0], [50.0, 0.0]], 12.0);
        let across = |range: f32| {
            let view = MapView {
                rect: [0.0, 0.0, 200.0, 200.0],
                player: [0.0; 3],
                heading: 0.0,
                range,
            };
            let (mut out, mut want) = (Vec::new(), Vec::new());
            road_strokes(&view, std::slice::from_ref(&road), &mut out, &mut want);
            assert_eq!(out.len(), 1, "solid: one quad");
            let [a, _, _, d] = out[0].corners;
            (a[0] - d[0]).hypot(a[1] - d[1])
        };
        // 12 m of 150 m to the rim, of a radius of 100: 8 across; twice
        // as far out, half as wide.
        assert!((across(150.0) - 8.0).abs() < 1e-3, "{}", across(150.0));
        assert!((across(300.0) - 4.0).abs() < 1e-3);
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

    #[test]
    fn the_tutorial_box_says_something_only_with_words() {
        let text = |widget, text: &str| DrawItem::Text {
            widget,
            font: "english_18".into(),
            text: text.into(),
            spans: Vec::new(),
            rect: [0.0; 4],
            size: 1.0,
            colour: [1.0; 4],
            justification: Anchor::Start,
        };
        let held = HashSet::from([1, 2]);
        let frame = DrawItem::Quad {
            widget: 1,
            texture: "frame".into(),
            uv: [0.0, 0.0, 1.0, 1.0],
            colour: [1.0; 4],
            rect: [0.0, 0.0, 10.0, 10.0],
            rotation: 0.0,
            pie: None,
        };
        // Its frame and an emptied line: nothing.
        assert!(!says_something(&[frame.clone(), text(2, " ")], &held));
        // Words elsewhere aren't its.
        assert!(!says_something(
            &[frame.clone(), text(3, "Open PDA")],
            &held
        ));
        assert!(says_something(&[frame, text(2, "Open PDA")], &held));
    }

    #[test]
    fn the_probe_finds_what_is_over_its_point() {
        let fill = DrawItem::Fill {
            widget: 4,
            colour: [1.0; 4],
            rect: [10.0, 20.0, 30.0, 40.0],
        };
        assert!(holds(&fill, [10.0, 40.0]));
        assert!(holds(&fill, [20.0, 30.0]));
        assert!(!holds(&fill, [9.0, 30.0]));
        assert!(!holds(&fill, [20.0, 41.0]));
    }

    /// What posing the timing wheel answers, with the HUD's scripts
    /// standing in: no overlay, then one without its minigame.
    #[test]
    fn the_wheel_says_what_it_found() {
        let lua = Lua::new();
        lua.load(
            r#"Sys = { StringToGuid = function() return 1 end }
            overlay = { CustomData = {} }
            MrxGuiBase = { WidgetIdIndex = { [7] = overlay } }
            MrxGui = { GetWidgetByNameAndOwner = function() return found end }"#,
        )
        .exec()
        .unwrap();
        let pose = || -> (Option<String>, Option<u32>) {
            lua.load(SATELLITE_WHEEL).call((0.5, 10.0)).unwrap()
        };
        assert_eq!(pose(), (Some("no overlay".into()), None));
        lua.load("found = overlay").exec().unwrap();
        assert_eq!(pose(), (Some("no minigame".into()), Some(7)));
    }
}

#[cfg(test)]
mod paint_tests {
    use super::*;

    /// A HUD with no thread behind it, at `status`, holding `prims`.
    fn test_hud(status: Status, prims: Vec<Prim>) -> GameHud {
        let (tx, _rx) = channel();
        let (_tx, rx) = channel();
        GameHud {
            tx,
            rx,
            status,
            callbacks: Vec::new(),
            choices: Vec::new(),
            dropped_fanfare: false,
            shell: ShellNews::default(),
            prims,
            pending: Vec::new(),
            textures: HashMap::new(),
            pixels: HashMap::new(),
            skip_covers: false,
            last_message: None,
            last_call: 0,
            last_time: None,
            pause_news: crate::pause::News::default(),
            pause_available: false,
            log: None,
        }
    }

    /// A fade (a black, three-quarters solid quad over the screen) leaves
    /// egui as a mesh over the screen.
    #[test]
    fn solid_quads_reach_the_tessellated_output() {
        let mut hud = test_hud(
            Status::Ready,
            vec![quad(
                None,
                [0.0, 0.0, 800.0, 600.0],
                [0.0; 4],
                [0.0, 0.0, 0.0, 0.75],
                0.0,
            )],
        );
        let ctx = egui::Context::default();
        hud.upload(&ctx);
        let out = ctx.run_ui(crate::ui::headless_input(800, 600, 0.0), |ui| {
            let ctx = ui.ctx();
            hud.paint(&ctx.layer_painter(egui::LayerId::background()));
        });
        let mut delta = out.textures_delta;
        delta.clear();
        let meshes = ctx.tessellate(out.shapes, out.pixels_per_point);
        let verts: Vec<_> = meshes
            .iter()
            .filter_map(|m| match &m.primitive {
                egui::epaint::Primitive::Mesh(m) => Some(m.vertices.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert!(verts.iter().any(|v| v.color.a() > 150 && v.pos.x > 700.0));
    }

    #[test]
    fn the_target_arc_reads_each_person_of_their_own_most() {
        use crate::npc::{Brain, Npc, tests::rig};
        let person = |spawn: Option<u32>| {
            Npc::new(
                0,
                &rig(),
                Vec3::ZERO,
                0.0,
                Brain::Post { facing: 0.0 },
                spawn,
            )
        };
        // A civilian keeps the default profile (75) but starts at 100:
        // full reads full, half reads half.
        let mut civilian = person(None);
        assert_eq!(target_health(&civilian), (0, 100, 100));
        civilian.hp = 50.0;
        assert_eq!(target_health(&civilian), (0, 50, 100));
        // A soldier placed with their template's health, turned on her.
        let mut soldier = person(Some(1));
        soldier.hp = soldier.ai.health;
        soldier.most_hp = soldier.ai.health;
        soldier.alert = true;
        let most = soldier.ai.health.round() as i32;
        assert_eq!(target_health(&soldier), (-1, most, most));
    }

    fn frame(log: &[&str]) -> Output {
        Output::Frame {
            prims: Vec::new(),
            images: Vec::new(),
            log: log.iter().map(|l| (*l).to_owned()).collect(),
            callbacks: Vec::new(),
            choices: Vec::new(),
            shell: Box::default(),
            pause: Box::default(),
        }
    }

    /// A failed fanfare call is noted; a Lua state out of stack hands over
    /// to the stand-in HUD, while a script's own error does not.
    #[test]
    fn a_broken_hud_hands_over_to_the_stand_in() {
        let mut hud = test_hud(Status::Loading, Vec::new());
        hud.take(frame(&[
            "hud call Fanfare.Create",
            "hud call SubtitleBuffer.AddMessage: [string \"x\"]:1: stack overflow",
        ]));
        assert!(hud.ready() && !hud.dropped_fanfare());
        hud.take(frame(&[
            "hud call Fanfare.Commence: the HUD has no such method",
        ]));
        assert!(hud.ready() && hud.dropped_fanfare());
        hud.take(frame(&[
            "gui call MrxGui.Show: out of Lua stack, too many arguments",
        ]));
        assert!(hud.failed() && !hud.ready());
        // A clean frame queued behind it does not take it back.
        hud.take(frame(&[]));
        assert!(hud.failed() && !hud.ready());
    }
}
