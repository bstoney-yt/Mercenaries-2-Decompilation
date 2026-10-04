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
}

/// The kinds the scripts use, by how often.
pub const KINDS: [&str; 12] = [
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
}

#[derive(Debug, Default)]
struct Store {
    next_id: u32,
    pending: Vec<Pending>,
    /// `Event.Post` calls not yet delivered: name and data.
    posted: Vec<(String, Value)>,
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
                    s.pending.push(Pending {
                        id,
                        kind,
                        params,
                        callback,
                        data,
                        persistent,
                        last: now,
                    });
                    Ok(MultiValue::from_vec(vec![Value::Integer(i64::from(id))]))
                })?,
            )?;
        }
        let store = Rc::clone(&self.store);
        event.set(
            "Delete",
            safe_function(lua, move |_, args| {
                if let Some(id) = args.front().and_then(as_id) {
                    store.borrow_mut().pending.retain(|p| p.id != id);
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
        lua.globals().set("Event", event)?;
        Ok(())
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

    /// Advances game time by `dt` seconds and fires every event whose
    /// condition holds. Callbacks run after the checks, so events they
    /// create wait for the next update. Returns the callbacks that failed.
    pub fn update(&self, dt: f64, world: &dyn World) -> Vec<EventError> {
        let fired = {
            let mut s = self.store.borrow_mut();
            s.now += dt;
            let now = s.now;
            let posted = std::mem::take(&mut s.posted);
            let mut fired = Vec::new();
            let mut keep = Vec::with_capacity(s.pending.len());
            for mut p in std::mem::take(&mut s.pending) {
                let extra = fires(&p, now, &posted, world);
                match extra {
                    Some(extra) => {
                        fired.push((p.kind.clone(), p.callback.clone(), p.data.clone(), extra));
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
        };
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
}

fn as_id(v: &Value) -> Option<u32> {
    match v {
        Value::Integer(i) => u32::try_from(*i).ok(),
        Value::Number(n) if *n >= 0.0 => Some(*n as u32),
        _ => None,
    }
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

/// Whether `p` fires now; `Some` with any values the callback gets after
/// its data.
fn fires(
    p: &Pending,
    now: f64,
    posted: &[(String, Value)],
    world: &dyn World,
) -> Option<Vec<Value>> {
    match p.kind.as_str() {
        // { seconds }: once that long after it was made (or, persistent,
        // every that long).
        "TimerRelative" => {
            let delay = number(&p.params, 1)?;
            (now - p.last >= delay).then(Vec::new)
        }
        // { object }: when it dies or is removed.
        "ObjectDeath" => {
            let g = guid(&p.params, 1)?;
            (!world.is_alive(g)).then(Vec::new)
        }
        // { object, other, radius }: when the two are within the radius.
        "ObjectProximity" => {
            let a = world.position(guid(&p.params, 1)?)?;
            let b = world.position(guid(&p.params, 2)?)?;
            let r = number(&p.params, 3)?;
            let d2: f32 = (0..3).map(|i| (a[i] - b[i]).powi(2)).sum();
            (f64::from(d2) <= r * r).then(Vec::new)
        }
        // { rider }: when it is in a vehicle.
        "ObjectInSeat" => {
            let v = world.vehicle_of(guid(&p.params, 1)?)?;
            Some(vec![Value::Integer(i64::from(v))])
        }
        // { name }: when something posts that name; the posted data
        // follows the callback's own.
        "ScriptEvent" => {
            let name: String = p.params.raw_get(1).ok()?;
            let (_, data) = posted.iter().find(|(n, _)| *n == name)?;
            Some(vec![data.clone()])
        }
        // Not simulated yet: never fire.
        _ => None,
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
    }

    fn setup() -> (ScriptHost, Events) {
        let host = ScriptHost::new().unwrap();
        let events = Events::default();
        events.install(host.lua()).unwrap();
        host.lua().load("log = {}").exec().unwrap();
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
                local function note(s) return function(...) table.insert(log, s .. table.concat({...}, ",")) end end
                Event.Create(Event.ObjectDeath, { 7 }, note("dead "), { 7 })
                Event.Create(Event.ObjectProximity, { 1, 2, 10 }, note("near "), {})
                Event.Create(Event.ObjectInSeat, { 1 }, note("seat "), {})
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
        world.seats.insert(1, 99);
        host.lua().load("Event.Post('Go', 'y')").exec().unwrap();
        events.update(0.1, &world);
        let mut got = log(&host);
        got.sort();
        assert_eq!(got, ["dead 7", "near ", "posted x,y", "seat 99"]);
        assert!(events.is_empty());
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
