//! The engine libraries the scripts call most (`Pg`, `Object`, `Player`,
//! `Vehicle`, `Ai`, `Net`, `Debug`), over a [`ScriptWorld`]: the scripts'
//! view of the game.
//!
//! The game (m2-alpha) keeps the world in step each frame: it registers
//! the objects scripts may name, updates their positions, health and
//! seats, and takes the [`Command`]s the scripts issued (spawn this, move
//! that, give this soldier a goal) to carry out. Keeping the scripts' view
//! separate means a Lua call never reaches into the game's own state.
//!
//! Argument orders come from the game's call sites (`wad -- calls`):
//! positions are three numbers, not tables; `Pg.Spawn(what, x, y, z,
//! yaw, ...)` returns the new object; `Ai.Goal{ AIGuid, Goal, Location,
//! Target, Priority, Force, Callback, CallbackData }`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use mlua::{Function, Lua, MultiValue, Table, Value};

use crate::events::World;
use crate::script::safe_function;

/// Guids handed to objects scripts spawn start here, above any layer's
/// entity ids.
pub const FIRST_SPAWNED: u32 = 0x4000_0000;

/// One object the scripts can see.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Object {
    pub name: String,
    pub position: [f32; 3],
    /// Radians about the vertical axis.
    pub yaw: f32,
    pub health: f32,
    pub max_health: f32,
    pub alive: bool,
    pub labels: Vec<String>,
    /// The vehicle it rides in.
    pub vehicle: Option<u32>,
    pub player_controlled: bool,
    /// Simulated near the player rather than hibernating.
    pub awake: bool,
}

impl Object {
    pub fn new(name: &str, position: [f32; 3]) -> Self {
        Self {
            name: name.to_owned(),
            position,
            health: 100.0,
            max_health: 100.0,
            alive: true,
            awake: true,
            ..Self::default()
        }
    }
}

/// Something a script asked the game to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Create `what` (an object template's name, such as `"Veyron"` or
    /// `"Explosion (Grenade)"`) as `guid`.
    Spawn {
        guid: u32,
        what: String,
        position: [f32; 3],
        yaw: f32,
    },
    Remove(u32),
    Kill(u32),
    Move {
        guid: u32,
        position: [f32; 3],
    },
    Turn {
        guid: u32,
        yaw: f32,
    },
    SetHealth {
        guid: u32,
        health: f32,
    },
    /// An AI order: `goal` is the game's name for it ("MoveTo", "Idle",
    /// "Exit", "Pickup", "HeliLand", ...).
    Goal {
        guid: u32,
        goal: String,
        location: Option<[f32; 3]>,
        target: Option<u32>,
    },
    ExitVehicle(u32),
    /// Stream a layer in or out; report it done with
    /// [`crate::layers::finished`].
    Layer {
        op: crate::layers::LayerOp,
        name: String,
    },
    /// A call the game only needs to hear about: `"Pg.SaveGame"`,
    /// `"Pg.ContractActivated"`, `"Pg.ContractCompleted"`, ...
    Notify(String),
}

/// The scripts' view of the game, and what they asked of it.
#[derive(Debug, Default)]
pub struct ScriptWorld {
    pub objects: HashMap<u32, Object>,
    /// The player's character.
    pub player: Option<u32>,
    pub commands: Vec<Command>,
    /// `Debug.Printf` output, oldest first.
    pub printed: Vec<String>,
    next_spawn: u32,
    /// What `Sys.GetAssetRequestMax` answers; scripts raise it while
    /// layers load and put it back.
    pub asset_request_max: u32,
    /// Layers the archives hold and which are loaded.
    pub layers: crate::layers::LayerState,
    /// Goal callbacks waiting for the AI to finish: guid, callback, data.
    goal_callbacks: Vec<(u32, Function, Option<Table>)>,
}

impl ScriptWorld {
    /// Adds (or replaces) an object scripts can name.
    pub fn insert(&mut self, guid: u32, object: Object) {
        self.objects.insert(guid, object);
    }

    /// The object called `name` (layer entity names are unique enough;
    /// the first match wins).
    pub fn by_name(&self, name: &str) -> Option<u32> {
        self.objects
            .iter()
            .filter(|(_, o)| o.name == name)
            .map(|(&g, _)| g)
            .min()
    }

    /// Takes the commands issued since the last call.
    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.commands)
    }

    fn spawn_guid(&mut self) -> u32 {
        self.next_spawn += 1;
        FIRST_SPAWNED + self.next_spawn
    }
}

impl World for ScriptWorld {
    fn position(&self, guid: u32) -> Option<[f32; 3]> {
        self.objects.get(&guid).map(|o| o.position)
    }
    fn is_alive(&self, guid: u32) -> bool {
        self.objects.get(&guid).is_some_and(|o| o.alive)
    }
    fn vehicle_of(&self, guid: u32) -> Option<u32> {
        self.objects.get(&guid)?.vehicle
    }
    fn player(&self) -> Option<u32> {
        self.player
    }
    fn is_awake(&self, guid: u32) -> bool {
        self.objects.get(&guid).is_some_and(|o| o.awake)
    }
    fn inside(&self, _: u32, _: u32) -> Option<bool> {
        None
    }
    fn labelled(&self, label: &str) -> Vec<u32> {
        let mut out: Vec<u32> = self
            .objects
            .iter()
            .filter(|(_, o)| o.labels.iter().any(|l| l == label))
            .map(|(&g, _)| g)
            .collect();
        out.sort_unstable();
        out
    }
}

/// The world shared between the game and the script libraries.
pub type Shared = Rc<RefCell<ScriptWorld>>;

/// Calls the goal callbacks of `guid`'s finished AI goals, with their data
/// unpacked. Returns any errors.
pub fn goal_finished(world: &Shared, guid: u32) -> Vec<mlua::Error> {
    let done: Vec<_> = {
        let mut w = world.borrow_mut();
        let (done, keep) = std::mem::take(&mut w.goal_callbacks)
            .into_iter()
            .partition(|(g, _, _)| *g == guid);
        w.goal_callbacks = keep;
        done
    };
    let mut errors = Vec::new();
    for (_, f, data) in done {
        let args: Vec<Value> = match data {
            Some(t) => t.sequence_values().filter_map(Result::ok).collect(),
            None => Vec::new(),
        };
        if let Err(e) = f.call::<()>(MultiValue::from_vec(args)) {
            errors.push(e);
        }
    }
    errors
}

fn id(v: Option<&Value>) -> Option<u32> {
    match v? {
        Value::Integer(i) => u32::try_from(*i).ok(),
        Value::Number(n) if *n >= 0.0 => Some(*n as u32),
        _ => None,
    }
}

fn num(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Integer(i) => Some(*i as f32),
        Value::Number(n) => Some(*n as f32),
        _ => None,
    }
}

fn guid_value(g: Option<u32>) -> Value {
    g.map_or(Value::Nil, |g| Value::Number(f64::from(g)))
}

fn values<I: IntoIterator<Item = Value>>(v: I) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_iter(v))
}

/// Installs the libraries over `world`. Functions not written here stay
/// missing (or stubs, with `ScriptHost::stub_missing_globals`).
/// The engine's libraries (the rest of the globals scripts use are their
/// own modules); see `ScriptHost::complete_libraries`.
pub const LIBRARIES: [&str; 26] = [
    "Ai",
    "Airstrike",
    "Camera",
    "Controller",
    "Debug",
    "Event",
    "Graphics",
    "Gui",
    "Hud",
    "Human",
    "Marker",
    "Math",
    "Munitions",
    "Net",
    "Object",
    "ObjectFilter",
    "ObjectState",
    "Pda",
    "Pg",
    "Player",
    "Sound",
    "String",
    "Sys",
    "Vehicle",
    "VO",
    "Weapon",
];

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
    // Each function gets the shared world and its arguments as a slice.
    let add = |table: &Table,
               name: &str,
               f: fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>|
     -> mlua::Result<()> {
        let w = Rc::clone(world);
        table.set(
            name,
            safe_function(lua, move |lua, args| {
                let args: Vec<Value> = args.into_iter().collect();
                f(&mut w.borrow_mut(), lua, &args)
            })?,
        )
    };

    let debug = lib("Debug")?;
    add(&debug, "Printf", |w, _, a| {
        let text: Vec<String> = a
            .iter()
            .map(|v| match v {
                Value::String(s) => s.to_string_lossy(),
                other => format!("{other:?}"),
            })
            .collect();
        w.printed.push(text.concat());
        values([])
    })?;

    add(&debug, "GetCallstack", |_, lua, _| {
        values([Value::String(lua.create_string("")?)])
    })?;

    let pg = lib("Pg")?;
    add(&pg, "GetGuidByName", |w, _, a| {
        let g = match a.first() {
            Some(Value::String(s)) => w.by_name(&s.to_string_lossy()),
            _ => None,
        };
        values([guid_value(g)])
    })?;
    add(&pg, "Spawn", |w, _, a| {
        let what = match a.first() {
            Some(Value::String(s)) => s.to_string_lossy(),
            // A guid names an object to copy (a delivery vehicle).
            Some(v) => match id(Some(v)).and_then(|g| w.objects.get(&g)) {
                Some(o) => o.name.clone(),
                None => return values([Value::Nil]),
            },
            None => return values([Value::Nil]),
        };
        let position = [
            num(a.get(1)).unwrap_or(0.0),
            num(a.get(2)).unwrap_or(0.0),
            num(a.get(3)).unwrap_or(0.0),
        ];
        let yaw = num(a.get(4)).unwrap_or(0.0);
        let guid = w.spawn_guid();
        let mut object = Object::new(&what, position);
        object.yaw = yaw;
        w.insert(guid, object);
        w.commands.push(Command::Spawn {
            guid,
            what,
            position,
            yaw,
        });
        values([Value::Number(f64::from(guid))])
    })?;

    let object = lib("Object")?;
    add(&object, "GetPosition", |w, _, a| {
        match id(a.first()).and_then(|g| w.objects.get(&g)) {
            Some(o) => values(o.position.map(|c| Value::Number(f64::from(c)))),
            None => values([]),
        }
    })?;
    add(&object, "SetPosition", |w, _, a| {
        if let (Some(guid), Some(x), Some(y), Some(z)) =
            (id(a.first()), num(a.get(1)), num(a.get(2)), num(a.get(3)))
        {
            let position = [x, y, z];
            if let Some(o) = w.objects.get_mut(&guid) {
                o.position = position;
            }
            w.commands.push(Command::Move { guid, position });
        }
        values([])
    })?;
    add(&object, "GetYaw", |w, _, a| {
        let yaw = id(a.first()).and_then(|g| w.objects.get(&g)).map(|o| o.yaw);
        values([yaw.map_or(Value::Nil, |y| Value::Number(f64::from(y)))])
    })?;
    add(&object, "SetYaw", |w, _, a| {
        if let (Some(guid), Some(yaw)) = (id(a.first()), num(a.get(1))) {
            if let Some(o) = w.objects.get_mut(&guid) {
                o.yaw = yaw;
            }
            w.commands.push(Command::Turn { guid, yaw });
        }
        values([])
    })?;
    add(&object, "IsAlive", |w, _, a| {
        values([Value::Boolean(id(a.first()).is_some_and(|g| w.is_alive(g)))])
    })?;
    add(&object, "IsAwake", |w, _, a| {
        values([Value::Boolean(id(a.first()).is_some_and(|g| w.is_awake(g)))])
    })?;
    add(&object, "GetHealth", |w, _, a| {
        let o = id(a.first()).and_then(|g| w.objects.get(&g));
        values([o.map_or(Value::Nil, |o| Value::Number(f64::from(o.health)))])
    })?;
    add(&object, "GetMaxHealth", |w, _, a| {
        let o = id(a.first()).and_then(|g| w.objects.get(&g));
        values([o.map_or(Value::Nil, |o| Value::Number(f64::from(o.max_health)))])
    })?;
    add(&object, "SetHealth", |w, _, a| {
        if let (Some(guid), Some(health)) = (id(a.first()), num(a.get(1))) {
            if let Some(o) = w.objects.get_mut(&guid) {
                o.health = health;
            }
            w.commands.push(Command::SetHealth { guid, health });
        }
        values([])
    })?;
    add(&object, "Kill", |w, _, a| {
        if let Some(guid) = id(a.first()) {
            if let Some(o) = w.objects.get_mut(&guid) {
                o.alive = false;
                o.health = 0.0;
            }
            w.commands.push(Command::Kill(guid));
        }
        values([])
    })?;
    add(&object, "Remove", |w, _, a| {
        if let Some(guid) = id(a.first()) {
            w.objects.remove(&guid);
            w.commands.push(Command::Remove(guid));
        }
        values([])
    })?;
    add(&object, "IsPlayerControlled", |w, _, a| {
        let o = id(a.first()).and_then(|g| w.objects.get(&g));
        values([Value::Boolean(o.is_some_and(|o| o.player_controlled))])
    })?;
    add(&object, "HasLabel", |w, _, a| {
        let has = match (id(a.first()).and_then(|g| w.objects.get(&g)), a.get(1)) {
            (Some(o), Some(Value::String(l))) => {
                let l = l.to_string_lossy();
                o.labels.contains(&l)
            }
            _ => false,
        };
        values([Value::Boolean(has)])
    })?;
    add(&object, "AddLabel", |w, _, a| {
        if let (Some(o), Some(Value::String(l))) =
            (id(a.first()).and_then(|g| w.objects.get_mut(&g)), a.get(1))
        {
            let l = l.to_string_lossy();
            if !o.labels.contains(&l) {
                o.labels.push(l);
            }
        }
        values([])
    })?;
    add(&object, "RemoveLabel", |w, _, a| {
        if let (Some(o), Some(Value::String(l))) =
            (id(a.first()).and_then(|g| w.objects.get_mut(&g)), a.get(1))
        {
            let l = l.to_string_lossy();
            o.labels.retain(|x| *x != l);
        }
        values([])
    })?;
    add(&object, "GetName", |w, lua, a| {
        let o = id(a.first()).and_then(|g| w.objects.get(&g));
        values([match o {
            Some(o) => Value::String(lua.create_string(&o.name)?),
            None => Value::Nil,
        }])
    })?;
    add(&object, "InSeat", |w, _, a| {
        values([Value::Boolean(
            id(a.first()).is_some_and(|g| w.vehicle_of(g).is_some()),
        )])
    })?;

    let player = lib("Player")?;
    // The PMC's fuel tank: invented, the game's figure is not decoded.
    add(&player, "GetFuelCapacity", |_, _, _| {
        values([Value::Number(100.0)])
    })?;
    for name in [
        "GetLocalCharacter",
        "GetAnyCharacter",
        "GetPrimaryCharacter",
        "GetCharacter",
    ] {
        add(&player, name, |w, _, _| values([guid_value(w.player)]))?;
    }
    // The second player of co-op: none.
    add(&player, "GetSecondaryCharacter", |_, _, _| {
        values([Value::Nil])
    })?;
    add(&player, "GetAllCharacters", |w, lua, _| {
        let t = lua.create_table()?;
        if let Some(p) = w.player {
            t.raw_push(i64::from(p))?;
        }
        values([Value::Table(t)])
    })?;
    // One player, numbered 1.
    for name in ["GetLocalPlayer", "GetPrimaryPlayer"] {
        add(&player, name, |_, _, _| values([Value::Integer(1)]))?;
    }
    add(&player, "GetSecondaryPlayer", |_, _, _| {
        values([Value::Nil])
    })?;
    add(&player, "IsLocal", |_, _, a| {
        values([Value::Boolean(id(a.first()) == Some(1))])
    })?;
    for name in ["GetAllPlayers", "GetCurrentPlayers"] {
        add(&player, name, |_, lua, _| {
            let t = lua.create_table()?;
            t.raw_push(1)?;
            values([Value::Table(t)])
        })?;
    }

    let vehicle = lib("Vehicle")?;
    add(&vehicle, "GetDriver", |w, _, a| {
        // The first rider found stands for the driver.
        let v = id(a.first());
        let driver = w
            .objects
            .iter()
            .filter(|(_, o)| o.vehicle.is_some() && o.vehicle == v)
            .map(|(&g, _)| g)
            .min();
        values([guid_value(driver)])
    })?;
    add(&vehicle, "GetFromRider", |w, _, a| {
        values([guid_value(id(a.first()).and_then(|g| w.vehicle_of(g)))])
    })?;
    add(&vehicle, "GetRiders", |w, lua, a| {
        let v = id(a.first());
        let t = lua.create_table()?;
        let mut riders: Vec<u32> = w
            .objects
            .iter()
            .filter(|(_, o)| o.vehicle.is_some() && o.vehicle == v)
            .map(|(&g, _)| g)
            .collect();
        riders.sort_unstable();
        for r in riders {
            t.raw_push(i64::from(r))?;
        }
        values([Value::Table(t)])
    })?;
    add(&vehicle, "Exit", |w, _, a| {
        if let Some(guid) = id(a.first()) {
            w.commands.push(Command::ExitVehicle(guid));
        }
        values([])
    })?;

    let ai = lib("Ai")?;
    add(&ai, "Goal", |w, _, a| {
        let Some(Value::Table(t)) = a.first() else {
            return values([]);
        };
        let Some(guid) = id(t.raw_get::<Value>("AIGuid").ok().as_ref()) else {
            return values([]);
        };
        let goal: String = t.raw_get::<Option<String>>("Goal")?.unwrap_or_default();
        let location = match t.raw_get::<Value>("Location")? {
            Value::Table(l) => {
                let c = |i| num(l.raw_get::<Value>(i).ok().as_ref());
                match (c(1), c(2), c(3)) {
                    (Some(x), Some(y), Some(z)) => Some([x, y, z]),
                    _ => None,
                }
            }
            _ => None,
        };
        let target = id(t.raw_get::<Value>("Target").ok().as_ref());
        if let Value::Function(f) = t.raw_get::<Value>("Callback")? {
            let data = t.raw_get::<Option<Table>>("CallbackData")?;
            w.goal_callbacks.push((guid, f, data));
        }
        w.commands.push(Command::Goal {
            guid,
            goal,
            location,
            target,
        });
        values([])
    })?;

    // Single player: this machine is the server.
    let net = lib("Net")?;
    add(&net, "IsServer", |_, _, _| values([Value::Boolean(true)]))?;
    for name in ["IsClient", "IsActive", "IsMultiplayer"] {
        add(&net, name, |_, _, _| values([Value::Boolean(false)]))?;
    }
    // Co-op bookkeeping around layer loads: nothing to do.
    for name in [
        "BeginLayerEventGroup",
        "EndLayerEventGroup",
        "DoneReloadingLayers",
    ] {
        add(&net, name, |_, _, _| values([]))?;
    }

    // Calls the game only needs to hear about.
    for name in [
        "SaveGame",
        "ContractActivated",
        "ContractCompleted",
        "ContractCancelled",
    ] {
        pg.set(name, notifier(lua, world, format!("Pg.{name}"))?)?;
    }
    add(&pg, "LoadIsRetry", |_, _, _| {
        values([Value::Boolean(false)])
    })?;
    // No saves yet: every load is a new game, which the bootstrap starts
    // itself (`LoadSingleton(nil)`) when this answers false.
    add(&pg, "LoadGame", |w, _, _| {
        w.commands.push(Command::Notify("Pg.LoadGame".into()));
        values([Value::Boolean(false)])
    })?;

    // Developer switches (skip to a mission, force a briefing): all off.
    let sys = lib("Sys")?;
    for name in ["GetSkipMission", "GetINIBriefing", "GetForceNewGame"] {
        add(&sys, name, |_, _, _| values([Value::Nil]))?;
    }
    for name in ["SetSkipMission", "RequestGameState"] {
        add(&sys, name, |_, _, _| values([]))?;
    }
    add(&sys, "GetIgnoreLayers", |_, lua, _| {
        values([Value::Table(lua.create_table()?)])
    })?;
    // The streaming budget the layer manager raises while it loads.
    add(&sys, "GetAssetRequestMax", |w, _, _| {
        values([Value::Number(f64::from(w.asset_request_max))])
    })?;
    add(&sys, "SetAssetRequestMax", |w, _, a| {
        if let Some(n) = num(a.first()) {
            w.asset_request_max = n as u32;
        }
        values([])
    })?;

    // The game's assert: logged, and the script carries on, as a release
    // build would.
    lua.globals().raw_set(
        "ASSERT",
        safe_function(lua, {
            let w = Rc::clone(world);
            move |_, args| {
                let args: Vec<Value> = args.into_iter().collect();
                let holds = !matches!(
                    args.first(),
                    None | Some(Value::Nil | Value::Boolean(false))
                );
                if !holds {
                    let what = match args.get(1) {
                        Some(Value::String(s)) => s.to_string_lossy(),
                        _ => String::new(),
                    };
                    w.borrow_mut()
                        .printed
                        .push(format!("ASSERT failed: {what}"));
                }
                Ok(MultiValue::new())
            }
        })?,
    )?;
    crate::layers::install(lua, world)
}

/// A function that records `call` as a [`Command::Notify`].
fn notifier(lua: &Lua, world: &Shared, call: String) -> mlua::Result<Function> {
    let w = Rc::clone(world);
    safe_function(lua, move |_, _| {
        w.borrow_mut().commands.push(Command::Notify(call.clone()));
        Ok(MultiValue::new())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::ScriptHost;

    fn setup() -> (ScriptHost, Shared) {
        let host = ScriptHost::new().unwrap();
        let world: Shared = Rc::default();
        {
            let mut w = world.borrow_mut();
            let mut hero = Object::new("Mattias", [10.0, 0.0, 5.0]);
            hero.player_controlled = true;
            w.insert(1, hero);
            w.player = Some(1);
            let mut jeep = Object::new("_global_veh_jeep 0x0001", [0.0, 0.0, 0.0]);
            jeep.labels.push("Vehicle".into());
            w.insert(50, jeep);
        }
        install(host.lua(), &world).unwrap();
        (host, world)
    }

    #[test]
    fn scripts_read_and_change_the_world() {
        let (host, world) = setup();
        let (x, name, alive): (f64, String, bool) = host
            .lua()
            .load(
                r#"
                local hero = Player.GetLocalCharacter()
                local x, y, z = Object.GetPosition(hero)
                local jeep = Pg.GetGuidByName("_global_veh_jeep 0x0001")
                Object.SetPosition(jeep, x + 1, y, z)
                Debug.Printf("jeep ", "moved")
                return x, Object.GetName(jeep), Object.IsAlive(jeep) and Object.HasLabel(jeep, "Vehicle")
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(
            (x, name.as_str(), alive),
            (10.0, "_global_veh_jeep 0x0001", true)
        );
        let mut w = world.borrow_mut();
        assert_eq!(
            w.take_commands(),
            [Command::Move {
                guid: 50,
                position: [11.0, 0.0, 5.0]
            }]
        );
        assert_eq!(w.printed, ["jeep moved"]);
    }

    #[test]
    fn event_callbacks_change_the_world() {
        let (host, world) = setup();
        let events = crate::events::Events::default();
        events.install(host.lua()).unwrap();
        host.lua()
            .load(
                r#"
                local hero = Player.GetLocalCharacter()
                Event.Create(Event.ObjectProximity, {hero, 50, "<", 5}, function()
                    Object.Kill(50)
                end)
                "#,
            )
            .exec()
            .unwrap();
        assert!(events.update_shared(0.1, &world).is_empty());
        assert!(world.borrow().objects[&50].alive);
        world.borrow_mut().objects.get_mut(&1).unwrap().position = [1.0, 0.0, 1.0];
        assert!(events.update_shared(0.1, &world).is_empty());
        assert!(!world.borrow().objects[&50].alive);
    }

    #[test]
    fn asserts_log_and_switches_are_off() {
        let (host, world) = setup();
        let n: f64 = host
            .lua()
            .load(
                r#"
                ASSERT(true, "fine")
                ASSERT(false, "Unable to load layer")
                Sys.SetAssetRequestMax(Sys.GetAssetRequestMax() + 8)
                Pg.ContractActivated()
                assert(Sys.GetSkipMission() == nil and #Sys.GetIgnoreLayers() == 0)
                return Sys.GetAssetRequestMax()
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(n, 8.0);
        let mut w = world.borrow_mut();
        assert_eq!(w.printed, ["ASSERT failed: Unable to load layer"]);
        assert_eq!(
            w.take_commands(),
            [Command::Notify("Pg.ContractActivated".into())]
        );
    }

    #[test]
    fn spawns_get_guids_and_goals_call_back() {
        let (host, world) = setup();
        host.lua()
            .load(
                r#"
                done = nil
                local g = Pg.Spawn("Veyron", 1, 2, 3, 0.5, false, true)
                Ai.Goal({ AIGuid = g, Goal = "MoveTo", Location = { 4, 5, 6 }, Priority = "hiPri",
                          Force = true, Callback = function(a, b) done = a .. b end,
                          CallbackData = { "arrived", "!" } })
                spawned = g
                "#,
            )
            .exec()
            .unwrap();
        let g: u32 = host.lua().globals().get("spawned").unwrap();
        assert!(g > FIRST_SPAWNED);
        assert_eq!(world.borrow().objects[&g].position, [1.0, 2.0, 3.0]);
        let commands = world.borrow_mut().take_commands();
        assert_eq!(commands.len(), 2);
        assert_eq!(
            commands[1],
            Command::Goal {
                guid: g,
                goal: "MoveTo".into(),
                location: Some([4.0, 5.0, 6.0]),
                target: None
            }
        );
        assert!(goal_finished(&world, g).is_empty());
        let done: String = host.lua().globals().get("done").unwrap();
        assert_eq!(done, "arrived!");
    }

    #[test]
    fn single_player_answers() {
        let (host, _) = setup();
        let (server, client, second): (bool, bool, Value) = host
            .lua()
            .load("return Net.IsServer(), Net.IsClient(), Player.GetSecondaryCharacter()")
            .eval()
            .unwrap();
        assert!(server && !client && second.is_nil());
    }
}
