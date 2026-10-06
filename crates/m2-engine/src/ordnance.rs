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
}

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
    Ordnance {
        projectiles,
        blasts,
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
