//! How the template database says a vehicle handles and fights: its mass
//! and top speed, how fast its turret turns and how far its gun tilts,
//! where its driver's camera sits, and how its main gun fires.
//!
//! The database keys fields by [`name_hash`] alone. The names used here
//! are guesses that hash to the fields' keys (`MaxSpeed`, `maxYawSpeed`,
//! `RateOfFire`, `iClipSize`, ...), each read against the AMX-30's values
//! and what its tank does in a playthrough: 45 km/h, a turret that keeps
//! up with the camera, a shot every 1.3 s and 40 rounds on the HUD.

use std::collections::HashMap;

use m2_assets::layer::Component;
use m2_assets::{Layer, name_hash};

/// What a template's components say of its handling and its main gun.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehicleTuning {
    /// Mass, kg (`Mass` of its `_TankPhysics`, or a wheeled vehicle's
    /// `_CarPhysicsV2`: the LAV III's 20 t).
    pub mass: Option<f32>,
    /// Top speed, m/s (`MaxSpeed`, which the database gives in km/h: the
    /// AMX-30's 45 is the 10-12 m/s its tank covers on a dirt track in
    /// the Xbox 360 playthrough, the LAV III's 85 the real one's road
    /// speed, near enough).
    pub max_speed: Option<f32>,
    /// The damage it takes before it is wrecked (its `Health`
    /// component's value, field d122919d: the AMX-30's 125, an M151's
    /// 100).
    pub health: Option<f32>,
    /// Its armour class (`ObjectHint`'s `Armor`): 1 none (cars, boats),
    /// 2 light (helicopters), 3 medium (the LAV III), 4 heavy (tanks).
    pub armour: Option<u32>,
    /// What blasts do to it, of their damage (`ExplosionFudge`'s field
    /// 6bcc541e: the AMX-30's 0.2, a truck's 0.4, a boat's 1.1).
    pub blast_scale: Option<f32>,
    /// Health below which it burns down by itself (`TickDamage`'s field
    /// 9a6663a4: the AMX-30's 30).
    pub burns_below: Option<f32>,
    /// What rounds do to it, of their damage, by its `ModifierKey` in the
    /// game's damage table (an M151's `Car`: a rifle round a fortieth).
    pub rounds: Option<RoundScale>,
    pub turret: Option<TurretTuning>,
    pub camera: Option<TankCamera>,
    pub gun: Option<GunTuning>,
}

/// A `Turret`: how fast it turns and how far its gun tilts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurretTuning {
    /// It is the template's `main_turret`, which the driver turns (not
    /// its only turret, taken for want of one so named).
    pub main: bool,
    /// Fastest turn, radians a second, and how quickly it gets there,
    /// radians a second squared (`maxYawSpeed`, `maxYawAccel`, degrees).
    pub yaw_speed: f32,
    pub yaw_accel: f32,
    /// How far the gun tilts down and up, radians (`MinPitch`,
    /// `MaxPitch`, degrees), as fast as `pitch_speed` (`maxPitchSpeed`).
    pub pitch: (f32, f32),
    pub pitch_speed: f32,
    pub pitch_accel: f32,
}

/// The damage table's scale for the two kinds of round: `Bullet` (rifles,
/// carbines) and `BulletLarge` (heavy machine guns).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RoundScale {
    pub bullet: f32,
    pub bullet_large: f32,
}

/// The game's damage table, asset `ModifierKey` (type de982d61, in the
/// resident block): what each kind of damage (a round's or blast's
/// `DamageKey`: `Bullet`, `Rocket`, `MeleeBash`, ...) does to each kind
/// of target (its `ModifierKey`: `Car`, `Tank`, `Human`, ...), of the
/// damage. A `Car` takes 0.025 of a `Bullet`, 0.1 of a `BulletLarge`, a
/// `Tank` nothing of either.
///
/// Layout (round 160): rows at +0x48 and columns at +0x4c (u32), from
/// +0x50 the rows' keys then the columns' (name hashes), then a float a
/// cell, row by row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DamageTable {
    rows: Vec<u32>,
    columns: Vec<u32>,
    cells: Vec<f32>,
}

/// The damage table's asset type.
pub const DAMAGE_TABLE_TYPE: u32 = 0xde98_2d61;
/// The damage table's asset name.
pub const DAMAGE_TABLE: &str = "ModifierKey";

impl DamageTable {
    pub fn parse(data: &[u8]) -> Option<Self> {
        let word = |at: usize| -> Option<u32> {
            let b = data.get(at..at + 4)?;
            Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let (rows, columns) = (word(0x48)? as usize, word(0x4c)? as usize);
        if rows == 0 || columns == 0 || rows * columns > 1 << 16 {
            return None;
        }
        let keys = 0x50;
        let cells = keys + 4 * (rows + columns);
        let words = |from: usize, n: usize| {
            (0..n)
                .map(|i| word(from + 4 * i))
                .collect::<Option<Vec<_>>>()
        };
        Some(Self {
            rows: words(keys, rows)?,
            columns: words(keys + 4 * rows, columns)?,
            cells: words(cells, rows * columns)?
                .into_iter()
                .map(f32::from_bits)
                .collect(),
        })
    }

    /// What damage of kind `damage` does to a target of kind `target`, of
    /// the damage (name hashes of the keys).
    pub fn scale(&self, target: u32, damage: u32) -> Option<f32> {
        let r = self.rows.iter().position(|&k| k == target)?;
        let c = self.columns.iter().position(|&k| k == damage)?;
        self.cells.get(r * self.columns.len() + c).copied()
    }

    /// The rounds' scales for a target of kind `target`.
    pub fn rounds(&self, target: u32) -> Option<RoundScale> {
        Some(RoundScale {
            bullet: self.scale(target, name_hash("Bullet"))?,
            bullet_large: self.scale(target, name_hash("BulletLarge"))?,
        })
    }
}

/// A `CameraTank`: the camera of the seat that turns the turret, which
/// turns about a point above the turret and keeps behind wherever the
/// turret aims.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TankCamera {
    /// How far behind the point it turns about, metres (`CameraOffset`).
    pub distance: f32,
    /// How far that point is above the turret ring, metres (field
    /// `2afa6ae4`, name unknown: 1.4 for the AMX-30, which puts the
    /// playthrough's crosshair just above its turret).
    pub height: f32,
    /// Where it looks when the drive starts and how far it may look down
    /// and up, radians, up positive (`DefaultPitch`, `MinPitch`,
    /// `MaxPitch`).
    pub pitch: f32,
    pub pitch_range: (f32, f32),
}

/// A main gun: the weapon its `PhysicalLink` mounts, the ammunition that
/// weapon fires, and its muzzle flash.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GunTuning {
    /// The weapon's template id (`Mounted Tank Gun (Default)`).
    pub weapon: u32,
    /// Shots a minute (`RateOfFire`).
    pub rate_of_fire: f32,
    /// Rounds it carries (`iClipSize`).
    pub rounds: u32,
    /// The shell's speed leaving the muzzle and its most, m/s (its
    /// ammunition's `Velocity` and `MaxVelocity`), and how hard gravity
    /// pulls it (`GravityScale`).
    pub velocity: f32,
    pub max_velocity: f32,
    pub gravity_scale: f32,
    /// How long its muzzle flash burns, seconds (the flash template's
    /// effect `duration`).
    pub flash: Option<f32>,
    /// The name hash of its shot's sound bank (`wpn_tankgun`).
    pub sound_bank: Option<u32>,
}

/// `PhysicalLink`'s field naming the template it mounts (name unknown).
const LINK: u32 = 0x5470_4554;
/// The `Turret` the driver's seat turns (`Vehicle Seat (Tank Default)
/// (Driver)` names it in its `ControllerTurret`): a tank's gun turret,
/// not the machine gun on its cupola, which has a `Turret` of its own.
const MAIN_TURRET: &str = "main_turret";
/// `SoundEffect`'s field naming a sound bank (see `ENGINE_BANK_FIELD`).
const BANK: u32 = 0x14a6_7fa6;
/// `CameraTank`'s height above the turret ring (name unknown).
const CAMERA_HEIGHT: u32 = 0x2afa_6ae4;
/// The seats whose camera the player gets, best first: driving, or
/// firing the main gun (the LAV III's gunner; its template also has a
/// camera for the rear passenger's hatch, `seat_passenger_rr`).
const CAMERA_SEATS: [&str; 2] = ["seat_driver", "seat_gunner"];
/// Unnamed fields (round 154): `Health`'s value, `ExplosionFudge`'s
/// scale and `TickDamage`'s threshold.
const HEALTH: u32 = 0xd122_919d;
const BLAST_SCALE: u32 = 0x6bcc_541e;
const BURNS_BELOW: u32 = 0x9a66_63a4;

/// One component's records by template id, and its fields by name hash.
pub(crate) struct Table<'a> {
    fields: HashMap<u32, (usize, u8, u8)>,
    records: HashMap<u32, Vec<&'a [u8]>>,
}

impl<'a> Table<'a> {
    pub(crate) fn new(db: &Layer<'a>, component: &str) -> Self {
        let Some(c) = db.component(component) else {
            return Self {
                fields: HashMap::new(),
                records: HashMap::new(),
            };
        };
        Self::of(c)
    }

    pub(crate) fn of(c: &Component<'a>) -> Self {
        let mut records: HashMap<u32, Vec<&'a [u8]>> = HashMap::new();
        for (id, r) in c.records().unwrap_or_default() {
            records.entry(id).or_default().push(r);
        }
        Self {
            fields: c
                .fields
                .iter()
                .map(|f| {
                    (
                        f.name_hash,
                        (usize::from(f.offset), f.bit_shift, f.bit_width),
                    )
                })
                .collect(),
            records,
        }
    }

    /// The templates with a record.
    pub(crate) fn templates(&self) -> impl Iterator<Item = u32> + '_ {
        self.records.keys().copied()
    }

    /// Template `id`'s records (a component may be on it more than once).
    pub(crate) fn of_template(&self, id: u32) -> &[&'a [u8]] {
        self.records.get(&id).map_or(&[], Vec::as_slice)
    }

    /// Field `field`'s word in `record`, its bits if it is a bit field.
    pub(crate) fn word(&self, record: &[u8], field: u32) -> Option<u32> {
        let &(at, shift, width) = self.fields.get(&field)?;
        let b = record.get(at..at + 4)?;
        let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        Some(if (1..32).contains(&width) {
            (v >> shift) & ((1 << width) - 1)
        } else {
            v
        })
    }

    pub(crate) fn float(&self, record: &[u8], field: u32) -> Option<f32> {
        self.word(record, field)
            .map(f32::from_bits)
            .filter(|f| f.is_finite())
    }

    /// Field `field` of template `id`'s first record as a number, stored
    /// as a float or a whole number.
    fn number(&self, id: u32, field: u32) -> Option<f32> {
        let w = self.word(self.of_template(id).first()?, field)?;
        let f = f32::from_bits(w);
        Some(if f.is_normal() { f } else { w as f32 }).filter(|v| (0.0..1e7).contains(v))
    }

    /// Field `name` of template `id`'s first record.
    fn get(&self, id: u32, name: &str) -> Option<f32> {
        self.float(self.of_template(id).first()?, name_hash(name))
    }

    /// Template `id`'s record whose `Name` is `name`.
    fn named(&self, id: u32, name: &str) -> Option<&'a [u8]> {
        let (field, want) = (name_hash("Name"), name_hash(name));
        self.of_template(id)
            .iter()
            .copied()
            .find(|r| self.word(r, field) == Some(want))
    }
}

/// The tuning of every template the database gives any, by template id.
/// `damage` is the game's damage table, if found.
/// `_HumanPhysics` fields that differ between the heroes as their
/// speeds do (heroes round 02; names unsolved, so read as speeds by
/// inference): Jennifer 2.0, 5.0 and 7.0, Chris 1.5, 4.5 and 6.25,
/// Mattias 1.5, 4.5 and 5.5. Her run (5.0) is the alpha's run speed.
const HUMAN_SPEEDS: [u32; 3] = [0xb535_3104, 0x5cc8_c4d9, 0x6167_d5fc];

/// Every person template's three `_HumanPhysics` speeds (walk, run,
/// sprint; inferred, see [`HUMAN_SPEEDS`]), by id.
pub fn human_speeds(db: &Layer<'_>) -> HashMap<u32, [f32; 3]> {
    let physics = Table::new(db, "_HumanPhysics");
    physics
        .records
        .keys()
        .filter_map(|&id| {
            let record = physics.of_template(id).first()?;
            let mut out = [0.0; 3];
            for (o, field) in out.iter_mut().zip(HUMAN_SPEEDS) {
                *o = f32::from_bits(physics.word(record, field)?);
            }
            out.iter()
                .all(|v| v.is_normal() && *v > 0.0)
                .then_some((id, out))
        })
        .collect()
}

/// Every template's `Health` value, by id: a vehicle's, a person's, a
/// hero's (the `Mattias` template's 120).
pub fn healths(db: &Layer<'_>) -> HashMap<u32, f32> {
    let health = Table::new(db, "Health");
    health
        .records
        .keys()
        .filter_map(|&id| Some((id, health.number(id, HEALTH)?)))
        .collect()
}

/// A hand-held gun that fires rounds (`WeaponProjectileBase` with
/// `FireType` 0; the RPG and the grenade launcher are 1): what a hero
/// carries as their rifle. The Carbine's 562 a minute, 30 a clip, 9
/// clips spare and its round's 6; the Bullpup Rifle's 487, 30, 7 and 8;
/// the Assault Rifle's 450, 30, 7 and 9 (round 137's dump).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HandGun {
    /// The weapon's template id.
    pub template: u32,
    /// Its `ModelName` (`global_weapon_bullpup`).
    pub model: Option<u32>,
    /// Shots a minute (`RateOfFire`).
    pub rate_of_fire: f32,
    /// Rounds a clip (`iClipSize`, two bytes) and clips kept beyond it
    /// (`MaxAmmoReserve`, one byte: it counts clips).
    pub clip: u32,
    pub reserve_clips: u32,
    /// Its round's `ProjectilePhysics` `Damage`, and the round's
    /// `DamageKey` (name hash; `Bullet` for every rifle).
    pub damage: f32,
    pub damage_key: Option<u32>,
}

/// Every hand-held gun that fires rounds ([`HandGun`]), by template id.
pub fn hand_guns(db: &Layer<'_>) -> HashMap<u32, HandGun> {
    let weapon = Table::new(db, "WeaponProjectileBase");
    let physics = Table::new(db, "ProjectilePhysics");
    let key = Table::new(db, "DamageKey");
    let models = db.hashes("ModelName");
    weapon
        .templates()
        .filter_map(|id| {
            let r = *weapon.of_template(id).first()?;
            if weapon.word(r, name_hash("FireType"))? != 0 {
                return None;
            }
            let ammo = weapon.word(r, name_hash("AmmoTemplate"))?;
            Some((
                id,
                HandGun {
                    template: id,
                    model: models.get(&id).copied().filter(|&m| m != 0),
                    rate_of_fire: weapon
                        .float(r, name_hash("RateOfFire"))
                        .filter(|&f| f > 0.0)?,
                    clip: weapon.word(r, name_hash("iClipSize"))? & 0xffff,
                    reserve_clips: weapon.word(r, name_hash("MaxAmmoReserve"))? & 0xff,
                    damage: physics.get(ammo, "Damage")?,
                    damage_key: key
                        .of_template(ammo)
                        .first()
                        .and_then(|k| key.word(k, name_hash("DamageKey"))),
                },
            ))
        })
        .collect()
}

/// What each template's `EquipmentLink`s hand it (a hero's `Grenade`,
/// `C4`, `RPG` and own rifle), template ids in the database's order.
pub fn equipment(db: &Layer<'_>) -> HashMap<u32, Vec<u32>> {
    let link = Table::new(db, "EquipmentLink");
    link.templates()
        .map(|id| {
            let items = link
                .of_template(id)
                .iter()
                .filter_map(|r| link.word(r, LINK))
                .filter(|&t| t != 0)
                .collect();
            (id, items)
        })
        .collect()
}

pub fn read(db: &Layer<'_>, damage: Option<&DamageTable>) -> HashMap<u32, VehicleTuning> {
    let tank = Table::new(db, "_TankPhysics");
    let car = Table::new(db, "_CarPhysicsV2");
    // A boat's, if it names them as the others do.
    let boat = Table::new(db, "_BoatPhysics");
    let turret = Table::new(db, "Turret");
    let camera = Table::new(db, "CameraTank");
    let links = Table::new(db, "PhysicalLink");
    let health = Table::new(db, "Health");
    let hint = Table::new(db, "ObjectHint");
    let fudge = Table::new(db, "ExplosionFudge");
    let tick = Table::new(db, "TickDamage");
    let modifier = Table::new(db, "ModifierKey");
    let guns = Guns {
        weapon: Table::new(db, "WeaponProjectileBase"),
        effects: Table::new(db, "WeaponEffects"),
        sound: Table::new(db, "SoundEffect"),
        velocity: Table::new(db, "ControllerVelocity"),
        physics: Table::new(db, "ProjectilePhysics"),
        effect: Table::new(db, "RedEffectComponent"),
    };
    let mut ids: Vec<u32> = tank
        .records
        .keys()
        .chain(car.records.keys())
        .chain(boat.records.keys())
        .chain(turret.records.keys())
        .chain(camera.records.keys())
        .copied()
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter()
        .map(|id| {
            let t = VehicleTuning {
                mass: [&tank, &car, &boat]
                    .iter()
                    .find_map(|t| t.get(id, "Mass").filter(|m| *m > 0.0)),
                max_speed: [&tank, &car, &boat]
                    .iter()
                    .find_map(|t| t.get(id, "MaxSpeed").filter(|s| *s > 0.0))
                    .map(|kmh| kmh / 3.6),
                health: health.number(id, HEALTH),
                armour: hint
                    .of_template(id)
                    .first()
                    .and_then(|r| hint.word(r, name_hash("Armor")))
                    .map(|a| a & 0xff)
                    .filter(|a| (1..=4).contains(a)),
                blast_scale: fudge.number(id, BLAST_SCALE),
                burns_below: tick.number(id, BURNS_BELOW),
                // The whole vehicle's key: its record with no `Name` (the
                // named ones are parts).
                rounds: modifier
                    .of_template(id)
                    .iter()
                    .find(|r| modifier.word(r, name_hash("Name")).is_none_or(|n| n == 0))
                    .and_then(|r| modifier.word(r, name_hash("ModifierKey")))
                    .and_then(|key| damage?.rounds(key)),
                turret: (|| {
                    let named = turret.named(id, MAIN_TURRET);
                    let r = match (named, turret.of_template(id)) {
                        (Some(r), _) => r,
                        (None, [only]) => only,
                        _ => return None,
                    };
                    let deg = |name| turret.float(r, name_hash(name)).map(f32::to_radians);
                    Some(TurretTuning {
                        main: named.is_some(),
                        yaw_speed: deg("maxYawSpeed")?,
                        yaw_accel: deg("maxYawAccel")?,
                        pitch: (deg("MinPitch")?, deg("MaxPitch")?),
                        pitch_speed: deg("maxPitchSpeed")?,
                        pitch_accel: deg("maxPitchAccel")?,
                    })
                })(),
                camera: (|| {
                    let records = camera.of_template(id);
                    let seat = name_hash("SeatName");
                    let r = CAMERA_SEATS
                        .iter()
                        .find_map(|&s| {
                            records
                                .iter()
                                .find(|r| camera.word(r, seat) == Some(name_hash(s)))
                        })
                        .or(records.first())
                        .copied()?;
                    Some(TankCamera {
                        distance: camera.float(r, name_hash("CameraOffset"))?,
                        height: camera.float(r, CAMERA_HEIGHT)?,
                        pitch: camera.float(r, name_hash("DefaultPitch"))?,
                        pitch_range: (
                            camera.float(r, name_hash("MinPitch"))?,
                            camera.float(r, name_hash("MaxPitch"))?,
                        ),
                    })
                })(),
                // Its main gun: of the weapons it mounts, the slowest to
                // fire (a tank's cannon, not its machine guns).
                gun: links
                    .of_template(id)
                    .iter()
                    .filter_map(|r| links.word(r, LINK))
                    .filter_map(|weapon| guns.gun(weapon))
                    .min_by(|a, b| a.rate_of_fire.total_cmp(&b.rate_of_fire)),
            };
            (id, t)
        })
        .collect()
}

/// The components a mounted gun's tuning comes from.
struct Guns<'a> {
    weapon: Table<'a>,
    effects: Table<'a>,
    sound: Table<'a>,
    velocity: Table<'a>,
    physics: Table<'a>,
    effect: Table<'a>,
}

impl Guns<'_> {
    /// Weapon template `weapon`'s, if it fires ammunition.
    fn gun(&self, weapon: u32) -> Option<GunTuning> {
        let r = *self.weapon.of_template(weapon).first()?;
        let ammo = self.weapon.word(r, name_hash("AmmoTemplate"))?;
        let flash = self
            .effects
            .of_template(weapon)
            .first()
            .and_then(|e| self.effects.word(e, name_hash("MuzzleFlashTemplate")))
            .and_then(|f| self.effect.get(f, "duration"));
        Some(GunTuning {
            weapon,
            rate_of_fire: self.weapon.float(r, name_hash("RateOfFire"))?,
            rounds: self.weapon.word(r, name_hash("iClipSize"))?,
            velocity: self.velocity.get(ammo, "Velocity")?,
            max_velocity: self.velocity.get(ammo, "MaxVelocity")?,
            gravity_scale: self.physics.get(ammo, "GravityScale").unwrap_or(1.0),
            flash,
            sound_bank: self
                .sound
                .of_template(weapon)
                .first()
                .and_then(|s| self.sound.word(s, BANK))
                .filter(|&b| b != 0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The field names hash to the keys the AMX-30's components use.
    #[test]
    fn the_guessed_names_hash_to_the_fields_keys() {
        for (name, key) in [
            ("Mass", 0xc3ae_b321),
            ("MaxSpeed", 0x9f12_33ca),
            ("maxYawSpeed", 0x0a7b_ae57),
            ("maxYawAccel", 0x7762_a022),
            ("MinPitch", 0x131f_787b),
            ("MaxPitch", 0x6c06_9f1d),
            ("maxPitchSpeed", 0x3c20_4efe),
            ("maxPitchAccel", 0xc0d9_879b),
            ("CameraOffset", 0x4fea_ee29),
            ("DefaultPitch", 0x937b_a9a2),
            ("RateOfFire", 0xbe7a_7a5c),
            ("iClipSize", 0x3e22_63d3),
            ("AmmoTemplate", 0xc5a7_d745),
            ("MuzzleFlashTemplate", 0x87ac_b4be),
            ("Velocity", 0x84c8_39c8),
            ("GravityScale", 0x0e57_f8c1),
            ("Name", 0x1de5_c824),
            (MAIN_TURRET, 0x801f_2cf7),
            ("SeatName", 0xbdd7_64f3),
            ("seat_gunner", 0xb7b9_67d0),
            ("seat_passenger_rr", 0x300a_f37e),
            ("Bullet", 0x74fd_1ba9),
            ("BulletLarge", 0x7f01_0e06),
            ("Car", 0xce27_c791),
            ("Explosion", 0x1693_0afe),
            ("ExplosionLarge", 0xdc0a_895f),
            (DAMAGE_TABLE, 0x99c2_b81f),
        ] {
            assert_eq!(name_hash(name), key, "{name}");
        }
    }

    // A table laid out as the game's: two targets, three kinds of damage.
    #[test]
    fn the_damage_table_reads_a_cell_by_its_keys() {
        let mut data = vec![0u8; 0x48];
        let mut put = |v: u32| data.extend_from_slice(&v.to_le_bytes());
        put(2);
        put(3);
        let (car, tank) = (name_hash("Car"), name_hash("Tank"));
        let (bullet, large, rocket) = (
            name_hash("Bullet"),
            name_hash("BulletLarge"),
            name_hash("Rocket"),
        );
        for k in [car, tank, bullet, large, rocket] {
            put(k);
        }
        for v in [0.025f32, 0.1, 1.0, 0.0, 0.0, 0.2] {
            put(v.to_bits());
        }
        let t = DamageTable::parse(&data).expect("parses");
        assert_eq!(t.scale(car, rocket), Some(1.0));
        assert_eq!(t.scale(tank, rocket), Some(0.2));
        assert_eq!(
            t.rounds(car),
            Some(RoundScale {
                bullet: 0.025,
                bullet_large: 0.1
            })
        );
        assert_eq!(t.scale(name_hash("Human"), bullet), None);
        assert!(DamageTable::parse(&data[..data.len() - 1]).is_none());
    }
}
