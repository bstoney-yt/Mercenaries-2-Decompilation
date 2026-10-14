//! Hijacking a vehicle, as the game's own scripts play it.
//!
//! Each hijackable vehicle names a script (`hijack_m1a2`, see
//! `m2_engine::hijack`) whose global `StartHijack(hijacker, hijackee,
//! seat, vehicle)` plays the whole thing through the shared
//! `MrxActionHijack` module: sections of animation for the hijacker, the
//! crewman and the vehicle, button prompts between them (the `Minigame`
//! event), a tug of war on the hatch, and on success the crewman removed
//! and the hijacker in his seat. This module is the engine side those
//! scripts call:
//!
//! - `Human.SetState(char, "InVehicle", "ActionHijackHijackerA")` starts a
//!   character's action; the game reports its end with
//!   [`HijackWorld::action_finished`], which fires the
//!   `HumanActionComplete` event the script waits on.
//! - `Event.Minigame` reads the player's presses ([`HijackWorld::press`],
//!   [`HijackWorld::hold`]) and answers the script's callback with
//!   `"success"`, `"failed"`, `"timeout"` or, for tapping, `"update"`.
//! - Animation of the vehicle (`Object.PlayAnimation` on channel
//!   `"hijack"`), props (`Object.Attach`), the crewman's AI and seat, and
//!   the `Vehicle.Hijack*` hooks become [`HijackCommand`]s for the game.
//!
//! The call shapes are the scripts' own (rounds H2 and H3 on the test
//! PC). What the engine answered is not in the scripts; the choices made
//! here are marked "inferred".

use std::collections::HashMap;

use m2_assets::animation_table::{self, ANIMATION_TABLE_TYPE, AnimationTable};
use mlua::{Lua, MultiValue, Table, Value};

use crate::api::{Command, Shared};
use crate::events::{World, guid_value};
use crate::script::{ScriptHost, safe_function};

/// The pad buttons as the scripts' `Controller` table numbers them:
/// `RPad_Up`/`Down`/`Left`/`Right` are the face buttons Y, A, X and B,
/// `LStick_*` pushes of the left stick. Inferred from the joystick ids the
/// hijack HUD script draws (5-8 the face buttons, 9-12 the stick).
pub const BUTTONS: [(&str, u32); 8] = [
    ("RPad_Up", 5),
    ("RPad_Down", 6),
    ("RPad_Left", 7),
    ("RPad_Right", 8),
    ("LStick_Left", 9),
    ("LStick_Right", 10),
    ("LStick_Up", 11),
    ("LStick_Down", 12),
];

/// What `Player.GetCamera` answers: the one camera, as a handle.
pub const PLAYER_CAMERA: u32 = 0x2fff_ff01;

/// What a hijack asks of the game, beyond the commands every script uses.
#[derive(Clone, Debug, PartialEq)]
pub enum HijackCommand {
    /// Put `guid` in a state (`"InVehicle"`, `"Upright"`) doing `action`:
    /// an animation key the character's animation set resolves
    /// (`"ActionHijackHijackerA"`), or `"Idle"`. Report the end of a keyed
    /// action with [`HijackWorld::action_finished`].
    State {
        guid: u32,
        state: String,
        action: String,
    },
    /// Play clip `clip` (by name: `all_M1A2_driverhatch_actionhijack_
    /// getin_section01_fb`) on an object's animation channel `channel`,
    /// blending in over `blend` seconds.
    PlayAnimation {
        guid: u32,
        clip: String,
        channel: String,
        looped: bool,
        blend: f32,
    },
    /// Stop whatever plays on that channel.
    StopAnimation {
        guid: u32,
        channel: String,
    },
    /// Hang `prop` (a copy of a prop template: the pistol, the grenade)
    /// off `parent`'s node `bone`.
    Attach {
        parent: u32,
        bone: String,
        prop: u32,
    },
    Detach {
        parent: u32,
        prop: u32,
    },
    /// Knock `guid` down for `seconds`.
    Knockdown {
        guid: u32,
        seconds: f32,
    },
    /// Physics on or off for `guid` (off while the hijacker is carried by
    /// her animation).
    Physics {
        guid: u32,
        on: bool,
    },
    /// AI on or off for `guid` (off for the crewman while he fights).
    Ai {
        guid: u32,
        on: bool,
    },
    Visible {
        guid: u32,
        on: bool,
    },
    /// Turn an object's turret on or off (`main_turret`, or `"head"` for
    /// a person's head turning), or level its gun.
    Turret {
        guid: u32,
        turret: String,
        on: bool,
    },
    /// Shake the pad for `seconds`.
    Rumble {
        guid: u32,
        seconds: f32,
    },
    /// Shake the camera.
    CameraShake {
        amplitude: f32,
        seconds: f32,
    },
    /// A hijack began, ended with the hijacker in the driver's seat, or
    /// failed (she is thrown off; `AbortDone` when the crewman is back in
    /// his seat).
    Started {
        hero: u32,
        driver: u32,
        vehicle: u32,
    },
    Completed {
        hero: u32,
    },
    Aborted {
        hero: u32,
    },
    AbortDone {
        hero: u32,
    },
}

/// A minigame waiting on the player: what to do with which buttons, and
/// how long it has left.
#[derive(Clone, Debug, PartialEq)]
pub struct Minigame {
    /// `"press"`, `"hold"`, `"tap"` or `"alternate"`.
    pub mode: String,
    /// The button (two, alternating), as [`BUTTONS`] numbers.
    pub buttons: Vec<u32>,
    /// Seconds left, and how long a hold must last.
    pub left: f64,
    pub hold: Option<f64>,
}

impl Minigame {
    pub(crate) fn of(params: &Table, age: f64) -> Option<Self> {
        let num = |i: i64| -> Option<f64> {
            match params.raw_get::<Value>(i).ok()? {
                Value::Integer(n) => Some(n as f64),
                Value::Number(n) => Some(n),
                _ => None,
            }
        };
        let mode = match params.raw_get::<Value>(3).ok()? {
            Value::String(s) => s.to_string_lossy().to_ascii_lowercase(),
            _ => return None,
        };
        let buttons: Vec<u32> = match mode.as_str() {
            "alternate" => [num(4), num(5)]
                .into_iter()
                .flatten()
                .map(|b| b as u32)
                .collect(),
            _ => num(4).map(|b| b as u32).into_iter().collect(),
        };
        Some(Self {
            hold: (mode == "hold").then(|| num(5)).flatten(),
            mode,
            buttons,
            left: (num(2)? - age).max(0.0),
        })
    }
}

/// What the hijack events read: actions playing and the player's presses.
#[derive(Debug, Default)]
pub struct HijackWorld {
    /// Each character's last keyed action and whether it has ended.
    actions: HashMap<u32, (String, bool)>,
    /// Every button pressed, in order, with when (game seconds); minigames
    /// keep their place in it.
    presses: Vec<(f64, u32)>,
    /// Hardpoints' places the game knows, by object and name hash.
    hardpoints: HashMap<(u32, u32), [f32; 3]>,
    /// Aircraft in the air (helicopters are hijacked only in flight).
    flying: std::collections::BTreeSet<u32>,
    /// Buttons held down, and since when (game seconds).
    held: HashMap<u32, f64>,
    /// The hijack under way: hero, driver, vehicle.
    pub current: Option<(u32, u32, u32)>,
}

impl HijackWorld {
    /// The game finished `guid`'s action (its clip ended).
    pub fn action_finished(&mut self, guid: u32) {
        if let Some(a) = self.actions.get_mut(&guid) {
            a.1 = true;
        }
    }

    /// The action `guid` plays, if it hasn't ended.
    pub fn action(&self, guid: u32) -> Option<&str> {
        self.actions
            .get(&guid)
            .filter(|a| !a.1)
            .map(|a| a.0.as_str())
    }

    /// The player pressed `button` (a [`BUTTONS`] number) at `at` (game
    /// seconds).
    pub fn press(&mut self, button: u32, at: f64) {
        self.presses.push((at, button));
    }

    /// Whether aircraft `guid` is in the air.
    pub fn set_flying(&mut self, guid: u32, flying: bool) {
        if flying {
            self.flying.insert(guid);
        } else {
            self.flying.remove(&guid);
        }
    }

    /// Where `guid`'s hardpoint (`hp_seat_lt`, by name hash) is in the
    /// world.
    pub fn set_hardpoint(&mut self, guid: u32, name: u32, at: [f32; 3]) {
        self.hardpoints.insert((guid, name), at);
    }

    /// The player is holding `button` down since `at` (game seconds), or
    /// let it go (`None`).
    pub fn hold(&mut self, button: u32, at: Option<f64>) {
        match at {
            Some(t) => {
                self.held.entry(button).or_insert(t);
            }
            None => {
                self.held.remove(&button);
            }
        }
    }

    pub(crate) fn action_done(&self, guid: u32) -> bool {
        self.actions.get(&guid).is_some_and(|a| a.1)
    }

    pub(crate) fn presses(&self) -> &[(f64, u32)] {
        &self.presses
    }

    pub(crate) fn held_since(&self, button: u32) -> Option<f64> {
        self.held.get(&button).copied()
    }
}

/// How a `Minigame` event answers this check: callbacks to make (status,
/// value), whether it is over, how far into the presses it has read, and
/// the last button it counted (alternating).
pub(crate) struct MinigameCheck {
    pub fires: Vec<(&'static str, Option<f64>)>,
    pub done: bool,
    pub read: usize,
    pub last: Option<u32>,
}

/// Checks a `Minigame` event: `{player, seconds, mode, button[, ...]}`,
/// made at `made`, having read `read` presses so far (`None`: none yet;
/// it reads those made since it was).
///
/// - `"press"`: `"success"` on the button; another button is
///   `"failed"` (inferred: the scripts handle a failure they can't get
///   otherwise).
/// - `"hold"` (`button`, `seconds held`): `"success"` once held that long.
/// - `"tap"`: `"update"` with 0 for each press of the button (inferred: the
///   scripts add `1 - value` to the player's score, so a press counts 1).
/// - `"alternate"` (`b1`, `b2`): `"update"` for each press of either
///   that isn't the one counted last.
///
/// Each ends in `"timeout"` after its seconds; tapping and alternating
/// stay until then or until the script deletes them.
pub(crate) fn check_minigame(
    params: &Table,
    made: f64,
    now: f64,
    read: Option<usize>,
    last: Option<u32>,
    world: &dyn World,
) -> MinigameCheck {
    let num = |i: i64| -> Option<f64> {
        match params.raw_get::<Value>(i).ok()? {
            Value::Integer(n) => Some(n as f64),
            Value::Number(n) => Some(n),
            _ => None,
        }
    };
    let mode = match params.raw_get::<Value>(3) {
        Ok(Value::String(s)) => s.to_string_lossy().to_ascii_lowercase(),
        _ => String::new(),
    };
    let button = num(4).map(|b| b as u32);
    let presses = world.hijack_presses();
    let read = read.unwrap_or_else(|| presses.partition_point(|p| p.0 < made));
    let new: Vec<u32> = presses
        .get(read..)
        .unwrap_or(&[])
        .iter()
        .map(|p| p.1)
        .collect();
    let mut out = MinigameCheck {
        fires: Vec::new(),
        done: false,
        read: presses.len(),
        last,
    };
    let status = |s: &'static str, v: Option<f64>| (s, v);
    match mode.as_str() {
        "press" => {
            if let Some(&b) = new.first() {
                let ok = Some(b) == button;
                out.fires
                    .push(status(if ok { "success" } else { "failed" }, None));
                out.done = true;
                return out;
            }
        }
        "hold" => {
            let need = num(5).unwrap_or(0.0);
            if button
                .and_then(|b| world.hijack_held_since(b))
                .is_some_and(|t| now - t >= need)
            {
                out.fires.push(status("success", None));
                out.done = true;
                return out;
            }
        }
        "tap" => {
            for &b in &new {
                if Some(b) == button {
                    out.fires.push(status("update", Some(0.0)));
                }
            }
        }
        "alternate" => {
            let pair = [num(4).map(|b| b as u32), num(5).map(|b| b as u32)];
            for &b in &new {
                if pair.contains(&Some(b)) && out.last != Some(b) {
                    out.last = Some(b);
                    out.fires.push(status("update", Some(0.0)));
                }
            }
        }
        _ => {}
    }
    if num(2).is_some_and(|t| now - made >= t) {
        out.fires.push(status("timeout", None));
        out.done = true;
    }
    out
}

fn id(v: Option<&Value>) -> Option<u32> {
    match v? {
        Value::Integer(n) => u32::try_from(*n).ok(),
        Value::Number(n) if *n >= 0.0 => Some(*n as u32),
        Value::LightUserData(p) => Some(p.0 as usize as u32),
        _ => None,
    }
}

fn num(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Integer(n) => Some(*n as f32),
        Value::Number(n) => Some(*n as f32),
        _ => None,
    }
}

fn text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

fn on(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::Boolean(true)))
}

/// Installs the engine calls the hijack scripts make, over `world`. Run
/// after the other libraries are in (it adds to them).
pub fn install(host: &ScriptHost, world: &Shared) -> mlua::Result<()> {
    let lua = host.lua();
    let globals = lua.globals();
    let lib = |name: &str| -> mlua::Result<Table> {
        match globals.get::<Value>(name)? {
            Value::Table(t) => Ok(t),
            _ => {
                let t = lua.create_table()?;
                globals.raw_set(name, &t)?;
                Ok(t)
            }
        }
    };
    let add = |table: &Table,
               name: &str,
               f: fn(&mut crate::api::ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>|
     -> mlua::Result<()> {
        let w = std::rc::Rc::clone(world);
        table.raw_set(
            name,
            safe_function(lua, move |lua, args| {
                let args: Vec<Value> = args.into_iter().collect();
                f(&mut w.borrow_mut(), lua, &args)
            })?,
        )
    };

    let controller = lib("Controller")?;
    for (name, n) in BUTTONS {
        controller.raw_set(name, n)?;
    }

    let human = lib("Human")?;
    add(&human, "SetState", |w, _, a| {
        if let (Some(guid), Some(state), Some(action)) =
            (id(a.first()), text(a.get(1)), text(a.get(2)))
        {
            // A keyed action plays once and ends; an idle never does.
            if !action.eq_ignore_ascii_case("Idle") {
                w.hijack.actions.insert(guid, (action.clone(), false));
            } else {
                w.hijack.actions.remove(&guid);
            }
            w.commands.push(Command::Hijack(HijackCommand::State {
                guid,
                state,
                action,
            }));
        }
        Ok(MultiValue::new())
    })?;
    add(&human, "ForceExitSeatNoSnap", |w, _, a| {
        if let Some(guid) = id(a.first()) {
            if let Some(o) = w.objects.get_mut(&guid) {
                o.vehicle = None;
            }
            w.commands.push(Command::ExitVehicle(guid));
        }
        Ok(MultiValue::new())
    })?;
    add(&human, "Knockdown", |w, _, a| {
        if let Some(guid) = id(a.first()) {
            let seconds = num(a.get(1)).unwrap_or(0.0);
            w.commands
                .push(Command::Hijack(HijackCommand::Knockdown { guid, seconds }));
        }
        Ok(MultiValue::new())
    })?;

    let object = lib("Object")?;
    add(&object, "PlayAnimation", |w, _, a| {
        if let (Some(guid), Some(clip)) = (id(a.first()), text(a.get(1))) {
            w.commands
                .push(Command::Hijack(HijackCommand::PlayAnimation {
                    guid,
                    clip,
                    looped: on(a.get(2)),
                    channel: text(a.get(3)).unwrap_or_default(),
                    blend: num(a.get(4)).unwrap_or(0.0),
                }));
        }
        Ok(MultiValue::new())
    })?;
    add(&object, "StopAnimationChannel", |w, _, a| {
        if let Some(guid) = id(a.first()) {
            let channel = text(a.get(1)).unwrap_or_default();
            w.commands
                .push(Command::Hijack(HijackCommand::StopAnimation {
                    guid,
                    channel,
                }));
        }
        Ok(MultiValue::new())
    })?;
    // (parent, bone, template): a copy of the template hung off the
    // parent's node. The scripts keep the second value as the prop
    // (inferred: the first says it worked).
    add(&object, "Attach", |w, _, a| {
        let (Some(parent), Some(bone), Some(child)) = (id(a.first()), text(a.get(1)), id(a.get(2)))
        else {
            return Ok(MultiValue::from_vec(vec![Value::Nil, Value::Nil]));
        };
        let name = w.objects.get(&child).map(|o| o.name.clone());
        let at = w.objects.get(&parent).map(|o| o.position);
        let prop = match (name, at) {
            (Some(name), Some(at)) => Some(w.spawn(name, at, 0.0)),
            _ => None,
        };
        if let Some(prop) = prop {
            w.commands.push(Command::Hijack(HijackCommand::Attach {
                parent,
                bone,
                prop,
            }));
        }
        Ok(MultiValue::from_vec(vec![
            Value::Boolean(prop.is_some()),
            prop.map_or(Value::Nil, guid_value),
        ]))
    })?;
    add(&object, "Detach", |w, _, a| {
        if let (Some(parent), Some(prop)) = (id(a.first()), id(a.get(1))) {
            w.commands
                .push(Command::Hijack(HijackCommand::Detach { parent, prop }));
        }
        Ok(MultiValue::new())
    })?;
    for (name, flag) in [("DisablePhysics", false), ("EnablePhysics", true)] {
        let w = std::rc::Rc::clone(world);
        object.raw_set(
            name,
            safe_function(lua, move |_, args| {
                if let Some(guid) = id(args.front()) {
                    w.borrow_mut()
                        .commands
                        .push(Command::Hijack(HijackCommand::Physics { guid, on: flag }));
                }
                Ok(MultiValue::new())
            })?,
        )?;
    }
    add(&object, "SetVisible", |w, _, a| {
        if let Some(guid) = id(a.first()) {
            let on = on(a.get(1));
            // Hidden, it is off screen for `Event.ObjectIsVisible`.
            if let Some(o) = w.objects.get_mut(&guid) {
                o.hidden = !on;
            }
            w.commands
                .push(Command::Hijack(HijackCommand::Visible { guid, on }));
        }
        Ok(MultiValue::new())
    })?;

    // (object, name): a hardpoint's place, as the game last gave it; else
    // the object's own place (inferred stand-in: the explosions it places
    // then go off at the vehicle's origin, a metre or two from the hatch).
    add(&object, "GetHardpointPosition", |w, _, a| {
        let (Some(guid), Some(name)) = (id(a.first()), text(a.get(1))) else {
            return Ok(MultiValue::new());
        };
        let at = w
            .hijack
            .hardpoints
            .get(&(guid, m2_assets::name_hash(&name)))
            .copied()
            .or_else(|| w.objects.get(&guid).map(|o| o.position));
        Ok(MultiValue::from_vec(match at {
            Some(p) => p.map(|c| Value::Number(f64::from(c))).to_vec(),
            None => Vec::new(),
        }))
    })?;

    // The player's camera, for the camera calls that take one (shakes,
    // the failure's look at her chest).
    let player = lib("Player")?;
    add(&player, "GetCamera", |_, _, _| {
        Ok(MultiValue::from_vec(vec![guid_value(PLAYER_CAMERA)]))
    })?;

    let ai = lib("Ai")?;
    add(&ai, "Enable", |w, _, a| {
        if let Some(guid) = id(a.first()) {
            let on = on(a.get(1));
            w.commands
                .push(Command::Hijack(HijackCommand::Ai { guid, on }));
        }
        Ok(MultiValue::new())
    })?;

    let vehicle = lib("Vehicle")?;
    // (object, turret, on, ...): `"head"` on a person, `main_turret` on a
    // vehicle.
    add(&vehicle, "EnableTurret", |w, _, a| {
        if let (Some(guid), Some(turret)) = (id(a.first()), text(a.get(1))) {
            let on = on(a.get(2));
            w.commands
                .push(Command::Hijack(HijackCommand::Turret { guid, turret, on }));
        }
        Ok(MultiValue::new())
    })?;
    add(&vehicle, "HijackStart", |w, _, a| {
        if let (Some(hero), Some(driver), Some(vehicle)) =
            (id(a.first()), id(a.get(1)), id(a.get(2)))
        {
            w.hijack.current = Some((hero, driver, vehicle));
            w.commands.push(Command::Hijack(HijackCommand::Started {
                hero,
                driver,
                vehicle,
            }));
        }
        Ok(MultiValue::new())
    })?;
    // The hijacker takes the driver's seat.
    add(&vehicle, "HijackComplete", |w, _, a| {
        let Some(hero) = id(a.first()) else {
            return Ok(MultiValue::new());
        };
        if let Some((_, _, vehicle)) = w.hijack.current.take()
            && let Some(o) = w.objects.get_mut(&hero)
        {
            o.vehicle = Some(vehicle);
        }
        w.commands
            .push(Command::Hijack(HijackCommand::Completed { hero }));
        Ok(MultiValue::new())
    })?;
    add(&vehicle, "HijackAbort", |w, _, a| {
        if let Some(hero) = id(a.first()) {
            w.commands
                .push(Command::Hijack(HijackCommand::Aborted { hero }));
        }
        Ok(MultiValue::new())
    })?;
    add(&vehicle, "HijackAbortDone", |w, _, a| {
        if let Some(hero) = id(a.first()) {
            w.hijack.current = None;
            w.commands
                .push(Command::Hijack(HijackCommand::AbortDone { hero }));
        }
        Ok(MultiValue::new())
    })?;
    // A helicopter is hijacked only in the air (its script checks first).
    add(&vehicle, "IsFlying", |w, _, a| {
        let flying = id(a.first()).is_some_and(|g| w.hijack.flying.contains(&g));
        Ok(MultiValue::from_vec(vec![Value::Boolean(flying)]))
    })?;
    // Played here, never on another machine.
    add(&vehicle, "IsHijackRemote", |_, _, _| {
        Ok(MultiValue::from_vec(vec![Value::Boolean(false)]))
    })?;
    // The success flag and section number the scripts tell the engine:
    // nothing reads them back.
    add(
        &vehicle,
        "SetHijackSuccess",
        |_, _, _| Ok(MultiValue::new()),
    )?;
    add(&vehicle, "SetHijackState", |_, _, _| Ok(MultiValue::new()))?;

    let pg = lib("Pg")?;
    add(&pg, "Rumble", |w, _, a| {
        if let Some(guid) = id(a.first()) {
            let seconds = num(a.get(1)).unwrap_or(0.0);
            w.commands
                .push(Command::Hijack(HijackCommand::Rumble { guid, seconds }));
        }
        Ok(MultiValue::new())
    })?;
    // (camera, preset, character, amplitude, seconds).
    let camera = lib("Camera")?;
    add(&camera, "Shake", |w, _, a| {
        let amplitude = num(a.get(3)).unwrap_or(0.0);
        let seconds = num(a.get(4)).unwrap_or(0.0);
        w.commands.push(Command::Hijack(HijackCommand::CameraShake {
            amplitude,
            seconds,
        }));
        Ok(MultiValue::new())
    })?;

    // The hijack waits for the atmosphere to stop changing before it
    // brightens the bloom; nothing changes it here.
    if let Value::Table(graphics) = globals.get::<Value>("Graphics")? {
        let atmosphere = match graphics.raw_get::<Value>("Atmosphere")? {
            Value::Table(t) => t,
            _ => {
                let t = host
                    .stub("Graphics.Atmosphere")
                    .map_err(mlua::Error::external)?;
                graphics.raw_set("Atmosphere", &t)?;
                t
            }
        };
        atmosphere.raw_set("IsInterpolating", lua.create_function(|_, ()| Ok(false))?)?;
    }
    Ok(())
}

/// How long hijack actions last: what the animation tables give each
/// character for an action in a vehicle, and the clips' lengths.
#[derive(Clone, Debug, Default)]
pub struct ClipTimes {
    /// Clip lengths, seconds, by name hash: the vehicles' clips in the
    /// hijack blocks (`hijack_m1a2_...`) and the people's in the vehicle
    /// animation groups (`vehiclenameanimgroup_m1a2`, ...).
    pub lengths: HashMap<u32, f64>,
    /// The stance/action table and the vehicle handle table.
    pub tables: Option<(AnimationTable, AnimationTable)>,
    /// `VehicleAnimationSet` class and name by template name, lower case.
    pub sets: HashMap<String, (u32, u32)>,
    /// Seats (name hash, hardpoint name hash) by template name, lower
    /// case ([`m2_engine::hijack::seat_links`]).
    pub seats: HashMap<String, Vec<(u32, Option<u32>)>>,
    /// Each `SeatLink` record whole (field name hash, word) by template
    /// name, lower case ([`m2_engine::hijack::seat_link_fields`]), and
    /// the names of the templates their words name (the seats' own
    /// templates, `Vehicle Seat (Tank Default) (Driver)`): for the log.
    pub seat_fields: HashMap<String, Vec<Vec<(u32, u32)>>>,
    pub seat_templates: HashMap<u32, String>,
    /// Parts' starting states (part node name hash, state) by template
    /// name, lower case ([`m2_engine::hijack::part_states`]).
    pub parts: HashMap<String, Vec<(u32, u32)>>,
}

impl ClipTimes {
    /// Reads them from an archive (`vz.wad`): the blocks holding clips
    /// of vehicles' people and hijacks, the tables and the templates.
    pub fn read<R: std::io::Read + std::io::Seek>(
        wad: &mut m2_assets::WadArchive<R>,
    ) -> Result<Self, m2_assets::WadError> {
        use m2_assets::animation::{ANIMATION_TYPE, Clip};
        use m2_assets::layer::TEMPLATE_DB_TYPE;
        use m2_assets::{BlockContents, Layer};
        let index = wad.index();
        let paths: Vec<String> = index
            .blocks()
            .iter()
            .map(|b| b.path.to_ascii_lowercase())
            .collect();
        // The blocks holding the tables and the template database.
        let wanted: std::collections::HashSet<usize> = index
            .assets()
            .iter()
            .filter(|a| {
                let t = index.type_hash(a);
                t == ANIMATION_TABLE_TYPE || t == TEMPLATE_DB_TYPE
            })
            .map(|a| usize::from(a.block))
            .collect();
        let mut out = Self::default();
        let (mut actions, mut vehicles) = (None, None);
        for (b, path) in paths.iter().enumerate() {
            let clips = ["hijack_", "vehiclenameanimgroup_", "vehicleclassanimgroup_"]
                .iter()
                .any(|p| path.contains(p));
            if !clips && !wanted.contains(&b) {
                continue;
            }
            let Ok(data) = wad.read_block(b) else {
                continue;
            };
            let Ok(contents) = BlockContents::parse(&data) else {
                continue;
            };
            for (e, asset) in contents.iter() {
                if clips
                    && e.type_hash == ANIMATION_TYPE
                    && let Ok(clip) = Clip::parse(asset)
                {
                    out.lengths
                        .insert(e.name_hash, f64::from(clip.header.duration));
                } else if e.type_hash == ANIMATION_TABLE_TYPE {
                    if e.name_hash == animation_table::ACTIONS {
                        actions = AnimationTable::parse(asset).ok();
                    } else if e.name_hash == animation_table::VEHICLE_ANIMATIONS {
                        vehicles = AnimationTable::parse(asset).ok();
                    }
                } else if e.type_hash == TEMPLATE_DB_TYPE
                    && let Ok(db) = Layer::parse(asset)
                {
                    let sets = m2_engine::hijack::animation_sets(&db);
                    let mut seats = m2_engine::hijack::seat_links(&db);
                    let mut parts = m2_engine::hijack::part_states(&db);
                    let mut fields = m2_engine::hijack::seat_link_fields(&db);
                    let names = db.names();
                    for row in fields.values().flatten() {
                        for (_, word) in row {
                            if let Some(n) = names.get(word) {
                                out.seat_templates.insert(*word, (*n).to_owned());
                            }
                        }
                    }
                    for (id, name) in names {
                        if let Some(&s) = sets.get(&id) {
                            out.sets.insert(name.to_ascii_lowercase(), s);
                        }
                        if let Some(s) = seats.remove(&id) {
                            out.seats.insert(name.to_ascii_lowercase(), s);
                        }
                        if let Some(f) = fields.remove(&id) {
                            out.seat_fields.insert(name.to_ascii_lowercase(), f);
                        }
                        if let Some(p) = parts.remove(&id) {
                            out.parts.insert(name.to_ascii_lowercase(), p);
                        }
                    }
                }
            }
        }
        out.tables = actions.zip(vehicles);
        Ok(out)
    }

    /// The clip the animation tables give a character in a vehicle of
    /// this animation set for a state and action, and its length when
    /// the clip was read.
    pub fn clip(
        &self,
        state: &str,
        action: &str,
        (class, name): (u32, u32),
    ) -> Option<(u32, Option<f64>)> {
        let (a, b) = self.tables.as_ref()?;
        let clip = animation_table::vehicle_clip(a, b, state, action, class, name)?;
        Some((clip, self.lengths.get(&clip).copied()))
    }

    /// The clip a character sits in `seat` (a name hash) of a vehicle of
    /// this animation set with: the scripts' `InVehicle`, `Idle`.
    pub fn seated(&self, set: (u32, u32), seat: Option<u32>) -> Option<u32> {
        let (a, b) = self.tables.as_ref()?;
        animation_table::seat_clip(a, b, "InVehicle", "Idle", set, seat)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Pad(HijackWorld);

    impl World for Pad {
        fn position(&self, _: u32) -> Option<[f32; 3]> {
            None
        }
        fn is_alive(&self, _: u32) -> bool {
            true
        }
        fn vehicle_of(&self, _: u32) -> Option<u32> {
            None
        }
        fn player(&self) -> Option<u32> {
            Some(1)
        }
        fn is_awake(&self, _: u32) -> bool {
            true
        }
        fn inside(&self, _: u32, _: u32) -> Option<bool> {
            None
        }
        fn labelled(&self, _: &str) -> Vec<u32> {
            Vec::new()
        }
        fn hijack_presses(&self) -> &[(f64, u32)] {
            self.0.presses()
        }
        fn hijack_held_since(&self, b: u32) -> Option<f64> {
            self.0.held_since(b)
        }
        fn action_done(&self, g: u32) -> bool {
            self.0.action_done(g)
        }
    }

    fn game(lua: &Lua, code: &str) -> Table {
        lua.load(code).eval().unwrap()
    }

    fn statuses(c: &MinigameCheck) -> Vec<&'static str> {
        c.fires.iter().map(|f| f.0).collect()
    }

    #[test]
    fn a_press_succeeds_on_its_button_and_fails_on_another() {
        let lua = Lua::new();
        let p = game(&lua, "return {1, 0.75, 'press', 5}");
        let mut pad = Pad::default();
        let c = check_minigame(&p, 0.0, 0.1, Some(0), None, &pad);
        assert!(c.fires.is_empty() && !c.done);
        pad.0.press(5, 0.0);
        let c = check_minigame(&p, 0.0, 0.2, Some(0), None, &pad);
        assert_eq!(statuses(&c), ["success"]);
        assert!(c.done);
        pad.0.press(6, 0.0);
        let c = check_minigame(&p, 0.0, 0.3, Some(1), None, &pad);
        assert_eq!(statuses(&c), ["failed"]);
    }

    #[test]
    fn a_press_made_as_the_prompt_shows_counts_but_not_one_before() {
        let lua = Lua::new();
        let p = game(&lua, "return {1, 1, 'press', 9}");
        let mut pad = Pad::default();
        pad.0.press(9, 1.9);
        pad.0.press(9, 2.0);
        let c = check_minigame(&p, 2.0, 2.05, None, None, &pad);
        assert_eq!(statuses(&c), ["success"]);
        let c = check_minigame(&p, 2.01, 2.05, None, None, &pad);
        assert!(c.fires.is_empty());
    }

    #[test]
    fn a_press_times_out() {
        let lua = Lua::new();
        let p = game(&lua, "return {1, 0.75, 'press', 5}");
        let c = check_minigame(&p, 1.0, 1.8, None, None, &Pad::default());
        assert_eq!(statuses(&c), ["timeout"]);
    }

    #[test]
    fn tapping_counts_each_press_until_the_time_is_up() {
        let lua = Lua::new();
        let p = game(&lua, "return {1, 35, 'tap', 8}");
        let mut pad = Pad::default();
        for b in [8, 7, 8] {
            pad.0.press(b, 0.0);
        }
        let c = check_minigame(&p, 0.0, 1.0, None, None, &pad);
        assert_eq!(statuses(&c), ["update", "update"]);
        assert!(!c.done);
        assert_eq!(c.read, 3);
        assert_eq!(c.fires[0].1, Some(0.0));
        let c = check_minigame(&p, 0.0, 35.0, Some(3), None, &pad);
        assert_eq!(statuses(&c), ["timeout"]);
    }

    #[test]
    fn alternating_counts_only_changes_of_button() {
        let lua = Lua::new();
        let p = game(&lua, "return {1, 10, 'alternate', 9, 10}");
        let mut pad = Pad::default();
        for b in [9, 9, 10, 9] {
            pad.0.press(b, 0.0);
        }
        let c = check_minigame(&p, 0.0, 1.0, None, None, &pad);
        assert_eq!(c.fires.len(), 3);
        assert_eq!(c.last, Some(9));
    }

    #[test]
    fn holding_succeeds_after_its_time() {
        let lua = Lua::new();
        let p = game(&lua, "return {1, 5, 'hold', 6, 1.5}");
        let mut pad = Pad::default();
        pad.0.hold(6, Some(1.0));
        assert!(
            check_minigame(&p, 0.0, 2.0, None, None, &pad)
                .fires
                .is_empty()
        );
        let c = check_minigame(&p, 0.0, 2.6, None, None, &pad);
        assert_eq!(statuses(&c), ["success"]);
    }

    #[test]
    fn the_script_hears_each_answer_and_the_end_of_an_action() {
        let host = ScriptHost::new().unwrap();
        let lua = host.lua();
        let events = crate::events::Events::default();
        events.install(lua).unwrap();
        lua.load(
            "heard = {}\n\
             local function on(self, status, value) table.insert(heard, status .. ':' .. tostring(value)) end\n\
             Event.Create(Event.Minigame, {1, 35, 'tap', 8}, on, {{}})\n\
             Event.Create(Event.HumanActionComplete, {2}, function() table.insert(heard, 'done') end, {{}})",
        )
        .exec()
        .unwrap();
        let mut pad = Pad::default();
        events.update(0.1, &pad);
        pad.0.press(8, 0.0);
        pad.0.press(8, 0.0);
        events.update(0.1, &pad);
        pad.0
            .actions
            .insert(2, ("ActionHijackHijackerA".into(), false));
        pad.0.action_finished(2);
        events.update(0.1, &pad);
        events.update(0.1, &pad);
        let heard: Vec<String> = lua.load("return heard").eval().unwrap();
        assert_eq!(heard, ["update:0", "update:0", "done"]);
        // The tapping stays until the script deletes it or time runs out.
        assert_eq!(events.minigames().len(), 1);
    }
}
