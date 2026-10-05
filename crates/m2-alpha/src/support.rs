//! Support: what Fiona's contacts send at her call. In the first contract
//! both gates fall to a Satellite Guided Bomb (rounds 93 and 94 looked at
//! the videos, the PC's, Xbox 360's and PS3's alike): the support box
//! shows it ("1 FREE"); used from the support menu, she takes up the
//! satellite designator and at once looks down through the satellite over
//! where she stands, moves the view over the target and confirms. A timing
//! wheel turns, and a press with its needle in each of its two grey wedges
//! sends the bomb. About seven seconds later it lands where the view was
//! confirmed: a flash that fills the screen, and a blast that brings the
//! gate and its wall down.
//!
//! The scripts call it `mrxsatelliteguidedbomb`: the designator's script
//! (`MrxSupportDesignatorSatellite`) runs the view, the satellite
//! interface's (`mrxguisatellite`) its wheel, and the bomb is a "Smart
//! Bomb Projectile" (the JDAM) whose blast is "Explosion (Airstike Bomb)"
//! (80008e3f: 20 m, 100 damage, a camera shake of 10 for 1.7 s) with its
//! frag "Explosion (Airstrike Frag)" (8000a3df: 50 m, 300) and, 2 s on,
//! "Explosion (Airstike Bomb Final Strike)" (8000af7d: 2 m, 500); see
//! [`crate::combat::Combat::bomb`]. The designator is the template
//! "Satellite Designator" (800052f3, model global_weapon_detonator).
//!
//! The first contract's script gives the bomb free
//! (`MrxSupportData.AddFreebie("VzaCon01_SatBomb", ...)`, as
//! [`Support::grant`] here) once she has opened and closed her PDA at the
//! first gate, and again at the second, and hears the call's steps
//! ([`Support::posts`]) as the satellite interface posts them. Should it
//! never give one while a gate stands, one is given anyway after a while
//! ([`Support::fallback`]).
//!
//! Measured in the Xbox 360 video (round 94): the view shows about 110 m
//! of ground from top to bottom, moves about 5 m/s and opens over where
//! she stands; the view goes back to her a fifth of a second after the
//! second press, and the bomb lands 7 s after that. From the scripts
//! (round 95): the satellite looks down from 170 m over her and the view
//! goes no further than 100 m from her; the wheel opens for half a second,
//! then its needle goes round from the top, the first time in a second and
//! each time after a fifth longer (to at most 7 s); its wedges (the bomb's
//! script sets them) are the quarter turns about the bottom and the top; a
//! press counts up to a tenth of a second after the needle has left a
//! wedge, and a press outside one undoes those made.

use std::f32::consts::{PI, TAU};

use m2_math::glam::{Mat4, Vec2, Vec3};
use m2_render::View;

/// The satellite looks down from this high over the ground she stands on
/// (metres), showing this much ground from the screen's top to its bottom
/// (metres); near and far planes (metres).
const SAT_HEIGHT: f32 = 170.0;
const SAT_SPAN: f32 = 110.0;
const SAT_NEAR: f32 = 5.0;
const SAT_FAR: f32 = 3000.0;
/// The view moves this fast with the keys (m/s), this far for a radian of
/// the mouse's turn (metres), and no further than this from her (metres).
const VIEW_SPEED: f32 = 5.0;
const VIEW_PER_RADIAN: f32 = 150.0;
pub const VIEW_REACH: f32 = 100.0;
/// The designator comes up this long before the view opens (seconds; the
/// videos: a moment, up to a second).
const EQUIP_TIME: f32 = 0.5;
/// The timing wheel opens over this long before its needle starts
/// (seconds). The needle goes round clockwise from the top in this long
/// the first time, each time after this many times as long, to at most
/// this long (seconds); a press this long after it has left a wedge still
/// counts (seconds).
const WHEEL_OPEN: f32 = 0.5;
const FIRST_LAP: f32 = 1.0;
const LAP_GROWTH: f32 = 1.2;
const LAST_LAP: f32 = 7.0;
const LATE_PRESS: f32 = 0.1;
/// The view moved this far from where it was as the needle started
/// (metres) puts the wheel away.
const WHEEL_MOVE: f32 = 5.0;
/// The wedges' middles, radians clockwise from the top (the bomb's script:
/// from 135 to 225 degrees, and from -45 to 45), each reaching this far
/// either side.
pub const SECTORS: [f32; 2] = [PI, 0.0];
pub const WEDGE: f32 = PI / 4.0;
/// A press's check or cross shows this long (seconds); after the last
/// check the view goes back to her this long after.
const MARK_SHOWS: f32 = 0.3;
const CHECK_SHOWS: f32 = 0.2;
/// The bomb lands this long after the last press (seconds), and its final
/// strike this long after (the template's TimerResponse).
const FALL_TIME: f32 = 7.2;
const FINAL_AFTER: f32 = 2.0;
/// The flash fades over this long (seconds), and fills the screen out to
/// this far from the bomb, fainter to this far (metres).
const FLASH_TIME: f32 = 1.2;
const FLASH_FULL: f32 = 60.0;
const FLASH_REACH: f32 = 600.0;
/// The template's camera shake (amplitude 10 for 1.7 s), as a turn of
/// the camera this big (radians) close by, nothing from this far
/// (metres).
pub const SHAKE_TIME: f32 = 1.7;
const SHAKE_TURN: f32 = 0.025;
const SHAKE_REACH: f32 = 250.0;
/// With a target only a strike brings down marked and nothing to call
/// one with, one is given her free after this long (seconds), lest she be
/// stuck: the scripts give it themselves well before.
pub const FALLBACK_AFTER: f32 = 90.0;

/// The events the satellite interface posts for the scripts
/// (`mrxguisatellite`): the view opened, the wheel's needle started, a
/// press in a wedge or out of one, and the view closed with a target sent
/// or without.
pub const TARGETTING_START: &str = "Satellite Targetting Start";
pub const MINIGAME_START: &str = "Satellite Minigame Start";
pub const SECTOR_HIT: &str = "Satellite Minigame Sector Hit";
pub const SECTOR_MISS: &str = "Satellite Minigame Sector Miss";
pub const TARGETTING_SUCCESS: &str = "Satellite Targetting Success";
pub const TARGETTING_CANCELLED: &str = "Satellite Targetting Cancelled";
/// The PDA opened and closed, as the scripts hear it.
pub const PDA_OPEN: &str = "PDA Open";
pub const PDA_CLOSE: &str = "PDA Close";

/// The support items there are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The Satellite Guided Bomb, called with the satellite designator.
    SatelliteBomb,
}

impl Kind {
    /// As the support box names it.
    pub fn name(self) -> &'static str {
        match self {
            Kind::SatelliteBomb => "Satellite Guided Bomb",
        }
    }

    /// How it is called in, as the support box says under it.
    pub fn method(self) -> &'static str {
        match self {
            Kind::SatelliteBomb => "Satellite",
        }
    }

    /// The template of what she calls it with.
    pub fn designator(self) -> &'static str {
        match self {
            Kind::SatelliteBomb => "Satellite Designator",
        }
    }

    /// The item a free support item of the scripts' is
    /// (`MrxSupportData.AddFreebie`'s key: the first contract's
    /// `"VzaCon01_SatBomb"`).
    pub fn from_freebie(key: &str) -> Option<Kind> {
        key.to_ascii_lowercase()
            .contains("satbomb")
            .then_some(Kind::SatelliteBomb)
    }
}

/// A support item she holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    pub kind: Kind,
    pub count: u32,
    /// Costs her nothing (the support box's "FREE").
    pub free: bool,
}

/// Where a call stands.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Phase {
    #[default]
    Idle,
    /// The support menu is open.
    Menu,
    /// The designator coming up: seconds until the view opens.
    Holding { left: f32 },
    /// Looking down through the satellite: the view's middle (x, z) and
    /// seconds since it opened.
    Viewing { at: Vec2, open: f32 },
    /// The timing wheel turns over the confirmed spot (`from`, where the
    /// view was as its needle started; the view may still move a little,
    /// to `at`): seconds since it came up, which
    /// wedges were pressed in, and the last press while its mark shows: in
    /// a wedge or not, seconds since.
    Timing {
        at: Vec2,
        from: Vec2,
        open: f32,
        turned: f32,
        hit: [bool; 2],
        mark: Option<(bool, f32)>,
    },
    /// Both wedges pressed in: the check shows, this many seconds more.
    Confirmed {
        at: Vec2,
        open: f32,
        turned: f32,
        left: f32,
    },
}

/// What she does at the support this tick, from her keys.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Call {
    /// The support key: the support menu opens, or closes.
    pub menu: bool,
    pub use_pressed: bool,
    /// Fire held.
    pub fire: bool,
    /// Steering the view with the keys: right and up the screen, -1 to 1.
    pub steer: Vec2,
    /// The mouse's turn (radians: left, up), which steers the view too.
    pub look: Vec2,
    /// A weapon taken up: the call is put away.
    pub select: bool,
    /// She was hurt: the designation is called off (a tutorial line says
    /// so: taking damage cancels it).
    pub hurt: bool,
    /// The PDA key: the PDA opens, or closes.
    pub pda: bool,
}

/// What came down this step, where (x, z).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Impact {
    Bomb(Vec2),
    /// The bomb's final strike, [`FINAL_AFTER`] seconds on.
    Final(Vec2),
}

impl Impact {
    pub fn at(self) -> Vec2 {
        match self {
            Impact::Bomb(at) | Impact::Final(at) => at,
        }
    }
}

/// What the HUD shows of the timing wheel.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WheelHud {
    /// The needle: radians clockwise from the top.
    pub angle: f32,
    /// The wedges: their middles (radians clockwise from the top), and
    /// whether pressed in; how far each reaches either side (radians).
    pub sectors: [(f32, bool); 2],
    pub wedge: f32,
    /// The last press's mark while it shows: a check (in a wedge) or a
    /// cross.
    pub mark: Option<bool>,
    /// 1 while it turns, growing from 0 as it opens and shrinking to 0 as
    /// it goes once both are pressed.
    pub size: f32,
}

/// What the HUD shows of the support.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SupportHud {
    /// The item ready to call: its name, how it is called in, how many,
    /// and whether free.
    pub item: Option<(String, String, u32, bool)>,
    /// The support menu is open.
    pub menu: bool,
    /// She holds the designator: its name.
    pub designator: Option<String>,
    /// Looking through the satellite: the view's middle (on the ground
    /// she stands on), seconds since it opened (the "time of operation")
    /// and the cost ($K).
    pub satellite: Option<(Vec3, f32, f32)>,
    /// The timing wheel.
    pub wheel: Option<WheelHud>,
    /// Seconds until the bomb on its way lands.
    pub inbound: Option<f32>,
    /// The impact's flash: 1 at its brightest.
    pub flash: f32,
    /// The PDA is open.
    pub pda: bool,
}

/// Her support: the items she holds, the call under way, the bombs on
/// their way.
#[derive(Debug, Default)]
pub struct Support {
    pub items: Vec<Item>,
    pub phase: Phase,
    /// Bombs on their way, and final strikes to come: where, seconds left.
    inbound: Vec<(Vec2, f32)>,
    finals: Vec<(Vec2, f32)>,
    /// Fire was held last tick: a press is fire going down.
    fire_was: bool,
    /// The screen's flash (1 at its brightest), and the camera's shake:
    /// how far it turns (radians) and seconds left.
    pub flash: f32,
    pub shake: (f32, f32),
    /// What came down in the last step.
    pub landed: Vec<Impact>,
    /// The events for the scripts from the last step
    /// ([`TARGETTING_START`]..., [`PDA_OPEN`], [`PDA_CLOSE`]).
    pub posts: Vec<&'static str>,
    /// The PDA is open.
    pub pda: bool,
    /// Seconds a strike has been wanted with nothing to call it with
    /// ([`fallback`](Self::fallback)).
    wanted: f32,
}

/// The wedge of the wheel the needle at `angle` (radians clockwise from
/// the top) is in, if any.
pub fn sector(angle: f32) -> Option<usize> {
    SECTORS
        .iter()
        .position(|&mid| ((angle - mid + PI).rem_euclid(TAU) - PI).abs() <= WEDGE)
}

/// The wedge a press with the needle at `angle` on lap `lap` counts in:
/// the needle is in it, or left it no more than [`LATE_PRESS`] ago.
fn pressed_in(lap: u32, angle: f32) -> Option<usize> {
    let late = TAU * LATE_PRESS / lap_time(lap);
    SECTORS
        .iter()
        .position(|&mid| (angle - (mid - WEDGE)).rem_euclid(TAU) <= 2.0 * WEDGE + late)
}

/// How long the needle's lap `lap` (from 0) takes, seconds.
fn lap_time(lap: u32) -> f32 {
    (FIRST_LAP * LAP_GROWTH.powi(lap.min(64) as i32)).min(LAST_LAP)
}

/// The needle `turned` seconds after the wheel came up: its lap (from 0)
/// and its angle (radians clockwise from the top, from 0 again each lap);
/// none while the wheel opens.
pub fn needle(turned: f32) -> Option<(u32, f32)> {
    let mut t = turned - WHEEL_OPEN;
    if t < 0.0 {
        return None;
    }
    let mut lap = 0;
    loop {
        let length = lap_time(lap);
        if length >= LAST_LAP {
            let laps = (t / length).floor();
            let into = (t - laps * length).max(0.0);
            return Some((lap + laps as u32, (into / length * TAU).rem_euclid(TAU)));
        }
        if t < length {
            return Some((lap, t / length * TAU));
        }
        t -= length;
        lap += 1;
    }
}

/// The satellite's view: straight down from high over `at` (x, z), the
/// ground about `ground` high, north (+z) up the screen.
pub fn satellite_view(at: Vec2, ground: f32, aspect: f32) -> View {
    let eye = Vec3::new(at.x, ground + SAT_HEIGHT, at.y);
    let fov = 2.0 * (SAT_SPAN / 2.0 / SAT_HEIGHT).atan();
    View {
        view: Mat4::look_to_rh(eye, Vec3::NEG_Y, Vec3::Z),
        // Reversed depth, as `View::perspective`.
        projection: Mat4::perspective_rh(fov, aspect, SAT_FAR, SAT_NEAR),
        near: SAT_NEAR,
        far: SAT_FAR,
        lighting: Default::default(),
    }
}

impl Support {
    /// The first item she can call.
    pub fn ready(&self) -> Option<Item> {
        self.items.iter().copied().find(|i| i.count > 0)
    }

    /// Gives her `quantity` free `kind` (the scripts' `AddFreebie`), to
    /// hold at most `limit` if given.
    pub fn grant(&mut self, kind: Kind, quantity: u32, limit: Option<u32>) {
        let most = limit.unwrap_or(u32::MAX);
        match self.items.iter_mut().find(|i| i.kind == kind) {
            Some(i) => {
                i.count = i.count.saturating_add(quantity).min(most);
                i.free = true;
            }
            None => self.items.push(Item {
                kind,
                count: quantity.min(most),
                free: true,
            }),
        }
    }

    /// Takes her free `kind` back (the scripts' `RemoveFreebie`).
    pub fn take_back(&mut self, kind: Kind) {
        self.items.retain(|i| i.kind != kind);
    }

    /// Keeps her from being stuck without a strike the mission needs:
    /// while `wanted` (a target only a strike brings down is marked) and
    /// she has no `kind`, isn't calling one and none is on its way, the
    /// seconds add up; after [`FALLBACK_AFTER`] she is given one free.
    /// Returns whether she was.
    pub fn fallback(&mut self, kind: Kind, wanted: bool, dt: f32) -> bool {
        let has = self.items.iter().any(|i| i.kind == kind && i.count > 0);
        if !wanted || has || self.busy() || !self.inbound.is_empty() {
            self.wanted = 0.0;
            return false;
        }
        self.wanted += dt;
        if self.wanted < FALLBACK_AFTER {
            return false;
        }
        self.wanted = 0.0;
        self.grant(kind, 1, Some(1));
        true
    }

    /// She holds the designator, before the view opens.
    pub fn holding(&self) -> bool {
        matches!(self.phase, Phase::Holding { .. })
    }

    /// Where the satellite looks (x, z), while she looks through it.
    pub fn looking(&self) -> Option<Vec2> {
        match self.phase {
            Phase::Viewing { at, .. } | Phase::Timing { at, .. } | Phase::Confirmed { at, .. } => {
                Some(at)
            }
            _ => None,
        }
    }

    /// The call or the PDA has her keys (the menu open, the designator
    /// coming up, the satellite's view up, the PDA open): she doesn't
    /// move, shoot or use what is near.
    pub fn busy(&self) -> bool {
        self.phase != Phase::Idle || self.pda
    }

    /// Seconds until the soonest bomb lands.
    pub fn inbound(&self) -> Option<f32> {
        self.inbound.iter().map(|b| b.1).min_by(f32::total_cmp)
    }

    /// The wheel's needle is in a wedge not yet pressed in (the moment to
    /// press).
    pub fn wheel_ready(&self) -> bool {
        matches!(self.phase, Phase::Timing { turned, hit, .. }
            if needle(turned).and_then(|n| sector(n.1)).is_some_and(|i| !hit[i]))
    }

    /// One tick of `dt` seconds with her at `her`.
    pub fn step(&mut self, call: Call, her: Vec3, dt: f32) {
        self.landed.clear();
        self.posts.clear();
        self.flash = (self.flash - dt / FLASH_TIME).max(0.0);
        self.shake.1 = (self.shake.1 - dt).max(0.0);
        let fired = call.fire && !self.fire_was;
        self.fire_was = call.fire;
        let press = fired || call.use_pressed;
        let here = Vec2::new(her.x, her.z);
        let back = call.menu || call.select;
        // The PDA opens and shuts with its key while no call is under way;
        // while it is open the call's keys do nothing.
        if call.pda && self.phase == Phase::Idle {
            self.pda = !self.pda;
            self.posts.push(if self.pda { PDA_OPEN } else { PDA_CLOSE });
        }
        // Screen right is the world's -x, looking down with north (+z) up;
        // the mouse to the right turns the look negative.
        let steer = |at: Vec2| {
            let moved = Vec2::new(-call.steer.x, call.steer.y) * VIEW_SPEED * dt
                + call.look * VIEW_PER_RADIAN;
            here + (at + moved - here).clamp_length_max(VIEW_REACH)
        };
        self.phase = match self.phase {
            _ if self.pda => self.phase,
            Phase::Idle if call.menu && self.ready().is_some() => Phase::Menu,
            Phase::Idle => Phase::Idle,
            Phase::Menu if back => Phase::Idle,
            Phase::Menu if call.use_pressed => Phase::Holding { left: EQUIP_TIME },
            Phase::Menu => Phase::Menu,
            Phase::Holding { .. } if back || call.hurt || self.ready().is_none() => Phase::Idle,
            Phase::Holding { left } if left <= dt => {
                self.posts.push(TARGETTING_START);
                Phase::Viewing {
                    at: here,
                    open: 0.0,
                }
            }
            Phase::Holding { left } => Phase::Holding { left: left - dt },
            Phase::Viewing { .. } | Phase::Timing { .. } if back || call.hurt => {
                self.posts.push(TARGETTING_CANCELLED);
                Phase::Idle
            }
            Phase::Viewing { at, open } if press => Phase::Timing {
                at,
                from: at,
                open: open + dt,
                turned: 0.0,
                hit: [false; 2],
                mark: None,
            },
            Phase::Viewing { at, open } => Phase::Viewing {
                at: steer(at),
                open: open + dt,
            },
            Phase::Timing {
                at,
                from,
                open,
                turned,
                mut hit,
                mark,
            } => {
                let open = open + dt;
                let at = steer(at);
                // The spot she mustn't move off is where the view is as the
                // needle starts, half a second after the press (round 96:
                // the satellite interface takes it then).
                let from = if needle(turned).is_none() { at } else { from };
                let mut mark = mark.map(|(h, t)| (h, t + dt)).filter(|m| m.1 < MARK_SHOWS);
                // A press counts once the needle goes: in a wedge not yet
                // pressed in, or out of them, which undoes those made.
                if press && let Some((lap, angle)) = needle(turned) {
                    match pressed_in(lap, angle).filter(|&i| !hit[i]) {
                        Some(i) => {
                            hit[i] = true;
                            mark = Some((true, 0.0));
                            self.posts.push(SECTOR_HIT);
                        }
                        None => {
                            hit = [false; 2];
                            mark = Some((false, 0.0));
                            self.posts.push(SECTOR_MISS);
                        }
                    }
                }
                if needle(turned).is_none() && needle(turned + dt).is_some() {
                    self.posts.push(MINIGAME_START);
                }
                if at.distance(from) > WHEEL_MOVE {
                    // Moved off the spot: the wheel goes, the view stays.
                    Phase::Viewing { at, open }
                } else if hit.iter().all(|&h| h) {
                    if let Some(i) = self.items.iter_mut().find(|i| i.count > 0) {
                        i.count -= 1;
                    }
                    self.inbound.push((from, FALL_TIME));
                    Phase::Confirmed {
                        at: from,
                        open,
                        turned: turned + dt,
                        left: CHECK_SHOWS,
                    }
                } else {
                    Phase::Timing {
                        at,
                        from,
                        open,
                        turned: turned + dt,
                        hit,
                        mark,
                    }
                }
            }
            Phase::Confirmed { left, .. } if left <= dt => {
                self.posts.push(TARGETTING_SUCCESS);
                Phase::Idle
            }
            Phase::Confirmed {
                at,
                open,
                turned,
                left,
            } => Phase::Confirmed {
                at,
                open: open + dt,
                turned: turned + dt,
                left: left - dt,
            },
        };
        let mut landed = Vec::new();
        self.inbound.retain_mut(|(at, left)| {
            *left -= dt;
            if *left > 0.0 {
                return true;
            }
            landed.push(Impact::Bomb(*at));
            false
        });
        self.finals.retain_mut(|(at, left)| {
            *left -= dt;
            if *left > 0.0 {
                return true;
            }
            landed.push(Impact::Final(*at));
            false
        });
        for &impact in &landed {
            if let Impact::Bomb(at) = impact {
                self.finals.push((at, FINAL_AFTER));
                let d = at.distance(here);
                let flash = 1.0 - ((d - FLASH_FULL) / (FLASH_REACH - FLASH_FULL)).clamp(0.0, 1.0);
                self.flash = self.flash.max(flash);
                let turn = SHAKE_TURN * (1.0 - d / SHAKE_REACH).clamp(0.0, 1.0);
                if turn > 0.0 {
                    self.shake = (
                        turn.max(self.shake.0 * self.shake.1 / SHAKE_TIME),
                        SHAKE_TIME,
                    );
                }
            }
        }
        self.landed = landed;
    }

    /// The camera's turn from the shake now: (about x, about y), radians.
    pub fn shake_turn(&self) -> Option<(f32, f32)> {
        let (turn, left) = self.shake;
        if left <= 0.0 || turn <= 0.0 {
            return None;
        }
        let k = turn * left / SHAKE_TIME;
        let t = (SHAKE_TIME - left) * 40.0;
        Some(((t * 1.7).sin() * k, (t * 2.3 + 1.0).cos() * k))
    }

    /// What the HUD shows, with her standing at height `ground`.
    pub fn hud(&self, ground: f32) -> SupportHud {
        let ready = self.ready();
        let on = |at: Vec2| Vec3::new(at.x, ground, at.y);
        // Free: it costs nothing.
        let cost = 0.0;
        // The needle stands at the top while the wheel opens.
        let wheel = |turned: f32, hit: [bool; 2], mark: Option<bool>, size: f32| {
            let (angle, opened) =
                needle(turned).map_or((0.0, (turned / WHEEL_OPEN).clamp(0.0, 1.0)), |n| (n.1, 1.0));
            WheelHud {
                angle,
                sectors: [(SECTORS[0], hit[0]), (SECTORS[1], hit[1])],
                wedge: WEDGE,
                mark,
                size: size * opened,
            }
        };
        let (satellite, wheel) = match self.phase {
            Phase::Viewing { at, open } => (Some((on(at), open, cost)), None),
            Phase::Timing {
                at,
                open,
                turned,
                hit,
                mark,
                ..
            } => (
                Some((on(at), open, cost)),
                Some(wheel(turned, hit, mark.map(|m| m.0), 1.0)),
            ),
            Phase::Confirmed {
                at,
                open,
                turned,
                left,
            } => (
                Some((on(at), open, cost)),
                Some(wheel(turned, [true; 2], Some(true), left / CHECK_SHOWS)),
            ),
            _ => (None, None),
        };
        SupportHud {
            item: ready.map(|i| {
                (
                    i.kind.name().to_owned(),
                    i.kind.method().to_owned(),
                    i.count,
                    i.free,
                )
            }),
            menu: self.phase == Phase::Menu,
            designator: (!matches!(self.phase, Phase::Idle | Phase::Menu)).then(|| {
                ready
                    .map_or(Kind::SatelliteBomb, |i| i.kind)
                    .designator()
                    .to_owned()
            }),
            satellite,
            wheel,
            inbound: self.inbound(),
            flash: self.flash,
            pda: self.pda,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 0.05;

    /// Steps `s` once, keeping what it posted.
    fn step(s: &mut Support, call: Call, her: Vec3, posts: &mut Vec<&'static str>) {
        s.step(call, her, DT);
        posts.extend(s.posts.iter().copied());
    }

    fn press_fire(s: &mut Support, her: Vec3, posts: &mut Vec<&'static str>) {
        let fire = Call {
            fire: true,
            ..Call::default()
        };
        step(s, fire, her, posts);
        step(s, Call::default(), her, posts);
    }

    /// Takes up the designator from the menu and waits for the view.
    fn open_view(s: &mut Support, her: Vec3, posts: &mut Vec<&'static str>) {
        for call in [
            Call {
                menu: true,
                ..Call::default()
            },
            Call {
                use_pressed: true,
                ..Call::default()
            },
        ] {
            step(s, call, her, posts);
        }
        assert!(s.holding());
        assert_eq!(
            s.hud(0.0).designator.as_deref(),
            Some("Satellite Designator")
        );
        while s.holding() {
            step(s, Call::default(), her, posts);
        }
    }

    /// Waits until the needle is (`into`) or isn't in a wedge not yet
    /// pressed in.
    fn wait_for(s: &mut Support, her: Vec3, into: bool, posts: &mut Vec<&'static str>) {
        for _ in 0..400 {
            if s.wheel_ready() == into {
                return;
            }
            step(s, Call::default(), her, posts);
        }
        panic!("the needle never came round: {:?}", s.phase);
    }

    #[test]
    fn a_called_bomb_lands_where_the_view_was_confirmed() {
        let her = Vec3::new(-1470.0, -18.0, 2760.0);
        let gate = Vec2::new(-1472.0, 2825.0);
        let mut s = Support::default();
        let mut posts = Vec::new();
        // Nothing to call yet: the support key does nothing.
        let menu = Call {
            menu: true,
            ..Call::default()
        };
        step(&mut s, menu, her, &mut posts);
        assert_eq!(s.phase, Phase::Idle);
        // The scripts give one, then one more to hold at most one.
        s.grant(Kind::SatelliteBomb, 1, Some(1));
        s.grant(Kind::SatelliteBomb, 1, Some(1));
        assert_eq!(s.ready().map(|i| (i.count, i.free)), Some((1, true)));
        open_view(&mut s, her, &mut posts);
        assert_eq!(posts, [TARGETTING_START]);
        assert!(s.busy());
        assert_eq!(s.looking(), Some(Vec2::new(her.x, her.z)));
        // Steered over the gate, up the screen (north), at 5 m/s.
        let mut seconds = 0.0;
        for _ in 0..1000 {
            let at = s.looking().unwrap();
            let to = gate - at;
            if to.length() < 0.5 {
                break;
            }
            // Screen right is -x.
            let steer = Vec2::new(-to.x, to.y).clamp_length_max(1.0);
            let call = Call {
                steer,
                ..Call::default()
            };
            step(&mut s, call, her, &mut posts);
            seconds += DT;
        }
        assert!(s.looking().unwrap().distance(gate) < 1.0);
        assert!((12.0..14.0).contains(&seconds), "{seconds}");
        press_fire(&mut s, her, &mut posts);
        assert!(matches!(s.phase, Phase::Timing { .. }));
        // The wheel opens with its needle at the top: presses don't count
        // yet, and it starts half a second on.
        assert_eq!(s.hud(0.0).wheel.map(|w| w.angle), Some(0.0));
        assert!(s.hud(0.0).wheel.unwrap().size < 1.0);
        press_fire(&mut s, her, &mut posts);
        assert_eq!(posts.last(), Some(&TARGETTING_START));
        wait_for(&mut s, her, true, &mut posts);
        assert_eq!(posts.last(), Some(&MINIGAME_START));
        // A press out of the wedges misses; one in each sends the bomb.
        wait_for(&mut s, her, false, &mut posts);
        // Well past the wedge, beyond a late press.
        for _ in 0..4 {
            step(&mut s, Call::default(), her, &mut posts);
        }
        press_fire(&mut s, her, &mut posts);
        assert_eq!(posts.last(), Some(&SECTOR_MISS));
        assert!(matches!(
            s.phase,
            Phase::Timing {
                hit: [false, false],
                ..
            }
        ));
        assert_eq!(s.hud(0.0).wheel.map(|w| w.mark), Some(Some(false)));
        // In a wedge: a hit; the same wedge again misses, undoing it.
        wait_for(&mut s, her, true, &mut posts);
        let press = Call {
            use_pressed: true,
            ..Call::default()
        };
        step(&mut s, press, her, &mut posts);
        assert_eq!(posts.last(), Some(&SECTOR_HIT));
        assert_eq!(s.hud(0.0).wheel.map(|w| w.mark), Some(Some(true)));
        assert!(!s.wheel_ready());
        step(&mut s, press, her, &mut posts);
        assert_eq!(posts.last(), Some(&SECTOR_MISS));
        assert!(matches!(
            s.phase,
            Phase::Timing {
                hit: [false, false],
                ..
            }
        ));
        for _ in 0..2 {
            wait_for(&mut s, her, false, &mut posts);
            wait_for(&mut s, her, true, &mut posts);
            step(&mut s, press, her, &mut posts);
            assert_eq!(posts.last(), Some(&SECTOR_HIT));
        }
        assert!(matches!(s.phase, Phase::Confirmed { .. }));
        assert_eq!(s.ready(), None);
        assert!(s.inbound().is_some());
        let mut landed = Vec::new();
        let mut back = None;
        for tick in 0..400 {
            step(&mut s, Call::default(), her, &mut posts);
            if back.is_none() && s.phase == Phase::Idle {
                back = Some(tick);
                assert_eq!(posts.last(), Some(&TARGETTING_SUCCESS));
            }
            landed.extend(s.landed.iter().map(|&l| (tick, l)));
        }
        assert_eq!(landed.len(), 2, "{landed:?}");
        assert!(matches!(landed[0].1, Impact::Bomb(at) if at.distance(gate) < 1.0));
        assert!(matches!(landed[1].1, Impact::Final(_)));
        // The view back to her at once, the bomb 7 s later, the final
        // strike 2 s after.
        let back = back.unwrap();
        assert!(back <= 4, "{back}");
        assert!((138..=145).contains(&(landed[0].0 - back)), "{landed:?}");
        assert!(
            (40..=41).contains(&(landed[1].0 - landed[0].0)),
            "{landed:?}"
        );
        // She was 65 m off: a bright flash, faded by now.
        assert_eq!(s.flash, 0.0);
        // Spent: none given while none is wanted.
        assert!(!s.fallback(Kind::SatelliteBomb, false, 100.0));
        assert_eq!(s.ready(), None);
    }

    #[test]
    fn the_view_stays_within_reach_and_backs_out() {
        let her = Vec3::ZERO;
        let mut s = Support::default();
        let mut posts = Vec::new();
        s.grant(Kind::SatelliteBomb, 1, None);
        open_view(&mut s, her, &mut posts);
        let right = Call {
            steer: Vec2::new(1.0, 0.0),
            ..Call::default()
        };
        for _ in 0..1000 {
            step(&mut s, right, her, &mut posts);
        }
        let at = s.looking().unwrap();
        assert!((at.length() - VIEW_REACH).abs() < 0.1, "{at}");
        // Right on the screen is -x.
        assert!(at.x < 0.0);
        let select = Call {
            select: true,
            ..Call::default()
        };
        step(&mut s, select, her, &mut posts);
        assert_eq!(s.phase, Phase::Idle);
        assert_eq!(posts, [TARGETTING_START, TARGETTING_CANCELLED]);
        assert_eq!(s.ready().map(|i| i.count), Some(1));
        // Hurt in the view, the call is off.
        open_view(&mut s, her, &mut posts);
        press_fire(&mut s, her, &mut posts);
        assert!(matches!(s.phase, Phase::Timing { .. }));
        let hurt = Call {
            hurt: true,
            ..Call::default()
        };
        step(&mut s, hurt, her, &mut posts);
        assert_eq!(s.phase, Phase::Idle);
        assert_eq!(posts.last(), Some(&TARGETTING_CANCELLED));
        assert_eq!(s.ready().map(|i| i.count), Some(1));
        // Moving the view off the spot puts the wheel away.
        open_view(&mut s, her, &mut posts);
        press_fire(&mut s, her, &mut posts);
        for _ in 0..40 {
            step(&mut s, right, her, &mut posts);
        }
        assert!(matches!(s.phase, Phase::Viewing { .. }), "{:?}", s.phase);
        // Taken back by the scripts, there is nothing to call.
        step(&mut s, select, her, &mut posts);
        s.take_back(Kind::SatelliteBomb);
        assert_eq!(s.ready(), None);
    }

    #[test]
    fn the_wheel_opens_then_its_needle_slows_lap_by_lap() {
        assert_eq!(needle(0.4), None);
        assert_eq!(needle(0.5), Some((0, 0.0)));
        let (lap, a) = needle(0.75).unwrap();
        assert!(lap == 0 && (a - PI / 2.0).abs() < 1e-4);
        // The second lap takes 1.2 s, from the top again.
        let (lap, a) = needle(1.5 + 0.6).unwrap();
        assert!(lap == 1 && (a - PI).abs() < 1e-3, "{a}");
        // At most 7 s a lap.
        let mut t = WHEEL_OPEN;
        for lap in 0..20 {
            t += lap_time(lap);
        }
        let (lap, a) = needle(t + 3.5).unwrap();
        assert!(lap == 20 && (a - PI).abs() < 1e-3, "{lap} {a}");
        // The wedges: a quarter turn about the bottom and the top; a press
        // counts up to a tenth of a second (36 degrees on the first lap)
        // past one's far edge.
        assert_eq!(sector(0.0), Some(1));
        assert_eq!(sector(TAU - 0.1), Some(1));
        assert_eq!(sector(PI + 0.7), Some(0));
        assert_eq!(sector(PI / 2.0), None);
        let edge = PI / 4.0;
        assert_eq!(pressed_in(0, edge + 30f32.to_radians()), Some(1));
        assert_eq!(pressed_in(0, edge + 40f32.to_radians()), None);
        assert_eq!(pressed_in(0, -edge - 0.05), None);
        let in_wedges = (0..1000)
            .filter(|&k| sector(k as f32 / 1000.0 * TAU).is_some())
            .count();
        assert!((495..=505).contains(&in_wedges), "{in_wedges}");
    }

    #[test]
    fn the_pda_opens_and_closes_with_no_call_under_way() {
        let her = Vec3::ZERO;
        let mut s = Support::default();
        let mut posts = Vec::new();
        s.grant(Kind::SatelliteBomb, 1, Some(1));
        let pda = Call {
            pda: true,
            ..Call::default()
        };
        step(&mut s, pda, her, &mut posts);
        assert!(s.pda && s.busy() && s.hud(0.0).pda);
        // The support key does nothing meanwhile.
        let menu = Call {
            menu: true,
            ..Call::default()
        };
        step(&mut s, menu, her, &mut posts);
        assert_eq!(s.phase, Phase::Idle);
        step(&mut s, pda, her, &mut posts);
        assert!(!s.pda && !s.busy());
        assert_eq!(posts, [PDA_OPEN, PDA_CLOSE]);
        // Nor does the PDA's key at the menu.
        step(&mut s, menu, her, &mut posts);
        step(&mut s, pda, her, &mut posts);
        assert!(!s.pda && s.phase == Phase::Menu);
    }

    #[test]
    fn a_bomb_is_given_should_the_scripts_never_give_one() {
        let mut s = Support::default();
        assert_eq!(
            Kind::from_freebie("VzaCon01_SatBomb"),
            Some(Kind::SatelliteBomb)
        );
        assert_eq!(Kind::from_freebie("Flare"), None);
        // Wanted for a while, then not: the count starts again.
        assert!(!s.fallback(Kind::SatelliteBomb, true, FALLBACK_AFTER - 1.0));
        assert!(!s.fallback(Kind::SatelliteBomb, false, 0.1));
        assert!(!s.fallback(Kind::SatelliteBomb, true, FALLBACK_AFTER - 1.0));
        assert!(s.fallback(Kind::SatelliteBomb, true, 1.5));
        assert_eq!(s.ready().map(|i| (i.count, i.free)), Some((1, true)));
        // She has one: no more.
        assert!(!s.fallback(Kind::SatelliteBomb, true, FALLBACK_AFTER * 2.0));
        assert_eq!(s.ready().map(|i| i.count), Some(1));
    }

    #[test]
    fn the_satellite_looks_down_with_north_up() {
        let v = satellite_view(Vec2::new(100.0, 200.0), 10.0, 16.0 / 9.0);
        let clip = |p: Vec3| {
            let c = v.projection * v.view * p.extend(1.0);
            c.truncate() / c.w
        };
        let middle = clip(Vec3::new(100.0, 10.0, 200.0));
        assert!(middle.x.abs() < 1e-4 && middle.y.abs() < 1e-4);
        assert!(clip(Vec3::new(100.0, 10.0, 230.0)).y > 0.1);
        assert!(clip(Vec3::new(70.0, 10.0, 200.0)).x > 0.1);
        // 110 m of ground from the top of the screen to the bottom.
        assert!((clip(Vec3::new(100.0, 10.0, 255.0)).y - 1.0).abs() < 1e-3);
        // In front of the camera, within depth.
        assert!((0.0..=1.0).contains(&middle.z));
    }
}
