//! Fighting: the rifle, the RPG and grenades, soldiers shooting back,
//! explosions that wreck vehicles, and the player going down.
//!
//! Everyone carries the game's AK-47 in the right hand, held the way the
//! game's two-handed aim clip holds it, as in the browser viewer. The
//! weapons, clips and sounds are the game's; THE RULES ARE INVENTED, not
//! the game's: damage, fire rates, accuracy, blast radius and health are
//! guesses.

use std::sync::atomic::{AtomicBool, Ordering};

use m2_engine::{GpuModel, Physics, Skeleton, StaticKey, VehicleId};
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
/// The attach points high on her back, right and left
/// (`Bone_Attach_BackRight`, `Bone_Attach_BackLeft`): where the weapons
/// she carries but doesn't hold hang.
const BACK_ATTACH: [u32; 2] = [0x3846_cb35, 0xdf2d_0826];
/// Her lower spine and her neck (`Bone_Spine1`, `Bone_Neck`): which way
/// her back runs as she is posed. (The attach points, behind her spine,
/// made a back leaning back 26 degrees, round 98: the weapons hung down
/// and forward, by her side.)
const SPINE: u32 = 0x5ca4_9bd8;
const NECK: u32 = 0xc187_105b;
/// A slung weapon hangs down her back, flat against it, as the PC video's
/// carbine does (round 97: nearly straight down, a little right of her
/// spine, from her shoulders to her hips): along its length, its middle
/// this far (metres) below the attach point and this far behind it,
/// leaning off straight down by this much (the tangent; its lower end
/// toward her left hip). Round 99 hung them along the line from the grip
/// to the muzzle, which runs well off the RPG's and the launcher's
/// length: they lay 25 to 30 degrees across her back, down to her shin
/// and thigh. Invented.
const SLUNG_DROP: f32 = 0.12;
const SLUNG_GAP: f32 = 0.07;
const SLUNG_LEAN: f32 = 0.14;
/// And its top no higher than this far above the attach point (about the
/// base of her neck), lowered as far as that takes, unless that would
/// take its bottom lower than this far below the point (just past her
/// knees): round 100 had the RPG's top 0.3 m over her head and the
/// launcher's stock in her hair, where the PC video has them end about
/// her shoulders.
const SLUNG_TOP: f32 = 0.16;
const SLUNG_FLOOR: f32 = 1.0;

/// Weapons drawn on the right hand's attach node turned as the node is
/// posed (the default: the grip sits in her palm, round 78), rather than
/// the hold worked out from the hands (`--grip hands`).
static ATTACH_FRAME: AtomicBool = AtomicBool::new(true);

pub fn use_attach_frame(on: bool) {
    ATTACH_FRAME.store(on, Ordering::Relaxed);
}
/// Muzzle marker node of the weapon models.
pub const MUZZLE: u32 = 0xb148_6ef3;

/// The grenade launcher (its template, `Grenade Launcher`): 75 rounds a
/// minute from a clip of 6; its round (`GL Projectile`) flies at 20 m/s
/// under the whole of gravity and bursts where it lands (`SpawnOnDeath`
/// an `Explosion (Grenade)`). A clip takes this long to load, seconds
/// (invented), and a round that never lands bursts this far out, metres.
pub const LAUNCHER_CLIP: u32 = 6;
const LOB_GAP: f32 = 60.0 / 75.0;
const LOB_SPEED: f32 = 20.0;
const LAUNCHER_RELOAD: f32 = 2.5;
const LOB_RANGE: f32 = 250.0;
const LOB_FLASH: f32 = 0.45;

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
/// A tank's main gun: a plume and a fireball about as wide as its turret,
/// burning as long as its template's muzzle flash (0.3 s for the AMX-30's,
/// as in the Xbox 360 playthrough) or this long.
const CANNON_FLASH: f32 = 5.0;
const CANNON_FIREBALL: f32 = 1.6;
const CANNON_FLASH_TIME: f32 = 0.3;
/// Vehicle guns: a tank's cannon (seconds to reload, shell speed m/s,
/// where its template gives no rate of fire or ammunition) and the
/// machine gun on anything else with a turret (seconds between rounds,
/// damage, range). Invented, like the rest of the rules.
const CANNON_RELOAD: f32 = 2.5;
/// A turret's gun fires at the crosshair when it points within this of
/// it, radians; else along its barrel.
const LAID: f32 = 0.06;
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
/// A melee bash: how far ahead of her it reaches (from her chest, or her
/// knees for a low one at something on the ground), how wide (the cosine
/// of its half-angle), how much it hurts a person and what of the world's
/// it meets (a supply crate's 100 health: one blow opens it), how hard it
/// shoves (out, up; m/s), and seconds before she can bash or fire again.
/// Invented, from the playthroughs.
const BASH_REACH: f32 = 1.8;
const BASH_ARC: f32 = 0.5;
const BASH_DAMAGE: f32 = 60.0;
const BASH_BREAK: f32 = 100.0;
const BASH_PUSH: (f32, f32) = (3.0, 1.0);
const BASH_GAP: f32 = 0.8;
/// Heights of a blow over her feet, high and low, metres.
const BASH_HIGH: f32 = 1.2;
const BASH_LOW: f32 = 0.45;
/// Blasts kill within `KILL` metres and hurt to `REACH`; vehicles within
/// `WRECK` are wrecked.
const KILL: f32 = 4.0;
/// How hard a blast throws people: (out, up) m/s at its heart, on top of
/// 3 out and 2 up at the edge of its reach. Invented, to look like the
/// game's flung bodies.
pub(crate) const BLAST_PUSH: (f32, f32) = (9.0, 6.0);
/// How hard a rifle round shoves the one it kills, m/s.
const SHOT_PUSH: f32 = 2.0;
pub(crate) const REACH: f32 = 9.0;
const WRECK: f32 = 6.0;
/// The satellite-guided bomb's blasts, from the template database's
/// laser-guided bomb ("Explosion (Airstike Bomb)" 80008e3f, its frag
/// "Explosion (Airstrike Frag)" 8000a3df, and its final strike 8000af7d
/// 2 s on): reach (metres) and damage at the heart, falling off to
/// nothing at the edge (the falloff is invented). The frag is stopped by
/// what stands between: the radio's word is to get behind cover.
pub const BOMB: (f32, f32) = (20.0, 100.0);
pub const BOMB_FRAG: (f32, f32) = (50.0, 300.0);
const BOMB_FINAL: (f32, f32) = (2.0, 500.0);
/// A vehicle a blast hits this hard is wrecked.
const WRECKING: f32 = 60.0;
/// What the bomb leaves burning: this many fires within this far
/// (metres), each for between these (seconds; the videos: 4 to 6 s).
const BOMB_FIRES: usize = 5;
const BOMB_FIRE_SPREAD: f32 = 9.0;
const BOMB_FIRE_TIME: (f32, f32) = (4.0, 6.0);

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
    /// The grenade launcher and its 40 mm round.
    pub launcher: Option<WeaponModel>,
    pub round: Option<WeaponModel>,
    /// Her own rifle, the carbine (soldiers carry the AK47), and the
    /// satellite designator.
    pub carbine: Option<WeaponModel>,
    pub designator: Option<WeaponModel>,
    /// The right hand's node in the human skeleton, and its attach point.
    pub hand: Option<usize>,
    pub attach: Option<usize>,
    /// The attach points on her back, right and left ([`BACK_ATTACH`]),
    /// and her lower spine and neck ([`SPINE`], [`NECK`]).
    pub back: [Option<usize>; 2],
    pub spine: Option<usize>,
    pub neck: Option<usize>,
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
            self.back = BACK_ATTACH.map(|h| node(&r.skeleton, h));
            self.spine = node(&r.skeleton, SPINE);
            self.neck = node(&r.skeleton, NECK);
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
            "mgl" => self.launcher = w,
            "40mm" => self.round = w,
            "m4" => self.carbine = w,
            "detonator" => self.designator = w,
            _ => {}
        }
    }

    /// Weapon to world for `weapon` slung on the back of a character
    /// drawn at `body` with posed node matrices `nodes`, from the attach
    /// point of `slot` (0 her right, 1 her left): down her back as her
    /// spine runs from its lower end up to her neck, whichever way she
    /// leans ([`slung_at`]).
    pub fn on_back(
        &self,
        weapon: &WeaponModel,
        body: Mat4,
        nodes: &[Mat4],
        slot: usize,
    ) -> Option<Mat4> {
        let (point, back) = self.back_at(body, nodes, slot)?;
        slung_at(weapon.muzzle, weapon.model.bounds, point, back, slot)
    }

    /// The attach point of `slot` on the back of a character drawn at
    /// `body` with posed node matrices `nodes`, and which way her back
    /// runs there, `(right, up)` (world space, not unit).
    fn back_at(&self, body: Mat4, nodes: &[Mat4], slot: usize) -> Option<(Vec3, (Vec3, Vec3))> {
        let at = |n: Option<usize>| Some(body.transform_point3(nodes.get(n?)?.w_axis.truncate()));
        let point = at(self.back.get(slot).copied().flatten())?;
        let [right_point, left_point] = self.back.map(at);
        let right = match (right_point, left_point) {
            (Some(r), Some(l)) if r.distance(l) > 0.02 => r - l,
            _ => body.transform_vector3(Vec3::NEG_X),
        };
        let up = match (at(self.spine), at(self.neck)) {
            (Some(s), Some(n)) if s.distance(n) > 0.05 => n - s,
            _ => body.transform_vector3(Vec3::Y),
        };
        Some((point, (right, up)))
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

/// Weapon to world for a weapon slung at `point` on a back running
/// `(right, up)` (world space, as posed), on her right (`slot` 0) or left:
/// along its length (the longest side of its `bounds`, the end its
/// `muzzle` is at down) down her back, leaning as [`SLUNG_LEAN`] has it,
/// flat against her back with its top to her side, its middle
/// [`SLUNG_DROP`] below the point and [`SLUNG_GAP`] behind it, or lower to
/// keep its top down ([`SLUNG_TOP`]).
fn slung_at(
    muzzle: Vec3,
    bounds: (Vec3, Vec3),
    point: Vec3,
    (right, up): (Vec3, Vec3),
    slot: usize,
) -> Option<Mat4> {
    let right = right.try_normalize()?;
    let up = (up - right * up.dot(right)).try_normalize()?;
    let ahead = up.cross(right);
    // The weapon's own frame: along its length toward the muzzle's end,
    // its top, and the side they leave.
    let (lo, hi) = bounds;
    let (size, middle) = (hi - lo, (lo + hi) * 0.5);
    let long = (0..3).max_by(|&a, &b| size[a].total_cmp(&size[b]))?;
    let mut barrel = Vec3::ZERO;
    barrel[long] = if muzzle[long] < middle[long] {
        -1.0
    } else {
        1.0
    };
    let top = (Vec3::Y - barrel * barrel.y)
        .try_normalize()
        .unwrap_or(Vec3::Z);
    let own = m2_math::glam::Mat3::from_cols(barrel, top, barrel.cross(top));
    // Where they go on her back.
    let down = (-up - right * SLUNG_LEAN).normalize();
    let side = if slot == 0 { right } else { -right };
    let to_side = (side - down * side.dot(down)).normalize();
    let turn = m2_math::glam::Mat3::from_cols(down, to_side, down.cross(to_side)) * own.transpose();
    // How far it reaches up and down her back from its middle.
    let reach = (0..3)
        .map(|k| (turn.col(k).dot(up) * size[k]).abs() * 0.5)
        .sum::<f32>();
    let lower = (reach - SLUNG_DROP - SLUNG_TOP)
        .min(SLUNG_FLOOR - SLUNG_DROP - reach)
        .max(0.0);
    let origin = point - up * (SLUNG_DROP + lower) - ahead * SLUNG_GAP - turn * middle;
    Some(Mat4::from_cols(
        turn.x_axis.extend(0.0),
        turn.y_axis.extend(0.0),
        turn.z_axis.extend(0.0),
        origin.extend(1.0),
    ))
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
    /// The first contract's supply crate's: taken up in the RPG's place.
    GrenadeLauncher,
}

impl Weapon {
    /// Rounds a clip holds: the HUD's 30 for the rifle, one rocket, and
    /// the grenade launcher's template's 6.
    pub fn clip(self) -> u32 {
        match self {
            Weapon::Rifle => 30,
            Weapon::Rpg => 1,
            Weapon::GrenadeLauncher => LAUNCHER_CLIP,
        }
    }

    /// Its model's asset name (its template's `ModelName`).
    pub fn model_name(self) -> &'static str {
        match self {
            Weapon::Rifle => "global_weapon_ak47",
            Weapon::Rpg => "global_weapon_rpg",
            Weapon::GrenadeLauncher => "global_weapon_mgl",
        }
    }

    /// The weapons of hers whose models are known by name.
    pub const ALL: [Weapon; 3] = [Weapon::Rifle, Weapon::Rpg, Weapon::GrenadeLauncher];
}

/// The rounds of a weapon that runs out: in its clip, and stored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rounds {
    pub clip: u32,
    pub stored: u32,
}

impl Rounds {
    /// `n` rounds: as many as a clip of `size` holds loaded, the rest
    /// stored.
    pub fn of(n: u32, size: u32) -> Self {
        let clip = n.min(size);
        Self {
            clip,
            stored: n - clip,
        }
    }

    pub fn total(self) -> u32 {
        self.clip + self.stored
    }
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
    /// Thick black smoke, growing and rising further than [`Smoke`](Self::Smoke).
    Soot { at: Vec3, size: f32 },
    /// Spray thrown off a boat's hull: a white puff flying `drift` metres
    /// over its life, growing and then gone.
    Spray { at: Vec3, drift: Vec3, size: f32 },
    /// Churned water behind a boat: a flat white patch on the surface,
    /// `size` across, spreading.
    Wake { at: Vec3, size: f32 },
}

/// A sound the fight makes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sound {
    Shot(Vec3),
    Launch(Vec3),
    /// The grenade launcher's.
    Lob(Vec3),
    Explosion(Vec3),
    /// A tank's main gun.
    Cannon(Vec3),
    /// The satellite-guided bomb landing.
    Bomb(Vec3),
    /// A building the bomb brought down coming apart.
    Collapse(Vec3),
}

/// A blow or a round that landed on something of the world's rather than
/// a person: where, how hard, and the static it hit if one of the
/// scene's (a supply crate's box), which takes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strike {
    pub at: Vec3,
    pub damage: f32,
    pub key: Option<StaticKey>,
}

/// Rounds she has picked up for her rifle and RPG: the play keeps no
/// ammunition of their own, so these only add to what the HUD counts (the
/// grenade launcher's are kept, in [`Combat::launcher_rounds`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Picked {
    pub rifle: u32,
    pub rockets: u32,
}

impl Picked {
    pub fn add(&mut self, weapon: Weapon, rounds: u32) {
        match weapon {
            Weapon::Rifle => self.rifle += rounds,
            Weapon::Rpg => self.rockets += rounds,
            Weapon::GrenadeLauncher => {}
        }
    }
}

/// What flies, to draw: a rocket, a thrown grenade or a launched round.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flying {
    Rocket,
    Grenade,
    Round,
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
    /// A grenade launcher's round: bursts where it lands.
    Round {
        at: Vec3,
        velocity: Vec3,
        flown: f32,
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
    /// A melee bash.
    pub melee: bool,
    pub select: Option<Weapon>,
    /// The support key: the support menu opens or closes
    /// ([`crate::support`]).
    pub support: bool,
    /// The PDA key: the PDA opens or closes.
    pub pda: bool,
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
    /// Seconds until the blow she is bashing with lands, and whether low.
    bash_in: Option<(f32, bool)>,
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
    /// Where blasts went off in the last step.
    pub went_off: Vec<Vec3>,
    /// Blows and rounds that struck the world in the last step.
    pub struck: Vec<Strike>,
    /// Rounds she has picked up, for the HUD's counters.
    pub picked: Picked,
    /// What she carries in the launcher's place, which the launcher key
    /// takes up: the RPG in free play; nothing at the first contract's
    /// start, where she lands with the carbine alone (the footage: nothing
    /// on her back, only "Carbine" in the weapon box) until the supply
    /// crate's grenade launcher.
    pub second: Option<Weapon>,
    /// The grenade launcher's rounds, while she carries one.
    pub launcher_rounds: Option<Rounds>,
    /// She holds the satellite designator instead of her weapon
    /// ([`crate::support`]).
    pub designator: bool,
    /// The last rocket: from the launcher, towards what the crosshair was on.
    pub last_launch: Option<(Vec3, Vec3)>,
    /// Blasts about to go off (something explosive caught): where, and
    /// seconds left.
    fuses: Vec<(Vec3, f32)>,
    /// What a building's break-up left burning: where, seconds left,
    /// seconds to the next flame.
    blazes: Vec<(Vec3, f32, f32)>,
    /// Spent explosives burning: where, how big, seconds left, seconds to
    /// the next flame.
    fires: Vec<(Vec3, f32, f32, f32)>,
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
            throw_in: None,
            bash_in: None,
            projectiles: Vec::new(),
            effects: Vec::new(),
            sounds: Vec::new(),
            wrecks: Vec::new(),
            remarks: Vec::new(),
            hits: 0,
            blasts: 0,
            last_blast: None,
            went_off: Vec::new(),
            struck: Vec::new(),
            picked: Picked::default(),
            second: Some(Weapon::Rpg),
            launcher_rounds: None,
            designator: false,
            last_launch: None,
            fuses: Vec::new(),
            blazes: Vec::new(),
            fires: Vec::new(),
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
/// How long a building's break-up burns, seconds (invented: the videos'
/// fires burn on past the 25 s they show).
const BLAZE_TIME: f32 = 40.0;
/// How far above the ground its flames' middle is, metres.
const BLAZE_LIFT: f32 = 1.2;

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
    /// Rockets, grenades and rounds in flight: what, where, velocity.
    pub fn projectiles(&self) -> impl Iterator<Item = (Flying, Vec3, Vec3)> + '_ {
        self.projectiles.iter().filter_map(|p| match *p {
            Projectile::Rocket { shell: true, .. } => None,
            Projectile::Rocket { at, velocity, .. } => Some((Flying::Rocket, at, velocity)),
            Projectile::Grenade { at, velocity, .. } => Some((Flying::Grenade, at, velocity)),
            Projectile::Round { at, velocity, .. } => Some((Flying::Round, at, velocity)),
        })
    }

    /// Takes up `weapon` (a launcher: the RPG or a grenade launcher) in
    /// the launcher's place, `rounds` with it, and has it up: what she
    /// carried there, if anything, she leaves, with its rounds (the
    /// RPG's are endless: none).
    pub fn equip(&mut self, weapon: Weapon, rounds: u32) -> Option<(Weapon, u32)> {
        let left = self.second.map(|w| match w {
            Weapon::GrenadeLauncher => (w, self.launcher_rounds.take().map_or(0, Rounds::total)),
            w => (w, 0),
        });
        if weapon == Weapon::GrenadeLauncher {
            self.launcher_rounds = Some(Rounds::of(rounds, LAUNCHER_CLIP));
        }
        self.second = Some(weapon);
        self.weapon = weapon;
        left
    }

    /// Lands her with the carbine alone, as the first contract starts.
    pub fn carbine_only(&mut self) {
        self.second = None;
        self.launcher_rounds = None;
        self.weapon = Weapon::Rifle;
    }

    /// Rounds taken from a pickup for `weapon`: the grenade launcher's go
    /// with the one she carries (none, they are lost); the rest only add
    /// to what the HUD counts.
    pub fn take_ammo(&mut self, weapon: Weapon, rounds: u32) {
        match (weapon, &mut self.launcher_rounds) {
            (Weapon::GrenadeLauncher, Some(r)) => r.stored += rounds,
            (Weapon::GrenadeLauncher, None) => {}
            _ => self.picked.add(weapon, rounds),
        }
    }

    /// The weapons she carries but doesn't hold, which hang on her back:
    /// the rifle on her right unless it is up in her hands, and what she
    /// carries in the launcher's place on her left unless that is (with
    /// `in_hand` false, her hands hold something else: a grenade, the
    /// designator); the one alone on her right, a little right of her
    /// spine, as the PC video's hangs (round 98).
    pub fn slung(&self, in_hand: bool) -> [Option<Weapon>; 2] {
        let up = |w: Weapon| in_hand && self.weapon == w;
        match [
            Some(Weapon::Rifle).filter(|&w| !up(w)),
            self.second.filter(|&w| !up(w)),
        ] {
            [None, alone] => [alone, None],
            both => both,
        }
    }

    /// Whether the weapon in the launcher's place can fire: the RPG
    /// always, a grenade launcher with rounds left.
    pub fn second_loaded(&self) -> bool {
        match self.second {
            Some(Weapon::GrenadeLauncher) => self.launcher_rounds.is_some_and(|r| r.total() > 0),
            Some(_) => true,
            None => false,
        }
    }

    /// Tank shells in flight (drawn as streaks, not models).
    pub fn shells(&self) -> usize {
        self.projectiles
            .iter()
            .filter(|p| matches!(p, Projectile::Rocket { shell: true, .. }))
            .count()
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

    /// Shows `e` for `seconds`, for what isn't the fight's (a boat's
    /// spray).
    pub fn show(&mut self, e: Effect, seconds: f32) {
        self.effect(e, seconds);
    }

    /// A fire burning on the ground under `at` for [`BLAZE_TIME`] with
    /// thick black smoke, where a building broke apart (a gate a bomb
    /// brought down: the videos' fires either side, smoke for many
    /// seconds; its points are high on it, and it goes down into the
    /// ground as it breaks).
    pub fn blaze(&mut self, physics: &Physics, at: Vec3) {
        let at = ground_under(physics, at).map_or(at, |g| g + Vec3::Y * BLAZE_LIFT);
        self.blazes.push((at, BLAZE_TIME, 0.0));
    }

    /// A building the bomb brought down comes apart at `at`: heard.
    pub fn collapse(&mut self, at: Vec3) {
        self.sounds.push(Sound::Collapse(at));
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
        self.went_off.clear();
        self.struck.clear();
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
        // Asked for a launcher: whichever she carries in its place.
        match (trigger.select, self.second) {
            (Some(Weapon::Rifle), _) => self.weapon = Weapon::Rifle,
            (Some(_), Some(w)) => self.weapon = w,
            _ => {}
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
        // A bash's blow lands at its moment in the clip.
        if let Some((t, low)) = &mut self.bash_in {
            *t -= dt;
            if *t <= 0.0 {
                let low = *low;
                self.bash_in = None;
                if on_foot {
                    self.bash(f, low);
                }
            }
        }
        let throwing = self.throw_in.is_some() || f.player.throwing.is_some();
        let bashing = self.bash_in.is_some() || f.player.bashing.is_some();
        if on_foot && !throwing && !bashing {
            if trigger.fire && self.reload <= 0.0 {
                match self.weapon {
                    Weapon::Rifle => self.player_shoot(f),
                    Weapon::Rpg => self.launch(f),
                    Weapon::GrenadeLauncher => self.lob(f),
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
            if trigger.melee && self.reload <= 0.0 && f.player.can_bash() {
                self.reload = BASH_GAP;
                let low = Self::low_blow(f);
                // Without her bash clip, it lands at once.
                match f.player.start_bash(low) {
                    Some(hit) => self.bash_in = Some((hit, low)),
                    None => self.bash(f, low),
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
    /// crosshair's line from where it passes her, so walls and slopes
    /// between the camera and her don't stop them.
    fn aim_ray(player: &Player) -> (Vec3, Vec3) {
        let (eye, dir) = (player.eye(), player.aim_forward());
        let past = (player.character.position - eye).dot(dir).max(0.0);
        (eye + dir * past, dir)
    }

    /// The point the crosshair is on (or a long way off along it), seen
    /// past vehicle `own`.
    pub fn aim_point(
        physics: &Physics,
        people: &People,
        player: &Player,
        own: Option<VehicleId>,
    ) -> Vec3 {
        let (eye, dir) = Self::aim_ray(player);
        let far =
            cast_shot(physics, people, eye, dir, MG_RANGE, None, own).map_or(MG_RANGE, |h| h.0);
        eye + dir * far
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
        self.reload = RIFLE_GAP;
        self.aim_left = AIM_AFTER_SHOT;
        let (eye, dir) = Self::aim_ray(f.player);
        let hit = cast_shot(f.physics, f.people, eye, dir, RIFLE_RANGE, None, None);
        let to = hit.map_or(eye + dir * RIFLE_RANGE, |h| eye + dir * h.0);
        let rifle = f.arsenal.carbine.as_ref().or(f.arsenal.rifle.as_ref());
        let from = Self::muzzle(f, rifle);
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
            // What it hit, if the scene's (a crate takes the round).
            let key = f
                .physics
                .cast_ray_keyed(eye, dir, t + 0.05)
                .and_then(|(_, _, key)| key);
            self.struck.push(Strike {
                at: eye + dir * t,
                damage: RIFLE_DAMAGE,
                key,
            });
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
            explosives: &mut [],
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
                rig.two_handed = false;
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

    /// Fires the grenade launcher at what the crosshair is on: the round
    /// is lobbed to come down there when it can reach (the lower arc),
    /// else as far as it goes that way. The template gives the round's
    /// speed and fall, not how the game aims it. An empty clip loads from
    /// the stored rounds; with none, nothing fires.
    fn lob(&mut self, f: &mut Field<'_>) {
        let Some(rounds) = self.launcher_rounds.as_mut() else {
            return;
        };
        if rounds.clip == 0 {
            if rounds.stored > 0 {
                *rounds = Rounds::of(rounds.stored, LAUNCHER_CLIP);
                self.reload = LAUNCHER_RELOAD;
            }
            return;
        }
        rounds.clip -= 1;
        self.reload = LOB_GAP;
        if rounds.clip == 0 && rounds.stored > 0 {
            *rounds = Rounds::of(rounds.stored, LAUNCHER_CLIP);
            self.reload = LAUNCHER_RELOAD;
        }
        self.aim_left = AIM_AFTER_SHOT * 2.0;
        let (eye, dir) = Self::aim_ray(f.player);
        let from = Self::muzzle(f, f.arsenal.launcher.as_ref().or(f.arsenal.rifle.as_ref()));
        let goal = cast_shot(f.physics, f.people, eye, dir, ROCKET_RANGE, None, None)
            .map_or(eye + dir * ROCKET_RANGE, |h| eye + dir * h.0);
        let gravity = m2_engine::physics::GRAVITY;
        let velocity = lob_velocity(from, goal, LOB_SPEED, gravity)
            .unwrap_or_else(|| farthest(goal - from, LOB_SPEED));
        self.projectiles.push(Projectile::Round {
            at: from,
            velocity,
            flown: 0.0,
        });
        self.effect(
            Effect::Flash {
                at: from,
                dir: velocity.normalize_or(dir),
                size: LOB_FLASH,
            },
            FLASH_TIME * 1.4,
        );
        self.sounds.push(Sound::Lob(from));
        alarm(f.people, f.player.character.position);
    }

    /// Fires the gun on the turret of vehicle `i`, which she drives, at
    /// what the crosshair is on: a cannon (a tank's, the Stryker's) fires
    /// a shell that bursts like a rocket, anything else's a machine gun.
    fn vehicle_fire(&mut self, f: &mut Field<'_>, i: usize) {
        let Some(v) = f.vehicles.get(i) else {
            return;
        };
        let (Some(id), Some(muzzle), false) = (v.id, v.spec.muzzle, v.wrecked) else {
            return;
        };
        // A cannon with no shells left is silent.
        if v.rounds() == Some(0) {
            return;
        }
        let Some(body) = f.physics.vehicle_transform(id) else {
            return;
        };
        let cannon = v.spec.cannon();
        let gun = v.spec.tuning.gun;
        let (from, barrel) = v
            .spec
            .muzzle_pose(v.turret, v.gun)
            .unwrap_or((muzzle, Vec3::Z));
        let (from, barrel) = (
            body.transform_point3(from),
            body.transform_vector3(barrel).normalize_or(Vec3::Z),
        );
        let goal = Self::aim_point(f.physics, f.people, f.player, Some(id));
        // At the crosshair once the gun is laid on it; until then, where
        // the barrel points.
        let to_goal = (goal - from).normalize_or(barrel);
        let aim = if to_goal.angle_between(barrel) <= LAID {
            to_goal
        } else {
            barrel
        };
        if cannon {
            // Its template's rate of fire (45 a minute for the AMX-30's)
            // and shell speed (72 m/s).
            self.reload = gun.map_or(CANNON_RELOAD, |g| 60.0 / g.rate_of_fire.max(1.0));
            let speed = gun.map_or(SHELL_SPEED, |g| g.velocity.max(1.0));
            let burn = gun.and_then(|g| g.flash).unwrap_or(CANNON_FLASH_TIME);
            self.projectiles.push(Projectile::Rocket {
                at: from,
                velocity: aim * speed,
                flown: 0.0,
                shell: true,
                owner: Some(id),
            });
            if let Some(v) = f.vehicles.get_mut(i) {
                v.fired += 1;
            }
            self.effect(
                Effect::Flash {
                    at: from,
                    dir: aim,
                    size: CANNON_FLASH,
                },
                burn,
            );
            self.effect(
                Effect::Fire {
                    at: from + aim * 0.8,
                    size: CANNON_FIREBALL,
                },
                burn,
            );
            self.effect(
                Effect::Smoke {
                    at: from + aim * 1.5,
                    size: 1.5,
                },
                1.5,
            );
            self.sounds.push(Sound::Cannon(from));
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

    /// Whether her blow goes low: something stands within reach ahead of
    /// her knees but nothing ahead of her chest (a crate on the ground).
    fn low_blow(f: &Field<'_>) -> bool {
        let feet = f.player.character.position;
        let ahead = Vec3::new(f.player.facing.sin(), 0.0, f.player.facing.cos());
        let meets = |h: f32| {
            f.physics
                .cast_ray(feet + Vec3::Y * h, ahead, BASH_REACH)
                .is_some()
        };
        meets(BASH_LOW) && !meets(BASH_HIGH)
    }

    /// Her blow, high or low, the way she faces: it hurts and shoves those
    /// within reach ahead of her and strikes what of the world's it meets
    /// (a supply crate breaks open).
    fn bash(&mut self, f: &mut Field<'_>, low: bool) {
        let feet = f.player.character.position;
        let ahead = Vec3::new(f.player.facing.sin(), 0.0, f.player.facing.cos());
        let from = feet + Vec3::Y * if low { BASH_LOW } else { BASH_HIGH };
        let push = ahead * BASH_PUSH.0 + Vec3::Y * BASH_PUSH.1;
        for n in f.people.npcs.iter_mut().filter(|n| n.alive) {
            let to = n.character.position - feet;
            let flat = to.with_y(0.0);
            if to.y.abs() > 1.5
                || flat.length() > BASH_REACH + BODY_RADIUS
                || flat.normalize_or_zero().dot(ahead) < BASH_ARC
            {
                continue;
            }
            let soldier = n.soldier;
            if n.hurt(BASH_DAMAGE, push) && soldier {
                self.remarks.push(Remark::Kill);
            }
            self.hits += 1;
        }
        if let Some((t, _, key)) = f.physics.cast_ray_keyed(from, ahead, BASH_REACH) {
            self.struck.push(Strike {
                at: from + ahead * t,
                damage: BASH_BREAK,
                key,
            });
        }
    }

    /// Whether a grenade is in her hand: she is throwing and hasn't let
    /// go yet.
    pub fn grenade_in_hand(&self) -> bool {
        self.throw_in.is_some()
    }

    /// A grenade thrown towards the crosshair, from `hand` (else from
    /// in front of her head).
    fn throw(&mut self, f: &mut Field<'_>, hand: Option<Vec3>) {
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
            Projectile::Round {
                at,
                velocity,
                flown,
            } => {
                velocity.y -= m2_engine::physics::GRAVITY * dt;
                let step = *velocity * dt;
                let len = step.length();
                if let Some((d, _)) = cast_shot(f.physics, f.people, *at, step, len, None, None) {
                    blasts.push(*at + step.normalize_or_zero() * d);
                    return false;
                }
                *at += step;
                *flown += len;
                // Into the sea, or never landing: it bursts there.
                if f.physics.water().is_some_and(|w| at.y < w) || *flown > LOB_RANGE {
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
        self.went_off.push(at);
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

    /// The satellite-guided bomb landing at `at` ([`BOMB`]): a great
    /// fireball and a column of smoke, fires left burning about it.
    pub fn bomb(&mut self, f: &mut Field<'_>, at: Vec3) {
        self.blasts += 1;
        self.last_blast = Some(at);
        self.went_off.push(at);
        self.sounds.push(Sound::Bomb(at));
        self.harm(f, at, BOMB, false);
        self.harm(f, at + Vec3::Y, BOMB_FRAG, true);
        self.effect(
            Effect::Fire {
                at: at + Vec3::Y * 3.0,
                size: 16.0,
            },
            1.4,
        );
        for k in 0..8 {
            let a = k as f32 * std::f32::consts::TAU / 8.0;
            let ring = Vec3::new(a.sin(), 0.3, a.cos());
            self.effect(
                Effect::Fire {
                    at: at + ring * 8.0,
                    size: 8.0,
                },
                1.0,
            );
            self.effect(
                Effect::Smoke {
                    at: at + ring * 7.0 + Vec3::Y * 2.0,
                    size: 8.0,
                },
                6.0,
            );
        }
        for k in 1..=5 {
            let k = k as f32;
            self.effect(
                Effect::Smoke {
                    at: at + Vec3::Y * (6.0 * k),
                    size: 10.0 + k,
                },
                7.0 + k,
            );
        }
        for k in 0..BOMB_FIRES {
            let a = k as f32 * std::f32::consts::TAU / BOMB_FIRES as f32 + self.random();
            let r = BOMB_FIRE_SPREAD * (0.7 + 0.6 * self.random());
            let (shortest, longest) = BOMB_FIRE_TIME;
            let burn = shortest + (longest - shortest) * (self.random() + 0.5);
            let spot = at + Vec3::new(a.sin() * r, 0.0, a.cos() * r);
            // On the ground: a bomb that hit a gate's top (which it brings
            // down) leaves its fires below.
            let ground = ground_under(f.physics, spot).unwrap_or(spot);
            self.fires.push((ground + Vec3::Y * 1.2, 2.5, burn, 0.0));
        }
        alarm(f.people, at);
    }

    /// The bomb's final strike at `at`, after it lands.
    pub fn final_strike(&mut self, f: &mut Field<'_>, at: Vec3) {
        self.went_off.push(at);
        self.sounds.push(Sound::Explosion(at));
        self.harm(f, at, BOMB_FINAL, false);
        self.effect(
            Effect::Fire {
                at: at + Vec3::Y,
                size: 6.0,
            },
            0.7,
        );
    }

    /// Harm from a blast at `at` out to `radius` metres: `damage` at its
    /// heart, falling off to nothing at the edge; with `cover`, what
    /// stands between (the ground, walls, props) stops it. People are
    /// thrown and those near run, vehicles hit hard enough are wrecked
    /// and thrown, explosives within it go off.
    fn harm(&mut self, f: &mut Field<'_>, at: Vec3, (radius, damage): (f32, f32), cover: bool) {
        let physics: &Physics = f.physics;
        let shielded = |to: Vec3| {
            let way = to - at;
            let d = way.length();
            cover && d > 1.0 && physics.cast_ray(at, way / d, d - 0.5).is_some()
        };
        let mut killed = false;
        for n in f.people.npcs.iter_mut() {
            let chest = n.chest();
            let d = chest.distance(at);
            if d < radius * 2.0 {
                n.panic(at);
            }
            if d >= radius || shielded(chest) {
                continue;
            }
            let near = 1.0 - d / radius;
            let away = (chest - at).with_y(0.0).normalize_or(Vec3::X);
            let push = away * (BLAST_PUSH.0 * near + 3.0) + Vec3::Y * (BLAST_PUSH.1 * near + 2.0);
            killed |= n.hurt(damage * near, push) && n.soldier;
        }
        let her = (f.player.driving.is_none() && f.player.landed)
            .then(|| f.player.character.position + Vec3::Y)
            .filter(|&p| p.distance(at) < radius && !shielded(p))
            .map(|p| damage * (1.0 - p.distance(at) / radius));
        let mut wrecking = Vec::new();
        for (i, v) in f.vehicles.iter().enumerate() {
            let Some(body) = v.id.and_then(|id| physics.vehicle_transform(id)) else {
                continue;
            };
            let (lo, hi) = v.spec.bounds;
            let centre = body.transform_point3((lo + hi) * 0.5);
            let d = centre.distance(at);
            let near = 1.0 - d / radius;
            if d < radius && damage * near >= WRECKING && !shielded(centre) {
                wrecking.push((i, centre, near));
            }
        }
        for (i, centre, near) in wrecking {
            let v = &mut f.vehicles[i];
            let Some(id) = v.id else { continue };
            let away = (centre - at).with_y(0.0).normalize_or(Vec3::X);
            let push = (away * 4.0 + Vec3::Y * 6.0) * v.spec.mass * near.max(0.3);
            f.physics
                .push_vehicle(id, push, centre + away * -0.5 + Vec3::Y * 0.3);
            if !v.wrecked {
                v.wrecked = true;
                f.physics.wreck_vehicle(id);
                self.wrecks.push((i, 0.0));
                self.remarks.push(Remark::Wreck);
            }
        }
        for e in f.explosives.iter_mut() {
            if !e.gone && e.at.distance(at) < e.size + radius {
                e.gone = true;
                self.fuses.push((e.at, CHAIN_DELAY));
                self.fires.push((e.at, e.size, BURN, 0.0));
            }
        }
        if let Some(d) = her {
            self.hurt_player(f, d);
        }
        if killed {
            self.remarks.push(Remark::Kill);
        }
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
        let mut blazing = Vec::new();
        self.blazes.retain_mut(|(at, left, next)| {
            *left -= dt;
            *next -= dt;
            if *next <= 0.0 {
                *next = 0.35;
                blazing.push(*at);
            }
            *left > 0.0
        });
        for at in blazing {
            self.effect(Effect::Fire { at, size: 3.0 }, 0.7);
            self.effect(
                Effect::Soot {
                    at: at + Vec3::Y * 3.0,
                    size: 3.5,
                },
                6.0,
            );
        }
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

/// The ground under `at` (from 6 m above it, so a spot up a slope is
/// found too, down to 60 m), seen through the state layers' models (a gate
/// a bomb brings down).
fn ground_under(physics: &Physics, at: Vec3) -> Option<Vec3> {
    let from = at + Vec3::Y * 6.0;
    physics
        .cast_ray_skipping(from, Vec3::NEG_Y, 60.0, |key| {
            matches!(key, StaticKey::Entity(_))
        })
        .map(|(d, _)| from - Vec3::Y * d)
}

/// The velocity at `speed` that lobs a round from `from` down onto `to`
/// under `gravity`: the lower of its two arcs, if it reaches.
fn lob_velocity(from: Vec3, to: Vec3, speed: f32, gravity: f32) -> Option<Vec3> {
    let d = to - from;
    let flat = d.with_y(0.0);
    let x = flat.length();
    if x < 1e-3 {
        return Some(d.normalize_or(Vec3::NEG_Y) * speed);
    }
    let v2 = speed * speed;
    let room = v2 * v2 - gravity * (gravity * x * x + 2.0 * d.y * v2);
    if room < 0.0 {
        return None;
    }
    let (sin, cos) = ((v2 - room.sqrt()) / (gravity * x)).atan().sin_cos();
    Some(flat / x * (speed * cos) + Vec3::Y * (speed * sin))
}

/// The velocity at `speed` that lobs a round furthest along `way`: 45
/// degrees up.
fn farthest(way: Vec3, speed: f32) -> Vec3 {
    let flat = way.with_y(0.0).normalize_or(Vec3::Z);
    (flat + Vec3::Y).normalize() * speed
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
        player.aim_crosshair_at(chest);
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
    fn a_bash_drops_the_soldier_ahead_and_goes_low_at_a_crate() {
        let mut physics = ground();
        let dt = 1.0 / 60.0;
        let mut player = Player::new(0.0, 0.0, 0.0, None);
        let mut people = People::default();
        // One just ahead of her, one just behind.
        for z in [1.2, -1.2] {
            people.npcs.push(Npc::new(
                0,
                &rig(),
                Vec3::new(0.0, 1.0, z),
                std::f32::consts::PI,
                Brain::Post { facing: 0.0 },
                Some(1),
            ));
        }
        for _ in 0..3 {
            people.update(&mut physics, &[], Vec3::ZERO, None, dt);
            player.update(&physics, Default::default(), dt);
        }
        assert!(player.landed);
        let arsenal = Arsenal::default();
        let mut vehicles = Vec::new();
        let mut combat = Combat::default();
        let mut step = |combat: &mut Combat,
                        physics: &mut Physics,
                        player: &mut Player,
                        people: &mut People,
                        melee: bool| {
            let mut field = Field {
                physics,
                player,
                people,
                vehicles: &mut vehicles,
                arsenal: &arsenal,
                explosives: &mut [],
            };
            combat.step(
                &mut field,
                Trigger {
                    melee,
                    ..Trigger::default()
                },
                dt,
            );
        };
        // Two blows a second apart (with no clip, each lands at once).
        for k in 0..60 {
            step(
                &mut combat,
                &mut physics,
                &mut player,
                &mut people,
                k % 60 == 0,
            );
        }
        assert!(people.npcs[0].alive, "one blow hurts");
        for k in 0..60 {
            step(&mut combat, &mut physics, &mut player, &mut people, k == 0);
        }
        assert!(!people.npcs[0].alive, "two drop him");
        assert!(people.npcs[1].alive && people.npcs[1].hp >= 100.0);
        assert_eq!(combat.hits, 2);
        // A crate on the ground ahead of her knees, below her chest: the
        // blow goes low and strikes it.
        people.npcs.clear();
        physics.add_static(
            StaticKey::Entity(7),
            &m2_engine::CollisionShape::cuboid(Vec3::new(0.5, 0.4, 0.5)),
            player.character.position.with_y(0.4) + Vec3::Z * 1.4,
            Quat::IDENTITY,
        );
        physics.step(dt);
        step(&mut combat, &mut physics, &mut player, &mut people, true);
        let s = combat.struck.first().copied().expect("struck");
        assert_eq!(s.key, Some(StaticKey::Entity(7)));
        assert!(s.at.y < 0.8 && s.damage >= 100.0, "{s:?}");
    }

    #[test]
    fn a_bombs_fires_burn_on_the_ground_under_a_gate() {
        let mut physics = ground();
        // A gate 15 m high (a state layer's) and a house 6 m high.
        let wall = |half: Vec3| m2_engine::CollisionShape::cuboid(half);
        physics.add_static(
            StaticKey::Entity(7),
            &wall(Vec3::new(10.0, 7.5, 2.0)),
            Vec3::new(0.0, 7.5, 0.0),
            Quat::IDENTITY,
        );
        physics.add_static(
            StaticKey::Object(3),
            &wall(Vec3::new(4.0, 3.0, 4.0)),
            Vec3::new(40.0, 3.0, 0.0),
            Quat::IDENTITY,
        );
        physics.step(1.0 / 60.0);
        let on = |at: Vec3| ground_under(&physics, at).map(|g| (g.y * 10.0).round() / 10.0);
        assert_eq!(on(Vec3::new(1.0, 15.0, 0.5)), Some(0.0));
        assert_eq!(on(Vec3::new(40.0, 6.0, 0.0)), Some(6.0));
        assert_eq!(on(Vec3::new(80.0, 2.0, 0.0)), Some(0.0));
    }

    #[test]
    fn a_slung_weapon_hangs_down_her_back() {
        // Her back as she stands facing +z, her right to -x.
        let (right, up) = (Vec3::NEG_X, Vec3::Y);
        // A carbine 0.85 m long, its barrel along +z from its grip, its
        // stock 0.3 m behind.
        let muzzle = Vec3::new(0.0, 0.05, 0.5);
        let (lo, hi) = (Vec3::new(-0.04, -0.15, -0.3), Vec3::new(0.04, 0.08, 0.55));
        let point = Vec3::new(-0.1, 1.4, -0.14);
        let at = slung_at(muzzle, (lo, hi), point, (right, up), 0).unwrap();
        let barrel = (at.transform_point3(muzzle) - at.transform_point3(Vec3::ZERO)).normalize();
        // Nearly straight down, its muzzle toward her left hip.
        assert!(barrel.y < -0.95 && barrel.dot(right) < 0.0, "{barrel}");
        // From the base of her neck down past her hips, all of it behind
        // her.
        let corners: Vec<Vec3> = (0..8)
            .map(|k| {
                let pick = |bit: usize, a: f32, b: f32| if k & bit == 0 { a } else { b };
                at.transform_point3(Vec3::new(
                    pick(1, lo.x, hi.x),
                    pick(2, lo.y, hi.y),
                    pick(4, lo.z, hi.z),
                ))
            })
            .collect();
        let top = corners.iter().map(|c| c.y).fold(f32::MIN, f32::max);
        let bottom = corners.iter().map(|c| c.y).fold(f32::MAX, f32::min);
        assert!(top > 1.5 && top < 1.6, "{top}");
        assert!(bottom > 0.6 && bottom < 0.8, "{bottom}");
        assert!(corners.iter().all(|c| c.z < point.z - 0.02), "{corners:?}");
        // On her left the same way, its top to her left.
        let left = slung_at(muzzle, (lo, hi), point + Vec3::X * 0.2, (right, up), 1).unwrap();
        assert!(left.transform_vector3(Vec3::Y).dot(right) < -0.9);
        assert!(at.transform_vector3(Vec3::Y).dot(right) > 0.9);
    }

    #[test]
    fn her_back_runs_up_her_spine() {
        // Her bind pose (the rig round 98 listed), facing +z: the attach
        // points high behind her spine, which runs up to her neck.
        let arsenal = Arsenal {
            back: [Some(0), Some(1)],
            spine: Some(2),
            neck: Some(3),
            ..Arsenal::default()
        };
        let at = |x: f32, y: f32, z: f32| Mat4::from_translation(Vec3::new(x, y, z));
        let nodes = [
            at(-0.084, 1.396, -0.186),
            at(0.084, 1.396, -0.186),
            at(0.0, 1.065, -0.025),
            at(0.0, 1.547, -0.053),
        ];
        let body = Mat4::from_translation(Vec3::new(5.0, 2.0, 0.0));
        let (point, (right, up)) = arsenal.back_at(body, &nodes, 0).unwrap();
        assert!(
            point.distance(Vec3::new(4.916, 3.396, -0.186)) < 1e-4,
            "{point}"
        );
        assert!(right.normalize().dot(Vec3::NEG_X) > 0.999, "{right}");
        // Up, leaning back no more than her spine does (not the 26
        // degrees from her lower spine to the attach points).
        let up = up.normalize();
        assert!(up.y > 0.99 && up.z < 0.0, "{up}");
        // A weapon slung there hangs behind her, not by her side.
        let muzzle = Vec3::new(0.0, 0.05, 0.5);
        let bounds = (Vec3::new(-0.04, -0.15, -0.3), Vec3::new(0.04, 0.08, 0.55));
        let slung = slung_at(muzzle, bounds, point, (right, up), 0).unwrap();
        let lower = slung.transform_point3(muzzle);
        assert!(lower.z < -0.15 && lower.y < 3.0, "{lower}");
        // The short one's top at the base of her neck; a long one (the
        // RPG's 1.28 m) reaches further down, and up no higher than her
        // head.
        let long = (Vec3::new(-0.05, -0.15, -0.5), Vec3::new(0.05, 0.15, 0.78));
        let raised = slung_at(muzzle, long, point, (right, up), 0).unwrap();
        let (short, longer) = (heights(slung, bounds), heights(raised, long));
        assert!(
            (short.1 - (point.y + SLUNG_TOP)).abs() < 0.02,
            "{short:?} {point}"
        );
        assert!(
            longer.0 < short.0 - 0.1 && longer.1 < short.1 + 0.2,
            "{short:?} {longer:?}"
        );
        assert!(
            (longer.0 - (point.y - SLUNG_FLOOR)).abs() < 0.02,
            "{longer:?} {point}"
        );
    }

    /// The lowest and highest a model's box `(lo, hi)` reaches placed by
    /// `m`.
    fn heights(m: Mat4, (lo, hi): (Vec3, Vec3)) -> (f32, f32) {
        box_corners(m, (lo, hi))
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), c| (a.min(c.y), b.max(c.y)))
    }

    /// A model's box `(lo, hi)`'s corners placed by `m`.
    fn box_corners(m: Mat4, (lo, hi): (Vec3, Vec3)) -> Vec<Vec3> {
        (0..8)
            .map(|k| {
                let pick = |bit: usize, a: f32, b: f32| if k & bit == 0 { a } else { b };
                m.transform_point3(Vec3::new(
                    pick(1, lo.x, hi.x),
                    pick(2, lo.y, hi.y),
                    pick(4, lo.z, hi.z),
                ))
            })
            .collect()
    }

    #[test]
    fn a_slung_weapon_hangs_along_its_length_whatever_its_muzzle() {
        // An RPG as round 99 had it: 1.28 m long down its z, its muzzle
        // marker well above that line (the line from its grip to the
        // marker runs 20 degrees off its length), on her right.
        let (right, up) = (Vec3::NEG_X, Vec3::Y);
        let muzzle = Vec3::new(0.0, 0.22, 0.6);
        let bounds = (Vec3::new(-0.055, -0.15, -0.5), Vec3::new(0.055, 0.15, 0.78));
        let point = Vec3::new(-0.084, 1.396, -0.186);
        let at = slung_at(muzzle, bounds, point, (right, up), 0).unwrap();
        // Its length straight down her back but for the lean, its muzzle
        // end down.
        let along = at.transform_vector3(Vec3::Z);
        assert!(along.y < -0.98, "{along}");
        // Across her back no wider than the lean takes it (round 99's
        // spread 0.9 m from her left to her right), right of her spine.
        let across: Vec<f32> = box_corners(at, bounds)
            .iter()
            .map(|c| c.dot(right))
            .collect();
        let (least, most) = across
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), &x| (a.min(x), b.max(x)));
        assert!(most - least < 0.55, "{least} {most}");
        assert!(least + most > 0.0, "{least} {most}");
        // Up to about the top of her head, no higher (round 100's stood
        // 0.3 m over it), and down to just past her knee, not her shin.
        let (bottom, top) = heights(at, bounds);
        assert!(bottom > 0.35 && top > 1.6 && top < 1.75, "{bottom} {top}");
    }

    #[test]
    fn the_weapons_she_carries_but_doesnt_hold_hang_on_her_back() {
        let mut combat = Combat::default();
        // The first contract's start: the carbine alone, up.
        combat.carbine_only();
        assert_eq!(combat.slung(true), [None, None]);
        // The crate's grenade launcher, up: the carbine on her back.
        combat.equip(Weapon::GrenadeLauncher, 18);
        assert_eq!(combat.slung(true), [Some(Weapon::Rifle), None]);
        // The carbine up again: the launcher on her back, on her right as
        // the carbine was.
        combat.weapon = Weapon::Rifle;
        assert_eq!(combat.slung(true), [Some(Weapon::GrenadeLauncher), None]);
        // The designator in her hands: both.
        assert_eq!(
            combat.slung(false),
            [Some(Weapon::Rifle), Some(Weapon::GrenadeLauncher)]
        );
    }

    #[test]
    fn a_tank_fires_no_more_shells_than_its_cannon_carries() {
        use m2_engine::VehicleSpec;
        use m2_engine::tuning::{GunTuning, VehicleTuning};
        use m2_engine::vehicle::WheelSpec;
        let mut physics = ground();
        let dt = 1.0 / 60.0;
        let wheel = |x: f32, z: f32| WheelSpec {
            centre: Vec3::new(x, 0.4, z),
            radius: 0.4,
            width: 0.4,
            steers: false,
        };
        let hull = (Vec3::new(-1.5, 0.0, -3.0), Vec3::new(1.5, 2.0, 3.0));
        // Its cannon carries two shells and could fire ten a second.
        let gun = GunTuning {
            weapon: 1,
            rate_of_fire: 600.0,
            rounds: 2,
            velocity: 72.0,
            max_velocity: 72.0,
            gravity_scale: 1.0,
            flash: None,
            sound_bank: None,
        };
        let spec = VehicleSpec {
            model: 1,
            kind: "tank".into(),
            label: "test".into(),
            wheels: vec![
                wheel(-1.3, 2.0),
                wheel(1.3, 2.0),
                wheel(-1.3, -2.0),
                wheel(1.3, -2.0),
            ],
            bounds: hull,
            seat: None,
            door: None,
            tracked: true,
            road_wheels: Vec::new(),
            rest: Vec::new(),
            node_wheel: Vec::new(),
            mass: 32_000.0,
            turret: Vec::new(),
            pivot: None,
            muzzle: Some(Vec3::new(0.0, 2.2, 5.0)),
            gun: Vec::new(),
            gun_pivot: None,
            muzzle_on_gun: false,
            elevation: (0.0, 0.0),
            hull,
            turret_box: None,
            tuning: VehicleTuning {
                gun: Some(gun),
                ..VehicleTuning::default()
            },
        };
        let id = physics.add_vehicle(&spec, Vec3::new(0.0, 0.1, 0.0), 0.0);
        physics.step(dt);
        let mut vehicles = vec![ParkedVehicle {
            spec,
            model: crate::npc::tests::empty_model(),
            id: Some(id),
            wrecked: false,
            guid: None,
            spot: None,
            turret: 0.0,
            gun: 0.0,
            rolled: 0.0,
            fired: 0,
        }];
        assert_eq!(vehicles[0].rounds(), Some(2));
        let mut player = Player::new(0.0, 0.0, 0.0, None);
        player.driving = Some(0);
        let mut people = People::default();
        let arsenal = Arsenal::default();
        let mut combat = Combat::default();
        let fire = Trigger {
            fire: true,
            ..Trigger::default()
        };
        // Two seconds of holding the trigger: two shells, then none.
        let mut shots = 0;
        for _ in 0..120 {
            let mut field = Field {
                physics: &mut physics,
                player: &mut player,
                people: &mut people,
                vehicles: &mut vehicles,
                arsenal: &arsenal,
                explosives: &mut [],
            };
            combat.step(&mut field, fire, dt);
            let sounds = combat.take_sounds();
            shots += sounds
                .iter()
                .filter(|s| matches!(s, Sound::Cannon(_)))
                .count();
        }
        assert_eq!(shots, 2);
        assert_eq!(vehicles[0].rounds(), Some(0));
        // A car's gun (none here) doesn't count.
        vehicles[0].spec.kind = "car".into();
        vehicles[0].spec.tuning.gun = None;
        assert_eq!(vehicles[0].rounds(), None);
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
        player.aim_crosshair_at(chest);
        let hit = Combat::aim_probe(&physics, &people, &player);
        assert!(
            matches!(hit, Some((_, Some(0)))),
            "{hit:?} eye {} chest {chest}",
            player.eye()
        );
    }

    #[test]
    fn a_lobbed_round_comes_down_where_it_is_aimed() {
        let g = m2_engine::physics::GRAVITY;
        let from = Vec3::new(0.0, 1.5, 0.0);
        for to in [
            Vec3::new(0.0, 0.0, 25.0),
            Vec3::new(10.0, 4.0, 10.0),
            Vec3::new(0.0, -6.0, 3.0),
        ] {
            let v = lob_velocity(from, to, LOB_SPEED, g).expect("in reach");
            assert!((v.length() - LOB_SPEED).abs() < 1e-3);
            // Flown in small steps, it passes within a hand of the mark.
            let (mut at, mut vel, mut closest) = (from, v, f32::MAX);
            for _ in 0..4000 {
                vel.y -= g * 0.001;
                at += vel * 0.001;
                closest = closest.min(at.distance(to));
            }
            assert!(closest < 0.1, "{to}: {closest}");
            // The lower arc: no higher than it need be.
            assert!(v.y < LOB_SPEED * 0.75, "{to}: {v}");
        }
        // Beyond its reach (about 40 m on the level): nothing; then it is
        // lobbed as far as it goes.
        assert!(lob_velocity(from, Vec3::new(0.0, 0.0, 60.0), LOB_SPEED, g).is_none());
        let far = farthest(Vec3::Z * 60.0, LOB_SPEED);
        assert!((far.y - far.z).abs() < 1e-4 && far.x == 0.0);
    }

    #[test]
    fn the_grenade_launcher_takes_the_rpgs_place_and_runs_out() {
        let mut physics = ground();
        let dt = 1.0 / 60.0;
        let mut player = Player::new(0.0, 0.0, 0.0, None);
        for _ in 0..3 {
            player.update(&physics, Default::default(), dt);
        }
        assert!(player.landed);
        let mut people = People::default();
        let arsenal = Arsenal::default();
        let mut vehicles = Vec::new();
        let mut combat = Combat::default();
        // Taken from the crate with 3 clips: the RPG is left behind.
        assert_eq!(
            combat.equip(Weapon::GrenadeLauncher, 18),
            Some((Weapon::Rpg, 0))
        );
        assert_eq!(combat.weapon, Weapon::GrenadeLauncher);
        assert_eq!(
            combat.launcher_rounds,
            Some(Rounds {
                clip: 6,
                stored: 12
            })
        );
        // The launcher key takes up what is in the launcher's place.
        let mut step = |combat: &mut Combat, player: &mut Player, trigger: Trigger| {
            let mut field = Field {
                physics: &mut physics,
                player,
                people: &mut people,
                vehicles: &mut vehicles,
                arsenal: &arsenal,
                explosives: &mut [],
            };
            combat.step(&mut field, trigger, dt);
        };
        for select in [Weapon::Rifle, Weapon::Rpg] {
            let trigger = Trigger {
                select: Some(select),
                ..Trigger::default()
            };
            step(&mut combat, &mut player, trigger);
        }
        assert_eq!(combat.weapon, Weapon::GrenadeLauncher);
        // At the ground 20 m ahead: the rounds come down there, a clip at
        // a time, until all 18 are gone.
        let mark = Vec3::new(0.0, 0.0, 20.0);
        player.aim_crosshair_at(mark);
        let fire = Trigger {
            fire: true,
            ..Trigger::default()
        };
        for _ in 0..(60.0 * 30.0) as usize {
            step(&mut combat, &mut player, fire);
        }
        assert_eq!(combat.launcher_rounds, Some(Rounds::default()));
        assert_eq!(combat.blasts, 18);
        let last = combat.last_blast.unwrap();
        assert!(last.with_y(0.0).distance(mark) < 1.0, "{last}");
        let lobs = combat
            .take_sounds()
            .iter()
            .filter(|s| matches!(s, Sound::Lob(_)))
            .count();
        assert_eq!(lobs, 18);
        assert!(!combat.second_loaded());
        // More rounds go with it; the RPG taken back leaves it, with them.
        combat.take_ammo(Weapon::GrenadeLauncher, 6);
        assert!(combat.second_loaded());
        assert_eq!(
            combat.equip(Weapon::Rpg, 0),
            Some((Weapon::GrenadeLauncher, 6))
        );
        assert_eq!(
            (combat.second, combat.launcher_rounds),
            (Some(Weapon::Rpg), None)
        );
        // Landed with the carbine alone: the launcher key does nothing, and
        // the crate's launcher leaves nothing behind.
        combat.carbine_only();
        let mut step = |combat: &mut Combat, trigger: Trigger| {
            let mut field = Field {
                physics: &mut physics,
                player: &mut player,
                people: &mut people,
                vehicles: &mut vehicles,
                arsenal: &arsenal,
                explosives: &mut [],
            };
            combat.step(&mut field, trigger, dt);
        };
        let two = Trigger {
            select: Some(Weapon::Rpg),
            ..Trigger::default()
        };
        step(&mut combat, two);
        assert_eq!(combat.weapon, Weapon::Rifle);
        assert_eq!(combat.equip(Weapon::GrenadeLauncher, 18), None);
        assert_eq!(combat.second, Some(Weapon::GrenadeLauncher));
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
