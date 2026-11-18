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

use m2_math::glam::{Quat, Vec3};
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
    /// `FlightNoise`: how it wobbles ([`FlightNoise`]).
    pub noise: Option<FlightNoise>,
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
    let noise = Table::new(db, "FlightNoise");
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
                    noise: FlightNoise::of(&noise, id),
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

/// `FlightNoise`'s last field (name unknown; 1 on every record): whether
/// the wobble is drawn afresh each sub-step (`true`) or a fixed turn
/// drawn once at launch.
const NOISE_RANDOM: u32 = 0x448d_0a0d;

/// A round's `FlightNoise`: how far it wobbles off its line.
///
/// From the 360 code: a projectile template with a `FlightNoise` record
/// (table 0x836529C0) is launched with flag 0x4000000 (0x822980C8) and a
/// wobble record (0x82298208), which its ordnance step turns it by every
/// sub-step (0x82295DF0). Field names from the 360's strings: `Pitch`,
/// `Yaw`, `Roll` (degrees; 0x82298208 multiplies them by π/180),
/// `Damping`, `CrouchModifier` (the angles' multiplier when the firer's
/// +0x414 is 1, read as crouched). PC data: AT Missile 15°, Dumb Rocket
/// 60°, RPG Rocket 20° (CrouchModifier 0.5), Vehicle AA Missile 90°,
/// Rocket 30°, Fuel Air Trail 30°, AA Missile 120°, each the same on all
/// three axes, Damping 0 to 5.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FlightNoise {
    /// `Pitch`, `Yaw`, `Roll`, radians.
    pub angles: Vec3,
    pub damping: f32,
    pub crouch: f32,
    /// [`NOISE_RANDOM`].
    pub random: bool,
}

impl FlightNoise {
    /// The `FlightNoise` record of template `id`, if it has one.
    pub(crate) fn of(table: &Table<'_>, id: u32) -> Option<Self> {
        let r = *table.of_template(id).first()?;
        let f = |n: &str| table.float(r, name_hash(n)).unwrap_or(0.0);
        Some(Self {
            angles: Vec3::new(f("Pitch"), f("Yaw"), f("Roll")) * std::f32::consts::PI / 180.0,
            damping: f("Damping"),
            crouch: table.float(r, name_hash("CrouchModifier")).unwrap_or(1.0),
            random: table.word(r, NOISE_RANDOM).is_some_and(|w| w != 0),
        })
    }
}

/// A launched round's wobble (0x82298208's record in table 0x836563DC):
/// its noise's angles, its turn so far, and the round's own orientation,
/// which the wobble turns (+Z forward; world up kept by a round that
/// falls, as 0x822879D0 faces it along its path each sub-step).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wobble {
    pub angles: Vec3,
    pub damping: f32,
    pub random: bool,
    /// Its turn so far (radians about its own x, y, z).
    pub offset: Vec3,
    pub facing: Quat,
}

/// A rotation facing +Z along `heading`, world up kept.
fn facing_along(heading: Vec3) -> Quat {
    let f = heading.normalize_or(Vec3::Z);
    let right = Vec3::Y.cross(f);
    if right.length_squared() < 1e-8 {
        return Quat::from_rotation_arc(Vec3::Z, f);
    }
    let right = right.normalize();
    Quat::from_mat3(&m2_math::glam::Mat3::from_cols(right, f.cross(right), f))
}

/// Three numbers each in [-1, 1) from `rand` (uniform in [0, 1)): the
/// 360's random vector (0x8215E160, (u - 0.5) x 2 each).
fn random_vector(rand: &mut impl FnMut() -> f32) -> Vec3 {
    let mut c = || (rand() - 0.5) * 2.0;
    Vec3::new(c(), c(), c())
}

impl Wobble {
    /// The wobble a round of `noise` starts with, heading along
    /// `heading`, from a firer `crouched` or not (0x82298208): its angles
    /// times `CrouchModifier` if crouched; one drawn afresh each sub-step
    /// starts with no turn, a fixed one with a random half of its angles.
    pub fn launch(
        noise: FlightNoise,
        heading: Vec3,
        crouched: bool,
        rand: &mut impl FnMut() -> f32,
    ) -> Self {
        let angles = if crouched {
            noise.angles * noise.crouch
        } else {
            noise.angles
        };
        let offset = if noise.random {
            Vec3::ZERO
        } else {
            random_vector(rand) * 0.5 * angles
        };
        Self {
            angles,
            damping: noise.damping,
            random: noise.random,
            offset,
            facing: facing_along(heading),
        }
    }

    /// Its turn for a sub-step of `s` seconds (0x82295DF0): a fixed one
    /// turns by its offset each sub-step; a random one draws a target of
    /// its angles times a random vector times half the sub-step, turns by
    /// that less its offset's distance from it times `Damping` times the
    /// sub-step (clamped to ±1), and adds the turn to its offset.
    fn turn(&mut self, s: f32, rand: &mut impl FnMut() -> f32) -> Vec3 {
        if !self.random {
            return self.offset;
        }
        let target = self.angles * random_vector(rand) * (s * 0.5);
        let k = (self.damping * s).clamp(-1.0, 1.0);
        let d = target - (self.offset - target) * k;
        self.offset += d;
        d
    }
}

/// [`step`], for a round that wobbles: each sub-step it first turns by
/// its wobble ([`Wobble`]; about its own x, then y, then z, 0x82854680),
/// then flies on along where it now faces (0x82295DF0).
pub fn step_wobbling(
    at: Vec3,
    velocity: Vec3,
    scale: f32,
    gravity: f32,
    dt: f32,
    wobble: &mut Wobble,
    rand: &mut impl FnMut() -> f32,
) -> (Vec3, Vec3) {
    let speed = velocity.length();
    if speed <= 0.0 {
        return (at + velocity * dt, velocity);
    }
    let mut at = at;
    let mut left = dt;
    while left > 0.0 {
        let s = left.min(SUB_STEP);
        let d = wobble.turn(s, rand);
        wobble.facing = (wobble.facing
            * Quat::from_rotation_x(d.x)
            * Quat::from_rotation_y(d.y)
            * Quat::from_rotation_z(d.z))
        .normalize();
        let heading = wobble.facing * Vec3::Z;
        if scale == 0.0 {
            at += heading * speed * s;
        } else {
            let v = heading * speed - Vec3::Y * (gravity * scale * s);
            at += v * s;
            wobble.facing = facing_along(v);
        }
        left -= s;
    }
    (at, (wobble.facing * Vec3::Z) * speed)
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

#[cfg(test)]
mod wobble_tests {
    use super::*;

    fn rng(mut seed: u64) -> impl FnMut() -> f32 {
        move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 40) as f32 / (1u64 << 24) as f32
        }
    }

    fn noise(degrees: f32, damping: f32) -> FlightNoise {
        FlightNoise {
            angles: Vec3::splat(degrees.to_radians()),
            damping,
            crouch: 0.5,
            random: true,
        }
    }

    /// With no angles a wobbling round flies as the plain step has it.
    #[test]
    fn a_still_wobble_flies_as_the_plain_step() {
        let v = Vec3::new(0.0, 5.0, 25.0);
        let mut r = rng(7);
        let mut w = Wobble::launch(noise(0.0, 5.0), v, false, &mut r);
        let (a, b) = step_wobbling(Vec3::ZERO, v, 1.0, 9.0, 0.5, &mut w, &mut r);
        let (c, d) = step(Vec3::ZERO, v, 1.0, 9.0, 0.5);
        assert!(
            (a - c).length() < 1e-3 && (b - d).length() < 1e-3,
            "{a} {c}"
        );
    }

    /// The RPG's 20 degrees: it keeps its speed and strays off its line,
    /// a little, not round; crouched, its angles are halved.
    #[test]
    fn a_wobbling_rocket_keeps_its_speed_and_strays_a_little() {
        let v = Vec3::new(0.0, 0.0, 25.0);
        let mut r = rng(11);
        let mut w = Wobble::launch(noise(20.0, 0.0), v, false, &mut r);
        let (mut at, mut vel) = (Vec3::ZERO, v);
        for _ in 0..60 {
            (at, vel) = step_wobbling(at, vel, 0.0, 9.0, 1.0 / 30.0, &mut w, &mut r);
        }
        assert!((vel.length() - 25.0).abs() < 1e-3);
        let off = at.with_z(0.0).length();
        assert!(off > 0.01 && off < 10.0, "{at}");
        assert!(at.z > 40.0, "{at}");
        let c = Wobble::launch(noise(20.0, 0.0), v, true, &mut r);
        assert!((c.angles.x - 10f32.to_radians()).abs() < 1e-6);
    }
}
