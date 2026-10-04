//! Fighting: the rifle, the RPG and grenades, soldiers shooting back,
//! explosions that wreck vehicles, and the player going down.
//!
//! Everyone carries the game's AK-47 in the right hand, held the way the
//! game's two-handed aim clip holds it, as in the browser viewer. The
//! weapons, clips and sounds are the game's; THE RULES ARE INVENTED, not
//! the game's: damage, fire rates, accuracy, blast radius and health are
//! guesses.

use m2_engine::{GpuModel, Physics, Skeleton};
use m2_math::glam::{Mat4, Quat, Vec3};

use crate::npc::People;
use crate::player::Player;
use crate::scene::{ParkedVehicle, Rig};

/// Skeleton nodes the rifle is held between.
const RIGHT_HAND: u32 = 0xeb6f_1b2d;
const LEFT_HAND: u32 = 0xd34f_7713;
/// Muzzle marker node of the weapon models.
pub const MUZZLE: u32 = 0xb148_6ef3;

/// Rifle: seconds between shots, range, and damage per hit (three kill).
const RIFLE_GAP: f32 = 0.11;
const RIFLE_RANGE: f32 = 400.0;
const RIFLE_DAMAGE: f32 = 34.0;
/// How long she keeps the rifle up after a shot, seconds.
const AIM_AFTER_SHOT: f32 = 0.6;
/// Gunfire puts soldiers this close on alert and sends civilians running.
const ALERT_RANGE: f32 = 60.0;
const PANIC_RANGE: f32 = 40.0;
/// A soldier's hit on her.
const NPC_DAMAGE: f32 = 8.0;
/// People are hit within this distance of the line from shins to head.
const BODY_RADIUS: f32 = 0.3;
/// Health comes back after this many quiet seconds, at this rate a second.
const REGEN_AFTER: f32 = 4.0;
const REGEN_RATE: f32 = 25.0;
/// Seconds she stays down at no health.
const DOWN_TIME: f32 = 2.5;
/// RPG: rocket speed (m/s), reload (s) and how far it flies.
const ROCKET_SPEED: f32 = 60.0;
const RPG_RELOAD: f32 = 1.2;
const ROCKET_RANGE: f32 = 400.0;
/// Grenade: throw speed (m/s), fuse (s), bounce.
const THROW: f32 = 12.0;
const FUSE: f32 = 2.5;
const BOUNCE: f32 = 0.4;
/// Blasts kill within `KILL` metres and hurt to `REACH`; vehicles within
/// `WRECK` are wrecked.
const KILL: f32 = 4.0;
const REACH: f32 = 9.0;
const WRECK: f32 = 6.0;

/// A hand weapon's model and where its muzzle is, model space.
#[derive(Debug)]
pub struct WeaponModel {
    pub model: GpuModel,
    pub muzzle: Vec3,
    /// Weapon to right-hand transform held in both hands (aiming, or at
    /// the ready).
    pub grip: Mat4,
    /// The same, carried low in the right hand alone: muzzle forward and
    /// down, as the game's player carries a rifle.
    pub carry: Mat4,
}

/// The weapon models, once loaded.
#[derive(Debug, Default)]
pub struct Arsenal {
    pub rifle: Option<WeaponModel>,
    pub rpg: Option<WeaponModel>,
    pub rocket: Option<WeaponModel>,
    pub grenade: Option<WeaponModel>,
    /// The hands' nodes (right, left) in the human skeleton.
    pub hands: Option<(usize, usize)>,
}

impl Arsenal {
    /// Adds weapon `name`, gripped in `rig`'s right hand.
    pub fn add(&mut self, name: &str, model: GpuModel, muzzle: Vec3, rig: Option<&Rig>) {
        let grip = rig.and_then(|r| grip(r, muzzle)).unwrap_or(Mat4::IDENTITY);
        let carry = rig.and_then(|r| carry(r, muzzle)).unwrap_or(grip);
        if let Some(r) = rig {
            self.hands = hands(&r.skeleton);
        }
        let w = Some(WeaponModel {
            model,
            muzzle,
            grip,
            carry,
        });
        match name {
            "ak47" => self.rifle = w,
            "rpg" => self.rpg = w,
            "rocket" => self.rocket = w,
            "grenade" => self.grenade = w,
            _ => {}
        }
    }

    /// Weapon to world for a character drawn at `body` with posed node
    /// matrices `nodes` and its hands at nodes `hands` (right, left): held
    /// in both when they are close together, else carried in the right.
    pub fn in_hand(
        &self,
        weapon: &WeaponModel,
        body: Mat4,
        nodes: &[Mat4],
        hands: Option<(usize, usize)>,
    ) -> Option<Mat4> {
        let (r, l) = hands?;
        let (right, left) = (nodes.get(r)?, nodes.get(l)?);
        let apart = right.w_axis.truncate().distance(left.w_axis.truncate());
        let grip = if apart < TWO_HANDED {
            weapon.grip
        } else {
            weapon.carry
        };
        Some(body * *right * grip)
    }
}

/// The upper body of a human skeleton (from its hands), for clips laid
/// over the legs.
pub fn upper_body(skeleton: &Skeleton) -> Option<std::sync::Arc<[bool]>> {
    skeleton.upper_body(node(skeleton, RIGHT_HAND)?, node(skeleton, LEFT_HAND)?)
}

/// The hand nodes of a human skeleton: (right, left).
pub fn hands(skeleton: &Skeleton) -> Option<(usize, usize)> {
    Some((node(skeleton, RIGHT_HAND)?, node(skeleton, LEFT_HAND)?))
}

/// Hands closer than this, metres, hold a weapon between them.
const TWO_HANDED: f32 = 0.5;
/// How far below level a weapon carried in one hand points, radians.
const CARRY_DOWN: f32 = 0.6;
/// And how far out to the side, radians.
const CARRY_OUT: f32 = 0.17;

fn node(skeleton: &Skeleton, hash: u32) -> Option<usize> {
    skeleton.name_hashes.iter().position(|&h| h == hash)
}

/// Where a weapon with its muzzle at `muzzle` sits in the right hand, from
/// the aim clip's pose: grip in the right hand, barrel towards the left,
/// upright about the barrel.
fn grip(rig: &Rig, muzzle: Vec3) -> Option<Mat4> {
    let skeleton = &rig.skeleton;
    let aim = rig
        .clips
        .iter()
        .find(|c| c.hash == crate::npc::FightClips::AIM)?;
    let pose = aim.sample(skeleton, aim.info.duration * 0.5);
    let nodes = skeleton.model_matrices(&pose);
    let rh = node(skeleton, RIGHT_HAND)?;
    let lh = node(skeleton, LEFT_HAND)?;
    let fingers = skeleton.parents.iter().position(|&p| p == Some(rh));
    let pr = nodes[rh].transform_point3(Vec3::ZERO);
    let pl = nodes[lh].transform_point3(Vec3::ZERO);
    let pf = fingers.map_or(pr, |f| nodes[f].transform_point3(Vec3::ZERO));
    place(nodes[rh], pr.lerp(pf, 0.6), muzzle, pl - pr)
}

/// Where a weapon with its muzzle at `muzzle` sits in the right hand when
/// carried in it alone, from the idle clip's pose: the barrel forward
/// (a little out to the side) and [`CARRY_DOWN`] below level, upright
/// about the barrel.
fn carry(rig: &Rig, muzzle: Vec3) -> Option<Mat4> {
    let skeleton = &rig.skeleton;
    let idle = rig.gait_clip(m2_engine::Gait::Idle)?;
    let clip = &rig.clips[idle];
    let forward = rig.locomotion.as_ref()?.forward;
    let pose = clip.sample(skeleton, clip.info.duration * 0.5);
    let nodes = skeleton.model_matrices(&pose);
    let rh = node(skeleton, RIGHT_HAND)?;
    let fingers = skeleton.parents.iter().position(|&p| p == Some(rh));
    let pr = nodes[rh].transform_point3(Vec3::ZERO);
    let pf = fingers.map_or(pr, |f| nodes[f].transform_point3(Vec3::ZERO));
    let ahead = Vec3::new(forward.x, 0.0, forward.y).normalize_or(Vec3::Z);
    // Turned a little out to the hand's side, clear of the leg.
    let side = Vec3::new(pr.x, 0.0, pr.z);
    let out = (side - ahead * side.dot(ahead)).normalize_or_zero();
    let ahead = (ahead * CARRY_OUT.cos() + out * CARRY_OUT.sin()).normalize();
    let dir = ahead * CARRY_DOWN.cos() - Vec3::Y * CARRY_DOWN.sin();
    place(nodes[rh], pr.lerp(pf, 0.6), muzzle, dir)
}

/// The weapon-to-hand transform putting a weapon's grip at `at` (model
/// space) with its barrel (towards `muzzle`) along `dir`, upright about
/// the barrel, for a hand posed at `hand`.
fn place(hand: Mat4, at: Vec3, muzzle: Vec3, dir: Vec3) -> Option<Mat4> {
    let dir = dir.normalize_or_zero();
    let barrel = muzzle.normalize_or_zero();
    if dir == Vec3::ZERO || barrel == Vec3::ZERO {
        return None;
    }
    let mut q = Quat::from_rotation_arc(barrel, dir);
    let flat = |v: Vec3| (v - dir * v.dot(dir)).normalize_or_zero();
    let (have, want) = (flat(q * Vec3::Y), flat(Vec3::Y));
    q = Quat::from_axis_angle(dir, have.cross(want).dot(dir).atan2(have.dot(want))) * q;
    Some(hand.inverse() * Mat4::from_rotation_translation(q, at))
}

/// Which weapon she has up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Weapon {
    #[default]
    Rifle,
    Rpg,
}

/// Something brief to draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    /// A shot's line.
    Tracer { from: Vec3, to: Vec3 },
    /// A muzzle flash.
    Flash { at: Vec3 },
    /// Smoke, growing and rising.
    Smoke { at: Vec3, size: f32 },
    /// A fireball, growing.
    Fire { at: Vec3, size: f32 },
}

/// A sound the fight makes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sound {
    Shot(Vec3),
    Launch(Vec3),
    Explosion(Vec3),
}

/// Something that happened to her worth a word from her.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remark {
    /// She killed someone.
    Kill,
    /// She wrecked a vehicle.
    Wreck,
    /// She was hit.
    Hurt,
    /// She went down.
    Down,
    /// She got into a vehicle.
    Vehicle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Projectile {
    Rocket {
        at: Vec3,
        velocity: Vec3,
        flown: f32,
    },
    Grenade {
        at: Vec3,
        velocity: Vec3,
        fuse: f32,
    },
}

/// What she asks of her weapons this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Trigger {
    /// Fire held.
    pub fire: bool,
    /// Aim held.
    pub aim: bool,
    pub throw: bool,
    pub select: Option<Weapon>,
}

/// The fight's state.
#[derive(Debug)]
pub struct Combat {
    pub hp: f32,
    pub weapon: Weapon,
    /// Seconds she stays down.
    pub down: f32,
    quiet: f32,
    reload: f32,
    aim_left: f32,
    projectiles: Vec<Projectile>,
    /// Effects with their seconds left and lifetime.
    pub effects: Vec<(Effect, f32, f32)>,
    /// Sounds made since the last call to [`take_sounds`](Self::take_sounds).
    sounds: Vec<Sound>,
    /// Wrecks smoke: (vehicle index, seconds to the next puff).
    wrecks: Vec<(usize, f32)>,
    /// What happened to her since [`take_remarks`](Self::take_remarks).
    remarks: Vec<Remark>,
    /// People shot and blasts set off, for the screenshot check.
    pub hits: u32,
    pub blasts: u32,
    seed: u64,
}

impl Default for Combat {
    fn default() -> Self {
        Self {
            hp: 100.0,
            weapon: Weapon::Rifle,
            down: 0.0,
            quiet: 0.0,
            reload: 0.0,
            aim_left: 0.0,
            projectiles: Vec::new(),
            effects: Vec::new(),
            sounds: Vec::new(),
            wrecks: Vec::new(),
            remarks: Vec::new(),
            hits: 0,
            blasts: 0,
            seed: 0x2545_f491_4f6c_dd1d,
        }
    }
}

/// The scene's parts a fight touches.
pub struct Field<'a> {
    pub physics: &'a mut Physics,
    pub player: &'a mut Player,
    pub people: &'a mut People,
    pub vehicles: &'a mut [ParkedVehicle],
    pub arsenal: &'a Arsenal,
}

impl Combat {
    /// Rockets and grenades in flight: (is a rocket, where, velocity).
    pub fn projectiles(&self) -> impl Iterator<Item = (bool, Vec3, Vec3)> + '_ {
        self.projectiles.iter().map(|p| match *p {
            Projectile::Rocket { at, velocity, .. } => (true, at, velocity),
            Projectile::Grenade { at, velocity, .. } => (false, at, velocity),
        })
    }

    /// Where blasts went off since sounds were last taken.
    pub fn explosions(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.sounds.iter().filter_map(|s| match s {
            Sound::Explosion(at) => Some(*at),
            _ => None,
        })
    }

    pub fn take_sounds(&mut self) -> Vec<Sound> {
        std::mem::take(&mut self.sounds)
    }

    pub fn take_remarks(&mut self) -> Vec<Remark> {
        std::mem::take(&mut self.remarks)
    }

    fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5
    }

    fn effect(&mut self, e: Effect, seconds: f32) {
        self.effects.push((e, seconds, seconds));
    }

    /// Where she is a target: her chest, or `None` while down, flying or
    /// not yet landed.
    pub fn target(&self, field_player: &Player, playing: bool) -> Option<Vec3> {
        (playing && field_player.landed && self.down <= 0.0)
            .then(|| field_player.character.position + Vec3::Y * 1.2)
    }

    /// One fixed tick: her weapons, the soldiers' shots, projectiles,
    /// health and effects.
    pub fn step(&mut self, f: &mut Field<'_>, trigger: Trigger, dt: f32) {
        for e in &mut self.effects {
            e.1 -= dt;
        }
        self.effects.retain(|e| e.1 > 0.0);
        self.reload -= dt;
        self.aim_left -= dt;
        self.quiet += dt;

        if self.down > 0.0 {
            self.down -= dt;
            if self.down <= 0.0 {
                self.hp = 100.0;
                f.player.down = false;
                f.player.aiming = false;
            }
        } else if self.quiet > REGEN_AFTER {
            self.hp = (self.hp + REGEN_RATE * dt).min(100.0);
        }

        let on_foot = f.player.landed && f.player.driving.is_none() && self.down <= 0.0;
        if let Some(w) = trigger.select {
            self.weapon = w;
        }
        if on_foot {
            if trigger.fire && self.reload <= 0.0 {
                match self.weapon {
                    Weapon::Rifle => self.player_shoot(f),
                    Weapon::Rpg => self.launch(f),
                }
            }
            if trigger.throw && self.reload <= 0.0 {
                self.throw(f);
            }
        }
        let holding = on_foot && (trigger.aim || self.aim_left > 0.0);
        f.player.aiming = holding;

        // Soldiers' shots.
        let target = self.target(f.player, true);
        for i in 0..f.people.npcs.len() {
            if std::mem::take(&mut f.people.npcs[i].fires) {
                self.npc_shoot(f, i, target);
            }
        }
        self.fly(f, dt);
        self.smoke_wrecks(f, dt);
    }

    /// Where shots start and the way the crosshair points: along the
    /// camera's line from where it passes her, so walls and slopes between
    /// the camera and her don't stop them.
    fn aim_ray(player: &Player) -> (Vec3, Vec3) {
        let (eye, dir) = (player.eye(), player.camera_forward());
        let past = (player.character.position - eye).dot(dir).max(0.0);
        (eye + dir * past, dir)
    }

    /// What a shot fired now would hit: the distance, and the person if
    /// one (for checking aim).
    pub fn aim_probe(
        physics: &Physics,
        people: &People,
        player: &Player,
    ) -> Option<(f32, Option<usize>)> {
        let (from, dir) = Self::aim_ray(player);
        cast_shot(physics, people, from, dir, RIFLE_RANGE, None)
    }

    /// A weapon's muzzle in her hands, or her chest.
    fn muzzle(f: &Field<'_>, weapon: Option<&WeaponModel>) -> Vec3 {
        let chest = f.player.character.position + Vec3::Y * 1.4;
        let (Some(w), Some((_, rig))) = (weapon, &f.player.model) else {
            return chest;
        };
        let (nodes, _) = rig.posed();
        f.arsenal
            .in_hand(w, f.player.model_matrix(), &nodes, f.arsenal.hands)
            .map_or(chest, |m| m.transform_point3(w.muzzle))
    }

    fn player_shoot(&mut self, f: &mut Field<'_>) {
        self.reload = RIFLE_GAP;
        self.aim_left = AIM_AFTER_SHOT;
        let (eye, dir) = Self::aim_ray(f.player);
        let hit = cast_shot(f.physics, f.people, eye, dir, RIFLE_RANGE, None);
        let to = hit.map_or(eye + dir * RIFLE_RANGE, |h| eye + dir * h.0);
        let from = Self::muzzle(f, f.arsenal.rifle.as_ref());
        self.effect(Effect::Tracer { from, to }, 0.07);
        self.effect(Effect::Flash { at: from }, 0.05);
        self.sounds.push(Sound::Shot(from));
        if let Some((_, Some(i))) = hit {
            let soldier = f.people.npcs[i].soldier;
            if f.people.npcs[i].hurt(RIFLE_DAMAGE) && soldier {
                self.remarks.push(Remark::Kill);
            }
            self.hits += 1;
        }
        alarm(f.people, f.player.character.position);
    }

    fn npc_shoot(&mut self, f: &mut Field<'_>, i: usize, target: Option<Vec3>) {
        let Some(aim) = target else { return };
        let n = &f.people.npcs[i];
        let from = match (&f.arsenal.rifle, f.people.kinds.get(n.kind)) {
            (Some(w), Some(_)) => {
                let (nodes, _) = n.rig.posed();
                f.arsenal
                    .in_hand(w, n.model_matrix(), &nodes, n.hands)
                    .map_or(n.chest(), |m| m.transform_point3(w.muzzle))
            }
            _ => n.chest(),
        };
        let distance = from.distance(aim);
        // Spread grows with range: about a third hit at 20 m.
        let spread = 0.02 + distance * 0.004;
        let dir = ((aim - from).normalize_or_zero()
            + Vec3::new(
                self.random() * spread * 2.0,
                self.random() * spread,
                self.random() * spread * 2.0,
            ))
        .normalize_or_zero();
        let hit = cast_shot(f.physics, f.people, from, dir, distance + 2.0, Some(i));
        let end = hit.map_or(from + dir * (distance + 2.0), |h| from + dir * h.0);
        self.effect(Effect::Tracer { from, to: end }, 0.07);
        self.effect(Effect::Flash { at: from }, 0.05);
        self.sounds.push(Sound::Shot(from));
        let closest = from + dir * (aim - from).dot(dir).max(0.0);
        let blocked = hit.is_some_and(|h| h.0 < distance - 1.0);
        if f.player.driving.is_none() && !blocked && closest.distance(aim) < 0.45 {
            self.hurt_player(f, NPC_DAMAGE);
        }
    }

    #[cfg(test)]
    fn npc_shoot_at_for_test(
        &mut self,
        physics: &mut Physics,
        player: &mut Player,
        people: &mut People,
        arsenal: &Arsenal,
        vehicles: &mut [ParkedVehicle],
    ) {
        let target = self.target(player, true);
        people.npcs.clear();
        people.npcs.push(crate::npc::Npc::new(
            0,
            &crate::npc::tests::rig(),
            player.character.position + Vec3::new(0.0, 0.0, 5.0),
            0.0,
            crate::npc::Brain::Post { facing: 0.0 },
            Some(1),
        ));
        let mut f = Field {
            physics,
            player,
            people,
            vehicles,
            arsenal,
        };
        self.npc_shoot(&mut f, 0, target);
    }

    fn hurt_player(&mut self, f: &mut Field<'_>, damage: f32) {
        if self.down > 0.0 {
            return;
        }
        self.quiet = 0.0;
        self.hp = (self.hp - damage).max(0.0);
        if self.hp <= 0.0 {
            self.remarks.push(Remark::Down);
            self.down = DOWN_TIME;
            f.player.down = true;
            f.player.aiming = false;
            if let (Some(die), Some((_, rig))) = (f.people.clips.die, &mut f.player.model) {
                rig.animator.set_upper(None);
                rig.animator.play_once(die);
            }
            f.people.stand_down();
        } else {
            self.remarks.push(Remark::Hurt);
        }
    }

    fn launch(&mut self, f: &mut Field<'_>) {
        self.reload = RPG_RELOAD;
        self.aim_left = AIM_AFTER_SHOT * 2.0;
        let (eye, dir) = Self::aim_ray(f.player);
        let from = Self::muzzle(f, f.arsenal.rpg.as_ref());
        // Aim the rocket from the launcher at what the crosshair is on.
        let goal = cast_shot(f.physics, f.people, eye, dir, ROCKET_RANGE, None)
            .map_or(eye + dir * ROCKET_RANGE, |h| eye + dir * h.0);
        let velocity = (goal - from).normalize_or(dir) * ROCKET_SPEED;
        self.projectiles.push(Projectile::Rocket {
            at: from,
            velocity,
            flown: 0.0,
        });
        self.effect(Effect::Flash { at: from }, 0.08);
        self.sounds.push(Sound::Launch(from));
        alarm(f.people, f.player.character.position);
    }

    fn throw(&mut self, f: &mut Field<'_>) {
        self.reload = RPG_RELOAD * 0.5;
        let (_, dir) = Self::aim_ray(f.player);
        let from = f.player.character.position + Vec3::Y * 1.6 + dir.with_y(0.0) * 0.4;
        let velocity = (dir + Vec3::Y * 0.35).normalize_or(Vec3::Y) * THROW;
        self.projectiles.push(Projectile::Grenade {
            at: from,
            velocity,
            fuse: FUSE,
        });
    }

    fn fly(&mut self, f: &mut Field<'_>, dt: f32) {
        let mut blasts = Vec::new();
        let mut puffs = Vec::new();
        self.projectiles.retain_mut(|p| match p {
            Projectile::Rocket {
                at,
                velocity,
                flown,
            } => {
                let step = *velocity * dt;
                let len = step.length();
                let hit = cast_shot(f.physics, f.people, *at, step, len, None);
                if let Some((d, _)) = hit {
                    blasts.push(*at + step.normalize_or_zero() * d);
                    return false;
                }
                *at += step;
                *flown += len;
                puffs.push(*at);
                if *flown > ROCKET_RANGE {
                    blasts.push(*at);
                    return false;
                }
                true
            }
            Projectile::Grenade { at, velocity, fuse } => {
                velocity.y -= m2_engine::physics::GRAVITY * dt;
                let step = *velocity * dt;
                if let Some((d, normal)) = f.physics.cast_ray(*at, step, step.length()) {
                    *at += step.normalize_or_zero() * (d - 0.05).max(0.0);
                    *velocity = (*velocity - normal * 2.0 * velocity.dot(normal)) * BOUNCE;
                } else {
                    *at += step;
                }
                *fuse -= dt;
                if *fuse <= 0.0 {
                    blasts.push(*at);
                    return false;
                }
                true
            }
        });
        for at in puffs {
            self.effect(Effect::Smoke { at, size: 0.4 }, 0.8);
        }
        for at in blasts {
            self.explode(f, at);
        }
    }

    /// A blast at `at`: fire and smoke, people killed or hurt, vehicles
    /// near it wrecked and thrown.
    pub fn explode(&mut self, f: &mut Field<'_>, at: Vec3) {
        self.blasts += 1;
        self.sounds.push(Sound::Explosion(at));
        self.effect(Effect::Fire { at, size: 4.0 }, 0.6);
        for k in 0..6 {
            let a = k as f32 * std::f32::consts::TAU / 6.0;
            let off = Vec3::new(a.sin(), 0.5, a.cos()) * 1.5;
            self.effect(
                Effect::Smoke {
                    at: at + off,
                    size: 2.5,
                },
                2.5,
            );
        }
        let mut killed = false;
        for n in &mut f.people.npcs {
            let d = n.chest().distance(at);
            if d < KILL {
                killed |= n.hurt(1000.0) && n.soldier;
            } else if d < REACH {
                killed |= n.hurt(100.0 * (1.0 - d / REACH)) && n.soldier;
                n.panic(at);
            } else if d < REACH * 3.0 {
                n.panic(at);
            }
        }
        if f.player.driving.is_none() && f.player.landed {
            let d = (f.player.character.position + Vec3::Y).distance(at);
            if d < REACH {
                self.hurt_player(f, 100.0 * (1.0 - d / REACH).max(0.0));
            }
        }
        for (i, v) in f.vehicles.iter_mut().enumerate() {
            let Some(id) = v.id else { continue };
            let Some(body) = f.physics.vehicle_transform(id) else {
                continue;
            };
            let (lo, hi) = v.spec.bounds;
            let centre = body.transform_point3((lo + hi) * 0.5);
            let d = centre.distance(at);
            if d < WRECK {
                let away = (centre - at).with_y(0.0).normalize_or(Vec3::X);
                let push = (away * 4.0 + Vec3::Y * 6.0) * v.spec.mass;
                // Off-centre, so it tumbles.
                f.physics
                    .push_vehicle(id, push, centre + away * -0.5 + Vec3::Y * 0.3);
                if !v.wrecked {
                    v.wrecked = true;
                    self.wrecks.push((i, 0.0));
                    self.remarks.push(Remark::Wreck);
                }
            }
        }
        if killed {
            self.remarks.push(Remark::Kill);
        }
        alarm(f.people, at);
    }

    fn smoke_wrecks(&mut self, f: &mut Field<'_>, dt: f32) {
        let mut puffs = Vec::new();
        for (i, next) in &mut self.wrecks {
            *next -= dt;
            if *next > 0.0 {
                continue;
            }
            *next = 0.4;
            let v = &f.vehicles[*i];
            if let Some(body) = v.id.and_then(|id| f.physics.vehicle_transform(id)) {
                let (lo, hi) = v.spec.bounds;
                let top = Vec3::new((lo.x + hi.x) * 0.5, hi.y, (lo.z + hi.z) * 0.5);
                puffs.push(body.transform_point3(top));
            }
        }
        for at in puffs {
            self.effect(Effect::Smoke { at, size: 1.5 }, 3.0);
        }
    }
}

/// Gunfire or a blast at `at`: soldiers close by turn on the player,
/// civilians close by run.
fn alarm(people: &mut People, at: Vec3) {
    for n in &mut people.npcs {
        if !n.alive {
            continue;
        }
        let d = n.character.position.distance(at);
        if n.soldier && d < ALERT_RANGE {
            n.alert = true;
        }
        if !n.soldier && d < PANIC_RANGE {
            n.panic(at);
        }
    }
}

/// What a shot from `origin` along `dir` hits first within `far`: the
/// distance and the person hit, if a person (living, not `skip`).
fn cast_shot(
    physics: &Physics,
    people: &People,
    origin: Vec3,
    dir: Vec3,
    far: f32,
    skip: Option<usize>,
) -> Option<(f32, Option<usize>)> {
    let dir = dir.normalize_or_zero();
    let mut best: Option<(f32, Option<usize>)> = None;
    for (i, n) in people.npcs.iter().enumerate() {
        if Some(i) == skip || !n.alive || !n.landed {
            continue;
        }
        let a = n.character.position + Vec3::Y * 0.25;
        let seg = Vec3::Y * 1.45;
        if let Some(t) = ray_capsule(origin, dir, a, seg, BODY_RADIUS)
            && t <= far
            && best.is_none_or(|b| t < b.0)
        {
            best = Some((t, Some(i)));
        }
    }
    let limit = best.map_or(far, |b| b.0);
    if let Some((t, _)) = physics.cast_ray(origin, dir, limit) {
        best = Some((t, None));
    }
    best
}

/// Distance along the ray (unit `dir`) to where it passes within `radius`
/// of the segment from `a` to `a + seg`, or `None`.
fn ray_capsule(origin: Vec3, dir: Vec3, a: Vec3, seg: Vec3, radius: f32) -> Option<f32> {
    let w0 = origin - a;
    let (b, c, d, e) = (dir.dot(seg), seg.dot(seg), dir.dot(w0), seg.dot(w0));
    let den = c - b * b;
    let u = if den > 1e-6 {
        ((e - b * d) / den).clamp(0.0, 1.0)
    } else {
        (e / c).clamp(0.0, 1.0)
    };
    let on_seg = a + seg * u;
    let s = dir.dot(on_seg - origin).max(0.0);
    ((origin + dir * s).distance(on_seg) <= radius).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::npc::tests::{ground, rig};
    use crate::npc::{Brain, Npc};

    #[test]
    fn her_rifle_drops_a_soldier_and_alerts_the_rest() {
        let mut physics = ground();
        let dt = 1.0 / 60.0;
        let mut player = Player::new(0.0, 0.0, 0.0, None);
        let mut people = People::default();
        for (x, z) in [(0.0, 12.0), (3.0, 30.0)] {
            people.npcs.push(Npc::new(
                0,
                &rig(),
                Vec3::new(x, 1.0, z),
                std::f32::consts::PI,
                Brain::Post { facing: 0.0 },
                Some(1),
            ));
        }
        for _ in 0..3 {
            people.update(&physics, &[], Vec3::ZERO, None, dt);
            player.update(&physics, Default::default(), dt);
        }
        assert!(player.landed && people.npcs.iter().all(|n| n.landed));
        // Crosshair on the near soldier's chest.
        let chest = people.npcs[0].chest();
        for _ in 0..4 {
            let to = chest - player.eye();
            player.yaw = to.x.atan2(to.z);
            player.pitch = (to.y / to.length()).asin();
        }
        let arsenal = Arsenal::default();
        let mut vehicles = Vec::new();
        let mut combat = Combat::default();
        let fire = Trigger {
            fire: true,
            ..Trigger::default()
        };
        for _ in 0..30 {
            let mut field = Field {
                physics: &mut physics,
                player: &mut player,
                people: &mut people,
                vehicles: &mut vehicles,
                arsenal: &arsenal,
            };
            combat.step(&mut field, fire, dt);
        }
        assert!(!people.npcs[0].alive);
        assert_eq!(combat.hits, 3);
        assert!(people.npcs[1].alert, "gunfire within 60 m alerts");
        assert!(
            combat
                .take_sounds()
                .iter()
                .all(|s| matches!(s, Sound::Shot(_)))
        );
    }

    #[test]
    fn soldiers_shots_wear_her_down() {
        let mut physics = ground();
        let dt = 1.0 / 60.0;
        let mut player = Player::new(0.0, 0.0, 0.0, None);
        for _ in 0..3 {
            player.update(&physics, Default::default(), dt);
        }
        let mut people = People::default();
        let mut combat = Combat::default();
        let arsenal = Arsenal::default();
        let mut vehicles = Vec::new();
        for _ in 0..13 {
            combat.npc_shoot_at_for_test(
                &mut physics,
                &mut player,
                &mut people,
                &arsenal,
                &mut vehicles,
            );
        }
        assert!(combat.hp < 100.0);
        assert!(combat.down > 0.0 || combat.hp > 0.0);
        // A blast at her feet puts her down.
        let at = player.character.position;
        let mut field = Field {
            physics: &mut physics,
            player: &mut player,
            people: &mut people,
            vehicles: &mut vehicles,
            arsenal: &arsenal,
        };
        combat.explode(&mut field, at);
        assert!(combat.down > 0.0 && player.down);
        assert_eq!(combat.target(&player, true), None);
    }

    #[test]
    fn the_crosshair_finds_a_soldier_off_to_the_side() {
        let physics = ground();
        let dt = 1.0 / 60.0;
        let mut player = Player::new(0.0, 0.0, 0.0, None);
        let mut people = People::default();
        people.npcs.push(Npc::new(
            0,
            &rig(),
            Vec3::new(17.0, 1.0, 18.0),
            0.0,
            Brain::Post { facing: 0.0 },
            Some(1),
        ));
        for _ in 0..30 {
            people.update(&physics, &[], Vec3::ZERO, None, dt);
            player.update(&physics, Default::default(), dt);
        }
        let chest = people.npcs[0].chest();
        for _ in 0..4 {
            let to = chest - player.eye();
            player.yaw = to.x.atan2(to.z);
            player.pitch = (to.y / to.length()).asin();
        }
        let hit = Combat::aim_probe(&physics, &people, &player);
        assert!(
            matches!(hit, Some((_, Some(0)))),
            "{hit:?} eye {} chest {chest}",
            player.eye()
        );
    }

    #[test]
    fn rays_hit_capsules_they_pass_close_to() {
        let a = Vec3::new(0.0, 0.25, 10.0);
        let seg = Vec3::Y * 1.45;
        // Straight at the chest.
        let t = ray_capsule(Vec3::new(0.0, 1.2, 0.0), Vec3::Z, a, seg, 0.3).unwrap();
        assert!((t - 10.0).abs() < 0.01, "{t}");
        // A metre wide.
        assert!(ray_capsule(Vec3::new(1.0, 1.2, 0.0), Vec3::Z, a, seg, 0.3).is_none());
        // Over the head.
        assert!(ray_capsule(Vec3::new(0.0, 2.5, 0.0), Vec3::Z, a, seg, 0.3).is_none());
        // Behind the shooter.
        assert!(ray_capsule(Vec3::new(0.0, 1.2, 20.0), Vec3::Z, a, seg, 0.3).is_none());
    }
}
