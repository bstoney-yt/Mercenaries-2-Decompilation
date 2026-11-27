//! A fighter's combat mover: the 360's `HumanCombat` move block (the
//! mind's +0x130), its state machine and the states that decide, fight
//! standing, crouched and lying, and side-step. From the 360 code; the
//! map of every state is `docs/fidelity/combat-mover.md`.
//!
//! The block holds where the fighter is told to go and how fast, the
//! stance it asks of the body, and a current and a queued state, each an
//! enter, an exit and an update with a return state. A tick (0x822C8530)
//! counts its timers, runs the current state's update, then swaps a
//! queued state in (the old one's exit, the timers zeroed, the new one's
//! enter). A change (0x822C3FE8) also asks the body to stand.
//!
//! Ported here (states by the table at 0x82B77F08):
//!
//! - 1, decide (0x822C4AC8/4B50/4BA8): stand, hold `(rand + 1) x 0.5` s,
//!   then a roll in 0..6000: under 1000 with the target 10 m or more off
//!   crouch and fight (10), under 1500 at 20 m or more lie and fight (11),
//!   else fight standing (2). The cover ask before the roll is round (c).
//! - 2, fight standing (0x822C5340/5398/53F8): with a shot, after 0.5 s
//!   stand; after 5 s in the state with the side-step wait out, side-step
//!   (15); after 10 s decide again. Without a shot for 2 s, and the
//!   target inside his area: to the target at `PursueSpeed`, a callout
//!   the first time.
//! - 10, fight crouched (0x822C67A0/6808/6868): as 2, crouched, with no
//!   side-step or grenade.
//! - 11, fight lying (0x822C6A08/6A58/6AA0): out after 30 s or with the
//!   target within 10 m; 5 s without a shot returns.
//! - 15, side-step (0x822C7708/7730): two probes 2 m to either side; a
//!   clear side (a random one of two) is stepped to by its action, and
//!   the next waits `10 + rand x 15` s; neither clear: decide.
//! - 16 (0x822C7CA0/7CC0): straight to its return.
//!
//! Not yet (rounds c and d): the cover states (3 to 9, 12, 17), the
//! grenade (14), state 13. Their ids are kept; the states reached only
//! through them are unreachable until then, and fall back to deciding.
//!
//! Inferred: the third stance (hash 0x5A86B86B) is lying; the side
//! probes' axis (0x82855ED8) is his right. The callout's speech and the
//! side-step actions' clips are the caller's to play.

use m2_math::glam::{Vec2, Vec3};

/// A state of the block, by the program's ids.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    /// The end (no functions).
    #[default]
    Idle = 0,
    Decide = 1,
    Stand = 2,
    ToSpot = 3,
    AtSpot = 4,
    LeanOut = 5,
    LeanFire = 6,
    LeanBack = 7,
    Dive = 8,
    SpotLost = 9,
    Crouch = 10,
    Lie = 11,
    TakeCover = 12,
    Ordered = 13,
    Grenade = 14,
    SideStep = 15,
    Pass = 16,
    Popup = 17,
}

/// The stance the block asks of the body (+0x90).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stance {
    /// `stand` (0xAB3B572B).
    #[default]
    Stand,
    /// `crouch` (0xE861BC73).
    Crouch,
    /// 0x5A86B86B (inferred: lying).
    Low,
}

/// A side of the fighter, for the side-step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Along his side axis (inferred: his right); action 0x94247D15.
    Right,
    /// Against it; action 0x0C697AF4.
    Left,
}

impl Side {
    /// The action the side-step plays (mind +0xC once started).
    pub fn action(self) -> u32 {
        match self {
            Side::Right => 0x9424_7d15,
            Side::Left => 0x0c69_7af4,
        }
    }
}

/// How far each side-step probe reaches, metres (0x820C77A4).
pub const PROBE: f32 = 2.0;

/// What the block is told each tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sense {
    /// The frame's step, seconds.
    pub dt: f32,
    /// Where he stands.
    pub at: Vec3,
    /// The target's position, as his record of it has it (+0x60's +0x80),
    /// or no target.
    pub target: Option<Vec3>,
    /// The target's distance over the ground, metres (the record's +0x60).
    pub ground_distance: f32,
    /// He has a shot: the fire line clear (mind +0xCC bit 0x80) and the
    /// target not beyond his gun's reach (mind +0x128's verdict not 4).
    pub has_shot: bool,
    /// The target is outside his area (0x822C0740, [`crate::ai_anchor`]).
    pub out_of_area: bool,
    /// He is busy with an action that must end first (0x822C8280).
    pub busy: bool,
    /// He is playing a one-shot action (0x822C04A0): the side-step's.
    pub one_shot: bool,
    /// The area allows crouching and lying (mind +0x274's +0xC bits 3
    /// and 4 clear).
    pub may_crouch: bool,
    pub may_lie: bool,
    /// The decide state waits (mind +0x70F bit 3, not traced).
    pub held: bool,
    /// His `CombatSpeed` (mind +0x9A8) and `PursueSpeed` (+0x9A4).
    pub combat_speed: f32,
    pub pursue_speed: f32,
    /// The side-step's probes, right and left, once asked for
    /// ([`Mover::probing`]): whether each came back clear.
    pub sides: Option<[bool; 2]>,
}

/// What a tick asks of the caller beyond the block's own fields.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Out {
    /// Say the closing-in callout (hash 0xBF0C1F5D, heard 50 m).
    pub callout: bool,
    /// Start the side-step action this way.
    pub side_step: Option<Side>,
}

/// The callout said when he starts closing in.
pub const CALLOUT: u32 = 0xbf0c_1f5d;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Slot {
    state: State,
    back: State,
}

/// The move block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mover {
    /// Where he is told to go (+0x0): his own place to stand.
    pub dest: Vec3,
    /// How fast (+0xC), a share of his full speed.
    pub speed: f32,
    /// The stance asked of the body (+0x90).
    pub stance: Stance,
    current: Slot,
    queued: Option<Slot>,
    /// Seconds in the state (+0x94) and the two free timers (+0x9C,
    /// +0xA0).
    in_state: f32,
    t9c: f32,
    ta0: f32,
    /// The decide state's hold (+0xB8).
    hold: f32,
    /// Seconds to the next side-step (+0xC0), counted down.
    side_wait: f32,
    /// The callout said (+0x124 bit 0x8000).
    called: bool,
    /// The side-step's step (+0xA4) and whether its action started
    /// (+0x124 bit 0x400000).
    side: u8,
    started: bool,
}

impl Default for Mover {
    /// As 0x822C3EB8 leaves it: deciding, speed 0.4 (0x820C7734).
    fn default() -> Self {
        Self {
            dest: Vec3::ZERO,
            speed: 0.4,
            stance: Stance::Stand,
            current: Slot {
                state: State::Decide,
                back: State::Idle,
            },
            queued: None,
            in_state: 0.0,
            t9c: 0.0,
            ta0: 0.0,
            hold: 0.0,
            side_wait: 0.0,
            called: false,
            side: 0,
            started: false,
        }
    }
}

impl Mover {
    /// `HumanCombat`'s start (0x822B3668): reset, deciding, standing where
    /// he is at his `CombatSpeed`.
    pub fn start(at: Vec3, combat_speed: f32) -> Self {
        Self {
            dest: at,
            speed: combat_speed,
            ..Self::default()
        }
    }

    /// The current state.
    pub fn state(&self) -> State {
        self.current.state
    }

    /// Whether the side-step wants its probes ([`Sense::sides`]).
    pub fn probing(&self) -> bool {
        self.current.state == State::SideStep && self.side == 0
    }

    /// The side being stepped to, while its action runs.
    pub fn side_stepping(&self) -> Option<Side> {
        match (self.current.state, self.side, self.started) {
            (State::SideStep, 1, true) => Some(Side::Right),
            (State::SideStep, 2, true) => Some(Side::Left),
            _ => None,
        }
    }

    /// Queues `state`, returning to `back` (0x822C3FE8): the body is asked
    /// to stand.
    fn go(&mut self, state: State, back: State) {
        self.stance = Stance::Stand;
        self.queued = Some(Slot { state, back });
    }

    /// Goes to the current state's return, or `default` without one.
    fn back(&mut self, default: State) {
        let to = match self.current.back {
            State::Idle => default,
            s => s,
        };
        self.go(to, State::Idle);
    }

    fn stand(&mut self, s: &Sense) {
        self.dest = s.at;
    }

    /// One tick (0x822C8530), `rand` giving numbers in 0..1.
    pub fn tick(&mut self, s: &Sense, rand: &mut dyn FnMut() -> f32) -> Out {
        let mut out = Out::default();
        if self.current.state != State::Idle {
            self.side_wait -= s.dt;
            self.in_state += s.dt;
            self.t9c += s.dt;
            self.ta0 += s.dt;
            self.update(s, rand, &mut out);
        }
        if let Some(next) = self.queued.take() {
            self.exit(s);
            self.current = next;
            self.in_state = 0.0;
            self.t9c = 0.0;
            self.ta0 = 0.0;
            self.enter(s, rand);
        }
        out
    }

    fn enter(&mut self, s: &Sense, rand: &mut dyn FnMut() -> f32) {
        match self.current.state {
            State::Decide => {
                self.stand(s);
                self.hold = (rand() + 1.0) * 0.5;
            }
            State::Stand | State::Crouch => {
                self.called = false;
                self.stand(s);
                if self.current.state == State::Crouch {
                    self.speed = s.combat_speed;
                }
            }
            State::Lie => {
                self.stance = Stance::Low;
                self.stand(s);
            }
            State::SideStep => {
                self.side = 0;
                self.started = false;
            }
            _ => {}
        }
    }

    fn exit(&mut self, s: &Sense) {
        match self.current.state {
            State::Decide | State::Lie => self.stand(s),
            State::Stand | State::Crouch => {
                self.stand(s);
                self.speed = s.combat_speed;
            }
            _ => {}
        }
    }

    fn update(&mut self, s: &Sense, rand: &mut dyn FnMut() -> f32, out: &mut Out) {
        let Some(target) = s.target else {
            return;
        };
        match self.current.state {
            State::Decide => {
                if s.busy {
                    return;
                }
                if self.hold > 0.0 {
                    self.hold -= s.dt;
                } else if !s.held {
                    // 0x821B1298 mod 6000.
                    let r = ((rand() * 6000.0) as u32).min(5999);
                    let next = if r < 1000 && s.ground_distance >= 10.0 && s.may_crouch {
                        State::Crouch
                    } else if r < 1500 && s.ground_distance >= 20.0 && s.may_lie {
                        State::Lie
                    } else {
                        State::Stand
                    };
                    self.go(next, State::Idle);
                }
            }
            State::Stand | State::Crouch => {
                let standing = self.current.state == State::Stand;
                if s.has_shot {
                    self.ta0 = 0.0;
                    if self.t9c >= 0.5 {
                        self.speed = s.combat_speed;
                        if !standing {
                            self.stance = Stance::Crouch;
                        }
                        self.called = false;
                        self.stand(s);
                        if standing && self.in_state > 5.0 && self.side_wait < 0.0 {
                            self.go(State::SideStep, State::Stand);
                        } else if self.t9c > 10.0 {
                            if standing {
                                self.go(State::Decide, State::Idle);
                            } else {
                                self.back(State::Decide);
                            }
                        }
                    }
                } else if !s.out_of_area && self.ta0 > 2.0 {
                    self.t9c = 0.0;
                    self.dest = target;
                    self.speed = s.pursue_speed;
                    if !self.called {
                        self.called = true;
                        out.callout = true;
                    }
                }
            }
            State::Lie => {
                if self.in_state > 30.0 || s.ground_distance < 10.0 {
                    self.go(State::Decide, State::Idle);
                }
                if s.has_shot {
                    self.t9c = 0.0;
                } else if self.t9c > 5.0 {
                    self.back(State::Decide);
                }
            }
            State::SideStep => self.side_step(s, rand, out),
            State::Pass => self.back(State::Lie),
            // Not yet ported (rounds c and d): unreachable until then.
            _ => self.go(State::Decide, State::Idle),
        }
    }

    fn side_step(&mut self, s: &Sense, rand: &mut dyn FnMut() -> f32, out: &mut Out) {
        if self.side == 0 {
            let Some([right, left]) = s.sides else {
                return;
            };
            self.side = match (right, left) {
                (true, true) => {
                    if rand() < 0.5 {
                        1
                    } else {
                        2
                    }
                }
                (true, false) => 1,
                (false, true) => 2,
                (false, false) => {
                    self.go(State::Decide, State::Idle);
                    return;
                }
            };
            return;
        }
        self.stand(s);
        if !self.started {
            self.started = true;
            self.side_wait = 10.0 + rand() * 15.0;
            self.t9c = 0.0;
            out.side_step = Some(if self.side == 1 {
                Side::Right
            } else {
                Side::Left
            });
            return;
        }
        if !s.one_shot && self.t9c >= 1.0 {
            self.back(State::Lie);
        }
    }
}

/// Where a side-step to `side` from `at` ends, his facing `forward`
/// (x/z): [`PROBE`] metres along the side axis. For a body without the
/// action's clip (invented: its root motion is not wired).
pub fn side_point(at: Vec3, forward: Vec3, side: Side) -> Vec3 {
    let f = Vec2::new(forward.x, forward.z).normalize_or(Vec2::Y);
    let right = Vec3::new(-f.y, 0.0, f.x);
    let sign = match side {
        Side::Right => 1.0,
        Side::Left => -1.0,
    };
    at + right * (PROBE * sign)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sense() -> Sense {
        Sense {
            dt: 0.1,
            at: Vec3::ZERO,
            target: Some(Vec3::new(0.0, 0.0, 30.0)),
            ground_distance: 30.0,
            has_shot: true,
            out_of_area: false,
            busy: false,
            one_shot: false,
            may_crouch: true,
            may_lie: true,
            held: false,
            combat_speed: 0.7,
            pursue_speed: 0.9,
            sides: None,
        }
    }

    /// A block already in `state`, entered.
    fn at_state(state: State, back: State) -> Mover {
        let mut m = Mover::start(Vec3::ZERO, 0.7);
        m.current = Slot { state, back };
        m.enter(&sense(), &mut || 0.5);
        m
    }

    fn run(m: &mut Mover, s: &Sense, seconds: f32, r: f32) -> Out {
        let mut any = Out::default();
        for _ in 0..(seconds / s.dt).round() as usize {
            let o = m.tick(s, &mut || r);
            any.callout |= o.callout;
            any.side_step = any.side_step.or(o.side_step);
        }
        any
    }

    #[test]
    fn decide_holds_half_a_second_to_a_second_then_rolls() {
        // rand 0.9: hold 0.95 s, roll 5400 -> standing.
        let mut m = Mover::start(Vec3::ZERO, 0.7);
        m.enter(&sense(), &mut || 0.9);
        run(&mut m, &sense(), 0.9, 0.9);
        assert_eq!(m.state(), State::Decide);
        run(&mut m, &sense(), 0.2, 0.9);
        assert_eq!(m.state(), State::Stand);
    }

    #[test]
    fn the_roll_picks_crouch_lie_or_stand_by_range() {
        let pick = |r: f32, d: f32| {
            let mut m = Mover::start(Vec3::ZERO, 0.7);
            let s = Sense {
                ground_distance: d,
                ..sense()
            };
            run(&mut m, &s, 2.0, r);
            m.state()
        };
        assert_eq!(pick(0.1, 30.0), State::Crouch); // 600
        assert_eq!(pick(0.2, 30.0), State::Lie); // 1200
        assert_eq!(pick(0.2, 15.0), State::Stand); // lying needs 20 m
        assert_eq!(pick(0.1, 8.0), State::Stand); // crouching needs 10 m
        assert_eq!(pick(0.5, 30.0), State::Stand);
    }

    #[test]
    fn standing_with_a_shot_side_steps_after_five_seconds() {
        let mut m = at_state(State::Stand, State::Idle);
        let s = sense();
        run(&mut m, &s, 4.9, 0.5);
        assert_eq!(m.state(), State::Stand);
        assert_eq!(m.dest, Vec3::ZERO);
        run(&mut m, &s, 0.3, 0.5);
        assert_eq!(m.state(), State::SideStep);
        assert!(m.probing());
        // Both sides clear, rand 0.7: the left; the next waits 20.5 s.
        let s2 = Sense {
            sides: Some([true, true]),
            ..s
        };
        let o = run(&mut m, &s2, 0.2, 0.7);
        assert_eq!(o.side_step, Some(Side::Left));
        assert!((m.side_wait - 20.5).abs() < 0.2);
        // Its action over and a second gone: back to fighting standing.
        run(&mut m, &s2, 1.2, 0.7);
        assert_eq!(m.state(), State::Stand);
    }

    #[test]
    fn no_side_clear_decides_again() {
        let mut m = at_state(State::SideStep, State::Stand);
        let s = Sense {
            sides: Some([false, false]),
            ..sense()
        };
        run(&mut m, &s, 0.3, 0.5);
        assert_eq!(m.state(), State::Decide);
    }

    #[test]
    fn without_a_shot_he_closes_in_after_two_seconds_inside_his_area() {
        let mut m = at_state(State::Stand, State::Idle);
        let s = Sense {
            has_shot: false,
            ..sense()
        };
        let o = run(&mut m, &s, 1.9, 0.5);
        assert!(!o.callout);
        assert_eq!(m.dest, Vec3::ZERO);
        let o = run(&mut m, &s, 0.3, 0.5);
        assert!(o.callout);
        assert_eq!(m.dest, Vec3::new(0.0, 0.0, 30.0));
        assert_eq!(m.speed, 0.9);
        // Outside his area he stays.
        let mut m = at_state(State::Stand, State::Idle);
        let s = Sense {
            out_of_area: true,
            ..s
        };
        run(&mut m, &s, 5.0, 0.5);
        assert_eq!(m.dest, Vec3::ZERO);
    }

    #[test]
    fn crouched_he_crouches_with_a_shot_and_decides_after_ten_seconds() {
        let mut m = at_state(State::Crouch, State::Idle);
        let s = sense();
        run(&mut m, &s, 1.0, 0.5);
        assert_eq!(m.stance, Stance::Crouch);
        run(&mut m, &s, 9.6, 0.5);
        assert_eq!(m.state(), State::Decide);
        assert_eq!(m.stance, Stance::Stand);
    }

    #[test]
    fn lying_leaves_after_thirty_seconds_or_within_ten_metres() {
        let mut m = at_state(State::Lie, State::Idle);
        let s = sense();
        run(&mut m, &s, 0.1, 0.5);
        assert_eq!(m.stance, Stance::Low);
        run(&mut m, &s, 29.7, 0.5);
        assert_eq!(m.state(), State::Lie);
        run(&mut m, &s, 0.4, 0.5);
        assert_eq!(m.state(), State::Decide);
        let mut m = at_state(State::Lie, State::Idle);
        let near = Sense {
            ground_distance: 9.0,
            ..s
        };
        run(&mut m, &near, 0.2, 0.5);
        assert_eq!(m.state(), State::Decide);
        // Five seconds without a shot returns.
        let mut m = at_state(State::Lie, State::Idle);
        let blind = Sense {
            has_shot: false,
            ..s
        };
        run(&mut m, &blind, 5.3, 0.5);
        assert_eq!(m.state(), State::Decide);
    }

    #[test]
    fn side_point_is_two_metres_to_his_right_or_left() {
        let p = side_point(Vec3::ZERO, Vec3::Z, Side::Right);
        assert!((p - Vec3::new(-2.0, 0.0, 0.0)).length() < 1e-5);
        let p = side_point(Vec3::ZERO, Vec3::Z, Side::Left);
        assert!((p - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-5);
    }
}
