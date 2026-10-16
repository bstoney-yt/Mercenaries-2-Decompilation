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

use crate::tuning::Table;

/// `PhysicalLink`'s field naming the template it links (name unknown).
const LINK: u32 = 0x5470_4554;
/// `ControllerVelocity`'s multiple of `Velocity` when a person fires it.
const HUMAN_BOOST: u32 = 0xf3f6_f0a9;
/// `ProjectilePhysics`' least damage, and the ranges (metres) over which
/// a round's damage falls from `Damage` to it: unnamed, read as such by
/// their values (the carbine round's 15 and 50 m, the shotgun's 5 and 50,
/// the heavy machine gun's 20 and 300); inferred.
const DAMAGE_MINIMUM: u32 = 0x33f1_fb12;
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
/// `HomingWeapon`'s lock range, metres (the Stinger's 400; inferred).
const LOCK_RANGE: u32 = 0x1a09_69d2;
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
    /// Its damage `distance` metres from its heart: from `damage` down to
    /// `min_damage` at its edge, none past it.
    pub fn damage_at(&self, distance: f32) -> f32 {
        if distance > self.radius {
            return 0.0;
        }
        let t = (distance / self.radius.max(0.01)).clamp(0.0, 1.0);
        self.damage + (self.min_damage - self.damage) * t
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

    /// Its damage at `distance` metres: `Damage` up to the near range,
    /// `DamageMinimum` from the far one, a straight fall between.
    pub fn damage_at(&self, distance: f32) -> f32 {
        let (near, far) = self.falloff;
        if far <= near || distance <= near {
            return self.damage;
        }
        let t = ((distance - near) / (far - near)).clamp(0.0, 1.0);
        self.damage + (self.damage_min.min(self.damage) - self.damage) * t
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
    /// Rounds an ammunition pickup of it gives (its `Pickup` of type 1).
    pub pickup: Option<u32>,
    /// Its shots' sound, muzzle flash and scatter.
    pub feel: Feel,
    /// How soldiers fire it (`WeaponHint`), if its template says.
    pub hint: Option<FireHint>,
}

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
}

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

    /// The share of the time a soldier who keeps firing spends in bursts.
    pub fn firing_share(&self, gap: f32, close: bool) -> f32 {
        let (_, off) = self.burst(close);
        let firing = self.rounds(gap, close) as f32 * gap;
        firing / (firing + off).max(1e-3)
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

/// Every weapon the database names.
#[derive(Clone, Debug, Default)]
pub struct Arms {
    pub weapons: HashMap<u32, WeaponSpec>,
    pub throwables: HashMap<u32, Throwable>,
    /// By vehicle template.
    pub armaments: HashMap<u32, Armament>,
}

impl Arms {
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
    let pickup = Table::new(db, "Pickup");
    let scatter = Table::new(db, "WeaponScatter");
    let hint = Table::new(db, "WeaponHint");
    let effects = Table::new(db, "WeaponEffects");
    let sound = Table::new(db, "SoundEffect");
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
                    clip: w("iClipSize").map_or(1, |c| c & 0xffff).max(1),
                    reserve_clips: w("MaxAmmoReserve").map_or(0, |c| c & 0xff),
                    pellets: w("iBulletsPerShot").map_or(1, |c| c & 0xff).max(1),
                    rounds_per_reload: w("iRoundsPerReload").map_or(0, |c| c & 0xff),
                    scatter: Parts::float(&scatter, id, SCATTER).unwrap_or(0.0),
                    ammo: parts.ammo(ammo_id),
                    lock_on,
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

    Arms {
        weapons,
        throwables,
        armaments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // The carbine round: 6 out to 15 m, 3 from 50 m; 340 m/s from her
    // hands, under its 300 m/s most.
    #[test]
    fn a_round_falls_off_with_range_and_flies_at_its_human_speed() {
        let a = ammo(6.0, 3.0, (15.0, 50.0));
        assert_eq!(a.damage_at(10.0), 6.0);
        assert!((a.damage_at(32.5) - 4.5).abs() < 1e-4);
        assert_eq!(a.damage_at(80.0), 3.0);
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
            clip: 8,
            reserve_clips: 0,
            pellets: 1,
            rounds_per_reload: 0,
            scatter: 0.0,
            ammo: ammo(50.0, 50.0, (0.0, 0.0)),
            lock_on: None,
            pickup: None,
            feel: Default::default(),
            hint: None,
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
    }
}
