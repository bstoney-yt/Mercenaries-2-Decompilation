//! What the template database says of air strikes' ordnance and the
//! blasts it makes: how a bomb or missile flies (`ControllerVelocity`,
//! `ProjectilePhysics`, `HomingProjectile`), what it leaves when it goes
//! off (`SpawnOnDeath`) or when its timer runs out (`TriggerOnTimer` and
//! `TimerResponse`), and how far an explosion reaches and how hard it
//! hits (`Explosive`, `CameraShake`), with the explosions it sets off in
//! turn (its `PhysicalLink`s: a frag).
//!
//! The strike scripts name their ordnance (`"Cluster Bomb Projectile"`)
//! and the game reads the rest from here: a "Cluster Bomb Projectile"
//! falls at 80 m/s and leaves "Explosion (Cluster Bomb)" (5 m, 160
//! damage, at least 50 within reach, with a grenade's frag).

use m2_math::glam::Vec3;
use std::collections::HashMap;

use m2_assets::{Layer, name_hash};

use crate::tuning::Table;

/// `SpawnOnDeath`'s and `TimerResponse`'s field naming the template left
/// (name unknown), and `PhysicalLink`'s naming the template linked.
const SPAWNS: u32 = 0x7461_c0cd;
const LINKED: u32 = 0x5470_4554;
/// `HomingProjectile`'s first two fields (names unknown): the Smart Bomb's
/// 30 and 360, the AT missile's 90 and 300; read as the cone it sees its
/// target in (degrees) and how fast it turns (degrees a second).
const HOMING_CONE: u32 = 0xcefb_c415;
const HOMING_TURN: u32 = 0xa10e_79fa;

/// How a piece of ordnance flies and what it leaves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectileInfo {
    /// Its `ModelName`'s model, by name hash.
    pub model: Option<u32>,
    /// `ControllerVelocity`: its speed (m/s) and the most it reaches.
    pub speed: f32,
    pub max_speed: f32,
    /// `ProjectilePhysics`: gravity's pull on it, as a share of gravity
    /// (0 for bombs the strike scripts aim, which fly straight), and the
    /// damage of a direct hit.
    pub gravity: f32,
    pub damage: f32,
    /// `HomingProjectile`: the cone it sees its target in (degrees) and
    /// how fast it turns to it (degrees a second).
    pub homing: Option<(f32, f32)>,
    /// What it leaves on going off, in data order.
    pub leaves: Vec<u32>,
    /// `TriggerOnTimer`: it goes off this long after it is made (seconds),
    /// leaving `TimerResponse`'s template, if any.
    pub timer: Option<(f32, Option<u32>)>,
}

/// An explosion's reach and force.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlastInfo {
    /// `Explosive`: how far it reaches (metres), its damage at the heart
    /// and the least it does within reach, and its push.
    pub radius: f32,
    pub damage: f32,
    pub min_damage: f32,
    pub force: f32,
    /// `CameraShake`: amplitude and seconds.
    pub shake: Option<(f32, f32)>,
    /// The templates it sets off with it (its frag, its light and effects
    /// by `PhysicalLink`), in data order.
    pub links: Vec<u32>,
    /// `TriggerOnTimer` and `TimerResponse`: what it sets off this long
    /// after (the satellite bomb's final strike, 2 s on).
    pub timer: Option<(f32, Option<u32>)>,
}

/// Ordnance and blasts by template id.
#[derive(Clone, Debug, Default)]
pub struct Ordnance {
    pub projectiles: HashMap<u32, ProjectileInfo>,
    pub blasts: HashMap<u32, BlastInfo>,
    /// Every template with a `TriggerOnTimer`, what its timer does
    /// (thrown designators and charges among them, which are not
    /// projectiles).
    pub timers: HashMap<u32, TimerInfo>,
}

/// A template's `TriggerOnTimer` and what its `TimerResponse` does.
///
/// From the 360 code (the timer's response, 0x8227ED30): when the timer
/// runs out, a response whose first word is a template id spawns that
/// template where the object is; then the object's script hears
/// `OnTimer`; then, if the response's second word is not 0, the object
/// is queued for removal (0x822567D0 on 0x8357F9B8, the queue
/// `Object.Remove` uses). From samples: the Smoke Grenade Projectile's
/// is 0 (its smoke stays), the beacon's and C4's 1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TimerInfo {
    pub seconds: f32,
    pub spawns: Option<u32>,
    pub removes: bool,
}

/// `TimerResponse`'s second word (name unknown): the object goes once its
/// timer has run out.
const REMOVES: u32 = 0x8ae3_d3bd;

impl Ordnance {
    /// The blasts `template` makes on going off, as a projectile (what it
    /// leaves) or an explosion itself, followed by those linked to them
    /// (a frag), each once.
    pub fn blasts_of(&self, template: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut todo: Vec<u32> = match self.projectiles.get(&template) {
            Some(p) => p.leaves.clone(),
            None => vec![template],
        };
        todo.reverse();
        while let Some(t) = todo.pop() {
            if out.contains(&t) {
                continue;
            }
            if let Some(b) = self.blasts.get(&t) {
                out.push(t);
                todo.extend(b.links.iter().rev());
            }
        }
        out
    }
}

pub fn read(db: &Layer<'_>) -> Ordnance {
    let velocity = Table::new(db, "ControllerVelocity");
    let physics = Table::new(db, "ProjectilePhysics");
    let homing = Table::new(db, "HomingProjectile");
    let model = Table::new(db, "ModelName");
    let spawn = Table::new(db, "SpawnOnDeath");
    let trigger = Table::new(db, "TriggerOnTimer");
    let response = Table::new(db, "TimerResponse");
    let explosive = Table::new(db, "Explosive");
    let shake = Table::new(db, "CameraShake");
    let link = Table::new(db, "PhysicalLink");
    let nonzero = |v: Option<u32>| v.filter(|&v| v != 0);
    let timer = |id: u32| -> Option<(f32, Option<u32>)> {
        let r = trigger.of_template(id).first()?;
        let time = trigger.float(r, name_hash("time"))?;
        let left = response
            .of_template(id)
            .first()
            .and_then(|r| nonzero(response.word(r, SPAWNS)));
        Some((time, left))
    };
    let projectiles = physics
        .templates()
        .filter_map(|id| {
            let p = *physics.of_template(id).first()?;
            let v = velocity.of_template(id).first();
            let f = |r: Option<&&[u8]>, n: &str| r.and_then(|r| velocity.float(r, name_hash(n)));
            Some((
                id,
                ProjectileInfo {
                    model: model
                        .of_template(id)
                        .first()
                        .and_then(|r| nonzero(model.word(r, name_hash("model")))),
                    speed: f(v, "Velocity").unwrap_or(0.0),
                    max_speed: f(v, "MaxVelocity").unwrap_or(0.0),
                    gravity: physics.float(p, name_hash("GravityScale")).unwrap_or(1.0),
                    damage: physics.float(p, name_hash("Damage")).unwrap_or(0.0),
                    homing: homing.of_template(id).first().and_then(|r| {
                        Some((homing.float(r, HOMING_CONE)?, homing.float(r, HOMING_TURN)?))
                    }),
                    leaves: spawn
                        .of_template(id)
                        .iter()
                        .filter_map(|r| nonzero(spawn.word(r, SPAWNS)))
                        .collect(),
                    timer: timer(id),
                },
            ))
        })
        .collect();
    let blasts = explosive
        .templates()
        .filter_map(|id| {
            let e = *explosive.of_template(id).first()?;
            let f = |n: &str| explosive.float(e, name_hash(n));
            Some((
                id,
                BlastInfo {
                    radius: f("Radius")?,
                    damage: f("Damage").unwrap_or(0.0),
                    min_damage: f("MinDamage").unwrap_or(0.0),
                    force: f("MaxForce").unwrap_or(0.0),
                    shake: shake.of_template(id).first().and_then(|r| {
                        Some((
                            shake.float(r, name_hash("Amplitude"))?,
                            shake.float(r, name_hash("duration"))?,
                        ))
                    }),
                    links: link
                        .of_template(id)
                        .iter()
                        .filter_map(|r| nonzero(link.word(r, LINKED)))
                        .collect(),
                    timer: timer(id),
                },
            ))
        })
        .collect();
    let timers = trigger
        .templates()
        .filter_map(|id| {
            let (seconds, spawns) = timer(id)?;
            let removes = response
                .of_template(id)
                .first()
                .and_then(|r| response.word(r, REMOVES))
                .is_some_and(|w| w != 0);
            Some((
                id,
                TimerInfo {
                    seconds,
                    spawns,
                    removes,
                },
            ))
        })
        .collect();
    Ordnance {
        projectiles,
        blasts,
        timers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_names_hash_as_the_survey_reads_them() {
        assert_eq!(name_hash("Velocity"), name_hash("velocity"));
        assert_ne!(name_hash("Radius"), name_hash("Damage"));
    }

    #[test]
    fn blasts_follow_links_once() {
        let mut o = Ordnance::default();
        o.projectiles.insert(
            1,
            ProjectileInfo {
                leaves: vec![2],
                ..ProjectileInfo::default()
            },
        );
        let blast = |links: Vec<u32>| BlastInfo {
            radius: 5.0,
            links,
            ..BlastInfo::default()
        };
        o.blasts.insert(2, blast(vec![3, 9]));
        o.blasts.insert(3, blast(vec![2]));
        // 9 is a light or an effect: no blast.
        assert_eq!(o.blasts_of(1), vec![2, 3]);
        assert_eq!(o.blasts_of(3), vec![3, 2]);
    }
}

/// The longest sub-step the 360's ordnance step takes, seconds
/// (0x82DE8B68, read by 0x82295DF0).
pub const SUB_STEP: f32 = 0.08;

/// Flies a round `dt` seconds on, as the 360's ordnance step does
/// (0x82295DF0): in sub-steps of at most [`SUB_STEP`], its velocity is its
/// heading times its speed plus the world's gravity (`gravity`, m/s^2
/// down) times its `GravityScale` (`scale`) times the sub-step; it moves
/// by that and turns to face along it. Its speed stays as it was: gravity
/// only bends its path. With `scale` 0 it flies straight. Returns where
/// it is and its velocity (heading times speed).
pub fn step(at: Vec3, velocity: Vec3, scale: f32, gravity: f32, dt: f32) -> (Vec3, Vec3) {
    let speed = velocity.length();
    if scale == 0.0 || speed <= 0.0 {
        return (at + velocity * dt, velocity);
    }
    let mut at = at;
    let mut heading = velocity / speed;
    let mut left = dt;
    while left > 0.0 {
        let s = left.min(SUB_STEP);
        let v = heading * speed - Vec3::Y * (gravity * scale * s);
        at += v * s;
        heading = v.normalize_or(heading);
        left -= s;
    }
    (at, heading * speed)
}

/// Where a round fired at `speed` and `elevation` (radians, above level)
/// lands back at its own height, by the 360's step ([`step`]) taken
/// continuously: it keeps its speed and turns at `g cos(theta) / speed`,
/// so it comes down at minus its elevation, `2 theta v^2 / g` out, after
/// `(2 v / g) ln(sec theta + tan theta)` seconds, for `g` the world's
/// gravity times its scale. `None` where it does not fall.
pub fn landing(speed: f32, elevation: f32, g: f32) -> Option<(f32, f32)> {
    if g <= 0.0 {
        return None;
    }
    let range = 2.0 * elevation * speed * speed / g;
    let time = 2.0 * speed / g * (1.0 / elevation.cos() + elevation.tan()).ln();
    Some((range, time))
}

#[cfg(test)]
mod step_tests {
    use super::*;

    #[test]
    fn a_round_without_gravity_flies_straight() {
        let (at, v) = step(Vec3::ZERO, Vec3::new(0.0, 0.0, 30.0), 0.0, 9.0, 0.5);
        assert_eq!(
            (at, v),
            (Vec3::new(0.0, 0.0, 15.0), Vec3::new(0.0, 0.0, 30.0))
        );
    }

    /// A round keeps its speed, and lands where the closed form says.
    #[test]
    fn a_round_keeps_its_speed_and_lands_as_the_closed_form_says() {
        let (speed, elev) = (20.0f32, 30f32.to_radians());
        let mut at = Vec3::ZERO;
        let mut v = Vec3::new(0.0, elev.sin(), elev.cos()) * speed;
        let mut t = 0.0;
        let dt = 1.0 / 600.0;
        while at.y >= 0.0 || t == 0.0 {
            (at, v) = step(at, v, 1.0, 9.0, dt);
            t += dt;
        }
        assert!((v.length() - speed).abs() < 1e-3);
        let (range, time) = landing(speed, elev, 9.0).unwrap();
        assert!((at.z - range).abs() < 0.1, "{} vs {range}", at.z);
        assert!((t - time).abs() < 0.01, "{t} vs {time}");
    }

    #[test]
    fn a_long_tick_is_taken_in_sub_steps() {
        let v = Vec3::new(0.0, 10.0, 10.0);
        let (one, _) = step(Vec3::ZERO, v, 1.0, 9.0, 0.24);
        let mut at = Vec3::ZERO;
        let mut w = v;
        for _ in 0..3 {
            (at, w) = step(at, w, 1.0, 9.0, 0.08);
        }
        assert!((one - at).length() < 1e-4);
    }
}
