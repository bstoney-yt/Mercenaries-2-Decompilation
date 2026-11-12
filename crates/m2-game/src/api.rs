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
/// The Lua registry name `Disguise.Init`'s `InvestigatorCallback` is
/// kept by.
pub const INVESTIGATOR: &str = "m2.disguise_investigator";

/// 0x82446790: an AI came onto her disguise's watcher list, or left it:
/// calls the `InvestigatorCallback` with (player, ai, came).
pub fn investigator(lua: &Lua, player: Option<u32>, ai: u32, came: bool) -> mlua::Result<()> {
    if let Value::Function(f) = lua.named_registry_value::<Value>(INVESTIGATOR)? {
        f.call::<()>((guid_value(player), guid_value(Some(ai)), came))?;
    }
    Ok(())
}

/// `Object.GetWinchState`'s names, by state (0x823336E8), and
/// `Object.SetWinchState`'s (0x823337E8, by their hashes 0xA62D3A27,
/// 0x07D744CF, 0x0A765168, 0xBB522D10).
pub const WINCH_STATES: [&str; 4] = ["retracted", "deployed", "deploying", "retracting"];
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
    /// Seated in `vehicle` other than at its controls: a passenger,
    /// gunner or commander (`Vehicle.Enter`'s seat `"p"`, `"g"`, `"c"`,
    /// the letters `Vehicle.GetRiders` filters by), or she boarding the
    /// scripts' helicopter as a passenger. Never its driver
    /// ([`ScriptWorld::driver_of`]).
    pub not_driving: bool,
    /// It got in (or out) at once, put there by a script, rather than
    /// climbing in itself: seat events tell the two apart.
    pub seated_instantly: bool,
    pub player_controlled: bool,
    /// Simulated near the player rather than hibernating
    /// ([`ScriptWorld::update_hibernation`]).
    pub awake: bool,
    /// Its own hibernation distance, whole metres, when a script set one
    /// (`Object.SetHibernationDistance`); else [`HIBERNATION_DISTANCE`].
    /// From the 360 code: the engine keeps it per object as a 16-bit
    /// count, which `Object.GetHibernationDistance` reads back.
    pub hibernation_distance: Option<u16>,
    /// Hidden by the scripts (`Object.SetVisible(o, false)`): not on
    /// screen.
    pub hidden: bool,
    /// Fuel for the player's support (`Player.GetFuel`), in the units of
    /// `Player.GetFuelCapacity`.
    pub fuel: f32,
    /// The faction it belongs to (`Ai.GetFactionGuid`): the name hash of
    /// the faction's template (`"VZ"`), as the HUD's faction guids are.
    pub faction: Option<u32>,
    /// Who killed or destroyed it, when the game knows (her, for what her
    /// shots, blasts and rams kill): `ObjectDeath` passes it on
    /// ([`crate::events::World::killer`]).
    pub killer: Option<u32>,
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

    /// The faction it belongs to, by its template's name (`"VZ"`): its own
    /// (a zone's), else the faction its template's name begins with (`VZ Heavy (AA
    /// Missile)`, `Chinese Soldier`): inferred from the layers' person
    /// templates, as `m2-alpha`'s `npc::faction_of`; a template's own
    /// faction link isn't read.
    pub fn faction_name(&self) -> Option<&'static str> {
        if let Some(h) = self.faction {
            return FACTIONS.into_iter().find(|f| m2_assets::name_hash(f) == h);
        }
        let n = self.name.to_ascii_lowercase();
        [
            ("vz ", "VZ"),
            ("allied ", "Allied"),
            ("chinese ", "China"),
            ("china ", "China"),
            ("guerilla ", "Guerilla"),
            ("guerrilla ", "Guerilla"),
            ("oc ", "OC"),
            ("pirate ", "Pirate"),
        ]
        .into_iter()
        .find_map(|(prefix, f)| n.starts_with(prefix).then_some(f))
    }

    /// Its labels as name hashes: those scripts gave it and the data's.
    pub fn all_labels(&self) -> Vec<u32> {
        self.labels
            .iter()
            .map(|l| m2_assets::name_hash(l))
            .chain(self.label_hashes.iter().copied())
            .collect()
    }

    /// How near the player must come to wake it, metres: its own
    /// distance, else [`HIBERNATION_DISTANCE`].
    pub fn hibernation_distance(&self) -> f32 {
        self.hibernation_distance
            .map_or(HIBERNATION_DISTANCE as f32, f32::from)
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
    /// What a script asked of a vehicle ([`crate::vehicle_state`]).
    Vehicle(crate::vehicle_state::VehicleCommand),
    /// A hijack's own requests (`crate::hijack`).
    Hijack(crate::hijack::HijackCommand),
    /// Open (or close) vehicle `guid`'s door `door`, a part's name hash
    /// (`"pivot"`): `Vehicle.OpenDoor` and `Vehicle.CloseDoor`, which the
    /// 360 queues for the vehicle to swing.
    Door {
        guid: u32,
        door: u32,
        open: bool,
    },
    /// Turn vehicle `guid`'s parts of kind `part` (1 to 8,
    /// [`vehicle_part`]) on or off: `Vehicle.SetParts`.
    Part {
        guid: u32,
        part: u8,
        on: bool,
    },
    /// Drop every goal person `guid` has (`Ai.RemoveGoal` with handle 0).
    ClearGoals {
        guid: u32,
    },
    /// Set (or clear) AI state `state` (`"Vip"`) on person `guid`
    /// (`Ai.SetState`).
    AiState {
        guid: u32,
        state: String,
        on: bool,
    },
    /// `Ai.AddInfraction(who, faction, amount)`: the faction's people who
    /// perceive `who` hold an infraction (the engine's kind 6) against
    /// them, `amount` of it; `faction` is its template's guid, or 0 for
    /// every faction.
    Infraction {
        who: u32,
        faction: u32,
        amount: f32,
    },
    /// Create `what` (an object template's name, such as `"Veyron"` or
    /// `"Explosion (Grenade)"`) as `guid`.
    Spawn {
        guid: u32,
        what: String,
        position: [f32; 3],
        yaw: f32,
    },
    /// What the natives of [crate::ported_player] ask of the game.
    Ported(crate::ported_player::PortedCommand),
    Remove(u32),
    Kill(u32),
    /// `Object.Revive(object, fraction)`: back in play, at that share of
    /// its most health.
    Revive {
        guid: u32,
        health: f32,
    },
    /// `Object.ApplyImpulse`: a push (kg m/s) through its centre of
    /// mass, world space, or its own space when `local` (the game turns
    /// it by the object's whole transform, as 0x82331638 does).
    Impulse {
        guid: u32,
        impulse: [f32; 3],
        local: bool,
    },
    /// `Object.ApplyPointImpulse`: a push (kg m/s) at a point, both
    /// world space, or both its own space when `local` (the game turns
    /// the push and places the point by the object's transform, as
    /// 0x82331748 does).
    PointImpulse {
        guid: u32,
        impulse: [f32; 3],
        point: [f32; 3],
        local: bool,
    },
    /// `Player.SetScopeEnabled`: whether she may use a scope now.
    ScopeEnabled(bool),
    /// `Human.SetFireLock`: whether the person may not fire.
    FireLock {
        guid: u32,
        on: bool,
    },
    /// `Sound.SetMasterVolume(volume, seconds)`.
    MasterVolume {
        volume: f32,
        seconds: f32,
    },
    /// `Camera.SetYaw`: the camera turned to this yaw (radians).
    CameraYaw(f32),
    /// `ObjectState.StartEmitter` / `StopEmitter`: an emitter of an
    /// object's state, by the two name hashes the scripts give.
    Emitter {
        guid: u32,
        state: u32,
        emitter: u32,
        on: bool,
    },
    /// `Object.PlayMaterialAnimation(object, name, loop)`.
    MaterialAnimation {
        guid: u32,
        name: String,
        looped: bool,
    },
    /// `Graphics.SetShadowBaseDistance(metres)`.
    ShadowBaseDistance(f32),
    /// `VO.SetCinematicMode(on)`.
    VoCinematic(bool),
    /// The armies go for `guid` before anything else
    /// (`Ai.SetPriorityTarget`).
    PriorityTarget(u32),
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
        /// A `PathMove`'s own settings ([`crate::path_order`]); `None`
        /// for other goals.
        path: Option<crate::path_order::PathOrder>,
    },
    ExitVehicle(u32),
    /// A person keeps near `target` (`Ai.Role` `Follow`: a VIP being
    /// delivered, `MrxFollow`), within `near` metres (its
    /// `MoveDistance`); `None`: no longer following (another role).
    Follow {
        guid: u32,
        target: Option<u32>,
        near: f32,
    },
    /// Open a gate or door (`Object.OpenGate`): PmcCon001's villa doors
    /// as the player comes near them.
    OpenGate(u32),
    /// A road junction is opened to traffic or closed (`Pg.EnableIntersection(on,
    /// object)`): ChiCon002 closes the two segments of the Maracaibo
    /// bridge it is about to bring down.
    Intersection {
        guid: u32,
        on: bool,
    },
    /// An AI takes part in the living world or stops (`Ai.LivingWorld`
    /// with `Attrib = "LivingWorldBehaviour"`): `MrxFollow` turns it off
    /// for someone following her.
    LivingWorld {
        guid: u32,
        on: bool,
    },
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
    /// An AI's haste from here on (`Ai.SetHaste(ai, haste)`), held to
    /// 0..=2 as the 360 code does (82311968).
    SetHaste {
        guid: u32,
        haste: f32,
    },
    /// An AI keeps within `radius` metres of a point (`Ai.Anchor`): `at`,
    /// or where `target` is, as the call gave one (from the 360 code,
    /// 82310EB8: `AnchorGuid`, else `AnchorPosition`, else where the AI
    /// itself stands).
    Anchor {
        guid: u32,
        radius: f32,
        target: Option<u32>,
        at: Option<[f32; 3]>,
    },
    /// An object fades from sight over `seconds` (`Object.FadeOut(object,
    /// seconds, flag)`); what the flag asks isn't known (OilCon001 passes
    /// true as its executive leaves).
    FadeOut {
        guid: u32,
        seconds: f32,
        flag: bool,
    },
    /// An ambience by name starts or stops (`Sound.CueAmbience`,
    /// `Sound.StopAmbience`).
    Ambience {
        name: String,
        on: bool,
    },
    /// The spawners attached to object `guid` are changed
    /// (`Ai.TweakAttachedSpawners`, `...InGroup`): the object's whole
    /// record as it stands after the call ([`crate::spawner_tweak`]); one
    /// with nothing set means the object has none any more.
    TweakSpawners {
        guid: u32,
        record: Box<crate::spawner_tweak::SpawnerRecord>,
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
    /// Character `guid` plays keyed action `action` (`Human.DoAction`).
    DoAction {
        guid: u32,
        action: String,
    },
    /// Character `guid` puts down whom it carries (`Human.Drop`).
    Drop(u32),
    /// Play the clip named `clip` once on character `guid`, holding them
    /// in it (`Human.PlayRawAnimation`: the faction HQs' animated
    /// briefings); the game reports the clip's events back
    /// ([`crate::mission::Mission::animation_event`]).
    RawClip {
        guid: u32,
        clip: String,
    },
    /// Character `guid` wears model `model` from now on
    /// (`Player.SetOutfit`: the wardrobe's pick, `pmc_hum_jen_v3`).
    Outfit {
        guid: u32,
        model: String,
    },
    /// Seat a character in a vehicle (seat as the scripts name it, `"d"`
    /// for the driver); `instant` skips the climb-in.
    EnterVehicle {
        guid: u32,
        vehicle: u32,
        seat: String,
        instant: bool,
    },
    /// A script set a helicopter's winch state (`Object.SetWinchState`:
    /// 0 retracted, 1 deployed, 2 deploying, 3 retracting).
    WinchState {
        carrier: u32,
        state: u8,
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
    /// `guid` can (no longer) be killed, though it can be hurt: on while
    /// the scripts hold any reason for it (`Object.SetUnkillable`).
    Unkillable {
        guid: u32,
        on: bool,
    },
    /// Play a music cue (`mu_fac_pmc_explore_01`), looped unless it is a
    /// one-shot, in place of what plays; `None` stops the music.
    Music(Option<String>),
    /// Run the world's sounds through this reverb, or none
    /// (`Sound.SetReverb`, `Sound.SetReverbPreset`).
    Reverb(Option<m2_engine::reverb::ReverbPreset>),
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
    /// A voice bank the scripts load (`true`) or let go (`false`) by its
    /// localised name (`vo_job_pmc.english`): `MrxSoundBanks.LoadTempBank`
    /// and `LoadSoundBank` for an HQ's briefings and a contract's lines,
    /// whose cue tables and sounds are not among the resident sets.
    VoiceBank {
        name: String,
        load: bool,
    },
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
    /// The player's aiming is on or off (`Player.SetAimMode(player, on)`):
    /// the HQ turns it off as she goes in, with `Human.DisableWeapons`,
    /// and on again as she leaves; off, she has no reticle.
    AimMode(bool),
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

/// Reach (metres) for a context action whose call gives none. From the
/// 360 code: `Pg.AddContextAction` (0x823381E8) defaults its reach to the
/// constant 2.0 at 0x820C77A4.
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
    /// The engine's own pursuit, which sends the faction's units
    /// ([`PursuitClock`]).
    pub clock: PursuitClock,
}

/// The engine's pursuit, as the 360 code keeps it (its manager at
/// 0x822371F8..0x8223A900): one faction at a time, at a level (0 to 2:
/// the pursuit tables' `Threat1` to `Threat3`) that its seconds left
/// decide. While it is active the faction's units are sent up to the
/// table's count for the level (the game's side, `crate::reinforce`).
///
/// From the 360 code: the defaults (0x82236410: level times 200 and 400,
/// at most 700 s and level 2), the level a number of seconds makes
/// (0x82238670: under 1 none, under the first time 0, from the second 2,
/// else 1), the seconds a level starts at (0x822384C0: the first time's
/// half, at most 100; the middle of the two times, at most the first
/// plus 100; the middle of the second and the most, at most the second
/// plus 100), `SetPursuit` (0x82238398: level 1 to 3 as 0 to 2, an
/// active one kept unless forced), `SetPursuitSeconds` (0x82238570: its
/// seconds, the level they make), a lock (0x823390C8) and the clock
/// (0x82238020: a second off each whole second unlocked, the level
/// dropping with it, over once below level 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PursuitClock {
    /// Under way (`GetPursuitState().Active`).
    pub active: bool,
    /// Held where it is (`LockPursuit`).
    pub locked: bool,
    /// The faction (its template's guid).
    pub faction: Option<u32>,
    /// Seconds left, whole.
    pub seconds: i32,
    /// Its level, 0 to [`max_level`](Self::max_level); -1 when none.
    pub level: i32,
    /// Where levels 1 and 2 start, in seconds left (`SetPursuitLevelTimes`).
    pub times: (i32, i32),
    /// The most seconds it may have.
    pub max_seconds: i32,
    /// The highest level it may reach.
    pub max_level: i32,
    /// The part of a second run since the last whole one.
    carry: f32,
}

impl Default for PursuitClock {
    fn default() -> Self {
        Self {
            active: false,
            locked: false,
            faction: None,
            seconds: 0,
            level: -1,
            times: (200, 400),
            max_seconds: 700,
            max_level: 2,
            carry: 0.0,
        }
    }
}

impl PursuitClock {
    /// The level `seconds` left make (-1: over).
    pub fn level_of(&self, seconds: i32) -> i32 {
        if seconds < 1 {
            -1
        } else if seconds < self.times.0 {
            0
        } else if seconds >= self.times.1 {
            self.max_level.min(2)
        } else {
            self.max_level.min(1)
        }
    }

    /// The seconds a pursuit at `level` starts with.
    pub fn start_of(&self, level: i32) -> i32 {
        let (a, b) = self.times;
        let s = match level {
            0 => (a / 2).min(100),
            1 => ((a + b) / 2).min(a + 100),
            2 => ((b + self.max_seconds) / 2).min(b + 100),
            _ => 0,
        };
        s.min(self.max_seconds)
    }

    /// Ends it.
    pub fn end(&mut self) {
        self.active = false;
        self.locked = false;
        self.faction = None;
        self.seconds = 0;
        self.level = -1;
        self.carry = 0.0;
    }

    /// `SetPursuit(faction, level, force)`: starts one at `level` (1 to 3);
    /// one under way stays unless `force`. False if it didn't start.
    pub fn set(&mut self, faction: u32, level: i32, force: bool) -> bool {
        if self.locked || (self.active && !force) || level < 1 {
            return false;
        }
        self.end();
        self.level = (level - 1).min(self.max_level);
        self.seconds = self.start_of(self.level);
        self.faction = Some(faction);
        self.active = true;
        true
    }

    /// `SetPursuitSeconds(faction, seconds, force)`: starts one with
    /// `seconds` left, at the level they make.
    pub fn set_seconds(&mut self, faction: u32, seconds: i32, force: bool) -> bool {
        if self.locked || (self.active && !force) || seconds < 1 {
            return false;
        }
        self.end();
        self.seconds = seconds.min(self.max_seconds);
        self.level = self.level_of(self.seconds);
        self.faction = Some(faction);
        self.active = true;
        true
    }

    /// `LockPursuit(faction, level)`: held at `level` until cleared.
    pub fn lock(&mut self, faction: u32, level: i32) {
        self.end();
        self.level = (level - 1).clamp(0, self.max_level);
        self.seconds = self.start_of(self.level);
        self.faction = Some(faction);
        self.active = true;
        self.locked = true;
    }

    /// Runs on by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        if !self.active {
            return;
        }
        self.carry += dt;
        let whole = self.carry.floor();
        self.carry -= whole;
        if whole < 1.0 || self.locked {
            return;
        }
        self.seconds -= whole as i32;
        while self.active && self.level_of(self.seconds) < self.level {
            if self.level == 0 {
                self.end();
            } else {
                self.level -= 1;
                self.seconds = self.start_of(self.level);
            }
        }
    }

    /// `GetPursuitState().Level`: its level counted from 1, 0 when none.
    pub fn shown_level(&self) -> i64 {
        if self.active {
            i64::from(self.level + 1)
        } else {
            0
        }
    }
}

impl Pursuit {
    /// Seconds a pursuit of `level` lasts.
    pub fn seconds(&self, level: i64) -> f32 {
        let (low, high) = self.times.unwrap_or((120.0, 300.0));
        if level >= 3 { high } else { low }
    }
}

/// How long after a honk the friendly faction gates know she is there,
/// seconds (invented: the gate script needs her within 20 m to judge
/// her, and the game's text says only that she must honk).
pub const HONK_HEARD: f32 = 3.0;

/// The reserve that stands for infinite ammunition
/// (`Object.SetInfiniteAmmo`), rounds (invented).
pub const INFINITE_RESERVE: u32 = 999;

/// `Pg.GetTetherDiameterStart`'s answer, metres (from the 360 code,
/// 82446B68).
pub const TETHER_DIAMETER_START: f64 = 400.0;

/// The scripts' view of the game, and what they asked of it.
#[derive(Debug, Default)]
pub struct ScriptWorld {
    pub objects: HashMap<u32, Object>,
    /// Each object's mass as its physics reports it, kg
    /// (`Object.GetMass`), as the game last said: its vehicles'.
    pub masses: HashMap<u32, f32>,
    /// Each character's last raw clip (`Human.PlayRawAnimation`): seconds
    /// left once the game has said its length ([`ScriptWorld::raw_clip_length`]),
    /// `None` until then.
    pub raw_clips: HashMap<u32, Option<f32>>,
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
    /// Who may ride something: those riding at the last pass over every
    /// object ([`Self::update_hibernation`]) and those put aboard or
    /// spawned aboard since (a few may have got out). Lets the crews
    /// riding be found without going over every object each tick, which
    /// with a contract's 76,000 objects cost a fifth of play's time (perf
    /// round 1).
    pub riders: Vec<u32>,
    /// What `Sys.GetAssetRequestMax` answers; scripts raise it while
    /// layers load and put it back.
    pub asset_request_max: u32,
    /// The game state (`Sys.RequestGameState`), `"Game"` while playing.
    pub game_state: String,
    /// What the system, sound and player ports of missing natives keep
    /// ([crate::ported_sys]).
    pub sys: crate::ported_sys::SysState,
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
    pub(crate) asset_loads: Vec<(Function, Value)>,
    /// The outlines of region objects (`LineRegion`), by the region's guid;
    /// placed at the object's position and yaw.
    pub regions: HashMap<u32, m2_assets::Region>,
    /// The object link table ([`crate::links`]).
    pub links: crate::links::Links,
    /// The layers' data-authored links by parent entity: (link name hash,
    /// child entity), made into [`Self::links`] as the parent is placed.
    pub physical_links: HashMap<u32, Vec<(u32, u32)>>,
    /// The sound engine's state the scripts set (`crate::script_sound`).
    pub sound: crate::script_sound::SoundSettings,
    /// The level and shell names and the post-processing values the
    /// scripts set (`crate::script_shell`).
    pub shell: crate::script_shell::ShellState,
    /// `Sys.RequiredAsset`'s list and `CameraFade` (`crate::script_effects`).
    pub effects: crate::script_effects::EffectsState,
    /// What `Pg.GetAllLandingZones(kind)` answers, by kind: the zones by
    /// their number. The transit script reads kinds 1 and 2 (where each
    /// player lands) as tables keyed alike.
    pub landing_zones: HashMap<i64, BTreeMap<i64, u32>>,
    /// Cargo on winches: what carries each, `None` once let go.
    pub winch: HashMap<u32, Option<u32>>,
    /// Vehicles' doors the scripts opened (true) or closed, by (vehicle,
    /// door's name hash).
    pub doors: HashMap<(u32, u32), bool>,
    /// Vehicles' parts the scripts turned on (true) or off, by (vehicle,
    /// kind of part, [`vehicle_part`]).
    pub parts: HashMap<(u32, u8), bool>,
    /// AI states the scripts set (`Ai.SetState`), by (person, state in
    /// lower case).
    pub ai_states: HashMap<(u32, String), bool>,
    /// `Pg.StartHeliWaveSpawner`'s waves under way.
    pub heli_waves: crate::heliwaves::HeliWaves,
    /// How each person regards another (`Ai.SetFeeling`), by (person,
    /// other).
    pub feelings: HashMap<(u32, u32), f32>,
    /// The last goal handle `Ai.Goal` gave out.
    pub goal_handles: u32,
    /// How many reasons the scripts gave for her scope to be off
    /// (`Player.SetScopeEnabled`); she may use it at 0.
    pub scope_off: u32,
    /// Textures the scripts asked the GUI to load (`Gui.LoadTexture`), by
    /// name hash.
    pub textures_asked: Vec<u32>,
    /// Whether the other players' markers show (`Gui.EnablePlayerMarkers`),
    /// once the scripts said.
    pub player_markers: Option<bool>,
    /// The PDA map's callbacks for its two modes
    /// (`Player.SetPDAMapModeCallback`).
    pub pda_map_callbacks: [Option<Function>; 2],
    /// `Player.SetWaitForInGame`: the player waits for the game proper.
    pub wait_for_in_game: bool,
    /// Places people kept by `Human.PersistTransform`: place and yaw.
    pub persisted: HashMap<u32, ([f32; 3], f32)>,
    /// The VO system's (speaker, priority) sequences (`VO.AddSequence`).
    pub vo_sequences: Vec<(u32, i32)>,
    /// The helicopters with a winch (a `Winch` component; the 360's
    /// `PhysicsActorWinch` store 0x83657220) and its state (+0x140: 0
    /// retracted, 1 deployed, 2 deploying, 3 retracting).
    pub winches: HashMap<u32, u8>,
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
    /// The developer's skip-to-mission switch (`Sys.GetSkipMission`, read
    /// from the game's INI in the original): the bootstrap's new game
    /// hands it to `MrxCheatBootstrap.EnableSkipMode`, and the mission
    /// flow then completes each story contract it unlocks until this one
    /// (read from the game's own scripts, `MrxMissionFlow.UnlockMission`).
    pub skip_mission: Option<String>,
    /// With [`skip_mission`](Self::skip_mission), whether it stops at the
    /// contract's briefing (`Sys.GetINIBriefing`).
    pub skip_briefing: bool,
    /// The score.
    pub music: crate::music::Music,
    /// The environmental reverb.
    pub reverb: crate::reverb::Reverb,
    /// What the single-player ports of missing natives keep
    /// ([crate::ported_sp]).
    pub sp: crate::ported_sp::SpState,
    /// Why each object is invincible (`Object.SetInvincible` reasons).
    pub invincible: HashMap<u32, std::collections::BTreeSet<String>>,
    /// `Object.SetUnkillable`'s reason bits by object ([`crate::ported_physics`]).
    pub unkillable: HashMap<u32, u16>,
    /// Each object's physics body kind as the 360 numbers them (0 prop,
    /// 1 human, 2 car, 3 tank, 4 helicopter, 8 jet, 10 boat...), as the game
    /// says (`Object.GetPhysicsType`).
    pub physics_kinds: HashMap<u32, u8>,
    /// The play area's boundaries (`Player.AddBoundary`: story-stage
    /// regions such as `BoundaryIntro`), the callback told when the player
    /// leaves or comes back, and whether the player is out.
    pub boundaries: Vec<u32>,
    pub boundary_callback: Option<Function>,
    /// What the setter ports of missing natives keep ([crate::ported_misc]).
    pub misc: crate::ported_misc::MiscState,
    /// The alarms in play and their messages ([crate::alarm]).
    pub alarms: crate::alarm::Alarms,
    pub outside: bool,
    /// What the initialiser ports of missing natives keep ([crate::ported_init]).
    pub init: crate::ported_init::InitState,
    /// The point under her reticle and the object there, as the game
    /// keeps them (`Player.GetTargetUnderReticle`: [`crate::player_target`]).
    pub reticle_target: Option<([f32; 3], Option<u32>)>,
    /// Templates' labels (name hashes) by name in lower case, for spawns.
    pub template_labels: HashMap<String, Vec<u32>>,
    /// Templates' cash values by name in lower case ([`crate::cash`]).
    pub template_cash: HashMap<String, i32>,
    /// Every template the template database names, in lower case: the
    /// names `Pg.GetGuidByName` answers with a template's guid
    /// ([`ScriptWorld::is_template`]).
    pub template_names: std::collections::HashSet<String>,
    /// Who drives each template's vehicle, by name in lower case, as its
    /// `Rider` in the driver's seat names them: a vehicle spawned without
    /// "(Driver)" in its name comes with them ([`ScriptWorld::spawn`]).
    pub template_drivers: HashMap<String, String>,
    /// Who else rides each template's vehicle (its other `Rider`
    /// records): a `(Full)` vehicle spawned comes with them too.
    pub template_passengers: HashMap<String, Vec<String>>,
    /// Every layer entity's place, loaded or not; and the layers' markers'
    /// guids by name in lower case ([`crate::mission::Entity::is_marker`]):
    /// `Pg.GetGuidByName` finds markers (landing zones) of layers that
    /// aren't in, and scripts measure to them. Any other object of a layer
    /// that isn't in is not found by name: from samples, the shipped HQ
    /// scripts spawn each faction's interior when no `HqInterior` exists
    /// (`MrxUtil.SpawnActor`), so the PMC's, of its own layer, must not
    /// stand in for it once that layer is out.
    pub entities: HashMap<u32, [f32; 3]>,
    /// The world box round the collision of those the game moves loose
    /// (goods riding a vehicle's bed), lowest corner then highest: what
    /// `Pg.GetObjectsInArea` meets of them ([`crate::Mission::set_bounds`]).
    pub bounds: HashMap<u32, ([f32; 3], [f32; 3])>,
    pub entity_names: HashMap<String, u32>,
    /// A save's script data to load, as Lua text ([`crate::savedata`]):
    /// while it is here `Pg.LoadGame` answers that a save is loading.
    pub saved: Option<String>,
    /// `Player.SetHealthClamp` and `Player.SetSurvivalMode`, as last set.
    pub health_clamp: bool,
    pub survival_mode: bool,
    /// The weapons people carry.
    pub inventory: crate::inventory::Inventory,
    /// Faction relations, -100 to 100, by (template, template) names in
    /// lower case, in order: the templates' own `Relationship` records,
    /// and what `Ai.SetRelation` wrote over them.
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
    /// Characters' human states (`Stance.Action`; `Upright.Idle` when
    /// none is set), their changes for `HumanStateTransition` events, and
    /// who carries whom (`Human.IsCarrying`) ([`crate::mission::Mission::set_human_state`]).
    pub human_states: HashMap<u32, String>,
    pub state_changes: Vec<crate::events::StateChange>,
    /// Objects a script killed (`Object.Kill` of a live one) whose own
    /// script has not heard of its death yet
    /// ([`crate::mission::Mission::tick`] tells it: `OnDeath`).
    pub script_killed: Vec<u32>,
    /// What the scripts have changed of each object's attached spawners,
    /// by object ([`crate::spawner_tweak`]).
    pub spawner_records: HashMap<u32, crate::spawner_tweak::SpawnerRecord>,
    /// How fast objects move, m/s, as the game last said
    /// ([`crate::mission::Mission::set_speed`]); none for one at rest or
    /// never reported (`Object.GetVelocity`).
    pub speeds: HashMap<u32, f32>,
    /// What characters did with their weapons, for `WeaponEvent` events
    /// ([`crate::mission::Mission::weapon_event`]).
    pub weapon_events: Vec<crate::events::WeaponChange>,
    /// The points of the layers' path entities in the world, by guid
    /// (`Pg.GetDistantSpawnPointOnPath` walks one).
    pub paths: HashMap<u32, Vec<[f32; 3]>>,
    pub carrying: HashMap<u32, u32>,
    /// Events met in the clips the scripts play by name
    /// (`Human.PlayRawAnimation`: the briefings' `CinematicStart` and
    /// `CinematicEnd`), by who plays them, with their game time
    /// ([`crate::events::World::animation_events`]).
    pub animation_events: Vec<(f64, u32, String)>,
    /// The costume the player's profile keeps (`Player.GetProfileCostume`,
    /// `SetProfileCostume`: the wardrobe's pick, counted from 0).
    pub profile_costume: f64,
    /// The scripts let the player grapple (`Player.SetGrappleEnabled`:
    /// on once the PMC has a mechanic, 01a266d2).
    pub grapple_enabled: bool,
    /// The player is on her grappling hook's line (`Human.IsGrappling`),
    /// as the game last said.
    pub grappling: bool,
    /// The crews the layers' vehicles came into play with, by vehicle:
    /// (seat, rider template), as each entity's own `Rider` records say
    /// (the HQ interior's `HqInterior` seats Fiona, its PMC boss), else
    /// its template's.
    pub crews: HashMap<u32, Vec<(String, String)>>,
    /// The riders those crews came in as, each with what it came in on: a
    /// rider out of its seat is still known by it (the HQ's starter, out
    /// of her seat for her briefing).
    pub crew_riders: HashMap<u32, u32>,
    /// The template each of the layers' objects in play was placed as, by
    /// object ([`crate::mission::Entity::template`]).
    pub placed_as: HashMap<u32, String>,
    /// The player is in a scene of the scripts' (`Player.SetCinematicMode`).
    pub cinematic: bool,
    /// The scripts' camera while a scene has it ([`crate::script_camera`]).
    pub script_camera: Option<crate::script_camera::Cinematic>,
    /// Characters swimming, as the game last said (`Human.IsSwimming`).
    pub swimming: HashSet<u32>,
    /// Witnesses reporting her, and what each faction has against her
    /// (`Report`, [`crate::report`]).
    pub reports: crate::report::Reports,
    /// The pursuit as the scripts set it ([`Pursuit`]).
    pub pursuit: Pursuit,
    /// The `Sys` flags: build switches and the player's options.
    pub sys_flags: crate::script_flags::SysFlags,
    /// The engine's game and real clocks (`Sys.MainTime`, the time
    /// stamps), stepped each frame.
    pub clocks: crate::script_time::Clocks,
    /// Strikes and designators under way.
    pub strikes: crate::airstrike::Strikes,
    /// Whether vehicle disguises work (`Player.SetVehicleDisguise`): the
    /// mission flow turns them off for a new game and back on only once
    /// OilCon021 is done, so off until a script says otherwise.
    pub vehicle_disguise: bool,
    /// `Player.SetVehicleControlsLock`: her vehicle's controls locked.
    pub vehicle_controls_locked: bool,
    /// `Player.VehicleDisguise`'s `AutoRemove`.
    pub vehicle_disguise_auto_remove: bool,
    /// The vehicle entrance markers' settings, by player
    /// ([`crate::vehicle_state::EntranceMarkers`]).
    pub entrance_markers: [crate::vehicle_state::EntranceMarkers; 2],
    /// Her disguise while she drives another faction's vehicle with
    /// disguises on: how much is left (0..=100) and whether it holds.
    /// `None` out of one.
    pub disguise: Option<(f32, bool)>,
    /// `Player.GetVehicleDisguiseState`'s second answer: the owner-chain
    /// root she is seen as (0x82445778), as the game last said.
    pub disguise_seen_as: Option<u32>,
    /// The friendly faction gates in play: objects the engine runs
    /// `friendlygate` on ([`crate::mission::OBJECT_SCRIPTS`]).
    pub gates: HashSet<u32>,
    /// Gates the scripts shut (`Object.CloseGate`) and haven't opened
    /// since: fully closed at once (inferred: the engine here plays no
    /// closing, so the `"gateFullyClosed"` physics event holds as soon as
    /// they are shut).
    pub shut: HashSet<u32>,
    /// Objects the scripts removed (`Object.Remove`) while alive: gone,
    /// not dead, so an `ObjectDeath` on them never fires (inferred:
    /// JetCon001's death events on its bunker busters outlive their
    /// pickup, `Event.Delete` being handed their names, and the
    /// munitions taken up are removed; were removal a death, the
    /// contract would cancel itself every time).
    pub removed_alive: HashSet<u32>,
    /// Seconds the gates still hear her last honk ([`HONK_HEARD`]): until
    /// then they know her for who she is, and she is a candidate for
    /// their opening as their script judges her.
    pub honk_heard: f32,
    /// The player's camera as the game last fed it; `None` until it has.
    pub camera: Option<Camera>,
    /// Objects the game found behind something solid from the camera at
    /// its last look (it looks at those `ObjectIsVisible` events wait
    /// for: [`crate::mission::Mission::sight_watched`]).
    pub unseen: HashSet<u32>,
    /// The vehicles off the ground, as the game last said
    /// ([`World::grounded`]: anything not here stands on it).
    pub airborne: HashSet<u32>,
    /// The vehicles lying flipped, as the game last said
    /// (`Vehicle.IsFlipped`).
    pub flipped: HashSet<u32>,
    /// The vehicles' seats and who sits in which
    /// ([`crate::vehicle_seats`]).
    pub seats: crate::vehicle_seats::Seats,
}

impl ScriptWorld {
    /// Merges a call's spawner settings into object `guid`'s record, for
    /// one group or all ([`crate::spawner_tweak`]), and tells the game
    /// the record as it now stands. A record left with nothing set is
    /// dropped.
    pub fn tweak_spawners(
        &mut self,
        guid: u32,
        group: Option<usize>,
        tweak: &crate::spawner_tweak::GroupTweak,
    ) {
        let record = self.spawner_records.entry(guid).or_default();
        record.merge(group, tweak);
        let record = record.clone();
        if record.is_empty() {
            self.spawner_records.remove(&guid);
        }
        self.commands.push(Command::TweakSpawners {
            guid,
            record: Box::new(record),
        });
    }

    /// Whether world spot (x, z) is out of the play area
    /// (`Player.IsPositionOutBoundary`).
    ///
    /// From the 360 code (0x8233d860, 0x8243e500): the player's play-area
    /// boundaries are tested in turn, and the spot is in bounds if any of
    /// them holds it. With no boundaries at all, nothing is out of bounds.
    /// The engine keeps exclusion boundaries in a second list (0x8243d4d0
    /// files a boundary there when its object is marked as one), and this
    /// test skips that list. It also tests only regions of kinds 2 and 4.
    ///
    /// From samples: every region shape in `vz.wad` (625) is of kind 2,
    /// so that filter passes them all here. Exclusions are told by name
    /// (see `mission::is_exclusion`).
    pub fn position_out_of_bounds(&self, x: f32, z: f32) -> bool {
        let mut any = false;
        for &b in &self.boundaries {
            let Some(at) = self.objects.get(&b) else {
                continue;
            };
            if crate::mission::is_exclusion(&at.name) {
                continue;
            }
            let Some(region) = self.regions.get(&b) else {
                continue;
            };
            any = true;
            if region.contains([at.position[0], at.position[2]], at.yaw, x, z) {
                return false;
            }
        }
        any
    }

    /// Adds (or replaces) an object scripts can name.
    pub fn insert(&mut self, guid: u32, object: Object) {
        if object.vehicle.is_some() {
            self.riders.push(guid);
        }
        self.objects.insert(guid, object);
    }

    /// Puts `guid` aboard `vehicle` (`seated_instantly` as `instant`), if
    /// it is an object.
    pub fn put_aboard(&mut self, guid: u32, vehicle: u32, instant: bool) {
        self.seat(guid, vehicle, instant, false);
    }

    /// Puts `guid` aboard `vehicle` as [`Self::put_aboard`], `not_driving`
    /// when it takes a seat other than the driver's ([`Object::not_driving`]).
    pub fn seat(&mut self, guid: u32, vehicle: u32, instant: bool, not_driving: bool) {
        self.seat_named(guid, vehicle, instant, not_driving, None);
    }

    /// [`Self::seat`] in the seat named `name` (its `SeatLink` `Name`,
    /// hashed), when the vehicle has one so named.
    pub fn seat_named(
        &mut self,
        guid: u32,
        vehicle: u32,
        instant: bool,
        not_driving: bool,
        name: Option<u32>,
    ) {
        if let Some(o) = self.objects.get_mut(&guid) {
            o.vehicle = Some(vehicle);
            o.seated_instantly = instant;
            o.not_driving = not_driving;
            self.riders.push(guid);
            self.seats.sit(guid, vehicle, name, !not_driving);
        }
    }

    /// Who drives `vehicle`, as `Vehicle.GetDriver` answers: the 360's
    /// (0x823467C8, through 0x8229C958) walks the vehicle's riders and
    /// returns the one in the driver's seat, never a passenger, and nil
    /// when that seat is empty. Riders whose seat isn't known here stand
    /// for the driver as before, the first made (lowest guid) first: a
    /// crewed spawn's driver is made before its passengers.
    pub fn driver_of(&self, vehicle: u32) -> Option<u32> {
        self.objects
            .iter()
            .filter(|(_, o)| o.vehicle == Some(vehicle) && !o.not_driving)
            .map(|(&g, _)| g)
            .min()
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

    /// The template whose guid `guid` is (a faction's, `"VZ"`), if any.
    pub fn template_name(&self, guid: u32) -> Option<&str> {
        self.templates
            .iter()
            .find(|(_, g)| **g == guid)
            .map(|(n, _)| n.as_str())
    }

    /// Whether `name` is an object template's: one the template database
    /// names (`"Supply Drop (GL)"`, `"AMX30"`, the factions' `"OC"`). A
    /// name that only looks like one is not: `"HqInterior"`, which the
    /// HQ scripts give the interiors they spawn.
    pub fn is_template(&self, name: &str) -> bool {
        self.template_names.contains(&name.to_ascii_lowercase())
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
        // A full one (`"Ka29b (Full)"`, a soldier delivery's) comes with its
        // passengers too, as its other `Rider` records name them
        // (inferred from the name: `MrxSoldierDelivery` takes them out with
        // `Ai.Deploy` once it has landed).
        if lower.contains("(full)") {
            for rider in self
                .template_passengers
                .get(&lower)
                .cloned()
                .unwrap_or_default()
            {
                let g = self.spawn_guid();
                let mut o = Object::new(&rider, position);
                o.vehicle = Some(guid);
                o.seated_instantly = true;
                if let Some(l) = self.template_labels.get(&rider.to_ascii_lowercase()) {
                    o.label_hashes.clone_from(l);
                }
                self.insert(g, o);
                self.commands.push(Command::Spawn {
                    guid: g,
                    what: rider,
                    position,
                    yaw,
                });
                self.commands.push(Command::EnterVehicle {
                    guid: g,
                    vehicle: guid,
                    seat: "p".into(),
                    instant: true,
                });
            }
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

    /// The game says how long `guid`'s last raw clip is (seconds): it
    /// runs out from now.
    pub fn raw_clip_length(&mut self, guid: u32, seconds: f32) {
        self.raw_clips.insert(guid, Some(seconds));
    }

    /// Moves the raw clips on by `dt` seconds.
    pub fn tick_raw_clips(&mut self, dt: f32) {
        for left in self.raw_clips.values_mut().flatten() {
            *left -= dt;
        }
    }

    /// Wakes the objects the player has come within their hibernation
    /// distance of (its own, [`Object::hibernation_distance`]; every
    /// other object's [`HIBERNATION_DISTANCE`] until the templates' own
    /// are read) and puts to sleep those she has left
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
    ///
    /// The same pass finds who rides something, for [`Self::riders`].
    pub fn update_hibernation(&mut self) {
        let fresh = std::mem::take(&mut self.fresh);
        self.riders.clear();
        let Some(her) = self.player.and_then(|p| self.objects.get(&p)) else {
            self.riders.extend(
                self.objects
                    .iter()
                    .filter(|(_, o)| o.vehicle.is_some())
                    .map(|(&g, _)| g),
            );
            return;
        };
        let (at, riding) = (Vec3::from(her.position), her.vehicle);
        for (g, o) in &mut self.objects {
            let wake = o.hibernation_distance();
            if o.vehicle.is_some() {
                self.riders.push(*g);
            }
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
    fn killer(&self, guid: u32) -> Option<u32> {
        self.objects.get(&guid)?.killer
    }
    fn vehicle_of(&self, guid: u32) -> Option<u32> {
        self.objects.get(&guid)?.vehicle
    }
    fn grounded(&self, guid: u32) -> bool {
        !self.airborne.contains(&guid)
    }
    fn player(&self) -> Option<u32> {
        self.player
    }
    fn is_awake(&self, guid: u32) -> bool {
        self.objects.get(&guid).is_some_and(|o| o.awake)
    }
    fn raw_clip_left(&self, guid: u32) -> Option<f32> {
        self.raw_clips.get(&guid).copied().flatten()
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
        let alternatives = crate::events::class_alternatives(label);
        let mut out: Vec<u32> = self
            .objects
            .iter()
            .filter(|(_, o)| alternatives.iter().any(|terms| o.is_of(terms)))
            .map(|(&g, _)| g)
            .collect();
        out.sort_unstable();
        out
    }
    fn visit_labels(&self, visit: &mut dyn FnMut(u32, &mut dyn Iterator<Item = u32>)) -> bool {
        for (&g, o) in &self.objects {
            let mut labels = o
                .labels
                .iter()
                .map(|l| m2_assets::name_hash(l))
                .chain(o.label_hashes.iter().copied());
            visit(g, &mut labels);
        }
        true
    }
    fn unheard_at_gate(&self, a: u32, gate: u32) -> bool {
        let hers = self
            .player
            .is_some_and(|p| a == p || self.vehicle_of(p) == Some(a));
        hers && self.gates.contains(&gate) && self.honk_heard <= 0.0
    }
    fn died(&self, guid: u32) -> bool {
        !self.is_alive(guid) && !self.removed_alive.contains(&guid)
    }
    fn physics_event(&self, object: u32, name: &str) -> Option<bool> {
        match name {
            "gatefullyclosed" => Some(self.shut.contains(&object)),
            "gatestuck" => Some(false),
            _ => None,
        }
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

    fn state_changes(&self) -> &[crate::events::StateChange] {
        &self.state_changes
    }

    fn weapon_events(&self) -> &[crate::events::WeaponChange] {
        &self.weapon_events
    }

    fn animation_events(&self) -> &[(f64, u32, String)] {
        &self.animation_events
    }

    fn hijack_held_since(&self, button: u32) -> Option<f64> {
        self.hijack.held_since(button)
    }
}

/// The world shared between the game and the script libraries.
pub type Shared = Rc<RefCell<ScriptWorld>>;

/// Whether `Vehicle.Enter`'s seat argument names a seat other than the
/// driver's. The scripts pass a letter (`"d"`, `"D"`, `"p"`); the 360's
/// `Vehicle.GetRiders` (0x82346530) reads the same letters as roles:
/// `d` driver, `g` gunner, `c` commander, `p` passenger. Anything else
/// (no seat given) is left as it was: it may be the driver.
fn seat_not_driving(seat: &str) -> bool {
    matches!(
        seat.as_bytes().first().map(u8::to_ascii_lowercase),
        Some(b'p' | b'g' | b'c')
    )
}

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
/// The bound of a relation either way (from the 360 code: -100 and 100
/// at 0x820f8c3c and 0x820d9f18).
const RELATION_LIMIT: f32 = 100.0;

/// One side of a relation, as the relation table keys it: a template's
/// name in lower case (`"oc"`). A name stands for itself; a template's
/// guid (`Pg.GetGuidByName("OC")`) for its name; anyone else's for their
/// faction's, the parent the 360 code walks up to (inferred: the parent
/// chain itself isn't read, only the faction), else their own name.
fn relation_side(w: &ScriptWorld, v: Option<&Value>) -> Option<String> {
    if let Some(Value::String(s)) = v {
        return Some(s.to_string_lossy().to_ascii_lowercase());
    }
    let g = id(v)?;
    let o = w.objects.get(&g)?;
    let name = if (FIRST_TEMPLATE..FIRST_SPAWNED).contains(&g) {
        o.name.as_str()
    } else {
        o.faction_name().unwrap_or(o.name.as_str())
    };
    Some(name.to_ascii_lowercase())
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

/// How many VO sequences the engine keeps (from the 360 code: 0x20).
const VO_SEQUENCES: usize = 32;

/// [`Object.GetHibernationDistance`]'s answer, metres.
const HIBERNATION_DISTANCE: f64 = 155.0;

/// A distance as `Object.SetHibernationDistance` keeps it: truncated to
/// whole metres (the PowerPC's float-to-integer conversion), low 16 bits
/// (from the 360 code).
fn hibernation_metres(d: f32) -> u16 {
    (d as i32) as u16
}

/// A place argument at `a[i]`: three numbers (x, y, z), else an object's
/// place; and the index of the argument after it.
fn place_arg(w: &ScriptWorld, a: &[Value], i: usize) -> Option<([f32; 3], usize)> {
    if let (Some(x), Some(y), Some(z)) = (num(a.get(i)), num(a.get(i + 1)), num(a.get(i + 2))) {
        return Some(([x, y, z], i + 3));
    }
    Some((w.place_of(id(a.get(i))?)?, i + 1))
}

/// `Object.GetDistanceFrom`'s measure from `p` to `q`: flat (x and z
/// only) or not, squared or not (0x8232F688).
fn distance_from(p: [f32; 3], q: [f32; 3], flat: bool, squared: bool) -> f32 {
    let (dx, dy, dz) = (p[0] - q[0], p[1] - q[1], p[2] - q[2]);
    let d2 = if flat {
        dx * dx + dz * dz
    } else {
        dx * dx + dy * dy + dz * dz
    };
    if squared { d2 } else { d2.sqrt() }
}

fn num(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Integer(i) => Some(*i as f32),
        Value::Number(n) => Some(*n as f32),
        _ => None,
    }
}

/// The people of squad `squad` (an object's name, `oc001.obj1.vzsquad1`)
/// among `objects` (guid, name): those placed as `oc001.obj1.vz1 0x...`,
/// its name less `squad`, in guid order. Inferred from the layers' names.
fn squad_members<'a>(squad: &str, objects: impl Iterator<Item = (u32, &'a str)>) -> Vec<u32> {
    let squad = squad.rfind(" 0x").map_or(squad, |i| &squad[..i]);
    let Some(i) = squad.rfind("squad") else {
        return Vec::new();
    };
    let unit = format!("{}{}", &squad[..i], &squad[i + "squad".len()..]);
    let mut out: Vec<u32> = objects
        .filter(|(_, n)| {
            n.strip_prefix(unit.as_str())
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(" 0x"))
        })
        .map(|(g, _)| g)
        .collect();
    out.sort_unstable();
    out
}

/// `Vehicle.OpenDoor(v, door)` or `Vehicle.CloseDoor`: the door is a part's
/// name, hashed (or the hash itself).
fn door_request(w: &mut ScriptWorld, a: &[Value], open: bool) -> mlua::Result<MultiValue> {
    let door = match a.get(1) {
        Some(Value::String(s)) => Some(m2_assets::name_hash(&s.to_string_lossy())),
        Some(Value::Integer(i)) => Some(*i as u32),
        _ => None,
    };
    if let (Some(guid), Some(door)) = (id(a.first()), door) {
        w.doors.insert((guid, door), open);
        w.commands.push(Command::Door { guid, door, open });
    }
    values([])
}

/// The kind of vehicle part `Vehicle.SetParts` names, by its name hash:
/// from the 360 code's table (0x82348af8). The names known are those the
/// scripts pass, and two whose hashes match: `LightBrake` 1,
/// `LightReverse` 2, `LightFront` 3, `CtrlRotation` 4; kinds 5 to 8 by
/// hash alone.
pub fn vehicle_part(hash: u32) -> Option<u8> {
    Some(match hash {
        0x8194_f448 => 1, // LightBrake
        0xdd45_b6c3 => 2, // LightReverse
        0x7139_75a4 => 3, // LightFront
        0xc079_3c78 => 4, // CtrlRotation
        0x98f3_790f => 5,
        0x4905_c3bd => 6,
        0xe91f_1b2f => 7,
        0x3273_505d => 8,
        _ => return None,
    })
}

/// `Pg.StartHeliWaveSpawner`'s argument: its waves, in order, as the 360
/// reads them (0x8233b4d0, 0x8233b1c0): `NumToSpawn` (1), `WaveDelay`
/// (15), `SpawnDist` (150) and `units`, each `{ template, weight }` with
/// the weight 1 when left out and at least 1. A unit that isn't a table
/// with a template is left out.
fn heli_waves(t: &mlua::Table) -> mlua::Result<Vec<crate::heliwaves::Wave>> {
    use crate::heliwaves::{DEFAULT_DELAY, DEFAULT_DISTANCE, Wave};
    let mut out = Vec::new();
    for v in t.sequence_values::<Value>() {
        let Value::Table(w) = v? else { continue };
        let number = |k: &str| num(w.raw_get::<Value>(k).ok().as_ref());
        let mut units = Vec::new();
        if let Ok(Value::Table(u)) = w.raw_get::<Value>("units") {
            for unit in u.sequence_values::<Value>() {
                let Value::Table(unit) = unit? else { continue };
                let template = id(unit.raw_get::<Value>(1).ok().as_ref());
                let weight = num(unit.raw_get::<Value>(2).ok().as_ref()).unwrap_or(1.0);
                if let Some(template) = template {
                    units.push((template, (weight as i32).max(1) as u32));
                }
            }
        }
        out.push(Wave {
            count: number("NumToSpawn").map_or(1, |n| (n as i32).max(1) as u32),
            delay: number("WaveDelay").unwrap_or(DEFAULT_DELAY),
            distance: number("SpawnDist").unwrap_or(DEFAULT_DISTANCE),
            units,
        });
    }
    Ok(out)
}

impl crate::heliwaves::WaveWorld for ScriptWorld {
    fn player(&self) -> Option<([f32; 3], f32)> {
        let o = self.objects.get(&self.player?)?;
        Some((o.position, o.yaw))
    }
    fn spawn(&mut self, template: u32, at: [f32; 3], yaw: f32) -> Option<u32> {
        let name = self.objects.get(&template)?.name.clone();
        Some(ScriptWorld::spawn(self, name, at, yaw))
    }
    fn alive_at(&self, guid: u32) -> Option<[f32; 3]> {
        self.objects
            .get(&guid)
            .filter(|o| o.alive)
            .map(|o| o.position)
    }
}

type Native = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

/// Natives ported from the 360 code one by one (the PMC thread's share of
/// the gap list), each with its function's address.
fn install_ported(
    lib: &dyn Fn(&str) -> mlua::Result<Table>,
    add: &dyn Fn(&Table, &str, Native) -> mlua::Result<()>,
) -> mlua::Result<()> {
    let object = lib("Object")?;
    // Object.Revive(object, fraction) (0x823300f8): its health back to
    // `fraction` (1 when left out) of its most, kept between 0 and the
    // most; out of the destroyed state; true, or nil for no object.
    add(&object, "Revive", |w, _, a| {
        let Some(guid) = id(a.first()) else {
            return values([Value::Nil]);
        };
        let fraction = num(a.get(1)).unwrap_or(1.0);
        let Some(o) = w.objects.get_mut(&guid) else {
            return values([Value::Nil]);
        };
        let health = (o.max_health * fraction).min(o.max_health).max(0.0);
        o.health = health;
        o.alive = true;
        w.commands.push(Command::Revive { guid, health });
        values([Value::Boolean(true)])
    })?;
    // Object.ApplyImpulse(object, x, y, z, local) (0x82331638): a push on
    // its physics body; with `local` true the push is in the object's own
    // space, which the game turns by its transform. True, or nil for no
    // body.
    add(&object, "ApplyImpulse", |w, _, a| {
        let Some(guid) = id(a.first()) else {
            return values([Value::Nil]);
        };
        let Some((impulse, next)) = place_arg(w, a, 1) else {
            return values([Value::Nil]);
        };
        let local = matches!(a.get(next), Some(Value::Boolean(true)));
        if !w.objects.contains_key(&guid) {
            return values([Value::Nil]);
        }
        w.commands.push(Command::Impulse {
            guid,
            impulse,
            local,
        });
        values([Value::Boolean(true)])
    })?;
    // Object.ApplyPointImpulse(object, ix, iy, iz, px, py, pz, local)
    // (0x82331748): a push at a point on its physics body, three numbers
    // each (0x82308620); with `local` true both are in the object's own
    // space (the push turned, 0x82855A48, the point placed, 0x828561A8,
    // by its transform, 0x823F8370). It goes to the body's vtable +0x14
    // and the native answers true. Nil here for an object it does not
    // know (the 360 raises an argument error, 0x82206F20).
    add(&object, "ApplyPointImpulse", |w, _, a| {
        let Some(guid) = id(a.first()) else {
            return values([Value::Nil]);
        };
        let three = |i: usize| Some([num(a.get(i))?, num(a.get(i + 1))?, num(a.get(i + 2))?]);
        let (Some(impulse), Some(point)) = (three(1), three(4)) else {
            return values([Value::Nil]);
        };
        let local = matches!(a.get(7), Some(Value::Boolean(true)));
        if !w.objects.contains_key(&guid) {
            return values([Value::Nil]);
        }
        w.commands.push(Command::PointImpulse {
            guid,
            impulse,
            point,
            local,
        });
        values([Value::Boolean(true)])
    })?;
    // Object.GetMass(object) (0x82330A90): its physics' mass, kg (vtable
    // +0x60: a vehicle's bodies' masses together, 0 while one of them
    // does not move), as the game last said ([`ScriptWorld::masses`]).
    // Nil here for an object whose mass the game has not said (the 360
    // raises an argument error for one with no physics).
    add(&object, "GetMass", |w, _, a| {
        let mass = id(a.first()).and_then(|g| w.masses.get(&g));
        values([mass.map_or(Value::Nil, |m| Value::Number(f64::from(*m)))])
    })?;

    let player = lib("Player")?;
    // Player.GetPlayer(number) (0x8233e2c0): the player in that slot, nil
    // for an empty one. One player here, in slot 0, guid 1.
    add(&player, "GetPlayer", |_, _, a| {
        let Some(n) = num(a.first()) else {
            return values([Value::Nil]);
        };
        values([guid_value((n == 0.0).then_some(1))])
    })?;
    // Player.IsRemote(number) (0x8233e610): whether that player plays on
    // another machine; never alone.
    add(&player, "IsRemote", |_, _, a| {
        if num(a.first()).is_none() {
            return values([Value::Nil]);
        }
        values([Value::Boolean(false)])
    })?;
    // Player.SetScopeEnabled(player, on) (0x82340030): a count of reasons
    // the scope is off, one more for each false, one fewer (not below 0)
    // for each true; she may use it at 0. True, nil for bad arguments.
    add(&player, "SetScopeEnabled", |w, _, a| {
        let (Some(_), Some(Value::Boolean(on))) = (id(a.first()), a.get(1)) else {
            return values([Value::Nil]);
        };
        let was = w.scope_off == 0;
        w.scope_off = if *on {
            w.scope_off.saturating_sub(1)
        } else {
            w.scope_off + 1
        };
        if was != (w.scope_off == 0) {
            w.commands.push(Command::ScopeEnabled(w.scope_off == 0));
        }
        values([Value::Boolean(true)])
    })?;

    let human = lib("Human")?;
    // Human.SetFireLock(person, on) (0x82325498): whether the person may
    // not fire; true, nil for bad arguments.
    add(&human, "SetFireLock", |w, _, a| {
        let (Some(guid), Some(Value::Boolean(on))) = (id(a.first()), a.get(1)) else {
            return values([Value::Nil]);
        };
        w.commands.push(Command::FireLock { guid, on: *on });
        values([Value::Boolean(true)])
    })?;

    let sound = lib("Sound")?;
    // Sound.SetMasterVolume(volume, seconds) (0x82343fb8): both numbers
    // needed (nil else); answers nothing.
    add(&sound, "SetMasterVolume", |w, _, a| {
        let (Some(volume), Some(seconds)) = (num(a.first()), num(a.get(1))) else {
            return values([Value::Nil]);
        };
        w.commands.push(Command::MasterVolume { volume, seconds });
        values([])
    })?;

    let camera = lib("Camera")?;
    // Camera.SetYaw(camera, degrees) (0x82316ba8): the camera turned
    // (degrees, as every angle the scripts pass); true when there is such
    // a camera (here: a known holder), nil for bad arguments.
    add(&camera, "SetYaw", |w, _, a| {
        let (Some(holder), Some(degrees)) = (id(a.first()), num(a.get(1))) else {
            return values([Value::Nil]);
        };
        let known = w.objects.contains_key(&holder);
        if known {
            w.commands.push(Command::CameraYaw(degrees.to_radians()));
        }
        values([Value::Boolean(known)])
    })?;

    let state = lib("ObjectState")?;
    // ObjectState.StartEmitter / StopEmitter(object, state, emitter)
    // (0x823341f0, 0x82334298): the two names as hashes; nothing back,
    // nil for bad arguments.
    fn emitter(w: &mut ScriptWorld, a: &[Value], on: bool) -> mlua::Result<MultiValue> {
        let hash = |v: Option<&Value>| match v {
            Some(Value::String(s)) => Some(m2_assets::name_hash(&s.to_string_lossy())),
            Some(Value::Integer(i)) => Some(*i as u32),
            _ => None,
        };
        let (Some(guid), Some(state), Some(emitter)) =
            (id(a.first()), hash(a.get(1)), hash(a.get(2)))
        else {
            return values([Value::Nil]);
        };
        w.commands.push(Command::Emitter {
            guid,
            state,
            emitter,
            on,
        });
        values([])
    }
    add(&state, "StartEmitter", |w, _, a| emitter(w, a, true))?;
    add(&state, "StopEmitter", |w, _, a| emitter(w, a, false))?;
    Ok(())
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
/// The global holding the port's own object-name lookup: `name =
/// __m2_object_name(guid)`, the text `Object.GetName` gave before it
/// answered the 360's hash.
pub const OBJECT_NAME: &str = "__m2_object_name";

pub const SUPPORT_EARS: &str = "__m2_support";

/// Installs the libraries over `world`. Functions not written here stay
/// missing (or stubs, with `ScriptHost::stub_missing_globals`).
/// The engine's libraries (the rest of the globals scripts use are their
/// own modules); see `ScriptHost::complete_libraries`.
pub const LIBRARIES: [&str; 29] = [
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
    "Junk",
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
    crate::ported_player::install(&lib, &add)?;
    crate::ported_sp::install(&lib, &add)?;
    crate::shop::install(lua, &lib("Hud")?)?;
    crate::ported::install(&lib, &add)?;
    crate::ported_sys::install(&lib, &add)?;
    crate::cash::install(&lib, &add)?;

    // A string's hash, as the scripts key tables by name (their recruits'
    // states). The archive's name hash: the scripts only compare hashes
    // with each other, so any one hash they all share serves.
    // The engine's hash, as light userdata (`crate::script_hash`).
    add(&lib("String")?, "GetHash", |_, _, a| {
        Ok(crate::script_hash::get_hash_native(a))
    })?;
    add(&lib("ObjectState")?, "GetStringHash", |_, _, a| {
        Ok(crate::script_hash::get_hash_native(a))
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

    let junk = lib("Junk")?;
    add(&junk, "FormatTime", |_, lua, a| {
        crate::script_junk::format_time_native(lua, a)
    })?;
    add(&junk, "IsInstallable", |_, _, _| {
        Ok(crate::script_junk::is_installable())
    })?;

    let pg = lib("Pg")?;
    // The helicopter wave spawner ([`crate::heliwaves`]): both natives
    // return nothing (from the 360 code, 0x8233b6c0 and 0x8233b990).
    add(&pg, "StartHeliWaveSpawner", |w, _, a| {
        if let Some(Value::Table(t)) = a.first() {
            let waves = heli_waves(t)?;
            w.heli_waves.start(waves);
        }
        values([])
    })?;
    add(&pg, "StopHeliWaveSpawner", |w, _, _| {
        w.heli_waves.stop();
        values([])
    })?;
    // The pursuit ([`Pursuit`]).
    add(&pg, "SetPursuitLevelTimes", |w, _, a| {
        if let (Some(low), Some(high)) = (num(a.first()), num(a.get(1))) {
            w.pursuit.times = Some((low, high));
            // The engine's level times, the lower first (0x8233A078).
            let (a, b) = (low as i32, high as i32);
            w.pursuit.clock.times = (a.min(b), a.max(b));
        }
        values([])
    })?;
    // The engine's pursuit as 0x82339808 gives it.
    add(&pg, "GetPursuitState", |w, lua, _| {
        let c = &w.pursuit.clock;
        let t = lua.create_table()?;
        t.set("Active", c.active)?;
        t.set("Locked", c.locked)?;
        t.set("Level", c.shown_level())?;
        t.set("SecondsLeftInLevel", c.seconds)?;
        t.set("SecondsLeft", c.seconds)?;
        if let Some(f) = c.faction.filter(|_| c.active) {
            t.set("Faction", guid_value(Some(f)))?;
        }
        values([Value::Table(t)])
    })?;
    // (faction, level, _) for a while; (faction, level) locked.
    //
    // The answer, from the 360 code (0x823392b8 -> 0x82238398 and
    // 0x823390c8): whether a pursuit started (SetPursuit: not with one
    // locked, nor over one under way without force, nor below level 1;
    // LockPursuit: a level of 1 or more); nil for bad arguments.
    fn pursue(w: &mut ScriptWorld, a: &[Value], locked: bool) -> Option<bool> {
        let f = id(a.first())?;
        let asked = num(a.get(1)).map_or(1, |l| l as i64);
        let level = asked.clamp(1, 3);
        let name = w
            .templates
            .iter()
            .find(|(_, g)| **g == f)
            .map(|(n, _)| n.clone())?;
        w.pursuit.level = level;
        w.pursuit.faction = Some(f);
        let started = if locked {
            w.pursuit.clock.lock(f, level as i32);
            true
        } else {
            let force = matches!(a.get(2), Some(Value::Boolean(true)));
            w.pursuit.clock.set(f, level as i32, force)
        };
        let seconds = (!locked).then(|| w.pursuit.seconds(level));
        w.commands.push(Command::Pursuit {
            faction: name,
            seconds,
        });
        Some(started && asked >= 1)
    }
    add(&pg, "SetPursuit", |w, _, a| {
        values([pursue(w, a, false).map_or(Value::Nil, Value::Boolean)])
    })?;
    add(&pg, "LockPursuit", |w, _, a| {
        values([pursue(w, a, true).map_or(Value::Nil, Value::Boolean)])
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
            // Over (0x82339230 ends it as 0x82237F70).
            w.pursuit.clock.end();
            w.commands.push(Command::PursuitClear);
            values([])
        })?;
    }
    // (faction, seconds, force): the engine's pursuit restarted with that
    // many seconds left (0x823393B0 -> 0x82238570), at the level they
    // make: the faction manager's `IncrementPursuit` gives 5, level 0.
    add(&pg, "SetPursuitSeconds", |w, _, a| {
        // The answer (0x82238570): whether it started; nil for bad
        // arguments.
        let (Some(f), Some(s)) = (id(a.first()), num(a.get(1))) else {
            return values([Value::Nil]);
        };
        let force = matches!(a.get(2), Some(Value::Boolean(true)));
        let started = w.pursuit.clock.set_seconds(f, s as i32, force);
        values([Value::Boolean(started)])
    })?;
    // The objects of a kind within a radius of a point (x, y, z, radius),
    // as a list: what the strikes hit (the bunker buster's buildings, the
    // tank buster's tanks, the air patrol's aircraft). Which labels make
    // each kind is inferred from the templates' labels ("Vehicle",
    // "helicopter"...), not read from the engine.
    add(&pg, "FastCollectBuildings", |w, lua, a| {
        collect(w, lua, a, &["Building", "Structure"], &[])
    })?;
    // Pg.GetAwakeObjects(x, y, z, radius, filter): the awake objects
    // within the radius of the point, as a list (the gunship looks for
    // what is in play within 100 m of where it aims). From the 360 code
    // (823356B0): every awake object when the radius is left out or not
    // above 0; the distance is in all three axes. Its last argument, an
    // object filter to pass as well, isn't applied here: the scripts'
    // one call gives none.
    add(&pg, "GetAwakeObjects", |w, lua, a| {
        let at = match (num(a.first()), num(a.get(1)), num(a.get(2))) {
            (Some(x), Some(y), Some(z)) => Some([x, y, z]),
            _ => None,
        };
        let radius = num(a.get(if at.is_some() { 3 } else { 0 })).unwrap_or(-1.0);
        let at = at.unwrap_or([0.0; 3]);
        let mut found: Vec<u32> = w
            .objects
            .iter()
            .filter(|&(&g, o)| {
                let off = [
                    at[0] - o.position[0],
                    at[1] - o.position[1],
                    at[2] - o.position[2],
                ];
                w.is_awake(g)
                    && (radius <= 0.0
                        || off[0] * off[0] + off[2] * off[2] + off[1] * off[1] <= radius * radius)
            })
            .map(|(&g, _)| g)
            .collect();
        found.sort_unstable();
        let list = lua.create_table()?;
        for (i, g) in found.into_iter().enumerate() {
            list.raw_set(i + 1, guid_value(Some(g)))?;
        }
        values([Value::Table(list)])
    })?;
    // Pg.GetTetherDiameterStart(): how far apart the two players may be
    // before the tether starts to pull, metres. From the 360 code
    // (823385A8 reads it; 82446B68, the only code that sets it, makes it
    // 400).
    add(&pg, "GetTetherDiameterStart", |_, _, _| {
        values([Value::Number(TETHER_DIAMETER_START)])
    })?;
    add(&pg, "FastCollectHumans", |w, lua, a| {
        collect(w, lua, a, &["Human", "Soldier", "Civilian"], &[])
    })?;
    // Pg.GetObjectsInArea(x, y, z, radius, name): the objects of that
    // name (a template's label: PirCon002's `"RumJug"`s on its truck's
    // bed, OilCon020's `"OilCon020gun"`s on its pickup's, counted with
    // `table.getn`) whose collision's box meets the box reaching `radius`
    // each way from the point. From the 360 code: the binding (0x82335908)
    // asks the physics broadphase (0x82153D28, through its query at
    // vtable 0x40) for the bodies whose boxes overlap that axis-aligned
    // cube, not a sphere; OilCon020 counts its 20 guns with a radius of 1
    // round `HP_Truckbed`, which a sphere round their origins misses
    // (one of the 20, as they were put down on the bed). Those without a
    // box of their own ([`ScriptWorld::bounds`]) meet it by their place.
    add(&pg, "GetObjectsInArea", |w, lua, a| {
        let list = lua.create_table()?;
        let (Some(x), Some(y), Some(z), Some(r), Some(Value::String(name))) = (
            num(a.first()),
            num(a.get(1)),
            num(a.get(2)),
            num(a.get(3)),
            a.get(4),
        ) else {
            return values([Value::Table(list)]);
        };
        let name = name.to_string_lossy();
        let label = [
            m2_assets::name_hash(&name),
            m2_assets::name_hash(&name.to_ascii_lowercase()),
        ];
        // By its label, or by its name (a layer's `RumJug 03`, or the
        // template name a spawned one carries).
        let mut found: Vec<u32> = w
            .objects
            .iter()
            .filter(|(_, o)| !o.hidden && o.alive)
            .filter(|(_, o)| {
                o.all_labels().iter().any(|l| label.contains(l))
                    || m2_engine::world::member_stem(&o.name).eq_ignore_ascii_case(&name)
            })
            .filter_map(|(&g, _)| {
                let (lo, hi) = match w.bounds.get(&g) {
                    Some(&b) => b,
                    None => {
                        let p = w.place_of(g)?;
                        (p, p)
                    }
                };
                let c = [x, y, z];
                (0..3)
                    .all(|k| lo[k] <= c[k] + r && hi[k] >= c[k] - r)
                    .then_some(g)
            })
            .collect();
        found.sort_unstable();
        for g in found {
            list.raw_push(guid_value(Some(g)))?;
        }
        values([Value::Table(list)])
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
    // Pg.GetDistantSpawnPointOnPath(path, from, other, distance): see
    // [`crate::spawn_point`]. Gives true, x, y, z and a heading in
    // degrees, or false alone.
    add(&pg, "GetDistantSpawnPointOnPath", |w, _, a| {
        let (Some(path), Some(from)) = (id(a.first()), id(a.get(1))) else {
            return values([]);
        };
        // The third is the other object (0 for none); all four are
        // needed.
        let Some(distance) = num(a.get(3)) else {
            return values([]);
        };
        let point = w
            .paths
            .get(&path)
            .zip(w.place_of(from))
            .and_then(|(points, from)| crate::spawn_point::distant(points, from, distance));
        match point {
            Some(p) if p.found => values([
                Value::Boolean(true),
                Value::Number(f64::from(p.at[0])),
                Value::Number(f64::from(p.at[1])),
                Value::Number(f64::from(p.at[2])),
                Value::Number(f64::from(p.heading())),
            ]),
            _ => values([Value::Boolean(false)]),
        }
    })?;
    add(&pg, "GetGuidByName", |w, _, a| {
        let g = match a.first() {
            Some(Value::String(s)) => {
                let name = s.to_string_lossy();
                w.by_name(&name)
                    .or_else(|| w.entity_names.get(&name.to_ascii_lowercase()).copied())
                    .or_else(|| w.is_template(&name).then(|| w.template(&name)))
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
    // Pg.FindPointFromCamera(distance, height, radius, player, degrees):
    // the point `distance` ahead of the player's camera, flat, turned by
    // `degrees`, `height` over the ground there. From the 360 code
    // (0x823374c0): distance 10 and height 1 when left out (0x820baeec,
    // 0x820baee8); the camera is the player's (the local one when left
    // out); the angle is degrees (x 0.01745, 0x820f8c74); the ground is
    // found by a ray (0x821d15a0) and, for a radius over 0, the point is
    // moved to somewhere near within it (0x82336528). Here the camera is
    // where she stands, facing her way. INVENTED: the ground is taken at
    // her height, and the radius isn't used.
    add(&pg, "FindPointFromCamera", |w, _, a| {
        let turn = num(a.get(4)).unwrap_or(0.0).to_radians();
        let her = w.player.and_then(|p| w.objects.get(&p));
        let at = her.map(|o| {
            let (s, c) = (o.yaw + turn).sin_cos();
            let distance = num(a.first()).unwrap_or(10.0);
            [
                o.position[0] + s * distance,
                o.position[1] + num(a.get(1)).unwrap_or(1.0),
                o.position[2] + c * distance,
            ]
        });
        values(match at {
            Some(p) => p.map(|c| Value::Number(f64::from(c))).to_vec(),
            None => vec![Value::Nil],
        })
    })?;
    // Pg.EnableIntersection(on, object): see [`Command::Intersection`].
    // From the 360 code (82338740): both are needed, and it gives
    // nothing.
    add(&pg, "EnableIntersection", |w, _, a| {
        if let (Some(Value::Boolean(on)), Some(guid)) = (a.first(), id(a.get(1))) {
            w.commands.push(Command::Intersection { guid, on: *on });
        }
        values([])
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
        // Degrees (the 360 multiplies by 0.0174533); scripts pass 0 or 90.
        let yaw = num(a.get(4)).unwrap_or(0.0).to_radians();
        values([crate::events::guid_value(w.spawn(what, position, yaw))])
    })?;

    crate::ported_physics::install(&lib, &add)?;
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
            // From the 360 code (0x8232eb58): one nil.
            None => values([Value::Nil]),
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
    // Object.SetName(object, name): what it is called from now on, so
    // `Pg.GetGuidByName(name)` finds it. `MrxUtil.SpawnActor` names a
    // faction HQ's spawned interior `HqInterior` this way; the HQ then
    // finds it by that name to teleport her in and, leaving, to remove it
    // (from the scripts: MrxUtil's `SpawnActor`, MrxHq's `_LoadInterior`
    // and `Unload`).
    add(&object, "SetName", |w, _, a| {
        if let (Some(g), Some(Value::String(name))) = (id(a.first()), a.get(1))
            && let Some(o) = w.objects.get_mut(&g)
        {
            o.name = name.to_string_lossy();
            // From the 360 code (0x8232f000): true.
            return values([Value::Boolean(true)]);
        }
        values([Value::Nil])
    })?;
    // Angles cross the script boundary in degrees (from the 360 code:
    // `Object.GetYaw` 0x8232ebd8 multiplies by 57.29583, `Object.SetYaw`
    // 0x8232f8b0 and `Pg.Spawn` 0x82336b70 by 0.0174533, `Camera.GetYaw`
    // 0x82316ad8 by 57.29578). Here objects keep radians.
    add(&object, "GetYaw", |w, _, a| {
        let yaw = id(a.first()).and_then(|g| w.objects.get(&g)).map(|o| o.yaw);
        values([yaw.map_or(Value::Nil, |y| Value::Number(f64::from(y.to_degrees())))])
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
        // From the 360 code (0x8232f510): true; nil when there is nothing
        // to take the place from.
        values([Value::Boolean(true)])
    })?;
    // How far off an object goes to sleep (metres). The deliveries place
    // a helicopter's cargo by it, with 155 as their own fallback: that,
    // until the templates' hibernation data is read.
    //
    // From the 360 code (`Object.GetHibernationDistance`, 0x82330de0): an
    // object without hibernation data answers -1. Here every object in
    // play has some.
    add(&object, "GetHibernationDistance", |w, _, a| {
        let d = id(a.first())
            .and_then(|g| w.objects.get(&g))
            .map_or(-1.0, |o| f64::from(o.hibernation_distance()));
        values([Value::Number(d)])
    })?;
    // From the 360 code (`Object.SetHibernationDistance`, 0x82330ea0): the
    // distance is cut to whole metres and kept in 16 bits in the object's
    // hibernation data, which the engine then updates; the answer is
    // whether the object has such data (else nothing changes). PmcCon004
    // keeps its paradropped APCs awake out to 300 m this way, then waits
    // for them to wake (`Event.ObjectHibernation`).
    add(&object, "SetHibernationDistance", |w, _, a| {
        let set = match (id(a.first()), num(a.get(1))) {
            (Some(g), Some(d)) => match w.objects.get_mut(&g) {
                Some(o) => {
                    o.hibernation_distance = Some(hibernation_metres(d));
                    true
                }
                None => false,
            },
            _ => false,
        };
        values([Value::Boolean(set)])
    })?;
    // Object.SetYaw(object, degrees): from the 360 code (0x8232f8b0) it
    // answers true, or nil for a bad argument or an object with no place.
    add(&object, "SetYaw", |w, _, a| {
        let (Some(guid), Some(degrees)) = (id(a.first()), num(a.get(1))) else {
            return values([Value::Nil]);
        };
        let yaw = degrees.to_radians();
        let Some(o) = w.objects.get_mut(&guid) else {
            return values([Value::Nil]);
        };
        o.yaw = yaw;
        w.commands.push(Command::Turn { guid, yaw });
        values([Value::Boolean(true)])
    })?;
    // (object, boundary): whether the object (a rider: its vehicle) is
    // inside the boundary, as the `Boundary` event judges it. From the 8
    // call sites: an object and a region's guid, read as a truth
    // (MecCon001 closes the garage once the truck is in
    // `mechanicHQ.rgn.inside`, else hands the park objective out again).
    add(&object, "InsideBoundary", |w, _, a| {
        let inside = match (id(a.first()), id(a.get(1))) {
            (Some(o), Some(b)) => w.inside(o, b).unwrap_or(false),
            _ => false,
        };
        values([Value::Boolean(inside)])
    })?;
    // From the 360 code (`Pg.IsPointInBoundary(x, y, z, boundary)`,
    // 0x82337828): a point, then the boundary object; the answer is
    // whether the boundary's shape holds the point (false when it has
    // none). The 360 tests only two kinds of shape there (kinds 2 and 4 of
    // the shape record); which of the regions read here those are isn't
    // established, so every region is tested (inferred). A bad argument
    // is the usual argument error (nil here). GurCon001 asks whether the
    // jammer is in `LR_TowerBase`.
    add(&pg, "IsPointInBoundary", |w, _, a| {
        let Some((p, next)) = place_arg(w, a, 0) else {
            return values([Value::Nil]);
        };
        let Some(boundary) = id(a.get(next)) else {
            return values([Value::Nil]);
        };
        let inside = match (w.regions.get(&boundary), w.objects.get(&boundary)) {
            (Some(region), Some(at)) => {
                region.contains([at.position[0], at.position[2]], at.yaw, p[0], p[2])
            }
            _ => false,
        };
        values([Value::Boolean(inside)])
    })?;
    // Object.IsValid(object): whether the call was handed an object at
    // all (ChiCon009 asks before it stops its ambulance's health bar).
    // From the 360 code (8232FF50): it does not look the object up.
    // Inferred: a null guid doesn't count.
    add(&object, "IsValid", |_, _, a| {
        values([Value::Boolean(id(a.first()).is_some_and(|g| g != 0))])
    })?;
    add(&object, "IsAlive", |w, _, a| {
        values([Value::Boolean(id(a.first()).is_some_and(|g| w.is_alive(g)))])
    })?;
    add(&object, "IsAwake", |w, _, a| {
        values([Value::Boolean(id(a.first()).is_some_and(|g| w.is_awake(g)))])
    })?;
    // Object.IsHibernated(object): not awake. From the 360 code
    // (82330BE8): the same test as `Object.IsAwake`'s (82330C90), turned
    // round; nothing for what isn't an object.
    add(&object, "IsHibernated", |w, _, a| match id(a.first()) {
        Some(g) if w.objects.contains_key(&g) => values([Value::Boolean(!w.is_awake(g))]),
        _ => values([]),
    })?;
    // Object.FadeOut(object, seconds, flag): the object fades from sight
    // over the seconds; gives true. From the 360 code (8232FDB0): the
    // object and the seconds are needed, the flag is false when left out.
    add(&object, "FadeOut", |w, _, a| {
        let (Some(guid), Some(seconds)) = (id(a.first()), num(a.get(1))) else {
            return values([]);
        };
        w.commands.push(Command::FadeOut {
            guid,
            seconds,
            flag: matches!(a.get(2), Some(Value::Boolean(true))),
        });
        values([Value::Boolean(true)])
    })?;
    // Object.GetVelocity(object): how fast it moves, one number (m/s),
    // not a vector. From the 360 code (8232ED50): the length of a moving
    // body's velocity, else the speed kept for it; nothing for what isn't
    // an object.
    add(&object, "GetVelocity", |w, _, a| match id(a.first()) {
        Some(g) if w.objects.contains_key(&g) => {
            let speed = w.speeds.get(&g).copied().unwrap_or(0.0);
            values([Value::Number(f64::from(speed))])
        }
        _ => values([]),
    })?;
    // Object.IsVisible(object): whether it is awake and on the player's
    // screen. From the 360 code (82331C10): false for one asleep, out of
    // view or not in play; nothing when the call names no object.
    add(&object, "IsVisible", |w, _, a| match id(a.first()) {
        Some(g) => {
            use crate::events::World as _;
            values([Value::Boolean(
                w.objects.contains_key(&g) && w.is_awake(g) && w.is_visible(g) == Some(true),
            )])
        }
        None => values([]),
    })?;
    add(&object, "GetHealth", |w, _, a| {
        let o = id(a.first()).and_then(|g| w.objects.get(&g));
        values([o.map_or(Value::Nil, |o| Value::Number(f64::from(o.health)))])
    })?;
    add(&object, "GetMaxHealth", |w, _, a| {
        let o = id(a.first()).and_then(|g| w.objects.get(&g));
        values([o.map_or(Value::Nil, |o| Value::Number(f64::from(o.max_health)))])
    })?;
    // From the 360 code (0x8232e6a8): the health is kept between 0 and the
    // object's most; the answer is true, or nil for no object (or no
    // health) or a bad argument.
    add(&object, "SetHealth", |w, _, a| {
        let (Some(guid), Some(health)) = (id(a.first()), num(a.get(1))) else {
            return values([Value::Nil]);
        };
        let Some(o) = w.objects.get_mut(&guid) else {
            return values([Value::Nil]);
        };
        // Most objects' most health isn't read here yet (100 stands in), so
        // only the player's, which the game sets, caps it.
        let capped = if Some(guid) == w.player {
            health.min(o.max_health)
        } else {
            health
        };
        let health = capped.max(0.0);
        o.health = health;
        w.commands.push(Command::SetHealth { guid, health });
        values([Value::Boolean(true)])
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
        // From the 360 code (0x82330450): true.
        values([Value::Boolean(true)])
    })?;
    add(&object, "Kill", |w, _, a| {
        if let Some(guid) = id(a.first()) {
            let mut died = false;
            if let Some(o) = w.objects.get_mut(&guid) {
                died = o.alive;
                o.alive = false;
                o.health = 0.0;
            }
            if died {
                w.script_killed.push(guid);
            }
            w.commands.push(Command::Kill(guid));
        }
        values([])
    })?;
    add(&object, "Remove", |w, _, a| {
        if let Some(guid) = id(a.first()) {
            if w.objects.remove(&guid).is_some_and(|o| o.alive) {
                w.removed_alive.insert(guid);
            }
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
            w.shut.remove(&guid);
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
            w.shut.insert(guid);
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
            Some(o) if w.is_template(&o.name) && !w.templates.values().any(|&t| t == g) => {
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
    crate::links::install(&lib, &add)?;
    crate::character_binding::install(&lib, &add)?;
    crate::script_sound::install(&lib, &add)?;
    crate::script_shell::install(lua, &lib, &add)?;
    crate::script_effects::install(lua, &lib, &add)?;
    crate::screen_queries::install(&lib, &add)?;
    let human = lib("Human")?;
    // Human.SetAllowCorpseCleanup(character, allow): whether the game may
    // clear its body away (the verify objective keeps a target's until it
    // is photographed). The verify objective prints what it answers; true
    // here (inferred), and bodies are never cleared yet.
    add(&human, "SetAllowCorpseCleanup", |_, _, _| {
        values([Value::Boolean(true)])
    })?;
    // Human.DoAction(character, action): a keyed action of the human state
    // table (`VerifyCamera`, the verify's photo; `Cower`), played once;
    // `HumanActionComplete` fires as it ends.
    add(&human, "DoAction", |w, _, a| {
        if let (Some(guid), Some(Value::String(action))) = (id(a.first()), a.get(1)) {
            let action = action.to_string_lossy();
            w.hijack.start_action(guid, &action);
            w.commands.push(Command::DoAction { guid, action });
        }
        values([])
    })?;
    add(&human, "IsCarrying", |w, _, a| {
        let carrying = id(a.first()).is_some_and(|g| w.carrying.contains_key(&g));
        values([Value::Boolean(carrying)])
    })?;
    // Human.Drop(character): puts down whom it carries.
    // From the 360 code (0x82325100): whether the person was carrying
    // something (and so drops it); nil for no person.
    add(&human, "Drop", |w, _, a| {
        let Some(guid) = id(a.first()) else {
            return values([Value::Nil]);
        };
        let carrying = w.carrying.contains_key(&guid);
        if carrying {
            w.commands.push(Command::Drop(guid));
        }
        values([Value::Boolean(carrying)])
    })?;
    // Human.PlayRawAnimation(character, clip, false, false, -1, true,
    // true): the one call site (WifHq._NextCinematicFrame) plays each
    // actor's clip of a briefing frame by name; the other five arguments
    // are the same every time and not read.
    add(&human, "PlayRawAnimation", |w, _, a| {
        if let (Some(guid), Some(Value::String(clip))) = (id(a.first()), a.get(1)) {
            let clip = clip.to_string_lossy();
            w.raw_clips.insert(guid, None);
            w.commands.push(Command::RawClip { guid, clip });
        }
        values([])
    })?;
    // Human.IsGrappling(character): the player's, as the game says; no
    // one else grapples.
    add(&human, "IsGrappling", |w, _, a| {
        // From the 360 code (0x823251e0): nil when the guid is no object
        // (a wildcard character among them), else the human's flag.
        match id(a.first()).filter(|g| w.objects.contains_key(g)) {
            Some(g) => values([Value::Boolean(Some(g) == w.player && w.grappling)]),
            None => values([Value::Nil]),
        }
    })?;
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
        let Some(who) = id(a.first()) else {
            return values([]);
        };
        // A list, or one weapon alone: PmcCon034's sniper tower hands her
        // `Pg.GetGuidByName("Anti-Material Rifle")` (from the scripts; a
        // list only, she was left with nothing to break its statues with).
        let items: Vec<Value> = match a.get(1) {
            Some(Value::Table(list)) => list
                .clone()
                .sequence_values::<Value>()
                .collect::<mlua::Result<_>>()?,
            Some(
                v @ (Value::String(_)
                | Value::LightUserData(_)
                | Value::Integer(_)
                | Value::Number(_)),
            ) => vec![v.clone()],
            _ => return values([]),
        };
        let mut templates = Vec::new();
        for v in items {
            match v {
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
    // Object.SetInfiniteAmmo(character, on): her weapons don't run dry
    // (PmcCon034's sniper tower, after handing her its rifle; from the
    // scripts). Stood in for by a reserve no range empties (invented).
    //
    // From the 360 code (0x823305f0): the flag is true when left out; false
    // takes the object off the engine's infinite-ammo list; the answer is
    // true (nil for no object). Here infinite is a full-to-the-brim reserve,
    // so false puts each weapon back to its template's most (inferred: the
    // 360 just stops topping it up).
    add(&object, "SetInfiniteAmmo", |w, _, a| {
        let Some(who) = id(a.first()) else {
            return values([Value::Nil]);
        };
        let on = !matches!(a.get(1), Some(Value::Boolean(false)));
        for g in w.inventory.of(who).to_vec() {
            let Some(weapon) = w.weapon_name(g) else {
                continue;
            };
            let rounds = if on {
                INFINITE_RESERVE
            } else {
                match w.inventory.max_reserve.get(&weapon.to_ascii_lowercase()) {
                    Some(&max) => max,
                    None => continue,
                }
            };
            if let Some(c) = w.inventory.carried.get_mut(&g) {
                // Off leaves a weapon that wasn't topped up as it is.
                if !on && c.reserve != INFINITE_RESERVE {
                    continue;
                }
                c.reserve = rounds;
                w.commands.push(Command::Reserve {
                    who,
                    weapon,
                    rounds,
                });
            }
        }
        values([Value::Boolean(true)])
    })?;
    add(&weapon, "SetReserveAmmo", |w, _, a| {
        let (Some(g), Some(rounds)) = (id(a.first()), num(a.get(1))) else {
            return values([Value::Nil]);
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
            // From the 360 code (0x82349b48): true for a weapon, nil else.
            return values([Value::Boolean(true)]);
        }
        values([Value::Nil])
    })?;
    // The number of the player controlling it (scripts pass it on to
    // `Player.IsLocal` and key tables by it), else nil.
    // Metres between two objects (or markers of layers not loaded).
    // From an object to another, or to a point x, y, z (`MrxUtil.
    // GetDistanceToObject(her, x, y, z)`, as the laser guided bomb and the
    // daisy cutter ask how far she is from their target).
    // From the 360 code (0x8232F688): two places, each an object or an x,
    // y, z, then two optional flags: the first measures flat (x and z
    // only: PmcCon004's nuke within 20 m of the bunker, measured from a
    // burst on its roof 53 m up), the second leaves the distance squared.
    add(&object, "GetDistanceFrom", |w, _, a| {
        let Some((p, i)) = place_arg(w, a, 0) else {
            return values([Value::Nil]);
        };
        let Some((q, j)) = place_arg(w, a, i) else {
            return values([Value::Nil]);
        };
        let flag = |k: usize| matches!(a.get(k), Some(Value::Boolean(true)));
        let d = distance_from(p, q, flag(j), flag(j + 1));
        values([Value::Number(f64::from(d))])
    })?;
    add(&object, "IsPlayerControlled", |w, _, a| {
        let o = id(a.first()).and_then(|g| w.objects.get(&g));
        // From the 360 code (0x8232ffc8): the player who controls it (as
        // Player.GetLocalPlayer names her, guid 1), else nil.
        values([guid_value(
            o.is_some_and(|o| o.player_controlled).then_some(1),
        )])
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
            // From the 360 code (0x82330700): true once it had a label and
            // an object.
            return values([Value::Boolean(true)]);
        }
        values([Value::Boolean(false)])
    })?;
    add(&object, "RemoveLabel", |w, _, a| {
        if let (Some(o), Some(Value::String(l))) =
            (id(a.first()).and_then(|g| w.objects.get_mut(&g)), a.get(1))
        {
            let l = l.to_string_lossy();
            o.labels.retain(|x| *x != l);
            let h = m2_assets::name_hash(&l);
            o.label_hashes.retain(|&x| x != h);
            // From the 360 code (0x823307f8): true once it had a label and
            // an object, false without.
            return values([Value::Boolean(true)]);
        }
        values([Value::Boolean(false)])
    })?;
    // From the 360 code (`0x8232EF88`): the object's name hash, pushed as
    // light userdata like `String.GetHash` (`crate::script_hash`), nil for
    // a guid without a name (or a name hashing to 0). Not the name's
    // text: the port's own Lua reads that with `OBJECT_NAME`.
    add(&object, "GetName", |w, _, a| {
        let g = id(a.first());
        let o = g.and_then(|g| w.objects.get(&g));
        values([match (o, g) {
            (Some(o), _) => match crate::script_hash::engine_hash(o.name.as_bytes()) {
                0 => Value::Nil,
                h => crate::script_hash::hash_value(h),
            },
            // A faction's name as the faction zones look it up: keyed by
            // `String.GetHash` of the name, which its guid already is.
            (None, Some(g)) if FACTIONS.iter().any(|f| m2_assets::name_hash(f) == g) => {
                crate::script_hash::hash_value(g)
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
    //
    // From the 360 code (`Object.IsWinched`, 0x82333330): not a boolean.
    // It walks the object's physics constraints for a rope whose owner's
    // winch (`PhysicsActorWinch`) holds this object, and answers that
    // owner (the helicopter); otherwise nil. The deliveries test the
    // answer against nil (`MrxTaskObjectiveDeliver`'s target setup:
    // `bWinched = IsWinched(o) ~= nil`), so a false here read as winched.
    add(&object, "IsWinched", |w, _, a| {
        let carrier = id(a.first()).and_then(|g| w.winch.get(&g).copied().flatten());
        values([guid_value(carrier)])
    })?;
    // A helicopter's winch (0x823331E0..0x823337E8, the store 0x83657220):
    // whether it has one, its state by name, what hangs on it.
    add(&object, "HasWinch", |w, _, a| {
        values([Value::Boolean(
            id(a.first()).is_some_and(|g| w.winches.contains_key(&g)),
        )])
    })?;
    add(&object, "GetWinchState", |w, lua, a| {
        match id(a.first()).and_then(|g| w.winches.get(&g)) {
            Some(&s) => values([Value::String(
                lua.create_string(WINCH_STATES[usize::from(s & 3)])?,
            )]),
            None => Err(mlua::Error::runtime("Object.GetWinchState: no winch")),
        }
    })?;
    add(&object, "SetWinchState", |w, _, a| {
        let state = match a.get(1) {
            Some(Value::String(s)) => {
                let h = m2_assets::name_hash(&s.to_string_lossy());
                WINCH_STATES
                    .iter()
                    .position(|n| m2_assets::name_hash(n) == h)
            }
            _ => None,
        };
        match (id(a.first()), state) {
            (Some(g), Some(s)) if w.winches.contains_key(&g) => {
                w.winches.insert(g, s as u8);
                w.commands.push(Command::WinchState {
                    carrier: g,
                    state: s as u8,
                });
                values([])
            }
            _ => Err(mlua::Error::runtime(
                "Object.SetWinchState: no winch or state",
            )),
        }
    })?;
    add(&object, "IsWinching", |w, _, a| {
        match id(a.first()).filter(|g| w.winches.contains_key(g)) {
            Some(heli) => {
                let cargo = w
                    .winch
                    .iter()
                    .filter(|(_, c)| **c == Some(heli))
                    .map(|(&g, _)| g)
                    .min();
                values([guid_value(cargo)])
            }
            None => Err(mlua::Error::runtime("Object.IsWinching: no winch")),
        }
    })?;
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

    crate::ported_misc::install(&lib, &add)?;
    let player = lib("Player")?;
    // From the 360 code (0x8233d400 -> 0x8243d4d0): true when the
    // boundary has a shape (already held or added), false without one;
    // nil for bad arguments.
    add(&player, "AddBoundary", |w, _, a| {
        let Some(b) = id(a.get(1)) else {
            return values([Value::Nil]);
        };
        // Kept even without a shape here: a region this world didn't read
        // may still be one the game has (the 360 keeps only shaped ones).
        if !w.boundaries.contains(&b) {
            w.boundaries.push(b);
        }
        values([Value::Boolean(w.regions.contains_key(&b))])
    })?;
    add(&player, "RemoveBoundary", |w, _, a| {
        // From the 360 code (0x8233d500 -> 0x8243d770): whether she had it.
        let Some(b) = id(a.get(1)) else {
            return values([Value::Nil]);
        };
        let had = w.boundaries.contains(&b);
        w.boundaries.retain(|&x| x != b);
        values([Value::Boolean(had)])
    })?;
    add(&player, "RemoveAllBoundary", |w, _, _| {
        w.boundaries.clear();
        // From the 360 code (0x8233d5d8): true.
        values([Value::Boolean(true)])
    })?;
    // (player, out): sets whether she is out of the play area, without the
    // callback hearing it: `WifVzBoundary`'s interior mode (the HQ) and the
    // tether's end clear it (false) once they took the boundaries away, so
    // that coming back is no "exit".
    add(&player, "SetOutBoundary", |w, _, a| {
        w.outside = matches!(a.get(1), Some(Value::Boolean(true)));
        // From the 360 code (0x8233ceb0): true.
        values([Value::Boolean(true)])
    })?;
    // Player.IsPositionOutBoundary(player, x, y, z): whether a spot is
    // outside the play area. The PDA's beacon check asks it
    // (`MrxGuiPda.HandleBeaconCheck`, OilCon020's tutorial): a beacon is
    // refused out of bounds, and also when this native is missing.
    add(&player, "IsPositionOutBoundary", |w, _, a| {
        let Some(_) = id(a.first()) else {
            return values([Value::Nil]);
        };
        let [x, z] = [num(a.get(1)), num(a.get(3))].map(|v| v.unwrap_or(0.0));
        values([Value::Boolean(w.position_out_of_bounds(x, z))])
    })?;
    // A character's camera: the game's one camera looks over the player,
    // so it is named by her guid, and its yaw is hers (`Camera.GetYaw`:
    // the carpet bomb's heading).
    add(&player, "GetCamera", |w, _, a| {
        let who = id(a.first()).or(w.player);
        values([guid_value(who)])
    })?;
    crate::ported_init::install(&lib, &add)?;
    crate::player_target::install(&lib, &add)?;
    let camera = lib("Camera")?;
    // The camera looks the way she faces; a camera whose holder this
    // world doesn't place (the HUD's own world) faces north. Always a
    // number: the PDA's map turns by it (`math.rad`), and nil broke its
    // Open half-way, leaving the PDA marked open and the support menu shut.
    crate::script_camera::install(&lib, &add)?;
    add(&camera, "GetYaw", |w, _, a| {
        let yaw = id(a.first())
            .or(w.player)
            .and_then(|g| w.objects.get(&g))
            .map_or(0.0, |o| o.yaw);
        values([Value::Number(f64::from(yaw.to_degrees()))])
    })?;
    // Camera.SetShot(camera, shot, base, target, ...): a named shot of the
    // engine's (the HQ's briefings ask `OverTheShoulderLeft` and `Right`,
    // from one speaker to the other, `MrxBriefing`'s cheap cinematic),
    // held while the player is in cinematic mode.
    add(&camera, "SetShot", |w, _, a| {
        // From the 360 code (0x82317990): true once the camera takes the
        // shot, nil for bad arguments (a shot needs both objects).
        let name = match a.get(1) {
            Some(Value::String(s)) => s.to_string_lossy(),
            _ => return values([Value::Nil]),
        };
        let (Some(base), Some(target)) = (id(a.get(2)), id(a.get(3))) else {
            return values([Value::Nil]);
        };
        w.commands.push(Command::CameraShot {
            shot: name,
            base,
            target,
        });
        values([Value::Boolean(true)])
    })?;
    // Player.SetCinematicMode([player,] on, ...): the scripts take her
    // controls and the camera for a scene (the HQ's briefings), and give
    // them back with false; the first boolean is the switch (the scripts'
    // 1-argument and 5-argument calls).
    // Player.SetAimMode(player, on): the HQ's entry (with
    // `Human.DisableWeapons`) turns her aiming off, its exit on again.
    add(&player, "SetAimMode", |w, _, a| {
        if let Some(on) = a.iter().find_map(|v| match v {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }) {
            w.commands.push(Command::AimMode(on));
        }
        // From the 360 code (0x8233fee0): true.
        values([Value::Boolean(true)])
    })?;
    add(&player, "SetCinematicMode", |w, _, a| {
        let Some(on) = a.iter().find_map(|v| match v {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }) else {
            return values([]);
        };
        if on != w.cinematic {
            w.cinematic = on;
            w.cinematic_camera(on);
            w.commands.push(Command::Cinematic(on));
        }
        // From the 360 code (0x8233ccc0): true.
        values([Value::Boolean(true)])
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
        // From the 360 code (0x8233d110): true.
        values([Value::Boolean(true)])
    })?;
    add(&player, "SetInputEnabled", |w, _, a| {
        let on = !matches!(a.get(1), Some(Value::Boolean(false)));
        w.commands.push(Command::InputEnabled(on));
        // From the 360 code (0x8233cf68): true.
        values([Value::Boolean(true)])
    })?;
    add(&player, "SetSurvivalMode", |w, _, a| {
        w.survival_mode = matches!(a.get(1), Some(Value::Boolean(true)));
        // From the 360 code (0x8233d030): true.
        values([Value::Boolean(true)])
    })?;
    add(&player, "SetBoundaryCallback", |w, _, a| {
        w.boundary_callback = match a.get(1) {
            Some(Value::Function(f)) => Some(f.clone()),
            _ => None,
        };
        // From the 360 code: true.
        values([Value::Boolean(true)])
    })?;
    // A new game starts at the usual marker.
    add(&player, "GetPlayerStart", |_, lua, _| {
        values([Value::String(lua.create_string("PlayerLocation_Start")?)])
    })?;
    // The PMC's fuel tank: invented, the game's figure is not decoded.
    add(&player, "GetCash", |w, _, _| {
        values([Value::Number(w.cash)])
    })?;
    // Player.AddCash(amount): her cash goes up (or down) by a whole
    // amount, and not below nothing. From the 360 code (8233F668): taking
    // more than she has leaves her with none. `MrxPmc` pays back a
    // client's reimbursement with it.
    add(&player, "AddCash", |w, _, a| {
        if let Some(amount) = num(a.first()) {
            w.cash = (w.cash + f64::from(amount).trunc()).max(0.0);
        }
        values([])
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
    // amount once it checked her tanks have room. From the 360 code
    // (`lua_Player_AddFuel` 0x8233F828): the engine only adds to the
    // player state's fuel, with no clamp and no look at the capacity
    // (`SetFuelCapacity` 0x8233F908 only stores it); any limit is the
    // scripts' own.
    add(&player, "AddFuel", |w, _, a| {
        if let Some(n) = num(a.first()) {
            if let Some(o) = w.player.and_then(|p| w.objects.get_mut(&p)) {
                o.fuel += n;
            }
            w.commands.push(Command::AddFuel(n));
        }
        values([])
    })?;
    // The wardrobe (WifPmcInterior._CompleteChangeOutfit): the pick's
    // number, counted from 0, kept in the profile, then the character
    // dressed in the pick's model. The profile's number is what a player
    // joining reads back (`iCostume`, 8d35cf13), 0 until set.
    add(&player, "GetProfileCostume", |w, _, _| {
        values([Value::Number(w.profile_costume)])
    })?;
    add(&player, "SetProfileCostume", |w, _, a| {
        if let Some(n) = num(a.first()) {
            w.profile_costume = f64::from(n);
        }
        values([])
    })?;
    add(&player, "SetOutfit", |w, _, a| {
        if let (Some(guid), Some(Value::String(model))) = (id(a.first()), a.get(1)) {
            let model = model.to_string_lossy();
            w.commands.push(Command::Outfit { guid, model });
            // From the 360 code (0x8233fb90): true.
            return values([Value::Boolean(true)]);
        }
        values([Value::Nil])
    })?;
    // Player.SetGrappleEnabled(player, on): the one call site turns it on
    // for each player as the mechanic joins the PMC.
    add(&player, "SetGrappleEnabled", |w, _, a| {
        w.grapple_enabled = !matches!(a.get(1), None | Some(Value::Nil | Value::Boolean(false)));
        // From the 360 code: true.
        values([Value::Boolean(true)])
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
        // From the 360 code (0x823401a0's setter): true.
        values([Value::Boolean(true)])
    })?;
    add(&player, "GetVehicleDisguise", |w, _, _| {
        values([Value::Boolean(w.vehicle_disguise)])
    })?;
    // `{ Player = character }`: whether her disguise holds, nil out of a
    // disguise (the disguise tutorial tells the two apart by
    // `tostring`).
    // 0x82340B20: `Disguise.Init { InvestigatorCallback = f }` keeps `f`
    // (0x83134548) for the disguise's watchers coming and going
    // ([`investigator`]); true for a table, false without one.
    let disguise = lib("Disguise")?;
    add(&disguise, "Init", |_, lua, a| {
        let Some(Value::Table(t)) = a.first() else {
            return values([Value::Boolean(false)]);
        };
        match t.get::<Value>("InvestigatorCallback")? {
            Value::Function(f) => lua.set_named_registry_value(INVESTIGATOR, f)?,
            _ => lua.unset_named_registry_value(INVESTIGATOR)?,
        }
        values([Value::Boolean(true)])
    })?;
    // 0x82340438 ({ Player = p }): two answers, whether she is disguised
    // (flag 0x40 with disguises on) and 0x82445778's value, the root of the
    // owner chain (store 0x836539A4, 0x82251E68) of her vehicle while
    // disguised, else of herself: inferred to be the faction she is seen
    // as. Nil where the game hasn't said.
    add(&player, "GetVehicleDisguiseState", |w, _, _| {
        let held = w.disguise.is_some_and(|(_, held)| held) && w.vehicle_disguise;
        values([
            w.disguise.map_or(Value::Nil, |_| Value::Boolean(held)),
            w.disguise_seen_as
                .map_or(Value::Nil, |g| Value::Number(f64::from(g))),
        ])
    })?;
    // One player slot; the game creates the player itself.
    add(&player, "GetMaximumPlayers", |_, _, _| {
        values([Value::Integer(1)])
    })?;
    add(&player, "CreatePlayer", |w, _, a| {
        let first = matches!(num(a.first()), Some(n) if n == 0.0);
        values([guid_value(w.player.filter(|_| first))])
    })?;
    for name in ["GetLocalCharacter", "GetPrimaryCharacter", "GetCharacter"] {
        add(&player, name, |w, _, _| values([guid_value(w.player)]))?;
    }
    // The wildcard characters (0x8233e8c8, 0x8233e928): light userdata
    // 0xf0000000 and 0xf0000001, which events take as any player's (every
    // player's) character ([`crate::events::ANY_CHARACTER`]).
    add(&player, "GetAnyCharacter", |_, _, _| {
        values([crate::events::guid_value(crate::events::ANY_CHARACTER)])
    })?;
    // The second player of co-op: none.
    add(&player, "GetSecondaryCharacter", |_, _, _| {
        values([Value::Nil])
    })?;
    add(&player, "GetAllCharacters", |_, _, _| {
        values([crate::events::guid_value(crate::events::ALL_CHARACTERS)])
    })?;
    // One player, guid 1, as userdata (from the scripts:
    // `MrxGuiBase.WidgetManager.GetWidgetByNameAndOwner` answers only for
    // a userdata owner, so a number found none of her widgets, the PDA
    // among them; the game HUD's thread answered the same already).
    for name in ["GetLocalPlayer", "GetPrimaryPlayer"] {
        add(&player, name, |_, _, _| values([guid_value(Some(1))]))?;
    }
    add(&player, "GetSecondaryPlayer", |_, _, _| {
        values([Value::Nil])
    })?;
    // From the 360 code (0x8233e528): a player's number (0 is hers) or a
    // player; nil for neither.
    add(&player, "IsLocal", |_, _, a| match a.first() {
        Some(Value::Integer(n)) => values([Value::Boolean(*n == 0)]),
        Some(Value::Number(n)) => values([Value::Boolean(*n == 0.0)]),
        v => match id(v) {
            Some(g) => values([Value::Boolean(g == 1)]),
            None => values([Value::Nil]),
        },
    })?;
    // The players, a list the scripts go through (`pairs`, `ipairs`)...
    add(&player, "GetAllPlayers", |_, lua, _| {
        let t = lua.create_table()?;
        t.raw_push(guid_value(Some(1)))?;
        values([Value::Table(t)])
    })?;
    // ...and how many there are, which they compare and count with
    // (`nPlayers == 2`, `1 < n`; from the scripts' use: PirCon004 did
    // sums on it and stopped short of its ending).
    // Player.IsCoopMultiplayer(): whether both players are in the game.
    // From the 360 code (8233E058): player 0 and player 1 both joined.
    // One player here.
    add(&player, "IsCoopMultiplayer", |_, _, _| {
        values([Value::Boolean(false)])
    })?;
    add(&player, "GetCurrentPlayers", |_, _, _| {
        values([Value::Integer(1)])
    })?;

    let vehicle = lib("Vehicle")?;
    // From the 360 code: `Vehicle.OpenDoor(v, door)` (0x82347ee0) and
    // `Vehicle.CloseDoor` (0x82347f80) read the vehicle and the door's
    // name (hashed), queue the request (open 0, close 1; at most 100 a
    // frame, 0x82245c98) and return nothing; bad arguments raise the
    // usual argument error. PmcCon033 opens and closes a vehicle's
    // `"pivot"` on timers, passing `Vehicle.OpenDoor` itself as the
    // timer's callback.
    add(&vehicle, "OpenDoor", |w, _, a| door_request(w, a, true))?;
    add(&vehicle, "CloseDoor", |w, _, a| door_request(w, a, false))?;
    // 0x82348358: whether a car lies flipped (the game's to say; the 360
    // asks the physics object's vtable +0x9C). True only for a car.
    add(&vehicle, "IsFlipped", |w, _, a| {
        let Some(v) = id(a.first()) else {
            return Err(mlua::Error::runtime("Vehicle.IsFlipped: bad argument"));
        };
        values([Value::Boolean(w.flipped.contains(&v))])
    })?;
    add(&vehicle, "GetDriver", |w, _, a| {
        let driver = id(a.first()).and_then(|v| w.driver_of(v));
        values([guid_value(driver)])
    })?;
    add(&vehicle, "GetFromRider", |w, _, a| {
        values([guid_value(id(a.first()).and_then(|g| w.vehicle_of(g)))])
    })?;
    // Vehicle.Usable(vehicle, usable): all three calls in the scripts pass
    // false, for vehicles the scripts keep for themselves (two of them a
    // race's tank, ChiCon008's and PmcCon016's). From the 360 code
    // (823487C0): it gives whether it was handed a vehicle; with a
    // vehicle and no second argument it does nothing and gives nothing.
    add(&vehicle, "Usable", |w, _, a| {
        let Some(guid) = id(a.first()) else {
            return values([Value::Boolean(false)]);
        };
        let Some(Value::Boolean(on)) = a.get(1) else {
            return values([]);
        };
        w.commands.push(Command::Usable { guid, on: *on });
        values([Value::Boolean(true)])
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
        // `"p"`: the passengers, not the driver (as `Vehicle.GetDriver`
        // has it; a soldier delivery's soldiers).
        if matches!(a.get(1), Some(Value::String(s)) if s.as_bytes() == b"p")
            && let Some(driver) = v.and_then(|v| w.driver_of(v))
        {
            riders.retain(|&r| r != driver);
        }
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
                o.not_driving = false;
            }
            w.seats.leave(guid);
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
        // The seat is named by its type's letter (`d`, `p`, `g`, `c`): the
        // first free seat of that type, else the first of it.
        let kind = seat.as_bytes().first().copied().unwrap_or(b'd');
        w.seat(guid, vehicle, instant, seat_not_driving(&seat));
        if let Some(s) = w
            .seats
            .by_type(vehicle, kind, true)
            .or_else(|| w.seats.by_type(vehicle, kind, false))
        {
            w.seats.move_to(guid, s);
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
    // How one person regards another (`Ai.GetFeeling(ai, other)`,
    // `Ai.SetFeeling(ai, other, value)`), as the AI keeps it per person
    // it has perceived. From the 360 code: `GetFeeling` (0x82313350,
    // 0x822F9398) looks the other up in the AI's own store and answers
    // its feeling; with no AI there or no record of the other it answers
    // nil. A record flagged so (bit 22 of +0xe8) answers no more than -38
    // (0x820f8d38). `SetFeeling` (0x82313408, 0x822F9438) finds or makes
    // the AI's record of the other and sets it, answering whether it
    // could (false without an AI).
    //
    // Inferred here: records come only from `SetFeeling` (the 360 also
    // makes one as the AI perceives someone, which happens in the game,
    // not in the scripts' world); the -38 cap's flag isn't modelled; the
    // value is kept as given. OilCon021 asks whether the mailman's
    // feeling for her is down to -33 (hostile); a script elsewhere puts a
    // feeling below 0 back to 100.
    add(&ai, "GetFeeling", |w, _, a| {
        let feeling = match (id(a.first()), id(a.get(1))) {
            (Some(ai), Some(other)) if w.objects.get(&ai).is_some_and(|o| o.alive) => {
                w.feelings.get(&(ai, other)).copied()
            }
            _ => None,
        };
        values([feeling.map_or(Value::Nil, |f| Value::Number(f64::from(f)))])
    })?;
    add(&ai, "SetFeeling", |w, _, a| {
        let (Some(ai), Some(other), Some(value)) = (id(a.first()), id(a.get(1)), num(a.get(2)))
        else {
            return values([Value::Nil]);
        };
        let set = w.objects.get(&ai).is_some_and(|o| o.alive);
        if set {
            w.feelings.insert((ai, other), value);
        }
        values([Value::Boolean(set)])
    })?;
    // Ai.SetInfractionMultiplier(faction, m), the faction as its
    // template's guid (`Pg.GetGuidByName("OC")`): see
    // [`crate::report::Reports::multipliers`].
    add(&ai, "SetInfractionMultiplier", |w, _, a| {
        let name = id(a.first())
            .and_then(|g| w.objects.get(&g))
            .map(|o| o.name.to_ascii_lowercase());
        // From the 360 code (0x823138b8): a multiplier of 1 takes the
        // faction off the list; the answer is true, nil for bad arguments.
        let (Some(name), Some(m)) = (name, num(a.get(1))) else {
            return values([Value::Nil]);
        };
        if m == 1.0 {
            w.reports.multipliers.remove(&name);
        } else {
            w.reports.multipliers.insert(name, m);
        }
        values([Value::Boolean(true)])
    })?;
    // From the 360 code (`Ai.AddInfraction(who, faction, amount)`,
    // 0x823137c8, and 0x822F6FB8): `amount` is 1 when left out. Every AI
    // of that faction (of any, for faction 0) that has a record of `who`
    // with its +0x8c below 0.5 notes an infraction of the engine's kind 6
    // against them, and unless it is flagged not to, its feeling for
    // them drops by its faction's cost of that kind (0x822dd4d8). The
    // answer is whether `who` was given, not whether anyone saw. Who
    // perceives whom is the game's, so the request goes to it as a
    // command. AllCon003 sets China against her by 100 this way.
    add(&ai, "AddInfraction", |w, _, a| {
        let (Some(who), Some(faction)) = (id(a.first()), id(a.get(1))) else {
            return values([Value::Nil]);
        };
        let amount = num(a.get(2)).unwrap_or(1.0);
        w.commands.push(Command::Infraction {
            who,
            faction,
            amount,
        });
        values([Value::Boolean(true)])
    })?;
    add(&ai, "GetFactionGuid", |w, _, a| {
        // A zone's own; a person's, their faction's template (as
        // `Pg.GetGuidByName("China")` has it: an outpost compares the two).
        let o = id(a.first()).and_then(|g| w.objects.get(&g));
        let faction = match o.map(|o| (o.faction, o.faction_name())) {
            Some((Some(own), _)) => Some(own),
            Some((None, Some(name))) => Some(w.template(name)),
            _ => None,
        };
        values([guid_value(faction)])
    })?;
    // How one faction's AI regards another, -100 (hostile) to 100
    // (friendly), as the faction manager sets it (`Ai.SetRelation(a, b,
    // value)`); the faction manager reads attitudes back from it.
    //
    // From the 360 code: `Ai.SetRelation` (0x82313578, store 0x822F0600)
    // clamps the value to -100..100 (0x820f8c3c, 0x820d9f18), keeps it
    // for the pair in that order only, and answers true. `Ai.GetRelation`
    // (0x823134e0, lookup 0x822F0440/0x822F02C0) reads the same table,
    // which also holds the templates' own `Relationship` records (seeded
    // here from the template database): the pair in order, walking each
    // side up to its parent (a person to its faction) while none is
    // found. Never the other way round. A pair with no record answers 0,
    // unless both sides come to the same template: then 100.
    add(&ai, "SetRelation", |w, _, a| {
        let set = match (
            relation_side(w, a.first()),
            relation_side(w, a.get(1)),
            num(a.get(2)),
        ) {
            (Some(from), Some(to), Some(value)) => {
                w.relations
                    .insert((from, to), value.clamp(-RELATION_LIMIT, RELATION_LIMIT));
                true
            }
            _ => false,
        };
        values([Value::Boolean(set)])
    })?;
    add(&ai, "GetRelation", |w, _, a| {
        let value = match (relation_side(w, a.first()), relation_side(w, a.get(1))) {
            (Some(from), Some(to)) => match w.relations.get(&(from.clone(), to.clone())) {
                Some(&r) => r,
                None if from == to => RELATION_LIMIT,
                None => 0.0,
            },
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
    // Pg.RemoveContextAction(guid): see [`ContextAction`]. Both return
    // true for an object: an action objective ASSERTs what they return
    // (`MrxTaskObjectiveAction`; inferred that they succeed).
    add(&pg, "AddContextAction", |w, _, a| {
        let (Some(guid), Some(Value::String(text))) = (id(a.first()), a.get(1)) else {
            // From the 360 code (0x823381e8): nil for bad arguments.
            return values([Value::Nil]);
        };
        let reach = num(a.get(2)).filter(|r| *r > 0.0).unwrap_or(CONTEXT_REACH);
        w.context_actions.insert(
            guid,
            ContextAction {
                text: text.to_string_lossy(),
                reach,
            },
        );
        values([Value::Boolean(true)])
    })?;
    add(&pg, "RemoveContextAction", |w, _, a| {
        if let Some(guid) = id(a.first()) {
            w.context_actions.remove(&guid);
            return values([Value::Boolean(true)]);
        }
        values([])
    })?;
    // A vehicle's riders get out (`Ai.Deploy{ Vehicle, Role = "Passenger",
    // Callback, CallbackData }`: a soldier delivery, once landed, puts its
    // soldiers down; its callback then has them follow her): those of the
    // role (all but the driver for passengers) step out beside it, and
    // the callback runs on the next tick with its data unpacked (when the
    // engine calls it, and with what more, is inferred).
    // Ai.Role{ AIGuid, Role, Target, MinDistance, MoveDistance,
    // MaxDistance, ... }: "Follow" keeps them near the target (the game's
    // follower: `MrxFollow`'s VIP); another role ("Idle") ends it. The
    // rest of a role (its priorities, the callback when cancelled) is not
    // kept.
    // Ai.LivingWorld({ AIGuid, Attrib, State }): see
    // [`Command::LivingWorld`]. From the 360 code (82311238): the AI and
    // the attribute are needed, the only attribute it knows is
    // `"LivingWorldBehaviour"`, the state is true when left out, and it
    // gives what setting it gave (taken here as whether there is such
    // an AI), false otherwise.
    add(&ai, "LivingWorld", |w, _, a| {
        let Some(Value::Table(t)) = a.first() else {
            return values([Value::Boolean(false)]);
        };
        let guid = id(t.raw_get::<Value>("AIGuid").ok().as_ref());
        let attrib = t.raw_get::<Option<String>>("Attrib").ok().flatten();
        let on = match t.raw_get::<Value>("State")? {
            Value::Boolean(b) => Some(b),
            Value::Nil => None,
            _ => Some(true),
        };
        let (Some(guid), Some(attrib), Some(on)) = (guid, attrib, on) else {
            return values([Value::Boolean(false)]);
        };
        if !attrib.eq_ignore_ascii_case("LivingWorldBehaviour") || !w.objects.contains_key(&guid) {
            return values([Value::Boolean(false)]);
        }
        w.commands.push(Command::LivingWorld { guid, on });
        values([Value::Boolean(true)])
    })?;
    add(&ai, "Role", |w, _, a| {
        let Some(Value::Table(t)) = a.first() else {
            return values([]);
        };
        let Some(guid) = id(t.raw_get::<Value>("AIGuid").ok().as_ref()) else {
            return values([]);
        };
        let follow = matches!(
            t.raw_get::<Value>("Role")?,
            Value::String(s) if s.as_bytes().eq_ignore_ascii_case(b"follow")
        );
        let target = id(t.raw_get::<Value>("Target").ok().as_ref()).filter(|_| follow);
        let near = num(t.raw_get::<Value>("MoveDistance").ok().as_ref()).unwrap_or(4.0);
        w.commands.push(Command::Follow { guid, target, near });
        // From the 360 code (0x82310d50): a number, 1 when there was an AI
        // to give the role to (0 when not).
        values([Value::Integer(mlua::Integer::from(
            w.objects.contains_key(&guid),
        ))])
    })?;
    // `Ai.Squad{ SquadGuid, Action = "GetUnits" }`: the squad's people
    // (OilCon001 waits on `oc001.obj1.vzsquad1`'s to fall). Inferred from
    // the layer's names, not read from a link: a squad `<p>.<side>squad<n>`
    // is the people placed as `<p>.<side><n> 0x...` (`vz_state_oilcon001_
    // part1`: `oc001.obj1.vz1 0x10001d9b` ...). Other actions do nothing.
    add(&ai, "Squad", |w, lua, a| {
        let Some(Value::Table(t)) = a.first() else {
            return values([]);
        };
        let get = matches!(
            t.raw_get::<Value>("Action")?,
            Value::String(s) if s.as_bytes().eq_ignore_ascii_case(b"getunits")
        );
        let squad = id(t.raw_get::<Value>("SquadGuid").ok().as_ref())
            .and_then(|g| w.objects.get(&g))
            .map(|o| o.name.clone());
        let (true, Some(squad)) = (get, squad) else {
            return values([]);
        };
        let units = squad_members(&squad, w.objects.iter().map(|(&g, o)| (g, o.name.as_str())));
        let list = lua.create_table()?;
        for (i, g) in units.into_iter().enumerate() {
            list.raw_set(i + 1, guid_value(Some(g)))?;
        }
        values([Value::Table(list)])
    })?;
    add(&ai, "Deploy", |w, _, a| {
        let Some(Value::Table(t)) = a.first() else {
            return values([Value::Boolean(false)]);
        };
        let Some(vehicle) = id(t.raw_get::<Value>("Vehicle").ok().as_ref()) else {
            return values([Value::Boolean(false)]);
        };
        let passengers = matches!(
            t.raw_get::<Value>("Role")?,
            Value::String(s) if s.as_bytes().eq_ignore_ascii_case(b"passenger")
        );
        let mut riders: Vec<u32> = w
            .objects
            .iter()
            .filter(|(_, o)| o.vehicle == Some(vehicle))
            .map(|(&g, _)| g)
            .collect();
        riders.sort_unstable();
        if passengers && let Some(driver) = w.driver_of(vehicle) {
            riders.retain(|&r| r != driver);
        }
        let at = w.objects.get(&vehicle).map_or([0.0; 3], |o| o.position);
        for (i, g) in riders.into_iter().enumerate() {
            // Round the vehicle, 4 m out, a step apart.
            let a = i as f32 * 0.7;
            let position = [at[0] + 4.0 * a.sin(), at[1], at[2] + 4.0 * a.cos()];
            if let Some(o) = w.objects.get_mut(&g) {
                o.vehicle = None;
                o.seated_instantly = false;
                o.not_driving = false;
                o.position = position;
            }
            w.commands.push(Command::ExitVehicle(g));
            w.commands.push(Command::Move { guid: g, position });
        }
        if let Value::Function(f) = t.raw_get::<Value>("Callback")? {
            let data = t.raw_get::<Value>("CallbackData")?;
            w.asset_loads.push((f, data));
        }
        // From the 360 code (0x82311ed0): true once it has a vehicle, false
        // without one (or without a table).
        values([Value::Boolean(true)])
    })?;
    // From the 360 code (`Ai.RemoveGoal{ AIGuid, Handle }`, 0x82310308
    // and 0x822F9188): without an AI there it answers false; `Handle` 0
    // clears all its goals and answers true; another handle removes that
    // goal and answers whether it did. Goals here hand out no handles
    // (`Ai.Goal` answers nothing), so another handle finds none
    // (inferred). PmcCon003 clears its pursuit helicopters' goals before
    // giving them new ones.
    add(&ai, "RemoveGoal", |w, _, a| {
        let Some(Value::Table(t)) = a.first() else {
            return values([Value::Boolean(false)]);
        };
        let guid = id(t.raw_get::<Value>("AIGuid").ok().as_ref());
        let handle = num(t.raw_get::<Value>("Handle").ok().as_ref()).unwrap_or(0.0);
        let Some(guid) = guid.filter(|g| w.objects.get(g).is_some_and(|o| o.alive)) else {
            return values([Value::Boolean(false)]);
        };
        if handle != 0.0 {
            return values([Value::Boolean(false)]);
        }
        w.goal_callbacks.retain(|(g, _, _)| *g != guid);
        w.commands.push(Command::ClearGoals { guid });
        values([Value::Boolean(true)])
    })?;
    // From the 360 code (`Ai.SetState{ AIGuid, State, Value }`,
    // 0x823114a0): `Value` is true when left out; the state goes to the
    // AI's controller by name, and the answer is the controller's (false
    // without an AI). What each state does is the controller's and isn't
    // traced; it is kept per person and passed to the game, answering
    // true (inferred). PmcCon003 makes Carmona a VIP (`State = "Vip"`).
    add(&ai, "SetState", |w, _, a| {
        let Some(Value::Table(t)) = a.first() else {
            return values([Value::Nil]);
        };
        let guid = id(t.raw_get::<Value>("AIGuid").ok().as_ref());
        let state = t.raw_get::<Option<String>>("State")?;
        let on = !matches!(t.raw_get::<Value>("Value")?, Value::Boolean(false));
        let (Some(guid), Some(state)) = (guid, state) else {
            return values([Value::Nil]);
        };
        if !w.objects.get(&guid).is_some_and(|o| o.alive) {
            return values([Value::Boolean(false)]);
        }
        w.ai_states.insert((guid, state.to_ascii_lowercase()), on);
        w.commands.push(Command::AiState { guid, state, on });
        values([Value::Boolean(true)])
    })?;
    // Ai.SetHaste(ai, haste): the AI's haste from here on, held to 0..=2;
    // gives whether there is such an AI. From the 360 code (82311968).
    add(&ai, "SetHaste", |w, _, a| {
        let (Some(guid), Some(haste)) = (id(a.first()), num(a.get(1))) else {
            return values([Value::Boolean(false)]);
        };
        if !w.objects.contains_key(&guid) {
            return values([Value::Boolean(false)]);
        }
        w.commands.push(Command::SetHaste {
            guid,
            haste: haste.clamp(0.0, 2.0),
        });
        values([Value::Boolean(true)])
    })?;
    // Ai.Anchor({ AIGuid, AnchorRadius, AnchorGuid or AnchorPosition }):
    // the AI keeps within the radius of the object or point given, else
    // of where it stands; gives whether it had a place to anchor to.
    // From the 360 code (82310EB8): the AI and the radius are needed.
    add(&ai, "Anchor", |w, _, a| {
        let Some(Value::Table(t)) = a.first() else {
            return values([Value::Boolean(false)]);
        };
        let (Some(guid), Some(radius)) = (
            id(t.raw_get::<Value>("AIGuid").ok().as_ref()),
            num(t.raw_get::<Value>("AnchorRadius").ok().as_ref()),
        ) else {
            return values([Value::Boolean(false)]);
        };
        let target = id(t.raw_get::<Value>("AnchorGuid").ok().as_ref()).filter(|&g| g != 0);
        let at = match t.raw_get::<Value>("AnchorPosition")? {
            Value::Table(l) => {
                let c = |i| num(l.raw_get::<Value>(i).ok().as_ref());
                match (c(1), c(2), c(3)) {
                    (Some(x), Some(y), Some(z)) => Some([x, y, z]),
                    _ => None,
                }
            }
            _ => None,
        };
        if target.is_none() && at.is_none() && w.place_of(guid).is_none() {
            return values([Value::Boolean(false)]);
        }
        w.commands.push(Command::Anchor {
            guid,
            radius,
            target,
            at: if target.is_some() { None } else { at },
        });
        values([Value::Boolean(true)])
    })?;
    // Ai.TweakAttachedSpawners(object, settings) and
    // Ai.TweakAttachedSpawnersInGroup(object, group, settings): see
    // [`crate::spawner_tweak`]. From the 360 code (8230D450, 8230D4E0):
    // the object and the table are needed (and the group, by name: a name
    // that is no group's does nothing), and neither gives anything.
    add(&ai, "TweakAttachedSpawners", |w, _, a| {
        if let (Some(guid), Some(Value::Table(t))) = (id(a.first()), a.get(1)) {
            w.tweak_spawners(guid, None, &crate::spawner_tweak::GroupTweak::read(t));
        }
        values([])
    })?;
    add(&ai, "TweakAttachedSpawnersInGroup", |w, _, a| {
        if let (Some(guid), Some(Value::String(group)), Some(Value::Table(t))) =
            (id(a.first()), a.get(1), a.get(2))
            && let Some(group) = crate::spawner_tweak::group_of(&group.to_string_lossy())
        {
            w.tweak_spawners(
                guid,
                Some(group),
                &crate::spawner_tweak::GroupTweak::read(t),
            );
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
        // The 360 answers nil for no AI. The goal still goes to the game
        // here: the scripts' world may not hold everyone the game does
        // (a layer's people named before their layer is read).
        let known = w.objects.contains_key(&guid);
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
        // A landing may name its pad as a Target instead of a Location
        // (OilCon002's `UPHeliLand`: Ewan's Lucky Lady on
        // "TransitHeli_Landing"): it lands where the pad is.
        let location = location.or_else(|| {
            (goal == crate::mission::LAND)
                .then(|| target.and_then(|g| w.place_of(g)))
                .flatten()
        });
        let haste = num(t.raw_get::<Value>("Haste").ok().as_ref());
        // A new goal replaces the person's last (inferred): an outpost's
        // rusher calls send the same soldiers again every 20 s, and each
        // callback kept holds Lua references until the scripts' state runs
        // out of them (found by the China contracts' thread: after about
        // 400 s).
        w.goal_callbacks.retain(|(g, _, _)| *g != guid);
        if let Value::Function(f) = t.raw_get::<Value>("Callback")? {
            let data = t.raw_get::<Option<Table>>("CallbackData")?;
            w.goal_callbacks.push((guid, f, data));
        }
        let path = (goal == "PathMove")
            .then(|| crate::path_order::PathOrder::read(t))
            .transpose()?;
        w.commands.push(Command::Goal {
            guid,
            goal,
            location,
            target,
            haste,
            path,
        });
        // From the 360 code (0x8230ffb8): the goal's handle, a number
        // (`Ai.RemoveGoal{ AIGuid, Handle }` takes it back); nil when
        // `AIGuid` is no AI. `Force = true` clears the AI's goals first,
        // which a new goal here does anyway (above). Handles count up
        // from 1 here; the 360's own numbering isn't traced.
        if !known {
            return values([Value::Nil]);
        }
        w.goal_handles += 1;
        values([Value::Number(f64::from(w.goal_handles))])
    })?;

    // The armies go for `guid` before anything else: it gets the object
    // property `PriorityTarget` (0x82311B98), which their target score
    // reads (0x822E0988). The support the player calls in is made one
    // (`MrxSupport.SetupDamageEvent`). The 360 has no way to unset it.
    add(&ai, "SetPriorityTarget", |w, _, a| {
        if let Some(guid) = id(a.first()) {
            w.commands.push(Command::PriorityTarget(guid));
        }
        values([])
    })?;

    // A pilot flies its vehicle, with any cargo on the winch, to a point
    // and lets the cargo go there: pilot, x, y, z, then how low the cargo
    // comes before it is let go (0x82311C50 puts the fifth value in the
    // order; 0x822DC468 makes it that height), carried as its haste.
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
            haste: num(a.get(4)),
            path: None,
        });
        // From the 360 code (0x82311c50): false, once it has a place to go;
        // nothing without one.
        if location.is_none() {
            return values([]);
        }
        values([Value::Boolean(false)])
    })?;

    // Single player: this machine is the server.
    let net = lib("Net")?;
    crate::ported_net::install(&lib, &add)?;
    add(&net, "IsServer", |_, _, _| values([Value::Boolean(true)]))?;
    // Net.SendCustomEvent(module, event, args, ...): a script's own event
    // sent to the other player's game (a race's gates). From the 360 code
    // (8232CC78): with no network game on it sends nothing and gives true.
    add(&net, "SendCustomEvent", |_, _, _| {
        values([Value::Boolean(true)])
    })?;
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
        // From the 360 code (0x8232db28): true.
        values([Value::Boolean(true)])
    })?;

    // Calls the game only needs to hear about.
    add(&pg, "SaveGame", |w, _, a| {
        let slot = match a.first() {
            Some(Value::String(s)) => s.to_string_lossy(),
            _ => String::new(),
        };
        w.commands.push(Command::Save { slot });
        // From the 360 code (0x823387c8): the save's own answer; the game
        // saves here, so true (inferred).
        values([Value::Boolean(true)])
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
    // The bootstrap's `Init` asks (`Pg.LoadGame("InitialSaveData")`)
    // whether a save is loading. Answered false, it starts a new game
    // itself (`LoadSingleton(nil)`); answered true it doesn't, and is
    // handed the save's data (`LoadSingleton(data)`) by the engine, here
    // by `Mission::start` (from the bootstrap's bytecode; the engine's
    // side is not read).
    add(&pg, "LoadGame", |w, _, _| {
        w.commands.push(Command::Notify("Pg.LoadGame".into()));
        values([Value::Boolean(w.saved.is_some())])
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

    // Sound.CueAmbience(name) and Sound.StopAmbience(name): an ambience
    // by name (OilCon001's `ksMissionAmbience`). From the 360 code
    // (82342F28, 82342FB0): the name is needed, and neither gives
    // anything.
    add(&sound, "CueAmbience", |w, _, a| {
        if let Some(Value::String(name)) = a.first() {
            w.commands.push(Command::Ambience {
                name: name.to_string_lossy(),
                on: true,
            });
        }
        values([])
    })?;
    add(&sound, "StopAmbience", |w, _, a| {
        if let Some(Value::String(name)) = a.first() {
            w.commands.push(Command::Ambience {
                name: name.to_string_lossy(),
                on: false,
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

    // The reverb (see `reverb`): `DefineReverbPreset(index, name, the
    // twelve I3DL2 numbers, three more)`, `SetReverbPreset(name or
    // index)`, `SetReverb(1 or 0)`.
    add(&sound, "DefineReverbPreset", |w, _, a| {
        let (Some(index), Some(Value::String(name))) = (num(a.first()), a.get(1)) else {
            return values([]);
        };
        let index = index as i64;
        let numbers: Vec<f32> = a[2..].iter().map_while(|v| num(Some(v))).collect();
        if let Some(p) =
            m2_engine::reverb::ReverbPreset::from_args(&name.to_string_lossy(), &numbers)
            && let Some(c) = w.reverb.define(index, p)
        {
            w.commands.push(c);
        }
        values([])
    })?;
    add(&sound, "SetReverbPreset", |w, _, a| {
        use crate::reverb::PresetName;
        let name = match a.first() {
            Some(Value::String(s)) => PresetName::Name(s.to_string_lossy()),
            Some(Value::Integer(i)) => PresetName::Index(lua_int(*i)),
            Some(Value::Number(n)) => PresetName::Index(*n as i64),
            _ => return values([Value::Boolean(false)]),
        };
        let (found, c) = w.reverb.choose(&name);
        w.commands.extend(c);
        values([Value::Boolean(found)])
    })?;
    add(&sound, "SetReverb", |w, _, a| {
        let on = match a.first() {
            Some(Value::Boolean(b)) => *b,
            v => num(v).is_some_and(|n| n != 0.0),
        };
        w.commands.extend(w.reverb.set_on(on));
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
        if let Some(c) = w.music.activate_region() {
            w.commands.push(c);
        }
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
    // The radios' playlists ([`crate::music::Music::playlist`]):
    // (name, _), (name, cue), (name).
    add(&sound, "AddMusicSourcePlaylist", |w, _, a| {
        if let Some(Value::String(name)) = a.first() {
            w.music.add_playlist(&name.to_string_lossy());
        }
        values([])
    })?;
    add(&sound, "AddCueToMusicSourcePlaylist", |w, _, a| {
        if let (Some(Value::String(name)), Some(Value::String(cue))) = (a.first(), a.get(1)) {
            w.music
                .add_to_playlist(&name.to_string_lossy(), &cue.to_string_lossy());
        }
        values([])
    })?;
    add(&sound, "ClearMusicSourcePlaylist", |w, _, a| {
        if let Some(Value::String(name)) = a.first() {
            w.music.clear_playlist(&name.to_string_lossy());
        }
        values([])
    })?;
    add(&sound, "LockFactionMusic", |w, _, a| {
        if let Some(c) = w
            .music
            .lock(matches!(a.first(), Some(Value::Boolean(true))))
        {
            w.commands.push(c);
        }
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
    // Sound.LoadTempBank(name, type, callback, data) and
    // Sound.LoadBankWithCallback(name, type, callback): how
    // `MrxSoundBanks` loads a bank, by its localised name
    // (`_GetLocalizedName`: `vo_job_pmc.english`). The HQ's briefings and
    // the contracts list theirs in `tAssetPreload` (`soundbank` and
    // `wavebank`, both the same name); a voice bank's lines are only
    // heard once it is in. The callback hears it loaded on the next tick,
    // as `Pg.LoadAsset`'s does; the unload calls let the bank go.
    fn bank(w: &mut ScriptWorld, a: &[Value], load: bool) -> mlua::Result<MultiValue> {
        if let Some(Value::String(name)) = a.first() {
            let name = name.to_string_lossy();
            if name.to_ascii_lowercase().starts_with("vo_") {
                w.commands.push(Command::VoiceBank { name, load });
            }
        }
        if let Some(Value::Function(f)) = a.get(2) {
            w.asset_loads
                .push((f.clone(), a.get(3).cloned().unwrap_or(Value::Nil)));
        }
        values([])
    }
    add(&sound, "LoadTempBank", |w, _, a| bank(w, a, true))?;
    add(&sound, "LoadBankWithCallback", |w, _, a| bank(w, a, true))?;
    add(&sound, "UnloadTempBank", |w, _, a| bank(w, a, false))?;
    add(&sound, "UnloadBankWithCallback", |w, _, a| {
        bank(w, a, false)
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
        // From the 360 code (0x82349230, 0x82277cc0): always true.
        values([Value::Boolean(true)])
    })?;
    // From the 360 code: the VO system keeps up to 32 (speaker, priority)
    // pairs. `VO.AddSequence(speaker, priority)` (0x82349690) adds one
    // and answers true; false when the list is full or the speaker
    // already holds one with a higher number than the new one. The
    // engine also starts the speaker's sequence then (0x822783b0, not
    // traced). `VO.RemoveSequence(speaker, priority)` (0x823497b0) takes
    // out that exact pair and answers whether it was there. Nil for bad
    // arguments.
    add(&vo, "AddSequence", |w, _, a| {
        let (Some(speaker), Some(priority)) = (id(a.first()), num(a.get(1))) else {
            return values([Value::Nil]);
        };
        let priority = priority as i32;
        let full = w.vo_sequences.len() >= VO_SEQUENCES;
        let outranked = w
            .vo_sequences
            .iter()
            .any(|&(s, p)| s == speaker && priority < p);
        if full || outranked {
            return values([Value::Boolean(false)]);
        }
        w.vo_sequences.push((speaker, priority));
        values([Value::Boolean(true)])
    })?;
    add(&vo, "RemoveSequence", |w, _, a| {
        let (Some(speaker), Some(priority)) = (id(a.first()), num(a.get(1))) else {
            return values([Value::Nil]);
        };
        let pair = (speaker, priority as i32);
        let Some(at) = w.vo_sequences.iter().position(|&p| p == pair) else {
            return values([Value::Boolean(false)]);
        };
        w.vo_sequences.swap_remove(at);
        values([Value::Boolean(true)])
    })?;
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

    // Developer switches: skip to a mission (off unless asked for), and
    // force a new game (off).
    install_ported(&lib, &add)?;
    let sys = lib("Sys")?;
    // The game shows its tutorial tips, unless the player turned them off.
    add(&sys, "TutorialsEnabled", |w, _, _| {
        values([Value::Boolean(!w.tutorials_off)])
    })?;
    add(&sys, "GetSkipMission", |w, lua, _| {
        values([match &w.skip_mission {
            Some(m) => Value::String(lua.create_string(m)?),
            None => Value::Nil,
        }])
    })?;
    // The bootstrap clears it (`""`) once it has taken it.
    add(&sys, "SetSkipMission", |w, _, args| {
        w.skip_mission = category(args.first()).filter(|m| !m.is_empty());
        values([])
    })?;
    add(&sys, "GetINIBriefing", |w, _, _| {
        values([Value::Boolean(w.skip_briefing)])
    })?;
    add(&sys, "GetForceNewGame", |_, _, _| values([Value::Nil]))?;
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
        // From the 360 code (0x82344660): "0x%08X"; nil for a bad argument.
        let Some(g) = id(a.first()) else {
            return values([Value::Nil]);
        };
        values([Value::String(lua.create_string(format!("0x{g:08X}"))?)])
    })?;
    add(&sys, "RequestGameState", |w, _, a| {
        if let Some(Value::String(s)) = a.first() {
            let state = s.to_string_lossy();
            w.game_state = state.clone();
            w.commands.push(Command::GameState(state));
        }
        // From the 360 code (0x82344710): true.
        values([Value::Boolean(true)])
    })?;
    {
        use crate::script_flags as f;
        add(&sys, "IsFinalConfig", |w, _, _| {
            Ok(f::yes_no(
                f::getter(&w.sys_flags, "IsFinalConfig").unwrap_or(false),
            ))
        })?;
        add(&sys, "NoHud", |w, _, _| {
            Ok(f::yes_no(f::getter(&w.sys_flags, "NoHud").unwrap_or(false)))
        })?;
        add(&sys, "AutoLoad", |w, _, _| {
            Ok(f::yes_no(
                f::getter(&w.sys_flags, "AutoLoad").unwrap_or(false),
            ))
        })?;
        add(&sys, "DisableAssetPreload", |w, _, _| {
            Ok(f::yes_no(
                f::getter(&w.sys_flags, "DisableAssetPreload").unwrap_or(false),
            ))
        })?;
        add(&sys, "FinishedShell", |w, _, _| {
            Ok(f::yes_no(
                f::getter(&w.sys_flags, "FinishedShell").unwrap_or(false),
            ))
        })?;
        add(&sys, "GetINILoadLastSave", |w, _, _| {
            Ok(f::yes_no(
                f::getter(&w.sys_flags, "GetINILoadLastSave").unwrap_or(false),
            ))
        })?;
        add(&sys, "IsDemoMode", |w, _, _| {
            Ok(f::yes_no(
                f::getter(&w.sys_flags, "IsDemoMode").unwrap_or(false),
            ))
        })?;
        add(&sys, "PlayIntroMovies", |w, _, _| {
            Ok(f::yes_no(
                f::getter(&w.sys_flags, "PlayIntroMovies").unwrap_or(false),
            ))
        })?;
        add(&sys, "StartWithResources", |w, _, _| {
            Ok(f::yes_no(
                f::getter(&w.sys_flags, "StartWithResources").unwrap_or(false),
            ))
        })?;
        add(&sys, "YAxisInverted", |w, _, _| {
            Ok(f::yes_no(
                f::getter(&w.sys_flags, "YAxisInverted").unwrap_or(false),
            ))
        })?;
        add(&sys, "IsAutosaveEnabled", |w, _, _| {
            Ok(f::yes_no(
                f::getter(&w.sys_flags, "IsAutosaveEnabled").unwrap_or(false),
            ))
        })?;
        add(&sys, "RumbleEnabled", |w, _, _| {
            Ok(f::yes_no(
                f::getter(&w.sys_flags, "RumbleEnabled").unwrap_or(false),
            ))
        })?;
        add(&sys, "IsConfirmOnCircle", |w, _, _| {
            Ok(f::yes_no(
                f::getter(&w.sys_flags, "IsConfirmOnCircle").unwrap_or(false),
            ))
        })?;
        add(&sys, "ForceNextAutosave", |w, _, _| {
            w.sys_flags.force_next_autosave = true;
            values([])
        })?;
        add(&sys, "SetAutosaveEnabled", |w, _, a| {
            Ok(f::set_autosave(&mut w.sys_flags, a))
        })?;
        add(&sys, "HaveActiveProfile", |w, _, _| {
            Ok(f::yes_no(w.sys_flags.have_profile))
        })?;
        add(&sys, "IsGermanSKU", |w, _, _| {
            Ok(f::yes_no(w.sys_flags.german_sku))
        })?;
        use crate::script_time::{self as t, ClockKind};
        add(&sys, "MainTime", |w, _, _| {
            Ok(t::time(&w.clocks, ClockKind::Main))
        })?;
        add(&sys, "RealTime", |w, _, _| {
            Ok(t::time(&w.clocks, ClockKind::Real))
        })?;
        add(&sys, "MainTimeStamp", |w, lua, _| {
            t::stamp(lua, &w.clocks, ClockKind::Main)
        })?;
        add(&sys, "RealTimeStamp", |w, lua, _| {
            t::stamp(lua, &w.clocks, ClockKind::Real)
        })?;
        add(&sys, "TimeStampMark", |w, _, a| t::mark(&w.clocks, a))?;
        add(&sys, "TimeStampGetElapsed", |w, _, a| {
            t::elapsed(&w.clocks, a)
        })?;
        add(&sys, "Clock", |_, _, _| Ok(t::clock()))?;
        add(&sys, "DiffTime", |_, _, a| t::diff_time_native(a))?;
    }
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
        // From the 360 code (0x82344e60 -> 0x8230aa70): true, unless it
        // would leave fewer slots than requests under way (false, and
        // nothing changes); nil for no number. No request waits here
        // (loads are at once), so true.
        let Some(n) = num(a.first()) else {
            return values([Value::Nil]);
        };
        w.asset_request_max = n as u32;
        values([Value::Boolean(true)])
    })?;

    // `Math` is the math library under the name some scripts use; the
    // engine's own additions go on both (`script_math`, from the 360).
    lua.load(
        r#"
        local M = rawget(_G, "Math")
        if type(M) ~= "table" then
            M = {}
            rawset(_G, "Math", M)
        end
        for k, v in pairs(math) do
            if rawget(M, k) == nil then M[k] = v end
        end
        "#,
    )
    .set_name("=m2-math")
    .exec()?;
    crate::script_math::install(lua, Rc::default())?;

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
            -- A list of them (`Player.GetAllCharacters()`): each.
            if type(g) == "table" then
                for _, one in ipairs(g) do F.RemoveObject(f, one) end
                return
            end
            for i, v in ipairs(f.objs) do
                if v == g then table.remove(f.objs, i) return end
            end
        end
        -- A copy to change on its own (the release objective's targets
        -- still far from her, `MrxTaskObjectiveRelease`).
        function F.Copy(f)
            if not ok(f) then return F.Create() end
            local c = F.Create()
            for i, v in ipairs(f.objs) do c.objs[i] = v end
            for k, v in pairs(f.ex) do c.ex[k] = v end
            c.players, c.label = f.players, f.label
            return c
        end
        function F.GetObjects(f)
            local r = {}
            if not ok(f) then return r end
            for _, v in ipairs(f.objs) do if not f.ex[v] then r[#r + 1] = v end end
            -- A label (`SetFilter`) lists nothing: it is only matched,
            -- object by object (`Eval`, the death events). From the 360
            -- code: GetObjects (823588F0) walks the filter's own objects
            -- and the players (82359D58); SetFilter (8235A0E0) only keeps
            -- the label's hash. So a label objective (a bounty's
            -- `sTgtLabelFilter`) puts no targets on the radar or the map,
            -- and counts its kills as they come.
            if f.players then
                local p = Player.GetLocalCharacter()
                if p ~= nil and not has(f, p) and not f.ex[p] then r[#r + 1] = p end
            end
            return r
        end
        function F.GetCoopPlayerGuid(f)
            if not ok(f) then return nil end
            local p = Player.GetLocalCharacter()
            if p ~= nil and (f.players or has(f, p)) then return p end
            return nil
        end
        function F.Eval(f, g)
            if not ok(f) or g == nil or f.ex[g] then return false end
            if has(f, g) then return true end
            if f.players and g == Player.GetLocalCharacter() then return true end
            return f.label ~= nil and Object.HasLabel(g, f.label) or false
        end
        ObjectFilter = F
        "#,
    )
    .set_name("=m2-objectfilter")
    .exec()?;

    // The port's own name lookup (the HUD shims, tests): an object's name
    // as text, which the scripts' `Object.GetName` no longer gives.
    lua.globals().raw_set(
        OBJECT_NAME,
        safe_function(lua, {
            let w = Rc::clone(world);
            move |lua, args| {
                let name = args
                    .front()
                    .and_then(|v| id(Some(v)))
                    .and_then(|g| w.borrow().objects.get(&g).map(|o| o.name.clone()));
                Ok(MultiValue::from_vec(vec![match name {
                    Some(n) => Value::String(lua.create_string(&n)?),
                    None => Value::Nil,
                }]))
            }
        })?,
    )?;

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
    crate::layers::install(lua, world)?;
    // Last, so the port's own versions (`Debug.Printf`) stay.
    crate::script_noops::install(lua)?;
    Ok(())
}

/// The objects in play within `a`'s radius of its point (x, y, z,
/// radius) with any of the labels `with` and none of `without`, as a
/// list ([`ScriptWorld::place_of`]: a rider is where its vehicle is).
/// `Pg.FastCollect..`'s search over objects labelled `with` and none of
/// `without` ([`collect`]).
pub(crate) fn collect_labelled(
    w: &mut ScriptWorld,
    lua: &Lua,
    a: &[Value],
    with: &[&str],
    without: &[&str],
) -> mlua::Result<MultiValue> {
    collect(w, lua, a, with, without)
}

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
    // Then the factions to keep, by name: `"China"`, or several joined
    // with `||` (an outpost's `"China||VZ"`); none keeps all.
    let factions: Vec<&str> = a
        .iter()
        .skip(4)
        .filter_map(|v| match v {
            Value::String(s) => Some(s.to_string_lossy()),
            _ => None,
        })
        .collect::<String>()
        .split("||")
        .filter_map(|f| {
            FACTIONS
                .into_iter()
                .find(|x| x.eq_ignore_ascii_case(f.trim()))
        })
        .collect();
    let mut found: Vec<u32> = w
        .objects
        .iter()
        .filter(|(_, o)| !o.hidden)
        .filter(|(_, o)| {
            factions.is_empty() || o.faction_name().is_some_and(|f| factions.contains(&f))
        })
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

/// A Lua integer as an `i64`: `mlua::Integer` is `i64` on 64-bit hosts and
/// `i32` on the 32-bit game target.
#[allow(
    clippy::useless_conversion,
    reason = "an identity conversion on 64-bit hosts, a widening one on i686"
)]
fn lua_int(i: mlua::Integer) -> i64 {
    i.into()
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
            // The template database's templates the tests' scripts name.
            w.template_names = [
                "carbine",
                "grenade",
                "supply drop (gl)",
                "oc",
                "allied",
                "pmc",
            ]
            .map(String::from)
            .into();
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
    fn a_scripts_hibernation_distance_is_kept_per_object_in_whole_metres() {
        // PmcCon004's APC drop: Object.SetHibernationDistance(apc, 300).
        let (host, world) = setup();
        let (before, set, after, other, missing, unknown): (f64, bool, f64, f64, bool, f64) = host
            .lua()
            .load(
                "local before = Object.GetHibernationDistance(50)
                 local set = Object.SetHibernationDistance(50, 300.9)
                 return before, set, Object.GetHibernationDistance(50),
                   Object.GetHibernationDistance(1),
                   Object.SetHibernationDistance(999, 300),
                   Object.GetHibernationDistance(999)",
            )
            .eval()
            .unwrap();
        assert_eq!(before, HIBERNATION_DISTANCE);
        // Cut to whole metres, for that object alone.
        assert!(set);
        assert_eq!(after, 300.0);
        assert_eq!(other, HIBERNATION_DISTANCE);
        // No such object: nothing kept, and -1 for its distance.
        assert!(!missing);
        assert_eq!(unknown, -1.0);
        // Kept in 16 bits, as the engine keeps it.
        assert_eq!(hibernation_metres(65_536.0 + 12.5), 12);
        assert_eq!(world.borrow().objects[&50].hibernation_distance, Some(300));
    }

    #[test]
    fn an_object_with_a_longer_hibernation_distance_stays_awake_further_off() {
        let (_host, world) = setup();
        let mut w = world.borrow_mut();
        w.objects.get_mut(&50).unwrap().hibernation_distance = Some(300);
        let gone = HIBERNATION_DISTANCE as f32 + HIBERNATION_MARGIN + 20.0;
        w.objects.get_mut(&1).unwrap().position = [gone, 0.0, 0.0];
        w.update_hibernation();
        assert!(w.is_awake(50));
        w.objects.get_mut(&1).unwrap().position = [300.0 + HIBERNATION_MARGIN + 1.0, 0.0, 0.0];
        w.update_hibernation();
        assert!(!w.is_awake(50));
        // It wakes at its own distance, not the usual one.
        w.objects.get_mut(&1).unwrap().position = [290.0, 0.0, 0.0];
        w.update_hibernation();
        assert!(w.is_awake(50));
    }

    #[test]
    fn custom_events_go_nowhere_in_single_player() {
        // PmcCon004 tells a co-op partner to change the atmosphere; alone,
        // the engine sends nothing and answers true.
        let (host, world) = setup();
        world.borrow_mut().take_commands();
        let sent: bool = host
            .lua()
            .load("return Net.SendCustomEvent('PmcCon004', 4, {}, true)")
            .eval()
            .unwrap();
        assert!(sent);
        assert!(world.borrow_mut().take_commands().is_empty());
        assert!(host.take_calls().is_empty());
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
    fn the_hqs_aim_mode_reaches_the_game() {
        let (host, world) = setup();
        host.lua()
            .load(
                "Player.SetAimMode(Player.GetLocalPlayer(), false)\n\
                 Player.SetAimMode(Player.GetLocalPlayer(), true)",
            )
            .exec()
            .unwrap();
        assert_eq!(
            world.borrow_mut().take_commands(),
            [Command::AimMode(false), Command::AimMode(true)]
        );
    }

    #[test]
    fn a_closed_junction_reaches_the_game_and_an_object_is_valid_or_not() {
        let (host, world) = setup();
        let (valid, nothing, number): (bool, bool, bool) = host
            .lua()
            .load(
                r#"
                local bridge = Sys.StringToGuid("568220")
                Pg.EnableIntersection(false, bridge)
                Pg.EnableIntersection(true, bridge)
                Pg.EnableIntersection(bridge)
                return Object.IsValid(bridge), Object.IsValid(nil), Object.IsValid("bridge")
                "#,
            )
            .eval()
            .unwrap();
        // It does not ask whether the object is in the world.
        assert!(valid && !nothing && !number);
        assert_eq!(
            world.borrow_mut().take_commands(),
            [
                Command::Intersection {
                    guid: 568220,
                    on: false
                },
                Command::Intersection {
                    guid: 568220,
                    on: true
                },
            ]
        );
    }

    #[test]
    fn spawner_tweaks_add_up_per_object_and_group() {
        let (host, world) = setup();
        host.lua()
            .load(
                r#"
                local tower = Sys.StringToGuid("70")
                -- As the dangerous building and GurJob020 do.
                Ai.TweakAttachedSpawners(tower, { SpawnerState = "on", SpawnList = "Spawnlist (VZ Balcony)" })
                Ai.TweakAttachedSpawnersInGroup(tower, "Ground", { SpawnList = "Spawnlist (VZ Ground)" })
                Ai.TweakAttachedSpawnersInGroup(tower, "ground", { SpawnerState = "off" })
                -- No such group: nothing happens.
                Ai.TweakAttachedSpawnersInGroup(tower, "cellar", { SpawnerState = "off" })
                -- Nothing the engine reads: no record is kept.
                Ai.TweakAttachedSpawners(Sys.StringToGuid("71"), { ActiveRadius = 100 })
                "#,
            )
            .exec()
            .unwrap();
        let mut w = world.borrow_mut();
        let record = w.spawner_records[&70].clone();
        assert!(record.off(1));
        assert_eq!(
            record.groups[1].spawn_list.as_deref(),
            Some("Spawnlist (VZ Ground)")
        );
        assert!(!record.off(2));
        assert_eq!(
            record.groups[2].spawn_list.as_deref(),
            Some("Spawnlist (VZ Balcony)")
        );
        assert!(!w.spawner_records.contains_key(&71));
        let told: Vec<(u32, bool)> = w
            .take_commands()
            .into_iter()
            .filter_map(|c| match c {
                Command::TweakSpawners { guid, record } => Some((guid, record.is_empty())),
                _ => None,
            })
            .collect();
        // Three calls for the tower, each with its record so far; the
        // fourth named no group; the other object's is told as empty.
        assert_eq!(told, [(70, false), (70, false), (70, false), (71, true)]);
    }

    #[test]
    fn the_shared_modules_calls_are_answered() {
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            w.cash = 1000.0;
            w.insert(70, Object::new("Near", [10.0, 0.0, 0.0]));
            w.insert(71, Object::new("Far", [500.0, 0.0, 0.0]));
        }
        let (near, tether, living, wrong, nobody): (i64, f64, bool, bool, bool) = host
            .lua()
            .load(
                r#"
                local g = Sys.StringToGuid
                Player.AddCash(250.9)
                Player.AddCash(-5000)
                Player.AddCash(40)
                local list = Pg.GetAwakeObjects(0, 0, 0, 100)
                local near = 0
                for _, o in pairs(list) do if o == g("70") then near = near + 1 end
                    if o == g("71") then near = near + 100 end end
                return near, Pg.GetTetherDiameterStart(),
                    Ai.LivingWorld({ AIGuid = g("70"), Attrib = "LivingWorldBehaviour", State = false }),
                    Ai.LivingWorld({ AIGuid = g("70"), Attrib = "Other", State = false }),
                    Ai.LivingWorld({ AIGuid = g("72"), Attrib = "LivingWorldBehaviour", State = false })
                "#,
            )
            .eval()
            .unwrap();
        // Within 100 m and awake: the near one, not the far one.
        assert_eq!(near, 1);
        assert_eq!(tether, 400.0);
        assert!(living && !wrong && !nobody);
        let mut w = world.borrow_mut();
        // 1000 + 250 (whole), then more taken than she has: none; + 40.
        assert_eq!(w.cash, 40.0);
        assert_eq!(
            w.take_commands(),
            [Command::LivingWorld {
                guid: 70,
                on: false
            }]
        );
    }

    #[test]
    fn gates_weapons_usable_vehicles_and_fuel_reach_the_game() {
        let (host, world) = setup();
        world.borrow_mut().fuel_capacity = Some(50.0);
        host.lua()
            .load(
                "Object.CloseGate('_global_veh_jeep 0x0001')\n\
                 Human.DisableWeapons(Player.GetLocalCharacter())\n\
                 Human.EnableWeapons(Player.GetLocalCharacter())\n\
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
        // Added whole: the engine doesn't hold it to the tanks' capacity
        // (the 360's `Player.AddFuel`).
        assert_eq!(w.objects[&1].fuel, 180.0);
    }

    #[test]
    fn oilcon001s_calls_reach_the_game() {
        let (host, world) = setup();
        world
            .borrow_mut()
            .insert(70, Object::new("OilCon001_Exec", [5.0, 0.0, 7.0]));
        let (hasty, nobody, anchored, bare, faded, hibernated, unknown): (
            bool,
            bool,
            bool,
            bool,
            Value,
            Value,
            Value,
        ) = host
            .lua()
            .load(
                r#"
                local exec = Sys.StringToGuid("70")
                Sound.CueAmbience("amb_oilcon001")
                Sound.StopAmbience("amb_oilcon001")
                return Ai.SetHaste(exec, 5), Ai.SetHaste(Sys.StringToGuid("71"), 1),
                    Ai.Anchor({ AIGuid = exec, AnchorRadius = 0 }),
                    Ai.Anchor({ AIGuid = exec }),
                    Object.FadeOut(exec, 1, true),
                    Object.IsHibernated(exec),
                    Object.IsHibernated(Sys.StringToGuid("71"))
                "#,
            )
            .eval()
            .unwrap();
        // Haste is held to 0..=2; no such AI, no haste.
        assert!(hasty && !nobody);
        // Anchored where it stands; without a radius, not at all.
        assert!(anchored && !bare);
        assert_eq!(faded, Value::Boolean(true));
        assert!(matches!(hibernated, Value::Boolean(_)));
        assert_eq!(unknown, Value::Nil);
        let commands = world.borrow_mut().take_commands();
        assert_eq!(
            commands,
            [
                Command::Ambience {
                    name: "amb_oilcon001".into(),
                    on: true
                },
                Command::Ambience {
                    name: "amb_oilcon001".into(),
                    on: false
                },
                Command::SetHaste {
                    guid: 70,
                    haste: 2.0
                },
                Command::Anchor {
                    guid: 70,
                    radius: 0.0,
                    target: None,
                    at: None
                },
                Command::FadeOut {
                    guid: 70,
                    seconds: 1.0,
                    flag: true
                },
            ]
        );
    }

    #[test]
    fn a_spawn_point_down_the_path_and_a_spawner_tweak_reach_the_game() {
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            w.insert(70, Object::new("OilCon001_Exec", [0.0, 0.0, 0.0]));
            w.paths.insert(90, vec![[0.0, 0.0, 0.0], [100.0, 0.0, 0.0]]);
        }
        let (found, x, y, z, heading, lost, bare): (bool, f64, f64, f64, f64, Value, Value) = host
            .lua()
            .load(
                r#"
                local g = Sys.StringToGuid
                Ai.TweakAttachedSpawners(g("70"),
                    { SpawnerState = "on", SpawnList = "Spawnlist (VZ Balcony)" })
                local found, x, y, z, heading = Pg.GetDistantSpawnPointOnPath(g("90"), g("70"), 0, 60)
                -- A path the game doesn't know: false alone.
                local lost, more = Pg.GetDistantSpawnPointOnPath(g("91"), g("70"), 0, 60)
                assert(more == nil)
                return found, x, y, z, heading, lost, (Pg.GetDistantSpawnPointOnPath(g("90")))
                "#,
            )
            .eval()
            .unwrap();
        assert!(found);
        assert!((x - 60.0).abs() < 1e-3 && y == 0.0 && z.abs() < 1e-3);
        assert!((heading - 90.0).abs() < 1e-3);
        assert_eq!(lost, Value::Boolean(false));
        assert_eq!(bare, Value::Nil);
        // Every group on, from the balcony's list.
        let mut on_from_the_balcony = crate::spawner_tweak::SpawnerRecord::default();
        on_from_the_balcony.merge(
            None,
            &crate::spawner_tweak::GroupTweak {
                state: Some(1),
                spawn_list: Some("Spawnlist (VZ Balcony)".into()),
                ..Default::default()
            },
        );
        let mut w = world.borrow_mut();
        assert_eq!(
            w.take_commands(),
            [Command::TweakSpawners {
                guid: 70,
                record: Box::new(on_from_the_balcony.clone()),
            }]
        );
        assert_eq!(w.spawner_records[&70], on_from_the_balcony);
    }

    #[test]
    fn the_oil_contracts_questions_are_answered() {
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            w.insert(70, Object::new("OilCon003_Truck", [5.0, 0.0, 7.0]));
            w.speeds.insert(70, 12.5);
            w.insert(71, Object::new("OilCon003_Parked", [9.0, 0.0, 7.0]));
        }
        let (moving, parked, nothing, coop, unseen, no_object): (
            f64,
            f64,
            Value,
            bool,
            Value,
            Value,
        ) = host
            .lua()
            .load(
                r#"
                local g = Sys.StringToGuid
                return Object.GetVelocity(g("70")), Object.GetVelocity(g("71")),
                    (Object.GetVelocity(g("72"))), Player.IsCoopMultiplayer(),
                    Object.IsVisible(g("72")), (Object.IsVisible())
                "#,
            )
            .eval()
            .unwrap();
        // A speed, not a vector; at rest, 0; no such object, nothing.
        assert_eq!((moving, parked), (12.5, 0.0));
        assert_eq!(nothing, Value::Nil);
        assert!(!coop);
        // Not in play: not visible. No object named: nothing.
        assert_eq!(unseen, Value::Boolean(false));
        assert_eq!(no_object, Value::Nil);
    }

    #[test]
    fn usable_answers_whether_it_was_handed_a_vehicle() {
        let (host, world) = setup();
        let (set, none, bare): (Value, Value, Value) = host
            .lua()
            .load(
                "local jeep = Pg.GetGuidByName('_global_veh_jeep 0x0001')\n\
                 return Vehicle.Usable(jeep, true), Vehicle.Usable(nil, false), \
                 (Vehicle.Usable(jeep))",
            )
            .eval()
            .unwrap();
        assert_eq!(set, Value::Boolean(true));
        assert_eq!(none, Value::Boolean(false));
        // A vehicle and nothing to set it to: nothing done, nothing given.
        assert_eq!(bare, Value::Nil);
        assert_eq!(
            world.borrow_mut().take_commands(),
            [Command::Usable { guid: 50, on: true }]
        );
    }

    #[test]
    fn the_scripts_radio_playlists_are_kept_by_name_hash() {
        let (host, world) = setup();
        host.lua()
            .load(
                "Sound.AddMusicSourcePlaylist('mu_src_radio', 0)\n\
                 Sound.AddCueToMusicSourcePlaylist('mu_src_radio', 'MU_SRC_PMC_HQ_01')\n\
                 Sound.AddCueToMusicSourcePlaylist('mu_src_radio', 'MU_SRC_PMC_HQ_02')\n\
                 Sound.AddMusicSourcePlaylist('mu_src_civ', 0)\n\
                 Sound.AddCueToMusicSourcePlaylist('mu_src_civ', 'mu_src_civ')\n\
                 Sound.ClearMusicSourcePlaylist('mu_src_civ')",
            )
            .exec()
            .unwrap();
        let w = world.borrow();
        assert_eq!(
            w.music.playlist(m2_assets::name_hash("mu_src_radio")),
            ["MU_SRC_PMC_HQ_01", "MU_SRC_PMC_HQ_02"]
        );
        assert!(
            w.music
                .playlist(m2_assets::name_hash("mu_src_civ"))
                .is_empty()
        );
        assert!(w.music.playlist(0x1234).is_empty());
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

    // The faction manager's `IncrementPursuit`: a level, then 5 s, which
    // leave level 1 (`Threat1`) for 5 s; the clock as the 360's.
    #[test]
    fn the_engines_pursuit_runs_by_its_seconds_as_the_360s() {
        let (host, world) = setup();
        let run = |code: &str| host.lua().load(code).exec().unwrap();
        run("Pg.SetPursuitLevelTimes(120, 300)\n\
             local oc = Pg.GetGuidByName('OC')\n\
             Pg.SetPursuit(oc, 2, true)\n\
             __two = Pg.GetPursuitState().Level\n\
             Pg.SetPursuitSeconds(oc, 5, true)\n\
             local s = Pg.GetPursuitState()\n\
             __one, __left, __active = s.Level, s.SecondsLeft, s.Active");
        let g = host.lua().globals();
        assert_eq!(g.get::<i64>("__two").unwrap(), 2);
        assert_eq!(g.get::<i64>("__one").unwrap(), 1);
        assert_eq!(g.get::<i64>("__left").unwrap(), 5);
        assert!(g.get::<bool>("__active").unwrap());
        let mut c = world.borrow().pursuit.clock;
        c.step(4.5);
        assert!(c.active && c.seconds == 1);
        c.step(1.0);
        assert!(!c.active);
        assert_eq!(c.shown_level(), 0);
        // Level 3 starts at the second time plus 100 at most, and drops a
        // level as its seconds fall under the second time.
        let oc = 1;
        assert!(c.set(oc, 3, false));
        assert_eq!((c.level, c.seconds), (2, 400));
        assert!(!c.set(oc, 1, false), "kept unless forced");
        c.step(101.0);
        assert_eq!((c.level, c.seconds), (1, 210));
        c.lock(oc, 1);
        c.step(1000.0);
        assert!(c.active && c.level == 0);
    }

    #[test]
    fn the_scripts_turn_the_grapple_on_and_hear_whether_she_grapples() {
        let (host, world) = setup();
        let lua = host.lua();
        // As the mechanic's joining does (01a266d2), then the HQ's checks.
        lua.load("Player.SetGrappleEnabled(Player.GetAnyCharacter(), true)")
            .exec()
            .unwrap();
        assert!(world.borrow().grapple_enabled);
        let ask = "return Human.IsGrappling(Player.GetLocalCharacter())";
        assert!(!lua.load(ask).eval::<bool>().unwrap());
        world.borrow_mut().grappling = true;
        assert!(lua.load(ask).eval::<bool>().unwrap());
        lua.load("Player.SetGrappleEnabled(Player.GetAnyCharacter(), false)")
            .exec()
            .unwrap();
        assert!(!world.borrow().grapple_enabled);
    }

    #[test]
    fn the_wardrobe_dresses_her_and_briefing_clips_reach_the_game() {
        let (host, world) = setup();
        let lua = host.lua();
        // WifPmcInterior._CompleteChangeOutfit's calls, then a briefing
        // frame's clip for the player.
        let before: f64 = lua
            .load("return Player.GetProfileCostume()")
            .eval()
            .unwrap();
        lua.load(
            "Player.SetProfileCostume(3)\n\
             Player.SetOutfit(Player.GetLocalCharacter(), 'pmc_hum_jen_v2')\n\
             Human.PlayRawAnimation(Player.GetLocalCharacter(), \
                'ALL01_Contract_Briefing_Jennifer', false, false, -1, true, true)",
        )
        .exec()
        .unwrap();
        let after: f64 = lua
            .load("return Player.GetProfileCostume()")
            .eval()
            .unwrap();
        assert_eq!((before, after), (0.0, 3.0));
        assert_eq!(
            world.borrow_mut().take_commands(),
            [
                Command::Outfit {
                    guid: 1,
                    model: "pmc_hum_jen_v2".into()
                },
                Command::RawClip {
                    guid: 1,
                    clip: "ALL01_Contract_Briefing_Jennifer".into()
                },
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
                "Object.SetTransformToObject(Player.GetLocalCharacter(), \
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
                "Object.SetTransformToObject(Player.GetLocalCharacter(), \
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
                return x, __m2_object_name(jeep), Object.IsAlive(jeep) and Object.HasLabel(jeep, "Vehicle")
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
    fn the_shell_and_post_processing_calls_answer_as_the_360_ones() {
        let (host, world) = setup();
        let (name, set, bad, a, b): (String, bool, usize, f64, f64) = host
            .lua()
            .load(
                r#"
                local before = Sys.GetLevelName()
                local set = Sys.SetLevelName("vz_island")
                Graphics.Bloom.SetThreshold(0.8)
                local bad = select('#', Graphics.Contrast.SetLimit())
                local a, b = Sys.GetVersion()
                return before .. " " .. Sys.GetLevelName(), set, bad, a, b
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(
            (name.as_str(), set, bad, a, b),
            ("vz vz_island", true, 1, 100000.0, 100000.0)
        );
        assert_eq!(world.borrow().shell.bloom_threshold, Some(0.8));
    }

    #[test]
    fn the_sound_settings_and_vo_calls_answer_as_the_360_ones() {
        let (host, world) = setup();
        let (vol, unset, bad, cancelled, paused, profile, sku): (
            f32,
            f32,
            usize,
            bool,
            bool,
            bool,
            bool,
        ) = host
            .lua()
            .load(
                r#"
                Sound.SetCategoryVolume("music", 0.5)
                return Sound.GetCategoryVolume("music"), Sound.GetCategoryPitch("music"),
                    select('#', Sound.SetCategoryVolume("music")),
                    VO.CancelAll(), VO.Unpause(), Sys.HaveActiveProfile(), Sys.IsGermanSKU()
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(
            (vol, unset, bad, cancelled, paused, profile, sku),
            (0.5, 1.0, 1, true, true, true, false)
        );
        host.lua()
            .load(r#"Sound.TestCueSound("sfx_test")"#)
            .exec()
            .unwrap();
        assert_eq!(
            world.borrow_mut().take_commands(),
            [Command::CueSound {
                at: Some(1),
                cue: "sfx_test".into()
            }]
        );
    }

    #[test]
    fn object_get_name_answers_the_names_hash_as_on_the_360() {
        let (host, _world) = setup();
        let (same, kind, renamed, none): (bool, String, bool, bool) = host
            .lua()
            .load(
                r#"
                local jeep = Pg.GetGuidByName("_global_veh_jeep 0x0001")
                local name = Object.GetName(jeep)
                local same = name == String.GetHash("_global_veh_jeep 0x0001")
                Object.SetName(jeep, "HqInterior")
                local renamed = Object.GetName(jeep) == String.GetHash("HqInterior")
                    and __m2_object_name(jeep) == "HqInterior"
                return same, type(name), renamed, Object.GetName(Sys.StringToGuid("0x7ffe")) == nil
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(
            (same, kind.as_str(), renamed, none),
            (true, "userdata", true, true)
        );
    }

    #[test]
    fn a_loadout_of_one_weapon_alone_is_given_too() {
        let (host, world) = setup();
        let name: String = host
            .lua()
            .load(
                r#"
                local hero = Player.GetLocalCharacter()
                Human.Inventory.SetAllWeapons(hero, Pg.GetGuidByName("Carbine"))
                return __m2_object_name(Human.Inventory.GetPrimaryWeapon(hero))
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(name, "Carbine");
        assert!(matches!(
            &world.borrow_mut().take_commands()[..],
            [Command::Loadout { weapons, .. }] if weapons == &["Carbine".to_owned()]
        ));
        // And it doesn't run dry when the scripts say so.
        host.lua()
            .load("Object.SetInfiniteAmmo(Player.GetLocalCharacter(), true)")
            .exec()
            .unwrap();
        assert!(matches!(
            &world.borrow_mut().take_commands()[..],
            [Command::Reserve {
                rounds: INFINITE_RESERVE,
                ..
            }]
        ));
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
                    return __m2_object_name(gun), Weapon.GetReserveAmmo(all[2]),
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
    fn the_wildcard_characters_stand_for_her_in_events_only() {
        // From the 360 code: GetAnyCharacter and GetAllCharacters answer
        // light userdata 0xf0000000 and 0xf0000001. Event subjects take
        // them as her; Human natives look the guid up and find nothing;
        // Player.SetGrappleEnabled falls back to the local player.
        let (host, world) = setup();
        let events = crate::events::Events::default();
        events.install(host.lua()).unwrap();
        world.borrow_mut().grappling = true;
        let lua = host.lua();
        let (same, wild, grappling): (bool, bool, Value) = lua
            .load(
                r#"
                local any, all = Player.GetAnyCharacter(), Player.GetAllCharacters()
                near = {}
                Event.Create(Event.ObjectProximity, {any, 50, "<", 5},
                    function() near[#near + 1] = "any" end)
                Event.Create(Event.ObjectProximity, {all, 50, "<", 5},
                    function() near[#near + 1] = "all" end)
                Player.SetGrappleEnabled(any, true)
                return any == Player.GetAnyCharacter(),
                    type(any) == "userdata" and any ~= Player.GetLocalCharacter()
                        and any ~= all,
                    Human.IsGrappling(any)
                "#,
            )
            .eval()
            .unwrap();
        assert!(same && wild);
        assert_eq!(grappling, Value::Nil);
        assert!(world.borrow().grapple_enabled);
        events.update_shared(0.1, &world);
        world.borrow_mut().objects.get_mut(&1).unwrap().position = [1.0, 0.0, 1.0];
        events.update_shared(0.1, &world);
        let mut near: Vec<String> = lua
            .globals()
            .get::<Table>("near")
            .unwrap()
            .sequence_values()
            .map(|v| v.unwrap())
            .collect();
        near.sort();
        assert_eq!(near, ["all", "any"]);
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

    // The 360's area search meets what overlaps a cube round the point
    // (0x82335908): a corner of it further than the radius counts, and a
    // body whose box reaches in counts though its origin is outside.
    #[test]
    fn objects_in_an_area_meet_a_cube_by_their_boxes() {
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            w.insert(10, Object::new("OilCon020gun 01", [0.9, 0.9, 0.9]));
            w.insert(11, Object::new("OilCon020gun 02", [0.0, -1.5, 0.0]));
            w.bounds.insert(11, ([-0.3, -1.8, -0.3], [0.3, -0.9, 0.3]));
            w.insert(12, Object::new("OilCon020gun 03", [0.0, -1.5, 0.0]));
        }
        let n: i64 = host
            .lua()
            .load("return table.getn(Pg.GetObjectsInArea(0, 0, 0, 1, 'OilCon020gun'))")
            .eval()
            .unwrap();
        assert_eq!(n, 2);
    }

    #[test]
    fn objects_in_an_area_are_found_by_name() {
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            w.insert(10, Object::new("RumJug 01", [1.0, 0.0, 0.0]));
            w.insert(11, Object::new("RumJug 02", [9.0, 0.0, 0.0]));
            w.insert(12, Object::new("Crate 01", [0.5, 0.0, 0.0]));
        }
        let n: i64 = host
            .lua()
            .load("return table.getn(Pg.GetObjectsInArea(0, 0, 0, 2, 'RumJug'))")
            .eval()
            .unwrap();
        assert_eq!(n, 1);
    }

    // The scripts count the players (`nPlayers == 2`, sums on it:
    // PirCon004) and go through the list of them.
    #[test]
    fn one_player_is_counted_and_listed() {
        let (host, _) = setup();
        let (n, more, listed): (i64, bool, i64) = host
            .lua()
            .load(
                "local n = Player.GetCurrentPlayers()                  return n * 2, 1 < n, table.getn(Player.GetAllPlayers())",
            )
            .eval()
            .unwrap();
        assert_eq!((n, more, listed), (2, false, 1));
    }

    #[test]
    fn a_custom_event_with_no_network_game_sends_nothing_and_gives_true() {
        let (host, _world) = setup();
        let sent: bool = host
            .lua()
            .load("return Net.SendCustomEvent('MrxTaskRace', 2, { 1 })")
            .eval()
            .unwrap();
        assert!(sent);
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
    fn a_raw_clip_nearly_done_fires_its_event_drawn_or_not() {
        // WifHq's cheap cinematic plays a briefing line's named clip and
        // waits for it to have 0.1 s left before the next line.
        let (host, world) = setup();
        let events = crate::events::Events::default();
        events.install(host.lua()).unwrap();
        host.lua()
            .load(
                r#"
                done = nil
                local her = Player.GetLocalCharacter()
                Human.PlayRawAnimation(her, "all_starter03_job_briefing_spiel", true, true, 0.5, false, true)
                Event.Create(Event.HumanAnimationNearlyCompleted, { her, 0.1 },
                    function() done = true end, {})
                "#,
            )
            .exec()
            .unwrap();
        // Its length unknown yet: it waits.
        events.update_shared(0.5, &world);
        assert!(
            host.lua()
                .globals()
                .get::<Option<u32>>("done")
                .unwrap()
                .is_none()
        );
        // The game says it runs 1 s: not yet at 0.5 s, done once 0.1 s is left.
        world.borrow_mut().raw_clip_length(1, 1.0);
        world.borrow_mut().tick_raw_clips(0.5);
        events.update_shared(0.5, &world);
        assert!(
            host.lua()
                .globals()
                .get::<Option<u32>>("done")
                .unwrap()
                .is_none()
        );
        world.borrow_mut().tick_raw_clips(0.45);
        events.update_shared(0.45, &world);
        assert!(
            host.lua()
                .globals()
                .get::<Option<mlua::Value>>("done")
                .unwrap()
                .is_some()
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
                    function(state, extra) left = state .. " " .. tostring(extra) end,
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
        // Nothing of the engine's own follows the data (as the 360).
        assert_eq!(left, "streaming nil");
        let r: bool = host
            .lua()
            .load(
                "local n, f = math.randi(3), Math.randf(2, 2)
                 return n >= 1 and n <= 3 and f >= 2 and f < 3",
            )
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
        assert_eq!(seated, "in:0x00000001/0x00000032");
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
    fn voice_banks_load_by_their_localised_names() {
        let (host, world) = setup();
        host.lua()
            .load(
                r#"
                log = {}
                Sound.LoadTempBank("vo_job_pmc.english", "soundbank", function(name, t, late)
                    table.insert(log, name .. " " .. t .. " " .. tostring(late))
                end, { "vo_job_pmc", "soundbank", false })
                Sound.LoadBankWithCallback("ambience_jungle", "wavebank", function() end)
                Sound.UnloadTempBank("vo_job_pmc.english", "soundbank")
                "#,
            )
            .exec()
            .unwrap();
        assert_eq!(
            world.borrow_mut().take_commands(),
            [
                Command::VoiceBank {
                    name: "vo_job_pmc.english".into(),
                    load: true
                },
                Command::VoiceBank {
                    name: "vo_job_pmc.english".into(),
                    load: false
                },
            ]
        );
        assert!(answer_asset_loads(&world).is_empty());
        let first: String = host.lua().load("return log[1]").eval().unwrap();
        assert_eq!(first, "vo_job_pmc soundbank false");
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
                haste: None,
                path: None,
            }
        );
        assert!(goal_finished(&world, g).is_empty());
        let done: String = host.lua().globals().get("done").unwrap();
        assert_eq!(done, "arrived!");
    }

    #[test]
    fn inside_boundary_reads_the_region_round_an_object() {
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            w.insert(
                70,
                Object::new("mechanicHQ.rgn.inside", [100.0, 0.0, 100.0]),
            );
            w.insert(71, Object::new("Monster Truck", [103.0, 0.0, 98.0]));
            w.insert(
                72,
                Object::new("Offroad Motorcycle (GR)", [120.0, 0.0, 100.0]),
            );
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
        let (truck, bike): (bool, bool) = host
            .lua()
            .load(
                "local r = Pg.GetGuidByName('mechanicHQ.rgn.inside') \
                 return Object.InsideBoundary(Pg.GetGuidByName('Monster Truck'), r), \
                 Object.InsideBoundary(Pg.GetGuidByName('Offroad Motorcycle (GR)'), r)",
            )
            .eval()
            .unwrap();
        assert_eq!((truck, bike), (true, false));
    }

    /// `Player.IsPositionOutBoundary` over the play-area boundaries, and the
    /// PDA's beacon check built on it (`MrxGuiPda.HandleBeaconCheck`: the
    /// map's spot less 35 and 40, a beacon allowed when that is in bounds).
    #[test]
    fn a_spot_is_out_of_bounds_outside_every_play_area_boundary() {
        let (host, world) = setup();
        let lua = host.lua();
        let out = |x: f64, z: f64| -> Option<bool> {
            lua.load(format!(
                "return Player.IsPositionOutBoundary(Player.GetLocalPlayer(), {x}, 0, {z})"
            ))
            .eval()
            .unwrap()
        };
        // No boundaries: nowhere is out.
        assert_eq!(out(5000.0, 5000.0), Some(false));
        let square = |half: f32| m2_assets::Region {
            flags: 0x0004_0002,
            unk_0x04: 0,
            corners: vec![[-half, -half], [half, -half], [half, half], [-half, half]],
        };
        {
            let mut w = world.borrow_mut();
            // A play area round OilCon020's tutorial box, and an HQ
            // exclusion far off (not part of this test).
            w.insert(80, Object::new("BoundaryVZ", [2750.0, 0.0, -425.0]));
            w.regions.insert(80, square(500.0));
            w.insert(81, Object::new("Boundary_ALHQ_Exclusion", [0.0, 0.0, 0.0]));
            w.regions.insert(81, square(100.0));
        }
        lua.load(
            "local p = Player.GetLocalPlayer() \
             Player.AddBoundary(p, Pg.GetGuidByName('BoundaryVZ')) \
             Player.AddBoundary(p, Pg.GetGuidByName('Boundary_ALHQ_Exclusion'))",
        )
        .exec()
        .unwrap();
        assert_eq!(out(2750.0, -425.0), Some(false));
        assert_eq!(out(3500.0, -425.0), Some(true));
        // Inside only the exclusion: the exclusion list isn't tested.
        assert_eq!(out(0.0, 0.0), Some(true));
        // Bad arguments: nil.
        let bad: Option<bool> = lua
            .load("return Player.IsPositionOutBoundary(nil, 1, 2, 3)")
            .eval()
            .unwrap();
        assert_eq!(bad, None);
        // The beacon check's answer for a map spot "x,z".
        let check = |spot: &str| -> bool {
            lua.load(format!(
                "local n = {{}} \
                 for v in string.gmatch('{spot}', '-*%d+') do n[#n + 1] = tonumber(v) end \
                 local f = Player.IsPositionOutBoundary \
                 return f ~= nil and not f(Player.GetLocalPlayer(), n[1] - 35, 0, n[2] - 40)"
            ))
            .eval()
            .unwrap()
        };
        assert!(check("2785,-385"));
        assert!(!check("3600,-385"));
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
        assert_eq!(arrived, "0x00000001@0x00000046");
    }

    // PmcCon034's out-of-bounds: the region an "exit" waits on is watched,
    // one an "enter" waits on is not.
    #[test]
    fn regions_waited_on_for_an_exit_are_watched() {
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            let corners = vec![[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0]];
            for (g, name) in [(70, "OutOfBounds"), (71, "Checkpoint")] {
                w.insert(g, Object::new(name, [100.0, 0.0, 100.0]));
                w.regions.insert(
                    g,
                    m2_assets::Region {
                        flags: 0x0004_0002,
                        unk_0x04: 0,
                        corners: corners.clone(),
                    },
                );
            }
        }
        let events = crate::events::Events::default();
        events.install(host.lua()).unwrap();
        host.lua()
            .load(
                r#"
                local f = ObjectFilter.Create()
                ObjectFilter.UsePlayers(f, true)
                Event.Create(Event.Boundary, {f, "OutOfBounds", "exit", true}, function() end)
                Event.Create(Event.Boundary, {f, "Checkpoint", "enter", true}, function() end)
                "#,
            )
            .exec()
            .unwrap();
        assert_eq!(events.exits_watched(&*world.borrow()), vec![70]);
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
        let faction: Value = host.lua().globals().get("faction").unwrap();
        assert_eq!(
            faction,
            crate::script_hash::hash_value(m2_assets::name_hash("VZ"))
        );
        // A faction's guid is the hash of its name, and both are userdata.
        let same: bool = host
            .lua()
            .load(r#"return String.GetHash("VZ") == faction and type(faction) == "userdata""#)
            .eval()
            .unwrap();
        assert!(same);
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

    // MC-16: WifVzAtmosphere's { char, region, "enter" | "exit", false }
    // boundaries read the direction after the boundary (its line 79).
    #[test]
    fn an_enter_or_exit_boundary_tells_its_callback_the_direction() {
        for mode in ["enter", "exit"] {
            let (host, world) = setup();
            {
                let mut w = world.borrow_mut();
                w.insert(70, Object::new("rgn_atmo_Caracas", [100.0, 0.0, 100.0]));
                let corners = vec![[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0]];
                w.regions.insert(
                    70,
                    m2_assets::Region {
                        flags: 0x0004_0002,
                        unk_0x04: 0,
                        corners,
                    },
                );
                if mode == "exit" {
                    w.objects.get_mut(&1).unwrap().position = [101.0, 0.0, 101.0];
                }
            }
            let events = crate::events::Events::default();
            events.install(host.lua()).unwrap();
            host.lua().globals().set("mode", mode).unwrap();
            host.lua()
                .load(
                    r#"
                    crossed = nil
                    local region = Pg.GetGuidByName("rgn_atmo_Caracas")
                    Event.Create(Event.Boundary,
                        {Player.GetLocalCharacter(), region, mode, false},
                        function(tag, who, where, how)
                            crossed = tag .. " " .. Sys.GuidToString(who) .. "@"
                                .. Sys.GuidToString(where) .. " " .. how
                        end, {"zone"})
                    "#,
                )
                .exec()
                .unwrap();
            assert!(events.update_shared(0.1, &world).is_empty());
            let crossed: Option<String> = host.lua().globals().get("crossed").unwrap();
            assert_eq!(crossed, None, "{mode}");
            let to = if mode == "enter" {
                [103.0, 0.0, 98.0]
            } else {
                [120.0, 0.0, 99.0]
            };
            world.borrow_mut().objects.get_mut(&1).unwrap().position = to;
            assert!(events.update_shared(0.1, &world).is_empty());
            let crossed: String = host.lua().globals().get("crossed").unwrap();
            assert_eq!(crossed, format!("zone 0x00000001@0x00000046 {mode}"));
        }
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
        // Disguise.Init keeps the investigator callback (0x82340B20).
        let ok: bool = host
            .lua()
            .load(
                "seen = {} return Disguise.Init({InvestigatorCallback =                  function(p, ai, came) table.insert(seen, came) end})",
            )
            .eval()
            .unwrap();
        assert!(ok);
        investigator(host.lua(), Some(1), 9, true).unwrap();
        let seen: Vec<bool> = host.lua().load("return seen").eval().unwrap();
        assert_eq!(seen, [true]);
        // Two answers (0x82340438): disguised, and who she is seen as.
        world.borrow_mut().disguise_seen_as = Some(42);
        let (held, seen): (bool, f64) = host
            .lua()
            .load("return Player.GetVehicleDisguiseState({})")
            .eval()
            .unwrap();
        assert_eq!((held, seen), (true, 42.0));
        ask("Player.SetVehicleDisguise(false)");
        assert_eq!(world.borrow().disguise, None);
        assert_eq!(ask("return Player.GetVehicleDisguiseState({})"), Value::Nil);
    }

    #[test]
    fn a_label_filter_lists_nothing_but_matches_what_carries_it() {
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            let mut statue = Object::new("_pmcoutpost_column", [0.0; 3]);
            statue.labels.push("PMCCon031StatueGL".into());
            w.insert(70, statue);
            w.insert(71, Object::new("_pmcoutpost_column", [0.0; 3]));
        }
        let n: i64 = host
            .lua()
            .load(
                "local f = ObjectFilter.Create() ObjectFilter.SetFilter(f, 'PMCCon031StatueGL') \
                 local n = #ObjectFilter.GetObjects(f) \
                 if ObjectFilter.Eval(f, Sys.StringToGuid('0x46')) then n = n + 10 end \
                 if ObjectFilter.Eval(f, Sys.StringToGuid('0x47')) then n = n + 100 end \
                 return n",
            )
            .eval()
            .unwrap();
        // None listed; the statue (70) matches, the plain column (71) not.
        assert_eq!(n, 10);
    }

    #[test]
    fn a_label_expression_takes_alternatives_in_brackets() {
        let (_host, world) = setup();
        let mut w = world.borrow_mut();
        w.objects.get_mut(&1).unwrap().labels.push("Hero".into());
        let mut allied = Object::new("al_veh_m151", [0.0; 3]);
        allied.labels.extend(["Allied".into(), "Vehicle".into()]);
        w.insert(60, allied);
        let mut soldier = Object::new("al_soldier", [0.0; 3]);
        soldier.labels.push("Allied".into());
        w.insert(61, soldier);
        // The friendly gates' filter: the hero, or an Allied vehicle.
        assert_eq!(w.labelled("Hero||(Allied&&Vehicle)"), vec![1, 60]);
        assert_eq!(w.labelled("Allied && !Vehicle"), vec![61]);
    }

    #[test]
    fn friendly_gates_know_her_only_as_she_honks() {
        let (_host, world) = setup();
        let mut w = world.borrow_mut();
        w.gates.insert(70);
        w.objects.get_mut(&1).unwrap().vehicle = Some(50);
        // She and her jeep go unheard by the gate, not by anything else;
        // others are seen as ever.
        assert!(w.unheard_at_gate(1, 70) && w.unheard_at_gate(50, 70));
        assert!(!w.unheard_at_gate(1, 71) && !w.unheard_at_gate(99, 70));
        w.honk_heard = HONK_HEARD;
        assert!(!w.unheard_at_gate(1, 70) && !w.unheard_at_gate(50, 70));
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
        assert_eq!(dead, ["0x0000003C", "0x0000003D"]);
    }

    // Only the template database's names are templates: `HqInterior`
    // looks like one but is the name the HQ scripts give the interiors
    // they spawn, and with none about it is nil, so `MrxUtil.SpawnActor`
    // spawns one.
    #[test]
    fn only_the_databases_names_are_templates() {
        let (host, world) = setup();
        assert!(world.borrow().is_template("Supply Drop (GL)"));
        assert!(world.borrow().is_template("oc"));
        assert!(!world.borrow().is_template("HqInterior"));
        let (interior, drop): (bool, bool) = host
            .lua()
            .load(
                r#"return Pg.GetGuidByName("HqInterior") == nil,
                    Pg.GetGuidByName("Supply Drop (GL)") ~= nil"#,
            )
            .eval()
            .unwrap();
        assert!(interior && drop);
        assert!(world.borrow().by_name("HqInterior").is_none());
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
                    __m2_object_name(cargo), Pg.GetGuidByName("vza001_gate") == nil
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
                .load(r#"return Object.IsWinched(Sys.StringToGuid("82")) ~= nil"#)
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
        assert_eq!(dropped, "0x00000052");
    }

    #[test]
    fn a_squad_is_the_people_placed_under_its_name() {
        let objects = [
            (0x1d96, "oc001.obj1.oc1 0x10001d96"),
            (0x1d9e, "oc001.obj1.vz1 0x10001d9e"),
            (0x1d9b, "oc001.obj1.vz1 0x10001d9b"),
            (0x1daa, "M151 .50Cal (VZ) 0x10001daa"),
            (0x1dab, "oc001.obj1.vz10 0x10001dab"),
        ];
        assert_eq!(
            super::squad_members("oc001.obj1.vzsquad1", objects.into_iter()),
            [0x1d9b, 0x1d9e]
        );
        assert_eq!(
            super::squad_members("oc001.obj1.ocsquad1", objects.into_iter()),
            [0x1d96]
        );
        assert!(super::squad_members("PirSquad", objects.into_iter()).is_empty());
    }

    #[test]
    fn a_delivery_hears_its_cargo_let_go_by_any_carrier() {
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            w.insert(80, Object::new("copter", [0.0, 100.0, 0.0]));
            let mut pilot = Object::new("pilot", [0.0, 100.0, 0.0]);
            pilot.vehicle = Some(80);
            w.insert(81, pilot);
            w.insert(82, Object::new("post", [0.0, 95.0, 0.0]));
        }
        let events = crate::events::Events::default();
        events.install(host.lua()).unwrap();
        // As `MrxTaskObjectiveDeliver` waits for cargo that reached its
        // place still hanging (OilCon002's listening posts).
        host.lua()
            .load(
                r#"
                heard = nil
                local copter, post = Sys.StringToGuid("80"), Sys.StringToGuid("82")
                Object.AttachCargoToWinch(post, copter)
                Event.Create(Event.ObjectWinched, {post, 0, "any"}, function(c)
                    heard = Object.IsWinched(c) == nil and "free" or "held"
                end)
                "#,
            )
            .exec()
            .unwrap();
        events.update_shared(0.1, &world);
        let heard =
            |host: &ScriptHost| -> Option<String> { host.lua().globals().get("heard").unwrap() };
        assert_eq!(heard(&host), None);
        assert!(arrived(&world, 81, [50.0, 0.0, 60.0]).is_empty());
        events.update_shared(0.1, &world);
        assert_eq!(heard(&host).as_deref(), Some("free"));
    }

    /// `Event.ObjectWinched` as the 360 reads it: the mode by its first
    /// letters (not "a" detach, "an" any, else attach), fired on the hook
    /// or the release, passing cargo, carrier and "attach"/"detach".
    #[test]
    fn winch_events_fire_on_hook_and_release_by_mode() {
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            w.insert(80, Object::new("copter", [0.0, 100.0, 0.0]));
            w.insert(82, Object::new("crate", [0.0, 95.0, 0.0]));
        }
        let events = crate::events::Events::default();
        events.install(host.lua()).unwrap();
        host.lua()
            .load(
                r#"
                log = {}
                local copter, box = Sys.StringToGuid("80"), Sys.StringToGuid("82")
                local names = { [copter] = "80", [box] = "82" }
                local function note(t) return function(cargo, carrier, how)
                    log[#log + 1] = t .. " " .. names[cargo] .. " "
                        .. names[carrier] .. " " .. how
                end end
                Event.CreatePersistent(Event.ObjectWinched, {box, copter, "Attach"}, note("att"))
                Event.CreatePersistent(Event.ObjectWinched, {box, 0, "Detach"}, note("det"))
                Event.CreatePersistent(Event.ObjectWinched, {box, 0, "any"}, note("any"))
                "#,
            )
            .exec()
            .unwrap();
        // The first look records it unhooked: nothing fires.
        events.update_shared(0.1, &world);
        let log = |host: &ScriptHost| -> Vec<String> { host.lua().globals().get("log").unwrap() };
        assert!(log(&host).is_empty());
        host.lua()
            .load("Object.AttachCargoToWinch(Sys.StringToGuid('82'), Sys.StringToGuid('80'))")
            .exec()
            .unwrap();
        events.update_shared(0.1, &world);
        let mut got = log(&host);
        got.sort();
        assert_eq!(got, ["any 82 80 attach", "att 82 80 attach"]);
        host.lua()
            .load("Object.DetachCargoFromWinch(Sys.StringToGuid('82'))")
            .exec()
            .unwrap();
        events.update_shared(0.1, &world);
        let mut got = log(&host);
        got.sort();
        assert_eq!(
            got,
            [
                "any 82 80 attach",
                "any 82 80 detach",
                "att 82 80 attach",
                "det 82 80 detach"
            ]
        );
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
        assert_eq!(guid, "0x00000010");
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
                return __m2_object_name(Vehicle.GetDriver(copter)), #Vehicle.GetRiders(copter),
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
    fn a_passenger_is_never_the_driver() {
        // PmcCon003's taxi: she (guid 1, the lowest) boards Ewan's
        // helicopter as a passenger; the scripts send its driver home, and
        // that must be its pilot, not her (360: 0x8229C958 takes only the
        // driver's seat).
        let (host, world) = setup();
        let (copter, pilot): (u32, String) = {
            let v = host
                .lua()
                .load(
                    r#"local c = Pg.Spawn("Mi26 (VZA Intro) (Driver)", 0, 50, 0, 0)
                    return c, __m2_object_name(Vehicle.GetDriver(c))"#,
                )
                .eval::<(Value, String)>()
                .unwrap();
            (id(Some(&v.0)).unwrap(), v.1)
        };
        assert_eq!(pilot, PILOT);
        world.borrow_mut().seat(1, copter, false, true);
        let (driver, passengers, others): (String, i64, i64) = host
            .lua()
            .load(
                r#"local c = ...
                return __m2_object_name(Vehicle.GetDriver(c)), #Vehicle.GetRiders(c, "p"),
                    #Vehicle.GetRiders(c)"#,
            )
            .call(crate::events::guid_value(copter))
            .unwrap();
        assert_eq!((driver.as_str(), passengers, others), (PILOT, 1, 2));
        // Seated by the scripts with a seat letter: `"p"` rides, `"d"`
        // drives; out again, the seat no longer counts.
        let jeep: Value = host
            .lua()
            .load(
                r#"local j = Pg.Spawn("M151 Softtop (VZ)", 5, 0, 0, 0)
                Vehicle.Enter(j, Sys.StringToGuid("1"), "p", true)
                assert(Vehicle.GetDriver(j) == nil)
                Vehicle.Exit(j, Sys.StringToGuid("1"))
                Vehicle.Enter(j, Sys.StringToGuid("1"), "D", true)
                return j"#,
            )
            .eval()
            .unwrap();
        let j = id(Some(&jeep)).unwrap();
        assert_eq!(world.borrow().driver_of(j), Some(1));
        assert!(seat_not_driving("p") && seat_not_driving("G") && !seat_not_driving("d"));
        assert!(!seat_not_driving(""));
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
                return __m2_object_name(Vehicle.GetDriver(copter))"#,
            )
            .eval()
            .unwrap();
        assert_eq!(pilot, "Recruit Ewen Garret");
    }

    #[test]
    fn the_current_players_are_a_count_and_all_players_a_list() {
        let (host, _world) = setup();
        let (more, n): (bool, usize) = host
            .lua()
            .load("return 1 < Player.GetCurrentPlayers(), #Player.GetAllPlayers()")
            .eval()
            .unwrap();
        assert_eq!((more, n), (false, 1));
    }

    #[test]
    fn an_object_removed_alive_is_gone_not_dead() {
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            w.insert(80, Object::new("JetCon001_BunkerBuster03", [0.0; 3]));
            w.insert(81, Object::new("VZ Soldier", [0.0; 3]));
        }
        host.lua()
            .load("Object.Remove(Pg.GetGuidByName('JetCon001_BunkerBuster03')) Object.Kill(Pg.GetGuidByName('VZ Soldier'))")
            .exec()
            .unwrap();
        let w = world.borrow();
        assert_eq!((w.died(80), w.died(81)), (false, true));
    }

    #[test]
    fn a_gate_the_scripts_shut_is_fully_closed_until_opened() {
        let (host, world) = setup();
        let closed = |w: &Rc<RefCell<ScriptWorld>>| w.borrow().physics_event(7, "gatefullyclosed");
        assert_eq!(closed(&world), Some(false));
        host.lua().load("Object.CloseGate(7)").exec().unwrap();
        assert_eq!(closed(&world), Some(true));
        assert_eq!(world.borrow().physics_event(7, "gatestuck"), Some(false));
        host.lua().load("Object.OpenGate(7)").exec().unwrap();
        assert_eq!(closed(&world), Some(false));
        assert_eq!(world.borrow().physics_event(7, "vehiclesinking"), None);
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
    fn load_game_says_a_save_loads_only_while_one_is_pending() {
        let (host, world) = setup();
        let ask = || -> bool {
            host.lua()
                .load(r#"return Pg.LoadGame("InitialSaveData")"#)
                .eval()
                .unwrap()
        };
        assert!(!ask(), "a new game");
        world.borrow_mut().saved = Some("{}".into());
        assert!(ask(), "a save to load");
    }

    #[test]
    fn a_landing_on_a_target_lands_where_the_target_is() {
        let (host, world) = setup();
        world
            .borrow_mut()
            .insert(60, Object::new("TransitHeli_Landing", [7.0, 8.0, 9.0]));
        host.lua()
            .load(
                r#"Ai.Goal({ AIGuid = Sys.StringToGuid("0x1"), Goal = "HeliLand",
                    Target = Pg.GetGuidByName("TransitHeli_Landing"), Haste = 0.3 })"#,
            )
            .exec()
            .unwrap();
        assert!(matches!(
            world.borrow_mut().take_commands().last(),
            Some(Command::Goal { location: Some(at), target: Some(60), .. }) if *at == [7.0, 8.0, 9.0]
        ));
    }

    #[test]
    fn a_goal_callback_hears_who_had_the_goal_and_that_it_was_done() {
        let (host, world) = setup();
        host.lua()
            .load(
                r#"Ai.Goal({ AIGuid = Sys.StringToGuid("0x1"), Goal = "HeliLand",
                    Location = { 1, 2, 3 }, Callback = function(me, pilot, result)
                        heard = { me, __m2_object_name(pilot), result } end,
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
    fn an_outpost_tells_its_attackers_from_its_defenders() {
        // `Outpost.CallForRushers` collects one side's people by the
        // capture point; `RusherGoalFulfilled` then compares each one's
        // faction with the outpost's `sAttackers` and `sDefenders`.
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            for (g, name) in [(70, "Chinese Soldier"), (71, "VZ Heavy (Light MG)")] {
                let mut o = Object::new(name, [5.0, 0.0, 0.0]);
                o.labels.push("Soldier".into());
                w.insert(g, o);
            }
        }
        let (china, all, same, other): (i64, i64, bool, bool) = host
            .lua()
            .load(
                r#"local china = Pg.FastCollectHumans(0, 0, 0, 50, "China")
                local all = Pg.FastCollectHumans(0, 0, 0, 50, "China||VZ")
                local c = Ai.GetFactionGuid(china[1])
                return #china, #all, c == Pg.GetGuidByName("China"),
                    c == Pg.GetGuidByName("VZ")"#,
            )
            .eval()
            .unwrap();
        assert_eq!((china, all, same, other), (1, 2, true, false));
    }

    #[test]
    fn a_follow_role_reaches_the_game_and_another_role_ends_it() {
        // `MrxFollow` gives a released VIP `Follow` on her, and `Idle`
        // when it lets him go.
        let (host, world) = setup();
        host.lua()
            .load(
                r#"Ai.Role({ AIGuid = Sys.StringToGuid("0x70"), Role = "Follow",
                    Target = Sys.StringToGuid("0x1"), MinDistance = 2, MoveDistance = 4,
                    MaxDistance = 30 })
                Ai.Role({ AIGuid = Sys.StringToGuid("0x70"), Role = "Idle" })"#,
            )
            .exec()
            .unwrap();
        assert_eq!(
            world.borrow_mut().take_commands(),
            [
                Command::Follow {
                    guid: 0x70,
                    target: Some(1),
                    near: 4.0
                },
                Command::Follow {
                    guid: 0x70,
                    target: None,
                    near: 4.0
                }
            ]
        );
    }

    #[test]
    fn a_copied_object_filter_changes_on_its_own() {
        // `MrxTaskObjectiveRelease` copies its targets' filter, takes the
        // players out of the copy (a list of them), and drops each target
        // from it as she comes near.
        let (host, _) = setup();
        let (orig, copy, players): (i64, i64, bool) = host
            .lua()
            .load(
                r#"local f = ObjectFilter.Create()
                ObjectFilter.AddObject(f, Sys.StringToGuid("0x70"))
                ObjectFilter.AddObject(f, Sys.StringToGuid("0x71"))
                ObjectFilter.UsePlayers(f, true)
                local c = ObjectFilter.Copy(f)
                ObjectFilter.RemoveObject(c, { Sys.StringToGuid("0x70") })
                return #f.objs, #c.objs, c.players"#,
            )
            .eval()
            .unwrap();
        assert_eq!((orig, copy, players), (2, 1, true));
    }

    #[test]
    fn relations_are_kept_in_order_clamped_and_read_through_the_templates() {
        // PmcCon018 reads OC's regard for the PMC, sets its own, and puts
        // the old one back at the end.
        let (host, world) = setup();
        world
            .borrow_mut()
            .relations
            .insert(("oc".into(), "pmc".into()), 25.0);
        let (authored, set, clamped, reverse, same, by_name): (f64, bool, f64, f64, f64, f64) =
            host.lua()
                .load(
                    "local oc, pmc = Pg.GetGuidByName('OC'), Pg.GetGuidByName('PMC')
                     local authored = Ai.GetRelation(oc, pmc)
                     local set = Ai.SetRelation(oc, pmc, -250)
                     return authored, set, Ai.GetRelation(oc, pmc),
                       Ai.GetRelation(pmc, oc), Ai.GetRelation(oc, oc),
                       Ai.GetRelation('OC', 'PMC')",
                )
                .eval()
                .unwrap();
        // The templates' value where no script has set the pair.
        assert_eq!(authored, 25.0);
        assert!(set);
        assert_eq!(clamped, -100.0);
        // Only the pair in that order.
        assert_eq!(reverse, 0.0);
        // A side regards itself fully.
        assert_eq!(same, 100.0);
        assert_eq!(by_name, -100.0);
    }

    #[test]
    fn a_persons_relation_is_their_factions() {
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            w.relations.insert(("vz".into(), "allied".into()), -100.0);
            w.insert(60, Object::new("VZ Soldier (AK)", [0.0; 3]));
        }
        let r: f64 = host
            .lua()
            .load("return Ai.GetRelation(60, Pg.GetGuidByName('Allied'))")
            .eval()
            .unwrap();
        assert_eq!(r, -100.0);
    }

    #[test]
    fn vehicle_doors_open_and_close_on_request() {
        // PmcCon033 hands `Vehicle.OpenDoor` to a timer as its callback.
        let (host, world) = setup();
        world.borrow_mut().take_commands();
        let (kind, n): (String, i64) = host
            .lua()
            .load(
                "local f = Vehicle.OpenDoor
                 local n = select('#', f(50, 'pivot'))
                 Vehicle.CloseDoor(50, 'pivot')
                 return type(f), n",
            )
            .eval()
            .unwrap();
        assert_eq!((kind.as_str(), n), ("function", 0));
        let pivot = m2_assets::name_hash("pivot");
        let mut w = world.borrow_mut();
        assert_eq!(
            w.take_commands(),
            [
                Command::Door {
                    guid: 50,
                    door: pivot,
                    open: true
                },
                Command::Door {
                    guid: 50,
                    door: pivot,
                    open: false
                },
            ]
        );
        assert_eq!(w.doors.get(&(50, pivot)), Some(&false));
        assert!(host.take_calls().is_empty());
    }

    #[test]
    fn remove_goal_clears_a_persons_goals_and_set_state_is_kept() {
        let (host, world) = setup();
        world.borrow_mut().take_commands();
        let (cleared, other, nobody, state, gone): (bool, bool, bool, bool, bool) = host
            .lua()
            .load(
                "return Ai.RemoveGoal{ AIGuid = 50, Handle = 0 },
                   Ai.RemoveGoal{ AIGuid = 50, Handle = 7 },
                   Ai.RemoveGoal{ AIGuid = 999, Handle = 0 },
                   Ai.SetState{ AIGuid = 50, State = 'Vip' },
                   Ai.SetState{ AIGuid = 999, State = 'Vip', Value = false }",
            )
            .eval()
            .unwrap();
        assert!(cleared && !other && !nobody && state && !gone);
        let mut w = world.borrow_mut();
        assert_eq!(
            w.take_commands(),
            [
                Command::ClearGoals { guid: 50 },
                Command::AiState {
                    guid: 50,
                    state: "Vip".into(),
                    on: true
                },
            ]
        );
        assert_eq!(w.ai_states.get(&(50, "vip".into())), Some(&true));
    }

    #[test]
    fn the_scripts_start_and_stop_helicopter_waves() {
        // PmcCon003's first wave: one Alouette at once, 200 m off.
        let (host, world) = setup();
        let n: i64 = host
            .lua()
            .load(
                "local alouette = Pg.GetGuidByName('Carbine')
                 return select('#', Pg.StartHeliWaveSpawner({
                   { NumToSpawn = 1, WaveDelay = 0, SpawnDist = 200, units = { { alouette, 1 } } },
                   { WaveDelay = 8, units = { { alouette } } },
                 }))",
            )
            .eval()
            .unwrap();
        assert_eq!(n, 0);
        world.borrow_mut().take_commands();
        {
            let mut w = world.borrow_mut();
            let mut waves = std::mem::take(&mut w.heli_waves);
            waves.update(0.1, &mut *w);
            w.heli_waves = waves;
            assert_eq!(w.heli_waves.following().len(), 1);
            assert!(w.take_commands().iter().any(
                |c| matches!(c, Command::Spawn { what, .. } if what.eq_ignore_ascii_case("carbine"))
            ));
        }
        host.lua().load("Pg.StopHeliWaveSpawner()").exec().unwrap();
        assert!(!world.borrow().heli_waves.running());
    }

    #[test]
    fn a_point_is_tested_against_a_boundarys_region() {
        // GurCon001: Pg.IsPointInBoundary(nX, nY, nZ, LR_TowerBase).
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            w.insert(70, Object::new("LR_TowerBase", [100.0, 0.0, 100.0]));
            w.regions.insert(
                70,
                m2_assets::Region {
                    flags: 0x0004_0002,
                    unk_0x04: 0,
                    corners: vec![[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0]],
                },
            );
        }
        let (inside, outside, no_region, bad): (bool, bool, bool, Value) = host
            .lua()
            .load(
                "return Pg.IsPointInBoundary(103, 0, 98, 70), Pg.IsPointInBoundary(120, 0, 100, 70),
                   Pg.IsPointInBoundary(103, 0, 98, 50), Pg.IsPointInBoundary(103, 0, 98)",
            )
            .eval()
            .unwrap();
        assert!(inside && !outside && !no_region);
        assert!(bad.is_nil());
    }

    #[test]
    fn feelings_are_nil_until_set_and_need_a_person() {
        // OilCon021: Ai.GetFeeling(mailman, her) <= -33 means hostile.
        let (host, _) = setup();
        let (before, set, after, nobody, unset): (Value, bool, f64, bool, Value) = host
            .lua()
            .load(
                "local before = Ai.GetFeeling(50, 1)
                 local set = Ai.SetFeeling(50, 1, -40)
                 return before, set, Ai.GetFeeling(50, 1), Ai.SetFeeling(999, 1, 5),
                   Ai.GetFeeling(1, 50)",
            )
            .eval()
            .unwrap();
        assert!(before.is_nil() && unset.is_nil());
        assert!(set && !nobody);
        assert_eq!(after, -40.0);
    }

    #[test]
    fn an_infraction_goes_to_the_game_for_its_witnesses() {
        // AllCon003: Ai.AddInfraction(her, China, 100).
        let (host, world) = setup();
        world.borrow_mut().take_commands();
        let (given, one, bad): (bool, bool, Value) = host
            .lua()
            .load(
                "local china = Pg.GetGuidByName('Allied')
                 return Ai.AddInfraction(1, china, 100), Ai.AddInfraction(1, china),
                   Ai.AddInfraction(1)",
            )
            .eval()
            .unwrap();
        assert!(given && one && bad.is_nil());
        let commands = world.borrow_mut().take_commands();
        let amounts: Vec<f32> = commands
            .iter()
            .filter_map(|c| match c {
                Command::Infraction { who: 1, amount, .. } => Some(*amount),
                _ => None,
            })
            .collect();
        assert_eq!(amounts, [100.0, 1.0]);
    }

    #[test]
    fn angles_cross_the_script_boundary_in_degrees() {
        // Scripts spawn turned 90 and read yaws back in degrees; objects
        // keep radians.
        let (host, world) = setup();
        let (spawned, set, bad, turned, camera): (f64, bool, Value, f64, f64) = host
            .lua()
            .load(
                "local g = Pg.Spawn('Veyron', 0, 0, 0, 90)
                 local spawned = Object.GetYaw(g)
                 local set = Object.SetYaw(50, 180)
                 return spawned, set, Object.SetYaw(999, 10), Object.GetYaw(50),
                   Camera.GetYaw(Player.GetCamera())",
            )
            .eval()
            .unwrap();
        assert!((spawned - 90.0).abs() < 1e-3);
        assert!(set && bad.is_nil());
        assert!((turned - 180.0).abs() < 1e-3);
        let w = world.borrow();
        assert!((w.objects[&50].yaw - std::f32::consts::PI).abs() < 1e-5);
        let her = f64::from(w.objects[&1].yaw.to_degrees());
        assert!((camera - her).abs() < 1e-3);
    }

    #[test]
    fn natives_answer_as_the_360_ones_do() {
        let (host, _) = setup();
        let lua = host.lua();
        let ask = |code: &str| -> Value { lua.load(code).eval().unwrap() };
        let handle = |v: Value| v.as_number().or(v.as_integer().map(|i| i as f64));
        // Ai.Goal: a handle per goal; nil for no AI.
        let a = handle(ask("return Ai.Goal{ AIGuid = 50, Goal = 'MoveTo' }")).unwrap();
        let b = handle(ask(
            "return Ai.Goal{ AIGuid = 50, Goal = 'MoveTo', Force = true }",
        ))
        .unwrap();
        assert!(b > a && a >= 1.0);
        assert!(ask("return Ai.Goal{ AIGuid = 999, Goal = 'MoveTo' }").is_nil());
        // Sys.GuidToString: "0x%08X".
        let s: String = lua
            .load("return Sys.GuidToString(Sys.StringToGuid('0x2a'))")
            .eval()
            .unwrap();
        assert_eq!(s, "0x0000002A");
        assert!(ask("return Sys.GuidToString(nil)").is_nil());
        // Object.IsPlayerControlled: her player, not a number.
        assert_eq!(
            ask("return Object.IsPlayerControlled(1) == Player.GetLocalPlayer()"),
            Value::Boolean(true)
        );
        assert!(ask("return Object.IsPlayerControlled(50)").is_nil());
        // True from these.
        for code in [
            "return Sys.RequestGameState('Game')",
            "return Object.SetInvincible(50, true, 'test')",
            "return Player.IsLocal(0)",
            "return Player.IsLocal(Player.GetLocalPlayer())",
        ] {
            assert_eq!(ask(code), Value::Boolean(true), "{code}");
        }
        assert_eq!(ask("return Player.IsLocal(1)"), Value::Boolean(false));
        // Object.GetPosition: one nil for no object.
        let n: i64 = lua
            .load("return select('#', Object.GetPosition(999))")
            .eval()
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn find_point_from_camera_defaults_and_turns_in_degrees() {
        // A support delivery: Pg.FindPointFromCamera(300, 200, -1, owner).
        let (host, world) = setup();
        let (at, yaw) = {
            let w = world.borrow();
            (w.objects[&1].position, w.objects[&1].yaw)
        };
        let point = |code: &str| -> [f32; 3] {
            let (x, y, z): (f32, f32, f32) = host.lua().load(code).eval().unwrap();
            [x, y, z]
        };
        let ahead = |d: f32, up: f32, turn: f32| {
            let (s, c) = (yaw + turn).sin_cos();
            [at[0] + s * d, at[1] + up, at[2] + c * d]
        };
        let near = |p: [f32; 3], q: [f32; 3]| p.iter().zip(q).all(|(a, b)| (a - b).abs() < 1e-3);
        assert!(near(
            point("return Pg.FindPointFromCamera()"),
            ahead(10.0, 1.0, 0.0)
        ));
        assert!(near(
            point("return Pg.FindPointFromCamera(300, 200, -1, Player.GetLocalPlayer())"),
            ahead(300.0, 200.0, 0.0)
        ));
        assert!(near(
            point("return Pg.FindPointFromCamera(20, 0, -1, nil, 90)"),
            ahead(20.0, 0.0, std::f32::consts::FRAC_PI_2)
        ));
    }

    #[test]
    fn more_natives_answer_as_the_360_ones_do() {
        let (host, world) = setup();
        crate::hijack::install(&host, &world).unwrap();
        let lua = host.lua();
        let ask = |code: &str| -> Value { lua.load(code).eval().unwrap() };
        for code in [
            "return Object.SetInfiniteAmmo(1)",
            "return Object.SetTransformToObject(1, 50)",
            "return Player.SetAimMode(Player.GetLocalPlayer(), true)",
            "return Net.SetLoadingScreen(false)",
            "return Object.Detach(50, 1)",
            "return Ai.Enable(50, true)",
            "return Vehicle.EnableTurret(50, 'main_turret', true)",
        ] {
            assert_eq!(ask(code), Value::Boolean(true), "{code}");
        }
        assert_eq!(ask("return Object.Detach(50)"), Value::Boolean(false));
        assert_eq!(ask("return Ai.Enable(999, true)"), Value::Boolean(false));
        assert!(ask("return Pg.AddContextAction(50)").is_nil());
        assert_eq!(
            ask("return Ai.Role{ AIGuid = 50, Role = 'Follow', Target = 1 }"),
            Value::Integer(1)
        );
        assert_eq!(
            ask("return Ai.Role{ AIGuid = 999, Role = 'Follow' }"),
            Value::Integer(0)
        );
    }

    #[test]
    fn infinite_ammo_defaults_on_and_comes_off_again() {
        let (host, world) = setup();
        {
            let mut w = world.borrow_mut();
            w.inventory.max_reserve.insert("carbine".into(), 90);
        }
        host.lua()
            .load("Human.Inventory.SetAllWeapons(Player.GetLocalCharacter(), { Pg.GetGuidByName('Carbine') })")
            .exec()
            .unwrap();
        let weapons = world.borrow().inventory.of(1).to_vec();
        assert_eq!(weapons.len(), 1);
        let reserve = |world: &Shared| world.borrow().inventory.carried[&weapons[0]].reserve;
        host.lua().load("Object.SetInfiniteAmmo(1)").exec().unwrap();
        assert_eq!(reserve(&world), INFINITE_RESERVE);
        host.lua()
            .load("Object.SetInfiniteAmmo(1, false)")
            .exec()
            .unwrap();
        assert_eq!(reserve(&world), 90);
    }

    #[test]
    fn attach_copies_a_template_and_attaches_an_object_itself() {
        let (host, world) = setup();
        crate::hijack::install(&host, &world).unwrap();
        world.borrow_mut().take_commands();
        // A hijack's pistol: a template, made new at her hand.
        let (ok, pistol, template): (bool, Value, Value) = host
            .lua()
            .load(
                "local t = Pg.GetGuidByName('Carbine')
                 local ok, g = Object.Attach(1, 'bone_attach_rhand', t)
                 return ok, g, t",
            )
            .eval()
            .unwrap();
        assert!(ok);
        let (pistol, template) = (
            crate::events::as_guid(&pistol).unwrap(),
            crate::events::as_guid(&template).unwrap(),
        );
        assert!(pistol >= FIRST_SPAWNED && pistol != template);
        // MrxTaskObjectiveVerify: an object in play goes to her left hand
        // itself.
        let (ok, same): (bool, bool) = host
            .lua()
            .load(
                "local ok, g = Object.Attach(1, 'bone_attach_lhand', 50)
                 return ok, g == Sys.StringToGuid('50')",
            )
            .eval()
            .unwrap();
        assert!(ok && same);
        let props: Vec<u32> = world
            .borrow_mut()
            .take_commands()
            .iter()
            .filter_map(|c| match c {
                Command::Hijack(crate::hijack::HijackCommand::Attach { prop, .. }) => Some(*prop),
                _ => None,
            })
            .collect();
        assert_eq!(props, [pistol, 50]);
    }

    #[test]
    fn natives_past_100_answer_as_the_360_ones_do() {
        let (host, world) = setup();
        crate::hijack::install(&host, &world).unwrap();
        let lua = host.lua();
        let ask = |code: &str| -> Value { lua.load(code).eval().unwrap() };
        for code in [
            "return Object.SetHealth(50, 40)",
            "return Object.AddLabel(50, 'Target')",
            "return Object.SetName(50, 'Renamed')",
            "return Ai.SetInfractionMultiplier(Pg.GetGuidByName('OC'), 0.2)",
            "return Player.SetCinematicMode(Player.GetLocalPlayer(), true)",
            "return Player.SetInputEnabled(Player.GetLocalPlayer(), false)",
            "return Object.SetVisible(50, false)",
            "return Camera.Shake(Player.GetCamera(), 'small', 50, 1, 2)",
            "return VO.Cancel(nil, 'line')",
            "return Ai.Deploy{ Vehicle = 50 }",
        ] {
            assert_eq!(ask(code), Value::Boolean(true), "{code}");
        }
        assert!(ask("return Object.SetHealth(999, 40)").is_nil());
        assert_eq!(
            ask("return Object.AddLabel(999, 'Target')"),
            Value::Boolean(false)
        );
        assert!(ask("return Object.SetName(999, 'x')").is_nil());
        assert_eq!(
            ask("return Object.SetVisible(999, true)"),
            Value::Boolean(false)
        );
        // Health never goes below 0, and hers never above her most.
        lua.load("Object.SetHealth(50, -5) Object.SetHealth(Player.GetLocalCharacter(), 1e6)")
            .exec()
            .unwrap();
        let w = world.borrow();
        assert_eq!(w.objects[&50].health, 0.0);
        assert_eq!(w.objects[&1].health, w.objects[&1].max_health);
        // A multiplier of 1 takes the faction off the list.
        drop(w);
        lua.load("Ai.SetInfractionMultiplier(Pg.GetGuidByName('OC'), 1)")
            .exec()
            .unwrap();
        assert!(!world.borrow().reports.multipliers.contains_key("oc"));
    }

    #[test]
    fn natives_past_160_answer_as_the_360_ones_do() {
        let (host, world) = setup();
        crate::hijack::install(&host, &world).unwrap();
        let lua = host.lua();
        let ask = |code: &str| -> Value { lua.load(code).eval().unwrap() };
        for code in [
            "Object.AddLabel(50, 'Target') return Object.RemoveLabel(50, 'Target')",
            "return Player.SetOutBoundary(Player.GetLocalPlayer(), false)",
            "return Pg.SaveGame('slot')",
            "return Human.Knockdown(50, 2)",
            "return Object.PlayAnimation(50, 'open', false)",
        ] {
            assert_eq!(ask(code), Value::Boolean(true), "{code}");
        }
        assert_eq!(
            ask("return Object.RemoveLabel(999, 'Target')"),
            Value::Boolean(false)
        );
        assert_eq!(ask("return Human.Drop(50)"), Value::Boolean(false));
        assert_eq!(
            ask("return Ai.Deliver(50, 1, 2, 3, 10)"),
            Value::Boolean(false)
        );
        assert!(ask("return Weapon.SetReserveAmmo(999, 3)").is_nil());
        assert!(ask("return Human.Knockdown(nil, 2)").is_nil());
    }

    #[test]
    fn player_and_pursuit_natives_answer_as_the_360_ones_do() {
        let (host, world) = setup();
        world.borrow_mut().regions.insert(
            50,
            m2_assets::Region {
                flags: 0x0004_0002,
                unk_0x04: 0,
                corners: vec![[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0]],
            },
        );
        let lua = host.lua();
        let ask = |code: &str| -> Value { lua.load(code).eval().unwrap() };
        let me = "Player.GetLocalPlayer()";
        for code in [
            format!("return Player.AddBoundary({me}, 50)"),
            format!("return Player.RemoveBoundary({me}, 50)"),
            format!("return Player.RemoveAllBoundary({me})"),
            format!("return Player.SetHealthClamp({me}, true)"),
            format!("return Player.SetSurvivalMode({me}, true)"),
            format!("return Player.SetBoundaryCallback({me}, function() end)"),
            "return Player.SetOutfit(1, 'outfit')".to_owned(),
            format!("return Player.SetGrappleEnabled({me}, true)"),
            "return Player.SetVehicleDisguise(true)".to_owned(),
            "return Pg.SetPursuit(Pg.GetGuidByName('OC'), 2)".to_owned(),
            "return Pg.LockPursuit(Pg.GetGuidByName('OC'), 1)".to_owned(),
        ] {
            assert_eq!(ask(&code), Value::Boolean(true), "{code}");
        }
        assert_eq!(
            ask(&format!("return Player.AddBoundary({me}, 999)")),
            Value::Boolean(false)
        );
        assert_eq!(
            ask(&format!("return Player.RemoveBoundary({me}, 998)")),
            Value::Boolean(false)
        );
        // Locked: a new pursuit doesn't start.
        assert_eq!(
            ask("return Pg.SetPursuit(Pg.GetGuidByName('OC'), 2)"),
            Value::Boolean(false)
        );
        assert_eq!(
            ask("return Pg.SetPursuitSeconds(Pg.GetGuidByName('OC'), 30)"),
            Value::Boolean(false)
        );
        assert!(ask("return Pg.SetPursuit(nil, 2)").is_nil());
    }

    #[test]
    fn camera_shot_and_asset_slots_answer_as_the_360_ones_do() {
        let (host, _) = setup();
        let lua = host.lua();
        let ask = |code: &str| -> Value { lua.load(code).eval().unwrap() };
        assert_eq!(
            ask("return Camera.SetShot(Player.GetCamera(), 'OverShoulder', 1, 50)"),
            Value::Boolean(true)
        );
        assert!(ask("return Camera.SetShot(Player.GetCamera(), 'OverShoulder', 1)").is_nil());
        assert_eq!(
            ask("return Sys.SetAssetRequestMax(8)"),
            Value::Boolean(true)
        );
        assert!(ask("return Sys.SetAssetRequestMax()").is_nil());
    }

    #[test]
    fn ported_natives_follow_the_360_code() {
        let (host, world) = setup();
        world.borrow_mut().take_commands();
        let lua = host.lua();
        let ask = |code: &str| -> Value { lua.load(code).eval().unwrap() };
        // Revive: half her most health, back alive.
        lua.load("Object.Kill(50)").exec().unwrap();
        assert_eq!(ask("return Object.Revive(50, 0.5)"), Value::Boolean(true));
        {
            let w = world.borrow();
            assert!(w.objects[&50].alive);
            assert_eq!(w.objects[&50].health, w.objects[&50].max_health * 0.5);
        }
        assert!(ask("return Object.Revive(999)").is_nil());
        // ApplyImpulse: world space, or its own space for the game to turn.
        assert_eq!(
            ask("return Object.ApplyImpulse(50, 0, 0, 10, true)"),
            Value::Boolean(true)
        );
        assert!(ask("return Object.ApplyImpulse(999, 0, 0, 10)").is_nil());
        // ApplyPointImpulse: push, then point, then local; true.
        assert_eq!(
            ask("return Object.ApplyPointImpulse(50, 0, 0, 50000, 0, -0.5, -1.5, true)"),
            Value::Boolean(true)
        );
        assert!(ask("return Object.ApplyPointImpulse(999, 0, 0, 1, 0, 0, 0)").is_nil());
        assert!(ask("return Object.ApplyPointImpulse(50, 0, 0, 1)").is_nil());
        // GetMass: what the game said, nil before it says.
        assert!(ask("return Object.GetMass(50)").is_nil());
        world.borrow_mut().masses.insert(50, 1500.0);
        assert_eq!(ask("return Object.GetMass(50)"), Value::Number(1500.0));
        // Players: one, in slot 0, never remote.
        assert_eq!(
            ask("return Player.GetPlayer(0) == Player.GetLocalPlayer()"),
            Value::Boolean(true)
        );
        assert!(ask("return Player.GetPlayer(1)").is_nil());
        assert_eq!(ask("return Player.IsRemote(0)"), Value::Boolean(false));
        // The scope: off twice, on once, still off; on again, on.
        lua.load(
            "local p = Player.GetLocalPlayer()
             Player.SetScopeEnabled(p, false) Player.SetScopeEnabled(p, false)
             Player.SetScopeEnabled(p, true)",
        )
        .exec()
        .unwrap();
        assert_eq!(world.borrow().scope_off, 1);
        assert_eq!(
            ask("return Player.SetScopeEnabled(Player.GetLocalPlayer(), true)"),
            Value::Boolean(true)
        );
        assert_eq!(world.borrow().scope_off, 0);
        assert_eq!(
            ask("return Human.SetFireLock(50, true)"),
            Value::Boolean(true)
        );
        assert_eq!(
            ask("return select('#', Sound.SetMasterVolume(0.5, 2))"),
            Value::Integer(0)
        );
        assert_eq!(
            ask("return Camera.SetYaw(Player.GetCamera(), 90)"),
            Value::Boolean(true)
        );
        assert_eq!(
            ask("return select('#', ObjectState.StartEmitter(50, 'Burning', 'fire'))"),
            Value::Integer(0)
        );
        let commands = world.borrow_mut().take_commands();
        assert!(commands.contains(&Command::Impulse {
            guid: 50,
            impulse: [0.0, 0.0, 10.0],
            local: true,
        }));
        assert!(commands.contains(&Command::PointImpulse {
            guid: 50,
            impulse: [0.0, 0.0, 50000.0],
            point: [0.0, -0.5, -1.5],
            local: true,
        }));
        assert_eq!(
            commands
                .iter()
                .filter(|c| matches!(c, Command::PointImpulse { .. }))
                .count(),
            1
        );
        assert!(commands.contains(&Command::ScopeEnabled(false)));
        assert!(commands.contains(&Command::ScopeEnabled(true)));
        assert!(commands.iter().any(|c| matches!(c, Command::CameraYaw(y) if (*y - std::f32::consts::FRAC_PI_2).abs() < 1e-5)));
        assert!(commands.iter().any(|c| matches!(
            c,
            Command::Emitter {
                guid: 50,
                on: true,
                ..
            }
        )));
    }

    #[test]
    fn vo_sequences_are_kept_as_the_360_keeps_them() {
        let (host, world) = setup();
        let lua = host.lua();
        let ask = |code: &str| -> Value { lua.load(code).eval().unwrap() };
        assert_eq!(ask("return VO.AddSequence(50, 5)"), Value::Boolean(true));
        // The speaker holds one numbered higher than 3: refused.
        assert_eq!(ask("return VO.AddSequence(50, 3)"), Value::Boolean(false));
        assert_eq!(ask("return VO.AddSequence(50, 7)"), Value::Boolean(true));
        assert_eq!(ask("return VO.RemoveSequence(50, 5)"), Value::Boolean(true));
        assert_eq!(
            ask("return VO.RemoveSequence(50, 5)"),
            Value::Boolean(false)
        );
        assert!(ask("return VO.AddSequence(50)").is_nil());
        assert_eq!(world.borrow().vo_sequences, [(50, 7)]);
        // At most 32.
        world.borrow_mut().vo_sequences = (0..32).map(|i| (100 + i, 1)).collect();
        assert_eq!(ask("return VO.AddSequence(1, 1)"), Value::Boolean(false));
    }

    #[test]
    fn is_winched_answers_the_helicopter_or_nil() {
        // MrxTaskObjectiveDeliver: `bWinched = Object.IsWinched(o) ~= nil`.
        let (host, _) = setup();
        let (before, carrier, after, unknown): (bool, bool, bool, bool) = host
            .lua()
            .load(
                "local before = Object.IsWinched(50) == nil
                 Object.AttachCargoToWinch(50, 1)
                 local carrier = Object.IsWinched(50) == Sys.StringToGuid('1')
                 Object.DetachCargoFromWinch(50)
                 return before, carrier, Object.IsWinched(50) == nil,
                   Object.IsWinched(999) == nil",
            )
            .eval()
            .unwrap();
        assert!(before && carrier && after && unknown);
    }

    #[test]
    fn a_winch_says_its_state_and_its_cargo() {
        let (host, world) = setup();
        world.borrow_mut().winches.insert(1, 0);
        let (has, none, state, hooked, after, set): (bool, bool, String, bool, bool, String) = host
            .lua()
            .load(
                "local has, none = Object.HasWinch(1), not Object.HasWinch(50)
                 local state = Object.GetWinchState(1)
                 Object.AttachCargoToWinch(50, 1)
                 local hooked = Object.IsWinching(1) == Sys.StringToGuid('50')
                 Object.DetachCargoFromWinch(50)
                 local after = Object.IsWinching(1) == nil
                 Object.SetWinchState(1, 'Deploying')
                 return has, none, state, hooked, after, Object.GetWinchState(1)",
            )
            .eval()
            .unwrap();
        assert!(has && none && hooked && after);
        assert_eq!((state.as_str(), set.as_str()), ("retracted", "deploying"));
        // The names' hashes are the ones 0x823337E8 compares.
        let h: Vec<u32> = WINCH_STATES
            .iter()
            .map(|n| m2_assets::name_hash(n))
            .collect();
        assert_eq!(h, [0xA62D_3A27, 0x07D7_44CF, 0x0A76_5168, 0xBB52_2D10]);
        assert!(host.lua().load("Object.GetWinchState(50)").exec().is_err());
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

#[cfg(test)]
mod distance_tests {
    use super::distance_from;

    #[test]
    fn get_distance_from_measures_flat_and_squared_as_its_flags_say() {
        // PmcCon004's nuke bursts on the bunker's roof, 53 m over its
        // origin and 17 m off it flat: within the script's 20 m only flat.
        let (bunker, burst) = ([-2721.4, 159.5, -3096.5], [-2737.0, 212.5, -3104.4]);
        let full = distance_from(bunker, burst, false, false);
        let flat = distance_from(bunker, burst, true, false);
        assert!(full > 50.0, "{full}");
        assert!(flat < 20.0, "{flat}");
        let squared = distance_from(bunker, burst, true, true);
        assert!((squared - flat * flat).abs() < 1e-2);
    }
}
