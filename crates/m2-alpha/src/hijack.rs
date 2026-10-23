//! Hijacking crewed vehicles, as the game's own hijack scripts play it.
//!
//! A vehicle whose template links a hijack entrance
//! ([`m2_engine::hijack`]) and has a crew can be hijacked: on foot within
//! reach of the entrance's hardpoint (the M1A2's `DriverHatch`), the use
//! key starts the entrance's script (`hijack_m1a2`) in a script host of
//! its own ([`m2_game::mission::Mission`], loaded in the background). The
//! script asks for buttons ([`m2_game::hijack::Minigame`]); the pad's face
//! buttons are [`Action::PadY`] and its kin, the left stick's pushes the
//! movement keys. Each character's action lasts as long as the clip the
//! game's animation tables give it ([`ClipTimes`]). Done, she drives the
//! vehicle; failed, she is let go beside it.
//!
//! A vehicle's crew is its template's riders ([`ParkedVehicle::crew`]):
//! the hijack throws out whoever has the driver's seat, and one without a
//! driver can't be hijacked. `--hijack-crew` (a testing aid) gives every
//! hijackable vehicle without one a driver of that template.
//!
//! Every rider of the vehicles near her is drawn in the seat its template
//! links (`SeatLink` and its hardpoint), playing the tables' `InVehicle`
//! `Idle` clip for that seat, as she sits driving ([`Rider`]); the driver
//! under a `DriverHatch` is inside, out of sight. From the data; the
//! 200 m reach is the people's.
//!
//! Her clips, the crewman's and the vehicle's play together. The
//! vehicle's clip moves its own nodes (the hatch thrown open). Her clips
//! play from the vehicle's origin, lifted so that, holding the pistol at
//! the hatch, she stands on the roof the hatch hinges in (the M1A2 0.9 m,
//! the AMX30 2.1 m). The crewman (drawn as a rider is, on his kind's
//! skeleton) plays his so that in his struggle loop his pelvis is at the
//! hatch: half out of it. The camera sits close at her chest, from her
//! side of the vehicle, as the footage's does. The scripts' own calls
//! to the HUD (the buttons to press) go to the game's HUD
//! ([`Hijacks::take_gui_calls`]).
//!
//! Provenance: which clips play is the tables' (from samples; the
//! crewman's by the driver's seat). Where they play is inferred from
//! samples against the footage (the hijack video's M1A2: her on the
//! turret roof, the crewman half out of the hatch): her clips sat 0.9 m
//! (M1A2) to 2.1 m (AMX30) under that roof, his some 6 m behind the tank
//! (his loop's pelvis then lands within 0.1 m of the hatch across and
//! up, both tanks). How the game places them was not read from the 360
//! code. The camera's distance and angle are from the footage.
//!
//! [`ParkedVehicle::crew`]: crate::scene::ParkedVehicle::crew
//!
//! Inferred: she must stand within the entrance's radius plus [`SLACK`]
//! of the vehicle's hull, measured flat (a tank's hatch radius is 1 m, and
//! the hatch is on top of the hull, out of her reach on foot); of several
//! entrances (a helicopter's two doors), the nearest is taken.

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use m2_assets::WadArchive;
use m2_assets::animation::{ANIMATION_TYPE, Clip};
use m2_assets::animation_table;
use m2_engine::Install;
use m2_engine::hijack::HijackEntrance;
use m2_game::api::Command;
use m2_game::hijack::{ClipTimes, HijackCommand, Minigame};
use m2_game::mission::{GameData, Mission};
use m2_math::glam::{Mat4, Vec3};

use crate::game::Game;
use crate::gamehud::Plain;
use crate::input::{Action, Input};

/// The scripts run ten times a second, as the mission's do.
const STEP: f32 = 0.1;
/// Reach beyond the entrance's radius, metres, measured flat from the
/// hull.
pub const SLACK: f32 = 2.0;
/// The camera during a hijack: this far from her chest ([`CHEST`] above
/// her pelvis), looking at her a little down. From the footage (the
/// hijack video's tanks: her and the crewman fill the frame, some 2-3 m
/// off); the 6 m read from the scripts' `Camera.SetFocusParams(cam, 0, 0,
/// 0.1, 4, 6, 1)` left them small across the hull (playtest 8).
const CAMERA_DISTANCE: f32 = 2.8;
const CAMERA_PITCH: f32 = -0.35;
/// Her chest above her pelvis, metres.
const CHEST: f32 = 0.35;
/// The camera goes to her side of the vehicle once she is this far off
/// its middle, metres; and looks back at her from this far ahead for
/// each metre out to the side. From the footage's framing; invented
/// numbers.
const SIDE_SWITCH: f32 = 0.3;
const CAMERA_AHEAD: f32 = 0.6;
/// How far above the vehicle's top (later her feet) the camera looks,
/// metres.
const CAMERA_LIFT: f32 = 1.0;
/// Mouse counts within [`FLICK_WINDOW`] that make a flick ([`Flick`]).
/// Invented.
const FLICK: f64 = 40.0;
/// How long the mouse's motion adds up toward a flick, seconds. Counted
/// per tick before (playtest 12: no tap of 9 rounds reached the scripts),
/// a flick had to land its counts inside one frame: 6 ms at Brendan's
/// 165 Hz, which an ordinary flick doesn't. Invented.
const FLICK_WINDOW: f64 = 0.12;

/// The mouse's motion of late, adding up toward a flick whatever the
/// frame rate: the counts since the window began, and how long ago.
#[derive(Debug, Default)]
struct Flick {
    dx: f64,
    dy: f64,
    seconds: f64,
}

impl Flick {
    /// Adds a tick's motion (`dt` seconds): the button a flick presses
    /// ([`m2_game::hijack::BUTTONS`]: up 5, down 6, left 7, right 8), with
    /// its counts and how long they took, once per flick.
    fn feed(&mut self, dx: f64, dy: f64, dt: f64) -> Option<(u32, f64, f64)> {
        self.dx += dx;
        self.dy += dy;
        self.seconds += dt;
        let (counts, button) = if self.dy.abs() > self.dx.abs() {
            (self.dy.abs(), if self.dy < 0.0 { 5 } else { 6 })
        } else {
            (self.dx.abs(), if self.dx < 0.0 { 7 } else { 8 })
        };
        let flicked = counts > FLICK;
        if flicked || self.seconds >= FLICK_WINDOW {
            let seconds = self.seconds;
            *self = Self::default();
            if flicked {
                return Some((button, counts, seconds));
            }
        }
        None
    }
}
/// How long an action lasts when its clip is unknown, seconds.
const UNKNOWN_CLIP: f64 = 2.0;
/// What the keyboard does for a hijack prompt ("K or flick the mouse
/// down", "hold K", "J and L in turn"), for the HUD to write under the
/// game's arrow, which on the PC names no key. `None` for a button no key
/// has.
pub fn key_hint(prompt: &Minigame, keys: &crate::input::Bindings) -> Option<String> {
    let key = |button: u32| -> Option<String> {
        let (action, _) = PAD.iter().find(|(_, b)| *b == button)?;
        keys.key_name(*action)
    };
    let flick = |button: u32| match button {
        5 => " or flick the mouse up",
        6 => " or flick the mouse down",
        7 => " or flick the mouse left",
        8 => " or flick the mouse right",
        _ => "",
    };
    let first = *prompt.buttons.first()?;
    Some(match prompt.mode.as_str() {
        "hold" => format!("hold {}", key(first)?),
        "alternate" => {
            let second = *prompt.buttons.get(1)?;
            format!("{} and {} in turn", key(first)?, key(second)?)
        }
        _ => format!("{}{}", key(first)?, flick(first)),
    })
}

/// The pad, as the scripts number its buttons
/// ([`m2_game::hijack::BUTTONS`]).
const PAD: [(Action, u32); 8] = [
    (Action::PadY, 5),
    (Action::PadA, 6),
    (Action::PadX, 7),
    (Action::PadB, 8),
    (Action::MoveLeft, 9),
    (Action::MoveRight, 10),
    (Action::MoveForward, 11),
    (Action::MoveBack, 12),
];

struct Loaded {
    mission: Mission,
    clips: ClipTimes,
}

/// The hijack scripts' calls to the HUD (`MrxGuiHudActionHijack.ShowButton`
/// and its kin, `MrxGuiManager.ToggleHud`), which run in the game HUD's own
/// Lua state: kept in `__m2_gui_calls` (module, method, then the call's
/// plain arguments, `n` of them) for [`Hijacks::take_gui_calls`].
const GUI_CALLS: &str = r##"
__m2_gui_calls = __m2_gui_calls or {}
local function record(module, method, f)
    local wrapped = function(...)
        local call = { module, method, n = select("#", ...) }
        for i = 1, call.n do
            local v = select(i, ...)
            if type(v) ~= "function" and type(v) ~= "table" then call[i + 2] = v end
        end
        table.insert(__m2_gui_calls, call)
        if #__m2_gui_calls > 64 then table.remove(__m2_gui_calls, 1) end
        if type(f) == "function" then return f(...) end
    end
    __m2_gui_wrapped = __m2_gui_wrapped or setmetatable({}, { __mode = "k" })
    __m2_gui_wrapped[wrapped] = true
    return wrapped
end
-- Wraps the module's functions where they are: the scripts import the
-- game's own module (over any table put here before), so this runs again
-- as they go.
local function wrap(module, methods)
    local m = _G[module]
    if type(m) ~= "table" then
        m = {}
        rawset(_G, module, m)
    end
    for _, method in ipairs(methods) do
        local f = m[method]
        if not (__m2_gui_wrapped and __m2_gui_wrapped[f]) then
            rawset(m, method, record(module, method, f))
        end
    end
end
wrap("MrxGuiHudActionHijack", { "ShowButton", "HideButton", "ShowFail", "SetDisplayVisible",
    "SetDisplayButton", "SetDisplayMashAnimation" })
wrap("MrxGuiManager", { "ToggleHud" })
"##;

/// A call the hijack scripts made to the HUD ([`GUI_CALLS`]).
#[derive(Clone, Debug, PartialEq)]
pub struct GuiCall {
    /// `MrxGuiHudActionHijack`, `MrxGuiManager`.
    pub module: String,
    pub method: String,
    /// Its arguments, nil ones in place.
    pub args: Vec<Plain>,
}

enum State {
    Loading(Receiver<Result<(GameData, ClipTimes), String>>),
    Ready(Box<Loaded>),
    Failed,
}

/// A vehicle she could hijack now.
#[derive(Clone, Debug)]
struct Candidate {
    vehicle: usize,
    template: String,
    entrance: HijackEntrance,
    crew: String,
    /// The entrance's hardpoint, in the world.
    at: Vec3,
}

/// A hijack under way.
struct Active {
    vehicle: usize,
    /// Her, as the scripts know her.
    hero: u32,
    /// The vehicle's `VehicleAnimationSet`, which picks the clips.
    set: Option<(u32, u32)>,
    /// Characters' actions and when they end (the scripts' time).
    actions: Vec<(u32, f64)>,
    /// Pad buttons held, as the scripts were last told.
    held: Vec<u32>,
    /// The mouse's motion of late, for a flick.
    flick: Flick,
    /// Her hijack clip playing (an index into her rig's clips).
    clip: Option<usize>,
    /// The entrance, in the world when it started: the camera looks at it.
    at: Vec3,
    /// The vehicle's model space from its pistol prop's parent node: the
    /// vehicle's clips carry the pistol she holds.
    pistol_parent: Option<Mat4>,
    /// The vehicle's hijack clip playing alongside hers.
    vehicle_clip: Option<Clip>,
    /// Her clips' height above the vehicle's origin (the AMX30's are cut
    /// ~1.2 m below its deck, the M1A2's on it), one for the whole hijack
    /// from her first clip, and that clip, played unlifted on the ground.
    lift: Option<f32>,
    first: Option<usize>,
    /// Whether the scripts last showed or hid her (`SetVisible`).
    shown: Option<bool>,
    /// The crewman, as the scripts know him, and his template.
    crewman: u32,
    crew: String,
    /// The crewman drawn playing his hijack clips, once one binds.
    hijackee: Option<Hijackee>,
    /// His clip while his model loads (hash, looped, seconds since).
    pending: Option<(u32, bool, f32)>,
    /// The height of the roof the hijack's hatch is set in (the vehicle's
    /// space): where its clip's hatch hinges.
    roof: Option<f32>,
    /// Where that hatch hinges (the vehicle's space).
    hatch: Option<Vec3>,
    /// The crewman's struggle loop, and how far his clips are moved to
    /// put him half out of the hatch there.
    crew_loop: Option<u32>,
    crew_offset: Option<Vec3>,
    /// The lift worked out ahead from her pistol loop, once.
    lift_known: Option<f32>,
    /// The lifts that put her feet on the hatch's roof in her clips holding
    /// the pistol (hand-to-pistol height straying under [`GRIP_SPREAD`]).
    grips: Vec<f32>,
    /// The vehicle's own nodes (local transforms and parents), for its
    /// hijack clip to move (the hatch thrown open), and when that clip
    /// started (seconds, the scripts' time).
    vehicle_nodes: Option<VehicleNodes>,
    vehicle_started: f64,
}

/// The crewman a hijack throws out, drawn as a rider of the vehicle is.
struct Hijackee {
    kind: usize,
    outfit: Vec<bool>,
    /// His rig, holding only the clip playing.
    rig: crate::scene::Rig,
    /// Whether the scripts last showed him (`SetVisible`).
    shown: bool,
}

/// A vehicle model's nodes: each one's transform to its parent, and its
/// parent.
struct VehicleNodes {
    locals: Vec<Mat4>,
    parents: Vec<Option<u16>>,
    names: Vec<u32>,
}

/// What the HUD shows of a hijack.
#[derive(Clone, Debug, PartialEq)]
pub struct HijackView {
    /// The vehicle's template (`M1A2`): the game's prompt is `Hijack %s`.
    pub vehicle: String,
    /// A hijack is under way (else she is only in reach of one).
    pub running: bool,
    /// The buttons the script waits on.
    pub prompts: Vec<Minigame>,
}

pub struct Hijacks {
    state: State,
    lag: f32,
    /// A testing aid: every hijackable vehicle without a driver is driven
    /// by this human template (`--hijack-crew`).
    pub crew: Option<String>,
    /// Hijackable templates by model hash: the name and entrances of the
    /// first by name.
    by_model: Option<HashMap<u32, (String, Vec<HijackEntrance>)>>,
    /// Models' hardpoints (model, name hash), model space.
    points: HashMap<(u32, u32), Option<Vec3>>,
    near: Option<Candidate>,
    active: Option<Active>,
    /// Each vehicle's driver's seat, once looked up, by its model and
    /// label (its template's name, which picks the seat).
    seats: HashMap<(u32, String), Option<Seat>>,
    /// Whether the hijack scripts left her seen or hidden in a vehicle
    /// (by the same key as [`Hijacks::seats`]), over the seat's own guess
    /// ([`Seat::shown`]).
    scripts_shown: HashMap<(u32, String), bool>,
    /// Vehicles (index, model) whose parts are set as their templates say.
    dressed: std::collections::HashSet<(usize, u32)>,
    /// Riders' seats, by the vehicle's model and label and the seat.
    rider_seats: HashMap<(u32, String, u32), Option<Seat>>,
    /// Seated clips read for riders, by hash.
    rider_clips: HashMap<u32, Option<Clip>>,
    /// Seated clips (hash) that don't bind to a people's kind (index).
    rider_unbound: std::collections::HashSet<(u32, usize)>,
    /// The riders' looks, by vehicle (index, model) and seat: kept while
    /// the vehicle is.
    rider_looks: HashMap<(usize, u32, u32), Vec<bool>>,
    /// The seats of props' riders ([`Hijacks::seat_prop_riders`]), by
    /// the prop's model, its template and the seat.
    prop_seats: HashMap<(u32, u32, u32), Option<Seat>>,
    /// The looks of props' riders, by the rider's guid.
    prop_looks: HashMap<u32, Vec<bool>>,
    /// The latest riders' tally, and the last one written to the log.
    pub riders: RiderTally,
    told_riders: RiderTally,
}

/// Riders are drawn in vehicles this near her, metres (as far as people
/// on foot are drawn, [`crate::npc::DRAW_DISTANCE`]).
const RIDER_DISTANCE: f32 = crate::npc::DRAW_DISTANCE;

/// Someone drawn sitting in a vehicle: one of its template's riders
/// ([`ParkedVehicle::crew`]), in the seat its `SeatLink` names, playing the
/// tables' `InVehicle` `Idle` clip for that seat, as she sits driving.
///
/// [`ParkedVehicle::crew`]: crate::scene::ParkedVehicle::crew
#[derive(Clone, Debug)]
pub struct Rider {
    /// Index into the scene's vehicles (unused for one on a prop).
    pub vehicle: usize,
    /// The layer object they sit on instead of a vehicle (a building:
    /// the PMC HQ interior's starter), by guid
    /// ([`crate::scene::Scene::prop_crews`]).
    pub prop: Option<u32>,
    /// The scripts' guid for the rider, when known (a prop's crew: Fiona,
    /// whom a briefing's clips play on, [`crate::raw`]).
    pub guid: Option<u32>,
    /// The seat, by its name's hash (`seat_driver`).
    pub seat: u32,
    /// Index into the people's kinds: whom they are drawn as.
    pub kind: usize,
    /// Their rig, holding only the seated clip.
    pub rig: crate::scene::Rig,
    /// Their look ([`crate::npc::Npc::outfit`]).
    pub outfit: Vec<bool>,
    /// Model to world: the clip's origin on the seat, with the vehicle's
    /// whole turn.
    pub frame: Mat4,
}

/// How the riders of the vehicles near her came out, for the log: seats
/// asked for, drawn, and why the rest were not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RiderTally {
    pub seats: usize,
    pub drawn: usize,
    pub no_seat: usize,
    pub no_kind: usize,
    pub no_clip: usize,
    pub hidden: usize,
}

/// Where and how she sits driving a vehicle.
#[derive(Clone, Copy, Debug)]
struct Seat {
    /// Her `InVehicle` `Idle` clip for the vehicle's animation set.
    clip: u32,
    /// The seat, model space: her clip's origin goes there.
    at: Vec3,
    /// She can be seen there: not inside, under a hatch. The vehicle's
    /// entrance being a `DriverHatch` says so ([`find_seat`]); the hijack
    /// scripts' showing or hiding her goes over it
    /// ([`Hijacks::scripts_shown`]).
    shown: bool,
}

impl Hijacks {
    /// Starts reading the scripts and the clips' lengths from `install`
    /// in the background.
    pub fn start(install: &Install, crew: Option<String>) -> Self {
        let wad = install.data.join("vz.wad");
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let load = || -> Result<(GameData, ClipTimes), String> {
                let open = || -> Result<WadArchive<BufReader<File>>, String> {
                    let f = File::open(&wad).map_err(|e| format!("{}: {e}", wad.display()))?;
                    WadArchive::open(BufReader::new(f)).map_err(|e| e.to_string())
                };
                let mut w = open()?;
                let data = GameData::read(&mut w).map_err(|e| e.to_string())?;
                let clips = ClipTimes::read(&mut open()?).map_err(|e| e.to_string())?;
                Ok((data, clips))
            };
            let _ = tx.send(load());
        });
        Self {
            state: State::Loading(rx),
            lag: 0.0,
            crew,
            by_model: None,
            points: HashMap::new(),
            near: None,
            seats: HashMap::new(),
            scripts_shown: HashMap::new(),
            dressed: Default::default(),
            rider_seats: HashMap::new(),
            rider_clips: HashMap::new(),
            rider_unbound: Default::default(),
            rider_looks: HashMap::new(),
            prop_seats: HashMap::new(),
            prop_looks: HashMap::new(),
            riders: RiderTally::default(),
            told_riders: RiderTally::default(),
            active: None,
        }
    }

    /// The hijack in reach or under way, for the HUD.
    pub fn view(&self) -> Option<HijackView> {
        let State::Ready(l) = &self.state else {
            return None;
        };
        if self.active.is_some() {
            let vehicle = self
                .near
                .as_ref()
                .map(|c| c.template.clone())
                .unwrap_or_default();
            return Some(HijackView {
                vehicle,
                running: true,
                prompts: l.mission.minigames(),
            });
        }
        self.near.as_ref().map(|c| HijackView {
            vehicle: c.template.clone(),
            running: false,
            prompts: Vec::new(),
        })
    }

    /// The hijack scripts' calls to the HUD since the last time, oldest
    /// first.
    pub fn take_gui_calls(&self) -> Vec<GuiCall> {
        let State::Ready(l) = &self.state else {
            return Vec::new();
        };
        let lua = l.mission.lua();
        // Again, over the modules the scripts have imported since.
        if let Err(e) = lua.load(GUI_CALLS).set_name("=m2-gui calls").exec() {
            eprintln!("hijacks: HUD calls not kept: {e}");
        }
        let Ok(Some(calls)) = lua.globals().get::<Option<mlua::Table>>("__m2_gui_calls") else {
            return Vec::new();
        };
        let out = calls
            .sequence_values::<mlua::Table>()
            .flatten()
            .map(|t| {
                let n: i64 = t.get("n").unwrap_or(0);
                GuiCall {
                    module: t.get(1).unwrap_or_default(),
                    method: t.get(2).unwrap_or_default(),
                    args: (1..=n)
                        .map(|i| Plain::of(&t.get(i + 2).unwrap_or(mlua::Value::Nil)))
                        .collect(),
                }
            })
            .collect();
        let _ = lua.globals().set("__m2_gui_calls", lua.create_table().ok());
        out
    }

    /// Runs before the game's own tick: takes the use key when a hijack
    /// is in reach, and the pad's buttons while one is under way.
    pub fn tick(&mut self, game: &mut Game, input: &mut Input, dt: f32) {
        if let State::Loading(rx) = &self.state {
            match rx.try_recv() {
                Ok(Ok((data, clips))) => match Mission::new(data, game.scene.hero.name(), [0.0; 3])
                {
                    Ok(mut mission) => {
                        eprintln!("hijacks: scripts loaded");
                        crate::mission::set_hero(
                            &mut mission,
                            game.scene.hero,
                            game.combat.max_hp,
                            "hijacks",
                        );
                        if let Err(e) = mission
                            .lua()
                            .load(GUI_CALLS)
                            .set_name("=m2-gui calls")
                            .exec()
                        {
                            eprintln!("hijacks: HUD calls not kept: {e}");
                        }
                        self.state = State::Ready(Box::new(Loaded { mission, clips }));
                    }
                    Err(e) => {
                        eprintln!("hijacks: could not start the scripts: {e}");
                        self.state = State::Failed;
                    }
                },
                Ok(Err(e)) => {
                    eprintln!("hijacks: could not load the scripts: {e}");
                    self.state = State::Failed;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.state = State::Failed,
            }
        }
        if !matches!(self.state, State::Ready(_)) {
            drain(input);
            return;
        }
        if self.by_model.is_none()
            && let Some(world) = &game.scene.world
        {
            let by_model = hijackable(&world.index.template_db);
            // Crewed, these are taken by their hijack alone.
            game.hijack_only = by_model.keys().copied().collect();
            self.by_model = Some(by_model);
        }
        if self.active.is_some() {
            self.pad(input, dt);
        } else {
            drain(input);
            self.near = self.find(game);
            // In reach, the crewman's model is asked for: it takes some
            // seconds to come in, and he is drawn once it has.
            if let Some(c) = &self.near {
                let _ = crew_kind(game, &c.crew);
            }
            // The use key, or the grappling hook having pulled her to a
            // helicopter's entrance ([`crate::grapple`]).
            let handed = std::mem::take(&mut game.grapple_handoff);
            if self.near.is_some() && (handed || input.take_pressed(Action::Use)) {
                self.begin(game);
            } else if handed {
                eprintln!("grapple: no hijack in reach at the entrance");
            }
        }
        let State::Ready(l) = &mut self.state else {
            return;
        };
        if self.active.is_none() {
            return;
        }
        self.lag += dt;
        while self.lag >= STEP {
            self.lag -= STEP;
            l.mission.tick(f64::from(STEP));
            for line in l.mission.take_log() {
                eprintln!("hijack: {line}");
            }
            let commands = l.mission.take_commands();
            if !follow(
                &mut self.active,
                &mut self.scripts_shown,
                &l.clips,
                &mut l.mission,
                commands,
                game,
            ) {
                self.active = None;
                game.input_locked = false;
                return;
            }
        }
    }

    /// Feeds the pad's presses and holds to the scripts.
    fn pad(&mut self, input: &mut Input, dt: f32) {
        let (State::Ready(l), Some(a)) = (&mut self.state, &mut self.active) else {
            return;
        };
        // A flick of the mouse the way an arrow points presses it, as the
        // PC HUD draws the face buttons as arrows (see
        // [`crate::input::Bindings`]; inferred).
        let (dx, dy) = input.take_mouse();
        if let Some((button, counts, seconds)) = a.flick.feed(dx, dy, f64::from(dt)) {
            eprintln!(
                "hijack: button {button} by a mouse flick ({counts:.0} counts in {seconds:.2} s)"
            );
            l.mission.press(button);
        }
        for (action, button) in PAD {
            if input.take_pressed(action) {
                eprintln!("hijack: button {button} pressed ({action:?})");
                l.mission.press(button);
            }
            let down = input.held(action);
            if down != a.held.contains(&button) {
                l.mission.hold(button, down);
                if down {
                    a.held.push(button);
                } else {
                    a.held.retain(|&b| b != button);
                }
            }
        }
    }

    /// The nearest crewed, hijackable vehicle whose entrance she is in
    /// reach of, on foot.
    fn find(&mut self, game: &mut Game) -> Option<Candidate> {
        let by_model = self.by_model.as_ref()?;
        if !game.on_foot {
            return None;
        }
        let player = game.scene.player.as_ref().filter(|p| p.driving.is_none())?;
        let here = player.character.position;
        let world = game.scene.world.as_ref()?;
        let mut best: Option<(f32, Candidate)> = None;
        for (i, v) in game.scene.vehicles.iter().enumerate() {
            if v.wrecked {
                continue;
            }
            let Some((template, entrances)) = by_model.get(&v.spec.model) else {
                continue;
            };
            let Some(crew) = driver(&v.crew).or_else(|| self.crew.clone()) else {
                continue;
            };
            let Some(at) = v.id.and_then(|id| world.physics.vehicle_transform(id)) else {
                continue;
            };
            for e in entrances {
                let local = e
                    .point
                    .and_then(|p| {
                        point(
                            &mut self.points,
                            game.scene.source.as_mut(),
                            v.spec.model,
                            p,
                        )
                    })
                    .or(v.spec.door)
                    .unwrap_or(Vec3::ZERO);
                let spot = at.transform_point3(local);
                // In reach of the hull; of its entrances, the nearest.
                let mine = at.inverse().transform_point3(here);
                let (lo, hi) = v.spec.bounds;
                let hull = (mine.clamp(lo, hi) - mine).truncate_y().length();
                let d = (spot - here).truncate_y().length();
                if hull <= e.radius + SLACK && best.as_ref().is_none_or(|b| d < b.0) {
                    best = Some((
                        d,
                        Candidate {
                            vehicle: i,
                            template: template.clone(),
                            entrance: *e,
                            crew: crew.clone(),
                            at: spot,
                        },
                    ));
                }
            }
        }
        best.map(|b| b.1)
    }

    /// Starts the hijack in reach: the vehicle and its crewman join the
    /// scripts' world where they are, and the entrance's script starts.
    fn begin(&mut self, game: &mut Game) {
        let (State::Ready(l), Some(c)) = (&mut self.state, self.near.clone()) else {
            return;
        };
        let Some(player) = &game.scene.player else {
            return;
        };
        let m = &mut l.mission;
        m.put_player(player.character.position.to_array());
        let Some(hero) = m.world().borrow().player else {
            return;
        };
        let world = game.scene.world.as_ref();
        let v = &game.scene.vehicles[c.vehicle];
        let Some(at) = world.and_then(|w| w.physics.vehicle_transform(v.id?)) else {
            return;
        };
        let origin = at.transform_point3(Vec3::ZERO);
        let v_top = v.spec.bounds.1.y;
        let forward = at.transform_vector3(Vec3::Z);
        let yaw = forward.x.atan2(forward.z);
        let (Some(vehicle), Some(crewman)) = (
            m.spawn(&c.template, origin.to_array()),
            m.spawn(&c.crew, origin.to_array()),
        ) else {
            eprintln!("hijack: could not spawn {} or {}", c.template, c.crew);
            return;
        };
        m.place(vehicle, origin.to_array(), yaw);
        // Every node of its model is a hardpoint the scripts may ask
        // for (the hatch the blast goes off at, `hp_seat_lt`).
        let model = v.spec.model;
        if let Some(source) = game.scene.source.as_mut() {
            for (name, local) in nodes(source, model) {
                m.set_hardpoint(vehicle, name, at.transform_point3(local).to_array());
            }
        }
        if let Some(p) = c.entrance.point {
            m.set_hardpoint(vehicle, p, c.at.to_array());
        }
        if m.start_hijack(hero, crewman, vehicle, c.entrance.script) {
            eprintln!("hijack: {} started", c.template);
            eprintln!(
                "hijack: hull {:.2?}, turret box {:.2?}",
                v.spec.hull, v.spec.turret_box
            );
            game.input_locked = true;
            self.lag = 0.0;
            let pistol_parent = game
                .scene
                .source
                .as_mut()
                .and_then(|source| parent_to_model(source, model, PISTOL));
            let vehicle_nodes = game
                .scene
                .source
                .as_mut()
                .and_then(|source| vehicle_nodes(source, model));
            // The crewman's struggle at the hatch, by the driver's seat.
            let set = l.clips.sets.get(&c.template.to_ascii_lowercase()).copied();
            let crew_loop = set.and_then(|s| {
                l.clips
                    .seat_clip(
                        "InVehicle",
                        "ActionHijackHijackeeBLoopA",
                        s,
                        m2_assets::name_hash("seat_driver"),
                    )
                    .map(|c| c.0)
            });
            self.active = Some(Active {
                pistol_parent,
                crewman,
                crew: c.crew.clone(),
                hijackee: None,
                pending: None,
                roof: None,
                hatch: None,
                crew_loop,
                crew_offset: None,
                lift_known: None,
                grips: Vec::new(),
                vehicle_nodes,
                vehicle_started: 0.0,
                vehicle_clip: None,
                lift: None,
                first: None,
                shown: None,
                vehicle: c.vehicle,
                hero,
                set,
                actions: Vec::new(),
                held: Vec::new(),
                flick: Flick::default(),
                clip: None,
                // Over the entrance at the top of the vehicle, where the
                // hijack plays out (the M1A2's driver's hatch is low on
                // the front deck, the fight on the turret).
                at: Vec3::new(
                    c.at.x,
                    at.transform_point3(Vec3::new(0.0, v_top, 0.0)).y + CAMERA_LIFT,
                    c.at.z,
                ),
            });
            // Her clips move her from here on (`down` keeps her own
            // moves and gaits from playing over them, and what she did on
            // foot stops: her weapon's pose would lie over the clips);
            // the camera looks across the vehicle from its side.
            if let Some(p) = &mut game.scene.player {
                p.down = true;
                p.off_foot();
                // From the entrance's side of the vehicle.
                let side = at.inverse().transform_point3(c.at).x.signum();
                let out = at.transform_vector3(Vec3::X * side);
                p.yaw = (-out.x).atan2(-out.z);
                p.pitch = CAMERA_PITCH;
            }
        } else {
            eprintln!("hijack: {}'s script did not start", c.template);
        }
        for line in m.take_log() {
            eprintln!("hijack: {line}");
        }
        let commands = m.take_commands();
        if !follow(
            &mut self.active,
            &mut self.scripts_shown,
            &l.clips,
            m,
            commands,
            game,
        ) {
            self.active = None;
            game.input_locked = false;
        }
    }
}

/// The pad as a player who answers every prompt at once would work it
/// (`--hijack-crew` with `--screenshot --drive`): presses the button a
/// press asks for, holds a hold's, taps (or alternates) nine times a
/// second. For testing.
#[derive(Default)]
pub struct Autopad {
    answered: Vec<(String, u32)>,
    tapped: f32,
    last: Option<u32>,
}

impl Autopad {
    /// Works `input` for the prompts up, `now` seconds in.
    pub fn answer(&mut self, prompts: &[Minigame], input: &mut Input, now: f32) {
        let action = |b: u32| PAD.iter().find(|p| p.1 == b).map(|p| p.0);
        // Let go of every key but a hold's.
        for (a, b) in PAD {
            let holding = prompts
                .iter()
                .any(|g| g.mode == "hold" && g.buttons.first() == Some(&b));
            if input.held(a) && !holding {
                input.button(a, false);
            }
        }
        let tap = now - self.tapped >= 1.0 / 9.0;
        for g in prompts {
            let first = g.buttons.first().copied().unwrap_or(0);
            let key = (g.mode.clone(), first);
            let fresh = !self.answered.contains(&key);
            if fresh {
                eprintln!(
                    "hijack: t={now:.1} prompt {} {:?} {:.2}s left",
                    g.mode, g.buttons, g.left
                );
                self.answered.push(key);
            }
            let button = match g.mode.as_str() {
                "press" if fresh => Some(first),
                "tap" if tap => Some(first),
                "alternate" if tap => {
                    let b = match (self.last, g.buttons.as_slice()) {
                        (Some(l), [a, b]) if l == *a => *b,
                        (_, [a, ..]) => *a,
                        _ => first,
                    };
                    self.last = Some(b);
                    Some(b)
                }
                "hold" if fresh => Some(first),
                _ => None,
            };
            if let Some(a) = button.and_then(action) {
                if g.mode != "press" {
                    self.tapped = now;
                }
                input.button(a, true);
            }
        }
        let open: Vec<(String, u32)> = prompts
            .iter()
            .map(|g| (g.mode.clone(), g.buttons.first().copied().unwrap_or(0)))
            .collect();
        self.answered.retain(|k| open.contains(k));
    }
}

impl Hijacks {
    /// Runs after the game's tick: while her hijack clip plays, she is
    /// where it carries her. Inferred: the people's hijack clips are cut
    /// in the vehicle's space (they start from its origin, facing its
    /// way), as the vehicle's own are.
    pub fn place(&self, game: &mut Game) {
        let Some(a) = &self.active else { return };
        // The vehicle's hijack clip moves its own parts (the hatch).
        if let (State::Ready(l), Some(clip), Some(nodes)) =
            (&self.state, &a.vehicle_clip, &a.vehicle_nodes)
            && let Some(v) = game.scene.vehicles.get_mut(a.vehicle)
        {
            let t = (l.mission.now() - a.vehicle_started) as f32;
            v.hijack_nodes = clip_nodes(nodes, clip, t);
        }
        let scene = &mut game.scene;
        let (Some(world), Some(p)) = (&scene.world, &mut scene.player) else {
            return;
        };
        p.chase_target = Some((a.at, CAMERA_DISTANCE));
        let Some(clip) = a.clip else { return };
        let Some(at) = scene.vehicles[a.vehicle]
            .id
            .and_then(|id| world.physics.vehicle_transform(id))
        else {
            return;
        };
        let Some((_, rig)) = &p.model else { return };
        let Some(c) = rig.clips.get(clip) else { return };
        let t = rig
            .animator
            .current()
            .filter(|cur| cur.0 == clip)
            .map_or(0.0, |cur| cur.1)
            .min(c.info.duration);
        let pose = c.sample(&rig.skeleton, t);
        let Some(root) = pose.0.get(rig.skeleton.root()).map(|l| l.translation) else {
            return;
        };
        let forward = rig
            .locomotion
            .as_ref()
            .map_or(m2_math::glam::Vec2::Y, |l| l.forward);
        let model_yaw = forward.x.atan2(forward.y);
        let ahead = at.transform_vector3(Vec3::Z);
        let yaw = ahead.x.atan2(ahead.z);
        let origin = at.transform_point3(Vec3::ZERO);
        let off = m2_math::glam::Quat::from_rotation_y(yaw)
            * Vec3::new(root.x - c.info.pin.x, 0.0, root.z - c.info.pin.y);
        p.facing = yaw + model_yaw;
        let lift = if a.clip == a.first {
            0.0
        } else {
            a.lift.unwrap_or(0.0)
        };
        p.character.position = origin + off + Vec3::Y * lift;
        // The camera follows her about the vehicle, close, at her chest
        // (her clip's pelvis, which climbs onto the hull with her) as
        // the footage's is.
        let pelvis = rig
            .skeleton
            .pelvis()
            .map_or(root.y, |i| rig.skeleton.model_matrices(&pose)[i].w_axis.y);
        p.chase_target = Some((
            p.character.position + Vec3::Y * (pelvis + CHEST),
            CAMERA_DISTANCE,
        ));
        // From her side of the hull and a little ahead, as the footage's
        // camera looks across her at the crewman (from the entrance's side
        // the turret hid her: round H20, playtest 8).
        if root.x.abs() > SIDE_SWITCH {
            let out = at
                .transform_vector3(Vec3::new(root.x.signum(), 0.0, CAMERA_AHEAD))
                .normalize_or(Vec3::X);
            p.yaw = (-out.x).atan2(-out.z);
        }
        p.character.velocity = Vec3::ZERO;
    }

    /// Seats the riders of the vehicles near her ([`Rider`]) in the
    /// scene's [`riders`](crate::scene::Scene::riders): every rider of
    /// every vehicle within [`RIDER_DISTANCE`] that isn't wrecked, but the
    /// driver of what she drives and the vehicle a hijack plays on (its
    /// crewman is out on foot). Their kinds load as people's do; until
    /// then they aren't drawn.
    fn seat_riders(&mut self, game: &mut Game, dt: f32) {
        let mut riders = std::mem::take(&mut game.scene.riders);
        let mut tally = RiderTally::default();
        let here = game.focus();
        let hers = game.scene.player.as_ref().and_then(|p| p.driving);
        let busy = self.active.as_ref().map(|a| a.vehicle);
        let driver_seat = m2_assets::name_hash("seat_driver");
        let mut wanted: Vec<(usize, u32, String)> = Vec::new();
        if let Some(world) = game.scene.world.as_ref() {
            for (i, v) in game.scene.vehicles.iter().enumerate() {
                if v.wrecked || v.crew.is_empty() || Some(i) == busy {
                    continue;
                }
                let Some(at) = v.id.and_then(|id| world.physics.vehicle_transform(id)) else {
                    continue;
                };
                if at.transform_point3(Vec3::ZERO).distance(here) > RIDER_DISTANCE {
                    continue;
                }
                for (seat, who) in &v.crew {
                    let hash = seat_hash(seat);
                    if Some(i) == hers && hash == driver_seat {
                        continue;
                    }
                    wanted.push((i, hash, who.clone()));
                }
            }
        }
        riders.retain(|r| {
            r.prop.is_some() || wanted.iter().any(|w| w.0 == r.vehicle && w.1 == r.seat)
        });
        for (i, hash, who) in wanted {
            tally.seats += 1;
            let v = &game.scene.vehicles[i];
            let vehicle_model = v.spec.model;
            let key = (vehicle_model, v.spec.label.to_ascii_lowercase(), hash);
            if !self.rider_seats.contains_key(&key) {
                let State::Ready(loaded) = &self.state else {
                    return;
                };
                let seat = find_seat(game, &mut self.points, &loaded.clips, i, Some(hash));
                eprintln!(
                    "rider: model {:08x} ({}) seat {hash:08x}: {seat:?}",
                    key.0, key.1
                );
                self.rider_seats.insert(key.clone(), seat);
            }
            let Some(seat) = self.rider_seats[&key] else {
                tally.no_seat += 1;
                continue;
            };
            if !seat.shown {
                tally.hidden += 1;
                continue;
            }
            let model = game.scene.world.as_ref().and_then(|w| {
                let db = &w.index.template_db;
                db.get(&who).and_then(|t| t.model)
            });
            let Some(kind) = game.scene.people.rider_kind(&who, model) else {
                tally.no_kind += 1;
                continue;
            };
            let place = riders.iter().position(|r| {
                r.prop.is_none() && r.vehicle == i && r.seat == hash && r.kind == kind
            });
            let index = match place {
                Some(p) => p,
                None => {
                    if let std::collections::hash_map::Entry::Vacant(e) =
                        self.rider_clips.entry(seat.clip)
                    {
                        e.insert(load(game, seat.clip));
                    }
                    let k = &game.scene.people.kinds[kind];
                    // A clip that won't bind to a kind's skeleton is tried
                    // once, not cloned and bound again every tick.
                    if self.rider_unbound.contains(&(seat.clip, kind)) {
                        tally.no_clip += 1;
                        continue;
                    }
                    let Some(bound) = self.rider_clips[&seat.clip]
                        .clone()
                        .and_then(|c| bind_rider(seat.clip, c, &k.rig.skeleton, &k.role))
                    else {
                        self.rider_unbound.insert((seat.clip, kind));
                        tally.no_clip += 1;
                        continue;
                    };
                    let mut rig = crate::scene::Rig {
                        clips: std::sync::Arc::new(vec![bound]),
                        locomotion: None,
                        animator: Default::default(),
                        cues: Vec::new(),
                        aim: None,
                        two_handed: false,
                        ..k.rig.clone()
                    };
                    rig.animator.play(0);
                    let look = (i, vehicle_model, hash);
                    let outfit = match self.rider_looks.get(&look) {
                        Some(o) => o.clone(),
                        None => {
                            let o = game.scene.people.dress_as(kind, &who);
                            self.rider_looks.insert(look, o.clone());
                            o
                        }
                    };
                    riders.retain(|r| !(r.prop.is_none() && r.vehicle == i && r.seat == hash));
                    riders.push(Rider {
                        vehicle: i,
                        prop: None,
                        guid: None,
                        seat: hash,
                        kind,
                        rig,
                        outfit,
                        frame: Mat4::IDENTITY,
                    });
                    riders.len() - 1
                }
            };
            let Some(at) = game
                .scene
                .world
                .as_ref()
                .zip(game.scene.vehicles[i].id)
                .and_then(|(w, id)| w.physics.vehicle_transform(id))
            else {
                continue;
            };
            let r = &mut riders[index];
            r.rig.advance(dt);
            r.frame = at * Mat4::from_translation(seat.at + seat_offset(&r.rig, 0));
            tally.drawn += 1;
        }
        // A vehicle gone (or its index reused) takes its riders' looks.
        let vehicles = &game.scene.vehicles;
        self.rider_looks
            .retain(|&(i, model, _), _| vehicles.get(i).is_some_and(|v| v.spec.model == model));
        game.scene.riders = riders;
        self.riders = tally;
        if tally.drawn != self.told_riders.drawn {
            eprintln!("riders: {tally:?}");
            self.told_riders = tally;
        }
    }

    /// Seats the riders of the layers' props near her
    /// ([`Scene::prop_crews`]) as [`Hijacks::seat_riders`] seats a
    /// vehicle's: on the seat the prop's template's `SeatLink` names, at
    /// its hardpoint on the prop's model, playing the tables' `InVehicle`
    /// `Idle` clip for the template's `VehicleAnimationSet`.
    ///
    /// From samples: the PMC HQ's interior (`HqInterior`, template
    /// `pmchq_interior`) has one `Rider` record of its own, Fiona Taylor
    /// in seat a651ff78, and its template one `SeatLink` for that seat,
    /// hardpoint 7ce3f665; the three recruits' interior pieces
    /// (`_pmcoutpost_interior_recruitheli`, `..mechanic`, `..jet`) each
    /// carry their recruit the same way. The scripts find a starter as the
    /// prop's first rider (`WifPmcInterior._EnableStarter`), and offer
    /// her `Talk` by where she is: where she sits goes back to them
    /// ([`Scene::prop_rider_spots`]).
    ///
    /// [`Scene::prop_crews`]: crate::scene::Scene::prop_crews
    /// [`Scene::prop_rider_spots`]: crate::scene::Scene::prop_rider_spots
    fn seat_prop_riders(&mut self, game: &mut Game, dt: f32) {
        let State::Ready(loaded) = &self.state else {
            return;
        };
        let here = game.focus();
        let mut riders = std::mem::take(&mut game.scene.riders);
        let mut wanted: Vec<(u32, u32, u32, String, Mat4, u32)> = Vec::new();
        for (g, crew) in &game.scene.prop_crews {
            let Some((p, _)) = game.scene.props.get(*g) else {
                continue;
            };
            if p.wrecked || p.position.distance(here) > RIDER_DISTANCE {
                continue;
            }
            let at = Mat4::from_rotation_translation(p.rotation, p.position);
            for (seat, rider, who) in crew {
                wanted.push((*g, *rider, seat_hash(seat), who.clone(), at, p.model));
            }
        }
        riders.retain(|r| {
            r.prop.is_none() || wanted.iter().any(|w| Some(w.0) == r.prop && w.2 == r.seat)
        });
        let mut spots = Vec::new();
        for (g, rider, hash, who, at, model) in wanted {
            let template = game.scene.props.get(g).map_or(0, |(p, _)| p.template);
            let key = (model, template, hash);
            if !self.prop_seats.contains_key(&key) {
                let seat =
                    find_prop_seat(game, &mut self.points, &loaded.clips, model, template, hash);
                eprintln!("rider: prop {g:#x} model {model:08x} seat {hash:08x}: {seat:?}");
                self.prop_seats.insert(key, seat);
            }
            let Some(seat) = self.prop_seats[&key] else {
                continue;
            };
            let model_of = game
                .scene
                .world
                .as_ref()
                .and_then(|w| w.index.template_db.get(&who).and_then(|t| t.model));
            let Some(kind) = game.scene.people.rider_kind(&who, model_of) else {
                continue;
            };
            let index = match riders
                .iter()
                .position(|r| r.prop == Some(g) && r.seat == hash && r.kind == kind)
            {
                Some(i) => i,
                None => {
                    if let std::collections::hash_map::Entry::Vacant(e) =
                        self.rider_clips.entry(seat.clip)
                    {
                        e.insert(load(game, seat.clip));
                    }
                    if self.rider_unbound.contains(&(seat.clip, kind)) {
                        continue;
                    }
                    let k = &game.scene.people.kinds[kind];
                    let Some(bound) = self.rider_clips[&seat.clip]
                        .clone()
                        .and_then(|c| bind_rider(seat.clip, c, &k.rig.skeleton, &k.role))
                    else {
                        self.rider_unbound.insert((seat.clip, kind));
                        continue;
                    };
                    let mut rig = crate::scene::Rig {
                        clips: std::sync::Arc::new(vec![bound]),
                        locomotion: None,
                        animator: Default::default(),
                        cues: Vec::new(),
                        aim: None,
                        two_handed: false,
                        ..k.rig.clone()
                    };
                    rig.animator.play(0);
                    let outfit = match self.prop_looks.get(&rider) {
                        Some(o) => o.clone(),
                        None => {
                            let o = game.scene.people.dress_as(kind, &who);
                            self.prop_looks.insert(rider, o.clone());
                            o
                        }
                    };
                    eprintln!(
                        "rider: {who} ({rider:#x}) sits on prop {g:#x}, clip {:08x}",
                        seat.clip
                    );
                    riders.retain(|r| !(r.prop == Some(g) && r.seat == hash));
                    riders.push(Rider {
                        vehicle: usize::MAX,
                        prop: Some(g),
                        guid: Some(rider),
                        seat: hash,
                        kind,
                        rig,
                        outfit,
                        frame: Mat4::IDENTITY,
                    });
                    riders.len() - 1
                }
            };
            let r = &mut riders[index];
            r.rig.advance(dt);
            r.frame = at * Mat4::from_translation(seat.at + seat_offset(&r.rig, 0));
            let ahead = r.frame.transform_vector3(Vec3::Z);
            spots.push((
                rider,
                r.frame.transform_point3(Vec3::ZERO),
                ahead.x.atan2(ahead.z),
            ));
        }
        game.scene.riders = riders;
        game.scene.prop_rider_spots = spots;
    }

    /// While she drives (and no hijack plays), she sits in the driver's
    /// seat, playing the clip the tables give that seat, turned and
    /// tilted with the vehicle. Runs after the game's tick, like
    /// [`place`](Self::place).
    pub fn seat(&mut self, game: &mut Game, dt: f32) {
        let State::Ready(loaded) = &self.state else {
            return;
        };
        dress(game, &mut self.dressed, &loaded.clips);
        self.seat_riders(game, dt);
        self.seat_prop_riders(game, dt);
        let State::Ready(loaded) = &self.state else {
            return;
        };
        if let Some(a) = &mut self.active {
            draw_hijackee(a, game, dt);
            return;
        }
        let Some(i) = game.scene.player.as_ref().and_then(|p| p.driving) else {
            if let Some(p) = &mut game.scene.player {
                p.seated = false;
                p.seat_frame = None;
            }
            return;
        };
        let Some(v) = game.scene.vehicles.get(i) else {
            return;
        };
        // By the template the seat comes from (its label names it): the
        // soft-top and gun M151s share a model, not their seats.
        let key = (v.spec.model, v.spec.label.to_ascii_lowercase());
        if !self.seats.contains_key(&key) {
            let seat = find_seat(game, &mut self.points, &loaded.clips, i, None);
            eprintln!("seat: model {:08x} ({}) {seat:?}", key.0, key.1);
            // Where the clip has her from its origin (pinned as a walk
            // is, her pelvis would go on the seat point).
            if let Some(index) = seat.and_then(|s| bind(game, s.clip))
                && let Some((_, rig)) = game.scene.player.as_ref().and_then(|p| p.model.as_ref())
            {
                let c = &rig.clips[index];
                eprintln!(
                    "seat: clip {:08x}: root starts at {:.2?} (x, z), pelvis {:.2} m up at {:.2?}",
                    c.hash,
                    c.info.root_start.to_array(),
                    c.info.hips,
                    (c.info.root_start - c.info.pin).to_array()
                );
            }
            self.seats.insert(key.clone(), seat);
        }
        // What she did on foot stops (Player::update, which would end it,
        // doesn't run while she drives).
        if let Some(p) = &mut game.scene.player {
            p.off_foot();
        }
        // Inside, under a hatch, she isn't seen: as the hijack scripts
        // left her, else as the vehicle's entrance has it (a DriverHatch:
        // inside).
        let shown = |s: &Seat| self.scripts_shown.get(&key).copied().unwrap_or(s.shown);
        let seated = self.seats[&key].filter(shown).and_then(|seat| {
            let index = bind(game, seat.clip)?;
            let scene = &mut game.scene;
            let world = scene.world.as_ref()?;
            let at = world.physics.vehicle_transform(scene.vehicles[i].id?)?;
            let p = scene.player.as_mut()?;
            let (_, rig) = p.model.as_mut()?;
            rig.animator.play(index);
            rig.animator.advance(dt);
            let forward = rig
                .locomotion
                .as_ref()
                .map_or(m2_math::glam::Vec2::Y, |l| l.forward);
            let ahead = at.transform_vector3(Vec3::Z);
            p.facing = ahead.x.atan2(ahead.z) + forward.x.atan2(forward.y);
            // Her clip's space is the vehicle's, its origin on the seat
            // (her model's axes the vehicle's, as the hijack clips'),
            // with the vehicle's whole turn.
            let frame = at * Mat4::from_translation(seat.at + seat_offset(rig, index));
            p.seat_frame = Some(frame);
            p.character.position = frame.transform_point3(Vec3::ZERO);
            p.character.velocity = Vec3::ZERO;
            Some(())
        });
        if let Some(p) = &mut game.scene.player {
            p.seated = seated.is_some();
            if !p.seated {
                p.seat_frame = None;
            }
        }
    }

    /// Where she is in the hijacked vehicle's space (x right, y up, z
    /// ahead), and the vehicle's model-space bounds: for checking her
    /// clips' placing.
    pub fn her_in_vehicle(&self, game: &Game) -> Option<(Vec3, (Vec3, Vec3), String)> {
        let a = self.active.as_ref()?;
        let v = game.scene.vehicles.get(a.vehicle)?;
        let at = game
            .scene
            .world
            .as_ref()?
            .physics
            .vehicle_transform(v.id?)?;
        let p = game.scene.player.as_ref()?;
        // Her clip, how far in, its root and her pelvis height as drawn.
        let mut body = String::new();
        if let Some((_, rig)) = &p.model
            && let Some((clip, t)) = rig.animator.current()
            && let Some(c) = rig.clips.get(clip)
        {
            let raw = c.sample(&rig.skeleton, t.min(c.info.duration));
            let root = raw.0.get(rig.skeleton.root()).map(|l| l.translation);
            let pelvis = rig.skeleton.pelvis().map(|i| {
                rig.skeleton
                    .model_matrices(&rig.animator.pose(&rig.skeleton, &rig.clips))[i]
                    .w_axis
                    .y
            });
            body = format!(
                "clip {:08x} t {t:.2}/{:.2} root {:.2?} pin {:.2?} pelvis {pelvis:.2?}",
                c.hash,
                c.info.duration,
                root.map(|r| r.to_array()),
                c.info.pin.to_array()
            );
        }
        Some((
            at.inverse().transform_point3(p.character.position),
            v.spec.bounds,
            body,
        ))
    }

    /// Whether the crewman of the hijack in reach is loaded to be drawn
    /// (asking for him if not). For testing: a player walks up to it.
    pub fn crew_in(&self, game: &mut Game) -> bool {
        self.near
            .as_ref()
            .is_some_and(|c| crew_kind(game, &c.crew).is_some())
    }

    /// Whether the scripts have loaded (or failed to).
    pub fn loading(&self) -> bool {
        matches!(self.state, State::Loading(_))
    }

    /// A hijack is under way.
    pub fn running(&self) -> bool {
        self.active.is_some()
    }

    /// She is `hero` from now on, whole at `max` health: the loaded
    /// scripts' player takes their name, labels and health
    /// ([`crate::mission::set_hero`]). Scripts still loading start as
    /// whoever she is then.
    pub fn set_hero(&mut self, hero: crate::hero::Hero, max: f32) {
        if let State::Ready(l) = &mut self.state {
            crate::mission::set_hero(&mut l.mission, hero, max, "hijacks");
        }
    }
}

/// Carries out the scripts' hijack commands and ends the actions due.
/// False once the hijack is over.
fn follow(
    active: &mut Option<Active>,
    scripts_shown: &mut HashMap<(u32, String), bool>,
    clips: &ClipTimes,
    mission: &mut Mission,
    commands: Vec<Command>,
    game: &mut Game,
) -> bool {
    let Some(a) = active else {
        return false;
    };
    let now = mission.now();
    // The vehicle's own clip started alongside, for actions whose clip
    // the tables don't give.
    let alongside = commands.iter().find_map(|c| match c {
        Command::Hijack(HijackCommand::PlayAnimation { clip, channel, .. })
            if channel == "hijack" =>
        {
            clips.lengths.get(&m2_assets::name_hash(clip)).copied()
        }
        _ => None,
    });
    let mut over = false;
    let mut started = false;
    for c in commands {
        if let Command::Hijack(HijackCommand::PlayAnimation { clip, channel, .. }) = &c
            && channel == "hijack"
        {
            a.vehicle_clip = load(game, m2_assets::name_hash(clip));
            // The hatch is thrown open in one of the vehicle's sections
            // (the scripts name them `..._section01_fb` to `06`): the
            // first that turns it gives its roof.
            if a.roof.is_none()
                && let Some(n) = &a.vehicle_nodes
            {
                let lower = clip.to_ascii_lowercase();
                let sections: Vec<String> = match lower.find("section") {
                    Some(at) => (1..=6)
                        .map(|k| format!("{}section{k:02}{}", &lower[..at], &lower[at + 9..]))
                        .collect(),
                    None => vec![lower.clone()],
                };
                a.hatch = sections.iter().find_map(|s| {
                    let c = load(game, m2_assets::name_hash(s))?;
                    hatch_at(n, &c)
                });
                a.roof = a.hatch.map(|h| h.y);
                eprintln!("hijack: the hatch's roof {:.2?} m up", a.roof);
            }
            // From her first clip at the hatch on (the climb up), she stands
            // on its roof as she does holding the pistol there.
            if a.lift_known.is_none()
                && let (Some(roof), Some(set)) = (a.roof, a.set)
                && let Some((hash, _)) = clips.clip("InVehicle", "ActionHijackHijackerDLoopA", set)
                && let Some(feet) = bind(game, hash).and_then(|i| her_feet(Some(i), game))
            {
                a.lift = Some(roof - feet);
                a.lift_known = Some(roof - feet);
                eprintln!(
                    "hijack: her clips {:.2} m up (her feet {feet:.2} m in her pistol loop {hash:08x}, on the hatch's roof)",
                    roof - feet
                );
            }
            a.vehicle_started = now;
            started = true;
        }
        // The hijack's blasts (the hatch blown open, a grenade): fire
        // and smoke only, not a blast that wrecks what is near; a
        // grenade's size (inferred).
        if let Command::Spawn { what, position, .. } = &c
            && what.to_ascii_lowercase().contains("explosion")
        {
            eprintln!("hijack: {what}");
            game.combat.effects.push((
                crate::combat::Effect::Fire {
                    at: Vec3::from(*position),
                    size: 2.0,
                },
                0.6,
                0.6,
            ));
            // The template the script spawns, its own effects.
            game.combat.show_fx(what, Vec3::from(*position));
        }
        let Command::Hijack(h) = c else {
            continue;
        };
        match h {
            HijackCommand::State {
                guid,
                state,
                action,
            } => {
                a.actions.retain(|x| x.0 != guid);
                // The crewman's, as the driver's seat has them (the tables
                // give the hijackee's actions by seat), else anyone's.
                let driver = m2_assets::name_hash("seat_driver");
                let clip = a.set.and_then(|set| {
                    (guid == a.crewman)
                        .then(|| clips.seat_clip(&state, &action, set, driver))
                        .flatten()
                        .or_else(|| clips.clip(&state, &action, set))
                });
                if guid == a.crewman && clip.is_none() {
                    eprintln!("hijack: no clip for the crewman's {state} {action}");
                }
                if guid == a.hero
                    && let Some((hash, _)) = clip
                {
                    a.clip = play(game, hash);
                    started = true;
                }
                if guid == a.crewman
                    && let Some((hash, _)) = clip
                {
                    let looping = action.to_ascii_lowercase().contains("loop");
                    hijackee(a, game, hash, looping);
                }
                if !action.eq_ignore_ascii_case("Idle") {
                    let own = clip.and_then(|c| c.1);
                    let seconds = own.or(alongside).unwrap_or(UNKNOWN_CLIP);
                    a.actions.push((guid, now + seconds));
                }
            }
            HijackCommand::Completed { hero } if hero == a.hero => {
                eprintln!("hijack: done");
                if let Some(p) = &mut game.scene.player {
                    p.down = false;
                }
                // Seen in the seat or not as the scripts left her (gone in
                // through the hatch, hidden).
                if let (Some(on), Some(v)) = (a.shown, game.scene.vehicles.get(a.vehicle)) {
                    let key = (v.spec.model, v.spec.label.to_ascii_lowercase());
                    eprintln!(
                        "seat: {}: the hijack scripts left her {}: so she is in its seat (over the seat's guess)",
                        key.1,
                        if on { "shown" } else { "hidden" }
                    );
                    scripts_shown.insert(key, on);
                }
                // The driver is out (the scripts remove or kill him).
                if let Some(v) = game.scene.vehicles.get_mut(a.vehicle) {
                    v.crew
                        .retain(|(seat, _)| !seat.to_ascii_lowercase().contains("driver"));
                }
                if let Some(p) = &mut game.scene.player {
                    p.driving = Some(a.vehicle);
                    p.landed = true;
                }
                over = true;
            }
            // Whether the scripts hide her (gone in through the hatch)
            // tells whether she is seen in the seat (taken when done).
            HijackCommand::Visible { guid, on } if guid == a.hero => {
                eprintln!(
                    "hijack: the scripts {} her",
                    if on { "show" } else { "hide" }
                );
                a.shown = Some(on);
            }
            HijackCommand::Visible { guid, on } if guid == a.crewman => {
                if let Some(h) = &mut a.hijackee {
                    h.shown = on;
                }
            }
            HijackCommand::AbortDone { hero } if hero == a.hero => {
                eprintln!("hijack: failed");
                if let Some(p) = &mut game.scene.player {
                    p.down = false;
                    p.chase_target = None;
                }
                over = true;
            }
            // A missed press shakes her pad (`Pg.Rumble(hijacker, s)`).
            HijackCommand::Rumble { guid, seconds } if guid == a.hero => {
                eprintln!("hijack: pad shakes {seconds:.1} s");
                game.rumble = game.rumble.max(f64::from(seconds));
            }
            _ => {}
        }
    }
    if a.first.is_none() {
        a.first = a.clip;
    }
    if started && let Some((gap, spread, across)) = lift(a, game) {
        eprintln!("hijack: hand to pistol {gap:.2} m, spread {spread:.2}, across {across:.2} m");
        if a.lift.is_none() && a.clip == a.first {
            a.lift = Some(gap - FIRST_GAP);
            eprintln!("hijack: her clips {:.2} m up", gap - FIRST_GAP);
        }
        // Holding the pistol (her hand keeps one height from it all
        // clip long), her hand is where it is: that height lifts her
        // clips from then on.
        // Holding the pistol at the hatch, she stands on the roof the
        // hatch is set in (the footage: on the turret, the crewman half
        // out of the hatch): her feet there lift her clips from then on.
        if a.clip != a.first
            && spread < GRIP_SPREAD
            && let Some(roof) = a.roof
            && let Some(feet) = her_feet(a.clip, game)
        {
            a.grips.push(roof - feet);
            let mut g = a.grips.clone();
            g.sort_by(f32::total_cmp);
            let lift = g[g.len() / 2];
            if a.lift != Some(lift) {
                eprintln!(
                    "hijack: her clips {lift:.2} m up (her feet {feet:.2} m on the hatch's roof {roof:.2} m, {} clips)",
                    g.len()
                );
            }
            a.lift = Some(lift);
        }
    }
    if started && let Some(g) = feet_over_hull(a, game) {
        eprintln!("hijack: her feet {g:.2} m over the hull under them");
    }
    let (done, left): (Vec<_>, Vec<_>) = a.actions.iter().partition(|x| x.1 <= now);
    a.actions = left;
    for (g, _) in done {
        mission.action_finished(g);
    }
    // Over, the vehicle's parts are its own again.
    if over && let Some(v) = game.scene.vehicles.get_mut(a.vehicle) {
        v.hijack_nodes.clear();
    }
    !over
}

/// Puts her on foot beside the nearest hijackable vehicle with a driver
/// whose name holds `want` (any, if `None`), on the side of its entrance;
/// the vehicle's index. For testing (`--hijack`).
pub fn beside(game: &mut Game, hijacks: &mut Hijacks, want: Option<&str>) -> Option<usize> {
    if hijacks.by_model.is_none() {
        hijacks.by_model = Some(hijackable(&game.scene.world.as_ref()?.index.template_db));
    }
    let by_model = hijacks.by_model.as_ref()?;
    let here = game.focus();
    let world = game.scene.world.as_ref()?;
    let (i, at) = game
        .scene
        .vehicles
        .iter()
        .enumerate()
        .filter(|(_, v)| !v.wrecked && by_model.contains_key(&v.spec.model))
        .filter(|(_, v)| driver(&v.crew).is_some() || hijacks.crew.is_some())
        .filter(|(_, v)| {
            want.is_none_or(|w| {
                (w == "helicopter" && v.spec.is_helicopter())
                    || v.spec.label.to_ascii_lowercase().contains(w)
            })
        })
        .filter_map(|(i, v)| Some((i, world.physics.vehicle_transform(v.id?)?)))
        .min_by(|a, b| {
            let d = |m: &m2_math::glam::Mat4| m.transform_point3(Vec3::ZERO).distance(here);
            d(&a.1).total_cmp(&d(&b.1))
        })?;
    let v = &game.scene.vehicles[i];
    let (lo, hi) = v.spec.bounds;
    let door = v.spec.door.unwrap_or(Vec3::ZERO);
    // Out from the hull on the entrance's side, a metre clear.
    let x = if door.x < 0.0 { lo.x - 1.0 } else { hi.x + 1.0 };
    let spot = at.transform_point3(Vec3::new(x, hi.y, door.z.clamp(lo.z, hi.z)));
    let p = game.scene.player.as_mut()?;
    p.driving = None;
    p.character.position = spot;
    p.character.velocity = Vec3::ZERO;
    p.landed = false;
    game.on_foot = true;
    eprintln!(
        "hijack: beside the {} ({}), crew {:?}",
        v.spec.label, by_model[&v.spec.model].0, v.crew
    );
    Some(i)
}

/// The vehicles' pistol prop (`bone_pistol`), which their hijack clips
/// move with her hand.
const PISTOL: u32 = 0xcea4_a847;

/// Clip `hash` from the archive.
fn load(game: &mut Game, hash: u32) -> Option<Clip> {
    let bytes = game
        .scene
        .source
        .as_mut()?
        .asset(hash, ANIMATION_TYPE)
        .ok()?;
    Clip::parse(&bytes).ok()
}

/// Her right hand's height below the pistol, metres, in her first clip
/// when that clip plays at the vehicle's origin: the pistol still rests
/// on the vehicle. Inferred from rounds H18 and H19: on both the M1A2
/// and the AMX30 the first clip's gap is 1.10-1.17 m more than the gap
/// while she holds the pistol (when it should be about none), and the
/// M1A2's clips, which fit its hull unlifted, give 1.08.
const FIRST_GAP: f32 = 1.1;

/// A clip whose hand-to-pistol height strays less than this, metres, has
/// her holding the pistol all through (round H20's loops: 0.06-0.12 m;
/// her clips reaching for it or struggling stray 0.5 m and more).
/// Inferred.
const GRIP_SPREAD: f32 = 0.25;

/// The crewman's clip `hash` bound to his kind's skeleton and playing
/// (looped, or once): he is drawn from now on, if his kind is in.
/// The kind the crewman of template `template` is drawn as, asking for
/// his model the first time (`None` till it is in). A template with no
/// model of its own and no people's kind by name (the testing aid's
/// `Mook`) is drawn as a plain VZ soldier (invented stand-in).
fn crew_kind(game: &mut Game, template: &str) -> Option<usize> {
    let model = game
        .scene
        .world
        .as_ref()
        .and_then(|w| w.index.template_db.get(template).and_then(|t| t.model));
    let people = &mut game.scene.people;
    people.rider_kind(template, model).or_else(|| {
        model
            .is_none()
            .then(|| people.kind_for("VZ Soldier"))
            .flatten()
    })
}

fn hijackee(a: &mut Active, game: &mut Game, hash: u32, looping: bool) {
    let Some(kind) = crew_kind(game, &a.crew) else {
        // His model is still loading: he takes this clip up once it is in.
        if a.pending.is_none() {
            eprintln!("hijack: the crewman ({}) isn't loaded yet", a.crew);
        }
        a.pending = Some((hash, looping, 0.0));
        return;
    };
    a.pending = None;
    let Some(clip) = load(game, hash) else {
        return;
    };
    let k = &game.scene.people.kinds[kind];
    let Some(bound) = bind_rider(hash, clip, &k.rig.skeleton, &k.role) else {
        eprintln!(
            "hijack: the crewman's clip {hash:08x} doesn't bind to {}",
            k.role
        );
        return;
    };
    let mut rig = crate::scene::Rig {
        clips: std::sync::Arc::new(vec![bound]),
        locomotion: None,
        animator: Default::default(),
        cues: Vec::new(),
        aim: None,
        two_handed: false,
        ..k.rig.clone()
    };
    if looping {
        rig.animator.play(0);
    } else {
        rig.animator.play_once(0);
    }
    let (shown, outfit) = match a.hijackee.take() {
        Some(h) if h.kind == kind => (h.shown, h.outfit),
        _ => (true, game.scene.people.dress_as(kind, &a.crew)),
    };
    // Where his clip has him (the vehicle's space, unlifted), for the log.
    let c = &rig.clips[0];
    let mid = c.sample(&rig.skeleton, c.info.duration * 0.5);
    let pelvis = rig
        .skeleton
        .pelvis()
        .map(|i| rig.skeleton.model_matrices(&mid)[i].w_axis.truncate());
    eprintln!(
        "hijack: the crewman ({}) plays {hash:08x}, his pelvis mid-clip at {:.2?}",
        game.scene.people.kinds[kind].role,
        pelvis.map(|p| p.to_array())
    );
    a.hijackee = Some(Hijackee {
        kind,
        outfit,
        rig,
        shown,
    });
}

/// Draws the crewman (unless the scripts hide him) where his clip has
/// him: cut in the vehicle's space as hers are, from its origin, lifted
/// as hers are.
fn draw_hijackee(a: &mut Active, game: &mut Game, dt: f32) {
    // His clip waiting on his model: taken up as far in as it has got.
    if let Some((hash, looping, waited)) = a.pending {
        a.pending = Some((hash, looping, waited + dt));
        if crew_kind(game, &a.crew).is_some() {
            hijackee(a, game, hash, looping);
            if let Some(h) = &mut a.hijackee {
                h.rig.animator.seek(waited + dt);
                eprintln!(
                    "hijack: the crewman is in, {:.1} s into his clip",
                    waited + dt
                );
            }
        }
    }
    let Some(h) = &mut a.hijackee else { return };
    h.rig.advance(dt);
    // His clips don't share hers' place in the vehicle (his pelvis plays
    // some 6.5 m behind the tank, M1A2 and AMX30 alike): in his struggle
    // loop his pelvis goes to the hatch, half out of it as the footage
    // has him.
    if a.crew_offset.is_none()
        && let (Some(hatch), Some(hash)) = (a.hatch, a.crew_loop)
        && let Some(clip) = load(game, hash)
    {
        let skeleton = &game.scene.people.kinds[h.kind].rig.skeleton;
        let pelvis = m2_engine::BoundClip::bind(hash, clip.clone(), skeleton)
            .or_else(|| m2_engine::BoundClip::bind_family(hash, clip, skeleton))
            .and_then(|c| {
                let i = skeleton.pelvis()?;
                let mut at: Vec<Vec3> = (0..8)
                    .map(|k| {
                        let pose = c.sample(skeleton, c.info.duration * k as f32 / 7.0);
                        skeleton.model_matrices(&pose)[i].w_axis.truncate()
                    })
                    .collect();
                at.sort_by(|p, q| p.y.total_cmp(&q.y));
                Some(at[at.len() / 2])
            });
        if let Some(p) = pelvis {
            a.crew_offset = Some(hatch - p);
            eprintln!(
                "hijack: the crewman's clips moved {:.2?} (his loop's pelvis {:.2?} to the hatch {:.2?})",
                (hatch - p).to_array(),
                p.to_array(),
                hatch.to_array()
            );
        }
    }
    let Some(offset) = a.crew_offset else {
        return;
    };
    if !h.shown {
        return;
    }
    let scene = &mut game.scene;
    let Some(at) = scene
        .world
        .as_ref()
        .zip(scene.vehicles.get(a.vehicle).and_then(|v| v.id))
        .and_then(|(w, id)| w.physics.vehicle_transform(id))
    else {
        return;
    };
    let Some(((_, t), c)) = h
        .rig
        .animator
        .current()
        .and_then(|cur| Some((cur, h.rig.clips.get(cur.0)?)))
    else {
        return;
    };
    let pose = c.sample(&h.rig.skeleton, t.min(c.info.duration));
    let Some(root) = pose.0.get(h.rig.skeleton.root()).map(|l| l.translation) else {
        return;
    };
    let off = Vec3::new(root.x - c.info.pin.x, 0.0, root.z - c.info.pin.y) + offset;
    scene.riders.push(Rider {
        vehicle: a.vehicle,
        seat: 0,
        kind: h.kind,
        rig: h.rig.clone(),
        outfit: h.outfit.clone(),
        frame: at * Mat4::from_translation(off),
        prop: None,
        guid: None,
    });
}

/// A vehicle model's nodes, for its hijack clip to move.
fn vehicle_nodes(
    source: &mut m2_engine::AssetSource<BufReader<File>>,
    model: u32,
) -> Option<VehicleNodes> {
    let data = source.model(model).ok()?;
    let m = data.parse().ok()?;
    Some(VehicleNodes {
        locals: m.nodes.iter().map(|n| Mat4::from(n.local)).collect(),
        parents: m.nodes.iter().map(|n| n.parent).collect(),
        names: m.nodes.iter().map(|n| n.name_hash).collect(),
    })
}

/// The model-space matrices of the nodes clip `clip` moves at `t`
/// seconds, and every node under them: (node, matrix).
fn clip_nodes(nodes: &VehicleNodes, clip: &Clip, t: f32) -> Vec<(usize, Mat4)> {
    let sample = clip.sample(t.clamp(0.0, clip.header.duration));
    let moved: Vec<(usize, Mat4)> = clip
        .track_nodes
        .iter()
        .enumerate()
        .filter_map(|(track, &name)| {
            let i = nodes.names.iter().position(|&n| n == name)?;
            let s = sample.get(track)?;
            let (p, r) = (s.translation, s.rotation);
            Some((
                i,
                Mat4::from_rotation_translation(
                    m2_math::glam::Quat::from_xyzw(r.x, r.y, r.z, r.w).normalize(),
                    Vec3::new(p.x, p.y, p.z),
                ),
            ))
        })
        .collect();
    posed_under(nodes, &moved)
}

/// The model-space matrices of nodes given new local transforms
/// (`moved`: node, local), and of every node under them; the rest are
/// left out (they stand as posed).
fn posed_under(nodes: &VehicleNodes, moved_locals: &[(usize, Mat4)]) -> Vec<(usize, Mat4)> {
    let mut locals = nodes.locals.clone();
    let mut moved = vec![false; locals.len()];
    for &(i, m) in moved_locals {
        if let Some(l) = locals.get_mut(i) {
            *l = m;
            moved[i] = true;
        }
    }
    let mut out = Vec::new();
    for i in 0..locals.len() {
        let (mut m, mut at, mut any) = (Mat4::IDENTITY, Some(i as u16), false);
        while let Some(n) = at {
            let n = usize::from(n);
            m = locals[n] * m;
            any |= moved[n];
            at = nodes.parents[n];
        }
        if any {
            out.push((i, m));
        }
    }
    out
}

/// How high her clip must play for her right hand to be where the
/// vehicle's clip puts the pistol: the median over the two clips' frames,
/// in height only, how far it strays (max - min), and the median distance
/// between them across (x, z, as her clip plays in the vehicle's space).
/// None without both clips or the prop.
fn lift(a: &Active, game: &Game) -> Option<(f32, f32, f32)> {
    let (vehicle, parent) = (a.vehicle_clip.as_ref()?, a.pistol_parent?);
    let track = vehicle.track_nodes.iter().position(|&n| n == PISTOL)?;
    // The pistol where the vehicle's clip has it, through every node
    // above it that the clip moves too; else under its parent at rest.
    let nodes = a.vehicle_nodes.as_ref();
    let node = nodes.and_then(|n| n.names.iter().position(|&h| h == PISTOL));
    let pistol_at = |t: f32| -> Option<Vec3> {
        if let (Some(nodes), Some(i)) = (nodes, node)
            && let Some(&(_, m)) = clip_nodes(nodes, vehicle, t).iter().find(|x| x.0 == i)
        {
            return Some(m.transform_point3(Vec3::ZERO));
        }
        let at = vehicle.sample(t).get(track)?.translation;
        Some(parent.transform_point3(Vec3::new(at.x, at.y, at.z)))
    };
    let (_, rig) = game.scene.player.as_ref()?.model.as_ref()?;
    let hers = rig.clips.get(a.clip?)?;
    let (hand, _) = crate::combat::hands(&rig.skeleton)?;
    let end = hers.info.duration.min(vehicle.header.duration);
    let (mut ups, mut across) = (Vec::new(), Vec::new());
    let mut t = 0.0;
    while t <= end {
        // Unpinned, her clip's root where it carries her about the
        // vehicle ([`Hijacks::place`]).
        let pose = hers.sample(&rig.skeleton, t);
        let hand_at = rig.skeleton.model_matrices(&pose)[hand].w_axis.truncate();
        let pistol = pistol_at(t)?;
        if (t - end * 0.5).abs() < STEP * 0.5 {
            eprintln!(
                "hijack: mid-clip hand {:.2?}, pistol {:.2?} (the vehicle's space)",
                hand_at.to_array(),
                pistol.to_array()
            );
        }
        ups.push(pistol.y - hand_at.y);
        across.push((pistol - hand_at).truncate_y().length());
        t += STEP;
    }
    ups.sort_by(f32::total_cmp);
    across.sort_by(f32::total_cmp);
    Some((
        *ups.get(ups.len() / 2)?,
        ups.last()? - ups.first()?,
        *across.get(across.len() / 2)?,
    ))
}

/// Where (the vehicle's space, at rest) the node its hijack clip
/// turns furthest: the hatch lid thrown open, hinged on the roof it is
/// set in. The props the clip carries (the pistol, the grenade) are left
/// out. None when the clip turns nothing.
fn hatch_at(nodes: &VehicleNodes, clip: &Clip) -> Option<Vec3> {
    let props = [PISTOL, m2_assets::name_hash("bone_grenade")];
    let (first, last) = (clip.sample(0.0), clip.sample(clip.header.duration));
    let quat = |r: m2_math::RawQuat| m2_math::glam::Quat::from_xyzw(r.x, r.y, r.z, r.w);
    let (track, _) = clip
        .track_nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| !props.contains(n))
        .filter_map(|(k, _)| {
            let (a, b) = (quat(first.get(k)?.rotation), quat(last.get(k)?.rotation));
            Some((k, a.normalize().angle_between(b.normalize())))
        })
        .filter(|&(_, turn)| turn > 0.2)
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    let i = nodes
        .names
        .iter()
        .position(|&n| n == clip.track_nodes[track])?;
    let mut at = Some(i as u16);
    let mut m = Mat4::IDENTITY;
    while let Some(n) = at {
        m = nodes.locals[usize::from(n)] * m;
        at = nodes.parents[usize::from(n)];
    }
    Some(m.w_axis.truncate())
}

/// Her feet's height in her clip as it plays (the vehicle's space, not
/// lifted): the lowest of her nodes, the median over the clip.
fn her_feet(clip: Option<usize>, game: &Game) -> Option<f32> {
    let (_, rig) = game.scene.player.as_ref()?.model.as_ref()?;
    let hers = rig.clips.get(clip?)?;
    let mut lows: Vec<f32> = (0..8)
        .filter_map(|k| {
            let pose = hers.sample(&rig.skeleton, hers.info.duration * k as f32 / 7.0);
            rig.skeleton
                .model_matrices(&pose)
                .iter()
                .map(|m| m.w_axis.y)
                .min_by(f32::total_cmp)
        })
        .collect();
    lows.sort_by(f32::total_cmp);
    lows.get(lows.len() / 2).copied()
}

/// How far her lowest point (her feet, mostly) is above the vehicle
/// under it, metres, at the middle of her clip as she is placed: the
/// median of a few frames of it that are over the vehicle. For the log:
/// standing on the hull it is about 0.
fn feet_over_hull(a: &Active, game: &Game) -> Option<f32> {
    let scene = &game.scene;
    let world = scene.world.as_ref()?;
    let id = scene.vehicles.get(a.vehicle)?.id?;
    let at = world.physics.vehicle_transform(id)?;
    let (_, rig) = scene.player.as_ref()?.model.as_ref()?;
    let hers = rig.clips.get(a.clip?)?;
    let lift = if a.clip == a.first {
        0.0
    } else {
        a.lift.unwrap_or(0.0)
    };
    let mut gaps: Vec<f32> = (1..8)
        .filter_map(|k| {
            let pose = hers.sample(&rig.skeleton, hers.info.duration * k as f32 / 8.0);
            let low = rig
                .skeleton
                .model_matrices(&pose)
                .iter()
                .map(|m| m.w_axis.truncate())
                .min_by(|p, q| p.y.total_cmp(&q.y))?;
            let foot = at.transform_point3(low + Vec3::Y * lift);
            let (d, hit) =
                world
                    .physics
                    .cast_ray_vehicle(foot + Vec3::Y * 5.0, -Vec3::Y, 10.0, None)?;
            (hit == Some(id)).then_some(5.0 - d)
        })
        .collect();
    gaps.sort_by(f32::total_cmp);
    gaps.get(gaps.len() / 2).copied()
}

/// Hides the parts each newly seen vehicle's template starts in a state
/// that shows nothing (its `OSMStateParameter`s): the soft-top jeep's gun
/// mount, the gun jeeps' canvas top. Vehicles are known by label, which
/// is their template's name when they have one.
fn dress(
    game: &mut Game,
    dressed: &mut std::collections::HashSet<(usize, u32)>,
    clips: &ClipTimes,
) {
    let scene = &mut game.scene;
    for (i, v) in scene.vehicles.iter_mut().enumerate() {
        if !dressed.insert((i, v.spec.model)) {
            continue;
        }
        let Some(parts) = clips.parts.get(&v.spec.label.to_ascii_lowercase()) else {
            continue;
        };
        let Some(source) = scene.source.as_mut() else {
            return;
        };
        let Ok(data) = source.model(v.spec.model) else {
            continue;
        };
        let Ok(m) = data.parse() else { continue };
        let Some(states) = &m.states else { continue };
        let mut off = vec![false; m.nodes.len()];
        for &(part, state) in parts {
            let Some(machine) = states.machines.iter().find(|x| {
                x.node
                    .and_then(|n| m.nodes.get(usize::from(n)))
                    .is_some_and(|n| n.name_hash == part)
            }) else {
                continue;
            };
            let shows = machine
                .states
                .iter()
                .find(|x| x.0 == state)
                .is_some_and(|x| x.1.iter().any(|c| c.1));
            if shows {
                continue;
            }
            // The part and everything under it.
            let Some(top) = machine.node else { continue };
            for (k, o) in off.iter_mut().enumerate() {
                let mut at = Some(k as u16);
                while let Some(n) = at {
                    if n == top {
                        *o = true;
                        break;
                    }
                    at = m.nodes.get(usize::from(n)).and_then(|x| x.parent);
                }
            }
        }
        if off.contains(&true) {
            eprintln!(
                "parts: the {} hides {} nodes ({parts:x?})",
                v.spec.label,
                off.iter().filter(|&&o| o).count()
            );
            v.model.hide(&off);
        }
    }
}

/// The driver's seat of vehicle `i`: its template (the first by name of
/// those with its model and an animation set), that template's
/// `seat_driver` (else first) `SeatLink` and the hardpoint it names, else
/// the model's driver's seat marker; and whether she is seen there
/// ([`Seat::shown`]: not when the vehicle's entrance is a `DriverHatch`).
/// `None` (she is not drawn) without a template with
/// an animation set, a seated clip or a place for the seat. Each says
/// why in the log.
///
/// With `wanted` (a rider's seat by name hash) that seat's own `SeatLink`
/// and hardpoint instead, with no stand-in: a seat the template doesn't
/// link, or whose hardpoint the model lacks, is `None`. A rider seen there
/// unless it is the driver's seat under a `DriverHatch`.
fn find_seat(
    game: &mut Game,
    points: &mut HashMap<(u32, u32), Option<Vec3>>,
    clips: &ClipTimes,
    i: usize,
    wanted: Option<u32>,
) -> Option<Seat> {
    let scene = &mut game.scene;
    let v = scene.vehicles.get(i)?;
    let (model, marker) = (v.spec.model, v.spec.seat);
    let label = v.spec.label.to_ascii_lowercase();
    let db = &scene.world.as_ref()?.index.template_db;
    // Its own template when its label names one (`M151 Softtop (GR)`),
    // else one with seats before one without (`M151 (Base)` has none).
    let of_model: Vec<(String, u32)> = db
        .infos
        .iter()
        .filter(|(_, info)| info.model == Some(model))
        .filter_map(|(&id, _)| Some((db.names.get(&id)?.to_ascii_lowercase(), id)))
        .collect();
    let mut names: Vec<&(String, u32)> = of_model
        .iter()
        .filter(|(name, _)| clips.sets.contains_key(name))
        .collect();
    names.sort_by_key(|(n, id)| (*n != label, !clips.seats.contains_key(n), n.clone(), *id));
    let Some((name, id)) = names.first().map(|e| (&e.0, e.1)) else {
        eprintln!("seat: {label}: no template of its model has a VehicleAnimationSet: not drawn");
        return None;
    };
    let set = clips.sets[name];
    let driver = m2_assets::name_hash("seat_driver");
    let links = clips.seats.get(name.as_str());
    let link = match wanted {
        Some(seat) => links.and_then(|l| l.iter().find(|s| s.0 == seat)),
        None => links.and_then(|l| l.iter().find(|s| s.0 == driver).or(l.first())),
    };
    if let (Some(seat), None) = (wanted, link) {
        eprintln!("seat: {name}: no SeatLink for seat {seat:08x}: its rider is not drawn");
        return None;
    }
    let marker = marker.filter(|_| wanted.is_none_or(|w| w == driver));
    let hardpoint = link
        .and_then(|l| l.1)
        .and_then(|h| point(points, scene.source.as_mut(), model, h));
    eprintln!("seat: {name} link {link:x?} hardpoint {hardpoint:?} marker {marker:?}");
    // Every SeatLink record of the template whole: which field (if any)
    // says the seat is enclosed is not known yet.
    for (k, row) in clips
        .seat_fields
        .get(name.as_str())
        .into_iter()
        .flatten()
        .enumerate()
    {
        let cells: Vec<String> = row
            .iter()
            .map(|&(field, word)| match clips.seat_templates.get(&word) {
                Some(t) => format!("{field:08x}={word:08x} ({t})"),
                None => format!("{field:08x}={word:08x}"),
            })
            .collect();
        eprintln!("seat: {name}: SeatLink {k}: {}", cells.join(", "));
    }
    // The tables' clip for the seat, and the row of the vehicle table
    // that gives it (the vehicle's own, its class's, or one for any).
    // The seat's own row first; one the seat-less query finds when that
    // gives no clip (a seat's row whose Animation is any).
    let Some(((clip, row), vehicles)) = clips.tables.as_ref().and_then(|(actions, vehicles)| {
        let seat = link.map(|l| l.0);
        let row = |seat| {
            animation_table::seat_clip_row(actions, vehicles, "InVehicle", "Idle", set, seat)
        };
        Some((row(seat).or_else(|| row(None))?, vehicles))
    }) else {
        eprintln!("seat: {name}: the tables give its seat no InVehicle Idle clip: not drawn");
        return None;
    };
    let cell = |column: &str| match vehicles.column(column).and_then(|c| vehicles.cell(row, c)) {
        Some(animation_table::ANY) => "any".to_owned(),
        Some(v) => format!("{v:08x}"),
        None => "-".to_owned(),
    };
    eprintln!(
        "seat: {name}: clip {clip:08x} from the vehicle table's row {row} (class {}, vehicle {}, seat {}; set {:08x?})",
        cell("VehicleClass"),
        cell("VehicleName"),
        cell("SeatName"),
        [set.0, set.1],
    );
    let Some(at) = hardpoint.or(marker) else {
        eprintln!("seat: {name}: no hardpoint or seat marker for the seat: not drawn");
        return None;
    };
    // The data's rule (the hijack scripts hiding her, which
    // [`Hijacks::seat`] takes, go over it): a driver got in through a
    // `DriverHatch` entrance is inside, out of sight. The PC footage of
    // the AMX30 shows her on top only while climbing in, hidden while
    // driving. Its own template's entrances, else those of another
    // template of its model.
    let hatch_point = m2_assets::name_hash("DriverHatch");
    let entrances = db
        .entrance_points
        .get(&id)
        .map(|p| (name.as_str(), p))
        .or_else(|| {
            of_model
                .iter()
                .find_map(|(n, t)| Some((n.as_str(), db.entrance_points.get(t)?)))
        });
    let hatch = entrances.is_some_and(|(_, p)| p.contains(&hatch_point))
        && link.is_none_or(|l| l.0 == driver);
    eprintln!(
        "seat: {name}: entrances {}: {}",
        entrances.map_or_else(
            || "none".to_owned(),
            |(of, p)| format!("{p:08x?} (of {of})")
        ),
        if hatch {
            "a DriverHatch: inside, she is not drawn while seated"
        } else {
            "no DriverHatch: she is drawn there"
        }
    );
    Some(Seat {
        clip,
        at,
        shown: !hatch,
    })
}

/// The seat `seat` (name hash) of a prop of model `model` and template
/// `template` (id), as [`find_seat`] finds a vehicle's: the template's
/// `SeatLink` for it gives the hardpoint on the model, its
/// `VehicleAnimationSet` the tables' `InVehicle` `Idle` clip. Always
/// shown (no entrances to hide it).
fn find_prop_seat(
    game: &mut Game,
    points: &mut HashMap<(u32, u32), Option<Vec3>>,
    clips: &ClipTimes,
    model: u32,
    template: u32,
    seat: u32,
) -> Option<Seat> {
    let scene = &mut game.scene;
    let db = &scene.world.as_ref()?.index.template_db;
    let name = db
        .names
        .get(&template)
        .or_else(|| db.names.get(&(template ^ 0x8000_0000)))?
        .to_ascii_lowercase();
    let Some(&set) = clips.sets.get(&name) else {
        eprintln!("seat: prop {name}: no VehicleAnimationSet: its rider is not drawn");
        return None;
    };
    let Some(&(_, hardpoint)) = clips
        .seats
        .get(&name)
        .and_then(|l| l.iter().find(|s| s.0 == seat))
    else {
        eprintln!("seat: prop {name}: no SeatLink for seat {seat:08x}: its rider is not drawn");
        return None;
    };
    let at = hardpoint.and_then(|h| point(points, scene.source.as_mut(), model, h));
    let clip = clips.tables.as_ref().and_then(|(actions, vehicles)| {
        let row = |seat| {
            animation_table::seat_clip_row(actions, vehicles, "InVehicle", "Idle", set, seat)
        };
        row(Some(seat)).or_else(|| row(None)).map(|(clip, _)| clip)
    });
    eprintln!(
        "seat: prop {name}: seat {seat:08x} hardpoint {hardpoint:08x?} at {at:?}, clip {clip:08x?}"
    );
    Some(Seat {
        clip: clip?,
        at: at?,
        shown: true,
    })
}

/// Where her seated loop (clip `clip`, an index into her rig's clips)
/// has its root now from the clip's origin, flat. The animator pins every
/// clip's pelvis over her model's origin, as a walk's must be; the seated
/// loops are cut with their origin on the seat (the viewer's, checked by
/// eye on the HMMWV, played them so), so the root goes back where the
/// clip has it, as [`Hijacks::place`] puts a hijack clip's.
fn seat_offset(rig: &crate::scene::Rig, clip: usize) -> Vec3 {
    let Some(c) = rig.clips.get(clip) else {
        return Vec3::ZERO;
    };
    let t = rig
        .animator
        .current()
        .filter(|cur| cur.0 == clip)
        .map_or(0.0, |cur| cur.1);
    let pose = c.sample_looped(&rig.skeleton, t);
    pose.0.get(rig.skeleton.root()).map_or(Vec3::ZERO, |l| {
        Vec3::new(
            l.translation.x - c.info.pin.x,
            0.0,
            l.translation.z - c.info.pin.y,
        )
    })
}

/// Clip `hash` on her rig (read from the archive the first time): its
/// index in her rig's clips.
fn bind(game: &mut Game, hash: u32) -> Option<usize> {
    let index = match game
        .scene
        .player
        .as_ref()?
        .model
        .as_ref()?
        .1
        .clips
        .iter()
        .position(|c| c.hash == hash)
    {
        Some(i) => i,
        None => {
            let clip = load(game, hash)?;
            let (_, rig) = game.scene.player.as_mut()?.model.as_mut()?;
            let bound = m2_engine::BoundClip::bind(hash, clip, &rig.skeleton)?;
            let clips = std::sync::Arc::make_mut(&mut rig.clips);
            clips.push(bound);
            clips.len() - 1
        }
    };
    Some(index)
}

/// A rider's seat clip on the skeleton of their kind (`role`, for the
/// log): every track on a bone, else (people's skeletons differ from the
/// heroes' in a few bones, as [`m2_engine::BoundClip::rebind`] has it)
/// most of them, the rest left out.
fn bind_rider(
    hash: u32,
    clip: Clip,
    skeleton: &m2_engine::Skeleton,
    role: &str,
) -> Option<m2_engine::BoundClip> {
    let (found, tracks) = m2_engine::BoundClip::tracks_found(&clip, skeleton);
    let bound = m2_engine::BoundClip::bind(hash, clip.clone(), skeleton)
        .or_else(|| m2_engine::BoundClip::bind_family(hash, clip, skeleton));
    if found < tracks {
        let how = if bound.is_some() {
            "bound without the rest"
        } else {
            "not bound"
        };
        eprintln!("rider: clip {hash:08x} on {role}: {found} of {tracks} tracks on a bone: {how}");
    }
    bound
}

/// Plays clip `hash` on her once; its index in her rig's clips.
fn play(game: &mut Game, hash: u32) -> Option<usize> {
    let index = bind(game, hash)?;
    let (_, rig) = game.scene.player.as_mut()?.model.as_mut()?;
    rig.animator.play_once(index);
    Some(index)
}

/// A seat's name hash: the name the population table gives
/// ([`m2_engine::population::seat_name`]), or its hash when that name is
/// unknown and so written in hex.
fn seat_hash(seat: &str) -> u32 {
    if seat.len() == 8
        && let Ok(h) = u32::from_str_radix(seat, 16)
    {
        return h;
    }
    m2_assets::name_hash(seat)
}

/// The template of whoever drives (the rider of the seat named for the
/// driver, `seat_driver`): the one a hijack throws out.
fn driver(crew: &[(String, String)]) -> Option<String> {
    crew.iter()
        .find(|(seat, _)| seat.to_ascii_lowercase().contains("driver"))
        .map(|(_, who)| who.clone())
}

/// Drops pad presses nothing waits on.
fn drain(input: &mut Input) {
    for (action, _) in &PAD[..4] {
        input.take_pressed(*action);
    }
}

/// The hijackable templates by model: of a model's several templates,
/// the first by name.
fn hijackable(db: &m2_engine::Templates) -> HashMap<u32, (String, Vec<HijackEntrance>)> {
    let mut named: Vec<(String, u32, Vec<HijackEntrance>)> = db
        .hijack
        .iter()
        .filter_map(|(&id, e)| {
            let (name, info) = db.by_id(id)?;
            Some((name.to_owned(), info.model?, e.clone()))
        })
        .collect();
    named.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = HashMap::new();
    for (name, model, e) in named {
        out.entry(model).or_insert((name, e));
    }
    out
}

/// A model's hardpoint (a node, by name hash), model space; read once.
fn point(
    cache: &mut HashMap<(u32, u32), Option<Vec3>>,
    source: Option<&mut m2_engine::AssetSource<BufReader<File>>>,
    model: u32,
    name: u32,
) -> Option<Vec3> {
    if let Some(&p) = cache.get(&(model, name)) {
        return p;
    }
    let source = source?;
    let p = source.model(model).ok().and_then(|data| {
        let m = data.parse().ok()?;
        m2_assets::vehicle::marker(&m, name).map(Vec3::from)
    });
    cache.insert((model, name), p);
    p
}

/// Every node of a model, by name hash, with where it is (model space).
/// The model space from the parent of `model`'s node named `node`.
fn parent_to_model(
    source: &mut m2_engine::AssetSource<BufReader<File>>,
    model: u32,
    node: u32,
) -> Option<Mat4> {
    let data = source.model(model).ok()?;
    let m = data.parse().ok()?;
    let n = m.nodes.iter().find(|n| n.name_hash == node)?;
    m.node_to_model(n.parent?)
}

pub(crate) fn nodes(
    source: &mut m2_engine::AssetSource<BufReader<File>>,
    model: u32,
) -> Vec<(u32, Vec3)> {
    let Ok(data) = source.model(model) else {
        return Vec::new();
    };
    let Ok(m) = data.parse() else {
        return Vec::new();
    };
    (0..m.nodes.len())
        .filter_map(|i| {
            let to_model = m.node_to_model(i as u16)?;
            Some((m.nodes[i].name_hash, to_model.w_axis.truncate()))
        })
        .collect()
}

trait Flat {
    fn truncate_y(self) -> m2_math::glam::Vec2;
}

impl Flat for Vec3 {
    fn truncate_y(self) -> m2_math::glam::Vec2 {
        m2_math::glam::Vec2::new(self.x, self.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seats_by_name_or_by_hex_hash() {
        assert_eq!(seat_hash("seat_driver"), 0xf8b8_325f);
        assert_eq!(seat_hash("d0559110"), 0xd055_9110);
        assert_eq!(
            seat_hash(&m2_engine::population::seat_name(0xf8b8_325f)),
            0xf8b8_325f
        );
    }

    #[test]
    fn the_scripts_hud_calls_are_kept_with_their_nils() {
        let lua = mlua::Lua::new();
        lua.load("MrxGuiManager = { ToggleHud = function(_, on) toggled = on end }")
            .exec()
            .unwrap();
        lua.load(GUI_CALLS).exec().unwrap();
        // The scripts import the game's own module over it; kept again.
        lua.load("MrxGuiHudActionHijack = { ShowButton = function() shown = true end }")
            .exec()
            .unwrap();
        lua.load(GUI_CALLS).exec().unwrap();
        lua.load(GUI_CALLS).exec().unwrap();
        lua.load(
            r#"MrxGuiHudActionHijack.ShowButton(1, 7, 0.75, nil, 0.5)
            MrxGuiManager.ToggleHud(1, false, "hijack")"#,
        )
        .exec()
        .unwrap();
        // The game's own functions still run.
        assert!(!lua.globals().get::<bool>("toggled").unwrap());
        assert!(lua.globals().get::<bool>("shown").unwrap());
        let calls: mlua::Table = lua.globals().get("__m2_gui_calls").unwrap();
        // Once each, though wrapped again.
        assert_eq!(calls.raw_len(), 2);
        let show: mlua::Table = calls.get(1).unwrap();
        assert_eq!(show.get::<String>(2).unwrap(), "ShowButton");
        assert_eq!(show.get::<i64>("n").unwrap(), 5);
        assert!(show.get::<mlua::Value>(6).unwrap().is_nil());
        assert_eq!(show.get::<f64>(7).unwrap(), 0.5);
        let toggle: mlua::Table = calls.get(2).unwrap();
        assert_eq!(toggle.get::<String>(1).unwrap(), "MrxGuiManager");
        assert_eq!(toggle.get::<String>(5).unwrap(), "hijack");
    }

    #[test]
    fn the_vehicle_clip_moves_the_hatch_and_what_hangs_on_it() {
        // Hull (0), hatch on it (1), handle on the hatch (2), gun (3).
        let t = |x: f32, y: f32| Mat4::from_translation(Vec3::new(x, y, 0.0));
        let nodes = VehicleNodes {
            locals: vec![Mat4::IDENTITY, t(0.0, 2.0), t(0.5, 0.0), t(0.0, 1.0)],
            parents: vec![None, Some(0), Some(1), Some(0)],
            names: vec![1, 2, 3, 4],
        };
        // The hatch thrown up a metre.
        let posed = posed_under(&nodes, &[(1, t(0.0, 3.0))]);
        let at: Vec<(usize, Vec3)> = posed
            .iter()
            .map(|&(i, m)| (i, m.transform_point3(Vec3::ZERO)))
            .collect();
        assert_eq!(
            at,
            vec![(1, Vec3::new(0.0, 3.0, 0.0)), (2, Vec3::new(0.5, 3.0, 0.0))],
            "the hatch and its handle move; the hull and gun stand as posed"
        );
    }

    #[test]
    fn the_pad_numbers_match_the_scripts() {
        for (name, number) in m2_game::hijack::BUTTONS {
            let ours = PAD.iter().find(|(_, n)| *n == number);
            assert!(ours.is_some(), "{name} ({number}) has no key");
        }
    }

    /// The Scorpion's rounds: press right (8), then tap down (6); a hold
    /// and an alternate of the face buttons; a stick push no key names.
    #[test]
    fn the_keyboard_hint_names_the_keys_the_arrows_mean() {
        let keys = crate::input::Bindings::default();
        let game = |mode: &str, buttons: &[u32]| Minigame {
            mode: mode.into(),
            buttons: buttons.to_vec(),
            left: 2.0,
            hold: None,
        };
        let hint = |mode: &str, buttons: &[u32]| key_hint(&game(mode, buttons), &keys);
        assert_eq!(
            hint("press", &[8]).as_deref(),
            Some("L or flick the mouse right")
        );
        assert_eq!(
            hint("tap", &[6]).as_deref(),
            Some("K or flick the mouse down")
        );
        assert_eq!(hint("hold", &[5]).as_deref(), Some("hold I"));
        assert_eq!(
            hint("alternate", &[7, 8]).as_deref(),
            Some("J and L in turn")
        );
        assert_eq!(hint("press", &[12]).as_deref(), Some("S"));
        assert_eq!(hint("press", &[]), None);
    }

    /// A flick spread over frames at 165 Hz (12 counts a frame, 40 crossed
    /// on the fourth) presses once, whatever any one frame holds; a drift of
    /// the same counts over a second presses nothing.
    #[test]
    fn a_flick_adds_up_across_frames_and_presses_once() {
        let tick = 1.0 / 165.0;
        let mut f = Flick::default();
        let mut pressed = Vec::new();
        for _ in 0..5 {
            if let Some((b, counts, _)) = f.feed(0.0, 12.0, tick) {
                pressed.push((b, counts));
            }
        }
        assert_eq!(pressed, vec![(6, 48.0)]);
        // Right, and up, by their larger axis.
        assert_eq!(f.feed(50.0, 10.0, tick).map(|p| p.0), Some(8));
        assert_eq!(f.feed(-10.0, -50.0, tick).map(|p| p.0), Some(5));
        let mut slow = Flick::default();
        let drift: Vec<_> = (0..165).filter_map(|_| slow.feed(0.0, 0.4, tick)).collect();
        assert!(drift.is_empty(), "{drift:?}");
    }
}
