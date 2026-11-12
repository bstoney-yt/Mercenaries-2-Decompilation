//! The score's state machine: the states `MrxMusic` declares for a
//! faction, the transitions between them, and when the engine moves from
//! one to another by itself.
//!
//! `Sound.AddMusicState(name, length, level, delay, interval, flags
//! [, least])` declares a state and `Sound.AddMusicTransition(from, to,
//! weight, sync, kind)` a way out of one. With dynamic music on, the
//! engine follows the action around the player: every `interval` seconds
//! a state is weighed against the action level, a state with a `length`
//! is left when it has passed, and a state whose music has ended moves
//! on. `Sound.TransitionMusic(state)` asks for a state outright, dynamic
//! or not.
//!
//! # Provenance
//!
//! From the 360 code (the music state machine, the sound system's member
//! at +0x48; a faction is a block of 0x119C bytes):
//!
//! - A state (`0x823633E8`, 0x128 bytes): its length (+0x108), level
//!   (+0x10C), delay (+0x110), interval (+0x114), least time (+0x118)
//!   and flags (+0x11C), its thresholds (+0x120, +0x124:
//!   `Sound.SetActionThresholdsMusic`, `0x82341F68`) and up to 15
//!   transitions (`0x82362C78`: target, weight, sync, kind).
//! - The update (`0x823634E0`), which [`Faction::update`] follows step by
//!   step, with its picks (`0x82363BD8`, `0x82363A48`, `0x82363B10`,
//!   `0x82363E68`) and the change of state (`0x82363CE0`).
//! - `Sound.TransitionMusic` (`0x82341570`) stores the state asked for
//!   (+0x115C); the update takes it up on its next run.
//! - The action level's inputs ([`Hostility`]): `0x823625E8`, fed by the
//!   AI's count of hostile characters aware of the player in four groups
//!   (`0x822DD7C8`, `0x822EB4E0`).
//!
//! From the scripts (`mrxmusic`): the states' numbers, e.g. `none` (15 s
//! long, level 0), `explore` (30 s, level 0), `action` (level 3, any
//! action), `pause` (delay 0.25 s, flag 2), and the thresholds 2 and 0
//! on `none` and `explore`.
//!
//! Not known: what the engine does with a state's cue beyond starting
//! it (the player's cross-fades).

/// The action around the player: four running values, one for each group
/// of hostile characters aware of them.
///
/// From the 360 code (`0x823625E8`): each value falls by the decay rate
/// a second while above 0 (`Sound.SetHostilityDecayRateMusic`, 1.0 to
/// start with), and is never less than the group's count this frame.
/// `Sound.SetActionLevelsMusic(a, b, c, d)` sets the four outright, the
/// last argument first (`0x82341A88`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hostility {
    pub values: [f32; 4],
}

impl Default for Hostility {
    fn default() -> Self {
        Self { values: [0.0; 4] }
    }
}

impl Hostility {
    /// Moves on `dt` seconds with `counts` hostile characters aware of
    /// the player in each group.
    pub fn update(&mut self, dt: f32, decay: f32, counts: [u32; 4]) {
        for (v, n) in self.values.iter_mut().zip(counts) {
            if *v > 0.0 {
                *v -= decay * dt;
            }
            *v = v.max(n as f32);
        }
    }

    /// The action level against a state's thresholds: 2 when the first
    /// two groups together pass the second threshold, else 1 when the
    /// last two pass the first, else 0.
    pub fn level(&self, thresholds: (f32, f32)) -> i32 {
        let [a, b, c, d] = self.values;
        if a + b > thresholds.1 {
            2
        } else if c + d > thresholds.0 {
            1
        } else {
            0
        }
    }
}

/// Which of [`Hostility`]'s four groups a hostile character aware of the
/// player counts in, by the AI's class of them.
///
/// From the 360 code: the class is set from the vehicle the character
/// is in (`0x822BB608`): 0 none, 1 a car (the vehicle has a
/// `ControllerCar`), 2 a tank (`ControllerTank`), 3 a helicopter
/// (`ControllerHelicopter`), 4 a boat (`ControllerBoat`), 7 a turret, 6
/// any other vehicle. The music's count (`0x822DD7C8`) puts 3 and 8 in
/// the first group, 2 and 4 in the second, 1 and 5 in the third and
/// the rest in the fourth. (Nothing found sets 5 or 8.)
pub fn group(class: u8) -> usize {
    match class {
        3 | 8 => 0,
        2 | 4 => 1,
        1 | 5 => 2,
        _ => 3,
    }
}

/// A state's level that any action answers.
pub const ANY_ACTION: i32 = 3;

/// A state's flags.
pub mod flag {
    /// When its music has played and plays no more, it takes a [`super::Kind::Ended`]
    /// transition.
    pub const MOVES_ON: u8 = 1;
    /// It interrupts: the state it is entered from is kept and comes
    /// back when it is left (`pause`).
    pub const INTERRUPTS: u8 = 2;
    /// Entered with its delay already run.
    pub const NO_DELAY: u8 = 4;
    /// It follows the action level at once
    /// ([`super::Kind::ActionNow`] transitions).
    pub const FOLLOWS_ACTION: u8 = 8;
}

/// When a transition is taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// At the state's interval, to a state of the action level.
    Action,
    /// When the state's length has passed.
    Timeout,
    /// When the state's music has ended.
    Ended,
    /// At once, as a state that follows the action is entered at another
    /// level.
    ActionNow,
}

impl Kind {
    /// The script's number.
    pub fn of(n: i32) -> Option<Self> {
        Some(match n {
            0 => Self::Action,
            1 => Self::Timeout,
            2 => Self::Ended,
            3 => Self::ActionNow,
            _ => return None,
        })
    }
}

/// A way out of a state.
#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    /// The state it leads to, by index.
    pub to: usize,
    pub weight: f32,
    /// 1: not taken while the state's music plays; 2: only then.
    pub sync: i32,
    pub kind: Kind,
}

/// A state of a faction's score.
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub name: String,
    /// Seconds after which it is left by a [`Kind::Timeout`] transition
    /// (0: never).
    pub length: f32,
    /// The action level it is for; [`ANY_ACTION`]; -1 for none.
    pub level: i32,
    /// Seconds before its music starts.
    pub delay: f32,
    /// Seconds between weighings against the action level (0: never).
    pub interval: f32,
    /// Seconds it stays at least.
    pub least: f32,
    pub flags: u8,
    /// `Sound.SetActionThresholdsMusic`.
    pub thresholds: (f32, f32),
    pub transitions: Vec<Transition>,
}

/// The most transitions a state holds.
pub const TRANSITIONS: usize = 15;

/// What changes as a faction's machine runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A state was entered.
    Entered(String),
    /// The state's music is to start (its delay has run).
    Start(String),
}

/// The running part of a faction's machine: the state it is in and its
/// clocks.
#[derive(Clone, Debug, Default, PartialEq)]
struct Running {
    current: Option<usize>,
    /// Since the state was entered, capped at its length and delay.
    since: f32,
    /// Since the last weighing against the action level.
    weighed: f32,
    /// Towards the state's least time.
    held: f32,
    started: bool,
    /// Its music has been heard playing.
    played: bool,
    timed_out: bool,
}

/// The most states a faction holds.
pub const STATES: usize = 15;

/// One faction's states and where its score is.
///
/// A new one has dynamic music and its clocks on, as the 360's starts
/// out (`0x8235B2D0`).
#[derive(Clone, Debug)]
pub struct Faction {
    pub states: Vec<State>,
    run: Running,
    /// What an interrupting state took over from.
    saved: Option<Running>,
    /// The state asked for (`Sound.TransitionMusic`), until taken up.
    asked: Option<usize>,
    /// `Sound.SetDynamicMusic`.
    pub dynamic: bool,
    /// `Sound.SetTimerUpdateMusic`: the clocks run.
    pub clocks: bool,
    /// `Sound.ForceActionTransition`: weigh at the next update.
    pub force: bool,
    has_run: bool,
    /// The action level last worked out.
    pub level: i32,
}

impl Default for Faction {
    fn default() -> Self {
        Self {
            states: Vec::new(),
            run: Running::default(),
            saved: None,
            asked: None,
            dynamic: true,
            clocks: true,
            force: false,
            has_run: true,
            level: 0,
        }
    }
}

impl Faction {
    fn index(&self, name: &str) -> Option<usize> {
        self.states
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// `Sound.AddMusicState`. A state added under the name of one
    /// already there becomes the one the machine is in, its clocks at 0
    /// (as the 360 does; its search finds the first of that name).
    pub fn add_state(&mut self, state: State) {
        if self.states.len() >= STATES {
            return;
        }
        let name = state.name.clone();
        self.states.push(state);
        if let Some(i) = self.index(&name) {
            self.run = Running {
                current: Some(i),
                ..Running::default()
            };
        }
    }

    /// `Sound.AddMusicTransition`: false when either state is not there,
    /// the kind is none of the four, or the state is full.
    pub fn add_transition(
        &mut self,
        from: &str,
        to: &str,
        weight: f32,
        sync: i32,
        kind: i32,
    ) -> bool {
        let (Some(from), Some(to), Some(kind)) = (self.index(from), self.index(to), Kind::of(kind))
        else {
            return false;
        };
        let list = &mut self.states[from].transitions;
        if list.len() >= TRANSITIONS {
            return false;
        }
        list.push(Transition {
            to,
            weight,
            sync,
            kind,
        });
        true
    }

    /// `Sound.SetActionThresholdsMusic`.
    pub fn set_thresholds(&mut self, state: &str, first: f32, second: f32) {
        for s in &mut self.states {
            if s.name.eq_ignore_ascii_case(state) {
                s.thresholds = (first, second);
            }
        }
    }

    /// `Sound.TransitionMusic`: the state is asked for; the next update
    /// takes it up. False when the faction has no such state.
    pub fn ask(&mut self, state: &str) -> bool {
        self.asked = self.index(state);
        self.has_run = true;
        self.asked.is_some()
    }

    /// The state it is in.
    pub fn state(&self) -> Option<&State> {
        self.run.current.and_then(|i| self.states.get(i))
    }

    /// One of `candidates` by weight (`0x82363E68`): a draw of 0 to 1
    /// against the running sum of the weights, passing over a transition
    /// whose sync does not suit; none when the draw is past them all.
    fn pick(&self, candidates: &[&Transition], playing: bool, draw: f32) -> Option<usize> {
        let mut sum = 0.0;
        for t in candidates {
            if (t.sync == 1 && playing) || (t.sync == 2 && !playing) {
                continue;
            }
            sum += t.weight;
            if draw < sum {
                return Some(t.to);
            }
        }
        None
    }

    /// The current state's transitions of `kind`, those to a state of
    /// `level` (or of any action) only where a level is given.
    fn ways(&self, of: usize, kind: Kind, level: Option<i32>) -> Vec<&Transition> {
        self.states[of]
            .transitions
            .iter()
            .filter(|t| t.kind == kind)
            .filter(|t| {
                level.is_none_or(|l| {
                    let to = self.states[t.to].level;
                    to == l || to == ANY_ACTION
                })
            })
            .collect()
    }

    /// Whether a state of level `own` is to be weighed at action level
    /// `level`: not when it is that level's, nor a state for any action
    /// while there is some.
    fn off_level(own: i32, level: i32) -> bool {
        level != own && (own != ANY_ACTION || level < 1)
    }

    /// Enters `to` (`0x82363CE0`). Leaving an interrupting state brings
    /// back what it took over from instead, whatever `to` is.
    fn enter(&mut self, to: usize, events: &mut Vec<Event>) {
        let leaving = self.run.current.map_or(0, |i| self.states[i].flags);
        if leaving & flag::INTERRUPTS != 0 {
            self.run = self.saved.take().unwrap_or_default();
        } else {
            let state = &self.states[to];
            if state.flags & flag::INTERRUPTS != 0 {
                self.saved = Some(self.run.clone());
            }
            self.run = Running {
                current: Some(to),
                since: if state.flags & flag::NO_DELAY != 0 {
                    state.delay
                } else {
                    0.0
                },
                weighed: if self.has_run { 0.0 } else { self.run.weighed },
                ..Running::default()
            };
        }
        if let Some(s) = self.state() {
            events.push(Event::Entered(s.name.clone()));
        }
    }

    /// Runs the machine for `dt` seconds (`0x823634E0`).
    ///
    /// `hostility` gives the action level, unless `locked`
    /// (`Sound.LockActionLevelMusic`), when the last one stands;
    /// `paused` stops the clocks; `playing` says the music player has a
    /// piece playing; `draw` answers a number from 0 to 1 for each pick.
    pub fn update(
        &mut self,
        dt: f32,
        hostility: &Hostility,
        locked: bool,
        paused: bool,
        playing: bool,
        mut draw: impl FnMut() -> f32,
    ) -> Vec<Event> {
        let mut events = Vec::new();
        let Some(cur) = self.run.current else {
            return events;
        };
        let state = self.states[cur].clone();
        let mut next = self.asked.take().filter(|&a| a != cur);
        if !paused && self.clocks {
            if state.delay + state.length > 0.0 {
                self.run.since += dt;
            }
            if state.interval > 0.0 {
                self.run.weighed += dt;
            }
            if state.least > 0.0 {
                self.run.held += dt;
            }
        }
        if !locked {
            self.level = hostility.level(state.thresholds);
        }
        let level = self.level;
        let mut go = next.is_some();
        if self.dynamic && next.is_none() {
            let due = state.interval > 0.0 && state.interval <= self.run.weighed;
            if due || self.force {
                self.force = false;
                if Self::off_level(state.level, level) {
                    let ways = self.ways(cur, Kind::Action, Some(level));
                    next = self.pick(&ways, playing, draw());
                }
            }
            if next.is_none()
                && !self.run.timed_out
                && state.length > 0.0
                && state.delay + state.length < self.run.since
            {
                let ways = self.ways(cur, Kind::Timeout, None);
                next = self.pick(&ways, playing, draw());
                self.run.timed_out = true;
            }
            if next.is_none()
                && state.flags & flag::MOVES_ON != 0
                && self.run.started
                && self.run.played
                && !playing
            {
                let ways = self.ways(cur, Kind::Ended, None);
                next = self.pick(&ways, playing, draw());
            }
            go = next.is_some();
        }
        if go && let Some(mut to) = next {
            // A state that follows the action hands on to the one for
            // the level there is.
            let target = &self.states[to];
            if target.flags & flag::FOLLOWS_ACTION != 0
                && level != target.level
                && (target.level != ANY_ACTION || level < 1)
            {
                let ways = self.ways(to, Kind::ActionNow, Some(level));
                if !ways.is_empty()
                    && let Some(t) = self.pick(&ways, playing, draw())
                {
                    to = t;
                }
            }
            if state.least <= self.run.held {
                self.enter(to, &mut events);
            }
        }
        // The state it is in now, entered or not.
        self.has_run = true;
        let Some(cur) = self.run.current else {
            return events;
        };
        let state = &self.states[cur];
        if !self.run.started && state.delay <= self.run.since {
            self.run.started = true;
            events.push(Event::Start(state.name.clone()));
        }
        if state.interval > 0.0 && state.interval < self.run.weighed {
            self.run.weighed = dt;
        }
        let most = state.delay + state.length;
        if most < self.run.since {
            self.run.since = most;
        }
        if state.least < self.run.held {
            self.run.held = state.least;
        }
        if !self.run.played {
            self.run.played = playing;
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(name: &str, length: f32, level: i32, interval: f32, flags: u8) -> State {
        State {
            name: name.into(),
            length,
            level,
            delay: 0.0,
            interval,
            least: 0.0,
            flags,
            thresholds: (2.0, 0.0),
            transitions: Vec::new(),
        }
    }

    /// A free-play faction much as `mrxmusic` declares one.
    fn faction() -> Faction {
        let mut f = Faction {
            dynamic: true,
            clocks: true,
            ..Faction::default()
        };
        f.add_state(State {
            delay: 0.25,
            ..state("pause", 0.0, -1, 0.0, flag::INTERRUPTS)
        });
        f.add_state(state("none", 15.0, 0, 10.0, 0));
        f.add_state(state("explore", 30.0, 0, 10.0, 0));
        f.add_state(state("action", 0.0, ANY_ACTION, 15.0, 0));
        assert!(f.add_transition("none", "explore", 1.0, 0, 1));
        assert!(f.add_transition("explore", "none", 1.0, 0, 1));
        assert!(f.add_transition("none", "action", 1.0, 0, 0));
        assert!(f.add_transition("explore", "action", 1.0, 0, 0));
        assert!(f.add_transition("action", "explore", 1.0, 0, 0));
        assert!(!f.add_transition("none", "nowhere", 1.0, 0, 0));
        assert!(!f.add_transition("none", "explore", 1.0, 0, 9));
        f
    }

    fn run(f: &mut Faction, seconds: f32, h: &Hostility) -> Vec<(f32, Event)> {
        let dt = 0.1;
        let mut out = Vec::new();
        for k in 1..=(seconds / dt).round() as usize {
            for e in f.update(dt, h, false, false, false, || 0.5) {
                out.push((k as f32 * dt, e));
            }
        }
        out
    }

    #[test]
    fn the_action_level_is_two_for_the_first_groups_one_for_the_last() {
        let mut h = Hostility::default();
        assert_eq!(h.level((2.0, 0.0)), 0);
        // Two of the last groups do not pass a threshold of 2; three do.
        h.update(0.1, 1.0, [0, 0, 1, 1]);
        assert_eq!(h.level((2.0, 0.0)), 0);
        h.update(0.1, 1.0, [0, 0, 1, 2]);
        assert_eq!(h.level((2.0, 0.0)), 1);
        // One of the first groups is enough.
        h.update(0.1, 1.0, [0, 1, 0, 0]);
        assert_eq!(h.level((2.0, 0.0)), 2);
    }

    #[test]
    fn the_groups_are_helicopters_tanks_and_boats_cars_and_the_rest() {
        assert_eq!([group(3), group(8)], [0, 0]);
        assert_eq!([group(2), group(4)], [1, 1]);
        assert_eq!([group(1), group(5)], [2, 2]);
        assert_eq!([group(0), group(6), group(7), group(9)], [3; 4]);
    }

    #[test]
    fn hostility_lingers_and_decays_by_the_rate() {
        let mut h = Hostility::default();
        h.update(0.5, 1.0, [3, 0, 0, 0]);
        assert_eq!(h.values[0], 3.0);
        // Nobody left: one a second.
        h.update(0.5, 1.0, [0; 4]);
        assert_eq!(h.values[0], 2.5);
        h.update(1.0, 2.0, [0; 4]);
        assert_eq!(h.values[0], 0.5);
        // Never under the count there is.
        h.update(1.0, 1.0, [2, 0, 0, 0]);
        assert_eq!(h.values[0], 2.0);
        // The count is its floor, 0 with nobody about.
        let mut h = Hostility {
            values: [0.25, 0.0, 0.0, 0.0],
        };
        h.update(1.0, 1.0, [0; 4]);
        assert_eq!(h.values[0], 0.0);
    }

    #[test]
    fn a_state_starts_its_music_and_is_left_when_its_length_has_passed() {
        let mut f = faction();
        // Declaring a state leaves the machine in it: "action", the last.
        assert_eq!(f.state().unwrap().name, "action");
        f.ask("none");
        let calm = Hostility::default();
        let events = run(&mut f, 50.0, &calm);
        let names: Vec<(f32, &Event)> = events.iter().map(|(t, e)| (*t, e)).collect();
        let at = |want: &Event| {
            names
                .iter()
                .filter(|(_, e)| *e == want)
                .map(|(t, _)| *t)
                .collect::<Vec<f32>>()
        };
        // "none" is entered and starts at once, and is left just past
        // 15 s; "explore" is left just past 30 s more.
        let none = at(&Event::Entered("none".into()));
        let explore = at(&Event::Entered("explore".into()));
        assert_eq!((none.len(), explore.len()), (2, 1), "{names:?}");
        assert!((none[0] - 0.1).abs() < 0.01, "{names:?}");
        assert!((at(&Event::Start("none".into()))[0] - 0.1).abs() < 0.01);
        assert!((explore[0] - 15.2).abs() < 0.21, "{names:?}");
        assert!(at(&Event::Start("explore".into()))[0] - explore[0] < 0.11);
        assert!((none[1] - 45.3).abs() < 0.31, "{names:?}");
    }

    #[test]
    fn the_action_is_weighed_at_the_state_s_interval() {
        let mut f = faction();
        f.ask("explore");
        let mut h = Hostility::default();
        run(&mut f, 1.0, &h);
        assert_eq!(f.state().unwrap().name, "explore");
        // A hostile one of the first groups: level 2, but not before
        // the interval (10 s) is up.
        h.values[1] = 1.0;
        let events = run(&mut f, 8.5, &h);
        assert!(events.is_empty(), "{events:?}");
        assert_eq!(f.level, 2);
        let events = run(&mut f, 1.0, &h);
        assert!(
            events
                .iter()
                .any(|(_, e)| *e == Event::Entered("action".into()))
        );
        // "action" is for any action: it stays while there is some, and
        // goes back at its own interval (15 s) once there is none.
        let events = run(&mut f, 20.0, &h);
        assert!(
            events.iter().all(|(_, e)| !matches!(e, Event::Entered(_))),
            "{events:?}"
        );
        h.values[1] = 0.0;
        let events = run(&mut f, 16.0, &h);
        assert!(
            events
                .iter()
                .any(|(_, e)| *e == Event::Entered("explore".into()))
        );
    }

    #[test]
    fn a_forced_weighing_does_not_wait_and_a_lock_keeps_the_level() {
        let mut f = faction();
        f.ask("explore");
        let mut h = Hostility::default();
        run(&mut f, 0.5, &h);
        h.values[0] = 1.0;
        f.force = true;
        let events = f.update(0.1, &h, false, false, false, || 0.5);
        assert_eq!(events[0], Event::Entered("action".into()));
        assert!(!f.force);
        // Locked: the level stays 2 with nobody about.
        h.values[0] = 0.0;
        for _ in 0..200 {
            f.update(0.1, &h, true, false, false, || 0.5);
        }
        assert_eq!((f.level, f.state().unwrap().name.as_str()), (2, "action"));
    }

    #[test]
    fn asking_works_without_dynamic_music_and_nothing_else_moves() {
        let mut f = faction();
        f.dynamic = false;
        f.ask("explore");
        let h = Hostility {
            values: [5.0, 0.0, 0.0, 0.0],
        };
        let events = run(&mut f, 60.0, &h);
        let entered: Vec<&Event> = events
            .iter()
            .map(|(_, e)| e)
            .filter(|e| matches!(e, Event::Entered(_)))
            .collect();
        assert_eq!(entered, [&Event::Entered("explore".into())]);
        assert!(!f.ask("nowhere"));
    }

    #[test]
    fn an_interrupting_state_gives_back_the_state_it_took_over_from() {
        let mut f = faction();
        f.ask("explore");
        let h = Hostility::default();
        run(&mut f, 5.0, &h);
        f.ask("pause");
        let events = run(&mut f, 1.0, &h);
        assert_eq!(events[0].1, Event::Entered("pause".into()));
        // Its music starts once its delay (0.25 s) has run.
        let start = events
            .iter()
            .find(|(_, e)| *e == Event::Start("pause".into()));
        assert!(
            start.is_some_and(|(t, _)| (*t - 0.3).abs() < 0.11),
            "{events:?}"
        );
        // Whatever is asked for next, "explore" comes back, its clock
        // where it was (5 s in, so it is left 25 s on, not 30).
        f.ask("none");
        let events = run(&mut f, 26.5, &h);
        assert_eq!(events[0].1, Event::Entered("explore".into()));
        let left = events
            .iter()
            .find(|(_, e)| *e == Event::Entered("none".into()));
        assert!(
            left.is_some_and(|(t, _)| (*t - 25.2).abs() < 0.31),
            "{events:?}"
        );
    }

    #[test]
    fn a_pick_is_by_weight_and_minds_whether_music_plays() {
        let mut f = faction();
        f.states[1].transitions.clear();
        f.add_transition("none", "explore", 0.25, 0, 1);
        f.add_transition("none", "action", 0.5, 2, 1);
        f.add_transition("none", "pause", 0.25, 1, 1);
        let ways = f.ways(1, Kind::Timeout, None);
        // Nothing playing: the one for a playing piece is passed over.
        assert_eq!(f.pick(&ways, false, 0.1), Some(2));
        assert_eq!(f.pick(&ways, false, 0.3), Some(0));
        assert_eq!(f.pick(&ways, false, 0.6), None);
        // Playing: the one for silence is.
        assert_eq!(f.pick(&ways, true, 0.3), Some(3));
        assert_eq!(f.pick(&ways, true, 0.8), None);
    }
}
