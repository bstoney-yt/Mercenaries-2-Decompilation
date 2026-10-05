//! Fighting: the rifle, the RPG and grenades, soldiers shooting back,
//! explosions that wreck vehicles, and the player going down.
//!
//! Everyone carries the game's AK-47 in the right hand, held the way the
//! game's two-handed aim clip holds it, as in the browser viewer. The
//! weapons, clips and sounds are the game's; THE RULES ARE INVENTED, not
//! the game's: damage, fire rates, accuracy, blast radius and health are
//! guesses.

use std::sync::atomic::{AtomicBool, Ordering};

use m2_engine::{GpuModel, Physics, Skeleton, VehicleId};
use m2_math::glam::{Mat4, Quat, Vec3};

use crate::npc::People;
use crate::player::Player;
use crate::scene::{ParkedVehicle, Rig};

/// Skeleton nodes the rifle is held between.
const RIGHT_HAND: u32 = 0xd34f_7713;
const LEFT_HAND: u32 = 0xeb6f_1b2d;
/// The weapon attach points under the hands, in the palms
/// (`Bone_Attach_RHand`, `bone_attach_lhand`): where a weapon's grip goes.
const RIGHT_ATTACH: u32 = 0xf450_ed0d;
const LEFT_ATTACH: u32 = 0xfa76_1573;

/// Weapons drawn on the right hand's attach node turned as the node is
/// posed (the default: the grip sits in her palm, round 78), rather than
/// the hold worked out from the hands (`--grip hands`).
static ATTACH_FRAME: AtomicBool = AtomicBool::new(true);

pub fn use_attach_frame(on: bool) {
    ATTACH_FRAME.store(on, Ordering::Relaxed);
}
/// Muzzle marker node of the weapon models.
pub const MUZZLE: u32 = 0xb148_6ef3;

/// Rifle: seconds between shots, range, and damage per hit (three kill).
const RIFLE_GAP: f32 = 0.11;
const RIFLE_RANGE: f32 = 400.0;
const RIFLE_DAMAGE: f32 = 34.0;
/// How long she keeps the rifle up after a shot, seconds.
const AIM_AFTER_SHOT: f32 = 0.6;
/// Muzzle flashes: how long they last (a few frames) and how far they
/// reach from the muzzle, metres. Invented, to look like the game's.
const FLASH_TIME: f32 = 0.05;
const RIFLE_FLASH: f32 = 0.3;
const RPG_FLASH: f32 = 0.7;
const MG_FLASH: f32 = 0.45;
const CANNON_FLASH: f32 = 2.2;
/// Vehicle guns: a tank's cannon (seconds to reload, shell speed m/s)
/// and the machine gun on anything else with a turret (seconds between
/// rounds, damage, range). Invented, like the rest of the rules.
const CANNON_RELOAD: f32 = 2.5;
const SHELL_SPEED: f32 = 150.0;
const MG_GAP: f32 = 0.09;
const MG_DAMAGE: f32 = 45.0;
const MG_RANGE: f32 = 500.0;
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
/// Grenade: throw speed (m/s), fuse (s), bounce, and seconds before she
/// can throw (or fire) again after starting a throw.
const THROW: f32 = 12.0;
const FUSE: f32 = 2.5;
const BOUNCE: f32 = 0.4;
const THROW_GAP: f32 = RPG_RELOAD * 0.5;
/// Blasts kill within `KILL` metres and hurt to `REACH`; vehicles within
/// `WRECK` are wrecked.
const KILL: f32 = 4.0;
/// How hard a blast throws people: (out, up) m/s at its heart, on top of
/// 3 out and 2 up at the edge of its reach. Invented, to look like the
/// game's flung bodies.
const BLAST_PUSH: (f32, f32) = (9.0, 6.0);
/// How hard a rifle round shoves the one it kills, m/s.
const SHOT_PUSH: f32 = 2.0;
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
    /// The hand `grip` is relative to: the rear (trigger) hand of the aim
    /// pose.
    pub grip_hand: Option<usize>,
    /// Carried low in the right hand alone, muzzle forward and down as
    /// the game's player carries a rifle: where the grip sits in the hand
    /// (hand space), and the weapon's turn in the body's space, so it
    /// points the same way however the hand turns.
    pub carry: Option<(Vec3, Quat)>,
}

/// The weapon models, once loaded.
#[derive(Debug, Default)]
pub struct Arsenal {
    pub rifle: Option<WeaponModel>,
    pub rpg: Option<WeaponModel>,
    pub rocket: Option<WeaponModel>,
    pub grenade: Option<WeaponModel>,
    /// The right hand's node in the human skeleton, and its attach point.
    pub hand: Option<usize>,
    pub attach: Option<usize>,
}

impl Arsenal {
    /// Adds weapon `name`, gripped in `rig`'s right hand.
    pub fn add(&mut self, name: &str, model: GpuModel, muzzle: Vec3, rig: Option<&Rig>) {
        let (grip, grip_hand) = match rig.and_then(|r| grip(r, muzzle)) {
            Some((m, hand)) => (m, Some(hand)),
            None => (Mat4::IDENTITY, None),
        };
        let carry = rig.and_then(|r| carry(r, muzzle));
        if let Some(r) = rig {
            self.hand = r.rifle_hand();
            self.attach = right_attach(&r.skeleton);
        }
        let w = Some(WeaponModel {
            model,
            muzzle,
            grip,
            grip_hand,
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
    /// matrices `nodes`, its right hand at node `hand` and the hand's
    /// attach point at `attach`: held in both hands (`two_handed`: a pose
    /// made for it, aiming or at the ready), else carried in the right.
    pub fn in_hand(
        &self,
        weapon: &WeaponModel,
        body: Mat4,
        nodes: &[Mat4],
        (hand, attach): (Option<usize>, Option<usize>),
        two_handed: bool,
    ) -> Option<Mat4> {
        if ATTACH_FRAME.load(Ordering::Relaxed)
            && let Some(a) = attach.and_then(|a| nodes.get(a))
        {
            return Some(body * *a);
        }
        let right = nodes.get(hand?)?;
        match weapon.carry {
            Some((at, turn)) if !two_handed => {
                Some(body * Mat4::from_rotation_translation(turn, right.transform_point3(at)))
            }
            _ => {
                let rear = weapon.grip_hand.and_then(|h| nodes.get(h)).unwrap_or(right);
                Some(body * *rear * weapon.grip)
            }
        }
    }
}

/// The upper body of a human skeleton (from its hands), for clips laid
/// over the legs.
pub fn upper_body(skeleton: &Skeleton) -> Option<std::sync::Arc<[bool]>> {
    skeleton.upper_body(node(skeleton, RIGHT_HAND)?, node(skeleton, LEFT_HAND)?)
}

/// The right- and left-hand nodes of a human skeleton.
pub fn hands(skeleton: &Skeleton) -> Option<(usize, usize)> {
    Some((node(skeleton, RIGHT_HAND)?, node(skeleton, LEFT_HAND)?))
}

/// How far below level a weapon carried in one hand points, radians.
const CARRY_DOWN: f32 = 0.95;
/// And how far out to the side, radians.
const CARRY_OUT: f32 = 0.17;

fn node(skeleton: &Skeleton, hash: u32) -> Option<usize> {
    skeleton.name_hashes.iter().position(|&h| h == hash)
}

/// The right hand's weapon attach point.
pub fn right_attach(skeleton: &Skeleton) -> Option<usize> {
    node(skeleton, RIGHT_ATTACH)
}

/// Where in `nodes` (model space) a weapon's grip sits in `hand`: its
/// attach point in the palm, or for a skeleton without one, most of the
/// way from the wrist to the hand's first child.
fn palm(skeleton: &Skeleton, nodes: &[Mat4], hand: usize) -> Vec3 {
    let at = |n: usize| nodes[n].w_axis.truncate();
    let attach = [RIGHT_ATTACH, LEFT_ATTACH]
        .into_iter()
        .filter_map(|h| node(skeleton, h))
        .find(|&a| skeleton.parents.get(a) == Some(&Some(hand)));
    if let Some(a) = attach {
        return at(a);
    }
    match skeleton.parents.iter().position(|&p| p == Some(hand)) {
        Some(f) => at(hand).lerp(at(f), 0.6),
        None => at(hand),
    }
}

/// Where a weapon with its muzzle at `muzzle` sits in the aim clip's
/// pose: grip in the rear hand, barrel towards the front one, upright
/// about the barrel; and the rear hand's node.
fn grip(rig: &Rig, muzzle: Vec3) -> Option<(Mat4, usize)> {
    let skeleton = &rig.skeleton;
    let (rear, front) = rig.aim_hands()?;
    let aim = rig
        .clips
        .iter()
        .find(|c| c.hash == crate::npc::FightClips::AIM)?;
    let pose = aim.sample(skeleton, aim.info.duration * 0.5);
    let nodes = skeleton.model_matrices(&pose);
    let pr = nodes[rear].transform_point3(Vec3::ZERO);
    let pl = nodes[front].transform_point3(Vec3::ZERO);
    Some((
        place(nodes[rear], palm(skeleton, &nodes, rear), muzzle, pl - pr)?,
        rear,
    ))
}

/// Of a human skeleton's hands, (rear, front) in the aim clip: the
/// trigger hand and the one under the barrel. Without the clip, right
/// then left.
pub fn aim_hands(rig: &Rig) -> Option<(usize, usize)> {
    let skeleton = &rig.skeleton;
    let (rh, lh) = hands(skeleton)?;
    let Some(aim) = rig
        .clips
        .iter()
        .find(|c| c.hash == crate::npc::FightClips::AIM)
    else {
        return Some((rh, lh));
    };
    let forward = rig
        .locomotion
        .as_ref()
        .map_or(Vec3::Z, |l| Vec3::new(l.forward.x, 0.0, l.forward.y));
    let pose = aim.sample(skeleton, aim.info.duration * 0.5);
    let nodes = skeleton.model_matrices(&pose);
    let ahead = |n: usize| nodes[n].w_axis.truncate().dot(forward);
    Some(if ahead(rh) >= ahead(lh) {
        (lh, rh)
    } else {
        (rh, lh)
    })
}

/// How a weapon with its muzzle at `muzzle` is carried in the right hand
/// alone, from the idle clip's pose: the grip in the hand, and the barrel
/// forward (a little out to the hand's side) and [`CARRY_DOWN`] below
/// level, upright about the barrel, in the body's space.
fn carry(rig: &Rig, muzzle: Vec3) -> Option<(Vec3, Quat)> {
    let skeleton = &rig.skeleton;
    let idle = rig.gait_clip(m2_engine::Gait::Idle)?;
    let clip = &rig.clips[idle];
    let forward = rig.locomotion.as_ref()?.forward;
    let pose = clip.sample(skeleton, clip.info.duration * 0.5);
    let nodes = skeleton.model_matrices(&pose);
    let rh = rig.rifle_hand()?;
    let pr = nodes[rh].transform_point3(Vec3::ZERO);
    let at = nodes[rh]
        .inverse()
        .transform_point3(palm(skeleton, &nodes, rh));
    let ahead = Vec3::new(forward.x, 0.0, forward.y).normalize_or(Vec3::Z);
    // Turned a little out to the hand's side of the body, clear of the leg.
    let centre = skeleton
        .pelvis()
        .map_or(Vec3::ZERO, |p| nodes[p].w_axis.truncate());
    let side = (pr - centre).with_y(0.0);
    let out = (side - ahead * side.dot(ahead)).normalize_or_zero();
    let ahead = (ahead * CARRY_OUT.cos() + out * CARRY_OUT.sin()).normalize();
    let dir = ahead * CARRY_DOWN.cos() - Vec3::Y * CARRY_DOWN.sin();
    Some((at, turn(muzzle, dir)?))
}

/// The weapon-to-hand transform putting a weapon's grip at `at` (model
/// space) with its barrel (towards `muzzle`) along `dir`, upright about
/// the barrel, for a hand posed at `hand`.
fn place(hand: Mat4, at: Vec3, muzzle: Vec3, dir: Vec3) -> Option<Mat4> {
    Some(hand.inverse() * Mat4::from_rotation_translation(turn(muzzle, dir)?, at))
}

/// The turn putting a weapon's barrel (towards `muzzle`) along `dir`,
/// upright about the barrel.
fn turn(muzzle: Vec3, dir: Vec3) -> Option<Quat> {
    let dir = dir.normalize_or_zero();
    let barrel = muzzle.normalize_or_zero();
    if dir == Vec3::ZERO || barrel == Vec3::ZERO {
        return None;
    }
    let q = Quat::from_rotation_arc(barrel, dir);
    let flat = |v: Vec3| (v - dir * v.dot(dir)).normalize_or_zero();
    let (have, want) = (flat(q * Vec3::Y), flat(Vec3::Y));
    Some(Quat::from_axis_angle(dir, have.cross(want).dot(dir).atan2(have.dot(want))) * q)
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
    /// A muzzle flash at the muzzle, `dir` the way the barrel points,
    /// `size` its length in metres.
    Flash { at: Vec3, dir: Vec3, size: f32 },
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
    /// A rocket, or a tank's shell (no smoke trail, drawn as a streak),
    /// passing through the vehicle that fired it.
    Rocket {
        at: Vec3,
        velocity: Vec3,
        flown: f32,
        shell: bool,
        owner: Option<VehicleId>,
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
    /// Seconds until the grenade she is throwing leaves her hand.
    throw_in: Option<f32>,
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
    /// Where the last one went off.
    pub last_blast: Option<Vec3>,
    /// The last rocket: from the launcher, towards what the crosshair was on.
    pub last_launch: Option<(Vec3, Vec3)>,
    /// Blasts about to go off (something explosive caught): where, and
    /// seconds left.
    fuses: Vec<(Vec3, f32)>,
    /// Spent explosives burning: where, how big, seconds left, seconds to
    /// the next flame.
    fires: Vec<(Vec3, f32, f32, f32)>,
    seed: u64,
    /// What she has fired, for the HUD's ammo counters.
    pub fired: Fired,
    /// Hurts since [`take_hurts`](Self::take_hurts): damage and where
    /// from, for the HUD.
    hurts: Vec<(f32, Option<Vec3>)>,
}

/// Hurts kept for [`Combat::take_hurts`].
const MAX_HURTS: usize = 16;

/// Rifle shots, rockets and grenades the player has fired.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fired {
    pub rifle: u32,
    pub rockets: u32,
    pub grenades: u32,
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
            throw_in: None,
            projectiles: Vec::new(),
            effects: Vec::new(),
            sounds: Vec::new(),
            wrecks: Vec::new(),
            remarks: Vec::new(),
            hits: 0,
            blasts: 0,
            last_blast: None,
            last_launch: None,
            fuses: Vec::new(),
            fires: Vec::new(),
            seed: 0x2545_f491_4f6c_dd1d,
            fired: Fired::default(),
            hurts: Vec::new(),
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
    pub explosives: &'a mut [Explosive],
}

/// Something placed on the island that blows up: fuel tanks, barrels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Explosive {
    /// Its index among the island's placed objects.
    pub object: usize,
    /// Its middle.
    pub at: Vec3,
    /// About how far its sides are from its middle, metres.
    pub size: f32,
    pub hp: f32,
    pub gone: bool,
}

/// Models that blow up when hit, by name (invented list, from what the
/// bases hold).
/// Sizes are guesses from how they look.
const EXPLOSIVE_MODELS: [(&str, f32); 6] = [
    ("vzoutpost_fueltanks", 5.0),
    ("global_barrelb", 0.5),
    ("global_barrelc", 0.5),
    ("global_barreloil", 0.5),
    ("global_barrel", 0.5),
    ("global_barrela", 0.5),
];
/// How much rifle fire an explosive takes.
const EXPLOSIVE_HP: f32 = 100.0;
/// How near its side a blast sets one off, metres.
const IGNITE: f32 = 4.0;
/// How far past its size a round can land and still hit it, metres.
const HIT_SLACK: f32 = 0.5;
/// How long one burns before it goes, seconds.
const CHAIN_DELAY: f32 = 0.3;
/// How long what is left of one burns, seconds (wrecks burn 20 s and
/// more in the Xbox 360 game).
const BURN: f32 = 25.0;

/// The island's explosives.
pub fn explosives(index: &m2_engine::WorldIndex) -> Vec<Explosive> {
    let models: Vec<(u32, f32)> = EXPLOSIVE_MODELS
        .iter()
        .map(|&(n, size)| (m2_assets::name_hash(n), size))
        .collect();
    index
        .objects
        .iter()
        .enumerate()
        .filter_map(|(object, o)| {
            let &(_, size) = models.iter().find(|m| m.0 == o.model)?;
            Some(Explosive {
                object,
                at: o.position + Vec3::Y * size.min(1.2),
                size,
                hp: EXPLOSIVE_HP,
                gone: false,
            })
        })
        .collect()
}

impl Combat {
    /// Rockets and grenades in flight: (is a rocket, where, velocity).
    pub fn projectiles(&self) -> impl Iterator<Item = (bool, Vec3, Vec3)> + '_ {
        self.projectiles.iter().filter_map(|p| match *p {
            Projectile::Rocket { shell: true, .. } => None,
            Projectile::Rocket { at, velocity, .. } => Some((true, at, velocity)),
            Projectile::Grenade { at, velocity, .. } => Some((false, at, velocity)),
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

    /// Hurts since the last call: damage and where from.
    pub fn take_hurts(&mut self) -> Vec<(f32, Option<Vec3>)> {
        std::mem::take(&mut self.hurts)
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
        f.player.launcher = self.weapon == Weapon::Rpg;
        // A grenade leaves her hand at its moment in the throw; knocked
        // down or in a vehicle by then, she never let go.
        if let Some(t) = &mut self.throw_in {
            *t -= dt;
            if *t <= 0.0 {
                self.throw_in = None;
                if on_foot {
                    let hand = Self::right_hand(f);
                    self.throw(f, hand);
                }
            }
        }
        let throwing = self.throw_in.is_some() || f.player.throwing.is_some();
        if on_foot && !throwing {
            if trigger.fire && self.reload <= 0.0 {
                match self.weapon {
                    Weapon::Rifle => self.player_shoot(f),
                    Weapon::Rpg => self.launch(f),
                }
            }
            if trigger.throw && self.reload <= 0.0 {
                self.reload = THROW_GAP;
                // Without her throw clip, it goes at once.
                match f.player.start_throw() {
                    Some(release) => self.throw_in = Some(release),
                    None => self.throw(f, None),
                }
            }
        }
        if let Some(i) = f.player.driving
            && self.down <= 0.0
            && trigger.fire
            && self.reload <= 0.0
        {
            self.vehicle_fire(f, i);
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
        cast_shot(physics, people, from, dir, RIFLE_RANGE, None, None)
    }

    /// A weapon's muzzle in her hands, or her chest.
    fn muzzle(f: &Field<'_>, weapon: Option<&WeaponModel>) -> Vec3 {
        let chest = f.player.character.position + Vec3::Y * 1.4;
        let (Some(w), Some((_, rig))) = (weapon, &f.player.model) else {
            return chest;
        };
        let (nodes, _) = rig.posed();
        f.arsenal
            .in_hand(
                w,
                f.player.model_matrix(),
                &nodes,
                (f.arsenal.hand, f.arsenal.attach),
                rig.two_handed,
            )
            .map_or(chest, |m| m.transform_point3(w.muzzle))
    }

    fn player_shoot(&mut self, f: &mut Field<'_>) {
        self.fired.rifle += 1;
        self.reload = RIFLE_GAP;
        self.aim_left = AIM_AFTER_SHOT;
        let (eye, dir) = Self::aim_ray(f.player);
        let hit = cast_shot(f.physics, f.people, eye, dir, RIFLE_RANGE, None, None);
        let to = hit.map_or(eye + dir * RIFLE_RANGE, |h| eye + dir * h.0);
        let from = Self::muzzle(f, f.arsenal.rifle.as_ref());
        self.effect(Effect::Tracer { from, to }, 0.07);
        self.effect(
            Effect::Flash {
                at: from,
                dir: (to - from).normalize_or(dir),
                size: RIFLE_FLASH,
            },
            FLASH_TIME,
        );
        self.sounds.push(Sound::Shot(from));
        if let Some((t, None)) = hit {
            self.hit_explosives_in(f.explosives, eye + dir * t, RIFLE_DAMAGE);
        }
        if let Some((_, Some(i))) = hit {
            let soldier = f.people.npcs[i].soldier;
            if f.people.npcs[i].hurt(RIFLE_DAMAGE, dir * SHOT_PUSH) && soldier {
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
                    .in_hand(
                        w,
                        n.model_matrix(),
                        &nodes,
                        (n.hand, n.attach),
                        n.rig.two_handed,
                    )
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
        let hit = cast_shot(
            f.physics,
            f.people,
            from,
            dir,
            distance + 2.0,
            Some(i),
            None,
        );
        let end = hit.map_or(from + dir * (distance + 2.0), |h| from + dir * h.0);
        self.effect(Effect::Tracer { from, to: end }, 0.07);
        self.effect(
            Effect::Flash {
                at: from,
                dir,
                size: RIFLE_FLASH,
            },
            FLASH_TIME,
        );
        self.sounds.push(Sound::Shot(from));
        let closest = from + dir * (aim - from).dot(dir).max(0.0);
        let blocked = hit.is_some_and(|h| h.0 < distance - 1.0);
        if f.player.driving.is_none() && !blocked && closest.distance(aim) < 0.45 {
            self.hurt_player(f, NPC_DAMAGE, Some(from));
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
            explosives: &mut [],
        };
        self.npc_shoot(&mut f, 0, target);
    }

    fn hurt_player(&mut self, f: &mut Field<'_>, damage: f32, from: Option<Vec3>) {
        if self.down > 0.0 {
            return;
        }
        // Kept for the HUD; only the latest if nothing reads them.
        if self.hurts.len() >= MAX_HURTS {
            self.hurts.remove(0);
        }
        self.hurts.push((damage, from));
        self.quiet = 0.0;
        self.hp = (self.hp - damage).max(0.0);
        if self.hp <= 0.0 {
            self.remarks.push(Remark::Down);
            self.down = DOWN_TIME;
            f.player.down = true;
            f.player.aiming = false;
            if let (Some(die), Some((_, rig))) = (f.people.clips.die, &mut f.player.model) {
                rig.animator.set_upper(None);
                rig.two_handed = false;
                rig.animator.play_once(die);
            }
            f.people.stand_down();
        } else {
            self.remarks.push(Remark::Hurt);
        }
    }

    fn launch(&mut self, f: &mut Field<'_>) {
        self.fired.rockets += 1;
        self.reload = RPG_RELOAD;
        self.aim_left = AIM_AFTER_SHOT * 2.0;
        let (eye, dir) = Self::aim_ray(f.player);
        let from = Self::muzzle(f, f.arsenal.rpg.as_ref());
        // Aim the rocket from the launcher at what the crosshair is on.
        let goal = cast_shot(f.physics, f.people, eye, dir, ROCKET_RANGE, None, None)
            .map_or(eye + dir * ROCKET_RANGE, |h| eye + dir * h.0);
        let velocity = (goal - from).normalize_or(dir) * ROCKET_SPEED;
        self.last_launch = Some((from, goal));
        self.projectiles.push(Projectile::Rocket {
            at: from,
            velocity,
            flown: 0.0,
            shell: false,
            owner: None,
        });
        self.effect(
            Effect::Flash {
                at: from,
                dir: velocity.normalize_or(dir),
                size: RPG_FLASH,
            },
            FLASH_TIME * 1.6,
        );
        // The back-blast: a puff of smoke behind the launcher.
        let back = from - velocity.normalize_or(dir) * 1.4;
        self.effect(
            Effect::Smoke {
                at: back,
                size: 0.7,
            },
            1.2,
        );
        self.sounds.push(Sound::Launch(from));
        alarm(f.people, f.player.character.position);
    }

    /// Fires the gun on the turret of vehicle `i`, which she drives, at
    /// what the crosshair is on: a tank's cannon fires a shell that bursts
    /// like a rocket, anything else's a machine gun.
    fn vehicle_fire(&mut self, f: &mut Field<'_>, i: usize) {
        let Some(v) = f.vehicles.get(i) else {
            return;
        };
        let (Some(id), Some(muzzle), false) = (v.id, v.spec.muzzle, v.wrecked) else {
            return;
        };
        let Some(body) = f.physics.vehicle_transform(id) else {
            return;
        };
        let cannon = v.spec.kind == "tank";
        let from =
            body.transform_point3(v.spec.turret_transform(v.turret).transform_point3(muzzle));
        let (eye, dir) = Self::aim_ray(f.player);
        let goal = cast_shot(f.physics, f.people, eye, dir, MG_RANGE, None, Some(id))
            .map_or(eye + dir * MG_RANGE, |h| eye + dir * h.0);
        let aim = (goal - from).normalize_or(dir);
        if cannon {
            self.reload = CANNON_RELOAD;
            self.projectiles.push(Projectile::Rocket {
                at: from,
                velocity: aim * SHELL_SPEED,
                flown: 0.0,
                shell: true,
                owner: Some(id),
            });
            self.effect(
                Effect::Flash {
                    at: from,
                    dir: aim,
                    size: CANNON_FLASH,
                },
                FLASH_TIME * 2.0,
            );
            self.effect(
                Effect::Smoke {
                    at: from + aim * 1.5,
                    size: 1.5,
                },
                1.5,
            );
            self.sounds.push(Sound::Launch(from));
        } else {
            self.reload = MG_GAP;
            let hit = cast_shot(f.physics, f.people, from, aim, MG_RANGE, None, Some(id));
            let to = hit.map_or(from + aim * MG_RANGE, |h| from + aim * h.0);
            self.effect(Effect::Tracer { from, to }, 0.07);
            self.effect(
                Effect::Flash {
                    at: from,
                    dir: aim,
                    size: MG_FLASH,
                },
                FLASH_TIME,
            );
            self.sounds.push(Sound::Shot(from));
            if let Some((_, Some(j))) = hit {
                let soldier = f.people.npcs[j].soldier;
                if f.people.npcs[j].hurt(MG_DAMAGE, aim * SHOT_PUSH) && soldier {
                    self.remarks.push(Remark::Kill);
                }
                self.hits += 1;
            }
            if let Some((t, None)) = hit {
                self.hit_explosives_in(f.explosives, from + aim * t, MG_DAMAGE);
            }
        }
        alarm(f.people, from);
    }

    /// A round landing at `at` hurts the explosive it hits, which goes up
    /// (after a moment) once spent.
    fn hit_explosives_in(&mut self, explosives: &mut [Explosive], at: Vec3, damage: f32) {
        for e in explosives.iter_mut() {
            if !e.gone && e.at.distance(at) < e.size + HIT_SLACK {
                e.hp -= damage;
                if e.hp <= 0.0 {
                    e.gone = true;
                    self.fuses.push((e.at, CHAIN_DELAY));
                    self.fires.push((e.at, e.size, BURN, 0.0));
                }
            }
        }
    }

    /// Her right hand's grip in the world as she is posed now (the hand's
    /// attach point in the palm), if she has a model.
    fn right_hand(f: &Field<'_>) -> Option<Vec3> {
        let (_, rig) = f.player.model.as_ref()?;
        let node = f
            .arsenal
            .attach
            .or_else(|| right_attach(&rig.skeleton))
            .or_else(|| rig.rifle_hand())?;
        let (nodes, _) = rig.posed();
        let at = nodes.get(node)?.w_axis.truncate();
        Some(f.player.model_matrix().transform_point3(at))
    }

    /// Whether a grenade is in her hand: she is throwing and hasn't let
    /// go yet.
    pub fn grenade_in_hand(&self) -> bool {
        self.throw_in.is_some()
    }

    /// A grenade thrown towards the crosshair, from `hand` (else from
    /// in front of her head).
    fn throw(&mut self, f: &mut Field<'_>, hand: Option<Vec3>) {
        self.fired.grenades += 1;
        let (_, dir) = Self::aim_ray(f.player);
        let from = hand
            .unwrap_or_else(|| f.player.character.position + Vec3::Y * 1.6 + dir.with_y(0.0) * 0.4);
        let velocity = (dir + Vec3::Y * 0.35).normalize_or(Vec3::Y) * THROW;
        self.projectiles.push(Projectile::Grenade {
            at: from,
            velocity,
            fuse: FUSE,
        });
    }

    fn fly(&mut self, f: &mut Field<'_>, dt: f32) {
        let mut blasts = Vec::new();
        self.fuses.retain_mut(|(at, left)| {
            *left -= dt;
            if *left <= 0.0 {
                blasts.push(*at);
            }
            *left > 0.0
        });
        let mut puffs = Vec::new();
        let mut streaks = Vec::new();
        self.projectiles.retain_mut(|p| match p {
            Projectile::Rocket {
                at,
                velocity,
                flown,
                shell,
                owner,
            } => {
                let step = *velocity * dt;
                let len = step.length();
                let hit = cast_shot(f.physics, f.people, *at, step, len, None, *owner);
                if let Some((d, _)) = hit {
                    blasts.push(*at + step.normalize_or_zero() * d);
                    return false;
                }
                if *shell {
                    streaks.push((*at, *at + step));
                } else {
                    puffs.push(*at);
                }
                *at += step;
                *flown += len;
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
        for (from, to) in streaks {
            self.effect(Effect::Tracer { from, to }, dt * 1.5);
        }
        for at in blasts {
            self.explode(f, at);
        }
    }

    /// A blast at `at`: fire and smoke, people killed or hurt, vehicles
    /// near it wrecked and thrown.
    pub fn explode(&mut self, f: &mut Field<'_>, at: Vec3) {
        self.blasts += 1;
        self.last_blast = Some(at);
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
            // Thrown out and up, harder the nearer.
            let near = (1.0 - d / REACH).max(0.0);
            let away = (n.chest() - at).with_y(0.0).normalize_or(Vec3::X);
            let push = away * (BLAST_PUSH.0 * near + 3.0) + Vec3::Y * (BLAST_PUSH.1 * near + 2.0);
            if d < KILL {
                killed |= n.hurt(1000.0, push) && n.soldier;
            } else if d < REACH {
                killed |= n.hurt(100.0 * near, push) && n.soldier;
                n.panic(at);
            } else if d < REACH * 3.0 {
                n.panic(at);
            }
        }
        // Fuel tanks and barrels near it catch and go off in turn.
        for e in f.explosives.iter_mut() {
            if !e.gone && e.at.distance(at) < e.size + IGNITE {
                e.gone = true;
                self.fuses.push((e.at, CHAIN_DELAY));
                self.fires.push((e.at, e.size, BURN, 0.0));
            }
        }
        if f.player.driving.is_none() && f.player.landed {
            let d = (f.player.character.position + Vec3::Y).distance(at);
            if d < REACH {
                self.hurt_player(f, 100.0 * (1.0 - d / REACH).max(0.0), Some(at));
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
                    f.physics.wreck_vehicle(id);
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
        let mut flames = Vec::new();
        self.fires.retain_mut(|(at, size, left, next)| {
            *left -= dt;
            *next -= dt;
            if *next <= 0.0 {
                *next = 0.3;
                flames.push((*at, *size));
            }
            *left > 0.0
        });
        for (at, size) in flames {
            let low = at.with_y(at.y - size.min(1.2) + 0.5);
            self.effect(
                Effect::Fire {
                    at: low,
                    size: (size * 0.5).clamp(0.8, 2.5),
                },
                0.5,
            );
            self.effect(
                Effect::Smoke {
                    at: low + Vec3::Y * 2.0,
                    size: (size * 0.6).clamp(1.0, 3.0),
                },
                3.0,
            );
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
    own: Option<VehicleId>,
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
    let world = match own {
        Some(v) => physics.cast_ray_past(origin, dir, limit, v),
        None => physics.cast_ray(origin, dir, limit),
    };
    if let Some((t, _)) = world {
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

    /// A pose standing side-on, as the game's aim clip does: the rifle
    /// (right hand to left) points along +x while the body faces +z.
    fn side_on() -> Rig {
        let local = |x: f32, y: f32| m2_engine::anim::Local {
            translation: Vec3::new(x, y, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        };
        let skeleton = Skeleton {
            name_hashes: vec![1, 2, 3, RIGHT_HAND, LEFT_HAND],
            parents: vec![None, Some(0), Some(1), Some(2), Some(2)],
            rest: vec![
                local(0.0, 0.0),
                local(0.0, 1.0),
                local(0.0, 0.3),
                local(0.0, 0.3),
                local(0.5, 0.3),
            ],
            inverse_bind: vec![Mat4::IDENTITY; 5],
        };
        let mut r = rig();
        r.upper = Some(vec![false, false, true, true, true].into());
        r.skeleton = std::sync::Arc::new(skeleton);
        r.two_handed = true;
        r
    }

    #[test]
    fn an_aimed_rifle_points_where_she_looks() {
        let mut r = side_on();
        let barrel = |r: &Rig| {
            let (nodes, _) = r.posed();
            (nodes[4].w_axis - nodes[3].w_axis).truncate().normalize()
        };
        assert!(barrel(&r).abs_diff_eq(Vec3::X, 1e-4));
        // Ahead: the whole body turns, as she stands to aim.
        r.aim_along(0.0, Some(Vec3::Z));
        assert!(barrel(&r).abs_diff_eq(Vec3::Z, 1e-4), "{}", barrel(&r));
        // Ahead and up: the rise is made at the waist.
        let up = Vec3::new(0.0, 1.0, 1.0).normalize();
        r.aim_along(0.0, Some(up));
        assert!(barrel(&r).abs_diff_eq(up, 1e-3), "{}", barrel(&r));
        // Not aiming, nothing turns.
        r.two_handed = false;
        assert!(barrel(&r).abs_diff_eq(Vec3::X, 1e-4));
    }

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
            people.update(&mut physics, &[], Vec3::ZERO, None, dt);
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
                explosives: &mut [],
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
    fn fuel_tanks_go_off_one_after_another() {
        let mut physics = ground();
        let dt = 1.0 / 60.0;
        let mut player = Player::new(0.0, 0.0, 0.0, None);
        let mut people = People::default();
        let arsenal = Arsenal::default();
        let mut vehicles = Vec::new();
        let mut combat = Combat::default();
        let tank = |object, x| Explosive {
            object,
            at: Vec3::new(x, 1.2, 40.0),
            size: 0.5,
            hp: EXPLOSIVE_HP,
            gone: false,
        };
        // Two tanks 4 m apart, a third well clear of both.
        let mut explosives = [tank(0, 0.0), tank(1, 4.0), tank(2, 30.0)];
        let mut field = Field {
            physics: &mut physics,
            player: &mut player,
            people: &mut people,
            vehicles: &mut vehicles,
            arsenal: &arsenal,
            explosives: &mut explosives,
        };
        combat.explode(&mut field, Vec3::new(-2.0, 1.0, 40.0));
        assert!(field.explosives[0].gone && !field.explosives[1].gone);
        for _ in 0..60 {
            combat.step(&mut field, Trigger::default(), dt);
        }
        assert!(field.explosives[1].gone, "the first tank sets off the next");
        assert_eq!(combat.fires.len(), 2, "what is left of both burns");
        assert!(!field.explosives[2].gone);
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
            explosives: &mut [],
        };
        combat.explode(&mut field, at);
        assert!(combat.down > 0.0 && player.down);
        assert_eq!(combat.target(&player, true), None);
    }

    #[test]
    fn the_crosshair_finds_a_soldier_off_to_the_side() {
        let mut physics = ground();
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
            people.update(&mut physics, &[], Vec3::ZERO, None, dt);
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
