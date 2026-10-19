//! Fighting: the rifle, the RPG and grenades, soldiers shooting back,
//! explosions that wreck vehicles, and the player going down.
//!
//! Everyone carries the game's AK-47 in the right hand, held the way the
//! game's two-handed aim clip holds it, as in the browser viewer. The
//! weapons, clips and sounds are the game's; THE RULES ARE INVENTED, not
//! the game's: damage, fire rates, accuracy, blast radius and health are
//! guesses.

use std::sync::atomic::{AtomicBool, Ordering};

use m2_engine::tuning::DamageTable;
use m2_engine::weapons::{Ammo, Blast, Feel, Flight, SoldierSpread, Throwable, WeaponSpec};
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

/// How a weapon hangs on her back ([`slung_at`]): muzzle down or up, how
/// far its lower end leans toward her left (the tangent), how far its
/// middle hangs below the attach point and how high its top may reach
/// above it (metres).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hang {
    muzzle_up: bool,
    lean: f32,
    drop: f32,
    top: f32,
}

/// The carbine's and the RPG's: down her back ([`SLUNG_LEAN`],
/// [`SLUNG_TOP`]).
const HANG_DOWN: Hang = Hang {
    muzzle_up: false,
    lean: SLUNG_LEAN,
    drop: SLUNG_DROP,
    top: SLUNG_TOP,
};
/// The grenade launcher's, as the PC video has it (frame pc460): muzzle
/// up behind her right shoulder, 0.15-0.2 m above it, across her back to
/// its stock at her left hip, 25-30 degrees off straight up; ours hung
/// muzzle down. Round 103 had it as laid out but about 35 degrees off
/// (her posed back adds some), its muzzle at her ear where the video's
/// is near the top of her head: so the lean is less than the video's
/// angle and the top higher. Round 105 had the angle right (27 degrees)
/// but the muzzle still 0.13 m under the top of her head, so it hangs
/// that much higher. Fitted by eye.
const HANG_LAUNCHER: Hang = Hang {
    muzzle_up: true,
    lean: 0.4,
    drop: SLUNG_DROP - 0.13,
    top: 0.55,
};

/// Weapons drawn on the right hand's attach node turned as the node is
/// posed (the default: the grip sits in her palm, round 78), rather than
/// the hold worked out from the hands (`--grip hands`).
static ATTACH_FRAME: AtomicBool = AtomicBool::new(true);

pub fn use_attach_frame(on: bool) {
    ATTACH_FRAME.store(on, Ordering::Relaxed);
}
/// Muzzle marker node of the weapon models.
pub const MUZZLE: u32 = 0xb148_6ef3;
/// The node spent cases fly from, `hp_shelleject`: every gun template's
/// `ShellEjectHardpoint` but the covert pistol's (found on 77 of the 86
/// guns' models, round 19).
pub const EJECT: u32 = 0x3d10_6439;

/// Where a mounted gun of model `model` firing from `from` along `dir`
/// throws its cases: as far behind its muzzle as its model's case node
/// lies from its muzzle node, where the arsenal holds that model (the
/// side and height of the node are not kept); else none.
fn case_behind(arsenal: &Arsenal, model: Option<u32>, from: Vec3, dir: Vec3) -> Option<Vec3> {
    let w = arsenal.by_model.get(&model?)?;
    Some(from - dir * w.muzzle.distance(w.eject?))
}

/// A gun model's case node ([`EJECT`]), model space.
pub fn eject_node(model: &m2_assets::model::Model) -> Option<Vec3> {
    m2_assets::vehicle::marker(model, EJECT).map(Vec3::from)
}

/// The grenade launcher (its template, `Grenade Launcher`): 75 rounds a
/// minute from a clip of 6; its round (`GL Projectile`) flies at 20 m/s
/// under the whole of gravity and bursts where it lands (`SpawnOnDeath`
/// an `Explosion (Grenade)`). A clip takes this long to load, seconds
/// (invented), and a round that never lands bursts this far out, metres.
pub const LAUNCHER_CLIP: u32 = 6;
/// Rounds the grenade launcher keeps beyond its clip: its reserve is
/// "very limited" (the wiki), and taken up it comes with 3 clips, the
/// footage's "6 2", which a heavy ammunition pickup tops up to.
pub const LAUNCHER_SPARE: u32 = 2 * LAUNCHER_CLIP;
const LOB_GAP: f32 = 60.0 / 75.0;
const LOB_SPEED: f32 = 20.0;
const LAUNCHER_RELOAD: f32 = 2.5;
const LOB_RANGE: f32 = 250.0;
const LOB_FLASH: f32 = 0.45;

/// The carbine's clip (its template's `iClipSize`, the HUD's 30) and the
/// rounds it keeps beyond it: its template's `MaxAmmoReserve` is 9, which
/// counts clips (every fired weapon's is 0 to 9, whatever its clip), and
/// which the first contract's scripts fill it to. Her rifle's own are in
/// [`Rifle`].
pub const RIFLE_CLIP: u32 = 30;
pub const RIFLE_SPARE: u32 = 9 * RIFLE_CLIP;

/// Her rifle, as its template says: the first contract's scripts hand
/// every hero the `Carbine`; in free roam each has their own, the one
/// their template's `EquipmentLink`s give (Jennifer the `Bullpup Rifle`,
/// Mattias the `Assault Rifle`, Chris the `Carbine`).
#[derive(Clone, Debug, PartialEq)]
pub struct Rifle {
    /// Its template's name.
    pub name: String,
    /// Which model of the [`Arsenal`] it is: "m4", "ak47" or "bullpup".
    pub model: &'static str,
    /// Seconds between shots (`RateOfFire`: the carbine's 562 a minute).
    pub gap: f32,
    /// Rounds a clip, and rounds kept beyond it (`MaxAmmoReserve` clips).
    pub clip: u32,
    pub spare: u32,
    /// Its round's `Damage` (the carbine's 6), all `Bullet`s.
    pub damage: f32,
    /// Its template, and its model's name hash, if read from the data.
    pub template: Option<u32>,
    pub model_hash: Option<u32>,
    /// One shot a pull (`FireType` 1), and rounds a shot (the shotgun's
    /// 10).
    pub semi: bool,
    pub pellets: u32,
    /// Its round as its template gives it: range falloff, damage key,
    /// speed, blast.
    pub ammo: Option<Ammo>,
    /// How its rounds fly, and how long it takes to lock on (homing
    /// launchers) and from how far.
    pub flight: Flight,
    pub lock_on: Option<(f32, f32)>,
    /// Its `Equipment` carry class (name hash: `Rifle`, `Pistol`,
    /// `Rocket`), which picks the clips she holds it with; `None` read as
    /// a rifle's in the rifle's place and a rocket's in the launcher's.
    pub class: Option<u32>,
    /// Its shots' sound set, muzzle flash and scatter, as its template
    /// gives them.
    pub feel: Feel,
    /// What the HUD shows of it (`WeaponUI`), if its template says.
    pub ui: Option<m2_engine::weapons::GunUi>,
    /// Its aim assist (`HumanCameraModifier`), if its template says.
    pub assist: Option<m2_engine::weapons::AimAssist>,
}

impl Default for Rifle {
    fn default() -> Self {
        Self {
            name: "Carbine".to_owned(),
            model: "m4",
            gap: 60.0 / 562.0,
            clip: RIFLE_CLIP,
            spare: RIFLE_SPARE,
            damage: 6.0,
            template: None,
            model_hash: Some(m2_assets::name_hash("global_weapon_m4")),
            semi: false,
            pellets: 1,
            ammo: None,
            flight: Flight::Round,
            lock_on: None,
            class: None,
            feel: Feel::default(),
            ui: None,
            assist: None,
        }
    }
}

impl Rifle {
    /// Gun template `name`'s, as the template database reads.
    pub fn of(name: &str, gun: &m2_engine::tuning::HandGun) -> Self {
        let model = match gun.model {
            Some(m) if m == m2_assets::name_hash("global_weapon_bullpup") => "bullpup",
            Some(m) if m == m2_assets::name_hash("global_weapon_ak47") => "ak47",
            _ => "m4",
        };
        let clip = gun.clip.max(1);
        Self {
            name: name.to_owned(),
            model,
            gap: 60.0 / gun.rate_of_fire,
            clip,
            spare: gun.reserve_clips * clip,
            damage: gun.damage,
            template: Some(gun.template),
            model_hash: gun.model,
            ..Self::default()
        }
    }

    /// Any gun, as its template says ([`WeaponSpec`]).
    pub fn from_spec(spec: &WeaponSpec) -> Self {
        let model = match spec.model {
            Some(m) if m == m2_assets::name_hash("global_weapon_bullpup") => "bullpup",
            Some(m) if m == m2_assets::name_hash("global_weapon_ak47") => "ak47",
            Some(m) if m == m2_assets::name_hash("global_weapon_rpg") => "rpg",
            Some(m) if m == m2_assets::name_hash("global_weapon_mgl") => "mgl",
            _ => "m4",
        };
        Self {
            name: spec.name.clone(),
            model,
            gap: spec.gap(),
            clip: spec.clip,
            spare: spec.reserve_clips * spec.clip,
            damage: spec.ammo.damage,
            template: Some(spec.template),
            model_hash: spec.model,
            semi: spec.semi,
            pellets: spec.pellets,
            ammo: Some(spec.ammo),
            flight: spec.flight(),
            lock_on: spec.lock_on,
            class: spec.class,
            feel: spec.feel,
            ui: spec.ui,
            assist: spec.assist,
        }
    }

    /// The launcher the alpha knew before it read any (no template
    /// database: tests, a model viewer): the RPG's 60 a minute, a rocket
    /// a clip and 3 spare (its template's), or the crate's grenade
    /// launcher's 75, 6 and 2 clips.
    pub fn launcher(weapon: Weapon) -> Self {
        match weapon {
            Weapon::GrenadeLauncher => Self {
                name: "Grenade Launcher".to_owned(),
                model: "mgl",
                gap: LOB_GAP,
                clip: LAUNCHER_CLIP,
                spare: LAUNCHER_SPARE,
                damage: 25.0,
                model_hash: Some(m2_assets::name_hash("global_weapon_mgl")),
                semi: true,
                flight: Flight::Lobbed,
                ..Self::default()
            },
            _ => Self {
                name: "RPG".to_owned(),
                model: "rpg",
                gap: RPG_RELOAD,
                clip: 1,
                spare: RPG_SPARE,
                damage: 100.0,
                model_hash: Some(m2_assets::name_hash("global_weapon_rpg")),
                semi: true,
                flight: Flight::Straight,
                ..Self::default()
            },
        }
    }

    /// Which slot tag it goes under: a bullet gun is her rifle; a launcher
    /// the RPG's place, lobbing ones as the grenade launcher.
    pub fn slot(&self) -> Weapon {
        match self.flight {
            Flight::Round => Weapon::Rifle,
            Flight::Lobbed => Weapon::GrenadeLauncher,
            Flight::Straight | Flight::Homing => Weapon::Rpg,
        }
    }

    /// Its rounds' speed from her hands, m/s: its template's (`Velocity`
    /// times `HumanBoost`), else `fallback`.
    pub fn speed(&self, fallback: f32) -> f32 {
        self.ammo
            .map(|a| a.human_speed())
            .filter(|&v| v > 0.0)
            .unwrap_or(fallback)
    }

    /// What its round does where it bursts.
    pub fn blast(&self) -> Option<Blast> {
        self.ammo.and_then(|a| a.burst())
    }
}
/// The seats of vehicle `v` whose riders fire something, the driver's
/// first.
pub fn armed_seats(v: &ParkedVehicle) -> Vec<u32> {
    let driver = m2_assets::name_hash("seat_driver");
    let mut seats = vec![driver];
    for m in &v.spec.armed {
        if !seats.contains(&m.seat) {
            seats.push(m.seat);
        }
    }
    seats
}

/// Seconds a clip takes to change when she has no reload clip to play
/// (invented); with one, the change lasts as long as it (playtest 8: the
/// rifle's `ReloadIdle` 49288f7f, 1.0 s; inferred, as no gun template
/// field gives a reload time).
const RIFLE_RELOAD: f32 = 2.0;
const RIFLE_RANGE: f32 = 400.0;
/// What a carbine round does to crates and explosive props (theirs is
/// the alpha's own scale, not yet the damage table's).
const RIFLE_DAMAGE: f32 = 34.0;
/// The carbine round's `Damage`, which [`RIFLE_DAMAGE`] stands for.
const CARBINE_ROUND: f32 = 6.0;
/// How long she keeps the rifle up after a shot, seconds.
const AIM_AFTER_SHOT: f32 = 0.6;
/// Muzzle flashes: how long they last (a few frames) and how far they
/// reach from the muzzle, metres. Invented, to look like the game's.
const FLASH_TIME: f32 = 0.05;
/// A round's impact is set off this far short of where it strikes,
/// metres, so it shows off the face rather than half inside it
/// (invented).
const IMPACT_BACK: f32 = 0.05;
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
/// A tank's machine gun beside its cannon, which the aim button fires: the
/// AMX-30's linked machine gun fires 450 rounds a minute (its template).
const TANK_MG_GAP: f32 = 60.0 / 450.0;
const MG_DAMAGE: f32 = 45.0;
const MG_RANGE: f32 = 500.0;
/// Gunfire puts soldiers this close on alert and sends civilians running.
const ALERT_RANGE: f32 = 60.0;
const PANIC_RANGE: f32 = 40.0;
/// A soldier's hit on her when his template names no gun (the AK47
/// stands in; the rest hit as their own gun's round does,
/// [`Combat::npc_round`]): the "Assault Rifle Bullet" template's Damage
/// (the VZ soldiers' rifle, 9, `DamageKey` `Bullet`) times the game's
/// damage table's `Hero` row for a `Bullet` (0.25; see
/// `m2_engine::tuning::DamageTable`), whoever the hero is (all three
/// templates' `ModifierKey` is `Hero`). Any falloff with range is not
/// decoded.
const NPC_DAMAGE: f32 = 9.0 * 0.25;
// Her rounds do their round's `HeroMultiplier` times their `Damage` to
// people ([`m2_engine::weapons::Ammo::hero`]: the carbine's 3). Round 15
// stood 1.8 in for it, the least that fits the PC playthrough's first
// fight (3:36-3:48: the carbine kills three VZ deathsquad mooks of 75
// `Health` with 7, 13 and 10 rounds; 6 a round needs 13 hits, so a kill in
// 7 rounds needs at least 1.79 times). At 3, five hits kill at up to 15 m,
// and the footage's rounds per kill hold the misses.
/// A soldier's gun's `LowSkillScatter` when his template names no gun
/// (the Assault Rifle's 1.5 degrees, as its round stands in above).
const NPC_CONE: f32 = 1.5;
/// What blasts do to her, of their damage: the game's damage table's
/// `Hero` row (all three heroes' `ModifierKey`) for the blasts'
/// `DamageKey`s. Her grenades', rockets', tank shells' and C4's
/// explosions are `Explosion` (0.2); the satellite bomb's and its final
/// strike's `ExplosionLarge` (0.5), its frag's `BulletLarge` (0.4). The
/// defaults are the table's cells as read on the test PC (rounds
/// 158-160, heroes round 04); the game reads them from the table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeroDamage {
    pub explosion: f32,
    pub explosion_large: f32,
    pub bullet_large: f32,
}

impl Default for HeroDamage {
    fn default() -> Self {
        Self {
            explosion: 0.2,
            explosion_large: 0.5,
            bullet_large: 0.4,
        }
    }
}

/// People are hit within this distance of the line from shins to head.
const BODY_RADIUS: f32 = 0.3;
/// Health comes back after this many quiet seconds, at this rate a second.
const REGEN_AFTER: f32 = 4.0;
const REGEN_RATE: f32 = 25.0;
/// Where the health clamp stops her health: the hero script's healing
/// keeps at least this much too (inferred; no script sets the floor).
const CLAMP_FLOOR: f32 = 1.0;
/// Seconds she stays down at no health.
const DOWN_TIME: f32 = 2.5;
/// RPG: rocket speed (m/s), reload (s) and how far it flies.
const ROCKET_SPEED: f32 = 60.0;
const RPG_RELOAD: f32 = 1.0;
/// The RPG's rockets beyond the one loaded (its template's
/// `MaxAmmoReserve`: 3 clips of 1).
pub const RPG_SPARE: u32 = 3;
const ROCKET_RANGE: f32 = 400.0;
/// Grenade: throw speed (m/s), fuse (s), bounce, and seconds before she
/// can throw (or fire) again after starting a throw.
const THROW: f32 = 12.0;
const FUSE: f32 = 2.5;
const BOUNCE: f32 = 0.4;
const THROW_GAP: f32 = RPG_RELOAD * 0.5;
/// Grenades she can carry (the `Grenade` template's `MaxAmmoReserve`);
/// her hero template starts her with them, and the hero script fills
/// every weapon she holds to its most.
pub const MAX_GRENADES: u32 = 9;
/// A melee bash: how far ahead of her it reaches (from her chest, or her
/// knees for a low one at something on the ground), how wide (the cosine
/// of its half-angle), how much it hurts a person and what of the world's
/// it meets (a supply crate's 100 health: one blow opens it), how hard it
/// shoves (out, up; m/s), and seconds before she can bash or fire again.
/// Invented, from the playthroughs. A person takes the blow times their
/// damage table row's `MeleeBash` cell (from the data; no template's
/// `DamageKey` is `MeleeBash`, so the blow's own size stays invented).
const BASH_REACH: f32 = 1.8;
const BASH_ARC: f32 = 0.5;
/// A vehicle whose up axis leans further than this from straight up
/// (its cosine) lies on its side or roof, for a blow to right (invented).
const OVERTURNED_UP: f32 = 0.5;

/// Whether a vehicle whose up axis is `up` lies on its side or roof.
fn overturned(up: Vec3) -> bool {
    up.y < OVERTURNED_UP
}

/// Whether her blow from `feet`, facing `ahead`, reaches a vehicle with its
/// middle at `middle` and about `half` metres from its middle to its side.
fn within_bash(feet: Vec3, ahead: Vec3, middle: Vec3, half: f32) -> bool {
    let to = (middle - feet).with_y(0.0);
    to.length() <= BASH_REACH + half && to.normalize_or_zero().dot(ahead) >= BASH_ARC
}
const BASH_DAMAGE: f32 = 60.0;
const BASH_BREAK: f32 = 100.0;
const BASH_PUSH: (f32, f32) = (3.0, 1.0);
const BASH_GAP: f32 = 0.8;
/// Heights of a blow over her feet, high and low, metres.
const BASH_HIGH: f32 = 1.2;
const BASH_LOW: f32 = 0.45;
/// Blasts kill within `KILL` metres and hurt to `REACH`.
const KILL: f32 = 4.0;
/// How hard a blast throws people: (out, up) m/s at its heart, on top of
/// 3 out and 2 up at the edge of its reach. Invented, to look like the
/// game's flung bodies.
pub(crate) const BLAST_PUSH: (f32, f32) = (9.0, 6.0);
/// How hard a rifle round shoves the one it kills, m/s.
const SHOT_PUSH: f32 = 2.0;
pub(crate) const REACH: f32 = 9.0;
/// The satellite-guided bomb's blasts, from the template database's
/// laser-guided bomb ("Explosion (Airstike Bomb)" 80008e3f, its frag
/// "Explosion (Airstrike Frag)" 8000a3df, and its final strike 8000af7d
/// 2 s on): reach (metres) and damage at the heart, falling off to
/// nothing at the edge (the falloff is invented). The frag is stopped by
/// what stands between: the radio's word is to get behind cover.
pub const BOMB: (f32, f32) = (20.0, 100.0);
pub const BOMB_FRAG: (f32, f32) = (50.0, 300.0);
const BOMB_FINAL: (f32, f32) = (2.0, 500.0);
/// What a round does to a vehicle it hits: its bullet template's
/// `Damage` (the carbine's "Carbine Bullet" 6, the soldiers' "Assault
/// Rifle Bullet" 9, both `DamageKey` `Bullet`; the tank's and jeeps'
/// machine guns' "Heavy MG Bullet" 15, `BulletLarge`) times what the
/// game's damage table gives the vehicle's `ModifierKey` for that kind
/// (a `Car` takes 0.025 of a `Bullet`, 0.1 of a `BulletLarge`; a `Tank`
/// nothing). A vehicle the table doesn't cover takes nothing.
const RIFLE_ON_VEHICLE: (f32, Round) = (9.0, Round::Bullet);
const MG_ON_VEHICLE: (f32, Round) = (15.0, Round::BulletLarge);

/// A round's `DamageKey`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Round {
    Bullet,
    BulletLarge,
}
/// A blast's damage to vehicles (the "Explosion (Tank Shell)" and RPG's:
/// 200 at its heart down to 50 at 5 m, nothing beyond), times the
/// vehicle's `ExplosionFudge` (the AMX-30 takes a fifth).
const BLAST_ON_VEHICLE: (f32, f32, f32) = (200.0, 50.0, 5.0);
/// Below its `TickDamage` health a vehicle burns down this fast, health a
/// second (the template's 1 every 0.5 s, read as seconds).
const BURN_DOWN: f32 = 2.0;
/// How often the effects vehicles' parts keep going are kept up, seconds
/// (well inside a held effect's life).
const PART_FX_EVERY: f32 = 0.3;
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
    /// Where its spent cases fly from: its model's `ShellEjectHardpoint`
    /// node ([`EJECT`]), if it has one.
    pub eject: Option<Vec3>,
    /// Weapon to right-hand transform held in both hands (aiming, or at
    /// the ready).
    pub grip: Mat4,
    /// The hand `grip` is relative to: the rear (trigger) hand of the aim
    /// pose.
    pub grip_hand: Option<usize>,
    /// The aim pose's front hand, and where the grip sits in the rear
    /// hand (its space): held in both hands, the barrel runs from the
    /// rear hand towards the front one in whatever pose she is in
    /// ([`Self::in_hand`]).
    pub grip_front: Option<(usize, Vec3)>,
    /// Carried low in the right hand alone, muzzle forward and down as
    /// the game's player carries a rifle: where the grip sits in the hand
    /// (hand space), and the weapon's turn in the body's space, so it
    /// points the same way however the hand turns.
    pub carry: Option<(Vec3, Quat)>,
    /// How it hangs when slung.
    pub hang: Hang,
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
    /// Jennifer's own rifle (Mattias's is the AK47).
    pub bullpup: Option<WeaponModel>,
    /// Every other hand weapon's model the game's templates name, by its
    /// model name's hash.
    pub by_model: std::collections::HashMap<u32, WeaponModel>,
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
    /// Adds weapon `name`, gripped in `rig`'s right hand, with its muzzle
    /// and case node.
    pub fn add(
        &mut self,
        name: &str,
        model: GpuModel,
        points: (Vec3, Option<Vec3>),
        rig: Option<&Rig>,
    ) {
        let w = Some(self.hold(name == "mgl", model, points, rig));
        match name {
            "ak47" => self.rifle = w,
            "rpg" => self.rpg = w,
            "rocket" => self.rocket = w,
            "grenade" => self.grenade = w,
            "mgl" => self.launcher = w,
            "40mm" => self.round = w,
            "m4" => self.carbine = w,
            "detonator" => self.designator = w,
            "bullpup" => self.bullpup = w,
            _ => {}
        }
    }

    /// Adds the hand weapon whose model name hashes to `hash`.
    pub fn add_model(
        &mut self,
        hash: u32,
        model: GpuModel,
        points: (Vec3, Option<Vec3>),
        rig: Option<&Rig>,
    ) {
        let launcher = hash == m2_assets::name_hash("global_weapon_mgl");
        let w = self.hold(launcher, model, points, rig);
        self.by_model.insert(hash, w);
    }

    /// The model of gun `gun`: its own model, else the one of the alpha's
    /// it stands in for.
    pub fn of_gun(&self, gun: &Rifle) -> Option<&WeaponModel> {
        gun.model_hash
            .and_then(|h| self.by_model.get(&h))
            .or_else(|| match gun.model {
                "rpg" => self.rpg.as_ref(),
                "mgl" => self.launcher.as_ref(),
                _ => self.hers(gun),
            })
    }

    /// `model` held in `rig`'s right hand, hung as a launcher or not.
    fn hold(
        &mut self,
        launcher: bool,
        model: GpuModel,
        (muzzle, eject): (Vec3, Option<Vec3>),
        rig: Option<&Rig>,
    ) -> WeaponModel {
        let mut w = WeaponModel {
            model,
            muzzle,
            eject,
            grip: Mat4::IDENTITY,
            grip_hand: None,
            grip_front: None,
            carry: None,
            hang: if launcher { HANG_LAUNCHER } else { HANG_DOWN },
        };
        if let Some(r) = rig {
            self.nodes_of(r);
            fit(&mut w, r);
        }
        w
    }

    /// Every weapon held in `rig`'s hands and hung on its back from now
    /// on, in place of the rig they were held by: a new hero's skeleton
    /// has its own nodes (Mattias's 116 are not Jennifer's 92), and the
    /// weapons mostly load during the front end, before the hero is
    /// picked, so they were held by the default hero's.
    pub fn rehold(&mut self, rig: &Rig) {
        self.nodes_of(rig);
        let named = [
            &mut self.rifle,
            &mut self.rpg,
            &mut self.rocket,
            &mut self.grenade,
            &mut self.launcher,
            &mut self.round,
            &mut self.carbine,
            &mut self.designator,
            &mut self.bullpup,
        ];
        for w in named
            .into_iter()
            .flatten()
            .chain(self.by_model.values_mut())
        {
            fit(w, rig);
        }
        eprintln!(
            "weapons: held by a {}-node skeleton: hand {:?}, attach {:?}, back {:?}, spine {:?}, neck {:?}",
            rig.skeleton.len(),
            self.hand,
            self.attach,
            self.back,
            self.spine,
            self.neck
        );
    }

    /// The hand, attach point, back, spine and neck nodes of `rig`'s
    /// skeleton.
    fn nodes_of(&mut self, rig: &Rig) {
        let s = &rig.skeleton;
        self.hand = rig.rifle_hand();
        self.attach = right_attach(s);
        self.back = BACK_ATTACH.map(|h| node(s, h));
        self.spine = node(s, SPINE);
        self.neck = node(s, NECK);
    }

    /// The model of her rifle `rifle`: else the carbine's, else the
    /// AK47's.
    pub fn hers(&self, rifle: &Rifle) -> Option<&WeaponModel> {
        match rifle.model {
            "bullpup" => self.bullpup.as_ref(),
            "ak47" => self.rifle.as_ref(),
            _ => None,
        }
        .or(self.carbine.as_ref())
        .or(self.rifle.as_ref())
    }

    /// The model a soldier holds: his template's gun's own, else the AK47
    /// that stands in (for templates naming no gun, or whose gun's model
    /// isn't loaded).
    pub fn npc_gun(&self, gun: Option<m2_engine::tuning::HandGun>) -> Option<&WeaponModel> {
        let own = gun.and_then(|g| g.model).and_then(|h| {
            self.by_model.get(&h).or_else(|| {
                [&self.rifle, &self.carbine, &self.bullpup]
                    .into_iter()
                    .flatten()
                    .find(|w| w.model.hash == h)
            })
        });
        own.or(self.rifle.as_ref())
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
        slung_at(
            weapon.muzzle,
            weapon.model.bounds,
            point,
            back,
            slot,
            weapon.hang,
        )
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
                // The barrel from the rear hand towards the front one as
                // they are posed now: the aim's grip fitted the aim pose
                // alone, and running (its own clip, the rifle low across
                // her) it pointed down between her legs (playtest 5).
                let between = weapon.grip_front.and_then(|(front, at)| {
                    let dir = nodes.get(front)?.w_axis.truncate() - rear.w_axis.truncate();
                    let turn = turn(weapon.muzzle, dir)?;
                    Some(Mat4::from_rotation_translation(
                        turn,
                        rear.transform_point3(at),
                    ))
                });
                Some(body * between.unwrap_or(*rear * weapon.grip))
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
/// `muzzle` is at down, or up as `hang` has it) down her back, leaning as
/// `hang` has it, flat against her back with its top to her side, its
/// middle [`SLUNG_DROP`] below the point and [`SLUNG_GAP`] behind it, or
/// lower to keep its top down (`hang`'s top).
fn slung_at(
    muzzle: Vec3,
    bounds: (Vec3, Vec3),
    point: Vec3,
    (right, up): (Vec3, Vec3),
    slot: usize,
    hang: Hang,
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
    // Along it toward the end that hangs down.
    barrel[long] = if (muzzle[long] < middle[long]) != hang.muzzle_up {
        -1.0
    } else {
        1.0
    };
    let top = (Vec3::Y - barrel * barrel.y)
        .try_normalize()
        .unwrap_or(Vec3::Z);
    let own = m2_math::glam::Mat3::from_cols(barrel, top, barrel.cross(top));
    // Where they go on her back.
    let down = (-up - right * hang.lean).normalize();
    let side = if slot == 0 { right } else { -right };
    let to_side = (side - down * side.dot(down)).normalize();
    let turn = m2_math::glam::Mat3::from_cols(down, to_side, down.cross(to_side)) * own.transpose();
    // How far it reaches up and down her back from its middle.
    let reach = (0..3)
        .map(|k| (turn.col(k).dot(up) * size[k]).abs() * 0.5)
        .sum::<f32>();
    let lower = (reach - hang.drop - hang.top)
        .min(SLUNG_FLOOR - hang.drop - reach)
        .max(0.0);
    let origin = point - up * (hang.drop + lower) - ahead * SLUNG_GAP - turn * middle;
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

/// Weapon `w`'s hold in `rig`'s hands, two-handed and carried
/// ([`grip`], [`carry`]).
fn fit(w: &mut WeaponModel, rig: &Rig) {
    (w.grip, w.grip_hand, w.grip_front) = match grip(rig, w.muzzle) {
        Some((m, hand, front)) => (m, Some(hand), Some(front)),
        None => (Mat4::IDENTITY, None, None),
    };
    w.carry = carry(rig, w.muzzle);
}

/// Where a weapon with its muzzle at `muzzle` sits in the aim clip's
/// pose: grip in the rear hand, barrel towards the front one, upright
/// about the barrel; the rear hand's node; and the front hand's, with
/// where the grip sits in the rear hand (its space).
fn grip(rig: &Rig, muzzle: Vec3) -> Option<(Mat4, usize, (usize, Vec3))> {
    let skeleton = &rig.skeleton;
    let (rear, front) = rig.aim_hands()?;
    let aim = aim_clip(rig)?;
    let pose = aim.sample(skeleton, aim.info.duration * 0.5);
    let nodes = skeleton.model_matrices(&pose);
    let pr = nodes[rear].transform_point3(Vec3::ZERO);
    let pl = nodes[front].transform_point3(Vec3::ZERO);
    let at = palm(skeleton, &nodes, rear);
    Some((
        place(nodes[rear], at, muzzle, pl - pr)?,
        rear,
        (front, nodes[rear].inverse().transform_point3(at)),
    ))
}

/// The clip `rig` aims with: the hero's own ([`Rig::aim_clip`]), else
/// the aim clip by name, unless that is only another hero's.
fn aim_clip(rig: &Rig) -> Option<&m2_engine::BoundClip> {
    match rig.aim_clip {
        Some(i) => rig.clips.get(i),
        None => rig
            .clips
            .iter()
            .find(|c| c.hash == crate::npc::FightClips::AIM && !rig.foreign.contains(&c.hash)),
    }
}

/// Of a human skeleton's hands, (rear, front) in the aim clip: the
/// trigger hand and the one under the barrel. Without the clip, right
/// then left.
pub fn aim_hands(rig: &Rig) -> Option<(usize, usize)> {
    let skeleton = &rig.skeleton;
    let (rh, lh) = hands(skeleton)?;
    let Some(aim) = aim_clip(rig) else {
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
            Weapon::Rifle => Rifle::default().clip,
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

/// `r` reloaded into a clip of `size`: `None` when it is full or nothing
/// is stored to load.
fn topped_up(r: Rounds, size: u32) -> Option<Rounds> {
    (r.clip < size && r.stored > 0).then(|| Rounds::of(r.total(), size))
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

/// One of the game's own particle effects to set off: those a template
/// shows (its `RedEffectComponent`s), at `at`. A held one (`hold`) keeps
/// going, following `at`, while its source keeps asking for it (a fire,
/// a smoking wreck); the rest run once.
#[derive(Clone, Debug, PartialEq)]
pub struct GameFx {
    pub template: FxTemplate,
    pub at: Vec3,
    pub hold: Option<u64>,
}

/// A template, by id, by name, or by its name's hash (a state machine's
/// `StartEmitter` names its effect so).
#[derive(Clone, Debug, PartialEq)]
pub enum FxTemplate {
    Id(u32),
    Name(String),
    Hash(u32),
}

/// A sound the fight makes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sound {
    /// A gun's shot: its template's sound set, if it has one (the cue it
    /// plays is the sound set's in the `Sounds` table,
    /// [`m2_engine::gun_sounds`]), and whether someone other than the
    /// hero fired it, and their number (0 for the hero), which picks
    /// their variant of the shot.
    Shot {
        at: Vec3,
        set: Option<u32>,
        npc: bool,
        shooter: u32,
    },
    /// A rocket or missile fired, a grenade lobbed, a tank's main gun: the
    /// weapon's sound set where its template has one (its cue is played
    /// as a shot's; every launcher's, cannon's and missile's reaches a
    /// sound but one, round 12 of the gap fills), else the alpha's own
    /// sound for each.
    Launch {
        at: Vec3,
        set: Option<u32>,
    },
    Lob {
        at: Vec3,
        set: Option<u32>,
    },
    Explosion(Vec3),
    Cannon {
        at: Vec3,
        set: Option<u32>,
    },
    /// Her gun reloaded (heard as her own, flat): its template's sound set, whose
    /// reload cue the `Sounds` table gives ([`m2_engine::gun_sounds::GunCues::reload`]).
    Reload {
        set: Option<u32>,
    },
    /// The satellite-guided bomb landing.
    Bomb(Vec3),
    /// A building the bomb brought down coming apart.
    Collapse(Vec3),
    /// Something with no building's collapse (a fence, a sign) breaking:
    /// its template's `SoundKey` (`prop_sheetmetal_sml`).
    Break {
        at: Vec3,
        key: u32,
    },
}

/// A blow or a round that landed on something of the world's rather than
/// a person: where, how hard, and the static it hit if one of the
/// scene's (a supply crate's box), which takes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strike {
    pub at: Vec3,
    pub damage: f32,
    pub key: Option<StaticKey>,
    /// The round's or blow's own `Damage`, and its `DamageKey` (name
    /// hash: `Bullet`, `BulletLarge`, `MeleeBash`; 0 for none): what the
    /// damage table's row of what it hit takes of it
    /// ([`crate::destruct::Destruction::wear`]).
    pub round: f32,
    pub kind: u32,
}

/// A round's `DamageKey`: its ammunition's, a plain `Bullet` for none.
fn round_kind(key: Option<u32>) -> u32 {
    key.filter(|&k| k != 0)
        .unwrap_or_else(|| m2_assets::name_hash("Bullet"))
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

/// What an ammunition pickup's `Pickup` records give her, by the weapons
/// she carries now ([`Combat::refill`]): rounds for her rifle and for the
/// gun in the launcher's place, and grenades.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Refill {
    pub rifle: u32,
    pub launcher: u32,
    pub grenades: u32,
}

/// What flies, to draw: a rocket, a thrown grenade or a launched round.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flying {
    Rocket,
    Grenade,
    Round,
    /// C4 or a mine: its model's name hash.
    Charge(Option<u32>),
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
    /// passing through the vehicle that fired it; a homing one turns after
    /// the vehicle it was locked on, at its turn rate (radians a second).
    Rocket {
        at: Vec3,
        velocity: Vec3,
        flown: f32,
        shell: bool,
        owner: Option<VehicleId>,
        blast: Option<Blast>,
        homing: Option<(VehicleId, f32)>,
        /// Its round's `DamageKey` (`Rocket`): what a weak point it
        /// strikes takes of its blast ([`DamageTable::weak_point`]).
        key: Option<u32>,
    },
    Grenade {
        at: Vec3,
        velocity: Vec3,
        fuse: f32,
        blast: Option<Blast>,
    },
    /// A grenade launcher's round: bursts where it lands.
    Round {
        at: Vec3,
        velocity: Vec3,
        flown: f32,
        blast: Option<Blast>,
    },
    /// A thrown charge or mine: it sticks where it lands (C4) or lies
    /// there; C4 waits for the detonator (or its timer), a mine arms after
    /// `arm` seconds and goes off when someone or something comes within
    /// `reach`, or is gone after `life`.
    Charge {
        at: Vec3,
        velocity: Vec3,
        model: Option<u32>,
        stuck: bool,
        blast: Option<Blast>,
        timer: f32,
        mine: Option<Mine>,
    },
}

/// A proximity mine's arming and reach.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Mine {
    arm: f32,
    reach: f32,
    life: f32,
    blast: Option<Blast>,
}

/// What she asks of her weapons this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Trigger {
    /// Fire held.
    pub fire: bool,
    /// Change the rifle's clip.
    pub reload: bool,
    /// Aim held.
    pub aim: bool,
    pub throw: bool,
    /// A melee bash.
    pub melee: bool,
    pub select: Option<Weapon>,
    /// Change which explosive she throws (grenades, C4, mines).
    pub next_explosive: bool,
    /// Set off her C4.
    pub detonate: bool,
    /// Move to the vehicle's next seat with a weapon.
    pub seat: bool,
    /// The support keys, up and down: the support menu opens, or its
    /// line moves ([`crate::support`]).
    pub support: bool,
    pub support_down: bool,
    /// The PDA key: the PDA opens or closes.
    pub pda: bool,
}

/// The game's effect templates the fight's own fire and smoke set off,
/// where no template of the fight's says which (names from the template
/// database; which source shows which is chosen by name: invented). A
/// blast from an explosion template shows that template's own.
pub mod fx {
    /// A blast with no template (a barrel, a mine's chain).
    pub const GRENADE: &str = "Explosion (Grenade)";
    /// The satellite-guided bomb, and an air strike's big blasts.
    pub const BOMB: &str = "Explosion (Airstike Bomb)";
    /// An air strike's small blasts.
    pub const BOMBLET: &str = "Explosion (Cluster Bomblet)";
    /// The bomb's final strike.
    pub const FINAL: &str = "Explosion (Smart Bomb)";
    /// The launcher's back-blast and flash.
    pub const BACK_BLAST: &str = "global_particle_muzzleflash_rpg";
    /// A tank's main gun.
    pub const CANNON: &str = "Muzzle Flash (RR)";
    /// A rocket's trail.
    pub const ROCKET: &str = "RPG Rocket";
    /// A thrown marker's smoke.
    pub const MARKER: &str = "FlareSmoke";
    /// A wrecked vehicle burning.
    pub const WRECK: [&str; 2] = [
        "global_particle_firemediumsmoke_vehicle_box_flame",
        "global_particle_firemediumsmoke_smoke_vehicle",
    ];
    /// A building's blaze.
    pub const BLAZE: [&str; 2] = [
        "global_particle_firelargesmoke",
        "global_particle_firelargesmoke_smoke",
    ];
    /// A barrel or a bomb's fire burning on.
    pub const FIRE: [&str; 2] = [
        "global_particle_firesmall",
        "global_particle_firesmall_smoke",
    ];
    /// A building coming apart.
    pub const COLLAPSE: &str = "fx_CollapseCoverDust";
    /// A supply crate broken open.
    pub const CRATE: &str = "global_particle_impact_wood";
}

/// A thrown marker smokes from this long after it leaves her hand until
/// this long, seconds, a puff every [`MARKER_PUFF`].
const MARKER_SMOKE: (f32, f32) = (1.0, 30.0);
const MARKER_PUFF: f32 = 0.3;

/// A designator she threw: thrown as a grenade is, it bounces and comes
/// to rest, and never goes off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thrown {
    pub guid: Option<u32>,
    pub at: Vec3,
    velocity: Vec3,
    pub age: f32,
    /// The scripts have been told it lies still.
    pub settled: bool,
}

/// The fight's state.
#[derive(Debug)]
pub struct Combat {
    pub hp: f32,
    /// Her health when whole: her hero template's `Health` (Mattias
    /// 120, Jennifer and Chris 100).
    pub max_hp: f32,
    pub weapon: Weapon,
    /// Seconds she stays down.
    pub down: f32,
    /// The scripts made her invincible (while the screen fades, at the
    /// fanfare): nothing hurts her.
    pub invincible: bool,
    /// Seconds before a vehicle can hurt her again by running into her
    /// ([`crate::npc::RAM_GAP`]).
    pub rammed: f32,
    /// Vehicles that ran into her, and the health they took.
    pub run_overs: (u32, f32),
    /// The scripts keep her from dying (`Player.SetHealthClamp`): her
    /// health stops at [`CLAMP_FLOOR`] for their survival system.
    pub health_clamp: bool,
    /// The scripts heal her (the hero script's timers), not the fight.
    pub scripted_health: bool,
    /// She just got back up after going down (taken by the mission).
    pub revived: bool,
    quiet: f32,
    reload: f32,
    /// Seconds until a tank's machine gun fires again.
    mg_reload: f32,
    aim_left: f32,
    /// What her shots have added to her gun's cone
    /// ([`m2_engine::weapons::Bloom`]).
    bloom: m2_engine::weapons::Bloom,
    /// Seconds until the grenade she is throwing leaves her hand.
    throw_in: Option<f32>,
    /// Seconds until the blow she is bashing with lands, and whether low.
    bash_in: Option<(f32, bool)>,
    projectiles: Vec<Projectile>,
    /// Effects with their seconds left and lifetime.
    pub effects: Vec<(Effect, f32, f32)>,
    /// The game's own effects set off since the scene last took them
    /// ([`GameFx`]).
    pub game_fx: Vec<GameFx>,
    /// Every gun's shots' sound set and muzzle flash, by template (for
    /// the guns people carry, [`arm`](Self::arm)).
    feels: std::collections::HashMap<u32, Feel>,
    /// Sounds made since the last call to [`take_sounds`](Self::take_sounds).
    sounds: Vec<Sound>,
    /// Wrecks smoke: (vehicle index, seconds to the next puff).
    wrecks: Vec<(usize, f32)>,
    /// Seconds to the next keeping-up of vehicles' parts' effects.
    part_fx_next: f32,
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
    /// Her rifle, and its rounds; it is her own (her template's), not
    /// one the scripts handed her.
    pub rifle: Rifle,
    pub own_rifle: bool,
    /// The contract's scripts have handed her her weapons or their
    /// reserves (`Command::Loadout`, `Command::Reserve`): what she carries
    /// to throw is theirs, not her template's.
    pub scripted_loadout: bool,
    /// What blasts do to her, of their damage.
    pub hero_damage: HeroDamage,
    pub rifle_rounds: Rounds,
    /// Seconds until the rifle's new clip is in, while she changes it.
    changing: Option<f32>,
    /// Grenades she has to throw.
    pub grenades: u32,
    /// She holds the satellite designator instead of her weapon
    /// ([`crate::support`]).
    pub designator: bool,
    /// She holds a designator to throw (a support call's smoke grenade,
    /// `Airstrike.EquipDesignator`): the throw key throws it in place of
    /// a grenade.
    pub to_throw: bool,
    /// Designators thrown: the scripts' guid once they have it, where it
    /// is, how it flies and seconds since it left her hand.
    pub thrown: Vec<Thrown>,
    /// The next throw is the designator's.
    throwing_designator: bool,
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
    /// The gun in the launcher's place, as its template says (what
    /// [`second`](Self::second) names).
    pub launcher: Rifle,
    /// The guns as the templates give them that the alpha hands out by
    /// kind: the first contract's carbine, the RPG, the grenade launcher.
    pub carbine: Rifle,
    pub rpg: Rifle,
    pub gl: Rifle,
    /// The game's damage table: what each kind of round and blast does to
    /// people and vehicles.
    pub damage_table: Option<DamageTable>,
    /// The thrown weapons she can carry, as their templates say: the
    /// grenade, C4 and the proximity mine; which she throws, and how many
    /// C4 charges and mines she has (grenades are
    /// [`grenades`](Self::grenades)).
    pub throwables: Vec<Option<Throwable>>,
    pub explosive: usize,
    pub charges: u32,
    pub mines: u32,
    /// Fire was held last tick (a one-shot-a-pull gun fires again only
    /// once it is let go).
    fire_held: bool,
    /// A homing launcher's lock: on which vehicle, and for how long.
    pub lock: Option<(VehicleId, f32)>,
    /// In a vehicle: which of its seats with weapons she rides in (0 the
    /// driver's), and how many rounds its weapons have fired, to fire
    /// from their hardpoints in turn.
    pub seat: usize,
    /// In a vehicle: which of its guns Fire fires, 0 its main gun (a
    /// tank's cannon) and 1 its machine gun, chosen with the weapon
    /// switch (Tab, the wheel) as the game's tank controls do
    /// (`Switch_Primary`).
    pub vehicle_gun: usize,
    ripple: usize,
    seed: u64,
    /// What she has fired, for the HUD's ammo counters.
    pub fired: Fired,
    /// Every shot fired, by its gun's sound set (0: none) and whether the
    /// hero fired it, and the muzzle flashes drawn as the game's effect
    /// and as the alpha's own: for the run's log.
    pub shots: std::collections::BTreeMap<(u32, bool), u32>,
    /// The people other than the hero who fired, by sound set: their
    /// numbers (which pick their variant of the shot), for the log.
    pub shooters: std::collections::BTreeMap<u32, std::collections::BTreeSet<u32>>,
    pub flashes: (u32, u32),
    /// Spent cases thrown out (each its gun's `ShellEjectTemplate`).
    pub cases: u32,
    /// Rounds that struck the world and threw up an impact: the one
    /// their surface's word names, or the generic one standing in
    /// ([`m2_engine::surface::bullet_impact`]).
    pub impacts: (u32, u32),
    /// Those impacts by template.
    pub impact_kinds: std::collections::BTreeMap<&'static str, u32>,
    /// What the fighting has come to (for checking).
    pub tally: Tally,
    /// Hurts since [`take_hurts`](Self::take_hurts): damage and where
    /// from, for the HUD.
    hurts: Vec<(f32, Option<Vec3>)>,
    /// The emplaced gun she mans, if any ([`crate::placed`]): fire fires
    /// it, and nothing she carries.
    pub manned: Option<Manned>,
}

/// An emplaced gun she mans (`Emplaced MG (VZ)`): its template's
/// `PhysicalLink`ed weapon (`Mounted Heavy MG (VZ)`) fired along the
/// crosshair, held within its `Turret`'s turn (`MinYaw`..`MaxYaw`,
/// `MinPitch`..`MaxPitch`, -90..90 and -20..30 degrees) from the way it
/// was placed facing.
#[derive(Clone, Debug, PartialEq)]
pub struct Manned {
    /// Its entity and template.
    pub entity: u32,
    pub template: u32,
    /// Where the gun turns: its place, raised to the barrel
    /// ([`MANNED_PIVOT`]), and the way it faces (yaw, radians; its
    /// model's +z).
    pub pivot: Vec3,
    pub facing: f32,
    /// How far it turns and tilts, radians.
    pub yaw: (f32, f32),
    pub pitch: (f32, f32),
    pub weapon: m2_engine::weapons::WeaponSpec,
    /// Rounds left: its clip and reserve clips together.
    pub rounds: u32,
}

/// An emplaced gun's barrel above its place, metres (invented: the
/// tripod's gun sits about a crouching soldier's shoulder high).
pub const MANNED_PIVOT: f32 = 1.1;

impl Manned {
    /// `dir` held within its turn and tilt.
    pub fn clamp(&self, dir: Vec3) -> Vec3 {
        turret_clamp(dir, self.facing, self.yaw, self.pitch)
    }
}

/// `dir` held within `yaw` (radians either side of `facing`, across the
/// ground) and `pitch` (radians, up positive).
fn turret_clamp(dir: Vec3, facing: f32, yaw: (f32, f32), pitch: (f32, f32)) -> Vec3 {
    use std::f32::consts::{PI, TAU};
    let flat = dir.with_y(0.0);
    let off = (flat.x.atan2(flat.z) - facing + PI).rem_euclid(TAU) - PI;
    let yaw = facing + off.clamp(yaw.0, yaw.1);
    let pitch = dir.y.atan2(flat.length()).clamp(pitch.0, pitch.1);
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    Vec3::new(sy * cp, sp, cy * cp)
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

/// What the fighting has come to, for checking the balance against the
/// footage (where her ammunition counter and health box can be read):
/// her rounds at people and what they did, and what was done to her.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tally {
    /// Her rounds that struck a living soldier, the damage they did him
    /// (after the damage table), and the soldiers they killed.
    pub hits: u32,
    pub dealt: f32,
    pub kills: u32,
    /// Damage done to her by soldiers' rounds and by blasts, the rounds
    /// that struck her, and the times she went down.
    pub taken_rounds: f32,
    pub taken_blasts: f32,
    pub rounds_on_her: u32,
    pub downs: u32,
}

impl Tally {
    /// The tally in a line: rounds fired is hers to give (the rifle's).
    pub fn line(&self, fired: u32) -> String {
        let per = |n: f32, d: u32| {
            if d == 0 {
                "-".to_owned()
            } else {
                format!("{:.1}", n / d as f32)
            }
        };
        format!(
            "{fired} rounds, {} hits on soldiers ({} a hit), {} killed by rounds ({} rounds, {} hits each); \
             she took {:.0} from {} rounds and {:.0} from blasts, down {} times",
            self.hits,
            per(self.dealt, self.hits),
            self.kills,
            per(fired as f32, self.kills),
            per(self.hits as f32, self.kills),
            self.taken_rounds,
            self.rounds_on_her,
            self.taken_blasts,
            self.downs,
        )
    }
}

impl Default for Combat {
    fn default() -> Self {
        Self {
            hp: 100.0,
            max_hp: 100.0,
            weapon: Weapon::Rifle,
            down: 0.0,
            invincible: false,
            rammed: 0.0,
            run_overs: (0, 0.0),
            health_clamp: false,
            scripted_health: false,
            revived: false,
            quiet: 0.0,
            reload: 0.0,
            mg_reload: 0.0,
            manned: None,
            aim_left: 0.0,
            bloom: m2_engine::weapons::Bloom::default(),
            throw_in: None,
            bash_in: None,
            projectiles: Vec::new(),
            effects: Vec::new(),
            game_fx: Vec::new(),
            feels: std::collections::HashMap::new(),
            sounds: Vec::new(),
            wrecks: Vec::new(),
            part_fx_next: 0.0,
            remarks: Vec::new(),
            hits: 0,
            blasts: 0,
            last_blast: None,
            went_off: Vec::new(),
            struck: Vec::new(),
            picked: Picked::default(),
            second: Some(Weapon::Rpg),
            launcher_rounds: Some(Rounds::of(1 + RPG_SPARE, 1)),
            rifle: Rifle::default(),
            own_rifle: true,
            scripted_loadout: false,
            hero_damage: HeroDamage::default(),
            rifle_rounds: Rounds::of(RIFLE_CLIP + RIFLE_SPARE, RIFLE_CLIP),
            changing: None,
            grenades: MAX_GRENADES,
            designator: false,
            to_throw: false,
            thrown: Vec::new(),
            throwing_designator: false,
            last_launch: None,
            fuses: Vec::new(),
            blazes: Vec::new(),
            fires: Vec::new(),
            launcher: Rifle::launcher(Weapon::Rpg),
            carbine: Rifle::default(),
            rpg: Rifle::launcher(Weapon::Rpg),
            gl: Rifle::launcher(Weapon::GrenadeLauncher),
            damage_table: None,
            throwables: Vec::new(),
            explosive: 0,
            charges: 0,
            mines: 0,
            fire_held: false,
            lock: None,
            seat: 0,
            vehicle_gun: 0,
            ripple: 0,
            seed: 0x2545_f491_4f6c_dd1d,
            fired: Fired::default(),
            shots: std::collections::BTreeMap::new(),
            shooters: std::collections::BTreeMap::new(),
            flashes: (0, 0),
            cases: 0,
            impacts: (0, 0),
            impact_kinds: std::collections::BTreeMap::new(),
            tally: Tally::default(),
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
            Projectile::Charge {
                at,
                velocity,
                model,
                ..
            } => Some((Flying::Charge(model), at, velocity)),
        })
    }

    /// Takes up `weapon` (a launcher: the RPG or a grenade launcher) in
    /// the launcher's place, `rounds` with it, and has it up: what she
    /// carried there, if anything, she leaves, with its rounds (the
    /// RPG's are endless: none).
    pub fn equip(&mut self, weapon: Weapon, rounds: u32) -> Option<(Weapon, u32)> {
        let gun = match weapon {
            Weapon::GrenadeLauncher => self.gl.clone(),
            Weapon::Rpg => self.rpg.clone(),
            Weapon::Rifle => self.carbine.clone(),
        };
        self.equip_gun(gun, rounds).map(|(g, n)| (g.slot(), n))
    }

    /// Takes up gun `gun` with `rounds` (none: full, a clip and its spare
    /// clips): a gun that fires rounds in her rifle's place, a launcher in
    /// the launcher's. What she carried there she leaves, with its rounds.
    pub fn equip_gun(&mut self, gun: Rifle, rounds: u32) -> Option<(Rifle, u32)> {
        let slot = gun.slot();
        let clip = gun.clip.max(1);
        let rounds = if rounds == 0 {
            clip + gun.spare
        } else {
            rounds
        };
        self.weapon = slot;
        if slot == Weapon::Rifle {
            let kept = self.rifle_rounds.total();
            let old = std::mem::replace(&mut self.rifle, gun);
            self.rifle_rounds = Rounds::of(rounds.min(clip + self.rifle.spare), clip);
            self.own_rifle = false;
            self.changing = None;
            return Some((old, kept));
        }
        let kept = self.launcher_rounds.take().map_or(0, Rounds::total);
        let old = std::mem::replace(&mut self.launcher, gun);
        let left = self.second.map(|_| (old, kept));
        self.launcher_rounds = Some(Rounds::of(rounds, clip));
        self.second = Some(slot);
        self.lock = None;
        left
    }

    /// Gun `spec` as she takes it up: the carbine, RPG or grenade
    /// launcher she knows, else as its template says.
    pub fn stock(&self, spec: &WeaponSpec) -> Rifle {
        [&self.carbine, &self.rpg, &self.gl]
            .into_iter()
            .find(|g| g.template == Some(spec.template))
            .cloned()
            .unwrap_or_else(|| Rifle::from_spec(spec))
    }

    /// Hands her the game's weapons as their templates say (`arms`), and
    /// the damage table: the carbine, the RPG and the grenade launcher by
    /// name, and the grenade, C4 and proximity mine to throw.
    pub fn arm(&mut self, arms: &m2_engine::weapons::Arms, table: Option<DamageTable>) {
        self.damage_table = table;
        self.feels = arms.weapons.iter().map(|(&id, w)| (id, w.feel)).collect();
        if let Some(c) = arms.named("Carbine") {
            self.carbine = Rifle::from_spec(c);
            if !self.own_rifle {
                self.rifle = self.carbine.clone();
            }
        }
        if let Some(r) = arms.named("RPG") {
            self.rpg = Rifle::from_spec(r);
        }
        if let Some(g) = arms.named("Grenade Launcher") {
            // Its `MaxAmmoReserve` is 0; the footage's HUD shows the
            // crate's 3 clips as 6 loaded and 2 more, so 2 spare clips.
            let gl = Rifle::from_spec(g);
            self.gl = Rifle {
                spare: if gl.spare == 0 {
                    LAUNCHER_SPARE
                } else {
                    gl.spare
                },
                ..gl
            };
        }
        if self.second.is_some() {
            self.launcher = match self.launcher.slot() {
                Weapon::GrenadeLauncher => self.gl.clone(),
                _ => self.rpg.clone(),
            };
            let clip = self.launcher.clip.max(1);
            self.launcher_rounds = Some(Rounds::of(clip + self.launcher.spare, clip));
        }
        if let Some(g) = arms.thrown("Grenade") {
            self.set_thrown(
                g.clone(),
                arms.thrown("C4").cloned(),
                arms.thrown("Proximity Mine").cloned(),
            );
        }
    }

    /// Lands her with the carbine alone, as the first contract starts:
    /// its scripts give her a carbine, grenades and C4 afresh and fill
    /// only the carbine, so she has no grenades (none in the footage until
    /// a pickup).
    pub fn carbine_only(&mut self) {
        self.rifle = self.carbine.clone();
        self.own_rifle = false;
        self.rifle_rounds = Rounds::of(RIFLE_CLIP, RIFLE_CLIP);
        self.changing = None;
        self.second = None;
        self.launcher_rounds = None;
        self.grenades = 0;
        self.charges = 0;
        self.mines = 0;
        self.explosive = 0;
        self.weapon = Weapon::Rifle;
    }

    /// Hands her `rifle` in the rifle's place as the contract's scripts
    /// give it (PmcCon001's `Assault Rifle`): one clip in, nothing in
    /// reserve until the scripts give her some, and nothing else.
    pub fn scripted_rifle(&mut self, rifle: Rifle) {
        self.carbine_only();
        self.rifle_rounds = Rounds::of(rifle.clip, rifle.clip);
        self.rifle = rifle;
    }

    /// Has her carry her own rifle `rifle` (free roam), full: a clip in
    /// and its template's spare clips.
    pub fn take_own_rifle(&mut self, rifle: Rifle) {
        self.rifle_rounds = Rounds::of(rifle.clip + rifle.spare, rifle.clip);
        self.rifle = rifle;
        self.own_rifle = true;
        self.changing = None;
    }

    /// Rounds taken from a pickup for `weapon`: the grenade launcher's go
    /// with the one she carries (none, they are lost), the rifle's to its
    /// spare clips; the RPG's only add to what the HUD counts.
    pub fn take_ammo(&mut self, weapon: Weapon, rounds: u32) {
        match (weapon, &mut self.launcher_rounds) {
            (Weapon::Rifle, _) => {
                let r = &mut self.rifle_rounds;
                r.stored = (r.stored + rounds).min(self.rifle.spare);
            }
            (w, Some(r)) if self.second == Some(w) => {
                r.stored = (r.stored + rounds).min(self.launcher.spare)
            }
            (Weapon::GrenadeLauncher, _) => {}
            _ => self.picked.add(weapon, rounds),
        }
    }

    /// Her health as the health box shows it: of 100.
    pub fn hp_percent(&self) -> f32 {
        self.hp * 100.0 / self.max_hp.max(1.0)
    }

    /// What an ammunition pickup's `Pickup` records (each its ammunition
    /// template and amount) give her: each record goes to the first of her
    /// rifle, the gun in the launcher's place and her grenades that
    /// `takes` it (asked with the weapon's template, the ammunition it
    /// fires and the record's). A gun's amount counts its clips (the
    /// crate launcher's 3 are the footage's "6 2"); a thrown weapon's
    /// counts itself.
    pub fn refill(
        &self,
        records: &[(u32, u32)],
        takes: impl Fn(Option<u32>, Option<u32>, u32) -> bool,
    ) -> Refill {
        let gun = |g: &Rifle, ammo| takes(g.template, g.ammo.map(|a| a.template), ammo);
        let grenade = self.throwables.first().and_then(Option::as_ref);
        let mut out = Refill::default();
        for &(ammo, n) in records {
            if gun(&self.rifle, ammo) {
                out.rifle += n * self.rifle.clip.max(1);
            } else if self.second.is_some() && gun(&self.launcher, ammo) {
                out.launcher += n * self.launcher.clip.max(1);
            } else if grenade.is_some_and(|g| takes(Some(g.template), Some(g.ammo.template), ammo))
            {
                out.grenades += n;
            }
        }
        out
    }

    /// What a pickup gives her ([`refill`](Self::refill)), and its health:
    /// as much as she has room for, health up to full.
    pub fn take_supplies(&mut self, refill: Refill, health: f32) {
        if refill.rifle > 0 {
            self.take_ammo(Weapon::Rifle, refill.rifle);
        }
        if let Some(w) = self.second.filter(|_| refill.launcher > 0) {
            self.take_ammo(w, refill.launcher);
        }
        self.grenades = (self.grenades + refill.grenades).min(MAX_GRENADES);
        self.hp = (self.hp + health).min(self.max_hp);
    }

    /// Whether ammunition or health lying there gives her anything she
    /// has room for: what she walks over she takes only then.
    pub fn can_use(&self, refill: Refill, health: f32) -> bool {
        let rifle = refill.rifle > 0 && self.rifle_rounds.stored < self.rifle.spare;
        let launcher = refill.launcher > 0
            && self.second.is_some()
            && self
                .launcher_rounds
                .is_some_and(|r| r.stored < self.launcher.spare);
        rifle
            || launcher
            || (refill.grenades > 0 && self.grenades < MAX_GRENADES)
            || (health > 0.0 && self.hp < self.max_hp)
    }

    /// Why walking over a pickup whose `records` (ammunition template,
    /// amount) and `health` give her nothing she has room for leaves it
    /// lying, for the log: each record a weapon of hers takes (as
    /// [`refill`](Self::refill) asks `takes`) with that weapon's rounds and
    /// room, how many records none of hers takes and what hers fire, and
    /// her health against the most. `name` names a template.
    pub fn refusal(
        &self,
        records: &[(u32, u32)],
        health: f32,
        takes: impl Fn(Option<u32>, Option<u32>, u32) -> bool,
        name: impl Fn(u32) -> String,
    ) -> String {
        let gun = |g: &Rifle, ammo| takes(g.template, g.ammo.map(|a| a.template), ammo);
        let grenade = self.throwables.first().and_then(Option::as_ref);
        let mut why = Vec::new();
        let mut none = 0;
        for &(ammo, n) in records {
            if gun(&self.rifle, ammo) {
                let r = self.rifle_rounds;
                why.push(format!(
                    "{} {n} to her {}, full at {}+{} of {} spare",
                    name(ammo),
                    self.rifle.name,
                    r.clip,
                    r.stored,
                    self.rifle.spare
                ));
            } else if self.second.is_some() && gun(&self.launcher, ammo) {
                let r = self.launcher_rounds.unwrap_or_default();
                why.push(format!(
                    "{} {n} to her {}, full at {}+{} of {} spare",
                    name(ammo),
                    self.launcher.name,
                    r.clip,
                    r.stored,
                    self.launcher.spare
                ));
            } else if grenade.is_some_and(|g| takes(Some(g.template), Some(g.ammo.template), ammo))
            {
                why.push(format!(
                    "{} {n} to her grenades, full at {} of {MAX_GRENADES}",
                    name(ammo),
                    self.grenades
                ));
            } else {
                none += 1;
            }
        }
        if none > 0 {
            let fires = |g: &Rifle| g.ammo.map_or_else(|| "?".to_owned(), |a| name(a.template));
            why.push(format!(
                "{none} of {} records for no weapon of hers (her {} fires {}{}{})",
                records.len(),
                self.rifle.name,
                fires(&self.rifle),
                match self.second {
                    Some(_) => format!(
                        ", her {} fires {}",
                        self.launcher.name,
                        fires(&self.launcher)
                    ),
                    None => String::new(),
                },
                match grenade {
                    Some(g) => format!(", her grenades are {}", name(g.ammo.template)),
                    None => ", no grenades".to_owned(),
                }
            ));
        }
        if health > 0.0 {
            why.push(format!(
                "{health} health, she has {:.0} of {:.0}",
                self.hp, self.max_hp
            ));
        }
        if why.is_empty() {
            "it gives nothing walked over".to_owned()
        } else {
            why.join("; ")
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

    /// The weapon up in her hands as her clips see it: its carry class
    /// and the side of her back it hangs on (the rifle's right, the
    /// launcher's left; [`slung`](Self::slung)).
    pub fn in_hand(&self) -> crate::held::InHand {
        use crate::held::{InHand, Side};
        use m2_assets::animation_table::RIFLE;
        match self.weapon {
            Weapon::Rifle => InHand {
                class: self.rifle.class.unwrap_or(RIFLE),
                side: Side::Right,
            },
            _ => InHand {
                class: self
                    .launcher
                    .class
                    .unwrap_or_else(|| m2_assets::name_hash("rocket")),
                side: Side::Left,
            },
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

    /// Hurts her as a hit from `from` does, without the hit (for
    /// screenshots of the HUD's hit arcs). Leaves her on her feet.
    pub fn hurt_from(&mut self, damage: f32, from: Vec3) {
        if self.hurts.len() >= MAX_HURTS {
            self.hurts.remove(0);
        }
        self.hurts.push((damage, Some(from)));
        self.hp = (self.hp - damage).max(1.0);
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

    /// Shows `e` for `seconds`, for what isn't the fight's (a boat's
    /// spray).
    pub fn show(&mut self, e: Effect, seconds: f32) {
        self.effect(e, seconds);
    }

    /// A gun firing from `from` along `dir`, its template's `feel` if
    /// known: its muzzle flash (the game's effect its template names,
    /// else the alpha's own flash `size` metres long) and its shot.
    /// A gun of `feel` fired from `from` along `dir`: its flash, its spent
    /// case thrown out at `case` (its `ShellEjectTemplate`'s effect, where
    /// its model's case node is known), and its shot's sound: the hero's
    /// (`shooter` `None`) or someone else's, by their number.
    fn gun_fired(
        &mut self,
        feel: Option<Feel>,
        (from, case): (Vec3, Option<Vec3>),
        dir: Vec3,
        size: f32,
        shooter: Option<u32>,
    ) {
        if let (Some(shell), Some(at)) = (feel.and_then(|f| f.shell), case) {
            self.cases += 1;
            self.game_fx.push(GameFx {
                template: FxTemplate::Id(shell),
                at,
                hold: None,
            });
        }
        match feel.and_then(|f| f.flash) {
            Some(flash) => {
                self.flashes.0 += 1;
                self.game_fx.push(GameFx {
                    template: FxTemplate::Id(flash),
                    at: from,
                    hold: None,
                });
            }
            None => {
                self.flashes.1 += 1;
                self.effect(
                    Effect::Flash {
                        at: from,
                        dir,
                        size,
                    },
                    FLASH_TIME,
                );
            }
        }
        let set = feel.and_then(|f| f.sound_set);
        let npc = shooter.is_some();
        *self.shots.entry((set.unwrap_or(0), !npc)).or_default() += 1;
        if let Some(n) = shooter
            && self.shooters.entry(set.unwrap_or(0)).or_default().insert(n)
        {
            eprintln!(
                "combat: a soldier numbered {n} fires sound set {:08x}",
                set.unwrap_or(0)
            );
        }
        self.sounds.push(Sound::Shot {
            at: from,
            set,
            npc,
            shooter: shooter.unwrap_or(0),
        });
    }

    /// The shots fired so far by sound set, and the flashes, in a line.
    pub fn shots_line(&self) -> String {
        let sets: Vec<String> = self
            .shots
            .iter()
            .map(|(&(set, hero), n)| format!("{set:08x}{} {n}", if hero { " (hero)" } else { "" }))
            .collect();
        let shooters: Vec<String> = self
            .shooters
            .iter()
            .map(|(set, who)| format!("{set:08x} {}", who.len()))
            .collect();
        format!(
            "shots by sound set: {} (others firing: {}); flashes: {} the game's, {} the \
             alpha's own; cases {}; impacts {} by surface, {} generic ({})",
            if sets.is_empty() {
                "none".to_owned()
            } else {
                sets.join(", ")
            },
            if shooters.is_empty() {
                "none".to_owned()
            } else {
                shooters.join(", ")
            },
            self.flashes.0,
            self.flashes.1,
            self.cases,
            self.impacts.0,
            self.impacts.1,
            self.impact_kinds
                .iter()
                .map(|(t, n)| format!("{} {n}", t.trim_start_matches("global_particle_impact_")))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }

    /// A round from `from` along `dir` striking the world `t` metres on:
    /// it throws up the impact for what the triangle it struck is made of
    /// ([`m2_engine::surface::bullet_impact`]), and this is what it
    /// struck, if one of the scene's.
    fn strike_world(
        &mut self,
        physics: &Physics,
        from: Vec3,
        dir: Vec3,
        t: f32,
    ) -> Option<StaticKey> {
        let (key, surface) = physics
            .cast_ray_surface(from, dir, t + 0.05)
            .map_or((None, 0), |(_, _, key, surface)| (key, surface));
        let (template, named) = m2_engine::surface::bullet_impact(surface);
        if named {
            self.impacts.0 += 1;
        } else {
            self.impacts.1 += 1;
        }
        *self.impact_kinds.entry(template).or_default() += 1;
        self.show_fx(template, from + dir * (t - IMPACT_BACK).max(0.0));
        key
    }

    /// Sets off the effects template `name` shows at `at`, once.
    pub fn show_fx(&mut self, name: &str, at: Vec3) {
        self.game_fx.push(GameFx {
            template: FxTemplate::Name(name.to_owned()),
            at,
            hold: None,
        });
    }

    /// Keeps the effects template `name` shows going at `at` for source
    /// `key` (see [`GameFx`]).
    fn hold_fx(&mut self, name: &str, at: Vec3, key: u64) {
        self.game_fx.push(GameFx {
            template: FxTemplate::Name(name.to_owned()),
            at,
            hold: Some(key),
        });
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

    /// Something that doesn't come down as a building does breaks at
    /// `at`: heard as its template's `SoundKey` `key` says.
    pub fn broke(&mut self, at: Vec3, key: u32) {
        self.sounds.push(Sound::Break { at, key });
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
        self.mg_reload -= dt;
        self.aim_left -= dt;
        let feel = self.rifle.feel;
        self.bloom.settle(&feel, dt);
        self.quiet += dt;

        if self.down > 0.0 {
            self.down -= dt;
            if self.down <= 0.0 {
                self.hp = self.max_hp;
                self.revived = true;
                f.player.down = false;
                f.player.aiming = false;
            }
        } else if !self.scripted_health && self.quiet > REGEN_AFTER {
            self.hp = (self.hp + REGEN_RATE * dt).min(self.max_hp);
        }

        let on_foot = f.player.landed && f.player.driving.is_none() && self.down <= 0.0;
        // At an emplaced gun: it fires, and nothing she carries.
        let on_foot = match self.manned {
            Some(_) if f.player.driving.is_none() && self.down <= 0.0 => {
                if trigger.fire && self.reload <= 0.0 {
                    self.manned_fire(f);
                }
                false
            }
            _ => on_foot,
        };
        // Asked for a launcher: whichever she carries in its place.
        match (trigger.select, self.second) {
            (Some(Weapon::Rifle), _) => self.weapon = Weapon::Rifle,
            (Some(_), Some(w)) => self.weapon = w,
            _ => {}
        }
        f.player.launcher = self.weapon == Weapon::Rpg;
        f.player.in_hand = self.in_hand();
        // A clip changed starts her reload clip over her upper body, and
        // the rifle's change lasts as long as it (playtest 8: only the
        // reload's sound was heard).
        if self.change_clip(trigger, on_foot, dt)
            && let Some(length) = f.player.start_reload()
            && self.changing.is_some()
        {
            self.changing = Some(length);
        }
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
            // One shot a pull for a gun whose `FireType` says so.
            let semi = match self.weapon {
                Weapon::Rifle => self.rifle.semi,
                _ => self.launcher.semi,
            };
            let pulled = trigger.fire && !(semi && self.fire_held);
            if pulled && self.reload <= 0.0 && self.changing.is_none() {
                match self.weapon {
                    Weapon::Rifle => self.player_shoot(f),
                    Weapon::Rpg => self.launch(f),
                    Weapon::GrenadeLauncher => self.lob(f),
                }
            }
            if trigger.next_explosive && self.throwables.len() > 1 {
                // The next she carries any of, round to the grenades.
                let (n, at) = (self.throwables.len(), self.explosive);
                self.explosive = (1..=n)
                    .map(|k| (at + k) % n)
                    .find(|&i| self.throwables[i].is_some() && self.explosives(i) > 0)
                    .unwrap_or(0);
            }
            if trigger.detonate {
                self.detonate(f);
            }
            let left = self.explosives(self.explosive);
            if trigger.throw && self.reload <= 0.0 && (self.to_throw || left > 0) {
                if self.to_throw {
                    self.to_throw = false;
                    self.throwing_designator = true;
                } else {
                    *self.explosives_mut(self.explosive) -= 1;
                }
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
        match f.player.driving {
            Some(i) if trigger.seat => {
                let seats = f.vehicles.get(i).map_or(1, |v| armed_seats(v).len());
                self.seat = (self.seat + 1) % seats.max(1);
            }
            Some(_) => {}
            None => self.seat = 0,
        }
        if f.player.driving.is_none() {
            self.vehicle_gun = 0;
        }
        // Fire fires the gun chosen with the switch: the main gun, or the
        // machine gun (as Aim does too).
        let mg = self.vehicle_gun == 1;
        if let Some(i) = f.player.driving
            && self.down <= 0.0
            && trigger.fire
            && (if mg { self.mg_reload } else { self.reload }) <= 0.0
            && !self.mounted_fire(f, i, usize::from(mg))
        {
            self.vehicle_fire(f, i, mg);
        }
        if let Some(i) = f.player.driving
            && self.down <= 0.0
            && trigger.aim
            && self.mg_reload <= 0.0
            && !self.mounted_fire(f, i, 1)
        {
            self.vehicle_fire(f, i, true);
        }
        self.fire_held = trigger.fire;
        self.track_lock(f, dt);
        // Changing the clip, she lowers the rifle.
        let holding = on_foot && (trigger.aim || self.aim_left > 0.0) && self.changing.is_none();
        f.player.aiming = holding;

        // Soldiers' shots and grenades.
        let target = self.target(f.player, true);
        for i in 0..f.people.npcs.len() {
            if std::mem::take(&mut f.people.npcs[i].fires) {
                self.npc_shoot(f, i, target);
            }
            if let Some(at) = f.people.npcs[i].throws.take() {
                self.npc_throw(f, i, at);
            }
        }
        self.fly(f, dt);
        self.burn_vehicles(f, dt);
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
        Self::muzzle_and_case(f, weapon).0
    }

    /// A weapon's muzzle in her hands (or her chest), and where its spent
    /// cases leave it (its model's [`EJECT`] node), if it has one.
    fn muzzle_and_case(f: &Field<'_>, weapon: Option<&WeaponModel>) -> (Vec3, Option<Vec3>) {
        let chest = f.player.character.position + Vec3::Y * 1.4;
        let (Some(w), Some((_, rig))) = (weapon, &f.player.model) else {
            return (chest, None);
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
            .map_or((chest, None), |m| {
                (
                    m.transform_point3(w.muzzle),
                    w.eject.map(|e| m.transform_point3(e)),
                )
            })
    }

    /// The rifle's clip: changed when she asks or once it is empty (as the
    /// game does), while she holds it on foot with rounds to spare; put
    /// away or out of her hands, the change starts over next time. True
    /// when a change (the rifle's, or the launcher's) starts this tick.
    fn change_clip(&mut self, trigger: Trigger, on_foot: bool, dt: f32) -> bool {
        // The launcher reloads on R too, its magazine part spent
        // (playtest 4: only once empty).
        if on_foot
            && trigger.reload
            && self.weapon == self.launcher.slot()
            && self.reload <= 0.0
            && let Some(r) = self.launcher_rounds.as_mut()
            && let Some(full) = topped_up(*r, self.launcher.clip.max(1))
        {
            *r = full;
            self.reload = LAUNCHER_RELOAD;
            return true;
        }
        let r = self.rifle_rounds;
        let holding = on_foot && self.weapon == Weapon::Rifle && !self.designator;
        if !holding {
            self.changing = None;
            return false;
        }
        match &mut self.changing {
            Some(t) => {
                *t -= dt;
                if *t <= 0.0 {
                    self.changing = None;
                    self.rifle_rounds = Rounds::of(r.total(), self.rifle.clip);
                }
            }
            None if r.stored > 0 && r.clip < self.rifle.clip && (trigger.reload || r.clip == 0) => {
                self.changing = Some(RIFLE_RELOAD);
                self.sounds.push(Sound::Reload {
                    set: self.rifle.feel.sound_set,
                });
                return true;
            }
            None => {}
        }
        false
    }

    /// Fires the emplaced gun she mans ([`Manned`]): a round of its
    /// weapon's from its barrel along the crosshair, held within its turn,
    /// as a vehicle's machine gun's rounds do (people, vehicles, crates
    /// and explosive props).
    fn manned_fire(&mut self, f: &mut Field<'_>) {
        let Some(m) = &mut self.manned else {
            return;
        };
        if m.rounds == 0 {
            return;
        }
        m.rounds -= 1;
        let (_, dir) = Self::aim_ray(f.player);
        let aim = m.clamp(dir);
        let from = m.pivot + aim * 0.9;
        let w = &m.weapon;
        self.reload = w.gap();
        let feel = w.feel;
        let case = case_behind(f.arsenal, w.model, from, aim);
        let ammo = w.ammo;
        let round = match ammo.key {
            Some(k) if k == m2_assets::name_hash("Bullet") => Round::Bullet,
            _ => Round::BulletLarge,
        };
        // A recoilless rifle's rocket, a howitzer's shell: as a vehicle's.
        match w.flight() {
            m2_engine::weapons::Flight::Round => {}
            m2_engine::weapons::Flight::Lobbed => {
                let goal = Self::aim_point(f.physics, f.people, f.player, None);
                let speed = ammo.velocity.max(1.0);
                let velocity = lob_velocity(from, goal, speed, m2_engine::physics::GRAVITY)
                    .unwrap_or(aim * speed);
                self.projectiles.push(Projectile::Round {
                    at: from,
                    velocity,
                    flown: 0.0,
                    blast: ammo.burst(),
                });
                self.sounds.push(Sound::Lob {
                    at: from,
                    set: feel.sound_set,
                });
                alarm(f.people, from);
                return;
            }
            _ => {
                self.projectiles.push(Projectile::Rocket {
                    at: from,
                    velocity: aim * ammo.velocity.max(1.0),
                    flown: 0.0,
                    shell: false,
                    owner: None,
                    blast: ammo.burst(),
                    homing: None,
                    key: ammo.key,
                });
                self.effect(
                    Effect::Flash {
                        at: from,
                        dir: aim,
                        size: MG_FLASH,
                    },
                    FLASH_TIME,
                );
                self.sounds.push(Sound::Launch {
                    at: from,
                    set: feel.sound_set,
                });
                alarm(f.people, from);
                return;
            }
        }
        let hit = cast_shot(f.physics, f.people, from, aim, MG_RANGE, None, None);
        let to = hit.map_or(from + aim * MG_RANGE, |h| from + aim * h.0);
        let damage = ammo.damage_at(hit.map_or(MG_RANGE, |h| h.0));
        let on_props = RIFLE_DAMAGE * damage / CARBINE_ROUND;
        if let Some((t, None)) = hit {
            self.round_on_vehicle(f, (from, aim, t), (damage, round), None);
            self.hit_explosives_in(f.explosives, from + aim * t, on_props);
            let key = self.strike_world(f.physics, from, aim, t);
            self.struck.push(Strike {
                at: from + aim * t,
                damage: on_props,
                key,
                round: damage,
                kind: round_kind(ammo.key),
            });
        }
        if let Some((_, Some(j))) = hit
            && self.round_on_person(f.people, j, (damage, ammo.hero), ammo.key, aim * SHOT_PUSH)
        {
            self.remarks.push(Remark::Kill);
        }
        self.fired.rifle += 1;
        self.effect(Effect::Tracer { from, to }, 0.07);
        self.gun_fired(Some(feel), (from, case), aim, MG_FLASH, None);
        alarm(f.people, from);
    }

    fn player_shoot(&mut self, f: &mut Field<'_>) {
        if self.rifle_rounds.clip == 0 {
            return;
        }
        self.rifle_rounds.clip -= 1;
        self.fired.rifle += 1;
        self.reload = self.rifle.gap;
        self.aim_left = AIM_AFTER_SHOT;
        let (eye, aim) = Self::aim_ray(f.player);
        let (from, case) = Self::muzzle_and_case(f, f.arsenal.of_gun(&self.rifle));
        let ammo = self.rifle.ammo;
        let round = match ammo.and_then(|a| a.key) {
            Some(k) if k == m2_assets::name_hash("Bullet") || k == 0 => Round::Bullet,
            Some(_) => Round::BulletLarge,
            None => Round::Bullet,
        };
        // Each of its rounds a shot (the shotgun's 10), scattered over its
        // cone (degrees across: `ScatterMin`, more each shot held, to
        // `ScatterMax`; [`Feel::cone`]).
        // Each round anywhere in the round cone, evenly over it
        // ([`m2_engine::weapons::stray_in_cone`]); then her gun's aim
        // assist draws a round that strays within its window of an enemy
        // onto him (playtest 8: at range the cone alone left soldiers
        // nearly impossible to hit; the rifles' windows are widest far).
        let assist = self.rifle.assist;
        let cone = self.rifle.feel.cone(self.bloom.added);
        let feel = self.rifle.feel;
        self.bloom.shot(&feel);
        let mut killed = false;
        let mut first = None;
        for _ in 0..self.rifle.pellets.max(1) {
            let (a, b) = (self.random(), self.random());
            let dir = m2_engine::weapons::stray_in_cone(aim, cone, a, b);
            let dir = assist.map_or(dir, |s| assisted_aim(&s, f.people, eye, dir, RIFLE_RANGE));
            let hit = cast_shot(f.physics, f.people, eye, dir, RIFLE_RANGE, None, None);
            let to = hit.map_or(eye + dir * RIFLE_RANGE, |h| eye + dir * h.0);
            first.get_or_insert(to);
            // Its round's `Damage`, falling to `DamageMinimum` with range.
            let damage = ammo.map_or(self.rifle.damage, |a| {
                a.damage_at(hit.map_or(RIFLE_RANGE, |h| h.0))
            });
            // To crates and explosive props, the alpha's own scale.
            let on_props = RIFLE_DAMAGE * damage / CARBINE_ROUND;
            if let Some((t, None)) = hit {
                self.round_on_vehicle(f, (eye, dir, t), (damage, round), None);
                self.hit_explosives_in(f.explosives, eye + dir * t, on_props);
                // What it hit, if the scene's (a crate takes the round),
                // and the impact it throws up.
                let key = self.strike_world(f.physics, eye, dir, t);
                self.struck.push(Strike {
                    at: eye + dir * t,
                    damage: on_props,
                    key,
                    round: damage,
                    kind: round_kind(ammo.and_then(|a| a.key)),
                });
            }
            // What it does to people: its `Damage` times the damage
            // table's cell for their `ModifierKey` and its `DamageKey`,
            // against their template's `Health`.
            if let Some((_, Some(i))) = hit {
                let key = ammo.and_then(|a| a.key);
                let hero = ammo.map_or(1.0, |a| a.hero);
                killed |= self.round_on_person(f.people, i, (damage, hero), key, dir * SHOT_PUSH);
            }
            if self.rifle.pellets > 1 {
                self.effect(Effect::Tracer { from, to }, 0.05);
            }
        }
        let to = first.unwrap_or(eye + aim * RIFLE_RANGE);
        if self.rifle.pellets <= 1 {
            self.effect(Effect::Tracer { from, to }, 0.07);
        }
        let feel = self.rifle.feel;
        self.gun_fired(
            Some(feel),
            (from, case),
            (to - from).normalize_or(aim),
            RIFLE_FLASH,
            None,
        );
        if killed {
            self.remarks.push(Remark::Kill);
        }
        alarm(f.people, f.player.character.position);
    }

    /// A soldier's grenade lobbed from over his head down onto `at` (as
    /// hard as she throws), or as far as it goes that way.
    fn npc_throw(&mut self, f: &mut Field<'_>, i: usize, at: Vec3) {
        let n = &f.people.npcs[i];
        let from = n.character.position + Vec3::Y * 1.8;
        eprintln!("grenade: a soldier throws from {from:.1} at {at:.1}");
        let gravity = m2_engine::physics::GRAVITY;
        let velocity =
            lob_velocity(from, at, THROW, gravity).unwrap_or_else(|| farthest(at - from, THROW));
        self.projectiles.push(Projectile::Grenade {
            at: from,
            velocity,
            fuse: FUSE,
            blast: None,
        });
    }

    fn npc_shoot(&mut self, f: &mut Field<'_>, i: usize, target: Option<Vec3>) {
        let Some(aim) = target else { return };
        let n = &f.people.npcs[i];
        let gun = n.gun;
        let shooter = n.shooter;
        let (from, case) = match (f.arsenal.npc_gun(gun), f.people.kinds.get(n.kind)) {
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
                    .map_or((n.chest(), None), |m| {
                        (
                            m.transform_point3(w.muzzle),
                            w.eject.map(|e| m.transform_point3(e)),
                        )
                    })
            }
            _ => (n.chest(), None),
        };
        let distance = from.distance(aim);
        // His gun's cone (`LowSkillScatter`) and a miss of his own by
        // range and `Accuracy` ([`SoldierSpread`]).
        let cone = gun
            .and_then(|g| self.feels.get(&g.template))
            .map_or(NPC_CONE, |f| f.scatter[0]);
        let spread = SoldierSpread::new(distance, n.accuracy(), cone);
        let (a, b) = (self.random(), self.random());
        let dir = spread.stray((aim - from).normalize_or(Vec3::Z), a, b);
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
        let feel = gun.and_then(|g| self.feels.get(&g.template).copied());
        self.gun_fired(feel, (from, case), dir, RIFLE_FLASH, Some(shooter));
        let closest = from + dir * (aim - from).dot(dir).max(0.0);
        let blocked = hit.is_some_and(|h| h.0 < distance - 1.0);
        if f.player.driving.is_none() && !blocked && closest.distance(aim) < 0.45 {
            let damage = self.npc_round(gun);
            let lost = self.hurt_player(f, damage, Some(from));
            self.tally.taken_rounds += lost;
            self.tally.rounds_on_her += u32::from(lost > 0.0);
        }
        if let Some((t, None)) = hit {
            self.round_on_vehicle(f, (from, dir, t), RIFLE_ON_VEHICLE, None);
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

    /// Vehicles running into her on foot: one moving faster than
    /// [`crate::npc::RUN_OVER`] whose hull (padded as a person's is,
    /// [`crate::npc::People::traffic`]) holds her hurts her by its ram times
    /// the damage table's cell for her row, `Hero`, and its ram's kind,
    /// `Car` ([`m2_engine::durability::run_over`]: the `Hero` row takes a
    /// fifth of a `Car`'s; from the data), at most once a
    /// [`crate::npc::RAM_GAP`]. What she takes is the table's; when the
    /// ram counts, its size and the gap are invented, as for people.
    pub fn vehicles_hit_her(&mut self, f: &mut Field<'_>, dt: f32) {
        self.rammed -= dt;
        // Only on foot: seated, her own vehicle's hull holds her (playtest
        // 5: driving hurt her once a second, 22 times in one drive).
        if self.rammed > 0.0 || f.player.driving.is_some() {
            return;
        }
        let hero = m2_assets::name_hash("Hero");
        let kind = m2_assets::name_hash(m2_engine::durability::RAM_KIND);
        let her = f.player.character.position + Vec3::Y * 0.9;
        let mut hit = None;
        for v in f.vehicles.iter() {
            let Some(id) = v.id else { continue };
            let Some(at) = f.physics.vehicle_transform(id) else {
                continue;
            };
            let velocity = f.physics.vehicle_velocity(id);
            let speed = velocity.length();
            if speed < crate::npc::RUN_OVER {
                continue;
            }
            let (lo, hi) = v.spec.hull;
            if crate::npc::in_ram(at.inverse().transform_point3(her), (lo, hi)) {
                let damage = self
                    .damage_table
                    .as_ref()
                    .map_or(m2_engine::durability::ram(v.spec.mass, speed), |t| {
                        m2_engine::durability::run_over(t, hero, kind, v.spec.mass, speed)
                    });
                hit = Some((damage, at.transform_point3((lo + hi) * 0.5)));
                break;
            }
        }
        if let Some((damage, from)) = hit {
            let lost = self.hurt_player(f, damage, Some(from));
            self.rammed = crate::npc::RAM_GAP;
            self.run_overs.0 += 1;
            self.run_overs.1 += lost;
            eprintln!("run over: a vehicle ran into her for {damage:.1} ({lost:.1} health lost)");
        }
    }

    /// `damage` done to her from `from`: the health she lost.
    fn hurt_player(&mut self, f: &mut Field<'_>, damage: f32, from: Option<Vec3>) -> f32 {
        if self.down > 0.0 || self.invincible {
            return 0.0;
        }
        // Kept for the HUD; only the latest if nothing reads them.
        if self.hurts.len() >= MAX_HURTS {
            self.hurts.remove(0);
        }
        self.hurts.push((damage, from));
        self.quiet = 0.0;
        let floor = if self.health_clamp { CLAMP_FLOOR } else { 0.0 };
        let before = self.hp;
        self.hp = (self.hp - damage).max(floor.min(self.hp));
        let lost = (before - self.hp).max(0.0);
        if self.hp <= 0.0 {
            self.tally.downs += 1;
            self.remarks.push(Remark::Down);
            self.down = DOWN_TIME;
            f.player.down = true;
            f.player.aiming = false;
            if let (Some(die), Some((_, rig))) = (f.player.clips.die, &mut f.player.model) {
                rig.animator.set_upper(None);
                rig.two_handed = false;
                rig.animator.play_once(die);
            }
            f.people.stand_down();
        } else {
            self.remarks.push(Remark::Hurt);
        }
        lost
    }

    /// Takes a round from the launcher's clip, loading the next clip from
    /// the stored rounds once it is spent: false with none to fire.
    fn spend_launcher_round(&mut self) -> bool {
        let clip = self.launcher.clip.max(1);
        let Some(rounds) = self.launcher_rounds.as_mut() else {
            return false;
        };
        if rounds.clip == 0 {
            if rounds.stored > 0 {
                *rounds = Rounds::of(rounds.stored, clip);
                self.reload = LAUNCHER_RELOAD;
            }
            return false;
        }
        rounds.clip -= 1;
        self.reload = self.launcher.gap;
        if rounds.clip == 0 && rounds.stored > 0 {
            *rounds = Rounds::of(rounds.stored, clip);
            self.reload = self.reload.max(LAUNCHER_RELOAD);
        }
        true
    }

    /// A homing launcher locks on the vehicle the crosshair stays on for
    /// its template's `LockOnTime`, within its lock range.
    fn track_lock(&mut self, f: &Field<'_>, dt: f32) {
        let Some((time, range)) = self.launcher.lock_on.filter(|_| {
            self.weapon == self.launcher.slot() && self.second.is_some() && f.player.aiming
        }) else {
            self.lock = None;
            return;
        };
        let (eye, dir) = Self::aim_ray(f.player);
        let range = if range > 0.0 { range } else { ROCKET_RANGE };
        let on = f
            .physics
            .cast_ray_vehicle(eye, dir, range, None)
            .and_then(|(_, id)| id);
        self.lock = match (on, self.lock) {
            (Some(id), Some((was, t))) if was == id => Some((id, (t + dt).min(time))),
            (Some(id), _) => Some((id, 0.0)),
            (None, _) => None,
        };
    }

    /// The vehicle a homing launcher has locked on, once held long enough.
    pub fn locked(&self) -> Option<VehicleId> {
        let (time, _) = self.launcher.lock_on?;
        self.lock.filter(|&(_, t)| t >= time).map(|(id, _)| id)
    }

    fn launch(&mut self, f: &mut Field<'_>) {
        if !self.spend_launcher_round() {
            return;
        }
        self.fired.rockets += 1;
        self.aim_left = AIM_AFTER_SHOT * 2.0;
        let (eye, dir) = Self::aim_ray(f.player);
        let from = Self::muzzle(f, f.arsenal.of_gun(&self.launcher));
        // Aim the rocket from the launcher at what the crosshair is on.
        let goal = cast_shot(f.physics, f.people, eye, dir, ROCKET_RANGE, None, None)
            .map_or(eye + dir * ROCKET_RANGE, |h| eye + dir * h.0);
        // Its template's speed (`Velocity` times `HumanBoost`: the RPG
        // rocket's 25 x 2).
        let velocity = (goal - from).normalize_or(dir) * self.launcher.speed(ROCKET_SPEED);
        let homing = self.locked().zip(
            self.launcher
                .ammo
                .and_then(|a| a.homing)
                .map(|(turn, _)| turn.to_radians()),
        );
        self.last_launch = Some((from, goal));
        self.projectiles.push(Projectile::Rocket {
            at: from,
            velocity,
            flown: 0.0,
            shell: false,
            owner: None,
            blast: self.launcher.blast(),
            homing,
            key: self.launcher.ammo.and_then(|a| a.key),
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
        self.show_fx(fx::BACK_BLAST, from);
        self.sounds.push(Sound::Launch {
            at: from,
            set: self.launcher.feel.sound_set,
        });
        alarm(f.people, f.player.character.position);
    }

    /// Fires the grenade launcher at what the crosshair is on: the round
    /// is lobbed to come down there when it can reach (the lower arc),
    /// else as far as it goes that way. The template gives the round's
    /// speed and fall, not how the game aims it. An empty clip loads from
    /// the stored rounds; with none, nothing fires.
    fn lob(&mut self, f: &mut Field<'_>) {
        if !self.spend_launcher_round() {
            return;
        }
        self.aim_left = AIM_AFTER_SHOT * 2.0;
        let (eye, dir) = Self::aim_ray(f.player);
        let from = Self::muzzle(
            f,
            f.arsenal
                .of_gun(&self.launcher)
                .or(f.arsenal.launcher.as_ref())
                .or(f.arsenal.rifle.as_ref()),
        );
        let goal = cast_shot(f.physics, f.people, eye, dir, ROCKET_RANGE, None, None)
            .map_or(eye + dir * ROCKET_RANGE, |h| eye + dir * h.0);
        let gravity = m2_engine::physics::GRAVITY;
        let speed = self.launcher.speed(LOB_SPEED);
        let velocity = lob_velocity(from, goal, speed, gravity)
            .unwrap_or_else(|| farthest(goal - from, speed));
        self.projectiles.push(Projectile::Round {
            at: from,
            velocity,
            flown: 0.0,
            blast: self.launcher.blast(),
        });
        self.effect(
            Effect::Flash {
                at: from,
                dir: velocity.normalize_or(dir),
                size: LOB_FLASH,
            },
            FLASH_TIME * 1.4,
        );
        self.sounds.push(Sound::Lob {
            at: from,
            set: self.launcher.feel.sound_set,
        });
        alarm(f.people, f.player.character.position);
    }

    /// Fires the gun on the turret of vehicle `i`, which she drives, at
    /// what the crosshair is on: a cannon (a tank's, the Stryker's) fires
    /// a shell that bursts like a rocket, anything else's a machine gun.
    /// Fires the gun of vehicle `i` she drives, or with `coax` the machine
    /// gun beside a tank's cannon (along it).
    /// Vehicle `i`'s weapon of `priority` for the seat she rides, as its
    /// template mounts it: rounds or rockets from its hardpoints in turn,
    /// at the crosshair, by its weapon's and ammunition's templates. A
    /// tank's driver fires its cannon as [`Combat::vehicle_fire`] does.
    /// False if that seat has no such weapon.
    fn mounted_fire(&mut self, f: &mut Field<'_>, i: usize, priority: usize) -> bool {
        let Some(v) = f.vehicles.get(i) else {
            return false;
        };
        let seats = armed_seats(v);
        let Some(&seat) = seats.get(self.seat) else {
            return false;
        };
        if self.seat == 0 && v.spec.cannon() {
            return false;
        }
        let Some(m) = v
            .spec
            .armed
            .iter()
            .find(|m| m.seat == seat && m.priority == priority)
            .cloned()
        else {
            return false;
        };
        let (Some(id), false) = (v.id, v.wrecked) else {
            return true;
        };
        let Some(body) = f.physics.vehicle_transform(id) else {
            return true;
        };
        // Its hardpoints in turn, where the model has them (on the turret,
        // turned with it); else the gun's muzzle, else its nose.
        let node = m
            .nodes
            .get(self.ripple % m.nodes.len().max(1))
            .copied()
            .flatten();
        self.ripple += 1;
        let (lo, hi) = v.spec.bounds;
        let local = match node {
            Some(n) => {
                let at = v.spec.rest[n].w_axis.truncate();
                if v.spec.turret.get(n).copied().unwrap_or(false) {
                    v.spec.turret_transform(v.turret).transform_point3(at)
                } else {
                    at
                }
            }
            None => v
                .spec
                .muzzle_pose(v.turret, v.gun)
                .map_or(Vec3::new(0.0, (lo.y + hi.y) * 0.5, hi.z), |(p, _)| p),
        };
        let from = body.transform_point3(local);
        let goal = Self::aim_point(f.physics, f.people, f.player, Some(id));
        let forward = body.transform_vector3(Vec3::Z).normalize_or(Vec3::Z);
        let aim = (goal - from).normalize_or(forward);
        let w = &m.weapon;
        if self.ripple == 1 {
            eprintln!(
                "vehicle fire: {} ({:?}, {:.0} a minute) from node {node:?} at {from:.2}, {} hardpoints, ammo {:.0} m/s {:?}",
                w.name,
                w.flight(),
                w.rate_of_fire,
                m.nodes.len(),
                w.ammo.velocity,
                w.ammo.burst().map(|b| (b.radius, b.damage))
            );
        }
        let gap = w.gap();
        if priority == 0 {
            self.reload = gap;
        } else {
            self.mg_reload = gap;
        }
        let ammo = w.ammo;
        match w.flight() {
            m2_engine::weapons::Flight::Round => {
                for _ in 0..w.pellets.max(1) {
                    let hit = cast_shot(f.physics, f.people, from, aim, MG_RANGE, None, Some(id));
                    let to = hit.map_or(from + aim * MG_RANGE, |h| from + aim * h.0);
                    self.effect(Effect::Tracer { from, to }, 0.07);
                    let damage = ammo.damage_at(hit.map_or(MG_RANGE, |h| h.0));
                    let on_props = RIFLE_DAMAGE * damage / CARBINE_ROUND;
                    if let Some((_, Some(j))) = hit
                        && self.round_on_person(
                            f.people,
                            j,
                            (damage, ammo.hero),
                            ammo.key,
                            aim * SHOT_PUSH,
                        )
                    {
                        self.remarks.push(Remark::Kill);
                    }
                    if let Some((t, None)) = hit {
                        let round = match ammo.key {
                            Some(k) if k == m2_assets::name_hash("Bullet") => Round::Bullet,
                            _ => Round::BulletLarge,
                        };
                        self.round_on_vehicle(f, (from, aim, t), (damage, round), Some(id));
                        self.hit_explosives_in(f.explosives, from + aim * t, on_props);
                    }
                }
                let case = case_behind(f.arsenal, w.model, from, aim);
                self.gun_fired(Some(w.feel), (from, case), aim, MG_FLASH, None);
            }
            m2_engine::weapons::Flight::Lobbed => {
                let speed = ammo.velocity.max(1.0);
                let velocity = lob_velocity(from, goal, speed, m2_engine::physics::GRAVITY)
                    .unwrap_or(aim * speed);
                self.projectiles.push(Projectile::Round {
                    at: from,
                    velocity,
                    flown: 0.0,
                    blast: ammo.burst(),
                });
                self.sounds.push(Sound::Lob {
                    at: from,
                    set: w.feel.sound_set,
                });
            }
            _ => {
                self.projectiles.push(Projectile::Rocket {
                    at: from,
                    velocity: aim * ammo.velocity.max(1.0),
                    flown: 0.0,
                    shell: false,
                    owner: Some(id),
                    blast: ammo.burst(),
                    homing: None,
                    key: ammo.key,
                });
                self.effect(
                    Effect::Flash {
                        at: from,
                        dir: aim,
                        size: MG_FLASH,
                    },
                    FLASH_TIME,
                );
                self.sounds.push(Sound::Launch {
                    at: from,
                    set: w.feel.sound_set,
                });
            }
        }
        alarm(f.people, from);
        true
    }

    fn vehicle_fire(&mut self, f: &mut Field<'_>, i: usize, coax: bool) {
        let Some(v) = f.vehicles.get(i) else {
            return;
        };
        let (Some(id), Some(muzzle), false) = (v.id, v.spec.muzzle, v.wrecked) else {
            return;
        };
        let cannon = v.spec.cannon();
        // A cannon with no shells left is silent; only a tank has a
        // machine gun beside its gun.
        if (!coax && v.rounds() == Some(0)) || (coax && !cannon) {
            return;
        }
        let Some(body) = f.physics.vehicle_transform(id) else {
            return;
        };
        let gun = v.spec.tuning.gun;
        // Its machine gun as its template arms it (the AMX-30's "Mounted
        // Heavy MG (Tank) (VZ)"; see
        // [`m2_engine::weapons::machine_gun`]). A template that mounts no
        // such gun has none to fire.
        let mg = match m2_engine::weapons::machine_gun(&v.spec.armed) {
            m2_engine::weapons::MachineGun::Mounted(w) => Some(w.clone()),
            m2_engine::weapons::MachineGun::Unmounted if coax || !cannon => return,
            _ => None,
        };
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
        if cannon && !coax {
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
                blast: None,
                homing: None,
                key: None,
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
            self.show_fx(fx::CANNON, from + aim * 0.8);
            self.sounds.push(Sound::Cannon {
                at: from,
                set: gun
                    .and_then(|g| self.feels.get(&g.weapon))
                    .and_then(|f| f.sound_set),
            });
        } else if let Some(w) = mg {
            // A tank's machine gun carries its template's clip and no more
            // (300 rounds, counted down in the gun box); run out, it is
            // silent, as the cannon is.
            if cannon {
                match f.vehicles.get_mut(i).filter(|v| v.machine_gun().is_some()) {
                    Some(v) if v.machine_gun().is_some_and(|(_, left)| left == 0) => return,
                    Some(v) => v.mg_fired += 1,
                    None => {}
                }
            }
            // Its own machine gun's rate, round and look and sound.
            if coax {
                self.mg_reload = w.gap();
            } else {
                self.reload = w.gap();
            }
            let ammo = w.ammo;
            let hit = cast_shot(f.physics, f.people, from, aim, MG_RANGE, None, Some(id));
            let to = hit.map_or(from + aim * MG_RANGE, |h| from + aim * h.0);
            self.effect(Effect::Tracer { from, to }, 0.07);
            let damage = ammo.damage_at(hit.map_or(MG_RANGE, |h| h.0));
            if let Some((_, Some(j))) = hit
                && self.round_on_person(f.people, j, (damage, ammo.hero), ammo.key, aim * SHOT_PUSH)
            {
                self.remarks.push(Remark::Kill);
            }
            if let Some((t, None)) = hit {
                let round = match ammo.key {
                    Some(k) if k == m2_assets::name_hash("Bullet") => Round::Bullet,
                    _ => Round::BulletLarge,
                };
                self.round_on_vehicle(f, (from, aim, t), (damage, round), Some(id));
                self.hit_explosives_in(
                    f.explosives,
                    from + aim * t,
                    RIFLE_DAMAGE * damage / CARBINE_ROUND,
                );
            }
            let case = case_behind(f.arsenal, w.model, from, aim);
            self.gun_fired(Some(w.feel), (from, case), aim, MG_FLASH, None);
        } else {
            if coax {
                self.mg_reload = TANK_MG_GAP;
            } else {
                self.reload = MG_GAP;
            }
            let hit = cast_shot(f.physics, f.people, from, aim, MG_RANGE, None, Some(id));
            let to = hit.map_or(from + aim * MG_RANGE, |h| from + aim * h.0);
            self.effect(Effect::Tracer { from, to }, 0.07);
            // A vehicle's machine gun with no template read: the alpha's
            // own flash and the rifle's shot.
            self.gun_fired(None, (from, None), aim, MG_FLASH, None);
            if let Some((_, Some(j))) = hit {
                let soldier = f.people.npcs[j].soldier;
                if f.people.npcs[j].hurt(MG_DAMAGE, aim * SHOT_PUSH) && soldier {
                    self.remarks.push(Remark::Kill);
                }
                self.hits += 1;
            }
            if let Some((t, None)) = hit {
                self.round_on_vehicle(f, (from, aim, t), MG_ON_VEHICLE, Some(id));
                self.hit_explosives_in(f.explosives, from + aim * t, MG_DAMAGE);
            }
        }
        alarm(f.people, from);
    }

    /// Vehicles hurt below their `TickDamage` health burn down until they
    /// are wrecked.
    fn burn_vehicles(&mut self, f: &mut Field<'_>, dt: f32) {
        for i in 0..f.vehicles.len() {
            let v = &mut f.vehicles[i];
            let Some(below) = v.spec.tuning.burns_below else {
                continue;
            };
            if !v.wrecked && v.damage > 0.0 && v.most_health() - v.damage <= below {
                self.hurt_vehicle(f.physics, v, i, BURN_DOWN * dt);
            }
        }
    }

    /// Wrecks vehicle `i`, `v`, if it isn't already: it burns and can't be
    /// driven.
    fn wreck(&mut self, physics: &mut Physics, v: &mut ParkedVehicle, i: usize) {
        if v.wrecked {
            return;
        }
        v.wrecked = true;
        if let Some(id) = v.id {
            physics.wreck_vehicle(id);
        }
        self.wrecks.push((i, 0.0));
        self.remarks.push(Remark::Wreck);
    }

    /// Vehicle `i`, `v`, takes `damage`: wrecked once it has taken its
    /// health.
    fn hurt_vehicle(
        &mut self,
        physics: &mut Physics,
        v: &mut ParkedVehicle,
        i: usize,
        damage: f32,
    ) {
        if v.wrecked {
            return;
        }
        v.damage += damage;
        if v.damage >= v.most_health() {
            self.wreck(physics, v, i);
        }
    }

    /// What a hit of `DamageKey` `key` at `at` (world) does to vehicle `v`
    /// (its place `body`), times its usual: where it strikes a part with a
    /// `ModifierKey` of its own (a tank's weak point), the damage table's
    /// [`weak_point`](DamageTable::weak_point) scale, else 1.
    fn weak_point(&self, v: &ParkedVehicle, body: Mat4, at: Vec3, key: Option<u32>) -> f32 {
        let (Some(table), Some(key), Some(whole)) =
            (self.damage_table.as_ref(), key, v.spec.tuning.key)
        else {
            return 1.0;
        };
        v.struck_key(body, at)
            .and_then(|part| table.weak_point(whole, part, key))
            .unwrap_or(1.0)
    }

    /// A round from `from` along `dir` that struck something solid `t`
    /// metres on: a vehicle (not `own`) takes `damage`.
    fn round_on_vehicle(
        &mut self,
        f: &mut Field<'_>,
        (from, dir, t): (Vec3, Vec3, f32),
        (damage, round): (f32, Round),
        own: Option<VehicleId>,
    ) {
        let Some((_, Some(id))) = f.physics.cast_ray_vehicle(from, dir, t + 0.05, own) else {
            return;
        };
        if let Some(i) = f.vehicles.iter().position(|v| v.id == Some(id)) {
            let body = f.physics.vehicle_transform(id);
            let key = m2_assets::name_hash(match round {
                Round::Bullet => "Bullet",
                Round::BulletLarge => "BulletLarge",
            });
            let v = &mut f.vehicles[i];
            let at = from + dir * t;
            let mut scale = v.spec.tuning.rounds.map_or(0.0, |r| match round {
                Round::Bullet => r.bullet,
                Round::BulletLarge => r.bullet_large,
            });
            // A part with a key of its own (a helicopter's weak point)
            // takes its own row's scale.
            if let (Some(body), Some(table)) = (body, self.damage_table.as_ref())
                && let Some(part) = v.struck_key(body, at)
                && let Some(own) = table.scale(part, key)
            {
                scale = own;
            }
            if damage * scale > 0.0 {
                // The part it struck takes the round's own damage (the
                // table's scale is the whole vehicle's; invented).
                if let Some(body) = body {
                    v.hit_parts(body, at, damage);
                }
                self.hurt_vehicle(f.physics, v, i, damage * scale);
            }
        }
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
    /// (a supply crate breaks open), and rights an overturned vehicle
    /// there.
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
            // Of the blow, what the damage table's row for them gives of a
            // `MeleeBash`: a thousand times for a `Human` or `HumanArmor`
            // (one blow fells any soldier), nothing for the bosses and
            // targets (`cab45462`: Carmona, Blanco, Peng).
            let taken = BASH_DAMAGE * self.on_person(n, Some(m2_assets::name_hash("MeleeBash")));
            let soldier = n.soldier;
            if n.hurt(taken, push) && soldier {
                self.remarks.push(Remark::Kill);
            }
            self.hits += 1;
        }
        // A vehicle on its side or roof in front of her: the blow rights it
        // (from the owner's play of the game; nothing in the data says so).
        for v in f
            .vehicles
            .iter()
            .filter(|v| !v.wrecked && f.player.driving.is_none())
        {
            let Some(id) = v.id else { continue };
            let Some(m) = f.physics.vehicle_transform(id) else {
                continue;
            };
            let (lo, hi) = v.model.bounds;
            let middle = m.transform_point3((lo + hi) * 0.5);
            let up = m.transform_vector3(Vec3::Y).normalize_or_zero();
            if within_bash(feet, ahead, middle, (hi - lo).max_element() * 0.5) && overturned(up) {
                f.physics.right_vehicle(id);
                eprintln!("bash: she rights {} (up {:.2})", v.spec.label, up.y);
                break;
            }
        }
        if let Some((t, _, key)) = f.physics.cast_ray_keyed(from, ahead, BASH_REACH) {
            self.struck.push(Strike {
                at: from + ahead * t,
                damage: BASH_BREAK,
                key,
                round: BASH_DAMAGE,
                kind: m2_assets::name_hash("MeleeBash"),
            });
        }
    }

    /// Sets off something that blows up where it stood, at `at` (`size`
    /// metres about its middle): a state layer's explosive barrel worn
    /// down ([`crate::destruct::Destruction::set_off`]). It goes off a
    /// moment later and burns, as the island's explosives do.
    pub fn set_off(&mut self, at: Vec3, size: f32) {
        self.fuses.push((at, CHAIN_DELAY));
        self.fires.push((at, size, BURN, 0.0));
    }

    /// A round of her rifle's landing at `at` on `key`, as a shot of hers
    /// strikes the world (the props check's).
    pub fn rifle_strike(&self, at: Vec3, key: Option<StaticKey>) -> Strike {
        Strike {
            at,
            damage: RIFLE_DAMAGE * self.rifle.damage / CARBINE_ROUND,
            key,
            round: self.rifle.damage,
            kind: round_kind(self.rifle.ammo.and_then(|a| a.key)),
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
        self.fired.grenades += 1;
        let (_, dir) = Self::aim_ray(f.player);
        let from = hand
            .unwrap_or_else(|| f.player.character.position + Vec3::Y * 1.6 + dir.with_y(0.0) * 0.4);
        if std::mem::take(&mut self.throwing_designator) {
            let velocity = (dir + Vec3::Y * 0.35).normalize_or(Vec3::Y) * THROW;
            self.thrown.push(Thrown {
                guid: None,
                at: from,
                velocity,
                age: 0.0,
                settled: false,
            });
            return;
        }
        let Some(t) = self.throwables.get(self.explosive).cloned().flatten() else {
            let velocity = (dir + Vec3::Y * 0.35).normalize_or(Vec3::Y) * THROW;
            self.projectiles.push(Projectile::Grenade {
                at: from,
                velocity,
                fuse: FUSE,
                blast: None,
            });
            return;
        };
        // Its template's throw: faster the higher she aims (the grenade's
        // 9, 13 and 16 m/s at -45, 0 and 45 degrees), leaving `ThrowAngle`
        // above the aim.
        let pitch = dir.y.clamp(-1.0, 1.0).asin().to_degrees();
        let up = (pitch + t.throw_angle).clamp(-80.0, 80.0).to_radians();
        let flat = dir.with_y(0.0).normalize_or(Vec3::Z);
        let velocity = (flat * up.cos() + Vec3::Y * up.sin()) * t.speed_at(pitch);
        let timer = t.ammo.timer.map(|(s, _)| s);
        if t.detonator.is_some() || t.planted.is_some() {
            self.projectiles.push(Projectile::Charge {
                at: from,
                velocity,
                model: t.ammo.model.or(t.model),
                stuck: false,
                blast: t.ammo.burst(),
                timer: timer.unwrap_or(f32::INFINITY),
                mine: t.planted.map(|(planted, reach)| Mine {
                    arm: timer.unwrap_or(0.0),
                    reach,
                    life: planted.timer.map_or(f32::INFINITY, |(s, _)| s),
                    blast: planted.burst(),
                }),
            });
        } else {
            self.projectiles.push(Projectile::Grenade {
                at: from,
                velocity,
                fuse: timer.unwrap_or(FUSE),
                blast: t.ammo.burst(),
            });
        }
    }

    /// How many of explosive `i` she has (0 grenades, 1 C4, 2 mines).
    fn explosives_mut(&mut self, i: usize) -> &mut u32 {
        match i {
            1 => &mut self.charges,
            2 => &mut self.mines,
            _ => &mut self.grenades,
        }
    }

    /// How many of explosive `i` she has.
    fn explosives(&self, i: usize) -> u32 {
        match i {
            1 => self.charges,
            2 => self.mines,
            _ => self.grenades,
        }
    }

    /// The explosive she throws now: its name, and how many she has.
    pub fn explosive_left(&self) -> (&str, u32) {
        let n = self.explosives(self.explosive);
        let name = self
            .throwables
            .get(self.explosive)
            .and_then(Option::as_ref)
            .map_or("Grenade", |t| &t.name);
        (name, n)
    }

    /// Hands her the game's thrown weapons, as their templates say: the
    /// grenade, C4 and the proximity mine, in that order.
    pub fn set_thrown(
        &mut self,
        grenade: Throwable,
        c4: Option<Throwable>,
        mine: Option<Throwable>,
    ) {
        self.throwables = vec![Some(grenade), c4, mine];
        self.explosive = 0;
    }

    /// Sets off every C4 charge she has thrown.
    fn detonate(&mut self, f: &mut Field<'_>) {
        let mut set_off = Vec::new();
        self.projectiles.retain(|p| match *p {
            Projectile::Charge {
                at,
                blast,
                mine: None,
                ..
            } => {
                set_off.push((at, blast));
                false
            }
            _ => true,
        });
        for (at, blast) in set_off {
            self.explode_blast(f, at, blast);
        }
    }

    fn fly(&mut self, f: &mut Field<'_>, dt: f32) {
        let mut blasts: Vec<(Vec3, Option<Blast>, Option<u32>)> = Vec::new();
        self.fuses.retain_mut(|(at, left)| {
            *left -= dt;
            if *left <= 0.0 {
                blasts.push((*at, None, None));
            }
            *left > 0.0
        });
        let mut marks = Vec::new();
        for t in &mut self.thrown {
            let puffed = (t.age / MARKER_PUFF).floor();
            t.age += dt;
            if t.age >= MARKER_SMOKE.0 && (t.age / MARKER_PUFF).floor() > puffed {
                marks.push(t.at);
            }
            t.velocity.y -= m2_engine::physics::GRAVITY * dt;
            let step = t.velocity * dt;
            if let Some((d, normal)) = f.physics.cast_ray(t.at, step, step.length()) {
                t.at += step.normalize_or_zero() * (d - 0.05).max(0.0);
                t.velocity = (t.velocity - normal * 2.0 * t.velocity.dot(normal)) * BOUNCE;
                // Rolling to a stop.
                if t.velocity.length() < 0.5 {
                    t.velocity = Vec3::ZERO;
                }
            } else {
                t.at += step;
            }
        }
        // A marker smokes from a moment after it lands, a while.
        self.thrown.retain(|t| t.age < MARKER_SMOKE.1);
        for at in marks {
            self.effect(
                Effect::Smoke {
                    at: at + Vec3::Y * 0.5,
                    size: 1.5,
                },
                3.0,
            );
            self.hold_fx(fx::MARKER, at, place_key(1, at));
        }
        let mut puffs = Vec::new();
        let mut streaks = Vec::new();
        // Where people and vehicles are, for mines.
        let near: Vec<Vec3> = f
            .people
            .npcs
            .iter()
            .filter(|n| n.alive)
            .map(|n| n.chest())
            .chain(
                f.vehicles
                    .iter()
                    .filter_map(|v| f.physics.vehicle_transform(v.id?))
                    .map(|m| m.w_axis.truncate()),
            )
            .collect();
        self.projectiles.retain_mut(|p| match p {
            Projectile::Rocket {
                at,
                velocity,
                flown,
                shell,
                owner,
                blast,
                homing,
                key,
            } => {
                // Turning after the vehicle it locked on, no faster than its
                // turn rate.
                if let Some((target, turn)) = *homing
                    && let Some(m) = f.physics.vehicle_transform(target)
                {
                    let want = (m.w_axis.truncate() - *at).normalize_or_zero();
                    let speed = velocity.length();
                    let have = velocity.normalize_or_zero();
                    let angle = have.angle_between(want);
                    if angle > 1e-4 && want != Vec3::ZERO {
                        let t = ((turn * dt) / angle).min(1.0);
                        *velocity = have.lerp(want, t).normalize_or(have) * speed;
                    }
                }
                let step = *velocity * dt;
                let len = step.length();
                let hit = cast_shot(f.physics, f.people, *at, step, len, None, *owner);
                if let Some((d, _)) = hit {
                    blasts.push((*at + step.normalize_or_zero() * d, *blast, *key));
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
                    blasts.push((*at, *blast, *key));
                    return false;
                }
                true
            }
            Projectile::Round {
                at,
                velocity,
                flown,
                blast,
            } => {
                velocity.y -= m2_engine::physics::GRAVITY * dt;
                let step = *velocity * dt;
                let len = step.length();
                if let Some((d, _)) = cast_shot(f.physics, f.people, *at, step, len, None, None) {
                    blasts.push((*at + step.normalize_or_zero() * d, *blast, None));
                    return false;
                }
                *at += step;
                *flown += len;
                // Into the sea, or never landing: it bursts there.
                if f.physics.water().is_some_and(|w| at.y < w) || *flown > LOB_RANGE {
                    blasts.push((*at, *blast, None));
                    return false;
                }
                true
            }
            Projectile::Charge {
                at,
                velocity,
                stuck,
                model: _,
                blast,
                timer,
                mine,
            } => {
                if !*stuck {
                    velocity.y -= m2_engine::physics::GRAVITY * dt;
                    let step = *velocity * dt;
                    if let Some((d, _)) = f.physics.cast_ray(*at, step, step.length()) {
                        // It sticks (C4) or settles (a mine) where it lands.
                        *at += step.normalize_or_zero() * (d - 0.03).max(0.0);
                        *velocity = Vec3::ZERO;
                        *stuck = true;
                    } else {
                        *at += step;
                    }
                }
                *timer -= dt;
                match mine {
                    Some(m) => {
                        m.life -= dt;
                        let armed = *timer <= 0.0;
                        if armed && near.iter().any(|p| p.distance(*at) < m.reach) {
                            blasts.push((*at, m.blast, None));
                            return false;
                        }
                        // Its planted template's timer: it is gone, unexploded.
                        !(armed && m.life <= -m.arm)
                    }
                    None if *timer <= 0.0 => {
                        blasts.push((*at, *blast, None));
                        false
                    }
                    None => true,
                }
            }
            Projectile::Grenade {
                at,
                velocity,
                fuse,
                blast,
            } => {
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
                    blasts.push((*at, *blast, None));
                    return false;
                }
                true
            }
        });
        for (k, at) in puffs.into_iter().enumerate() {
            self.effect(Effect::Smoke { at, size: 0.4 }, 0.8);
            // Rockets have no id: the k-th in flight (one at a time, as a
            // rule).
            self.hold_fx(fx::ROCKET, at, (2 << 32) | k as u64);
        }
        for (from, to) in streaks {
            self.effect(Effect::Tracer { from, to }, dt * 1.5);
        }
        for (at, blast, key) in blasts {
            self.explode_keyed(f, at, blast, key);
        }
    }

    /// A blast at `at`: fire and smoke, people killed or hurt, vehicles
    /// near it wrecked and thrown.
    pub fn explode(&mut self, f: &mut Field<'_>, at: Vec3) {
        self.burst(at, 1.0);
        self.show_fx(fx::GRENADE, at);
        let mut killed = false;
        let key = Some(m2_assets::name_hash("Explosion"));
        for n in &mut f.people.npcs {
            let d = n.chest().distance(at);
            // Thrown out and up, harder the nearer.
            let near = (1.0 - d / REACH).max(0.0);
            let away = (n.chest() - at).with_y(0.0).normalize_or(Vec3::X);
            let push = away * (BLAST_PUSH.0 * near + 3.0) + Vec3::Y * (BLAST_PUSH.1 * near + 2.0);
            // Each by their own row of the table (`Human` 1 of an
            // `Explosion`, `HumanArmor` 0.5).
            let own = self.on_person(n, key);
            if d < KILL {
                killed |= n.hurt(1000.0 * own, push) && n.soldier;
            } else if d < REACH {
                killed |= n.hurt(100.0 * near * own, push) && n.soldier;
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
                let damage = 100.0 * (1.0 - d / REACH).max(0.0);
                self.tally.taken_blasts +=
                    self.hurt_player(f, damage * self.hero_damage.explosion, Some(at));
            }
        }
        for (i, v) in f.vehicles.iter_mut().enumerate() {
            let Some(id) = v.id else { continue };
            let Some(body) = f.physics.vehicle_transform(id) else {
                continue;
            };
            let (lo, hi) = v.spec.bounds;
            let centre = body.transform_point3((lo + hi) * 0.5);
            // Nearest its hull's side, roughly.
            let d = (centre.distance(at) - (hi - lo).max_element() * 0.25).max(0.0);
            let (heart, edge, radius) = BLAST_ON_VEHICLE;
            if d < radius && !v.wrecked {
                let damage = heart + (edge - heart) * d / radius;
                v.blast_parts(body, at, radius, |p| heart + (edge - heart) * p / radius);
                self.hurt_vehicle(
                    f.physics,
                    v,
                    i,
                    damage * v.spec.tuning.blast_scale.unwrap_or(1.0),
                );
                // What it wrecks, it throws: off-centre, so it tumbles.
                if v.wrecked {
                    let away = (centre - at).with_y(0.0).normalize_or(Vec3::X);
                    let push = (away * 4.0 + Vec3::Y * 6.0) * v.spec.mass;
                    f.physics
                        .push_vehicle(id, push, centre + away * -0.5 + Vec3::Y * 0.3);
                }
            }
        }
        if killed {
            self.remarks.push(Remark::Kill);
        }
        alarm(f.people, at);
    }

    /// A blast's fire, smoke and sound, `size` times a grenade's.
    fn burst(&mut self, at: Vec3, size: f32) {
        self.blasts += 1;
        self.last_blast = Some(at);
        self.went_off.push(at);
        self.sounds.push(Sound::Explosion(at));
        self.effect(
            Effect::Fire {
                at,
                size: 4.0 * size,
            },
            0.6,
        );
        for k in 0..6 {
            let a = k as f32 * std::f32::consts::TAU / 6.0;
            let off = Vec3::new(a.sin(), 0.5, a.cos()) * 1.5 * size;
            self.effect(
                Effect::Smoke {
                    at: at + off,
                    size: 2.5 * size,
                },
                2.5,
            );
        }
    }

    /// Of a round's or blast's `Damage` with `DamageKey` `key` (a round
    /// without one is a `Bullet`), what the game's damage table gives
    /// person `n`: the row of their template's `ModifierKey` (`Human`:
    /// `Bullet` 1, `BulletLarge` 1.5, `Explosion` 1; `HumanArmor`, the VZ
    /// heavies' and defenders': 0.25, 0.75, 0.5), the `Human` row for
    /// those whose template names none or whose key the table lacks.
    fn on_person(&self, n: &crate::npc::Npc, key: Option<u32>) -> f32 {
        let key = key
            .filter(|&k| k != 0)
            .unwrap_or_else(|| m2_assets::name_hash("Bullet"));
        let human = m2_assets::name_hash("Human");
        self.damage_table
            .as_ref()
            .and_then(|t| {
                t.scale(n.ai.modifier.unwrap_or(human), key)
                    .or_else(|| t.scale(human, key))
            })
            .unwrap_or(1.0)
    }

    /// Her round of `damage` (its `Damage` at that range) and `DamageKey`
    /// `key` striking person `j`, shoving the body `push` if it kills:
    /// they take what [`on_person`](Self::on_person) gives of it. True if
    /// it killed a soldier.
    fn round_on_person(
        &mut self,
        people: &mut People,
        j: usize,
        (damage, hero): (f32, f32),
        key: Option<u32>,
        push: Vec3,
    ) -> bool {
        let taken = damage * hero * self.on_person(&people.npcs[j], key);
        let n = &mut people.npcs[j];
        let counts = n.alive && n.soldier;
        let killed = n.hurt(taken, push) && n.soldier;
        self.hits += 1;
        if counts {
            self.tally.hits += 1;
            self.tally.dealt += taken;
            self.tally.kills += u32::from(killed);
        }
        killed
    }

    /// What a soldier's round does to her: his template's gun's round's
    /// `Damage` times the game's damage table's `Hero` row for its
    /// `DamageKey` (a round without a key is a `Bullet`); the Assault
    /// Rifle Bullet's [`NPC_DAMAGE`] when his template names no gun, or
    /// the table isn't found.
    fn npc_round(&self, gun: Option<m2_engine::tuning::HandGun>) -> f32 {
        let Some(g) = gun else { return NPC_DAMAGE };
        let key = g
            .damage_key
            .filter(|&k| k != 0)
            .unwrap_or_else(|| m2_assets::name_hash("Bullet"));
        self.damage_table
            .as_ref()
            .and_then(|t| t.scale(m2_assets::name_hash("Hero"), key))
            .map_or(NPC_DAMAGE, |scale| g.damage * scale)
    }

    /// Explosion template `blast` going off at `at`, by its own `Radius`,
    /// `Damage` and `MinDamage` and the damage table's scale for its
    /// `DamageKey` (people's `Human` row, the hero's `Hero` row; vehicles
    /// take their `ExplosionFudge` of it). Without one, the old blast
    /// ([`Combat::explode`]).
    fn explode_blast(&mut self, f: &mut Field<'_>, at: Vec3, blast: Option<Blast>) {
        self.explode_keyed(f, at, blast, None);
    }

    /// [`explode_blast`](Self::explode_blast) of a round whose own
    /// `DamageKey` is `key` (a rocket's `Rocket`): a vehicle part with a
    /// key of its own at the blast's heart (a tank's weak point) has the
    /// vehicle take [`DamageTable::weak_point`] times its share.
    fn explode_keyed(
        &mut self,
        f: &mut Field<'_>,
        at: Vec3,
        blast: Option<Blast>,
        key: Option<u32>,
    ) {
        let Some(b) = blast else {
            self.explode(f, at);
            return;
        };
        // Fire and smoke the size of a grenade's at its 5 m reach.
        self.burst(at, (b.radius / 5.0).clamp(0.4, 3.0));
        self.game_fx.push(GameFx {
            template: FxTemplate::Id(b.template),
            at,
            hold: None,
        });
        let hers = if b.key == Some(m2_assets::name_hash("ExplosionLarge")) {
            self.hero_damage.explosion_large
        } else {
            self.hero_damage.explosion
        };
        let reach = b.radius.max(0.5);
        let mut killed = false;
        for j in 0..f.people.npcs.len() {
            // Their own row of the damage table for its `DamageKey`; all
            // of it without a key.
            let scale = match b.key {
                Some(_) => self.on_person(&f.people.npcs[j], b.key),
                None => 1.0,
            };
            let n = &mut f.people.npcs[j];
            let d = n.chest().distance(at);
            if d < reach {
                let near = 1.0 - d / reach;
                let away = (n.chest() - at).with_y(0.0).normalize_or(Vec3::X);
                let push =
                    away * (BLAST_PUSH.0 * near + 3.0) + Vec3::Y * (BLAST_PUSH.1 * near + 2.0);
                killed |= n.hurt(b.damage_at(d) * scale, push) && n.soldier;
                n.panic(at);
            } else if d < reach * 3.0 {
                n.panic(at);
            }
        }
        for e in f.explosives.iter_mut() {
            if !e.gone && e.at.distance(at) < e.size + reach {
                e.gone = true;
                self.fuses.push((e.at, CHAIN_DELAY));
                self.fires.push((e.at, e.size, BURN, 0.0));
            }
        }
        if f.player.driving.is_none() && f.player.landed {
            let d = (f.player.character.position + Vec3::Y).distance(at);
            if d < reach {
                self.tally.taken_blasts += self.hurt_player(f, b.damage_at(d) * hers, Some(at));
            }
        }
        for (i, v) in f.vehicles.iter_mut().enumerate() {
            let Some(id) = v.id else { continue };
            let Some(body) = f.physics.vehicle_transform(id) else {
                continue;
            };
            let (lo, hi) = v.spec.bounds;
            let centre = body.transform_point3((lo + hi) * 0.5);
            let d = (centre.distance(at) - (hi - lo).max_element() * 0.25).max(0.0);
            if d < reach && !v.wrecked {
                let weak = self.weak_point(v, body, at, key);
                if weak != 1.0 {
                    eprintln!("weak point: {} takes {weak:.1}x of the blast", v.spec.label);
                }
                let damage = b.damage_at(d) * v.spec.tuning.blast_scale.unwrap_or(1.0) * weak;
                v.blast_parts(body, at, reach, |p| b.damage_at(p));
                self.hurt_vehicle(f.physics, v, i, damage);
                if v.wrecked {
                    let away = (centre - at).with_y(0.0).normalize_or(Vec3::X);
                    let push = (away * 4.0 + Vec3::Y * 6.0) * v.spec.mass;
                    f.physics
                        .push_vehicle(id, push, centre + away * -0.5 + Vec3::Y * 0.3);
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
        let hers = self.hero_damage;
        self.harm(f, at, BOMB, false, (hers.explosion_large, "ExplosionLarge"));
        self.show_fx(fx::BOMB, at);
        self.harm(
            f,
            at + Vec3::Y,
            BOMB_FRAG,
            true,
            (hers.bullet_large, "BulletLarge"),
        );
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

    /// An air strike's blast at `at`, as its explosion template has it:
    /// reaching `radius` metres with `damage` at its heart. Fire and smoke
    /// in proportion (the template's own effects aren't drawn yet).
    pub fn blast(&mut self, f: &mut Field<'_>, at: Vec3, radius: f32, damage: f32) {
        self.blasts += 1;
        self.last_blast = Some(at);
        self.went_off.push(at);
        self.sounds.push(if radius >= BOMB.0 {
            Sound::Bomb(at)
        } else {
            Sound::Explosion(at)
        });
        // The strikes' ordnance keys aren't read yet: an `Explosion`.
        let hers = self.hero_damage.explosion;
        self.harm(f, at, (radius, damage), false, (hers, "Explosion"));
        self.show_fx(
            if radius >= BOMB.0 {
                fx::BOMB
            } else {
                fx::BOMBLET
            },
            at,
        );
        // Fire and smoke grow with the reach's square: a 20 m bomb's 10 m
        // across, a 15 m bomblet's under 6, so that forty bomblets
        // together don't fill the view. (Sizes judged by eye, not read.)
        let size = (radius * radius / 40.0).clamp(1.5, 12.0);
        self.effect(
            Effect::Fire {
                at: at + Vec3::Y * size * 0.2,
                size,
            },
            0.5 + size * 0.04,
        );
        let puffs = ((radius / 6.0) as usize).clamp(1, 6);
        for k in 0..puffs {
            let a = k as f32 * std::f32::consts::TAU / puffs as f32;
            let ring = Vec3::new(a.sin(), 0.4, a.cos()) * size * 0.4;
            self.effect(
                Effect::Smoke {
                    at: at + ring,
                    size: size * 0.6,
                },
                2.0 + size * 0.2,
            );
        }
        alarm(f.people, at);
    }

    /// Harm a blast's frag does around `at`, without fire or sound of its
    /// own.
    pub fn harm_at(&mut self, f: &mut Field<'_>, at: Vec3, radius: f32, damage: f32) {
        // Frag, as the satellite bomb's: a `BulletLarge`.
        let hers = self.hero_damage.bullet_large;
        self.harm(f, at, (radius, damage), false, (hers, "BulletLarge"));
    }

    /// The bomb's final strike at `at`, after it lands.
    pub fn final_strike(&mut self, f: &mut Field<'_>, at: Vec3) {
        self.went_off.push(at);
        self.sounds.push(Sound::Explosion(at));
        let hers = self.hero_damage.explosion_large;
        self.harm(f, at, BOMB_FINAL, false, (hers, "ExplosionLarge"));
        self.show_fx(fx::FINAL, at);
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
    /// and thrown, explosives within it go off. She takes `hers` of what
    /// reaches her ([`HeroDamage`]), each person what the damage table's
    /// row for their `ModifierKey` gives of damage kind `key`
    /// ([`on_person`](Self::on_person): a `HumanArmor` heavy half of an
    /// `Explosion`).
    fn harm(
        &mut self,
        f: &mut Field<'_>,
        at: Vec3,
        (radius, damage): (f32, f32),
        cover: bool,
        (hers, key): (f32, &str),
    ) {
        let key = Some(m2_assets::name_hash(key));
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
            let own = self.on_person(n, key);
            killed |= n.hurt(damage * near * own, push) && n.soldier;
        }
        let her = (f.player.driving.is_none() && f.player.landed)
            .then(|| f.player.character.position + Vec3::Y)
            .filter(|&p| p.distance(at) < radius && !shielded(p))
            .map(|p| damage * (1.0 - p.distance(at) / radius) * hers);
        let mut wrecking = Vec::new();
        for (i, v) in f.vehicles.iter().enumerate() {
            let Some(body) = v.id.and_then(|id| physics.vehicle_transform(id)) else {
                continue;
            };
            let (lo, hi) = v.spec.bounds;
            let centre = body.transform_point3((lo + hi) * 0.5);
            let d = centre.distance(at);
            let near = 1.0 - d / radius;
            if d < radius && !shielded(centre) {
                wrecking.push((i, centre, near));
            }
        }
        for (i, centre, near) in wrecking {
            let v = &mut f.vehicles[i];
            let Some(id) = v.id else { continue };
            if v.wrecked {
                continue;
            }
            let scale = v.spec.tuning.blast_scale.unwrap_or(1.0);
            if let Some(body) = f.physics.vehicle_transform(id) {
                v.blast_parts(body, at, radius, |p| damage * (1.0 - p / radius));
            }
            self.hurt_vehicle(f.physics, v, i, damage * near * scale);
            if v.wrecked {
                let away = (centre - at).with_y(0.0).normalize_or(Vec3::X);
                let push = (away * 4.0 + Vec3::Y * 6.0) * v.spec.mass * near.max(0.3);
                f.physics
                    .push_vehicle(id, push, centre + away * -0.5 + Vec3::Y * 0.3);
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
            // For checking what hurts her (autoplay logs).
            eprintln!("blast: at {at:.1}, she takes {d:.0}");
            self.tally.taken_blasts += self.hurt_player(f, d, Some(at));
        }
        if killed {
            self.remarks.push(Remark::Kill);
        }
    }

    fn smoke_wrecks(&mut self, f: &mut Field<'_>, dt: f32) {
        // The effects vehicles' parts keep going (a burning hood, a
        // wreck's smoke and fire), as their state machines say.
        self.part_fx_next -= dt;
        let mut data_smoke = vec![false; f.vehicles.len()];
        let mut held = Vec::new();
        for (i, v) in f.vehicles.iter().enumerate() {
            let (Some(p), Some(dm)) = (&v.parts, v.spec.damage.as_deref()) else {
                continue;
            };
            let fx = p.emitters(dm);
            data_smoke[i] = !fx.is_empty();
            if self.part_fx_next > 0.0 || fx.is_empty() {
                continue;
            }
            let Some(body) = v.id.and_then(|id| f.physics.vehicle_transform(id)) else {
                continue;
            };
            for (k, (part, effect, at)) in fx.into_iter().enumerate() {
                let key = (5 << 32) | ((i as u64) << 16) | ((part as u64) << 6) | (k as u64 & 63);
                held.push((effect, body.transform_point3(at), key));
            }
        }
        for (effect, at, key) in held {
            self.game_fx.push(GameFx {
                template: FxTemplate::Hash(effect),
                at,
                hold: Some(key),
            });
        }
        if self.part_fx_next <= 0.0 {
            self.part_fx_next = PART_FX_EVERY;
        }
        let mut puffs = Vec::new();
        for (i, next) in &mut self.wrecks {
            *next -= dt;
            if *next > 0.0 {
                continue;
            }
            *next = 0.4;
            // Its parts' own smoke stands in for the made-up puffs.
            if data_smoke.get(*i).copied().unwrap_or(false) {
                continue;
            }
            let v = &f.vehicles[*i];
            if let Some(body) = v.id.and_then(|id| f.physics.vehicle_transform(id)) {
                let (lo, hi) = v.spec.bounds;
                let top = Vec3::new((lo.x + hi.x) * 0.5, hi.y, (lo.z + hi.z) * 0.5);
                puffs.push((*i, body.transform_point3(top)));
            }
        }
        for (i, at) in puffs {
            self.effect(Effect::Smoke { at, size: 1.5 }, 3.0);
            for name in fx::WRECK {
                self.hold_fx(name, at, (3 << 32) | i as u64);
            }
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
            for name in fx::BLAZE {
                self.hold_fx(name, at, place_key(4, at));
            }
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
            for name in fx::FIRE {
                self.hold_fx(name, low, place_key(5, at));
            }
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
/// those close by who don't fight run (not story characters).
fn alarm(people: &mut People, at: Vec3) {
    for n in &mut people.npcs {
        if !n.alive {
            continue;
        }
        let d = n.character.position.distance(at);
        if n.soldier && !n.ai.flags.pacifist && d < ALERT_RANGE * n.ai.hearing {
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

/// A round from `eye` along unit `aim`, drawn by her gun's aim assist
/// `assist` onto the enemy (a soldier turned on her) whose chest lies
/// nearest its line within the assist's window and `far` metres
/// ([`m2_engine::weapons::AimAssist::draw`]); `aim` itself with none.
/// That the assist draws each round (after its stray), who counts as a
/// target (the soldiers the reticle marks as enemies) and that it draws
/// onto the chest are invented.
fn assisted_aim(
    assist: &m2_engine::weapons::AimAssist,
    people: &People,
    eye: Vec3,
    aim: Vec3,
    far: f32,
) -> Vec3 {
    people
        .npcs
        .iter()
        .filter(|n| n.alive && n.landed && n.soldier && n.alert)
        .map(|n| n.character.position + Vec3::Y * ASSIST_CHEST - eye)
        .filter(|off| off.length() <= far)
        .filter_map(|off| {
            let drawn = assist.draw(aim, off);
            (drawn != aim).then(|| (off.normalize_or_zero().dot(aim), drawn))
        })
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map_or(aim, |(_, drawn)| drawn)
}

/// Where on a target the aim assist draws her aim: his chest, metres up
/// from his feet (invented).
const ASSIST_CHEST: f32 = 1.2;

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

/// A held effect's key for a source that stays put (a fire, a marker):
/// its kind and where it is, to the decimetre.
fn place_key(kind: u64, at: Vec3) -> u64 {
    let q = |v: f32| u64::from((v * 10.0).round() as i32 as u16);
    (kind << 48) | (q(at.x) << 32) | (q(at.y) << 16) | q(at.z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::npc::tests::{ground, rig};
    use crate::npc::{Brain, Npc};

    #[test]
    fn a_part_spent_launcher_reloads_from_what_is_stored() {
        // 2 of 6 loaded, 10 stored: 6 loaded, 6 stored.
        let r = Rounds {
            clip: 2,
            stored: 10,
        };
        assert_eq!(topped_up(r, 6), Some(Rounds { clip: 6, stored: 6 }));
        // 2 loaded and 1 stored: 3 loaded.
        let r = Rounds { clip: 2, stored: 1 };
        assert_eq!(topped_up(r, 6), Some(Rounds { clip: 3, stored: 0 }));
        // Full, or nothing stored: no reload.
        assert_eq!(topped_up(Rounds { clip: 6, stored: 4 }, 6), None);
        assert_eq!(topped_up(Rounds { clip: 2, stored: 0 }, 6), None);
    }

    #[test]
    fn a_blow_rights_a_vehicle_on_its_side_in_front_of_her() {
        // Upright, leaning on a slope, on its side, on its roof.
        assert!(!overturned(Vec3::Y));
        assert!(!overturned(Vec3::new(0.5, 0.86, 0.0)));
        assert!(overturned(Vec3::X));
        assert!(overturned(-Vec3::Y));
        // A jeep 1 m wide lying 2.5 m ahead of her is in reach; behind her
        // or 5 m off it isn't.
        let (feet, ahead) = (Vec3::ZERO, Vec3::Z);
        assert!(within_bash(feet, ahead, Vec3::new(0.0, 0.5, 2.5), 1.0));
        assert!(!within_bash(feet, ahead, Vec3::new(0.0, 0.5, -2.5), 1.0));
        assert!(!within_bash(feet, ahead, Vec3::new(0.0, 0.5, 5.0), 1.0));
    }

    #[test]
    fn rounds_striking_the_world_throw_up_their_surfaces_impact() {
        let mut physics = Physics::new();
        let v = [
            Vec3::new(-5.0, 0.0, -5.0),
            Vec3::new(5.0, 0.0, -5.0),
            Vec3::new(5.0, 0.0, 5.0),
            Vec3::new(-5.0, 0.0, 5.0),
        ];
        let metal = m2_assets::name_hash("METAL");
        let floor =
            m2_engine::CollisionShape::mesh_of(&v, &[[0, 2, 1], [0, 3, 2]], &[metal; 2]).unwrap();
        physics.add_static(StaticKey::Cell(3), &floor, Vec3::ZERO, Quat::IDENTITY);
        let bare = m2_engine::CollisionShape::mesh(&v, &[[0, 2, 1], [0, 3, 2]]).unwrap();
        physics.add_static(
            StaticKey::Cell(4),
            &bare,
            Vec3::new(20.0, 0.0, 0.0),
            Quat::IDENTITY,
        );
        physics.step(1.0 / 60.0);
        let mut combat = Combat::default();
        let from = Vec3::new(0.0, 2.0, 0.0);
        let key = combat.strike_world(&physics, from, Vec3::NEG_Y, 2.0);
        assert_eq!(key, Some(StaticKey::Cell(3)));
        let shown = |c: &Combat| match &c.game_fx.last().unwrap().template {
            FxTemplate::Name(n) => n.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(shown(&combat), "global_particle_impact_bullet_metal");
        // Just short of the floor, off its face.
        assert!((combat.game_fx[0].at.y - IMPACT_BACK).abs() < 1e-4);
        // Ground that says nothing throws up the generic impact.
        combat.strike_world(&physics, from + Vec3::X * 20.0, Vec3::NEG_Y, 2.0);
        assert_eq!(shown(&combat), m2_engine::surface::GENERIC_IMPACT);
        assert_eq!(combat.impacts, (1, 1));
    }

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
        // A VZ soldier's `Health` of 100 takes 17 carbine rounds of 6 (no
        // template here, so no `HeroMultiplier`).
        people.npcs[0].hp = m2_engine::ai::AiProfile::default().health;
        for _ in 0..600 {
            let mut field = Field {
                physics: &mut physics,
                player: &mut player,
                people: &mut people,
                vehicles: &mut vehicles,
                arsenal: &arsenal,
                explosives: &mut [],
            };
            combat.step(&mut field, fire, dt);
            if !people.npcs[0].alive {
                break;
            }
        }
        assert!(!people.npcs[0].alive);
        assert_eq!(combat.hits, 17);
        assert!(people.npcs[1].alert, "gunfire within 60 m alerts");
        assert!(
            combat
                .take_sounds()
                .iter()
                .all(|s| matches!(s, Sound::Shot { .. }))
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

    // With the damage table, a blow hurts by the row's `MeleeBash` cell:
    // a `Human` takes a thousand times it (one blow drops him), a boss's
    // row (`cab45462`) nothing.
    #[test]
    fn a_bash_hurts_by_the_damage_table() {
        let (human, boss) = (m2_assets::name_hash("Human"), 0xcab4_5462);
        let mut data = vec![0u8; 0x48];
        let mut put = |v: u32| data.extend_from_slice(&v.to_le_bytes());
        put(2);
        put(1);
        put(human);
        put(boss);
        put(m2_assets::name_hash("MeleeBash"));
        put(1000.0f32.to_bits());
        put(0.0f32.to_bits());
        for (modifier, dies) in [(None, true), (Some(boss), false)] {
            let mut physics = ground();
            let dt = 1.0 / 60.0;
            let mut player = Player::new(0.0, 0.0, 0.0, None);
            let mut people = People::default();
            let mut n = Npc::new(
                0,
                &rig(),
                Vec3::new(0.0, 1.0, 1.2),
                std::f32::consts::PI,
                Brain::Post { facing: 0.0 },
                Some(1),
            );
            n.ai.modifier = modifier;
            people.npcs.push(n);
            for _ in 0..3 {
                people.update(&mut physics, &[], Vec3::ZERO, None, dt);
                player.update(&physics, Default::default(), dt);
            }
            let arsenal = Arsenal::default();
            let mut vehicles = Vec::new();
            let mut combat = Combat {
                damage_table: m2_engine::tuning::DamageTable::parse(&data),
                ..Combat::default()
            };
            for k in 0..30 {
                let mut field = Field {
                    physics: &mut physics,
                    player: &mut player,
                    people: &mut people,
                    vehicles: &mut vehicles,
                    arsenal: &arsenal,
                    explosives: &mut [],
                };
                let trigger = Trigger {
                    melee: k == 0,
                    ..Trigger::default()
                };
                combat.step(&mut field, trigger, dt);
            }
            assert_eq!(combat.hits, 1);
            assert_eq!(!people.npcs[0].alive, dies);
            if !dies {
                assert_eq!(people.npcs[0].hp, 100.0);
            }
        }
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
        let at = slung_at(muzzle, (lo, hi), point, (right, up), 0, HANG_DOWN).unwrap();
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
        let left = slung_at(
            muzzle,
            (lo, hi),
            point + Vec3::X * 0.2,
            (right, up),
            1,
            HANG_DOWN,
        )
        .unwrap();
        assert!(left.transform_vector3(Vec3::Y).dot(right) < -0.9);
        assert!(at.transform_vector3(Vec3::Y).dot(right) > 0.9);
    }

    #[test]
    fn the_launcher_hangs_muzzle_up_across_her_back() {
        // Her back as she stands facing +z, her right to -x.
        let (right, up) = (Vec3::NEG_X, Vec3::Y);
        // A launcher 0.8 m long, its barrel along +z.
        let muzzle = Vec3::new(0.0, 0.05, 0.5);
        let (lo, hi) = (Vec3::new(-0.06, -0.15, -0.3), Vec3::new(0.06, 0.1, 0.5));
        let point = Vec3::new(-0.1, 1.4, -0.14);
        let at = slung_at(muzzle, (lo, hi), point, (right, up), 0, HANG_LAUNCHER).unwrap();
        let tip = at.transform_point3(muzzle);
        let barrel = at.transform_vector3(Vec3::Z).normalize();
        // Muzzle up toward her right shoulder, about 22 degrees off
        // straight up on a back standing straight.
        let off = barrel.y.clamp(-1.0, 1.0).acos().to_degrees();
        assert!(
            barrel.dot(right) > 0.0 && (20.0..25.0).contains(&off),
            "{barrel} {off}"
        );
        // Its muzzle above the attach point, and on her right.
        assert!(tip.y > point.y + 0.35 && tip.y < point.y + 0.6, "{tip}");
        assert!(tip.dot(right) > point.dot(right), "{tip}");
        assert!(
            at.transform_point3(Vec3::new(0.0, 0.0, -0.3)).z < point.z,
            "behind her"
        );
    }

    #[test]
    fn a_new_hero_holds_the_weapons_in_his_own_hands() {
        // Two skeletons with the hands, attach points, back and neck at
        // different nodes, as Jennifer's and Mattias's have.
        let rig_of = |names: Vec<u32>| {
            let n = names.len();
            let mut r = rig();
            r.skeleton = std::sync::Arc::new(Skeleton {
                name_hashes: names,
                parents: (0..n).map(|i| i.checked_sub(1)).collect(),
                rest: vec![
                    m2_engine::anim::Local {
                        translation: Vec3::ZERO,
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    };
                    n
                ],
                inverse_bind: vec![Mat4::IDENTITY; n],
            });
            r
        };
        let hers = rig_of(vec![1, RIGHT_HAND, RIGHT_ATTACH, LEFT_HAND, SPINE, NECK]);
        let [back_right, back_left] = BACK_ATTACH;
        let his = rig_of(vec![
            1,
            2,
            3,
            SPINE,
            NECK,
            LEFT_HAND,
            4,
            RIGHT_HAND,
            RIGHT_ATTACH,
            back_right,
            back_left,
        ]);
        let mut arsenal = Arsenal::default();
        arsenal.add(
            "m4",
            crate::npc::tests::empty_model(),
            (Vec3::Z, None),
            Some(&hers),
        );
        assert_eq!((arsenal.hand, arsenal.attach), (Some(1), Some(2)));
        arsenal.rehold(&his);
        assert_eq!((arsenal.hand, arsenal.attach), (Some(7), Some(8)));
        assert_eq!(arsenal.back, [Some(9), Some(10)]);
        assert_eq!((arsenal.spine, arsenal.neck), (Some(3), Some(4)));
        // Drawn at his right hand's attach point, not at the node hers
        // was (his left hand's neighbour).
        let nodes: Vec<Mat4> = (0..11)
            .map(|i| Mat4::from_translation(Vec3::X * i as f32))
            .collect();
        let w = arsenal.carbine.as_ref().unwrap();
        let at = arsenal
            .in_hand(
                w,
                Mat4::IDENTITY,
                &nodes,
                (arsenal.hand, arsenal.attach),
                false,
            )
            .unwrap();
        assert_eq!(at.w_axis.x, 8.0);
    }

    // Held in both hands, the barrel runs from the rear hand to the front
    // one as they are posed now, whatever the aim's grip was fitted to,
    // with the grip where it sits in the rear hand.
    #[test]
    fn both_hands_point_the_barrel_from_rear_to_front() {
        let w = WeaponModel {
            model: crate::npc::tests::empty_model(),
            muzzle: Vec3::Z,
            eject: None,
            // As fitted to an aim pose: barrel along +z from the hand.
            grip: Mat4::IDENTITY,
            grip_hand: Some(0),
            grip_front: Some((1, Vec3::new(0.0, -0.05, 0.0))),
            carry: None,
            hang: HANG_DOWN,
        };
        // Running: the front hand low and to her left of the rear one.
        let nodes = [
            Mat4::from_translation(Vec3::new(0.2, 1.0, 0.1)),
            Mat4::from_translation(Vec3::new(-0.2, 0.8, 0.4)),
        ];
        let at = Arsenal::default()
            .in_hand(&w, Mat4::IDENTITY, &nodes, (Some(0), None), true)
            .unwrap();
        let barrel = at.transform_vector3(Vec3::Z).normalize();
        let want = (Vec3::new(-0.2, 0.8, 0.4) - Vec3::new(0.2, 1.0, 0.1)).normalize();
        assert!(barrel.distance(want) < 1e-5, "{barrel} against {want}");
        assert!(at.w_axis.truncate().distance(Vec3::new(0.2, 0.95, 0.1)) < 1e-5);
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
        let slung = slung_at(muzzle, bounds, point, (right, up), 0, HANG_DOWN).unwrap();
        let lower = slung.transform_point3(muzzle);
        assert!(lower.z < -0.15 && lower.y < 3.0, "{lower}");
        // The short one's top at the base of her neck; a long one (the
        // RPG's 1.28 m) reaches further down, and up no higher than her
        // head.
        let long = (Vec3::new(-0.05, -0.15, -0.5), Vec3::new(0.05, 0.15, 0.78));
        let raised = slung_at(muzzle, long, point, (right, up), 0, HANG_DOWN).unwrap();
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
        let at = slung_at(muzzle, bounds, point, (right, up), 0, HANG_DOWN).unwrap();
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
        use m2_engine::tuning::{GunTuning, RoundScale, VehicleTuning};
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
            node_names: Vec::new(),
            armed: Vec::new(),
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
            node_rolls: Vec::new(),
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
            rotors: Vec::new(),
            damage: None,
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
            crew: Vec::new(),
            turret: 0.0,
            hijack_nodes: Vec::new(),
            gun: 0.0,
            rolled: 0.0,
            fired: 0,
            mg_fired: 0,
            damage: 0.0,
            parts: None,
            loose: Vec::new(),
            debris_thrown: false,
        }];
        assert_eq!(vehicles[0].rounds(), Some(2));
        let mut player = Player::new(0.0, 0.0, 0.0, None);
        player.driving = Some(0);
        let mut people = People::default();
        let arsenal = Arsenal::default();
        let mut combat = Combat::default();
        // Its gun's template (`Mounted Tank Gun (Default)`) has a sound set:
        // its shots carry it, to be heard as that set's shot.
        combat.feels.insert(
            gun.weapon,
            Feel {
                sound_set: Some(0x40b7_1f4e),
                ..Feel::default()
            },
        );
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
                .filter(|s| {
                    matches!(
                        s,
                        Sound::Cannon {
                            set: Some(0x40b7_1f4e),
                            ..
                        }
                    )
                })
                .count();
        }
        assert_eq!(shots, 2);
        assert_eq!(vehicles[0].rounds(), Some(0));
        // A tank's template: 125 health, heavy armour, a fifth of blasts.
        let t = &mut vehicles[0].spec.tuning;
        (t.health, t.armour, t.blast_scale, t.burns_below) =
            (Some(125.0), Some(4), Some(0.2), Some(30.0));
        // The damage table's `Tank` row: rounds do nothing.
        t.rounds = Some(RoundScale::default());
        assert_eq!(vehicles[0].health(), 100.0);
        let mut field = Field {
            physics: &mut physics,
            player: &mut player,
            people: &mut people,
            vehicles: &mut vehicles,
            arsenal: &arsenal,
            explosives: &mut [],
        };
        // A rifle round does nothing to it; a blast beside it some.
        let side = (Vec3::new(5.0, 1.0, 0.0), Vec3::NEG_X, 4.0);
        combat.round_on_vehicle(&mut field, side, (6.0, Round::Bullet), None);
        assert_eq!(field.vehicles[0].damage, 0.0);
        combat.explode(&mut field, Vec3::new(2.0, 1.0, 0.0));
        let hit = field.vehicles[0].health();
        assert!((60.0..90.0).contains(&hit), "{hit}");
        assert!(!field.vehicles[0].wrecked);
        // A `Car`, the carbine tells, a fortieth of a round at a time;
        // burning, it goes in the end.
        field.vehicles[0].spec.tuning.rounds = Some(RoundScale {
            bullet: 0.025,
            bullet_large: 0.1,
        });
        let before = field.vehicles[0].damage;
        combat.round_on_vehicle(&mut field, side, (6.0, Round::Bullet), None);
        assert!((field.vehicles[0].damage - before - 0.15).abs() < 1e-4);
        assert!(field.vehicles[0].health() < hit);
        field.vehicles[0].damage = 100.0;
        for _ in 0..60 * 15 {
            combat.burn_vehicles(&mut field, dt);
        }
        assert!(field.vehicles[0].wrecked);
        assert_eq!(field.vehicles[0].health(), 0.0);
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
        // A blast at her feet does her a fifth of what it does others (the
        // damage table's `Hero` row for an `Explosion`); a few put her
        // down.
        let at = player.character.position;
        let mut field = Field {
            physics: &mut physics,
            player: &mut player,
            people: &mut people,
            vehicles: &mut vehicles,
            arsenal: &arsenal,
            explosives: &mut [],
        };
        let before = combat.hp;
        combat.explode(&mut field, at);
        let lost = before - combat.hp;
        let blast = 100.0 * (1.0 - 1.0 / REACH);
        assert!((lost - blast * 0.2).abs() < 0.01, "{lost}");
        assert!(combat.down == 0.0 && !field.player.down);
        for _ in 0..4 {
            combat.explode(&mut field, at);
        }
        assert!(combat.down > 0.0 && player.down);
        assert_eq!(combat.target(&player, true), None);
    }

    // A soldier's round hurts her as his template's gun's does, by the
    // damage table's `Hero` row; with no gun of his own, as the Assault
    // Rifle Bullet does.
    #[test]
    fn soldiers_rounds_hurt_her_as_their_guns_do() {
        let mut data = vec![0u8; 0x48];
        let mut put = |v: u32| data.extend_from_slice(&v.to_le_bytes());
        put(1);
        put(1);
        put(m2_assets::name_hash("Hero"));
        put(m2_assets::name_hash("Bullet"));
        put(0.25f32.to_bits());
        let combat = Combat {
            damage_table: m2_engine::tuning::DamageTable::parse(&data),
            ..Combat::default()
        };
        assert!(combat.damage_table.is_some());
        let gun = m2_engine::tuning::HandGun {
            template: 1,
            model: None,
            rate_of_fire: 600.0,
            clip: 30,
            reserve_clips: 7,
            damage: 12.0,
            damage_key: None,
            hint: None,
        };
        assert_eq!(combat.npc_round(Some(gun)), 3.0);
        assert_eq!(combat.npc_round(None), NPC_DAMAGE);
    }

    // Her rounds hurt each soldier by his own `ModifierKey` row (the VZ
    // heavies' `HumanArmor` takes a quarter of a `Bullet`), the `Human`
    // row for those without one, times their round's `HeroMultiplier`;
    // and the tally
    // counts the hits and the kill.
    #[test]
    fn her_rounds_hurt_soldiers_by_their_own_row() {
        let (human, armor) = (
            m2_assets::name_hash("Human"),
            m2_assets::name_hash("HumanArmor"),
        );
        let mut data = vec![0u8; 0x48];
        let mut put = |v: u32| data.extend_from_slice(&v.to_le_bytes());
        put(2);
        put(1);
        put(human);
        put(armor);
        put(m2_assets::name_hash("Bullet"));
        put(1.0f32.to_bits());
        put(0.25f32.to_bits());
        let mut combat = Combat {
            damage_table: m2_engine::tuning::DamageTable::parse(&data),
            ..Combat::default()
        };
        let mut people = People::default();
        for modifier in [None, Some(armor)] {
            let mut n = crate::npc::Npc::new(
                0,
                &crate::npc::tests::rig(),
                Vec3::ZERO,
                0.0,
                crate::npc::Brain::Post { facing: 0.0 },
                Some(1),
            );
            n.ai.modifier = modifier;
            n.hp = 75.0;
            people.npcs.push(n);
        }
        assert_eq!(combat.on_person(&people.npcs[0], None), 1.0);
        assert_eq!(combat.on_person(&people.npcs[1], None), 0.25);
        // The carbine's round (6 to 15 m, `HeroMultiplier` 3): a deathsquad
        // mook's 75 health goes in 5 hits.
        let kills: Vec<bool> = (0..5)
            .map(|_| combat.round_on_person(&mut people, 0, (6.0, 3.0), None, Vec3::ZERO))
            .collect();
        assert_eq!(kills.iter().filter(|k| **k).count(), 1);
        assert!(kills[4]);
        combat.round_on_person(&mut people, 1, (6.0, 3.0), None, Vec3::ZERO);
        assert!((people.npcs[1].hp - (75.0 - 6.0 * 3.0 * 0.25)).abs() < 1e-4);
        assert_eq!((combat.tally.hits, combat.tally.kills), (6, 1));
    }

    #[test]
    fn her_own_rifle_is_her_templates_until_the_scripts_hand_her_the_carbine() {
        let bullpup = m2_engine::tuning::HandGun {
            template: 0x8000_6881,
            model: Some(m2_assets::name_hash("global_weapon_bullpup")),
            rate_of_fire: 487.0,
            clip: 30,
            reserve_clips: 7,
            damage: 8.0,
            damage_key: Some(m2_assets::name_hash("Bullet")),
            hint: None,
        };
        let rifle = Rifle::of("Bullpup Rifle", &bullpup);
        assert_eq!((rifle.model, rifle.clip, rifle.spare), ("bullpup", 30, 210));
        assert!((rifle.gap - 60.0 / 487.0).abs() < 1e-6);
        let mut combat = Combat::default();
        combat.take_own_rifle(rifle);
        assert_eq!(combat.rifle_rounds.total(), 240);
        assert!(combat.own_rifle);
        // The first contract's scripts give every hero the carbine.
        combat.carbine_only();
        assert_eq!(combat.rifle, Rifle::default());
        assert!(!combat.own_rifle);
        // Its model, or the carbine's when it isn't loaded.
        let arsenal = Arsenal::default();
        assert!(arsenal.hers(&combat.rifle).is_none());
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
    fn her_aim_is_drawn_onto_an_enemy_near_the_crosshair() {
        let assist = m2_engine::weapons::AimAssist {
            level: 3,
            close: [0.65, 1.0],
            far: [1.5, 1.5],
        };
        let eye = Vec3::Y * ASSIST_CHEST;
        let aim = Vec3::Z;
        // 1 m beside her line at 100 m (0.57 degrees).
        let at = Vec3::new(1.0, 0.0, 100.0);
        let mut people = People::default();
        let mut n = Npc::new(0, &rig(), at, 0.0, Brain::Post { facing: 0.0 }, Some(1));
        n.landed = true;
        n.soldier = true;
        n.alert = false;
        people.npcs.push(n);
        // Not turned on her: the aim stays.
        assert_eq!(assisted_aim(&assist, &people, eye, aim, 400.0), aim);
        people.npcs[0].alert = true;
        let drawn = assisted_aim(&assist, &people, eye, aim, 400.0);
        assert!(drawn.angle_between(at.normalize()) < 1e-4, "{drawn}");
        // Out of reach: the aim stays.
        assert_eq!(assisted_aim(&assist, &people, eye, aim, 50.0), aim);
    }

    #[test]
    fn the_rifle_changes_clips_when_asked_or_empty() {
        let mut combat = Combat::default();
        combat.carbine_only();
        combat.take_ammo(Weapon::Rifle, 45);
        combat.rifle_rounds.clip = 10;
        let ask = Trigger {
            reload: true,
            ..Default::default()
        };
        // Not while driving.
        assert!(!combat.change_clip(ask, false, 0.1));
        assert!(combat.changing.is_none());
        // Started: her reload clip is played for it (playtest 8).
        assert!(combat.change_clip(ask, true, 0.1));
        assert!(combat.changing.is_some());
        for _ in 0..30 {
            assert!(!combat.change_clip(Trigger::default(), true, 0.1));
        }
        assert_eq!(
            combat.rifle_rounds,
            Rounds {
                clip: 30,
                stored: 25
            }
        );
        // Empty, it changes by itself; with nothing to spare, it can't.
        combat.rifle_rounds.clip = 0;
        combat.change_clip(Trigger::default(), true, 0.1);
        assert!(combat.changing.is_some());
        combat.changing = None;
        combat.rifle_rounds = Rounds::default();
        combat.change_clip(ask, true, 0.1);
        assert!(combat.changing.is_none());
    }

    #[test]
    fn an_emplaced_gun_turns_and_tilts_only_as_far_as_its_turret() {
        // `Emplaced MG (VZ)`'s `Turret`: -90..90 degrees round, -20..30 up.
        let clamp = |dir: Vec3| {
            turret_clamp(
                dir,
                0.0,
                (-90f32.to_radians(), 90f32.to_radians()),
                (-20f32.to_radians(), 30f32.to_radians()),
            )
        };
        // Ahead and a little to the side: as asked.
        let ahead = Vec3::new(0.3, 0.1, 1.0).normalize();
        assert!(clamp(ahead).distance(ahead) < 1e-5);
        // Behind it: as far round as it goes, to the nearer side.
        let back = clamp(Vec3::new(0.2, 0.0, -1.0).normalize());
        assert!(back.distance(Vec3::X) < 1e-5, "{back}");
        // Straight up: tilted its most.
        let up = clamp(Vec3::new(0.0, 1.0, 0.01).normalize());
        assert!((up.y.asin().to_degrees() - 30.0).abs() < 1e-3, "{up}");
    }

    #[test]
    fn a_pickup_refills_the_weapons_she_carries_by_their_ammunition() {
        let ammo = |template| Ammo {
            template,
            velocity: 0.0,
            max_velocity: 0.0,
            human_boost: 1.0,
            gravity: 0.0,
            damage: 0.0,
            damage_min: 0.0,
            falloff: (0.0, 0.0),
            hero: 1.0,
            key: None,
            blast: None,
            timer: None,
            timer_blast: None,
            homing: None,
            model: None,
            sticky: false,
        };
        let mut combat = Combat {
            carbine: Rifle {
                template: Some(10),
                ammo: Some(ammo(11)),
                ..Rifle::default()
            },
            ..Combat::default()
        };
        combat.set_thrown(
            Throwable {
                template: 20,
                name: "Grenade".to_owned(),
                model: None,
                ammo: ammo(21),
                max: MAX_GRENADES,
                speeds: [0.0; 3],
                pitches: [0.0; 3],
                throw_angle: 0.0,
                cook: false,
                detonator: None,
                planted: None,
            },
            None,
            None,
        );
        combat.carbine_only();
        // A record per kind of ammunition: the carbine's 3 clips, 2
        // grenades, and rounds for a launcher she doesn't carry.
        let records = [(99, 5), (11, 3), (21, 2), (31, 4)];
        let fires = |_: Option<u32>, fires: Option<u32>, ammo: u32| fires == Some(ammo);
        let refill = combat.refill(&records, fires);
        assert_eq!(
            refill,
            Refill {
                rifle: 90,
                launcher: 0,
                grenades: 2
            }
        );
        assert!(combat.can_use(refill, 0.0));
        combat.take_supplies(refill, 0.0);
        assert_eq!(
            combat.rifle_rounds,
            Rounds {
                clip: 30,
                stored: 90
            }
        );
        assert_eq!(combat.grenades, 2);
        // Ammunition for none of hers gives nothing; a weapon's own link
        // (`takes` by its template) counts as well as what it fires.
        assert!(!combat.can_use(combat.refill(&[(99, 5)], fires), 0.0));
        combat.launcher = Rifle {
            template: Some(30),
            ammo: Some(ammo(32)),
            ..Rifle::launcher(Weapon::GrenadeLauncher)
        };
        combat.second = Some(Weapon::GrenadeLauncher);
        combat.launcher_rounds = Some(Rounds::default());
        let own =
            |w: Option<u32>, f: Option<u32>, a: u32| f == Some(a) || (w == Some(30) && a == 31);
        assert_eq!(combat.refill(&records, own).launcher, 24);
        // Full, the rifle's rounds leave the pickup lying.
        combat.rifle_rounds.stored = combat.rifle.spare;
        let rifle_only = combat.refill(&[(11, 1)], fires);
        assert!(!combat.can_use(rifle_only, 0.0));
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
        // Taken from the crate with 3 clips: the RPG is left behind, with
        // its rockets (its template's clip of 1 and 3 more).
        assert_eq!(
            combat.equip(Weapon::GrenadeLauncher, 18),
            Some((Weapon::Rpg, 1 + RPG_SPARE))
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
        // One round a pull (its `FireType`): the trigger pulled and let go.
        for k in 0..(60.0 * 30.0) as usize {
            let t = if k % 2 == 0 { fire } else { Trigger::default() };
            step(&mut combat, &mut player, t);
        }
        assert_eq!(combat.launcher_rounds, Some(Rounds::default()));
        assert_eq!(combat.blasts, 18);
        let last = combat.last_blast.unwrap();
        assert!(last.with_y(0.0).distance(mark) < 1.0, "{last}");
        let lobs = combat
            .take_sounds()
            .iter()
            .filter(|s| matches!(s, Sound::Lob { .. }))
            .count();
        assert_eq!(lobs, 18);
        assert!(!combat.second_loaded());
        // More rounds go with it, two clips beyond the one at most (a
        // rocket pickup's 20 clips); the RPG taken back leaves it, with
        // them.
        let rockets = Refill {
            launcher: 120,
            ..Refill::default()
        };
        assert!(combat.can_use(rockets, 0.0));
        combat.take_supplies(rockets, 0.0);
        assert_eq!(combat.launcher_rounds.map(Rounds::total), Some(12));
        assert!(!combat.can_use(rockets, 0.0));
        combat.launcher_rounds = Some(Rounds::default());
        combat.take_ammo(Weapon::GrenadeLauncher, 6);
        assert!(combat.second_loaded());
        assert_eq!(
            combat.equip(Weapon::Rpg, 0),
            Some((Weapon::GrenadeLauncher, 6))
        );
        assert_eq!(
            (combat.second, combat.launcher_rounds),
            (Some(Weapon::Rpg), Some(Rounds::of(1 + RPG_SPARE, 1)))
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
