//! People on the island: soldiers at the posts and on the patrol routes the
//! entity layers give, and civilians wandering around the player.
//!
//! Who a spawner really spawns is not decoded, so a spawn's model is picked
//! from its entity name (`VZ Soldier ..`, `VZ Heavy ..`, `VZ Deathsquad ..`),
//! as in the browser viewer. The behaviour (standing at posts, walking
//! routes there and back with pauses, civilians strolling about) is
//! invented: the game's AI is not decoded.

use std::collections::HashSet;

use m2_engine::world::SpawnPoint;
use m2_engine::{Character, Gait, GpuModel, Physics};
use m2_math::glam::{Mat4, Vec2, Vec3};

use crate::scene::Rig;

/// Soldiers come and go within these distances of the player, metres.
const ACTIVE: f32 = 250.0;
const INACTIVE: f32 = 300.0;
/// Most soldiers about at once.
const MAX_SOLDIERS: usize = 48;
/// Civilians about the player, and how far from her they stroll.
const CIVILIANS: usize = 24;
const CIVILIAN_RANGE: (f32, f32) = (15.0, 120.0);
const WANDER: f32 = 30.0;
/// Walking speed, m/s, and the pause at each end of a route or stroll.
const WALK: f32 = 1.4;
const PAUSE: (f32, f32) = (2.0, 6.0);
/// People are drawn this close to the camera, metres.
pub const DRAW_DISTANCE: f32 = 200.0;
const TURN_RATE: f32 = 6.0;

/// A person model and the rig that animates it.
#[derive(Debug)]
pub struct PersonKind {
    pub role: String,
    pub soldier: bool,
    pub model: GpuModel,
    pub rig: Rig,
}

/// What a person is doing.
#[derive(Clone, Debug, PartialEq)]
enum Brain {
    /// Standing at a post, facing `facing`.
    Post { facing: f32 },
    /// Walking a route: heading to point `next`, `step` +1 or -1 along it
    /// (open routes are walked there and back).
    Patrol {
        route: Vec<Vec3>,
        next: usize,
        step: isize,
        wait: f32,
    },
    /// Strolling to `target`, near `home`.
    Wander { home: Vec3, target: Vec3, wait: f32 },
}

#[derive(Debug)]
pub struct Npc {
    pub kind: usize,
    pub character: Character,
    pub facing: f32,
    pub rig: Rig,
    /// False until there is ground under them to stand on.
    pub landed: bool,
    /// The layer entity that placed them (soldiers).
    pub spawn: Option<u32>,
    brain: Brain,
}

impl Npc {
    fn new(
        kind: usize,
        rig: &Rig,
        at: Vec3,
        facing: f32,
        brain: Brain,
        spawn: Option<u32>,
    ) -> Self {
        let mut rig = rig.clone();
        rig.set_gait(Gait::Idle);
        Self {
            kind,
            character: Character::new(at),
            facing,
            rig,
            landed: false,
            spawn,
            brain,
        }
    }

    pub fn patrolling(&self) -> bool {
        matches!(self.brain, Brain::Patrol { .. })
    }

    /// Model to world, turned so the model faces `facing`.
    pub fn model_matrix(&self) -> Mat4 {
        let forward = self.rig.locomotion.as_ref().map_or(Vec2::Y, |l| l.forward);
        Mat4::from_translation(self.character.position)
            * Mat4::from_rotation_y(self.facing - forward.x.atan2(forward.y))
    }

    fn update(&mut self, physics: &Physics, rng: &mut Rng, dt: f32) {
        if !self.landed {
            // Onto the ground (or the roof or floor of their post) under
            // where they were placed.
            let p = self.character.position;
            let from = Vec3::new(p.x, p.y + 2.0, p.z);
            let hit = physics
                .cast_ray(from, -Vec3::Y, 60.0)
                .map(|(d, _)| from.y - d)
                .or_else(|| {
                    physics
                        .cast_ray(Vec3::new(p.x, 3000.0, p.z), -Vec3::Y, 6000.0)
                        .map(|(d, _)| 3000.0 - d)
                });
            if let Some(y) = hit {
                self.character.position.y = y + 0.05;
                self.landed = true;
            }
            return;
        }
        let at = self.character.position;
        let mut wanted = Vec3::ZERO;
        let mut face = None;
        match &mut self.brain {
            Brain::Post { facing } => face = Some(*facing),
            Brain::Patrol {
                route,
                next,
                step,
                wait,
            } => {
                if *wait > 0.0 {
                    *wait -= dt;
                } else {
                    let to = route[*next] - at;
                    let flat = Vec3::new(to.x, 0.0, to.z);
                    if flat.length() < 0.6 {
                        let last = route.len() - 1;
                        let closed = route.len() > 2 && route[0].distance(route[last]) < 2.0;
                        let n = *next as isize + *step;
                        if n < 0 || n > last as isize {
                            if closed {
                                *next = if *step > 0 { 0 } else { last };
                            } else {
                                *step = -*step;
                                *next = (*next as isize + *step) as usize;
                                *wait = rng.range(PAUSE.0, PAUSE.1);
                            }
                        } else {
                            *next = n as usize;
                        }
                    } else {
                        wanted = flat.normalize() * WALK;
                    }
                }
            }
            Brain::Wander { home, target, wait } => {
                if *wait > 0.0 {
                    *wait -= dt;
                } else {
                    let to = *target - at;
                    let flat = Vec3::new(to.x, 0.0, to.z);
                    if flat.length() < 0.8 {
                        *wait = rng.range(PAUSE.0, PAUSE.1);
                        *target = *home + rng.disc(WANDER);
                    } else {
                        wanted = flat.normalize() * WALK;
                    }
                }
            }
        }
        physics.move_character(&mut self.character, wanted, None, dt);
        // Walked into something: give up on that point (strolls) or wait
        // and try again (routes).
        let moved = Vec2::new(self.character.velocity.x, self.character.velocity.z).length();
        if wanted != Vec3::ZERO && moved < 0.2 {
            match &mut self.brain {
                Brain::Wander { home, target, .. } => *target = *home + rng.disc(WANDER),
                Brain::Patrol { wait, .. } => *wait = 1.0,
                Brain::Post { .. } => {}
            }
        }
        if wanted.length_squared() > 0.01 {
            face = Some(wanted.x.atan2(wanted.z));
        }
        if let Some(target) = face {
            let mut turn = (target - self.facing).rem_euclid(std::f32::consts::TAU);
            if turn > std::f32::consts::PI {
                turn -= std::f32::consts::TAU;
            }
            self.facing += turn.clamp(-TURN_RATE * dt, TURN_RATE * dt);
        }
        let gait = if moved < 0.3 { Gait::Idle } else { Gait::Walk };
        if gait != self.rig.gait {
            self.rig.set_gait(gait);
        }
        let rate = match self.rig.gait_speed(gait) {
            Some(clip) if gait != Gait::Idle && clip > 0.1 => (moved / clip).clamp(0.6, 1.4),
            _ => 1.0,
        };
        self.rig.animator.advance(dt * rate);
    }
}

/// Everyone on the island but the player.
#[derive(Debug, Default)]
pub struct People {
    pub kinds: Vec<PersonKind>,
    pub npcs: Vec<Npc>,
    active: HashSet<u32>,
    rng: Rng,
}

impl People {
    /// The kind a spawn's entity name asks for: VZ soldiers by rank;
    /// spawns of vehicles, targets, buildings and other armies get none.
    pub fn kind_for(&self, name: &str) -> Option<usize> {
        let n = name.to_ascii_lowercase();
        let other = [
            "amx30", "scorpion", "hind", "m151", "boat", "target", "_bld_", "heli", "tank", "truck",
        ];
        if other.iter().any(|w| n.contains(w)) || !n.starts_with("vz ") {
            return None;
        }
        let role = if n.contains("deathsquad") || n.contains("mook") {
            "vz_hum_deathsquad"
        } else if n.contains("heavy") || n.contains("elite") {
            "vz_hum_soldierelite"
        } else {
            "vz_hum_soldier"
        };
        self.kinds.iter().position(|k| k.role == role)
    }

    /// One fixed tick: people come and go around `focus`, then move.
    pub fn update(&mut self, physics: &Physics, spawns: &[SpawnPoint], focus: Vec3, dt: f32) {
        self.populate(spawns, focus);
        for npc in &mut self.npcs {
            npc.update(physics, &mut self.rng, dt);
        }
    }

    fn populate(&mut self, spawns: &[SpawnPoint], focus: Vec3) {
        let far = |p: Vec3, d: f32| Vec2::new(p.x - focus.x, p.z - focus.z).length() > d;
        // Soldiers out of range go; civilians left behind come back round
        // the player.
        let active = &mut self.active;
        self.npcs.retain(|n| match n.spawn {
            Some(id) if far(n.character.position, INACTIVE) => {
                active.remove(&id);
                false
            }
            _ => true,
        });
        let civilian_kinds: Vec<usize> = (0..self.kinds.len())
            .filter(|&k| !self.kinds[k].soldier)
            .collect();
        for n in self.npcs.iter_mut().filter(|n| n.spawn.is_none()) {
            if far(n.character.position, INACTIVE) {
                let home = focus + self.rng.ring(CIVILIAN_RANGE.0, CIVILIAN_RANGE.1);
                n.character.position = Vec3::new(home.x, focus.y, home.z);
                n.character.velocity = Vec3::ZERO;
                n.landed = false;
                n.brain = Brain::Wander {
                    home,
                    target: home,
                    wait: 0.0,
                };
            }
        }
        // Soldiers who come into range, nearest first.
        let soldiers = self.npcs.iter().filter(|n| n.spawn.is_some()).count();
        let mut near: Vec<(f32, &SpawnPoint)> = spawns
            .iter()
            .filter(|s| !self.active.contains(&s.entity) && !far(s.position, ACTIVE))
            .map(|s| (s.position.distance(focus), s))
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, s) in near.into_iter().take(MAX_SOLDIERS.saturating_sub(soldiers)) {
            let Some(kind) = self.kind_for(&s.name) else {
                continue;
            };
            let forward = s.rotation * Vec3::Z;
            let facing = forward.x.atan2(forward.z);
            let brain = if s.route.len() >= 2 {
                Brain::Patrol {
                    route: s.route.clone(),
                    next: 0,
                    step: 1,
                    wait: 0.0,
                }
            } else {
                Brain::Post { facing }
            };
            self.active.insert(s.entity);
            self.npcs.push(Npc::new(
                kind,
                &self.kinds[kind].rig,
                s.position,
                facing,
                brain,
                Some(s.entity),
            ));
        }
        // Civilians, once their models are in.
        let civilians = self.npcs.len() - self.npcs.iter().filter(|n| n.spawn.is_some()).count();
        if !civilian_kinds.is_empty() {
            for i in civilians..CIVILIANS {
                let kind = civilian_kinds[i % civilian_kinds.len()];
                let home = focus + self.rng.ring(CIVILIAN_RANGE.0, CIVILIAN_RANGE.1);
                let at = Vec3::new(home.x, focus.y, home.z);
                let facing = self.rng.range(0.0, std::f32::consts::TAU);
                let brain = Brain::Wander {
                    home,
                    target: home,
                    wait: self.rng.range(0.0, PAUSE.1),
                };
                self.npcs.push(Npc::new(
                    kind,
                    &self.kinds[kind].rig,
                    at,
                    facing,
                    brain,
                    None,
                ));
            }
        }
    }
}

/// A small xorshift generator: where civilians go needs no more.
#[derive(Debug)]
struct Rng(u64);

impl Default for Rng {
    fn default() -> Self {
        Self(0x9e37_79b9_7f4a_7c15)
    }
}

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }

    /// A point on the ground plane between `lo` and `hi` metres away.
    fn ring(&mut self, lo: f32, hi: f32) -> Vec3 {
        let a = self.range(0.0, std::f32::consts::TAU);
        let r = self.range(lo, hi);
        Vec3::new(a.sin() * r, 0.0, a.cos() * r)
    }

    /// A point on the ground plane within `r` metres.
    fn disc(&mut self, r: f32) -> Vec3 {
        self.ring(0.0, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_engine::{Animator, CollisionShape, Skeleton, StaticKey};
    use m2_math::glam::Quat;
    use std::sync::Arc;

    fn ground() -> Physics {
        let mut p = Physics::new();
        let h = 200.0;
        let v = [
            Vec3::new(-h, 0.0, -h),
            Vec3::new(h, 0.0, -h),
            Vec3::new(h, 0.0, h),
            Vec3::new(-h, 0.0, h),
        ];
        let shape = CollisionShape::mesh(&v, &[[0, 1, 2], [0, 2, 3]]).unwrap();
        p.add_static(StaticKey::Cell(1), &shape, Vec3::ZERO, Quat::IDENTITY);
        p.step(1.0 / 60.0);
        p
    }

    fn empty_model() -> GpuModel {
        let cpu = m2_engine::CpuModel {
            hash: 0,
            pieces: Vec::new(),
            materials: Vec::new(),
            bounds: (Vec3::ZERO, Vec3::ZERO),
            skipped_groups: 0,
            collision: None,
        };
        let mut backend = m2_render::NullBackend::default();
        let mut materials = m2_engine::MaterialCache::new();
        GpuModel::upload(&mut backend, &mut materials, &cpu).unwrap()
    }

    fn rig() -> Rig {
        Rig {
            skeleton: Arc::new(Skeleton::from_nodes(&[])),
            clips: Arc::new(Vec::new()),
            locomotion: None,
            animator: Animator::default(),
            gait: Gait::Idle,
        }
    }

    #[test]
    fn patrols_walk_there_and_back() {
        let physics = ground();
        let route = vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 10.0)];
        let brain = Brain::Patrol {
            route,
            next: 0,
            step: 1,
            wait: 0.0,
        };
        let mut npc = Npc::new(0, &rig(), Vec3::new(0.0, 1.0, 0.0), 0.0, brain, Some(1));
        let mut rng = Rng::default();
        let dt = 1.0 / 60.0;
        let mut furthest: f32 = 0.0;
        for _ in 0..60 * 12 {
            npc.update(&physics, &mut rng, dt);
            furthest = furthest.max(npc.character.position.z);
        }
        assert!(npc.landed);
        assert!(furthest > 9.0, "{furthest}");
        // Back at the start (or pausing near it) by now: 10 m at 1.4 m/s
        // each way plus a pause.
        for _ in 0..60 * 14 {
            npc.update(&physics, &mut rng, dt);
        }
        assert!(npc.character.position.z < 2.0, "{}", npc.character.position);
    }

    #[test]
    fn soldiers_come_and_go_with_the_player() {
        let physics = ground();
        let mut people = People::default();
        people.kinds.push(PersonKind {
            role: "vz_hum_soldier".into(),
            soldier: true,
            model: empty_model(),
            rig: rig(),
        });
        let spawn = |entity, name: &str, x| SpawnPoint {
            entity,
            name: name.into(),
            position: Vec3::new(x, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            route: Vec::new(),
        };
        let spawns = [
            spawn(1, "VZ Soldier 01", 10.0),
            spawn(2, "VZ Soldier 02", 1000.0),
            spawn(3, "VZ Hind 01", 20.0),
        ];
        people.update(&physics, &spawns, Vec3::ZERO, 1.0 / 60.0);
        assert_eq!(
            people.npcs.iter().map(|n| n.spawn).collect::<Vec<_>>(),
            [Some(1)]
        );
        people.update(&physics, &spawns, Vec3::new(1000.0, 0.0, 0.0), 1.0 / 60.0);
        assert_eq!(
            people.npcs.iter().map(|n| n.spawn).collect::<Vec<_>>(),
            [Some(2)]
        );
    }

    #[test]
    fn rng_stays_in_range() {
        let mut r = Rng::default();
        for _ in 0..1000 {
            let x = r.range(2.0, 6.0);
            assert!((2.0..6.0).contains(&x));
            let p = r.ring(15.0, 120.0);
            let d = p.length();
            assert!((14.99..120.01).contains(&d), "{d}");
        }
    }
}
