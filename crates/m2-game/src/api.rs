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
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use mlua::{Function, Lua, MultiValue, Table, Value};

use crate::events::World;
use crate::script::safe_function;

/// Guids handed to objects scripts spawn start here, above any layer's
/// entity ids.
pub const FIRST_SPAWNED: u32 = 0x4000_0000;
/// What crews a vehicle spawned with its driver.
pub const PILOT: &str = "Pilot";
/// Guids of object templates start here.
pub const FIRST_TEMPLATE: u32 = 0x3000_0000;

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
    /// Labels from the data (its template's and its own), as name hashes.
    pub label_hashes: Vec<u32>,
    /// The vehicle it rides in.
    pub vehicle: Option<u32>,
    /// It got in (or out) at once, put there by a script, rather than
    /// climbing in itself: seat events tell the two apart.
    pub seated_instantly: bool,
    pub player_controlled: bool,
    /// Simulated near the player rather than hibernating.
    pub awake: bool,
    /// Fuel for the player's support (`Player.GetFuel`), in the units of
    /// `Player.GetFuelCapacity`.
    pub fuel: f32,
}

impl Object {
    /// Its labels as name hashes: those scripts gave it and the data's.
    pub fn all_labels(&self) -> Vec<u32> {
        self.labels
            .iter()
            .map(|l| m2_assets::name_hash(l))
            .chain(self.label_hashes.iter().copied())
            .collect()
    }

    pub fn new(name: &str, position: [f32; 3]) -> Self {
        Self {
            fuel: 100.0,
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
    /// Seat a character in a vehicle (seat as the scripts name it, `"d"`
    /// for the driver); `instant` skips the climb-in.
    EnterVehicle {
        guid: u32,
        vehicle: u32,
        seat: String,
        instant: bool,
    },
    /// Hook cargo onto a carrier's winch, or let it go (`carrier` `None`).
    Winch {
        cargo: u32,
        carrier: Option<u32>,
    },
    /// Stream a layer in or out; report it done with
    /// [`crate::layers::finished`].
    Layer {
        op: crate::layers::LayerOp,
        name: String,
    },
    /// The scripts asked for a game state (`"WaitForStreaming"`): the
    /// game goes into it, and sets [`ScriptWorld::game_state`] back to
    /// `"Game"` when it is done (the world has streamed in).
    GameState(String),
    /// A call the game only needs to hear about: `"Pg.SaveGame"`,
    /// `"Pg.ContractActivated"`, `"Pg.ContractCompleted"`, ...
    Notify(String),
    /// The scripts gave the player a support item free
    /// (`MrxSupportData.AddFreebie`): its key (`"VzaCon01_SatBomb"`), how
    /// many, and the most she may hold, if they said.
    Freebie {
        key: String,
        quantity: u32,
        limit: Option<u32>,
    },
    /// The scripts took a free support item back
    /// (`MrxSupportData.RemoveFreebie`), by its key.
    RemoveFreebie(String),
    /// A scripted voice line (`VO.Cue`): `cue` names it
    /// (`"Fiona-In-Mission-Contract-Vz01-146"`), `speaker` says it
    /// (`None`: over the radio). Report its length with
    /// [`crate::mission::Mission::line_length`].
    Say {
        id: u32,
        speaker: Option<u32>,
        cue: String,
    },
    /// Stop line `id` (`VO.Cancel`).
    Hush(u32),
    /// `guid` can (no longer) be hurt: on while the scripts hold any
    /// reason for it (`"MrxState"` while the screen fades, `"Fanfare"`).
    Invincible {
        guid: u32,
        on: bool,
    },
    /// Play a music cue (`mu_fac_pmc_explore_01`), looped unless it is a
    /// one-shot, in place of what plays; `None` stops the music.
    Music(Option<String>),
    /// Play a sound cue by name (`Sound.CueSound`):
    /// `"ui_HUD_Objective_New"`; `at` the object it sounds from (`None`:
    /// flat, as the interface's sounds).
    CueSound {
        at: Option<u32>,
        cue: String,
    },
    /// Stop a sound cue started with [`Command::CueSound`]
    /// (`Sound.StopSound`): the scripts name it by where it sounds from
    /// and its cue, never by a handle.
    StopSound {
        at: Option<u32>,
        cue: String,
    },
    /// The player can't be killed while on (`Player.SetHealthClamp`): her
    /// health stops short of nothing, for the survival system to step in.
    HealthClamp(bool),
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
    /// The game state (`Sys.RequestGameState`), `"Game"` while playing.
    pub game_state: String,
    /// Layers the archives hold and which are loaded.
    pub layers: crate::layers::LayerState,
    /// Goal callbacks waiting for the AI to finish: guid, callback, data.
    goal_callbacks: Vec<(u32, Function, Option<Table>)>,
    /// The outlines of region objects (`LineRegion`), by the region's guid;
    /// placed at the object's position and yaw.
    pub regions: HashMap<u32, m2_assets::Region>,
    /// What `Pg.GetAllLandingZones(kind)` answers, by kind: the zones by
    /// their number. The transit script reads kinds 1 and 2 (where each
    /// player lands) as tables keyed alike.
    pub landing_zones: HashMap<i64, BTreeMap<i64, u32>>,
    /// Cargo on winches: what carries each, `None` once let go.
    pub winch: HashMap<u32, Option<u32>>,
    /// What the scripts want on the player's screen.
    pub hud: crate::hud::Hud,
    /// Object templates scripts have named (`"Supply Drop (GL)"`), by
    /// name in lower case.
    templates: HashMap<String, u32>,
    /// Scripted voice lines playing.
    pub vo: crate::vo::Lines,
    /// The score.
    pub music: crate::music::Music,
    /// Why each object is invincible (`Object.SetInvincible` reasons).
    pub invincible: HashMap<u32, std::collections::BTreeSet<String>>,
    /// The play area's boundaries (`Player.AddBoundary`: story-stage
    /// regions such as `BoundaryIntro`), the callback told when the player
    /// leaves or comes back, and whether the player is out.
    pub boundaries: Vec<u32>,
    pub boundary_callback: Option<Function>,
    pub outside: bool,
    /// Templates' labels (name hashes) by name in lower case, for spawns.
    pub template_labels: HashMap<String, Vec<u32>>,
    /// `Player.SetHealthClamp` and `Player.SetSurvivalMode`, as last set.
    pub health_clamp: bool,
    pub survival_mode: bool,
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
            .filter(|(_, o)| o.name.eq_ignore_ascii_case(name))
            .map(|(&g, _)| g)
            .min()
    }

    /// Takes the commands issued since the last call.
    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.commands)
    }

    /// The template `name` names, made on first use: an object scripts
    /// can copy with `Pg.Spawn`, but not in the world.
    fn template(&mut self, name: &str) -> u32 {
        let key = name.to_ascii_lowercase();
        if let Some(&g) = self.templates.get(&key) {
            return g;
        }
        let g = FIRST_TEMPLATE + self.templates.len() as u32;
        self.templates.insert(key, g);
        let mut o = Object::new(name, [0.0; 3]);
        o.awake = false;
        self.insert(g, o);
        g
    }

    /// Spawns a copy of `what` and tells the game to. A crewed vehicle
    /// (`"Mi26 (VZA Intro) (Driver)"`) comes with its pilot in the
    /// driver's seat, as scripts expect of `Vehicle.GetDriver`.
    fn spawn(&mut self, what: String, position: [f32; 3], yaw: f32) -> u32 {
        let crewed = what.to_ascii_lowercase().ends_with("(driver)");
        let guid = self.spawn_guid();
        let mut object = Object::new(&what, position);
        object.yaw = yaw;
        if let Some(l) = self.template_labels.get(&what.to_ascii_lowercase()) {
            object.label_hashes.clone_from(l);
        }
        self.insert(guid, object);
        self.commands.push(Command::Spawn {
            guid,
            what,
            position,
            yaw,
        });
        if crewed {
            let pilot = self.spawn_guid();
            let mut o = Object::new(PILOT, position);
            o.vehicle = Some(guid);
            o.seated_instantly = true;
            self.insert(pilot, o);
            self.commands.push(Command::Spawn {
                guid: pilot,
                what: PILOT.into(),
                position,
                yaw,
            });
            self.commands.push(Command::EnterVehicle {
                guid: pilot,
                vehicle: guid,
                seat: "d".into(),
                instant: true,
            });
        }
        guid
    }

    /// The point `distance` in front of an object (the player when `None`)
    /// and `up` above it; the player is where the camera looks from.
    fn ahead(&self, guid: Option<u32>, distance: f32, up: f32) -> Option<[f32; 3]> {
        let o = self.objects.get(&guid.or(self.player)?)?;
        let (s, c) = o.yaw.sin_cos();
        Some([
            o.position[0] + s * distance,
            o.position[1] + up,
            o.position[2] + c * distance,
        ])
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
    fn seated_instantly(&self, guid: u32) -> bool {
        self.objects.get(&guid).is_some_and(|o| o.seated_instantly)
    }
    fn labels(&self, guid: u32) -> Vec<u32> {
        self.objects
            .get(&guid)
            .map_or_else(Vec::new, Object::all_labels)
    }
    fn health(&self, guid: u32) -> Option<f32> {
        self.objects.get(&guid).map(|o| o.health)
    }
    fn inside(&self, guid: u32, boundary: u32) -> Option<bool> {
        let region = self.regions.get(&boundary)?;
        let at = self.objects.get(&boundary)?;
        // A rider is where its vehicle is.
        let o = self.objects.get(&guid)?;
        let p = o
            .vehicle
            .and_then(|v| self.objects.get(&v))
            .map_or(o.position, |v| v.position);
        Some(region.contains([at.position[0], at.position[2]], at.yaw, p[0], p[2]))
    }
    fn winch(&self, cargo: u32) -> Option<Option<u32>> {
        self.winch.get(&cargo).copied()
    }
    fn labelled(&self, label: &str) -> Vec<u32> {
        let mut out: Vec<u32> = self
            .objects
            .iter()
            .filter(|(_, o)| crate::events::class_matches(label, &o.all_labels()))
            .map(|(&g, _)| g)
            .collect();
        out.sort_unstable();
        out
    }

    fn game_state(&self) -> Option<String> {
        Some(self.game_state.clone())
    }

    fn named(&self, name: &str) -> Option<u32> {
        self.by_name(name)
    }
}

/// The world shared between the game and the script libraries.
pub type Shared = Rc<RefCell<ScriptWorld>>;

/// Reports that `guid` reached `at` (its AI goal's location): it, its
/// vehicle and any cargo on that vehicle's winch are there, the cargo is
/// let go (a delivery) and moved there in play too (a [`Command::Move`]:
/// the game may have put it down where it was made, before the winch took
/// it), and the goal's callbacks run. Returns any errors.
pub fn arrived(world: &Shared, guid: u32, at: [f32; 3]) -> Vec<mlua::Error> {
    {
        let mut w = world.borrow_mut();
        let carrier = w.vehicle_of(guid).unwrap_or(guid);
        let cargo: Vec<u32> = w
            .winch
            .iter()
            .filter(|(_, c)| **c == Some(carrier))
            .map(|(&g, _)| g)
            .collect();
        let riders: Vec<u32> = w
            .objects
            .iter()
            .filter(|(_, o)| o.vehicle == Some(carrier))
            .map(|(&g, _)| g)
            .collect();
        for g in [carrier]
            .into_iter()
            .chain(riders)
            .chain(cargo.iter().copied())
        {
            if let Some(o) = w.objects.get_mut(&g) {
                o.position = at;
            }
        }
        for g in cargo {
            w.winch.insert(g, None);
            w.commands.push(Command::Move {
                guid: g,
                position: at,
            });
        }
    }
    goal_finished(world, guid)
}

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
    crate::events::as_guid(v?)
}

fn num(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Integer(i) => Some(*i as f32),
        Value::Number(n) => Some(*n as f32),
        _ => None,
    }
}

/// Whether a name no object has is an object template's: templates are
/// named in words (`"Supply Drop (GL)"`, `"AMX30"`), layer objects in
/// snake case (`vza001_gate`). A guess from the names scripts use.
fn is_template(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase()) && !name.contains('_')
}

fn guid_value(g: Option<u32>) -> Value {
    g.map_or(Value::Nil, crate::events::guid_value)
}

fn values<I: IntoIterator<Item = Value>>(v: I) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_iter(v))
}

/// The table of the game's own functions that hear the scripts' free
/// support items (`AddFreebie`, `RemoveFreebie`).
pub const SUPPORT_EARS: &str = "__m2_support";

/// Installs the libraries over `world`. Functions not written here stay
/// missing (or stubs, with `ScriptHost::stub_missing_globals`).
/// The engine's libraries (the rest of the globals scripts use are their
/// own modules); see `ScriptHost::complete_libraries`.
pub const LIBRARIES: [&str; 27] = [
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
    "Table",
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
            Some(Value::String(s)) => {
                let name = s.to_string_lossy();
                w.by_name(&name)
                    .or_else(|| is_template(&name).then(|| w.template(&name)))
            }
            _ => None,
        };
        values([guid_value(g)])
    })?;
    // A copy of a template (a name or guid) in front of a character, as
    // supply drops make their cargo: template, distance, then flags.
    add(&pg, "SpawnFromCamera", |w, _, a| {
        let what = match a.first() {
            Some(Value::String(s)) => s.to_string_lossy(),
            v => match id(v).and_then(|g| w.objects.get(&g)) {
                Some(o) => o.name.clone(),
                None => return values([Value::Nil]),
            },
        };
        let by = a.get(4).and_then(|v| id(Some(v)));
        let Some(at) = w.ahead(by, num(a.get(1)).unwrap_or(10.0), 0.0) else {
            return values([Value::Nil]);
        };
        let yaw = w
            .objects
            .get(&by.or(w.player).unwrap_or(0))
            .map_or(0.0, |o| o.yaw);
        values([crate::events::guid_value(w.spawn(what, at, yaw))])
    })?;
    // The point a distance in front of the camera and a height above it.
    add(&pg, "FindPointFromCamera", |w, _, a| {
        let at = w.ahead(
            None,
            num(a.first()).unwrap_or(0.0),
            num(a.get(1)).unwrap_or(0.0),
        );
        values(match at {
            Some(p) => p.map(|c| Value::Number(f64::from(c))).to_vec(),
            None => vec![Value::Nil],
        })
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
        values([crate::events::guid_value(w.spawn(what, position, yaw))])
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
    add(&object, "SetInvincible", |w, _, a| {
        let Some(guid) = id(a.first()) else {
            return values([]);
        };
        let on = matches!(a.get(1), Some(Value::Boolean(true)));
        let reason = match a.get(2) {
            Some(Value::String(r)) => r.to_string_lossy(),
            _ => String::new(),
        };
        let reasons = w.invincible.entry(guid).or_default();
        let was = !reasons.is_empty();
        if on {
            reasons.insert(reason);
        } else {
            reasons.remove(&reason);
        }
        let now = !reasons.is_empty();
        if now != was {
            w.commands.push(Command::Invincible { guid, on: now });
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
    // The number of the player controlling it (scripts pass it on to
    // `Player.IsLocal` and key tables by it), else nil.
    add(&object, "IsPlayerControlled", |w, _, a| {
        let o = id(a.first()).and_then(|g| w.objects.get(&g));
        values([if o.is_some_and(|o| o.player_controlled) {
            Value::Integer(1)
        } else {
            Value::Nil
        }])
    })?;
    add(&object, "HasLabel", |w, _, a| {
        let has = match (id(a.first()).and_then(|g| w.objects.get(&g)), a.get(1)) {
            (Some(o), Some(Value::String(l))) => o
                .all_labels()
                .contains(&m2_assets::name_hash(&l.to_string_lossy())),
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
            let h = m2_assets::name_hash(&l);
            o.label_hashes.retain(|&x| x != h);
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
    // Winches: cargo hangs under a helicopter until it is let go.
    add(&object, "AttachCargoToWinch", |w, _, a| {
        if let (Some(cargo), Some(carrier)) = (id(a.first()), id(a.get(1))) {
            w.winch.insert(cargo, Some(carrier));
            w.commands.push(Command::Winch {
                cargo,
                carrier: Some(carrier),
            });
        }
        values([])
    })?;
    add(&object, "DetachCargoFromWinch", |w, _, a| {
        if let Some(cargo) = id(a.first())
            && w.winch.get(&cargo).is_some_and(Option::is_some)
        {
            w.winch.insert(cargo, None);
            w.commands.push(Command::Winch {
                cargo,
                carrier: None,
            });
        }
        values([])
    })?;
    // Hanging from a winch, not yet let go: a delivery's objective waits
    // for its cargo to be put down, not merely brought over the place.
    add(&object, "IsWinched", |w, _, a| {
        values([Value::Boolean(
            id(a.first()).is_some_and(|g| w.winch.get(&g).is_some_and(Option::is_some)),
        )])
    })?;
    // How far the winch is let out: the game's to animate.
    add(&object, "SetWinchState", |_, _, _| values([]))?;
    // No string tables yet: an object's own name.
    add(&object, "GetLocalizedName", |w, lua, a| {
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
    add(&player, "AddBoundary", |w, _, a| {
        if let Some(b) = id(a.get(1))
            && !w.boundaries.contains(&b)
        {
            w.boundaries.push(b);
        }
        values([])
    })?;
    // (player number, on): the hero's survival script turns the clamp on
    // at set-up and off once its one rescue has cooled down.
    add(&player, "SetHealthClamp", |w, _, a| {
        let on = matches!(a.get(1), Some(Value::Boolean(true)));
        if on != w.health_clamp {
            w.health_clamp = on;
            w.commands.push(Command::HealthClamp(on));
        }
        values([])
    })?;
    add(&player, "SetSurvivalMode", |w, _, a| {
        w.survival_mode = matches!(a.get(1), Some(Value::Boolean(true)));
        values([])
    })?;
    add(&player, "SetBoundaryCallback", |w, _, a| {
        w.boundary_callback = match a.get(1) {
            Some(Value::Function(f)) => Some(f.clone()),
            _ => None,
        };
        values([])
    })?;
    // A new game starts at the usual marker.
    add(&player, "GetPlayerStart", |_, lua, _| {
        values([Value::String(lua.create_string("PlayerLocation_Start")?)])
    })?;
    // The PMC's fuel tank: invented, the game's figure is not decoded.
    add(&player, "GetFuelCapacity", |_, _, _| {
        values([Value::Number(100.0)])
    })?;
    add(&player, "GetFuel", |w, _, _| {
        let fuel = w
            .player
            .and_then(|p| w.objects.get(&p))
            .map_or(0.0, |o| o.fuel);
        values([Value::Number(f64::from(fuel))])
    })?;
    add(&player, "SetFuel", |w, _, a| {
        if let (Some(p), Some(f)) = (w.player, num(a.first()))
            && let Some(o) = w.objects.get_mut(&p)
        {
            o.fuel = f;
        }
        values([])
    })?;
    // One player slot; the game creates the player itself.
    add(&player, "GetMaximumPlayers", |_, _, _| {
        values([Value::Integer(1)])
    })?;
    add(&player, "CreatePlayer", |w, _, a| {
        let first = matches!(num(a.first()), Some(n) if n == 0.0);
        values([guid_value(w.player.filter(|_| first))])
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
            t.raw_push(crate::events::guid_value(p))?;
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
            t.raw_push(crate::events::guid_value(r))?;
        }
        values([Value::Table(t)])
    })?;
    // Riders change seats at once here, so seat events see it on the next
    // update; the command lets the game animate it.
    add(&vehicle, "Exit", |w, _, a| {
        if let Some(guid) = id(a.first()) {
            if let Some(o) = w.objects.get_mut(&guid) {
                o.vehicle = None;
                o.seated_instantly = false;
            }
            w.commands.push(Command::ExitVehicle(guid));
        }
        values([])
    })?;
    // Vehicle.Enter(vehicle, character, seat, instant).
    add(&vehicle, "Enter", |w, _, a| {
        let (Some(vehicle), Some(guid)) = (id(a.first()), id(a.get(1))) else {
            return values([Value::Boolean(false)]);
        };
        let seat = match a.get(2) {
            Some(Value::String(s)) => s.to_string_lossy(),
            _ => String::new(),
        };
        let instant = matches!(a.get(3), Some(Value::Boolean(true)));
        if let Some(o) = w.objects.get_mut(&guid) {
            o.vehicle = Some(vehicle);
            o.seated_instantly = instant;
        }
        w.commands.push(Command::EnterVehicle {
            guid,
            vehicle,
            seat,
            instant,
        });
        values([Value::Boolean(true)])
    })?;

    // The sound library's version: the bootstrap picks named reverb
    // presets from version 10.
    let sound = lib("Sound")?;
    add(&sound, "_GetLibVersion", |_, _, _| {
        values([Value::Integer(10)])
    })?;
    // Where the sound banks are, relative to the install (`data\Audios`
    // holds `music.pws` and the rest), and the language of the voice
    // banks (`vo_stream.english.pws`).
    add(&sound, "GetAudioDir", |_, lua, _| {
        values([Value::String(lua.create_string(r"data\Audios")?)])
    })?;
    let gui = lib("Gui")?;
    add(&gui, "GetLanguageName", |_, lua, _| {
        values([Value::String(lua.create_string("english")?)])
    })?;

    let ai = lib("Ai")?;
    // Faction relations are not modelled yet: everyone is neutral (0 on
    // the scale the faction manager converts to attitude levels).
    add(&ai, "GetRelation", |_, _, _| values([Value::Number(0.0)]))?;
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

    // A pilot flies its vehicle, with any cargo on the winch, to a point
    // and lets the cargo go there: pilot, x, y, z, then how it flies.
    add(&ai, "Deliver", |w, _, a| {
        let Some(guid) = id(a.first()) else {
            return values([]);
        };
        let location = match (num(a.get(1)), num(a.get(2)), num(a.get(3))) {
            (Some(x), Some(y), Some(z)) => Some([x, y, z]),
            _ => None,
        };
        w.commands.push(Command::Goal {
            guid,
            goal: "Deliver".into(),
            location,
            target: None,
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
    add(&pg, "GetAllLandingZones", |w, lua, a| {
        let kind = num(a.first()).map_or(1, |k| k as i64);
        let t = lua.create_table()?;
        for (&n, &g) in w.landing_zones.get(&kind).into_iter().flatten() {
            t.raw_set(n, crate::events::guid_value(g))?;
        }
        values([Value::Table(t)])
    })?;
    add(&pg, "LoadIsRetry", |_, _, _| {
        values([Value::Boolean(false)])
    })?;
    // No saves yet: every load is a new game, which the bootstrap starts
    // itself (`LoadSingleton(nil)`) when this answers false.
    add(&pg, "LoadGame", |w, _, _| {
        w.commands.push(Command::Notify("Pg.LoadGame".into()));
        values([Value::Boolean(false)])
    })?;

    // The game's ears on the scripts' support items: `MrxSupportData`'s
    // freebie calls are wrapped to call these as well
    // (`Mission::start`), as (key, quantity, player, flag, limit) and
    // (key).
    let support = lib(SUPPORT_EARS)?;
    add(&support, "AddFreebie", |w, _, a| {
        if let Some(Value::String(key)) = a.first() {
            w.commands.push(Command::Freebie {
                key: key.to_string_lossy(),
                quantity: num(a.get(1)).map_or(1, |q| q.max(0.0) as u32),
                limit: num(a.get(4)).map(|l| l.max(0.0) as u32),
            });
        }
        values([])
    })?;
    add(&support, "RemoveFreebie", |w, _, a| {
        if let Some(Value::String(key)) = a.first() {
            w.commands
                .push(Command::RemoveFreebie(key.to_string_lossy()));
        }
        values([])
    })?;

    let sound = lib("Sound")?;
    add(&sound, "CueSound", |w, _, a| {
        if let Some(Value::String(cue)) = a.get(1) {
            w.commands.push(Command::CueSound {
                at: id(a.first()).filter(|&g| g != 0),
                cue: cue.to_string_lossy(),
            });
        }
        values([])
    })?;

    add(&sound, "StopSound", |w, _, a| {
        if let Some(Value::String(cue)) = a.get(1) {
            w.commands.push(Command::StopSound {
                at: id(a.first()).filter(|&g| g != 0),
                cue: cue.to_string_lossy(),
            });
        }
        values([])
    })?;

    // The score (see `music`): factions, bindings and states.
    add(&sound, "SetFactionMusic", |w, _, a| {
        if let Some(Value::String(f)) = a.first() {
            w.music.set_faction(&f.to_string_lossy());
        }
        values([])
    })?;
    add(&sound, "SetRootFactionRegionMusic", |w, _, a| {
        if let Some(Value::String(f)) = a.first() {
            w.music.set_root(&f.to_string_lossy());
        }
        values([])
    })?;
    add(&sound, "ActivateFactionRegionMusic", |w, _, _| {
        w.music.activate_region();
        values([])
    })?;
    add(&sound, "BindMusicCue", |w, _, a| {
        if let (Some(Value::String(cue)), Some(Value::String(state))) = (a.first(), a.get(1)) {
            w.music
                .bind(&cue.to_string_lossy(), &state.to_string_lossy());
        }
        values([])
    })?;
    add(&sound, "ClearMusicCues", |w, _, a| {
        if let Some(Value::String(state)) = a.first() {
            w.music.clear(&state.to_string_lossy());
        }
        values([])
    })?;
    add(&sound, "TransitionMusic", |w, _, a| {
        if let Some(Value::String(state)) = a.first()
            && let Some(c) = w.music.transition(&state.to_string_lossy())
        {
            w.commands.push(c);
        }
        values([])
    })?;
    add(&sound, "LockFactionMusic", |w, _, a| {
        w.music.locked = matches!(a.first(), Some(Value::Boolean(true)));
        values([])
    })?;
    add(&sound, "IsFactionLockedMusic", |w, _, _| {
        values([Value::Boolean(w.music.locked)])
    })?;
    add(&sound, "SetDynamicMusic", |w, _, a| {
        w.music.dynamic = matches!(a.first(), Some(Value::Boolean(true)));
        values([])
    })?;
    add(&sound, "IsDynamicMusic", |w, _, _| {
        values([Value::Boolean(w.music.dynamic)])
    })?;
    // Set-up and the action level the score is not driven by yet.
    for name in [
        "AddFactionMusic",
        "AddMusicState",
        "AddMusicTransition",
        "SetActionThresholdsMusic",
        "SetActionLevelsMusic",
        "LockActionLevelMusic",
    ] {
        add(&sound, name, |_, _, _| values([]))?;
    }

    // Scripted voice lines, as (speaker, cue, callback, values, priority,
    // send over the network); speaker 0 is the radio. The sequences that
    // cue them (`MrxVoSequence`) do their own ordering by priority, so
    // `AddSequence`/`RemoveSequence` need do nothing here.
    let vo = lib("VO")?;
    add(&vo, "Cue", |w, _, a| {
        let Some(Value::String(cue)) = a.get(1) else {
            return values([Value::Boolean(false)]);
        };
        let speaker = id(a.first()).filter(|&g| g != 0);
        let callback = match a.get(2) {
            Some(Value::Function(f)) => Some(f.clone()),
            _ => None,
        };
        let say = w.vo.cue(speaker, cue.to_string_lossy(), callback);
        w.commands.push(say);
        values([Value::Boolean(true)])
    })?;
    add(&vo, "Cancel", |w, _, a| {
        let speaker = id(a.first()).filter(|&g| g != 0);
        let cue = match a.get(1) {
            Some(Value::String(s)) => Some(s.to_string_lossy()),
            _ => None,
        };
        let hush = w.vo.cancel(speaker, cue.as_deref());
        w.commands.extend(hush);
        values([])
    })?;
    for name in ["AddSequence", "RemoveSequence"] {
        add(&vo, name, |_, _, _| values([]))?;
    }
    // Priorities the sequences copy at load and compare: a new sequence
    // whose number is no higher than the running one's cuts it off, so the
    // most important is lowest. Only the order is known, not the values.
    for (i, name) in [
        "PRIORITY_CINEMATIC",
        "PRIORITY_BRIEFING",
        "PRIORITY_SCRIPTED_CONTRACT",
        "PRIORITY_SCRIPTED_BOUNTIES",
        "PRIORITY_SCRIPTED_FREEPLAY",
    ]
    .into_iter()
    .enumerate()
    {
        vo.set(name, i as i64)?;
    }

    // Developer switches (skip to a mission, force a briefing): all off.
    let sys = lib("Sys")?;
    // The game shows its tutorial tips.
    add(&sys, "TutorialsEnabled", |_, _, _| {
        values([Value::Boolean(true)])
    })?;
    for name in ["GetSkipMission", "GetINIBriefing", "GetForceNewGame"] {
        add(&sys, name, |_, _, _| values([Value::Nil]))?;
    }
    add(&sys, "SetSkipMission", |_, _, _| values([]))?;
    // Guids print as their number.
    add(&sys, "StringToGuid", |_, _, a| {
        let g = match a.first() {
            Some(Value::String(s)) => {
                let text = s.to_string_lossy();
                let text = text.trim();
                match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => text.parse::<u32>().ok(),
                }
            }
            _ => None,
        };
        values([guid_value(g)])
    })?;
    // Scripts also call it bare.
    globals.raw_set("StringToGuid", sys.get::<Function>("StringToGuid")?)?;
    add(&sys, "GuidToString", |_, lua, a| {
        let text = id(a.first()).map_or_else(|| "nil".to_owned(), |g| g.to_string());
        values([Value::String(lua.create_string(&text)?)])
    })?;
    add(&sys, "RequestGameState", |w, _, a| {
        if let Some(Value::String(s)) = a.first() {
            let state = s.to_string_lossy();
            w.game_state = state.clone();
            w.commands.push(Command::GameState(state));
        }
        values([])
    })?;
    add(&sys, "GetLevelName", |_, lua, _| {
        values([Value::String(lua.create_string("vz")?)])
    })?;
    add(&sys, "GetIgnoreLayers", |_, lua, _| {
        values([Value::Table(lua.create_table()?)])
    })?;
    // The streaming budget the layer manager raises while it loads.
    {
        let mut w = world.borrow_mut();
        if w.asset_request_max == 0 {
            w.asset_request_max = 8;
        }
        if w.game_state.is_empty() {
            w.game_state = "Game".into();
        }
    }
    add(&sys, "GetAssetRequestMax", |w, _, _| {
        values([Value::Number(f64::from(w.asset_request_max))])
    })?;
    add(&sys, "SetAssetRequestMax", |w, _, a| {
        if let Some(n) = num(a.first()) {
            w.asset_request_max = n as u32;
        }
        values([])
    })?;

    // The engine's additions to the math library: random integers in
    // [a, b] (or [1, a]) and random reals in [a, b].
    lua.load(
        r#"
        local function randi(a, b)
            if b then return math.random(a, b) end
            return math.random(a)
        end
        local function randf(a, b)
            a, b = a or 0, b or 1
            return a + (b - a) * math.random()
        end
        math.randi, math.randf = randi, randf
        local M = rawget(_G, "Math")
        if type(M) ~= "table" then
            M = {}
            rawset(_G, "Math", M)
        end
        M.randi, M.randf = randi, randf
        -- Math is the math library with the additions.
        for k, v in pairs(math) do
            if rawget(M, k) == nil then M[k] = v end
        end
        "#,
    )
    .set_name("=m2-math")
    .exec()?;

    // The engine's table helpers: `Table.Create(narray, nhash)` makes a
    // table with room reserved, `Table.InsertI(t, value, i)` stores at
    // an index (MrxGuiBase's Widget:GetChildren builds its list so).
    lua.load(
        r#"
        local T = rawget(_G, "Table")
        if type(T) ~= "table" or getmetatable(T) ~= nil then
            T = {}
            rawset(_G, "Table", T)
        end
        function T.Create() return {} end
        function T.InsertI(t, value, i)
            if i == nil then
                t[#t + 1] = value
            else
                t[i] = value
            end
        end
        "#,
    )
    .set_name("=m2-table")
    .exec()?;

    // The menus' widget library, which `MrxGuiBase` builds every screen
    // with. Nothing is drawn yet: widgets get ids and keep their child
    // lists (the HUD walks them), other queries answer zero, and
    // everything else does nothing, so menus and the end of a contract
    // run.
    lua.load(
        r#"
        local n = 0
        local children = {}
        local G = {}
        function G.GetWidgetChildren(id)
            local list, copy = children[id] or {}, {}
            for i, c in ipairs(list) do copy[i] = c end
            return copy, #copy
        end
        function G.AddWidgetChild(parent, child)
            local list = children[parent]
            if list then list[#list + 1] = child end
        end
        function G.SetWidgetChild(parent, index, child)
            local list = children[parent]
            if not list then return end
            if type(index) == "number" and index >= 1 and index <= #list then
                list[index] = child
            else
                list[#list + 1] = child
            end
        end
        function G.RemoveWidgetChild(parent, child)
            local list = children[parent]
            if not list then return end
            for i = #list, 1, -1 do
                if list[i] == child then table.remove(list, i) end
            end
        end
        function G.RemoveAllWidgetChildren(parent)
            if children[parent] then children[parent] = {} end
        end
        function G.DeleteWidget(id)
            children[id] = nil
        end
        setmetatable(G, { __index = function(t, k)
            local f
            if type(k) ~= "string" then
                return nil
            elseif k:sub(1, 6) == "Create" then
                f = function() n = n + 1; children[n] = {}; return n end
            elseif k:sub(1, 3) == "Get" then
                f = function() return 0, 0, 0, 0 end
            elseif k:sub(1, 2) == "Is" then
                f = function() return false end
            else
                f = function() end
            end
            rawset(t, k, f)
            return f
        end })
        rawset(_G, "_GuiInternal", G)
        "#,
    )
    .set_name("=m2-gui")
    .exec()?;

    // Object filters: sets of objects scripts build (objects added or
    // excluded one by one, the players, a label) and hand to objectives
    // and events. Events read the same fields (see `events::objects`).
    lua.load(
        r#"
        local F = {}
        local meta = { __index = F }
        local function has(f, g)
            for _, v in ipairs(f.objs) do if v == g then return true end end
            return false
        end
        local function ok(f) return type(f) == "table" and getmetatable(f) == meta end
        function F.Create() return setmetatable({ objs = {}, ex = {}, players = false }, meta) end
        function F.UsePlayers(f, b) if ok(f) then f.players = b and true or false end end
        function F.SetFilter(f, label) if ok(f) then f.label = label end end
        function F.AddObject(f, g, exclude)
            if not ok(f) or g == nil then return end
            if exclude then f.ex[g] = true elseif not has(f, g) then f.objs[#f.objs + 1] = g end
        end
        function F.RemoveObject(f, g)
            if not ok(f) then return end
            for i, v in ipairs(f.objs) do
                if v == g then table.remove(f.objs, i) return end
            end
        end
        function F.GetObjects(f)
            local r = {}
            if not ok(f) then return r end
            for _, v in ipairs(f.objs) do if not f.ex[v] then r[#r + 1] = v end end
            if f.players then
                local p = Player.GetAnyCharacter()
                if p ~= nil and not has(f, p) and not f.ex[p] then r[#r + 1] = p end
            end
            return r
        end
        function F.GetCoopPlayerGuid(f)
            if not ok(f) then return nil end
            local p = Player.GetAnyCharacter()
            if p ~= nil and (f.players or has(f, p)) then return p end
            return nil
        end
        function F.Eval(f, g)
            if not ok(f) or g == nil or f.ex[g] then return false end
            if has(f, g) then return true end
            if f.players and g == Player.GetAnyCharacter() then return true end
            return f.label ~= nil and Object.HasLabel(g, f.label) or false
        end
        ObjectFilter = F
        "#,
    )
    .set_name("=m2-objectfilter")
    .exec()?;

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
    crate::hud::install(lua, world)?;
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
        assert_eq!(n, 16.0);
        let mut w = world.borrow_mut();
        assert_eq!(w.printed, ["ASSERT failed: Unable to load layer"]);
        assert_eq!(
            w.take_commands(),
            [Command::Notify("Pg.ContractActivated".into())]
        );
    }

    #[test]
    fn game_state_changes_fire_events() {
        let (host, world) = setup();
        let events = crate::events::Events::default();
        events.install(host.lua()).unwrap();
        host.lua()
            .load(
                r#"
                left = nil
                Sys.RequestGameState("WaitForStreaming")
                Event.Create(Event.GameStateChange, {"WaitForStreaming", "exit"},
                    function(state, name, how) left = state .. " " .. name .. " " .. how end,
                    {"streaming"})
                "#,
            )
            .exec()
            .unwrap();
        assert_eq!(
            world.borrow_mut().take_commands(),
            [Command::GameState("WaitForStreaming".into())]
        );
        events.update_shared(0.5, &world);
        let left: Option<String> = host.lua().globals().get("left").unwrap();
        assert_eq!(left, None);
        world.borrow_mut().game_state = "Game".into();
        events.update_shared(0.5, &world);
        let left: String = host.lua().globals().get("left").unwrap();
        assert_eq!(left, "streaming WaitForStreaming exit");
        let r: bool = host
            .lua()
            .load("local n = math.randi(3); return n >= 1 and n <= 3 and Math.randf(2, 2) == 2")
            .eval()
            .unwrap();
        assert!(r);
    }

    #[test]
    fn entering_a_named_vehicle_fires_seat_events() {
        let (host, world) = setup();
        let events = crate::events::Events::default();
        events.install(host.lua()).unwrap();
        host.lua()
            .load(
                r#"
                seated = nil
                Event.Create(Event.ObjectInSeat, {1, "_global_veh_jeep 0x0001", "d", "enter"},
                    function(tag, who, what)
                        seated = tag .. Sys.GuidToString(who) .. "/" .. Sys.GuidToString(what)
                    end, {"in:"})
                assert(Vehicle.Enter(50, 1, "d", true))
                "#,
            )
            .exec()
            .unwrap();
        assert!(events.update_shared(0.1, &world).is_empty());
        let seated: String = host.lua().globals().get("seated").unwrap();
        assert_eq!(seated, "in:1/50");
    }

    #[test]
    fn seat_events_read_mode_letters_and_classes() {
        let (host, world) = setup();
        world
            .borrow_mut()
            .objects
            .get_mut(&50)
            .unwrap()
            .labels
            .push("Car".into());
        let events = crate::events::Events::default();
        events.install(host.lua()).unwrap();
        host.lua()
            .load(
                r#"
                log = ""
                assert(Vehicle.Enter(50, 1, "d", false))
                local function note(t) return function() log = log .. t end end
                Event.Create(Event.ObjectInSeat, {1, "Car", "D", "E"}, note("car "))
                Event.Create(Event.ObjectInSeat, {1, "Car", "D", "I"}, note("put in "))
                Event.Create(Event.ObjectInSeat, {1, "Boat", "D", "E"}, note("boat "))
                Event.Create(Event.ObjectInSeat, {1, "Car && !APC", "D", "X"}, note("out "))
                "#,
            )
            .exec()
            .unwrap();
        assert!(events.update_shared(0.1, &world).is_empty());
        assert!(events.update_shared(0.1, &world).is_empty());
        let log: String = host.lua().globals().get("log").unwrap();
        assert_eq!(log, "car ");
        host.lua().load("Vehicle.Exit(1)").exec().unwrap();
        assert!(events.update_shared(0.1, &world).is_empty());
        let log: String = host.lua().globals().get("log").unwrap();
        assert_eq!(log, "car out ");
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
        let g =
            crate::events::as_guid(&host.lua().globals().get::<Value>("spawned").unwrap()).unwrap();
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
    fn entering_a_named_region_fires_boundary_events() {
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            w.insert(70, Object::new("region_vza_beach", [100.0, 0.0, 100.0]));
            let corners = vec![[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0]];
            w.regions.insert(
                70,
                m2_assets::Region {
                    flags: 0x0004_0002,
                    unk_0x04: 0,
                    corners,
                },
            );
        }
        let events = crate::events::Events::default();
        events.install(host.lua()).unwrap();
        host.lua()
            .load(
                r#"
                arrived = nil
                local f = ObjectFilter.Create()
                ObjectFilter.UsePlayers(f, true)
                Event.Create(Event.Boundary, {f, "region_vza_beach", "enter", true},
                    function(who, where)
                        arrived = Sys.GuidToString(who) .. "@" .. Sys.GuidToString(where)
                    end)
                "#,
            )
            .exec()
            .unwrap();
        assert!(events.update_shared(0.1, &world).is_empty());
        let arrived: Option<String> = host.lua().globals().get("arrived").unwrap();
        assert_eq!(arrived, None);
        world.borrow_mut().objects.get_mut(&1).unwrap().position = [103.0, 9.0, 98.0];
        assert!(events.update_shared(0.1, &world).is_empty());
        let arrived: String = host.lua().globals().get("arrived").unwrap();
        assert_eq!(arrived, "1@70");
    }

    #[test]
    fn destroy_targets_each_report_their_death() {
        let (host, world) = setup();
        for g in [60, 61] {
            world
                .borrow_mut()
                .insert(g, Object::new(&format!("vza_beachguard_{g}"), [0.0; 3]));
        }
        let events = crate::events::Events::default();
        events.install(host.lua()).unwrap();
        host.lua()
            .load(
                r#"
                dead = {}
                local f = ObjectFilter.Create()
                ObjectFilter.AddObject(f, Pg.GetGuidByName("vza_beachguard_60"))
                ObjectFilter.AddObject(f, Pg.GetGuidByName("vza_beachguard_61"))
                Event.CreatePersistent(Event.ObjectDeath, {f}, function(g)
                    dead[#dead + 1] = Sys.GuidToString(g)
                end)
                Object.Kill(StringToGuid("60"))
                "#,
            )
            .exec()
            .unwrap();
        events.update_shared(0.1, &world);
        host.lua()
            .load("Object.Kill(Sys.StringToGuid('61'))")
            .exec()
            .unwrap();
        events.update_shared(0.1, &world);
        events.update_shared(0.1, &world);
        let dead: Vec<String> = host.lua().globals().get("dead").unwrap();
        assert_eq!(dead, ["60", "61"]);
    }

    #[test]
    fn supply_drops_spawn_templates_in_front_of_the_player() {
        let (host, world) = setup();
        let (x, y, z, same, name, gate): (f64, f64, f64, bool, String, bool) = host
            .lua()
            .load(
                r#"
                local crate = Pg.GetGuidByName("Supply Drop (GL)")
                local cargo = Pg.SpawnFromCamera(crate, 4, 1, true, Player.GetLocalCharacter(), false, true)
                local x, y, z = Pg.FindPointFromCamera(10, 50, 10)
                return x, y, z, crate == Pg.GetGuidByName("Supply Drop (GL)"),
                    Object.GetName(cargo), Pg.GetGuidByName("vza001_gate") == nil
                "#,
            )
            .eval()
            .unwrap();
        // The hero faces +z (yaw 0) from (10, 0, 5).
        assert_eq!((x, y, z), (10.0, 50.0, 15.0));
        assert!(same && gate);
        assert_eq!(name, "Supply Drop (GL)");
        let commands = world.borrow_mut().take_commands();
        assert!(matches!(
            &commands[..],
            [Command::Spawn { what, position, .. }]
                if what == "Supply Drop (GL)" && *position == [10.0, 0.0, 9.0]
        ));
    }

    #[test]
    fn copters_deliver_winched_cargo() {
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            w.insert(80, Object::new("copter", [0.0, 100.0, 0.0]));
            let mut pilot = Object::new("pilot", [0.0, 100.0, 0.0]);
            pilot.vehicle = Some(80);
            w.insert(81, pilot);
            w.insert(82, Object::new("crate", [0.0, 95.0, 0.0]));
        }
        let events = crate::events::Events::default();
        events.install(host.lua()).unwrap();
        host.lua()
            .load(
                r#"
                dropped = nil
                local copter, crate = Sys.StringToGuid("80"), Sys.StringToGuid("82")
                Object.AttachCargoToWinch(crate, copter)
                Ai.Deliver(Vehicle.GetDriver(copter) or Sys.StringToGuid("81"), 50, 0, 60, 0.5)
                Event.Create(Event.ObjectWinched, {crate, copter, "Detach"}, function(c)
                    dropped = Sys.GuidToString(c)
                    Object.DetachCargoFromWinch(c, copter)
                end)
                "#,
            )
            .exec()
            .unwrap();
        events.update_shared(0.1, &world);
        let dropped: Option<String> = host.lua().globals().get("dropped").unwrap();
        assert_eq!(dropped, None);
        let commands = world.borrow_mut().take_commands();
        assert!(matches!(
            commands.last(),
            Some(Command::Goal { guid: 81, location: Some(at), .. }) if *at == [50.0, 0.0, 60.0]
        ));
        let winched = |host: &ScriptHost| -> bool {
            host.lua()
                .load(r#"return Object.IsWinched(Sys.StringToGuid("82"))"#)
                .eval()
                .unwrap()
        };
        // Hanging until it is let go over the place.
        assert!(winched(&host));
        assert!(arrived(&world, 81, [50.0, 0.0, 60.0]).is_empty());
        assert!(!winched(&host));
        assert_eq!(world.borrow().objects[&82].position, [50.0, 0.0, 60.0]);
        // Moved in play too, where the game may have put it down already.
        let commands = world.borrow_mut().take_commands();
        assert!(commands.iter().any(|c| matches!(
            c,
            Command::Move { guid: 82, position } if *position == [50.0, 0.0, 60.0]
        )));
        events.update_shared(0.1, &world);
        let dropped: String = host.lua().globals().get("dropped").unwrap();
        assert_eq!(dropped, "82");
    }

    #[test]
    fn menus_build_widgets_and_math_has_the_basics() {
        let (host, _) = setup();
        let (a, b, w, m, kids, left, guid): (i64, i64, f64, f64, i64, i64, String) = host
            .lua()
            .load(
                r#"
                local a = _GuiInternal.CreateWidget("x")
                local b = _GuiInternal.CreateFlashWidget("y")
                _GuiInternal.SetWidgetVisible(a, true)
                local w = _GuiInternal.GetWidgetViewport(a)
                _GuiInternal.AddWidgetChild(a, b)
                _GuiInternal.AddWidgetChild(a, _GuiInternal.CreateWidget("z"))
                local list, count = _GuiInternal.GetWidgetChildren(a)
                assert(list[1] == b and #list == count)
                local t = Table.Create(count, 0)
                for k, id in ipairs(list) do Table.InsertI(t, id * 10, k) end
                assert(t[1] == 20 and #t == 2)
                _GuiInternal.RemoveWidgetChild(a, b)
                local _, left = _GuiInternal.GetWidgetChildren(a)
                return a, b, w, Math.min(3, 2), count, left,
                    Sys.GuidToString(Sys.StringToGuid("0x10"))
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!((a, b, w, m, kids, left), (1, 2, 0.0, 2.0, 2, 1));
        assert_eq!(guid, "16");
    }

    #[test]
    fn crewed_vehicles_spawn_with_a_pilot_and_the_player_has_fuel() {
        let (host, _) = setup();
        let (pilot, riders, fuel, slots, version): (String, i64, f64, i64, i64) = host
            .lua()
            .load(
                r#"
                local copter = Pg.Spawn("Mi26 (VZA Intro) (Driver)", 0, 50, 0, 0)
                local jeep = Pg.Spawn("M151 Softtop (VZ)", 5, 0, 0, 0)
                assert(Vehicle.GetDriver(jeep) == nil)
                assert(Sound.GetAudioDir() .. "\\vo_x." .. Gui.GetLanguageName() == "data\\Audios\\vo_x.english")
                Player.SetFuel(40)
                return Object.GetName(Vehicle.GetDriver(copter)), #Vehicle.GetRiders(copter),
                    Player.GetFuel(), Player.GetMaximumPlayers(), Sound._GetLibVersion()
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(
            (pilot.as_str(), riders, fuel, slots, version),
            (PILOT, 1, 40.0, 1, 10)
        );
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
