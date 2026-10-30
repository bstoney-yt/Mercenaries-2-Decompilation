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
use std::collections::{BTreeSet, HashSet};
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
}

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
/// The kinds the scripts use, by how often.
pub const KINDS: [&str; 19] = [
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
    params: Table,
    callback: Value,
    data: Option<Table>,
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
    /// `Event.Post` calls not yet delivered: name and data.
    posted: Vec<(String, Value)>,
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
        self.store.borrow_mut().lua = LuaRef(Some(lua.weak()));
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
                    s.pending.push(Pending {
                        id,
                        kind,
                        params,
                        callback,
                        data,
                        persistent,
                        last: now,
                        held: false,
                        held_by: BTreeSet::new(),
                        rode: std::collections::BTreeMap::new(),
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
                    s.deleted.insert(id);
                    s.pending.retain(|p| p.id != id);
                }
                Ok(MultiValue::new())
            })?,
        )?;
        let store = Rc::clone(&self.store);
        event.set(
            "Post",
            safe_function(lua, move |_, args| {
                let mut args = args.into_iter();
                if let Some(Value::String(name)) = args.next() {
                    let data = args.next().unwrap_or(Value::Nil);
                    store
                        .borrow_mut()
                        .posted
                        .push((name.to_string_lossy(), data));
                }
                Ok(MultiValue::new())
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
                        .map(|p| (p.callback.clone(), p.data.clone()))
                        .collect();
                    s.pending.retain(|p| p.persistent || p.kind != kind);
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
            .filter_map(|p| crate::hijack::Minigame::of(&p.params, s.now - p.made))
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
        let mut out: Vec<u32> = self
            .store
            .borrow()
            .pending
            .iter()
            .filter(|p| p.kind == "ObjectIsVisible")
            .flat_map(|p| objects(&p.params, 1, world))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The regions pending `Boundary` events wait for something to leave
    /// (mode `"exit"`: PmcCon034's `PMCCon034OutOfBounds`, which cancels
    /// the contract), for the autoplay to keep her within.
    pub fn exits_watched(&self, world: &dyn World) -> Vec<u32> {
        let mut out: Vec<u32> = self
            .store
            .borrow()
            .pending
            .iter()
            .filter(|p| p.kind == "Boundary" && text(&p.params, 3).as_deref() == Some("exit"))
            .filter_map(|p| boundary_of(&p.params, world))
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
        self.store
            .borrow()
            .pending
            .iter()
            .map(|p| {
                let params: Vec<String> = p
                    .params
                    .clone()
                    .sequence_values::<Value>()
                    .filter_map(Result::ok)
                    .map(|v| crate::script::show_arg(&v))
                    .collect();
                format!("{}{{{}}}", p.kind, params.join(", "))
            })
            .collect()
    }

    /// The names pending `ScriptEvent` events wait to be posted.
    pub fn awaited(&self) -> Vec<String> {
        self.store
            .borrow()
            .pending
            .iter()
            .filter(|p| p.kind == "ScriptEvent")
            .filter_map(|p| p.params.raw_get::<String>(1).ok())
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
                    let name = p.params.raw_get::<String>(1).ok()?;
                    let f = p.params.raw_get::<Option<Function>>(2).ok().flatten()?;
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
        {
            let mut s = self.store.borrow_mut();
            s.now += dt;
            let now = s.now;
            let posted = std::mem::take(&mut s.posted);
            s.deleted.clear();
            let mut fired = Vec::new();
            let mut keep = Vec::with_capacity(s.pending.len());
            for mut p in std::mem::take(&mut s.pending) {
                let extra = match check(&p, now, &posted, accepted, world) {
                    Check::Fire(extra) => Some(vec![extra]),
                    // Fires as the condition starts to hold.
                    Check::Holds(holds, extra) => {
                        let rising = holds && !p.held;
                        p.held = holds;
                        rising.then_some(vec![extra])
                    }
                    // `held_by` keeps who was seated at the last check
                    // (none before the first).
                    Check::Seats(riders) => {
                        let mode = seat_mode(&p);
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
                            let labelled = by_label(&p.params, world);
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
                            fired.push((
                                p.id,
                                p.kind.clone(),
                                p.callback.clone(),
                                p.data.clone(),
                                extra,
                            ));
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
                            fired.push((
                                p.id,
                                p.kind.clone(),
                                p.callback.clone(),
                                p.data.clone(),
                                extra,
                            ));
                        }
                        if p.persistent {
                            p.last = now;
                            keep.push(p);
                        }
                    }
                    None => keep.push(p),
                }
            }
            s.pending = keep;
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
                .map(|&i| {
                    std::mem::replace(
                        &mut fired[i],
                        (0, String::new(), Value::Nil, None, Vec::new()),
                    )
                })
                .collect();
            near.sort_by_key(|f| std::cmp::Reverse(f.0));
            for (slot, f) in slots.into_iter().zip(near) {
                fired[slot] = f;
            }
            fired
        }
    }
}

/// An event that fired: its id, kind, callback, its data, and the values to
/// pass after the data.
type Fired = (u32, String, Value, Option<Table>, Vec<Value>);

/// Runs the callbacks of the events that fired, in turn: one deleted by
/// an earlier callback of the same pass doesn't run (OilCon002's transit:
/// the trip cancelled at the teleport, its arrival check then ran on the
/// cleaned-up objective).
fn fire(store: &RefCell<Store>, fired: Vec<Fired>) -> Vec<EventError> {
    let mut errors = Vec::new();
    for (id, kind, callback, data, extra) in fired {
        if store.borrow().deleted.contains(&id) {
            continue;
        }
        let Value::Function(f) = callback else {
            continue;
        };
        if let Err(error) = call(&f, data.as_ref(), extra) {
            errors.push(EventError { kind, error });
        }
    }
    errors
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
fn seat_mode(p: &Pending) -> String {
    match text(&p.params, 4).as_deref() {
        Some("enter") | None => "ei".to_owned(),
        Some("exit") => "xo".to_owned(),
        Some(m) => m.to_owned(),
    }
}

enum Check {
    /// Fire now with these values after the callback's data.
    Fire(Vec<Value>),
    /// A condition: whether it holds, and the values to pass when it
    /// starts to.
    Holds(bool, Vec<Value>),
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

/// The objects a parameter names: a guid, `"hero"` (the player), or a
/// label. Anything else (an object filter) names none yet.
fn objects(t: &Table, i: i64, world: &dyn World) -> Vec<u32> {
    match t.raw_get::<Value>(i) {
        Ok(Value::String(s)) => {
            let s = s.to_string_lossy();
            if s.eq_ignore_ascii_case("hero") {
                world.player().into_iter().collect()
            } else {
                let labelled = world.labelled(&s);
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
                out.extend(world.labelled(&label));
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

fn check(
    p: &Pending,
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
        "TimerRelative" => match number(&p.params, 1) {
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
            objects(&p.params, 1, world)
                .into_iter()
                .map(|g| {
                    let mut pass = vec![id(g)];
                    if let Some(k) = world.killer(g) {
                        pass.extend([Value::Nil, id(k)]);
                    }
                    (g, !world.is_alive(g), pass)
                })
                .collect(),
        ),
        // { object }: once it is gone from play (removed, or faded out):
        // the support helicopter's pilot is free again once his
        // helicopter is (`MakeRecruitAvailable("Copter")`). Passes the
        // object.
        "ObjectDelete" => match guid(&p.params, 1) {
            Some(g) => Check::Holds(world.position(g).is_none(), vec![id(g)]),
            None => no,
        },
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
            let r = number(&p.params, 4).unwrap_or(0.0);
            let beyond = text(&p.params, 3).is_some_and(|op| op.starts_with('>'));
            let flat = matches!(p.params.raw_get::<Value>(6), Ok(Value::Boolean(true)));
            let (aa, bb) = (objects(&p.params, 1, world), objects(&p.params, 2, world));
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
                p.params.raw_get::<Value>(1),
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
            let wanted = |v: u32| match p.params.raw_get::<Value>(2) {
                Ok(Value::String(s)) => {
                    let named = objects(&p.params, 2, world);
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
                objects(&p.params, 1, world)
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
            let on_foot = match text(&p.params, 2).as_deref() {
                Some("human") => true,
                Some("vehicle") => false,
                _ => return no,
            };
            let exit = text(&p.params, 3).as_deref() == Some("exit");
            Check::Holds((riding != on_foot) != exit, Vec::new())
        }
        // { state, "enter" or "exit" }: as the game enters or leaves a
        // state; passes both.
        "GameStateChange" => {
            let (Some(state), Some(current)) = (text(&p.params, 1), world.game_state()) else {
                return no;
            };
            let exit = text(&p.params, 2).as_deref() == Some("exit");
            Check::Holds(
                current.eq_ignore_ascii_case(&state) != exit,
                vec![
                    p.params.raw_get::<Value>(1).unwrap_or(Value::Nil),
                    p.params.raw_get::<Value>(2).unwrap_or(Value::Nil),
                ],
            )
        }
        // { object, boundary, "enter" or "exit" }.
        "Boundary" => {
            let boundary = boundary_of(&p.params, world);
            let mode = text(&p.params, 3);
            // `"any"`: both ways, the callback told which (the faction
            // zones' trespass watch).
            if let (Some(b), Some("any")) = (boundary, mode.as_deref()) {
                return Check::Crossings(
                    objects(&p.params, 1, world)
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
            let direction = p.params.raw_get::<Value>(3).unwrap_or(Value::Nil);
            for o in objects(&p.params, 1, world) {
                if let Some(inside) = world.inside(o, b)
                    && inside != exit
                {
                    return Check::Holds(true, vec![id(o), id(b), direction]);
                }
            }
            no
        }
        // { cargo, carrier, "Attach" or "Detach" }: when the cargo is
        // hooked onto (or let go from) a winch. Passes cargo and carrier.
        // `MrxTaskObjectiveDeliver` waits on { cargo, 0, "any" } for cargo
        // that reached its place still hanging (OilCon002's listening
        // posts): carrier 0 is any; "any" is taken to hold once it is let
        // go, the change that objective waits for (inferred from its
        // `_OnObjectWinched` use, not checked against the game).
        "ObjectWinched" => {
            let Some(cargo) = guid(&p.params, 1) else {
                return no;
            };
            let mode = text(&p.params, 3);
            let detach = matches!(mode.as_deref(), Some("detach" | "any"));
            let carrier = guid(&p.params, 2).filter(|&c| c != 0);
            let holds = match world.winch(cargo) {
                Some(None) => detach,
                Some(Some(c)) => !detach && carrier.is_none_or(|w| w == c),
                None => false,
            };
            let mut extra = vec![id(cargo)];
            extra.extend(carrier.map(id));
            Check::Holds(holds, extra)
        }
        // { object, "<" or ">", health }: while its health is below (or
        // above) the figure; passes the object. The hero's survival
        // script watches for drops below her current health this way.
        "ObjectHealth" => {
            let Some(figure) = number(&p.params, 3) else {
                return no;
            };
            let above = text(&p.params, 2).as_deref() == Some(">");
            for o in objects(&p.params, 1, world) {
                if let Some(h) = world.health(o)
                    && (f64::from(h) > figure) == above
                    && f64::from(h) != figure
                {
                    return Check::Holds(true, vec![id(o)]);
                }
            }
            no
        }
        // { character (or filter), from, to }: as one of them goes from a
        // state matching `from` to one matching `to` ([`state_matches`]:
        // the verify objective's `{ target, "*", "KnockedDown.Idle" }`),
        // after the event was made. Passes the character.
        "HumanStateTransition" => {
            let (Some(from), Some(to)) = (text(&p.params, 2), text(&p.params, 3)) else {
                return no;
            };
            let who = objects(&p.params, 1, world);
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
        // { character }: when the action `Human.SetState` gave it has
        // played to its end.
        "HumanActionComplete" => match guid(&p.params, 1) {
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
            let targets = objects(&p.params, 2, world);
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
        "AnimationEvent" => match text(&p.params, 2) {
            Some(event) => {
                let who = guid(&p.params, 1).filter(|&g| g != 0);
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
        "Button" => match text(&p.params, 2) {
            Some(button) => {
                let pressed = world
                    .button_presses()
                    .iter()
                    .any(|(t, b)| *b == button && *t >= p.last && (*t > p.last || !p.persistent));
                if pressed { Check::Fire(Vec::new()) } else { no }
            }
            None => no,
        },
        "Minigame" => Check::Minigame(crate::hijack::check_minigame(
            &p.params, p.made, now, p.read, p.memo, world,
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
        "HumanAnimationNearlyCompleted" => match guid(&p.params, 1) {
            Some(g) => {
                let margin = p.params.raw_get::<f64>(2).unwrap_or(0.1) as f32;
                match world.raw_clip_left(g) {
                    Some(left) if left <= margin => Check::Holds(true, vec![id(g)]),
                    _ => no,
                }
            }
            None => no,
        },
        "ObjectHibernation" | "ObjectIsReady" => match guid(&p.params, 1) {
            Some(g) => {
                let want_awake = !text(&p.params, 2).is_some_and(|t| t.starts_with("hibernat"));
                Check::Holds(world.is_awake(g) == want_awake, vec![id(g)])
            }
            None => no,
        },
        // { object }: while it is on the player's screen
        // ([`World::is_visible`]): PmcCon001 waits, once she is inside the
        // villa, for the outpost's tank column to come into view to load
        // the tank (`LoadTank`). A label or filter names several: the
        // first seen. Passes the object.
        "ObjectIsVisible" => {
            let seen = objects(&p.params, 1, world)
                .into_iter()
                .find(|&g| world.is_visible(g) == Some(true));
            match seen {
                Some(g) => Check::Holds(true, vec![id(g)]),
                None => no,
            }
        }
        // { name, filter }: when something posts that name, and the
        // filter, if any, says yes to the posted data (the faction
        // manager's attitude watches compare it with what they want);
        // passes the posted data.
        "ScriptEvent" => {
            let Ok(name) = p.params.raw_get::<String>(1) else {
                return no;
            };
            // Filters ran before the checks ([`Events::filter_posts`]).
            let filter = p.params.raw_get::<Option<Function>>(2).ok().flatten();
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
        host.lua().load("Event.Delete(id)").exec().unwrap();
        events.update(1.0, &world);
        assert_eq!(log(&host).len(), 3);
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
                Event.Create(Event.ObjectHibernation, { 3, "awake" }, note("awake "), {})
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
                r#"
                Event.Create(Event.ObjectHibernation, { 3, "awake" },
                    function(d) table.insert(log, "awake " .. gid(d)) end)
                Event.Create(Event.ObjectHibernation, { 3, "hibernated" },
                    function(d) table.insert(log, "hibernated " .. gid(d)) end)
                Event.Create(Event.ObjectHibernation, { 3, "hibernate" },
                    function(d) table.insert(log, "hibernate " .. gid(d)) end)
                "#,
            )
            .exec()
            .unwrap();
        let mut world = TestWorld::default();
        world.positions.insert(3, [0.0, 0.0, 0.0]);
        for _ in 0..5 {
            events.update(0.1, &world);
        }
        assert_eq!(log(&host), ["awake 3"]);
        assert_eq!(events.len(), 2);
        world.positions.remove(&3);
        events.update(0.1, &world);
        let mut got = log(&host);
        got.sort();
        assert_eq!(got, ["awake 3", "hibernate 3", "hibernated 3"]);
        assert!(events.is_empty());
    }

    #[test]
    fn object_is_visible_fires_once_it_is_on_screen() {
        let (host, events) = setup();
        host.lua()
            .load(
                r#"
                Event.Create(Event.ObjectIsVisible, { 9 },
                    function(who, seen) table.insert(log, who .. " saw " .. gid(seen)) end,
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
        assert_eq!(log(&host), ["LoadTank saw 9"]);
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
}
