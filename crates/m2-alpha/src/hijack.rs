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
//! shown. The prompts are for the HUD to draw ([`Hijacks::view`]).
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
use m2_engine::Install;
use m2_engine::hijack::HijackEntrance;
use m2_game::api::Command;
use m2_game::hijack::{ClipTimes, HijackCommand, Minigame};
use m2_game::mission::{GameData, Mission};
use m2_math::glam::Vec3;

use crate::game::Game;
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

    /// Runs before the game's own tick: takes the use key when a hijack
    /// is in reach, and the pad's buttons while one is under way.
    pub fn tick(&mut self, game: &mut Game, input: &mut Input, dt: f32) {
        if let State::Loading(rx) = &self.state {
            match rx.try_recv() {
                Ok(Ok((data, clips))) => match Mission::new(data, crate::mission::HERO, [0.0; 3]) {
                    Ok(mission) => {
                        eprintln!("hijacks: scripts loaded");
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
            if !follow(&mut self.active, &l.clips, &mut l.mission, commands, game) {
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
        if let Some(p) = c.entrance.point {
            m.set_hardpoint(vehicle, p, c.at.to_array());
        }
        if m.start_hijack(hero, crewman, vehicle, c.entrance.script) {
            eprintln!("hijack: {} started", c.template);
            game.input_locked = true;
            self.lag = 0.0;
            self.active = Some(Active {
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
            // moves and gaits from playing over them); the camera looks
            // across the vehicle from its side.
            if let Some(p) = &mut game.scene.player {
                p.down = true;
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
        if !follow(&mut self.active, &l.clips, m, commands, game) {
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
        p.character.position = origin + off;
        // The camera follows her about the vehicle.
        p.chase_target = Some((
            p.character.position + Vec3::Y * CAMERA_LIFT,
            CAMERA_DISTANCE,
        ));
        p.character.velocity = Vec3::ZERO;
    }

    /// Whether the scripts have loaded (or failed to).
    pub fn loading(&self) -> bool {
        matches!(self.state, State::Loading(_))
    }

    /// A hijack is under way.
    pub fn running(&self) -> bool {
        self.active.is_some()
    }
}

/// Carries out the scripts' hijack commands and ends the actions due.
/// False once the hijack is over.
fn follow(
    active: &mut Option<Active>,
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
    for c in commands {
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

/// Plays clip `hash` on her (read from the archive the first time); its
/// index in her rig's clips.
fn play(game: &mut Game, hash: u32) -> Option<usize> {
    use m2_assets::animation::{ANIMATION_TYPE, Clip};
    let scene = &mut game.scene;
    let (_, rig) = scene.player.as_mut()?.model.as_mut()?;
    let index = match rig.clips.iter().position(|c| c.hash == hash) {
        Some(i) => i,
        None => {
            let bytes = scene.source.as_mut()?.asset(hash, ANIMATION_TYPE).ok()?;
            let clip = Clip::parse(&bytes).ok()?;
            let bound = m2_engine::BoundClip::bind(hash, clip, &rig.skeleton)?;
            let clips = std::sync::Arc::make_mut(&mut rig.clips);
            clips.push(bound);
            clips.len() - 1
        }
    };
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
    fn the_pad_numbers_match_the_scripts() {
        for (name, number) in m2_game::hijack::BUTTONS {
            let ours = PAD.iter().find(|(_, n)| *n == number);
            assert!(ours.is_some(), "{name} ({number}) has no key");
        }
    }
}
