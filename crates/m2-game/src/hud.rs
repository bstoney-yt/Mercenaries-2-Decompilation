//! What the scripts put on the player's screen: the mission's objective
//! list, objectives on the map and radar, markers over targets in the
//! world, and "objective added / completed" messages.
//!
//! The game draws these with its own (Flash) interface; scripts reach it
//! through `Net.SendEvent_*` calls (sent to every player in co-op) and
//! `Marker`. Here they fill a [`Hud`] the game draws its own way. Texts are
//! ids such as `[VzaCon001.Objectives.001]` into the language's string
//! table; [`render`] puts the table's text in, and [`readable`] makes
//! something showable of an id the table lacks.

use std::collections::BTreeMap;

use m2_assets::StringTable;
use mlua::{Lua, MultiValue, Table, Value};

use crate::api::Shared;
use crate::events::{as_guid, guid_value};
use crate::script::safe_function;

/// An objective on the map (the PDA) or radar.
#[derive(Clone, Debug, PartialEq)]
pub struct Objective {
    /// What it is about, if an object (a place, a target).
    pub target: Option<u32>,
    /// Text id, with progress appended (`"[VzaCon001.Objectives.003] (0/4)"`).
    pub text: String,
    /// The game's icon index.
    pub icon: i64,
    /// The mission it is for: its index in the mission data
    /// (`WifMissionData.GetMissionIndexFromId`; VzaCon001 is 36).
    pub mission: i64,
}

/// [`Blip::icon`] for a blip given no icon index (`Hud.Radar:AddObjective`).
pub const NO_ICON: i64 = -1;

/// A blip on the radar.
#[derive(Clone, Debug, PartialEq)]
pub struct Blip {
    pub target: Option<u32>,
    /// Offset from the target, metres (without a target, where it is).
    pub offset: [f32; 3],
    pub colour: [u8; 3],
    /// Index into the scripts' radar icons
    /// (`MrxUtil.MarkerGetNameByIndex_Radar`: `"objective_action"`);
    /// [`NO_ICON`] for none.
    pub icon: i64,
    /// Kept on the radar's rim while out of its range; else not drawn
    /// there (from the 360 code, see [`Blip::sticky`]'s use in the
    /// minimap). An objective's targets are sticky while its mission is
    /// tracked (`MrxTaskObjective.EnableTracking`).
    pub sticky: bool,
    /// Draw order, 1 to 7: lower sorts draw over higher (from the 360
    /// code: the minimap's renderer draws 7 down to 1, clamping to that).
    pub sort: i64,
    /// Its icon by name (`temp_radar_icon_tank`), when the call gave a
    /// texture and not an index (`Hud.Radar:AddObjective`); read before
    /// [`Self::icon`].
    pub texture: Option<String>,
    /// Width and height as given (the scripts' `nWidth`, `nHeight`).
    pub size: [f32; 2],
    /// `bRotate` and `bOriented` as given: the vehicles' and the planes'
    /// blips set both.
    pub rotate: bool,
    pub oriented: bool,
    /// Its opacity, 255 unless an animation fades it
    /// ([`crate::radar_anim`]).
    pub alpha: u8,
    /// Its animations.
    pub anims: crate::radar_anim::Anims,
}

impl Hud {
    /// Moves the radar blips' animations on by `dt` seconds
    /// ([`crate::radar_anim`]).
    pub fn step_radar(&mut self, dt: f32) {
        for b in self.radar.values_mut() {
            if !b.anims.is_empty() {
                b.anims.step(&mut b.size, &mut b.alpha, dt);
            }
        }
    }
}

/// A disc on the ground round a target or a point (`Marker.AddDisc`).
#[derive(Clone, Debug, PartialEq)]
pub struct Disc {
    pub target: Option<u32>,
    /// Its point, when it was given one rather than an object.
    pub at: Option<[f32; 3]>,
    /// Metres.
    pub radius: f32,
    pub colour: [u8; 3],
    /// Over the ground, metres.
    pub lift: f32,
}

/// A tripwire across a race's gate (`Marker.AddTripwire`). From the 360
/// code (`Gui._MarkerAddTripwire`, 8231B230): the call is
/// `(x, y, z, width, yaw, r, g, b)`; the first five are needed (without
/// them no marker is made), the colour is 255 each where left out, the
/// width is held within [`TRIPWIRE_WIDTH`] and the yaw is in degrees.
#[derive(Clone, Debug, PartialEq)]
pub struct Tripwire {
    pub at: [f32; 3],
    /// Across, metres.
    pub width: f32,
    /// Degrees, as the scripts give it (`Object.GetYaw` of the gate).
    pub yaw: f32,
    pub colour: [u8; 3],
}

/// The least and most a tripwire's width is held to, metres (from the 360
/// code, 8231B230: 1.0 and 50.0).
pub const TRIPWIRE_WIDTH: (f32, f32) = (1.0, 50.0);

/// A model drawn at a target as a marker (`Marker.Add3D`): a race's ring
/// (`global_airring`) and its finish (`global_tripwirefinish`). From the
/// 360 code (`Gui._MarkerAdd3D`, 8231B5A8): the call is
/// `(target, model, r, g, b, scale)`; the target and the model are
/// needed, the colour is 255 each and the scale 1 where left out, and it
/// fades as other markers do, between [`MARKER_NEAR`] and [`MARKER_FAR`].
#[derive(Clone, Debug, PartialEq)]
pub struct Model3d {
    pub target: Option<u32>,
    pub model: String,
    pub colour: [u8; 3],
    pub scale: f32,
}

/// A coloured mark held at an offset from a target (`Marker.Add`): the
/// mark over someone being delivered (`MrxTaskObjectiveDeliver` puts one
/// 2 m over each, `Marker.Add(0, 2, 0, who, 255, 255, 0, 0.05)`).
///
/// From the 360 code (`Gui._MarkerAddOld`, 8231B828; the scripts' one
/// call fits it and not `Gui._MarkerAdd`, 8231AEE8, which is
/// `Marker.AddBlip`'s `(target, icon, size, ...)`): the call is
/// `(x, y, z, target, r, g, b, scale, icon, ...)` and every argument may
/// be left out: the offset is `(0, 2, 0)`, the colour 255 each and the
/// scale 0.25 by default, with no target it is a point in the world, and
/// it fades as other markers do, between [`MARKER_NEAR`] and
/// [`MARKER_FAR`]. Three more arguments after the icon (a number, a
/// boolean, a number) are passed on to the marker and aren't understood;
/// the scripts never give them.
#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub target: Option<u32>,
    /// From the target (or the world's origin), metres.
    pub offset: [f32; 3],
    pub colour: [u8; 3],
    pub scale: f32,
    /// Its icon by name; `None` for the game's own default.
    pub icon: Option<String>,
}

/// A [`Pin`]'s offset and scale when the call gives none (from the 360
/// code, 8231B828).
pub const PIN_OFFSET: [f32; 3] = [0.0, 2.0, 0.0];
pub const PIN_SCALE: f32 = 0.25;

/// A marker over a target in the world.
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub target: Option<u32>,
    /// `"HUD_objective_deliverable"`, `"HUD_objective_action"`,
    /// `"HUD_objective_destroy"`.
    pub icon: String,
    pub colour: [u8; 4],
    /// Height above the target, metres.
    pub height: f32,
    /// Its fade, metres (from the 360 code, `Gui._MarkerAdd` and the
    /// marker renderer): opaque within `near`, gone beyond `far`, and in
    /// between fading linearly in the squared distance. The call's own
    /// values; [`MARKER_NEAR`] and [`MARKER_FAR`] when it gives none.
    pub near: f32,
    pub far: f32,
    /// The call's two sizes (its third and eleventh arguments: 32 and 32
    /// in the first contract's); which is for what isn't known yet.
    pub sizes: [f32; 2],
    /// The call's last argument ("2D only"): its fade measures the
    /// distance over the ground, leaving height out (from the 360 code).
    pub flat: bool,
    /// The call's twelfth argument, the mission it is for, as given (of a
    /// list, the first).
    pub mission: String,
}

/// What the scripts want shown.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hud {
    /// The current mission's objective list: text id and icon tag.
    pub mission: Vec<(String, String)>,
    /// Tutorial lines asked for and not yet shown
    /// (`Net.SetTutorialMessage`): each call's arguments as text, in
    /// order (see [`tutorial_text`]).
    pub tutorials: Vec<Vec<String>>,
    /// Objectives on the map, by the scripts' name for them (the target's
    /// guid as text, or `<mission>_PreMission`).
    pub objectives: BTreeMap<String, Objective>,
    pub radar: BTreeMap<String, Blip>,
    /// World markers by handle.
    pub markers: BTreeMap<u32, Marker>,
    /// Discs on the ground by handle (`Marker.AddDisc`).
    pub discs: BTreeMap<u32, Disc>,
    /// Tripwires across gates by handle (`Marker.AddTripwire`).
    pub tripwires: BTreeMap<u32, Tripwire>,
    /// Models drawn as markers by handle (`Marker.Add3D`).
    pub models: BTreeMap<u32, Model3d>,
    /// Marks held by a target by handle (`Marker.Add`).
    pub pins: BTreeMap<u32, Pin>,
    /// Markers that pulse, by handle, each in its colour (`Marker.Pulse`,
    /// until `Marker.HaltPulse` or the marker's removal). From the 360
    /// code (`Gui._MarkerPulse`, 8231BB88, and `_MarkerHaltPulse`,
    /// 8231BC90): the call is `(marker, r, g, b)`, 255 each where left
    /// out. The faction manager pulses a faction's map marker red.
    pub pulses: BTreeMap<u32, [u8; 3]>,
    /// Objective messages not yet shown.
    pub messages: Vec<ObjectiveMessage>,
    /// How the engine's markers over pickups are drawn, as the scripts
    /// set them.
    pub pickup_markers: PickupMarkers,
    next_marker: u32,
}

/// The size and reach of the markers over pickups, for each of the two
/// views the engine keeps them for: `[0]` play, `[1]` the view its
/// renderer calls mode 4 (unidentified). From the 360 code: the HUD's
/// marker init (82374FB0) sets size 12 and reach 20 m and 100 m;
/// `Gui.SetPickupMarkerSize(n, view)` (8231C350) and
/// `Gui.SetPickupMarkerVisibleDistance(m, view)` (8231C3E0) set one view's,
/// `view` a boolean, true or left out the second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PickupMarkers {
    pub size: [f32; 2],
    pub reach: [f32; 2],
}

impl Default for PickupMarkers {
    fn default() -> Self {
        Self {
            size: [12.0, 12.0],
            reach: [20.0, 100.0],
        }
    }
}

/// Which view a pickup marker call sets: its second argument, a boolean,
/// true or left out the second (from the 360 code: it reads the argument
/// as a byte, 1 if absent).
fn pickup_view(v: Option<&Value>) -> usize {
    match v {
        Some(Value::Boolean(false)) => 0,
        Some(Value::Integer(0)) => 0,
        Some(Value::Number(n)) if *n == 0.0 => 0,
        _ => 1,
    }
}

/// An objective message as the scripts send it
/// (`Net.SendEvent_ObjectiveMessage`, for `MrxGuiInterface.
/// DisplayObjectiveMessage` to show).
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectiveMessage {
    /// Its kind, a key of `MrxGuiInterface.tObjectiveMessageConfigs`:
    /// `"add"`, `"upd"`, `"cpl"`, `"ccl"`, `"bonus_add"`...
    pub kind: String,
    /// Its icon, an index into `MrxUtil.tInlineIcons` (`[objdeliver]`...).
    pub icon: i64,
    /// The objective's text id.
    pub text: String,
}

impl ObjectiveMessage {
    /// An objective completed (else added, updated or cancelled).
    pub fn done(&self) -> bool {
        self.kind.ends_with("cpl")
    }
}

/// A marker's fade when `Marker.AddBlip` gives none, metres (from the 360
/// code: `Gui._MarkerAdd`'s defaults, 200 and 300).
pub const MARKER_NEAR: f32 = 200.0;
pub const MARKER_FAR: f32 = 300.0;

/// Marker handles start here.
const FIRST_MARKER: u32 = 0x5000_0000;

/// A text with its ids replaced by the table's strings
/// (`[VzaCon001.Objectives.003] (0/4)` as `Secure the beach (0/4)`), and
/// ids the table lacks made [`readable`].
pub fn render(text: &str, table: &StringTable) -> String {
    render_with(text, |id| table.lookup(id).map(str::to_owned))
}

/// A text id made readable without the string table: icon tags dropped,
/// `[VzaCon001.Objectives.003] (0/4)` as `VzaCon001 objective 3 (0/4)`.
pub fn readable(text: &str) -> String {
    render_with(text, |_| None)
}

/// Line by line: the scripts join texts with line breaks, and the table's
/// own texts break with `[n]`. Lines left empty go.
fn render_with(text: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    text.split('\n')
        .map(|line| render_line(line, &lookup))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_line(text: &str, lookup: &impl Fn(&str) -> Option<String>) -> String {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let before = rest[..open].trim();
        if !before.is_empty() {
            out.push(before.to_owned());
        }
        let Some(close) = rest[open..].find(']') else {
            break;
        };
        let id = &rest[open + 1..open + close];
        rest = &rest[open + close + 1..];
        if let Some(found) = lookup(id) {
            let found = found.trim();
            if !found.is_empty() {
                out.push(found.replace("[n]", "\n"));
            }
            continue;
        }
        let parts: Vec<&str> = id.split('.').collect();
        match parts.as_slice() {
            // An icon tag: `[objdeliver]`.
            [_] => {}
            [mission, "Objectives", n] => {
                let n = n.trim_start_matches('0');
                out.push(format!(
                    "{mission} objective {}",
                    if n.is_empty() { "0" } else { n }
                ));
            }
            [.., last] => out.push(last.replace('_', " ")),
            [] => {}
        }
    }
    let rest = rest.trim();
    if !rest.is_empty() {
        out.push(rest.to_owned());
    }
    out.join(" ")
}

fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.to_string_lossy(),
        Some(Value::Integer(i)) => i.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// A script value as text, for a call whose arguments aren't known yet:
/// strings and numbers as they are, others by their type.
fn shown(v: &Value) -> String {
    match v {
        Value::String(s) => s.to_string_lossy(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Nil => "nil".to_owned(),
        other => other.type_name().to_owned(),
    }
}

/// The text id of a tutorial line, from its call's arguments
/// ([`Hud::tutorials`]): the first in brackets (`[Tutorial.OpenPDA]`),
/// none when the call clears the line.
pub fn tutorial_text(args: &[String]) -> Option<&str> {
    args.iter()
        .map(|a| a.trim())
        .find(|a| a.starts_with('[') && a.contains(']'))
}

fn number(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Integer(i) => Some(*i as f64),
        Value::Number(n) => Some(*n),
        _ => None,
    }
}

/// A radar objective's name as its key: a text or a number as it is, an
/// object's guid (the object scripts name their blip by their object) as
/// `guid 0x...`.
fn blip_name(v: Option<&Value>) -> Option<String> {
    let v = v?;
    match as_guid(v) {
        Some(g) if !matches!(v, Value::Integer(_) | Value::Number(_) | Value::String(_)) => {
            Some(format!("guid {g:#x}"))
        }
        _ => Some(text(Some(v))).filter(|s| !s.is_empty()),
    }
}

fn int(v: Option<&Value>) -> i64 {
    number(v).map_or(0, |n| n as i64)
}

fn byte(v: Option<&Value>) -> u8 {
    number(v).map_or(255, |n| n.clamp(0.0, 255.0) as u8)
}

fn guid(v: Option<&Value>) -> Option<u32> {
    as_guid(v?)
}

/// Installs the HUD calls on `Net` and `Marker`.
pub fn install(lua: &Lua, world: &Shared) -> mlua::Result<()> {
    let globals = lua.globals();
    let lib = |name: &str| -> mlua::Result<Table> {
        match globals.raw_get::<Value>(name)? {
            Value::Table(t) if t.metatable().is_none() => Ok(t),
            _ => {
                let t = lua.create_table()?;
                globals.raw_set(name, &t)?;
                Ok(t)
            }
        }
    };
    let add = |table: &Table,
               name: &str,
               f: fn(&mut Hud, &Lua, &[Value]) -> mlua::Result<MultiValue>|
     -> mlua::Result<()> {
        let w = std::rc::Rc::clone(world);
        table.set(
            name,
            safe_function(lua, move |lua, args| {
                let args: Vec<Value> = args.into_iter().collect();
                f(&mut w.borrow_mut().hud, lua, &args)
            })?,
        )
    };
    let none = || Ok(MultiValue::new());

    let net = lib("Net")?;
    // A tutorial line: the call's arguments are kept as they come (what
    // each is for isn't known yet; the text id is the one in brackets).
    add(&net, "SetTutorialMessage", |hud, _, a| {
        hud.tutorials.push(a.iter().map(shown).collect());
        Ok(MultiValue::new())
    })?;
    // (mission index, {{text id, icon tag}, ...}, repeatable): the whole
    // list, sent again whenever it changes.
    add(&net, "SendEvent_AddPDAMission", |hud, _, a| {
        hud.mission.clear();
        if let Some(Value::Table(list)) = a.get(1) {
            for pair in list.sequence_values::<Table>() {
                let pair = pair?;
                let get = |i| text(pair.raw_get::<Value>(i).ok().as_ref());
                hud.mission.push((get(1), get(2)));
            }
        }
        Ok(MultiValue::new())
    })?;
    // (name, target, text id, icon, mission index, state, sort order).
    add(&net, "SendEvent_AddPdaObjective", |hud, _, a| {
        hud.objectives.insert(
            text(a.first()),
            Objective {
                target: guid(a.get(1)),
                text: text(a.get(2)),
                icon: int(a.get(3)),
                mission: int(a.get(4)),
            },
        );
        Ok(MultiValue::new())
    })?;
    add(&net, "SendEvent_RemovePdaObjective", |hud, _, a| {
        hud.objectives.remove(&text(a.first()));
        Ok(MultiValue::new())
    })?;
    // (name, x, y, z, r, g, b, width, height, icon, target, sticky,
    // rotate, oriented, sort order).
    add(&net, "SendEvent_AddRadarObjective", |hud, _, a| {
        let f = |i: usize| number(a.get(i)).unwrap_or(0.0) as f32;
        hud.radar.insert(
            text(a.first()),
            Blip {
                target: guid(a.get(10)),
                offset: [f(1), f(2), f(3)],
                colour: [byte(a.get(4)), byte(a.get(5)), byte(a.get(6))],
                icon: int(a.get(9)),
                sticky: matches!(a.get(11), Some(Value::Boolean(true))),
                sort: int(a.get(14)).clamp(1, 7),
                texture: None,
                size: [f(7), f(8)],
                rotate: matches!(a.get(12), Some(Value::Boolean(true))),
                oriented: matches!(a.get(13), Some(Value::Boolean(true))),
                alpha: 255,
                anims: Default::default(),
            },
        );
        Ok(MultiValue::new())
    })?;
    add(&net, "SendEvent_RemoveRadarObjective", |hud, _, a| {
        hud.radar.remove(&text(a.first()));
        Ok(MultiValue::new())
    })?;
    // (icon index, kind, text id, player).
    add(&net, "SendEvent_ObjectiveMessage", |hud, _, a| {
        hud.messages.push(ObjectiveMessage {
            kind: text(a.get(1)),
            icon: int(a.first()),
            text: text(a.get(2)),
        });
        Ok(MultiValue::new())
    })?;
    // The network copies of world markers: the markers are already here.
    for name in [
        "SendEvent_AddMarkerObjective",
        "SendEvent_RemoveMarkerObjective",
    ] {
        net.set(name, safe_function(lua, move |_, _| none())?)?;
    }

    let gui = lib("Gui")?;
    // (size, view): the size as a byte, 32 if not given (8231C350).
    add(&gui, "SetPickupMarkerSize", |hud, _, a| {
        let size = number(a.first()).map_or(32, |n| n as i64 as u8);
        hud.pickup_markers.size[pickup_view(a.get(1))] = f32::from(size);
        Ok(MultiValue::new())
    })?;
    // (metres, view): nothing without a distance (8231C3E0).
    add(&gui, "SetPickupMarkerVisibleDistance", |hud, _, a| {
        if let Some(reach) = number(a.first()) {
            hud.pickup_markers.reach[pickup_view(a.get(1))] = reach as f32;
        }
        Ok(MultiValue::new())
    })?;

    // The engine's `Hud` library hands `Hud.Radar:AddObjective(t)` and
    // `:RemoveObjective(t)` to the HUD's interface (`HudInterface.Radar`,
    // script 7f4bf50e), which gives the minimap `t`'s `sName`, `nX`,
    // `nY`, `nZ`, `nR`, `nG`, `nB`, `nWidth`, `nHeight`, `sTexture`,
    // `uGuid`, `bSticky`, `bRotate`, `bOriented` and `nSortOrder` as they
    // are. What is left out takes the minimap widget's defaults for the
    // place and the colour (0, 2, 0 and 255, 255, 0) and, for the size and
    // the sort order, those the same script sends over the network (3,
    // 3 and 5). The object scripts' blips come this way (`Blippable`:
    // the munitions', the tanks', the helicopters' and the planes'), with
    // `bDontNetSync` set, so no `Net.SendEvent_AddRadarObjective` follows.
    let radar = match lib("Hud")?.raw_get::<Value>("Radar")? {
        Value::Table(t) => t,
        _ => {
            let t = lua.create_table()?;
            lib("Hud")?.raw_set("Radar", &t)?;
            t
        }
    };
    add(&radar, "AddObjective", |hud, _, a| {
        let Some(Value::Table(t)) = a.get(1) else {
            return Ok(MultiValue::new());
        };
        let get = |k: &str| t.raw_get::<Value>(k).ok();
        let f = |k: &str, default: f32| number(get(k).as_ref()).map_or(default, |n| n as f32);
        let flag = |k: &str| matches!(get(k), Some(Value::Boolean(true)));
        let Some(name) = blip_name(get("sName").as_ref()) else {
            return Ok(MultiValue::new());
        };
        hud.radar.insert(
            name,
            Blip {
                target: guid(get("uGuid").as_ref()),
                offset: [f("nX", 0.0), f("nY", 2.0), f("nZ", 0.0)],
                colour: [
                    f("nR", 255.0).clamp(0.0, 255.0) as u8,
                    f("nG", 255.0).clamp(0.0, 255.0) as u8,
                    f("nB", 0.0).clamp(0.0, 255.0) as u8,
                ],
                // No index: the radar's objectives take only `sTexture`
                // (from the 360 code, the minimap native 0x82320DF0 reads
                // a texture or nothing).
                icon: NO_ICON,
                sticky: flag("bSticky"),
                sort: (f("nSortOrder", 5.0) as i64).clamp(1, 7),
                texture: Some(text(get("sTexture").as_ref())).filter(|s| !s.is_empty()),
                size: [f("nWidth", 3.0), f("nHeight", 3.0)],
                rotate: flag("bRotate"),
                oriented: flag("bOriented"),
                alpha: 255,
                anims: Default::default(),
            },
        );
        Ok(MultiValue::new())
    })?;
    add(&radar, "RemoveObjective", |hud, _, a| {
        if let Some(Value::Table(t)) = a.get(1)
            && let Some(name) = blip_name(t.raw_get::<Value>("sName").ok().as_ref())
        {
            hud.radar.remove(&name);
        }
        Ok(MultiValue::new())
    })?;

    // The blips' animations ([`crate::radar_anim`]), by the fields the
    // HUD's interface hands the minimap: a size animation needs its
    // duration and its four limits, an opacity one its duration and two
    // limits (the natives answer nothing without them); the speeds default
    // to 10 and 10, and 0.5.
    add(&radar, "AnimateObjectiveSize", |hud, _, a| {
        let Some(Value::Table(t)) = a.get(1) else {
            return Ok(MultiValue::new());
        };
        let f = |k: &str| number(t.raw_get::<Value>(k).ok().as_ref()).map(|n| n as f32);
        let (Some(left), Some(min_w), Some(min_h), Some(max_w), Some(max_h)) = (
            f("nDuration"),
            f("nMinWidth"),
            f("nMinHeight"),
            f("nMaxWidth"),
            f("nMaxHeight"),
        ) else {
            return Ok(MultiValue::new());
        };
        if let Some(b) =
            blip_name(t.raw_get::<Value>("sName").ok().as_ref()).and_then(|n| hud.radar.get_mut(&n))
        {
            // One of the kind at a time: the old one gives the size back.
            b.anims.remove("size", &mut b.size, &mut b.alpha);
            b.anims.size = Some(crate::radar_anim::SizeAnim {
                left,
                min: [min_w, min_h],
                max: [max_w, max_h],
                speed: [
                    f("nSpeedWidth").unwrap_or(10.0),
                    f("nSpeedHeight").unwrap_or(10.0),
                ],
                one_way: matches!(t.raw_get::<Value>("bOneWay"), Ok(Value::Boolean(true))),
                original: b.size,
            });
        }
        Ok(MultiValue::new())
    })?;
    add(&radar, "AnimateObjectiveAlpha", |hud, _, a| {
        let Some(Value::Table(t)) = a.get(1) else {
            return Ok(MultiValue::new());
        };
        let f = |k: &str| number(t.raw_get::<Value>(k).ok().as_ref()).map(|n| n as f32);
        let (Some(left), Some(min), Some(max)) = (f("nDuration"), f("nMinAlpha"), f("nMaxAlpha"))
        else {
            return Ok(MultiValue::new());
        };
        if let Some(b) =
            blip_name(t.raw_get::<Value>("sName").ok().as_ref()).and_then(|n| hud.radar.get_mut(&n))
        {
            b.anims.remove("alpha", &mut b.size, &mut b.alpha);
            b.anims.alpha = Some(crate::radar_anim::AlphaAnim {
                left,
                min,
                max,
                speed: f("nSpeed").unwrap_or(0.5),
                one_way: matches!(t.raw_get::<Value>("bOneWay"), Ok(Value::Boolean(true))),
                value: f32::from(b.alpha) / 255.0,
                original: b.alpha,
            });
        }
        Ok(MultiValue::new())
    })?;
    // A sonar needs its duration, texture, two counts and five numbers;
    // the grow speed is 5 and the colour white unless given (the
    // interface's defaults).
    add(&radar, "AnimateObjectiveSonar", |hud, _, a| {
        let Some(Value::Table(t)) = a.get(1) else {
            return Ok(MultiValue::new());
        };
        let f = |k: &str| number(t.raw_get::<Value>(k).ok().as_ref()).map(|n| n as f32);
        let texture = match t.raw_get::<Value>("sTexture") {
            Ok(Value::String(s)) => s.to_string_lossy(),
            _ => return Ok(MultiValue::new()),
        };
        let (
            Some(left),
            Some(total),
            Some(visible),
            Some(min),
            Some(max),
            Some(delay),
            Some(at_min),
            Some(at_max),
        ) = (
            f("nDuration"),
            f("nTotalBlips"),
            f("nVisibleBlips"),
            f("nMinWidth"),
            f("nMaxWidth"),
            f("nBlipDelay"),
            f("nAlphaAtMin"),
            f("nAlphaAtMax"),
        )
        else {
            return Ok(MultiValue::new());
        };
        let c = |k: &str| f(k).unwrap_or(255.0).clamp(0.0, 255.0) as u8;
        if let Some(b) =
            blip_name(t.raw_get::<Value>("sName").ok().as_ref()).and_then(|n| hud.radar.get_mut(&n))
        {
            b.anims.sonar = Some(crate::radar_anim::SonarAnim::new(
                left,
                texture,
                total.max(0.0) as u32,
                visible.max(0.0) as u32,
                [min, max],
                delay,
                [at_min, at_max],
                f("nGrowSpeed").unwrap_or(5.0),
                [c("nRed"), c("nGreen"), c("nBlue")],
            ));
        }
        Ok(MultiValue::new())
    })?;
    add(&radar, "UnanimateObjective", |hud, _, a| {
        let Some(Value::Table(t)) = a.get(1) else {
            return Ok(MultiValue::new());
        };
        let kind = match t.raw_get::<Value>("sType") {
            Ok(Value::String(s)) => s.to_string_lossy(),
            _ => "all".to_owned(),
        };
        if let Some(b) =
            blip_name(t.raw_get::<Value>("sName").ok().as_ref()).and_then(|n| hud.radar.get_mut(&n))
        {
            b.anims.remove(&kind, &mut b.size, &mut b.alpha);
        }
        Ok(MultiValue::new())
    })?;

    let marker = lib("Marker")?;
    // (target, icon, size, r, g, b, a, height, near, far, size, mission,
    // 2D only): returns the marker's handle.
    add(&marker, "AddBlip", |hud, _, a| {
        let handle = FIRST_MARKER + hud.next_marker;
        hud.next_marker += 1;
        let f = |i: usize, d: f64| number(a.get(i)).unwrap_or(d) as f32;
        hud.markers.insert(
            handle,
            Marker {
                target: guid(a.first()),
                icon: text(a.get(1)),
                colour: [
                    byte(a.get(3)),
                    byte(a.get(4)),
                    byte(a.get(5)),
                    byte(a.get(6)),
                ],
                height: f(7, 0.0),
                near: f(8, f64::from(MARKER_NEAR)),
                far: f(9, f64::from(MARKER_FAR)),
                sizes: [f(2, 0.0), f(10, 0.0)],
                flat: matches!(a.get(12), Some(Value::Boolean(true))),
                // A list of missions (a race's gates pass one) gives its
                // first.
                mission: match a.get(11) {
                    Some(Value::Table(t)) => t
                        .clone()
                        .pairs::<Value, Value>()
                        .filter_map(Result::ok)
                        .map(|(k, v)| format!("{}={}", shown(&k), shown(&v)))
                        .collect::<Vec<_>>()
                        .join(","),
                    Some(v) => shown(v),
                    None => "nil".to_owned(),
                },
            },
        );
        Ok(MultiValue::from_vec(vec![guid_value(handle)]))
    })?;
    // Marker.AddDisc(target, radius, r, g, b, lift): a disc on the ground
    // round an object or a point, `radius` metres, in the colour, `lift`
    // metres over the ground (inferred from the 3 call sites: 0.02 to 0.1,
    // too small for anything else; the wardrobe's is 0.5 m across its
    // 1 m reach, a delivery's its drop-off radius `fDist`). Returns its
    // handle, which `Marker.Remove` takes as a blip's.
    add(&marker, "AddDisc", |hud, _, a| {
        let handle = FIRST_MARKER + hud.next_marker;
        hud.next_marker += 1;
        let f = |i: usize, d: f64| number(a.get(i)).unwrap_or(d) as f32;
        let at = match a.first() {
            Some(Value::Table(t)) => {
                let c = |k: &str, i: i64| {
                    t.get::<Option<f32>>(k)
                        .ok()
                        .flatten()
                        .or_else(|| t.get::<Option<f32>>(i).ok().flatten())
                };
                c("x", 1)
                    .zip(c("y", 2))
                    .zip(c("z", 3))
                    .map(|((x, y), z)| [x, y, z])
            }
            _ => None,
        };
        hud.discs.insert(
            handle,
            Disc {
                target: guid(a.first()),
                at,
                radius: f(1, 1.0),
                colour: [byte(a.get(2)), byte(a.get(3)), byte(a.get(4))],
                lift: f(5, 0.0),
            },
        );
        Ok(MultiValue::from_vec(vec![guid_value(handle)]))
    })?;
    // Marker.AddTripwire(x, y, z, width, yaw, r, g, b) and
    // Marker.Add3D(target, model, r, g, b, scale): see [`Tripwire`] and
    // [`Model3d`]. Their handles come off the counter the other markers
    // use (from the 360 code: one counter, 8313452C+0x10), and
    // `Marker.Remove` takes them.
    add(&marker, "AddTripwire", |hud, _, a| {
        let f = |i: usize| number(a.get(i)).map(|v| v as f32);
        let (Some(x), Some(y), Some(z), Some(width), Some(yaw)) = (f(0), f(1), f(2), f(3), f(4))
        else {
            return Ok(MultiValue::new());
        };
        let c = |i: usize| number(a.get(i)).map_or(255, |v| v.clamp(0.0, 255.0) as u8);
        let handle = FIRST_MARKER + hud.next_marker;
        hud.next_marker += 1;
        hud.tripwires.insert(
            handle,
            Tripwire {
                at: [x, y, z],
                width: width.clamp(TRIPWIRE_WIDTH.0, TRIPWIRE_WIDTH.1),
                yaw,
                colour: [c(5), c(6), c(7)],
            },
        );
        Ok(MultiValue::from_vec(vec![guid_value(handle)]))
    })?;
    add(&marker, "Add3D", |hud, _, a| {
        let target = guid(a.first());
        if target.is_none() || !matches!(a.get(1), Some(Value::String(_))) {
            return Ok(MultiValue::new());
        }
        let c = |i: usize| number(a.get(i)).map_or(255, |v| v.clamp(0.0, 255.0) as u8);
        let handle = FIRST_MARKER + hud.next_marker;
        hud.next_marker += 1;
        hud.models.insert(
            handle,
            Model3d {
                target,
                model: text(a.get(1)),
                colour: [c(2), c(3), c(4)],
                scale: number(a.get(5)).map_or(1.0, |v| v as f32),
            },
        );
        Ok(MultiValue::from_vec(vec![guid_value(handle)]))
    })?;
    // Marker.Add(x, y, z, target, r, g, b, scale, icon): see [`Pin`]. An
    // argument of another kind is skipped, as the 360's reader does (each
    // is read from where the last one read ended).
    add(&marker, "Add", |hud, _, a| {
        let mut next = 0;
        let mut num = |d: f32| match number(a.get(next)) {
            Some(v) => {
                next += 1;
                v as f32
            }
            None => d,
        };
        let offset = [num(PIN_OFFSET[0]), num(PIN_OFFSET[1]), num(PIN_OFFSET[2])];
        let target = guid(a.get(next));
        if target.is_some() {
            next += 1;
        }
        let mut num = |d: f32| match number(a.get(next)) {
            Some(v) => {
                next += 1;
                v as f32
            }
            None => d,
        };
        let colour = [num(255.0), num(255.0), num(255.0)].map(|c| c.clamp(0.0, 255.0) as u8);
        let scale = num(PIN_SCALE);
        let icon = match a.get(next) {
            Some(Value::String(s)) => Some(s.to_string_lossy()),
            _ => None,
        };
        let handle = FIRST_MARKER + hud.next_marker;
        hud.next_marker += 1;
        hud.pins.insert(
            handle,
            Pin {
                target,
                offset,
                colour,
                scale,
                icon,
            },
        );
        Ok(MultiValue::from_vec(vec![guid_value(handle)]))
    })?;
    add(&marker, "Pulse", |hud, _, a| {
        if let Some(h) = guid(a.first()) {
            let c = |i: usize| number(a.get(i)).map_or(255, |v| v.clamp(0.0, 255.0) as u8);
            hud.pulses.insert(h, [c(1), c(2), c(3)]);
        }
        Ok(MultiValue::new())
    })?;
    add(&marker, "HaltPulse", |hud, _, a| {
        if let Some(h) = guid(a.first()) {
            hud.pulses.remove(&h);
        }
        Ok(MultiValue::new())
    })?;
    add(&marker, "Remove", |hud, _, a| {
        if let Some(h) = guid(a.first()) {
            hud.pulses.remove(&h);
            hud.markers.remove(&h);
            hud.discs.remove(&h);
            hud.tripwires.remove(&h);
            hud.models.remove(&h);
            hud.pins.remove(&h);
        }
        Ok(MultiValue::new())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{self, ScriptWorld};
    use crate::script::ScriptHost;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn texts_read_until_the_string_table_is_known() {
        assert_eq!(
            readable("[VzaCon001.Objectives.003] (0/4)"),
            "VzaCon001 objective 3 (0/4)"
        );
        assert_eq!(
            readable(
                "[objt][objdeliver] [objective.Objectivecompleted] [VzaCon001.Objectives.001]"
            ),
            "Objectivecompleted VzaCon001 objective 1"
        );
        assert_eq!(readable("plain"), "plain");
    }

    #[test]
    fn the_tables_text_replaces_the_id() {
        let mut table = StringTable::default();
        for (id, text) in [
            ("VzaCon001.Objectives.001", "Secure the beach"),
            ("objective.Objectivecompleted", "Objective completed:"),
        ] {
            table
                .strings
                .insert(m2_assets::name_hash(id), text.to_owned());
        }
        assert_eq!(
            render("[VzaCon001.Objectives.001] (0/4)", &table),
            "Secure the beach (0/4)"
        );
        assert_eq!(
            render(
                "[objt][objdeliver] [objective.Objectivecompleted] [VzaCon001.Objectives.002]",
                &table
            ),
            "Objective completed: VzaCon001 objective 2"
        );
    }

    #[test]
    fn line_breaks_stay() {
        let mut table = StringTable::default();
        for (id, text) in [("Test.First", "[pda] One[n]two"), ("Test.Second", "Three")] {
            table
                .strings
                .insert(m2_assets::name_hash(id), text.to_owned());
        }
        assert_eq!(
            render("[Test.First]\n[Test.Second]\n", &table),
            "[pda] One\ntwo\nThree"
        );
        assert_eq!(readable("[Test.First]\n[Test.Second]"), "First\nSecond");
    }

    #[test]
    fn a_marker_without_a_fade_takes_the_games_defaults() {
        let host = ScriptHost::new().unwrap();
        let world: Shared = Rc::new(RefCell::new(ScriptWorld::default()));
        api::install(host.lua(), &world).unwrap();
        // Some scripts pass nil for near (and leave far out): the game's
        // defaults, 200 and 300 m, not the beta's former 5 and 175.
        host.lua()
            .load(
                r#"
                local t = Sys.StringToGuid("734801")
                Marker.AddBlip(t, "HUD_objective_destroy", 32, 255, 0, 0, 255, 2, nil)
                "#,
            )
            .exec()
            .unwrap();
        let w = world.borrow();
        let m = w.hud.markers.values().next().unwrap();
        assert_eq!((m.near, m.far), (MARKER_NEAR, MARKER_FAR));
        assert_eq!((m.near, m.far), (200.0, 300.0));
        assert!(!m.flat);
    }

    #[test]
    fn a_race_gate_has_its_tripwire_and_models() {
        let host = ScriptHost::new().unwrap();
        let world: Shared = Rc::new(RefCell::new(ScriptWorld::default()));
        api::install(host.lua(), &world).unwrap();
        host.lua()
            .load(
                r#"
                local gate = Sys.StringToGuid("734801")
                wire = Marker.AddTripwire(10, 2, -30, 80, 90, 51, 204, 153)
                plain = Marker.AddTripwire(1, 2, 3, 0.5, 45)
                none = Marker.AddTripwire(1, 2, 3, 10)
                ring = Marker.Add3D(gate, "global_airring", 255, 200, 0, 12)
                finish = Marker.Add3D(gate, "global_tripwirefinish", 255, 200, 0)
                bare = Marker.Add3D(gate, "global_airring")
                nomodel = Marker.Add3D(gate)
                "#,
            )
            .exec()
            .unwrap();
        let g = |n: &str| as_guid(&host.lua().globals().get::<Value>(n).unwrap());
        {
            let w = world.borrow();
            // Width held to 1..50 m; colour 255 each where left out.
            let wire = &w.hud.tripwires[&g("wire").unwrap()];
            assert_eq!(wire.at, [10.0, 2.0, -30.0]);
            assert_eq!((wire.width, wire.yaw), (50.0, 90.0));
            assert_eq!(wire.colour, [51, 204, 153]);
            let plain = &w.hud.tripwires[&g("plain").unwrap()];
            assert_eq!((plain.width, plain.colour), (1.0, [255, 255, 255]));
            // Without its five numbers, or its model, nothing is made.
            assert_eq!(g("none"), None);
            assert_eq!(g("nomodel"), None);
            let ring = &w.hud.models[&g("ring").unwrap()];
            assert_eq!((ring.model.as_str(), ring.scale), ("global_airring", 12.0));
            assert_eq!(ring.colour, [255, 200, 0]);
            assert_eq!(w.hud.models[&g("finish").unwrap()].scale, 1.0);
            assert_eq!(w.hud.models[&g("bare").unwrap()].colour, [255, 255, 255]);
            // One counter for every kind of marker: no handle twice.
            let mut hs = [g("wire"), g("plain"), g("ring"), g("finish"), g("bare")];
            hs.sort();
            assert!(hs.windows(2).all(|p| p[0] != p[1]));
        }
        host.lua()
            .load("Marker.Remove(wire) Marker.Remove(ring)")
            .exec()
            .unwrap();
        let w = world.borrow();
        assert_eq!((w.hud.tripwires.len(), w.hud.models.len()), (1, 2));
    }

    #[test]
    fn someone_being_delivered_is_marked_two_metres_over_them() {
        let host = ScriptHost::new().unwrap();
        let world: Shared = Rc::new(RefCell::new(ScriptWorld::default()));
        api::install(host.lua(), &world).unwrap();
        host.lua()
            .load(
                r#"
                local vip = Sys.StringToGuid("734801")
                -- As MrxTaskObjectiveDeliver marks an attached person.
                mark = Marker.Add(0, 2, 0, vip, 255, 255, 0, 0.05)
                bare = Marker.Add()
                named = Marker.Add(1, 3, 5, vip, 10, 20, 30, 2, "HUD_objective_action")
                "#,
            )
            .exec()
            .unwrap();
        let g = |n: &str| as_guid(&host.lua().globals().get::<Value>(n).unwrap()).unwrap();
        {
            let w = world.borrow();
            assert_eq!(
                w.hud.pins[&g("mark")],
                Pin {
                    target: Some(734801),
                    offset: [0.0, 2.0, 0.0],
                    colour: [255, 255, 0],
                    scale: 0.05,
                    icon: None,
                }
            );
            // Everything left out: the game's defaults, at no target.
            assert_eq!(
                w.hud.pins[&g("bare")],
                Pin {
                    target: None,
                    offset: PIN_OFFSET,
                    colour: [255, 255, 255],
                    scale: PIN_SCALE,
                    icon: None,
                }
            );
            let named = &w.hud.pins[&g("named")];
            assert_eq!(named.offset, [1.0, 3.0, 5.0]);
            assert_eq!(named.icon.as_deref(), Some("HUD_objective_action"));
            assert_eq!((named.colour, named.scale), ([10, 20, 30], 2.0));
        }
        host.lua().load("Marker.Remove(mark)").exec().unwrap();
        assert_eq!(world.borrow().hud.pins.len(), 2);
    }

    #[test]
    fn a_marker_pulses_in_its_colour_until_halted_or_removed() {
        let host = ScriptHost::new().unwrap();
        let world: Shared = Rc::new(RefCell::new(ScriptWorld::default()));
        api::install(host.lua(), &world).unwrap();
        host.lua()
            .load(
                r#"
                local t = Sys.StringToGuid("734801")
                a = Marker.AddBlip(t, "MiniMap_Icon_Faction_PMC", 32, 255, 255, 255, 255, 2)
                b = Marker.AddBlip(t, "MiniMap_Icon_Faction_GR", 32, 255, 255, 255, 255, 2)
                Marker.Pulse(a, 255, 0, 0)
                Marker.Pulse(b)
                "#,
            )
            .exec()
            .unwrap();
        let g = |n: &str| as_guid(&host.lua().globals().get::<Value>(n).unwrap()).unwrap();
        assert_eq!(world.borrow().hud.pulses[&g("a")], [255, 0, 0]);
        assert_eq!(world.borrow().hud.pulses[&g("b")], [255, 255, 255]);
        host.lua()
            .load("Marker.HaltPulse(a) Marker.Remove(b)")
            .exec()
            .unwrap();
        let w = world.borrow();
        assert!(w.hud.pulses.is_empty());
        // Halted, the marker itself stays.
        assert!(w.hud.markers.contains_key(&g("a")));
    }

    #[test]
    fn the_scripts_set_each_views_pickup_markers() {
        let host = ScriptHost::new().unwrap();
        let world: Shared = Rc::new(RefCell::new(ScriptWorld::default()));
        api::install(host.lua(), &world).unwrap();
        assert_eq!(world.borrow().hud.pickup_markers, PickupMarkers::default());
        host.lua()
            .load(
                r#"
                Gui.SetPickupMarkerSize(18, false)
                Gui.SetPickupMarkerSize(24)
                Gui.SetPickupMarkerVisibleDistance(30, false)
                Gui.SetPickupMarkerVisibleDistance(nil, true)
                "#,
            )
            .exec()
            .unwrap();
        let p = world.borrow().hud.pickup_markers;
        // Left out, the view is the second; a size is a byte.
        assert_eq!(p.size, [18.0, 24.0]);
        // No distance, no change.
        assert_eq!(p.reach, [30.0, 100.0]);
    }

    #[test]
    fn objectives_and_markers_come_and_go() {
        let host = ScriptHost::new().unwrap();
        let world: Shared = Rc::new(RefCell::new(ScriptWorld::default()));
        api::install(host.lua(), &world).unwrap();
        host.lua()
            .load(
                r#"
                local beach = Sys.StringToGuid("734801")
                Net.SendEvent_AddPDAMission(36, {{"[VzaCon001.Objectives.001]", "[objdeliver]"}}, false)
                Net.SendEvent_AddPdaObjective("734801", beach, "[VzaCon001.Objectives.001]", 17, 36, 0, 2)
                Net.SendEvent_AddRadarObjective("734801", 0, 2, 0, 255, 200, 0, 10.667, 10.667, 2, beach, true, false, false, 5)
                Net.SendEvent_ObjectiveMessage(5, "add", "[VzaCon001.Objectives.001]", nil)
                handle = Marker.AddBlip(beach, "HUD_objective_deliverable", 32, 255, 200, 0, 255, 2, 5, 175, 32, "VzaCon001", false)
                Net.SendEvent_AddMarkerObjective(beach, handle, 255, 200, 0, 2, 8, 1, 16, false, 5, 175, 0)
                "#,
            )
            .exec()
            .unwrap();
        {
            let w = world.borrow();
            let hud = &w.hud;
            assert_eq!(
                hud.mission,
                [(
                    "[VzaCon001.Objectives.001]".to_owned(),
                    "[objdeliver]".to_owned()
                )]
            );
            assert_eq!(hud.objectives["734801"].target, Some(734_801));
            assert_eq!(hud.radar["734801"].colour, [255, 200, 0]);
            assert!(hud.radar["734801"].sticky);
            assert_eq!(hud.radar["734801"].sort, 5);
            assert_eq!(
                hud.messages,
                [ObjectiveMessage {
                    kind: "add".to_owned(),
                    icon: 5,
                    text: "[VzaCon001.Objectives.001]".to_owned(),
                }]
            );
            let m = hud.markers.values().next().unwrap();
            // The call's own fade is kept, not the defaults.
            assert_eq!((m.target, m.height), (Some(734_801), 2.0));
            assert_eq!((m.near, m.far), (5.0, 175.0));
            assert_eq!((m.sizes, m.flat), ([32.0, 32.0], false));
        }
        host.lua()
            .load(
                r#"
                Net.SendEvent_RemovePdaObjective("734801")
                Net.SendEvent_RemoveRadarObjective("734801")
                Marker.Remove(handle)
                "#,
            )
            .exec()
            .unwrap();
        let w = world.borrow();
        assert!(w.hud.objectives.is_empty() && w.hud.radar.is_empty() && w.hud.markers.is_empty());
    }

    /// The scripts' animation calls reach the blip they name, step with
    /// the HUD, and come off by kind.
    #[test]
    fn radar_blips_animate_as_the_scripts_ask() {
        let host = ScriptHost::new().unwrap();
        let world: Shared = Rc::new(RefCell::new(ScriptWorld::default()));
        api::install(host.lua(), &world).unwrap();
        host.lua()
            .load(
                r#"
                Hud.Radar:AddObjective({ sName = "db_1", nWidth = 8, nHeight = 8 })
                Hud.Radar:AnimateObjectiveSize({ sName = "db_1", nDuration = 0,
                    nMinWidth = 8, nMinHeight = 8, nMaxWidth = 12, nMaxHeight = 12 })
                Hud.Radar:AnimateObjectiveAlpha({ sName = "db_1", nDuration = 0,
                    nMinAlpha = 0.5, nMaxAlpha = 1, nSpeed = -1 })
                Hud.Radar:AnimateObjectiveSonar({ sName = "db_1", nDuration = 0,
                    sTexture = "ring", nTotalBlips = 3, nVisibleBlips = 2, nMinWidth = 2,
                    nMaxWidth = 20, nBlipDelay = 1, nAlphaAtMin = 1, nAlphaAtMax = 0 })
                -- Without its limits, or for a blip that is not there: nothing.
                Hud.Radar:AnimateObjectiveSize({ sName = "db_1", nDuration = 0 })
                Hud.Radar:AnimateObjectiveSize({ sName = "none", nDuration = 0,
                    nMinWidth = 1, nMinHeight = 1, nMaxWidth = 2, nMaxHeight = 2 })
                "#,
            )
            .exec()
            .unwrap();
        {
            let mut w = world.borrow_mut();
            assert_eq!(w.hud.radar["db_1"].anims.size.unwrap().speed, [10.0, 10.0]);
            w.hud.step_radar(0.25);
            let b = &w.hud.radar["db_1"];
            assert_eq!(b.size, [10.5, 10.5]);
            assert_eq!(b.alpha, 191);
            // The interface's speed, 5 a second: a ring at once.
            let sonar = b.anims.sonar.as_ref().unwrap();
            assert_eq!((sonar.speed, sonar.rings.as_slice()), (5.0, &[2.0][..]));
        }
        host.lua()
            .load(r#"Hud.Radar:UnanimateObjective({ sName = "db_1", sType = "size" })"#)
            .exec()
            .unwrap();
        {
            let w = world.borrow();
            let b = &w.hud.radar["db_1"];
            assert_eq!(b.size, [8.0, 8.0]);
            assert!(b.anims.size.is_none() && b.anims.alpha.is_some());
            assert!(b.anims.sonar.is_some());
        }
        host.lua()
            .load(r#"Hud.Radar:UnanimateObjective({ sName = "db_1" })"#)
            .exec()
            .unwrap();
        let w = world.borrow();
        assert_eq!(w.hud.radar["db_1"].alpha, 255);
        assert!(w.hud.radar["db_1"].anims.is_empty());
    }

    /// The object scripts' blips (`Blippable`'s `Hud.Radar:AddObjective`
    /// with a table): named by their object, with the texture's name, the
    /// size and the turning flags as given; what is left out takes the
    /// defaults; `RemoveObjective` takes the blip off by the same name.
    #[test]
    fn the_radar_takes_objectives_by_table() {
        let host = ScriptHost::new().unwrap();
        let world: Shared = Rc::new(RefCell::new(ScriptWorld::default()));
        api::install(host.lua(), &world).unwrap();
        host.lua()
            .load(
                r#"
                tank = Sys.StringToGuid("0xb4bc2")
                Hud.Radar:AddObjective({ sName = tank, nR = 100, nG = 100, nB = 100,
                    nWidth = 5, nHeight = 5, sTexture = "temp_radar_icon_tank", uGuid = tank,
                    bRotate = true, bOriented = true, bDontNetSync = true })
                Hud.Radar:AddObjective({ sName = "plain" })
                Hud.Radar:AddObjective({})
                "#,
            )
            .exec()
            .unwrap();
        {
            let w = world.borrow();
            assert_eq!(w.hud.radar.len(), 2);
            let b = &w.hud.radar["guid 0xb4bc2"];
            assert_eq!(b.target, Some(0xb4bc2));
            assert_eq!(b.texture.as_deref(), Some("temp_radar_icon_tank"));
            assert_eq!((b.colour, b.size), ([100, 100, 100], [5.0, 5.0]));
            assert!(b.rotate && b.oriented && !b.sticky);
            assert_eq!((b.offset, b.sort), ([0.0, 2.0, 0.0], 5));
            let p = &w.hud.radar["plain"];
            assert_eq!((p.target, p.texture.clone()), (None, None));
            assert_eq!((p.colour, p.size), ([255, 255, 0], [3.0, 3.0]));
        }
        host.lua()
            .load(r#"Hud.Radar:RemoveObjective({ sName = tank, bDontNetSync = true })"#)
            .exec()
            .unwrap();
        let w = world.borrow();
        assert_eq!(w.hud.radar.keys().collect::<Vec<_>>(), ["plain"]);
    }

    #[test]
    fn tutorial_lines_keep_their_arguments() {
        let host = ScriptHost::new().unwrap();
        let world: Shared = Rc::new(RefCell::new(ScriptWorld::default()));
        api::install(host.lua(), &world).unwrap();
        host.lua()
            .load(
                r#"
                Net.SetTutorialMessage(nil, "[Tutorial.OpenPDA]", 8, true)
                Net.SetTutorialMessage(nil, "")
                "#,
            )
            .exec()
            .unwrap();
        let w = world.borrow();
        let t = &w.hud.tutorials;
        assert_eq!(t[0], ["nil", "[Tutorial.OpenPDA]", "8", "true"]);
        assert_eq!(tutorial_text(&t[0]), Some("[Tutorial.OpenPDA]"));
        assert_eq!(tutorial_text(&t[1]), None);
    }
}
