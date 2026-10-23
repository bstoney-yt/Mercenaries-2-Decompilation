//! The action hijacks the contract scripts stage on vehicles of their own,
//! played in the contract's own scripts.
//!
//! PmcCon004 ends on one: Solano flees in the `Mi35 (Solano Hijack)` an
//! `MrxTaskObjectiveEnterVehicle` names, and the contract waits on the
//! `SolanoHijackComplete` script event, which only the hijack's script
//! (`hijack_mi35_solano`, the `Mi35 (Solano)` template's entrance
//! `Vehicle Entrance (Action Hijack) (Mi35 Solano)`) posts, from its
//! `EndContract` once the last of its sections is passed. Boarding the
//! helicopter as a passenger (round 10) completed the objective and
//! nothing followed. From the scripts.
//!
//! Unlike [`crate::hijack`], whose scripts run in a host of their own on a
//! copy of one of her vehicles, this one runs in the mission's: its script
//! finds the helicopter, Solano and the props by name
//! (`Pg.GetGuidByName("Mi35 (Solano Hijack)")`, `Vehicle.GetRiders(mi35,
//! "p")`, `"Action Hijack Prop (Pistol)"`), opens the bunker's doors and
//! posts the contract's event, all things of the contract's world. So the
//! helicopter is taken where it stands: the use key at it starts
//! `StartHijack(her, its first rider, "d", it)`, its prompts are answered
//! on the pad ([`RideHijack::press`]), and each character's action lasts
//! as long as the vehicle's clip started with it, else [`UNKNOWN_CLIP`]
//! (inferred: the alpha has no clip tables for this host). Nothing of it
//! is drawn but her standing by it.

use m2_assets::animation::{ANIMATION_TYPE, Clip};
use m2_engine::hijack::HijackEntrance;
use m2_game::api::Command;
use m2_game::hijack::{HijackCommand, Minigame};
use m2_game::mission::{Mission, PLAYER};

use crate::game::Game;

/// How long a character's action lasts when no clip of the vehicle's
/// started with it, seconds (as [`crate::hijack`]'s; invented).
const UNKNOWN_CLIP: f64 = 2.0;
/// The longest a hijack may run before it is given up, seconds. The
/// Mi35's eleven sections run some 60 s (their clips' lengths); invented.
const GIVE_UP: f64 = 240.0;

/// A hijack under way in the mission's scripts.
pub struct RideHijack {
    /// The vehicle, as the scripts know it.
    vehicle: u32,
    /// When it started, the scripts' time.
    started: f64,
    /// Characters' actions and when they end (the scripts' time).
    actions: Vec<(u32, f64)>,
    /// Pad buttons held, as the scripts were last told.
    held: Vec<u32>,
}

/// The hijack entrance of the scripts' object `g`, by its template (the
/// one its layer placed it as: `Mi35 (Solano)` for the `Mi35 (Solano
/// Hijack)`, round 5 here finding none by the object's own name), else
/// by its name, if it has one.
pub fn entrance(game: &Game, mission: &Mission, g: u32) -> Option<(String, HijackEntrance)> {
    let world = game.scene.world.as_ref()?;
    let db = &world.index.template_db;
    let own = {
        let w = mission.world().borrow();
        let n = &w.objects.get(&g)?.name;
        n.rfind(" 0x").map_or(n.as_str(), |i| &n[..i]).to_owned()
    };
    let names = [
        mission.template_of(g),
        world.index.state_vehicles.get(&g).map(|t| t.name.clone()),
        Some(own.clone()),
    ];
    let found = names.iter().flatten().find_map(|name| {
        db.ids(name)
            .into_iter()
            .find_map(|id| Some((db.by_id(id)?.0.to_owned(), *db.hijack.get(&id)?.first()?)))
    });
    if found.is_none() {
        eprintln!("hijack: {own} ({g:#x}) has no action hijack ({names:?})");
    }
    found
}

impl RideHijack {
    /// Starts the hijack of the scripts' vehicle `g` through `entrance`,
    /// its first rider the hijackee. None if the script refused.
    pub fn start(
        mission: &mut Mission,
        g: u32,
        template: &str,
        entrance: &HijackEntrance,
    ) -> Option<Self> {
        let crewman = {
            let mut w = mission.world().borrow_mut();
            // Seated in it by the scripts (`Vehicle.Enter(mi35, hero, "d")`,
            // which is how PmcCon004 starts it), she is not in it until the
            // hijack is done (inferred).
            if let Some(her) = w.objects.get_mut(&PLAYER)
                && her.vehicle == Some(g)
            {
                her.vehicle = None;
            }
            let mut riders: Vec<u32> = w
                .objects
                .iter()
                .filter(|(r, o)| **r != PLAYER && o.alive && o.vehicle == Some(g))
                .map(|(&r, _)| r)
                .collect();
            riders.sort_unstable();
            riders.first().copied()
        };
        let Some(crewman) = crewman else {
            eprintln!("hijack: {template} ({g:#x}) has nobody aboard to hijack it from");
            return None;
        };
        if !mission.start_hijack(PLAYER, crewman, g, entrance.script) {
            eprintln!(
                "hijack: {template} ({g:#x}): script {:08x} did not start",
                entrance.script
            );
            return None;
        }
        eprintln!(
            "hijack: {template} ({g:#x}) started in the mission's scripts, script {:08x}, from {crewman:#x}",
            entrance.script
        );
        Some(Self {
            vehicle: g,
            started: mission.now(),
            actions: Vec::new(),
            held: Vec::new(),
        })
    }

    /// The prompts waiting on her pad.
    pub fn prompts(mission: &Mission) -> Vec<Minigame> {
        mission.minigames()
    }

    /// Takes what the scripts asked this tick: a character's keyed action
    /// is timed, and its end reported; the hijack's end ends it. False
    /// once it is over.
    pub fn follow(&mut self, game: &mut Game, mission: &mut Mission, commands: &[Command]) -> bool {
        let alongside = commands.iter().find_map(|c| match c {
            Command::Hijack(HijackCommand::PlayAnimation { clip, channel, .. })
                if channel == "hijack" =>
            {
                clip_seconds(game, clip)
            }
            _ => None,
        });
        let (going, rumble) = self.step(mission, commands, alongside);
        if let Some(s) = rumble {
            game.rumble = game.rumble.max(s);
        }
        going
    }

    /// [`Self::follow`], with the vehicle's clip started this tick lasting
    /// `alongside` seconds: whether it goes on, and how long her pad is
    /// to shake.
    fn step(
        &mut self,
        mission: &mut Mission,
        commands: &[Command],
        alongside: Option<f64>,
    ) -> (bool, Option<f64>) {
        let now = mission.now();
        let mut over = false;
        let mut rumble = None;
        for c in commands {
            match c {
                Command::Hijack(HijackCommand::State { guid, action, .. }) => {
                    self.actions.retain(|a| a.0 != *guid);
                    if !action.eq_ignore_ascii_case("Idle") {
                        self.actions
                            .push((*guid, now + alongside.unwrap_or(UNKNOWN_CLIP)));
                    }
                }
                Command::Hijack(HijackCommand::Completed { hero }) if *hero == PLAYER => {
                    eprintln!(
                        "hijack: {:#x} done after {:.1} s",
                        self.vehicle,
                        now - self.started
                    );
                    over = true;
                }
                Command::Hijack(HijackCommand::AbortDone { hero }) if *hero == PLAYER => {
                    eprintln!(
                        "hijack: {:#x} failed after {:.1} s",
                        self.vehicle,
                        now - self.started
                    );
                    over = true;
                }
                Command::Hijack(HijackCommand::Rumble { guid, seconds }) if *guid == PLAYER => {
                    eprintln!("hijack: pad shakes {seconds:.1} s");
                    rumble = Some(f64::from(*seconds));
                }
                _ => {}
            }
        }
        let (done, going): (Vec<_>, Vec<_>) = self.actions.iter().partition(|a| a.1 <= now);
        self.actions = going;
        for (guid, _) in done {
            mission.action_finished(guid);
        }
        if !over && now - self.started > GIVE_UP {
            eprintln!("hijack: {:#x} given up after {GIVE_UP:.0} s", self.vehicle);
            over = true;
        }
        (!over, rumble)
    }

    /// She pressed pad button `button` (a [`m2_game::hijack::BUTTONS`]
    /// number).
    pub fn press(mission: &mut Mission, button: u32) {
        mission.press(button);
    }

    /// The pad buttons now held: the scripts are told of each change.
    pub fn hold(&mut self, mission: &mut Mission, held: &[u32]) {
        for &b in held {
            if !self.held.contains(&b) {
                mission.hold(b, true);
            }
        }
        for &b in &self.held {
            if !held.contains(&b) {
                mission.hold(b, false);
            }
        }
        self.held = held.to_vec();
    }
}

/// How long clip `name` lasts, seconds, from the archive.
fn clip_seconds(game: &mut Game, name: &str) -> Option<f64> {
    let bytes = game
        .scene
        .source
        .as_mut()?
        .asset(m2_assets::name_hash(name), ANIMATION_TYPE)
        .ok()?;
    Clip::parse(&bytes)
        .ok()
        .map(|c| f64::from(c.header.duration))
}

#[cfg(test)]
mod tests {
    use m2_game::api::Object;
    use m2_game::mission::GameData;

    use super::*;

    /// A hijack script as the Mi35's plays it, cut down: her action, then
    /// a press, then the contract's event.
    const SCRIPT: &str = r#"
        Scripts = Scripts or {}
        Scripts["000000aa"] = { StartHijack = function(hijacker, hijackee, seat, vehicle)
            Human.SetState(hijacker, "InVehicle", "ActionHijackHijackerA")
            Event.Create(Event.HumanActionComplete, { hijacker }, function()
                Event.Create(Event.Minigame, { hijacker, 5, "press", 6 }, function(status)
                    if status == "success" then Event.Post("HijackComplete") end
                end, {})
            end, {})
            return true
        end }
        heard = false
        Event.Create(Event.ScriptEvent, { "HijackComplete" }, function() heard = true end, {})
    "#;

    const MI35: u32 = 0x00d2_9ad0;
    const SOLANO: u32 = 0x00d2_9ad1;

    fn entrance() -> HijackEntrance {
        HijackEntrance {
            entrance: 1,
            script: 0xaa,
            point: None,
            radius: 3.0,
            hardpoint: None,
            name: None,
            flag: None,
        }
    }

    fn mission() -> Mission {
        let m = Mission::new(GameData::default(), "Jennifer", [0.0; 3]).unwrap();
        m.lua().load(SCRIPT).exec().unwrap();
        let mut w = m.world().borrow_mut();
        let object = |name: &str, vehicle| Object {
            name: name.into(),
            alive: true,
            vehicle,
            ..Object::default()
        };
        w.objects.insert(MI35, object("Mi35 (Solano Hijack)", None));
        w.objects.insert(SOLANO, object("Solano", Some(MI35)));
        drop(w);
        m
    }

    #[test]
    fn the_mission_hears_its_hijack_end_once_her_action_and_press_are_done() {
        let mut m = mission();
        let mut h = RideHijack::start(&mut m, MI35, "Mi35 (Solano)", &entrance()).unwrap();
        let heard = |m: &Mission| m.lua().load("return heard").eval::<bool>().unwrap();
        let mut pressed = false;
        for _ in 0..50 {
            m.tick(0.1);
            let commands = m.take_commands();
            assert!(h.step(&mut m, &commands, None).0);
            let prompts = RideHijack::prompts(&m);
            if let Some(g) = prompts.first() {
                assert_eq!((g.mode.as_str(), g.buttons.as_slice()), ("press", &[6][..]));
                RideHijack::press(&mut m, 6);
                pressed = true;
            }
            if heard(&m) {
                break;
            }
        }
        assert!(pressed, "the press came once her action was over");
        assert!(heard(&m), "the contract's script heard the hijack's event");
        // Her action lasted the unknown clip's two seconds.
        assert!(m.now() >= UNKNOWN_CLIP);
    }

    #[test]
    fn nobody_aboard_is_no_hijack() {
        let mut m = mission();
        m.world().borrow_mut().objects.remove(&SOLANO);
        assert!(RideHijack::start(&mut m, MI35, "Mi35 (Solano)", &entrance()).is_none());
    }

    #[test]
    fn an_action_lasts_the_vehicles_clip_started_with_it() {
        let mut m = mission();
        let mut h = RideHijack::start(&mut m, MI35, "Mi35 (Solano)", &entrance()).unwrap();
        let commands = m.take_commands();
        h.step(&mut m, &commands, Some(5.0));
        assert_eq!(h.actions.len(), 1);
        assert!((h.actions[0].1 - (m.now() + 5.0)).abs() < 1e-9);
    }
}
