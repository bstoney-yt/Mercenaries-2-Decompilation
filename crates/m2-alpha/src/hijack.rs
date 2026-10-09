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
//! Not yet: the hero's, crewman's and vehicle's clips are timed but not
//! shown. The scripts' own calls to the HUD (the buttons to press) go to
//! the game's HUD ([`Hijacks::take_gui_calls`]).
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
/// The camera during a hijack: this far from the entrance, looking at it
/// across the vehicle and a little down. Inferred from the scripts'
/// `Camera.SetFocusParams(cam, 0, 0, 0.1, 4, 6, 1)` (read as a 4-6 m
/// distance); the on-foot camera would sit inside the hull.
const CAMERA_DISTANCE: f32 = 6.0;
const CAMERA_PITCH: f32 = -0.5;
/// How far above the vehicle's top (later her feet) the camera looks,
/// metres.
const CAMERA_LIFT: f32 = 1.0;
/// How long an action lasts when its clip is unknown, seconds.
const UNKNOWN_CLIP: f64 = 2.0;
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
            self.by_model = Some(hijackable(&world.index.template_db));
        }
        if self.active.is_some() {
            self.pad(input);
        } else {
            drain(input);
            self.near = self.find(game);
            if self.near.is_some() && input.take_pressed(Action::Use) {
                self.begin(game);
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
    fn pad(&mut self, input: &mut Input) {
        let (State::Ready(l), Some(a)) = (&mut self.state, &mut self.active) else {
            return;
        };
        for (action, button) in PAD {
            if input.take_pressed(action) {
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
            game.input_locked = true;
            self.lag = 0.0;
            let pistol_parent = game
                .scene
                .source
                .as_mut()
                .and_then(|source| parent_to_model(source, model, PISTOL));
            self.active = Some(Active {
                pistol_parent,
                vehicle_clip: None,
                lift: None,
                first: None,
                shown: None,
                vehicle: c.vehicle,
                hero,
                set: l.clips.sets.get(&c.template.to_ascii_lowercase()).copied(),
                actions: Vec::new(),
                held: Vec::new(),
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
        // The camera follows her about the vehicle.
        p.chase_target = Some((
            p.character.position + Vec3::Y * CAMERA_LIFT,
            CAMERA_DISTANCE,
        ));
        p.character.velocity = Vec3::ZERO;
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
        if self.active.is_some() {
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
            let seat = find_seat(game, &mut self.points, &loaded.clips, i);
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
                let clip = a.set.and_then(|set| clips.clip(&state, &action, set));
                if guid == a.hero
                    && let Some((hash, _)) = clip
                {
                    a.clip = play(game, hash);
                    started = true;
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
            HijackCommand::AbortDone { hero } if hero == a.hero => {
                eprintln!("hijack: failed");
                if let Some(p) = &mut game.scene.player {
                    p.down = false;
                    p.chase_target = None;
                }
                over = true;
            }
            _ => {}
        }
    }
    if a.first.is_none() {
        a.first = a.clip;
    }
    if started && let Some((gap, spread)) = lift(a, game) {
        eprintln!("hijack: hand to pistol {gap:.2} m, spread {spread:.2}");
        if a.lift.is_none() && a.clip == a.first {
            a.lift = Some(gap - FIRST_GAP);
            eprintln!("hijack: her clips {:.2} m up", gap - FIRST_GAP);
        }
    }
    let (done, left): (Vec<_>, Vec<_>) = a.actions.iter().partition(|x| x.1 <= now);
    a.actions = left;
    for (g, _) in done {
        mission.action_finished(g);
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
        .filter(|(_, v)| want.is_none_or(|w| v.spec.label.to_ascii_lowercase().contains(w)))
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

/// How high her clip must play for her right hand to be where the
/// vehicle's clip puts the pistol: the median over the two clips' frames,
/// in height only, and how far it strays (max - min). None without both
/// clips or the prop.
fn lift(a: &Active, game: &Game) -> Option<(f32, f32)> {
    let (vehicle, parent) = (a.vehicle_clip.as_ref()?, a.pistol_parent?);
    let track = vehicle.track_nodes.iter().position(|&n| n == PISTOL)?;
    let (_, rig) = game.scene.player.as_ref()?.model.as_ref()?;
    let hers = rig.clips.get(a.clip?)?;
    let (hand, _) = crate::combat::hands(&rig.skeleton)?;
    let end = hers.info.duration.min(vehicle.header.duration);
    let mut ups = Vec::new();
    let mut t = 0.0;
    while t <= end {
        let mut pose = hers.sample(&rig.skeleton, t);
        pose.strip_root_motion(rig.skeleton.root(), hers.info.pin);
        let hand_y = rig.skeleton.model_matrices(&pose)[hand].w_axis.y;
        let at = vehicle.sample(t).get(track)?.translation;
        let pistol = parent.transform_point3(Vec3::new(at.x, at.y, at.z));
        ups.push(pistol.y - hand_y);
        t += STEP;
    }
    ups.sort_by(f32::total_cmp);
    Some((*ups.get(ups.len() / 2)?, ups.last()? - ups.first()?))
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
fn find_seat(
    game: &mut Game,
    points: &mut HashMap<(u32, u32), Option<Vec3>>,
    clips: &ClipTimes,
    i: usize,
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
    let link = links.and_then(|l| l.iter().find(|s| s.0 == driver).or(l.first()));
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
    let hatch = entrances.is_some_and(|(_, p)| p.contains(&hatch_point));
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

/// Plays clip `hash` on her once; its index in her rig's clips.
fn play(game: &mut Game, hash: u32) -> Option<usize> {
    let index = bind(game, hash)?;
    let (_, rig) = game.scene.player.as_mut()?.model.as_mut()?;
    rig.animator.play_once(index);
    Some(index)
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

fn nodes(source: &mut m2_engine::AssetSource<BufReader<File>>, model: u32) -> Vec<(u32, Vec3)> {
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
    fn the_pad_numbers_match_the_scripts() {
        for (name, number) in m2_game::hijack::BUTTONS {
            let ours = PAD.iter().find(|(_, n)| *n == number);
            assert!(ours.is_some(), "{name} ({number}) has no key");
        }
    }
}
