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
//! - 3 to 9 and 17, cover (0x822C56E0 to 0x822C6728, 0x822C62E0/6330/
//!   6398): asked for in deciding and fighting standing (0x822C43B8; the
//!   next ask 5 s after an answer). A granted spot is gone to, settled at
//!   for 2 s, then used by its `CoverAnimation`: SideStep leans out, fires
//!   a burst, leans back; Dive dives (the spot bad for 20 s) and fights
//!   standing; Popup ducks 3 s and comes up for a burst. A spot lost: he
//!   decides again, free to ask at once.
//! - 14, grenade (0x822C7208/72D0/7348): the gate (0x822C6B78) in
//!   fighting standing, leaning out and popping up; the throw; his next
//!   15 to 25 s off, the shared pause 0 to 7 s as it starts and 5 to 15 s
//!   after.
//!
//! Not ported: `HumanTakeCover`'s entry (12) and the ordered action (13),
//! which `HumanCombat` does not reach; they fall back to deciding.
//!
//! Inferred: the third stance (hash 0x5A86B86B) is lying; the side
//! probes' axis (0x82855ED8) is his left (the first probe plays
//! `SidestepLeft`). The callout's speech and the
//! side-step actions' clips and the throw are the caller's to play.
//! Invented: the cover clips' times ([`LEAN`], [`DIVING`]), as the
//! spot's clips are not wired.

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
    /// Along his side axis (0x82855ED8), the first probe: `Upright`
    /// `SidestepLeft` (0x94247D15, the human state table), so that axis
    /// is his left.
    Left,
    /// Against it, the second probe: `SidestepRight` (0x0C697AF4).
    Right,
}

impl Side {
    /// The action the side-step plays (mind +0xC once started).
    pub fn action(self) -> u32 {
        match self {
            Side::Left => 0x9424_7d15,
            Side::Right => 0x0c69_7af4,
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
    /// The side-step's probes, left (along the side axis) and right, once asked for
    /// ([`Mover::probing`]): whether each came back clear.
    pub sides: Option<[bool; 2]>,
    /// The spot the cover service granted him and still holds for him
    /// (the block's +0x10 with +0x5C bit 0x80).
    pub spot: Option<Spot>,
    /// His cover ask is still out (+0x124 bit 0x8000000).
    pub asking: bool,
    /// He may ask for cover (mind +0x7F0 bit 4; inferred).
    pub may_cover: bool,
    /// A burst is under way (mind +0x128 & 0xF00000 at 0x300000).
    pub firing: bool,
    /// He has a grenade to throw (mind +0x4EC).
    pub has_grenade: bool,
    /// The grenade pause all fighters share (one global) has run out.
    pub grenade_pause_out: bool,
}

/// A spot the cover service granted: where, and its hint's
/// `CoverAnimation` ([`DIVE`], [`SIDE_STEP`], [`POPUP`]; 0 none).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub at: Vec3,
    pub animation: u8,
    /// The actions it gives (its +0x3C, +0x40, +0x48; 0 none), as the
    /// cover service fills them (0x822CCBD0, 0x822CD0E0, 0x822CC908):
    /// back into cover, settling in, and out of it (or the dive)
    /// ([`spot_clips`]).
    pub clips: [u32; 3],
}

/// The human state table's cover actions the spots give.
pub const ENTER_COVER_LEFT: u32 = 0x5873_af69;
pub const ENTER_COVER_RIGHT: u32 = 0x4809_f990;
pub const ENTER_COVER_LEFT_IN_PLACE: u32 = 0x59c8_145d;
pub const ENTER_COVER_RIGHT_IN_PLACE: u32 = 0xd9ce_3342;
pub const ENTER_COVER_KNEEL: u32 = 0x3d37_e829;
pub const EXIT_COVER: u32 = 0xe2bc_1dde;
pub const DIVE_LEFT: u32 = 0x9bcb_d11c;
pub const DIVE_RIGHT: u32 = 0xa20d_465b;

/// A spot's actions (+0x3C, +0x40, +0x48) by its `CoverAnimation` and side
/// (`left`: the cover service's side test, 0x822ED520, true), as the
/// candidate builders fill them (0x822CCBD0 at 0x822CCE7C): SideStep, the
/// way in and the way in in place on that side, and `ExitCover`; Dive, the
/// dive the other way; Popup (0x822CC908), `EnterCoverKneel` to settle.
pub fn spot_clips(animation: u8, left: bool) -> [u32; 3] {
    match animation {
        SIDE_STEP if left => [ENTER_COVER_LEFT, ENTER_COVER_LEFT_IN_PLACE, EXIT_COVER],
        SIDE_STEP => [ENTER_COVER_RIGHT, ENTER_COVER_RIGHT_IN_PLACE, EXIT_COVER],
        DIVE if left => [0, 0, DIVE_RIGHT],
        DIVE => [0, 0, DIVE_LEFT],
        POPUP => [0, ENTER_COVER_KNEEL, 0],
        _ => [0; 3],
    }
}

/// `CoverAnimation` Dive, SideStep and Popup (+0x54).
pub const DIVE: u8 = 1;
pub const SIDE_STEP: u8 = 2;
pub const POPUP: u8 = 3;

/// Seconds after an answer before he asks for cover again (0x822C43B8,
/// 0x820BAF84).
pub const ASK_AGAIN: f32 = 5.0;
/// How long a Dive spot is marked bad, seconds (0x822C6138, 0x820BB204).
pub const DIVE_BAD: f32 = 20.0;
/// How near a spot he must stay: at it (0x822C5920: 0.25 squared) and at
/// a Popup one (0x822C6398: 0.2 squared), metres.
const AT_SPOT: f32 = 0.5;
const AT_POPUP: f32 = 0.447;
/// How near a destination counts as there: 0.1 squared (0x820BB468).
const THERE: f32 = 0.316;
/// The cover clips' stand-ins where a spot gives no action (invented):
/// seconds a lean out or back and a dive take.
pub const LEAN: f32 = 0.5;
pub const DIVING: f32 = 1.0;
/// A Popup spot's time down before he comes up again (0x820C7A20).
const POPUP_DOWN: f32 = 3.0;

/// The grenade gate's range, metres (0x822C6B78: 0x820BAEEC, 0x820BAFA8).
pub const GRENADE_NEAR: f32 = 10.0;
pub const GRENADE_FAR: f32 = 40.0;
/// After a throw his next waits `GRENADE_RETRY + GRENADE_SPREAD x rand`
/// seconds (+0xB4, 0x822C7348); the shared pause is set to `PAUSE_START x
/// rand` as a throw starts and `PAUSE + PAUSE_SPREAD x rand` once it is
/// over (0x822C7208, 0x822C7348).
pub const GRENADE_RETRY: f32 = 15.0;
pub const GRENADE_SPREAD: f32 = 10.0;
pub const PAUSE_START: f32 = 7.0;
pub const PAUSE: f32 = 5.0;
pub const PAUSE_SPREAD: f32 = 10.0;

/// The speed a grenade is thrown at for a target `distance` metres off,
/// m/s: 15 at 10 m to 20 at 40 m (0x822C6B78).
pub fn throw_speed(distance: f32) -> f32 {
    (distance - GRENADE_NEAR) * (1.0 / 30.0) * 5.0 + 15.0
}

/// What a tick asks of the caller beyond the block's own fields.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Out {
    /// Say the closing-in callout (hash 0xBF0C1F5D, heard 50 m).
    pub callout: bool,
    /// Start the side-step action this way.
    pub side_step: Option<Side>,
    /// Ask the cover service for a spot (0x822C43B8).
    pub ask_cover: bool,
    /// Throw a grenade at this (the target's position).
    pub throw: Option<Vec3>,
    /// Set the shared grenade pause to this many seconds.
    pub grenade_pause: Option<f32>,
    /// Mark the spot bad for this many seconds and let it go.
    pub drop_spot: Option<f32>,
    /// Play this action once (a cover clip of the spot's, or
    /// [`EXIT_COVER`] at a Popup spot): the caller holds
    /// [`Sense::one_shot`] while it runs.
    pub play: Option<u32>,
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
    /// Seconds to his next cover ask (+0xB0) and grenade (+0xB4).
    ask_wait: f32,
    grenade_wait: f32,
    /// A cover ask out from this block.
    asked: bool,
    /// A cover state's or the grenade's step (+0xA4), and a burst seen
    /// from it (+0x124 bit 0x20000).
    step: u8,
    fired: bool,
    /// The state's (or the Popup step's) action has been asked for
    /// (+0x124 bits 0x4000000, 0x2000000, 0x1000000).
    played: bool,
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
            ask_wait: 0.0,
            grenade_wait: 0.0,
            asked: false,
            step: 0,
            fired: false,
            played: false,
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
            (State::SideStep, 1, true) => Some(Side::Left),
            (State::SideStep, 2, true) => Some(Side::Right),
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
            self.ask_wait -= s.dt;
            self.grenade_wait -= s.dt;
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
            self.enter(s, rand, &mut out);
        }
        out
    }

    /// The cover ask (0x822C43B8): whether a spot is held. An answer
    /// come back puts the next ask [`ASK_AGAIN`] off.
    fn ask_cover(&mut self, s: &Sense, out: &mut Out) -> bool {
        if self.asked && !s.asking {
            self.asked = false;
            self.ask_wait = ASK_AGAIN;
        }
        if s.spot.is_some() {
            return true;
        }
        if !self.asked && s.may_cover && self.ask_wait <= 0.0 {
            self.asked = true;
            out.ask_cover = true;
        }
        false
    }

    /// The grenade gate (0x822C6B78). Taken to pass: the area's bar
    /// (+0xC bit 12) and the record's freshness (a shot means seen now);
    /// the caller solves the arc.
    fn grenade_gate(&self, s: &Sense) -> bool {
        self.grenade_wait < 0.0
            && s.has_grenade
            && s.grenade_pause_out
            && (GRENADE_NEAR..=GRENADE_FAR).contains(&s.ground_distance)
            && s.has_shot
            && !s.one_shot
    }

    /// He is further than `within` metres from his spot, or has none.
    fn off_spot(s: &Sense, within: f32) -> bool {
        s.spot
            .is_none_or(|spot| Vec2::new(spot.at.x - s.at.x, spot.at.z - s.at.z).length() > within)
    }

    fn enter(&mut self, s: &Sense, rand: &mut dyn FnMut() -> f32, out: &mut Out) {
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
            State::ToSpot => {
                if let Some(spot) = s.spot {
                    self.dest = spot.at;
                }
                self.speed = s.combat_speed;
            }
            State::LeanFire | State::Popup | State::Grenade => {
                self.stand(s);
                self.step = 0;
                self.fired = false;
                self.played = false;
                if self.current.state == State::Grenade {
                    out.grenade_pause = Some(rand() * PAUSE_START);
                }
            }
            State::Dive => {
                self.stand(s);
                self.played = false;
                out.drop_spot = Some(DIVE_BAD);
            }
            State::AtSpot | State::LeanOut | State::LeanBack => {
                self.stand(s);
                self.played = false;
            }
            _ => self.stand(s),
        }
    }

    /// Plays `action` once and says when it is over: asked for this tick,
    /// over once the caller's one-shot has ended. With no action (0), over
    /// after `fallback` seconds in the state.
    fn clip_done(&mut self, action: u32, fallback: f32, s: &Sense, out: &mut Out) -> bool {
        if action == 0 {
            return self.t9c >= fallback;
        }
        if !self.played {
            self.played = true;
            self.t9c = 0.0;
            out.play = Some(action);
            return false;
        }
        self.t9c > 0.0 && !s.one_shot
    }

    fn exit(&mut self, s: &Sense) {
        match self.current.state {
            State::Decide | State::Lie | State::ToSpot | State::AtSpot => self.stand(s),
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
                } else if self.ask_cover(s, out) {
                    self.go(State::ToSpot, State::Idle);
                } else if !self.asked && !s.held {
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
                if standing && self.ask_cover(s, out) {
                    self.go(State::ToSpot, State::Idle);
                    return;
                }
                if s.has_shot {
                    self.ta0 = 0.0;
                    if self.t9c >= 0.5 && standing && self.grenade_gate(s) {
                        self.go(State::Grenade, State::Stand);
                    } else if self.t9c >= 0.5 {
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
            State::ToSpot => {
                if s.busy {
                    return;
                }
                match s.spot {
                    None => self.go(State::SpotLost, State::Idle),
                    Some(spot) => {
                        self.dest = spot.at;
                        if !Self::off_spot(s, THERE) {
                            self.back(State::AtSpot);
                        }
                    }
                }
            }
            State::AtSpot => {
                let Some(spot) = s.spot else {
                    self.go(State::SpotLost, State::Idle);
                    return;
                };
                if Self::off_spot(s, AT_SPOT) {
                    self.go(State::Decide, State::Idle);
                } else if self.in_state >= 2.0 && !s.one_shot {
                    // Its settling-in action first (+0x40), once.
                    if spot.clips[1] != 0 && !self.clip_done(spot.clips[1], 0.0, s, out) {
                        return;
                    }
                    match spot.animation {
                        SIDE_STEP => self.go(State::LeanOut, State::Idle),
                        DIVE => self.go(State::Dive, State::Idle),
                        POPUP => self.go(State::Popup, State::Idle),
                        _ => {}
                    }
                }
            }
            State::LeanOut | State::LeanBack => {
                let Some(spot) = s.spot else {
                    self.go(State::SpotLost, State::Idle);
                    return;
                };
                // Out (+0x48, 0x822C5C80) or back in (+0x3C, 0x822C6028).
                let out_of = self.current.state == State::LeanOut;
                let action = spot.clips[if out_of { 2 } else { 0 }];
                if self.clip_done(action, LEAN, s, out) {
                    let next = if out_of {
                        State::LeanFire
                    } else {
                        State::AtSpot
                    };
                    self.go(next, State::Idle);
                }
            }
            State::LeanFire => {
                if s.spot.is_none() {
                    self.go(State::SpotLost, State::Idle);
                    return;
                }
                self.fired |= s.firing;
                if self.fired && !s.firing {
                    if self.grenade_gate(s) {
                        self.go(State::Grenade, State::LeanBack);
                    } else {
                        self.go(State::LeanBack, State::Idle);
                    }
                }
            }
            State::Dive => {
                // Its dive (+0x48, 0x822C6208).
                let action = s.spot.map_or(0, |spot| spot.clips[2]);
                if self.clip_done(action, DIVING, s, out) {
                    self.back(State::Stand);
                }
            }
            State::Popup => self.popup(s, out),
            State::SpotLost => {
                self.ask_wait = 0.0;
                if !s.busy {
                    self.go(State::Decide, State::Idle);
                }
            }
            State::Grenade => {
                if self.step == 0 {
                    if !s.has_grenade {
                        self.back(State::Decide);
                    } else if !s.busy {
                        self.step = 1;
                        self.t9c = 0.0;
                        out.throw = Some(target);
                    }
                } else if !s.one_shot && self.t9c >= 1.0 {
                    self.grenade_wait = GRENADE_RETRY + rand() * GRENADE_SPREAD;
                    out.grenade_pause = Some(PAUSE + rand() * PAUSE_SPREAD);
                    self.back(State::Decide);
                }
            }
            // `HumanTakeCover`'s entry and the ordered action: not reached
            // by `HumanCombat`, not ported.
            State::TakeCover | State::Ordered | State::Idle => self.go(State::Decide, State::Idle),
        }
    }

    /// A Popup spot (0x822C6398): down, up after [`POPUP_DOWN`] s, a
    /// burst, the grenade gate, down again.
    fn popup(&mut self, s: &Sense, out: &mut Out) {
        if Self::off_spot(s, AT_POPUP) {
            self.go(State::SpotLost, State::Idle);
            return;
        }
        let kneel = s.spot.map_or(0, |spot| spot.clips[1]);
        match self.step {
            0 => {
                // Down: the spot's settling in (`EnterCoverKneel`, to the
                // `CrouchCover` stance).
                self.stance = Stance::Crouch;
                if !s.one_shot && self.clip_done(kneel, 0.0, s, out) {
                    self.step = 1;
                    self.played = false;
                    self.t9c = 0.0;
                }
            }
            1 => {
                if self.t9c > POPUP_DOWN && !s.held {
                    self.step = 4;
                    self.played = false;
                }
            }
            4 => {
                // Up: `ExitCover` (0x822C6398's step 2).
                if self.clip_done(EXIT_COVER, 0.0, s, out) {
                    self.step = 2;
                    self.stance = Stance::Stand;
                    self.fired = false;
                    self.played = false;
                }
            }
            2 => {
                self.fired |= s.firing;
                if self.fired && !s.firing {
                    if self.grenade_gate(s) {
                        self.go(State::Grenade, State::Popup);
                    } else {
                        self.step = 3;
                    }
                }
            }
            _ => {
                self.step = 0;
                self.played = false;
            }
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
                Side::Left
            } else {
                Side::Right
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
    let left = Vec3::new(-f.y, 0.0, f.x);
    let sign = match side {
        Side::Left => 1.0,
        Side::Right => -1.0,
    };
    at + left * (PROBE * sign)
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
            spot: None,
            asking: false,
            may_cover: false,
            firing: false,
            has_grenade: false,
            grenade_pause_out: true,
        }
    }

    /// A block already in `state`, entered.
    fn at_state(state: State, back: State) -> Mover {
        let mut m = Mover::start(Vec3::ZERO, 0.7);
        m.current = Slot { state, back };
        m.enter(&sense(), &mut || 0.5, &mut Out::default());
        m
    }

    fn run(m: &mut Mover, s: &Sense, seconds: f32, r: f32) -> Out {
        let mut any = Out::default();
        for _ in 0..(seconds / s.dt).round() as usize {
            let o = m.tick(s, &mut || r);
            any.callout |= o.callout;
            any.side_step = any.side_step.or(o.side_step);
            any.ask_cover |= o.ask_cover;
            any.throw = any.throw.or(o.throw);
            any.grenade_pause = o.grenade_pause.or(any.grenade_pause);
            any.drop_spot = any.drop_spot.or(o.drop_spot);
            any.play = o.play.or(any.play);
        }
        any
    }

    #[test]
    fn decide_holds_half_a_second_to_a_second_then_rolls() {
        // rand 0.9: hold 0.95 s, roll 5400 -> standing.
        let mut m = Mover::start(Vec3::ZERO, 0.7);
        m.enter(&sense(), &mut || 0.9, &mut Out::default());
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
        // Both sides clear, rand 0.7: the second, the right; the next
        // waits 20.5 s.
        let s2 = Sense {
            sides: Some([true, true]),
            ..s
        };
        let o = run(&mut m, &s2, 0.2, 0.7);
        assert_eq!(o.side_step, Some(Side::Right));
        assert_eq!(Side::Right.action(), 0x0c69_7af4);
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
    fn decide_asks_for_cover_and_waits_for_the_answer() {
        let mut m = at_state(State::Decide, State::Idle);
        let s = Sense {
            may_cover: true,
            ..sense()
        };
        // rand 0.5: a 0.75 s hold, then the ask.
        let o = run(&mut m, &s, 0.9, 0.5);
        assert!(o.ask_cover);
        // The ask out: he waits.
        let out = Sense { asking: true, ..s };
        run(&mut m, &out, 1.0, 0.5);
        assert_eq!(m.state(), State::Decide);
        // A spot granted: to it.
        let spot = Spot {
            at: Vec3::new(4.0, 0.0, 0.0),
            animation: SIDE_STEP,
            clips: [0; 3],
        };
        let got = Sense {
            spot: Some(spot),
            ..s
        };
        run(&mut m, &got, 0.2, 0.5);
        assert_eq!(m.state(), State::ToSpot);
        assert_eq!(m.dest, spot.at);
        // None: the roll, and the next ask 5 s off.
        let mut m = at_state(State::Decide, State::Idle);
        run(&mut m, &s, 0.9, 0.5);
        let o = run(&mut m, &s, 0.2, 0.5);
        assert!(!o.ask_cover);
        assert_eq!(m.state(), State::Stand);
        assert!((m.ask_wait - ASK_AGAIN).abs() < 0.3);
    }

    #[test]
    fn a_side_step_spot_leans_out_fires_a_burst_and_leans_back() {
        let spot = Spot {
            at: Vec3::ZERO,
            animation: SIDE_STEP,
            clips: [0; 3],
        };
        let s = Sense {
            spot: Some(spot),
            ..sense()
        };
        let mut m = at_state(State::ToSpot, State::Idle);
        run(&mut m, &s, 0.1, 0.5);
        assert_eq!(m.state(), State::AtSpot);
        run(&mut m, &s, 1.9, 0.5);
        assert_eq!(m.state(), State::AtSpot);
        run(&mut m, &s, 0.2, 0.5);
        assert_eq!(m.state(), State::LeanOut);
        run(&mut m, &s, LEAN + 0.1, 0.5);
        assert_eq!(m.state(), State::LeanFire);
        let burst = Sense { firing: true, ..s };
        run(&mut m, &burst, 1.0, 0.5);
        assert_eq!(m.state(), State::LeanFire);
        run(&mut m, &s, 0.1, 0.5);
        assert_eq!(m.state(), State::LeanBack);
        run(&mut m, &s, LEAN + 0.1, 0.5);
        assert_eq!(m.state(), State::AtSpot);
        // The spot lost: decide, free to ask at once.
        let lost = Sense { spot: None, ..s };
        run(&mut m, &lost, 0.2, 0.5);
        assert_eq!(m.state(), State::Decide);
        assert!(m.ask_wait <= 0.0);
    }

    #[test]
    fn a_dive_spot_is_marked_bad_and_he_fights_standing() {
        let s = Sense {
            spot: Some(Spot {
                at: Vec3::ZERO,
                animation: DIVE,
                clips: [0; 3],
            }),
            ..sense()
        };
        let mut m = at_state(State::AtSpot, State::Idle);
        let o = run(&mut m, &s, 2.2, 0.5);
        assert_eq!(m.state(), State::Dive);
        assert_eq!(o.drop_spot, Some(DIVE_BAD));
        run(&mut m, &Sense { spot: None, ..s }, DIVING + 0.2, 0.5);
        assert_eq!(m.state(), State::Stand);
    }

    #[test]
    fn a_popup_spot_ducks_three_seconds_then_comes_up() {
        let s = Sense {
            spot: Some(Spot {
                at: Vec3::ZERO,
                animation: POPUP,
                clips: [0; 3],
            }),
            ..sense()
        };
        let mut m = at_state(State::Popup, State::Idle);
        run(&mut m, &s, 0.2, 0.5);
        assert_eq!(m.stance, Stance::Crouch);
        // Up after 3 s down, by `ExitCover` (asked for, then over: no
        // clip held here).
        let o = run(&mut m, &s, 3.4, 0.5);
        assert_eq!(o.play, Some(EXIT_COVER));
        assert_eq!(m.stance, Stance::Stand);
        // Off the spot: lost.
        let off = Sense {
            at: Vec3::new(1.0, 0.0, 0.0),
            ..s
        };
        run(&mut m, &off, 0.1, 0.5);
        assert_eq!(m.state(), State::SpotLost);
    }

    #[test]
    fn the_grenade_gate_throws_and_sets_both_waits() {
        let s = Sense {
            has_grenade: true,
            ..sense()
        };
        let mut m = at_state(State::Stand, State::Idle);
        let o = run(&mut m, &s, 0.7, 0.5);
        assert_eq!(m.state(), State::Grenade);
        assert_eq!(o.grenade_pause, Some(0.5 * PAUSE_START));
        assert_eq!(o.throw, Some(Vec3::new(0.0, 0.0, 30.0)));
        let o = run(&mut m, &s, 1.1, 0.5);
        assert_eq!(m.state(), State::Stand);
        assert_eq!(o.grenade_pause, Some(PAUSE + 0.5 * PAUSE_SPREAD));
        assert!((m.grenade_wait - (GRENADE_RETRY + 0.5 * GRENADE_SPREAD)).abs() < 0.5);
        // Out of range, or the shared pause still on: no throw.
        for blocked in [
            Sense {
                ground_distance: 45.0,
                ..s
            },
            Sense {
                grenade_pause_out: false,
                ..s
            },
        ] {
            let mut m = at_state(State::Stand, State::Idle);
            run(&mut m, &blocked, 0.7, 0.5);
            assert_eq!(m.state(), State::Stand);
        }
        assert_eq!(throw_speed(10.0), 15.0);
        assert!((throw_speed(40.0) - 20.0).abs() < 1e-5);
    }

    #[test]
    fn a_side_step_spot_plays_its_actions_and_waits_for_them() {
        // The cover service's actions for a SideStep spot on the left
        // (0x822CCBD0), a Dive the other way, a Popup's kneel.
        assert_eq!(
            spot_clips(SIDE_STEP, true),
            [ENTER_COVER_LEFT, ENTER_COVER_LEFT_IN_PLACE, EXIT_COVER]
        );
        assert_eq!(spot_clips(DIVE, true), [0, 0, DIVE_RIGHT]);
        assert_eq!(spot_clips(POPUP, false), [0, ENTER_COVER_KNEEL, 0]);
        let spot = Spot {
            at: Vec3::ZERO,
            animation: SIDE_STEP,
            clips: spot_clips(SIDE_STEP, false),
        };
        let s = Sense {
            spot: Some(spot),
            ..sense()
        };
        let mut m = at_state(State::AtSpot, State::Idle);
        // After 2 s, the settling in, once.
        let mut asked = None;
        for _ in 0..30 {
            asked = m.tick(&s, &mut || 0.5).play;
            if asked.is_some() {
                break;
            }
        }
        assert_eq!(asked, Some(ENTER_COVER_RIGHT_IN_PLACE));
        assert!(m.in_state >= 2.0);
        // While its clip runs he stays; then out by `ExitCover`.
        let busy = Sense {
            one_shot: true,
            ..s
        };
        let o = run(&mut m, &busy, 1.0, 0.5);
        assert_eq!((m.state(), o.play), (State::AtSpot, None));
        let o = run(&mut m, &s, 0.2, 0.5);
        assert_eq!(m.state(), State::LeanOut);
        assert_eq!(o.play, Some(EXIT_COVER));
        run(&mut m, &busy, 1.0, 0.5);
        assert_eq!(m.state(), State::LeanOut);
        run(&mut m, &s, 0.2, 0.5);
        assert_eq!(m.state(), State::LeanFire);
    }

    #[test]
    fn side_point_is_two_metres_to_his_right_or_left() {
        let p = side_point(Vec3::ZERO, Vec3::Z, Side::Left);
        assert!((p - Vec3::new(-2.0, 0.0, 0.0)).length() < 1e-5);
        let p = side_point(Vec3::ZERO, Vec3::Z, Side::Right);
        assert!((p - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-5);
    }
}
