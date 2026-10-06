//! A mission session: the game's scripts, read from the player's install,
//! running over a world built from its entity layers, as the headless
//! `mission` example and m2-alpha both run them.
//!
//! [`GameData::read`] collects the scripts, the layers' named entities,
//! the region shapes and the English strings from `vz.wad`. [`Mission::new`] loads every module
//! and entry script and sets the modules up; [`Mission::start`] then starts
//! play as the engine does (the bootstrap's `Init`, which starts a new game,
//! then its "GUI loaded" step), and [`Mission::tick`] advances it. Layer
//! requests and AI deliveries finish at once; everything the scripts ask
//! the game to do is also kept for the game in [`Mission::take_commands`],
//! and a readable account of the session in [`Mission::take_log`].

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek};
use std::rc::Rc;

use m2_assets::layer::{Component, LAYER_TYPE, TEMPLATE_DB_TYPE};
use m2_assets::lua::{Chunk, SCRIPT_TYPE};
use m2_assets::region::REGION_TYPE;
use m2_assets::strings::STRINGS_TYPE;
use m2_assets::{BlockContents, Layer, Region, StringTable, WadArchive, WadError, name_hash};
use mlua::Function;

use crate::api::{self, Command, Object, ScriptWorld, Shared};
use crate::events::Events;
use crate::hud::render;
use crate::layers::{LayerOp, LayerState, finished, layer_name};
use crate::library::{KNOWN_MODULES, LOADERS, Library};
use crate::script::{ScriptError, ScriptHost};

/// The script that starts the game (`Init`, then `LoadGame`).
pub const BOOTSTRAP: u32 = 0xb442_0059;
/// Activates the player's character as the engine does its object
/// scripts: the hero module's `OnActivate` (healing, the survival
/// system's one rescue, her inventory), which goes on once she is awake.
const HERO_ACTIVATE: &str = "Hero.OnActivate(Player.GetCharacter(1))";

/// The scripts' free support items (`MrxSupportData.AddFreebie`, as the
/// first contract gives its Satellite Guided Bomb) reach the game too:
/// each call goes on as it would, and the game hears of it
/// ([`Command::Freebie`], [`Command::RemoveFreebie`]).
const FOLLOW_FREEBIES: &str = r#"
local data, ears = MrxSupportData, __m2_support
if type(data) ~= "table" or type(data.AddFreebie) ~= "function" then
    return false
end
local add, remove = data.AddFreebie, data.RemoveFreebie
data.AddFreebie = function(...)
    ears.AddFreebie(...)
    return add(...)
end
data.RemoveFreebie = function(...)
    ears.RemoveFreebie(...)
    if type(remove) == "function" then return remove(...) end
end
return true
"#;

#[derive(Debug, thiserror::Error)]
pub enum MissionError {
    #[error("archive: {0}")]
    Wad(#[from] WadError),
    #[error("script: {0}")]
    Script(#[from] ScriptError),
    #[error("script: {0}")]
    Lua(#[from] mlua::Error),
}

/// A named entity of a layer.
#[derive(Debug, Clone)]
pub struct Entity {
    /// The layer's entity id, which scripts know it by.
    pub guid: u32,
    pub name: String,
    pub at: [f32; 3],
    pub yaw: f32,
    /// The outline of a region entity (`LineRegion`).
    pub region: Option<Region>,
    /// Its labels' name hashes: its template's and its own (`Label`).
    pub labels: Vec<u32>,
}

/// What a mission session needs from the archives.
#[derive(Debug, Default)]
pub struct GameData {
    pub library: Library,
    /// Named entities by layer (lower case).
    pub layers: BTreeMap<String, Vec<Entity>>,
    /// Region shapes read.
    pub shapes: usize,
    /// The game's English text by id, for the HUD's texts.
    pub strings: StringTable,
    /// Templates' labels (name hashes) by the template's name in lower
    /// case, for objects the scripts spawn.
    pub template_labels: HashMap<String, Vec<u32>>,
}

/// `Label`'s field: the label's name hash.
const LABEL_FIELD: u32 = 0xfd08_4cce;

/// Every `Label` record's name hash, by entity (or template).
fn labels_of(c: Option<&Component<'_>>) -> HashMap<u32, Vec<u32>> {
    let mut out: HashMap<u32, Vec<u32>> = HashMap::new();
    let Some(c) = c else {
        return out;
    };
    let at = c
        .fields
        .iter()
        .find(|f| f.name_hash == LABEL_FIELD)
        .map_or(0, |f| usize::from(f.offset));
    for (id, r) in c.records().unwrap_or_default() {
        if let Some(b) = r.get(at..at + 4) {
            out.entry(id)
                .or_default()
                .push(u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        }
    }
    out
}

/// The string table the HUD's texts come from.
pub const LANGUAGE: &str = "english";

impl GameData {
    /// Reads the scripts, layers, region shapes and the English string
    /// table of an archive (`vz.wad`). Blocks that don't parse are
    /// skipped.
    pub fn read<R: Read + Seek>(wad: &mut WadArchive<R>) -> Result<Self, MissionError> {
        let paths: Vec<String> = wad
            .index()
            .blocks()
            .iter()
            .map(|b| b.path.clone())
            .collect();
        let mut data = Self::default();
        let mut shapes: HashMap<u32, Region> = HashMap::new();
        // Entities with the shape hash their LineRegion names.
        let mut regions: Vec<(String, usize, u32)> = Vec::new();
        // Entities' templates, to give them their templates' labels.
        let mut templated: Vec<(String, usize, u32)> = Vec::new();
        let mut template_labels: HashMap<u32, Vec<u32>> = HashMap::new();
        let language = name_hash(LANGUAGE);
        for (block, path) in paths.iter().enumerate() {
            let decoded = wad.read_block(block)?;
            let Ok(contents) = BlockContents::parse(&decoded) else {
                continue;
            };
            for (entry, asset) in contents.iter() {
                if entry.type_hash == SCRIPT_TYPE {
                    if let Ok(c) = Chunk::from_asset(asset) {
                        data.library.add(entry.name_hash, c);
                    }
                } else if entry.type_hash == REGION_TYPE {
                    if let Ok(r) = Region::parse(asset) {
                        shapes.insert(entry.name_hash, r);
                    }
                } else if entry.type_hash == STRINGS_TYPE && entry.name_hash == language {
                    if let Ok(t) = StringTable::parse(asset) {
                        data.strings = t;
                    }
                } else if entry.type_hash == TEMPLATE_DB_TYPE
                    && let Ok(db) = Layer::parse(asset)
                {
                    template_labels = labels_of(db.component("Label"));
                    for (id, name) in db.names() {
                        if let Some(l) = template_labels.get(&id) {
                            data.template_labels
                                .insert(name.to_ascii_lowercase(), l.clone());
                        }
                    }
                } else if entry.type_hash == LAYER_TYPE
                    && let Ok(layer) = Layer::parse(asset)
                {
                    let name = layer_name(path);
                    let list = data.layers.entry(name.clone()).or_default();
                    let lines = layer.hashes("LineRegion");
                    let placements = layer.placements();
                    let mut own = labels_of(layer.component("Label"));
                    for (guid, entity) in layer.names() {
                        let p = placements.get(&guid);
                        if let Some(&h) = lines.get(&guid) {
                            regions.push((name.clone(), list.len(), h));
                        }
                        if let Some(p) = p {
                            templated.push((name.clone(), list.len(), p.parent));
                        }
                        list.push(Entity {
                            guid,
                            name: entity.to_owned(),
                            at: p.map_or([0.0; 3], |p| p.position),
                            yaw: p.map_or(0.0, |p| yaw(p.rotation)),
                            region: None,
                            labels: own.remove(&guid).unwrap_or_default(),
                        });
                    }
                }
            }
        }
        for (layer, i, template) in templated {
            if let Some(l) = template_labels.get(&template) {
                let e = &mut data.layers.get_mut(&layer).expect("indexed above")[i];
                e.labels.extend(l.iter().copied());
            }
        }
        for (layer, i, hash) in regions {
            if let Some(r) = shapes.get(&hash) {
                data.layers.get_mut(&layer).expect("indexed above")[i].region = Some(r.clone());
            }
        }
        data.shapes = shapes.len();
        Ok(data)
    }

    /// The transit landing zones, by player (1, 2) and then zone number.
    pub fn landing_zones(&self) -> [BTreeMap<i64, u32>; 2] {
        let mut out: [BTreeMap<i64, u32>; 2] = Default::default();
        for e in self.layers.values().flatten() {
            if let Some((n, player)) = landing_zone(&e.name) {
                out[player - 1].insert(n, e.guid);
            }
        }
        out
    }
}

/// A transit landing zone's number and player (1 or 2), from its name:
/// `06_gua_upperclass_lz_playerone` is zone 6 for player 1 (one is
/// spelled `_llz_`).
pub fn landing_zone(name: &str) -> Option<(i64, usize)> {
    let lower = name.to_ascii_lowercase();
    let (rest, player) = if let Some(r) = lower.strip_suffix("z_playerone") {
        (r, 1)
    } else {
        (lower.strip_suffix("z_playertwo")?, 2)
    };
    let rest = rest
        .strip_suffix('l')?
        .trim_end_matches('l')
        .strip_suffix('_')?;
    let n = rest.get(..2)?.parse().ok()?;
    rest[2..].starts_with('_').then_some((n, player))
}

/// The turn about the vertical axis of an (x, y, z, w) rotation.
fn yaw([x, y, z, w]: [f32; 4]) -> f32 {
    (2.0 * (w * y + x * z)).atan2(1.0 - 2.0 * (x * x + y * y))
}

/// A running mission session.
#[derive(Debug)]
pub struct Mission {
    host: ScriptHost,
    events: Events,
    world: Shared,
    layers: BTreeMap<String, Vec<Entity>>,
    strings: StringTable,
    /// Modules loaded before play started.
    started: usize,
    /// Game time, seconds.
    now: f64,
    /// When the current `WaitFor...` game state was asked for.
    waiting_since: Option<f64>,
    log: Vec<String>,
    outbox: Vec<Command>,
    /// The game flies deliveries ([`fly_deliveries`](Self::fly_deliveries)).
    flown: bool,
}

impl Mission {
    /// Loads every module and entry script, with the player `hero`
    /// (`"Mattias"`, `"Jennifer"` or `"Chris"`) standing at `at`, and sets
    /// the modules up. Failures to load or set up are logged.
    pub fn new(data: GameData, hero: &str, at: [f32; 3]) -> Result<Self, MissionError> {
        let world: Shared = Rc::new(RefCell::new(ScriptWorld::default()));
        let [one, two] = data.landing_zones();
        let GameData {
            mut library,
            layers,
            shapes,
            strings,
            template_labels,
        } = data;
        let regions = layers
            .values()
            .flatten()
            .filter(|e| e.region.is_some())
            .count();
        let mut log = vec![format!(
            "{} scripts; {} layers; {shapes} region shapes; {regions} regions; {} landing zones; {} strings",
            library.scripts().len(),
            layers.len(),
            one.len(),
            strings.strings.len()
        )];
        {
            let mut w = world.borrow_mut();
            w.layers.known = layers.keys().cloned().collect();
            w.template_labels = template_labels;
            for e in layers.values().flatten() {
                w.entities.insert(e.guid, e.at);
                w.entity_names
                    .entry(e.name.to_ascii_lowercase())
                    .or_insert(e.guid);
            }
            w.landing_zones.insert(1, one);
            w.landing_zones.insert(2, two);
            let mut player = Object::new(hero, at);
            player.player_controlled = true;
            // Scripts tell the heroes apart by these labels.
            player.labels.push(hero.to_ascii_lowercase());
            w.insert(PLAYER, player);
            w.player = Some(PLAYER);
        }
        // The static layers are always in.
        for (name, entities) in &layers {
            if LayerState::is_static(name) {
                place(&world, entities, true);
                world.borrow_mut().layers.loaded.insert(name.clone());
            }
        }
        for (name, hash) in KNOWN_MODULES {
            library.pin(name, hash);
        }
        let library = Rc::new(library);

        let host = ScriptHost::new()?;
        host.stub_missing_globals()?;
        api::install(host.lua(), &world)?;
        host.complete_libraries(&api::LIBRARIES)?;
        let events = Events::default();
        events.install(host.lua())?;
        host.install_library(Rc::clone(&library), true)?;

        // Every module, by the name scripts spell it with, so code run
        // later can use any module as a global.
        let mut spelled: BTreeMap<String, String> = BTreeMap::new();
        for script in library.scripts() {
            for call in script.chunk.main.calls() {
                if let Some(name) = call
                    .first
                    .filter(|_| LOADERS.contains(&call.callee.as_str()))
                {
                    spelled.entry(name.to_ascii_lowercase()).or_insert(name);
                }
            }
        }
        let globals = host.lua().globals();
        let import: Function = globals.get("import")?;
        let mut failures = Vec::new();
        for name in spelled.values() {
            match import.call::<mlua::Value>(name.as_str()) {
                Ok(module) => globals.raw_set(name.as_str(), module)?,
                Err(e) => failures.push(format!("module {name}: {e}")),
            }
        }
        // Entry scripts' environments, as `Scripts.<hash>`.
        let modules: Vec<u32> = host.modules().iter().filter_map(|m| m.1).collect();
        let entries = host.lua().create_table()?;
        for script in library.scripts() {
            if modules.contains(&script.hash) {
                continue;
            }
            match host.run_script(&library, script.hash) {
                Ok(env) => entries.raw_set(format!("{:08x}", script.hash), env)?,
                Err(e) => failures.push(format!("script {:08x}: {e}", script.hash)),
            }
        }
        globals.raw_set("Scripts", entries)?;
        for f in host.init_modules()? {
            failures.push(format!("Init of {f}"));
        }
        let started = host.modules().len();
        log.push(format!(
            "started: {started} modules, {} failures",
            failures.len()
        ));
        log.extend(failures.iter().map(|f| format!("  {}", first_lines(f))));
        let mut mission = Self {
            host,
            events,
            world,
            layers,
            strings,
            started,
            now: 0.0,
            waiting_since: None,
            log,
            outbox: Vec::new(),
            flown: false,
        };
        mission.settle("set-up");
        Ok(mission)
    }

    /// Puts the layers whose names contain `part` (any case) in the world
    /// now, as if the scripts had loaded them; returns their names.
    pub fn preload(&mut self, part: &str) -> Vec<String> {
        let part = part.to_ascii_lowercase();
        let names: Vec<String> = self
            .layers
            .keys()
            .filter(|n| n.contains(&part))
            .cloned()
            .collect();
        for name in &names {
            place(&self.world, &self.layers[name], true);
            self.world.borrow_mut().layers.loaded.insert(name.clone());
        }
        names
    }

    /// The layers in the world now, lower case: blocks of their own
    /// (`vz_state_vzacon001`) and the layers inside `layers_static`
    /// (`vz_mar_village`), which the game loads by name.
    pub fn loaded_layers(&self) -> Vec<String> {
        self.world.borrow().layers.loaded.iter().cloned().collect()
    }

    /// The entities of a layer that is a block of its own (lower case).
    pub fn layer(&self, name: &str) -> Option<&[Entity]> {
        self.layers.get(name).map(Vec::as_slice)
    }

    /// The world as the scripts see it.
    pub fn world(&self) -> &Shared {
        &self.world
    }

    pub fn lua(&self) -> &mlua::Lua {
        self.host.lua()
    }

    /// Runs Lua (a test driver, say) with every module as a global, then
    /// carries out what it asked for.
    pub fn run(&mut self, code: &str, name: &str) -> Result<(), MissionError> {
        let result = self
            .host
            .lua()
            .load(code)
            .set_name(format!("={name}"))
            .exec();
        self.settle(name);
        Ok(result?)
    }

    /// Starts play as the engine does: intro movies skipped, the
    /// bootstrap's `Init` (which starts a new game when there is no save),
    /// then the step the engine runs once the GUI has loaded. The first
    /// contract's mission is then waiting at its start.
    pub fn start(&mut self) {
        self.follow_freebies();
        let steps = [
            (
                "skip mode",
                "MrxCheatBootstrap.IsSkipModeEnabled = function() return true end".to_owned(),
            ),
            ("bootstrap", format!("Scripts[\"{BOOTSTRAP:08x}\"].Init()")),
            // The callback MrxGuiBootstrap.SetOnGuiLoadedFunc registers.
            ("gui loaded", "MrxBootstrap._GuiLoaded()".to_owned()),
            ("hero activated", HERO_ACTIVATE.to_owned()),
        ];
        for (name, code) in steps {
            if let Err(e) = self.run(&code, name) {
                self.log
                    .push(format!("  {name}: {}", first_lines(&e.to_string())));
            }
        }
    }

    /// Has the scripts' free support items reach the game
    /// ([`FOLLOW_FREEBIES`]); logs whether they can.
    pub fn follow_freebies(&mut self) {
        let followed = self
            .host
            .lua()
            .load(FOLLOW_FREEBIES)
            .set_name("=follow freebies")
            .eval::<bool>();
        self.log.push(match followed {
            Ok(true) => "free support items: followed (MrxSupportData.AddFreebie)".to_owned(),
            Ok(false) => "free support items: no MrxSupportData.AddFreebie to follow".to_owned(),
            Err(e) => format!("free support items: {}", first_lines(&e.to_string())),
        });
    }

    /// Advances game time by `dt` seconds: fires the events that are due
    /// and carries out what their callbacks asked for.
    pub fn tick(&mut self, dt: f64) {
        self.now += dt;
        let now = self.now;
        // Streaming is instant here: a waiting state ends a second after
        // it was asked for, and the scripts hear that it did.
        {
            let mut w = self.world.borrow_mut();
            if w.game_state.starts_with("WaitFor") {
                let since = *self.waiting_since.get_or_insert(now);
                if now - since >= 1.0 {
                    self.log
                        .push(format!("t={now:.1} game state {} done", w.game_state));
                    w.game_state = "Game".into();
                    self.waiting_since = None;
                }
            }
        }
        for t in self.events.take_trace() {
            self.log.push(format!("  t={:.1} event {t}", self.now));
        }
        for e in self.events.update_shared(dt, &self.world) {
            self.log.push(format!(
                "  t={now:.1} {} callback: {}",
                e.kind,
                first_lines(&e.error.to_string())
            ));
        }
        self.check_boundary(now);
        let ended = crate::vo::update(&self.world, self.host.lua(), now);
        for (cue, error) in ended {
            self.log.push(match error {
                None => format!("  t={now:.1} line {cue} ended"),
                Some(e) => format!(
                    "  t={now:.1} line {cue} ended; callback: {}",
                    first_lines(&e.to_string())
                ),
            });
        }
        self.settle(&format!("t={now:.1}"));
    }

    /// Tells the boundary callback when the player leaves the play area
    /// (zone `"out"`, `"enter"`) or comes back (`"exit"`). The player is
    /// out when inside none of the boundaries the game can place.
    fn check_boundary(&mut self, now: f64) {
        let call = {
            let mut w = self.world.borrow_mut();
            let Some(f) = w.boundary_callback.clone() else {
                return;
            };
            // While a state change holds her (the black screen as a
            // contract loads, `MrxState`), she is still at the last spot
            // and about to be placed: not a real exit.
            if w.invincible
                .get(&PLAYER)
                .is_some_and(|r| r.contains("MrxState"))
            {
                return;
            }
            let found: Vec<bool> = w
                .boundaries
                .iter()
                .filter_map(|&b| crate::events::World::inside(&*w, PLAYER, b))
                .collect();
            let out = !found.is_empty() && !found.contains(&true);
            if out == w.outside {
                return;
            }
            w.outside = out;
            (f, if out { "enter" } else { "exit" })
        };
        let (f, event) = call;
        self.log
            .push(format!("  t={now:.1} play area: out {event}"));
        if let Err(e) = f.call::<()>((mlua::Value::Nil, "out", event)) {
            self.log.push(format!(
                "  t={now:.1} boundary callback: {}",
                first_lines(&e.to_string())
            ));
        }
    }

    /// The game is playing scripted line `id` (a [`Command::Say`]), which
    /// lasts `seconds`.
    pub fn line_length(&mut self, id: u32, seconds: f64) {
        self.world.borrow_mut().vo.set_length(id, seconds);
    }

    /// Game time so far.
    pub fn now(&self) -> f64 {
        self.now
    }

    /// Moves the player (and anything it rides in).
    pub fn put_player(&mut self, at: [f32; 3]) {
        let mut w = self.world.borrow_mut();
        let vehicle = w.objects.get(&PLAYER).and_then(|o| o.vehicle);
        for g in [Some(PLAYER), vehicle].into_iter().flatten() {
            if let Some(o) = w.objects.get_mut(&g) {
                o.position = at;
            }
        }
    }

    /// Puts the player in a vehicle (or out of one, with `None`): seat
    /// events fire on the next tick.
    pub fn ride(&mut self, vehicle: Option<u32>) {
        if let Some(o) = self.world.borrow_mut().objects.get_mut(&PLAYER)
            && o.vehicle != vehicle
        {
            o.vehicle = vehicle;
            o.seated_instantly = false;
        }
    }

    /// Reports where an object is in play (a vehicle that was driven or
    /// pushed about).
    pub fn place(&mut self, guid: u32, at: [f32; 3], yaw: f32) {
        if let Some(o) = self.world.borrow_mut().objects.get_mut(&guid) {
            o.position = at;
            o.yaw = yaw;
        }
    }

    /// Leaves deliveries to the game: an `Ai.Deliver` order stays a
    /// [`Command::Goal`] (`"Deliver"`) for it to fly, and it reports the
    /// arrival with [`arrived`](Self::arrived). Without this a delivery
    /// arrives as soon as it is ordered.
    pub fn fly_deliveries(&mut self) {
        self.flown = true;
    }

    /// Reports that `guid` (a delivery's pilot) reached `at`: its cargo is
    /// let go there and the scripts hear of it ([`api::arrived`]); what
    /// they do about it is carried out at once.
    pub fn arrived(&mut self, guid: u32, at: [f32; 3]) {
        self.log.push(format!(
            "t={:.1} delivery by {guid} arrived at {at:?}",
            self.now
        ));
        for e in api::arrived(&self.world, guid, at) {
            self.log
                .push(format!("  goal callback: {}", first_lines(&e.to_string())));
        }
        self.settle(&format!("t={:.1}, delivered", self.now));
    }

    /// Reports that an object died in play (a soldier the player shot).
    /// The game's reading of an object's health (the player's, as she is
    /// hurt and healed).
    pub fn set_health(&mut self, guid: u32, health: f32) {
        let was = self
            .world
            .borrow_mut()
            .objects
            .get_mut(&guid)
            .map(|o| std::mem::replace(&mut o.health, health));
        // A drop the game made (not the scripts' own healing): what the
        // scripts are watching for, while the health system is checked.
        if guid == PLAYER && was.is_some_and(|w| health < w - 5.0) && crate::script::stub_args() {
            self.log.push(format!(
                "  t={:.1} her health {:.1} -> {health:.1}; pending: {}",
                self.now,
                was.unwrap_or(0.0),
                self.events.describe().join(" ")
            ));
        }
    }

    /// The player died and is back on her feet: the hero module's
    /// `OnDeath` sets its survival system up again once she is healthy.
    pub fn player_died(&mut self) {
        self.log.push(format!("  t={:.1} player died", self.now));
        if let Err(e) = self.run("Hero.OnDeath(Player.GetCharacter(1))", "hero died") {
            self.log
                .push(format!("  hero died: {}", first_lines(&e.to_string())));
        }
    }

    pub fn kill(&mut self, guid: u32) {
        if let Some(o) = self.world.borrow_mut().objects.get_mut(&guid) {
            o.alive = false;
            o.health = 0.0;
        }
    }

    /// Everything the scripts asked the game to do since the last call.
    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.outbox)
    }

    /// A readable account of the session since the last call.
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }

    /// A HUD text with its ids replaced by the game's English strings
    /// (`[VzaCon001.Objectives.001]` as `Secure the beach`).
    pub fn text(&self, text: &str) -> String {
        render(text, &self.strings)
    }

    /// The game's English strings.
    pub fn strings(&self) -> &StringTable {
        &self.strings
    }

    /// Modules loaded since play started: name and script.
    pub fn late_modules(&self) -> Vec<(String, Option<u32>)> {
        self.host.modules().into_iter().skip(self.started).collect()
    }

    /// Pending events by kind.
    pub fn pending_events(&self) -> Vec<String> {
        self.events.kinds()
    }

    /// The names the scripts' pending `ScriptEvent` events wait for.
    pub fn awaited_posts(&self) -> Vec<String> {
        self.events.awaited()
    }

    /// Posts a script event as the engine does (`Event.Post(name)`): the
    /// scripts waiting for it hear of it at the next tick.
    pub fn post(&mut self, name: &str) {
        self.events.post(name);
    }

    /// Logs what happened, then carries out layer requests and deliveries
    /// (at once) and logs what their callbacks did, until nothing is left.
    fn settle(&mut self, when: &str) {
        let mut commands = self.report(when);
        for _ in 0..50 {
            // Modules a script loaded "in the background" are ready.
            let ran = match self.host.run_deferred() {
                Ok((ran, errors)) => {
                    for e in errors {
                        self.log
                            .push(format!("  dynamic_import callback: {}", first_lines(&e)));
                    }
                    ran
                }
                Err(e) => {
                    self.log.push(format!("  dynamic_import callbacks: {e}"));
                    0
                }
            };
            if ran > 0 {
                commands.extend(self.report(&format!("{when}, {ran} modules loaded")));
            }
            let mut delivered = 0;
            for c in &commands {
                if let Command::Goal {
                    guid,
                    goal,
                    location: Some(at),
                    ..
                } = c
                    && goal == "Deliver"
                    && !self.flown
                {
                    self.log
                        .push(format!("  delivery by {guid} arrived at {at:?}"));
                    for e in api::arrived(&self.world, *guid, *at) {
                        self.log
                            .push(format!("  goal callback: {}", first_lines(&e.to_string())));
                    }
                    delivered += 1;
                }
            }
            let requests: Vec<(LayerOp, String)> = commands
                .into_iter()
                .filter_map(|c| match c {
                    Command::Layer { op, name } => Some((op, name)),
                    _ => None,
                })
                .collect();
            if requests.is_empty() {
                if delivered > 0 {
                    commands = self.report(&format!("{when}, deliveries done"));
                    continue;
                }
                if ran == 0 {
                    return;
                }
                commands = Vec::new();
                continue;
            }
            for (op, name) in requests {
                let found = self.layers.get(&name);
                if let Some(entities) = found {
                    match op {
                        LayerOp::Load => place(&self.world, entities, true),
                        LayerOp::Unload => place(&self.world, entities, false),
                        LayerOp::Reload => {
                            place(&self.world, entities, false);
                            place(&self.world, entities, true);
                        }
                    }
                }
                let what = found.map_or("part of layers_static".to_owned(), |e| {
                    format!("{} entities", e.len())
                });
                self.log.push(format!("  layer {op:?} {name}: {what}"));
                for e in finished(&self.world, &name, true) {
                    self.log
                        .push(format!("  layer callback: {}", first_lines(&e.to_string())));
                }
            }
            commands = self.report(&format!("{when}, layers done"));
        }
        self.log
            .push("  requests still coming after 50 rounds; carrying on".into());
    }

    /// Logs what happened since the last report, if anything; returns the
    /// commands (also kept for the game).
    fn report(&mut self, when: &str) -> Vec<Command> {
        let calls = self.host.take_calls();
        let (commands, printed) = {
            let mut w = self.world.borrow_mut();
            (w.take_commands(), std::mem::take(&mut w.printed))
        };
        self.outbox.extend(commands.iter().cloned());
        if calls.is_empty() && commands.is_empty() && printed.is_empty() {
            return commands;
        }
        let log = &mut self.log;
        log.push(String::new());
        log.push(format!("{when} ({} events pending):", self.events.len()));
        log.extend(printed.iter().take(60).map(|p| format!("  print {p}")));
        log.extend(commands.iter().take(60).map(|c| format!("  command {c:?}")));
        // Repeated calls in a row collapse to one line with a count.
        let mut runs: Vec<(&str, usize)> = Vec::new();
        for c in &calls {
            match runs.last_mut() {
                Some((l, n)) if *l == c.as_str() => *n += 1,
                _ => runs.push((c.as_str(), 1)),
            }
        }
        // With arguments shown every call differs: keep them all.
        let cap = if crate::script::stub_args() {
            usize::MAX
        } else {
            120
        };
        for (c, n) in runs.iter().take(cap) {
            log.push(match n {
                1 => format!("  stub {c}"),
                _ => format!("  stub {c} x{n}"),
            });
        }
        if runs.len() > cap {
            log.push(format!("  ... {} stub calls in all", calls.len()));
        }
        commands
    }
}

/// The player's guid.
pub const PLAYER: u32 = 1;

/// Puts a layer's entities in the world as objects scripts can find by
/// name, or takes them out.
fn place(world: &Shared, entities: &[Entity], load: bool) {
    let mut w = world.borrow_mut();
    for e in entities {
        if load {
            let mut o = Object::new(&e.name, e.at);
            o.yaw = e.yaw;
            o.label_hashes.clone_from(&e.labels);
            w.insert(e.guid, o);
            if let Some(r) = &e.region {
                w.regions.insert(e.guid, r.clone());
            }
        } else {
            w.objects.remove(&e.guid);
            w.regions.remove(&e.guid);
        }
    }
}

/// The first lines of a (Lua) error, on one line.
pub fn first_lines(s: &str) -> String {
    s.lines().take(4).collect::<Vec<_>>().join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landing_zones_come_from_names() {
        assert_eq!(landing_zone("01_pmc_hq_lz_playerone"), Some((1, 1)));
        assert_eq!(landing_zone("30_chi_hqb_lz_PlayerTwo"), Some((30, 2)));
        assert_eq!(landing_zone("03_mar_airport_llz_playerone"), Some((3, 1)));
        assert_eq!(landing_zone("lz_helipad_playerone"), None);
        assert_eq!(landing_zone("06_gua_upperclass_playerone"), None);
    }

    #[test]
    fn the_scripts_free_support_items_reach_the_game() {
        let mut m = Mission::new(GameData::default(), "Mattias", [0.0; 3]).unwrap();
        m.follow_freebies();
        assert!(m.take_log().iter().any(|l| l.contains("no MrxSupportData")));
        m.run(
            r#"
            MrxSupportData = { tGiven = {} }
            function MrxSupportData.AddFreebie(sKey, nQty, uPlayer, bFlag, nLimit)
                local t = MrxSupportData.tGiven
                t[sKey] = math.min((t[sKey] or 0) + nQty, nLimit or 99)
            end
            function MrxSupportData.RemoveFreebie(sKey)
                MrxSupportData.tGiven[sKey] = nil
            end
            "#,
            "support data",
        )
        .unwrap();
        m.follow_freebies();
        m.run(
            r#"
            MrxSupportData.AddFreebie("VzaCon01_SatBomb", 1, nil, nil, 1)
            MrxSupportData.AddFreebie("VzaCon01_SatBomb", 1, nil, nil, 1)
            assert(MrxSupportData.tGiven.VzaCon01_SatBomb == 1)
            MrxSupportData.RemoveFreebie("VzaCon01_SatBomb")
            assert(MrxSupportData.tGiven.VzaCon01_SatBomb == nil)
            MrxSupportData.AddFreebie("Flare", 2)
            "#,
            "give",
        )
        .unwrap();
        let bomb = || Command::Freebie {
            key: "VzaCon01_SatBomb".into(),
            quantity: 1,
            limit: Some(1),
        };
        assert_eq!(
            m.take_commands(),
            [
                bomb(),
                bomb(),
                Command::RemoveFreebie("VzaCon01_SatBomb".into()),
                Command::Freebie {
                    key: "Flare".into(),
                    quantity: 2,
                    limit: None
                }
            ]
        );
    }

    #[test]
    fn a_conversation_moves_on_as_each_line_ends() {
        let mut m = Mission::new(GameData::default(), "Jennifer", [0.0; 3]).unwrap();
        m.run(
            r#"
            local function second(state) VO.Cue(0, "B", function() heard = state end) end
            VO.Cue(0, "A", second, {}, 1, true)
            VO.Cue(0, "C", function() cancelled = true end)
            VO.Cancel(0, "C", true)
            ordered = VO.PRIORITY_CINEMATIC < VO.PRIORITY_SCRIPTED_CONTRACT
            "#,
            "test",
        )
        .unwrap();
        let say = |id, cue: &str| Command::Say {
            id,
            speaker: None,
            cue: cue.into(),
        };
        assert_eq!(
            m.take_commands(),
            [say(1, "A"), say(2, "C"), Command::Hush(2)]
        );
        m.tick(0.1);
        m.line_length(1, 1.0);
        m.tick(0.5);
        assert!(m.take_commands().is_empty());
        m.tick(0.6);
        assert_eq!(m.take_commands(), [say(3, "B")]);
        for _ in 0..50 {
            m.tick(0.1);
        }
        let g = m.host.lua().globals();
        assert_eq!(g.raw_get::<String>("heard").unwrap(), "complete");
        assert_eq!(g.raw_get::<Option<bool>>("cancelled").unwrap(), None);
        assert!(g.raw_get::<bool>("ordered").unwrap());
    }

    #[test]
    fn invincible_while_any_reason_holds() {
        let mut m = Mission::new(GameData::default(), "Jennifer", [0.0; 3]).unwrap();
        m.run(
            r#"
            local hero = Player.GetCharacter(1)
            Object.SetInvincible(hero, true, "MrxState")
            Object.SetInvincible(hero, true, "Fanfare")
            Object.SetInvincible(hero, false, "MrxState")
            Object.SetInvincible(hero, false, "Fanfare")
            "#,
            "test",
        )
        .unwrap();
        let on = |on| Command::Invincible { guid: PLAYER, on };
        assert_eq!(m.take_commands(), [on(true), on(false)]);
    }

    #[test]
    fn health_watches_fire_as_she_is_hurt() {
        let mut m = Mission::new(GameData::default(), "Jennifer", [0.0; 3]).unwrap();
        m.run(
            r#"
            dropped = 0
            local hero = Player.GetCharacter(1)
            Player.SetHealthClamp(1, true)
            Event.Create(Event.ObjectHealth, { hero, "<", Object.GetHealth(hero) },
                function() dropped = dropped + 1 end)
            Sound.StopSound(0, "sfx_survival_lp")
            "#,
            "test",
        )
        .unwrap();
        assert_eq!(
            m.take_commands(),
            [
                Command::HealthClamp(true),
                Command::StopSound {
                    at: None,
                    cue: "sfx_survival_lp".into()
                }
            ]
        );
        m.tick(0.1);
        m.set_health(PLAYER, 60.0);
        m.tick(0.1);
        m.tick(0.1);
        let dropped: i64 = m.lua().globals().raw_get("dropped").unwrap();
        assert_eq!(dropped, 1);
    }

    #[test]
    fn play_reports_rides_and_places() {
        let mut m = Mission::new(GameData::default(), "Mattias", [0.0; 3]).unwrap();
        m.ride(Some(77));
        m.place(PLAYER, [1.0, 2.0, 3.0], 0.5);
        let w = m.world().borrow();
        let p = &w.objects[&PLAYER];
        assert_eq!(
            (p.vehicle, p.position, p.yaw),
            (Some(77), [1.0, 2.0, 3.0], 0.5)
        );
    }

    #[test]
    fn flown_deliveries_wait_for_the_game_to_say_they_arrived() {
        for flown in [false, true] {
            let mut m = Mission::new(GameData::default(), "Mattias", [0.0; 3]).unwrap();
            if flown {
                m.fly_deliveries();
            }
            {
                let mut w = m.world().borrow_mut();
                w.insert(80, Object::new("copter", [0.0, 100.0, 0.0]));
                let mut pilot = Object::new("pilot", [0.0, 100.0, 0.0]);
                pilot.vehicle = Some(80);
                w.insert(81, pilot);
                w.insert(82, Object::new("crate", [0.0, 95.0, 0.0]));
            }
            m.run(
                r#"
                local copter, crate = Sys.StringToGuid("80"), Sys.StringToGuid("82")
                Object.AttachCargoToWinch(crate, copter)
                Ai.Deliver(Sys.StringToGuid("81"), 50, 0, 60, 0.5)
                "#,
                "deliver",
            )
            .unwrap();
            let at = [50.0, 0.0, 60.0];
            let moved = |m: &mut Mission| {
                m.take_commands()
                    .iter()
                    .any(|c| matches!(c, Command::Move { guid: 82, position } if *position == at))
            };
            let there = |m: &Mission| m.world().borrow().objects[&82].position == at;
            // At once, unless the game flies it.
            assert_eq!(moved(&mut m), !flown);
            assert_eq!(there(&m), !flown);
            if flown {
                m.arrived(81, at);
                assert!(moved(&mut m));
                assert!(there(&m));
            }
        }
    }

    #[test]
    fn yaw_reads_turns_about_up() {
        let half = std::f32::consts::FRAC_PI_4;
        let q = [0.0, half.sin(), 0.0, half.cos()];
        assert!((yaw(q) - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    }
}
