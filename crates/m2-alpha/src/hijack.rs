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
//! Not yet: the hero's, crewman's and vehicle's clips are timed but not
//! shown, and the island's vehicles have no crews, so only `--hijack-crew`
//! (a testing aid: every hijackable vehicle is crewed by that template)
//! lets one be hijacked. The prompts are for the HUD to draw
//! ([`Hijacks::view`]).
//!
//! Inferred: she must stand within the entrance's radius of its hardpoint
//! plus [`SLACK`], measured flat (a tank's hatch radius is 1 m, on top of
//! the hull).

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
/// Reach beyond the entrance's radius, metres, measured flat.
pub const SLACK: f32 = 2.0;
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
    /// A testing aid: every hijackable vehicle is crewed by this human
    /// template (`--hijack-crew`).
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
        let crew = self.crew.clone()?;
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
                let flat = (spot - here).truncate_y();
                let d = flat.length();
                if d <= e.radius + SLACK && best.as_ref().is_none_or(|b| d < b.0) {
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
            });
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
                if !action.eq_ignore_ascii_case("Idle") {
                    let own = a
                        .set
                        .and_then(|set| clips.clip(&state, &action, set))
                        .and_then(|c| c.1);
                    let seconds = own.or(alongside).unwrap_or(UNKNOWN_CLIP);
                    a.actions.push((guid, now + seconds));
                }
            }
            HijackCommand::Completed { hero } if hero == a.hero => {
                eprintln!("hijack: done");
                if let Some(p) = &mut game.scene.player {
                    p.driving = Some(a.vehicle);
                    p.landed = true;
                }
                over = true;
            }
            HijackCommand::AbortDone { hero } if hero == a.hero => {
                eprintln!("hijack: failed");
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
