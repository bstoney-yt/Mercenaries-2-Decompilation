//! What a tick does: the player on foot (or the free camera), physics and
//! animation, and where the frame is seen from.

use std::collections::{HashMap, HashSet};

use m2_engine::atmosphere::{AtmosphereMix, ScriptAtmosphere};
use m2_engine::durability::{BlastPush, CrashHow, Explosion};
use m2_engine::physics::float_depth;
use m2_engine::{
    CatalogueEntry, Install, Physics, StaticKey, Templates, VehicleControls, VehicleSpec, World,
};
use m2_math::glam::{Mat4, Quat, Vec2, Vec3, Vec4, Vec4Swizzles};
use m2_render::View;

use crate::binoculars::Binoculars;
use crate::camera::FlyCamera;
use crate::combat::{self, Combat, Effect, Field, Weapon};
use crate::config::CameraConfig;
use crate::destruct::{Blast, Crashed, Crasher, Destruction};
use crate::drops::{Drops, Soldier};
use crate::input::{Action, Input};
use crate::mission::{WATERLINE, template_models};
use crate::moving::MovingProps;
use crate::player::{ChaseRig, Intent, Player};
use crate::props::Props;
use crate::save::{self, PlayerSave, SaveGame, VehicleSave};
use crate::scene::{ParkedVehicle, Scene};
use crate::startup::Startup;
use crate::support::{self, Call, Impact, Support};
use crate::ui::HudInfo;

/// Where a vehicle of the layers stands.
struct Spot<'a> {
    entity: u32,
    name: &'a str,
    at: Vec3,
    yaw: f32,
}

/// How quickly her lens eases to its gun's share aiming and back, a second
/// (invented).
const LENS_EASE: f32 = 8.0;
/// How fast a flown helicopter's rotors turn, radians a second (about five
/// turns; invented, the rotor blur hides it).
const ROTOR_SPIN: f32 = 30.0;
/// How fast a turret turns after the camera, and its gun tilts to the
/// crosshair, radians a second, when its template doesn't say (invented).
const TURRET_RATE: f32 = 1.2;
const GUN_RATE: f32 = 0.6;
/// How near a building the town's sounds play, metres.
const TOWN_REACH: f32 = 150.0;

/// The scripts put vehicles they don't need yet far under the island:
/// those below this height aren't placed.
const HIDDEN_BELOW: f32 = -1000.0;

/// A vehicle held where it stands, far from her, is let go once she is
/// this much nearer than the island's full-detail ground reaches and
/// ground is found within this far under it, metres.
const HOLD_SLACK: f32 = 100.0;
const HOLD_GROUND: f32 = 30.0;

/// Boats going faster than this throw spray off their sides and churn a
/// wake, the most from the second speed (m/s), puffs every this many
/// seconds, each lasting this long, the wake longer. Invented, from the
/// playthroughs' speedboat: spray wider than the boat at full speed.
const SPRAY_SPEED: f32 = 4.0;
const SPRAY_FULL: f32 = 16.0;
const SPRAY_GAP: f32 = 0.05;
const SPRAY_LIFE: f32 = 0.7;
const WAKE_LIFE: f32 = 1.2;

/// A vehicle knocks loose what it hits that comes loose (traffic lights,
/// plants, barricades, poles: their `PHY2` has bodies that move) at this
/// speed or more (m/s), pushing with its engine counting as this one, and
/// when its momentum is at least this many times the mass to shift
/// (kg m/s per kg). Invented: a car fells a lamp post or a plant, a tank
/// a parked car.
const KNOCK_SPEED: f32 = 1.0;
const PUSH_SPEED: f32 = 1.5;
const KNOCK_RATIO: f32 = 4.0;
/// What it knocks loose flies off a little faster than the two go on
/// together, so it clears the vehicle, this much up (m/s), toppling the
/// way it was hit at this many radians a second per m/s.
const FLING: f32 = 1.1;
const FLING_UP: f32 = 1.0;
const TOPPLE: f32 = 0.3;

/// A bomb lands on what it first meets falling from this far above her
/// (metres).
const BOMB_DROP: f32 = 600.0;

/// A move by the scripts longer than this, metres, is a load with the
/// loading screen up (inferred: the footage shows it on the move to PMC
/// HQ at 19:54; the length that makes one is not known).
pub const FAR_TELEPORT: f32 = 300.0;
/// Seconds an arrival's load lasts at least, and at most (invented; the
/// footage's screen there is up about 4 s with its wipes).
pub const ARRIVAL_MIN: f32 = 1.0;
pub const ARRIVAL_CAP: f32 = 20.0;

/// Whether an arrival's load `t` seconds in is over: its place `in_place`
/// (nothing near still loading) after [`ARRIVAL_MIN`], or at
/// [`ARRIVAL_CAP`] whatever.
fn arrived(t: f32, in_place: bool) -> bool {
    t >= ARRIVAL_CAP || (t >= ARRIVAL_MIN && in_place)
}

#[derive(Debug)]
pub struct Game {
    pub scene: Scene,
    pub camera: FlyCamera,
    /// The menus' Brightness and shadows ([`Picture`]).
    pub picture: Picture,
    /// Playing as Jennifer rather than flying the camera.
    pub on_foot: bool,
    /// Seconds to the next look for parking spots.
    park_wait: f32,
    /// Looks for parking so far; each widens the search and allows
    /// steeper ground, until [`PARK_TRIES`] give up.
    park_tries: u32,
    /// Vehicles from a loaded save, placed as their models load.
    restore: Vec<VehicleSave>,
    pub combat: Combat,
    /// The island's placed objects worn down and brought down by blasts.
    pub destruction: Destruction,
    /// Each vehicle model's load bed, its `HP_Truckbed` node in its own
    /// space, by model (`None`: it has none), as looked up.
    beds: HashMap<u32, Option<Vec3>>,
    /// Each vehicle model with a load bed, its own collision (its `PHY2`,
    /// model space), which the goods on the bed lie on, by model.
    bed_shapes: HashMap<u32, BedShape>,
    /// The scripts' vehicles whose load bed's goods were logged (taken up,
    /// or what stood near and why not), by index.
    beds_told: HashSet<usize>,
    /// Layer entities of the island's vehicles asked for so far.
    fetched: HashSet<u32>,
    /// The island's traffic spawners and what they have out.
    pub traffic: crate::traffic::Traffic,
    /// A pursuit's gunships ([`crate::gunships`]).
    pub gunships: crate::gunships::Gunships,
    /// A pursuit's cars ([`crate::pursuers`]).
    pub pursuers: crate::pursuers::Pursuers,
    /// Vehicles the mission scripts send along a path (`crate::paths`).
    pub paths: crate::paths::PathMoves,
    /// Traffic on the road network (`crate::roads`).
    pub roads: crate::roads::RoadTraffic,
    /// With `M2_LIFE_CENSUS=<seconds>` (or `--traffic`): how often to log
    /// what lives near her ([`Game::life_census`]), the seconds run and
    /// the seconds since the last.
    pub census: Option<(f32, f32, f32)>,
    /// Seconds to the next look for the island's vehicles near the player.
    vehicle_wait: f32,
    /// The layers the game starts with, which the island's vehicles come
    /// from; `None` places them from every static layer (`--mission`,
    /// where the scripts' own layers rule).
    pub startup: Option<Startup>,
    /// Seconds since the scripts moved her far ([`teleported`]), while
    /// the place she came to still loads: the loading screen is up.
    ///
    /// [`teleported`]: Self::teleported
    pub arrival: Option<f32>,
    /// The start-up's state layers have their models up.
    props_up: bool,
    /// The driver's throttle last tick, -1 (reverse) to 1.
    pub throttle: f32,
    /// Seconds to the next puffs of spray off boats going fast.
    spray_wait: f32,
    /// The light of the atmosphere region the player is in.
    pub sky: AtmosphereMix,
    /// Lights blasts' scripts set, seconds before each comes in
    /// ([`m2_game::atmosphere_script`]).
    script_lights: Vec<(f32, ScriptAtmosphere)>,
    /// Props that move on their own.
    moving: MovingProps,
    /// Her support: the items she holds and the call under way; her
    /// health when it last stepped (hurt, a call is called off).
    pub support: Support,
    support_hp: f32,
    /// Money she has picked up since the scripts last heard: cash and
    /// fuel ([`crate::mission::MissionLink`] gives them to the scripts).
    pub earned: Vec<(u32, u32)>,
    /// The soldier script's count of deaths and rolls: what the dead
    /// leave ([`crate::drops`]), and how many have.
    drops: Drops,
    dropped: u32,
    /// The pickups she has stood at and left lying, each logged once
    /// with why.
    refused: std::collections::HashSet<(u32, u32)>,
    /// The pickups, crates and emplaced guns the state layers place.
    pub placed: crate::placed::Placed,
    /// The scripts have turned her controls off (a loading fade).
    pub input_locked: bool,
    /// A dialog box of the scripts is up (Fiona's contract menu): it
    /// has the controls (`MrxGuiDialogBox._HandleInputEvent` takes the
    /// pad's events), so she stands still and has no crosshair.
    pub dialog_up: bool,
    /// The scripts turned her aiming off (`Player.SetAimMode`, the HQ):
    /// no reticle until they turn it on again.
    pub aim_off: bool,
    /// The scripts' countdown ticked since autoplay last looked (a
    /// `ui_HUD_Timer_` cue: OilCon002's 15 minutes).
    pub countdown_tick: bool,
    /// Models taken only by their action hijack while crewed (an entrance
    /// in the data, [`crate::hijack`]): the use key doesn't get her in.
    pub hijack_only: HashSet<u32>,
    /// Vehicles (by guid) the scripts keep for themselves
    /// (`Vehicle.Usable(vehicle, false)`): the use key doesn't get her in.
    pub unusable: HashSet<u32>,
    /// The vehicle she last got into: the use key sticks to its entrances
    /// ([`entrance_pick`]).
    pub last_vehicle: Option<usize>,
    /// The island's ladders ([`crate::ladders`]).
    pub ladders: crate::ladders::Ladders,
    /// The ladder she is on.
    pub climb: Option<crate::ladders::Climbing>,
    /// The vehicle the use key seated her in and her seat's `SeatLink`
    /// name ([`crate::ride`]); while she rides any other, or none is
    /// given, she sits at the wheel.
    pub ride: Option<(usize, u32)>,
    /// Her getting in or out by the clips, under way ([`crate::boarding`]).
    pub boarding: Option<crate::boarding::Boarding>,
    /// The field of view (degrees, bottom to top) and the screen's width
    /// over its height the last frame was drawn with: what the scripts'
    /// camera sees (`Event.ObjectIsVisible`).
    pub lens: (f32, f32),
    /// The scripts seated her in the vehicle she is in (the start's boat), rather
    /// than she getting in herself: no remark on it ([`car_remark`]).
    pub scripts_seated: bool,
    /// The scripts' context action she stands at (`Pg.AddContextAction`:
    /// an HQ exit, a starter's talk), its object and prompt, as the
    /// mission last found it; the use key takes it before anything else.
    pub context: Option<(u32, String)>,
    /// The context action she took with the use key, for the mission to
    /// tell the scripts.
    pub context_taken: Option<u32>,
    /// The scripts' helicopter she rides in as a passenger, its own pilot
    /// flying it (PmcCon003's Lucky Lady): not one of hers to fly, so she is
    /// kept where the scripts have it ([`crate::mission`]).
    pub aboard: Option<u32>,
    /// The scripts' camera shot for a scene (the HQ's briefings), while
    /// it lasts: the frame is seen from it ([`Shot::view`]).
    pub shot: Option<Shot>,
    /// Seconds the HUD's save icon has left up after the game was saved
    /// ([`SAVE_ICON_SECONDS`]).
    pub save_icon: f32,
    /// Clips the scripts hold characters in by name (a faction HQ's
    /// briefing, [`crate::raw`]).
    pub raw: crate::raw::RawClips,
    /// The model the scripts last dressed the player in
    /// (`Player.SetOutfit`: the wardrobe), till it is put on her
    /// ([`Game::dress`]).
    pub outfit_wanted: Option<String>,
    /// Her grappling hook ([`crate::grapple`]).
    pub grapple: crate::grapple::Grappler,
    /// The grapple pulled her to a helicopter's entrance: its hijack is
    /// to start as the use key starts it ([`crate::hijack`]).
    pub grapple_handoff: bool,
    /// Seconds before the factions' attitudes are asked of the scripts again.
    pub attitude_wait: f32,
    /// Witnesses reporting her: guid, faction and seconds since they set
    /// off ([`crate::mission`]).
    pub reporters: Vec<(u32, &'static str, f32)>,
    /// Whether an army pursued her last frame.
    pub pursued: bool,
    /// The horn held this tick (driving), and its presses not yet taken
    /// ([`Game::take_honks`]).
    pub honking: bool,
    honks: u32,
    /// Her vehicle disguise ([`crate::disguise`]).
    pub disguise: crate::disguise::Disguise,
    /// Seconds the scripts asked her pad to shake (`Pg.Rumble`), for the
    /// pad to take.
    pub rumble: f64,
    /// Keyed actions the scripts asked for (`Human.DoAction`: the player's
    /// verify photo `VerifyCamera`), each with its seconds left, ended for the
    /// scripts' `HumanActionComplete` ([`crate::mission`]).
    pub actions: Vec<(u32, String, f32)>,
}

/// A camera shot the scripts asked for (`Camera.SetShot`): the engine's
/// named shot from one object to another.
#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    /// `OverTheShoulderLeft`, `OverTheShoulderRight` (the HQ's briefings).
    pub name: String,
    pub base: u32,
    pub target: u32,
    /// Where it sees from and looks at, as last placed
    /// ([`Shot::place`]); `None` until both objects are known.
    pub view: Option<(Vec3, Vec3)>,
    /// Whether the log has said it could not be placed.
    pub unplaced_logged: bool,
}

/// The over-the-shoulder shots, from the 360 code (the shot camera's
/// update, 0x824C7E78, keyed by the shot's name hash): the eye stands at a
/// hardpoint of the base's (one for each side), this far along the base's
/// own x axis (+ for `OverTheShoulderLeft`, - for `Right`) and its z axis
/// (behind it), and this far up; it looks across to half way between the
/// two (level), and down by [`SHOULDER_DROP`] for each metre of that.
const SHOULDER_ASIDE: f32 = 0.55;
const SHOULDER_ALONG: f32 = -0.53;
const SHOULDER_UP: f32 = 0.3;
/// The look's fall: its direction is (to the middle, level) with this as
/// its downward part before it is made unit length.
const SHOULDER_DROP: f32 = 0.4;
/// The base's hardpoints the eye stands at, by name hash (their names are
/// not in the code): `OverTheShoulderLeft`'s and `Right`'s.
const SHOULDER_HARDPOINT_LEFT: u32 = 0xa3a5_094a;
const SHOULDER_HARDPOINT_RIGHT: u32 = 0xba8c_a3a8;

/// Where a shot's actor stands: feet, facing (radians clockwise from +z,
/// as the player's and people's) and, for the base, its shot hardpoint
/// in the world if its model has it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShotActor {
    pub feet: Vec3,
    pub facing: f32,
    pub hardpoint: Option<Vec3>,
}

/// How long the HUD's save icon stays up once the game is saved, seconds:
/// the HUD's own icon (`HandleSaveIconShow`) takes `_knSaveIconTime`
/// (0.5 s, from the scripts) to come in and as long to go; a second
/// between is invented (the save itself is at once here).
pub const SAVE_ICON_SECONDS: f32 = 2.0;

impl Shot {
    /// Whether it is `OverTheShoulderRight` (any other shot is taken as
    /// the left one).
    fn right(&self) -> bool {
        self.name.ends_with("Right")
    }

    /// The base's hardpoint the shot's eye stands at.
    pub fn hardpoint(&self) -> u32 {
        if self.right() {
            SHOULDER_HARDPOINT_RIGHT
        } else {
            SHOULDER_HARDPOINT_LEFT
        }
    }

    /// Places the shot from where its base and target stand: the eye by
    /// the base's hardpoint (its feet when its model has none, as the
    /// engine falls back to the base's place), looking down across to half
    /// way between the two.
    pub fn place(&mut self, base: ShotActor, target: Vec3) {
        let (sin, cos) = base.facing.sin_cos();
        // The base's own axes: z the way it faces, x across.
        let z = Vec3::new(sin, 0.0, cos);
        let x = Vec3::new(cos, 0.0, -sin);
        let side = if self.right() {
            -SHOULDER_ASIDE
        } else {
            SHOULDER_ASIDE
        };
        let anchor = base.hardpoint.unwrap_or(base.feet);
        let eye = anchor + x * side + z * SHOULDER_ALONG + Vec3::Y * SHOULDER_UP;
        let middle = (base.feet + target) * 0.5;
        let level = (middle - eye).with_y(0.0);
        let look = Vec3::new(level.x, -SHOULDER_DROP, level.z).normalize_or(Vec3::Z);
        self.view = Some((eye, eye + look));
    }
}

/// A soldier's pickup is told from the others by its "crate": this, plus
/// how many were left before it.
const DROPPED: u32 = 0x7000_0000;

/// Gives her what her template hands her to throw (`explosives`, from
/// [`Scene::hero_explosives`]: free roam; the contract's scripts give her
/// their own), as many of each as it says, and of what it doesn't name as
/// many as a new game starts with.
fn own_explosives(combat: &mut Combat, explosives: Vec<(String, u32)>) {
    let fresh = Combat::default();
    combat.grenades = fresh.grenades;
    combat.charges = fresh.charges;
    combat.mines = fresh.mines;
    for (name, n) in explosives {
        match name.to_ascii_lowercase().as_str() {
            "grenade" => combat.grenades = n,
            "c4" => combat.charges = n,
            "proximity mine" => combat.mines = n,
            _ => {}
        }
    }
}

impl Game {
    pub fn new(scene: Scene, camera: FlyCamera) -> Self {
        let on_foot = scene.player.is_some();
        let mut combat = Combat::default();
        if let Some(hp) = scene.hero_health() {
            combat.max_hp = hp;
            combat.hp = hp;
        }
        combat.hero_damage = scene.hero_damage();
        if let Some(rifle) = scene.hero_rifle() {
            combat.take_own_rifle(rifle);
        }
        if let Some(arms) = scene.arms() {
            combat.arm(arms, scene.damage_table());
        }
        own_explosives(&mut combat, scene.hero_explosives());
        eprintln!(
            "hero: carries {} grenades, {} C4, {} mines; launcher {} ({:?})",
            combat.grenades,
            combat.charges,
            combat.mines,
            combat.launcher.name,
            combat.launcher_rounds
        );
        let r = &combat.rifle;
        eprintln!(
            "hero: {} rifle {} ({}), {:.0} a minute, clip {}, spare {}, round {}; blasts on her {:?}",
            scene.hero,
            r.name,
            r.model,
            60.0 / r.gap,
            r.clip,
            r.spare,
            r.damage,
            combat.hero_damage
        );
        Self {
            picture: Picture::default(),
            scene,
            camera,
            on_foot,
            park_wait: 0.0,
            park_tries: 0,
            restore: Vec::new(),
            combat,
            destruction: Destruction::default(),
            beds: HashMap::new(),
            bed_shapes: HashMap::new(),
            beds_told: HashSet::new(),
            fetched: HashSet::new(),
            traffic: crate::traffic::Traffic::default(),
            gunships: crate::gunships::Gunships::default(),
            pursuers: crate::pursuers::Pursuers::default(),
            paths: crate::paths::PathMoves::default(),
            roads: crate::roads::RoadTraffic::default(),
            census: std::env::var("M2_LIFE_CENSUS")
                .ok()
                .and_then(|s| s.parse().ok())
                .map(|every: f32| (every, 0.0, every)),
            vehicle_wait: 0.0,
            startup: None,
            arrival: None,
            props_up: false,
            throttle: 0.0,
            spray_wait: 0.0,
            sky: AtmosphereMix::default(),
            script_lights: Vec::new(),
            moving: MovingProps::default(),
            support: Support::default(),
            earned: Vec::new(),
            drops: Drops::default(),
            dropped: 0,
            refused: std::collections::HashSet::new(),
            placed: crate::placed::Placed::default(),
            input_locked: false,
            dialog_up: false,
            aim_off: false,
            countdown_tick: false,
            hijack_only: HashSet::new(),
            unusable: HashSet::new(),
            last_vehicle: None,
            ladders: Default::default(),
            climb: None,
            ride: None,
            boarding: None,
            support_hp: 0.0,
            lens: (60.0, 16.0 / 9.0),
            context: None,
            context_taken: None,
            aboard: None,
            shot: None,
            save_icon: 0.0,
            raw: crate::raw::RawClips::default(),
            outfit_wanted: None,
            grapple: crate::grapple::Grappler::default(),
            grapple_handoff: false,
            attitude_wait: 0.0,
            reporters: Vec::new(),
            pursued: false,
            honking: false,
            honks: 0,
            disguise: crate::disguise::Disguise::default(),
            rumble: 0.0,
            actions: Vec::new(),
            scripts_seated: false,
        }
    }

    /// Hands her weapon `name` by its template's name, full: a gun taken
    /// up in its place (what she held there dropped), or an explosive
    /// chosen to throw. False if the templates have no hand weapon so
    /// named.
    pub fn give(&mut self, name: &str) -> bool {
        let Some(arms) = self.scene.arms() else {
            return false;
        };
        if let Some(spec) = arms.named(name).filter(|w| w.class.is_some()) {
            let gun = self.combat.stock(spec);
            eprintln!(
                "gun: {} ({:?}), {:.0} a minute, clip {}, spare {}, {} a shot, round {:?}",
                gun.name,
                gun.slot(),
                60.0 / gun.gap,
                gun.clip,
                gun.spare,
                gun.pellets,
                gun.ammo
            );
            self.combat.equip_gun(gun, 0);
            return true;
        }
        let slot = self.combat.throwables.iter().position(|t| {
            t.as_ref()
                .is_some_and(|t| t.name.eq_ignore_ascii_case(name))
        });
        let Some(i) = slot else { return false };
        let max = self.combat.throwables[i].as_ref().map_or(0, |t| t.max);
        match i {
            1 => self.combat.charges = max,
            2 => self.combat.mines = max,
            _ => self.combat.grenades = max,
        }
        self.combat.explosive = i;
        true
    }

    /// Plays as `hero` from now on: their body, clips and health (whole),
    /// and in free roam their own rifle and what they carry to throw.
    pub fn set_hero(
        &mut self,
        backend: &mut dyn m2_render::RenderBackend,
        hero: crate::hero::Hero,
    ) {
        self.scene.load_hero(backend, hero);
        if let Some(hp) = self.scene.hero_health() {
            self.combat.max_hp = hp;
            self.combat.hp = hp;
        }
        // Their own rifle and explosives, as a game started as them has
        // ([`Game::new`]): the rifle unless she has taken up another, the
        // explosives unless the contract's scripts handed her theirs.
        if self.combat.own_rifle
            && let Some(rifle) = self.scene.hero_rifle()
        {
            self.combat.take_own_rifle(rifle);
        }
        if !self.combat.scripted_loadout {
            own_explosives(&mut self.combat, self.scene.hero_explosives());
            eprintln!(
                "hero: {hero} carries {} grenades, {} C4, {} mines",
                self.combat.grenades, self.combat.charges, self.combat.mines
            );
        }
    }

    /// Among the town's buildings: a building placement (a town's batch
    /// or a lone building) within [`TOWN_REACH`] of the listener.
    pub fn near_town(&self) -> bool {
        let (ear, _) = self.listener();
        self.scene.world.as_ref().is_some_and(|w| {
            w.index
                .objects
                .iter()
                .any(|o| !o.named && o.position.distance(ear) < TOWN_REACH)
        })
    }

    /// Island vehicles asked for that have not been placed yet.
    pub fn island_vehicles_loading(&self) -> bool {
        if self.startup.as_ref().is_some_and(Startup::loading) {
            return true;
        }
        let placed = self
            .scene
            .vehicles
            .iter()
            .filter(|v| v.spot.is_some() && v.id.is_some())
            .count();
        placed < self.fetched.len()
    }

    /// Puts up the models of the state layers the game starts with, once
    /// the start-up has said which.
    fn startup_props(&mut self) {
        if self.props_up {
            return;
        }
        let Some(s) = &mut self.startup else { return };
        s.poll();
        let set = match s {
            Startup::Ready(set) => set,
            // Not knowing which, every state layer's people stand.
            Startup::Failed => {
                self.props_up = true;
                self.scene.people.state_layers = None;
                return;
            }
            Startup::Loading(_) => return,
        };
        self.props_up = true;
        self.scene.people.state_layers = Some(set.hashes.clone());
        for e in &set.entities {
            self.scene.props.note_layer([e.entity], &e.layer);
        }
        let shown = self.scene.show_props(set.entities.iter().map(|e| e.entity));
        eprintln!("start-up state layers: {shown} models put up");
    }

    /// Asks for the island's own vehicles near the player, once the
    /// vehicle catalogue is known, and places the ones that have loaded
    /// where the layers park them.
    fn fetch_world_vehicles(&mut self, dt: f32) {
        let scene = &mut self.scene;
        let (Some(world), Some(player)) = (&mut scene.world, &scene.player) else {
            return;
        };
        for v in scene.vehicles.iter_mut().filter(|v| v.id.is_none()) {
            // A gunship is put in the air where it comes in
            // ([`crate::gunships::Gunships::fly`]), a pursuit's car on the
            // road ([`crate::pursuers::Pursuers::drive`]), or is gone.
            if v.spot
                .is_some_and(|(e, _, _)| e >= crate::pursuers::FIRST_ENTITY)
            {
                continue;
            }
            if let Some((entity, at, yaw)) = v.spot
                && place(&mut world.physics, v, at, yaw)
            {
                eprintln!(
                    "island vehicle {entity:#x}: {} parked at ({:.0}, {:.0}, {:.0})",
                    v.spec.label, at.x, at.y, at.z
                );
            }
        }
        self.vehicle_wait -= dt;
        if self.vehicle_wait > 0.0 || !player.landed {
            return;
        }
        self.vehicle_wait = VEHICLE_LOOK;
        let Some(catalogue) = &scene.catalogue else {
            return;
        };
        if let Some(s) = &mut self.startup {
            s.poll();
        }
        // Where they may stand: the static layers the game starts with
        // (all of them if that set isn't known) and its state layers; with
        // `--mission`, the static layers the scripts have loaded so far
        // (the people's set; their state layers' vehicles are theirs).
        let (static_layers, state_layers) = match &self.startup {
            Some(Startup::Loading(_)) => return,
            Some(Startup::Ready(set)) => (Some(&set.hashes), set.entities.as_slice()),
            _ => (scene.people.state_layers.as_ref(), &[][..]),
        };

        let near = player.character.position;
        let mut wanted: Vec<(f32, Spot<'_>)> = world
            .index
            .templates
            .iter()
            .filter(|t| static_layers.is_none_or(|h| h.contains(&t.layer)))
            .map(|t| {
                let f = t.rotation * Vec3::Z;
                Spot {
                    entity: t.entity,
                    name: &t.name,
                    at: t.position,
                    yaw: f.x.atan2(f.z),
                }
            })
            .chain(state_layers.iter().map(|e| Spot {
                entity: e.entity,
                name: &e.name,
                at: e.position,
                yaw: e.yaw,
            }))
            .filter(|t| !self.fetched.contains(&t.entity) && t.at.y > HIDDEN_BELOW)
            .filter_map(|t| {
                let d = t.at.distance(near);
                (d <= VEHICLE_REACH).then_some((d, t))
            })
            .collect();
        wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Traffic has its own limit.
        let traffic = &self.traffic;
        let roads = &self.roads;
        let mut placed = scene
            .vehicles
            .iter()
            .filter(|v| v.spot.is_some_and(|(e, _, _)| !traffic.owns(e)) && !roads.owns(v))
            .count();
        let mut loads = Vec::new();
        for (_, t) in wanted {
            if placed >= WORLD_VEHICLES {
                break;
            }
            let models = template_models(t.name, &world.index.template_db, &catalogue.vehicles);
            if models.is_empty() {
                continue;
            }
            self.fetched.insert(t.entity);
            placed += 1;
            eprintln!(
                "island vehicle {:#x}: {} as {}",
                t.entity, t.name, models[0].label
            );
            // A vehicle's ruin stands wrecked.
            if crate::mission::is_ruin(&world.index.template_db, t.name) {
                scene.ruins.insert(t.entity);
            }
            let crew = world.index.template_db.crew(t.name);
            loads.push((t.entity, t.at, t.yaw, crew, models));
        }
        for (entity, at, yaw, crew, models) in loads {
            scene.load_world_vehicle(entity, at, yaw, crew, models);
        }
    }

    /// One fixed tick from the keyboard and mouse.
    pub fn tick(&mut self, input: &mut Input, config: &CameraConfig, mouse_look: bool, dt: f32) {
        if input.take_pressed(Action::ToggleFly) {
            self.toggle_fly();
        }
        if self.on_foot {
            // A context action she stands at takes the use key first.
            if let Some((g, _)) = &self.context
                && !self.input_locked
                && input.take_pressed(Action::Use)
            {
                eprintln!("context: she takes the action at {g:#x}");
                self.context_taken = Some(*g);
            }
            let mut intent = Intent::from_input(input, config, mouse_look);
            // Aim assist on a pad or the touch controls (not the mouse):
            // aiming or firing on foot draws the view to the hostile
            // nearest the crosshair ([`crate::aim`]).
            if input.pad()
                && (intent.trigger.aim || intent.trigger.fire)
                && intent.look.length() < crate::aim::LET_GO
                && let Some(p) = self.scene.player.as_ref().filter(|p| p.driving.is_none())
            {
                let people = &self.scene.people;
                let targets = people
                    .npcs
                    .iter()
                    .filter(|n| n.alive && n.faction.is_some_and(|f| people.fights_for(f)))
                    .map(|n| {
                        n.character.position + Vec3::Y * n.character.height * crate::aim::CHEST
                    });
                // The crosshair's pitch (above the camera's: shots fly
                // along it, [`crate::player::Player::aim_forward`]).
                let aim_pitch = p.aim_forward().y.clamp(-1.0, 1.0).asin();
                if let Some(turn) = crate::aim::assist(p.eye(), p.yaw, aim_pitch, targets, dt) {
                    intent.look += turn;
                }
            }
            let toggle = input.take_pressed(Action::Binoculars);
            let zoom = i32::from(input.take_pressed(Action::ZoomIn))
                - i32::from(input.take_pressed(Action::ZoomOut));
            let switch = input.take_pressed(Action::SwitchWeapon);
            if self.binoculars(toggle, zoom, switch, &mut intent) {
                self.step(intent, dt);
                return;
            }
            self.aim_lens(&mut intent, zoom, dt);
            let driving = self
                .scene
                .player
                .as_ref()
                .is_some_and(|p| p.driving.is_some());
            match switch_weapon(switch, driving, self.combat.weapon, self.combat.vehicle_gun) {
                Some(Switched::VehicleGun(gun)) => {
                    self.combat.vehicle_gun = gun;
                    eprintln!(
                        "vehicle: {} chosen",
                        if gun == 1 { "machine gun" } else { "main gun" }
                    );
                }
                Some(Switched::Hands(w)) if intent.trigger.select.is_none() => {
                    intent.trigger.select = Some(w);
                }
                _ => {}
            }
            self.step(intent, dt);
        } else {
            if let Some(p) = self.scene.player.as_mut() {
                p.binoculars = None;
            }
            self.camera.update(input, config, mouse_look, dt);
            self.step(Intent::default(), dt);
        }
    }

    /// Her gun's lens ([`m2_engine::weapons::CameraFeel`]): aiming on foot,
    /// the view narrows to its `CrouchFov` share and her look slows to its
    /// `CrouchYawSpeed` and `CrouchPitchSpeed`; else it widens back to its
    /// `FOV` (1 on every gun). The lens eases there at [`LENS_EASE`] a
    /// second (invented). A gun with a scope (`WeaponScope`) looks through
    /// it instead, aiming: at its least magnification, the wheel stepping
    /// it a level a notch up to its most (the steps invented), with the
    /// game's scope screen up (the binocular screen with `bSniper`).
    pub fn aim_lens(&mut self, intent: &mut Intent, zoom: i32, dt: f32) {
        let feel = self.combat.camera_feel();
        let scope = self.combat.scope();
        let Some(player) = self.scene.player.as_mut() else {
            return;
        };
        let aiming = player.aiming && player.driving.is_none();
        player.scope = scope
            .filter(|_| aiming)
            .map(|(least, most)| scope_step(player.scope, zoom, least, most));
        player.lens = lens_step(player.lens, feel, aiming, dt);
        if let Some(f) = feel.filter(|_| aiming) {
            intent.look.x *= f.aim_look[0];
            intent.look.y *= f.aim_look[1];
        }
    }

    /// Her binoculars ([`crate::binoculars`]): `toggle` puts them up or
    /// down, `zoom` steps their zoom (the wheel) and `switch` steps it up
    /// and round (Tab, the pad's right bumper). They come up only on
    /// foot, standing, with no support call or PDA in hand, and go down
    /// when that changes or the support menu or PDA is asked for. While
    /// they are up she stands and only looks, slower by the zoom: `intent`
    /// is cut down to that, and true is returned.
    fn binoculars(&mut self, toggle: bool, zoom: i32, switch: bool, intent: &mut Intent) -> bool {
        let busy = self.support.busy() || self.support.pda;
        let Some(player) = self.scene.player.as_mut() else {
            return false;
        };
        let free = player.driving.is_none() && !player.down && player.landed && !busy;
        let wanted = intent.trigger.support || intent.trigger.pda;
        player.binoculars = match player.binoculars {
            Some(_) if toggle || !free || wanted => None,
            None if toggle && free => Some(Binoculars::default()),
            b => b,
        };
        let Some(b) = &mut player.binoculars else {
            return false;
        };
        b.step(zoom);
        if switch && zoom == 0 {
            b.cycle();
        }
        let look = intent.look * b.look_scale();
        *intent = Intent {
            look,
            ..Intent::default()
        };
        true
    }

    /// The scripts moved her from `from` to `to`: past [`FAR_TELEPORT`]
    /// that is a load (the second contract's end takes her to PMC HQ), and
    /// the loading screen stays up while the place loads.
    pub fn teleported(&mut self, from: Vec3, to: Vec3) {
        if from.distance(to) > FAR_TELEPORT {
            self.arrival = Some(0.0);
        }
    }

    /// The arrival's load, `dt` seconds on: over once her surroundings are
    /// in (after [`ARRIVAL_MIN`]), or after [`ARRIVAL_CAP`] whatever.
    fn arrive(&mut self, dt: f32) {
        let Some(t) = &mut self.arrival else { return };
        *t += dt;
        let counts = self.scene.props.counts();
        let in_place = counts.loading + counts.waiting_near == 0
            && self
                .scene
                .world
                .as_ref()
                .is_none_or(|w| w.stats().pending == 0);
        if arrived(*t, in_place) {
            eprintln!("arrival: loaded after {t:.1} s");
            self.arrival = None;
        }
    }

    /// What lives near the player, for measuring against the footage: the
    /// vehicles moving within 150 m (and of them, the road traffic's), and
    /// the people standing within 100 m (fighting or not).
    pub fn life_census(&self) -> String {
        let focus = self.focus();
        let near = |p: m2_math::glam::Vec3, d: f32| {
            m2_math::glam::Vec2::new(p.x - focus.x, p.z - focus.z).length() < d
        };
        let (mut moving, mut road) = (0, 0);
        if let Some(world) = &self.scene.world {
            for v in &self.scene.vehicles {
                let Some(id) = v.id.filter(|_| !v.wrecked) else {
                    continue;
                };
                let Some(m) = world.physics.vehicle_transform(id) else {
                    continue;
                };
                if near(m.w_axis.truncate(), 150.0) && world.physics.vehicle_speed(id) > 2.0 {
                    moving += 1;
                    road += usize::from(self.roads.owns(v));
                }
            }
        }
        let people = &self.scene.people.npcs;
        let about = people
            .iter()
            .filter(|n| n.alive && near(n.character.position, 100.0));
        let soldiers = about.clone().filter(|n| n.soldier).count();
        let others = about.count() - soldiers;
        let zone = self.scene.world.as_ref().and_then(|w| {
            let people = &self.scene.people;
            let zones = w
                .index
                .zones
                .iter()
                .filter(|z| people.layer_in(Some(z.layer)));
            m2_engine::population::zone_at(zones, focus.x, focus.z)
        });
        let zone = zone.map_or("none".to_owned(), |z| {
            format!("{} ({} vehicles, {} people)", z.name, z.vehicles, z.people)
        });
        format!(
            "{moving} vehicles moving within 150 m ({road} road traffic), {others} people within 100 m not fighting, {soldiers} soldiers; zone {zone}"
        )
    }

    /// One fixed tick with `intent` for the player (ignored while flying).
    pub fn step(&mut self, intent: Intent, dt: f32) {
        self.save_icon = (self.save_icon - dt).max(0.0);
        if let Some((every, run, since)) = &mut self.census {
            *run += dt;
            *since += dt;
            if *since >= *every {
                *since = 0.0;
                let run = *run;
                eprintln!("life at {run:.0} s: {}", self.life_census());
            }
        }
        let mut lap = std::time::Instant::now();
        let mut split = |name: &'static str| {
            crate::spans::add(name, lap.elapsed());
            lap = std::time::Instant::now();
        };
        self.update_sky(dt);
        let focus = self.focus();
        self.moving.update(&mut self.scene, focus, dt);
        self.startup_props();
        split("game: moving");
        self.fetch_world_vehicles(dt);
        self.arrive(dt);
        let intent = if self.input_locked || self.dialog_up {
            Intent::default()
        } else {
            intent
        };
        // Getting in or out by the clips she only looks about.
        let intent = if self.boarding.is_some() {
            Intent {
                look: intent.look,
                ..Intent::default()
            }
        } else {
            intent
        };
        if self.on_foot
            && let Some(p) = self.scene.player.as_ref().filter(|p| p.landed)
        {
            let focus = p.character.position;
            let people = &self.scene.people;
            self.traffic
                .spawn(&self.scene, focus, |l| people.layer_in(l), dt);
        }
        // The road traffic comes on foot and at the wheel alike, as the
        // traffic zone there says, as do the passers-by.
        if self.scene.player.as_ref().is_some_and(|p| p.landed) {
            let focus = self.focus();
            let layers = self.scene.people.state_layers.clone();
            let layer_in = |l: u32| layers.as_ref().is_none_or(|set| set.contains(&l));
            if let Some(world) = &self.scene.world {
                let zones = world.index.zones.iter().filter(|z| layer_in(z.layer));
                self.scene.people.walker_cap =
                    m2_engine::population::zone_at(zones, focus.x, focus.z)
                        .map_or(0, |z| z.people as usize);
            }
            self.roads.spawn(&mut self.scene, focus, layer_in, dt);
        }
        let intent = self.call_support(intent, dt);
        split("game: arrival");
        // The use key at what lies within reach: taken with the fight's.
        let mut taking = false;
        let scene = &mut self.scene;
        // The models the layers put up near her, still loading: with
        // nothing yet under where the scripts put her, she waits for them.
        // The PMC's interior, 100 m over the mountain, loads after the
        // ground under it (playtest 5: put there after the villa, she
        // landed on the mountain below and saw the hall overhead).
        let props = scene.props.counts();
        let models_coming = props.loading + props.waiting_near > 0;
        if let Some(world) = &mut scene.world {
            let streaming = world.stats().pending > 0;
            let physics = &mut world.physics;
            let player = scene.player.as_mut().filter(|_| self.on_foot);
            if let Some(player) = player {
                player.streaming = streaming;
                player.models_coming = models_coming;
                self.park_wait -= dt;
                if !self.restore.is_empty() {
                    restore(physics, &mut scene.vehicles, &mut self.restore);
                }
                let waiting = scene.vehicles.iter().any(parkable);
                if player.landed && waiting && self.park_wait <= 0.0 && self.park_tries < PARK_TRIES
                {
                    // Each look can cast thousands of rays: not every tick.
                    self.park_wait = 0.5;
                    self.park_tries += 1;
                    park(physics, &mut scene.vehicles, player, self.park_tries);
                }
                // A boarding nothing plays ends as the use key used to:
                // she is in, or out at the door. One for a vehicle she is
                // not in any more is dropped.
                if let Some(b) = &mut self.boarding
                    && (player.driving != Some(b.vehicle) || b.age(dt))
                {
                    let b = self.boarding.take().expect("a boarding");
                    if b.exiting && player.driving == Some(b.vehicle) {
                        eprintln!("boarding: no clip played: she gets out at once");
                        get_out(physics, &scene.vehicles[b.vehicle], b.seat, player);
                        self.ride = None;
                    }
                }
                // Out of the vehicle the use key seated her in, or in
                // another one, her seat is forgotten.
                if self.ride.is_some_and(|(i, _)| player.driving != Some(i)) {
                    self.ride = None;
                }
                // The seat key: over her seat's link to a free one.
                if intent.trigger.seat
                    && let Some(i) = player.driving
                    && let Some(v) = scene.vehicles.get(i)
                    && v.spec.seating.is_some()
                {
                    let from = self.ride.map(|(_, s)| s);
                    match crate::ride::transfer(crate::ride::Ride::of(v), from) {
                        Some(to) => {
                            // Over by the clip, from the seat she leaves.
                            if v.spec.seating.is_some() {
                                self.boarding =
                                    Some(crate::boarding::Boarding::transfer(i, from, to));
                            }
                            self.ride = Some((i, to));
                            self.combat.seat = crate::ride::armed_index(
                                crate::ride::Ride::of(v),
                                Some(to),
                                &combat::armed_seats(v),
                            );
                            eprintln!(
                                "seat: she moves over to {to:08x} ({:?})",
                                crate::ride::Ride::of(v).role(Some(to))
                            );
                        }
                        None => eprintln!("seat: no free seat linked to hers"),
                    }
                }
                // On a ladder the climb takes the stick, and the use and
                // jump keys let go.
                let climbing = self.climb.is_some();
                if let Some(c) = self.climb.as_mut() {
                    let let_go = intent.use_pressed || intent.jump;
                    let floor = crate::ladders::floor_under(
                        physics,
                        c.climb.feet(&c.moves),
                        c.climb.floor_probe(),
                    );
                    match c.tick(intent.movement.y, let_go, floor, dt) {
                        Some(off) => {
                            let (m2_engine::ladder::Off::Stepped(at)
                            | m2_engine::ladder::Off::LetGo(at)) = off;
                            eprintln!("ladder: she gets off at rung {} ({off:?})", c.climb.rung);
                            // Where the clip leaves her, or where she let
                            // go: her body stands or falls from there.
                            player.ladder = None;
                            player.character.position = at;
                            player.character.velocity = Vec3::ZERO;
                            self.climb = None;
                        }
                        None => player.ladder = Some(c.pose()),
                    }
                }
                if intent.use_pressed && !climbing {
                    // The emplaced gun she would man, if nothing nearer
                    // takes the key.
                    let manning = (player.driving.is_none()
                        && self.combat.manned.is_none()
                        && scene.flights.within(player.character.position).is_none())
                    .then(|| {
                        self.placed.emplacement_near(
                            &scene.props,
                            &world.index.template_db,
                            player.character.position,
                        )
                    })
                    .flatten()
                    .map(|mut m| {
                        m.barrel = crate::placed::barrel(scene.source.as_mut(), &scene.props, &m);
                        m
                    });
                    // Its seat data: got on through its entrance as a
                    // vehicle is (gathered and picked with theirs, and the
                    // ladders'), manned from its seat's hardpoint, turned
                    // and tilted within its seat's `Turret`.
                    let points = manning.as_ref().and_then(|m| {
                        crate::placed::seat_points(
                            scene.source.as_mut(),
                            &scene.props,
                            &world.index.template_db,
                            m,
                        )
                    });
                    let her = player.character.position;
                    let manning = match (manning, points) {
                        (Some(mut m), Some(p)) => m2_engine::seats::in_search(
                            her,
                            player.eye(),
                            player.camera_forward(),
                            p.entrance,
                            p.use_distance,
                            m2_engine::seats::SEARCH_PAD,
                        )
                        .then(|| {
                            if let Some(t) = p.turret {
                                m.yaw = t.yaw;
                                m.pitch = t.pitch;
                            }
                            m.seat = Some(p.seat);
                            m
                        }),
                        (m, _) => m,
                    };
                    let gun =
                        points
                            .filter(|_| manning.is_some())
                            .map(|p| m2_engine::seats::Candidate {
                                vehicle: EMPLACED,
                                distance_sq: her.distance_squared(p.entrance),
                                use_distance: p.use_distance,
                                upright: true,
                                offered: true,
                            });
                    let ends = if player.driving.is_none() {
                        self.ladders.gathered(
                            &world.index,
                            &world.destroyed,
                            scene.source.as_mut(),
                            her,
                            player.eye(),
                            player.camera_forward(),
                        )
                    } else {
                        Vec::new()
                    };
                    let picked = entrance_pick(
                        physics,
                        &scene.vehicles,
                        (&self.hijack_only, &self.unusable),
                        player,
                        self.last_vehicle,
                        &ends,
                        gun,
                    );
                    // Without seat data the gun comes before any vehicle
                    // (the alpha's former rule); with it, if it is the
                    // nearest entrance.
                    let man_it = manning.is_some()
                        && (gun.is_none() || picked.is_some_and(|(i, _)| i == EMPLACED));
                    let manning = manning.filter(|_| man_it);
                    let picked = picked.filter(|&(i, _)| i != EMPLACED);
                    match player.driving {
                        // Out over deep water she swims (`Player::update_swim`).
                        Some(i) => {
                            let seat = self.ride.filter(|r| r.0 == i).map(|r| r.1);
                            match animated_exit(physics, &scene.vehicles[i], seat) {
                                // By the clips: out when they end.
                                Some(entrance) => {
                                    eprintln!(
                                        "boarding: she gets out of the {} by the clips \
                                         (seat {seat:08x?}, entrance {entrance:08x?})",
                                        scene.vehicles[i].spec.label
                                    );
                                    self.boarding =
                                        Some(crate::boarding::Boarding::exit(i, seat, entrance));
                                }
                                None => {
                                    get_out(physics, &scene.vehicles[i], seat, player);
                                    self.ride = None;
                                }
                            }
                        }
                        // Off the emplaced gun she mans.
                        None if self.combat.manned.is_some() => {
                            if let Some(m) = self.combat.manned.take() {
                                eprintln!(
                                    "placed: she leaves the emplaced gun {:#x} with {} rounds",
                                    m.entity, m.rounds
                                );
                            }
                        }
                        // What lies within reach before any vehicle.
                        None if scene.flights.within(player.character.position).is_some() => {
                            taking = true;
                        }
                        // An emplaced gun.
                        None if manning.is_some() => {
                            let Some(m) = manning else { unreachable!() };
                            eprintln!(
                                "placed: she mans {} ({:#x}) at {:.1}: {} ({} rounds, {:.0} a \
                                 minute), turning {:.0}..{:.0} and tilting {:.0}..{:.0} degrees",
                                world
                                    .index
                                    .template_db
                                    .names
                                    .get(&m.template)
                                    .map_or("?", String::as_str),
                                m.entity,
                                m.pivot,
                                m.weapon.name,
                                m.rounds,
                                m.weapon.rate_of_fire,
                                m.yaw.0.to_degrees(),
                                m.yaw.1.to_degrees(),
                                m.pitch.0.to_degrees(),
                                m.pitch.1.to_degrees()
                            );
                            // At its seat (else behind it), facing the way
                            // it does.
                            player.facing = m.facing;
                            let feet = player.character.position.y;
                            player.character.position = match m.seat {
                                Some(s) => s.with_y(feet),
                                None => crate::placed::stand_behind(m.pivot, m.facing, feet),
                            };
                            self.combat.manned = Some(m);
                        }
                        None => {
                            // A ladder's end, nearer than any vehicle's
                            // entrance.
                            let reach = picked
                                .and_then(|(i, _)| i.checked_sub(LADDER))
                                .and_then(|j| ends.get(j).copied());
                            let picked = picked.filter(|&(i, _)| i < LADDER);
                            if let Some(r) = reach {
                                let c = crate::ladders::Climbing::start(
                                    r,
                                    crate::ladders::moves(player),
                                );
                                eprintln!(
                                    "ladder: she gets on {} at the {} of object {} ({:.1} m, \
                                     {} rungs, from rung {})",
                                    world
                                        .index
                                        .template_db
                                        .names
                                        .get(&world.index.objects[r.object].template)
                                        .map_or("?", String::as_str),
                                    if r.top { "top" } else { "bottom" },
                                    r.object,
                                    r.ladder.length(),
                                    r.ladder.rungs(&c.moves),
                                    c.climb.rung
                                );
                                player.off_foot();
                                player.ladder = Some(c.pose());
                                self.climb = Some(c);
                            }
                            let db = &world.index.template_db;
                            // The seat the entrance gives her.
                            let boarding = picked.and_then(|(i, entrance)| {
                                let v = &scene.vehicles[i];
                                let labels = db.vehicle_labels_for(&v.spec.label, v.spec.model);
                                let b = crate::ride::board(
                                    crate::ride::Ride::of(v),
                                    entrance,
                                    crate::ride::car_or_truck(&labels),
                                    &scene.people,
                                );
                                log_entry(physics, v, player, entrance, b.as_ref());
                                Some((i, b?, entrance))
                            });
                            // Swimming 3 m or more off a dive entrance, she
                            // dives for it (0x8229F890).
                            let dive = boarding.as_ref().and_then(|(i, _, entrance)| {
                                let v = &scene.vehicles[*i];
                                let e = (*entrance)?;
                                let local = v.spec.entrance_at(e)?;
                                let at = physics.vehicle_transform(v.id?)?;
                                let p = v.spec.seating.as_deref()?.entrance(e)?.params;
                                let her = player.character.position;
                                let flat = (at.transform_point3(local) - her).with_y(0.0).length();
                                m2_engine::seats::dives(
                                    &p,
                                    player.swimming(),
                                    afloat(physics, v, at),
                                    flat,
                                )
                                .then(|| {
                                    eprintln!(
                                        "enter: she dives for the {} from {flat:.2} m",
                                        v.spec.label
                                    );
                                    her
                                })
                            });
                            player.driving = boarding.as_ref().map(|(i, _, _)| *i);
                            if let Some((i, b, entrance)) = &boarding {
                                self.last_vehicle = Some(*i);
                                // By the clips, through the entrance she took, for
                                // the seat it gave her (before any slide over).
                                if scene.vehicles[*i].spec.seating.is_some() {
                                    let mut boarding = crate::boarding::Boarding::enter(
                                        *i,
                                        b.slid_from.or(b.seat),
                                        *entrance,
                                    );
                                    boarding.transfer_to = b.seat.filter(|_| b.slid_from.is_some());
                                    boarding.dive =
                                        dive.map(|from| (from, m2_engine::seats::DIVE_TIME));
                                    self.boarding = Some(boarding);
                                }
                                self.ride = b.seat.map(|s| (*i, s));
                                let v = &scene.vehicles[*i];
                                self.combat.seat = crate::ride::armed_index(
                                    crate::ride::Ride::of(v),
                                    b.seat,
                                    &combat::armed_seats(v),
                                );
                            }
                            // Seated: she no longer waits to land (people
                            // wait on that, and she may have hopped out and
                            // back in before the ground streamed).
                            if let Some((i, b, _)) = boarding {
                                player.landed = true;
                                let v = &mut scene.vehicles[i];
                                let out = crate::ride::empty_seats(&mut v.crew, &b.out);
                                if !out.is_empty() {
                                    eprintln!(
                                        "hijack: she takes the {} from {} rider(s)",
                                        v.spec.label,
                                        out.len()
                                    );
                                }
                                let out = out
                                    .into_iter()
                                    .map(|who| {
                                        let model = db.get(&who).and_then(|t| t.model);
                                        (who, model)
                                    })
                                    .collect();
                                put_out(
                                    &mut scene.people,
                                    &scene.riders,
                                    &scene.vehicles[i],
                                    physics,
                                    i,
                                    out,
                                    player.character.position,
                                );
                            }
                        }
                    }
                }
                let id = player
                    .driving
                    .and_then(|i| scene.vehicles.get(i))
                    .and_then(|v| v.id.map(|id| (id, v)));
                self.throttle = 0.0;
                let was_honking = std::mem::take(&mut self.honking);
                // In a gunner's or passenger's seat (any but the driver's
                // armed seat 0) nobody drives: it stops.
                let gunner = self.combat.seat > 0;
                if let Some((id, v)) = id
                    && !v.wrecked
                    && gunner
                {
                    physics.drive(
                        id,
                        VehicleControls {
                            // A helicopter holds its height.
                            parked: !v.spec.is_helicopter(),
                            ..VehicleControls::default()
                        },
                    );
                    physics.strafe(id, 0.0);
                    physics.fly(id, 0.0);
                } else if let Some((id, v)) = id
                    && !v.wrecked
                {
                    self.throttle = intent.movement.y;
                    // A helicopter flies as the 360 code's pad maps it
                    // (0x824307C0): the movement stick, seen from the camera,
                    // turned into its own ahead and across (its forward and
                    // strafe), and its turn from the camera's turn (the right
                    // stick's across there; here the mouse's turn this frame
                    // over the helicopter's fastest turn, inferred for a
                    // mouse).
                    let heli = v.spec.tuning.flight.filter(|_| v.spec.is_helicopter());
                    let (ahead, across, steer) = match (heli, physics.vehicle_transform(id)) {
                        (Some(f), Some(at)) => {
                            let yaw = player.yaw;
                            let cam_ahead = Vec3::new(yaw.sin(), 0.0, yaw.cos());
                            let cam_right = Vec3::new(-yaw.cos(), 0.0, yaw.sin());
                            let way = cam_ahead * intent.movement.y + cam_right * intent.movement.x;
                            let h = at.transform_vector3(Vec3::Z);
                            let h = h.x.atan2(h.z);
                            let own_ahead = Vec3::new(h.sin(), 0.0, h.cos());
                            let own_right = Vec3::new(-h.cos(), 0.0, h.sin());
                            let turn = intent.look.x / dt.max(1e-3) / f.yaw.max(1e-3);
                            (
                                way.dot(own_ahead).clamp(-1.0, 1.0),
                                way.dot(own_right).clamp(-1.0, 1.0),
                                turn.clamp(-1.0, 1.0),
                            )
                        }
                        _ => (intent.movement.y, intent.movement.x, -intent.movement.x),
                    };
                    if heli.is_some() {
                        physics.strafe(id, across);
                    }
                    // The horn, as long as it is held; each press counts.
                    self.honking = intent.honk;
                    if self.honking && !was_honking {
                        self.honks += 1;
                    }
                    physics.drive(
                        id,
                        VehicleControls {
                            throttle: ahead,
                            // Wheels turned the positive way steer left.
                            steer,
                            brake: intent.hold_jump,
                            parked: false,
                        },
                    );
                    physics.fly(
                        id,
                        f32::from(u8::from(intent.hold_jump))
                            - f32::from(u8::from(intent.reload_held)),
                    );
                    if intent.right_vehicle {
                        physics.right_vehicle(id);
                    }
                }
                physics.step(dt);
                match id.and_then(|(id, v)| Some((physics.vehicle_transform(id)?, v, id))) {
                    Some((at, v, id)) => {
                        let speed = physics.vehicle_speed(id);
                        player.chase(at, ChaseRig::of(&v.spec), intent.look, speed, dt);
                        player.keep_chase_clear(physics, Some(id));
                    }
                    None => {
                        player.driving = None;
                        player.chase_target = None;
                        // At an emplaced gun she stands behind it and only
                        // looks.
                        let intent = match self.combat.manned {
                            Some(_) => Intent {
                                movement: Vec2::ZERO,
                                sprint: false,
                                jump: false,
                                ..intent
                            },
                            None => intent,
                        };
                        let waiting = !player.landed;
                        player.update(physics, intent, dt);
                        if waiting && player.landed && std::mem::take(&mut player.floor_landed) {
                            log_floor(&scene.props, physics, player.character.position);
                        }
                    }
                }
                // A turret turns after the camera, at its own pace, and
                // its gun tilts to what the crosshair is on: the turret
                // her seat aims (its `ControllerTurret`; see
                // [`m2_engine::vehicle::Aimer`]), the main one only from
                // the seat that names it (the driver's, without seat data).
                let her_seat = player
                    .driving
                    .and_then(|i| self.ride.filter(|r| r.0 == i).map(|r| r.1));
                if let Some(v) = player.driving.and_then(|i| scene.vehicles.get_mut(i))
                    && !v.wrecked
                    && let Some(at) = v.id.and_then(|id| physics.vehicle_transform(id))
                    && let Some(k) = her_seat.and_then(|s| v.spec.aimer_of_seat(s))
                {
                    let goal = Combat::aim_point(physics, &scene.people, player, v.id);
                    v.aims.resize(v.spec.aimers.len(), (0.0, 0.0));
                    let want = v.spec.aim_toward(k, at, v.turret, goal);
                    v.aims[k] = v.spec.aimers[k].step(v.aims[k], want, dt);
                }
                if let Some(v) = player.driving.and_then(|i| scene.vehicles.get_mut(i))
                    && !v.wrecked
                    && v.spec.turns_main_turret(her_seat)
                    && let Some(at) = v.id.and_then(|id| physics.vehicle_transform(id))
                {
                    let f = at.transform_vector3(Vec3::Z);
                    // Turned on from where it was last asked to (on its own
                    // body it lags that, [`m2_engine::Physics::turret_target`]).
                    let target =
                        v.id.and_then(|id| physics.turret_target(id))
                            .unwrap_or(v.turret);
                    let mut turn =
                        (player.yaw - f.x.atan2(f.z) - target).rem_euclid(std::f32::consts::TAU);
                    if turn > std::f32::consts::PI {
                        turn -= std::f32::consts::TAU;
                    }
                    let turret = v.spec.tuning.turret;
                    let rate = turret.map_or(TURRET_RATE, |t| t.yaw_speed);
                    // Asked at its turn rate; drawn where its own body has got
                    // to, if it has one ([`m2_engine::Physics::turret_yaw`]).
                    let asked = target + turn.clamp(-rate * dt, rate * dt);
                    v.turret = asked;
                    if let Some(id) = v.id {
                        physics.turn_turret(id, asked);
                        v.turret = physics.turret_yaw(id).unwrap_or(asked);
                    }
                    if let Some(pivot) = v.spec.gun_pivot {
                        let goal = Combat::aim_point(physics, &scene.people, player, v.id);
                        // The crosshair's point as seen from the gun with
                        // the turret straight ahead.
                        let local = (at * v.spec.turret_transform(v.turret))
                            .inverse()
                            .transform_point3(goal)
                            - pivot;
                        let (down, up) = v.spec.elevation;
                        let want = local.y.atan2(local.with_y(0.0).length()).clamp(down, up);
                        let rate = turret.map_or(GUN_RATE, |t| t.pitch_speed);
                        // Asked on from where it was last asked to; a gun
                        // on its own body is drawn where it has got to
                        // ([`m2_engine::Physics::gun_pitch`]).
                        let from = v.id.and_then(|id| physics.gun_target(id)).unwrap_or(v.gun);
                        v.gun = from + (want - from).clamp(-rate * dt, rate * dt);
                        if let Some(id) = v.id {
                            physics.aim_gun(id, v.gun);
                            v.gun = physics.gun_pitch(id).unwrap_or(v.gun);
                        }
                    }
                }
            } else {
                physics.step(dt);
            }
        }
        split("game: player and physics");
        // The vehicles' load beds, looked up once a model.
        if let Some(source) = self.scene.source.as_mut() {
            for v in &self.scene.vehicles {
                self.beds.entry(v.spec.model).or_insert_with(|| {
                    crate::hijack::nodes(source, v.spec.model)
                        .into_iter()
                        .find(|(n, _)| *n == m2_assets::name_hash(BED_NODE))
                        .map(|(_, at)| at)
                });
                if self.beds.get(&v.spec.model).copied().flatten().is_some() {
                    self.bed_shapes
                        .entry(v.spec.model)
                        .or_insert_with(|| bed_shape(source, v.spec.model));
                }
            }
        }
        // Each one's own collision goes on it for the goods on its bed.
        if let Some(world) = &mut self.scene.world {
            for v in &self.scene.vehicles {
                if let (Some(id), Some(Some(shape))) = (v.id, self.bed_shapes.get(&v.spec.model)) {
                    world.physics.add_bed(id, shape);
                }
            }
        }
        let beds: Vec<Option<Vec3>> = self
            .scene
            .vehicles
            .iter()
            .map(|v| self.beds.get(&v.spec.model).copied().flatten())
            .collect();
        let shapes: Vec<BedShape> = self
            .scene
            .vehicles
            .iter()
            .map(|v| self.bed_shapes.get(&v.spec.model).cloned().flatten())
            .collect();
        let focus = self.focus();
        if let Some(world) = &mut self.scene.world {
            let driven = self
                .scene
                .player
                .as_ref()
                .filter(|_| self.on_foot)
                .and_then(|p| p.driving);
            let hold = world.settings.detail_radius;
            for (i, v) in self.scene.vehicles.iter_mut().enumerate() {
                let Some(id) = v.id else { continue };
                // A gunship flies by its pilot ([`crate::gunships`]): not
                // held, parked or still.
                if v.spot.is_some_and(|(e, _, _)| self.gunships.owns(e)) {
                    if !v.wrecked {
                        v.rolled += ROTOR_SPIN * dt;
                    }
                    continue;
                }
                // A pursuit's car is driven at her ([`crate::pursuers`]):
                // not held or parked.
                if v.spot.is_some_and(|(e, _, _)| self.pursuers.owns(e)) {
                    v.rolled += world.physics.vehicle_rolling(id) * dt;
                    continue;
                }
                // Far from her, the island's ground streams out from under
                // a vehicle: it is held where it stands until she comes
                // back and the ground under it is in again.
                if driven != Some(i)
                    && let Some(at) = world
                        .physics
                        .vehicle_transform(id)
                        .map(|m| m.w_axis.truncate())
                {
                    let far = at.distance(focus);
                    if far > hold {
                        world.physics.hold_vehicle(id, true);
                    } else if far < hold - HOLD_SLACK
                        && world.physics.vehicle_held(id)
                        // A boat floats on the sea, whatever is under it.
                        && ((v.spec.is_boat() && world.physics.water().is_some())
                            || world
                                .physics
                                .cast_ray_past(at + Vec3::Y, Vec3::NEG_Y, HOLD_GROUND, id)
                                .is_some())
                    {
                        world.physics.hold_vehicle(id, false);
                    }
                }
                if v.spec.is_helicopter() {
                    // Her helicopter's rotors turn; parked ones stand.
                    if driven == Some(i) && !v.wrecked {
                        v.rolled += ROTOR_SPIN * dt;
                    }
                } else {
                    // As the chassis moves forward: a parked or held
                    // vehicle's tracks stand still (playtest 5).
                    v.rolled += world.physics.vehicle_rolling(id) * dt;
                }
                // Nobody at the wheel: it brakes to a stop and stays put,
                // on a hill too (a wreck drags on its rims, driven by
                // nobody).
                if driven != Some(i) {
                    world.physics.drive(
                        id,
                        VehicleControls {
                            parked: true,
                            ..VehicleControls::default()
                        },
                    );
                }
            }
            self.traffic
                .drive(&mut world.physics, &self.scene.vehicles, driven);
            // A pursuit's gunships, at her.
            if let Some(p) = self.scene.player.as_ref() {
                let her = p.character.position;
                let her_velocity = match p.driving.and_then(|i| self.scene.vehicles.get(i)?.id) {
                    Some(id) => world.physics.vehicle_velocity(id),
                    None => p.character.velocity,
                };
                let mut seed = self.gunships.random();
                let dropped = self.gunships.fly(
                    &mut world.physics,
                    &mut self.scene.vehicles,
                    her,
                    her_velocity,
                    crate::gunships::VANISH,
                    || {
                        seed = (seed * 7919.17).fract();
                        seed
                    },
                    dt,
                );
                for i in dropped {
                    if let Some(id) = self.scene.vehicles[i].id.take() {
                        world.physics.remove_vehicle(id);
                    }
                }
                // And its cars.
                let in_vehicle = p.driving.is_some();
                let dropped = self.pursuers.drive(
                    &mut world.physics,
                    &mut self.scene.vehicles,
                    her,
                    her_velocity,
                    in_vehicle,
                    crate::gunships::VANISH,
                    |v| {
                        world
                            .index
                            .template_db
                            .vehicle_labels_for(&v.spec.label, v.spec.model)
                    },
                    dt,
                );
                for i in dropped {
                    if let Some(id) = self.scene.vehicles[i].id.take() {
                        world.physics.remove_vehicle(id);
                    }
                }
                self.combat.pilot_shots = self.gunships.shots.clone();
                self.combat
                    .pilot_shots
                    .extend(self.pursuers.shots.iter().copied());
                self.combat.pilot_view = Some((self.camera.position, self.camera.forward()));
            }
            self.paths
                .drive(&mut world.physics, &self.scene.vehicles, driven, dt);
            let walking = self
                .scene
                .player
                .as_ref()
                .filter(|p| p.driving.is_none())
                .map(|p| p.character.position);
            self.roads.drive(
                &mut world.physics,
                &self.scene.vehicles,
                &world.index.roads,
                driven,
                walking,
                dt,
            );
            run_over_pickups(world, &mut self.scene.flights, &self.scene.vehicles);
            knock_loose(
                world,
                &mut self.scene.props,
                &self.scene.vehicles,
                (&beds, &shapes, &mut self.beds_told),
                &mut self.destruction,
                self.scene.install.as_ref(),
            );
            // What rides loose on a vehicle goes where its body went; what
            // slid or tipped off the bed is noted.
            let vehicles = &self.scene.vehicles;
            let physics = &world.physics;
            let aboard = |index: usize, centre: Vec3| {
                let Some(v) = vehicles.get(index) else {
                    return false;
                };
                let (Some(id), Some(bed)) = (v.id, beds.get(index).copied().flatten()) else {
                    return true;
                };
                physics
                    .vehicle_transform(id)
                    .is_none_or(|place| within_bed(place, v.model.bounds, bed, centre))
            };
            for (e, index, at) in self.scene.props.carry(physics, aboard) {
                let v = &vehicles[index];
                let speed = v.id.map_or(0.0, |id| physics.vehicle_speed(id));
                eprintln!(
                    "cargo: {e:#x} comes off the {} at {at:.1}, it going {speed:.1} m/s",
                    v.spec.label
                );
            }
            // Vehicles running into each other hurt each other and the
            // parts where they met.
            for (struck, other, closing, damage) in self
                .combat
                .vehicle_crashes(&mut world.physics, &mut self.scene.vehicles)
            {
                let v = &self.scene.vehicles[struck];
                eprintln!(
                    "crash: the {} hit by the {} at {closing:.1} m/s takes {damage:.1} ({:.0} of {:.0} left)",
                    v.spec.label,
                    self.scene.vehicles[other].spec.label,
                    (v.most_health() - v.damage).max(0.0),
                    v.most_health()
                );
            }
            // What came down (to a vehicle, the traffic's too, or a blast
            // last tick) comes down whether she is on foot or the free
            // camera flies: its island copy has lost its collision already.
            self.destruction.burn(
                world,
                &mut self.scene.props,
                self.scene.install.as_ref(),
                dt,
            );
            self.destruction.update(
                world,
                &mut self.scene.props,
                &mut self.combat,
                self.scene.install.as_ref(),
            );
            self.scene.props.advance(dt);
            // Those whose collapse ended leave their `CollapseState`, and
            // are heard as what leaving it sends says (the crash after
            // the rumble).
            for e in self.scene.props.take_collapsed() {
                let props = &self.scene.props;
                let Some(at) = props.centre(e) else { continue };
                let template = props.get(e).map_or(0, |(p, _)| p.template);
                let sent = props.state_messages(e, m2_engine::StateFx::COLLAPSE, true);
                eprintln!("collapse: {e:#x} ({template:08x}) is down; leaving sends {sent:08x?}");
                self.combat.collapse_ended(at, template, &sent);
            }
            self.spray_wait -= dt;
            if self.spray_wait <= 0.0 {
                self.spray_wait = SPRAY_GAP;
                spray(world, &self.scene.vehicles, &mut self.combat);
                exhaust_smoke(
                    world,
                    &self.scene.vehicles,
                    &self.scene.smokes,
                    self.scene.player.as_ref(),
                    &mut self.combat,
                );
            }
        }
        split("game: vehicles");
        let threat = self
            .scene
            .player
            .as_ref()
            .and_then(|p| self.combat.target(p, self.on_foot));
        // What she is in, for soldiers whose guns are no use against it.
        self.scene.people.target_armour = self
            .scene
            .player
            .as_ref()
            .and_then(|p| p.driving)
            .and_then(|i| self.scene.vehicles.get(i))
            .and_then(|v| v.spec.tuning.armour)
            .unwrap_or(1);
        if let Some(world) = &mut self.scene.world
            && (!self.on_foot || self.scene.player.as_ref().is_none_or(|p| p.landed))
        {
            self.scene
                .people
                .update(&mut world.physics, &world.index.spawns, focus, threat, dt);
            let cars: Vec<_> = self
                .scene
                .vehicles
                .iter()
                .filter_map(|v| {
                    let id = v.id?;
                    let at = world.physics.vehicle_transform(id)?;
                    let velocity = world.physics.vehicle_velocity(id);
                    Some((at, velocity, v.spec.hull, v.spec.mass))
                })
                .collect();
            let run_over = self
                .scene
                .people
                .traffic(&cars, self.combat.damage_table.as_ref());
            if run_over > 0 {
                eprintln!("run over: {run_over} soldier(s) killed by a vehicle");
            }
        }
        split("game: people");
        let scene = &mut self.scene;
        if self.on_foot
            && let (Some(world), Some(player)) = (&mut scene.world, &mut scene.player)
        {
            let mut field = Field {
                physics: &mut world.physics,
                player,
                people: &mut scene.people,
                vehicles: &mut scene.vehicles,
                arsenal: &scene.arsenal,
                explosives: &mut scene.explosives,
            };
            crate::spans::time("on foot: combat", || {
                // Her weapons away (swimming, or put away by the scripts):
                // nothing to fire, aim or throw (the on-foot table's `Swim`
                // stance has no firing or aiming rows).
                // An emplaced gun she mans fires all the same (PmcCon031:
                // the scripts put her weapons away for its shooting range,
                // where the targets are brought down from its emplaced
                // guns; inferred from the scripts).
                // The 360's on-foot input (0x824333E0) puts both ammo boxes
                // up (`GuiShowAmmoCounter`) as the reload key or either
                // weapon switch is pressed: its input bits 0x80 (the reload
                // the Lua `Reload` calls), 0x200000 and 8 (the weapon equip
                // and stow `EquipWeapon` and `StowWeapon` call).
                if field.player.driving.is_none()
                    && (intent.trigger.reload
                        || intent.trigger.select.is_some()
                        || intent.trigger.next_explosive)
                {
                    self.combat.ammo_shown += 1;
                }
                let manning = self.combat.manned.is_some() && !field.player.swimming();
                let mut trigger = if field.player.weapons_away() && !manning {
                    intent.trigger.weapons_away()
                } else {
                    intent.trigger
                };
                // The seat key moved her over her seat's links already.
                let seated = field.player.driving.and_then(|i| field.vehicles.get(i));
                if seated.is_some_and(|v| v.spec.seating.is_some()) {
                    trigger.seat = false;
                }
                self.combat.step(&mut field, trigger, dt);
            });
            crate::spans::time("on foot: vehicles hit her", || {
                self.combat.vehicles_hit_her(&mut field, dt);
            });
            world
                .destroyed
                .extend(scene.explosives.iter().filter(|e| e.gone).map(|e| e.object));
            // The cover service: one spot for each soldier who asked
            // ([`m2_engine::ai_cover`]).
            crate::spans::time("on foot: cover", || {
                serve_cover(world, &mut scene.people);
            });
            crate::spans::time("on foot: hit world", || {
                hit_world(
                    world,
                    &mut scene.props,
                    &mut scene.flights,
                    scene.install.as_ref(),
                    &mut self.placed,
                    &mut self.destruction,
                    &mut self.combat,
                );
            });
            // Blockers of soldiers' lines of fire, cleared as worth
            // shooting by the 360's test (0x822EFE48).
            for n in scene.people.npcs.iter_mut() {
                if let Some(b) = n.blocker.as_mut().filter(|b| !b.cleared) {
                    let round = n.gun.map_or(0.0, |g| g.damage);
                    let kind = crate::combat::round_kind(n.gun.and_then(|g| g.damage_key));
                    b.cleared =
                        self.destruction
                            .shootable_blocker(world, &scene.props, b.key, round, kind);
                    if b.cleared {
                        eprintln!(
                            "people: a soldier shoots {:?} in his way at {:.1}",
                            b.key, b.at
                        );
                    }
                }
            }
            // What lies within her reach, taken as the real game takes it
            // ([`crate::mission::offer`]): health and the small ammunition
            // box as she walks over them, if they give her anything; an
            // ammunition box, or a weapon she carries, with the use key for
            // its ammunition ("Take Ammo"); a weapon she doesn't carry with
            // the use key, taken up ("Equip").
            use crate::mission::Offer;
            let templates = &world.index.template_db;
            // What she would take comes ahead of what she would leave
            // lying ([`crate::flights::Flights::within_by`]).
            let chosen = scene.flights.within_by(player.character.position, |p| {
                would_leave(templates, &self.combat, p)
            });
            let chosen_key = chosen.map(crate::flights::Pickup::key);
            let near = chosen.map(|p| {
                let refill = crate::mission::refill(templates, &self.combat, &p.what);
                let usable = p.what.is_money() || self.combat.can_use(refill, p.what.health);
                let offer = crate::mission::offer(templates, &self.combat, &p.what);
                // Each pickup she stands at and leaves lying, once, with
                // why: what each record would go to and its room.
                if !usable && offer != Offer::Equip && self.refused.insert(p.key()) {
                    eprintln!(
                        "pickup: she leaves {} ({:#x}) at {:.1} lying ({offer:?}): {}",
                        templates.by_id(p.what.template).map_or("?", |t| t.0),
                        p.what.template,
                        p.at,
                        crate::mission::refusal(templates, &self.combat, &p.what),
                    );
                }
                (offer, usable)
            });
            let take = match near {
                Some((Offer::Walk, usable)) => usable,
                Some((Offer::TakeAmmo, usable)) => taking && usable,
                Some((Offer::Equip, _)) => taking,
                None => false,
            };
            if player.driving.is_none()
                && self.combat.manned.is_none()
                && take
                && let Some(p) = chosen_key.and_then(|k| scene.flights.take(k))
            {
                let walked = near.is_some_and(|n| n.0 != Offer::Equip);
                let from = p.from;
                if walked && let Some(effect) = taken_effect(templates, p.what.template) {
                    self.combat.game_fx.push(combat::GameFx {
                        template: combat::FxTemplate::Hash(effect),
                        at: p.at,
                        hold: None,
                    });
                }
                // Heard as its own sound set says it is taken (the ammo
                // box's beep: [`m2_engine::spawn_sound::TAKEN_EVENT`]).
                if walked {
                    self.combat.taken(p.at, p.what.template);
                }
                if p.what.is_money() {
                    eprintln!(
                        "pickup: she takes {} ({:#x}) at {:.1}: cash {}, fuel {}",
                        templates.by_id(p.what.template).map_or("?", |t| t.0),
                        p.what.template,
                        p.at,
                        p.what.cash,
                        p.what.fuel
                    );
                    self.earned.push((p.what.cash, p.what.fuel));
                } else {
                    take_up(
                        &mut self.combat,
                        &mut scene.flights,
                        &world.index.template_db,
                        player,
                        p,
                        walked,
                    );
                    // "Take Ammo" is the weapon's context action as
                    // "Equip" is: the 360 posts `PickupWeapon` (8768c3b1)
                    // to her for either before the pickup is settled
                    // (8228FE58), so she picks it up off the ground too.
                    if near.is_some_and(|n| n.0 == Offer::TakeAmmo) {
                        player.start_pickup();
                    }
                }
                // A placed one's prop comes down with it.
                if self.placed.is_pickup(from) {
                    self.placed
                        .taken(from, &mut scene.props, &mut world.physics);
                }
            }
            // Off an emplaced gun once she is down, in a vehicle, or it
            // is gone with its layer.
            if let Some(m) = &self.combat.manned
                && (self.combat.down > 0.0
                    || player.driving.is_some()
                    || scene.props.get(m.entity).is_none_or(|(p, _)| p.wrecked))
            {
                self.combat.manned = None;
            }
        }
        split("game: on foot");
        self.drop_pickups();
        // What the state layers place, as they come and go.
        if let Some(world) = &self.scene.world {
            let new = self.placed.sync(
                &self.scene.props,
                &mut self.scene.flights,
                &world.index.template_db,
                dt,
            );
            if new != crate::placed::Counts::default() {
                eprintln!(
                    "placed: {new:?} more stand; in play {:?}",
                    self.placed.counts()
                );
            }
        }
        split("game: pickups");
        self.support_impacts();
        self.strike_impacts(dt);
        self.run_particles(dt);
        split("game: effects");
        self.scene.tick(dt);
        self.raw.tick(&mut self.scene, dt);
        self.grapple_tick(dt);
        split("game: scene");
    }

    /// The grappling hook's line: the hook flies, or she is pulled in; on
    /// arrival the helicopter's hijack is handed the start.
    fn grapple_tick(&mut self, dt: f32) {
        if let Some(world) = &self.scene.world
            && let Some(&g) = world.index.template_db.grapple.values().next()
        {
            self.grapple.params = g;
        }
        let mut grapple = std::mem::take(&mut self.grapple);
        match grapple.tick(self, dt) {
            Some(crate::grapple::Outcome::Hooked { seconds, metres }) => {
                eprintln!("grapple: hooked {metres:.1} m away after {seconds:.2} s")
            }
            Some(crate::grapple::Outcome::Missed) => eprintln!("grapple: the hook missed"),
            Some(crate::grapple::Outcome::Arrived { seconds }) => {
                eprintln!("grapple: pulled to the entrance in {seconds:.2} s");
                self.grapple_handoff = true;
            }
            None => {}
        }
        self.grapple = grapple;
    }

    /// Puts the player in the model the scripts last dressed her in
    /// (`Player.SetOutfit`), if any is waiting: her body and clips
    /// reloaded from it, her health, place and what she holds kept.
    pub fn dress(&mut self, backend: &mut dyn m2_render::RenderBackend) {
        let Some(model) = self.outfit_wanted.take() else {
            return;
        };
        self.scene.outfit = Some(m2_assets::name_hash(&model));
        eprintln!("outfit: {} dressed as {model}", self.scene.hero);
        let hero = self.scene.hero;
        self.scene.load_hero(backend, hero);
    }

    /// Sets off the game's effects the fight asked for and runs them.
    fn run_particles(&mut self, dt: f32) {
        if let Some(world) = self.scene.world.as_ref() {
            state_effects(world, &mut self.combat);
        }
        let asked = std::mem::take(&mut self.combat.game_fx);
        let scene = &mut self.scene;
        let (Some(p), Some(world)) = (scene.particles.as_mut(), scene.world.as_ref()) else {
            return;
        };
        let created = p.take(asked, &world.index.template_db);
        // What came into being is heard as its template's `SoundEffect`
        // says.
        self.combat.created(created);
        p.step(dt);
    }

    /// The soldiers killed this tick leave what their death script picks,
    /// what their template's `SpawnOnDeath`s name that is a pickup (as a
    /// broken crate's do), and the weapons they carry that are pickups
    /// themselves, where they fell: the `VZ Soldier`'s `Assault Rifle`
    /// (8000436f) has its own `Pickup` records (an equipment record of
    /// radius 1.5, and 8 `Assault Rifle Bullet`) and the ak47's model, so
    /// the rifle a soldier dropped lies there to be taken (PC data,
    /// 2026-10-09).
    fn drop_pickups(&mut self) {
        let deaths = self.scene.people.take_deaths();
        let Some(world) = &self.scene.world else {
            return;
        };
        let templates = &world.index.template_db;
        for (at, spawn) in deaths {
            // Their template, and its labels (the first contract's AT
            // defenders are heavy rocket soldiers).
            let template = spawn
                .and_then(|e| world.index.spawns.iter().find(|s| s.entity == e))
                .and_then(|s| s.template.as_deref());
            let id = template.and_then(|t| templates.id(t));
            let labels = id
                .and_then(|id| templates.leavings.labels.get(&id))
                .map_or(&[][..], Vec::as_slice);
            let is = |label: &str| labels.contains(&m2_assets::name_hash(label));
            let soldier = Soldier {
                heavy: is("HeavySoldier"),
                rocket: is("RocketSoldier"),
                machine_gun: is("MGSoldier"),
                seated: false,
            };
            // What the template names: its pickups on dying, and the
            // weapons it carries, those with their own `Pickup` records
            // dropped as they are.
            let mut left = template.map_or_else(Vec::new, |t| crate::mission::leaves(templates, t));
            let name_of = |t: u32| templates.by_id(t).map_or("?", |t| t.0);
            let equipment = id
                .and_then(|id| templates.equipment.get(&id))
                .map_or(&[][..], Vec::as_slice);
            let carries: Vec<String> = equipment
                .iter()
                .map(|&w| format!("{} {}", name_of(w), crate::mission::records(templates, w)))
                .collect();
            let guns: Vec<_> = equipment
                .iter()
                .filter_map(|&w| crate::mission::pickup(templates, w))
                .collect();
            eprintln!(
                "drop: a {} dies at {at:.1}; its template leaves [{}] and carries [{}]",
                template.unwrap_or("soldier"),
                left.iter()
                    .map(|l| name_of(l.template))
                    .collect::<Vec<_>>()
                    .join(", "),
                carries.join(", ")
            );
            for gun in &guns {
                eprintln!(
                    "drop: its {} ({:#x}, model {:x?}) falls with it, a pickup of reach {:.1}: \
                     taken up with {:?}, walked over for {}",
                    name_of(gun.template),
                    gun.template,
                    gun.model,
                    gun.reach,
                    gun.gives,
                    templates
                        .leavings
                        .pickups
                        .get(&gun.template)
                        .map_or_else(String::new, |p| format!("{:x?}", p.walked))
                );
            }
            left.extend(guns);
            if let Some(name) =
                self.drops
                    .on_death(soldier, Some(self.combat.grenades), self.combat.hp)
            {
                match templates
                    .id(name)
                    .and_then(|id| crate::mission::pickup(templates, id))
                {
                    Some(what) => left.push(what),
                    None => eprintln!("drop: {name} isn't a pickup"),
                }
            }
            for what in left {
                self.dropped += 1;
                eprintln!(
                    "drop: a soldier leaves {} {} at {at:.1}; for her now {:?} (her {} fires {})",
                    name_of(what.template),
                    crate::mission::records(templates, what.template),
                    crate::mission::refill(templates, &self.combat, &what),
                    self.combat.rifle.name,
                    self.combat.rifle.ammo.map_or("?", |a| name_of(a.template))
                );
                self.scene.flights.leave(crate::flights::Pickup {
                    from: DROPPED + self.dropped,
                    what,
                    at: at + Vec3::Y * 0.1,
                    yaw: 0.0,
                });
            }
        }
    }

    /// Her support call this tick ([`crate::support`]): on foot, the
    /// support key, the use key and fire go to it while it is under way,
    /// and the view's steering; what is left of `intent` is hers.
    fn call_support(&mut self, mut intent: Intent, dt: f32) -> Intent {
        let her = self
            .scene
            .player
            .as_ref()
            .filter(|p| self.on_foot && p.landed && p.driving.is_none() && !p.down);
        let at = her
            .map(|p| p.character.position)
            .unwrap_or(self.camera.position);
        let hp = self.combat.hp;
        // Lasing, the point the crosshair is on.
        let aim_at = match (her, &self.scene.world) {
            (Some(p), Some(world)) if self.support.lasing() => Some(Combat::aim_point(
                &world.physics,
                &self.scene.people,
                p,
                None,
            )),
            _ => None,
        };
        let call = match her {
            Some(_) => Call {
                aim_at,
                menu: intent.trigger.support,
                menu_down: intent.trigger.support_down,
                // At the menu or the view, the jump (the pad's A, which
                // the texts' `[confirm]` shows) confirms as well.
                use_pressed: intent.use_pressed || (intent.jump && self.support.busy()),
                fire: intent.trigger.fire,
                steer: intent.walking(),
                select: intent.trigger.select.is_some(),
                hurt: hp < self.support_hp,
                pda: intent.trigger.pda,
            },
            None => Call::default(),
        };
        self.support_hp = hp;
        let before = self.support.busy();
        // In a vehicle, down or flying the camera, a call is dropped (and
        // the PDA shut).
        if her.is_none() && self.support.busy() {
            self.support.phase = support::Phase::Idle;
            if self.support.pda {
                self.support.pda = false;
                self.support.posts.push(support::PDA_CLOSE);
            }
        }
        self.support.step(call, at, dt);
        self.combat.designator = self.support.holding() || self.support.looking().is_some();
        if before || self.support.busy() {
            // At the menu, the designator in her hands or looking through
            // the satellite, she stands, and fire is the satellite's.
            // Lasing, she still looks and aims about.
            let (look, aim) = (intent.look, intent.trigger.aim);
            intent = Intent::default();
            if self.support.lasing() {
                intent.look = look;
                intent.trigger.aim = aim;
            }
        }
        intent
    }

    /// What the support brought down this step: the bomb on what it
    /// first meets at the spot, its blast through the fight, and what
    /// comes loose thrown (the sturdy too).
    fn support_impacts(&mut self) {
        if self.support.landed.is_empty() {
            return;
        }
        let scene = &mut self.scene;
        let (Some(world), Some(player)) = (&mut scene.world, &mut scene.player) else {
            return;
        };
        let high = player.character.position.y + BOMB_DROP;
        for impact in self.support.landed.clone() {
            let Vec2 { x, y: z } = impact.at();
            let top = Vec3::new(x, high, z);
            let at = world
                .physics
                .cast_ray(top, Vec3::NEG_Y, BOMB_DROP * 2.0)
                .map_or(Vec3::new(x, player.character.position.y, z), |(d, _)| {
                    top - Vec3::Y * d
                });
            let mut field = Field {
                physics: &mut world.physics,
                player,
                people: &mut scene.people,
                vehicles: &mut scene.vehicles,
                arsenal: &scene.arsenal,
                explosives: &mut scene.explosives,
            };
            match impact {
                Impact::Bomb(_) => {
                    eprintln!("support: the bomb lands at {at:.1}");
                    self.combat.bomb(&mut field, at);
                    let blasts = bomb_blasts(&world.index.template_db, at);
                    self.combat.last_push = blasts.first().map(|b| b.push);
                    if let Some(light) = blast_light(&world.index.template_db, BOMB_BLAST) {
                        self.script_lights.push(light);
                    }
                    let pushes: Vec<BlastPush> =
                        blasts.first().map(|b| b.push).into_iter().collect();
                    blast_loose(world, &mut scene.props, &pushes, true);
                    if let Some(install) = &scene.install {
                        for blast in blasts {
                            self.destruction
                                .blast(world, &mut scene.props, Some(install), blast);
                        }
                    }
                }
                Impact::Final(_) => self.combat.final_strike(&mut field, at),
            }
            world
                .destroyed
                .extend(scene.explosives.iter().filter(|e| e.gone).map(|e| e.object));
        }
    }

    /// Air strikes fly on; each piece of ordnance gone off makes the
    /// blasts its template leaves, through the fight, throwing what is
    /// loose. The bursts stay in [`Scene::strikes`] for the scripts.
    fn strike_impacts(&mut self, dt: f32) {
        let scene = &mut self.scene;
        let (Some(world), Some(player)) = (&mut scene.world, &mut scene.player) else {
            return;
        };
        let before = scene.strikes.burst.len();
        scene.strikes.step(&world.physics, dt);
        let fresh: Vec<_> = scene.strikes.burst[before..].to_vec();
        for (_, template, at) in fresh {
            let ordnance = &world.index.template_db.ordnance;
            let (ids, infos): (Vec<u32>, Vec<_>) = template
                .map(|t| ordnance.blasts_of(t))
                .unwrap_or_default()
                .iter()
                .filter_map(|b| ordnance.blasts.get(b).map(|i| (*b, i)))
                .unzip();
            let blasts: Vec<(f32, f32)> = infos.iter().map(|b| (b.radius, b.damage)).collect();
            // The strongest push of those going off together
            // ([`crate::destruct::together`]).
            let push = infos
                .iter()
                .max_by(|a, b| a.force.total_cmp(&b.force))
                .map(|b| BlastPush {
                    at,
                    radius: b.radius,
                    force: b.force,
                });
            let mut field = Field {
                physics: &mut world.physics,
                player,
                people: &mut scene.people,
                vehicles: &mut scene.vehicles,
                arsenal: &scene.arsenal,
                explosives: &mut scene.explosives,
            };
            // The first is the blast itself; the rest (its frag) reach
            // further with the same push.
            if let Some(&(radius, damage)) = blasts.first() {
                self.combat
                    .blast(&mut field, at, radius, damage, ids.first().copied());
            }
            for (&(radius, damage), &id) in blasts.iter().zip(&ids).skip(1) {
                self.combat.harm_at(&mut field, at, radius, damage, id);
            }
            if let (Some(reach), Some(push)) = (blasts.iter().map(|b| b.0).reduce(f32::max), push) {
                blast_loose(world, &mut scene.props, &[push], reach >= combat::BOMB.0);
            }
            world
                .destroyed
                .extend(scene.explosives.iter().filter(|e| e.gone).map(|e| e.object));
        }
    }

    /// The game as it stands, to save; `None` without an island or player.
    pub fn save(&self) -> Option<SaveGame> {
        let world = self.scene.world.as_ref()?;
        let player = self.scene.player.as_ref()?;
        let mut at = player.character.position;
        if let Some(v) = player.driving.and_then(|i| self.scene.vehicles.get(i))
            && let Some(m) = v.id.and_then(|id| world.physics.vehicle_transform(id))
        {
            // Saved beside the vehicle, not in it.
            at = m.transform_point3(Vec3::new(v.spec.bounds.0.x - 1.0, 0.0, 0.0));
        }
        let vehicles = self
            .scene
            .vehicles
            .iter()
            // The mission's vehicles are the scripts' to place, the
            // island's the layers'.
            .filter(|v| v.guid.is_none() && v.spot.is_none())
            .filter_map(|v| {
                let m = world.physics.vehicle_transform(v.id?)?;
                let f = m.transform_vector3(Vec3::Z);
                Some(VehicleSave {
                    model: v.spec.model,
                    position: m.transform_point3(Vec3::ZERO).to_array(),
                    yaw: f.x.atan2(f.z),
                    wrecked: v.wrecked,
                })
            })
            // Vehicles a save placed whose models haven't loaded yet.
            .chain(self.restore.iter().cloned())
            .collect();
        Some(SaveGame {
            version: save::VERSION,
            hero: Some(self.scene.hero),
            player: PlayerSave {
                position: at.to_array(),
                facing: player.facing,
                yaw: player.yaw,
                pitch: player.pitch,
                hp: self.combat.hp,
                weapon: match self.combat.weapon {
                    Weapon::Rifle => "rifle",
                    Weapon::Rpg => "rpg",
                    Weapon::GrenadeLauncher => "grenade launcher",
                }
                .into(),
                second: match self.combat.second {
                    Some(Weapon::GrenadeLauncher) => "grenade launcher",
                    Some(_) => "rpg",
                    None => "",
                }
                .into(),
                launcher: self
                    .combat
                    .launcher_rounds
                    .filter(|_| self.combat.second.is_some())
                    .map(|r| [r.clip, r.stored]),
                grenades: self.combat.grenades,
                rifle: Some([
                    self.combat.rifle_rounds.clip,
                    self.combat.rifle_rounds.stored,
                ]),
                guns: [
                    self.combat.rifle.name.clone(),
                    self.combat.launcher.name.clone(),
                ],
                explosives: [
                    self.combat.explosive as u32,
                    self.combat.charges,
                    self.combat.mines,
                ],
            },
            vehicles,
            killed: self.scene.people.all_killed(),
            // The scripts' to fill in ([`crate::mission::MissionLink::pmc_save`]).
            pmc: None,
            scripts: None,
        })
    }

    /// Puts the game back as `save` left it: the player drops to the ground
    /// where she was, vehicles go back as their models load, and the dead
    /// stay dead.
    pub fn load(&mut self, save: SaveGame) {
        let (Some(world), Some(player)) = (&mut self.scene.world, &mut self.scene.player) else {
            return;
        };
        for v in &mut self.scene.vehicles {
            if let Some(id) = v.id.take() {
                world.physics.remove_vehicle(id);
            }
            v.wrecked = false;
            v.damage = 0.0;
        }
        let p = &save.player;
        player.drop_at(Vec3::from(p.position));
        player.driving = None;
        player.chase_target = None;
        player.down = false;
        player.aiming = false;
        player.facing = p.facing;
        player.yaw = p.yaw;
        player.pitch = p.pitch;
        let max_hp = self.combat.max_hp;
        let old = std::mem::take(&mut self.combat);
        self.combat.max_hp = max_hp;
        self.combat.rifle = old.rifle;
        self.combat.own_rifle = old.own_rifle;
        self.combat.scripted_loadout = old.scripted_loadout;
        self.combat.hero_damage = old.hero_damage;
        // What the templates gave her weapons stays.
        self.combat.damage_table = old.damage_table;
        self.combat.throwables = old.throwables;
        self.combat.carbine = old.carbine;
        self.combat.rpg = old.rpg;
        self.combat.gl = old.gl;
        self.combat.launcher = self.combat.rpg.clone();
        self.combat.hp = p.hp.clamp(1.0, max_hp);
        let [rifle, launcher] = &p.guns;
        let named = |name: &str| {
            self.scene
                .arms()
                .and_then(|a| a.named(name))
                .filter(|w| w.class.is_some() && !name.is_empty())
                .map(|spec| self.combat.stock(spec))
        };
        let (rifle, launcher) = (named(rifle), named(launcher));
        match (p.second.as_str(), p.launcher, launcher) {
            (_, rounds, Some(gun)) if gun.slot() != Weapon::Rifle => {
                self.combat.equip_gun(gun, 0);
                if let Some([clip, stored]) = rounds {
                    self.combat.launcher_rounds = Some(combat::Rounds { clip, stored });
                }
            }
            ("grenade launcher", Some([clip, stored]), _) => {
                self.combat.equip(Weapon::GrenadeLauncher, 0);
                self.combat.launcher_rounds = Some(combat::Rounds { clip, stored });
            }
            ("rpg", rounds, _) => {
                if let Some([clip, stored]) = rounds {
                    self.combat.launcher_rounds = Some(combat::Rounds { clip, stored });
                }
            }
            _ => self.combat.carbine_only(),
        }
        // The gun in her rifle's place, if the save names one.
        if let Some(gun) = rifle.filter(|g| g.slot() == Weapon::Rifle) {
            self.combat.equip_gun(gun, 0);
        }
        self.combat.weapon = match p.weapon.as_str() {
            "rifle" => Weapon::Rifle,
            _ => self.combat.second.unwrap_or(Weapon::Rifle),
        };
        self.combat.grenades = p.grenades.min(combat::MAX_GRENADES);
        let [explosive, charges, mines] = p.explosives;
        self.combat.charges = charges;
        self.combat.mines = mines;
        self.combat.explosive = (explosive as usize).min(2);
        if let Some([clip, stored]) = p.rifle {
            self.combat.rifle_rounds = combat::Rounds {
                clip: clip.min(self.combat.rifle.clip),
                stored: stored.min(self.combat.rifle.spare),
            };
        }
        self.combat.weapon = match (p.weapon.as_str(), self.combat.second) {
            ("rpg" | "grenade launcher", Some(w)) => w,
            _ => Weapon::Rifle,
        };
        self.scene.people.reset(save.killed);
        self.restore = save.vehicles;
        // Vehicles not in the save park afresh.
        self.park_tries = 0;
        self.on_foot = true;
    }

    /// Parking is finished: every vehicle loaded is parked, or the search
    /// gave up.
    pub fn parking_done(&self) -> bool {
        self.park_tries >= PARK_TRIES || !self.scene.vehicles.iter().any(parkable)
    }

    /// What the HUD shows, its words from `strings` (the game's English
    /// strings, once the scripts have read them).
    pub fn hud_info(&self, strings: Option<&m2_assets::StringTable>) -> HudInfo {
        let player = self.scene.player.as_ref();
        let loading = match &self.scene.world {
            // The island-wide tiles; nearby detail streams in quietly.
            Some(w) if w.stats().tiles + w.stats().failed < w.index.tiles.len() => {
                Some("Loading the island...".to_owned())
            }
            Some(_) if self.scene.vehicles_loading() => {
                Some("Finding vehicles and people...".to_owned())
            }
            _ if self.arrival.is_some() => Some("Arriving...".to_owned()),
            _ => None,
        };
        let driving = player.and_then(|p| p.driving).and_then(|i| {
            let v = self.scene.vehicles.get(i)?;
            let speed = self.driving().map_or(0.0, |d| d.1);
            // In a gunner's or passenger's seat, so marked.
            let label = match self.combat.seat {
                0 => v.spec.label.clone(),
                combat::NO_GUN => format!("{} (passenger)", v.spec.label),
                _ => format!("{} (gunner)", v.spec.label),
            };
            Some((label, speed, v.wrecked))
        });
        // What the crosshair is on: on foot, or in a vehicle with a gun.
        let crosshair = match (&self.scene.world, player) {
            (Some(w), Some(p)) if self.combat.down <= 0.0 && !self.dialog_up && !self.aim_off => {
                match p.driving {
                    Some(i) => self
                        .scene
                        .vehicles
                        .get(i)
                        .filter(|v| {
                            (v.spec.muzzle.is_some() || !v.spec.armed.is_empty()) && !v.wrecked
                        })
                        .map(|v| Combat::aim_point(&w.physics, &self.scene.people, p, v.id)),
                    None if self.on_foot && p.landed => {
                        Some(Combat::aim_point(&w.physics, &self.scene.people, p, None))
                    }
                    None => None,
                }
            }
            _ => None,
        };
        let can_enter = match (&self.scene.world, player) {
            (Some(w), Some(p)) if p.driving.is_none() && p.landed => entrance_pick(
                &w.physics,
                &self.scene.vehicles,
                (&self.hijack_only, &self.unusable),
                p,
                self.last_vehicle,
                &[],
                None,
            )
            .map(|(i, _)| {
                let spec = &self.scene.vehicles[i].spec;
                vehicle_name(&w.index.template_db, strings, &spec.label, spec.model)
            }),
            _ => None,
        };
        // What the use key takes, as the game words it.
        let prompt = match (&self.scene.world, player) {
            (Some(w), Some(p)) if p.driving.is_none() && p.landed && self.on_foot => {
                use crate::mission::Offer;
                let db = &w.index.template_db;
                let word = |key: u32, english: &str| {
                    strings
                        .and_then(|s| s.strings.get(&key).cloned())
                        .unwrap_or_else(|| english.to_owned())
                };
                match self
                    .scene
                    .flights
                    .within_by(p.character.position, |l| would_leave(db, &self.combat, l))
                {
                    Some(lying) => match crate::mission::offer(db, &self.combat, &lying.what) {
                        // Walked over, unworded.
                        Offer::Walk => None,
                        Offer::TakeAmmo => {
                            let refill = crate::mission::refill(db, &self.combat, &lying.what);
                            self.combat
                                .can_use(refill, lying.what.health)
                                .then(|| word(TAKE_AMMO, "Take Ammo"))
                        }
                        Offer::Equip => Some(word(EQUIP, "Equip %s").replace(
                            "%s",
                            &shown_name(db, strings, lying.what.template, lying.what.gun),
                        )),
                    },
                    None if self.combat.manned.is_none() => self
                        .placed
                        .emplacement_near(&self.scene.props, db, p.character.position)
                        .map(|m| {
                            word(USE, "Use %s")
                                .replace("%s", &shown_name(db, strings, m.template, None))
                        }),
                    None => None,
                }
            }
            _ => None,
        };
        HudInfo {
            playing: self.on_foot && player.is_some(),
            // In a vehicle, the health box shows the vehicle's.
            hp: player
                .and_then(|p| p.driving)
                .and_then(|i| self.scene.vehicles.get(i))
                .map_or(self.combat.hp_percent(), ParkedVehicle::health),
            down: self.combat.down > 0.0,
            weapon: self.combat.weapon,
            rifle: self.combat.rifle.name.clone(),
            second: self.combat.second,
            launcher: self.combat.launcher.name.clone(),
            grenades: self.combat.explosive_left().1,
            explosive: match self.combat.explosive {
                0 => String::new(),
                _ => self.combat.explosive_left().0.to_owned(),
            },
            // At an emplaced gun, its rounds.
            rounds: match (&self.combat.manned, self.combat.weapon) {
                (Some(m), _) => Some(combat::Rounds {
                    clip: m.rounds,
                    stored: 0,
                }),
                (None, Weapon::Rifle) => Some(self.combat.rifle_rounds),
                (None, Weapon::GrenadeLauncher | Weapon::Rpg) => self.combat.launcher_rounds,
            }
            .map(|r| (r.clip, r.stored)),
            // Looking through the satellite, its own overlay is shown.
            prompt: self
                .context
                .as_ref()
                .filter(|_| self.on_foot && !self.input_locked)
                .map(|(_, text)| text.clone())
                .or(prompt)
                .filter(|_| !self.support.busy()),
            hijack_keys: None,
            driving,
            crosshair: crosshair.filter(|_| self.support.looking().is_none()),
            can_enter: can_enter.filter(|_| !self.support.busy()),
            loading,
            support: self
                .support
                .hud(player.map_or(self.camera.position.y, |p| p.character.position.y)),
            hero: self.scene.hero,
            pickups: player.map_or_else(Vec::new, |p| {
                self.scene
                    .flights
                    .marker_points(p.character.position, crate::pickup_marker::NEAR_HER)
                    .into_iter()
                    .filter_map(|(at, lying)| {
                        Some(crate::pickup_marker::PickupMarker {
                            at,
                            icon: lying.what.icon?,
                            flat: lying.what.icon_flat,
                        })
                    })
                    .collect()
            }),
            entrance_markers: match (&self.scene.world, player) {
                // None while she rides anything: a hijack has her in the
                // vehicle's `InVehicle` stance with the controls off, and
                // the 360 queues none then (82376A48).
                (Some(w), Some(p)) if p.driving.is_none() && !self.input_locked => {
                    entrance_markers(
                        &w.physics,
                        &w.index.template_db,
                        &self.scene,
                        (&self.hijack_only, &self.unusable),
                        p.character.position,
                    )
                }
                _ => Vec::new(),
            },
            ..HudInfo::default()
        }
    }

    /// Puts the player in the nearest vehicle within `reach` metres (any
    /// distance for `f32::INFINITY`). False if there is none.
    pub fn enter_nearest(&mut self, reach: f32) -> bool {
        let (Some(world), Some(player)) = (&self.scene.world, &mut self.scene.player) else {
            return false;
        };
        player.driving = nearest(
            &world.physics,
            &self.scene.vehicles,
            (&self.hijack_only, &self.unusable),
            player,
            reach,
        );
        player.landed |= player.driving.is_some();
        let Some(i) = player.driving else {
            return false;
        };
        self.take_from_driver(i);
        true
    }

    /// She sits in vehicle `i`'s driver's seat, as the use key has it:
    /// whoever drove it is put out.
    pub fn take_from_driver(&mut self, i: usize) {
        let (Some(world), Some(player)) = (&self.scene.world, &self.scene.player) else {
            return;
        };
        let db = &world.index.template_db;
        let out = take_seat(&mut self.scene.vehicles[i])
            .into_iter()
            .map(|who| {
                let model = db.get(&who).and_then(|t| t.model);
                (who, model)
            })
            .collect();
        put_out(
            &mut self.scene.people,
            &self.scene.riders,
            &self.scene.vehicles[i],
            &world.physics,
            i,
            out,
            player.character.position,
        );
    }

    /// Loads the first of `models` that drives and parks it near her, as
    /// the vehicles she starts with are.
    pub fn park_more(&mut self, models: Vec<CatalogueEntry>) {
        self.scene.load_parked(models);
        self.park_tries = 0;
        self.park_wait = 0.0;
    }

    /// The vehicle being driven: its position and speed (m/s).
    pub fn driving(&self) -> Option<(Vec3, f32)> {
        let world = self.scene.world.as_ref()?;
        let i = self.scene.player.as_ref()?.driving?;
        let id = self.scene.vehicles.get(i)?.id?;
        let at = world.physics.vehicle_transform(id)?;
        Some((
            at.transform_point3(Vec3::ZERO),
            world.physics.vehicle_speed(id),
        ))
    }

    /// The nearest vehicles to `ear` someone else drives, as their engines
    /// are heard ([`crate::sound::ENGINES_TRACKED`]).
    pub fn others_engines(&self, ear: Vec3) -> Vec<crate::sound::HeardEngine> {
        let Some(world) = self.scene.world.as_ref() else {
            return Vec::new();
        };
        let driven = self.scene.player.as_ref().and_then(|p| p.driving);
        let mut near: Vec<(f32, crate::sound::HeardEngine)> = self
            .scene
            .vehicles
            .iter()
            .enumerate()
            .filter(|(i, v)| driven != Some(*i) && v.exhausting(false))
            .filter_map(|(i, v)| {
                let set = *self.scene.engine_sets.get(&v.spec.model)?;
                let id = v.id?;
                let at = world.physics.vehicle_transform(id)?.w_axis.truncate();
                (set != 0).then_some((
                    at.distance(ear),
                    crate::sound::HeardEngine {
                        id: i,
                        at,
                        set,
                        speed: world.physics.vehicle_speed(id),
                        top: v.spec.tuning.max_speed.unwrap_or(if v.spec.tracked {
                            14.0
                        } else {
                            32.0
                        }),
                    },
                ))
            })
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        near.truncate(crate::sound::ENGINES_TRACKED);
        near.into_iter().map(|n| n.1).collect()
    }

    /// The vehicle being driven's engine: its sound bank (the one its
    /// template names, else one for its kind), speed and throttle. A wreck
    /// has none.
    pub fn engine(&self) -> Option<crate::sound::EngineInput> {
        let i = self.scene.player.as_ref()?.driving?;
        let v = self.scene.vehicles.get(i).filter(|v| !v.wrecked)?;
        let (_, speed) = self.driving()?;
        let bank = self
            .scene
            .world
            .as_ref()
            .and_then(|w| w.index.template_db.engine_for_model(v.spec.model))
            .unwrap_or_else(|| {
                m2_assets::name_hash(crate::sound::engine_bank(&v.spec.kind, &v.spec.label))
            });
        let set = self
            .scene
            .world
            .as_ref()
            .and_then(|w| w.index.template_db.engine_set_for_model(v.spec.model))
            .unwrap_or(0);
        let physics = self.scene.world.as_ref().map(|w| &w.physics);
        let id = v.id;
        let (mut revs, mut slide, mut surface, mut braking) = (None, 0.0, 0.0, false);
        if let (Some(p), Some(id)) = (physics, id) {
            // The top revs are its `GearMaxRPM` default of 8000 where the
            // gearbox's own aren't to hand (360's divides by a record's).
            revs = p.vehicle_gear(id).map(|(_, rpm)| (rpm / 8000.0).min(1.0));
            braking = p.vehicle_braking(id);
            if let Some(at) = p.vehicle_transform(id) {
                slide = p.vehicle_velocity(id).dot(at.x_axis.truncate());
                surface = m2_engine::surface::tyre_class(p.vehicle_ground(id, 5.0));
            }
        }
        Some(crate::sound::EngineInput {
            bank,
            set,
            speed,
            top: v
                .spec
                .tuning
                .max_speed
                .unwrap_or(if v.spec.tracked { 14.0 } else { 32.0 }),
            throttle: self.throttle,
            horn: self.honking,
            heavy: heavy_horn(&v.spec.kind, &v.spec.label, v.spec.tracked),
            revs,
            slide,
            surface,
            braking,
        })
    }

    /// The horn presses since the last call (the faction gates, the
    /// disguise and the friendly soldiers hear each one).
    pub fn take_honks(&mut self) -> u32 {
        std::mem::take(&mut self.honks)
    }

    /// Switches between walking and flying (a developer's aid, `--dev`).
    /// Flying starts from the player's camera, and not while the scripts
    /// have her controls off; walking takes up her body where it was left
    /// (or where the scripts moved it meanwhile), not where the camera is:
    /// on foot she lands there again once the ground around it has
    /// streamed back in.
    pub fn toggle_fly(&mut self) {
        let Some(player) = &mut self.scene.player else {
            return;
        };
        if self.on_foot {
            if self.input_locked {
                eprintln!("free camera: not while the scripts have her controls off");
                return;
            }
            self.camera.position = player.eye();
            self.camera.yaw = player.yaw;
            self.camera.pitch = player.pitch;
        } else if player.driving.is_none() && player.landed {
            player.drop_at(player.character.position);
        }
        self.on_foot = !self.on_foot;
    }

    pub fn walking(&self) -> Option<&crate::player::Player> {
        self.scene.player.as_ref().filter(|_| self.on_foot)
    }

    /// The heading of the camera the frame is seen from ([`Self::view`]),
    /// radians as [`FlyCamera::yaw`]: the HUD's compass and minimap turn
    /// with it.
    pub fn view_heading(&self) -> f32 {
        view_heading(self.walking(), &self.camera)
    }

    /// Moves the light on to the atmosphere region the player is in.
    fn update_sky(&mut self, dt: f32) {
        let focus = self.focus();
        let Some(world) = &self.scene.world else {
            return;
        };
        let Some(default) = &world.index.atmosphere else {
            return;
        };
        let regions = &world.index.atmosphere_regions;
        for (left, light) in &mut self.script_lights {
            *left -= dt;
            if *left <= 0.0 {
                let unused = self.sky.script(default, light);
                eprintln!(
                    "atmosphere: a blast's script sets the light (restored over {:?} s; not drawn: {unused:?})",
                    light.restore
                );
            }
        }
        self.script_lights.retain(|(left, _)| *left > 0.0);
        if let Some(now) = self.sky.update(default, regions, focus, dt) {
            match now {
                Some((r, s)) => {
                    let (region, (_, a)) = (&regions[r], &regions[r].settings[s]);
                    eprintln!(
                        "atmosphere: region {} record {s} (light time {}, intensity {}, light {:?}, tone {:?}, coverage {}, tone map {:?})",
                        region.name,
                        a.light_time[0],
                        a.light_intensity,
                        a.light_color,
                        a.tone,
                        a.coverage,
                        a.tone_map
                    );
                }
                None => eprintln!("atmosphere: the island's default"),
            }
        }
    }

    /// Puts atmosphere region `region` (a script's guid for it) on its
    /// record named `setting`, as `ChangeLineRegionSetting` does.
    pub fn atmosphere_setting(&mut self, region: u32, setting: &str) {
        let Some(world) = &self.scene.world else {
            return;
        };
        if !self
            .sky
            .choose(&world.index.atmosphere_regions, region, setting)
        {
            eprintln!("atmosphere: no region {region:08x} with a record {setting}");
        }
    }

    /// Where the world streams around.
    pub fn focus(&self) -> Vec3 {
        match self.walking() {
            Some(p) => p.character.position,
            None => self.camera.position,
        }
    }

    /// Where sound is heard from: the camera's position and direction.
    pub fn listener(&self) -> (Vec3, Vec3) {
        match self.walking() {
            Some(p) => (p.eye(), p.camera_forward()),
            None => (self.camera.position, self.camera.forward()),
        }
    }

    /// Where the frame is seen from: her camera (shaken by a bomb close
    /// by), the satellite's while she looks through it, or the free one.
    pub fn view(&self, fov: f32, aspect: f32) -> View {
        let shot = self.shot.as_ref().and_then(|s| s.view);
        let mut view = match (self.walking(), shot) {
            // A scene's shot of the scripts' (the HQ's briefings).
            (Some(_), Some((eye, at))) => FlyCamera::looking_at(eye, at).view(fov, aspect),
            (p, _) => match p {
                Some(p) => match self.support.looking() {
                    Some(at) => support::satellite_view(at, p.character.position.y, aspect),
                    None => p.view(fov, aspect),
                },
                None => self.camera.view(fov, aspect),
            },
        };
        if let Some((a, b)) = self.support.shake_turn()
            && self.support.looking().is_none()
        {
            view.view = Mat4::from_rotation_x(a) * Mat4::from_rotation_y(b) * view.view;
        }
        // The island's own ambient light, sunlight strength, haze and
        // clouds.
        if let Some(world) = &self.scene.world {
            if let Some(a) = world.index.atmosphere {
                let now = self.sky.current(&a);
                now.apply(&mut view.lighting);
                // `M2_ATMO_TRACE=on` logs the light about every half
                // second, to follow a change between records.
                if std::env::var("M2_ATMO_TRACE").is_ok_and(|v| v == "on") {
                    use std::sync::atomic::{AtomicU32, Ordering};
                    static LAST: AtomicU32 = AtomicU32::new(u32::MAX);
                    let since = self.sky.since_change();
                    let tick = (since * 2.0) as u32;
                    if LAST.swap(tick, Ordering::Relaxed) != tick {
                        eprintln!(
                            "atmosphere trace {:.1} s into the change: light {:?} li {:.3} tone {:?} darkness {:.1} coverage {:.3} time {:.3} inscatter {:.2} mono {:?} sun {:?} eye {:?}",
                            since,
                            now.light_color,
                            now.light_intensity,
                            now.tone,
                            now.cloud_darkness,
                            now.coverage,
                            now.light_time[0],
                            now.inscattering,
                            now.monochrome,
                            now.sun_direction(),
                            view.view.inverse().z_axis.truncate(),
                        );
                    }
                }
            }
            view.lighting.clouds.noise = world.cloud_noise;
        }
        finish(&mut view.lighting);
        post_override(&mut view.lighting);
        if self.support.looking().is_some() {
            satellite_look(&mut view.lighting);
        }
        self.picture.apply(&mut view.lighting);
        // The effects' lights (a blast's flash, a fire), the nearest first.
        if let Some(p) = &self.scene.particles {
            let eye = view.view.inverse().w_axis.truncate();
            view.lighting.point_lights = p.point_lights(eye);
        }
        view
    }
}

/// How far the world is drawn for the menus' View Distance (10 to 100, as
/// their setters clamp it): placed models, props and ground scatter out
/// to their usual reach at 100, a quarter of it at 10, in a line between
/// (invented: the game's own distances for it are not known).
pub fn view_distance_scale(v: f64) -> f32 {
    let t = (v.clamp(10.0, 100.0) - 10.0) / 90.0;
    (0.25 + 0.75 * t) as f32
}

impl Game {
    /// Draws the world as far as the menus' View Distance says
    /// ([`view_distance_scale`]).
    pub fn set_view_distance(&mut self, v: f64) {
        let scale = view_distance_scale(v);
        let usual = m2_engine::WorldSettings::default();
        if let Some(world) = &mut self.scene.world {
            let s = &mut world.settings;
            s.object_distance = usual.object_distance * scale;
            s.prop_distance = usual.prop_distance * scale;
            s.scatter_distance = usual.scatter_distance * scale;
        }
    }
}

/// The menus' picture options as the frame takes them: Brightness (0 to
/// 100, 50 as the game is lit) and the advanced video screen's shadows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Picture {
    pub brightness: f64,
    pub shadows: bool,
}

impl Default for Picture {
    fn default() -> Self {
        Self {
            brightness: 50.0,
            shadows: true,
        }
    }
}

impl Picture {
    /// Brightness scales the finished frame's light, a stop down at 0 and
    /// up at 100 (invented: the game's gamma curve is not known); shadows
    /// off, the sun casts none.
    pub fn apply(&self, lighting: &mut m2_render::Lighting) {
        let stops = (self.brightness.clamp(0.0, 100.0) - 50.0) / 50.0;
        lighting.grade *= 2f32.powf(stops as f32);
        if !self.shadows {
            lighting.shadow_distance = 0.0;
        }
    }
}

/// The satellite's picture, as the first contract's videos show it: grey,
/// its colours nearly gone, and grainy. Grey and grain are matched by eye
/// to the videos, not read from the game.
fn satellite_look(lighting: &mut m2_render::Lighting) {
    // The game's tone map takes the monochrome ramp, the fitted pass its
    // saturation.
    lighting.monochrome = Some(Vec4::new(0.46, 0.46, 0.44, 0.9));
    lighting.saturation *= 0.1;
    lighting.grain = 0.07;
}

/// The finishing pass, fitted to footage of the game: whole-frame
/// brightness, saturation, warmth and shade against sunlight from PC
/// footage of the jungle, held back where the strongest fit made the
/// AMX-30 too dark and orange against console footage of it. The game runs
/// a tone map of its own (see [`game_tone_map`]).
fn finish(lighting: &mut m2_render::Lighting) {
    if game_tone_map() && lighting.tone_map.is_some() {
        // The game's own, no fitted pass.
        lighting.exposure = 1.0;
        return;
    }
    lighting.tone_map = None;
    lighting.exposure = 0.9;
    lighting.contrast = 1.35;
    lighting.saturation = 1.5;
    lighting.grade *= Vec3::new(1.3, 0.95, 0.7);
    // The eye adapts as the game's does (its atmospheres set a target
    // luminance and how much of the way to adapt): dusk and shade are
    // brought up part of the way. Key, strength and speed are fitted.
    lighting.adaptation = Some(m2_render::Adaptation {
        key: ADAPT_KEY,
        strength: 0.5,
        speed: 1.5,
        limits: [0.5, 4.0],
    });
}

/// Frames are finished with the game's own adaptation and tone map from
/// the atmosphere's settings; `M2_TONEMAP=fit` uses the earlier pass fitted
/// to footage instead, for comparison.
fn game_tone_map() -> bool {
    static GAME: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *GAME.get_or_init(|| std::env::var("M2_TONEMAP").map_or(true, |v| v != "fit"))
}

/// The frame's average luminance (before exposure) that adaptation leaves
/// as it is: about the island's sunlit town by day.
const ADAPT_KEY: f32 = 0.25;

/// `M2_POST=exposure=0.8,bloom=0.35,threshold=1,contrast=1.3,
/// saturation=1.6,grade=1.05/1/0.9,haze=1,inscatter=1,density=4,
/// key=0.25,adapt=0.7,mono=1` sets the finishing pass and haze by hand (any
/// subset; `haze` and `inscatter` scale the scattering and the light it
/// adds, `density` replaces the air's thickness near the ground, `key`
/// and `adapt` are the adaptation's key and strength, 0 for none, `mono`
/// scales how much the game's monochrome ramp takes), for
/// matching reference frames.
fn post_override(lighting: &mut m2_render::Lighting) {
    static POST: std::sync::OnceLock<Vec<(String, Vec<f32>)>> = std::sync::OnceLock::new();
    let settings = POST.get_or_init(|| {
        let Ok(s) = std::env::var("M2_POST") else {
            return Vec::new();
        };
        s.split(',')
            .filter_map(|kv| {
                let (k, v) = kv.split_once('=')?;
                let v: Vec<f32> = v.split('/').filter_map(|x| x.trim().parse().ok()).collect();
                Some((k.trim().to_owned(), v))
            })
            .collect()
    });
    for (k, v) in settings {
        match (k.as_str(), v.as_slice()) {
            ("exposure", [x]) => lighting.exposure = *x,
            ("bloom", [x]) => lighting.bloom = *x,
            ("threshold", [x]) => lighting.bloom_threshold = *x,
            ("contrast", [x]) => lighting.contrast = *x,
            ("saturation", [x]) => lighting.saturation = *x,
            ("grade", [r, g, b]) => lighting.grade = Vec3::new(*r, *g, *b),
            ("haze", [x]) => {
                lighting.haze.rayleigh *= *x;
                lighting.haze.mie *= *x;
            }
            ("inscatter", [x]) => lighting.haze.inscattering *= *x,
            ("density", [x]) => lighting.haze.density = *x,
            ("key", [x]) => {
                if let Some(a) = &mut lighting.adaptation {
                    a.key = *x;
                }
            }
            ("mono", [x]) => {
                if let Some(m) = &mut lighting.monochrome {
                    m.w *= *x;
                }
            }
            ("adapt", [x]) => {
                if let Some(a) = &mut lighting.adaptation {
                    a.strength = *x;
                }
            }
            _ => {}
        }
    }
}

/// How close she must be to a vehicle's door (or side) to get in, metres.
const REACH: f32 = 4.0;

/// A weapon she leaves for another lies this far to her right, metres.
const LEFT_BESIDE: f32 = 0.6;

/// What she takes from a pickup: a weapon is taken up in the launcher's
/// place (what she carried there left lying beside her, to be taken
/// back), ammunition goes with its weapon. One `walked` over (not taken
/// with the use key) gives only its ammunition, a weapon's too.
fn take_up(
    combat: &mut Combat,
    flights: &mut crate::flights::Flights,
    templates: &m2_engine::Templates,
    player: &mut Player,
    p: crate::flights::Pickup,
    walked: bool,
) {
    let rounds = p.what.gives.map_or(0, |(_, n)| n);
    let Some(weapon) = p.what.weapon.filter(|_| !walked) else {
        let refill = crate::mission::refill(templates, combat, &p.what);
        combat.take_supplies(refill, p.what.health);
        eprintln!(
            "pickup: she takes {} ({:#x}) at {:.1}: {} rounds for her {}, {} for her {}, \
             {} grenades, {} health; she has {}+{}, {:?}, {} grenades",
            templates.by_id(p.what.template).map_or("?", |t| t.0),
            p.what.template,
            p.at,
            refill.rifle,
            combat.rifle.name,
            refill.launcher,
            combat
                .second
                .map_or("nothing", |_| combat.launcher.name.as_str()),
            refill.grenades,
            p.what.health,
            combat.rifle_rounds.clip,
            combat.rifle_rounds.stored,
            combat.launcher_rounds.map(|r| (r.clip, r.stored)),
            combat.grenades
        );
        return;
    };
    let spec = p.what.gun.and_then(|g| templates.arms.weapons.get(&g));
    let gun = match (spec, weapon) {
        (Some(spec), _) => combat.stock(spec),
        (None, Weapon::GrenadeLauncher) => combat.gl.clone(),
        (None, Weapon::Rpg) => combat.rpg.clone(),
        (None, Weapon::Rifle) => combat.carbine.clone(),
    };
    let set = gun.feel.sound_set;
    let left = combat
        .equip_gun(gun, rounds)
        .map(|(g, n)| (g.slot(), n, g.template, g.model_hash, g.name));
    // Taken up with the use key, she picks it up off the ground
    // (`PickupWeapon`); walked over, it is just had.
    if !walked {
        player.start_pickup();
    }
    // Its sound set's `equip_pickup` cue.
    combat.cue_sound(crate::combat::Sound::Pickup { set });
    eprintln!(
        "pickup: she takes up the {} ({:#x}) at {:.1} with {rounds} rounds{}",
        spec.map_or(format!("{weapon:?}"), |s| s.name.clone()),
        p.what.template,
        p.at,
        match &left {
            Some((_, 0, _, _, name)) => format!(", leaving her {name}"),
            Some((_, n, _, _, name)) => format!(", leaving her {name} with {n} rounds"),
            None => String::new(),
        }
    );
    let Some((left, kept, gun, model, _)) = left else {
        return;
    };
    let model = model.unwrap_or_else(|| m2_assets::name_hash(left.model_name()));
    // Its template: the pickup with its model (the lowest id of several).
    let template = templates
        .leavings
        .pickups
        .iter()
        .filter(|(id, _)| {
            templates
                .by_id(**id)
                .is_some_and(|(_, t)| t.model == Some(model))
        })
        .min_by_key(|(id, _)| **id);
    let (sin, cos) = player.facing.sin_cos();
    let right = Vec3::new(-cos, 0.0, sin);
    flights.leave(crate::flights::Pickup {
        from: 0,
        what: crate::flights::Leaves {
            template: template.map_or(0, |(id, _)| *id),
            model: Some(model),
            reach: template.map_or(1.0, |(_, info)| info.radius),
            weapon: Some(left),
            gun,
            gives: Some((left, kept)),
            icon: template.and_then(|(id, info)| crate::mission::marker_icon(templates, *id, info)),
            icon_flat: template.is_some_and(|(_, info)| info.marker_flat()),
            ..crate::flights::Leaves::default()
        },
        at: player.character.position + right * LEFT_BESIDE,
        yaw: player.facing,
    });
}

/// Parking: the first spot looked at is this far ahead of her; spots are
/// tried on a grid this fine, out to this far.
const PARK_AHEAD: f32 = 14.0;
const PARK_GRID: f32 = 4.0;
const PARK_SPOTS: i32 = 12;
/// Looks before parking gives up on the vehicles left (cramped ground: a
/// ledge, a town).
const PARK_TRIES: u32 = 40;
/// A spot will do when the ground under the wheels slopes less than this
/// (radians, about 10°; later looks take up to twice it), nothing under
/// the hull rises more than this above them (metres: a rock, a ledge),
/// and it is near the height of her feet (not a roof or a boulder's top):
/// within [`PARK_RISE`] metres, and [`PARK_GRADE`] more for each metre
/// away, up to [`PARK_ROOF`].
const PARK_SLOPE: f32 = 0.18;
const PARK_BUMPS: f32 = 0.3;
const PARK_RISE: f32 = 1.5;
const PARK_GRADE: f32 = 0.15;
const PARK_ROOF: f32 = 8.0;
/// The ground under a spot is looked for from this far above her feet.
const PARK_LOOK: f32 = 60.0;
/// A spot whose middle is less than this above the sea is in it, metres;
/// one whose middle is further than this beyond the rise allowed (see
/// [`PARK_RISE`]) isn't looked at more closely.
const PARK_SHORE: f32 = 0.5;
const PARK_SLACK: f32 = 2.0;
/// Vehicles are let go this far above where they rest, to settle onto
/// their wheels, metres.
const DROP: f32 = 0.3;
/// Room left between parked vehicles, metres.
const PARK_GAP: f32 = 1.5;

/// Parks vehicles that have loaded but have no body yet on flat, clear
/// ground near the spot ahead of the player, facing the way she faces.
fn park(physics: &mut Physics, vehicles: &mut [ParkedVehicle], player: &Player, tries: u32) {
    if !vehicles.iter().any(parkable) {
        return;
    }
    let (s, c) = player.facing.sin_cos();
    let forward = Vec3::new(s, 0.0, c);
    let right = Vec3::new(c, 0.0, -s);
    let feet = player.character.position;
    let ahead = feet + forward * PARK_AHEAD;
    // Spots nearest the one ahead first.
    // Later looks reach further and take steeper ground.
    let reach = PARK_SPOTS + (tries / 2) as i32;
    let steep = PARK_SLOPE * (1.0 + tries as f32 / 8.0).min(2.0);
    let mut spots: Vec<Vec3> = (-reach..=reach)
        .flat_map(|i| (-reach..=reach).map(move |j| (i, j)))
        .map(|(i, j)| ahead + right * (i as f32 * PARK_GRID) + forward * (j as f32 * PARK_GRID))
        .filter(|p| p.distance(feet) > 6.0)
        .collect();
    spots.sort_by(|a, b| a.distance(ahead).total_cmp(&b.distance(ahead)));
    // Footprints taken: centre and radius.
    let mut taken: Vec<(Vec3, f32)> = vehicles
        .iter()
        .filter_map(|v| {
            let at = physics.vehicle_transform(v.id?)?;
            let (lo, hi) = v.spec.bounds;
            Some((
                at.transform_point3((lo + hi) * 0.5),
                footprint_radius(&v.spec),
            ))
        })
        .collect();
    // The ground under each spot's middle, looked at once a look: a spot
    // in the sea, or far above or below her, is passed over before the
    // ground under each vehicle's wheels there is (a look on a coast or a
    // hillside otherwise casts a ray or two dozen for every spot).
    let mut middles: Vec<Option<Option<f32>>> = vec![None; spots.len()];
    for v in vehicles.iter_mut().filter(|v| parkable(v)) {
        let (lo, hi) = v.spec.bounds;
        let radius = footprint_radius(&v.spec);
        let middle = Vec3::new((lo.x + hi.x) * 0.5, 0.0, (lo.z + hi.z) * 0.5);
        // From high above, so a boulder or a cliff taller than her isn't
        // missed by a ray starting inside it (the spot is then too high).
        let ground = |p: Vec3| {
            let top = Vec3::new(p.x, feet.y + PARK_LOOK, p.z);
            physics
                .cast_ray(top, -Vec3::Y, PARK_LOOK + PARK_ROOF * 2.0)
                .map(|(d, _)| top.y - d)
        };
        let spot = spots.iter().zip(&mut middles).find_map(|(&centre, under)| {
            if taken
                .iter()
                .any(|&(c, r)| (c - centre).with_y(0.0).length() < r + radius + PARK_GAP)
            {
                return None;
            }
            let away = (centre - feet).with_y(0.0).length();
            let rise = (PARK_RISE + PARK_GRADE * away).min(PARK_ROOF);
            let h = (*under.get_or_insert_with(|| ground(centre)))?;
            if h < WATERLINE + PARK_SHORE || (h - feet.y).abs() > rise + PARK_SLACK {
                return None;
            }
            // Tilted to the ground under its wheels, clear of what is
            // under its hull.
            let origin = centre - (right * middle.x + forward * middle.z);
            let rest = v.spec.rest_on(origin, player.facing, ground)?;
            (rest.slope <= steep
                && rest.bumps <= PARK_BUMPS
                && (rest.position.y - feet.y).abs() < rise)
                .then_some((centre, rest))
        });
        // Nowhere yet (the ground may still be loading): try next tick.
        let Some((centre, rest)) = spot else { continue };
        let at = rest.position + Vec3::Y * DROP;
        v.id = Some(physics.add_vehicle_turned(&v.spec, at, rest.rotation));
        taken.push((centre, radius));
    }
}

/// What the fight's rounds, blows and blasts of the last step
/// ([`Combat::struck`], [`Combat::went_off`]) do to the world: blasts
/// knock loose what comes loose and wear down what they reach
/// ([`Destruction::blast`]); supply crates, delivered and placed, break
/// open (a puff of dust off their boards).
fn hit_world(
    world: &mut World,
    props: &mut Props,
    flights: &mut crate::flights::Flights,
    install: Option<&Install>,
    placed: &mut crate::placed::Placed,
    destruction: &mut Destruction,
    combat: &mut Combat,
) {
    // Rounds, blows and blasts wear down what they reach, each as the
    // damage table's row for it says (a café chair breaks to a clip; a
    // box or a wall takes nothing of a round); what still stands and
    // comes loose is knocked loose after. A round does nothing to what
    // has no row.
    for s in &combat.struck {
        if let Some(key) = s.key {
            destruction.wear(world, props, install, key, (s.round, s.kind, 0.0), s.at);
        }
    }
    let mut pushes = Vec::new();
    for at in combat.went_off.clone() {
        let blasts = rocket_blasts(&world.index.template_db, at);
        combat.last_push = blasts.first().map(|b| b.push);
        pushes.extend(combat.last_push);
        for blast in blasts {
            destruction.blast(world, props, install, blast);
        }
    }
    blast_loose(world, props, &pushes, false);
    blast_pickups(world, flights, &pushes);
    // What was worn down and blows up goes off at once, with the blast
    // its template leaves, and shows what its model's `CollapseState`
    // starts.
    for (at, template, shows) in destruction.set_off.drain(..) {
        combat.set_off(at, combat::left_blast(&world.index.template_db, template));
        for (effect, at) in shows {
            combat.game_fx.push(combat::GameFx {
                template: combat::FxTemplate::Hash(effect),
                at,
                hold: None,
            });
        }
    }
    let mut broken = flights.strike(&mut world.physics, &combat.struck, &combat.went_off);
    // The state layers' crates too ([`crate::placed`]).
    broken.extend(placed.strike(
        props,
        &mut world.physics,
        flights,
        &world.index.template_db,
        &combat.struck,
        &combat.went_off,
    ));
    for (_, at) in broken {
        combat.show(
            Effect::Smoke {
                at: at + Vec3::Y * 0.6,
                size: 1.4,
            },
            1.2,
        );
        combat.show_fx(crate::combat::fx::CRATE, at + Vec3::Y * 0.6);
    }
}

impl Game {
    /// What `struck` and `went_off` do to the world, as the fight's own
    /// rounds and blasts would ([`hit_world`]): the props check's hits
    /// ([`crate::breakables::check`]). The fight's own of the last step
    /// are dropped.
    pub fn hit_world_with(&mut self, struck: Vec<combat::Strike>, went_off: Vec<Vec3>) {
        let scene = &mut self.scene;
        let Some(world) = &mut scene.world else {
            return;
        };
        self.combat.struck = struck;
        self.combat.went_off = went_off;
        hit_world(
            world,
            &mut scene.props,
            &mut scene.flights,
            scene.install.as_ref(),
            &mut self.placed,
            &mut self.destruction,
            &mut self.combat,
        );
        self.combat.struck.clear();
        self.combat.went_off.clear();
    }

    /// The load bed of vehicle model `model` (its `HP_Truckbed` node),
    /// once looked up.
    pub fn bed(&self, model: u32) -> Option<Vec3> {
        self.beds.get(&model).copied().flatten()
    }

    /// What a vehicle like `crasher` going at its velocity does to what
    /// its box meets, as one driven would ([`run_into`]): the props
    /// check's crashes ([`crate::breakables::check`]).
    pub fn crash_with(&mut self, crasher: &Crasher) {
        let scene = &mut self.scene;
        let Some(world) = &mut scene.world else {
            return;
        };
        let keys = world.physics.statics_in_box(crasher.place, crasher.bounds);
        run_into(
            world,
            &mut scene.props,
            &mut self.destruction,
            scene.install.as_ref(),
            crasher,
            keys,
            (crasher.velocity.length(), "props check's vehicle", None),
        );
    }
}

/// A vehicle that may carry what stands on it: its index among the
/// scene's and its physics id, and its load bed in its space.
type Carrier = ((usize, m2_engine::VehicleId), Vec3);

/// A vehicle model's own collision, which its load bed's goods lie on.
type BedShape = Option<std::sync::Arc<m2_engine::CollisionShape>>;

/// The node of a vehicle model that is its load bed (the scripts count
/// the goods on it there: PirCon002's `GetHardpointPosition(truck,
/// "HP_Truckbed")`).
pub(crate) const BED_NODE: &str = "HP_Truckbed";

/// Vehicle model `model`'s own collision, its `PHY2` shapes (model space):
/// a pickup's bed floor and sides, for the goods on it to lie on.
fn bed_shape(
    source: &mut m2_engine::AssetSource<std::io::BufReader<std::fs::File>>,
    model: u32,
) -> Option<std::sync::Arc<m2_engine::CollisionShape>> {
    let data = source.model(model).ok()?;
    let cpu = m2_engine::CpuModel::from_data(&data, 0).ok()?;
    cpu.collision.map(std::sync::Arc::new)
}

/// The props within this of a scripts' vehicle's load bed are logged
/// once, metres.
const TOLD_REACH: f32 = 6.0;

/// Props up to this far under the bed node, or this far over it
/// (metres), count as on it, over the vehicle's footprint (invented:
/// PirCon002's rum jugs stand 0.1 m above its wheels' bottom, about
/// 1.5 m under the node, when the truck is put down).
const BED_UNDER: f32 = 2.0;
const BED_OVER: f32 = 1.0;

/// Whether prop `entity` stands on `vehicle`'s load bed, whose node is at
/// `bed` in its space: over its footprint, from [`BED_UNDER`] under the
/// node's height to [`BED_OVER`] over it.
fn on_bed(props: &Props, vehicle: &Crasher, bed: Vec3, entity: u32) -> bool {
    props
        .centre(entity)
        .is_some_and(|c| within_bed(vehicle.place, vehicle.bounds, bed, c))
}

/// Whether point `c` is over the footprint `(lo, hi)` of a vehicle placed
/// at `place`, from [`BED_UNDER`] under its bed node `bed` (its space) to
/// [`BED_OVER`] over it.
fn within_bed(place: Mat4, (lo, hi): (Vec3, Vec3), bed: Vec3, c: Vec3) -> bool {
    let l = place.inverse().transform_point3(c);
    l.x > lo.x
        && l.x < hi.x
        && l.z > lo.z
        && l.z < hi.z
        && l.y > bed.y - BED_UNDER
        && l.y < bed.y + BED_OVER
}

/// Vehicles knock loose what they hit that comes loose, with the weight
/// and speed for it ([`KNOCK_RATIO`]): it flies off ahead, and the vehicle
/// ploughs on slowed as by the mass it shifted, rather than stopped as by
/// a wall (the step just taken stopped it). First they break what they
/// run into that their weight breaks ([`Destruction::crash`]: plants,
/// bushes, fences, lampposts, the island's and the state layers'), going
/// on through it.
fn knock_loose(
    world: &mut World,
    props: &mut Props,
    vehicles: &[ParkedVehicle],
    (beds, shapes, told): (&[Option<Vec3>], &[BedShape], &mut HashSet<usize>),
    destruction: &mut Destruction,
    install: Option<&Install>,
) {
    for (index, (v, bed)) in vehicles
        .iter()
        .zip(beds.iter().chain(std::iter::repeat(&None)))
        .enumerate()
    {
        let Some(id) = v.id else { continue };
        let shape = shapes.get(index).and_then(|s| s.as_deref());
        // The scripts' vehicles with a load bed take up what stands on it
        // as soon as they are placed, standing or not: the scripts count
        // the goods on the bed from then on (PirCon002 counted none, and
        // gave up, while the truck stood with its jugs still the layer's).
        if let (Some(_), Some(b), Some(place)) = (v.guid, bed, world.physics.vehicle_transform(id))
        {
            let standing = Crasher {
                place,
                bounds: v.model.bounds,
                velocity: Vec3::ZERO,
                mass: v.spec.mass,
            };
            // What stands in its box with collision, and what has none
            // (PirCon004's organ boxes: none rode, and the truck left
            // them standing where it was parked).
            let (lo, hi) = v.model.bounds;
            let mut near: Vec<u32> = world
                .physics
                .statics_in_box(place, v.model.bounds)
                .into_iter()
                .filter_map(|k| match k {
                    StaticKey::Entity(e) => Some(e),
                    _ => None,
                })
                .collect();
            near.extend(props.standing_near(
                place.transform_point3((lo + hi) * 0.5),
                (hi - lo).length() * 0.5,
            ));
            near.sort_unstable();
            near.dedup();
            // Once the props round it have loaded: what stands round the
            // bed, and how, for the log.
            let bed_at = place.transform_point3(*b);
            if !told.contains(&index) {
                let round = props.told_near(bed_at, TOLD_REACH);
                if !round.is_empty()
                    && (round.iter().all(|t| !t.contains("(not "))
                        || round.iter().any(|t| t.starts_with("0x4")))
                {
                    told.insert(index);
                    eprintln!(
                        "cargo: by the {} (bed at {bed_at:.2}): {}",
                        v.spec.label,
                        round.join("; ")
                    );
                }
            }
            for e in near {
                if !props.is_cargo(e)
                    && on_bed(props, &standing, *b, e)
                    && props.load(&mut world.physics, e, (index, id), shape)
                {
                    eprintln!("cargo: {e:#x} rides on the {}", v.spec.label);
                }
            }
        }
        let before = world.physics.vehicle_velocity_before(id);
        let mut speed = before.length();
        if world.physics.vehicle_pulling(id) {
            speed = speed.max(PUSH_SPEED);
        }
        if speed < KNOCK_SPEED {
            continue;
        }
        let mass = v.spec.mass;
        let Some(place) = world.physics.vehicle_transform(id) else {
            continue;
        };
        let crasher = Crasher {
            place,
            bounds: v.model.bounds,
            velocity: before,
            mass,
        };
        let keys = world.physics.vehicle_statics(id);
        let (shifted, ploughed) = run_into(
            world,
            props,
            destruction,
            install,
            &crasher,
            keys,
            (speed, &v.spec.label, bed.map(|b| ((index, id), b))),
        );
        if shifted > 0.0 || ploughed {
            world
                .physics
                .set_vehicle_velocity(id, before * (mass / (mass + shifted)));
        }
    }
}

/// What `vehicle` does to what it runs into: what its body touched in
/// the step (`keys`) and what its box meets ([`Destruction::crash`]):
/// what its weight breaks comes down, and what comes loose it knocks
/// loose with the weight and `speed` for it ([`KNOCK_RATIO`]: as fast as
/// it pushes, pulling against what holds it). Returns the mass it
/// shifted (kg) and whether it ploughed on through something it broke.
/// `label` names it for the log; `carrier` is the vehicle (its index and
/// id) and its load bed, for what stands on it to ride ([`on_bed`]).
fn run_into(
    world: &mut World,
    props: &mut Props,
    destruction: &mut Destruction,
    install: Option<&Install>,
    vehicle: &Crasher,
    mut keys: Vec<StaticKey>,
    (speed, label, carrier): (f32, &str, Option<Carrier>),
) -> (f32, bool) {
    // A building of a merged batch touched is its own placed object (a
    // shanty town's shack), where it is placed by its own model too.
    for k in &mut keys {
        *k = world.as_object(*k);
    }
    let mut seen = std::collections::HashSet::new();
    keys.retain(|k| seen.insert(*k));
    let mass = vehicle.mass;
    let mut shifted = 0.0;
    let mut ploughed = false;
    let mut down = Vec::new();
    if let Some(install) = install {
        // What its body touched in the step counts as run into, as
        // what its box meets does.
        for c in destruction.crash(world, props, install, vehicle, &keys) {
            match c {
                Crashed::Down(key) => {
                    ploughed |= keys.contains(&key);
                    down.push(key);
                }
                Crashed::Loose(key) => {
                    if !keys.contains(&key) {
                        keys.push(key);
                    }
                }
            }
        }
    }
    // What broke is not knocked loose as well, nor what rides on a
    // vehicle ([`Props::load`]). What stands on this one's load bed (a
    // prop resting on top of it, inside its footprint: PirCon002's rum
    // jugs) rides on it, not knocked loose by it (inferred: the game's
    // goods ride a driven truck).
    keys.retain(|k| match *k {
        _ if down.contains(k) => false,
        StaticKey::Entity(e) if props.is_cargo(e) => false,
        StaticKey::Entity(e) => match carrier {
            Some((carrier, bed))
                if on_bed(props, vehicle, bed, e)
                    && props.load(&mut world.physics, e, carrier, None) =>
            {
                eprintln!("cargo: {e:#x} rides on the {label}");
                false
            }
            Some(_) => {
                if std::env::var_os("M2_CARGO_LOG").is_some() {
                    let l = props
                        .centre(e)
                        .map(|c| vehicle.place.inverse().transform_point3(c));
                    eprintln!(
                        "cargo: {e:#x} at {l:?} of the {label}, box {:?}",
                        vehicle.bounds
                    );
                }
                true
            }
            None => true,
        },
        _ => true,
    });
    for key in keys {
        let (loose, template) = match key {
            StaticKey::Object(i) => (world.loose_mass(i), world.index.objects[i].template),
            StaticKey::Entity(e) => (
                props.loose_mass(&world.physics, e),
                props.get(e).map_or(0, |(p, _)| p.template),
            ),
            _ => (None, 0),
        };
        // What its template says a vehicle this light doesn't break (a
        // wall a jeep runs into) stands, loose or not.
        let holds = world
            .index
            .template_db
            .crashable
            .get(&template)
            .is_some_and(|c| c.how == CrashHow::Breaks && mass < c.takes);
        let Some(m) = loose.filter(|&m| !holds && mass * speed >= KNOCK_RATIO * m) else {
            continue;
        };
        let (fling, topple) = flung(vehicle.velocity, mass, m);
        let knocked = match key {
            StaticKey::Object(i) => world.knock(i, fling, topple),
            StaticKey::Entity(e) => props.knock(&mut world.physics, e, fling, topple, false),
            _ => false,
        };
        if knocked {
            shifted += m;
            eprintln!("knocked loose: {key:?} ({m:.0} kg) by the {label} at {speed:.1} m/s");
        }
    }
    (shifted, ploughed)
}

/// How what a vehicle of `mass` (kg) going at `velocity` (m/s) knocks
/// loose, `shifted` kg of it, is flung: its velocity, a little faster than
/// the two would go on together and up ([`FLING`], [`FLING_UP`]), and its
/// spin, toppling the way it was hit ([`TOPPLE`]). What a vehicle breaks
/// apart goes the same way ([`Destruction::crash`]).
pub(crate) fn flung(velocity: Vec3, mass: f32, shifted: f32) -> (Vec3, Vec3) {
    let ahead = velocity.with_y(0.0);
    let fling = velocity * (mass / (mass + shifted)) * FLING + Vec3::Y * FLING_UP;
    let topple = Vec3::Y.cross(ahead.normalize_or_zero()) * (ahead.length() * TOPPLE);
    (fling, topple)
}

/// Blasts knock loose what comes loose within `reach` (as far as they
/// hurt people), thrown out and up as people are, harder the nearer; a
/// support `strike`'s the sturdy too.
/// The template database's blasts the alpha's own stand for: a rocket's,
/// a grenade's or a shell's (combat doesn't yet tell them apart, so the
/// first of these found), and the satellite bomb's and its frag's.
const ROCKET_BLASTS: [&str; 4] = [
    "Explosion (Handheld RPG)",
    "Explosion (RPG)",
    "Explosion (AT Rocket)",
    "Explosion (AT Missile)",
];
const BOMB_BLAST: &str = "Explosion (Airstike Bomb)";
const BOMB_FRAG: &str = "Explosion (Airstrike Frag)";

/// What the script of the blast `name` does to the light, if it does
/// ([`m2_game::atmosphere_script`]): seconds until, and the light.
fn blast_light(db: &Templates, name: &str) -> Option<(f32, ScriptAtmosphere)> {
    // `M2_BLAST_LIGHT=off` leaves the light alone (to measure against).
    if std::env::var("M2_BLAST_LIGHT").is_ok_and(|v| v == "off") {
        return None;
    }
    let bytes = db.blast_scripts.get(&db.id(name)?)?;
    let chunk = m2_assets::lua::Chunk::from_asset(bytes).ok()?;
    match m2_game::atmosphere_script::activation_light(&chunk) {
        Ok(light) => light,
        Err(e) => {
            eprintln!("atmosphere: {name}'s script did not run: {e}");
            None
        }
    }
}

/// The first of `names` the template database gives a blast.
fn explosion(db: &Templates, names: &[&str]) -> Option<Explosion> {
    names
        .iter()
        .find_map(|n| db.explosions.get(&db.id(n)?).copied())
}

/// An ordinary blast's reach and damage, heart to edge: a rocket's from
/// the template database, else combat's own reach and 100 to nothing.
pub(crate) fn blast_strength(db: &Templates) -> (f32, f32, f32) {
    explosion(db, &ROCKET_BLASTS).map_or((combat::REACH, 100.0, 0.0), |e| {
        (e.radius, e.damage, e.least)
    })
}

/// The RPG's frag and its rocket (`Explosion (Handheld RPG)` links
/// `Explosion (Handheld RPG Frag)` as its `Frag`; the `RPG Rocket` is
/// the projectile, PC data).
const ROCKET_FRAG: &str = "Explosion (Handheld RPG Frag)";
const ROCKET: &str = "RPG Rocket";
/// How near a rocket's heart what it lands on is, metres (invented): its
/// own `Damage` is done to what it strikes, not around.
const ROCKET_HIT: f32 = 0.5;

/// What an ordinary blast (a rocket's: combat doesn't yet tell them
/// apart) does to what stands around it, as the data has an RPG's: its
/// explosion (`Explosion`, 200 to nothing over 5 m), its frag (`Bullet`,
/// 300 to 40 over 15 m) and the rocket itself on what it lands on
/// (`Rocket`, 100, within [`ROCKET_HIT`]), each at `at`. Those the data
/// doesn't give are left out; with none, combat's own reach and 100 to
/// nothing.
pub(crate) fn rocket_blasts(db: &Templates, at: Vec3) -> Vec<Blast> {
    let mut out: Vec<Blast> = [explosion(db, &ROCKET_BLASTS), explosion(db, &[ROCKET_FRAG])]
        .into_iter()
        .flatten()
        .map(|e| of_explosion(e, at))
        .collect();
    if let Some(p) = db.id(ROCKET).and_then(|id| db.projectiles.get(&id)) {
        // The rocket itself pushes nothing: a projectile, no `Explosive`.
        out.push(Blast {
            at,
            radius: ROCKET_HIT,
            damage: p.damage,
            least: p.damage,
            kind: p.kind,
            push: BlastPush::none(at),
        });
    }
    if out.is_empty() {
        let (radius, damage, least) = blast_strength(db);
        out.push(Blast {
            at,
            radius,
            damage,
            least,
            kind: 0,
            push: BlastPush {
                at,
                radius,
                force: EXPLOSIVE_FORCE,
            },
        });
    }
    crate::destruct::together(out)
}

/// The air strike bomb's blasts going off at `at`: its explosion and its
/// frag, as the data has them (combat's own where it hasn't).
pub(crate) fn bomb_blasts(db: &Templates, at: Vec3) -> Vec<Blast> {
    let blasts = [(BOMB_BLAST, combat::BOMB), (BOMB_FRAG, combat::BOMB_FRAG)].map(
        |(name, (radius, damage))| match explosion(db, &[name]) {
            Some(e) => of_explosion(e, at),
            None => Blast {
                at,
                radius,
                damage,
                least: 0.0,
                kind: 0,
                push: BlastPush {
                    at,
                    radius,
                    force: EXPLOSIVE_FORCE,
                },
            },
        },
    );
    crate::destruct::together(blasts.to_vec())
}

/// The `MaxForce` an `Explosive` has when its template gives none (the
/// schema's default, 0x823EF9F0), for a blast not found in the data.
const EXPLOSIVE_FORCE: f32 = 500.0;

/// Explosion `e` going off at `at` as a blast that wears things down and
/// pushes what it breaks.
fn of_explosion(e: Explosion, at: Vec3) -> Blast {
    Blast {
        at,
        radius: e.radius,
        damage: e.damage,
        least: e.least,
        kind: e.kind,
        push: BlastPush {
            at,
            radius: e.radius,
            force: e.force,
        },
    }
}

/// Whether an ordinary blast throws loose what template `t`'s objects
/// are: a light physics prop (`_PropPhysics`, not one of the heavy ones:
/// a chair, a box, a barricade), or what the damage table's row for it
/// takes something of a blast's explosion or frag (`Explosion`,
/// `Bullet`: a lamp, a fence). A wall (`Building`) or a shipping
/// container (`Tank`, 20,000 kg) takes nothing of either and stands, its
/// breakable parts with it; one with no row is thrown as before.
fn blown_loose(db: &Templates, t: u32) -> bool {
    if let Some(p) = db.prop_physics.get(&t) {
        return !p.heavy;
    }
    let (Some(&row), Some(table)) = (db.modifiers.get(&t), db.damage.as_ref()) else {
        return true;
    };
    ["Explosion", "Bullet"].iter().any(|k| {
        table
            .scale(row, m2_assets::name_hash(k))
            .is_some_and(|s| s > 0.0)
    })
}

/// What a pickup of template `t` weighs, if it is a physics prop
/// (`_PropPhysics`): kilograms and its gravity scale.
pub(crate) fn pickup_weight(db: &m2_engine::Templates, t: u32) -> Option<(f32, f32)> {
    let p = db.prop_physics.get(&t)?;
    Some((p.kg(), db.gravity_scales.get(&t).copied().unwrap_or(1.0)))
}

/// Vehicles on the move throw the physics props lying to be taken that
/// they run into, as they knock props loose ([`flung`]).
fn run_over_pickups(
    world: &mut World,
    flights: &mut crate::flights::Flights,
    vehicles: &[ParkedVehicle],
) {
    for v in vehicles {
        let Some(id) = v.id else { continue };
        let Some(place) = world.physics.vehicle_transform(id) else {
            continue;
        };
        let velocity = world.physics.vehicle_velocity(id);
        let db = &world.index.template_db;
        let threw = flights.run_over(
            &mut world.physics,
            (place, v.model.bounds, velocity, v.spec.mass),
            |t| pickup_weight(db, t),
            flung,
        );
        if threw > 0 {
            eprintln!("pickups: the {} throws {threw}", v.spec.label);
        }
    }
}

/// Blasts throw the physics props lying to be taken (the weapons and
/// grenades soldiers leave) within each one's reach, as it pushes them
/// ([`BlastPush::throw`], each taken as a point: none is anywhere near
/// the 2 m³ the push is full up to).
fn blast_pickups(world: &mut World, flights: &mut crate::flights::Flights, pushes: &[BlastPush]) {
    let db = &world.index.template_db;
    for push in pushes {
        let threw = flights.blow(
            &mut world.physics,
            &[push.at],
            push.radius,
            |t| pickup_weight(db, t),
            |_, at, _| push.throw(at, at, 1.0),
        );
        if threw > 0 {
            eprintln!("pickups: a blast throws {threw}");
        }
    }
}

/// What `pushes` knock loose within each one's reach (what comes loose:
/// a lamp, a fence, a chair) flies as the blast pushes it, by its own
/// box and its template's `ExplosionFudge` ([`BlastPush::throw`], the 360
/// code's push on what moves, 0x8215CBC8). An ordinary blast knocks loose
/// only what [`blown_loose`] says; a `strike` (an air strike's) all of it.
fn blast_loose(world: &mut World, props: &mut Props, pushes: &[BlastPush], strike: bool) {
    let fudge = |db: &Templates, t: u32| db.explosion_fudges.get(&t).copied().unwrap_or(1.0);
    for push in pushes {
        for (i, _) in world.loose_near(push.at, push.radius) {
            let o = world.index.objects[i];
            let db = &world.index.template_db;
            if !strike && !blown_loose(db, o.template) {
                continue;
            }
            let placed = Mat4::from_rotation_translation(o.rotation, o.position);
            let (lo, hi) = world.object_model(i).map_or((o.position, o.position), |m| {
                crate::props::box_around(placed, m.bounds)
            });
            let (velocity, spin) = push.throw(lo, hi, fudge(db, o.template));
            world.knock(i, velocity, spin);
        }
        for (e, _) in props.loose_near(&world.physics, push.at, push.radius, strike) {
            let Some((p, template)) = props.get(e).map(|(p, _)| (p.position, p.template)) else {
                continue;
            };
            let db = &world.index.template_db;
            if !strike && !blown_loose(db, template) {
                continue;
            }
            let (lo, hi) = props
                .placed_bounds(e)
                .map_or((p, p), |(at, bounds)| crate::props::box_around(at, bounds));
            let (velocity, spin) = push.throw(lo, hi, fudge(db, template));
            props.knock(&mut world.physics, e, velocity, spin, strike);
        }
    }
}

/// What the island's explosives' models start on entering the states the
/// fight put them in ([`Combat::explosive_states`]): each effect their
/// part states start there (`StartEmitter`), on its node, or at the
/// object for a locator the model doesn't carry (inferred), each running
/// as long as its template says (the fuel tanks' fire jets 10 s, their
/// wreck's smoke 100 s). Asked of the particles as the fight's own are.
fn state_effects(world: &World, combat: &mut Combat) {
    for (i, state) in std::mem::take(&mut combat.explosive_states) {
        let (Some(o), Some(model)) = (world.index.objects.get(i), world.object_model(i)) else {
            continue;
        };
        let placed = Mat4::from_rotation_translation(o.rotation, o.position);
        for fx in model.state_fx.iter().filter(|fx| fx.state == state) {
            combat.game_fx.push(combat::GameFx {
                template: combat::FxTemplate::Hash(fx.effect),
                at: fx.at.map_or(o.position, |at| placed.transform_point3(at)),
                hold: None,
            });
        }
    }
}

/// Keeps each crewed vehicle's exhaust effects going at their nodes
/// (`m2_engine::vehicle_parts`; the heat haze is drawn with the vehicle),
/// however far off: from the 360 code, 0x822772B0 and 0x822766F0 look at
/// no distance.
fn exhaust_smoke(
    world: &World,
    vehicles: &[ParkedVehicle],
    smokes: &HashMap<(u32, String), Vec<(usize, u32)>>,
    player: Option<&crate::player::Player>,
    combat: &mut Combat,
) {
    let Some(player) = player else {
        return;
    };
    for (i, v) in vehicles.iter().enumerate() {
        if !v.exhausting(player.driving == Some(i)) {
            continue;
        }
        let key = (v.spec.model, v.spec.label.clone());
        let Some(parts) = smokes.get(&key).filter(|p| !p.is_empty()) else {
            continue;
        };
        let Some(at) = v.id.and_then(|id| world.physics.vehicle_transform(id)) else {
            continue;
        };
        for (k, &(node, effect)) in parts.iter().enumerate() {
            let Some(rest) = v.spec.rest.get(node) else {
                continue;
            };
            combat.game_fx.push(combat::GameFx {
                template: combat::FxTemplate::Hash(effect),
                at: (at * *rest).w_axis.truncate(),
                hold: Some(0xe000_0000_0000_0000 | (i as u64) << 8 | k as u64),
            });
        }
    }
}

/// A vehicle of ours (not the mission's or the island's) still to park.
fn parkable(v: &ParkedVehicle) -> bool {
    v.id.is_none() && v.guid.is_none() && v.spot.is_none()
}

/// Spray off both sides of each boat going fast on the water, a third of
/// the way up from its stern, and churned water behind it.
fn spray(world: &World, vehicles: &[ParkedVehicle], combat: &mut Combat) {
    let Some(water) = world.physics.water() else {
        return;
    };
    for v in vehicles.iter().filter(|v| v.spec.is_boat() && !v.wrecked) {
        let Some(id) = v.id else { continue };
        let Some(m) = world.physics.vehicle_transform(id) else {
            continue;
        };
        let speed = world.physics.vehicle_speed(id);
        if speed < SPRAY_SPEED || (m.w_axis.y - water).abs() > 1.5 {
            continue;
        }
        let k = ((speed - SPRAY_SPEED) / (SPRAY_FULL - SPRAY_SPEED)).clamp(0.0, 1.0);
        let (lo, hi) = v.spec.hull;
        let back = m
            .transform_vector3(Vec3::NEG_Z)
            .with_y(0.0)
            .normalize_or_zero();
        for x in [lo.x, hi.x] {
            let out = m
                .transform_vector3(Vec3::X * x.signum())
                .with_y(0.0)
                .normalize_or_zero();
            let at = m.transform_point3(Vec3::new(x, 0.0, lo.z + (hi.z - lo.z) * 0.3));
            combat.show(
                Effect::Spray {
                    at: at.with_y(water + 0.2),
                    drift: out * (2.0 + 4.0 * k)
                        + back * (1.0 + 2.0 * k)
                        + Vec3::Y * (1.0 + 2.0 * k),
                    size: 0.6 + k,
                },
                SPRAY_LIFE,
            );
        }
        let stern = m.transform_point3(Vec3::new(0.0, 0.0, lo.z));
        combat.show(
            Effect::Wake {
                at: stern.with_y(water + 0.03),
                size: (hi.x - lo.x) * (0.6 + 0.6 * k),
            },
            WAKE_LIFE,
        );
    }
}

/// Puts a vehicle on the ground at `at` facing `yaw`, tilted to the
/// ground under its wheels; false while the ground there hasn't loaded.
/// A boat on the sea floats on it.
pub(crate) fn place(physics: &mut Physics, v: &mut ParkedVehicle, at: Vec3, yaw: f32) -> bool {
    // From a little above where the layers put it (so one under a bridge
    // or a gantry stays under it), else from well above.
    let ground = |p: Vec3| {
        [6.0, 30.0].into_iter().find_map(|up| {
            let top = Vec3::new(p.x, at.y + up, p.z);
            physics
                .cast_ray(top, Vec3::NEG_Y, up + 50.0)
                .map(|(d, _)| top.y - d)
        })
    };
    // Level, as deep as it floats, where the seabed is under its keel (or
    // hasn't loaded).
    let float = physics.water().filter(|&water| {
        let keel = water - float_depth(&v.spec) + v.spec.hull.0.y;
        v.spec.is_boat() && ground(at).is_none_or(|h| h < keel)
    });
    if let Some(water) = float {
        let at = Vec3::new(at.x, water - float_depth(&v.spec), at.z);
        v.id = Some(physics.add_vehicle_turned(&v.spec, at, Quat::from_rotation_y(yaw)));
        return true;
    }
    // (Never where it came out NaN: round 88 lost the start boat, and her
    // with it, to a rest fitted to a boat's no wheels.)
    let rest = v
        .spec
        .rest_on(at, yaw, ground)
        .filter(|r| r.position.is_finite() && r.rotation.is_finite());
    // A wheel over nothing (a quay's edge): level, on the ground under
    // its origin.
    let (at, turn) = match rest {
        Some(r) => (r.position, r.rotation),
        None => {
            let Some(h) = ground(at) else {
                return false;
            };
            (
                Vec3::new(at.x, h + v.spec.ride_height(), at.z),
                Quat::from_rotation_y(yaw),
            )
        }
    };
    v.id = Some(physics.add_vehicle_turned(&v.spec, at + Vec3::Y * DROP, turn));
    true
}

/// The game's own parked vehicles within this many metres of the player
/// are loaded and placed where the layers put them, nearest first, up to
/// [`WORLD_VEHICLES`] at a time; ones she drives away stay.
const VEHICLE_REACH: f32 = 400.0;
const WORLD_VEHICLES: usize = 12;
/// Seconds between looks for vehicles to load.
const VEHICLE_LOOK: f32 = 1.0;

/// Puts vehicles from a save back where they were, as their models load.
fn restore(physics: &mut Physics, vehicles: &mut [ParkedVehicle], saved: &mut Vec<VehicleSave>) {
    for v in vehicles.iter_mut().filter(|v| parkable(v)) {
        let Some(k) = saved.iter().position(|s| s.model == v.spec.model) else {
            continue;
        };
        let s = saved.swap_remove(k);
        // A little high, to settle onto its wheels.
        let at = s.position() + Vec3::Y * 0.3;
        let id = physics.add_vehicle(&v.spec, at, s.yaw);
        v.id = Some(id);
        v.wrecked = s.wrecked;
        if v.wrecked {
            physics.wreck_vehicle(id);
        }
    }
}

impl Game {
    /// What her binoculars' reticle is on: the nearest vehicle or person
    /// along it, not behind the ground or a building
    /// ([`crate::binoculars`]).
    pub fn binocular_focus(&self) -> Option<crate::binoculars::Focus> {
        use crate::binoculars::{Focus, along_ray, faction_texture, marker_texture};
        let world = self.scene.world.as_ref()?;
        let db = &world.index.template_db;
        let camera = self.walking()?.view(60.0, 16.0 / 9.0).view.inverse();
        let (eye, ahead) = (
            camera.w_axis.truncate(),
            -camera.z_axis.truncate().normalize_or_zero(),
        );
        let of_template = |id: Option<u32>| {
            let id = id?;
            let texture = db.faction_markers.get(&id).copied();
            Some(Focus {
                texture: texture.and_then(marker_texture),
                name: db.localized.get(&id).copied(),
            })
        };
        let vehicles = self.scene.vehicles.iter().filter_map(|v| {
            let m = world.physics.vehicle_transform(v.id?)?;
            let (lo, hi) = v.spec.bounds;
            let t = along_ray(eye, ahead, m.transform_point3((lo + hi) * 0.5))?;
            let names = db.names_for(&v.spec.label, v.spec.model);
            let id = names.first().and_then(|n| db.id(n));
            Some((t, of_template(id)?))
        });
        let people = self.scene.people.npcs.iter().filter_map(|n| {
            let t = along_ray(eye, ahead, n.character.position + Vec3::Y)?;
            let kind = self.scene.people.kinds.get(n.kind)?;
            let own = kind.named.then(|| db.id(&kind.role)).flatten();
            let focus = of_template(own).unwrap_or(Focus {
                texture: n.faction.and_then(faction_texture),
                name: None,
            });
            Some((t, focus))
        });
        let (t, focus) = vehicles.chain(people).min_by(|a, b| a.0.total_cmp(&b.0))?;
        // Behind something, more than its own body's length before it:
        // nothing's shown.
        let hit = world.physics.cast_ray(eye, ahead, t);
        hit.is_none_or(|(d, _)| d + 6.0 >= t).then_some(focus)
    }
}

/// Half the diagonal of a vehicle's footprint.
fn footprint_radius(spec: &m2_engine::VehicleSpec) -> f32 {
    let (lo, hi) = spec.bounds;
    (hi.x - lo.x).hypot(hi.z - lo.z) * 0.5
}

/// What the use prompt calls a vehicle: the name the game shows for it,
/// the English string its template's `LocalizedName` keys in `strings`
/// (the `Chopper`'s reads "Chopper"), of the template its label names,
/// else of the first sharing its model that the strings hold
/// ([`Templates::localized_keys`]);
/// without one, its template's name as the template database writes it
/// (`M151 Softtop (V2)`); its label (a texture name, `vz_veh_truck_M151`)
/// only if no template is found. Each label's name is logged once.
fn vehicle_name(
    db: &Templates,
    strings: Option<&m2_assets::StringTable>,
    label: &str,
    model: u32,
) -> String {
    static NAMED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let keys = db.localized_keys(label, model);
    let found = keys
        .iter()
        .find_map(|&k| Some((k, strings?.strings.get(&k)?.clone())));
    let key = found.as_ref().map(|f| f.0).or(keys.first().copied());
    let shown = found.map(|f| f.1);
    let name = db
        .names_for(label, model)
        .first()
        .and_then(|n| db.names.get(&db.id(n)?))
        .cloned();
    // Logged once its strings are read, if they ever are.
    if strings.is_some()
        && let Ok(mut named) = NAMED.lock()
        && !named.iter().any(|l| l == label)
    {
        named.push(label.to_owned());
        eprintln!(
            "vehicle: {label} (model {model:08x}) is shown as {shown:?} (string {key:08x?}), \
             its template {name:?}, to get in"
        );
    }
    shown.or(name).unwrap_or_else(|| label.to_owned())
}

/// The game's words for the use key at what lies within reach, by their
/// string table keys (`English.wad`; PC data, 2026-10-10): "Take Ammo",
/// "Equip %s", "Use %s".
const TAKE_AMMO: u32 = 0xbd07_d121;
const EQUIP: u32 = 0xdb58_ae4b;
const USE: u32 = 0x93e3_1c00;

/// The name the game shows for template `t` (or, without one, for `or`):
/// the English string its `LocalizedName` keys; else its template's name.
fn shown_name(
    db: &Templates,
    strings: Option<&m2_assets::StringTable>,
    t: u32,
    or: Option<u32>,
) -> String {
    let localized = |t: u32| strings?.strings.get(db.localized.get(&t)?).cloned();
    localized(t)
        .or_else(|| or.and_then(localized))
        .or_else(|| db.names.get(&t).cloned())
        .unwrap_or_else(|| format!("{t:#x}"))
}

/// Once she has landed after her floor was taken down: the layers' models
/// near her and whether each stands with collision, to see what (if
/// anything) came in its place.
fn log_floor(props: &Props, physics: &Physics, at: Vec3) {
    let mut near: Vec<_> = props
        .shown()
        .filter(|(_, p)| (p.position - at).with_y(0.0).length() < 40.0)
        .map(|(e, p)| {
            let loaded = props.get(e).is_some_and(|(_, l)| l);
            format!(
                "{e:#x} model {:08x} at y {:.1} layer {} loaded {loaded} collision {}{}",
                p.model,
                p.position.y,
                props.layer(e).unwrap_or("-"),
                physics.has_static(m2_engine::physics::StaticKey::Entity(e)),
                if p.wrecked { " wrecked" } else { "" }
            )
        })
        .collect();
    near.sort();
    eprintln!(
        "player: landed at {:.1?} after her floor went; {} layer models within 40 m:",
        at.to_array(),
        near.len()
    );
    for n in near {
        eprintln!("  {n}");
    }
}

/// Whether a vehicle of `kind` named `label` whose bank holds no horn is
/// heard with the lorry's (tracked ones, lorries, buses, the APCs), not
/// the car's (invented: which spare horn the game gives such a vehicle,
/// if any, is not in the data).
pub fn heavy_horn(kind: &str, label: &str, tracked: bool) -> bool {
    let key = format!("{kind} {label}").to_lowercase();
    tracked
        || [
            "truck", "lorry", "bus", "semi", "apc", "lav", "stryker", "tank",
        ]
        .iter()
        .any(|w| key.contains(w))
}

/// Whether someone sits in the driver's seat of a crew.
/// Whether getting into a vehicle whose template's physics is `kind`
/// earns her vehicle remark ([`combat::Remark::Vehicle`]): her lines for
/// it are about taking someone's car (`jennifer_cues`), so only a car she
/// got into herself. Not a boat, tank or helicopter, nor a seat the
/// scripts put her in (playtest 8: seated in the start's speedboat, she
/// called it a nice car).
pub fn car_remark(kind: Option<m2_engine::world::VehicleKind>, scripts_seated: bool) -> bool {
    !scripts_seated && kind == Some(m2_engine::world::VehicleKind::Car)
}

impl Game {
    /// Whether her getting into the vehicle she now drives earns her
    /// vehicle remark ([`car_remark`]).
    pub fn car_remark_due(&self) -> bool {
        let kind = self
            .scene
            .player
            .as_ref()
            .and_then(|p| p.driving)
            .and_then(|i| self.scene.vehicles.get(i))
            .zip(self.scene.world.as_ref())
            .and_then(|(v, w)| w.index.template_db.get(&v.spec.label)?.vehicle);
        car_remark(kind, self.scripts_seated)
    }
}

fn has_driver(crew: &[(String, String)]) -> bool {
    crew.iter()
        .any(|(seat, _)| seat.to_ascii_lowercase().contains("driver"))
}

/// She gets into `v`'s driver's seat: whoever drove it is put out (its
/// crew forgets them: playtest 4, no car could be taken from traffic).
/// The templates of those put out.
fn take_seat(v: &mut ParkedVehicle) -> Vec<String> {
    let out = crate::ride::empty_seats(&mut v.crew, &[None]);
    if !out.is_empty() {
        eprintln!("hijack: she takes the {} from its driver", v.spec.label);
    }
    out
}

/// Those put out of vehicle `v`'s (`vehicles[i]`) seats (`out`, by
/// template and its own model) stand by its door, in the looks they had
/// seated, and run from her (at `her`): in playtest 6 the driver she took
/// a car from was gone.
fn put_out(
    people: &mut crate::npc::People,
    riders: &[crate::hijack::Rider],
    v: &ParkedVehicle,
    physics: &Physics,
    i: usize,
    out: Vec<(String, Option<u32>)>,
    her: Vec3,
) {
    let Some(at) =
        v.id.and_then(|id| physics.vehicle_transform(id))
            .filter(|at| at.is_finite())
    else {
        return;
    };
    let door = door_out(at, &v.spec) + Vec3::Y * 0.5;
    for (who, model) in out {
        let kind = people.rider_kind(&who, model);
        let outfit = riders
            .iter()
            .find(|r| r.prop.is_none() && r.vehicle == i && Some(r.kind) == kind)
            .map(|r| r.outfit.clone());
        let stood = people.put_out(&who, model, outfit, door, her);
        eprintln!(
            "hijack: {who} put out at {:.1?}: {}",
            door.to_array(),
            if stood {
                "stands there"
            } else {
                "their kind isn't in: not drawn"
            }
        );
    }
}

/// Whether a vehicle of catalogue kind `kind` has a seat to get into: not
/// a trailer (ChiCon003's `civ_veh_trailer_flatbed`, kind `trailer`, was
/// got into for the M2A3 it carries, and "driven" stuck for 3,200 s).
/// Invented: the trailer's model has no seat the alpha reads.
fn can_sit_in(kind: &str) -> bool {
    kind != "trailer"
}

/// Where [`entrance_pick`] numbers the ladder ends it picks from.
const LADDER: usize = usize::MAX / 2;
/// The number [`entrance_pick`] gives an emplaced gun's entrance.
const EMPLACED: usize = usize::MAX / 4;

/// The vehicle she gets into at a press of the use key, and the entrance
/// she gets in by (`None` for a vehicle without entrance data), as the 360
/// does ([`m2_engine::seats`]): of the entrances in front of the camera
/// within their `UseDistance` + 1 m of her, the nearest, sticking to
/// `last`, the vehicle she last got into. A vehicle without entrance
/// data is reached at its door marker (or hull) within [`REACH`]
/// (Invented stand-in, the beta's rule).
///
/// The ladder ends gathered (`ladders`) are in the same list after the
/// vehicles' entrances (0x82440F30 gathers both): one picked is
/// [`LADDER`] + its index.
fn entrance_pick(
    physics: &Physics,
    vehicles: &[ParkedVehicle],
    rules: (&HashSet<u32>, &HashSet<u32>),
    player: &Player,
    last: Option<usize>,
    ladders: &[crate::ladders::Reach],
    gun: Option<m2_engine::seats::Candidate>,
) -> Option<(usize, Option<u32>)> {
    use m2_engine::seats::{self, Candidate};
    let her = player.character.position;
    let (eye, aim) = (player.eye(), player.camera_forward());
    let mut found: Vec<(Candidate, Option<u32>)> = Vec::new();
    for (i, v) in usable(vehicles, rules) {
        let Some(at) = v.id.and_then(|id| physics.vehicle_transform(id)) else {
            continue;
        };
        let upright = at.transform_vector3(Vec3::Y).normalize_or_zero().y >= seats::UPRIGHT_MIN_Y;
        if v.spec.entrances.is_empty() {
            let d = reach_of(at, &v.spec, her);
            if d <= REACH {
                let c = Candidate {
                    vehicle: i,
                    distance_sq: d * d,
                    use_distance: REACH,
                    upright,
                    offered: true,
                };
                found.push((c, None));
            }
            continue;
        }
        for &(e, local) in &v.spec.entrances {
            let place = at.transform_point3(local);
            let p = e.params;
            // Swimming, a boat's long-search entrances from afar.
            let pad = seats::search_pad(&p, player.swimming(), afloat(physics, v, at));
            if seats::in_search(her, eye, aim, place, p.use_distance, pad) {
                let c = Candidate {
                    vehicle: i,
                    distance_sq: her.distance_squared(place),
                    use_distance: p.use_distance,
                    upright,
                    // The scripts' `Vehicle.Usable` stays the alpha's rule
                    // ([`usable`]); ladders' seat paths are not checked.
                    offered: seats::offered(&p, true),
                };
                found.push((c, Some(e.name)));
            }
        }
    }
    for (j, r) in ladders.iter().enumerate() {
        let c = Candidate {
            vehicle: LADDER + j,
            distance_sq: r.distance_sq,
            use_distance: r.use_distance,
            upright: true,
            offered: true,
        };
        found.push((c, None));
    }
    // An emplaced gun's entrance, its vehicle [`EMPLACED`].
    found.extend(gun.map(|c| (c, None)));
    found.truncate(seats::LIST_CAP);
    let cands: Vec<Candidate> = found.iter().map(|(c, _)| *c).collect();
    let k = seats::pick(&cands, last)?;
    Some((found[k].0.vehicle, found[k].1))
}

/// The markers over the entrances near her, standing at `her` on foot
/// ([`crate::entrance_marker`]): of each vehicle she may get into within
/// [`NEAR_VEHICLE`](crate::entrance_marker::NEAR_VEHICLE), each entrance
/// offered for plain entry within the markers' reach that gives her a
/// seat ([`crate::ride::board`]'s first seat, before any slide over: the
/// 360 marks the seat 0x82299D68 gives).
fn entrance_markers(
    physics: &Physics,
    db: &Templates,
    scene: &Scene,
    rules: (&HashSet<u32>, &HashSet<u32>),
    her: Vec3,
) -> Vec<crate::entrance_marker::EntranceMarker> {
    use crate::entrance_marker::{EntranceMarker, LIFT, NEAR_VEHICLE, REACH};
    let mut out = Vec::new();
    for (_, v) in usable(&scene.vehicles, rules) {
        let Some(at) = v.id.and_then(|id| physics.vehicle_transform(id)) else {
            continue;
        };
        if at.w_axis.truncate().distance_squared(her) >= NEAR_VEHICLE * NEAR_VEHICLE {
            continue;
        }
        let labels = db.vehicle_labels_for(&v.spec.label, v.spec.model);
        for &(e, local) in &v.spec.entrances {
            let place = at.transform_point3(local);
            if e.params.use_distance <= 0.0
                || !m2_engine::seats::offered(&e.params, true)
                || place.distance_squared(her) >= REACH * REACH
            {
                continue;
            }
            let Some(b) = crate::ride::board(
                crate::ride::Ride::of(v),
                Some(e.name),
                crate::ride::car_or_truck(&labels),
                &scene.people,
            ) else {
                continue;
            };
            // The seat the entrance gives, not the one she slides to.
            let seat = b.slid_from.or(b.seat);
            out.push(EntranceMarker {
                at: place + Vec3::Y * LIFT,
                role: crate::ride::Ride::of(v).role(seat),
            });
        }
    }
    out
}

/// Logs her getting into `v` by `entrance`: how far she stood from it and
/// the seat [`crate::ride::board`] gave her (`None`: no way in).
fn log_entry(
    physics: &Physics,
    v: &ParkedVehicle,
    player: &Player,
    entrance: Option<u32>,
    boarding: Option<&crate::ride::Boarding>,
) {
    let Some(at) = v.id.and_then(|id| physics.vehicle_transform(id)) else {
        return;
    };
    match boarding {
        Some(b) => eprintln!(
            "seat: in the {}: {:x?} ({:?}){}{}",
            v.spec.label,
            b.seat,
            b.role,
            b.slid_from
                .map_or(String::new(), |f| format!(", slid over from {f:08x}")),
            b.stayed.map_or(String::new(), |w| format!(", stays: {w}"))
        ),
        None => eprintln!("seat: the {}'s entrance gives her no seat", v.spec.label),
    }
    let her = at.inverse().transform_point3(player.character.position);
    let Some(e) = entrance else {
        eprintln!(
            "enter: {} by its door marker (no entrance data), her local {:.2?}",
            v.spec.label,
            her.to_array()
        );
        return;
    };
    let place = v.spec.entrance_at(e).unwrap_or(Vec3::ZERO);
    let ud = v
        .spec
        .entrances
        .iter()
        .find(|(x, _)| x.name == e)
        .map_or(0.0, |(x, _)| x.params.use_distance);
    eprintln!(
        "enter: {} by entrance {e:#010x} at local {:.2?} (UseDistance {ud}), her local {:.2?}, \
         {:.2} m",
        v.spec.label,
        place.to_array(),
        her.to_array(),
        her.distance(place),
    );
}

/// Her distance from vehicle `spec` standing at `at`: from its door marker,
/// else its hull.
fn reach_of(at: Mat4, spec: &VehicleSpec, her: Vec3) -> f32 {
    let local = at.inverse() * her.extend(1.0);
    let (lo, hi) = spec.bounds;
    match spec.door {
        Some(door) => (local.xyz() - door).length(),
        None => (local.xyz().clamp(lo, hi) - local.xyz()).length(),
    }
}

/// The vehicles she may get into, with their indices: not wrecked, with a
/// seat, and not kept by the scripts or an action hijack while crewed.
fn usable<'a>(
    vehicles: &'a [ParkedVehicle],
    (hijack_only, unusable): (&'a HashSet<u32>, &'a HashSet<u32>),
) -> impl Iterator<Item = (usize, &'a ParkedVehicle)> + 'a {
    vehicles
        .iter()
        .enumerate()
        .filter(|(_, v)| !v.wrecked && can_sit_in(&v.spec.kind))
        .filter(|(_, v)| !v.wrecked && can_sit_in(&v.spec.kind))
        // Crewed, one with an action hijack is taken only by it (playtest
        // 5: the villa's tank, whose scripts make it usable by the hijack
        // alone, `Vehicle.Usable(tank, false)`, was got into at a press).
        .filter(|(_, v)| !(hijack_only.contains(&v.spec.model) && has_driver(&v.crew)))
        // Kept by the scripts for themselves (`Vehicle.Usable(v, false)`)
        // while crewed: an empty one is still got into (ChiCon009's ZBD2000
        // and ChiCon008's race tank, the only `Usable(false)` vehicles in
        // the runs, are both made so and then to be got into; the villa's
        // crewed tank is the hijack's alone). From those samples; the 360
        // code not checked.
        .filter(|(_, v)| !(v.guid.is_some_and(|g| unusable.contains(&g)) && has_driver(&v.crew)))
}

/// The vehicle nearest her within `reach` metres of its door marker (or its
/// hull, for those without one): `--drive`'s pick, at any distance. One
/// with someone in the driver's seat is taken from them ([`take_seat`]).
fn nearest(
    physics: &Physics,
    vehicles: &[ParkedVehicle],
    rules: (&HashSet<u32>, &HashSet<u32>),
    player: &Player,
    reach: f32,
) -> Option<usize> {
    let p = player.character.position;
    usable(vehicles, rules)
        .filter_map(|(i, v)| {
            let d = reach_of(physics.vehicle_transform(v.id?)?, &v.spec, p);
            (d <= reach).then_some((i, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Where she steps out of a vehicle standing at `at`: on the driver's
/// side, beside the hull.
fn door_out(at: Mat4, spec: &VehicleSpec) -> Vec3 {
    let (lo, hi) = spec.bounds;
    let door = spec
        .door
        .unwrap_or(Vec3::new(lo.x, 0.0, (lo.z + hi.z) * 0.5));
    let x = if door.x < (lo.x + hi.x) * 0.5 {
        lo.x - 0.8
    } else {
        hi.x + 0.8
    };
    at.transform_point3(Vec3::new(x, 0.0, door.z))
}

/// Whether she can get out of `v` where it is on her feet: not from a boat
/// over water
/// deeper than she can stand in ([`WADE`]); the autoplay waits for it
/// (the player may get out anywhere, and swims).
pub(crate) fn can_get_out(physics: &Physics, v: &ParkedVehicle) -> bool {
    let (Some(water), Some(id)) = (physics.water(), v.id.filter(|_| v.spec.is_boat())) else {
        return true;
    };
    let Some(at) = physics.vehicle_transform(id) else {
        return true;
    };
    let out = door_out(at, &v.spec);
    let top = Vec3::new(out.x, water + 2.0, out.z);
    physics
        .cast_ray_past(top, Vec3::NEG_Y, 2.0 + WADE, id)
        .is_some()
}

/// The deepest water she gets out of a boat into, metres: chest deep.
const WADE: f32 = 1.4;

/// How and where she gets out of `v`, standing at `at` and moving at
/// `velocity`, from seat `her_seat` (its `SeatLink` name; `None`, the
/// driver's) ([`m2_engine::seats::exit_kind`]),
/// and where she lands, world space: the hardpoint of the seat's exit
/// entrance at the hull's height (0x822A1B70 puts a ditching rider there;
/// Invented for the animated exit, whose clip is not played), or beside
/// the hull by the door when the way there is blocked (Invented stand-in
/// for 0x822A13D0's clear point) or the hardpoint lies within the hull (a
/// boat's driver gets out at the seat itself).
fn exit_plan(
    physics: &Physics,
    v: &ParkedVehicle,
    her_seat: Option<u32>,
    at: Mat4,
    velocity: Vec3,
) -> (m2_engine::seats::ExitKind, Vec3) {
    use m2_engine::seats::{self, ExitKind};
    let seating = v.spec.seating.as_deref();
    // Her seat, else the driver's.
    let index = seating.and_then(|s| her_seat.and_then(|n| s.seat(n)).or_else(|| s.driver()));
    let seat = seating.zip(index).map(|(s, i)| &s.seats[i]);
    let exit = seating.zip(index).and_then(|(s, i)| s.exit_of(i));
    let local = exit
        .and_then(|e| v.spec.entrance_at(e.name))
        .map(|p| Vec3::new(p.x, 0.0, p.z));
    let (lo, hi) = v.spec.bounds;
    let local = local.filter(|p| !(lo.x..=hi.x).contains(&p.x) || !(lo.z..=hi.z).contains(&p.z));
    let place = local.map(|p| at.transform_point3(p));
    // The 360 sweeps her capsule (0.4 m, 0.6 to 1.8 m up) from her seat to
    // the hardpoint (0x8229AC90); here one ray at 1.2 m.
    let blocked = place.zip(v.id).is_some_and(|(to, id)| {
        let from = at.transform_point3(v.spec.seat.unwrap_or(Vec3::ZERO)) + Vec3::Y * 1.2;
        let way = to + Vec3::Y * 1.2 - from;
        physics.cast_ray_past(from, way, way.length(), id).is_some()
    });
    let up_y = at.transform_vector3(Vec3::Y).normalize_or_zero().y;
    // A helicopter's landed share is not modelled: it counts as landed
    // (Invented; 0x822A2B60 ditches from one in the air).
    let kind = seats::exit_kind(
        seat,
        exit.map(|e| &e.params),
        velocity.length(),
        false,
        up_y,
        blocked,
    );
    let place = match (kind, place) {
        (ExitKind::Snap, _) | (_, None) => door_out(at, &v.spec),
        (_, Some(p)) => p,
    };
    (kind, place)
}

/// Whether `v`, standing at `at`, is a boat afloat: its origin (its
/// waterline, [`m2_engine::Physics`]'s boats float on it) under the water
/// there. The 360 asks more of it (a value of the boat's body over 0.01 to
/// dive for it, over 0.9 to be offered it from afar, 0x8229F890 and
/// 0x82440F30, read off its first float point's record +0x50); that this
/// is how far it sits in the water is inferred.
fn afloat(physics: &Physics, v: &ParkedVehicle, at: Mat4) -> bool {
    let o = at.transform_point3(Vec3::ZERO);
    v.spec.is_boat() && physics.water_at(o.x, o.z).is_some_and(|w| o.y < w)
}

/// Whether she gets out of `v` from seat `seat` (`None`, the driver's) by
/// the clips ([`crate::boarding`]): the exit kind is `Animated` and the
/// vehicle has seat data; with the entrance she leaves by (the seat's
/// exit). Not from a boat, whose exits into water swim.
fn animated_exit(physics: &Physics, v: &ParkedVehicle, seat: Option<u32>) -> Option<Option<u32>> {
    let seating = v.spec.seating.as_deref()?;
    if v.spec.is_boat() {
        return None;
    }
    let id = v.id?;
    let at = physics.vehicle_transform(id).filter(|at| at.is_finite())?;
    let (kind, _) = exit_plan(physics, v, seat, at, physics.vehicle_velocity(id));
    if kind != m2_engine::seats::ExitKind::Animated {
        return None;
    }
    let index = seat
        .and_then(|n| seating.seat(n))
        .or_else(|| seating.driver());
    Some(index.and_then(|i| seating.exit_of(i)).map(|e| e.name))
}

/// Gets her out of `v` at the end of her exit clips: as [`get_out`],
/// standing at `place` (world space), where the last clip left her.
pub(crate) fn get_out_at(
    physics: &Physics,
    v: &ParkedVehicle,
    seat: Option<u32>,
    player: &mut Player,
    place: Vec3,
    facing: f32,
) {
    get_out(physics, v, seat, player);
    player.drop_at(place);
    player.facing = facing;
    eprintln!(
        "boarding: out of the {} where her clip ends, at {:.2?}",
        v.spec.label,
        place.to_array()
    );
}

/// Gets her out of `v` as [`exit_plan`] has it, to drop to the ground
/// there; ditching or tossed, she keeps the push off (a toss's own push,
/// 0x822A1B70's passed speed, is not traced: it gets the ditch's).
pub(crate) fn get_out(
    physics: &Physics,
    v: &ParkedVehicle,
    seat: Option<u32>,
    player: &mut Player,
) {
    if let Some((id, at)) =
        v.id.and_then(|id| Some((id, physics.vehicle_transform(id)?)))
            .filter(|(_, at)| at.is_finite())
    {
        use m2_engine::seats::ExitKind;
        let velocity = physics.vehicle_velocity(id);
        let (kind, place) = exit_plan(physics, v, seat, at, velocity);
        // At the hull's height: onto the ground by the door, not onto a
        // boulder or roof over it.
        player.drop_at(place);
        if matches!(kind, ExitKind::Ditch | ExitKind::Toss) {
            let out = place - at.transform_point3(Vec3::ZERO);
            player.character.velocity =
                m2_engine::seats::ditch_velocity(velocity, Vec3::new(out.x, 0.0, out.z));
        }
        let f = at.transform_vector3(Vec3::Z);
        player.facing = f.x.atan2(f.z);
        let local = at.inverse().transform_point3(place);
        eprintln!(
            "exit: {} {kind:?} at {:.1} m/s, out at local {:.2?}",
            v.spec.label,
            velocity.length(),
            local.to_array()
        );
    }
    player.driving = None;
    player.chase_target = None;
}

/// Her camera's heading while she is played, on foot or driving (its yaw
/// turns with the mouse; the free camera's stays where it was left), else
/// the free camera's.
fn view_heading(player: Option<&Player>, fly: &FlyCamera) -> f32 {
    player.map_or(fly.yaw, |p| p.yaw)
}

/// The effect template (name hash) pickup template `template` sets off
/// where it lay as it is taken: its `Pickup` record's `PickupEffect`
/// (`Cash (Case)`: `global_particle_explosion_pickup_money`).
///
/// From the 360 code: taking a pickup, walked over (8228F5E8) or its
/// ammunition taken with the use key (8228FE58), runs 0x822905B8, which
/// finds the template the record's `PickupEffect` (word 0x18) names
/// (0x82408B88) and spawns it at the pickup, upright (0x821DD438, through
/// the game's spawner 0x82409B38), before deleting the pickup (0x8228F540).
/// A weapon taken up in place of hers (the swap, 8228FE58) is deleted with
/// no effect. Read here as the first record naming an effect
/// ([`m2_engine::pickup::PickupInfo::effect`]), the 360's choice when no
/// record is passed it; with the use key it is passed the record the
/// ammunition matched, whose own effect, if set, comes first (not
/// followed here: inferred the same).
fn taken_effect(templates: &m2_engine::Templates, template: u32) -> Option<u32> {
    templates.leavings.pickups.get(&template)?.effect
}

/// How far down the pickups she stands at `p` comes
/// ([`crate::flights::Flights::within_by`]): 0 for what she would take
/// (walked over or with the use key, given room for it, or a weapon to
/// equip), 1 for what she would leave lying.
fn would_leave(
    templates: &m2_engine::Templates,
    combat: &Combat,
    p: &crate::flights::Pickup,
) -> u8 {
    let refill = crate::mission::refill(templates, combat, &p.what);
    let usable = p.what.is_money() || combat.can_use(refill, p.what.health);
    let offer = crate::mission::offer(templates, combat, &p.what);
    u8::from(!usable && offer != crate::mission::Offer::Equip)
}

/// What a press of the weapon switch picks.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Switched {
    /// Of the vehicle she drives: 0 its main gun, 1 the machine gun.
    VehicleGun(usize),
    /// In her hands.
    Hands(Weapon),
}

/// The weapon switch (Tab, the wheel, the pad's right bumper), `pressed`
/// this tick: at the wheel it picks the vehicle's other gun, the main gun
/// or the machine gun (the game's pad layout for a tank has no button of
/// its own for the machine gun, only `Switch_Primary`, from the pause
/// screen's `tControlMap.tank`, vz.wad); on foot the other of the rifle
/// and what she carries in the launcher's place (invented: the layout's
/// `Switch_Primary` steps through her guns). Playtest 8: the vehicle's
/// switch had been left in the free camera's branch, so at the wheel the
/// press went to her hands and the machine gun could not be chosen.
fn switch_weapon(
    pressed: bool,
    driving: bool,
    weapon: Weapon,
    vehicle_gun: usize,
) -> Option<Switched> {
    if !pressed {
        return None;
    }
    Some(if driving {
        Switched::VehicleGun((vehicle_gun + 1) % 2)
    } else {
        Switched::Hands(match weapon {
            Weapon::Rifle => Weapon::Rpg,
            _ => Weapon::Rifle,
        })
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    #[test]
    fn a_pickup_taken_sets_off_its_pickup_effect() {
        let mut db = m2_engine::Templates::default();
        let money = m2_assets::name_hash("global_particle_explosion_pickup_money");
        db.leavings.pickups.insert(
            1,
            m2_engine::pickup::PickupInfo {
                kind: m2_engine::pickup::CASH,
                radius: 1.0,
                ammo: None,
                rounds: 25000,
                gives: Vec::new(),
                walked: Vec::new(),
                heals: 0,
                icon: None,
                effect: Some(money),
            },
        );
        assert_eq!(super::taken_effect(&db, 1), Some(money));
        assert_eq!(super::taken_effect(&db, 2), None);
    }

    #[test]
    fn a_trailer_is_not_got_into() {
        assert!(!super::can_sit_in("trailer"));
        assert!(super::can_sit_in("tank"));
        assert!(super::can_sit_in("car"));
    }

    #[test]
    fn a_shoulder_shot_stands_by_the_bases_hardpoint() {
        use super::{ShotActor, Vec3};
        // Fiona at the origin facing +z, her shot hardpoint 1.6 m up;
        // the player 2 m ahead of her.
        let mut shot = super::Shot {
            name: "OverTheShoulderLeft".into(),
            base: 1,
            target: 2,
            view: None,
            unplaced_logged: false,
        };
        assert_eq!(shot.hardpoint(), 0xa3a5_094a);
        let base = ShotActor {
            feet: Vec3::ZERO,
            facing: 0.0,
            hardpoint: Some(Vec3::new(0.0, 1.6, 0.0)),
        };
        let target = Vec3::new(0.0, 0.0, 2.0);
        shot.place(base, target);
        let (eye, at) = shot.view.unwrap();
        // The 360's offsets: 0.55 along her x, 0.53 behind, 0.3 up.
        assert!((eye - Vec3::new(0.55, 1.9, -0.53)).length() < 1e-5, "{eye}");
        // Looking to half way between them (1 m ahead of her feet), down
        // 0.4 for the level part's length.
        let look = at - eye;
        let level = Vec3::new(-0.55, 0.0, 1.53);
        let want = Vec3::new(level.x, -0.4, level.z).normalize();
        assert!((look - want).length() < 1e-5, "{look}");
        // The right one: the other side, its own hardpoint.
        shot.name = "OverTheShoulderRight".into();
        assert_eq!(shot.hardpoint(), 0xba8c_a3a8);
        shot.place(base, target);
        assert!((shot.view.unwrap().0.x + 0.55).abs() < 1e-5);
        // No hardpoint on the model: from the feet, as the engine does.
        shot.place(
            ShotActor {
                hardpoint: None,
                ..base
            },
            target,
        );
        assert!((shot.view.unwrap().0.y - 0.3).abs() < 1e-5);
    }

    #[test]
    fn the_switch_picks_the_vehicles_gun_at_the_wheel() {
        use super::{Switched, Weapon, switch_weapon};
        // Playtest 8: at the wheel the switch steps main gun, machine
        // gun, main gun; her hands are left alone.
        assert_eq!(
            switch_weapon(true, true, Weapon::Rifle, 0),
            Some(Switched::VehicleGun(1))
        );
        assert_eq!(
            switch_weapon(true, true, Weapon::Rifle, 1),
            Some(Switched::VehicleGun(0))
        );
        // On foot, her rifle and launcher.
        assert_eq!(
            switch_weapon(true, false, Weapon::Rifle, 0),
            Some(Switched::Hands(Weapon::Rpg))
        );
        assert_eq!(
            switch_weapon(true, false, Weapon::Rpg, 0),
            Some(Switched::Hands(Weapon::Rifle))
        );
        assert_eq!(switch_weapon(false, true, Weapon::Rifle, 0), None);
    }

    #[test]
    fn a_crew_has_a_driver_by_its_seat() {
        let seat = |s: &str| (s.to_owned(), "VZ Tank Commander".to_owned());
        assert!(super::has_driver(&[seat("Gunner"), seat("seat_driver")]));
        assert!(!super::has_driver(&[seat("Gunner")]));
    }

    #[test]
    fn heavy_vehicles_borrow_the_lorrys_horn_others_the_cars() {
        assert!(super::heavy_horn("car", "Transport Truck", false));
        assert!(super::heavy_horn("car", "LAV III (Base)", false));
        assert!(super::heavy_horn("tank", "ZTZ98", true));
        assert!(!super::heavy_horn("car", "Valiant (4door)", false));
        assert!(!super::heavy_horn("boat", "RIB36", false));
        assert!(!super::has_driver(&[]));
    }

    use m2_engine::TemplateInfo;

    use super::*;

    #[test]
    fn an_arrival_loads_until_its_place_is_in_or_its_time_is_up() {
        // Not before the least time, even with everything in.
        assert!(!arrived(ARRIVAL_MIN - 0.1, true));
        assert!(arrived(ARRIVAL_MIN, true));
        // Still loading: up until the cap.
        assert!(!arrived(ARRIVAL_CAP - 0.1, false));
        assert!(arrived(ARRIVAL_CAP, false));
    }

    #[test]
    fn only_a_car_she_takes_herself_earns_her_car_remark() {
        use m2_engine::world::VehicleKind as K;
        assert!(car_remark(Some(K::Car), false));
        // Seated in the start's speedboat by the scripts (playtest 8).
        assert!(!car_remark(Some(K::Boat), true));
        assert!(!car_remark(Some(K::Car), true));
        for kind in [K::Boat, K::Tank, K::Helicopter, K::Unknown] {
            assert!(!car_remark(Some(kind), false), "{kind:?}");
        }
        assert!(!car_remark(None, false));
    }

    #[test]
    fn the_use_prompt_names_vehicles_as_the_game_shows_them() {
        let info = |model| TemplateInfo {
            model: Some(model),
            vehicle: None,
            engine: None,
            person: false,
            human: false,
        };
        let db = Templates {
            by_name: HashMap::from([
                ("m151 softtop (v2)".to_owned(), info(7)),
                ("amx30".to_owned(), info(9)),
            ]),
            names: HashMap::from([
                (0x8000_0001, "M151 Softtop (V2)".to_owned()),
                (0x8000_0002, "AMX30".to_owned()),
            ]),
            ..Templates::default()
        };
        // A template's label, whatever its case.
        assert_eq!(vehicle_name(&db, None, "amx30", 9), "AMX30");
        // A texture's (the play test's `vz_veh_truck_M151`): the template
        // that shares its model.
        assert_eq!(
            vehicle_name(&db, None, "vz_veh_truck_M151", 7),
            "M151 Softtop (V2)"
        );
        // Neither: as it is.
        assert_eq!(vehicle_name(&db, None, "vz_veh_boat", 3), "vz_veh_boat");
        // With the strings read, the name the game shows: its template's
        // `LocalizedName` string.
        let mut db = db;
        db.localized.insert(0x8000_0001, 0x1177_c0fc);
        let mut strings = m2_assets::StringTable::default();
        strings.strings.insert(0x1177_c0fc, "Jeep".to_owned());
        assert_eq!(
            vehicle_name(&db, Some(&strings), "vz_veh_truck_M151", 7),
            "Jeep"
        );
        // A key the strings lack: the template's name.
        strings.strings.clear();
        assert_eq!(
            vehicle_name(&db, Some(&strings), "vz_veh_truck_M151", 7),
            "M151 Softtop (V2)"
        );
    }

    #[test]
    fn an_rpg_blasts_with_its_explosion_frag_and_rocket_from_the_data() {
        let key = m2_assets::name_hash;
        let e = |radius, damage, least, kind: &str| Explosion {
            radius,
            damage,
            least,
            force: 500.0,
            kind: key(kind),
        };
        let db = Templates {
            names: HashMap::from([
                (0x8000_0001, "Explosion (Handheld RPG)".to_owned()),
                (0x8000_0002, "Explosion (Handheld RPG Frag)".to_owned()),
                (0x8000_0003, "RPG Rocket".to_owned()),
            ]),
            explosions: HashMap::from([
                (0x8000_0001, e(5.0, 200.0, 0.0, "Explosion")),
                (0x8000_0002, e(15.0, 300.0, 40.0, "Bullet")),
            ]),
            projectiles: HashMap::from([(
                0x8000_0003,
                m2_engine::durability::Projectile {
                    damage: 100.0,
                    kind: key("Rocket"),
                },
            )]),
            ..Templates::default()
        };
        let at = Vec3::new(1.0, 2.0, 3.0);
        let blasts = rocket_blasts(&db, at);
        let kinds: Vec<u32> = blasts.iter().map(|b| b.kind).collect();
        assert_eq!(kinds, [key("Explosion"), key("Bullet"), key("Rocket")]);
        assert!(blasts.iter().all(|b| b.at == at));
        // The rocket only on what it lands on.
        assert_eq!(blasts[2].at_distance(0.2), 100.0);
        assert_eq!(blasts[2].at_distance(1.0), 0.0);
        // The frag still bites at 10 m: 300 x (1 - 10 / 15), and never
        // under its 40 within reach.
        assert!((blasts[1].at_distance(10.0) - 100.0).abs() < 1e-3);
        assert_eq!(blasts[1].at_distance(14.9), 40.0);
        // With none in the data: one blast of combat's own, taken in full.
        let none = rocket_blasts(&Templates::default(), at);
        assert_eq!(none.len(), 1);
        assert_eq!(none[0].kind, 0);
    }

    #[test]
    fn the_compass_reads_the_camera_the_frame_is_seen_from() {
        // The free camera left looking north; she looks east, then
        // south-west, as the mouse turns her camera.
        let fly = FlyCamera {
            position: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
        };
        for yaw in [std::f32::consts::FRAC_PI_2, -2.4] {
            let p = Player::new(0.0, 0.0, yaw, None);
            let heading = view_heading(Some(&p), &fly);
            // The heading her view looks along (right-handed: down -z).
            let ahead = -p.view(60.0, 16.0 / 9.0).view.inverse().z_axis.truncate();
            let seen = ahead.x.atan2(ahead.z);
            assert!((heading - seen).abs() < 1e-4, "{heading} {seen}");
        }
        // Flying, the free camera's.
        assert_eq!(view_heading(None, &fly), 0.0);
    }

    #[test]
    fn a_blast_throws_light_props_not_heavy_ones() {
        let prop = |heavy| m2_engine::durability::PropPhysics { mass: None, heavy };
        let db = Templates {
            prop_physics: HashMap::from([(1, prop(false)), (2, prop(true))]),
            ..Templates::default()
        };
        // A café chair flies; a 20 t container stands.
        assert!(blown_loose(&db, 1));
        assert!(!blown_loose(&db, 2));
        // One the data gives no damage table row: thrown, as before.
        assert!(blown_loose(&db, 3));
    }
}

/// Her scope's magnification `was` (`None`: just raised, at `least`)
/// stepped `zoom` levels, held between `least` and `most`.
fn scope_step(was: Option<f32>, zoom: i32, least: f32, most: f32) -> f32 {
    (was.unwrap_or(least) + zoom as f32).clamp(least, most.max(least))
}

/// Her lens' share of the field of view after `dt` seconds from `lens`,
/// easing to her gun's share: `CrouchFov` aiming, `FOV` else, 1 for a gun
/// whose template says neither.
fn lens_step(
    lens: f32,
    feel: Option<m2_engine::weapons::CameraFeel>,
    aiming: bool,
    dt: f32,
) -> f32 {
    let want = match feel {
        Some(f) if aiming => f.aim_fov,
        Some(f) => f.fov,
        None => 1.0,
    };
    lens + (want - lens) * (LENS_EASE * dt).min(1.0)
}

#[cfg(test)]
mod lens_tests {
    use super::*;

    // The sniper rifle's record: its view narrows to half aiming, and
    // widens back to the whole lens when she lowers it.
    #[test]
    fn aiming_narrows_the_lens_to_its_guns_share() {
        let sniper = m2_engine::weapons::CameraFeel {
            fov: 1.0,
            aim_fov: 0.5,
            look: [0.8, 0.8],
            aim_look: [0.5, 0.5],
        };
        let mut lens = 1.0;
        for _ in 0..60 {
            lens = lens_step(lens, Some(sniper), true, 1.0 / 60.0);
        }
        assert!((lens - 0.5).abs() < 0.01, "{lens}");
        for _ in 0..60 {
            lens = lens_step(lens, Some(sniper), false, 1.0 / 60.0);
        }
        assert!((lens - 1.0).abs() < 0.01, "{lens}");
        assert_eq!(lens_step(0.7, None, true, 1.0), 1.0);
    }

    // The sniper rifle's scope: raised at 4x, the wheel up to 8x and no
    // further, down to 4x and no further.
    #[test]
    fn the_scope_steps_between_its_magnifications() {
        let mut z = scope_step(None, 0, 4.0, 8.0);
        assert_eq!(z, 4.0);
        for _ in 0..6 {
            z = scope_step(Some(z), 1, 4.0, 8.0);
        }
        assert_eq!(z, 8.0);
        assert_eq!(scope_step(Some(z), -9, 4.0, 8.0), 4.0);
    }

    // Taking up a gun with the use key cues its set's `equip_pickup`;
    // walking over it (its rounds only) cues none.
    #[test]
    fn a_gun_taken_up_cues_its_pickup_sound() {
        let templates = m2_engine::Templates::default();
        let mut flights = crate::flights::Flights::default();
        let mut player = Player::new(0.0, 0.0, 0.0, None);
        for walked in [true, false] {
            let mut combat = Combat::default();
            let pickup = crate::flights::Pickup {
                from: 0,
                what: crate::flights::Leaves {
                    weapon: Some(crate::combat::Weapon::Rpg),
                    gives: Some((crate::combat::Weapon::Rpg, 2)),
                    ..Default::default()
                },
                at: Vec3::ZERO,
                yaw: 0.0,
            };
            take_up(
                &mut combat,
                &mut flights,
                &templates,
                &mut player,
                pickup,
                walked,
            );
            let cued = combat
                .take_sounds()
                .iter()
                .filter(|s| matches!(s, crate::combat::Sound::Pickup { .. }))
                .count();
            assert_eq!(cued, usize::from(!walked));
        }
    }
}

#[cfg(test)]
mod picture_tests {
    use super::Picture;

    #[test]
    fn view_distance_draws_a_quarter_to_all_of_the_usual_reach() {
        use super::view_distance_scale;
        assert_eq!(view_distance_scale(100.0), 1.0);
        assert_eq!(view_distance_scale(10.0), 0.25);
        assert_eq!(view_distance_scale(0.0), 0.25);
        assert_eq!(view_distance_scale(55.0), 0.625);
    }

    #[test]
    fn brightness_is_a_stop_either_way_and_shadows_can_go() {
        let lit = |p: Picture| {
            let mut l = m2_render::Lighting {
                shadow_distance: 120.0,
                ..Default::default()
            };
            p.apply(&mut l);
            (l.grade.x, l.shadow_distance)
        };
        assert_eq!(lit(Picture::default()), (1.0, 120.0));
        let bright = Picture {
            brightness: 100.0,
            ..Picture::default()
        };
        assert_eq!(lit(bright).0, 2.0);
        let dark = Picture {
            brightness: 0.0,
            shadows: false,
        };
        assert_eq!(lit(dark), (0.5, 0.0));
    }
}

/// The cover service (0x822CC020): for each soldier who asked this tick, the
/// best spot among the cover hints of the objects within the search radius
/// of him ([`m2_engine::ai_cover`]), from the threat he named, away from
/// his fellow soldiers (at most 15); none found, he asks again after
/// [`m2_engine::ai_cover::RETRY`]. Rays are the physics' (the program's
/// own filters are not modelled); the threat's eye is taken 1.3 m over
/// where she stands (inferred).
fn serve_cover(world: &m2_engine::World, people: &mut crate::npc::People) {
    use m2_engine::ai_cover::{self, Placed, Request};
    let asks: Vec<(usize, Vec3, Vec3)> = people
        .npcs
        .iter()
        .enumerate()
        .filter_map(|(i, n)| Some((i, n.character.position, n.cover_ask?)))
        .collect();
    for (i, centre, threat) in asks {
        let avoid: Vec<Vec3> = people
            .npcs
            .iter()
            .enumerate()
            .filter(|&(j, n)| j != i && n.alive && n.soldier)
            .map(|(_, n)| n.character.position)
            .filter(|p| p.distance(centre) < ai_cover::SEARCH * 2.0)
            .take(15)
            .collect();
        let req = Request {
            centre,
            radius: ai_cover::SEARCH,
            threat: threat + Vec3::Y * 1.3,
            avoid,
        };
        let mut cands = Vec::new();
        for (o, _) in world.near(centre, ai_cover::SEARCH) {
            let p = &world.index.objects[o];
            let Some(hints) = world.index.template_db.cover_hints.get(&p.template) else {
                continue;
            };
            let Some(model) = world.object_model(o) else {
                continue;
            };
            let placed = Placed {
                position: p.position,
                rotation: p.rotation,
                bounds: model.bounds,
            };
            for h in hints {
                cands.extend(ai_cover::candidates(&req, &placed, h));
            }
        }
        let physics = &world.physics;
        let ground = |at: Vec3| {
            physics
                .cast_ray(at + Vec3::Y * 0.5, -Vec3::Y, 2.0)
                .map(|(d, _)| at.y + 0.5 - d)
        };
        let blocked = |a: Vec3, b: Vec3| {
            let to = b - a;
            physics
                .cast_ray(a, to, to.length())
                .is_some_and(|(d, _)| d < to.length() - 0.05)
        };
        let found = ai_cover::choose(&req, &cands, ground, blocked);
        let n = &mut people.npcs[i];
        n.cover_ask = None;
        n.cover_spot = found;
        if let Some(at) = found {
            eprintln!(
                "people: {:08x} takes cover at {at:.1} ({} spots looked at)",
                n.spawn.unwrap_or(u32::MAX),
                cands.len()
            );
        }
    }
}
