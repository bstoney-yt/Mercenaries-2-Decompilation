//! The grappling hook: she hooks a helicopter and is reeled in to hijack it.
//!
//! The game's own hints say how it is used: "Helicopters can be hijacked
//! with a grappling hook", "To hijack a helicopter get as close to it as
//! you can", "Then aim at the helicopter and press [action] when
//! prompted"; once on the line the prompt is "Stop grappling". The hero's
//! data says how far and how fast ([`m2_engine::grapple::Grapple`]: the
//! hook reaches 65 m, flies at 55 m/s for at most 10 s, reels her in at
//! 10 m/s and lets go 3 m short). The scripts turn it on once the PMC has
//! a mechanic (`Player.SetGrappleEnabled`) and ask whether she is on the
//! line (`Human.IsGrappling`).
//!
//! Here the hook flies from her chest to the helicopter's entrance (its
//! hijack hardpoint's side, the vehicle's door); hooked, she is pulled
//! straight along the line, unaffected by gravity, and 3 m short she is
//! put at the entrance and the helicopter's hijack starts as the use key
//! would start it ([`crate::hijack`]). The helicopters' hijack scripts take
//! only one in the air (`MrxActionHijack.CheckGoodStart`: the helicopter
//! rule set asks `Vehicle.IsFlying`), so on a parked one the hijack is
//! refused, as the game's would be; the alpha's helicopters do not fly
//! with a crew yet. Invented: the aim test (the
//! helicopter within [`AIM_DEGREES`] of where the camera looks), the
//! straight pull, and where the hook starts.

use m2_engine::grapple::Grapple;
use m2_math::glam::Vec3;

use crate::game::Game;

/// How far off where she looks a helicopter may be and still be hooked,
/// degrees (invented).
pub const AIM_DEGREES: f32 = 12.0;

/// The hook leaves her this high above her feet, metres (invented: her
/// chest).
const CHEST: f32 = 1.3;

/// Where the line is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Line {
    /// Not fired.
    #[default]
    Idle,
    /// The hook flies toward vehicle `vehicle`: how far it has come and
    /// how long it has flown.
    Flying {
        vehicle: usize,
        flown: f32,
        time: f32,
    },
    /// Hooked to vehicle `vehicle`: she is pulled in.
    Reeling {
        vehicle: usize,
        time: f32,
        /// Where the pull has her (kept, so that her own fall between
        /// ticks does not drag the line down).
        at: Vec3,
    },
}

/// What a tick of the line came to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    /// The hook caught after this many seconds, this far away.
    Hooked { seconds: f32, metres: f32 },
    /// The hook flew its life without catching.
    Missed,
    /// She was pulled to the entrance in this many seconds.
    Arrived { seconds: f32 },
}

/// The player's grappling hook.
#[derive(Clone, Debug, Default)]
pub struct Grappler {
    /// Its reach and pull (the hero's data).
    pub params: Grapple,
    /// The scripts (or `--grapple`) let her grapple.
    pub enabled: bool,
    pub line: Line,
    /// The helicopter in her sights that the use key would hook, if any.
    pub target: Option<usize>,
}

/// Whether `to` is within `degrees` across of the way `look` points, from
/// `from`, seen from above (a helicopter overhead is in sight whatever
/// its height).
pub fn in_sight(from: Vec3, look: Vec3, to: Vec3, degrees: f32) -> bool {
    let d = (to - from).with_y(0.0).normalize_or_zero();
    let l = look.with_y(0.0).normalize_or_zero();
    d != Vec3::ZERO && l != Vec3::ZERO && d.dot(l) >= degrees.to_radians().cos()
}

/// One step of a pull from `her` toward `to` at `speed` m/s for `dt`
/// seconds: where she is, and whether she is within `min` metres of it.
pub fn reel(her: Vec3, to: Vec3, speed: f32, min: f32, dt: f32) -> (Vec3, bool) {
    let gap = to - her;
    let left = gap.length();
    if left <= min {
        return (her, true);
    }
    let step = (speed * dt).min(left - min);
    let at = her + gap / left * step;
    (at, left - step <= min + 1e-4)
}

impl Grappler {
    /// She is on the line (the hook out or pulling her).
    pub fn grappling(&self) -> bool {
        self.line != Line::Idle
    }

    /// Lets go of the line ("Stop grappling").
    pub fn release(&mut self) {
        self.line = Line::Idle;
    }

    /// Picks the helicopter she aims at, on foot, between the line's least
    /// and farthest reach; `look` is where the camera looks.
    pub fn aim(&mut self, game: &Game, look: Vec3) {
        self.target = None;
        if !self.enabled || self.grappling() || !game.on_foot {
            return;
        }
        let Some(her) = game.scene.player.as_ref().filter(|p| p.driving.is_none()) else {
            return;
        };
        let from = her.character.position + Vec3::Y * CHEST;
        self.target = (0..game.scene.vehicles.len())
            .filter_map(|i| Some((i, entrance(game, i)?)))
            .filter(|&(_, at)| {
                let d = at.distance(from);
                d > self.params.min && d <= self.params.max && in_sight(from, look, at, AIM_DEGREES)
            })
            .min_by(|a, b| a.1.distance(from).total_cmp(&b.1.distance(from)))
            .map(|(i, _)| i);
    }

    /// Fires the hook at the helicopter in her sights; true if it flies.
    pub fn fire(&mut self) -> bool {
        match self.target.take() {
            Some(vehicle) if !self.grappling() => {
                self.line = Line::Flying {
                    vehicle,
                    flown: 0.0,
                    time: 0.0,
                };
                true
            }
            _ => false,
        }
    }

    /// Moves the hook on, or pulls her in, by `dt` seconds.
    pub fn tick(&mut self, game: &mut Game, dt: f32) -> Option<Outcome> {
        match self.line {
            Line::Idle => None,
            Line::Flying {
                vehicle,
                flown,
                time,
            } => {
                let (Some(at), Some(her)) = (entrance(game, vehicle), her_chest(game)) else {
                    self.line = Line::Idle;
                    return Some(Outcome::Missed);
                };
                let flown = flown + self.params.hook_speed * dt;
                let time = time + dt;
                let gap = at.distance(her);
                if flown >= gap {
                    let at = her - Vec3::Y * CHEST;
                    self.line = Line::Reeling {
                        vehicle,
                        time: 0.0,
                        at,
                    };
                    return Some(Outcome::Hooked {
                        seconds: time,
                        metres: gap,
                    });
                }
                if time >= self.params.hook_life {
                    self.line = Line::Idle;
                    return Some(Outcome::Missed);
                }
                self.line = Line::Flying {
                    vehicle,
                    flown,
                    time,
                };
                None
            }
            Line::Reeling {
                vehicle,
                time,
                at: from,
            } => {
                let Some(at) = entrance(game, vehicle) else {
                    self.line = Line::Idle;
                    return Some(Outcome::Missed);
                };
                let p = game.scene.player.as_mut()?;
                let feet = at - Vec3::Y * CHEST;
                let (to, there) = reel(from, feet, self.params.reel, self.params.min, dt);
                let time = time + dt;
                if there {
                    // Let go at the entrance: on the hull's side, where
                    // the hijack is started from.
                    p.character.position = feet;
                    p.character.velocity = Vec3::ZERO;
                    p.landed = false;
                    self.line = Line::Idle;
                    return Some(Outcome::Arrived { seconds: time });
                }
                p.character.position = to;
                p.character.velocity = Vec3::ZERO;
                self.line = Line::Reeling {
                    vehicle,
                    time,
                    at: to,
                };
                None
            }
        }
    }
}

/// Her chest, where the line leaves her.
fn her_chest(game: &Game) -> Option<Vec3> {
    game.scene
        .player
        .as_ref()
        .map(|p| p.character.position + Vec3::Y * CHEST)
}

/// Where helicopter `i` is got into, world space: its door's side (the
/// vehicle's `door`), else its middle; `None` for anything else, or one
/// wrecked or not in the world.
pub fn entrance(game: &Game, i: usize) -> Option<Vec3> {
    let v = game.scene.vehicles.get(i)?;
    if v.wrecked || !v.spec.is_helicopter() {
        return None;
    }
    let at = game
        .scene
        .world
        .as_ref()?
        .physics
        .vehicle_transform(v.id?)?;
    Some(at.transform_point3(v.spec.door.unwrap_or(Vec3::ZERO)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn she_is_reeled_at_the_reel_speed_and_let_go_short_of_it() {
        let p = Grapple::default();
        let to = Vec3::new(0.0, 20.0, 40.0);
        let mut her = Vec3::ZERO;
        let mut t = 0.0;
        loop {
            let (at, there) = reel(her, to, p.reel, p.min, 0.05);
            her = at;
            t += 0.05;
            if there {
                break;
            }
            assert!(t < 10.0, "never got there");
        }
        // 44.7 m, let go 3 m short, at 10 m/s.
        let want = (to.length() - p.min) / p.reel;
        assert!((t - want).abs() < 0.06, "{t} s, want {want}");
        assert!((her.distance(to) - p.min).abs() < 1e-3);
    }

    #[test]
    fn a_helicopter_is_in_sight_within_the_aim_cone_only() {
        let from = Vec3::ZERO;
        let look = Vec3::Z;
        assert!(in_sight(from, look, Vec3::new(5.0, 0.0, 40.0), AIM_DEGREES));
        assert!(!in_sight(
            from,
            look,
            Vec3::new(15.0, 0.0, 40.0),
            AIM_DEGREES
        ));
        assert!(!in_sight(
            from,
            look,
            Vec3::new(0.0, 0.0, -40.0),
            AIM_DEGREES
        ));
    }
}
