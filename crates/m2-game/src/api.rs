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
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use m2_math::glam::Vec3;
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
/// How far off an object can be seen on screen, metres (inferred: about
/// as far as the game draws the scripts' vehicles; the engine's own draw
/// distance isn't read).
pub const VISIBLE_RANGE: f32 = 400.0;
/// The scripts put objects they don't want seen far under the island
/// (about 40 km down): those below this height are hidden.
const HIDDEN_BELOW: f32 = -1000.0;
/// An awake object goes to sleep this far beyond its hibernation distance
/// (metres), so one at the edge doesn't wake and sleep by turns
/// (inferred).
const HIBERNATION_MARGIN: f32 = 10.0;

/// Where the player's camera looks from and how wide it sees: the game
/// feeds it each tick, for `Event.ObjectIsVisible`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub eye: [f32; 3],
    /// The way it looks.
    pub forward: [f32; 3],
    /// The field of view from bottom to top, radians.
    pub fov_y: f32,
    /// The screen's width over its height.
    pub aspect: f32,
    /// How far off anything is drawn, metres ([`VISIBLE_RANGE`]).
    pub range: f32,
}

impl Camera {
    /// Whether `point` is on screen: in front of the camera, within its
    /// field of view's four sides and its range (`+y` up).
    pub fn sees(&self, point: [f32; 3]) -> bool {
        let forward = Vec3::from(self.forward).normalize_or_zero();
        let d = Vec3::from(point) - Vec3::from(self.eye);
        let depth = d.dot(forward);
        if forward == Vec3::ZERO || depth <= 0.0 || d.length() > self.range {
            return false;
        }
        let right = forward.cross(Vec3::Y).normalize_or(Vec3::X);
        let up = right.cross(forward);
        let tall = (self.fov_y * 0.5).tan();
        d.dot(up).abs() <= depth * tall && d.dot(right).abs() <= depth * tall * self.aspect
    }
}

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
    /// Simulated near the player rather than hibernating
    /// ([`ScriptWorld::update_hibernation`]).
    pub awake: bool,
    /// Hidden by the scripts (`Object.SetVisible(o, false)`): not on
    /// screen.
    pub hidden: bool,
    /// Fuel for the player's support (`Player.GetFuel`), in the units of
    /// `Player.GetFuelCapacity`.
    pub fuel: f32,
    /// The faction it belongs to (`Ai.GetFactionGuid`): the name hash of
    /// the faction's template (`"VZ"`), as the HUD's faction guids are.
    pub faction: Option<u32>,
}

impl Object {
    /// Whether it has every label of `terms` that isn't negated and none
    /// that is ([`crate::events::class_terms`]).
    pub fn is_of(&self, terms: &[(u32, bool)]) -> bool {
        terms.iter().all(|&(h, not)| {
            let has = self.label_hashes.contains(&h)
                || self.labels.iter().any(|l| m2_assets::name_hash(l) == h);
            has != not
        })
    }

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
    /// A hijack's own requests (`crate::hijack`).
    Hijack(crate::hijack::HijackCommand),
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
        /// Its `Haste`, if given (PmcCon001's garage smash gives 1).
        haste: Option<f32>,
    },
    ExitVehicle(u32),
    /// Open a gate or door (`Object.OpenGate`): PmcCon001's villa doors
    /// as the player comes near them.
    OpenGate(u32),
    /// Shut a gate or door again (`Object.CloseGate`).
    CloseGate(u32),
    /// Whether `guid` may take up its weapons (`Human.DisableWeapons` and
    /// `EnableWeapons`): the HQ's briefings put the starter's and the
    /// player's away, a teleport the hero's while she moves.
    Weapons {
        guid: u32,
        on: bool,
    },
    /// Whether vehicle `guid` may be got into (`Vehicle.Usable(vehicle,
    /// usable)`): story vehicles the scripts keep for themselves (JetCon001's
    /// scramble copter, ChiCon009's ZBD2000, a race's tank).
    Usable {
        guid: u32,
        on: bool,
    },
    /// The player's fuel goes up by this (`Player.AddFuel`: a fuel
    /// pickup's script), up to the tanks' capacity.
    AddFuel(f32),
    /// The soldiers of faction `faction` (its template's name: `OC`,
    /// `Allied`) pursue the player for `seconds`, or until the scripts
    /// clear it (`None`: `Pg.LockPursuit`, a custom pursuit of -1 s).
    Pursuit {
        faction: String,
        seconds: Option<f32>,
    },
    /// A locked or custom pursuit ends (`Pg.ClearPursuitLock`,
    /// `Pg.ClearCustomPursuit`).
    PursuitClear,
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
    /// Put an atmosphere region (`rgn_atmo_carmonaislandrain`) on its
    /// record named `setting` (`"night"`).
    Atmosphere {
        region: u32,
        setting: String,
    },
    /// Atmosphere changes from now on happen at once (true) or ease in
    /// over their records' time (false).
    AtmosphereImmediate(bool),
    /// The loading screen up (true) or down (`Net.SetLoadingScreen`): the
    /// teleport helpers and state changes put it up before a move; the
    /// engine takes it down once the place has loaded.
    LoadingScreen(bool),
    /// A call the game only needs to hear about:
    /// `"Pg.ContractActivated"`, `"Pg.ContractCompleted"`, ...
    Notify(String),
    /// Save the game now, in slot `slot`: the scripts' checkpoints
    /// (`Pg.SaveGame("retry")`), and their autosaves (`Sys.RequestAutosave`
    /// after a contract, `Pg.SaveGame("autosave")`) in slot [`AUTOSAVE`].
    Save {
        slot: String,
    },
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
    /// Fade sound category `category` (`non_ui`, `chatter`, `music`,
    /// `vo`) to `level` over `seconds` (`Sound.FadeCategoryDown`/`Up`:
    /// the world ducks under a radio conversation, and more in the
    /// survival rescue).
    FadeSound {
        category: String,
        level: f32,
        seconds: f32,
    },
    /// Play sound category `category` at `pitch` times its own, gliding
    /// over `seconds` (`Sound.PitchCategoryActivate`/`Deactivate`: the
    /// survival rescue slows the world).
    PitchSound {
        category: String,
        pitch: f32,
        seconds: f32,
    },
    /// The player can't be killed while on (`Player.SetHealthClamp`): her
    /// health stops short of nothing, for the survival system to step in.
    HealthClamp(bool),
    /// The player's controls on or off (`Player.SetInputEnabled`): the
    /// state system turns them off on entering a loading fade and back on
    /// once the screen has faded in.
    InputEnabled(bool),
    /// `who` now carries weapons of these templates, in order, each with
    /// nothing in reserve (`Human.Inventory.SetAllWeapons`: VzaCon001
    /// hands her a carbine, grenades and C4).
    Loadout {
        who: u32,
        weapons: Vec<String>,
    },
    /// A script set the reserve of `who`'s weapon of template `weapon`
    /// (`Weapon.SetReserveAmmo`).
    Reserve {
        who: u32,
        weapon: String,
        rounds: u32,
    },
    /// `who` let go of their weapon of template `weapon`
    /// (`Human.Inventory.DropWeapon`: Carmona, seated, is disarmed).
    DropWeapon {
        who: u32,
        weapon: String,
    },
    /// An aircraft of template `what` flies in level from `from` to `to`
    /// at `speed` m/s (`Airstrike.Flyby`); report it there with
    /// [`crate::mission::Mission::flyby_arrived`].
    Flyby {
        guid: u32,
        what: String,
        from: [f32; 3],
        to: [f32; 3],
        speed: f32,
    },
    /// Ordnance of template `what` (`"Cluster Bomb Projectile"`) sets off
    /// from `at`, aimed at `target` (an object, else the point) if it
    /// homes; report it going off with
    /// [`crate::mission::Mission::ordnance_burst`].
    Ordnance {
        guid: u32,
        what: String,
        at: [f32; 3],
        launch: crate::airstrike::Launch,
        fuse: crate::airstrike::Fuse,
        target: Option<(Option<u32>, [f32; 3])>,
        owner: Option<u32>,
    },
    /// An effect (`global_particle_airstrike_fuelairbomb`) at `at`,
    /// facing `direction` (`Airstrike.SpawnDirectedObject`).
    DirectedEffect {
        guid: u32,
        what: String,
        at: [f32; 3],
        direction: [f32; 3],
    },
    /// `who` takes up a designator (`Airstrike.EquipDesignator`: `kind`
    /// is its template, `"Smoke Designator"`), as weapon `weapon`; when
    /// she throws it, tell [`crate::mission::Mission::designator_thrown`].
    EquipDesignator {
        who: u32,
        weapon: u32,
        kind: String,
    },
    /// `who` puts the designator away (`Airstrike.RemoveDesignator`).
    RemoveDesignator {
        who: u32,
    },
    /// The camera takes the engine's named shot `shot`
    /// (`"OverTheShoulderLeft"`) from object `base` to `target`
    /// (`Camera.SetShot`), until the scene ends ([`Command::Cinematic`]).
    CameraShot {
        shot: String,
        base: u32,
        target: u32,
    },
    /// A scene of the scripts' begins (true) or ends
    /// (`Player.SetCinematicMode`): on its end the camera is hers again.
    Cinematic(bool),
}

/// An action the player can take beside an object (`Pg.AddContextAction`):
/// the HQ's exits and entrances, talking to a starter, the wardrobe.
///
/// From the scripts: `Pg.AddContextAction(guid, text, reach, 0, 0, 255,
/// reach, false)`; the HQ's exits pass 2 (twice), a starter's talk 4, the
/// wardrobe only its guid, text and 1. The third argument is taken as how
/// near (metres) she must stand: inferred, as the only distance-like value
/// every call gives, growing with the size of what is used; the rest are
/// not read. `Event.ContextAction` `{ 0, guid }` waits for her to take it.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextAction {
    /// The prompt's text id (`[ContextAction.Exit]`).
    pub text: String,
    /// Metres from the object within which it is offered.
    pub reach: f32,
}

/// Reach (metres) for a context action whose call gives none (invented).
pub const CONTEXT_REACH: f32 = 2.0;

/// The pursuit (`Pg.SetPursuit`, `LockPursuit`, `SetCustomPursuit`,
/// `GetPursuitState`): the faction manager raises its level by one each
/// time a faction at its lowest standing reports her (`IncrementPursuit`,
/// at most 3), and the story locks one for a scene (ChiCon003's Allied, a
/// VZ chase).
///
/// From the scripts: the calls and their arguments, the level read back as
/// `GetPursuitState().Level`, and the level times (`SetPursuitLevelTimes(120,
/// 300)`). Inferred: that a level below 3 lasts the first time and level 3
/// the second. Invented: what `SetPursuitSeconds` (5 s, the HUD gauge's own
/// fill time) does to it (nothing here), and the times when no script set
/// them.
#[derive(Debug, Default)]
pub struct Pursuit {
    /// Seconds a pursuit lasts below level 3 and at it, once set.
    pub times: Option<(f32, f32)>,
    /// The level under way (0: none).
    pub level: i64,
    /// The faction pursuing (its template's guid).
    pub faction: Option<u32>,
}

impl Pursuit {
    /// Seconds a pursuit of `level` lasts.
    pub fn seconds(&self, level: i64) -> f32 {
        let (low, high) = self.times.unwrap_or((120.0, 300.0));
        if level >= 3 { high } else { low }
    }
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
    /// Spawned since hibernation was last updated: they start in play, so
    /// the scripts' next look finds them awake wherever they are
    /// ([`Self::update_hibernation`]).
    fresh: Vec<u32>,
    /// What `Sys.GetAssetRequestMax` answers; scripts raise it while
    /// layers load and put it back.
    pub asset_request_max: u32,
    /// The game state (`Sys.RequestGameState`), `"Game"` while playing.
    pub game_state: String,
    /// The player's cash (`Player.GetCash`, `Player.SetCash`); the
    /// scripts' `MrxPmc.AddCashQty` reads it, adds and clamps.
    pub cash: f64,
    /// What the scripts set her fuel tanks to hold
    /// (`Player.SetFuelCapacity`), if they have.
    pub fuel_capacity: Option<f32>,
    /// Layers the archives hold and which are loaded.
    pub layers: crate::layers::LayerState,
    /// Goal callbacks waiting for the AI to finish: guid, callback, data.
    goal_callbacks: Vec<(u32, Function, Option<Table>)>,
    /// Drop and landing zone tests asked for (`Ai.TestDropZone`): the
    /// callback to answer and the spot tested.
    drop_tests: Vec<(Function, [f32; 3])>,
    /// Assets asked for with a callback (`Pg.LoadAsset(name, type,
    /// callback, data)`): answered as loaded on the next tick
    /// ([`answer_asset_loads`]).
    asset_loads: Vec<(Function, Value)>,
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
    /// The player has subtitles off (`Sys.SubtitlesEnabled`).
    pub subtitles_off: bool,
    /// The player has the tutorial tips off (`Sys.TutorialsEnabled`).
    pub tutorials_off: bool,
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
    /// Who drives each template's vehicle, by name in lower case, as its
    /// `Rider` in the driver's seat names them: a vehicle spawned without
    /// "(Driver)" in its name comes with them ([`ScriptWorld::spawn`]).
    pub template_drivers: HashMap<String, String>,
    /// Every layer entity's place, and its guid by name in lower case,
    /// loaded or not: `Pg.GetGuidByName` finds markers (landing zones) of
    /// layers that aren't in, and scripts measure to them.
    pub entities: HashMap<u32, [f32; 3]>,
    pub entity_names: HashMap<String, u32>,
    /// `Player.SetHealthClamp` and `Player.SetSurvivalMode`, as last set.
    pub health_clamp: bool,
    pub survival_mode: bool,
    /// The weapons people carry.
    pub inventory: crate::inventory::Inventory,
    /// Faction relations (`Ai.SetRelation`), -100 to 100, by (faction,
    /// faction) in lower case.
    pub relations: HashMap<(String, String), f32>,
    /// Actions playing and the pad, for hijacks.
    pub hijack: crate::hijack::HijackWorld,
    /// The player's latest presses of the GUI's named buttons, with
    /// their game time ([`crate::events::World::button_presses`]).
    pub buttons: Vec<(f64, String)>,
    /// The context actions on offer, by object ([`ContextAction`]).
    pub context_actions: std::collections::BTreeMap<u32, ContextAction>,
    /// The player's latest takings of a context action, by object, with
    /// their game time ([`crate::events::World::context_presses`]).
    pub context_presses: Vec<(f64, u32)>,
    /// The crews the layers' vehicles came into play with, by vehicle:
    /// (seat, rider template), as each entity's own `Rider` records say
    /// (the HQ interior's `HqInterior` seats Fiona, its PMC boss), else
    /// its template's.
    pub crews: HashMap<u32, Vec<(String, String)>>,
    /// The riders those crews came in as, each with what it came in on: a
    /// rider out of its seat is still known by it (the HQ's starter, out
    /// of her seat for her briefing).
    pub crew_riders: HashMap<u32, u32>,
    /// The player is in a scene of the scripts' (`Player.SetCinematicMode`).
    pub cinematic: bool,
    /// Characters swimming, as the game last said (`Human.IsSwimming`).
    pub swimming: HashSet<u32>,
    /// Witnesses reporting her, and what each faction has against her
    /// (`Report`, [`crate::report`]).
    pub reports: crate::report::Reports,
    /// The pursuit as the scripts set it ([`Pursuit`]).
    pub pursuit: Pursuit,
    /// Strikes and designators under way.
    pub strikes: crate::airstrike::Strikes,
    /// Whether vehicle disguises work (`Player.SetVehicleDisguise`): the
    /// mission flow turns them off for a new game and back on only once
    /// OilCon021 is done, so off until a script says otherwise.
    pub vehicle_disguise: bool,
    /// Her disguise while she drives another faction's vehicle with
    /// disguises on: how much is left (0..=100) and whether it holds.
    /// `None` out of one.
    pub disguise: Option<(f32, bool)>,
    /// The player's camera as the game last fed it; `None` until it has.
    pub camera: Option<Camera>,
    /// Objects the game found behind something solid from the camera at
    /// its last look (it looks at those `ObjectIsVisible` events wait
    /// for: [`crate::mission::Mission::sight_watched`]).
    pub unseen: HashSet<u32>,
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
            .filter(|(g, o)| {
                o.name.eq_ignore_ascii_case(name) && !self.inventory.carried.contains_key(g)
            })
            .map(|(&g, _)| g)
            .min()
    }

    /// Where an object is: in play, else where its layer places it.
    fn place_of(&self, guid: u32) -> Option<[f32; 3]> {
        self.objects
            .get(&guid)
            .map(|o| o.position)
            .or_else(|| self.entities.get(&guid).copied())
    }

    /// Takes the commands issued since the last call.
    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.commands)
    }

    /// The template `name` names, made on first use: an object scripts
    /// can copy with `Pg.Spawn`, but not in the world.
    pub(crate) fn template(&mut self, name: &str) -> u32 {
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
    /// driver's seat, as scripts expect of `Vehicle.GetDriver`; so does one
    /// whose template seats its own driver (the support helicopters,
    /// `"AH1Z (Ewan)"`: Ewan, `"Recruit Ewen Garret"`).
    pub(crate) fn spawn(&mut self, what: String, position: [f32; 3], yaw: f32) -> u32 {
        let lower = what.to_ascii_lowercase();
        let driver = if lower.ends_with("(driver)") {
            Some(PILOT.to_owned())
        } else {
            self.template_drivers.get(&lower).cloned()
        };
        let guid = self.spawn_guid();
        let mut object = Object::new(&what, position);
        object.yaw = yaw;
        if let Some(l) = self.template_labels.get(&what.to_ascii_lowercase()) {
            object.label_hashes.clone_from(l);
        }
        self.insert(guid, object);
        self.fresh.push(guid);
        self.commands.push(Command::Spawn {
            guid,
            what,
            position,
            yaw,
        });
        if let Some(driver) = driver {
            let pilot = self.spawn_guid();
            let mut o = Object::new(&driver, position);
            o.vehicle = Some(guid);
            o.seated_instantly = true;
            self.insert(pilot, o);
            self.commands.push(Command::Spawn {
                guid: pilot,
                what: driver,
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

    /// Gives `who` a weapon of each of `templates` (guids) in place of
    /// what they carried, each with nothing in reserve.
    fn arm(&mut self, who: u32, templates: &[u32]) {
        for w in self.inventory.weapons.remove(&who).unwrap_or_default() {
            self.inventory.carried.remove(&w);
            self.objects.remove(&w);
        }
        let at = self.objects.get(&who).map_or([0.0; 3], |o| o.position);
        let (mut list, mut names) = (Vec::new(), Vec::new());
        for &template in templates {
            let Some(name) = self.objects.get(&template).map(|o| o.name.clone()) else {
                continue;
            };
            let guid = self.spawn_guid();
            let mut o = Object::new(&name, at);
            if let Some(l) = self.template_labels.get(&name.to_ascii_lowercase()) {
                o.label_hashes.clone_from(l);
            }
            self.insert(guid, o);
            self.inventory.carried.insert(
                guid,
                crate::inventory::Carried {
                    owner: who,
                    template,
                    reserve: 0,
                },
            );
            list.push(guid);
            names.push(name);
        }
        self.inventory.weapons.insert(who, list);
        self.commands.push(Command::Loadout {
            who,
            weapons: names,
        });
    }

    /// The name of weapon `weapon`'s template.
    fn weapon_name(&self, weapon: u32) -> Option<String> {
        let c = self.inventory.carried.get(&weapon)?;
        self.objects.get(&c.template).map(|o| o.name.clone())
    }

    /// `who`'s first (or second) weapon that isn't thrown.
    fn held(&self, who: u32, nth: usize) -> Option<u32> {
        self.inventory
            .of(who)
            .iter()
            .copied()
            .filter(|&w| {
                self.weapon_name(w)
                    .is_some_and(|n| !self.inventory.thrown.contains(&n.to_ascii_lowercase()))
            })
            .nth(nth)
    }

    /// Sets `who`'s weapon of template `name` to `rounds` in reserve, as
    /// the game spends or picks up its ammunition.
    pub fn set_reserve(&mut self, who: u32, name: &str, rounds: u32) {
        let found = self.inventory.of(who).iter().copied().find(|&w| {
            self.weapon_name(w)
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        });
        if let Some(c) = found.and_then(|w| self.inventory.carried.get_mut(&w)) {
            c.reserve = rounds;
        }
    }

    pub(crate) fn spawn_guid(&mut self) -> u32 {
        self.next_spawn += 1;
        FIRST_SPAWNED + self.next_spawn
    }

    /// Wakes the objects the player has come within their hibernation
    /// distance of ([`HIBERNATION_DISTANCE`], every object's until the
    /// templates' own are read) and puts to sleep those she has left
    /// [`HIBERNATION_MARGIN`] beyond it: what `Object.IsAwake` and
    /// `Event.ObjectHibernation` read. The engine hibernates what is far
    /// from the player (inferred from the scripts: PmcCon001's gate
    /// opener waits for the villa's front door to sleep, then to wake as
    /// she comes back). She, what she rides, templates and carried
    /// weapons keep theirs. Objects of layers not loaded aren't in play
    /// at all, and never awake.
    ///
    /// What the scripts spawned since the last update is left awake this
    /// once: a spawn starts in play, and the supply drops wait for their
    /// helicopter to be awake (`Event.ObjectHibernation`) before they hang
    /// its cargo and send it off. The helicopters start at the contract's
    /// spawn points, which can lie beyond the hibernation distance
    /// (VzaCon001's weapon drop: about 190 m from where she lands, 172 m
    /// from the drop zone); in the real game's recording that helicopter
    /// comes 4 s after the objective is shown, so it isn't waiting for her
    /// to come near (inferred from the scripts and the footage).
    pub fn update_hibernation(&mut self) {
        let fresh = std::mem::take(&mut self.fresh);
        let Some(her) = self.player.and_then(|p| self.objects.get(&p)) else {
            return;
        };
        let (at, riding) = (Vec3::from(her.position), her.vehicle);
        let wake = HIBERNATION_DISTANCE as f32;
        for (g, o) in &mut self.objects {
            if Some(*g) == self.player
                || Some(*g) == riding
                || (FIRST_TEMPLATE..FIRST_SPAWNED).contains(g)
                || self.inventory.carried.contains_key(g)
                || fresh.contains(g)
            {
                continue;
            }
            let d = Vec3::from(o.position).distance(at);
            if o.awake && d > wake + HIBERNATION_MARGIN {
                o.awake = false;
            } else if !o.awake && d <= wake {
                o.awake = true;
            }
        }
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
    /// On screen: in play, not hidden (nor put under the island), its
    /// place (a rider's, its vehicle's) inside the camera's view, and not
    /// found behind something solid at the game's last look.
    fn is_visible(&self, guid: u32) -> Option<bool> {
        let camera = self.camera?;
        let Some(o) = self.objects.get(&guid) else {
            return Some(false);
        };
        let at = o
            .vehicle
            .and_then(|v| self.objects.get(&v))
            .map_or(o.position, |v| v.position);
        Some(!o.hidden && at[1] > HIDDEN_BELOW && camera.sees(at) && !self.unseen.contains(&guid))
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
        let terms = crate::events::class_terms(label);
        let mut out: Vec<u32> = self
            .objects
            .iter()
            .filter(|(_, o)| o.is_of(&terms))
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

    fn action_done(&self, guid: u32) -> bool {
        self.hijack.action_done(guid)
    }

    fn hijack_presses(&self) -> &[(f64, u32)] {
        self.hijack.presses()
    }

    fn button_presses(&self) -> &[(f64, String)] {
        &self.buttons
    }

    fn context_presses(&self) -> &[(f64, u32)] {
        &self.context_presses
    }

    fn hijack_held_since(&self, button: u32) -> Option<f64> {
        self.hijack.held_since(button)
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

/// Answers the drop zone tests asked for since the last call: each spot
/// will do, as tested (`callback(true, x, y, z)`; the engine's own test of
/// the ground there isn't modelled). Returns any errors.
pub fn answer_drop_tests(world: &Shared) -> Vec<mlua::Error> {
    let tests = std::mem::take(&mut world.borrow_mut().drop_tests);
    let mut errors = Vec::new();
    for (f, [x, y, z]) in tests {
        if let Err(e) = f.call::<()>((true, x, y, z)) {
            errors.push(e);
        }
    }
    errors
}

/// Tells the scripts the assets they asked for have loaded: each load's
/// callback with its data unpacked, as `MrxUtil.CallWithOptionalArgs`
/// calls it (a table's items, else the value itself). The callbacks take
/// that shape (`MrxBriefing`'s takes the asset's name and false for "not
/// timed out", `MrxStarter`'s its name, type and false, `MrxUtil.
/// LoadingCallback` its counter); their own 15 s timers give the same call
/// with true. Returns any errors.
pub fn answer_asset_loads(world: &Shared) -> Vec<mlua::Error> {
    let loads = std::mem::take(&mut world.borrow_mut().asset_loads);
    let mut errors = Vec::new();
    for (f, data) in loads {
        let args: Vec<Value> = match data {
            Value::Table(t) => t.sequence_values().filter_map(Result::ok).collect(),
            Value::Nil => Vec::new(),
            other => vec![other],
        };
        if let Err(e) = f.call::<()>(MultiValue::from_vec(args)) {
            errors.push(e);
        }
    }
    errors
}

/// Calls the goal callbacks of `guid`'s finished AI goals, with their data
/// unpacked, then who had the goal and 1 for done. Those last two are
/// inferred from the support helicopter's landing callback
/// (`MrxSupportCopterDelivery._VehicleLanded(self, copter, pilot,
/// result)`: it gets the pilot out of the helicopter, and takes a result
/// of 0 for a landing that failed). Returns any errors.
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
        let mut args: Vec<Value> = match data {
            Some(t) => t.sequence_values().filter_map(Result::ok).collect(),
            None => Vec::new(),
        };
        args.extend([guid_value(Some(guid)), Value::Integer(1)]);
        if let Err(e) = f.call::<()>(MultiValue::from_vec(args)) {
            errors.push(e);
        }
    }
    errors
}

fn id(v: Option<&Value>) -> Option<u32> {
    crate::events::as_guid(v?)
}

/// A faction argument (`"Pmc"`), in lower case.
fn faction(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.to_string_lossy().to_ascii_lowercase()),
        Value::Integer(i) => Some(i.to_string()),
        _ => None,
    }
}

/// A sound category argument (`non_ui`), as named.
fn category(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

/// The factions' templates' names: their name hashes are the factions'
/// guids (`MrxGuiHudVehicleDisguise.Init`'s faction textures are keyed by
/// them).
pub const FACTIONS: [&str; 9] = [
    "Allied", "China", "Civ", "Guerilla", "OC", "Pirate", "PMC", "VZ", "Police",
];

/// [`Object.GetHibernationDistance`]'s answer, metres.
const HIBERNATION_DISTANCE: f64 = 155.0;

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

/// The save slot of the scripts' autosaves ([`Command::Save`]).
pub const AUTOSAVE: &str = "autosave";

/// The table of the game's own functions that hear the scripts' free
/// support items (`AddFreebie`, `RemoveFreebie`).
pub const SUPPORT_EARS: &str = "__m2_support";

/// Installs the libraries over `world`. Functions not written here stay
/// missing (or stubs, with `ScriptHost::stub_missing_globals`).
/// The engine's libraries (the rest of the globals scripts use are their
/// own modules); see `ScriptHost::complete_libraries`.
pub const LIBRARIES: [&str; 28] = [
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
    "Report",
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

    crate::airstrike::install(&lib("Airstrike")?, &add)?;
    crate::shop::install(lua, &lib("Hud")?)?;

    // A string's hash, as the scripts key tables by name (their recruits'
    // states). The archive's name hash: the scripts only compare hashes
    // with each other, so any one hash they all share serves.
    add(&lib("String")?, "GetHash", |_, _, a| {
        values(a.first().and_then(|v| match v {
            // A number (Lua 5.1 has no integers), whole on 32-bit builds.
            Value::String(s) => Some(Value::Number(f64::from(m2_assets::name_hash(
                &s.to_string_lossy(),
            )))),
            _ => None,
        }))
    })?;

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
    // The pursuit ([`Pursuit`]).
    add(&pg, "SetPursuitLevelTimes", |w, _, a| {
        if let (Some(low), Some(high)) = (num(a.first()), num(a.get(1))) {
            w.pursuit.times = Some((low, high));
        }
        values([])
    })?;
    add(&pg, "GetPursuitState", |w, lua, _| {
        let t = lua.create_table()?;
        t.set("Level", w.pursuit.level)?;
        if let Some(f) = w.pursuit.faction {
            t.set("Faction", guid_value(Some(f)))?;
        }
        values([Value::Table(t)])
    })?;
    // (faction, level, _) for a while; (faction, level) locked.
    fn pursue(w: &mut ScriptWorld, a: &[Value], locked: bool) {
        let Some(f) = id(a.first()) else { return };
        let level = num(a.get(1)).map_or(1, |l| l as i64).clamp(1, 3);
        let Some(name) = w
            .templates
            .iter()
            .find(|(_, g)| **g == f)
            .map(|(n, _)| n.clone())
        else {
            return;
        };
        w.pursuit.level = level;
        w.pursuit.faction = Some(f);
        let seconds = (!locked).then(|| w.pursuit.seconds(level));
        w.commands.push(Command::Pursuit {
            faction: name,
            seconds,
        });
    }
    add(&pg, "SetPursuit", |w, _, a| {
        pursue(w, a, false);
        values([])
    })?;
    add(&pg, "LockPursuit", |w, _, a| {
        pursue(w, a, true);
        values([])
    })?;
    // (faction, seconds, levels): -1 s until cleared.
    add(&pg, "SetCustomPursuit", |w, _, a| {
        let forever = num(a.get(1)).is_none_or(|s| s < 0.0);
        let Some(f) = id(a.first()) else {
            return values([]);
        };
        let Some(name) = w
            .templates
            .iter()
            .find(|(_, g)| **g == f)
            .map(|(n, _)| n.clone())
        else {
            return values([]);
        };
        w.pursuit.level = w.pursuit.level.max(1);
        w.pursuit.faction = Some(f);
        let seconds = (!forever).then(|| num(a.get(1)).unwrap_or(0.0));
        w.commands.push(Command::Pursuit {
            faction: name,
            seconds,
        });
        values([])
    })?;
    for clear in ["ClearPursuitLock", "ClearCustomPursuit"] {
        add(&pg, clear, |w, _, _| {
            w.pursuit.level = 0;
            w.pursuit.faction = None;
            w.commands.push(Command::PursuitClear);
            values([])
        })?;
    }
    // (faction, seconds, _): see [`Pursuit`].
    add(&pg, "SetPursuitSeconds", |_, _, _| values([]))?;
    // The objects of a kind within a radius of a point (x, y, z, radius),
    // as a list: what the strikes hit (the bunker buster's buildings, the
    // tank buster's tanks, the air patrol's aircraft). Which labels make
    // each kind is inferred from the templates' labels ("Vehicle",
    // "helicopter"...), not read from the engine.
    add(&pg, "FastCollectBuildings", |w, lua, a| {
        collect(w, lua, a, &["Building", "Structure"], &[])
    })?;
    add(&pg, "FastCollectHumans", |w, lua, a| {
        collect(w, lua, a, &["Human", "Soldier", "Civilian"], &[])
    })?;
    add(&pg, "FastCollectTanks", |w, lua, a| {
        collect(w, lua, a, &["Tank"], &[])
    })?;
    add(&pg, "FastCollectHelicopters", |w, lua, a| {
        collect(w, lua, a, &["helicopter"], &[])
    })?;
    add(&pg, "FastCollectFlying", |w, lua, a| {
        collect(w, lua, a, &["helicopter", "Plane", "Aircraft"], &[])
    })?;
    add(&pg, "FastCollectGroundVehicles", |w, lua, a| {
        collect(
            w,
            lua,
            a,
            &["Vehicle"],
            &["helicopter", "Plane", "Aircraft"],
        )
    })?;
    // A region's outline in the world: the corners' x and z as two lists
    // (the PDA map and radar draw the play area with them).
    add(&pg, "GetLineRegionPoints", |w, lua, a| {
        let guid = match a.first() {
            Some(Value::String(s)) => w.by_name(&s.to_string_lossy()),
            v => id(v),
        };
        let (xs, zs) = (lua.create_table()?, lua.create_table()?);
        if let Some(g) = guid
            && let (Some(r), Some(o)) = (w.regions.get(&g), w.objects.get(&g))
        {
            // Out of the region's own space, as `Region::contains` goes in.
            let (s, c) = (-o.yaw).sin_cos();
            for (i, [px, pz]) in r.corners.iter().enumerate() {
                xs.raw_set(i + 1, o.position[0] + px * c - pz * s)?;
                zs.raw_set(i + 1, o.position[2] + px * s + pz * c)?;
            }
        }
        values([Value::Table(xs), Value::Table(zs)])
    })?;
    add(&pg, "GetGuidByName", |w, _, a| {
        let g = match a.first() {
            Some(Value::String(s)) => {
                let name = s.to_string_lossy();
                w.by_name(&name)
                    .or_else(|| w.entity_names.get(&name.to_ascii_lowercase()).copied())
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
    // Whether the player passes for her vehicle's faction (the faction
    // manager then takes the vehicle's faction for hers).
    add(&object, "IsDisguised", |w, _, a| {
        let her = id(a.first()).is_some_and(|g| Some(g) == w.player);
        values([Value::Boolean(
            her && w.disguise.is_some_and(|(_, held)| held),
        )])
    })?;
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
    // (holder, object): whether `object` is still in `holder`'s hands. A
    // designator the game reports thrown has left them.
    add(&object, "IsAttached", |_, _, _| {
        values([Value::Boolean(false)])
    })?;
    // Object.SetTransformToObject(object, target[, hardpoint]): `object`
    // goes where `target` (a guid, or an object's name: the sickbay's
    // "Sickbay Hero Respawn 1") stands, turned as it is. From the scripts'
    // 4 call sites: the HQ's briefings put the player and the starter at
    // their `..._BriefingLoc` locators, the sickbay its hero at a respawn
    // point, the teleport helper its hero at an object's hardpoint (where
    // the game knows that hardpoint, else at the object itself).
    add(&object, "SetTransformToObject", |w, _, a| {
        let target = match a.get(1) {
            Some(Value::String(s)) => w.by_name(&s.to_string_lossy()),
            v => id(v),
        };
        let (Some(guid), Some(t)) = (id(a.first()), target) else {
            return values([]);
        };
        let Some(to) = w.objects.get(&t).map(|o| (o.position, o.yaw)) else {
            return values([]);
        };
        let hardpoint = match a.get(2) {
            Some(Value::String(s)) => w
                .hijack
                .hardpoint(t, m2_assets::name_hash(&s.to_string_lossy())),
            _ => None,
        };
        let (position, yaw) = (hardpoint.unwrap_or(to.0), to.1);
        if let Some(o) = w.objects.get_mut(&guid) {
            o.position = position;
            o.yaw = yaw;
        }
        w.commands.push(Command::Move { guid, position });
        w.commands.push(Command::Turn { guid, yaw });
        values([])
    })?;
    // How far off an object goes to sleep (metres). The deliveries place
    // a helicopter's cargo by it, with 155 as their own fallback: that,
    // until the templates' hibernation data is read.
    add(&object, "GetHibernationDistance", |_, _, _| {
        values([Value::Number(HIBERNATION_DISTANCE)])
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
            w.inventory.drop(guid);
            w.commands.push(Command::Remove(guid));
        }
        values([])
    })?;
    // (gate): opens a gate or door, by guid (or name); the game swings it
    // out of the way. PmcCon001's gate opener opens the villa's doors
    // this way as each wakes.
    add(&object, "OpenGate", |w, _, a| {
        let gate = id(a.first()).or_else(|| match a.first() {
            Some(Value::String(s)) => w.by_name(&s.to_string_lossy()),
            _ => None,
        });
        if let Some(guid) = gate {
            w.commands.push(Command::OpenGate(guid));
        }
        values([])
    })?;
    // (gate): shuts it again (3 scripts: the mine's gate, `vz_mine_gate`,
    // and two outposts' gate scripts once their convoy is through).
    add(&object, "CloseGate", |w, _, a| {
        let gate = id(a.first()).or_else(|| match a.first() {
            Some(Value::String(s)) => w.by_name(&s.to_string_lossy()),
            _ => None,
        });
        if let Some(guid) = gate {
            w.commands.push(Command::CloseGate(guid));
        }
        values([])
    })?;
    // What an object is a copy of: a weapon's template (saves keep it and
    // hand it back to `SetAllWeapons`), a spawned object's.
    add(&object, "GetParent", |w, _, a| {
        let Some(g) = id(a.first()) else {
            return values([Value::Nil]);
        };
        if let Some(c) = w.inventory.carried.get(&g) {
            return values([guid_value(Some(c.template))]);
        }
        let name = match w.objects.get(&g) {
            Some(o) if is_template(&o.name) && !w.templates.values().any(|&t| t == g) => {
                o.name.clone()
            }
            _ => return values([Value::Nil]),
        };
        values([guid_value(Some(w.template(&name)))])
    })?;

    // The witnesses' reports ([`crate::report`]).
    let report = lib("Report")?;
    add(&report, "Init", |w, _, a| {
        if let Some(Value::Table(t)) = a.first() {
            w.reports.callback = t.get::<Option<Function>>("Callback").ok().flatten();
        }
        values([])
    })?;
    add(&report, "Failed", |w, _, a| {
        if let Some(g) = id(a.first())
            && w.reports.reporting.remove(&g).is_some()
        {
            w.reports.ended.push((g, false));
        }
        values([])
    })?;
    add(&report, "Completed", |w, _, a| {
        if let Some(g) = id(a.first())
            && w.reports.reporting.remove(&g).is_some()
        {
            w.reports.ended.push((g, true));
        }
        values([])
    })?;
    add(&report, "GetInfractions", |w, lua, a| {
        let g = id(a.first()).unwrap_or(0);
        values([Value::Table(w.reports.infractions(lua, g)?)])
    })?;
    // (reporter, seconds): a reporter kept from reporting for a while (the
    // faction manager delays a disabled one by 3 s); not used yet.
    add(&report, "SetDelay", |_, _, _| values([]))?;
    // Weapons as objects: a loadout of templates, each weapon with its own
    // reserve of ammunition.
    let human = lib("Human")?;
    // Human.DisableWeapons(character) / EnableWeapons(character).
    add(&human, "DisableWeapons", |w, _, a| {
        if let Some(guid) = id(a.first()) {
            w.commands.push(Command::Weapons { guid, on: false });
        }
        values([])
    })?;
    add(&human, "EnableWeapons", |w, _, a| {
        if let Some(guid) = id(a.first()) {
            w.commands.push(Command::Weapons { guid, on: true });
        }
        values([])
    })?;
    // Human.IsSwimming(character): out of her depth in the water, as the
    // game says ([`ScriptWorld::swimming`]); a vehicle's script asks it of
    // a character before acting (7c99e43e).
    add(&human, "IsSwimming", |w, _, a| {
        let on = id(a.first()).is_some_and(|g| w.swimming.contains(&g));
        values([Value::Boolean(on)])
    })?;
    let inventory = match human.raw_get::<Value>("Inventory")? {
        Value::Table(t) => t,
        _ => {
            let t = lua.create_table()?;
            human.raw_set("Inventory", &t)?;
            t
        }
    };
    add(&inventory, "GetAllWeapons", |w, lua, a| {
        let list = id(a.first()).map_or(&[][..], |who| w.inventory.of(who));
        let t = lua.create_sequence_from(list.iter().map(|&g| crate::events::guid_value(g)))?;
        values([Value::Table(t)])
    })?;
    add(&inventory, "GetPrimaryWeapon", |w, _, a| {
        values([guid_value(id(a.first()).and_then(|who| w.held(who, 0)))])
    })?;
    add(&inventory, "GetSecondaryWeapon", |w, _, a| {
        values([guid_value(id(a.first()).and_then(|who| w.held(who, 1)))])
    })?;
    add(&inventory, "SetAllWeapons", |w, _, a| {
        let (Some(who), Some(Value::Table(list))) = (id(a.first()), a.get(1)) else {
            return values([]);
        };
        let mut templates = Vec::new();
        for v in list.clone().sequence_values::<Value>() {
            match v? {
                Value::String(s) => templates.push(w.template(&s.to_string_lossy())),
                v => templates.extend(id(Some(&v))),
            }
        }
        w.arm(who, &templates);
        values([])
    })?;
    // (character, weapon): the weapon leaves whoever carries it.
    add(&inventory, "DropWeapon", |w, _, a| {
        let Some(weapon) = id(a.get(1)) else {
            return values([]);
        };
        let name = w.weapon_name(weapon);
        if let (Some(c), Some(weapon)) = (w.inventory.drop(weapon), name) {
            w.commands.push(Command::DropWeapon {
                who: c.owner,
                weapon,
            });
        }
        values([])
    })?;
    // Clips are the game's (`ReloadAll`), and so is which is in hand.
    for name in ["ReloadAll", "EquipWeapon"] {
        add(&inventory, name, |_, _, _| values([]))?;
    }
    let weapon = lib("Weapon")?;
    add(&weapon, "GetMaxReserveAmmo", |w, _, a| {
        let max = id(a.first()).and_then(|g| w.weapon_name(g)).and_then(|n| {
            w.inventory
                .max_reserve
                .get(&n.to_ascii_lowercase())
                .copied()
        });
        values([max.map_or(Value::Nil, |m| Value::Number(f64::from(m)))])
    })?;
    add(&weapon, "GetReserveAmmo", |w, _, a| {
        let n = id(a.first())
            .and_then(|g| w.inventory.carried.get(&g))
            .map(|c| c.reserve);
        values([n.map_or(Value::Nil, |n| Value::Number(f64::from(n)))])
    })?;
    add(&weapon, "SetReserveAmmo", |w, _, a| {
        let (Some(g), Some(rounds)) = (id(a.first()), num(a.get(1))) else {
            return values([]);
        };
        let rounds = rounds.max(0.0) as u32;
        let name = w.weapon_name(g);
        if let (Some(c), Some(weapon)) = (w.inventory.carried.get_mut(&g), name) {
            c.reserve = rounds;
            let who = c.owner;
            w.commands.push(Command::Reserve {
                who,
                weapon,
                rounds,
            });
        }
        values([])
    })?;
    // The number of the player controlling it (scripts pass it on to
    // `Player.IsLocal` and key tables by it), else nil.
    // Metres between two objects (or markers of layers not loaded).
    // From an object to another, or to a point x, y, z (`MrxUtil.
    // GetDistanceToObject(her, x, y, z)`, as the laser guided bomb and the
    // daisy cutter ask how far she is from their target).
    add(&object, "GetDistanceFrom", |w, _, a| {
        let to = match (num(a.get(1)), num(a.get(2)), num(a.get(3))) {
            (Some(x), Some(y), Some(z)) => Some([x, y, z]),
            _ => id(a.get(1)).and_then(|g| w.place_of(g)),
        };
        let (Some(p), Some(q)) = (id(a.first()).and_then(|g| w.place_of(g)), to) else {
            return values([Value::Nil]);
        };
        let d: f32 = (0..3).map(|i| (p[i] - q[i]).powi(2)).sum::<f32>().sqrt();
        values([Value::Number(f64::from(d))])
    })?;
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
        let g = id(a.first());
        let o = g.and_then(|g| w.objects.get(&g));
        values([match (o, g) {
            (Some(o), _) => Value::String(lua.create_string(&o.name)?),
            // A faction's name as the faction zones look it up: keyed by
            // `String.GetHash` of the name, which its guid already is.
            (None, Some(g)) if FACTIONS.iter().any(|f| m2_assets::name_hash(f) == g) => {
                Value::Number(f64::from(g))
            }
            _ => Value::Nil,
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
    // A character's camera: the game's one camera looks over the player,
    // so it is named by her guid, and its yaw is hers (`Camera.GetYaw`:
    // the carpet bomb's heading).
    add(&player, "GetCamera", |w, _, a| {
        let who = id(a.first()).or(w.player);
        values([guid_value(who)])
    })?;
    let camera = lib("Camera")?;
    // The camera looks the way she faces; a camera whose holder this
    // world doesn't place (the HUD's own world) faces north. Always a
    // number: the PDA's map turns by it (`math.rad`), and nil broke its
    // Open half-way, leaving the PDA marked open and the support menu shut.
    add(&camera, "GetYaw", |w, _, a| {
        let yaw = id(a.first())
            .or(w.player)
            .and_then(|g| w.objects.get(&g))
            .map_or(0.0, |o| o.yaw);
        values([Value::Number(f64::from(yaw))])
    })?;
    // Camera.SetShot(camera, shot, base, target, ...): a named shot of the
    // engine's (the HQ's briefings ask `OverTheShoulderLeft` and `Right`,
    // from one speaker to the other, `MrxBriefing`'s cheap cinematic),
    // held while the player is in cinematic mode.
    add(&camera, "SetShot", |w, _, a| {
        let name = match a.get(1) {
            Some(Value::String(s)) => s.to_string_lossy(),
            _ => return values([]),
        };
        if let (Some(base), Some(target)) = (id(a.get(2)), id(a.get(3))) {
            w.commands.push(Command::CameraShot {
                shot: name,
                base,
                target,
            });
        }
        values([])
    })?;
    // Player.SetCinematicMode([player,] on, ...): the scripts take her
    // controls and the camera for a scene (the HQ's briefings), and give
    // them back with false; the first boolean is the switch (the scripts'
    // 1-argument and 5-argument calls).
    add(&player, "SetCinematicMode", |w, _, a| {
        let Some(on) = a.iter().find_map(|v| match v {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }) else {
            return values([]);
        };
        if on != w.cinematic {
            w.cinematic = on;
            w.commands.push(Command::Cinematic(on));
        }
        values([])
    })?;
    add(&player, "InCinematicMode", |w, _, _| {
        values([Value::Boolean(w.cinematic)])
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
    add(&player, "SetInputEnabled", |w, _, a| {
        let on = !matches!(a.get(1), Some(Value::Boolean(false)));
        w.commands.push(Command::InputEnabled(on));
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
    add(&player, "GetCash", |w, _, _| {
        values([Value::Number(w.cash)])
    })?;
    add(&player, "SetCash", |w, _, a| {
        if let Some(c) = num(a.first()) {
            w.cash = f64::from(c);
        }
        values([])
    })?;
    // 100 until the scripts set it: invented, the game's figure is not
    // decoded.
    add(&player, "GetFuelCapacity", |w, _, _| {
        values([Value::Number(f64::from(w.fuel_capacity.unwrap_or(100.0)))])
    })?;
    add(&player, "SetFuelCapacity", |w, _, a| {
        if let Some(c) = num(a.first()) {
            w.fuel_capacity = Some(c);
        }
        values([])
    })?;
    // Player.AddFuel(amount): a fuel pickup's script (133dde94) adds its
    // amount once it checked her tanks have room; clamped to the
    // capacity here too.
    add(&player, "AddFuel", |w, _, a| {
        if let Some(n) = num(a.first()) {
            let cap = w.fuel_capacity.unwrap_or(100.0);
            if let Some(o) = w.player.and_then(|p| w.objects.get_mut(&p)) {
                o.fuel = (o.fuel + n).clamp(0.0, cap);
            }
            w.commands.push(Command::AddFuel(n));
        }
        values([])
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
    // What the player controls: the vehicle she rides, else herself (the
    // hit arcs size a hit by its maximum health).
    add(&player, "GetControlledObject", |w, _, a| {
        let p = id(a.first()).or(w.player);
        values([guid_value(p.map(|p| w.vehicle_of(p).unwrap_or(p)))])
    })?;
    // Whether vehicle disguises work at all: the vehicle box shows its
    // disguise gauge only then. `MrxMissionFlow.SetVehicleDisguiseEnabled`
    // passes a boolean; 0 or 1 are taken too.
    add(&player, "SetVehicleDisguise", |w, _, a| {
        w.vehicle_disguise = match a.first() {
            Some(Value::Boolean(b)) => *b,
            v => num(v).is_some_and(|n| n != 0.0),
        };
        if !w.vehicle_disguise {
            w.disguise = None;
        }
        values([])
    })?;
    add(&player, "GetVehicleDisguise", |w, _, _| {
        values([Value::Boolean(w.vehicle_disguise)])
    })?;
    // `{ Player = character }`: whether her disguise holds, nil out of a
    // disguise (the disguise tutorial tells the two apart by
    // `tostring`).
    add(&player, "GetVehicleDisguiseState", |w, _, _| {
        values([w
            .disguise
            .map_or(Value::Nil, |(_, held)| Value::Boolean(held))])
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
    // Vehicle.Usable(vehicle, usable): all three calls in the scripts pass
    // false, for vehicles the scripts keep for themselves.
    add(&vehicle, "Usable", |w, _, a| {
        if let Some(guid) = id(a.first()) {
            let on = !matches!(a.get(1), Some(Value::Boolean(false)));
            w.commands.push(Command::Usable { guid, on });
        }
        values([])
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
    // update; the command lets the game animate it. Vehicle.Exit(vehicle,
    // rider, instant): all 21 calls in the scripts pass the vehicle first
    // (`Vehicle.GetFromRider(hero)`, `Player.GetControlledObject`) and the
    // rider second; a lone argument is the rider.
    add(&vehicle, "Exit", |w, _, a| {
        if let Some(guid) = id(a.get(1)).or_else(|| id(a.first())) {
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
    // `Graphics.Atmosphere`: the light of a line region, by the name of
    // one of its records.
    let graphics = lib("Graphics")?;
    // `Graphics.Camera` and `Graphics.Effect` (the hijack's
    // `SetFocusParams`, `CameraFade`): tables of their own, whose members
    // the library's stubbing fills in. Left out, the library's stubs
    // would answer them with functions.
    for name in ["Camera", "Effect"] {
        if !matches!(graphics.raw_get::<Value>(name)?, Value::Table(_)) {
            graphics.raw_set(name, lua.create_table()?)?;
        }
    }
    let atmosphere = match graphics.raw_get::<Value>("Atmosphere")? {
        Value::Table(t) if t.metatable().is_none() => t,
        _ => {
            let t = lua.create_table()?;
            graphics.raw_set("Atmosphere", &t)?;
            t
        }
    };
    add(&atmosphere, "ChangeLineRegionSetting", |w, _, a| {
        if let (Some(region), Some(Value::String(s))) = (id(a.first()), a.get(1)) {
            w.commands.push(Command::Atmosphere {
                region,
                setting: s.to_string_lossy(),
            });
        }
        values([])
    })?;
    add(&atmosphere, "EnableImmediatelyChangeMode", |w, _, a| {
        if let Some(Value::Boolean(on)) = a.first() {
            w.commands.push(Command::AtmosphereImmediate(*on));
        }
        values([])
    })?;
    let gui = lib("Gui")?;
    add(&gui, "GetLanguageName", |_, lua, _| {
        values([Value::String(lua.create_string("english")?)])
    })?;

    let ai = lib("Ai")?;
    add(&ai, "GetFactionGuid", |w, _, a| {
        let faction = id(a.first()).and_then(|g| w.objects.get(&g)?.faction);
        values([guid_value(faction)])
    })?;
    // How one faction's AI regards another, -100 (hostile) to 100
    // (friendly), as the faction manager sets it (`Ai.SetRelation(a, b,
    // value)`); the faction manager reads attitudes back from it. Pairs
    // never set are neutral, 0; one set only the other way round answers
    // for both (inferred).
    add(&ai, "SetRelation", |w, _, a| {
        if let (Some(from), Some(to), Some(value)) =
            (faction(a.first()), faction(a.get(1)), num(a.get(2)))
        {
            w.relations.insert((from, to), value);
        }
        values([])
    })?;
    add(&ai, "GetRelation", |w, _, a| {
        let value = match (faction(a.first()), faction(a.get(1))) {
            (Some(from), Some(to)) => w
                .relations
                .get(&(from.clone(), to.clone()))
                .or_else(|| w.relations.get(&(to, from)))
                .copied()
                .unwrap_or(0.0),
            _ => 0.0,
        };
        values([Value::Number(f64::from(value))])
    })?;
    // Whether a helicopter can set a load down near `Location` (a supply
    // drop, a vehicle): answered later through `Callback`
    // ([`answer_drop_tests`]); true now, the test under way.
    add(&ai, "TestDropZone", |w, _, a| {
        let Some(Value::Table(t)) = a.first() else {
            return values([Value::Boolean(false)]);
        };
        let at = match t.raw_get::<Value>("Location")? {
            Value::Table(l) => {
                let c = |i| num(l.raw_get::<Value>(i).ok().as_ref());
                c(1).zip(c(2)).zip(c(3)).map(|((x, y), z)| [x, y, z])
            }
            _ => None,
        };
        match (t.raw_get::<Value>("Callback")?, at) {
            (Value::Function(f), Some(at)) => {
                w.drop_tests.push((f, at));
                values([Value::Boolean(true)])
            }
            _ => values([Value::Boolean(false)]),
        }
    })?;
    // Pg.LoadAsset(name, type[, callback, data]): the alpha reads what it
    // draws itself, so every asset is there at once; a callback hears so
    // on the next tick ([`answer_asset_loads`]). Of the scripts' calls,
    // most preload (name and type); four pass a callback: the HQ's starter
    // and its briefing wait on them before the screen comes down.
    add(&pg, "LoadAsset", |w, _, a| {
        if let Some(Value::Function(f)) = a.get(2) {
            w.asset_loads
                .push((f.clone(), a.get(3).cloned().unwrap_or(Value::Nil)));
        }
        values([])
    })?;
    add(&pg, "UnloadAsset", |_, _, _| values([]))?;
    // Pg.AddContextAction(guid, text[, reach, ...]) and
    // Pg.RemoveContextAction(guid): see [`ContextAction`].
    add(&pg, "AddContextAction", |w, _, a| {
        let (Some(guid), Some(Value::String(text))) = (id(a.first()), a.get(1)) else {
            return values([]);
        };
        let reach = num(a.get(2)).filter(|r| *r > 0.0).unwrap_or(CONTEXT_REACH);
        w.context_actions.insert(
            guid,
            ContextAction {
                text: text.to_string_lossy(),
                reach,
            },
        );
        values([])
    })?;
    add(&pg, "RemoveContextAction", |w, _, a| {
        if let Some(guid) = id(a.first()) {
            w.context_actions.remove(&guid);
        }
        values([])
    })?;
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
        let haste = num(t.raw_get::<Value>("Haste").ok().as_ref());
        if let Value::Function(f) = t.raw_get::<Value>("Callback")? {
            let data = t.raw_get::<Option<Table>>("CallbackData")?;
            w.goal_callbacks.push((guid, f, data));
        }
        w.commands.push(Command::Goal {
            guid,
            goal,
            location,
            target,
            haste,
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
            haste: None,
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
    add(&net, "SetLoadingScreen", |w, _, a| {
        let on = matches!(a.first(), Some(Value::Boolean(true)));
        w.commands.push(Command::LoadingScreen(on));
        values([])
    })?;

    // Calls the game only needs to hear about.
    add(&pg, "SaveGame", |w, _, a| {
        let slot = match a.first() {
            Some(Value::String(s)) => s.to_string_lossy(),
            _ => String::new(),
        };
        w.commands.push(Command::Save { slot });
        values([])
    })?;
    for name in [
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

    // The scripts' sound sets (`MrxSoundCategories`) duck categories under
    // conversations and slow them in the survival rescue: Down and
    // Activate take (category, level or pitch, seconds), Up and Deactivate
    // (category, seconds) and bring it back.
    add(&sound, "FadeCategoryDown", |w, _, a| {
        if let Some(category) = category(a.first()) {
            w.commands.push(Command::FadeSound {
                category,
                level: num(a.get(1)).unwrap_or(0.3),
                seconds: num(a.get(2)).unwrap_or(0.5),
            });
        }
        values([])
    })?;
    add(&sound, "FadeCategoryUp", |w, _, a| {
        if let Some(category) = category(a.first()) {
            w.commands.push(Command::FadeSound {
                category,
                level: 1.0,
                seconds: num(a.get(1)).unwrap_or(0.5),
            });
        }
        values([])
    })?;
    add(&sound, "PitchCategoryActivate", |w, _, a| {
        if let Some(category) = category(a.first()) {
            w.commands.push(Command::PitchSound {
                category,
                pitch: num(a.get(1)).unwrap_or(1.0),
                seconds: num(a.get(2)).unwrap_or(0.5),
            });
        }
        values([])
    })?;
    add(&sound, "PitchCategoryDeactivate", |w, _, a| {
        if let Some(category) = category(a.first()) {
            w.commands.push(Command::PitchSound {
                category,
                pitch: 1.0,
                seconds: num(a.get(1)).unwrap_or(0.5),
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
    // `CueWithoutSubtitles` says the same with no subtitle; `Cue` puts
    // the line's text up as the game does, through `MrxSubtitle.Add`
    // (see [`crate::vo`]).
    fn cue(w: &mut ScriptWorld, a: &[Value], subtitled: bool) -> mlua::Result<MultiValue> {
        let Some(Value::String(cue)) = a.get(1) else {
            return values([Value::Boolean(false)]);
        };
        let speaker = id(a.first()).filter(|&g| g != 0);
        let callback = match a.get(2) {
            Some(Value::Function(f)) => Some(f.clone()),
            _ => None,
        };
        let subtitled = subtitled && !w.subtitles_off;
        let say =
            w.vo.cue(speaker, cue.to_string_lossy(), callback, subtitled);
        w.commands.push(say);
        values([Value::Boolean(true)])
    }
    add(&vo, "Cue", |w, _, a| cue(w, a, true))?;
    add(&vo, "CueWithoutSubtitles", |w, _, a| cue(w, a, false))?;
    add(&vo, "Cancel", |w, _, a| {
        let speaker = id(a.first()).filter(|&g| g != 0);
        let cue = match a.get(1) {
            Some(Value::String(s)) => Some(s.to_string_lossy()),
            _ => None,
        };
        let hush = w.vo.cancel(speaker, cue.as_deref());
        // As called, and what it stopped: a nil speaker (or one that is no
        // guid) is taken for the radio, which is ours to decide.
        let raw = |v: Option<&Value>| match v {
            None | Some(Value::Nil) => "nil".to_owned(),
            Some(Value::String(s)) => format!("{:?}", s.to_string_lossy()),
            Some(Value::Integer(i)) => i.to_string(),
            Some(v) => v.type_name().to_owned(),
        };
        let ids: Vec<u32> = hush
            .iter()
            .filter_map(|c| match c {
                Command::Hush(id) => Some(*id),
                _ => None,
            })
            .collect();
        w.printed.push(format!(
            "VO.Cancel({}, {}): speaker {speaker:?}, stops lines {ids:?}",
            raw(a.first()),
            raw(a.get(1))
        ));
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
    // The game shows its tutorial tips, unless the player turned them off.
    add(&sys, "TutorialsEnabled", |w, _, _| {
        values([Value::Boolean(!w.tutorials_off)])
    })?;
    for name in ["GetSkipMission", "GetINIBriefing", "GetForceNewGame"] {
        add(&sys, name, |_, _, _| values([Value::Nil]))?;
    }
    add(&sys, "SetSkipMission", |_, _, _| values([]))?;
    // `WifMissionFlow.Autosave` asks this after a contract (with whether
    // a mission is on, the last contract, the time played and the share
    // of the game done): the game saves.
    add(&sys, "RequestAutosave", |w, _, _| {
        w.commands.push(Command::Save {
            slot: AUTOSAVE.into(),
        });
        values([])
    })?;
    add(&sys, "SubtitlesEnabled", |w, _, _| {
        values([Value::Boolean(!w.subtitles_off)])
    })?;
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
    // [a, b] (or [1, a]), random reals in [a, b] and normalising.
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
        -- A vector scaled to length 1 (the strikes aim their bombs so);
        -- the zero vector stays as it is.
        function M.Normalize(x, y, z)
            local l = math.sqrt(x * x + y * y + z * z)
            if l == 0 then return x, y, z end
            return x / l, y / l, z / l
        end
        -- A vector's length (the fuel-air bomb measures its drop so).
        function M.Length(x, y, z)
            return math.sqrt(x * x + y * y + (z or 0) * (z or 0))
        end
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
    // The widget store the HUD draws from replaces the stand-in above.
    crate::gui_runtime::install(lua)?;

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

/// The objects in play within `a`'s radius of its point (x, y, z,
/// radius) with any of the labels `with` and none of `without`, as a
/// list ([`ScriptWorld::place_of`]: a rider is where its vehicle is).
fn collect(
    w: &mut ScriptWorld,
    lua: &Lua,
    a: &[Value],
    with: &[&str],
    without: &[&str],
) -> mlua::Result<MultiValue> {
    let (Some(x), Some(y), Some(z), Some(r)) =
        (num(a.first()), num(a.get(1)), num(a.get(2)), num(a.get(3)))
    else {
        return values([Value::Table(lua.create_table()?)]);
    };
    let hashes = |names: &[&str]| -> Vec<u32> {
        names
            .iter()
            .map(|n| m2_assets::name_hash(&n.to_ascii_lowercase()))
            .chain(names.iter().map(|n| m2_assets::name_hash(n)))
            .collect()
    };
    let (with, without) = (hashes(with), hashes(without));
    let mut found: Vec<u32> = w
        .objects
        .iter()
        .filter(|(_, o)| !o.hidden)
        .filter(|(_, o)| {
            let labels = o.all_labels();
            labels.iter().any(|l| with.contains(l)) && !labels.iter().any(|l| without.contains(l))
        })
        .filter_map(|(&g, _)| {
            let p = w.place_of(g)?;
            let d = ((p[0] - x).powi(2) + (p[1] - y).powi(2) + (p[2] - z).powi(2)).sqrt();
            (d <= r).then_some(g)
        })
        .collect();
    found.sort_unstable();
    let list = lua.create_table()?;
    for g in found {
        list.raw_push(guid_value(Some(g)))?;
    }
    values([Value::Table(list)])
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
    fn the_camera_sees_what_is_in_front_within_its_view_and_range() {
        // Looking down +z, 60 degrees tall, twice as wide.
        let camera = Camera {
            eye: [0.0, 2.0, 0.0],
            forward: [0.0, 0.0, 1.0],
            fov_y: 60f32.to_radians(),
            aspect: 2.0,
            range: VISIBLE_RANGE,
        };
        assert!(camera.sees([0.0, 2.0, 50.0]));
        // Behind, beyond the range, above the top edge (tan 30 = 0.58).
        assert!(!camera.sees([0.0, 2.0, -50.0]));
        assert!(!camera.sees([0.0, 2.0, VISIBLE_RANGE + 1.0]));
        assert!(!camera.sees([0.0, 2.0 + 35.0, 50.0]));
        // Off to the side: within the wider sides (2 x 0.58), not past.
        assert!(camera.sees([50.0, 2.0, 50.0]));
        assert!(!camera.sees([-60.0, 2.0, 50.0]));
    }

    #[test]
    fn objects_are_visible_on_screen_unless_hidden_or_behind_something() {
        let (_host, world) = setup();
        let mut w = world.borrow_mut();
        // No camera fed yet: unknown.
        assert_eq!(w.is_visible(50), None);
        w.camera = Some(Camera {
            eye: [0.0, 2.0, -20.0],
            forward: [0.0, 0.0, 1.0],
            fov_y: 1.0,
            aspect: 16.0 / 9.0,
            range: VISIBLE_RANGE,
        });
        assert_eq!(w.is_visible(50), Some(true));
        assert_eq!(w.is_visible(999), Some(false));
        w.unseen.insert(50);
        assert_eq!(w.is_visible(50), Some(false));
        w.unseen.clear();
        // Hidden (`Object.SetVisible`), or put under the island.
        w.objects.get_mut(&50).unwrap().hidden = true;
        assert_eq!(w.is_visible(50), Some(false));
        w.objects.get_mut(&50).unwrap().hidden = false;
        w.objects.get_mut(&50).unwrap().position[1] = -40_000.0;
        assert_eq!(w.is_visible(50), Some(false));
    }

    #[test]
    fn objects_far_from_the_player_hibernate_and_wake_as_she_nears() {
        let (_host, world) = setup();
        let mut w = world.borrow_mut();
        w.update_hibernation();
        assert!(w.is_awake(50));
        let far = HIBERNATION_DISTANCE as f32 + 5.0;
        // Just past the distance: still awake (the margin), then asleep.
        w.objects.get_mut(&1).unwrap().position = [far, 0.0, 0.0];
        w.update_hibernation();
        assert!(w.is_awake(50));
        w.objects.get_mut(&1).unwrap().position = [far + HIBERNATION_MARGIN, 0.0, 0.0];
        w.update_hibernation();
        assert!(!w.is_awake(50));
        // She keeps hers.
        assert!(w.is_awake(1));
        w.objects.get_mut(&1).unwrap().position = [far - 10.0, 0.0, 0.0];
        w.update_hibernation();
        assert!(w.is_awake(50));
    }

    #[test]
    fn a_spawn_far_off_is_awake_when_the_scripts_next_look() {
        // VzaCon001's weapon drop: the helicopter starts about 190 m from
        // her, and the drop waits for it to be awake before sending it.
        let (_host, world) = setup();
        let mut w = world.borrow_mut();
        w.objects.get_mut(&1).unwrap().position = [0.0; 3];
        let far = [0.0, 80.0, 175.0];
        let copter = w.spawn("Alouette3 Transport (VZA Intro) (Driver)".into(), far, 0.0);
        w.update_hibernation();
        assert!(w.is_awake(copter));
        // After that look it sleeps as anything that far off does.
        w.update_hibernation();
        assert!(!w.is_awake(copter));
    }

    #[test]
    fn open_gate_asks_the_game_to_open_it() {
        let (host, world) = setup();
        host.lua()
            .load(
                "Object.OpenGate(Pg.GetGuidByName('_global_veh_jeep 0x0001'))\n\
                 Object.OpenGate('_global_veh_jeep 0x0001')",
            )
            .exec()
            .unwrap();
        assert_eq!(
            world.borrow_mut().take_commands(),
            [Command::OpenGate(50), Command::OpenGate(50)]
        );
    }

    #[test]
    fn gates_weapons_usable_vehicles_and_fuel_reach_the_game() {
        let (host, world) = setup();
        world.borrow_mut().fuel_capacity = Some(50.0);
        host.lua()
            .load(
                "Object.CloseGate('_global_veh_jeep 0x0001')\n\
                 Human.DisableWeapons(Player.GetAnyCharacter())\n\
                 Human.EnableWeapons(Player.GetAnyCharacter())\n\
                 Vehicle.Usable(Pg.GetGuidByName('_global_veh_jeep 0x0001'), false)\n\
                 Player.AddFuel(80)",
            )
            .exec()
            .unwrap();
        let mut w = world.borrow_mut();
        assert_eq!(
            w.take_commands(),
            [
                Command::CloseGate(50),
                Command::Weapons { guid: 1, on: false },
                Command::Weapons { guid: 1, on: true },
                Command::Usable {
                    guid: 50,
                    on: false
                },
                Command::AddFuel(80.0),
            ]
        );
        // Up to the tanks' capacity.
        assert_eq!(w.objects[&1].fuel, 50.0);
    }

    #[test]
    fn the_scripts_pursuit_calls_reach_the_game_and_read_back_their_level() {
        let (host, world) = setup();
        let run = |code: &str| host.lua().load(code).exec().unwrap();
        run("Pg.SetPursuitLevelTimes(120, 300)\n\
             __before = Pg.GetPursuitState().Level\n\
             Pg.SetPursuit(Pg.GetGuidByName('OC'), 1, true)\n\
             __one = Pg.GetPursuitState().Level\n\
             Pg.SetPursuit(Pg.GetGuidByName('OC'), 3, true)\n\
             Pg.LockPursuit(Pg.GetGuidByName('Allied'), 2)\n\
             Pg.ClearPursuitLock(true)\n\
             __after = Pg.GetPursuitState().Level");
        let g = host.lua().globals();
        assert_eq!(g.get::<i64>("__before").unwrap(), 0);
        assert_eq!(g.get::<i64>("__one").unwrap(), 1);
        assert_eq!(g.get::<i64>("__after").unwrap(), 0);
        let pursuit = |faction: &str, seconds| Command::Pursuit {
            faction: faction.into(),
            seconds,
        };
        assert_eq!(
            world.borrow_mut().take_commands(),
            [
                pursuit("oc", Some(120.0)),
                pursuit("oc", Some(300.0)),
                pursuit("allied", None),
                Command::PursuitClear,
            ]
        );
    }

    #[test]
    fn set_transform_to_object_puts_one_where_another_stands() {
        // The HQ's briefing: the hero to `..._BriefingLoc_Hero1`, by guid
        // or by name; a hardpoint the game reported goes over the origin.
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            let mut spot = Object::new(
                "PmcInterior_StarterFiona_BriefingLoc_Hero1",
                [3.0, 4.0, 5.0],
            );
            spot.yaw = 1.5;
            w.insert(60, spot);
            w.hijack
                .set_hardpoint(50, m2_assets::name_hash("hp_seat"), [7.0, 8.0, 9.0]);
        }
        host.lua()
            .load(
                "Object.SetTransformToObject(Player.GetAnyCharacter(), \
                 'PmcInterior_StarterFiona_BriefingLoc_Hero1')",
            )
            .exec()
            .unwrap();
        {
            let mut w = world.borrow_mut();
            assert_eq!(w.objects[&1].position, [3.0, 4.0, 5.0]);
            assert_eq!(w.objects[&1].yaw, 1.5);
            assert_eq!(
                w.take_commands(),
                [
                    Command::Move {
                        guid: 1,
                        position: [3.0, 4.0, 5.0]
                    },
                    Command::Turn { guid: 1, yaw: 1.5 }
                ]
            );
        }
        host.lua()
            .load(
                "Object.SetTransformToObject(Player.GetAnyCharacter(), \
                 Pg.GetGuidByName('_global_veh_jeep 0x0001'), 'hp_seat')",
            )
            .exec()
            .unwrap();
        assert_eq!(world.borrow().objects[&1].position, [7.0, 8.0, 9.0]);
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
    fn a_loadout_of_templates_becomes_weapons_with_reserves() {
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            w.inventory.max_reserve.insert("carbine".into(), 9);
            w.inventory.max_reserve.insert("grenade".into(), 9);
            w.inventory.thrown.insert("grenade".into());
            w.template_labels
                .insert("grenade".into(), vec![m2_assets::name_hash("Grenade")]);
        }
        let (primary, grenades, max, same, thrown, count): (String, i64, i64, bool, bool, i64) =
            host.lua()
                .load(
                    r#"
                    local hero = Player.GetLocalCharacter()
                    Human.Inventory.SetAllWeapons(hero, {
                        Pg.GetGuidByName("Carbine"), Pg.GetGuidByName("Grenade") })
                    local gun = Human.Inventory.GetPrimaryWeapon(hero)
                    Weapon.SetReserveAmmo(gun, Weapon.GetMaxReserveAmmo(gun))
                    local all = Human.Inventory.GetAllWeapons(hero)
                    return Object.GetName(gun), Weapon.GetReserveAmmo(all[2]),
                        Weapon.GetReserveAmmo(gun),
                        Object.GetParent(all[1]) == Pg.GetGuidByName("Carbine"),
                        Object.HasLabel(all[2], "Grenade"), #all
                    "#,
                )
                .eval()
                .unwrap();
        assert_eq!(
            (primary.as_str(), grenades, max, same, thrown, count),
            ("Carbine", 0, 9, true, true, 2)
        );
        let mut w = world.borrow_mut();
        assert_eq!(
            w.take_commands(),
            [
                Command::Loadout {
                    who: 1,
                    weapons: vec!["Carbine".into(), "Grenade".into()]
                },
                Command::Reserve {
                    who: 1,
                    weapon: "Carbine".into(),
                    rounds: 9
                }
            ]
        );
        w.set_reserve(1, "grenade", 4);
        let all = w.inventory.of(1).to_vec();
        assert_eq!(w.inventory.carried[&all[1]].reserve, 4);
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
    fn atmosphere_calls_reach_the_game_and_the_rest_are_stubs() {
        let (host, world) = setup();
        host.complete_libraries(&LIBRARIES).unwrap();
        host.lua()
            .load(
                r#"
                Graphics.Atmosphere.EnableImmediatelyChangeMode(true)
                Graphics.Atmosphere.SetSky("afternoon")
                Graphics.Atmosphere.ChangeLineRegionSetting(Pg.GetGuidByName("Mattias"), "night")
                Graphics.Camera.SetFocusParams(0, 0, 0.1, 4, 6, 1)
                Graphics.Effect.CameraFade(1, 0.5)
                "#,
            )
            .exec()
            .unwrap();
        assert_eq!(
            world.borrow_mut().take_commands(),
            [
                Command::AtmosphereImmediate(true),
                Command::Atmosphere {
                    region: 1,
                    setting: "night".into()
                }
            ]
        );
    }

    #[test]
    fn context_actions_are_offered_until_removed() {
        let (host, world) = setup();
        host.lua()
            .load(
                r#"
                -- WifPmcInterior's exit, its starter's talk, the wardrobe.
                Pg.AddContextAction(11, "[ContextAction.Exit]", 2, 0, 0, 255, 2, false)
                Pg.AddContextAction(12, "[ContextAction.Talk]", 4, 0, 0, 255, 4, false)
                Pg.AddContextAction(13, "[ContextAction.ChangeOutfit]", 1)
                Pg.AddContextAction(14, "[ContextAction.Enter]")
                Pg.RemoveContextAction(12)
                "#,
            )
            .exec()
            .unwrap();
        let w = world.borrow();
        let offered: Vec<(u32, &str, f32)> = w
            .context_actions
            .iter()
            .map(|(&g, a)| (g, a.text.as_str(), a.reach))
            .collect();
        assert_eq!(
            offered,
            [
                (11, "[ContextAction.Exit]", 2.0),
                (13, "[ContextAction.ChangeOutfit]", 1.0),
                (14, "[ContextAction.Enter]", CONTEXT_REACH),
            ]
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
        // The vehicle first, then its rider, as the scripts call it.
        host.lua().load("Vehicle.Exit(50, 1, true)").exec().unwrap();
        assert!(events.update_shared(0.1, &world).is_empty());
        let log: String = host.lua().globals().get("log").unwrap();
        assert_eq!(log, "car out ");
        assert_eq!(world.borrow().objects.get(&1).and_then(|o| o.vehicle), None);
    }

    #[test]
    fn asset_loads_call_back_with_their_data_unpacked() {
        let (host, world) = setup();
        host.lua()
            .load(
                r#"
                log = {}
                Pg.LoadAsset("preload", "animation")
                Pg.LoadAsset("fx", "facefxanimationset", function(name, timed_out)
                    table.insert(log, name .. " " .. tostring(timed_out))
                end, { "fx", false })
                local counter = { n = 1 }
                Pg.LoadAsset("bank", "soundbank", function(t)
                    t.n = t.n - 1
                    table.insert(log, "counter " .. t.n)
                end, { counter })
                Pg.UnloadAsset("preload", "animation")
                "#,
            )
            .exec()
            .unwrap();
        // Nothing yet: the load answers on the next tick.
        let count = |host: &ScriptHost| host.lua().load("return #log").eval::<i64>().unwrap();
        assert_eq!(count(&host), 0);
        assert!(answer_asset_loads(&world).is_empty());
        let log: Vec<String> = host
            .lua()
            .globals()
            .get::<Table>("log")
            .unwrap()
            .sequence_values()
            .map(Result::unwrap)
            .collect();
        assert_eq!(log, ["fx false", "counter 0"]);
        // Each once.
        assert!(answer_asset_loads(&world).is_empty());
        assert_eq!(count(&host), 2);
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
                target: None,
                haste: None
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
    fn an_any_boundary_tells_its_callback_each_way_she_crosses() {
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            let mut zone = Object::new("RGN_Zone", [100.0, 0.0, 100.0]);
            zone.faction = Some(m2_assets::name_hash("VZ"));
            w.insert(70, zone);
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
                crossed = {}
                local zone = Pg.GetGuidByName("RGN_Zone")
                faction = Object.GetName(Ai.GetFactionGuid(zone))
                Event.CreatePersistent(Event.Boundary,
                    {Player.GetLocalCharacter(), zone, "any", false},
                    function(tag, who, where, how)
                        crossed[#crossed + 1] = tag .. " " .. how
                    end, {"zone"})
                "#,
            )
            .exec()
            .unwrap();
        let faction: f64 = host.lua().globals().get("faction").unwrap();
        assert_eq!(faction as u32, m2_assets::name_hash("VZ"));
        let crossed = || -> Vec<String> { host.lua().globals().get("crossed").unwrap() };
        assert!(events.update_shared(0.1, &world).is_empty());
        assert!(crossed().is_empty());
        let walk = |at: [f32; 3]| {
            world.borrow_mut().objects.get_mut(&1).unwrap().position = at;
            assert!(events.update_shared(0.1, &world).is_empty());
        };
        walk([103.0, 0.0, 98.0]);
        walk([104.0, 0.0, 99.0]);
        walk([120.0, 0.0, 99.0]);
        walk([101.0, 0.0, 101.0]);
        assert_eq!(crossed(), ["zone enter", "zone exit", "zone enter"]);
    }

    #[test]
    fn vehicle_disguises_work_once_a_script_turns_them_on() {
        let (host, world) = setup();
        let ask = |code: &str| -> Value { host.lua().load(code).eval().unwrap() };
        assert_eq!(
            ask("return Player.GetVehicleDisguise()"),
            Value::Boolean(false)
        );
        assert_eq!(ask("return Player.GetVehicleDisguiseState({})"), Value::Nil);
        ask("Player.SetVehicleDisguise(true)");
        assert_eq!(
            ask("return Player.GetVehicleDisguise()"),
            Value::Boolean(true)
        );
        world.borrow_mut().disguise = Some((80.0, true));
        assert_eq!(
            ask("return Object.IsDisguised(Player.GetLocalCharacter())"),
            Value::Boolean(true)
        );
        ask("Player.SetVehicleDisguise(false)");
        assert_eq!(world.borrow().disguise, None);
        assert_eq!(ask("return Player.GetVehicleDisguiseState({})"), Value::Nil);
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
    fn a_support_helicopter_comes_with_the_driver_its_template_seats() {
        let (host, world) = setup();
        world
            .borrow_mut()
            .template_drivers
            .insert("ah1z (ewan)".into(), "Recruit Ewen Garret".into());
        let pilot: String = host
            .lua()
            .load(
                r#"local copter = Pg.Spawn("AH1Z (Ewan)", 0, 50, 0, 0)
                return Object.GetName(Vehicle.GetDriver(copter))"#,
            )
            .eval()
            .unwrap();
        assert_eq!(pilot, "Recruit Ewen Garret");
    }

    #[test]
    fn the_scripts_save_requests_reach_the_game_with_their_slots() {
        let (host, world) = setup();
        host.lua()
            .load(r#"Pg.SaveGame("retry") Sys.RequestAutosave(true, "VzaCon001", 600, 2)"#)
            .exec()
            .unwrap();
        let saves: Vec<Command> = world
            .borrow_mut()
            .take_commands()
            .into_iter()
            .filter(|c| matches!(c, Command::Save { .. }))
            .collect();
        assert_eq!(
            saves,
            [
                Command::Save {
                    slot: "retry".into()
                },
                Command::Save {
                    slot: AUTOSAVE.into()
                }
            ]
        );
    }

    #[test]
    fn a_goal_callback_hears_who_had_the_goal_and_that_it_was_done() {
        let (host, world) = setup();
        host.lua()
            .load(
                r#"Ai.Goal({ AIGuid = Sys.StringToGuid("0x1"), Goal = "HeliLand",
                    Location = { 1, 2, 3 }, Callback = function(me, pilot, result)
                        heard = { me, Object.GetName(pilot), result } end,
                    CallbackData = { "copter" } })"#,
            )
            .exec()
            .unwrap();
        assert!(goal_finished(&world, 1).is_empty());
        let (me, pilot, result): (String, String, i64) = host
            .lua()
            .load("return heard[1], heard[2], heard[3]")
            .eval()
            .unwrap();
        assert_eq!(
            (me.as_str(), pilot.as_str(), result),
            ("copter", "Mattias", 1)
        );
    }

    #[test]
    fn strikes_collect_what_is_near_by_kind_and_measure_to_points() {
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            let mut tank = Object::new("Tank", [20.0, 0.0, 0.0]);
            tank.labels.extend(["Vehicle".into(), "Tank".into()]);
            w.insert(60, tank);
            let mut far = Object::new("Tank", [500.0, 0.0, 0.0]);
            far.labels.push("Tank".into());
            w.insert(61, far);
        }
        let (tanks, ground, flying, d): (i64, i64, i64, f64) = host
            .lua()
            .load(
                r#"return #Pg.FastCollectTanks(0, 0, 0, 200),
                    #Pg.FastCollectGroundVehicles(0, 0, 0, 200),
                    #Pg.FastCollectFlying(0, 0, 0, 200),
                    Object.GetDistanceFrom(Sys.StringToGuid("0x1"), 10, 0, 8)"#,
            )
            .eval()
            .unwrap();
        // The tank and the jeep are ground vehicles; the far tank is out.
        assert_eq!((tanks, ground, flying), (1, 2, 0));
        assert!((d - 3.0).abs() < 1e-5);
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

    #[test]
    fn the_scripts_put_the_loading_screen_up() {
        let (host, world) = setup();
        world.borrow_mut().take_commands();
        host.lua()
            .load("Net.SetLoadingScreen(true) Net.SetLoadingScreen(false)")
            .exec()
            .unwrap();
        assert_eq!(
            world.borrow_mut().take_commands(),
            [Command::LoadingScreen(true), Command::LoadingScreen(false)]
        );
    }

    #[test]
    fn a_camera_nobody_here_holds_faces_north() {
        // The HUD's world places no one: the PDA's map still gets a yaw.
        let (host, _) = setup();
        let yaw: f64 = host
            .lua()
            .load("return Camera.GetYaw(Player.GetCamera(Sys.StringToGuid('0x7fff')))")
            .eval()
            .unwrap();
        assert_eq!(yaw, 0.0);
    }
}
