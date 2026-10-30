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
    /// How hard blasts throw it, of their usual (`ExplosionFudge`'s field
    /// 6bcc541e: the AMX-30's 0.2, a truck's 0.4, a boat's 1.1).
    pub blast_scale: Option<f32>,
    /// Health below which it burns down by itself (`TickDamage`'s field
    /// 9a6663a4: the AMX-30's 30).
    pub burns_below: Option<f32>,
    /// What rounds do to it, of their damage, by its `ModifierKey` in the
    /// game's damage table (an M151's `Car`: a rifle round a fortieth).
    pub rounds: Option<RoundScale>,
    /// That `ModifierKey` itself (`Car`, `Tank`), the whole vehicle's:
    /// its record with no `Name`.
    pub key: Option<u32>,
    pub turret: Option<TurretTuning>,
    pub camera: Option<TankCamera>,
    pub gun: Option<GunTuning>,
    /// A wheeled vehicle's engine and gearbox (`_CarPhysicsV2`).
    pub engine: Option<EngineTuning>,
    /// A wheeled vehicle's steering, brakes and grip (`_CarPhysicsV2`).
    pub handling: Option<Handling>,
    /// A wheeled vehicle's springs (`_CarPhysicsV2`).
    pub springs: Option<Springs>,
    /// A tracked vehicle's pull at a standstill, a third, two thirds and
    /// all of its top speed, of its most (`_TankPhysics`'s
    /// `EngineTorqueRatio0`..`3`, names found by hash: the AMX-30's 1,
    /// 0.45, 0.3, 0.1; read as a curve over its speed, inferred).
    pub tracks: Option<[f32; 4]>,
    /// A helicopter's handling (`_HelicopterPhysics`).
    pub flight: Option<FlightTuning>,
    /// A boat's engine and the water's drag on it (`_BoatPhysics`).
    pub boat: Option<BoatTuning>,
    /// A motorbike's balance and damping (`_CarPhysicsV2`'s `Bike*`
    /// fields).
    pub bike: Option<BikeTuning>,
    /// How far behind a helicopter its pilot's camera sits, metres, and
    /// how far it looks down, radians (`CameraHelicopter`'s field
    /// `ee30a43d`, name unknown: 20 for the light helicopters, 39 for the
    /// Mi-26; and `DefaultPitch`).
    pub chase: Option<(f32, f32)>,
}

/// How a helicopter flies, from its `_HelicopterPhysics` (names found by
/// hash, 2026-10-08; the AH-1Z's values in brackets).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlightTuning {
    /// Fastest climb or descent, forwards, sideways, m/s
    /// (`MaxElevationSpeed` 25, `MaxFwdSpeed` 30, `MaxRightSpeed` 30).
    pub climb: f32,
    pub forward: f32,
    pub sideways: f32,
    /// Fastest turn (`maxYawSpeed` 3.5) and, below `low_speed`
    /// (`LowSpeedLimit` 25), its turn on the spot (`LowSpeedYawSpeed`
    /// 6.5); units unknown.
    pub yaw: f32,
    pub low_speed: f32,
    pub low_speed_yaw: f32,
    /// How far the nose dips flying forwards, radians (`MaxPitch` 0.4),
    /// and how quickly (`PitchAccel` 0.9).
    pub pitch: f32,
    pub pitch_accel: f32,
    /// How far it rolls sliding sideways and banking into a turn,
    /// radians (`MaxRollSide` 0.3, `MaxRollBank` 0.3), and how quickly
    /// (`RollAccel` 1.0); it banks fully from `bank_speed` (`BankSpeedLimit`
    /// 25 m/s).
    pub roll_side: f32,
    pub roll_bank: f32,
    pub roll_accel: f32,
    pub bank_speed: f32,
}

/// A boat's engine and how the water slows it, from its `_BoatPhysics`
/// (names found by hash, gap round 17; the Speed Boat's values in
/// brackets, the range over the 110 records after them). Its top speed
/// is [`VehicleTuning::max_speed`] (`MaxSpeed` 120 km/h).
///
/// Named but not read here: `Steering`, `LowSpeedSteering`,
/// `LowSpeedLimit`, `SteerInertia`, `EngineInertia`, `HullFriction`,
/// `RollInertia`, `PitchUpSpeed`, `PitchDownInertia`, `HandbrakeInertia`,
/// `HandbrakeSteerBoost`; some 45 fields are still unnamed. The steering
/// fields are not a turn rate (gap-list sweep, `boatsteer` over the 25
/// distinct records): `Steering` rises with the boat's size, 0.3 on the
/// Jetski and the Dinghy to 4 on the LCUR landing craft and the PR's
/// Salton Seahorse, so as radians a second a landing craft would turn 13
/// times as fast as a jetski. It reads as a push whose scale needs the
/// hull's inertia, which no named field gives, so the alpha's one
/// invented turn rate stays. `LowSpeedLimit` (10 to 130, km/h as
/// `MaxSpeed` is, inferred) splits `Steering` from `LowSpeedSteering`
/// (0 on 7 records: no steering slow, inferred); `SteerInertia` (0 to
/// 0.98) and the other `*Inertia` read as a share kept each step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoatTuning {
    /// Top speed backing up, m/s (`MaxSpeedReverse`, km/h as `MaxSpeed`
    /// is: 40; 6 to 45).
    pub reverse_speed: f32,
    /// Its engine's pull ahead and backing up (`EngineTorque` 1,000 and
    /// `EngineReverseTorque` 500; 30 to 2,500 and 10 to 3,000). Only
    /// their ratio is read: backing up it pulls that share of what it
    /// pulls ahead (inferred: the Air Boat's 0.5 and the Barco's 0.44 of
    /// their `MaxSpeed` are their `MaxSpeedReverse`). The units are
    /// unknown: the torque over the mass varies 12-fold between boats
    /// whose drag and top speed ask the same pull.
    pub torque: (f32, f32),
    /// How quickly the water slows it along, across and up and down, a
    /// second (`WaterDragFwd` 0.2, `WaterDragSide` 2, `WaterDragUp` 2;
    /// 0.1 to 0.5, 0.5 to 10, 0.5 to 26), read as a linear drag's rate
    /// (inferred: they sit where the alpha's invented 0.3, 2.5 and 3
    /// did; the Air Boat's sideways 0.5 lets it skid, the landing craft's
    /// 10 holds it to its course).
    pub drag: [f32; 3],
    /// How much more its brake slows it, a second (`BrakeDrag` 3; 0 to 8;
    /// added to its drag along, inferred).
    pub brake: f32,
    /// Its pull at a standstill, a third, two thirds and all of its top
    /// speed (`EngineTorqueRatio0`..`3`, as a tank's are named: 1, 0.7,
    /// 0.6, 0.5; read as a curve over its speed, as a tank's are,
    /// inferred). Only their shape is read: scaled so that at its top
    /// speed it pulls what holds it there.
    pub curve: [f32; 4],
}

impl BoatTuning {
    /// Its pull flat out at `speed` m/s, m/s²: at `top` m/s, what holds
    /// it there against its drag along (inferred: the data gives no pull
    /// its torque fits, see [`torque`](Self::torque)), more or less
    /// below it as its [`curve`](Self::curve) says; backing up, its
    /// share of that.
    pub fn push(&self, speed: f32, top: f32, backwards: bool) -> f32 {
        let c = self.curve;
        let shape = if c[3] > 0.0 && top > 0.0 {
            let at = (speed.abs() / top).clamp(0.0, 1.0) * 3.0;
            let i = (at as usize).min(2);
            (c[i] + (c[i + 1] - c[i]) * (at - i as f32)) / c[3]
        } else {
            1.0
        };
        let ahead = self.drag[0] * top * shape;
        if backwards {
            ahead * (self.torque.1 / self.torque.0).clamp(0.0, 2.0)
        } else {
            ahead
        }
    }
}

/// How a motorbike holds itself up, from its `_CarPhysicsV2`'s `Bike*`
/// fields (names found by hash, gap round 17; read here gap round 18;
/// the Offroad Motorcycle (AL)'s values in brackets, the range over the
/// 11 distinct bike records after them). Only bikes give them (every
/// other record's `BikeRollStabilityStrength` is 0).
///
/// Read as the suspension's strengths and damps are, per kilogram the
/// bike weighs (inferred from samples, not checked against the game's
/// code): a torque of strength x lean x mass, newton-metres. Gravity
/// tips a bike with 9.81 x its centre of mass's height (0.33 to 0.48 m
/// in the alpha's bodies), 3.3 to 4.7 N m/kg a radian, so even the
/// weakest record (the player's off-road bike's 10) holds itself up
/// twice over, and the AI's 100 and the street `Motorcycle`'s 250 with
/// room to spare. Read the other way, as a turning acceleration per
/// radian, the five records at 10 to 20 barely beat gravity: shoved at
/// speed they hang leaning over 3 degrees for 6 s (biketest, gap round
/// 18), where per kilogram every bike stands back up within 1.7 s. The damps against
/// its roll, pitch and yaw, and its control in the air, read per
/// kilogram alike: in the air its stick then turns it at control / damp
/// rad/s, 0.2 to 0.5 for the nose (the AI-only bike's 3.5).
///
/// Named but not read: `BikeWheeliePitch` (0.85; 0.5 to 1) and
/// `BikeWheelieSideDamp` (10; 10 throughout), as the alpha has no
/// wheelie. The four fields between them (@148 to @154) are unnamed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BikeTuning {
    /// What holds it up towards the lean that balances its turn,
    /// newton-metres per radian off it, per kilogram
    /// (`BikeRollStabilityStrength` 10; 10 to 250).
    pub roll: f32,
    /// Damping against its roll, pitch and yaw, newton-metres per rad/s,
    /// per kilogram (`BikeExtraDampRoll` 6, `BikeExtraDampPitch` 10,
    /// `BikeExtraDampYaw` 0; 5 to 50, 10 throughout, 0 to 3).
    pub damp: [f32; 3],
    /// In the air, what its stick turns its nose and its heading with,
    /// newton-metres per kilogram at full stick (`BikeAirControlPitch`
    /// 2, `BikeAirControlYaw` 1.5; 1 to 35, 1.5 to 4).
    pub air: (f32, f32),
}

/// A wheeled vehicle's engine and gearbox, from its `_CarPhysicsV2`
/// (names found by hash, round 18 of the playtest fixes; the M151's
/// values in brackets). The fields are those of a Havok vehicle's
/// engine and transmission (`minRPM`, `optRPM`, `maxRPM`, upshift and
/// downshift RPM, primary ratio, a ratio a gear), so the game's own top
/// speeds come out of them: the M151 in fifth at its optimum 3,000 rpm
/// runs at 84 km/h against its `MaxSpeed` of 85.
///
/// Not read: `GearSlipRPM` (0, 1,000 or 2,000; Havok's clutch slip,
/// meaning here unchecked), and the six fields after `GearRatio5`
/// (@bc to @d0, unnamed: no name tried hashes to them). Five of the six
/// are 1 in every record, so a factor of them changes nothing; the
/// first is 0.5 on the Sidecar Motorcycle and the Sportbike, 1.5 on the
/// Phoenix and the RTR and 0.7 on the Garbage Truck (31 of 454
/// records), its meaning unknown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EngineTuning {
    /// Most torque, newton-metres (`EngineTorque` 130).
    pub torque: f32,
    /// Engine speeds, rpm: idle, the most torque, the most it turns
    /// (`GearMinRPM` 900, `GearOptRPM` 3,000, `GearMaxRPM` 12,000).
    pub rpm: (f32, f32, f32),
    /// It changes up a gear above this, rpm (`GearUpshiftRPM` 4,000),
    /// and down one below this (`GearDownshiftRPM` 2,500; 1,500 to
    /// 5,500 over the 454 records).
    pub upshift: f32,
    pub downshift: f32,
    /// How long a change of gear takes, seconds, the engine driving
    /// nothing meanwhile (`ClutchDelayTime` 0; 0 in 307 of the 454
    /// records, up to 1.75).
    pub clutch_delay: f32,
    /// The final drive's ratio (`GearRatioPrimary` 3.5), each forward
    /// gear's (`GearRatio1`..`5`: 4, 3.1, 2.4, 2, 1.5; a 0 is no gear)
    /// and reverse's (`GearRatioReverse` 3.7).
    pub primary: f32,
    pub gears: [f32; 5],
    pub reverse: f32,
    /// The driven wheels' radius, metres (`RearWheelRadius` 0.39).
    pub radius: f32,
    /// Of its most torque, what it gives at idle and at its most rpm
    /// (`TorqueFactorAtMinRPM` 0.8 and `TorqueFactorAtMaxRPM` 1.0; 0.5 to
    /// 1 both; names found by hash, gap round 24, as Havok's engine
    /// names them; between idle, the optimum and the most rpm read as
    /// straight lines, inferred).
    pub torque_at: (f32, f32),
    /// Top speed backing up, m/s (`ReverseMaxSpeed`, km/h: 55).
    pub reverse_speed: f32,
}

/// How a wheeled vehicle steers, brakes and holds the road, from its
/// `_CarPhysicsV2` (names found by hash, gap round 4; the M151's values
/// in brackets, the range over the 454 records after them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Handling {
    /// How far its front wheels turn, radians (`SteerMaxAngle` 0.7; 0.5
    /// to 1.1, 29 to 63 degrees; radians inferred, as the database gives
    /// the helicopters' and cameras' angles in them too).
    pub steer_lock: f32,
    /// Each axle's brake, front and back (`FrontBrake` 7, `RearBrake` 6;
    /// 2.5 to 11 and 1 to 13). Read as the slowing it gives the whole
    /// vehicle, m/s², if every wheel braked as that axle's do (inferred:
    /// the heavy trucks' are the lowest, the garbage truck's 4 and 1, the
    /// light cars' and bikes' the highest).
    pub brake: (f32, f32),
    /// Each axle's sideways grip (`FrontWheelFrictionSide` 1.5,
    /// `RearWheelFrictionSide` 1.5; 0.5 to 3 and 0.7 to 5), read as the
    /// tyres' friction coefficient (inferred).
    pub grip: (f32, f32),
    /// How quickly its front wheels swing towards where the driver
    /// steers, radians a second (`SteerSpeed` 0.6; 0.1 to 10; gap round
    /// 14, the unit inferred, as the lock's is radians).
    pub steer_speed: f32,
    /// Below this speed, m/s, they swing at `low_steer_speed` instead
    /// (`LowSpeedLimit` 35 and `LowSpeedSteerSpeed` 0.9; 0 to 80 and 0
    /// to 6). The limit read as km/h, as its `MaxSpeed` is (inferred);
    /// the pair read as the helicopters' `LowSpeedLimit` and
    /// `LowSpeedYawSpeed` are. 0 when its template gives no limit (7 of
    /// the 454 records).
    pub low_speed: f32,
    pub low_steer_speed: f32,
}

impl Handling {
    /// How quickly its front wheels swing at `speed` m/s, radians a
    /// second.
    pub fn swing(&self, speed: f32) -> f32 {
        if speed.abs() < self.low_speed {
            self.low_steer_speed
        } else {
            self.steer_speed
        }
    }
}

/// A wheeled vehicle's springs, from its `_CarPhysicsV2` (the M151's
/// values in brackets), as the Xbox 360 code builds and drives each
/// wheel (from the 360 code, driving round 1: the wheel set-up at
/// 0x8219EF90 and the wheel step at 0x8219F510):
///
/// - Each wheel's spring hangs from a point its `*WheelSuspCmpLength`
///   above where the model has the wheel's centre, and is
///   `ExtraInvisibleSpringLength + *WheelSuspExpLength +
///   *WheelSuspCmpLength` long unloaded ([`Springs::length`]). The
///   wheel is cast down that far plus its radius; fully stretched it
///   hangs `ExtraInvisibleSpringLength + *WheelSuspExpLength` below the
///   model's wheel, squeezed flat `*WheelSuspCmpLength` above it.
/// - Touching the ground, it pushes the chassis along the contact's
///   normal with mass x (strength x squeeze - damp x stretching speed),
///   newtons, where squeeze is how much shorter than unloaded it is and
///   the stretching speed its length's change over the step: one damp,
///   squeezed or stretched.
/// - With more than half its wheels on the ground the chassis is pulled
///   down by (`GravityFactor` - 1) x its weight more (at 0x8219EE14).
///
/// A field the record leaves out takes the 360 code's own default
/// (0x823EB210): 40.1 and 3 for a spring's strength and damp, 0.2 m
/// for each of its lengths, 0 extra, a `GravityFactor` of 2.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Springs {
    /// Each axle's spring, front and back (`FrontWheelSuspStrength` 25,
    /// `RearWheelSuspStrength` 25; 4.5 to 80 and 8 to 80): its push per
    /// metre squeezed, per kilogram the vehicle weighs.
    pub strength: (f32, f32),
    /// Each axle's damp (`FrontWheelSuspDamp` 1.7, `RearWheelSuspDamp`
    /// 1.7), per kilogram as the strength is.
    pub damping: (f32, f32),
    /// How far each axle's spring stretches below the model's wheel and
    /// squeezes above it, metres (`*WheelSuspExpLength`,
    /// `*WheelSuspCmpLength`).
    pub stretch: (f32, f32),
    pub squeeze: (f32, f32),
    /// Unloaded length every spring has besides those, metres
    /// (`ExtraInvisibleSpringLength`).
    pub extra: f32,
    /// Its weight on the ground, of its plain weight (`GravityFactor`).
    pub gravity_factor: f32,
}

impl Springs {
    /// The 360 code's defaults, for a vehicle with no `_CarPhysicsV2`.
    pub const DEFAULT: Springs = Springs {
        strength: (40.1, 40.1),
        damping: (3.0, 3.0),
        stretch: (0.2, 0.2),
        squeeze: (0.2, 0.2),
        extra: 0.0,
        gravity_factor: 2.0,
    };

    /// One axle's (front, the steering one, or back) strength, damp,
    /// unloaded length and how far its spring's top sits above the
    /// model's wheel centre, metres.
    pub fn axle(&self, front: bool) -> (f32, f32, f32, f32) {
        let pick = |p: (f32, f32)| if front { p.0 } else { p.1 };
        let squeeze = pick(self.squeeze);
        (
            pick(self.strength),
            pick(self.damping),
            self.extra + pick(self.stretch) + squeeze,
            squeeze,
        )
    }

    /// An axle's spring's unloaded length, metres.
    pub fn length(&self, front: bool) -> f32 {
        self.axle(front).2
    }
}

impl EngineTuning {
    /// How many forward gears it has.
    pub fn gear_count(&self) -> usize {
        self.gears.iter().take_while(|&&g| g > 0.0).count().max(1)
    }

    /// How fast the engine turns, rpm, in a gear of `ratio` at `speed`
    /// m/s.
    pub fn rpm_at(&self, ratio: f32, speed: f32) -> f32 {
        speed.abs() / self.radius * self.primary * ratio * 60.0 / std::f32::consts::TAU
    }

    /// The gear it settles in at `speed` m/s (0 is first): the lowest
    /// that keeps the engine below its upshift speed, else its top gear.
    /// What it drives in as it goes is [`shift`](Self::shift)'s.
    pub fn gear(&self, speed: f32) -> usize {
        let n = self.gear_count();
        (0..n)
            .find(|&g| self.rpm_at(self.gears[g], speed) <= self.upshift)
            .unwrap_or(n - 1)
    }

    /// The gear it changes to from `gear` at `speed` m/s: up one once
    /// the engine turns faster than its upshift speed (short of its top
    /// gear), down one once slower than its downshift speed (short of
    /// first), else the same. Read as a Havok gearbox's upshift and
    /// downshift RPM are, one gear at a time (inferred: the names are
    /// Havok's, and in all 454 records a change up lands the engine
    /// above the next gear's downshift speed, so it never changes
    /// straight back).
    pub fn shift(&self, gear: usize, speed: f32) -> usize {
        let n = self.gear_count();
        let gear = gear.min(n - 1);
        let rpm = self.rpm_at(self.gears[gear], speed);
        if rpm > self.upshift && gear + 1 < n {
            gear + 1
        } else if rpm < self.downshift && gear > 0 {
            gear - 1
        } else {
            gear
        }
    }

    /// Of its most torque, what the engine gives at `rpm`: rising from
    /// idle to the optimum, falling from there to its most, nothing
    /// beyond.
    pub fn torque_factor(&self, rpm: f32) -> f32 {
        let (min, opt, max) = self.rpm;
        let rpm = rpm.max(min);
        let along = |a: f32, b: f32| ((rpm - a) / (b - a).max(1.0)).clamp(0.0, 1.0);
        if rpm <= opt {
            self.torque_at.0 + (1.0 - self.torque_at.0) * along(min, opt)
        } else if rpm <= max {
            1.0 + (self.torque_at.1 - 1.0) * along(opt, max)
        } else {
            0.0
        }
    }

    /// How hard the driven wheels push flat out at `speed` m/s, newtons,
    /// forwards or backing up, in the gear it settles in there.
    pub fn push(&self, speed: f32, backwards: bool) -> f32 {
        self.push_in(self.gear(speed), speed, backwards)
    }

    /// How hard the driven wheels push flat out at `speed` m/s in `gear`
    /// (0 is first), newtons, or backing up in reverse.
    pub fn push_in(&self, gear: usize, speed: f32, backwards: bool) -> f32 {
        let ratio = if backwards {
            self.reverse
        } else {
            self.gears[gear.min(self.gear_count() - 1)]
        };
        let rpm = self.rpm_at(ratio, speed);
        self.torque * self.torque_factor(rpm) * self.primary * ratio / self.radius
    }
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
    /// How far it turns either way from straight ahead, radians (`MinYaw`,
    /// `MaxYaw`, degrees: an emplaced machine gun's -90 and 90; a tank's
    /// turret's go all the way round), if the template gives them.
    pub yaw: Option<(f32, f32)>,
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

    /// Its targets' keys (rows) and its damage kinds' keys (columns).
    pub fn keys(&self) -> (&[u32], &[u32]) {
        (&self.rows, &self.columns)
    }

    /// What damage of kind `damage` does to a part with a key of its own
    /// (`part`: `WeakPointTank`), times what it does to the whole vehicle
    /// (`vehicle`: `Tank`): a rocket (`Rocket`) on a tank's weak point 5
    /// (1 against 0.2), a `RocketLarge` 10. `None` where the table has
    /// either key not, or the whole vehicle takes none of it (then there
    /// is no damage of its own to scale).
    ///
    /// That a part's key stands in for the vehicle's where the part is
    /// struck is read off the rows' shape (the weak points' rows give
    /// rockets more, nothing else), not checked against the 360 code.
    pub fn weak_point(&self, vehicle: u32, part: u32, damage: u32) -> Option<f32> {
        let whole = self.scale(vehicle, damage)?;
        let own = self.scale(part, damage)?;
        (whole > 0.0).then(|| own / whole)
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
/// `CameraHelicopter`'s distance behind (name unknown).
const CHASE_DISTANCE: u32 = 0xee30_a43d;
/// Unnamed fields (round 154): `Health`'s value, `ExplosionFudge`'s
/// scale and `TickDamage`'s threshold.
const HEALTH: u32 = 0xd122_919d;
const BLAST_SCALE: u32 = 0x6bcc_541e;
const BURNS_BELOW: u32 = 0x9a66_63a4;

/// Template `id`'s whole `ModifierKey`: its record with no `Name` (the
/// named ones are its parts').
fn whole_key(modifier: &Table<'_>, id: u32) -> Option<u32> {
    modifier
        .of_template(id)
        .iter()
        .find(|r| modifier.word(r, name_hash("Name")).is_none_or(|n| n == 0))
        .and_then(|r| modifier.word(r, name_hash("ModifierKey")))
        .filter(|&k| k != 0)
}

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

    /// The fields' name hashes, in order.
    pub(crate) fn fields(&self) -> Vec<u32> {
        let mut f: Vec<u32> = self.fields.keys().copied().collect();
        f.sort_unstable();
        f
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

/// Every person template's `HumanAnimationSet`: its `Gender` and
/// `CharacterName` (name hashes: Mattias's `Hero` and `mattias`), which
/// pick the character's rows of the on-foot animation table
/// ([`m2_assets::animation_table::character_clip`]), by id.
pub fn human_animation_sets(db: &Layer<'_>) -> HashMap<u32, (u32, u32)> {
    let sets = Table::new(db, "HumanAnimationSet");
    let (gender, name) = (name_hash("Gender"), name_hash("CharacterName"));
    sets.templates()
        .filter_map(|t| {
            let r = sets.of_template(t).first()?;
            Some((t, (sets.word(r, gender)?, sets.word(r, name)?)))
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
    /// How soldiers fire it (`WeaponHint`, [`crate::weapons::FireHint`]).
    pub hint: Option<crate::weapons::FireHint>,
    /// Rounds a shot (`iBulletsPerShot`: the shotgun's 10, else 1).
    pub pellets: u32,
}

impl HandGun {
    /// The same fields of a gun as [`crate::weapons::read`] gives it: for
    /// one shot a pull (`FireType` 1), which [`hand_guns`] leaves out.
    pub fn from_spec(w: &crate::weapons::WeaponSpec) -> Self {
        Self {
            template: w.template,
            model: w.model,
            rate_of_fire: w.rate_of_fire,
            clip: w.clip,
            reserve_clips: w.reserve_clips,
            damage: w.ammo.damage,
            damage_key: w.ammo.key,
            hint: w.hint,
            pellets: w.pellets,
        }
    }
}

/// Every hand-held gun that fires rounds ([`HandGun`]), by template id.
pub fn hand_guns(db: &Layer<'_>) -> HashMap<u32, HandGun> {
    let weapon = Table::new(db, "WeaponProjectileBase");
    let physics = Table::new(db, "ProjectilePhysics");
    let key = Table::new(db, "DamageKey");
    let hint = Table::new(db, "WeaponHint");
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
                    hint: crate::weapons::FireHint::read(&hint, id),
                    pellets: weapon
                        .word(r, name_hash("iBulletsPerShot"))
                        .map_or(1, |c| c & 0xff)
                        .max(1),
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
    let heli = Table::new(db, "_HelicopterPhysics");
    let heli_camera = Table::new(db, "CameraHelicopter");
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
        .chain(heli.records.keys())
        .chain(turret.records.keys())
        .chain(camera.records.keys())
        .copied()
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter()
        .map(|id| {
            let t = VehicleTuning {
                mass: [&tank, &car, &boat, &heli]
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
                rounds: whole_key(&modifier, id).and_then(|key| damage?.rounds(key)),
                key: whole_key(&modifier, id),
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
                        yaw: deg("MinYaw").zip(deg("MaxYaw")),
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
                engine: (|| {
                    let r = *car.of_template(id).first()?;
                    let f = |name| car.float(r, name_hash(name));
                    let gear = |name| f(name).unwrap_or(0.0).max(0.0);
                    let e = EngineTuning {
                        torque: f("EngineTorque")?,
                        rpm: (f("GearMinRPM")?, f("GearOptRPM")?, f("GearMaxRPM")?),
                        upshift: f("GearUpshiftRPM")?,
                        downshift: f("GearDownshiftRPM").unwrap_or(0.0).max(0.0),
                        clutch_delay: f("ClutchDelayTime").unwrap_or(0.0).max(0.0),
                        primary: f("GearRatioPrimary")?,
                        gears: [
                            gear("GearRatio1"),
                            gear("GearRatio2"),
                            gear("GearRatio3"),
                            gear("GearRatio4"),
                            gear("GearRatio5"),
                        ],
                        reverse: f("GearRatioReverse")?,
                        radius: f("RearWheelRadius")?,
                        torque_at: (f("TorqueFactorAtMinRPM")?, f("TorqueFactorAtMaxRPM")?),
                        reverse_speed: f("ReverseMaxSpeed")? / 3.6,
                    };
                    (e.torque > 0.0
                        && e.primary > 0.0
                        && e.radius > 0.05
                        && e.gears[0] > 0.0
                        && e.rpm.0 < e.rpm.1
                        && e.rpm.1 < e.rpm.2)
                        .then_some(e)
                })(),
                handling: (|| {
                    let r = *car.of_template(id).first()?;
                    let f = |name| car.float(r, name_hash(name)).filter(|v| *v > 0.0);
                    let low = (|| Some((f("LowSpeedLimit")? / 3.6, f("LowSpeedSteerSpeed")?)))();
                    Some(Handling {
                        steer_lock: f("SteerMaxAngle").filter(|a| *a < 1.5)?,
                        brake: (f("FrontBrake")?, f("RearBrake")?),
                        grip: (f("FrontWheelFrictionSide")?, f("RearWheelFrictionSide")?),
                        steer_speed: f("SteerSpeed")?,
                        low_speed: low.map_or(0.0, |l| l.0),
                        low_steer_speed: low.map_or(0.0, |l| l.1),
                    })
                })(),
                springs: (|| {
                    let r = *car.of_template(id).first()?;
                    let d = Springs::DEFAULT;
                    let f = |name, or: f32| car.float(r, name_hash(name)).unwrap_or(or);
                    let pair = |front, rear, or: (f32, f32)| (f(front, or.0), f(rear, or.1));
                    let length = |front, rear, or| {
                        let (a, b) = pair(front, rear, or);
                        (a.max(0.0), b.max(0.0))
                    };
                    Some(Springs {
                        strength: pair(
                            "FrontWheelSuspStrength",
                            "RearWheelSuspStrength",
                            d.strength,
                        ),
                        damping: pair("FrontWheelSuspDamp", "RearWheelSuspDamp", d.damping),
                        stretch: length(
                            "FrontWheelSuspExpLength",
                            "RearWheelSuspExpLength",
                            d.stretch,
                        ),
                        squeeze: length(
                            "FrontWheelSuspCmpLength",
                            "RearWheelSuspCmpLength",
                            d.squeeze,
                        ),
                        extra: f("ExtraInvisibleSpringLength", d.extra).max(0.0),
                        gravity_factor: f("GravityFactor", d.gravity_factor),
                    })
                })(),
                tracks: (|| {
                    let r = *tank.of_template(id).first()?;
                    let f = |name| tank.float(r, name_hash(name));
                    let curve = [
                        f("EngineTorqueRatio0")?,
                        f("EngineTorqueRatio1")?,
                        f("EngineTorqueRatio2")?,
                        f("EngineTorqueRatio3")?,
                    ];
                    curve
                        .iter()
                        .all(|c| (0.0..=1.0).contains(c))
                        .then_some(curve)
                })(),
                flight: (|| {
                    let f = |name| heli.get(id, name);
                    Some(FlightTuning {
                        climb: f("MaxElevationSpeed")?,
                        forward: f("MaxFwdSpeed")?,
                        sideways: f("MaxRightSpeed")?,
                        yaw: f("maxYawSpeed")?,
                        low_speed: f("LowSpeedLimit")?,
                        low_speed_yaw: f("LowSpeedYawSpeed")?,
                        pitch: f("MaxPitch")?,
                        pitch_accel: f("PitchAccel")?,
                        roll_side: f("MaxRollSide")?,
                        roll_bank: f("MaxRollBank")?,
                        roll_accel: f("RollAccel")?,
                        bank_speed: f("BankSpeedLimit")?,
                    })
                })(),
                boat: (|| {
                    let r = *boat.of_template(id).first()?;
                    let f = |name| boat.float(r, name_hash(name)).filter(|v| *v >= 0.0);
                    let b = BoatTuning {
                        reverse_speed: f("MaxSpeedReverse")? / 3.6,
                        torque: (f("EngineTorque")?, f("EngineReverseTorque")?),
                        drag: [f("WaterDragFwd")?, f("WaterDragSide")?, f("WaterDragUp")?],
                        brake: f("BrakeDrag")?,
                        curve: [
                            f("EngineTorqueRatio0")?,
                            f("EngineTorqueRatio1")?,
                            f("EngineTorqueRatio2")?,
                            f("EngineTorqueRatio3")?,
                        ],
                    };
                    (b.torque.0 > 0.0 && b.drag[0] > 0.0).then_some(b)
                })(),
                bike: (|| {
                    let r = *car.of_template(id).first()?;
                    let f = |name| car.float(r, name_hash(name)).filter(|v| *v >= 0.0);
                    let b = BikeTuning {
                        roll: f("BikeRollStabilityStrength")?,
                        damp: [
                            f("BikeExtraDampRoll")?,
                            f("BikeExtraDampPitch")?,
                            f("BikeExtraDampYaw")?,
                        ],
                        air: (f("BikeAirControlPitch")?, f("BikeAirControlYaw")?),
                    };
                    (b.roll > 0.0).then_some(b)
                })(),
                chase: (|| {
                    let records = heli_camera.of_template(id);
                    let seat = name_hash("SeatName");
                    let r = records
                        .iter()
                        .find(|r| heli_camera.word(r, seat) == Some(name_hash("seat_driver")))
                        .or(records.first())?;
                    Some((
                        heli_camera.float(r, CHASE_DISTANCE)?,
                        heli_camera.float(r, name_hash("DefaultPitch"))?,
                    ))
                })(),
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
            ("MaxElevationSpeed", 0x68cf_364b),
            ("MaxFwdSpeed", 0x4fc8_d911),
            ("MaxRightSpeed", 0xd86d_f802),
            ("LowSpeedLimit", 0xd17f_d0ef),
            ("LowSpeedYawSpeed", 0xe121_ab18),
            ("PitchAccel", 0x05e9_f647),
            ("MaxRollSide", 0x520d_bfb9),
            ("MaxRollBank", 0xde44_cb52),
            ("RollAccel", 0x8700_d608),
            ("BankSpeedLimit", 0xb027_46f1),
            // `_CarPhysicsV2`'s engine and gearbox (round 18).
            ("EngineTorque", 0x354a_82b9),
            ("GearMinRPM", 0x0c4f_7cff),
            ("GearOptRPM", 0x5c4c_668e),
            ("GearMaxRPM", 0x0d64_8cb5),
            ("GearUpshiftRPM", 0x973e_48b8),
            ("GearDownshiftRPM", 0x0476_13b3),
            ("GearRatioPrimary", 0x6936_5ef9),
            ("GearRatioReverse", 0x105f_f72d),
            ("GearRatio1", 0x9324_acd2),
            ("GearRatio5", 0x0b1a_dc5e),
            ("RearWheelRadius", 0x77c6_830e),
            ("ReverseMaxSpeed", 0x1958_7e8a),
            // Its steering, brakes and grip (gap round 4).
            ("SteerMaxAngle", 0x8b96_7513),
            ("FrontBrake", 0x3cbe_aa13),
            ("RearBrake", 0x0175_b464),
            ("FrontWheelFrictionSide", 0x0d87_1558),
            ("RearWheelFrictionSide", 0x45a5_a093),
            // How quickly it steers (gap round 14; `SteeringInertia`
            // named, unused).
            ("SteerSpeed", 0x0db1_c8d9),
            ("SteeringInertia", 0xefc4_dc7a),
            ("LowSpeedSteerSpeed", 0xb36b_7486),
            // Its springs and dampers (gap round 16; `FrontWheelRadius`
            // named, unused: the model's own wheels are used).
            ("FrontWheelSuspStrength", 0x6ae9_c849),
            ("RearWheelSuspStrength", 0xeda3_7cb2),
            ("FrontWheelSuspDamp", 0x7a4b_af98),
            ("RearWheelSuspDamp", 0x9198_0ac7),
            ("FrontWheelRadius", 0xbf59_a7fd),
            // `_TankPhysics`'s pull over its speed.
            ("EngineTorqueRatio0", 0xe8a9_e7ee),
            ("EngineTorqueRatio3", 0x4ab1_3df9),
            // `_BoatPhysics`'s engine and drag (gap round 17); the rest
            // named, unused.
            ("MaxSpeedReverse", 0x1a48_1908),
            ("EngineReverseTorque", 0x77f0_1515),
            ("WaterDragFwd", 0xdf30_e58f),
            ("WaterDragSide", 0x00a6_a021),
            ("WaterDragUp", 0x2129_f9cf),
            ("BrakeDrag", 0x5b27_5d58),
            ("EngineTorqueRatio1", 0x4aac_c0cb),
            ("EngineTorqueRatio2", 0x70af_3b34),
            ("Steering", 0x0d6b_34d0),
            ("LowSpeedSteering", 0xa9d5_82eb),
            ("HullFriction", 0xb50c_fb8a),
            ("RollInertia", 0x80dc_9cc6),
            ("PitchUpSpeed", 0xf72b_c08f),
            ("PitchDownInertia", 0xe125_7555),
            ("HandbrakeInertia", 0x83d7_b1ab),
            ("HandbrakeSteerBoost", 0x8572_4b6b),
            // `_CarPhysicsV2`'s @108 and @170 (gap-list sweep, by hashing;
            // unused): `GravityFactor` 1 to 3 (2 and 1.7 on 46 records
            // each), `CanCrush` 1 on the Monster Trucks, the EXT (monster)
            // and the WZ551 only, 0 on the rest.
            ("GravityFactor", 0x49e4_fde4),
            ("CanCrush", 0x711b_067a),
            // `_CarPhysicsV2`'s bike fields (gap round 18); the
            // wheelie's named, unused.
            ("BikeRollStabilityStrength", 0x215f_7ff3),
            ("BikeExtraDampRoll", 0x0e1e_7f73),
            ("BikeExtraDampPitch", 0x118c_dd84),
            ("BikeExtraDampYaw", 0xc0d6_6bd3),
            ("BikeWheeliePitch", 0x2943_a829),
            ("BikeWheelieSideDamp", 0xdb84_6836),
            ("BikeAirControlPitch", 0x280f_15d5),
            ("BikeAirControlYaw", 0x1f16_345a),
            // `Rotor`: its hub node, and which way it spins (1 or -1);
            // its blade and blur nodes are `vehicle::MAIN_ROTOR`'s.
            ("RotorHubBoneName", 0xc93e_7607),
            ("SpinDir", 0xc9d2_9446),
            // `HumanAnimationSet`'s values for the heroes (heroes round
            // 02): `Gender` `Hero`, `CharacterName` `mattias`, `Chris`.
            ("Hero", 0x5172_8909),
            ("mattias", 0x030e_6c38),
            ("Chris", 0xd64b_b122),
        ] {
            assert_eq!(name_hash(name), key, "{name}");
        }
    }

    /// The M151's `_CarPhysicsV2` engine and gearbox.
    fn m151() -> EngineTuning {
        EngineTuning {
            torque: 130.0,
            rpm: (900.0, 3000.0, 12000.0),
            upshift: 4000.0,
            downshift: 2500.0,
            clutch_delay: 0.0,
            primary: 3.5,
            gears: [4.0, 3.1, 2.4, 2.0, 1.5],
            reverse: 3.7,
            radius: 0.39,
            torque_at: (0.8, 1.0),
            reverse_speed: 55.0 / 3.6,
        }
    }

    // Its top gear at its optimum rpm is its template's `MaxSpeed`, 85
    // km/h, near enough.
    #[test]
    fn the_m151_tops_out_in_fifth_near_its_max_speed() {
        let e = m151();
        assert_eq!(e.gear_count(), 5);
        let kmh = 3000.0 / (e.primary * 1.5) * std::f32::consts::TAU / 60.0 * e.radius * 3.6;
        assert!((kmh - 85.0).abs() < 2.0, "{kmh}");
        assert_eq!(e.gear(85.0 / 3.6), 4);
        assert!((e.rpm_at(1.5, 85.0 / 3.6) - 3034.0).abs() < 5.0);
    }

    // It pulls away in first at its idle torque (0.8 of 130 Nm through
    // 3.5 x 4 on 0.39 m wheels: 3.7 kN) and changes up as it goes.
    #[test]
    fn the_engine_pulls_through_its_gears() {
        let e = m151();
        assert_eq!(e.gear(0.0), 0);
        assert!((e.push(0.0, false) - 3733.0).abs() < 2.0);
        let gears: Vec<usize> = (0..25).map(|v| e.gear(v as f32)).collect();
        assert!(gears.windows(2).all(|w| w[1] >= w[0]), "{gears:?}");
        assert_eq!(gears[24], 4);
        // Harder in first than in fifth; nothing past its most rpm.
        assert!(e.push(5.0, false) > e.push(23.0, false));
        assert_eq!(e.torque_factor(12_001.0), 0.0);
        assert_eq!(e.torque_factor(3000.0), 1.0);
        assert!((e.torque_factor(500.0) - 0.8).abs() < 1e-6);
        // Backing up in reverse's ratio.
        assert!((e.push(0.0, true) - 130.0 * 0.8 * 3.5 * 3.7 / 0.39).abs() < 2.0);
    }

    // Gap round 24: it changes up at its 4,000 rpm and back down only
    // below its 2,500, one gear at a time, so it never changes straight
    // back (by speed alone it changed down at 4,000 rpm in the gear
    // below, and the garbage truck hunted between gears 225 times in a
    // run).
    #[test]
    fn the_gearbox_changes_up_and_down_at_its_shift_speeds() {
        let e = m151();
        let at = |gear: usize, rpm: f32| rpm / e.rpm_at(e.gears[gear], 1.0);
        assert_eq!(e.shift(0, at(0, 3990.0)), 0);
        assert_eq!(e.shift(0, at(0, 4010.0)), 1);
        // Just changed up: 4,010 x 3.1 / 4 = 3,108 rpm in second.
        assert_eq!(e.shift(1, at(0, 4010.0)), 1);
        assert_eq!(e.shift(1, at(1, 2510.0)), 1);
        assert_eq!(e.shift(1, at(1, 2490.0)), 0);
        // Its top gear and first are the ends.
        assert_eq!(e.shift(4, 60.0), 4);
        assert_eq!(e.shift(0, 0.0), 0);
        // Rolling at 1,500 rpm in fifth it changes down one at a time,
        // to second (3,100 rpm there).
        let mut gear = 4;
        let mut seen = vec![gear];
        for _ in 0..5 {
            gear = e.shift(gear, at(4, 1500.0));
            seen.push(gear);
        }
        assert_eq!(seen, [4, 3, 2, 1, 1, 1]);
        // It pulls harder in a lower gear.
        assert!(e.push_in(0, 5.0, false) > e.push_in(1, 5.0, false));
    }

    // A gearbox with fewer than five gears stops at its last.
    #[test]
    fn a_missing_gear_is_no_gear() {
        let e = EngineTuning {
            gears: [3.0, 2.0, 1.0, 0.0, 0.0],
            ..m151()
        };
        assert_eq!(e.gear_count(), 3);
        assert_eq!(e.gear(60.0), 2);
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

    // The game's rows for a tank and its weak point (round 15's dump):
    // rockets 0.2 and 0.2 against 1 and 2, rounds nothing either way.
    #[test]
    fn a_weak_point_takes_its_own_row_of_the_vehicles() {
        let mut data = vec![0u8; 0x48];
        let mut put = |v: u32| data.extend_from_slice(&v.to_le_bytes());
        put(2);
        put(3);
        let (tank, weak) = (name_hash("Tank"), name_hash("WeakPointTank"));
        let (bullet, rocket, large) = (
            name_hash("Bullet"),
            name_hash("Rocket"),
            name_hash("RocketLarge"),
        );
        for k in [tank, weak, bullet, rocket, large] {
            put(k);
        }
        for v in [0.0f32, 0.2, 0.2, 0.0, 1.0, 2.0] {
            put(v.to_bits());
        }
        let t = DamageTable::parse(&data).expect("parses");
        let near = |a: Option<f32>, b: f32| a.is_some_and(|a| (a - b).abs() < 1e-5);
        assert!(near(t.weak_point(tank, weak, rocket), 5.0));
        assert!(near(t.weak_point(tank, weak, large), 10.0));
        // A tank takes no round at all: nothing to scale.
        assert_eq!(t.weak_point(tank, weak, bullet), None);
        assert_eq!(t.weak_point(tank, name_hash("Car"), rocket), None);
    }
}
