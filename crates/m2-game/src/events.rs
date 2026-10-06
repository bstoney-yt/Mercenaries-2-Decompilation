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
use std::collections::BTreeSet;
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
    /// The vehicle an object rides in, if any.
    fn vehicle_of(&self, guid: u32) -> Option<u32>;
    /// The player's character (scripts write it `"hero"`).
    fn player(&self) -> Option<u32>;
    /// Whether an object is simulated near the player rather than
    /// hibernating far away.
    fn is_awake(&self, guid: u32) -> bool;
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
}

/// The kinds the scripts use, by how often.
pub const KINDS: [&str; 14] = [
    "TimerRelative",
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
}

#[derive(Debug, Default)]
struct Store {
    next_id: u32,
    pending: Vec<Pending>,
    /// `Event.Post` calls not yet delivered: name and data.
    posted: Vec<(String, Value)>,
    /// With `M2_STUB_ARGS`: events made and deleted, for diagnostics.
    trace: Vec<String>,
    /// Game time, seconds.
    now: f64,
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
        let event = lua.create_table()?;
        for kind in KINDS {
            event.set(kind, kind)?;
        }
        for (name, persistent) in [("Create", false), ("CreatePersistent", true)] {
            let store = Rc::clone(&self.store);
            event.set(
                name,
                safe_function(lua, move |_, args| {
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

    /// Events made and deleted since the last call (with `M2_STUB_ARGS`).
    pub fn take_trace(&self) -> Vec<String> {
        std::mem::take(&mut self.store.borrow_mut().trace)
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
        self.store
            .borrow_mut()
            .posted
            .push((name.to_owned(), Value::Nil));
    }

    /// Advances game time by `dt` seconds and fires every event whose
    /// condition holds. Callbacks run after the checks, so events they
    /// create wait for the next update. Returns the callbacks that failed.
    pub fn update(&self, dt: f64, world: &dyn World) -> Vec<EventError> {
        let fired = self.due(dt, world);
        fire(fired)
    }

    /// [`Events::update`] for a world the scripts also change: it is
    /// borrowed only while the conditions are checked, so callbacks can
    /// move, spawn and kill.
    pub fn update_shared<W: World>(&self, dt: f64, world: &RefCell<W>) -> Vec<EventError> {
        let fired = self.due(dt, &*world.borrow());
        fire(fired)
    }

    fn due(&self, dt: f64, world: &dyn World) -> Vec<Fired> {
        {
            let mut s = self.store.borrow_mut();
            s.now += dt;
            let now = s.now;
            let posted = std::mem::take(&mut s.posted);
            let mut fired = Vec::new();
            let mut keep = Vec::with_capacity(s.pending.len());
            for mut p in std::mem::take(&mut s.pending) {
                let extra = match check(&p, now, &posted, world) {
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
                        for (r, (inside, instantly), extra) in riders {
                            let was = p.held_by.contains(&r);
                            if inside {
                                seated.insert(r);
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
                };
                match extra {
                    Some(extras) => {
                        for extra in extras {
                            fired.push((p.kind.clone(), p.callback.clone(), p.data.clone(), extra));
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
            fired
        }
    }
}

/// An event that fired: kind, callback, its data, and the values to pass
/// after the data.
type Fired = (String, Value, Option<Table>, Vec<Value>);

fn fire(fired: Vec<Fired>) -> Vec<EventError> {
    let mut errors = Vec::new();
    for (kind, callback, data, extra) in fired {
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
    class_terms(expr)
        .iter()
        .all(|&(h, not)| labels.contains(&h) != not)
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
        Ok(Value::Table(f)) => {
            let mut out: Vec<u32> = f
                .raw_get::<Option<Table>>("objs")
                .ok()
                .flatten()
                .map(|t| {
                    t.sequence_values::<Value>()
                        .filter_map(|v| as_id(&v.ok()?))
                        .collect()
                })
                .unwrap_or_default();
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

fn text(t: &Table, i: i64) -> Option<String> {
    match t.raw_get::<Value>(i).ok()? {
        Value::String(s) => Some(s.to_string_lossy().to_ascii_lowercase()),
        _ => None,
    }
}

/// Checks `p` against the world. Readings of each kind's parameters come
/// from the game's call sites.
fn check(p: &Pending, now: f64, posted: &[(String, Value)], world: &dyn World) -> Check {
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
        "ObjectDeath" => Check::Each(
            objects(&p.params, 1, world)
                .into_iter()
                .map(|g| (g, !world.is_alive(g), vec![id(g)]))
                .collect(),
        ),
        // { a, b, "<" or ">", metres, ... }: when some a is within (or
        // beyond) the distance of some b; a and b are guids, "hero" or
        // labels. Passes the pair.
        "ObjectProximity" => {
            let r = number(&p.params, 4).unwrap_or(0.0);
            let beyond = text(&p.params, 3).as_deref() == Some(">");
            let (aa, bb) = (objects(&p.params, 1, world), objects(&p.params, 2, world));
            for &a in &aa {
                for &b in &bb {
                    let (Some(pa), Some(pb)) = (world.position(a), world.position(b)) else {
                        continue;
                    };
                    let d2: f32 = (0..3).map(|i| (pa[i] - pb[i]).powi(2)).sum();
                    let within = f64::from(d2) <= r * r;
                    if within != beyond {
                        return Check::Holds(true, vec![id(a), id(b)]);
                    }
                }
            }
            no
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
            // The boundary is a guid or the region's name.
            let boundary = guid(&p.params, 2).or_else(|| match p.params.raw_get::<Value>(2) {
                Ok(Value::String(s)) => world.named(&s.to_string_lossy()),
                _ => None,
            });
            let (Some(b), exit) = (boundary, text(&p.params, 3).as_deref() == Some("exit")) else {
                return no;
            };
            for o in objects(&p.params, 1, world) {
                if let Some(inside) = world.inside(o, b)
                    && inside != exit
                {
                    return Check::Holds(true, vec![id(o), id(b)]);
                }
            }
            no
        }
        // { cargo, carrier, "Attach" or "Detach" }: when the cargo is
        // hooked onto (or let go from) a winch. Passes cargo and carrier.
        "ObjectWinched" => {
            let (Some(cargo), detach) = (
                guid(&p.params, 1),
                text(&p.params, 3).is_some_and(|t| t.eq_ignore_ascii_case("detach")),
            ) else {
                return no;
            };
            let carrier = guid(&p.params, 2);
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
        // { object, "awake" or "hibernate" }.
        "ObjectHibernation" => match guid(&p.params, 1) {
            Some(g) => {
                let want_awake = text(&p.params, 2).as_deref() != Some("hibernate");
                Check::Holds(world.is_awake(g) == want_awake, vec![id(g)])
            }
            None => no,
        },
        // { name, ... }: when something posts that name; passes the
        // posted data.
        "ScriptEvent" => {
            let Ok(name) = p.params.raw_get::<String>(1) else {
                return no;
            };
            match posted.iter().find(|(n, _)| *n == name) {
                Some((_, data)) => Check::Fire(vec![data.clone()]),
                None => no,
            }
        }
        // Not simulated yet: never fires.
        _ => no,
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
    }

    impl World for TestWorld {
        fn position(&self, guid: u32) -> Option<[f32; 3]> {
            self.positions.get(&guid).copied()
        }
        fn is_alive(&self, guid: u32) -> bool {
            !self.dead.contains(&guid)
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
        fn inside(&self, _: u32, _: u32) -> Option<bool> {
            None
        }
        fn labelled(&self, label: &str) -> Vec<u32> {
            if label == "Prisoner" {
                vec![2]
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
    fn deaths_proximity_seats_and_posts() {
        let (host, events) = setup();
        host.lua()
            .load(
                r#"
                local function note(s) return function(...)
                    local t = {}
                    for i, v in ipairs({...}) do t[i] = tostring(gid(v)) end
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
            ["awake 3", "dead 7,7", "near 1,2", "posted x,y", "seat 1,99"]
        );
        assert!(events.is_empty());
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
