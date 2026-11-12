//! The `Event` library the scripts schedule everything with.
//!
//! Scripts register an event with a kind, a parameter table, a callback
//! and the callback's data:
//!
//! ```lua
//! Event.Create(Event.TimerRelative, { 5 }, OnTimer, { self })
//! ```
//!
//! [`Events::update`] checks every pending event against the world each
//! frame and calls the callbacks of those that fire. An event made with
//! `Event.Create` fires once and is gone; one made with
//! `Event.CreatePersistent` stays until `Event.Delete`. `Event.Post(name,
//! data)` fires the `ScriptEvent` events listening for `name`.
//!
//! What each kind's parameters mean is read from how the scripts call it
//! (see `wad -- calls`); the readings are noted per kind below.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

use mlua::{Function, Lua, MultiValue, Table, Value};

use crate::script::safe_function;

/// What the events need to know about the world. Objects are named by
/// guid (an entity id).
pub trait World {
    /// Where an object is, if it exists.
    fn position(&self, guid: u32) -> Option<[f32; 3]>;
    /// Whether an object exists and isn't dead or destroyed.
    fn is_alive(&self, guid: u32) -> bool;
    /// Who killed an object, when known. The job scripts' hero-only
    /// bounties (`MrxTaskObjectiveDestroy._TargetDestroyed`) count only
    /// what a player's character killed.
    fn killer(&self, _guid: u32) -> Option<u32> {
        None
    }
    /// The vehicle an object rides in, if any.
    fn vehicle_of(&self, guid: u32) -> Option<u32>;
    /// The player's character (scripts write it `"hero"`).
    fn player(&self) -> Option<u32>;
    /// Whether an object is simulated near the player rather than
    /// hibernating far away.
    fn is_awake(&self, guid: u32) -> bool;
    /// Whether an object is on the player's screen: in front of her
    /// camera, inside its field of view and draw range, not hidden, and
    /// (when the game can tell) not behind something solid. `None` when
    /// the world doesn't know where the camera is.
    fn is_visible(&self, _guid: u32) -> Option<bool> {
        None
    }
    /// Seconds left of the raw clip a character was last given
    /// (`Human.PlayRawAnimation`), once the game has said how long it is;
    /// `None` while unknown.
    fn raw_clip_left(&self, _guid: u32) -> Option<f32> {
        None
    }
    /// Whether an object is inside a boundary object's area; `None` when
    /// the boundary's shape isn't known.
    fn inside(&self, guid: u32, boundary: u32) -> Option<bool>;
    /// Objects carrying a label (`"Prisoner"`), for parameters that name
    /// a kind of object rather than one.
    fn labelled(&self, label: &str) -> Vec<u32>;
    /// Gives `visit` every object with its labels as name hashes, which
    /// [`labelled`](Self::labelled) matches, so that many labels can be
    /// looked up from one reading; false if the world doesn't list them.
    fn visit_labels(&self, _visit: &mut dyn FnMut(u32, &mut dyn Iterator<Item = u32>)) -> bool {
        false
    }
    /// The game state the scripts last asked for with
    /// `Sys.RequestGameState` and the game is in (`"WaitForStreaming"`
    /// while the world streams in), if the world tracks it.
    fn game_state(&self) -> Option<String> {
        None
    }
    /// The object with this name in its layer (`"VzaCon001_StartingBoat"`),
    /// for parameters that name one object by name.
    fn named(&self, _name: &str) -> Option<u32> {
        None
    }
    /// Whether an object stands on the ground (`ObjectIsGrounded`,
    /// 0x82354AE8): true for all but vehicles off it, and for anything the
    /// game doesn't know (the 360 answers 1 without an object).
    fn grounded(&self, _guid: u32) -> bool {
        true
    }
    /// Cargo on a winch: `None` if it never was, else what carries it now
    /// (`None` once let go).
    fn winch(&self, _cargo: u32) -> Option<Option<u32>> {
        None
    }
    /// Whether `gate`, a friendly faction gate (`friendlygate`), doesn't
    /// know `a` is there: the player, or the vehicle she rides, when she
    /// hasn't honked lately (the gates' tutorial: "The faction won't know
    /// who you are or open their gate unless you honk your horn").
    fn unheard_at_gate(&self, _a: u32, _gate: u32) -> bool {
        false
    }
    /// Whether a physics event of `object` holds: `"gateFullyClosed"`
    /// while a gate the scripts shut (`Object.CloseGate`) stays shut,
    /// `"gateStuck"` never (gates shut at once here, so none sticks);
    /// `None` for one not known (`"VehicleSinking"`).
    fn physics_event(&self, _object: u32, _name: &str) -> Option<bool> {
        None
    }
    /// Whether it died: not alive, and not merely taken out of play
    /// (`ObjectDeath` fires on this; see `ScriptWorld::removed_alive`).
    fn died(&self, guid: u32) -> bool {
        !self.is_alive(guid)
    }
    /// Whether it got into (or out of) its seat at once, put there by a
    /// script.
    fn seated_instantly(&self, _guid: u32) -> bool {
        false
    }
    /// An object's labels (`"Boat"`, `"Tank"`) as name hashes, for class
    /// expressions.
    fn labels(&self, _guid: u32) -> Vec<u32> {
        Vec::new()
    }
    /// An object's health, if the world tracks it.
    fn health(&self, _guid: u32) -> Option<f32> {
        None
    }
    /// Whether a character's action (`Human.SetState`'s animation key)
    /// has played to its end.
    fn action_done(&self, _guid: u32) -> bool {
        false
    }
    /// Every pad button the player pressed, in order, for minigames
    /// ([`crate::hijack`]).
    fn hijack_presses(&self) -> &[(f64, u32)] {
        &[]
    }
    /// Since when (game seconds) the player holds a pad button down.
    fn hijack_held_since(&self, _button: u32) -> Option<f64> {
        None
    }
    /// The player's presses of the GUI's named buttons (`"selection"`,
    /// `"cancel"`), each with its game time, for `Button` events.
    fn button_presses(&self) -> &[(f64, String)] {
        &[]
    }
    /// The player's takings of context actions (`Pg.AddContextAction`),
    /// by object, each with its game time, for `ContextAction` events.
    fn context_presses(&self) -> &[(f64, u32)] {
        &[]
    }
    /// Characters' state changes (`Stance.Action`, as the human state
    /// table names them: `Upright.Idle` to `KnockedDown.Idle`), each with
    /// its game time, for `HumanStateTransition` events.
    fn state_changes(&self) -> &[StateChange] {
        &[]
    }
    /// Events met in clips the scripts played by name, by who played
    /// them, each with its game time, for `AnimationEvent` events.
    fn animation_events(&self) -> &[(f64, u32, String)] {
        &[]
    }
    /// What characters did with their weapons (`"Equip"`, `"Stow"`,
    /// `"Drop"`), each with its game time, for `WeaponEvent` events.
    fn weapon_events(&self) -> &[WeaponChange] {
        &[]
    }
}

/// Something a character did with a weapon: game time, the character,
/// what (`"Equip"`, `"Stow"`, `"Drop"`: the three the scripts wait for),
/// the weapon.
pub type WeaponChange = (f64, u32, String, u32);

/// A character's state change: game time, character, from, to.
pub type StateChange = (f64, u32, String, String);

/// Whether a state (`KnockedDown.Idle`) matches a `HumanStateTransition`
/// pattern: `*` any, `KnockedDown.*` any of that stance, else the same
/// (case aside).
pub fn state_matches(pattern: &str, state: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    match pattern.strip_suffix(".*") {
        Some(stance) => state
            .split('.')
            .next()
            .is_some_and(|s| s.eq_ignore_ascii_case(stance)),
        None => pattern.eq_ignore_ascii_case(state),
    }
}
/// What a `Button` event passes when `button` is pressed: the pad that
/// pressed it, the button's name, and "press".
///
/// From the 360 code (arguments slot 0x823576f8): the pad number first,
/// then a name for six of the nine buttons parse 0x82357328 knows, then
/// the mode ("press", "held" or "release"). "lsleft" and "lsright" pass no
/// name. "rtrigger" passes "rtrigger" and then "lbutton" as well: its jump
/// table entry runs on into the "lbutton" one. Only presses are seen here,
/// and the one local player's pad is taken as 0 (inferred).
fn button_args(button: &str) -> (Vec<Value>, Vec<&'static str>) {
    let names: &[&'static str] = match button.to_ascii_lowercase().as_str() {
        "action" => &["action"],
        "selection" => &["selection"],
        "cancel" => &["cancel"],
        "lbutton" => &["lbutton"],
        "rtrigger" => &["rtrigger", "lbutton"],
        "start" => &["start"],
        "select" => &["select"],
        _ => &[],
    };
    let mut words = names.to_vec();
    words.push("press");
    (vec![Value::Number(0.0)], words)
}

/// The kinds the scripts use, by how often.
pub const KINDS: [&str; 23] = [
    "TimerRelative",
    // Every update, in the mission's scripts too: the support manager's
    // recruit cooldowns count with it (`SupportTimer`, which takes
    // `GuiGameTimer`, there being one), and a soldier delivery's crew is
    // free for the next call 12 s after she calls it.
    "GuiUpdate",
    "GuiGameTimer",
    "ObjectHibernation",
    "Boundary",
    "ObjectDeath",
    "ObjectProximity",
    "ObjectInSeat",
    "ScriptEvent",
    "ObjectHealth",
    "ContextAction",
    "ObjectIsReady",
    "TimerAbsolute",
    "ObjectDamage",
    "GameStateChange",
    "ObjectWinched",
    "ObjectIsVisible",
    "ObjectDelete",
    "HumanStateTransition",
    "ObjectPhysicsEvent",
    "WeaponEvent",
    "ObjectIsGrounded",
    "ObjectHealthLessThan",
];

/// Kinds the game delivers itself through `Event.__m2_fire` (besides
/// every `Gui...` kind), never checked against the world: not reported
/// as unhandled.
const ENGINE_FIRED: [&str; 4] = [
    "TogglePDA",
    "PlayerCashChanged",
    "ShowSaveIcon",
    "HideSaveIcon",
];

#[derive(Debug)]
struct Pending {
    id: u32,
    kind: String,
    persistent: bool,
    /// For persistent timers: when it last fired.
    last: f64,
    /// For conditions: whether it held at the last check, so persistent
    /// events fire as it starts to hold, not every frame.
    held: bool,
    /// For conditions on each of several objects: those it held for.
    held_by: BTreeSet<u32>,
    /// For seat events: the vehicle each rider was last seen in, passed
    /// as they get out (the scripts look it up: `MrxTaskObjectiveDeliver.
    /// _OnPlayerInVehicle(self, rider, vehicle)` erred on the 0 a rider
    /// out of every vehicle has).
    rode: std::collections::BTreeMap<u32, Value>,
    /// `ObjectWinched`: the cargo's winch as last seen (who held it), none
    /// before the first look.
    winched: Option<Option<u32>>,
    /// When it was made.
    made: f64,
    /// For minigames: how many of the player's presses it has read (none
    /// before its first check: it reads those made since it was), and
    /// the button it counted last.
    read: Option<usize>,
    memo: Option<u32>,
    /// It has been checked against the world once.
    checked: bool,
    /// The scripts' state, to make tables to pass.
    lua: LuaHandle,
}

/// A pending event's values of the scripts': its parameters, its callback
/// and the callback's data. They are kept in a table of the scripts'
/// ([`Store::kept`]) by the event's id and read out for a check, not held
/// from here: each value held from Rust takes a slot of mlua's reference
/// stack, which is about 8000 deep, and three for each pending event ran
/// it out ("out of auxiliary stack space") with some 80 script-run layers
/// in at once.
#[derive(Debug)]
struct Live {
    params: Table,
    callback: Value,
    data: Option<Table>,
}

impl Live {
    /// Event id's values, kept in kept.
    fn of(kept: &Table, id: u32) -> Option<Self> {
        let entry: Table = kept.raw_get(id).ok()?;
        Some(Self {
            params: entry.raw_get(1).ok()?,
            callback: entry.raw_get(2).ok()?,
            data: entry.raw_get(3).ok()?,
        })
    }
}

/// The scripts' state, weakly (the state holds the events).
struct LuaHandle(mlua::WeakLua);

impl std::fmt::Debug for LuaHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LuaHandle")
    }
}

#[derive(Debug, Default)]
struct Store {
    next_id: u32,
    pending: Vec<Pending>,
    /// The pending events' values, by id ([`Live`]): a table of the
    /// scripts' state, made as the library is installed.
    kept: Option<Table>,
    /// `Event.Post` calls not yet delivered: name and data.
    posted: Vec<(String, Value)>,
    /// The names whose `Event.Post`s are copied out too
    /// ([`Events::relay`]), and the copies not yet taken.
    relay: BTreeSet<String>,
    relayed: Vec<(String, Value)>,
    /// Events deleted since the last check began: what fired with them
    /// in that check doesn't run ([`fire`]).
    deleted: BTreeSet<u32>,
    /// With `M2_STUB_ARGS`: events made and deleted, for diagnostics.
    trace: Vec<String>,
    /// Game time, seconds.
    now: f64,
    /// The Lua state, for the strings minigames answer with.
    lua: LuaRef,
    /// Kinds pending events were made of that [`check`] doesn't know
    /// (they never fire), each once; and those not yet taken for the log.
    unhandled: BTreeSet<String>,
    fresh_unhandled: Vec<String>,
}

#[derive(Default)]
struct LuaRef(Option<mlua::WeakLua>);

impl std::fmt::Debug for LuaRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LuaRef")
    }
}

impl Store {
    /// A pending event's values, read out of the scripts' table.
    fn live(&self, p: &Pending) -> Option<Live> {
        Live::of(self.kept.as_ref()?, p.id)
    }

    /// Lets go of the values of events no longer pending.
    fn forget(&self, ids: &[u32]) {
        if let Some(kept) = &self.kept {
            for &id in ids {
                let _ = kept.raw_set(id, Value::Nil);
            }
        }
    }
}

/// The pending events, shared with the `Event` library.
#[derive(Debug, Clone, Default)]
pub struct Events {
    store: Rc<RefCell<Store>>,
}

/// A callback that fired and failed.
#[derive(Debug)]
pub struct EventError {
    pub kind: String,
    pub error: mlua::Error,
}

impl Events {
    /// Installs the `Event` library: the kind constants and `Create`,
    /// `CreatePersistent`, `Delete` and `Post`.
    pub fn install(&self, lua: &Lua) -> mlua::Result<()> {
        {
            let mut s = self.store.borrow_mut();
            s.lua = LuaRef(Some(lua.weak()));
            s.kept = Some(lua.create_table()?);
        }
        let event = lua.create_table()?;
        for kind in KINDS {
            event.set(kind, kind)?;
        }
        for (name, persistent) in [("Create", false), ("CreatePersistent", true)] {
            let store = Rc::clone(&self.store);
            event.set(
                name,
                safe_function(lua, move |lua, args| {
                    let mut args = args.into_iter();
                    let kind = match args.next() {
                        Some(Value::String(s)) => s.to_string_lossy(),
                        other => {
                            return Err(mlua::Error::runtime(format!(
                                "Event.{name}: unknown event kind {other:?}"
                            )));
                        }
                    };
                    let params = match args.next() {
                        Some(Value::Table(t)) => t,
                        _ => {
                            return Err(mlua::Error::runtime(format!(
                                "Event.{name}({kind}): parameters must be a table"
                            )));
                        }
                    };
                    let callback = args.next().unwrap_or(Value::Nil);
                    let data = match args.next() {
                        Some(Value::Table(t)) => Some(t),
                        _ => None,
                    };
                    let mut s = store.borrow_mut();
                    s.next_id += 1;
                    let id = s.next_id;
                    let now = s.now;
                    if crate::script::stub_args() && !kind.starts_with("Gui") {
                        let shown: Vec<String> = params
                            .clone()
                            .sequence_values::<Value>()
                            .filter_map(Result::ok)
                            .map(|v| crate::script::show_arg(&v))
                            .collect();
                        s.trace
                            .push(format!("{name} {id} {kind}{{{}}}", shown.join(", ")));
                    }
                    if let Some(kept) = &s.kept {
                        let entry = lua.create_table_with_capacity(3, 0)?;
                        entry.raw_set(1, params)?;
                        entry.raw_set(2, callback)?;
                        entry.raw_set(3, data)?;
                        kept.raw_set(id, entry)?;
                    }
                    s.pending.push(Pending {
                        id,
                        kind,
                        persistent,
                        last: now,
                        held: false,
                        held_by: BTreeSet::new(),
                        rode: std::collections::BTreeMap::new(),
                        winched: None,
                        made: now,
                        read: None,
                        memo: None,
                        checked: false,
                        lua: LuaHandle(lua.weak()),
                    });
                    Ok(MultiValue::from_vec(vec![Value::Number(f64::from(id))]))
                })?,
            )?;
        }
        let store = Rc::clone(&self.store);
        event.set(
            "Delete",
            safe_function(lua, move |_, args| {
                if let Some(id) = args.front().and_then(as_id) {
                    let mut s = store.borrow_mut();
                    if crate::script::stub_args() {
                        let gone = s
                            .pending
                            .iter()
                            .find(|p| p.id == id)
                            .map(|p| p.kind.clone());
                        s.trace.push(format!(
                            "Delete {id} ({})",
                            gone.as_deref().unwrap_or("none")
                        ));
                    }
                    // From the 360 code (0x823583d8): it answers whether
                    // there was such an event to delete.
                    let found = s.pending.iter().any(|p| p.id == id);
                    s.deleted.insert(id);
                    s.pending.retain(|p| p.id != id);
                    s.forget(&[id]);
                    return Ok(MultiValue::from_vec(vec![Value::Boolean(found)]));
                }
                Ok(MultiValue::from_vec(vec![Value::Boolean(false)]))
            })?,
        )?;
        let store = Rc::clone(&self.store);
        event.set(
            "Post",
            safe_function(lua, move |_, args| {
                let mut args = args.into_iter();
                if let Some(Value::String(name)) = args.next() {
                    let data = args.next();
                    let has_data = data.is_some();
                    let data = data.unwrap_or(Value::Nil);
                    let name = name.to_string_lossy();
                    let mut s = store.borrow_mut();
                    if s.relay.contains(&name) {
                        s.relayed.push((name.clone(), data.clone()));
                    }
                    s.posted.push((name, data));
                    // From the 360 code (0x82358470): true once it has a
                    // name and something to send with it, else false.
                    return Ok(MultiValue::from_vec(vec![Value::Boolean(has_data)]));
                }
                Ok(MultiValue::from_vec(vec![Value::Boolean(false)]))
            })?,
        )?;
        // `Event.__m2_fire(kind, ...)`: the engine's own events (the GUI's
        // `GuiHealthUpdate` and the like): calls the callbacks of pending
        // events of that kind with their data then the arguments (one-shot
        // events are used up). Returns how many ran.
        let store = Rc::clone(&self.store);
        event.set(
            "__m2_fire",
            safe_function(lua, move |_, args| {
                let mut args = args.into_iter();
                let Some(Value::String(kind)) = args.next() else {
                    return Ok(MultiValue::from_vec(vec![Value::Integer(0)]));
                };
                let kind = kind.to_string_lossy();
                let extra: Vec<Value> = args.collect();
                let hits: Vec<(Value, Option<Table>)> = {
                    let mut s = store.borrow_mut();
                    let hits = s
                        .pending
                        .iter()
                        .filter(|p| p.kind == kind)
                        .filter_map(|p| s.live(p))
                        .map(|h| (h.callback, h.data))
                        .collect();
                    let gone: Vec<u32> = s
                        .pending
                        .iter()
                        .filter(|p| !p.persistent && p.kind == kind)
                        .map(|p| p.id)
                        .collect();
                    s.pending.retain(|p| p.persistent || p.kind != kind);
                    s.forget(&gone);
                    hits
                };
                let mut ran = 0;
                for (callback, data) in hits {
                    if let Value::Function(f) = callback {
                        call(&f, data.as_ref(), extra.clone())?;
                        ran += 1;
                    }
                }
                Ok(MultiValue::from_vec(vec![Value::Integer(ran)]))
            })?,
        )?;
        // Kinds not listed (`Event.Minigame`) read as their own name, so
        // events of them can be made and simply never fire.
        let meta = lua.create_table()?;
        meta.set(
            "__index",
            lua.create_function(|_, (_, k): (Value, Value)| Ok(k))?,
        )?;
        event.set_metatable(Some(meta))?;
        lua.globals().set("Event", event)?;
        Ok(())
    }

    /// The minigames waiting on the player's pad, for the prompt.
    pub fn minigames(&self) -> Vec<crate::hijack::Minigame> {
        let s = self.store.borrow();
        s.pending
            .iter()
            .filter(|p| p.kind == "Minigame")
            .filter_map(|p| crate::hijack::Minigame::of(&s.live(p)?.params, s.now - p.made))
            .collect()
    }

    /// Events made and deleted since the last call (with `M2_STUB_ARGS`).
    pub fn take_trace(&self) -> Vec<String> {
        std::mem::take(&mut self.store.borrow_mut().trace)
    }

    /// Kinds of pending events no check knows, first met since the last
    /// call: each is reported once, so the kinds still missing show in
    /// the log (an `ObjectIsVisible` that never fired was PmcCon001's
    /// soft lock at the villa).
    pub fn take_unhandled(&self) -> Vec<String> {
        std::mem::take(&mut self.store.borrow_mut().fresh_unhandled)
    }

    /// The objects pending `ObjectIsVisible` events wait to see, for the
    /// game to look whether something solid stands between each and the
    /// camera.
    pub fn sight_watched(&self, world: &dyn World) -> Vec<u32> {
        let s = self.store.borrow();
        let mut out: Vec<u32> = s
            .pending
            .iter()
            .filter(|p| p.kind == "ObjectIsVisible")
            .filter_map(|p| s.live(p))
            .flat_map(|h| objects(&h.params, 1, world))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The regions pending `Boundary` events wait for something to leave
    /// (mode `"exit"`: PmcCon034's `PMCCon034OutOfBounds`, which cancels
    /// the contract), for the autoplay to keep her within.
    pub fn exits_watched(&self, world: &dyn World) -> Vec<u32> {
        let s = self.store.borrow();
        let mut out: Vec<u32> = s
            .pending
            .iter()
            .filter(|p| p.kind == "Boundary")
            .filter_map(|p| s.live(p))
            .filter(|h| text(&h.params, 3).as_deref() == Some("exit"))
            .filter_map(|h| boundary_of(&h.params, world))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// How many events are pending.
    pub fn len(&self) -> usize {
        self.store.borrow().pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Pending events by kind, for diagnostics.
    pub fn kinds(&self) -> Vec<String> {
        self.store
            .borrow()
            .pending
            .iter()
            .map(|p| p.kind.clone())
            .collect()
    }

    /// Pending events with their parameters, for diagnostics:
    /// `ObjectHealth{1, "<", 100}`.
    pub fn describe(&self) -> Vec<String> {
        let s = self.store.borrow();
        s.pending
            .iter()
            .map(|p| {
                let params: Vec<String> = s
                    .live(p)
                    .into_iter()
                    .flat_map(|h| h.params.sequence_values::<Value>().collect::<Vec<_>>())
                    .filter_map(Result::ok)
                    .map(|v| crate::script::show_arg(&v))
                    .collect();
                format!("{}{{{}}}", p.kind, params.join(", "))
            })
            .collect()
    }

    /// The names pending `ScriptEvent` events wait to be posted.
    pub fn awaited(&self) -> Vec<String> {
        let s = self.store.borrow();
        s.pending
            .iter()
            .filter(|p| p.kind == "ScriptEvent")
            .filter_map(|p| s.live(p)?.params.raw_get::<String>(1).ok())
            .collect()
    }

    /// Posts `name` with no data, as `Event.Post(name)` does: the engine's
    /// own posts (the satellite's view, say). It is delivered at the next
    /// update.
    pub fn post(&self, name: &str) {
        self.post_data(name, Value::Nil);
    }

    /// Posts `name` with `data`, as `Event.Post(name, data)` does.
    pub fn post_data(&self, name: &str, data: Value) {
        self.store.borrow_mut().posted.push((name.to_owned(), data));
    }

    /// Copies the scripts' own `Event.Post`s of `names` out as well, for
    /// another script state to hear ([`Events::take_relayed`]): the game
    /// runs its interface's scripts and its contracts in one state, so
    /// what `MrxGuiPda` posts the contracts hear.
    pub fn relay(&self, names: &[&str]) {
        self.store.borrow_mut().relay = names.iter().map(|n| (*n).to_owned()).collect();
    }

    /// The scripts' posts of the [`Events::relay`] names since the last
    /// call, oldest first.
    pub fn take_relayed(&self) -> Vec<(String, Value)> {
        std::mem::take(&mut self.store.borrow_mut().relayed)
    }

    /// Advances game time by `dt` seconds and fires every event whose
    /// condition holds. Callbacks run after the checks, so events they
    /// create wait for the next update. Returns the callbacks that failed.
    pub fn update(&self, dt: f64, world: &dyn World) -> Vec<EventError> {
        let (accepted, mut errors) = self.filter_posts();
        let fired = self.due(dt, world, &accepted);
        errors.extend(fire(&self.store, fired));
        errors
    }

    /// [`Events::update`] for a world the scripts also change: it is
    /// borrowed only while the conditions are checked, so callbacks can
    /// move, spawn and kill.
    pub fn update_shared<W: World>(&self, dt: f64, world: &RefCell<W>) -> Vec<EventError> {
        let (accepted, mut errors) = self.filter_posts();
        let fired = self.due(dt, &*world.borrow(), &accepted);
        errors.extend(fire(&self.store, fired));
        errors
    }

    /// Asks each waiting `ScriptEvent`'s filter about each post, before
    /// anything is borrowed: filters are script functions and may call
    /// the game (`Debug.Printf`). The (filter, post) pairs it says yes to,
    /// and the filters that failed.
    fn filter_posts(&self) -> (Accepted, Vec<EventError>) {
        let (posted, filters) = {
            let s = self.store.borrow();
            let filters: Vec<(String, Function)> = s
                .pending
                .iter()
                .filter(|p| p.kind == "ScriptEvent")
                .filter_map(|p| {
                    let h = s.live(p)?;
                    let name = h.params.raw_get::<String>(1).ok()?;
                    let f = h.params.raw_get::<Option<Function>>(2).ok().flatten()?;
                    Some((name, f))
                })
                .collect();
            if filters.is_empty() {
                return (Accepted::new(), Vec::new());
            }
            (s.posted.clone(), filters)
        };
        let (mut accepted, mut errors) = (Accepted::new(), Vec::new());
        for (name, f) in &filters {
            for (i, (n, data)) in posted.iter().enumerate().filter(|(_, (n, _))| n == name) {
                match f.call::<Value>(data.clone()) {
                    Ok(Value::Nil | Value::Boolean(false)) => {}
                    Ok(_) => {
                        accepted.insert((f.to_pointer() as usize, i));
                    }
                    Err(error) => errors.push(EventError {
                        kind: format!("ScriptEvent {n} filter"),
                        error,
                    }),
                }
            }
        }
        (accepted, errors)
    }

    fn due(&self, dt: f64, world: &dyn World, accepted: &Accepted) -> Vec<Fired> {
        // The world is borrowed and unchanged while the events are checked.
        let _memo = LabelMemo::start();
        {
            let mut s = self.store.borrow_mut();
            s.now += dt;
            let now = s.now;
            let posted = std::mem::take(&mut s.posted);
            s.deleted.clear();
            let mut fired = Vec::new();
            let mut keep = Vec::with_capacity(s.pending.len());
            let before: Vec<u32> = s.pending.iter().map(|p| p.id).collect();
            for mut p in std::mem::take(&mut s.pending) {
                // An event whose values are gone can't be checked or run.
                let Some(mut h) = s.live(&p) else {
                    continue;
                };
                // The wildcard characters stand for her ([`with_wildcards`]).
                with_wildcards(&mut h, &p, world);
                let checked = check(&p, &h, now, &posted, accepted, world);
                let extra = match checked {
                    Check::Fire(extra) => Some(vec![extra]),
                    Check::FireWords(mut extra, words) => {
                        let lua = s.lua.0.as_ref().and_then(mlua::WeakLua::try_upgrade);
                        extra.extend(words.iter().filter_map(|w| {
                            lua.as_ref()?.create_string(w).ok().map(Value::String)
                        }));
                        Some(vec![extra])
                    }
                    // Fires as the condition starts to hold.
                    Check::Holds(holds, extra) => {
                        let rising = holds && !p.held;
                        p.held = holds;
                        rising.then_some(vec![extra])
                    }
                    Check::Health(holds, health, comparison) => {
                        let rising = holds && !p.held;
                        p.held = holds;
                        let lua = s.lua.0.as_ref().and_then(mlua::WeakLua::try_upgrade);
                        let text = lua.as_ref().and_then(|l| l.create_string(comparison).ok());
                        rising.then(|| {
                            let mut extra = vec![Value::Number(health)];
                            extra.extend(text.map(Value::String));
                            vec![extra]
                        })
                    }
                    // `held_by` keeps who was seated at the last check
                    // (none before the first).
                    Check::Seats(riders) => {
                        let mode = seat_mode(&h);
                        let any = mode.contains('a');
                        let mut fire = Vec::new();
                        let mut seated = BTreeSet::new();
                        for (r, (inside, instantly), mut extra) in riders {
                            let was = p.held_by.contains(&r);
                            if inside {
                                seated.insert(r);
                                if let Some(v) = extra.get(1) {
                                    p.rode.insert(r, v.clone());
                                }
                            } else if let Some(v) = p.rode.remove(&r)
                                && let Some(slot) = extra.get_mut(1)
                            {
                                *slot = v;
                            }
                            let (inn, out) = if instantly { ('i', 'o') } else { ('e', 'x') };
                            let got_in = inside && !was;
                            let got_out = !inside && was;
                            if (got_in && (any || mode.contains(inn)))
                                || (got_out && (any || mode.contains(out)))
                            {
                                fire.push(extra);
                            }
                        }
                        p.held_by = seated;
                        if !p.persistent {
                            fire.truncate(1);
                        }
                        (!fire.is_empty()).then_some(fire)
                    }
                    // Fires for each object as the condition starts to
                    // hold for it; once only, unless persistent.
                    Check::Each(members) => {
                        // A death over a label (alone or a filter's) counts
                        // those dying from when it was made: who of the
                        // label lay dead at its first check doesn't fire it
                        // (playtest 5: the villa's wave objective, 10 of
                        // label `VZ`, was met at once by the VZ she had
                        // killed before). Objects named one by one still
                        // fire, dead already or not.
                        if !p.checked && p.kind == "ObjectDeath" {
                            let labelled = by_label(&h.params, world);
                            p.held_by = members
                                .iter()
                                .filter(|m| m.1 && labelled.contains(&m.0))
                                .map(|m| m.0)
                                .collect();
                        }
                        p.checked = true;
                        let mut rising = Vec::new();
                        let mut held_by = BTreeSet::new();
                        for (g, holds, extra) in members {
                            if holds {
                                held_by.insert(g);
                                if !p.held_by.contains(&g) {
                                    rising.push(extra);
                                }
                            }
                        }
                        p.held_by = held_by;
                        if !p.persistent {
                            rising.truncate(1);
                        }
                        (!rising.is_empty()).then_some(rising)
                    }
                    // `held_by` keeps who was inside at the last check
                    // (none before the first: one inside then enters).
                    // The winch's changes since the last look, as the
                    // 360's attach (1) and detach (2) occurrences.
                    Check::Winch(cargo, now, carrier, mode) => {
                        let before = p.winched.replace(now);
                        let change = match (before, now) {
                            (Some(was), now) if was != now => match (was, now) {
                                (Some(c), None) => Some((c, 2u8)),
                                (_, Some(c)) => Some((c, 1u8)),
                                _ => None,
                            },
                            _ => None,
                        };
                        match change {
                            Some((c, bit)) if bit & mode != 0 && (carrier == 0 || carrier == c) => {
                                let lua = s.lua.0.as_ref().and_then(mlua::WeakLua::try_upgrade);
                                let word = lua
                                    .as_ref()
                                    .and_then(|l| {
                                        l.create_string(if bit == 1 { "attach" } else { "detach" })
                                            .ok()
                                    })
                                    .map_or(Value::Nil, Value::String);
                                Some(vec![vec![guid_value(cargo), guid_value(c), word]])
                            }
                            _ => None,
                        }
                    }
                    Check::Crossings(members) => {
                        let lua = s.lua.0.as_ref().and_then(mlua::WeakLua::try_upgrade);
                        let mut crossed = Vec::new();
                        let mut inside = BTreeSet::new();
                        for (g, holds, mut extra) in members {
                            if holds {
                                inside.insert(g);
                            }
                            if holds != p.held_by.contains(&g)
                                && let Some(word) = lua.as_ref().and_then(|l| {
                                    l.create_string(if holds { "enter" } else { "exit" }).ok()
                                })
                            {
                                extra.push(Value::String(word));
                                crossed.push(extra);
                            }
                        }
                        p.held_by = inside;
                        if !p.persistent {
                            crossed.truncate(1);
                        }
                        (!crossed.is_empty()).then_some(crossed)
                    }
                    // Never fires; its kind is reported once.
                    Check::Unhandled => {
                        if s.unhandled.insert(p.kind.clone()) {
                            s.fresh_unhandled.push(p.kind.clone());
                        }
                        keep.push(p);
                        continue;
                    }
                    Check::Minigame(m) => {
                        p.read = Some(m.read);
                        p.memo = m.last;
                        let lua = s.lua.0.as_ref().and_then(mlua::WeakLua::try_upgrade);
                        for (status, value) in m.fires {
                            let Some(text) =
                                lua.as_ref().and_then(|l| l.create_string(status).ok())
                            else {
                                continue;
                            };
                            let mut extra = vec![Value::String(text)];
                            extra.extend(value.map(Value::Number));
                            fired.push((p.id, p.kind.clone(), extra));
                        }
                        if !m.done {
                            keep.push(p);
                        }
                        continue;
                    }
                };
                match extra {
                    Some(extras) => {
                        for extra in extras {
                            fired.push((p.id, p.kind.clone(), extra));
                        }
                        if p.persistent {
                            // A repeating timer keeps what it overran by
                            // (from the 360 code: 0x823554d0 takes the
                            // delay off the time counted, 0x82355480
                            // counts on from there), so it keeps its rate
                            // whatever the frame length.
                            p.last = match (p.kind.as_str(), number(&h.params, 1)) {
                                ("TimerRelative", Some(delay)) => p.last + delay,
                                _ => now,
                            };
                            keep.push(p);
                        }
                    }
                    None => keep.push(p),
                }
            }
            s.pending = keep;
            // Those that fired keep their values until they have run
            // ([`fire`]).
            let gone: Vec<u32> = if s.pending.len() == before.len() {
                Vec::new()
            } else {
                let stay: std::collections::HashSet<u32> = s
                    .pending
                    .iter()
                    .map(|p| p.id)
                    .chain(fired.iter().map(|f: &Fired| f.0))
                    .collect();
                before.into_iter().filter(|id| !stay.contains(id)).collect()
            };
            s.forget(&gone);
            // Proximities that hold at once go newest first: OilCon002's
            // transit teleports her, and its arrival check (made as the
            // trip began) is to end the trip before the older one that
            // cancels it once she is 20 m from the helicopter (its
            // `fOnComplete` deletes that one). Inferred from the scripts;
            // the engine's own order is not read.
            let slots: Vec<usize> = (0..fired.len())
                .filter(|&i| fired[i].1 == "ObjectProximity")
                .collect();
            let mut near: Vec<Fired> = slots
                .iter()
                .map(|&i| std::mem::take(&mut fired[i]))
                .collect();
            near.sort_by_key(|f| std::cmp::Reverse(f.0));
            for (slot, f) in slots.into_iter().zip(near) {
                fired[slot] = f;
            }
            fired
        }
    }
}

/// An event that fired: its id, kind, and the values to pass after its
/// data. Its callback and data are read from the scripts' table as it
/// runs, not held meanwhile.
type Fired = (u32, String, Vec<Value>);

/// Runs the callbacks of the events that fired, in turn: one deleted by
/// an earlier callback of the same pass doesn't run (OilCon002's transit:
/// the trip cancelled at the teleport, its arrival check then ran on the
/// cleaned-up objective).
fn fire(store: &RefCell<Store>, fired: Vec<Fired>) -> Vec<EventError> {
    let mut errors = Vec::new();
    let ids: Vec<u32> = fired.iter().map(|f| f.0).collect();
    for (id, kind, extra) in fired {
        let live = {
            let s = store.borrow();
            if s.deleted.contains(&id) {
                continue;
            }
            s.kept.as_ref().and_then(|kept| Live::of(kept, id))
        };
        let Some(Live {
            callback: Value::Function(f),
            data,
            ..
        }) = live
        else {
            continue;
        };
        if let Err(error) = call(&f, data.as_ref(), extra) {
            errors.push(EventError { kind, error });
        }
    }
    // The one-shots among them are used up: their values let go.
    if !ids.is_empty() {
        let s = store.borrow();
        let pending: std::collections::HashSet<u32> = s.pending.iter().map(|p| p.id).collect();
        let gone: Vec<u32> = ids.into_iter().filter(|id| !pending.contains(id)).collect();
        s.forget(&gone);
    }
    errors
}

/// `Player.GetAnyCharacter()`'s answer: the wildcard for any player's
/// character (0x8233e8c8).
pub const ANY_CHARACTER: u32 = 0xf000_0000;
/// `Player.GetAllCharacters()`'s answer: the wildcard for every player's
/// character (0x8233e928).
pub const ALL_CHARACTERS: u32 = 0xf000_0001;

/// Puts the player's character in the place of the wildcard characters in
/// a pending event's parameters, for one check; returns the scripts' own
/// table to put back. None when there are none.
///
/// From the 360 code: an event subject of `ANY_CHARACTER` holds when the
/// condition holds for any active player's character; `ALL_CHARACTERS`
/// only when it holds for every one (a bit per player, 0x82359f80). With
/// one player both are her. Other natives handed a wildcard find no object
/// in it, as on the 360.
fn with_wildcards(h: &mut Live, p: &Pending, world: &dyn World) {
    let wild = |v: &Value| {
        matches!(as_guid(v), Some(ANY_CHARACTER | ALL_CHARACTERS))
            && matches!(v, Value::LightUserData(_))
    };
    let values: Vec<(Value, Value)> = h
        .params
        .pairs::<Value, Value>()
        .filter_map(Result::ok)
        .collect();
    if !values.iter().any(|(_, v)| wild(v)) {
        return;
    }
    let Some(lua) = p.lua.0.try_upgrade() else {
        return;
    };
    let her = world.player().map_or(Value::Nil, guid_value);
    let Ok(copy) = lua.create_table() else {
        return;
    };
    for (k, v) in values {
        let v = if wild(&v) { her.clone() } else { v };
        if copy.raw_set(k, v).is_err() {
            return;
        }
    }
    h.params = copy;
}
/// A guid as scripts see it: userdata, as in the game (scripts check
/// `type(target) == "userdata"`). Light userdata compares by value, so the
/// same guid is equal to itself and works as a table key.
pub fn guid_value(guid: u32) -> Value {
    Value::LightUserData(mlua::LightUserData(guid as usize as *mut std::ffi::c_void))
}

/// The guid a script value names: guid userdata, or a number (scripts and
/// tests write some guids as numbers).
pub fn as_guid(v: &Value) -> Option<u32> {
    match v {
        Value::LightUserData(p) => u32::try_from(p.0 as usize).ok(),
        Value::Integer(i) => u32::try_from(*i).ok(),
        Value::Number(n) if *n >= 0.0 => Some(*n as u32),
        _ => None,
    }
}

fn as_id(v: &Value) -> Option<u32> {
    as_guid(v)
}

fn number(t: &Table, i: i64) -> Option<f64> {
    match t.raw_get::<Value>(i).ok()? {
        Value::Integer(n) => Some(n as f64),
        Value::Number(n) => Some(n),
        _ => None,
    }
}

fn guid(t: &Table, i: i64) -> Option<u32> {
    as_id(&t.raw_get::<Value>(i).ok()?)
}

/// Whether an object with `labels` is of class `expr`: labels joined by
/// `&&`, each maybe negated with `!` (`"Tank && !APC"`), in any case.
pub(crate) fn class_matches(expr: &str, labels: &[u32]) -> bool {
    class_alternatives(expr)
        .iter()
        .any(|terms| terms.iter().all(|&(h, not)| labels.contains(&h) != not))
}

/// A class expression's alternatives, each as [`class_terms`] reads one:
/// `||` between them, at the top or in brackets (the friendly gates'
/// filter, `"Hero||(Allied&&Vehicle)"`: the hero, or an Allied
/// vehicle).
pub(crate) fn class_alternatives(expr: &str) -> Vec<Vec<(u32, bool)>> {
    expr.split("||")
        .map(|alt| class_terms(alt.trim().trim_start_matches('(').trim_end_matches(')')))
        .collect()
}

/// A class expression's labels as name hashes, each with whether it is
/// negated.
pub(crate) fn class_terms(expr: &str) -> Vec<(u32, bool)> {
    expr.split("&&")
        .map(|term| {
            let term = term.trim();
            match term.strip_prefix('!') {
                Some(not) => (m2_assets::name_hash(not.trim()), true),
                None => (m2_assets::name_hash(term), false),
            }
        })
        .collect()
}

/// How a seat event's mode letters (`"E"`, `"xo"`, ...) read; the old
/// words `"enter"` and `"exit"` as `"ei"` and `"xo"`.
fn seat_mode(h: &Live) -> String {
    match text(&h.params, 4).as_deref() {
        Some("enter") | None => "ei".to_owned(),
        Some("exit") => "xo".to_owned(),
        Some(m) => m.to_owned(),
    }
}

enum Check {
    /// Fire now with these values after the callback's data.
    Fire(Vec<Value>),
    /// Fire now with these values and then these words as strings.
    FireWords(Vec<Value>, Vec<&'static str>),
    /// A condition: whether it holds, and the values to pass when it
    /// starts to.
    Holds(bool, Vec<Value>),
    /// ObjectHealth's condition: whether it holds, the health that made
    /// it, and the comparison as text, which it passes (both, no guid).
    Health(bool, f64, &'static str),
    /// Riders and whether each is in a wanted seat, with the values to
    /// pass: fired by the event's mode as they get in or out.
    Seats(Vec<(u32, (bool, bool), Vec<Value>)>),
    /// A condition on each of several objects (an object filter's): per
    /// object, whether it holds and the values to pass.
    Each(Vec<(u32, bool, Vec<Value>)>),
    /// A minigame's answers, each a status and maybe a value; it stays
    /// (made with `Create` or not) until `done`.
    Minigame(crate::hijack::MinigameCheck),
    /// Whether each of several objects is inside a region, with the values
    /// to pass: fired as each goes in or out, the values then followed by
    /// `"enter"` or `"exit"`.
    Crossings(Vec<(u32, bool, Vec<Value>)>),
    /// `ObjectWinched`: the cargo, who holds it now (`None`: nobody),
    /// the carrier filter (0: any) and the mode bits (1 attach, 2 detach).
    Winch(u32, Option<u32>, u32, u8),
    /// A kind no check knows: it never fires.
    Unhandled,
}

/// The objects parameter 1 names by a label, alone or an object filter's
/// (as they come and go), not one by one.
fn by_label(t: &Table, world: &dyn World) -> Vec<u32> {
    match t.raw_get::<Value>(1) {
        Ok(Value::Table(f)) => match f.raw_get::<Option<String>>("label") {
            Ok(Some(label)) => world.labelled(&label),
            _ => Vec::new(),
        },
        Ok(Value::String(s)) => {
            let s = s.to_string_lossy();
            if s.eq_ignore_ascii_case("hero") {
                Vec::new()
            } else {
                world.labelled(&s)
            }
        }
        _ => Vec::new(),
    }
}

thread_local! {
    /// While the pending events are checked ([`LabelMemo`]): the label
    /// index, once a check has looked a label up.
    static LABELLED: RefCell<Option<Option<LabelIndex>>> = const { RefCell::new(None) };
}

/// Every object's labels, read once, and what each label looked up named:
/// a lookup went over every object and hashed each of its labels again
/// for each label term, and with a contract's layers in (76,000 objects,
/// 30,000 of them labelled, and a few hundred pending events) those
/// lookups took over half of play's time and stalled frames for seconds
/// (perf round 1).
#[derive(Debug, Default)]
struct LabelIndex {
    /// The labelled objects, each with where its labels are in `hashes`.
    objects: Vec<(u32, std::ops::Range<usize>)>,
    /// The labelled objects' labels as name hashes.
    hashes: Vec<u32>,
    /// The objects without labels.
    unlabelled: Vec<u32>,
    /// Where in `objects` the objects with each label are.
    by_label: HashMap<u32, Vec<usize>>,
    /// What each label expression named.
    found: HashMap<String, Vec<u32>>,
}

impl LabelIndex {
    /// The index of what `world` lists ([`World::visit_labels`]).
    fn of(world: &dyn World) -> Option<Self> {
        let mut index = Self::default();
        world
            .visit_labels(&mut |g, labels| index.add(g, labels))
            .then_some(index)
    }

    fn add(&mut self, guid: u32, labels: &mut dyn Iterator<Item = u32>) {
        let start = self.hashes.len();
        self.hashes.extend(labels);
        if self.hashes.len() == start {
            self.unlabelled.push(guid);
            return;
        }
        let i = self.objects.len();
        for &h in &self.hashes[start..] {
            let at = self.by_label.entry(h).or_default();
            if at.last() != Some(&i) {
                at.push(i);
            }
        }
        self.objects.push((guid, start..self.hashes.len()));
    }

    /// The objects `expr` names, as [`World::labelled`]: those matching
    /// any of its alternatives ([`class_alternatives`]), by guid.
    fn labelled(&mut self, expr: &str) -> Vec<u32> {
        if let Some(found) = self.found.get(expr) {
            return found.clone();
        }
        let mut out = Vec::new();
        for terms in class_alternatives(expr) {
            let matches = |i: &usize| {
                let labels = &self.hashes[self.objects[*i].1.clone()];
                terms.iter().all(|&(h, not)| labels.contains(&h) != not)
            };
            match terms.iter().find(|t| !t.1) {
                // Only those with its first label can match ...
                Some((h, _)) => out.extend(
                    self.by_label
                        .get(h)
                        .into_iter()
                        .flatten()
                        .filter(|i| matches(i))
                        .map(|&i| self.objects[i].0),
                ),
                // ... and with none (all negated), any can, those without
                // labels too.
                None => {
                    out.extend(
                        (0..self.objects.len())
                            .filter(matches)
                            .map(|i| self.objects[i].0),
                    );
                    out.extend_from_slice(&self.unlabelled);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        self.found.insert(expr.to_owned(), out.clone());
        out
    }
}

/// Keeps a [`LabelIndex`] while it lives. Only for while the world is
/// borrowed and cannot change.
struct LabelMemo;

impl LabelMemo {
    fn start() -> Self {
        LABELLED.set(Some(None));
        Self
    }
}

impl Drop for LabelMemo {
    fn drop(&mut self) {
        LABELLED.set(None);
    }
}

/// The objects with `label` ([`World::labelled`]), from the index while
/// a [`LabelMemo`] lives and the world lists its labels.
fn labelled(world: &dyn World, label: &str) -> Vec<u32> {
    LABELLED.with_borrow_mut(|memo| match memo {
        Some(index) => {
            if index.is_none() {
                *index = LabelIndex::of(world);
            }
            match index {
                Some(index) => index.labelled(label),
                None => world.labelled(label),
            }
        }
        None => world.labelled(label),
    })
}

/// The objects a parameter names: a guid, `"hero"` (the player), or a
/// label. Anything else (an object filter) names none yet.
fn objects(t: &Table, i: i64, world: &dyn World) -> Vec<u32> {
    match t.raw_get::<Value>(i) {
        Ok(Value::String(s)) => {
            let s = s.to_string_lossy();
            if s.eq_ignore_ascii_case("hero") {
                world.player().into_iter().collect()
            } else {
                let labelled = labelled(world, &s);
                if labelled.is_empty() {
                    world.named(&s).into_iter().collect()
                } else {
                    labelled
                }
            }
        }
        // An object filter (see `api`): its objects, the player when it
        // uses players, and objects with its label, less those excluded.
        // A plain list of objects (`Player.GetAllCharacters()`, which
        // `MrxTaskObjectiveEnterVehicle` waits on: PmcCon003's "Board the
        // Lucky Lady") is its objects.
        Ok(Value::Table(f)) => {
            let ids = |t: Table| -> Vec<u32> {
                t.sequence_values::<Value>()
                    .filter_map(|v| as_id(&v.ok()?))
                    .collect()
            };
            let mut out: Vec<u32> = match f.raw_get::<Option<Table>>("objs").ok().flatten() {
                Some(t) => ids(t),
                None => ids(f.clone()),
            };
            if f.raw_get::<bool>("players").unwrap_or(false) {
                out.extend(world.player());
            }
            if let Ok(Some(label)) = f.raw_get::<Option<String>>("label") {
                out.extend(labelled(world, &label));
            }
            if let Ok(Some(ex)) = f.raw_get::<Option<Table>>("ex") {
                let excluded: Vec<u32> = ex
                    .pairs::<Value, Value>()
                    .filter_map(|p| as_id(&p.ok()?.0))
                    .collect();
                out.retain(|g| !excluded.contains(g));
            }
            out.sort_unstable();
            out.dedup();
            out
        }
        Ok(v) => as_id(&v).into_iter().collect(),
        Err(_) => Vec::new(),
    }
}

/// A `Boundary` event's region (its second parameter): a guid or the
/// region's name.
fn boundary_of(t: &Table, world: &dyn World) -> Option<u32> {
    guid(t, 2).or_else(|| match t.raw_get::<Value>(2) {
        Ok(Value::String(s)) => world.named(&s.to_string_lossy()),
        _ => None,
    })
}

fn text(t: &Table, i: i64) -> Option<String> {
    match t.raw_get::<Value>(i).ok()? {
        Value::String(s) => Some(s.to_string_lossy().to_ascii_lowercase()),
        _ => None,
    }
}

/// Checks `p` against the world. Readings of each kind's parameters come
/// from the game's call sites.
/// The (filter function's address, post's index) pairs a `ScriptEvent`'s
/// filter said yes to.
type Accepted = HashSet<(usize, usize)>;

/// `ObjectHealth`'s comparison text as (equal, less, more), as the 360 reads
/// it (0x8234c970): equal when either of the first two characters is `=`,
/// then less or more by the first. Anything else compares as nothing
/// (none here).
fn health_comparison(s: &str) -> Option<(bool, bool, bool)> {
    let b = s.as_bytes();
    let equal = b.first() == Some(&b'=') || b.get(1) == Some(&b'=');
    let less = b.first() == Some(&b'<');
    let more = b.first() == Some(&b'>');
    (equal || less || more).then_some((equal, less, more))
}

fn check(
    p: &Pending,
    h: &Live,
    now: f64,
    posted: &[(String, Value)],
    accepted: &Accepted,
    world: &dyn World,
) -> Check {
    let no = Check::Holds(false, Vec::new());
    let id = guid_value;
    match p.kind.as_str() {
        // { seconds }: that long after it was made (persistent: every
        // that long).
        "TimerRelative" => match number(&h.params, 1) {
            Some(delay) if now - p.last >= delay => Check::Fire(Vec::new()),
            _ => no,
        },
        // { object }: when it dies or is removed; passes the object. A
        // filter or label names several, each firing as it dies (a
        // Destroy objective counts its targets this way).
        // Passes the object, then (when the game knows who killed it) a
        // nil and the killer: `MrxTaskObjectiveDestroy._TargetDestroyed(
        // self, target, _, killer)` checks its fourth parameter against
        // the players' characters for a hero-only bounty (from the
        // scripts; what the second value is was not found).
        "ObjectDeath" => Check::Each(
            objects(&h.params, 1, world)
                .into_iter()
                .map(|g| {
                    let mut pass = vec![id(g)];
                    if let Some(k) = world.killer(g) {
                        pass.extend([Value::Nil, id(k)]);
                    }
                    (g, world.died(g), pass)
                })
                .collect(),
        ),
        // { object }: once it is gone from play (removed, or faded out):
        // the support helicopter's pilot is free again once his
        // helicopter is (`MakeRecruitAvailable("Copter")`). Passes the
        // object.
        // Passes nothing (from the 360 code: its arguments slot is the
        // empty 0x8225e998; the support helicopter's callback takes only
        // its data).
        "ObjectDelete" => match guid(&h.params, 1) {
            Some(g) => Check::Holds(world.position(g).is_none(), Vec::new()),
            None => no,
        },
        // { object, health }: once the object's health is below the figure
        // (4 scripts: ChiCon001's VIP and the like wait for half of
        // `Object.GetHealth`). From the 360 code: parse 0x82350e28 takes a
        // guid and a number, test 0x82350ee8 compares the object's health
        // with it as its health changes (a health it can't read counts as
        // below), and nothing is passed (0x8225e998). Here it is checked
        // every update; an object this world doesn't hold is left waiting
        // (inferred: the engine only tests on a health change).
        "ObjectHealthLessThan" => {
            let (Some(g), Some(figure)) = (guid(&h.params, 1), number(&h.params, 2)) else {
                return no;
            };
            match world.health(g) {
                Some(h) => Check::Holds(f64::from(h) < figure, Vec::new()),
                None => no,
            }
        }
        // { a, b, op, metres, stopped, flat }: when some a is within
        // ("<", "<=") or beyond (">", ">=") the distance of some b,
        // checked whenever it is pending (scripts wanting repeats make a
        // "<" and a ">" in turn); flat measures across the ground only
        // (`stopped` isn't followed yet). a and b are guids, "hero",
        // labels or filters. Passes the a's that hold: one guid as a
        // guid, a label's or filter's as a list (as WifTutorialTankHijack
        // and the gate scripts read it), and "hero"'s as a list too
        // (PmcCon004's `ProximityCallback(self, tGuid)` takes `tGuid[1]`
        // as the hero near Solano's helicopter; the only other "hero"
        // proximity, `MrxTaskObjectiveFlee.HeroProximity`, doesn't read
        // it). From the scripts.
        "ObjectProximity" => {
            let r = number(&h.params, 4).unwrap_or(0.0);
            let beyond = text(&h.params, 3).is_some_and(|op| op.starts_with('>'));
            let flat = matches!(h.params.raw_get::<Value>(6), Ok(Value::Boolean(true)));
            let (aa, bb) = (objects(&h.params, 1, world), objects(&h.params, 2, world));
            let holds = |a: u32| {
                let Some(pa) = world.position(a) else {
                    return false;
                };
                bb.iter()
                    .filter(|&&b| !world.unheard_at_gate(a, b))
                    .filter_map(|&b| world.position(b))
                    .any(|pb| {
                        let d2: f32 = [0, 1, 2]
                            .into_iter()
                            .filter(|&i| !(flat && i == 1))
                            .map(|i| (pa[i] - pb[i]).powi(2))
                            .sum();
                        (f64::from(d2) <= r * r) != beyond
                    })
            };
            let near: Vec<u32> = aa.into_iter().filter(|&a| holds(a)).collect();
            if near.is_empty() {
                return no;
            }
            let one = !matches!(
                h.params.raw_get::<Value>(1),
                Ok(Value::String(_) | Value::Table(_))
            );
            if one {
                return Check::Holds(true, vec![id(near[0])]);
            }
            let list = p
                .lua
                .0
                .try_upgrade()
                .and_then(|lua| lua.create_sequence_from(near.iter().map(|&g| id(g))).ok());
            match list {
                Some(t) => Check::Holds(true, vec![Value::Table(t)]),
                None => no,
            }
        }
        // { rider, vehicle, seat, mode }: rider is a guid, "hero" or a
        // label; vehicle a guid (0: any), a vehicle's name or a class
        // expression of labels ("Boat", "Tank && !APC"); seat a seat code
        // ("D" the driver's; not told apart yet); mode letters, each a
        // way of getting in or out: e climbing in, i put in at once, x
        // climbing out, o put out ("ei" and "xo" either; "a" any). A
        // vehicle script waits for "E" and "I" alike and leaves the other
        // pending, so the two never fire for the same move. Passes the
        // rider and vehicle.
        "ObjectInSeat" => {
            let wanted = |v: u32| match h.params.raw_get::<Value>(2) {
                Ok(Value::String(s)) => {
                    let named = objects(&h.params, 2, world);
                    if named.is_empty() {
                        class_matches(&s.to_string_lossy(), &world.labels(v))
                    } else {
                        named.contains(&v)
                    }
                }
                Ok(other) => as_id(&other).is_none_or(|want| want == 0 || want == v),
                Err(_) => true,
            };
            Check::Seats(
                objects(&h.params, 1, world)
                    .into_iter()
                    .map(|r| {
                        let seated = world.vehicle_of(r).filter(|&v| wanted(v));
                        (
                            r,
                            (seated.is_some(), world.seated_instantly(r)),
                            vec![id(r), id(seated.unwrap_or(0))],
                        )
                    })
                    .collect(),
            )
        }
        // { player, "Human" or "Vehicle", "Enter" or "Exit" }: as the
        // player takes control of her character on foot (or of a vehicle),
        // or gives it up. The one call site (`MrxUtil._TeleportHeroes`)
        // takes her out of her vehicle and waits for "Human", "Enter"
        // before it moves her. Passes nothing of its own.
        "Player" => {
            let Some(hero) = world.player() else {
                return no;
            };
            let riding = world.vehicle_of(hero).is_some();
            let on_foot = match text(&h.params, 2).as_deref() {
                Some("human") => true,
                Some("vehicle") => false,
                _ => return no,
            };
            let exit = text(&h.params, 3).as_deref() == Some("exit");
            Check::Holds((riding != on_foot) != exit, Vec::new())
        }
        // { object, want = true } (0x82354CB0; test 0x82354AA0): when the
        // object's being on the ground is `want`; passes both.
        "ObjectIsGrounded" => {
            let want = !matches!(h.params.raw_get::<Value>(2), Ok(Value::Boolean(false)));
            for o in objects(&h.params, 1, world) {
                if world.grounded(o) == want {
                    return Check::Holds(true, vec![id(o), Value::Boolean(want)]);
                }
            }
            no
        }
        // { state, "enter" or "exit" }: as the game enters or leaves a
        // state. Passes nothing (from the 360 code: parse 0x823555a8,
        // test 0x82355628, and the empty arguments slot 0x8225e998; the
        // scripts' callbacks take only their data).
        "GameStateChange" => {
            let (Some(state), Some(current)) = (text(&h.params, 1), world.game_state()) else {
                return no;
            };
            let exit = text(&h.params, 2).as_deref() == Some("exit");
            Check::Holds(current.eq_ignore_ascii_case(&state) != exit, Vec::new())
        }
        // { object, boundary, "enter" or "exit" }.
        "Boundary" => {
            let boundary = boundary_of(&h.params, world);
            let mode = text(&h.params, 3);
            // `"any"`: both ways, the callback told which (the faction
            // zones' trespass watch).
            if let (Some(b), Some("any")) = (boundary, mode.as_deref()) {
                return Check::Crossings(
                    objects(&h.params, 1, world)
                        .into_iter()
                        .filter_map(|o| Some((o, world.inside(o, b)?, vec![id(o), id(b)])))
                        .collect(),
                );
            }
            let (Some(b), exit) = (boundary, mode.as_deref() == Some("exit")) else {
                return no;
            };
            // The callback is also told the direction it waited on, after
            // the boundary, as the "any" path above does. From samples: the
            // shipped scripts read the direction as the callback's argument
            // after the boundary (`WifVzAtmosphere.CrossedBoundary`
            // concatenates it). The 360's argument order is not yet traced.
            let direction = h.params.raw_get::<Value>(3).unwrap_or(Value::Nil);
            for o in objects(&h.params, 1, world) {
                if let Some(inside) = world.inside(o, b)
                    && inside != exit
                {
                    return Check::Holds(true, vec![id(o), id(b), direction]);
                }
            }
            no
        }
        // { object, name }: as the engine reports that physics event of the
        // object (MecCon001 and the mechanic's garage wait for theirs:
        // `{ garage, "gateFullyClosed" }` after `Object.CloseGate`, and
        // `"gateStuck"`); passes the object and the name (inferred: no
        // callback reads either).
        "ObjectPhysicsEvent" => {
            let (Some(object), Some(name)) = (guid(&h.params, 1), text(&h.params, 2)) else {
                return no;
            };
            match world.physics_event(object, &name) {
                Some(holds) => Check::Holds(
                    holds,
                    vec![
                        id(object),
                        h.params.raw_get::<Value>(2).unwrap_or(Value::Nil),
                    ],
                ),
                None => Check::Unhandled,
            }
        }
        // { cargo, carrier, mode }: as the cargo is hooked onto a winch or
        // let go from one. From the 360 code (parse 0x823520b8, test
        // 0x82352190, arguments 0x82352218):
        // - The mode is read by its first letters: not "a" is detach (2),
        //   "an..." is any (3), any other "a..." is attach (1).
        // - Each occurrence is { carrier, cargo, 1 attach or 2 detach }; it
        //   holds when the cargo filter takes the cargo, the carrier filter
        //   (0: any) takes the carrier, and the mode has its bit.
        // - It passes the cargo, the carrier, and "attach" or "detach".
        // Here the occurrences are the changes in the world's winch state
        // ([`World::winch`]) between checks. The first look only records it.
        "ObjectWinched" => {
            let Some(cargo) = guid(&h.params, 1) else {
                return no;
            };
            let carrier = guid(&h.params, 2).unwrap_or(0);
            let mode = match text(&h.params, 3) {
                Some(m) => {
                    let mut c = m.chars().map(|c| c.to_ascii_lowercase());
                    match (c.next(), c.next()) {
                        (Some('a'), Some('n')) => 3,
                        (Some('a'), _) => 1,
                        _ => 2,
                    }
                }
                None => return no,
            };
            // Cargo the world has no winch record for hangs from nothing.
            Check::Winch(cargo, world.winch(cargo).flatten(), carrier, mode)
        }
        // { object, "<" or ">", health }: while its health is below (or
        // above) the figure; passes the object. The hero's survival
        // script watches for drops below her current health this way.
        // From the 360 code (the kind's parse 0x82351050, test 0x823512a8,
        // arguments 0x82351420): `{ object, comparison, value }`, or with
        // four `{ object, part, comparison, value }` (a part's name hash,
        // `"*"` for any of its parts). The comparison is read as `=`
        // (either character), then `<` or `>` (0x8234c970): `<`, `<=`,
        // `>`, `>=` or `=`. The callback gets the health that made it hold
        // and the comparison as text, after its data; no guid.
        //
        // Here objects have no parts: `"*"` takes the object's health
        // (inferred), a named part never holds (the 360 answers false for
        // a part the object lacks).
        "ObjectHealth" => {
            let four = h.params.raw_len() >= 4;
            let (at, part) = if four {
                (3, text(&h.params, 2))
            } else {
                (2, None)
            };
            let (Some(comparison), Some(figure)) = (text(&h.params, at), number(&h.params, at + 1))
            else {
                return no;
            };
            if part.is_some_and(|s| s != "*") {
                return Check::Health(false, 0.0, "=");
            }
            let Some((equal, less, more)) = health_comparison(&comparison) else {
                return no;
            };
            let shown = match (less, more, equal) {
                (true, _, true) => "<=",
                (true, _, false) => "<",
                (_, true, true) => ">=",
                (_, true, false) => ">",
                _ => "=",
            };
            for o in objects(&h.params, 1, world) {
                if let Some(h) = world.health(o).map(f64::from) {
                    let holds =
                        (equal && h == figure) || (less && h < figure) || (more && h > figure);
                    if holds {
                        return Check::Health(true, h, shown);
                    }
                }
            }
            Check::Health(false, 0.0, shown)
        }
        // { character (or filter), from, to }: as one of them goes from a
        // state matching `from` to one matching `to` ([`state_matches`]:
        // the verify objective's `{ target, "*", "KnockedDown.Idle" }`),
        // after the event was made. Passes the character.
        "HumanStateTransition" => {
            let (Some(from), Some(to)) = (text(&h.params, 2), text(&h.params, 3)) else {
                return no;
            };
            let who = objects(&h.params, 1, world);
            let hit = world.state_changes().iter().find(|(t, g, a, b)| {
                *t >= p.last
                    && (*t > p.last || !p.persistent)
                    && who.contains(g)
                    && state_matches(&from, a)
                    && state_matches(&to, b)
            });
            match hit {
                Some((_, g, _, _)) => Check::Fire(vec![id(*g)]),
                None => no,
            }
        }
        // { holder (or filter), what, weapon (or filter) }: as one of the
        // holders does `what` with a weapon, after the event was made
        // (`MrxStatsManager` times her favourite weapon by her `"Equip"`,
        // `"Stow"` and `"Drop"`). From the 360 code (the event's reader
        // 82358000, its test 823580F8 and what it passes 82358190): the
        // holder and what are needed; `"Any"` is anything done; the
        // weapon may be left out, for any weapon; it passes the holder
        // and the weapon.
        "WeaponEvent" => {
            let Some(what) = text(&h.params, 2) else {
                return no;
            };
            let holders = objects(&h.params, 1, world);
            if holders.is_empty() {
                return no;
            }
            let weapons = objects(&h.params, 3, world);
            let hit = world.weapon_events().iter().find(|(t, who, did, weapon)| {
                *t >= p.last
                    && (*t > p.last || !p.persistent)
                    && holders.contains(who)
                    && (what.eq_ignore_ascii_case("Any") || what.eq_ignore_ascii_case(did))
                    && (weapons.is_empty() || weapons.contains(weapon))
            });
            match hit {
                Some((_, who, _, weapon)) => Check::Fire(vec![id(*who), id(*weapon)]),
                None => no,
            }
        }
        // { character }: when the action `Human.SetState` gave it has
        // played to its end.
        "HumanActionComplete" => match guid(&h.params, 1) {
            Some(g) => Check::Holds(world.action_done(g), Vec::new()),
            None => no,
        },
        // { player, seconds, mode, button, ... }: see `crate::hijack`.
        // { player, button, "press", ... }: when the player presses the
        // named button (`"selection"`: the fanfare's skip), after the
        // event was made (persistent: after it last fired).
        // { 0, object }: as the player takes the context action offered
        // beside the object (`Pg.AddContextAction`: the HQ's exits, a
        // starter's talk). Passes who took it and the object: the wardrobe's
        // callback (`WifPmcInterior._SelectOutfit(who)`, made with no data)
        // opens only for the primary character, and the verify objective's
        // `_TargetActioned(self, who, target)` turns her to the target.
        // { _, object or filter }: the player took its context action (an
        // object filter: on any of its objects, as the action objectives'
        // `{ 0, _uTgtObjFilter }`, a prisoner to release).
        "ContextAction" => {
            let targets = objects(&h.params, 2, world);
            let taken = world.context_presses().iter().find_map(|(t, o)| {
                (targets.contains(o) && *t >= p.last && (*t > p.last || !p.persistent))
                    .then_some(*o)
            });
            match taken {
                Some(g) => Check::Fire(world.player().map(id).into_iter().chain([id(g)]).collect()),
                None => no,
            }
        }
        // { who, event }: as a clip the scripts played on `who` by name
        // (`Human.PlayRawAnimation`) meets the named event; who 0 is
        // anyone. The HQ briefings' frames wait on `{ 0, "CinematicStart"
        // }` to fade in and `{ actor, "CinematicEnd" }` to go on, events
        // 252 of their 258 clips carry (from samples).
        "AnimationEvent" => match text(&h.params, 2) {
            Some(event) => {
                let who = guid(&h.params, 1).filter(|&g| g != 0);
                let met = world.animation_events().iter().any(|(t, g, e)| {
                    who.is_none_or(|w| w == *g)
                        && e.eq_ignore_ascii_case(&event)
                        && *t >= p.last
                        && (*t > p.last || !p.persistent)
                });
                if met { Check::Fire(Vec::new()) } else { no }
            }
            None => no,
        },
        "Button" => match text(&h.params, 2) {
            Some(button) => {
                let pressed = world
                    .button_presses()
                    .iter()
                    .any(|(t, b)| *b == button && *t >= p.last && (*t > p.last || !p.persistent));
                if pressed {
                    let (pad, words) = button_args(&button);
                    Check::FireWords(pad, words)
                } else {
                    no
                }
            }
            None => no,
        },
        "Minigame" => Check::Minigame(crate::hijack::check_minigame(
            &h.params, p.made, now, p.read, p.memo, world,
        )),
        // { object, "awake" or "hibernated" }: while it is simulated near
        // the player (or hibernates away from her); passes the object.
        // The scripts write "hibernated" (25 calls in vz.wad: PmcCon001's
        // gate opener waits for its front door to sleep, then to wake
        // again); "hibernate" reads the same. Anything else is awake.
        // `ObjectIsReady` (10 scripts: the wardrobe's `{ who, "awake", 0 }`
        // after dressing her, the HQ waiting for a player, props'
        // destroyed states) takes the same parameters and is read the
        // same: ready as awake is inferred, the engine's own test is not
        // seen.
        // { character, seconds }: once the raw clip she was last given has
        // that little left (the briefings' named spiel clips: `WifHq`'s
        // cheap cinematic waits on it with 0.1 before the next line; from
        // the scripts). Passes the character.
        "HumanAnimationNearlyCompleted" => match guid(&h.params, 1) {
            Some(g) => {
                let margin = h.params.raw_get::<f64>(2).unwrap_or(0.1) as f32;
                // Passes nothing (from the 360 code: parse 0x82356228, test
                // 0x823562a0, and the empty arguments slot 0x8225e998).
                match world.raw_clip_left(g) {
                    Some(left) if left <= margin => Check::Holds(true, Vec::new()),
                    _ => no,
                }
            }
            None => no,
        },
        // { object, mode, [level] }: while the object is awake (a mode
        // starting with "a") or hibernating (any other mode, or none).
        // From the 360 code: parse 0x82352728 reads only the first letter,
        // so the HQ manager's `{ hq, "s" }` (wait for a destroyed HQ to
        // hibernate before reviving it) waits for hibernation, as do
        // "hibernated" and a missing mode. Its arguments slot (0x8225e998)
        // passes nothing. The optional level (a streaming detail the
        // object must have reached) isn't modelled.
        "ObjectHibernation" => match guid(&h.params, 1) {
            Some(g) => {
                let want_awake = text(&h.params, 2).is_some_and(|t| t.starts_with('a'));
                Check::Holds(world.is_awake(g) == want_awake, Vec::new())
            }
            None => no,
        },
        // { object, [level] }: once the object is in the world. From the
        // 360 code (0x82352c28, 0x82352ab0) it checks the object is loaded
        // rather than awake; here awake stands in for that (inferred).
        // Passes nothing (0x8225e998).
        "ObjectIsReady" => match guid(&h.params, 1) {
            Some(g) => Check::Holds(world.is_awake(g), Vec::new()),
            None => no,
        },
        // { object }: while it is on the player's screen
        // ([`World::is_visible`]): PmcCon001 waits, once she is inside the
        // villa, for the outpost's tank column to come into view to load
        // the tank (`LoadTank`). A label or filter names several: the
        // first seen. Passes nothing (from the 360 code: parse 0x823551e8,
        // test 0x82355230, and the empty arguments slot 0x8225e998).
        "ObjectIsVisible" => {
            let seen = objects(&h.params, 1, world)
                .into_iter()
                .any(|g| world.is_visible(g) == Some(true));
            if seen {
                Check::Holds(true, Vec::new())
            } else {
                no
            }
        }
        // { name, filter }: when something posts that name, and the
        // filter, if any, says yes to the posted data (the faction
        // manager's attitude watches compare it with what they want);
        // passes the posted data.
        "ScriptEvent" => {
            let Ok(name) = h.params.raw_get::<String>(1) else {
                return no;
            };
            // Filters ran before the checks ([`Events::filter_posts`]).
            let filter = h.params.raw_get::<Option<Function>>(2).ok().flatten();
            let wanted = |i: usize| {
                filter
                    .as_ref()
                    .is_none_or(|f| accepted.contains(&(f.to_pointer() as usize, i)))
            };
            match posted
                .iter()
                .enumerate()
                .find(|(i, (n, _))| *n == name && wanted(*i))
            {
                Some((_, (_, data))) => Check::Fire(vec![data.clone()]),
                None => no,
            }
        }
        // { }: every update, passing the seconds since it last fired
        // (`SupportTimer._EventCallback(self, dt)` adds them up).
        "GuiUpdate" | "GuiGameTimer" => Check::Fire(vec![Value::Number(now - p.last)]),
        // The game's own (the GUI's): fired by `Event.__m2_fire`.
        kind if kind.starts_with("Gui") || ENGINE_FIRED.contains(&kind) => no,
        // Not simulated yet: never fires, and is reported.
        _ => Check::Unhandled,
    }
}

/// Calls `f` with the event's data unpacked, then `extra`.
fn call(f: &Function, data: Option<&Table>, extra: Vec<Value>) -> mlua::Result<()> {
    let mut args: Vec<Value> = match data {
        Some(t) => t.sequence_values::<Value>().collect::<mlua::Result<_>>()?,
        None => Vec::new(),
    };
    args.extend(extra);
    f.call::<()>(MultiValue::from_vec(args))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::ScriptHost;
    use std::collections::HashMap;

    #[derive(Default)]
    struct TestWorld {
        positions: HashMap<u32, [f32; 3]>,
        dead: Vec<u32>,
        seats: HashMap<u32, u32>,
        /// Whether the camera is known, and what it sees.
        camera: bool,
        visible: Vec<u32>,
        /// Context actions taken: (game time, object).
        context: Vec<(f64, u32)>,
        /// Clip events met: (game time, who, event).
        anims: Vec<(f64, u32, String)>,
        /// Who killed whom.
        killers: HashMap<u32, u32>,
        /// Vehicles off the ground.
        airborne: Vec<u32>,
    }

    impl World for TestWorld {
        fn is_visible(&self, guid: u32) -> Option<bool> {
            self.camera.then(|| self.visible.contains(&guid))
        }
        fn position(&self, guid: u32) -> Option<[f32; 3]> {
            self.positions.get(&guid).copied()
        }
        fn is_alive(&self, guid: u32) -> bool {
            !self.dead.contains(&guid)
        }
        fn killer(&self, guid: u32) -> Option<u32> {
            self.killers.get(&guid).copied()
        }
        fn grounded(&self, guid: u32) -> bool {
            !self.airborne.contains(&guid)
        }
        fn vehicle_of(&self, guid: u32) -> Option<u32> {
            self.seats.get(&guid).copied()
        }
        fn player(&self) -> Option<u32> {
            Some(1)
        }
        fn is_awake(&self, guid: u32) -> bool {
            self.positions.contains_key(&guid)
        }
        fn context_presses(&self) -> &[(f64, u32)] {
            &self.context
        }
        fn animation_events(&self) -> &[(f64, u32, String)] {
            &self.anims
        }
        fn inside(&self, _: u32, _: u32) -> Option<bool> {
            None
        }
        fn labelled(&self, label: &str) -> Vec<u32> {
            if label == "Prisoner" {
                vec![2]
            } else if label == "tank" {
                vec![4, 5]
            } else {
                Vec::new()
            }
        }
    }

    fn setup() -> (ScriptHost, Events) {
        let host = ScriptHost::new().unwrap();
        let events = Events::default();
        events.install(host.lua()).unwrap();
        host.lua().load("log = {}").exec().unwrap();
        // Guids reach callbacks as userdata; tests note them as numbers.
        let gid = safe_function(host.lua(), |_, args| {
            let v = args.front().cloned().unwrap_or(Value::Nil);
            Ok(MultiValue::from_vec(vec![match as_guid(&v) {
                Some(g) if matches!(v, Value::LightUserData(_)) => Value::Number(f64::from(g)),
                _ => v,
            }]))
        })
        .unwrap();
        host.lua().globals().set("gid", gid).unwrap();
        (host, events)
    }

    fn log(host: &ScriptHost) -> Vec<String> {
        host.lua()
            .globals()
            .get::<Table>("log")
            .unwrap()
            .sequence_values::<String>()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn grounded_events_fire_as_a_vehicle_lands_or_leaves_the_ground() {
        // A jump's ResetJump waits for its car to land:
        // `Event.Create(Event.ObjectIsGrounded, { car }, ..)`.
        let (host, events) = setup();
        host.lua()
            .load(
                "Event.Create(Event.ObjectIsGrounded, { 7 }, function(o, want)                  table.insert(log, 'landed ' .. gid(o) .. ' ' .. tostring(want)) end)                  Event.Create(Event.ObjectIsGrounded, { 7, false }, function(o, want)                  table.insert(log, 'off ' .. gid(o) .. ' ' .. tostring(want)) end)",
            )
            .exec()
            .unwrap();
        let mut world = TestWorld {
            airborne: vec![7],
            ..TestWorld::default()
        };
        world.positions.insert(7, [0.0; 3]);
        events.update(0.1, &world);
        assert_eq!(log(&host), ["off 7 false"]);
        world.airborne.clear();
        events.update(0.1, &world);
        assert_eq!(log(&host), ["off 7 false", "landed 7 true"]);
    }

    #[test]
    fn timers_fire_once_after_their_delay() {
        let (host, events) = setup();
        host.lua()
            .load(
                "Event.Create(Event.TimerRelative, { 2 }, function(who) \
                 table.insert(log, 'timer ' .. who) end, { 'a' })",
            )
            .exec()
            .unwrap();
        let world = TestWorld::default();
        assert!(events.update(1.0, &world).is_empty());
        assert!(log(&host).is_empty());
        events.update(1.0, &world);
        assert_eq!(log(&host), ["timer a"]);
        events.update(5.0, &world);
        assert_eq!(log(&host), ["timer a"]);
        assert!(events.is_empty());
    }

    #[test]
    fn gui_updates_pass_the_time_since_the_last() {
        // The support manager's `SupportTimer` adds them up for a
        // recruit's cooldown.
        let (host, events) = setup();
        host.lua()
            .load(
                "total = 0 Event.CreatePersistent(Event.GuiGameTimer, {}, \
                 function(self, dt) total = total + dt end, { 'timer' })",
            )
            .exec()
            .unwrap();
        let world = TestWorld::default();
        for _ in 0..4 {
            events.update(0.5, &world);
        }
        let total: f64 = host.lua().globals().get("total").unwrap();
        assert!((total - 2.0).abs() < 1e-9, "{total}");
    }

    #[test]
    fn persistent_events_repeat_until_deleted() {
        let (host, events) = setup();
        host.lua()
            .load(
                "id = Event.CreatePersistent(Event.TimerRelative, { 1 }, \
                 function() table.insert(log, 'tick') end)",
            )
            .exec()
            .unwrap();
        let world = TestWorld::default();
        for _ in 0..3 {
            events.update(1.0, &world);
        }
        assert_eq!(log(&host).len(), 3);
        // Event.Delete answers whether there was one (0x823583d8).
        let (first, again): (bool, bool) = host
            .lua()
            .load("return Event.Delete(id), Event.Delete(id)")
            .eval()
            .unwrap();
        assert!(first && !again);
        events.update(1.0, &world);
        assert_eq!(log(&host).len(), 3);
        // Event.Post answers whether it had a name and data (0x82358470).
        let (posted, bare): (bool, bool) = host
            .lua()
            .load("return Event.Post('Ping', {}), Event.Post('Ping')")
            .eval()
            .unwrap();
        assert!(posted && !bare);
    }

    /// A repeating timer keeps its rate when the frames don't divide its
    /// delay: 0.1 s at 30 frames a second is 30 a second, not one every
    /// fourth frame (22 or so).
    #[test]
    fn a_repeating_timer_carries_what_it_overran() {
        let (host, events) = setup();
        host.lua()
            .load(
                "Event.CreatePersistent(Event.TimerRelative, { 0.1 }, \
                 function() table.insert(log, 'tick') end)",
            )
            .exec()
            .unwrap();
        let world = TestWorld::default();
        for _ in 0..90 {
            events.update(1.0 / 30.0, &world);
        }
        let n = log(&host).len();
        assert!((29..=30).contains(&n), "{n} ticks in 3 s");
    }

    #[test]
    fn a_context_action_fires_when_taken_at_its_object() {
        let (host, events) = setup();
        // As the HQ's portals wait on their exits (`WifPmcInterior`).
        host.lua()
            .load(
                "Event.CreatePersistent(Event.ContextAction, { 0, 7 }, \
                 function(d) table.insert(log, 'exit ' .. d) end, { 'A' })",
            )
            .exec()
            .unwrap();
        let mut world = TestWorld {
            // Taken before it was waited for, and at another object.
            context: vec![(-1.0, 7), (0.25, 8)],
            ..TestWorld::default()
        };
        events.update(0.5, &world);
        assert!(log(&host).is_empty());
        world.context.push((0.75, 7));
        events.update(0.5, &world);
        events.update(0.5, &world);
        assert_eq!(log(&host), ["exit A"]);
        // Persistent: taken again, it fires again.
        world.context.push((1.25, 7));
        events.update(0.5, &world);
        assert_eq!(log(&host), ["exit A", "exit A"]);
        assert!(events.take_unhandled().is_empty());
        // Made with no data (the wardrobe's `_SelectOutfit(who)`): its
        // callback gets who took it.
        host.lua()
            .load(
                "Event.CreatePersistent(Event.ContextAction, { 0, 9 }, \
                 function(who) table.insert(log, 'outfit ' .. tostring(who ~= nil)) end)",
            )
            .exec()
            .unwrap();
        world.context.push((2.25, 9));
        events.update(0.5, &world);
        events.update(0.5, &world);
        assert_eq!(log(&host), ["exit A", "exit A", "outfit true"]);
        // On an object filter's object (a prisoner to release:
        // `MrxTaskObjectiveAction`'s `{ 0, _uTgtObjFilter }`).
        host.lua()
            .load(
                "Event.CreatePersistent(Event.ContextAction, { 0, { objs = { 11, 12 } } }, \
                 function(d) table.insert(log, 'release ' .. d) end, { 'B' })",
            )
            .exec()
            .unwrap();
        world.context.push((3.25, 12));
        events.update(0.5, &world);
        events.update(0.5, &world);
        assert_eq!(log(&host), ["exit A", "exit A", "outfit true", "release B"]);
    }

    #[test]
    fn object_is_ready_fires_once_it_is_awake() {
        let (host, events) = setup();
        // As the wardrobe waits after dressing her (_CompleteChangeOutfit).
        host.lua()
            .load(
                "Event.Create(Event.ObjectIsReady, { 3, 'awake', 0 }, \
                 function(d) table.insert(log, 'ready ' .. d) end, { 'preen' })",
            )
            .exec()
            .unwrap();
        let mut world = TestWorld::default();
        events.update(0.5, &world);
        assert!(log(&host).is_empty());
        world.positions.insert(3, [0.0; 3]);
        events.update(0.5, &world);
        assert_eq!(log(&host), ["ready preen"]);
        assert!(events.take_unhandled().is_empty());
    }

    #[test]
    fn a_clip_event_fires_for_its_actor_or_anyone() {
        let (host, events) = setup();
        // As WifHq._NextCinematicFrame waits: the first frame's fade on
        // anyone's start, the frame's end on its OnComplete actor's.
        host.lua()
            .load(
                "Event.Create(Event.AnimationEvent, { 0, 'CinematicStart' }, \
                 function(d) table.insert(log, 'start ' .. d) end, { 'fade' }) \
                 Event.Create(Event.AnimationEvent, { 1, 'CinematicEnd' }, \
                 function(d) table.insert(log, 'end ' .. d) end, { 1 })",
            )
            .exec()
            .unwrap();
        let mut world = TestWorld {
            // Another actor's end, and one met before it was waited for.
            anims: vec![
                (-1.0, 1, "CinematicEnd".into()),
                (0.25, 5, "CinematicEnd".into()),
            ],
            ..TestWorld::default()
        };
        events.update(0.5, &world);
        assert!(log(&host).is_empty());
        world.anims.push((0.75, 5, "CinematicStart".into()));
        world.anims.push((0.75, 1, "CinematicEnd".into()));
        events.update(0.5, &world);
        events.update(0.5, &world);
        assert_eq!(log(&host), ["start fade", "end 1"]);
        assert!(events.take_unhandled().is_empty());
    }

    #[test]
    fn a_seat_event_on_a_list_of_characters_fires_as_one_gets_in() {
        let (host, events) = setup();
        // As `MrxTaskObjectiveEnterVehicle` waits on
        // `Player.GetAllCharacters()` (PmcCon003's "Board the Lucky Lady").
        host.lua()
            .load(
                "Event.Create(Event.ObjectInSeat, { { 1 }, 99, 'a', 'ei' }, \
                 function(d) table.insert(log, 'in ' .. d) end, { 'lady' })",
            )
            .exec()
            .unwrap();
        let mut world = TestWorld::default();
        events.update(0.1, &world);
        assert!(log(&host).is_empty());
        world.seats.insert(1, 99);
        events.update(0.1, &world);
        assert_eq!(log(&host), ["in lady"]);
    }

    #[test]
    fn a_context_action_on_a_filter_passes_the_object_taken_at() {
        let (host, events) = setup();
        // As an action objective waits on its targets
        // (`MrxTaskObjectiveAction`: PmcCon002's "Enter office").
        host.lua()
            .load(
                "Event.CreatePersistent(Event.ContextAction, { 0, { objs = { 11, 12 } } }, \
                 function(d, who, target) table.insert(log, d .. ' ' .. tostring(who ~= nil) \
                 .. ' ' .. type(target) .. ' ' .. tostring(gid(target))) end, { 'office' })",
            )
            .exec()
            .unwrap();
        let mut world = TestWorld {
            context: vec![(0.25, 13)],
            ..TestWorld::default()
        };
        events.update(0.5, &world);
        assert!(log(&host).is_empty());
        world.context.push((0.75, 12));
        events.update(0.5, &world);
        events.update(0.5, &world);
        assert_eq!(log(&host), ["office true userdata 12"]);
    }

    #[test]
    fn the_player_event_fires_once_she_is_on_foot() {
        let (host, events) = setup();
        host.lua()
            .load(
                "Event.Create('Player', { 1, 'Human', 'Enter' }, function(a) \
                 table.insert(log, 'on foot ' .. a) end, { 'teleport' }) \
                 Event.Create('Player', { 1, 'Vehicle', 'Exit' }, function() \
                 table.insert(log, 'out') end)",
            )
            .exec()
            .unwrap();
        // In her boat: neither holds.
        let mut world = TestWorld::default();
        world.seats.insert(1, 9);
        events.update(0.5, &world);
        assert!(log(&host).is_empty());
        // Out of it: both fire, once.
        world.seats.clear();
        events.update(0.5, &world);
        events.update(0.5, &world);
        assert_eq!(log(&host), ["on foot teleport", "out"]);
        assert!(events.take_unhandled().is_empty());
    }

    #[test]
    fn deaths_proximity_seats_and_posts() {
        let (host, events) = setup();
        host.lua()
            .load(
                r#"
                local function note(s) return function(...)
                    local t = {}
                    for i, v in ipairs({...}) do
                        if type(v) == "table" then v = v[1] end
                        t[i] = tostring(gid(v))
                    end
                    table.insert(log, s .. table.concat(t, ","))
                end end
                Event.Create(Event.ObjectDeath, { 7 }, note("dead "), { 7 })
                Event.Create(Event.ObjectProximity, { "hero", 2, "<", 10, false, false }, note("near "), {})
                Event.Create(Event.ObjectInSeat, { "hero", nil, "Any", "enter" }, note("seat "), {})
                Event.Create(Event.ObjectHibernation, { 3, "awake" }, note("awake "), { 3 })
                Event.Create(Event.ScriptEvent, { "Go" }, note("posted "), { "x" })
                "#,
            )
            .exec()
            .unwrap();
        let mut world = TestWorld::default();
        world.positions.insert(1, [0.0, 0.0, 0.0]);
        world.positions.insert(2, [20.0, 0.0, 0.0]);
        events.update(0.1, &world);
        assert!(log(&host).is_empty());
        world.dead.push(7);
        world.positions.insert(2, [5.0, 0.0, 0.0]);
        world.positions.insert(3, [0.0, 0.0, 0.0]);
        world.seats.insert(1, 99);
        host.lua().load("Event.Post('Go', 'y')").exec().unwrap();
        events.update(0.1, &world);
        let mut got = log(&host);
        got.sort();
        assert_eq!(
            got,
            ["awake 3", "dead 7,7", "near 1", "posted x,y", "seat 1,99"]
        );
        assert!(events.is_empty());
    }

    #[test]
    fn a_death_passes_its_killer_when_known() {
        // As `MrxTaskObjectiveDestroy._TargetDestroyed(self, target, _,
        // killer)` reads it for a hero-only bounty.
        let (host, events) = setup();
        host.lua()
            .load(
                r##"
                local function note(s) return function(...)
                    local n = select("#", ...)
                    local a, b, c = ...
                    table.insert(log, s .. n .. " " .. tostring(gid(a)) .. " "
                        .. tostring(b) .. " " .. tostring(c and gid(c)))
                end end
                Event.Create(Event.ObjectDeath, { 7 }, note("dead "), {})
                Event.Create(Event.ObjectDeath, { 8 }, note("dead "), {})
                "##,
            )
            .exec()
            .unwrap();
        let mut world = TestWorld::default();
        world.dead.extend([7, 8]);
        world.killers.insert(7, 1);
        events.update(0.1, &world);
        let mut got = log(&host);
        got.sort();
        assert_eq!(got, ["dead 1 8 nil nil", "dead 3 7 nil 1"]);
    }

    #[test]
    fn deaths_of_a_label_count_from_when_the_event_was_made() {
        let (host, events) = setup();
        host.lua()
            .load(
                r#"
                Event.CreatePersistent(Event.ObjectDeath, { "tank" },
                    function(g) table.insert(log, "dead " .. tostring(gid(g))) end)
                -- An object filter as `api`'s makes it, with a label.
                local f = { objs = {}, ex = {}, players = false, label = "tank" }
                Event.CreatePersistent(Event.ObjectDeath, { f },
                    function(g) table.insert(log, "filter " .. tostring(gid(g))) end)
                Event.Create(Event.ObjectDeath, { 4 },
                    function(g) table.insert(log, "one " .. tostring(gid(g))) end)
                "#,
            )
            .exec()
            .unwrap();
        // 4 lay dead before: only the lone object's event fires for it.
        let mut world = TestWorld {
            dead: vec![4],
            ..TestWorld::default()
        };
        events.update(0.1, &world);
        assert_eq!(log(&host), ["one 4"]);
        world.dead.push(5);
        events.update(0.1, &world);
        let mut got = log(&host);
        got.sort();
        assert_eq!(got, ["dead 5", "filter 5", "one 4"]);
    }

    #[test]
    fn a_filter_picks_which_posts_fire() {
        let (host, events) = setup();
        host.lua()
            .load(
                r#"
                -- Filters may call the game (VzaCon001's print first).
                local function hostile(d)
                    Event.Post("Checked")
                    return d[2] == "Pmc" and d[4] == "Hostile"
                end
                Event.CreatePersistent(Event.ScriptEvent, { "Attitude", hostile },
                    function(d) table.insert(log, "angry " .. d[1]) end)
                Event.Post("Attitude", { "Gur", "Pmc", "Neutral", "Friendly" })
                "#,
            )
            .exec()
            .unwrap();
        let world = TestWorld::default();
        events.update(0.1, &world);
        assert!(log(&host).is_empty());
        host.lua()
            .load(r#"Event.Post("Attitude", { "Gur", "Pmc", "Neutral", "Hostile" })"#)
            .exec()
            .unwrap();
        events.update(0.1, &world);
        assert_eq!(log(&host), ["angry Gur"]);
    }

    #[test]
    fn proximity_to_a_label_passes_the_list_of_those_near() {
        let (host, events) = setup();
        host.lua()
            .load(
                r#"
                Event.Create(Event.ObjectProximity, { "tank", "hero", "<", 12, false, true },
                    function(tanks)
                        for _, t in ipairs(tanks) do table.insert(log, "tank " .. gid(t)) end
                    end)
                "#,
            )
            .exec()
            .unwrap();
        let mut world = TestWorld::default();
        world.positions.insert(1, [0.0, 0.0, 0.0]);
        // Above her: near across the ground, not in a straight line.
        world.positions.insert(4, [10.0, 15.0, 0.0]);
        world.positions.insert(5, [30.0, 0.0, 0.0]);
        events.update(0.1, &world);
        assert_eq!(log(&host), ["tank 4"]);
        assert!(events.is_empty());
    }

    #[test]
    fn the_newer_of_two_proximities_at_once_goes_first_and_can_delete_the_older() {
        let (host, events) = setup();
        host.lua()
            .load(
                r#"
                -- OilCon002's trip: cancelled 20 m from the helicopter (2),
                -- done at the landing zone (3), made after.
                away = Event.Create(Event.ObjectProximity, { "hero", 2, ">", 20, false, true },
                    function() table.insert(log, "cancelled") end)
                Event.Create(Event.ObjectProximity, { "hero", 3, "<", 10, false, true },
                    function() table.insert(log, "arrived"); Event.Delete(away) end)
                "#,
            )
            .exec()
            .unwrap();
        let mut world = TestWorld::default();
        world.positions.insert(1, [0.0, 0.0, 0.0]);
        world.positions.insert(2, [5.0, 0.0, 0.0]);
        world.positions.insert(3, [2000.0, 0.0, 0.0]);
        events.update(0.1, &world);
        assert!(log(&host).is_empty());
        // Teleported to the landing zone: both hold at once.
        world.positions.insert(1, [2000.0, 0.0, 0.0]);
        events.update(0.1, &world);
        assert_eq!(log(&host), ["arrived"]);
        assert!(events.is_empty());
    }

    #[test]
    fn a_hero_proximity_passes_the_heroes_near_as_a_list() {
        // As PmcCon004's `ProximityCallback(self, tGuid)` reads it:
        // `tGuid[1]` is the hero by Solano's helicopter.
        let (host, events) = setup();
        host.lua()
            .load(
                r#"
                Event.Create(Event.ObjectProximity, { "hero", 2, "<", 50 },
                    function(self, near) table.insert(log, type(near) .. " " .. tostring(gid(near[1]))) end, { {} })
                "#,
            )
            .exec()
            .unwrap();
        let mut world = TestWorld::default();
        world.positions.insert(1, [0.0, 0.0, 0.0]);
        world.positions.insert(2, [30.0, 0.0, 0.0]);
        events.update(0.1, &world);
        assert_eq!(log(&host), ["table 1"]);
    }

    #[test]
    fn the_engine_posts_what_scripts_wait_for() {
        let (host, events) = setup();
        host.lua()
            .load(
                r#"Event.Create(Event.ScriptEvent, { "Satellite Targetting Start" },
                    function() table.insert(log, "started") end)"#,
            )
            .exec()
            .unwrap();
        assert_eq!(events.awaited(), ["Satellite Targetting Start"]);
        let world = TestWorld::default();
        events.post("Something else");
        events.update(0.1, &world);
        assert!(log(&host).is_empty());
        events.post("Satellite Targetting Start");
        events.update(0.1, &world);
        assert_eq!(log(&host), ["started"]);
        assert!(events.awaited().is_empty());
    }

    #[test]
    fn the_pdas_opening_names_her_as_the_player_object_the_watch_compares() {
        // OilCon020's tutorial: `Player.GetLocalPlayer() == tData.uPlayer`
        // (here `me`, her guid as the game hands it). A number never
        // matched, so the tutorial never went on.
        let (host, events) = setup();
        let lua = host.lua();
        lua.globals().set("me", guid_value(1)).unwrap();
        lua.load(
            r#"Event.Create(Event.ScriptEvent, { "PDA Open", function(t) return me == t.uPlayer end },
                function() table.insert(log, "in the PDA") end)"#,
        )
        .exec()
        .unwrap();
        let world = TestWorld::default();
        let post = |player: Value| {
            let t = lua.create_table().unwrap();
            t.set("uPlayer", player).unwrap();
            events.post_data("PDA Open", Value::Table(t));
        };
        post(Value::Integer(1));
        events.update(0.1, &world);
        assert!(log(&host).is_empty());
        post(guid_value(1));
        events.update(0.1, &world);
        assert_eq!(log(&host), ["in the PDA"]);
    }

    /// Each pending event used to hold three references into the scripts'
    /// state from Rust, and mlua has room for about 8000: some 80
    /// script-run layers in at once ran it out ("out of auxiliary stack
    /// space"). Their values are kept in a table of the scripts' now.
    #[test]
    fn thousands_of_pending_events_wait_fire_and_are_let_go() {
        let (host, events) = setup();
        host.lua()
            .load(
                r#"ids, fired = {}, 0
                for i = 1, 6000 do
                    ids[i] = Event.Create(Event.ScriptEvent, { "Go" },
                        function(self) fired = fired + self.n end, { { n = 1 } })
                end
                for i = 1, 6000 do
                    Event.CreatePersistent(Event.ScriptEvent, { "Never" }, function() end, {})
                end"#,
            )
            .exec()
            .unwrap();
        assert_eq!(events.len(), 12000);
        let world = TestWorld::default();
        assert!(events.update(0.1, &world).is_empty());
        assert_eq!(events.len(), 12000);
        events.post("Go");
        let errors = events.update(0.1, &world);
        assert!(errors.is_empty(), "{:?}", errors.first());
        let fired: u32 = host.lua().globals().get("fired").unwrap();
        assert_eq!(fired, 6000);
        // The one-shots are used up, and their values let go with them.
        assert_eq!(events.len(), 6000);
        let kept = events.store.borrow().kept.clone().unwrap();
        assert_eq!(kept.pairs::<Value, Value>().count(), 6000);
        host.lua()
            .load("for id = 6001, 12000 do Event.Delete(id) end")
            .exec()
            .unwrap();
        assert!(events.is_empty());
        assert_eq!(kept.pairs::<Value, Value>().count(), 0);
    }

    #[test]
    fn posts_of_relayed_names_are_copied_out_and_still_heard_here() {
        let (host, events) = setup();
        events.relay(&["GPS Beacon Set"]);
        host.lua()
            .load(
                r#"Event.Create(Event.ScriptEvent, { "GPS Beacon Set" },
                    function(self, t) table.insert(log, "here " .. t.nX) end, { {} })
                Event.Post("GPS Beacon Set", { nX = 2791, nY = -383 })
                Event.Post("PDA Open", {})"#,
            )
            .exec()
            .unwrap();
        let relayed = events.take_relayed();
        assert_eq!(relayed.len(), 1);
        assert_eq!(relayed[0].0, "GPS Beacon Set");
        let Value::Table(t) = &relayed[0].1 else {
            panic!("no data");
        };
        assert_eq!(t.get::<f64>("nY").unwrap(), -383.0);
        assert!(events.take_relayed().is_empty());
        events.update(0.1, &TestWorld::default());
        assert_eq!(log(&host), ["here 2791"]);
    }

    #[test]
    fn persistent_conditions_fire_as_they_start_to_hold() {
        let (host, events) = setup();
        host.lua()
            .load(
                "Event.CreatePersistent(Event.ObjectProximity, { 'Prisoner', 'hero', '>', 50 }, \
                 function() table.insert(log, 'far') end)",
            )
            .exec()
            .unwrap();
        let mut world = TestWorld::default();
        world.positions.insert(1, [0.0, 0.0, 0.0]);
        world.positions.insert(2, [100.0, 0.0, 0.0]);
        events.update(0.1, &world);
        events.update(0.1, &world);
        assert_eq!(log(&host), ["far"]);
        world.positions.insert(2, [10.0, 0.0, 0.0]);
        events.update(0.1, &world);
        world.positions.insert(2, [60.0, 0.0, 0.0]);
        events.update(0.1, &world);
        assert_eq!(log(&host), ["far", "far"]);
        // Unknown kinds can be made and never fire.
        host.lua()
            .load("Event.Create(Event.Minigame, {}, function() table.insert(log, 'x') end)")
            .exec()
            .unwrap();
        events.update(0.1, &world);
        assert_eq!(log(&host).len(), 2);
    }

    #[test]
    fn hibernated_waits_for_sleep_and_awake_for_waking() {
        let (host, events) = setup();
        // PmcCon001's gate opener: the door awake opens the gates; the
        // door hibernated waits for her to leave, then for it to wake.
        host.lua()
            .load(
                r##"
                -- The engine passes nothing of its own: only the data.
                local function note(s) return function(...)
                    table.insert(log, s .. select("#", ...))
                end end
                Event.Create(Event.ObjectHibernation, { 3, "awake" }, note("awake "))
                Event.Create(Event.ObjectHibernation, { 3, "hibernated" }, note("hibernated "))
                Event.Create(Event.ObjectHibernation, { 3, "hibernate" }, note("hibernate "))
                -- The HQ manager's wait before reviving a destroyed HQ: any
                -- mode not starting with "a" waits for hibernation.
                Event.Create(Event.ObjectHibernation, { 3, "s" }, note("s "))
                Event.Create(Event.ObjectHibernation, { 3 }, note("none "))
                "##,
            )
            .exec()
            .unwrap();
        let mut world = TestWorld::default();
        world.positions.insert(3, [0.0, 0.0, 0.0]);
        for _ in 0..5 {
            events.update(0.1, &world);
        }
        assert_eq!(log(&host), ["awake 0"]);
        assert_eq!(events.len(), 4);
        world.positions.remove(&3);
        events.update(0.1, &world);
        let mut got = log(&host);
        got.sort();
        assert_eq!(
            got,
            ["awake 0", "hibernate 0", "hibernated 0", "none 0", "s 0"]
        );
        assert!(events.is_empty());
    }

    #[test]
    fn object_is_visible_fires_once_it_is_on_screen() {
        let (host, events) = setup();
        host.lua()
            .load(
                r#"
                Event.Create(Event.ObjectIsVisible, { 9 },
                    function(who, seen) table.insert(log, who .. " saw " .. tostring(seen)) end,
                    { "LoadTank" })
                "#,
            )
            .exec()
            .unwrap();
        let mut world = TestWorld::default();
        // Nowhere to see from yet, then not in view: it waits.
        events.update(0.1, &world);
        world.camera = true;
        events.update(0.1, &world);
        assert!(log(&host).is_empty());
        assert_eq!(events.sight_watched(&world), [9]);
        world.visible.push(9);
        events.update(0.1, &world);
        // Only the data: the engine passes nothing of its own.
        assert_eq!(log(&host), ["LoadTank saw nil"]);
        assert!(events.is_empty());
        assert!(events.take_unhandled().is_empty());
    }

    #[test]
    fn unknown_kinds_are_reported_once() {
        let (host, events) = setup();
        host.lua()
            .load(
                "Event.Create(Event.ObjectSpotted, {}, function() end)\n\
                 Event.Create(Event.ObjectSpotted, {}, function() end)\n\
                 Event.Create(Event.GuiHealthUpdate, {}, function() end)",
            )
            .exec()
            .unwrap();
        let world = TestWorld::default();
        events.update(0.1, &world);
        assert_eq!(events.take_unhandled(), ["ObjectSpotted"]);
        events.update(0.1, &world);
        assert!(events.take_unhandled().is_empty());
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn failing_callbacks_are_reported_not_fatal() {
        let (host, events) = setup();
        host.lua()
            .load("Event.Create(Event.TimerRelative, { 0 }, function() error('boom') end)")
            .exec()
            .unwrap();
        let errors = events.update(0.1, &TestWorld::default());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].error.to_string().contains("boom"));
    }

    #[test]
    fn label_index_names_what_a_scan_names() {
        let h = m2_assets::name_hash;
        let objects = vec![
            (5, vec![h("Prisoner"), h("VZ")]),
            (2, vec![h("Prisoner")]),
            (9, vec![h("VZ"), h("Tank")]),
            (7, vec![]),
        ];
        // What `World::labelled` would name: every object matching an
        // alternative, by guid.
        let scan = |expr: &str| -> Vec<u32> {
            let alts = class_alternatives(expr);
            let mut out: Vec<u32> = objects
                .iter()
                .filter(|(_, l)| {
                    alts.iter()
                        .any(|t| t.iter().all(|&(x, not)| l.contains(&x) != not))
                })
                .map(|o| o.0)
                .collect();
            out.sort_unstable();
            out
        };
        let mut index = LabelIndex::default();
        for (g, labels) in &objects {
            index.add(*g, &mut labels.iter().copied());
        }
        for expr in [
            "Prisoner",
            "prisoner",
            "Prisoner && VZ",
            "Prisoner && !VZ",
            "!VZ",
            "Tank || Prisoner",
            "(VZ && Tank) || (!Prisoner)",
            "Nothing",
        ] {
            assert_eq!(index.labelled(expr), scan(expr), "{expr}");
            // And again, from what it found.
            assert_eq!(index.labelled(expr), scan(expr), "{expr}");
        }
        assert_eq!(index.labelled("Prisoner"), [2, 5]);
        assert_eq!(index.labelled("!VZ"), [2, 7]);
    }

    #[test]
    fn script_world_labels_index_as_it_looks_them_up() {
        use crate::api::{Object, ScriptWorld};
        let mut w = ScriptWorld::default();
        let mut add = |g: u32, labels: &[&str], hashed: &[&str]| {
            let mut o = Object::new("x", [0.0; 3]);
            o.labels = labels.iter().map(|l| (*l).to_owned()).collect();
            o.label_hashes = hashed.iter().map(|l| m2_assets::name_hash(l)).collect();
            w.objects.insert(g, o);
        };
        add(1, &["Prisoner"], &[]);
        add(2, &[], &["prisoner", "VZ"]);
        add(3, &["Tank"], &["VZ"]);
        add(4, &[], &[]);
        let mut index = LabelIndex::of(&w).unwrap();
        for expr in [
            "Prisoner",
            "Prisoner && !VZ",
            "!Tank",
            "VZ || Prisoner",
            "Gone",
        ] {
            assert_eq!(index.labelled(expr), w.labelled(expr), "{expr}");
        }
    }
}
