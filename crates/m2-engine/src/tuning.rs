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

/// One component's records by template id, and its fields by name hash.
struct Table<'a> {
    fields: HashMap<u32, (usize, u8, u8)>,
    records: HashMap<u32, Vec<&'a [u8]>>,
}

impl<'a> Table<'a> {
    fn new(db: &Layer<'a>, component: &str) -> Self {
        let Some(c) = db.component(component) else {
            return Self {
                fields: HashMap::new(),
                records: HashMap::new(),
            };
        };
        Self::of(c)
    }

    fn of(c: &Component<'a>) -> Self {
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

    /// Template `id`'s records (a component may be on it more than once).
    fn of_template(&self, id: u32) -> &[&'a [u8]] {
        self.records.get(&id).map_or(&[], Vec::as_slice)
    }

    /// Field `field`'s word in `record`, its bits if it is a bit field.
    fn word(&self, record: &[u8], field: u32) -> Option<u32> {
        let &(at, shift, width) = self.fields.get(&field)?;
        let b = record.get(at..at + 4)?;
        let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        Some(if (1..32).contains(&width) {
            (v >> shift) & ((1 << width) - 1)
        } else {
            v
        })
    }

    fn float(&self, record: &[u8], field: u32) -> Option<f32> {
        self.word(record, field)
            .map(f32::from_bits)
            .filter(|f| f.is_finite())
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
pub fn read(db: &Layer<'_>) -> HashMap<u32, VehicleTuning> {
    let tank = Table::new(db, "_TankPhysics");
    let car = Table::new(db, "_CarPhysicsV2");
    let turret = Table::new(db, "Turret");
    let camera = Table::new(db, "CameraTank");
    let links = Table::new(db, "PhysicalLink");
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
        .chain(turret.records.keys())
        .chain(camera.records.keys())
        .copied()
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter()
        .map(|id| {
            let t = VehicleTuning {
                mass: [&tank, &car]
                    .iter()
                    .find_map(|t| t.get(id, "Mass").filter(|m| *m > 0.0)),
                max_speed: [&tank, &car]
                    .iter()
                    .find_map(|t| t.get(id, "MaxSpeed").filter(|s| *s > 0.0))
                    .map(|kmh| kmh / 3.6),
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
        ] {
            assert_eq!(name_hash(name), key, "{name}");
        }
    }
}
