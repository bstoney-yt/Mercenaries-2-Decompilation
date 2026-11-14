//! What the template database says of every weapon: the guns people
//! carry, what they throw, and what vehicles mount (survey `arsenal`,
//! PC round r01 of the weapons thread: 283 weapon templates).
//!
//! A gun's template has a `WeaponProjectileBase`: its `FireType` (0 for
//! guns that fire while the trigger is held, 1 for one shot a pull: the
//! pistols, the shotgun, sniper rifles and every launcher), `RateOfFire`
//! (a minute), `iClipSize`, `MaxAmmoReserve` (clips), `iBulletsPerShot`
//! (the shotgun's 10), `iRoundsPerReload` (the shotgun's 1: it loads a
//! shell at a time) and its `AmmoTemplate`. The ammunition's
//! `ControllerVelocity` gives its `Velocity` and `HumanBoost` (the
//! velocity's multiple when a person fires it: a rifle round's 8, a
//! rocket's 2), its `ProjectilePhysics` its `Damage`, `DamageMinimum` and
//! `GravityScale`, its `DamageKey` the kind of damage (the damage table's
//! column), and its `SpawnOnDeath` (else its `TimerResponse`) the
//! explosion it becomes, whose `Explosive` gives `Radius`, `Damage`,
//! `MinDamage` and `MaxForce`.
//!
//! A thrown weapon (`Grenade`, `C4`, `Proximity Mine`) has a
//! `WeaponThrown` instead: its projectile, how many she carries, the
//! throw's speed at three reticle pitches (`ReticlePitchMin`/`Middle`/
//! max: -45, 0 and 45 degrees for the grenade, 9, 13 and 16 m/s),
//! `ThrowAngle`, whether it can be cooked (`bCook`) and the detonator it
//! needs (`TriggerTemplate`: C4's `C4 Detonator`).
//!
//! A vehicle's `PhysicalLink`s mount weapon templates at hardpoints under
//! link names; each seat's template has a `ControllerWeapon` per weapon
//! its rider fires (the link name, and its `Priority`: 0 the first), and
//! the vehicle's `WeaponCoupling`s chain link names that fire in turn
//! (a helicopter's rocket pods, one hardpoint after the next).

use std::collections::HashMap;

use m2_assets::{Layer, name_hash};
use m2_math::glam::Vec3;

use crate::tuning::Table;

/// `PhysicalLink`'s field naming the template it links (name unknown).
const LINK: u32 = 0x5470_4554;
/// `ControllerVelocity`'s multiple of `Velocity` when a person fires it.
const HUMAN_BOOST: u32 = 0xf3f6_f0a9;
/// `ProjectilePhysics`' `DamageMinimum`, `DamageDropoffStart` and
/// `DamageDropoffStop` (metres: the carbine round's 15 and 50 m, the
/// shotgun's 5 and 50, the heavy machine gun's 20 and 300) and
/// `DamageDropoff` (names found by hashing, gap-list sweep; before, read
/// by their values alone). See [`Ammo::damage_at`].
const DAMAGE_MINIMUM: u32 = 0x33f1_fb12;
const DAMAGE_DROPOFF: u32 = 0xd698_8c68;
/// `ProjectilePhysics`' `HeroMultiplier` (name found by hashing guesses,
/// round 19): the carbine's, rifles', pistols' and machine guns' rounds 3,
/// the automatic rifle's and light machine gun's 2, the RPG's 1.5, the
/// sniper and anti-material rifles' and every bomb's 1.
const HERO_MULTIPLIER: u32 = 0x0858_81b0;
const FALLOFF_NEAR: u32 = 0xe184_b8a0;
const FALLOFF_FAR: u32 = 0x1d98_7340;
/// `SpawnOnDeath`'s and `TimerResponse`'s template field (name unknown).
const SPAWNS: u32 = 0x7461_c0cd;
/// `Equipment`'s carry class (`Rifle`, `Pistol`, `Rocket`, `Grenade`).
const CLASS: u32 = 0xd641_1a49;
/// `HomingProjectile`'s turn rate (degrees a second) and the widest angle
/// off its nose it keeps a target in (degrees).
const TURN_SPEED: u32 = 0xcefb_c415;
const MAX_ANGLE: u32 = 0xa10e_79fa;
/// `HomingWeapon`'s lock cone, degrees (its name not found; the 360's
/// weapon-equipped event sizes the lock-on frame by it, `0x8234F458`).
const LOCK_CONE: u32 = 0x6749_c828;
/// `HomingWeapon`'s lock range, metres (the Stinger's 400): the cone
/// test's range (360 0x822938C0, record +0xc).
const LOCK_RANGE: u32 = 0x1a09_69d2;
/// `HomingWeapon`'s kind (+0: 1 ground, 2 air, 3 the TOW's, 4 the
/// grapple's), least target signature (+4) and cone half-angle (+8,
/// degrees), as 0x822938C0 and 0x82294318 read them (names unknown).
const SEEK_KIND: u32 = 0x28e1_0525;
const SEEK_SIGNATURE: u32 = 0x8b2b_83f1;
const SEEK_CONE: u32 = LOCK_CONE;
/// `HomingTarget`'s ground and air signatures (+0, +4; the runtime
/// component's +0x24 / +0x28 that 0x822938C0 picks by the weapon's kind:
/// a CRX's 1 and 0, a helicopter's 0 and 1). Names unknown.
const SIGNATURE_GROUND: u32 = 0xe49a_d93b;
const SIGNATURE_AIR: u32 = 0x4d93_dc23;
/// `WeaponThrown`'s throw speeds at the reticle's lowest, middle and
/// highest pitch (unnamed: the grenade's 9, 13 and 16 m/s; inferred from
/// `ReticlePitchMin`/`Middle` beside them), and the throw's rise.
const THROW_SPEEDS: [u32; 3] = [0xd4c5_2205, 0x2666_7136, 0x6cef_3429];
const RETICLE_PITCHES: [u32; 3] = [0x0c14_ca3d, 0x7349_6c4c, 0xee87_3859];
/// `Pickup`'s refill template and amount (names unknown).
const PICKUP_REFILL: u32 = 0xc867_d0d0;
const PICKUP_AMOUNT: u32 = 0xa06b_3faa;
/// `ControllerWeapon`'s link name (name unknown).
const SEAT_WEAPON: u32 = 0x31e1_a9b0;
/// `WeaponCoupling`'s link names: fired, then the one that fires next.
const COUPLE_FROM: u32 = 0x96a0_4514;
const COUPLE_TO: u32 = 0xadfb_c57f;
/// `Stimulus`' first field: a planted mine's reach, metres (the planted
/// proximity mine's 3; inferred).
const STIMULUS_REACH: u32 = 0x1417_f4dd;
/// `WeaponScatter`'s first field (see [`WeaponSpec::scatter`]).
const SCATTER: u32 = 0x0006_5d26;

/// `WeaponEffects`' `ShellEjectHardpoint` (the gun model's node a spent
/// case flies from, a name hash: `hp_shelleject` on all guns but the
/// covert pistol's `7eabfa05`; round 17 read its bits as a float, 0.035,
/// and took it for the flash's time) and `ShellEjectTemplate` (the case's effect,
/// `global_particle_shellsmall`). Both names found by hashing guesses.
const SHELL_HARDPOINT: u32 = 0xd62d_ab7b;
const SHELL: u32 = 0x1efe_cdea;
/// `SoundEffect`'s field naming a sound bank (the carbine's
/// `wpn_combatrifle` block's bank `50333181`; vehicles' engines too).
const SOUND_BANK: u32 = 0x14a6_7fa6;

/// What firing a gun looks and sounds like, as its template says.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Feel {
    /// Its shots' sound bank (`SoundEffect`), a name hash: each gun
    /// family has its own (`wpn_combatrifle`, `wpn_shotgun`, ...).
    pub sound_bank: Option<u32>,
    /// Its `SoundSet` (a hash; the carbine's `01b47c45`): what the
    /// `Sounds` table keys its cues by ([`crate::gun_sounds`]).
    pub sound_set: Option<u32>,
    /// Its muzzle flash's template (`MuzzleFlashTemplate`:
    /// `global_particle_muzzleflash_blue` for the carbine).
    pub flash: Option<u32>,
    /// The effect of a spent case thrown out (`ShellEjectTemplate`:
    /// `global_particle_shellsmall`), and the model node it flies from
    /// (`ShellEjectHardpoint`, see [`SHELL_HARDPOINT`]).
    pub shell: Option<u32>,
    pub shell_hardpoint: Option<u32>,
    /// `WeaponScatter`'s seven fields in their order (see
    /// [`SCATTER_FIELDS`]; [`Feel::scatter_min`] and the next two are
    /// named, the first four are not).
    pub scatter: [f32; 7],
}

impl Feel {
    /// `ScatterMin`, `ScatterMax` and `ScatterPerShot`: the cone a gun's
    /// rounds leave in (degrees across, as the alpha reads them: the
    /// carbine's 1 to 2, the assault rifle's 1.5 to 3, the sniper rifle's 0
    /// to 3), and what each shot adds to it (0.2; the sniper rifle's 1).
    pub fn scatter_min(&self) -> f32 {
        self.scatter[4]
    }

    pub fn scatter_max(&self) -> f32 {
        self.scatter[5]
    }

    pub fn scatter_per_shot(&self) -> f32 {
        self.scatter[6]
    }

    /// The cone its next round leaves in, degrees across, when its shots
    /// so far have added `bloom` ([`Bloom::added`]): `ScatterMin` plus that,
    /// no more than `ScatterMax`.
    pub fn cone(&self, bloom: f32) -> f32 {
        let (lo, hi) = (self.scatter_min(), self.scatter_max());
        (lo + bloom.max(0.0)).min(hi.max(lo))
    }
}

/// The way a round goes for an aim along unit `aim` out of a cone `cone`
/// degrees across ([`Feel::cone`]), given two uniform picks `a` and `b`
/// in [-0.5, 0.5): anywhere in the round cone, evenly over its area.
///
/// Provenance: that the cone is round and its rounds spread evenly over
/// it is invented (the 360 code that strays a round was not read). Before
/// playtest 8 the alpha took each round sideways and up on their own, in
/// a square as wide as the cone, whose corners reach 1.41 times as far
/// out, so more rounds went wide of a soldier at range (`playerspread`
/// example, the carbine at rest at 100 m: 31% of rounds on him, 40%
/// now; her 56 guns' mean at 100 m: 19% to 25%).
pub fn stray_in_cone(aim: Vec3, cone: f32, a: f32, b: f32) -> Vec3 {
    let radius = 0.5 * cone.max(0.0).to_radians();
    if radius <= 0.0 {
        return aim;
    }
    let side = aim.cross(Vec3::Y).normalize_or(Vec3::X);
    let up = side.cross(aim).normalize_or(Vec3::Y);
    let r = radius * (a + 0.5).clamp(0.0, 1.0).sqrt();
    let turn = (b + 0.5) * std::f32::consts::TAU;
    (aim + side * (r * turn.cos()) + up * (r * turn.sin())).normalize_or(aim)
}

/// What a gun's shots have added to its cone ([`Feel::cone`]): each
/// shot `ScatterPerShot`, up to `ScatterMax`. Once the gun has gone
/// [`Bloom::REST`] seconds without firing, the cone settles back to
/// `ScatterMin`, in [`Bloom::SETTLE`] seconds from the most.
///
/// Provenance: the three fields and their names are the template's (from
/// samples); that they add up so is inferred from the names; when and how
/// fast the cone settles is invented (the game's own rule is not decoded:
/// the first field, once taken for a rate, is `LowSkillScatter`; none of
/// the 31 components on the guns' templates names a settle rate among
/// the fields named so far, gap round 13).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bloom {
    /// Degrees added to `ScatterMin`.
    pub added: f32,
    /// Seconds since the last shot.
    idle: f32,
}

impl Bloom {
    /// Seconds without a shot before the cone settles (invented).
    pub const REST: f32 = 0.25;
    /// Seconds from `ScatterMax` back to `ScatterMin` (invented).
    pub const SETTLE: f32 = 1.0;

    /// After a shot of a gun that feels as `feel`.
    pub fn shot(&mut self, feel: &Feel) {
        let room = (feel.scatter_max() - feel.scatter_min()).max(0.0);
        self.added = (self.added + feel.scatter_per_shot().max(0.0)).min(room);
        self.idle = 0.0;
    }

    /// After `dt` seconds.
    pub fn settle(&mut self, feel: &Feel, dt: f32) {
        self.idle += dt;
        if self.idle < Self::REST {
            return;
        }
        let room = (feel.scatter_max() - feel.scatter_min()).max(0.0);
        self.added = (self.added - room * dt / Self::SETTLE).max(0.0);
    }
}

/// `WeaponScatter`'s fields in the order the template database lists
/// them. The last three are `ScatterMin`, `ScatterMax` and
/// `ScatterPerShot` (names found by hashing guesses, round 19); the first
/// is `LowSkillScatter` (gap round 13: a soldier's cone, not a settle
/// rate); the second to fourth are not named. The third and fourth look like a second pair of a
/// least and a most (the third is never above the fourth, and both are
/// mostly below `ScatterMin` and `ScatterMax`: the carbine's 1 and 1.5
/// beside 1 and 2), perhaps for aiming or crouching; not decided.
pub const SCATTER_FIELDS: [u32; 7] = [
    0x0006_5d26,
    0xa2f3_cad9,
    0x3a4e_af3b,
    0x0bbd_cc45,
    0x2de4_f3c3,
    0x2f0e_093d,
    0x9318_44be,
];

/// The hand guns' sound banks, by name: each is its own block
/// (`wpn_combatrifle_P000_Q3.block`), and a gun template's `SoundEffect`
/// names one by hash (the carbine's `50333181`, `wpn_combatrifle`).
pub const GUN_BANKS: [&str; 26] = [
    "wpn_antiair",
    "wpn_antimaterialrifle",
    "wpn_assaultrifle",
    "wpn_automaticrifle",
    "wpn_bullpuprifle",
    "wpn_coilgun",
    "wpn_combatrifle",
    "wpn_covertpistol",
    "wpn_covertsmg",
    "wpn_designator_flare",
    "wpn_emplacedheavymg",
    "wpn_emplacedlightmg",
    "wpn_grapplegun",
    "wpn_grenadelauncher",
    "wpn_heavyatmissile",
    "wpn_huntingpistol",
    "wpn_lightmg",
    "wpn_machinepistol",
    "wpn_minigun",
    "wpn_pistol",
    "wpn_rocketlauncher",
    "wpn_rpg",
    "wpn_shotgun",
    "wpn_smg",
    "wpn_sniperrifle",
    "wpn_tankgun",
];
/// An explosion: `Explosive` and the explosion template's `DamageKey`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blast {
    /// Its template id (`Explosion (Grenade)`).
    pub template: u32,
    /// How far it reaches, metres, its damage at its heart and at its
    /// edge, and the push at its heart.
    pub radius: f32,
    pub damage: f32,
    pub min_damage: f32,
    pub max_force: f32,
    /// Its `DamageKey` (`Explosion`, `ExplosionLarge`).
    pub key: Option<u32>,
}

impl Blast {
    /// Its damage `distance` metres from its heart: `damage` falling off
    /// to none at its edge, but never under `min_damage`; none past it
    /// ([`crate::durability::blast_damage`], from the 360 code).
    pub fn damage_at(&self, distance: f32) -> f32 {
        crate::durability::blast_damage(self.damage, self.min_damage, self.radius, distance)
    }
}

/// What a weapon fires.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ammo {
    pub template: u32,
    /// Its `Velocity` and `MaxVelocity`, m/s, and `HumanBoost`.
    pub velocity: f32,
    pub max_velocity: f32,
    pub human_boost: f32,
    /// `GravityScale`: 0 flies straight, 1 falls as anything does.
    pub gravity: f32,
    /// `Damage`, and `DamageMinimum` with the ranges it falls over.
    pub damage: f32,
    pub damage_min: f32,
    pub falloff: (f32, f32),
    /// `DamageDropoff`: the share of `Damage` lost over the falloff range
    /// (the carbine round's 0.1, the shotgun's 0.5, the tank shells' 1;
    /// 0 on the sniper and anti-material rifles' rounds and every
    /// rocket).
    pub dropoff: f32,
    /// `HeroMultiplier`: what the hero's rounds of it do, times `Damage`
    /// (1 where the template gives none).
    pub hero: f32,
    pub key: Option<u32>,
    /// What it becomes where it strikes (`SpawnOnDeath`).
    pub blast: Option<Blast>,
    /// Seconds until its timer goes (`TriggerOnTimer`), what it then
    /// becomes (`TimerResponse`), if anything, and that template's blast
    /// if it is an explosion (a grenade's, C4's).
    pub timer: Option<(f32, Option<u32>)>,
    pub timer_blast: Option<Blast>,
    /// A homing round's turn rate and widest angle, degrees.
    pub homing: Option<(f32, f32)>,
    /// Its model, if it has one of its own.
    pub model: Option<u32>,
    /// It sticks where it lands (`Sticky`).
    pub sticky: bool,
}

impl Ammo {
    /// Its speed fired by a person: `Velocity` times `HumanBoost`, no more
    /// than `MaxVelocity`.
    pub fn human_speed(&self) -> f32 {
        let v = self.velocity * self.human_boost.max(1.0);
        if self.max_velocity > 0.0 {
            v.min(self.max_velocity)
        } else {
            v
        }
    }

    /// The explosion it becomes: where it strikes, else when its timer
    /// goes.
    pub fn burst(&self) -> Option<Blast> {
        self.blast.or(self.timer_blast)
    }

    /// Its damage at `distance` metres: `Damage` up to
    /// `DamageDropoffStart`, then falling straight by its `DamageDropoff`
    /// share of `Damage` by `DamageDropoffStop`, never below
    /// `DamageMinimum` (nor above `Damage`). Inferred from the names and
    /// values (gap-list sweep): the carbine round's 6 falls to 5.4 at 50 m
    /// (it fell to its minimum 3 before), the shotgun's 15 to 7.5, a tank
    /// shell's 100 to its minimum 50 as before, and a sniper round (0)
    /// keeps its 60 (it fell to 15 at 100 m before).
    pub fn damage_at(&self, distance: f32) -> f32 {
        let (near, far) = self.falloff;
        if far <= near || distance <= near {
            return self.damage;
        }
        let t = ((distance - near) / (far - near)).clamp(0.0, 1.0);
        let floor = self.damage_min.min(self.damage);
        (self.damage * (1.0 - self.dropoff.clamp(0.0, 1.0) * t)).max(floor)
    }
}

/// How a weapon's rounds travel, as its ammunition says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flight {
    /// A bullet: it strikes along the line at once.
    Round,
    /// A rocket or shell: it flies straight and bursts where it strikes.
    Straight,
    /// It falls as it flies (a grenade launcher's round).
    Lobbed,
    /// It turns after what it was locked on.
    Homing,
}

/// The cosine of the cone round a vehicle's nose an AI's fixed vehicle
/// gun fires inside, for its weapon's [`WeaponSpec::aim_ai`]: from the
/// 360 code, the weapon's build (0x823F8D90) keeps cos(`MaxAimAngleAi`),
/// and the AI's vehicle attack (0x822C95D8, 0x822CAB34) fires only when
/// the way to its target is that close to the nose, else turns to face
/// it. A vehicle gun whose angle is 0 keeps 0 (a quarter turn either way;
/// only hand-held weapons take 0.98 instead).
pub fn ai_aim_cosine(aim_ai: f32) -> f32 {
    if aim_ai == 0.0 { 0.0 } else { aim_ai.cos() }
}

/// How much wider than its arc and tilt an AI's turret weapon may fire,
/// radians, for its weapon's [`WeaponSpec::aim_ai`]: from the 360 code,
/// 0x822EF8C8 widens the turret's limits, kept in degrees, by
/// acos(cos `MaxAimAngleAi`), in radians, so 15 degrees widens them by
/// 0.26 of a degree. The 360's mix of units, kept.
pub fn ai_turret_margin(aim_ai: f32) -> f32 {
    aim_ai.to_radians()
}

/// A gun: any weapon template with a `WeaponProjectileBase`.
#[derive(Clone, Debug, PartialEq)]
pub struct WeaponSpec {
    pub template: u32,
    pub name: String,
    /// `ModelName`, if it has one.
    pub model: Option<u32>,
    /// `Equipment`'s carry class: `Rifle`, `Pistol`, `Rocket` (name hash).
    pub class: Option<u32>,
    /// One shot a pull (`FireType` 1).
    pub semi: bool,
    pub rate_of_fire: f32,
    /// How far off an AI's aim it may fire, radians (`MaxAimAngleAi`,
    /// 15 degrees when its record gives none; the firing record's +0x18,
    /// schema 0x823EED38). From the 360 code ([`ai_aim_cosine`]).
    pub aim_ai: f32,
    pub clip: u32,
    pub reserve_clips: u32,
    pub pellets: u32,
    /// Rounds a reload puts in (0: the whole clip).
    pub rounds_per_reload: u32,
    /// `WeaponScatter`'s first field, `LowSkillScatter` (the sniper
    /// rifle's 0.1, the carbine's 1, the shotgun's 3.5). Round 17 read it as the cone; the
    /// cone is now [`Feel::cone`], from the named fields.
    pub scatter: f32,
    pub ammo: Ammo,
    /// Seconds to lock on (`HomingWeapon` `LockOnTime`), and how far.
    pub lock_on: Option<(f32, f32)>,
    /// Its whole `HomingWeapon` record, if it has one.
    pub seeker: Option<Seeker>,
    /// Rounds an ammunition pickup of it gives (its `Pickup` of type 1).
    pub pickup: Option<u32>,
    /// Its shots' sound, muzzle flash and scatter.
    pub feel: Feel,
    /// How soldiers fire it (`WeaponHint`), if its template says.
    pub hint: Option<FireHint>,
    /// What the HUD shows of it in her hands (`WeaponUI`), if its
    /// template says.
    pub ui: Option<GunUi>,
    /// How far its aim is helped onto a target (`HumanCameraModifier`),
    /// if its template says.
    pub assist: Option<AimAssist>,
    /// How it sets her camera (`HumanCameraModifier`), if its template
    /// says.
    pub camera: Option<CameraFeel>,
    /// Its scope's magnifications, least and most (`WeaponScope`'s
    /// `MinZoomLevel` and `MaxZoomLevel`, names found by hashing, gap-list
    /// sweep: the sniper rifles' 4 and 8, the anti-material rifle's 5 and
    /// 10; no other gun has a scope).
    pub scope: Option<(f32, f32)>,
}

/// A gun's aim assist, from its `HumanCameraModifier`: the named fields
/// `AimAssist`, `HorizontalAimAssistClose`, `VerticalAimAssistClose`,
/// `HorizontalAimAssistFar` and `VerticalAimAssistFar` (names from
/// samples: found by hashing, gap rounds 1 and 13).
///
/// Provenance: the values are the template's. What they mean is
/// invented, as the 360 code is not read: the four windows are taken as
/// degrees either side of the crosshair, sideways and up, within which a
/// target draws her rounds onto him ([`AimAssist::window`]), close at
/// [`AimAssist::CLOSE`] metres or nearer and far at [`AimAssist::FAR`] or
/// beyond, between the two in proportion. That the windows are close and
/// far is borne out by the values: the machine pistols' 2 degrees close
/// fall to 0 and 0.2 far, the rifles' 0.5 to 1 grow to 1.5. `AimAssist`
/// is a whole number (3 on all 24 guns that have one), taken as on when
/// not 0; a round that strays within the window of a target is then
/// drawn onto him (the alpha's `combat::assisted_aim`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AimAssist {
    pub level: u32,
    pub close: [f32; 2],
    pub far: [f32; 2],
}

impl AimAssist {
    /// Metres at which the close and the far windows hold (invented).
    pub const CLOSE: f32 = 10.0;
    pub const FAR: f32 = 100.0;

    /// The window, degrees sideways and up, at `distance` metres.
    pub fn window(&self, distance: f32) -> [f32; 2] {
        let t = ((distance - Self::CLOSE) / (Self::FAR - Self::CLOSE)).clamp(0.0, 1.0);
        [0, 1].map(|k| (self.close[k] + (self.far[k] - self.close[k]) * t).max(0.0))
    }

    /// The way to aim along unit `aim` given a target `off` that way:
    /// onto him if the assist is on and he is within the window, else
    /// `aim` as it is.
    pub fn draw(&self, aim: Vec3, off: Vec3) -> Vec3 {
        if self.level == 0 {
            return aim;
        }
        let distance = off.length();
        let Some(to) = off.try_normalize() else {
            return aim;
        };
        if aim.dot(to) <= 0.0 {
            return aim;
        }
        let side = aim.cross(Vec3::Y).normalize_or(Vec3::X);
        let up = side.cross(aim).normalize_or(Vec3::Y);
        let ahead = aim.dot(to);
        let across = [side.dot(to).atan2(ahead), up.dot(to).atan2(ahead)].map(f32::to_degrees);
        let [h, v] = self.window(distance);
        if across[0].abs() > h || across[1].abs() > v {
            return aim;
        }
        to
    }
}

/// How a gun sets her camera, from its `HumanCameraModifier` (names found
/// by hashing, gap round 1; values from samples, gap-list sweep: 44
/// records, 14 distinct). `FOV` is 1 on every one; `CrouchFov` 0.4 (the
/// Combat Rifle, the Laser Designator) to 0.9 (the Machine Pistol), the
/// sniper rifles 0.5. Read as shares of the lens' field of view standing
/// and aiming (inferred: "crouch" is the game's word for the aimed stance,
/// the pad layout's LT, and the scoped guns narrow most). The look speeds
/// (`YawSpeed`, `PitchSpeed`; `CrouchYawSpeed`, `CrouchPitchSpeed`), 0.35
/// (RPG) to 1.5 (pistols) standing and 0.25 to 0.95 aiming, read as
/// shares of her turning speed (inferred). `CameraShakeScale` is 1 on
/// every record and `RotationSpeed` a whole 3: not read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraFeel {
    /// The lens' share standing and aiming (`FOV`, `CrouchFov`).
    pub fov: f32,
    pub aim_fov: f32,
    /// Turning speed shares, sideways and up, standing and aiming.
    pub look: [f32; 2],
    pub aim_look: [f32; 2],
}

/// `HumanCameraModifier`'s aim-assist fields ([`AimAssist`]).
const ASSIST: &str = "AimAssist";
const ASSIST_CLOSE: [&str; 2] = ["HorizontalAimAssistClose", "VerticalAimAssistClose"];
const ASSIST_FAR: [&str; 2] = ["HorizontalAimAssistFar", "VerticalAimAssistFar"];

/// A gun's `WeaponUI`: the gun box's picture of it and its reticle, as
/// name hashes of their textures (the pistol's `weapons_pistol_1` and
/// `reticle_bullets`, the shotgun's `weapons_shotgun_1` and
/// `reticle_shotgun`), and whether the reticle has its crosshair (the
/// shotgun's and the launchers' have none). From samples (PC data,
/// playtest 4). The record's fields, in order: `texture`,
/// `ReticleTexture`, `ReticleHealthType`, `ReticleType`, one unnamed and
/// `bReticleCrosshair`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GunUi {
    pub icon: u32,
    pub reticle: u32,
    pub crosshair: bool,
    /// `ReticleType`, as the 360's weapon-equipped event reads it
    /// (`0x8234F458`): 1 the homing launchers' lock-on reticle, 3 the
    /// laser's, 4 none, anything else the normal one. From the data: 0
    /// for the guns, 1 for the Stinger and the mounted missile launchers,
    /// 2 for the TOW launchers, 3 for the Laser Designator.
    pub reticle_type: u32,
    /// `ReticleHealthType`: 1 the straight target-health bar under the
    /// reticle, anything else the curved one (the same event).
    pub health_type: u32,
}

/// `WeaponUI`'s reticle texture field, `ReticleTexture`.
const UI_RETICLE: u32 = 0x0e36_56d4;

/// How a soldier fires a gun, from its template's `WeaponHint` (names
/// found by hashing guesses, gap round 13; the carbine's values in
/// brackets): within what range, and in bursts of how long with what
/// pauses between, far off and close in.
///
/// Provenance: the names and values are the template's (from samples).
/// That the lengths are seconds is inferred from the names and values
/// (the carbine's 0.3 s burst is 3 rounds at its 562 a minute, the
/// minigun's 2 s 30 at 900); that a negative close length means "as far
/// off" is inferred (it is -1 or a length, never 0); the range at which
/// a target counts as close is not in this table and is invented
/// ([`FireHint::CLOSE`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FireHint {
    /// Nearest and farthest a soldier fires it at, metres (`MinFireRange`
    /// 1, `FireRange` 70).
    pub min_range: f32,
    pub range: f32,
    /// `FireAngle`'s word (0, 1 or 2: the carbine's 0, the assault
    /// rifle's 2; a whole number, so not degrees; meaning unknown, unused).
    pub angle: u32,
    /// A burst's length and the pause after it, seconds
    /// (`BurstOnLength` 0.3, `BurstOffLength` 4.5).
    pub on: f32,
    pub off: f32,
    /// The same at close range (`BurstOnCloseLength` -1,
    /// `BurstOffCloseLength` 0.75); below 0, as far off.
    pub on_close: f32,
    pub off_close: f32,
    /// How well it does against a target of no, light, medium and heavy
    /// armour, 0 to 3 (`ArmorNone`, `ArmorLight`, `ArmorMedium`,
    /// `ArmorHeavy`: four 2-bit fields of one word, names found by hashing,
    /// gap-list sweep). The carbine's 3, 1, 0, 0; the pistol's 2, 0, 0, 0;
    /// the AT rocket's 1, 3, 3, 3; the Stinger's 1, 3, 2, 0. Read as the
    /// AI's rating of the gun by its target (inferred from the names and
    /// the values: rifles against people, launchers against armour). Not
    /// used: which armour class a vehicle or a person is has not been
    /// read.
    pub against: [u32; 4],
    /// `SingleShotWeapon` (@20): one round a burst (its burst length
    /// taken as none, 0x822F0098).
    pub single: bool,
    /// The two unnamed words at @0 and @4 (fields 63e66df5, f34f9919):
    /// what kind of target and what domain the weapon is for, matched
    /// against the target's `ObjectHint` words at the same places (its
    /// @0: 2 vehicle, 3 human; @4: 2 ground, 3 air) by the AI's threat
    /// rating (0x822EC060, [`crate::ai_target::weapon_threat`]); 1 is any
    /// (rifles 3 and 1, the AT rocket 1 and 2). Meaning from the 360 code
    /// and the values; names not found.
    pub kinds: [u32; 2],
}
// Also in `WeaponHint`, named by hashing (gap-list sweep) and not read:
// `SingleShotWeapon` (read: [`FireHint::single`]; @20: 1 on the 31
// records whose `BurstOnLength` is 0 but the rocket pod's). Unnamed: @24
// (1 on the guided missiles only).

/// `WeaponHint`'s @0 and @4 fields ([`FireHint::kinds`]), by hash.
const KINDS: [u32; 2] = [0x63e6_6df5, 0xf34f_9919];

/// A launcher a soldier carries beside his gun (an `EquipmentLink` to a
/// weapon firing rockets or missiles: the AT soldiers' `AT Rocket`), and
/// how he fires it: its `WeaponHint` (when to reach for it, by the
/// [`FireHint::against`] ratings) and its round.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NpcLauncher {
    pub template: u32,
    /// Its `HomingWeapon` record: a homing launcher locks before it fires.
    pub seeker: Option<Seeker>,
    pub hint: FireHint,
    /// Shots a minute (its `RateOfFire`).
    pub rate_of_fire: f32,
    pub ammo: Ammo,
    /// Its shots' sound set and flash.
    pub feel: Feel,
}

impl NpcLauncher {
    /// Gun `w`'s, if it fires rockets flying straight or homing and its
    /// template gives a `WeaponHint`.
    pub fn of(w: &WeaponSpec) -> Option<Self> {
        matches!(w.flight(), Flight::Straight | Flight::Homing).then_some(())?;
        Some(Self {
            template: w.template,
            seeker: w.seeker,
            hint: w.hint?,
            rate_of_fire: 60.0 / w.gap().max(1e-3),
            ammo: w.ammo,
            feel: w.feel,
        })
    }
}

/// A `HomingWeapon` record (360 0x822938C0, 0x82294318).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seeker {
    /// What it locks on: 1 ground signatures, 2 air (the AT missiles 1;
    /// the Stinger, AA missiles and SAM 2); 3 the TOW's own guidance, 4
    /// the grapple's.
    pub kind: u32,
    /// The least signature of its kind a target must show (0: any).
    pub min_signature: f32,
    /// Its cone's half-angle, degrees.
    pub cone: f32,
    /// How far, metres.
    pub range: f32,
    /// Seconds to lock on (`LockOnTime`).
    pub lock_time: f32,
}

/// The [`Seeker::kind`] of a wire-guided weapon (the TOW): its missiles
/// fly at where its holder aims (360 0x82294C98).
pub const TOW: u32 = 3;

impl Seeker {
    /// Whether it locks on a target's signature (kinds 1 and 2).
    pub fn locks(&self) -> bool {
        matches!(self.kind, 1 | 2)
    }
}

/// Template `name`'s homing signatures (ground, air) among `targets`,
/// else the lowest id's of `model`'s templates that have them.
pub fn homing_signature(targets: &[HomingTarget], name: &str, model: u32) -> Option<[f32; 2]> {
    targets
        .iter()
        .find(|t| t.name.eq_ignore_ascii_case(name))
        .or_else(|| {
            targets
                .iter()
                .filter(|t| t.model == Some(model))
                .min_by_key(|t| t.template)
        })
        .map(|t| t.signature)
}

/// A template's `HomingTarget` record: what homing weapons see of it.
#[derive(Clone, Debug, PartialEq)]
pub struct HomingTarget {
    pub template: u32,
    pub name: String,
    pub model: Option<u32>,
    /// Ground and air signatures.
    pub signature: [f32; 2],
}

/// `WeaponHint`'s armour ratings ([`FireHint::against`]).
const AGAINST: [&str; 4] = ["ArmorNone", "ArmorLight", "ArmorMedium", "ArmorHeavy"];

impl FireHint {
    /// Within this of its target, metres, a soldier fires the close
    /// bursts (invented: no field read gives it).
    pub const CLOSE: f32 = 15.0;

    /// Template `id`'s, from the `WeaponHint` table.
    pub(crate) fn read(hint: &Table<'_>, id: u32) -> Option<Self> {
        let r = *hint.of_template(id).first()?;
        let f = |name: &str| hint.float(r, name_hash(name));
        Some(Self {
            min_range: f("MinFireRange")?,
            range: f("FireRange")?,
            angle: hint.word(r, name_hash("FireAngle")).unwrap_or(0),
            on: f("BurstOnLength")?,
            off: f("BurstOffLength")?,
            on_close: f("BurstOnCloseLength")?,
            off_close: f("BurstOffCloseLength")?,
            against: AGAINST.map(|n| hint.word(r, name_hash(n)).unwrap_or(0)),
            kinds: KINDS.map(|h| hint.word(r, h).unwrap_or(0)),
            single: hint.word(r, name_hash("SingleShotWeapon")) == Some(1),
        })
    }

    /// A burst's length and the pause after it, seconds, far off or
    /// `close`.
    pub fn burst(&self, close: bool) -> (f32, f32) {
        let pick = |near: f32, far: f32| if close && near >= 0.0 { near } else { far };
        (
            pick(self.on_close, self.on).max(0.0),
            pick(self.off_close, self.off).max(0.0),
        )
    }

    /// Rounds a burst fires `gap` seconds apart: as many as start within
    /// its length (the first at once), so a length of 0 is one round.
    pub fn rounds(&self, gap: f32, close: bool) -> u32 {
        let (on, _) = self.burst(close);
        (on / gap.max(1e-3) + 1e-3).floor() as u32 + 1
    }

    /// A burst's length and the pause after it, seconds, as the AI's fire
    /// control works them out for a target `dist` metres off (0x822F0098):
    /// each the close value at `MinFireRange`, the far one at `FireRange`,
    /// straight between, unless the close value is -1 (then the far one
    /// throughout). A single-shot weapon's burst is none. With no hint,
    /// 2 s and 2 s ([`FireHint::NO_HINT`]).
    pub fn cycle(hint: Option<&Self>, dist: f32) -> (f32, f32) {
        let Some(h) = hint else {
            return Self::NO_HINT;
        };
        let t = (dist - h.min_range) / (h.range - h.min_range);
        // Past 1 (or not a number) it is 1, below 0 it is 0.
        let t = if t < 1.0 { t.max(0.0) } else { 1.0 };
        let lerp = |near: f32, far: f32| {
            if near == -1.0 {
                far
            } else {
                (far - near) * t + near
            }
        };
        let on = if h.single {
            0.0
        } else {
            lerp(h.on_close, h.on)
        };
        (on, lerp(h.off_close, h.off))
    }

    /// The burst and pause of a weapon with no `WeaponHint` (0x822F0098's
    /// 2.0 for both).
    pub const NO_HINT: (f32, f32) = (2.0, 2.0);

    /// The share of the time a soldier who keeps firing spends in bursts.
    pub fn firing_share(&self, gap: f32, close: bool) -> f32 {
        let (_, off) = self.burst(close);
        let firing = self.rounds(gap, close) as f32 * gap;
        firing / (firing + off).max(1e-3)
    }
}

/// What an AI's fire control does with its trigger this frame
/// (0x822AE598's modes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// Mode 1: one round (a single-shot weapon as its cycle starts).
    One,
    /// Mode 2: held, the weapon firing at its own rate.
    Hold,
    /// Mode 3: let go.
    Off,
}

/// An AI gunner's fire cycle (its mind's +0x84 and +0x88, as `HeliAttack`
/// keeps them, 0x822B0578): each time the cycle runs out a new one
/// starts, its burst [`FireHint::cycle`]'s length, the next after the
/// burst and the pause; the pause twice as long when it is out of her
/// view (0x822F0098) and a fifth as long on an attack run. Inferred: that
/// both count down with the frame (where the 360 counts them down is not
/// traced).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FireCycle {
    pub cycle: f32,
    pub burst: f32,
}

impl FireCycle {
    /// Its timers `dt` on.
    pub fn tick(&mut self, dt: f32) {
        self.cycle -= dt;
        self.burst -= dt;
    }

    /// The trigger this frame for a weapon of `hint` at a target `dist`
    /// off, `seen` or not, on a `run` or not.
    pub fn trigger(
        &mut self,
        hint: Option<&FireHint>,
        dist: f32,
        seen: bool,
        run: bool,
    ) -> Trigger {
        let (on, mut off) = FireHint::cycle(hint, dist);
        if !seen {
            off *= 2.0;
        }
        if run {
            off *= 0.2;
        }
        let mut t = Trigger::Off;
        if self.cycle <= 0.0 {
            self.burst = on;
            self.cycle = off + on;
            if hint.is_some_and(|h| h.single) {
                t = Trigger::One;
            }
        }
        if self.burst > 0.0 {
            t = Trigger::Hold;
        }
        t
    }
}

/// An AI rider's clip in a vehicle's weapon: the rounds left in it, and
/// how a pilot and a gunner reload it.
///
/// Provenance, from the 360 code:
/// - A weapon's clip (its runtime weapon +0x30) starts full, its
///   `WeaponProjectileBase` `iClipSize` (+0x1C; 0x82285800). Every shot
///   takes one round (the shot loop 0x82283A10, however many projectiles
///   the shot throws), and a weapon with none left cannot fire
///   (0x8227F908: rounds above 0 and its shot timer run out).
/// - Weapons fired in turn (the links coupled after the first) are each
///   their own weapon, with their own clip, the shot going to the one
///   whose turn it is (0x822835D0, 0x8365622C): the group holds
///   `iClipSize` rounds for each mount ([`AiClip::size`]). Inferred: that
///   their turns share the rounds out evenly enough that the group runs
///   dry together.
/// - A reload ([`AiClip::reload`]) refills the clip at once for any weapon
///   that is no hand-held `Equipment` (0x82285548 skips the reload clip,
///   0x822853F8, 0x82285118; the runtime weapon's flag 0x1000, set by
///   0x823F8D90 only where an `Equipment` record exists). Its reserve is
///   not spent: every mounted weapon's `MaxAmmoReserve` is 0, so it
///   reloads only because its owner never runs out (0x8227F600 asks the
///   set 0x83657F28, which a thing with an `Ai` component is put in,
///   0x82400128, as `SetInfiniteAmmo` 0x823305F0 puts one). Inferred: that
///   every AI vehicle is so (0x82400128's caller is a vtable slot whose
///   class is not named).
/// - A pilot (`HeliAttack` 0x822B0578, fire mode 3 of 0x822AE598) asks
///   for a reload every frame its trigger is let go
///   ([`AiClip::pilot_off`]): a clip it has fired from is full again
///   after each burst.
/// - A gunner (0x822C95D8) reloads only once the clip is empty (its
///   slot's flag 0x200, from 0x822EDBC0): then, its own timer run out, it
///   lets go of the trigger, sets the timer to 0.5 s and asks for the
///   reload, firing nothing that frame ([`AiClip::gunner_empty`]).
///   Inferred: that the timer counts down with the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AiClip {
    /// Rounds left; `None` until first asked (full).
    pub left: Option<u32>,
    /// A gunner's reload timer, seconds.
    pub wait: f32,
}

impl AiClip {
    /// A gunner's wait after asking for a reload (0x822C95D8, 0x820D9EDC).
    pub const GUNNER_WAIT: f32 = 0.5;

    /// The rounds a full clip of mount `m` holds: its `iClipSize` for each
    /// mount it fires in turn.
    pub fn size(m: &Mounted) -> u32 {
        m.weapon.clip.max(1) * (m.nodes.len().max(1) as u32)
    }

    /// Its timer `dt` on.
    pub fn tick(&mut self, dt: f32) {
        self.wait -= dt;
    }

    /// Rounds left, of a clip of `size`.
    pub fn rounds(&mut self, size: u32) -> u32 {
        *self.left.get_or_insert(size)
    }

    /// Whether it can fire, from a clip of `size`.
    pub fn can_fire(&mut self, size: u32) -> bool {
        self.rounds(size) > 0
    }

    /// One shot fired, from a clip of `size`.
    pub fn shot(&mut self, size: u32) {
        let left = self.rounds(size);
        self.left = Some(left.saturating_sub(1));
    }

    /// Refilled to `size` (a mounted weapon's reload, at once).
    pub fn reload(&mut self, size: u32) {
        self.left = Some(size);
    }

    /// A pilot's trigger let go: its clip refilled.
    pub fn pilot_off(&mut self, size: u32) {
        self.reload(size);
    }

    /// A gunner's turn to fire from a clip of `size`: when it is empty and
    /// his timer has run out, he reloads, waits [`Self::GUNNER_WAIT`]
    /// before he may again, and fires nothing this frame. True when he may
    /// fire.
    pub fn gunner_empty(&mut self, size: u32) -> bool {
        if self.rounds(size) > 0 {
            return true;
        }
        if self.wait < 0.0 {
            self.wait = Self::GUNNER_WAIT;
            self.reload(size);
        }
        false
    }
}

/// How far a soldier's rounds stray from where he aims: widths across,
/// radians, sideways and up, each round a uniform pick within them
/// ([`SoldierSpread::stray`]), as her own rounds scatter over her gun's
/// cone. Two parts add up:
///
/// - his gun's `LowSkillScatter` (`WeaponScatter`'s first field, degrees
///   across: the sniper rifle's 0.1, the rifles' 0.8 to 1.5, the pistols'
///   3, the machine pistols' 4, the shotgun's 3.5), the same both ways;
/// - a miss of his own that grows with range and with what his
///   `AiAttributes` `Accuracy` lacks of a perfect 1, twice as wide
///   sideways as up (rounds 1-14's whole rule, scaled by
///   [`SoldierSpread::MISS`]).
///
/// Provenance: the field's name and values are the template's (from
/// samples, gap round 13); that it is a soldier's cone is inferred from
/// its name ("low skill") and that its values are cones of the same size
/// as `ScatterMin`'s; that it is degrees across as `ScatterMin` is, and
/// that the two parts add, are inferred. The miss by range and
/// `Accuracy` is invented (no field read gives it), and its scale is
/// fitted so that a VZ soldier ([`SoldierSpread::VZ_ACCURACY`]) with the
/// Assault Rifle hits at 20 m as often as under the old rule.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoldierSpread {
    pub side: f32,
    pub up: f32,
}

impl SoldierSpread {
    /// A VZ soldier's `Accuracy` (the old rule's anchor).
    pub const VZ_ACCURACY: f32 = 0.4;
    /// The old rule's miss, scaled down so his gun's cone added to it
    /// leaves the VZ soldier's Assault Rifle hitting as often at 20 m
    /// (fitted with the `soldierspread` example).
    pub const MISS: f32 = 0.75;

    /// For a soldier of `accuracy` firing a gun of `low_skill` degrees
    /// (`LowSkillScatter`) at `distance` metres.
    pub fn new(distance: f32, accuracy: f32, low_skill: f32) -> Self {
        let lack = ((1.0 - accuracy) / (1.0 - Self::VZ_ACCURACY)).clamp(0.2, 2.0);
        let miss = Self::MISS * (0.02 + distance.max(0.0) * 0.004) * lack;
        let cone = low_skill.max(0.0).to_radians();
        Self {
            side: cone + 2.0 * miss,
            up: cone + miss,
        }
    }

    /// The way a round goes for an aim along unit `aim`, given two
    /// uniform picks in [-0.5, 0.5).
    pub fn stray(&self, aim: Vec3, a: f32, b: f32) -> Vec3 {
        let side = aim.cross(Vec3::Y).normalize_or(Vec3::X);
        let up = side.cross(aim).normalize_or(Vec3::Y);
        (aim + side * (a * self.side) + up * (b * self.up)).normalize_or(aim)
    }
}

impl WeaponSpec {
    pub fn flight(&self) -> Flight {
        let a = &self.ammo;
        if a.homing.is_some() || self.lock_on.is_some() {
            Flight::Homing
        } else if a.burst().is_none() {
            Flight::Round
        } else if a.gravity > 0.0 {
            Flight::Lobbed
        } else {
            Flight::Straight
        }
    }

    /// Seconds between shots.
    pub fn gap(&self) -> f32 {
        60.0 / self.rate_of_fire.max(1.0)
    }

    /// Whether people carry it in the launcher's place: it fires rockets
    /// or lobbed rounds, not bullets.
    pub fn is_launcher(&self) -> bool {
        self.flight() != Flight::Round
    }
}

/// A thrown weapon: any template with a `WeaponThrown`.
#[derive(Clone, Debug, PartialEq)]
pub struct Throwable {
    pub template: u32,
    pub name: String,
    pub model: Option<u32>,
    /// What leaves her hand.
    pub ammo: Ammo,
    /// How many she carries (`MaxAmmoReserve`).
    pub max: u32,
    /// The throw's speed at the reticle's three pitches, and those pitches
    /// (degrees), and how far above the aim it leaves (`ThrowAngle`).
    pub speeds: [f32; 3],
    pub pitches: [f32; 3],
    pub throw_angle: f32,
    /// `bCook`: it can be held with its fuse burning.
    pub cook: bool,
    /// What sets it off (`TriggerTemplate`: C4's detonator).
    pub detonator: Option<u32>,
    /// What it becomes once it has lain long enough (the proximity mine's
    /// planted mine), and how near something must come to set that off.
    pub planted: Option<(Ammo, f32)>,
}

impl Throwable {
    /// Its throw speed with the reticle at `pitch` degrees.
    pub fn speed_at(&self, pitch: f32) -> f32 {
        let [p0, p1, p2] = self.pitches;
        let [s0, s1, s2] = self.speeds;
        if pitch <= p1 {
            let t = if p1 > p0 {
                (pitch - p0) / (p1 - p0)
            } else {
                1.0
            };
            s0 + (s1 - s0) * t.clamp(0.0, 1.0)
        } else {
            let t = if p2 > p1 {
                (pitch - p1) / (p2 - p1)
            } else {
                1.0
            };
            s1 + (s2 - s1) * t.clamp(0.0, 1.0)
        }
    }
}

/// A weapon a vehicle mounts: its link name, template and hardpoint node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mount {
    pub link: u32,
    pub weapon: u32,
    pub hardpoint: u32,
}

/// What a vehicle fights with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Armament {
    pub mounts: Vec<Mount>,
    /// Link names that fire in turn: (fired, next).
    pub couplings: Vec<(u32, u32)>,
    /// Per seat (its `SeatLink` name hash: `seat_driver`): the link names
    /// its rider fires, first the `Priority` 0 one.
    pub seats: Vec<(u32, Vec<u32>)>,
}

impl Armament {
    /// The link names seat `seat` fires, first the first.
    pub fn of_seat(&self, seat: u32) -> &[u32] {
        self.seats
            .iter()
            .find(|(s, _)| *s == seat)
            .map_or(&[], |(_, l)| l.as_slice())
    }

    /// The mounts fired as link `link`, in turn: it, then those its
    /// couplings chain after it, each once.
    pub fn group(&self, link: u32) -> Vec<Mount> {
        let mut names = vec![link];
        let mut at = link;
        while let Some(&(_, next)) = self.couplings.iter().find(|(from, _)| *from == at) {
            if names.contains(&next) {
                break;
            }
            names.push(next);
            at = next;
        }
        let mut out = Vec::new();
        for n in names {
            // A vehicle may list a mount twice (its base and full
            // templates' links): each hardpoint once.
            for m in self.mounts.iter().filter(|m| m.link == n) {
                if !out.contains(m) {
                    out.push(*m);
                }
            }
        }
        out
    }
}

/// A weapon a vehicle's rider fires.
#[derive(Clone, Debug, PartialEq)]
pub struct Mounted {
    /// The seat whose rider fires it (its `SeatLink` name hash), and its
    /// `Priority` there (0 the first: the fire button; 1 the next).
    pub seat: u32,
    pub priority: usize,
    pub weapon: WeaponSpec,
    /// The model nodes it fires from in turn (its link and those coupled
    /// after it), where the model has them.
    pub nodes: Vec<Option<usize>>,
}

/// What armament `arm` has each seat fire, on a model whose nodes' names
/// hash to `nodes`.
pub fn mounted(arm: &Armament, arms: &Arms, nodes: &[u32]) -> Vec<Mounted> {
    let mut out = Vec::new();
    for (seat, links) in &arm.seats {
        for (priority, &link) in links.iter().enumerate() {
            let group = arm.group(link);
            let Some(weapon) = group.first().and_then(|m| arms.weapons.get(&m.weapon)) else {
                continue;
            };
            out.push(Mounted {
                seat: *seat,
                priority,
                weapon: weapon.clone(),
                nodes: group
                    .iter()
                    .map(|m| nodes.iter().position(|&h| h == m.hardpoint))
                    .collect(),
            });
        }
    }
    out
}

/// What a vehicle's machine gun fires as ([`machine_gun`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MachineGun<'a> {
    /// A gun its template mounts: its own rate, round, flash and sound.
    Mounted(&'a WeaponSpec),
    /// Its template mounts weapons, none of them a gun for this: it has
    /// no such gun, and fires nothing.
    Unmounted,
    /// No template armament was read: the alpha's own gun stands in.
    StandIn,
}

/// What a vehicle armed with `armed` fires as its machine gun (beside a
/// tank's cannon, or a turret gun no seat's mount fires): the first
/// mount whose rounds are bullets ([`Flight::Round`]: the AMX-30's
/// `Mounted Heavy MG (Tank) (VZ)`). A vehicle whose armament mounts none
/// has no machine gun: the AMX-30 AA and the PGZ95 mount missiles and
/// their own gun, nothing beside it.
///
/// Provenance: the mounts are the template's (from samples); that a
/// template with no bullet gun has no machine gun to fire is inferred
/// from the mounts (the game's seat logic is not decoded).
pub fn machine_gun(armed: &[Mounted]) -> MachineGun<'_> {
    match armed.iter().find(|m| m.weapon.flight() == Flight::Round) {
        Some(m) => MachineGun::Mounted(&m.weapon),
        None if armed.is_empty() => MachineGun::StandIn,
        None => MachineGun::Unmounted,
    }
}

/// Every weapon the database names.
#[derive(Clone, Debug, Default)]
pub struct Arms {
    pub weapons: HashMap<u32, WeaponSpec>,
    /// The homing weapons' lock cones, degrees, by weapon template
    /// (`HomingWeapon`'s third field: the Stinger's 10).
    pub lock_cones: HashMap<u32, f32>,
    pub throwables: HashMap<u32, Throwable>,
    /// By vehicle template.
    pub armaments: HashMap<u32, Armament>,
    /// Every template's `HomingTarget`, by template.
    pub homing_targets: Vec<HomingTarget>,
}

impl Arms {
    /// Template `name`'s homing signatures (ground, air), else the lowest
    /// id's of `model`'s templates that have them (as the vehicles' own
    /// tuning is found).
    pub fn signature_for(&self, name: &str, model: u32) -> Option<[f32; 2]> {
        homing_signature(&self.homing_targets, name, model)
    }

    /// The weapon template named `name`, whatever its case: of several so
    /// named, the lowest id's.
    pub fn named(&self, name: &str) -> Option<&WeaponSpec> {
        self.weapons
            .values()
            .filter(|w| w.name.eq_ignore_ascii_case(name))
            .min_by_key(|w| w.template)
    }

    /// The thrown weapon named `name`, whatever its case.
    pub fn thrown(&self, name: &str) -> Option<&Throwable> {
        self.throwables
            .values()
            .filter(|w| w.name.eq_ignore_ascii_case(name))
            .min_by_key(|w| w.template)
    }

    /// Every hand weapon's model (guns with an `Equipment`, and thrown
    /// weapons), each once, lowest first.
    pub fn hand_models(&self) -> Vec<u32> {
        let mut m: Vec<u32> = self
            .weapons
            .values()
            .filter(|w| w.class.is_some())
            .filter_map(|w| w.model)
            .chain(self.throwables.values().filter_map(|t| t.model))
            .chain(self.throwables.values().filter_map(|t| t.ammo.model))
            .collect();
        m.sort_unstable();
        m.dedup();
        m
    }
}

/// The components weapons are read from.
struct Parts<'a> {
    velocity: Table<'a>,
    physics: Table<'a>,
    key: Table<'a>,
    spawn: Table<'a>,
    timer: Table<'a>,
    trigger: Table<'a>,
    explosive: Table<'a>,
    homing: Table<'a>,
    sticky: Table<'a>,
    stimulus: Table<'a>,
    links: Table<'a>,
    models: HashMap<u32, u32>,
}

impl Parts<'_> {
    fn first(t: &Table<'_>, id: u32, field: u32) -> Option<u32> {
        t.word(t.of_template(id).first()?, field)
    }

    fn float(t: &Table<'_>, id: u32, field: u32) -> Option<f32> {
        t.float(t.of_template(id).first()?, field)
    }

    fn damage_key(&self, id: u32) -> Option<u32> {
        Self::first(&self.key, id, name_hash("DamageKey")).filter(|&k| k != 0)
    }

    /// Explosion template `id`'s own `Explosive`.
    fn own_blast(&self, id: u32) -> Option<Blast> {
        let f = |name: &str| Self::float(&self.explosive, id, name_hash(name));
        Some(Blast {
            template: id,
            radius: f("Radius")?,
            damage: f("Damage").unwrap_or(0.0),
            min_damage: f("MinDamage").unwrap_or(0.0),
            max_force: f("MaxForce").unwrap_or(0.0),
            key: self.damage_key(id),
        })
    }

    /// Explosion template `id` as one blast: its own `Explosive` and those
    /// of the explosions it links (`PhysicalLink`: C4's primary 1 m blast
    /// links its secondary; the fuel-air rocket's has none of its own),
    /// the widest reach and hardest heart of them. A simplification: the
    /// game sets each off.
    fn blast(&self, id: u32) -> Option<Blast> {
        let linked = self
            .links
            .of_template(id)
            .iter()
            .filter_map(|r| self.links.word(r, LINK))
            .filter_map(|t| self.own_blast(t));
        let all: Vec<Blast> = self.own_blast(id).into_iter().chain(linked).collect();
        let widest = all
            .iter()
            .copied()
            .max_by(|a, b| a.radius.total_cmp(&b.radius))?;
        Some(Blast {
            template: id,
            damage: all.iter().map(|b| b.damage).fold(widest.damage, f32::max),
            key: self.damage_key(id).or(widest.key),
            ..widest
        })
    }

    fn ammo(&self, id: u32) -> Ammo {
        let v = |name: &str| Self::float(&self.velocity, id, name_hash(name));
        let p = |field: u32| Self::float(&self.physics, id, field);
        let spawned = |t: &Table<'_>| Self::first(t, id, SPAWNS).filter(|&s| s != 0);
        let blast = spawned(&self.spawn).and_then(|s| self.blast(s));
        let timer = Self::float(&self.trigger, id, name_hash("time"))
            .filter(|&t| t > 0.0)
            .map(|t| (t, spawned(&self.timer)));
        let timer_blast = timer.and_then(|(_, then)| self.blast(then?));
        let homing = (|| {
            let r = *self.homing.of_template(id).first()?;
            Some((
                self.homing.float(r, TURN_SPEED)?,
                self.homing.float(r, MAX_ANGLE)?,
            ))
        })();
        Ammo {
            template: id,
            velocity: v("Velocity").unwrap_or(0.0),
            max_velocity: v("MaxVelocity").unwrap_or(0.0),
            human_boost: Self::float(&self.velocity, id, HUMAN_BOOST).unwrap_or(1.0),
            gravity: p(name_hash("GravityScale")).unwrap_or(0.0),
            damage: p(name_hash("Damage")).unwrap_or(0.0),
            damage_min: p(DAMAGE_MINIMUM).unwrap_or(0.0),
            falloff: (
                p(FALLOFF_NEAR).unwrap_or(0.0),
                p(FALLOFF_FAR).unwrap_or(0.0),
            ),
            dropoff: p(DAMAGE_DROPOFF).unwrap_or(1.0),
            hero: p(HERO_MULTIPLIER).filter(|m| *m > 0.0).unwrap_or(1.0),
            key: self.damage_key(id),
            blast,
            timer,
            timer_blast,
            homing,
            model: self.models.get(&id).copied().filter(|&m| m != 0),
            sticky: !self.sticky.of_template(id).is_empty(),
        }
    }
}

/// Reads every weapon, thrown weapon and vehicle armament.
pub fn read(db: &Layer<'_>) -> Arms {
    let names = db.names();
    let parts = Parts {
        velocity: Table::new(db, "ControllerVelocity"),
        physics: Table::new(db, "ProjectilePhysics"),
        key: Table::new(db, "DamageKey"),
        spawn: Table::new(db, "SpawnOnDeath"),
        timer: Table::new(db, "TimerResponse"),
        trigger: Table::new(db, "TriggerOnTimer"),
        explosive: Table::new(db, "Explosive"),
        homing: Table::new(db, "HomingProjectile"),
        sticky: Table::new(db, "Sticky"),
        stimulus: Table::new(db, "Stimulus"),
        links: Table::new(db, "PhysicalLink"),
        models: db.hashes("ModelName"),
    };
    let base = Table::new(db, "WeaponProjectileBase");
    let thrown = Table::new(db, "WeaponThrown");
    let equipment = Table::new(db, "Equipment");
    let lock = Table::new(db, "HomingWeapon");
    let seeker_of = |id: u32| {
        let l = *lock.of_template(id).first()?;
        Some(Seeker {
            kind: lock.word(l, SEEK_KIND)?,
            min_signature: lock.float(l, SEEK_SIGNATURE).unwrap_or(0.0),
            cone: lock.float(l, SEEK_CONE)?,
            range: lock.float(l, LOCK_RANGE)?,
            lock_time: lock.float(l, name_hash("LockOnTime"))?,
        })
    };
    let target = Table::new(db, "HomingTarget");
    let pickup = Table::new(db, "Pickup");
    let scatter = Table::new(db, "WeaponScatter");
    let hint = Table::new(db, "WeaponHint");
    let effects = Table::new(db, "WeaponEffects");
    let sound = Table::new(db, "SoundEffect");
    let ui = Table::new(db, "WeaponUI");
    let camera = Table::new(db, "HumanCameraModifier");
    let scope = Table::new(db, "WeaponScope");
    let scope_of = |id: u32| {
        let f = |name: &str| Parts::float(&scope, id, name_hash(name)).filter(|v| *v > 0.0);
        Some((f("MinZoomLevel")?, f("MaxZoomLevel")?))
    };
    let assist_of = |id: u32| {
        let f = |name: &str| Parts::float(&camera, id, name_hash(name));
        Some(AimAssist {
            level: Parts::first(&camera, id, name_hash(ASSIST))?,
            close: [f(ASSIST_CLOSE[0])?, f(ASSIST_CLOSE[1])?],
            far: [f(ASSIST_FAR[0])?, f(ASSIST_FAR[1])?],
        })
    };
    let camera_of = |id: u32| {
        let f = |name: &str| Parts::float(&camera, id, name_hash(name)).filter(|v| *v > 0.0);
        Some(CameraFeel {
            fov: f("FOV")?,
            aim_fov: f("CrouchFov")?,
            look: [f("YawSpeed")?, f("PitchSpeed")?],
            aim_look: [f("CrouchYawSpeed")?, f("CrouchPitchSpeed")?],
        })
    };
    let name_of = |id: u32| names.get(&id).map_or(String::new(), |n| (*n).to_owned());
    let model_of = |id: u32| parts.models.get(&id).copied().filter(|&m| m != 0);
    let class_of = |id: u32| Parts::first(&equipment, id, CLASS).filter(|&c| c != 0);
    // An ammunition pickup's amount: its `Pickup` of type 1 refilling
    // `ammo`.
    let refill = |id: u32, ammo: u32| {
        pickup.of_template(id).iter().find_map(|r| {
            (pickup.word(r, name_hash("Type")) == Some(1)
                && pickup.word(r, PICKUP_REFILL) == Some(ammo))
            .then(|| pickup.word(r, PICKUP_AMOUNT))
            .flatten()
        })
    };

    let weapons = base
        .templates()
        .filter_map(|id| {
            let r = *base.of_template(id).first()?;
            let w = |name: &str| base.word(r, name_hash(name));
            let ammo_id = w("AmmoTemplate").filter(|&a| a != 0)?;
            let lock_on = (|| {
                let l = *lock.of_template(id).first()?;
                Some((
                    lock.float(l, name_hash("LockOnTime"))?,
                    lock.float(l, LOCK_RANGE).unwrap_or(0.0),
                ))
            })();
            Some((
                id,
                WeaponSpec {
                    template: id,
                    name: name_of(id),
                    model: model_of(id),
                    class: class_of(id),
                    semi: w("FireType") == Some(1),
                    rate_of_fire: base.float(r, name_hash("RateOfFire")).unwrap_or(60.0),
                    aim_ai: base
                        .float(r, name_hash("MaxAimAngleAi"))
                        .unwrap_or(15.0)
                        .to_radians(),
                    clip: w("iClipSize").map_or(1, |c| c & 0xffff).max(1),
                    reserve_clips: w("MaxAmmoReserve").map_or(0, |c| c & 0xff),
                    pellets: w("iBulletsPerShot").map_or(1, |c| c & 0xff).max(1),
                    rounds_per_reload: w("iRoundsPerReload").map_or(0, |c| c & 0xff),
                    scatter: Parts::float(&scatter, id, SCATTER).unwrap_or(0.0),
                    ammo: parts.ammo(ammo_id),
                    lock_on,
                    seeker: seeker_of(id),
                    pickup: refill(id, ammo_id),
                    feel: Feel {
                        sound_bank: Parts::first(&sound, id, SOUND_BANK).filter(|&b| b != 0),
                        sound_set: Parts::first(&sound, id, name_hash("SoundSet"))
                            .filter(|&s| s != 0),
                        flash: Parts::first(&effects, id, name_hash("MuzzleFlashTemplate"))
                            .filter(|&f| f != 0),
                        shell: Parts::first(&effects, id, SHELL).filter(|&s| s != 0),
                        shell_hardpoint: Parts::first(&effects, id, SHELL_HARDPOINT)
                            .filter(|&h| h != 0),
                        scatter: SCATTER_FIELDS
                            .map(|f| Parts::float(&scatter, id, f).unwrap_or(0.0)),
                    },
                    hint: FireHint::read(&hint, id),
                    ui: (|| {
                        Some(GunUi {
                            icon: Parts::first(&ui, id, name_hash("texture"))
                                .filter(|&t| t != 0)?,
                            reticle: Parts::first(&ui, id, UI_RETICLE).filter(|&t| t != 0)?,
                            crosshair: Parts::first(&ui, id, name_hash("bReticleCrosshair"))
                                .is_some_and(|c| c & 0xff != 0),
                            reticle_type: Parts::first(&ui, id, name_hash("ReticleType"))
                                .unwrap_or(0),
                            health_type: Parts::first(&ui, id, name_hash("ReticleHealthType"))
                                .unwrap_or(0),
                        })
                    })(),
                    assist: assist_of(id),
                    camera: camera_of(id),
                    scope: scope_of(id),
                },
            ))
        })
        .collect();

    let throwables = thrown
        .templates()
        .filter_map(|id| {
            let r = *thrown.of_template(id).first()?;
            let ammo_id = thrown
                .word(r, name_hash("AmmoTemplate"))
                .filter(|&a| a != 0)?;
            let ammo = parts.ammo(ammo_id);
            // A projectile that becomes another after its timer (the
            // proximity mine's planted mine) is planted, not set off.
            let planted = ammo
                .timer
                .and_then(|(_, then)| then)
                .filter(|&t| parts.own_blast(t).is_none() && parts.blast(t).is_none())
                .map(|t| {
                    let reach = Parts::float(&parts.stimulus, t, STIMULUS_REACH).unwrap_or(0.0);
                    (parts.ammo(t), reach)
                });
            let f = |field: u32| thrown.float(r, field).unwrap_or(0.0);
            Some((
                id,
                Throwable {
                    template: id,
                    name: name_of(id),
                    model: model_of(id),
                    ammo,
                    max: thrown.word(r, name_hash("MaxAmmoReserve")).unwrap_or(0) & 0xff,
                    speeds: THROW_SPEEDS.map(f),
                    pitches: RETICLE_PITCHES.map(f),
                    throw_angle: thrown.float(r, name_hash("ThrowAngle")).unwrap_or(0.0),
                    cook: thrown.word(r, name_hash("bCook")) == Some(1),
                    detonator: thrown
                        .word(r, name_hash("TriggerTemplate"))
                        .filter(|&d| d != 0),
                    planted,
                },
            ))
        })
        .collect();

    let links = Table::new(db, "PhysicalLink");
    let couplings = Table::new(db, "WeaponCoupling");
    let seat_links = Table::new(db, "SeatLink");
    let controllers = Table::new(db, "ControllerWeapon");
    let armed: &HashMap<u32, WeaponSpec> = &weapons;
    let armaments = links
        .templates()
        .filter_map(|id| {
            let mounts: Vec<Mount> = links
                .of_template(id)
                .iter()
                .filter_map(|r| {
                    let weapon = links.word(r, LINK)?;
                    armed.contains_key(&weapon).then_some(Mount {
                        link: links.word(r, name_hash("Name")).unwrap_or(0),
                        weapon,
                        hardpoint: links.word(r, name_hash("Hardpoint")).unwrap_or(0),
                    })
                })
                .collect();
            if mounts.is_empty() {
                return None;
            }
            let couplings = couplings
                .of_template(id)
                .iter()
                .filter_map(|r| {
                    Some((
                        couplings.word(r, COUPLE_FROM)?,
                        couplings.word(r, COUPLE_TO)?,
                    ))
                })
                .collect();
            let mut seats: Vec<(u32, Vec<u32>)> = Vec::new();
            for r in seat_links.of_template(id) {
                let (Some(seat), Some(name)) = (
                    seat_links.word(r, LINK),
                    seat_links.word(r, name_hash("Name")),
                ) else {
                    continue;
                };
                if seats.iter().any(|(s, _)| *s == name) {
                    continue;
                }
                let mut fired: Vec<(u32, u32)> = controllers
                    .of_template(seat)
                    .iter()
                    .filter_map(|c| {
                        Some((
                            controllers.word(c, name_hash("Priority")).unwrap_or(0),
                            controllers.word(c, SEAT_WEAPON)?,
                        ))
                    })
                    .collect();
                fired.sort_by_key(|&(p, _)| p);
                if !fired.is_empty() {
                    seats.push((name, fired.into_iter().map(|(_, l)| l).collect()));
                }
            }
            Some((
                id,
                Armament {
                    mounts,
                    couplings,
                    seats,
                },
            ))
        })
        .collect();

    let mut homing_targets: Vec<HomingTarget> = target
        .templates()
        .filter_map(|id| {
            let r = *target.of_template(id).first()?;
            Some(HomingTarget {
                template: id,
                name: name_of(id),
                model: model_of(id),
                signature: [
                    target.float(r, SIGNATURE_GROUND).unwrap_or(0.0),
                    target.float(r, SIGNATURE_AIR).unwrap_or(0.0),
                ],
            })
        })
        .collect();
    homing_targets.sort_by_key(|t| t.template);

    let lock_cones = lock
        .templates()
        .filter_map(|id| {
            let l = *lock.of_template(id).first()?;
            Some((id, lock.float(l, LOCK_CONE)?))
        })
        .collect();
    Arms {
        weapons,
        lock_cones,
        throwables,
        armaments,
        homing_targets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A vehicle's homing signatures are its own template's, else the
    // lowest template's of its model; only kinds 1 and 2 lock.
    #[test]
    fn homing_signatures_by_name_then_model() {
        let t = |template, name: &str, model, signature| HomingTarget {
            template,
            name: name.to_owned(),
            model: Some(model),
            signature,
        };
        let targets = [
            t(5, "Helicopter", 9, [0.0, 1.0]),
            t(3, "Gunship", 9, [0.0, 2.0]),
            t(8, "CRX", 4, [1.0, 0.0]),
        ];
        assert_eq!(homing_signature(&targets, "crx", 1), Some([1.0, 0.0]));
        assert_eq!(homing_signature(&targets, "Other", 9), Some([0.0, 2.0]));
        assert_eq!(homing_signature(&targets, "Other", 2), None);
        let s = |kind| Seeker {
            kind,
            min_signature: 1.0,
            cone: 10.0,
            range: 400.0,
            lock_time: 2.0,
        };
        assert!(s(1).locks() && s(2).locks() && !s(3).locks() && !s(4).locks());
    }

    // An AI's fixed vehicle gun fires inside cos(MaxAimAngleAi) of its
    // nose (a quarter turn for 0); its turret's limits widen by that
    // angle's radians read as degrees (0x823F8D90, 0x822EF8C8).
    #[test]
    fn ai_vehicle_guns_fire_inside_their_cone_and_turret_margin() {
        let deg15 = 15f32.to_radians();
        assert!((ai_aim_cosine(deg15) - 0.965_926).abs() < 1e-5);
        assert_eq!(ai_aim_cosine(0.0), 0.0);
        assert!((ai_turret_margin(deg15) - 0.261_799f32.to_radians()).abs() < 1e-7);
    }

    #[test]
    fn aim_assist_draws_her_aim_onto_a_target_within_its_window() {
        // The fields' names hash as the survey found them (gap round 13).
        let found = [0xbcf5_4d6b, 0x4bba_9acc, 0x8b11_d32d, 0xee17_304a];
        for name in ASSIST_CLOSE.iter().chain(&ASSIST_FAR) {
            assert!(found.contains(&name_hash(name)), "{name}");
        }
        // The carbine's: 0.65 and 1 degrees close, 1.5 far.
        let a = AimAssist {
            level: 3,
            close: [0.65, 1.0],
            far: [1.5, 1.5],
        };
        assert_eq!(a.window(5.0), [0.65, 1.0]);
        assert_eq!(a.window(200.0), [1.5, 1.5]);
        let mid = a.window(55.0);
        assert!((mid[0] - 1.075).abs() < 1e-4 && (mid[1] - 1.25).abs() < 1e-4);
        // A soldier 1 degree beside the crosshair at 100 m: onto him.
        let aim = Vec3::Z;
        let him = Vec3::new(1f32.to_radians().tan(), 0.0, 1.0) * 100.0;
        let drawn = a.draw(aim, him);
        assert!(drawn.angle_between(him.normalize()) < 1e-4);
        // 2 degrees off: past the window, the aim stays.
        let wide = Vec3::new(2f32.to_radians().tan(), 0.0, 1.0) * 100.0;
        assert_eq!(a.draw(aim, wide), aim);
        // Close, 1 degree is past the 0.65; behind her, nothing.
        assert_eq!(a.draw(aim, him * 0.05), aim);
        assert_eq!(a.draw(aim, -him), aim);
        // Off (level 0): nothing.
        let off = AimAssist { level: 0, ..a };
        assert_eq!(off.draw(aim, him), aim);
    }

    // The fire control's burst and pause run from the close values at
    // `MinFireRange` to the far ones at `FireRange`; a -1 close value
    // keeps the far one; a single-shot weapon has no burst; no hint, 2 s
    // and 2 s (0x822F0098).
    #[test]
    fn the_fire_cycle_runs_from_close_to_far() {
        let h = FireHint {
            min_range: 10.0,
            range: 50.0,
            angle: 0,
            on: 1.0,
            off: 4.0,
            on_close: -1.0,
            off_close: 2.0,
            against: [0; 4],
            kinds: [1, 1],
            single: false,
        };
        assert_eq!(FireHint::cycle(Some(&h), 0.0), (1.0, 2.0));
        assert_eq!(FireHint::cycle(Some(&h), 30.0), (1.0, 3.0));
        assert_eq!(FireHint::cycle(Some(&h), 90.0), (1.0, 4.0));
        let h = FireHint {
            on_close: 3.0,
            single: true,
            ..h
        };
        assert_eq!(FireHint::cycle(Some(&h), 30.0), (0.0, 3.0));
        let h = FireHint { single: false, ..h };
        assert_eq!(FireHint::cycle(Some(&h), 30.0), (2.0, 3.0));
        assert_eq!(FireHint::cycle(None, 30.0), (2.0, 2.0));
    }

    // A cycle holds the trigger for its burst, lets go for its pause
    // (twice as long out of view, a fifth on a run), and a single-shot
    // weapon fires once a cycle (0x822B0578, 0x822F0098).
    #[test]
    fn a_fire_cycle_holds_then_lets_go() {
        let h = FireHint {
            min_range: 0.0,
            range: 100.0,
            angle: 0,
            on: 1.0,
            off: 2.0,
            on_close: -1.0,
            off_close: -1.0,
            against: [0; 4],
            kinds: [1, 1],
            single: false,
        };
        let held = |seen: bool, run: bool, h: &FireHint| {
            let mut c = FireCycle::default();
            let (mut hold, mut one) = (0, 0);
            for _ in 0..1200 {
                c.tick(0.01);
                match c.trigger(Some(h), 50.0, seen, run) {
                    Trigger::Hold => hold += 1,
                    Trigger::One => one += 1,
                    Trigger::Off => {}
                }
            }
            (hold, one)
        };
        // 12 s: four 3 s cycles, each 1 s held.
        let (hold, one) = held(true, false, &h);
        assert!((396..=404).contains(&hold) && one == 0, "{hold}");
        // Out of view: 5 s cycles.
        let (hold, _) = held(false, false, &h);
        assert!((240..=306).contains(&hold), "{hold}");
        // On a run: 1.4 s cycles.
        let (hold, _) = held(true, true, &h);
        assert!((800..=900).contains(&hold), "{hold}");
        let single = FireHint { single: true, ..h };
        assert_eq!(held(true, false, &single), (0, 6));
    }

    #[test]
    fn rounds_stray_evenly_over_the_round_cone() {
        // A 2-degree cone: no round more than 1 degree out, corners
        // included (the old square reached 1.41), and half of them
        // within 0.71 degrees (half the disc's area).
        let aim = Vec3::Z;
        let off = |a: f32, b: f32| {
            stray_in_cone(aim, 2.0, a, b)
                .angle_between(aim)
                .to_degrees()
        };
        let mut inside = 0;
        let n = 40;
        for i in 0..n {
            for j in 0..n {
                let (a, b) = (i as f32 / n as f32 - 0.5, j as f32 / n as f32 - 0.5);
                let d = off(a, b);
                assert!(d <= 1.0 + 1e-3, "{d}");
                inside += usize::from(d < 0.5f32.sqrt());
            }
        }
        let share = inside as f32 / (n * n) as f32;
        assert!((share - 0.5).abs() < 0.03, "{share}");
        // The corner pick: at the rim, not past it.
        assert!((off(0.4999, -0.5) - 1.0).abs() < 1e-2);
        // No cone: along the aim.
        assert_eq!(stray_in_cone(aim, 0.0, 0.3, 0.2), aim);
    }

    // The carbine's `WeaponScatter` as its template has it: 1, 0, 1, 1.5,
    // then `ScatterMin` 1, `ScatterMax` 2, `ScatterPerShot` 0.2.
    #[test]
    fn a_guns_cone_grows_per_shot_to_its_most_and_settles() {
        assert_eq!(SCATTER_FIELDS[4], name_hash("ScatterMin"));
        assert_eq!(SCATTER_FIELDS[5], name_hash("ScatterMax"));
        assert_eq!(SCATTER_FIELDS[6], name_hash("ScatterPerShot"));
        assert_eq!(SHELL_HARDPOINT, name_hash("ShellEjectHardpoint"));
        assert_eq!(SHELL, name_hash("ShellEjectTemplate"));
        // The carbine's (as read on a legally installed copy).
        assert_eq!(name_hash("hp_shelleject"), 0x3d10_6439);
        let carbine = Feel {
            scatter: [1.0, 0.0, 1.0, 1.5, 1.0, 2.0, 0.2],
            ..Feel::default()
        };
        let mut bloom = Bloom::default();
        assert_eq!(carbine.cone(bloom.added), 1.0);
        // Held at 562 a minute: it grows, settling not while it fires.
        for _ in 0..3 {
            bloom.shot(&carbine);
            bloom.settle(&carbine, 60.0 / 562.0);
        }
        assert!((carbine.cone(bloom.added) - 1.6).abs() < 1e-5);
        for _ in 0..10 {
            bloom.shot(&carbine);
        }
        assert_eq!(carbine.cone(bloom.added), 2.0);
        // Let go: after a rest it settles, from the most in a second.
        bloom.settle(&carbine, Bloom::REST);
        bloom.settle(&carbine, 0.25);
        assert!((carbine.cone(bloom.added) - 1.5).abs() < 1e-5);
        bloom.settle(&carbine, Bloom::SETTLE);
        assert_eq!(carbine.cone(bloom.added), 1.0);
        // The sniper rifle: dead on first, 1 more a shot, at most 3.
        let sniper = Feel {
            scatter: [0.1, 0.0, 0.0, 2.0, 0.0, 3.0, 1.0],
            ..Feel::default()
        };
        let mut bloom = Bloom::default();
        assert_eq!(sniper.cone(bloom.added), 0.0);
        bloom.shot(&sniper);
        assert_eq!(sniper.cone(bloom.added), 1.0);
    }

    // The gun banks' names hash to the banks the templates' `SoundEffect`
    // name (read on a legally installed copy: the carbine's `50333181`,
    // the pistol's `4a1f94d0`, the shotgun's `08e43a91`), and the flash
    // field to its key.
    #[test]
    fn gun_banks_hash_to_the_templates_banks() {
        assert_eq!(name_hash("wpn_combatrifle"), 0x5033_3181);
        assert_eq!(name_hash("wpn_pistol"), 0x4a1f_94d0);
        assert_eq!(name_hash("wpn_shotgun"), 0x08e4_3a91);
        assert!(GUN_BANKS.contains(&"wpn_combatrifle"));
        assert_eq!(name_hash("MuzzleFlashTemplate"), 0x87ac_b4be);
    }

    // The names that hash to the weapon fields' keys (survey `arsenal`
    // and the weapons thread's brute force).
    #[test]
    fn the_names_hash_to_the_fields_keys() {
        for (name, key) in [
            ("DamageMinimum", DAMAGE_MINIMUM),
            ("HeroMultiplier", HERO_MULTIPLIER),
            ("HumanBoost", HUMAN_BOOST),
            ("TurnSpeed", TURN_SPEED),
            ("MaxAngle", MAX_ANGLE),
            ("ReticlePitchMiddle", RETICLE_PITCHES[1]),
            ("TriggerTemplate", 0xcb06_b821),
            ("bCook", 0xce01_7ad9),
            ("ThrowAngle", 0xd2b5_7e44),
        ] {
            assert_eq!(name_hash(name), key, "{name}");
        }
    }

    fn ammo(damage: f32, min: f32, falloff: (f32, f32)) -> Ammo {
        Ammo {
            template: 1,
            velocity: 42.5,
            max_velocity: 300.0,
            human_boost: 8.0,
            gravity: 0.0,
            damage,
            damage_min: min,
            falloff,
            dropoff: 1.0,
            hero: 1.0,
            key: None,
            blast: None,
            timer: None,
            timer_blast: None,
            homing: None,
            model: None,
            sticky: false,
        }
    }

    fn mount(template: u32, ammo: Ammo, homing: bool) -> Mounted {
        Mounted {
            seat: 1,
            priority: 0,
            weapon: WeaponSpec {
                template,
                name: format!("gun {template}"),
                model: None,
                class: None,
                semi: false,
                rate_of_fire: 400.0,
                aim_ai: 15f32.to_radians(),
                clip: 100,
                reserve_clips: 0,
                pellets: 1,
                rounds_per_reload: 0,
                scatter: 0.0,
                ammo,
                lock_on: homing.then_some((1.0, 500.0)),
                seeker: None,
                pickup: None,
                feel: Default::default(),
                hint: None,
                ui: None,
                assist: None,
                camera: None,
                scope: None,
            },
            nodes: vec![Some(0)],
        }
    }

    // The AMX-30: a heavy MG beside its gun; the PGZ95: its own bursting
    // gun and missiles, no machine gun.
    #[test]
    fn a_vehicles_machine_gun_is_its_mounted_bullet_gun_or_none() {
        let blast = Blast {
            template: 9,
            radius: 2.0,
            damage: 40.0,
            min_damage: 0.0,
            max_force: 0.0,
            key: None,
        };
        let shell = Ammo {
            blast: Some(blast),
            ..ammo(40.0, 40.0, (0.0, 0.0))
        };
        let bullet = mount(1, ammo(6.0, 3.0, (15.0, 50.0)), false);
        let cannon = mount(2, shell, false);
        let missile = mount(3, ammo(100.0, 100.0, (0.0, 0.0)), true);

        let armed = [cannon.clone(), bullet.clone()];
        assert_eq!(machine_gun(&armed), MachineGun::Mounted(&bullet.weapon));
        assert_eq!(machine_gun(&[cannon, missile]), MachineGun::Unmounted);
        assert_eq!(machine_gun(&[]), MachineGun::StandIn);
    }

    // The carbine round: 6 out to 15 m, a tenth less by 50 m; 340 m/s
    // from her hands, under its 300 m/s most. A round whose dropoff is
    // whole falls to its minimum; one of 0 keeps its damage.
    #[test]
    fn a_round_falls_off_with_range_and_flies_at_its_human_speed() {
        assert_eq!(
            [DAMAGE_DROPOFF, DAMAGE_MINIMUM, FALLOFF_NEAR, FALLOFF_FAR],
            [
                "DamageDropoff",
                "DamageMinimum",
                "DamageDropoffStart",
                "DamageDropoffStop"
            ]
            .map(name_hash)
        );
        let a = Ammo {
            dropoff: 0.1,
            ..ammo(6.0, 3.0, (15.0, 50.0))
        };
        assert_eq!(a.damage_at(10.0), 6.0);
        assert!((a.damage_at(32.5) - 5.7).abs() < 1e-4);
        assert!((a.damage_at(80.0) - 5.4).abs() < 1e-4);
        let shell = ammo(100.0, 50.0, (25.0, 300.0));
        assert_eq!(shell.damage_at(300.0), 50.0);
        let sniper = Ammo {
            dropoff: 0.0,
            ..ammo(60.0, 15.0, (50.0, 100.0))
        };
        assert_eq!(sniper.damage_at(100.0), 60.0);
        assert_eq!(a.human_speed(), 300.0);
    }

    // The grenade's throw: 9 m/s at -45 degrees, 13 level, 16 at 45.
    #[test]
    fn a_throw_is_faster_the_higher_she_aims() {
        let t = Throwable {
            template: 1,
            name: "Grenade".into(),
            model: None,
            ammo: ammo(0.0, 0.0, (0.0, 0.0)),
            max: 9,
            speeds: [9.0, 13.0, 16.0],
            pitches: [-45.0, 0.0, 45.0],
            throw_angle: 20.0,
            cook: true,
            detonator: None,
            planted: None,
        };
        assert_eq!(t.speed_at(-90.0), 9.0);
        assert_eq!(t.speed_at(0.0), 13.0);
        assert!((t.speed_at(22.5) - 14.5).abs() < 1e-4);
        assert_eq!(t.speed_at(60.0), 16.0);
    }

    // `WeaponHint`'s names hash to the fields the database lists, in
    // order after the four leading words (gap round 13).
    #[test]
    fn weapon_hint_names_hash_as_listed() {
        let listed = [
            ("MinFireRange", 0x67a3_cde6),
            ("FireRange", 0x12b4_9564),
            ("FireAngle", 0x8d62_0c3a),
            ("BurstOnLength", 0xee64_1a7e),
            ("BurstOffLength", 0xb5ef_5ea8),
            ("ChargeTime", 0x2d59_5f06),
            ("BurstOnCloseLength", 0x169f_a172),
            ("BurstOffCloseLength", 0x6a2c_0fb4),
        ];
        for (name, hash) in listed {
            assert_eq!(name_hash(name), hash, "{name}");
        }
        // And `WeaponScatter`'s first field.
        assert_eq!(SCATTER_FIELDS[0], name_hash("LowSkillScatter"));
    }

    // The armour ratings' names hash as the fields of `WeaponHint`'s word
    // at 8 (bits 0, 2, 4 and 6).
    #[test]
    fn the_armour_ratings_are_named() {
        assert_eq!(
            AGAINST.map(name_hash),
            [0xa421_1ea4, 0x1e14_e116, 0xccdc_6fc3, 0x3fc0_56f3]
        );
        assert_eq!(name_hash("SingleShotWeapon"), 0xdfd7_d1dd);
    }

    // The carbine's and the minigun's bursts (their `WeaponHint`s).
    #[test]
    fn bursts_follow_the_hint() {
        let carbine = FireHint {
            min_range: 1.0,
            range: 70.0,
            angle: 0,
            on: 0.3,
            off: 4.5,
            on_close: -1.0,
            off_close: 0.75,
            against: [3, 1, 0, 0],
            kinds: [3, 1],
            single: false,
        };
        let gap = 60.0 / 562.0;
        // 0.3 s at 562 a minute: rounds at 0, 0.107 and 0.214 s.
        assert_eq!(carbine.rounds(gap, false), 3);
        // Close in, the far burst (-1) with the close pause.
        assert_eq!(carbine.burst(true), (0.3, 0.75));
        assert_eq!(carbine.rounds(gap, true), 3);
        let share = carbine.firing_share(gap, false);
        assert!((share - 0.32 / 4.82).abs() < 0.01, "{share}");
        // A 0 s burst is a single round; the pistol's 0 s pause fires at
        // its rate.
        let pistol = FireHint {
            on: 0.0,
            off: 0.0,
            ..carbine
        };
        assert_eq!(pistol.rounds(0.4, false), 1);
        assert_eq!(pistol.firing_share(0.4, false), 1.0);
        let minigun = FireHint {
            on: 3.0,
            off: 4.0,
            ..carbine
        };
        assert_eq!(minigun.rounds(60.0 / 900.0, false), 46);
    }

    // A rocket pod chain: each pod once, in the coupling's order.
    #[test]
    fn coupled_mounts_fire_in_turn() {
        let m = |link, hp| Mount {
            link,
            weapon: 7,
            hardpoint: hp,
        };
        let a = Armament {
            mounts: vec![m(1, 10), m(2, 20), m(3, 30), m(1, 10), m(9, 90)],
            couplings: vec![(1, 2), (2, 3), (3, 1)],
            seats: vec![(5, vec![1, 9])],
        };
        let g: Vec<u32> = a.group(1).iter().map(|m| m.hardpoint).collect();
        assert_eq!(g, vec![10, 20, 30]);
        assert_eq!(a.of_seat(5), &[1, 9]);
        assert!(a.of_seat(6).is_empty());
        // Placed on a model with nodes named 10 and 30 (none 20): the pods
        // fire from those, the seat's second weapon has no template.
        let spec = WeaponSpec {
            template: 7,
            name: "Rocket Pod".into(),
            model: None,
            class: None,
            semi: false,
            rate_of_fire: 120.0,
            aim_ai: 15f32.to_radians(),
            clip: 8,
            reserve_clips: 0,
            pellets: 1,
            rounds_per_reload: 0,
            scatter: 0.0,
            ammo: ammo(50.0, 50.0, (0.0, 0.0)),
            lock_on: None,
            seeker: None,
            pickup: None,
            feel: Default::default(),
            hint: None,
            ui: None,
            assist: None,
            camera: None,
            scope: None,
        };
        let arms = Arms {
            weapons: HashMap::from([(7, spec)]),
            ..Arms::default()
        };
        let m = mounted(&a, &arms, &[30, 99, 10]);
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].seat, m[0].priority), (5, 0));
        assert_eq!(m[0].nodes, vec![Some(2), None, Some(0)]);
        assert_eq!(m[1].nodes, vec![None]);
        // Three pods of 8 fired in turn hold 24 rounds between them.
        assert_eq!(AiClip::size(&m[0]), 24);
    }

    #[test]
    fn ai_clips_empty_and_reload() {
        // A pilot's pod of 7 held for 10 rounds' worth: 7 go, then none
        // until the trigger is let go, which fills it.
        let mut p = AiClip::default();
        let fired = (0..10)
            .filter(|_| {
                let ok = p.can_fire(7);
                if ok {
                    p.shot(7);
                }
                ok
            })
            .count();
        assert_eq!(fired, 7);
        assert_eq!(p.rounds(7), 0);
        p.pilot_off(7);
        assert_eq!(p.rounds(7), 7);
        // A gunner with an empty clip: the frame his timer has run out he
        // reloads and fires nothing, then may fire; he cannot ask again
        // for 0.5 s.
        let mut g = AiClip {
            left: Some(1),
            wait: 0.0,
        };
        assert!(g.gunner_empty(3));
        g.shot(3);
        g.tick(1.0 / 60.0);
        assert!(!g.gunner_empty(3));
        assert_eq!((g.rounds(3), g.wait), (3, AiClip::GUNNER_WAIT));
        assert!(g.gunner_empty(3));
        g.left = Some(0);
        g.tick(0.25);
        assert!(!g.gunner_empty(3));
        assert_eq!(g.rounds(3), 0);
        g.tick(0.3);
        assert!(!g.gunner_empty(3));
        assert_eq!(g.rounds(3), 3);
    }

    /// Share of rounds within 0.45 m of a mark `distance` ahead, by a
    /// fixed xorshift, for `spread`.
    fn hit_share(spread: SoldierSpread, distance: f32) -> f32 {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut pick = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5
        };
        let (n, at) = (20_000, Vec3::Z * distance);
        let hits = (0..n)
            .filter(|_| {
                let dir = spread.stray(Vec3::Z, pick(), pick());
                (dir * at.dot(dir)).distance(at) < 0.45
            })
            .count();
        hits as f32 / n as f32
    }

    #[test]
    fn soldiers_spread_by_their_guns_cone() {
        let vz = SoldierSpread::VZ_ACCURACY;
        // A perfect shot with the sniper rifle's 0.1 degrees still misses
        // a little; the shotgun's 3.5 is wider than the carbine's 1.
        let sniper = SoldierSpread::new(0.0, 1.0, 0.1);
        assert!(sniper.up > 0.1f32.to_radians());
        let (carbine, shotgun) = (
            SoldierSpread::new(20.0, vz, 1.0),
            SoldierSpread::new(20.0, vz, 3.5),
        );
        assert!((shotgun.side - carbine.side - 2.5f32.to_radians()).abs() < 1e-6);
        assert!((shotgun.up - carbine.up - 2.5f32.to_radians()).abs() < 1e-6);
        // The miss is twice as wide sideways and grows with range.
        let far = SoldierSpread::new(40.0, vz, 0.0);
        assert!((far.side - 2.0 * far.up).abs() < 1e-6);
        assert!(far.up > SoldierSpread::new(20.0, vz, 0.0).up);
        // A negative cone counts as none.
        assert_eq!(
            SoldierSpread::new(20.0, vz, -1.0),
            SoldierSpread::new(20.0, vz, 0.0)
        );
    }

    #[test]
    fn a_vz_soldiers_rifle_hits_at_20_m_as_before() {
        // The old rule (0.1 rad up, 0.2 along the world's x and z, the
        // same for every gun) hit 9.0% of his rounds at 20 m, over all
        // headings (the soldierspread example). The fitted miss keeps his
        // Assault Rifle (1.5 degrees) hitting as often.
        let new = SoldierSpread::new(20.0, SoldierSpread::VZ_ACCURACY, 1.5);
        let now = hit_share(new, 20.0);
        assert!((now - 0.090).abs() < 0.01, "{now}");
        // The machine pistols' 4 degrees hit less, the carbine's 1 more.
        let vz = SoldierSpread::VZ_ACCURACY;
        assert!(hit_share(SoldierSpread::new(10.0, vz, 4.0), 10.0) < 0.4);
        assert!(hit_share(SoldierSpread::new(10.0, vz, 1.0), 10.0) > 0.74);
    }

    #[test]
    fn a_stray_round_stays_within_the_widths() {
        let s = SoldierSpread { side: 0.2, up: 0.1 };
        let aim = Vec3::new(1.0, 0.0, 1.0).normalize();
        let d = s.stray(aim, 0.5, -0.5);
        let side = aim.cross(Vec3::Y).normalize();
        assert!((d.dot(side) / d.dot(aim) - 0.1).abs() < 1e-5);
        assert!((d.dot(Vec3::Y) / d.dot(aim) + 0.05).abs() < 1e-5);
        assert!(s.stray(aim, 0.0, 0.0).distance(aim) < 1e-6);
    }
}
