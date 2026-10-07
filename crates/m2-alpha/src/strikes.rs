//! Air strikes in flight: the aircraft the scripts' strike modules fly
//! over (`Airstrike.Flyby`) and the bombs and missiles they let go
//! (`Airstrike.SpawnOrdnance` and its kin), flown here and reported back
//! (the aircraft over its mark, the ordnance going off) so the scripts
//! carry their strike on as the game's own do.
//!
//! How a bomb flies and what its blast does is its template's
//! ([`m2_engine::ordnance`]): a "Cluster Bomb Projectile" at 80 m/s,
//! leaving "Explosion (Cluster Bomb)".

use m2_engine::physics::GRAVITY;
use m2_engine::{Physics, Templates};
use m2_game::airstrike::{Fuse, Launch};
use m2_game::api::Command;
use m2_math::glam::{Mat4, Quat, Vec3};
use m2_render::DrawQueue;

use crate::flights::Flights;

/// An aircraft flies on this far past its mark before it is gone, metres.
const FLY_ON: f32 = 1500.0;
/// Ordnance the template gives no speed falls at this, m/s.
const FALL_SPEED: f32 = 80.0;
/// Ordnance still flying this long is gone (it fell off the world).
const LIFETIME: f32 = 30.0;

/// An aircraft on its run.
#[derive(Clone, Debug, PartialEq)]
struct Jet {
    guid: u32,
    model: Option<u32>,
    at: Vec3,
    /// Its heading, level, and speed (m/s).
    dir: Vec3,
    speed: f32,
    /// Its mark, and how far on it has flown since it passed it.
    to: Vec3,
    past: Option<f32>,
}

/// A bomb or missile in flight.
#[derive(Clone, Debug, PartialEq)]
struct Shell {
    guid: u32,
    template: Option<u32>,
    model: Option<u32>,
    at: Vec3,
    velocity: Vec3,
    /// Gravity's pull on it, as a share of gravity.
    gravity: f32,
    /// The cone it sees its target in and how fast it turns (degrees).
    homing: Option<(f32, f32)>,
    target: Option<Vec3>,
    fuse: Fuse,
    travelled: f32,
    age: f32,
    /// Its template's timer: it goes off this long after it is made.
    timer: Option<f32>,
}

/// The strikes under way.
#[derive(Debug, Default)]
pub struct Strikes {
    jets: Vec<Jet>,
    shells: Vec<Shell>,
    /// Aircraft over their mark since the last take (guid, where).
    pub arrived: Vec<(u32, Vec3)>,
    /// Ordnance gone off since the last take (guid, template, where).
    pub burst: Vec<(u32, Option<u32>, Vec3)>,
    /// Ordnance gone off in all, and where the first went off.
    pub went_off: u32,
    pub first: Option<Vec3>,
    /// Models to load (taken into [`Flights`]).
    wanted: Vec<u32>,
}

impl Strikes {
    /// Starts what command `c` flies, if it is a strike's.
    pub fn start(&mut self, c: &Command, templates: &Templates) {
        match c {
            Command::Flyby {
                guid,
                what,
                from,
                to,
                speed,
            } => {
                let (from, to) = (Vec3::from(*from), Vec3::from(*to));
                let model = templates.get(what).and_then(|t| t.model);
                self.wanted.extend(model);
                eprintln!("strike: {what} flies from {from:.0} over {to:.0} at {speed} m/s");
                self.jets.push(Jet {
                    guid: *guid,
                    model,
                    at: from,
                    dir: (to - from).with_y(0.0).normalize_or(Vec3::Z),
                    speed: speed.max(1.0),
                    to,
                    past: None,
                });
            }
            Command::Ordnance {
                guid,
                what,
                at,
                launch,
                fuse,
                target,
                ..
            } => {
                let template = templates.id(what);
                let info = template.and_then(|t| templates.ordnance.projectiles.get(&t));
                let model = info
                    .and_then(|i| i.model)
                    .or_else(|| templates.get(what).and_then(|t| t.model));
                self.wanted.extend(model);
                let speed = info.map_or(0.0, |i| i.speed);
                let velocity = match launch {
                    Launch::Velocity(v) => Vec3::from(*v),
                    Launch::Along(d) => {
                        Vec3::from(*d).normalize_or(Vec3::NEG_Y)
                            * if speed > 0.0 { speed } else { FALL_SPEED }
                    }
                };
                self.shells.push(Shell {
                    guid: *guid,
                    template,
                    model,
                    at: Vec3::from(*at),
                    velocity,
                    gravity: info.map_or(1.0, |i| i.gravity),
                    homing: info.and_then(|i| i.homing),
                    target: target.map(|(_, p)| Vec3::from(p)),
                    fuse: fuse.clone(),
                    travelled: 0.0,
                    age: 0.0,
                    timer: info.and_then(|i| i.timer).map(|t| t.0),
                });
            }
            _ => {}
        }
    }

    /// Flies everything on by `dt` seconds.
    pub fn step(&mut self, physics: &Physics, dt: f32) {
        for j in &mut self.jets {
            j.at += j.dir * j.speed * dt;
            match &mut j.past {
                Some(p) => *p += j.speed * dt,
                None if (j.to - j.at).with_y(0.0).dot(j.dir) <= 0.0 => {
                    j.past = Some(0.0);
                    self.arrived.push((j.guid, j.to));
                }
                None => {}
            }
        }
        self.jets.retain(|j| j.past.is_none_or(|p| p < FLY_ON));
        let mut burst = Vec::new();
        self.shells.retain_mut(|s| {
            s.age += dt;
            if let (Some((cone, turn)), Some(target)) = (s.homing, s.target) {
                home(&mut s.velocity, s.at, target, cone, turn, dt);
            }
            s.velocity.y -= GRAVITY * s.gravity * dt;
            let step = s.velocity * dt;
            let length = step.length();
            let dir = step.normalize_or(Vec3::NEG_Y);
            let hit = physics
                .cast_ray(s.at, dir, length)
                .map(|(d, _)| s.at + dir * d);
            let off = match &s.fuse {
                // Opens when what lies ahead is this near.
                Fuse::Obstructed(near) => physics
                    .cast_ray(s.at, dir, length + near)
                    .map(|_| s.at)
                    .or(hit),
                Fuse::Distance(far) if s.travelled + length >= *far => {
                    Some(s.at + dir * (far - s.travelled).max(0.0))
                }
                _ => hit,
            }
            .or_else(|| s.timer.filter(|&t| s.age >= t).map(|_| s.at));
            s.at += step;
            s.travelled += length;
            if let Some(at) = off {
                burst.push((s.guid, s.template, at));
                return false;
            }
            s.age < LIFETIME
        });
        self.went_off += burst.len() as u32;
        if self.first.is_none() {
            self.first = burst.first().map(|b| b.2);
        }
        self.burst.extend(burst);
    }

    /// Hands the models wanted to `flights`, which loads them.
    pub fn want_models(&mut self, flights: &mut Flights) {
        for m in self.wanted.drain(..) {
            flights.want(m);
        }
    }

    /// Draws the aircraft and ordnance whose models have loaded.
    pub fn draw(&self, queue: &mut DrawQueue, flights: &Flights) {
        for j in &self.jets {
            if let Some(m) = j.model {
                let turn = Quat::from_rotation_y(j.dir.x.atan2(j.dir.z));
                flights.draw_model(queue, m, Mat4::from_rotation_translation(turn, j.at));
            }
        }
        for s in &self.shells {
            if let Some(m) = s.model {
                let turn = Quat::from_rotation_arc(Vec3::Z, s.velocity.normalize_or(Vec3::NEG_Y));
                flights.draw_model(queue, m, Mat4::from_rotation_translation(turn, s.at));
            }
        }
    }
}

/// Turns `velocity` towards `target` by at most `turn` degrees a second,
/// if it lies within `cone` degrees of where it is heading.
fn home(velocity: &mut Vec3, at: Vec3, target: Vec3, cone: f32, turn: f32, dt: f32) {
    let speed = velocity.length();
    let (Some(now), Some(want)) = (velocity.try_normalize(), (target - at).try_normalize()) else {
        return;
    };
    let angle = now.angle_between(want);
    if angle.to_degrees() > cone {
        return;
    }
    let most = (turn * dt).to_radians();
    let t = if angle <= most { 1.0 } else { most / angle };
    *velocity = now.slerp(want, t).normalize_or(now) * speed;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn homing_turns_no_faster_than_its_rate() {
        let mut v = Vec3::Z * 10.0;
        home(&mut v, Vec3::ZERO, Vec3::X * 100.0, 120.0, 90.0, 0.5);
        let turned = Vec3::Z.angle_between(v.normalize()).to_degrees();
        assert!((turned - 45.0).abs() < 0.5, "{turned}");
        assert!((v.length() - 10.0).abs() < 1e-3);
        // Outside its cone it flies on.
        let mut v = Vec3::Z * 10.0;
        home(&mut v, Vec3::ZERO, -Vec3::Z * 100.0, 30.0, 90.0, 0.5);
        assert_eq!(v, Vec3::Z * 10.0);
    }

    #[test]
    fn a_jet_reports_its_mark_once_and_flies_on() {
        let mut s = Strikes::default();
        s.start(
            &Command::Flyby {
                guid: 7,
                what: "Support Vehicle (Mig27)".into(),
                from: [0.0, 100.0, 0.0],
                to: [0.0, 0.0, 200.0],
                speed: 100.0,
            },
            &Templates::default(),
        );
        let physics = Physics::default();
        for _ in 0..30 {
            s.step(&physics, 0.1);
        }
        assert_eq!(s.arrived, vec![(7, Vec3::new(0.0, 0.0, 200.0))]);
        assert_eq!(s.jets.len(), 1);
    }
}
