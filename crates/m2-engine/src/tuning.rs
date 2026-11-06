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
    /// `Stimulus` `Visibility`: from how far it can be seen, metres
    /// (0x822E89B0; `M151 .50Cal (VZ) (Full)` 200).
    pub visibility: Option<f32>,
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
    /// How a wheeled vehicle's tyres slow it along (`_CarPhysicsV2`).
    pub tyres: Option<Tyres>,
    /// A tracked vehicle's drive, tracks and steering (`_TankPhysics`).
    pub tank: Option<TankTuning>,
    /// A helicopter's handling (`_HelicopterPhysics`).
    pub flight: Option<FlightTuning>,
    /// A helicopter's handling with an AI at its stick
    /// (`_HelicopterPhysicsAi`, schema 0x823EC220: the same fields but
    /// `Mass`; 0x8218C1F0 picks it for one not flown by the player).
    pub flight_ai: Option<FlightTuning>,
    /// How its AI pilot flies it (`AiHelicopter`).
    pub pilot: Option<PilotTuning>,
    /// A boat's engine and the water's drag on it (`_BoatPhysics`).
    pub boat: Option<BoatTuning>,
    /// A motorbike's balance and damping (`_CarPhysicsV2`'s `Bike*`
    /// fields).
    pub bike: Option<BikeTuning>,
    /// A wheeled vehicle's inertia and centre of mass (`_CarPhysicsV2`),
    /// as the 360 code sets every car's body (0x8219BA08, 0x8219BABC).
    pub body: Option<CarBody>,
    /// How far behind a helicopter its pilot's camera sits, metres, and
    /// how far it looks down, radians (`CameraHelicopter`'s field
    /// `ee30a43d`, name unknown: 20 for the light helicopters, 39 for the
    /// Mi-26; and `DefaultPitch`).
    pub chase: Option<(f32, f32)>,
}

/// How a tracked vehicle drives, from its `_TankPhysics` (names from the
/// 360 code's record, 0x823EBBE8, in its order; the AMX-30's values in
/// brackets), as the 360 code runs it (step 0x821A49E8; see
/// `physics::drive_tracks`). Fields the record leaves out take the 360
/// record's defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TankTuning {
    /// Top speed, m/s (`MaxSpeed` 45 km/h): the drive stops pushing at it.
    pub max_speed: f32,
    /// Each track point's grip along it braking, per second (`Brake` 4).
    pub brake: f32,
    /// What its steering turns it with (`SteerSpeed` 12).
    pub steer_speed: f32,
    /// How much of its steering's gap it keeps each sixtieth of a second,
    /// steering and letting go (`SteerInertia` 0.95,
    /// `SteerInertiaNoInput` 0.65).
    pub steer_inertia: (f32, f32),
    /// Each track point's grip along it rolling, and across it, per second
    /// (`FwdFriction` 0.1, `SideFriction` 0.9).
    pub friction: (f32, f32),
    /// Where its drive pushes it, metres up the ground's normal from its
    /// centre of mass (`EngineOffsetY` -0.3).
    pub engine_height: f32,
    /// Its weight off the ground, of its plain weight (`GravityFactor` 2).
    pub gravity_factor: f32,
    /// Of its steering, what its top speed takes off (`HiSpeedSteerLoss`
    /// 0.1).
    pub steer_loss: f32,
    /// Its tracks' grip at its top speed, of their grip (`HiSpeedGrip` 1).
    pub high_speed_grip: f32,
    /// Damping against its spin about its own sideways, up and ahead
    /// axes, per second (`AngDampX`, `Y`, `Z`: 9, 0, 9).
    pub ang_damp: [f32; 3],
    /// Its centre of mass from its body's, metres (`CenterOfMassOffset`).
    pub centre_of_mass: [f32; 3],
    /// Its drive's push at its best, of 200 newtons (`EngineTorque` 2000).
    pub torque: f32,
    /// How much of its throttle's gap it keeps each sixtieth of a second
    /// (`EngineInertia` 0.85).
    pub engine_inertia: f32,
    /// Its push over its speed: (of its top speed, of `torque`) at 0,
    /// `EngineTorquePos1`, `EngineTorquePos2` and 1, in a line between
    /// (`EngineTorqueRatio0`..`3` 1, 0.45, 0.3, 0.1 at 0, 0.4, 0.6, 1).
    pub curve: [(f32, f32); 4],
    /// Its springs: strength and damp per kilogram, how far they stretch
    /// and squeeze, metres (`SpringStrength` 20, `SpringDamp` 2.5,
    /// `SpringExpLength` 0.35, `SpringCmpLength` 0.7).
    pub springs: (f32, f32, f32, f32),
}

impl TankTuning {
    /// How long each track point's spring is, metres: its stretch and
    /// squeeze (its ray's length, 0x821A4A60).
    pub fn spring_length(&self) -> f32 {
        self.springs.2 + self.springs.3
    }
}

/// How a helicopter's AI pilot flies it: its `AiHelicopter` record
/// (schema 0x823EF200; the pilot's state takes it at 0x822BCC08). All
/// metres or seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PilotTuning {
    /// `MinAltitude`: the least it holds over the ground (10).
    pub min_altitude: f32,
    /// `AttackRadius`, `AttackHeight` (80, 20).
    pub attack_radius: f32,
    pub attack_height: f32,
    /// `SearchRadius`, `SearchHeight`, `SearchTime` (30, 20, 5).
    pub search_radius: f32,
    pub search_height: f32,
    pub search_time: f32,
    /// `LandingSlopeTolerance` (1), `OuterHeightTolerance` (-1: taken
    /// from the template's other records, at least 1.5, 0x822BCC08).
    pub landing_slope: f32,
    pub outer_height: f32,
    /// `FlyoverAttackRate` (20).
    pub flyover_rate: f32,
}

impl PilotTuning {
    /// The schema's defaults (0x823EF200).
    pub const DEFAULT: Self = Self {
        min_altitude: 10.0,
        attack_radius: 80.0,
        attack_height: 20.0,
        search_radius: 30.0,
        search_height: 20.0,
        search_time: 5.0,
        landing_slope: 1.0,
        outer_height: -1.0,
        flyover_rate: 20.0,
    };
}

/// A helicopter's handling from its record in `table`
/// (`_HelicopterPhysics` or `_HelicopterPhysicsAi`), if it has one.
fn flight_of(table: &Table<'_>, id: u32) -> Option<FlightTuning> {
    let r = *table.of_template(id).first()?;
    let d = FlightTuning::DEFAULT;
    let f = |name, default| table.float(r, name_hash(name)).unwrap_or(default);
    Some(FlightTuning {
        climb: f("MaxElevationSpeed", d.climb),
        forward: f("MaxFwdSpeed", d.forward),
        sideways: f("MaxRightSpeed", d.sideways),
        forward_accel: f("FwdSpeedAcc", d.forward_accel),
        sideways_accel: f("RightSpeedAcc", d.sideways_accel),
        yaw: f("MaxYawSpeed", d.yaw),
        yaw_accel: f("YawAcc", d.yaw_accel),
        yaw_damp: f("YawDamp", d.yaw_damp),
        pitch: f("MaxPitch", d.pitch),
        pitch_accel: f("PitchAccel", d.pitch_accel),
        roll_forward: f("MaxRollFwd", d.roll_forward),
        roll_side: f("MaxRollSide", d.roll_side),
        roll_accel: f("RollAccel", d.roll_accel),
        low_speed_yaw: f("LowSpeedYawSpeed", d.low_speed_yaw),
        low_speed: f("LowSpeedLimit", d.low_speed),
        high_speed_yaw_loss: f("HiSpeedYawLoss", d.high_speed_yaw_loss),
        bank_speed: f("BankSpeedLimit", d.bank_speed),
        roll_bank: f("MaxRollBank", d.roll_bank),
        bank_yaw: f("BankYawAmplification", d.bank_yaw),
        no_input_inertia: f("FwdSpeedNoInputInertia", d.no_input_inertia),
    })
}

/// How a helicopter flies: its `_HelicopterPhysics` record, field for
/// field as the 360 code's schema (0x823EBFA0) lays it out, with the
/// schema's defaults ([`FlightTuning::DEFAULT`]) for a field the record
/// leaves out (the AH-1Z's values in brackets). Speeds are m/s and turn
/// rates radians a second: the 360 code sets them against the body's own
/// velocities. The `*Acc`, `*Accel` and `YawDamp` fields are gains a
/// second. `physics.rs` (`fly_helicopters`) flies it with them.
///
/// The record's `Mass` is the body's; its `KickBack` isn't read by the
/// flight code.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlightTuning {
    /// `MaxElevationSpeed`, `MaxFwdSpeed`, `MaxRightSpeed`: its fastest
    /// climb, ahead and sideways (25, 30, 30).
    pub climb: f32,
    pub forward: f32,
    pub sideways: f32,
    /// `FwdSpeedAcc`, `RightSpeedAcc`: how hard it pulls towards the speed
    /// asked ahead and sideways.
    pub forward_accel: f32,
    pub sideways_accel: f32,
    /// `MaxYawSpeed` (3.5), `YawAcc`, `YawDamp`: its fastest turn, how
    /// hard it turns towards the rate asked, and how hard it stops
    /// turning when asked for almost none.
    pub yaw: f32,
    pub yaw_accel: f32,
    pub yaw_damp: f32,
    /// `MaxPitch` (0.4), `PitchAccel` (0.9): how far its nose dips,
    /// radians, and how quickly it pitches to it.
    pub pitch: f32,
    pub pitch_accel: f32,
    /// `MaxRollFwd`, `MaxRollSide` (0.3), `RollAccel` (1): how far it
    /// rolls turning and sliding sideways, radians, and how quickly.
    pub roll_forward: f32,
    pub roll_side: f32,
    pub roll_accel: f32,
    /// `LowSpeedYawSpeed` (6.5), its turn below `LowSpeedLimit` (25), and
    /// the share of its turn it loses at top speed (`HiSpeedYawLoss`).
    pub low_speed_yaw: f32,
    pub low_speed: f32,
    pub high_speed_yaw_loss: f32,
    /// `BankSpeedLimit` (25), `MaxRollBank` (0.3), `BankYawAmplification`:
    /// how it banks into a sideways slide at speed and turns with it.
    pub bank_speed: f32,
    pub roll_bank: f32,
    pub bank_yaw: f32,
    /// `FwdSpeedNoInputInertia`: how much less it pulls with the stick
    /// let go.
    pub no_input_inertia: f32,
}

impl FlightTuning {
    /// The schema's defaults (0x823EBFA0).
    pub const DEFAULT: Self = Self {
        climb: 10.0,
        forward: 30.0,
        sideways: 20.0,
        forward_accel: 1.0,
        sideways_accel: 3.0,
        yaw: 1.5,
        yaw_accel: 2.0,
        yaw_damp: 15.0,
        pitch: 0.75,
        pitch_accel: 5.0,
        roll_forward: 0.5,
        roll_side: 0.33,
        roll_accel: 3.33,
        low_speed_yaw: 1.5,
        low_speed: 0.0,
        high_speed_yaw_loss: 0.0,
        bank_speed: 0.0,
        roll_bank: 0.0,
        bank_yaw: 0.0,
        no_input_inertia: 0.0,
    };
}

/// A boat's engine and how the water slows it, from its `_BoatPhysics`
/// (names found by hash, gap round 17; the Speed Boat's values in
/// brackets, the range over the 110 records after them). Its top speed
/// is [`VehicleTuning::max_speed`] (`MaxSpeed` 120 km/h).
///
/// Named but not read here: `Steering`, `LowSpeedSteering`,
/// `LowSpeedLimit`, `SteerInertia`, `EngineInertia`,
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
    /// `EngineReverseTorque` 500; 30 to 2,500 and 10 to 3,000): x 200, N
    /// (`physics::float_hull`, from 0x82198F08 and 0x82199B08).
    pub torque: (f32, f32),
    /// How quickly the water slows it along, across and up and down, a
    /// second (`WaterDragFwd` 0.2, `WaterDragSide` 2, `WaterDragUp` 2;
    /// 0.1 to 0.5, 0.5 to 10, 0.5 to 26), read as a linear drag's rate
    /// (inferred: they sit where the alpha's invented 0.3, 2.5 and 3
    /// did; the Air Boat's sideways 0.5 lets it skid, the landing craft's
    /// 10 holds it to its course).
    pub drag: [f32; 3],
    /// How hard its brake slows it, m/s² (`BrakeDrag` 3; 0 to 8): from
    /// the 360 code (0x82199B08), a steady slowing, not a drag.
    pub brake: f32,
    /// Its pull, of its torque, at a standstill, at
    /// [`BoatFloat::curve_at`] and at its top speed
    /// (`EngineTorqueRatio0`..`3`; 0x82198F08).
    pub curve: [f32; 4],
    /// Its hull's friction (`HullFriction`, 0.3 when the record gives
    /// none, as the 360 code's schema reader defaults it at 0x823EA968):
    /// its sim sets it on its body (0x82198FF8, through 0x82629DB0). From
    /// the 360 code.
    pub hull_friction: f32,
    /// What floats it and how it leans and lifts its bow
    /// ([`BoatFloat`]).
    pub float: BoatFloat,
}

/// One of a boat's float springs as its speed sets them ([`BoatFloat`]):
/// the heights, model space, metres, of its top and bottom
/// (`Spring..SpeedStartOffset`, `..EndOffset`), and its strength (a
/// second², of its mass per metre the water stands over its bottom) and
/// damping (a second, of its mass per m/s it rises), `..Strength` and
/// `..Dampening`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloatSpring {
    pub start: f32,
    pub end: f32,
    pub strength: f32,
    pub damping: f32,
}

impl FloatSpring {
    /// Its length, metres: its top above its bottom, 0 the other way up.
    pub fn length(&self) -> f32 {
        (self.start - self.end).max(0.0)
    }
}

/// How a boat floats, from its `_BoatPhysics` (record 0x823EA868, which
/// the 360 code copies into its boat sim at 0x82198FF8). From the 360
/// code:
///
/// - It floats on four springs, one at each corner of its hull's box
///   seen from above (0x82199328, the box from its body's shape), each
///   pushing its body straight up, at the corner at its origin's height,
///   by its mass x step x (strength x how far the water stands over its
///   bottom, not below 0, less damping x how fast that corner rises x how
///   far in the water is, its length's share, 0 to 1), never down
///   (0x82199EA8).
/// - Its springs are [`slow`](Self::slow)'s at a standstill and
///   [`fast`](Self::fast)'s at its top speed ahead (`MaxSpeed`),
///   between them as far as it goes; a spring's bottom is its length
///   under its top, its top where its height puts it on the turned
///   boat, its length measured up the world.
/// - The water each spring stands in is eased each 1/60 s towards the
///   water under it, keeping [`smoothing`](Self::smoothing) of the way
///   (`SpringWaterSmoothing`).
/// - Turning, it leans into the turn: with the helm to port its
///   starboard springs (-x) feel the water stand higher, its port ones
///   (+x) lower, by sin([`roll`](Self::roll)) x half its beam x its
///   lean, eased toward its eased helm (positive to port, the 360 code's
///   own sign: its car code turns it round, 0x82189448's 0x3483DBF1)
///   (keeping [`roll_inertia`](Self::roll_inertia) a 1/60 s) x its speed
///   over [`low_speed`](Self::low_speed) (at most 1) x (1 -
///   [`steer_loss`](Self::steer_loss) x its speed over its top speed, at
///   most 1); its helm eases toward the helm asked (backwards when
///   backing up faster than a quarter of its reverse top speed, 10 km/h
///   at most) keeping [`steer_inertia`](Self::steer_inertia), or
///   [`steer_release`](Self::steer_release) with the helm let go
///   (0x82199548, 0x8219A390, 0x82199B08).
/// - Its helm turns it about the world's up ([`steering`](Self::steering)
///   and the rest; `physics::float_hull`, from 0x8219A390), as much as it
///   sits in the water, and its springs' pushes are scaled by that share,
///   eased toward 1 while any is in the water and 0 when none is
///   (0x8219A248).
/// - Under throttle its bow lifts: its bow springs feel the water stand
///   higher by sin([`pitch`](Self::pitch)) x its length x its lift, which
///   rises toward its throttle by [`pitch_up`](Self::pitch_up) a second
///   and eases down toward it keeping
///   [`pitch_down_inertia`](Self::pitch_down_inertia) (0x82199548).
///
/// Fields the record leaves out take the 360 record's defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoatFloat {
    pub slow: FloatSpring,
    pub fast: FloatSpring,
    pub smoothing: f32,
    /// Radians (`RollFromTurning`).
    pub roll: f32,
    /// `RollInertia`.
    pub roll_inertia: f32,
    /// Radians (`PitchFromAcceleration`).
    pub pitch: f32,
    /// A second (`PitchUpSpeed`).
    pub pitch_up: f32,
    /// `PitchDownInertia`.
    pub pitch_down_inertia: f32,
    /// m/s (`LowSpeedLimit`, km/h).
    pub low_speed: f32,
    /// `HighSpeedSteerLoss`.
    pub steer_loss: f32,
    /// `SteerInertia`.
    pub steer_inertia: f32,
    /// `SteerNoInputInertia`.
    pub steer_release: f32,
    /// Its top speeds ahead and backing up, m/s (`MaxSpeed`,
    /// `MaxSpeedReverse`, km/h).
    pub top: f32,
    pub reverse: f32,
    /// How hard its helm turns it (`physics::float_hull`, 0x8219A390): `Steering`,
    /// `NoSpeedSteering`, `LowSpeedSteering`, `HandbrakeSteerBoost` and
    /// `AirCtrlSteering`.
    pub steering: f32,
    pub no_speed_steering: f32,
    pub low_speed_steering: f32,
    pub handbrake_boost: f32,
    pub air_steering: f32,
    /// Where its engine's pull curve has its middle two points, of its
    /// top speed (`EngineTorquePos1`, `EngineTorquePos2`).
    pub curve_at: (f32, f32),
    /// How its spin about its roll, pitch and yaw axes dies down
    /// (`AngDampRoll`, `AngDampPitch`, `AngDampYaw`; 0x8219A5E0).
    pub ang_damp: [f32; 3],
    /// How much of its roll it loses just after it hits something or
    /// while out of the water (`CollisionStabilization`), and how its
    /// engine pulls the first 1/8 s after a hit
    /// (`EngineTorqueMultiAtCollision`); `physics::float_hull`, 0x8219A7xx.
    pub stabilization: f32,
    pub hit_torque: f32,
    /// How its handbrake eases off once let go (`HandBrakeInertia`).
    pub handbrake_inertia: f32,
    /// On its handbrake, how much of the water's drag across it is
    /// lifted (x its handbrake x its share of its top speed;
    /// `HandBrakeAntiSideDrag`), and how much of the drag across it left
    /// pushes it on ahead instead (`HandBrakeMomentumKeep`);
    /// `physics::Physics::float_boats`, 0x8219A9F0 and 0x8219AB4C.
    pub anti_side_drag: f32,
    pub momentum_keep: f32,
    /// How its engine's throttle eases to the throttle asked
    /// (`EngineInertia`, kept each 1/60 s; 0x82198F08).
    pub engine_inertia: f32,
}

impl BoatFloat {
    /// The 360 record's defaults (0x823EA868).
    pub const DEFAULT: Self = Self {
        slow: FloatSpring {
            start: 1.0,
            end: 0.0,
            strength: 5.0,
            damping: 1.0,
        },
        fast: FloatSpring {
            start: 1.0,
            end: 0.0,
            strength: 5.0,
            damping: 1.0,
        },
        smoothing: 0.9,
        roll: 0.0,
        roll_inertia: 0.95,
        pitch: 0.0,
        pitch_up: 0.25,
        pitch_down_inertia: 0.95,
        low_speed: 10.0 / 3.6,
        steer_loss: 0.0,
        steer_inertia: 0.0,
        steer_release: 0.0,
        top: 40.0 / 3.6,
        reverse: 10.0 / 3.6,
        steering: 2.0,
        no_speed_steering: 0.0,
        low_speed_steering: 0.0,
        handbrake_boost: 0.0,
        air_steering: 0.0,
        curve_at: (0.25, 0.5),
        handbrake_inertia: 0.0,
        anti_side_drag: 0.0,
        momentum_keep: 0.0,
        stabilization: 1.0,
        hit_torque: 1.0,
        ang_damp: [0.0, 0.0, 0.5],
        engine_inertia: 0.975,
    };

    /// Its springs at `going` of its top speed (0 to 1).
    pub fn spring(&self, going: f32) -> FloatSpring {
        let (a, b, s) = (self.slow, self.fast, going.clamp(0.0, 1.0));
        let mix = |x: f32, y: f32| x * (1.0 - s) + y * s;
        FloatSpring {
            start: mix(a.start, b.start),
            end: mix(a.end, b.end),
            strength: mix(a.strength, b.strength),
            damping: mix(a.damping, b.damping),
        }
    }

    /// Its springs' length at `going` of its top speed, metres: each
    /// end's length mixed, as the 360 code mixes them (0x8219A00C).
    pub fn length(&self, going: f32) -> f32 {
        let s = going.clamp(0.0, 1.0);
        self.slow.length() * (1.0 - s) + self.fast.length() * s
    }

    /// How deep its origin rests under still water, level and stopped,
    /// metres, under `gravity` m/s²: its four springs hold its weight
    /// with the water this far over their bottoms.
    pub fn rest_depth(&self, gravity: f32) -> f32 {
        let s = self.slow;
        s.start.min(s.end) + gravity / (4.0 * s.strength.max(1e-3))
    }
}

/// A wheeled vehicle's body as its `_CarPhysicsV2` sets it, from the 360
/// code's car set-up (0x8219B774 on): the body's mass is the record's
/// `Mass` (0x82629D80), its inertia a diagonal of [`inertia`](Self::inertia)
/// x that mass (0x8219BA08 -> 0x82629DC0), and its centre of mass the
/// mean of its wheels' positions plus
/// [`centre_of_mass`](Self::centre_of_mass) (0x8219BABC -> 0x8262A680).
/// Fields the record leaves out take the record's defaults (inertia 2, 1,
/// 1.5; offset 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarBody {
    /// Its inertia about its own sideways, up and ahead axes, m² per
    /// kilogram (`InertiaTensorX`, `Y`, `Z`).
    pub inertia: [f32; 3],
    /// Where its centre of mass sits from the middle of its wheels, model
    /// space, metres (`CenterOfMassOffset`).
    pub centre_of_mass: [f32; 3],
}

/// How a motorbike holds itself up, from its `_CarPhysicsV2`'s `Bike*`
/// fields (names from the 360 code's record, 0x823EB210, in its order
/// @130..@16C; the 360 code copies them 0xE8 on into the running car,
/// 0x8219B774, and its balance reads them there, 0x8219D800). Only bikes
/// give them: the 360 code balances a car only if one of its roll
/// strength and damp, pitch damp and strengths, wheelie steering or air
/// controls is not 0 (0x8219D800's test), and every other record's are.
///
/// Each value is a torque per kilogram: the 360 code turns it into an
/// angular impulse of value x step x mass (0x8262C120 is the body's mass)
/// against the body's inertia, [`inertia`](Self::inertia) x mass
/// (0x8219BA08). Angles are radians. Fields the record leaves out take
/// the 360 record's defaults (all 0; inertia 2, 1, 1.5).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BikeTuning {
    /// The turning rate, rad/s, at which it leans the most
    /// (`BikeRollAngVelForMaxRoll`).
    pub roll_rate: f32,
    /// That most lean, radians, as the 360 code blends it by its speed s
    /// of its top speed: `(1 - s) x hi + s x lo` (`BikeRollAngleAtLoSpeed`,
    /// `BikeRollAngleAtHiSpeed`; the code weights them so, 0x8219DC00,
    /// against what the names suggest).
    pub roll_angle: (f32, f32),
    /// The lean its steering asks on its own at full lock, radians, faded
    /// in from a standstill up to `low_speed` (`BikeRollAngleFromInput`).
    pub roll_input: f32,
    /// What turns it towards that lean, per radian off it
    /// (`BikeRollStabilityStrength`).
    pub roll: f32,
    /// How far its nose lifts in a wheelie, radians (`BikeWheeliePitch`).
    pub wheelie: f32,
    /// What turns its nose towards the pitch it wants on the ground, per
    /// radian: when that asks more than it is off the ground's slope, and
    /// when less (`BikeWheeliePitchStrengthUp`, `...StrengthDown`).
    pub pitch_strength: (f32, f32),
    /// Of those, what is left at its top speed
    /// (`BikeWheeliePitchHiSpeedRatio`).
    pub pitch_high_speed: f32,
    /// How well it steers in a wheelie (`BikeWheelieSteerAbility`).
    pub wheelie_steer: f32,
    /// What holds it from sliding sideways in a wheelie, per m/s
    /// (`BikeWheelieSideDamp`).
    pub side_damp: f32,
    /// Damping against its roll, pitch and yaw, per rad/s
    /// (`BikeExtraDampRoll`, `BikeExtraDampPitch`, `BikeExtraDampYaw`).
    pub damp: [f32; 3],
    /// In the air, what its stick turns its nose and its heading with
    /// (`BikeAirControlPitch`, `BikeAirControlYaw`).
    pub air: (f32, f32),
    /// Up to this speed, m/s, its steering's lean fades in
    /// (`LowSpeedLimit`, km/h).
    pub low_speed: f32,
    /// Its inertia about its own sideways, up and ahead axes, m² per
    /// kilogram (`InertiaTensorX`, `Y`, `Z`).
    pub inertia: [f32; 3],
    /// Where its centre of mass sits from the middle of its wheels, model
    /// space, metres (`CenterOfMassOffset`; the 360 code adds it to the
    /// mean of its wheels' positions, 0x8219BABC).
    pub centre_of_mass: [f32; 3],
}

impl BikeTuning {
    /// Whether the 360 code balances it at all (0x8219D800's test).
    pub fn balances(&self) -> bool {
        [
            self.air.0,
            self.air.1,
            self.damp[0],
            self.damp[1],
            self.damp[2],
            self.roll,
            self.pitch_strength.0,
            self.pitch_strength.1,
            self.wheelie_steer,
        ]
        .iter()
        .any(|&v| v != 0.0)
    }
}

/// A wheeled vehicle's engine and gearbox, from its `_CarPhysicsV2` (the
/// M151's values in brackets), as the Xbox 360 code runs them (from the
/// 360 code, driving round 6: the gearbox and engine at 0x821A0528, the
/// push at 0x8219C518; [`EngineTuning::step`]). A field the record
/// leaves out takes the code's default (0x823EB210,
/// [`EngineTuning::DEFAULT`]).
///
/// Not read: `GearTorqueMultiReverse` and `GearTorqueMulti1`..`5` (the
/// six fields after `GearRatio5`; 1 in every record but the
/// `...Reverse` of 31: 0.5 to 1.5), which the step doesn't use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EngineTuning {
    /// Most torque, newton-metres (`EngineTorque` 130).
    pub torque: f32,
    /// Engine speeds, rpm: idle, the most torque, the most it turns
    /// (`GearMinRPM` 900, `GearOptRPM` 3,000, `GearMaxRPM` 12,000).
    pub rpm: (f32, f32, f32),
    /// It changes up a gear above this, rpm (`GearUpshiftRPM` 4,000),
    /// and down one below this (`GearDownshiftRPM` 2,500).
    pub upshift: f32,
    pub downshift: f32,
    /// How long after a change of gear it may change again, seconds
    /// (`ClutchDelayTime` 0; up to 1.75): the engine drives on meanwhile.
    pub clutch_delay: f32,
    /// The final drive's ratio (`GearRatioPrimary` 3.5), each forward
    /// gear's (`GearRatio1`..`5`: 4, 3.1, 2.4, 2, 1.5; a 0 is no gear)
    /// and reverse's (`GearRatioReverse` 3.7).
    pub primary: f32,
    pub gears: [f32; 5],
    pub reverse: f32,
    /// The rear wheels' radius, metres (`RearWheelRadius` 0.39).
    pub radius: f32,
    /// Of its most torque, what it gives at idle and at its most rpm
    /// (`TorqueFactorAtMinRPM` 0.8 and `TorqueFactorAtMaxRPM` 1.0).
    pub torque_at: (f32, f32),
    /// Top speed backing up, m/s (`ReverseMaxSpeed`, km/h: 55). The
    /// 360's push fades by `MaxSpeed` alone; where this is used isn't
    /// traced.
    pub reverse_speed: f32,
    /// How far over idle the engine runs as the clutch slips at a crawl,
    /// rpm, at full throttle (`GearSlipRPM`; 0, 1,000 or 2,000).
    pub slip: f32,
    /// The rear wheels' share of the drive (`RearToFrontTorqueDistrib`,
    /// default 1): each rear wheel weighs twice that over their number
    /// in the engine's speed, the front the rest (0x821A0390).
    pub split: f32,
}

/// How a wheeled vehicle steers, from its `_CarPhysicsV2` (the M151's
/// values in brackets, the range over the 454 records after them), as
/// the Xbox 360 code turns its front wheels (from the 360 code, driving
/// round 4: each front wheel's angle at 0x821A020C, in the wheel step;
/// the record's defaults at 0x823EB210).
///
/// Each step the wheels' angle heads for the stick's share of the lock,
/// the lock shrinking with speed ([`Handling::range`]). With
/// `SteeringInertia` below 0 (the default, -1) the angle moves there at
/// most its swing speed, which also shrinks with speed; let go of the
/// stick, or pushed the other way, it first eases back towards straight
/// by `SteerNoInputInertia` or `SteerDiffSignInertia`. With
/// `SteeringInertia` 0 or more, it eases to where the stick asks by that
/// (or those two). Easing keeps that share of the way left each 1/60 s
/// ([`ease`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Handling {
    /// How far its front wheels turn, radians (`SteerMaxAngle` 0.7; 0.5
    /// to 1.1, 29 to 63 degrees; the wheels take it as it is).
    pub steer_lock: f32,
    /// How quickly its front wheels swing towards where the driver
    /// steers, radians a second (`SteerSpeed` 0.6; 0.1 to 10).
    pub steer_speed: f32,
    /// At a standstill they swing at `low_steer_speed` instead, easing to
    /// `steer_speed` at this speed, m/s (`LowSpeedLimit` 35 km/h and
    /// `LowSpeedSteerSpeed` 0.9; 0 to 80 and 0 to 6). 0: no low speed.
    pub low_speed: f32,
    pub low_steer_speed: f32,
    /// At its top speed its lock and swing are this much less
    /// (`HighSpeedSteerLoss`, default 0.1), times `loss_braking`
    /// braking (`HighSpeedSteerLossBrakeMulti`, 0.5) or `loss_coasting`
    /// off the throttle (`HighSpeedSteerLossNoThrottleMulti`, 0.8).
    pub high_speed_loss: f32,
    pub loss_braking: f32,
    pub loss_coasting: f32,
    /// How its wheels ease to where the stick asks (`SteeringInertia`,
    /// default -1: below 0, they swing at the swing speed instead), back
    /// to straight with the stick let go (`SteerNoInputInertia`, 0) and
    /// back to straight when steered the other way
    /// (`SteerDiffSignInertia`, 0.9).
    pub inertia: f32,
    pub no_input_inertia: f32,
    pub diff_sign_inertia: f32,
}

impl Handling {
    /// The 360 code's defaults, for a vehicle with no `_CarPhysicsV2`.
    pub const DEFAULT: Handling = Handling {
        steer_lock: 1.0,
        steer_speed: 10.0,
        low_speed: 30.0 / 3.6,
        low_steer_speed: 5.0,
        high_speed_loss: 0.1,
        loss_braking: 0.5,
        loss_coasting: 0.8,
        inertia: -1.0,
        no_input_inertia: 0.0,
        diff_sign_inertia: 0.9,
    };

    /// Its lock's (and swing's) share left at `fast` (its speed's share
    /// of its `MaxSpeed`), with the throttle open or the brake on.
    pub fn range(&self, fast: f32, throttle: f32, brake: f32) -> f32 {
        let multi = if throttle >= 1e-4 {
            1.0
        } else if brake >= 1e-4 {
            self.loss_braking
        } else {
            self.loss_coasting
        };
        1.0 - self.high_speed_loss * multi * fast.clamp(0.0, 1.0)
    }

    /// How quickly its front wheels swing at `speed` m/s, radians a
    /// second (before [`range`](Self::range)).
    pub fn swing(&self, speed: f32) -> f32 {
        let low = if self.low_speed > 0.0 {
            1.0 - (speed.abs() / self.low_speed).min(1.0)
        } else {
            0.0
        };
        self.steer_speed + (self.low_steer_speed - self.steer_speed) * low
    }

    /// Its front wheels' angle after `dt` s from `angle`, radians, with
    /// the stick at `steer` (-1 to 1), at `speed` m/s, `fast` its share
    /// of its `MaxSpeed`, the throttle and brake as [`range`] takes them.
    ///
    /// [`range`]: Self::range
    #[allow(clippy::too_many_arguments)]
    pub fn steer(
        &self,
        angle: f32,
        steer: f32,
        speed: f32,
        fast: f32,
        throttle: f32,
        brake: f32,
        dt: f32,
    ) -> f32 {
        let range = self.range(fast, throttle, brake);
        let want = self.steer_lock * range * steer;
        let back = if steer == 0.0 {
            Some(self.no_input_inertia)
        } else if angle * steer < 0.0 {
            Some(self.diff_sign_inertia)
        } else {
            None
        };
        if self.inertia >= 0.0 {
            return ease(angle, want, back.unwrap_or(self.inertia), dt);
        }
        let angle = back.map_or(angle, |keep| ease(angle, 0.0, keep, dt));
        let most = self.swing(speed) * range * dt;
        angle + (want - angle).clamp(-most, most)
    }
}

/// Eases `from` towards `to` over `dt` s as the 360 code does (from the
/// 360 code, 0x824BB050): each 1/60 s it keeps `keep` of the way left,
/// the rest of a 1/60 s a share of a step; done when the way left is
/// under `keep` x 1e-5. Over 16.7 s, there at once unless `keep` is over
/// 0.9999.
pub fn ease(from: f32, to: f32, keep: f32, dt: f32) -> f32 {
    const TICK: f32 = 1.0 / 60.0;
    if dt > 1000.0 * TICK {
        return if keep <= 0.9999 { to } else { from };
    }
    let (mut at, mut left) = (from, dt);
    while left > TICK {
        let way = to - at;
        if way * way < (keep * 1e-5) * (keep * 1e-5) {
            return to;
        }
        left -= TICK;
        at += way * (1.0 - keep);
    }
    let share = left * 60.0;
    let next = at + (to - at) * (1.0 - keep);
    next * share + (1.0 - share) * at
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

/// How a wheeled vehicle's tyres hold it, along the way they roll and
/// across it, from its `_CarPhysicsV2` (the M151's values in brackets),
/// as the Xbox 360 code works it out (from the 360 code, driving rounds 2
/// and 3: each wheel's coefficients at 0x8219FB08, summed by axle at
/// 0x8219FE80 and averaged at 0x8219AF38, the axle's push at 0x821A08D8,
/// applied per axle at 0x8219C118):
///
/// - Each axle pushes the chassis, at a point between its wheels' spring
///   tops and their contacts ([`Tyres::sway`]), with mass x (its
///   coefficient along ([`Tyres::slowing`], m/s²) x the velocity along
///   its wheels clamped to +-1 m/s, plus its coefficient across
///   ([`Tyres::grip`], per second) x the level velocity across them):
///   so the axles' coefficients add up, every axle's on its own; along,
///   it is a slowing, across, a damping of the sideways slide, with no
///   limit.
/// - Each wheel's coefficients are scaled by the share of the vehicle's
///   wheels on the ground; an axle's are its wheels' mean, a wheel off
///   the ground giving none.
/// - Each step's coefficients are the new ones outright: the code eases
///   them from the last step's (by 0.9, 0.5 braking) only until the first
///   time one falls, which zeroes the easing for good (a static at
///   0x8387C0F8).
///
/// A field the record leaves out takes the 360 code's own default
/// (0x823EB210).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tyres {
    /// Each axle's rolling friction, front and back
    /// (`FrontWheelFrictionFwd`, `RearWheelFrictionFwd`; default 0.5).
    pub rolling: (f32, f32),
    /// Each axle's brake (`FrontBrake` 7, `RearBrake` 6; default 2).
    pub brake: (f32, f32),
    /// Each axle's handbrake (`FrontEmergencyBrake`,
    /// `RearEmergencyBrake`; default 1): up to 1, the share of its
    /// sideways grip it keeps on the handbrake; above, its slowing.
    pub emergency: (f32, f32),
    /// Each axle's sideways grip (`FrontWheelFrictionSide` 1.5,
    /// `RearWheelFrictionSide` 1.5; default 1.5), which the handbrake
    /// slows it with.
    pub side: (f32, f32),
    /// Below this speed, m/s, it rolls harder (`LowSpeedLimit`, km/h,
    /// as the code compares it with its speed in km/h: default 30).
    pub low_speed: f32,
    /// Its sideways grip flat out, of its grip standing (`HiSpeedGrip`;
    /// default 0.8), eased in over its speed's share of its `MaxSpeed`.
    pub high_speed_grip: f32,
    /// How far from its wheels' spring tops towards their contacts its
    /// axles push, at a standstill (`SuspSteerSwayAmp`; default 1).
    pub sway: f32,
    /// The steepest ground it stands and drives on, radians (`MaxSlope`:
    /// 45 degrees for its first value, 60 for the others; from the 360 code,
    /// 0x8219B740). From 20 degrees less its springs push it more and more
    /// as the ground leans and its engine less ([`Tyres::footing`]).
    pub max_slope: f32,
}

impl Tyres {
    /// The 360 code's defaults, for a vehicle with no `_CarPhysicsV2`.
    pub const DEFAULT: Tyres = Tyres {
        rolling: (0.5, 0.5),
        brake: (2.0, 2.0),
        emergency: (1.0, 1.0),
        side: (1.5, 1.5),
        low_speed: 30.0 / 3.6,
        high_speed_grip: 0.8,
        sway: 1.0,
        max_slope: std::f32::consts::FRAC_PI_4,
    };

    /// One axle's coefficient (the front, steering one, or the back),
    /// m/s², at `speed` m/s with the throttle open `throttle` (0 to 1),
    /// the brake pressed `brake` (0 to 1), the handbrake on `handbrake`
    /// (0 to 1) and the stick steering `steer` (-1 to 1):
    ///
    /// - rolling: its rolling friction less what the throttle opens,
    ///   three times as hard at a standstill, easing to once at its low
    ///   speed limit;
    /// - braking: from that to its brake as the brake is pressed;
    /// - else on the handbrake: from that to its sideways grip times
    ///   0.33 + 0.67 x its handbrake x (1 - 0.55 |steer| front, 0.75
    ///   back) (a handbrake over 1: that itself).
    ///
    /// Not here: a reversing or spun wheel's `*Rev180FrictionMulti`.
    pub fn slowing(
        &self,
        front: bool,
        speed: f32,
        throttle: f32,
        brake: f32,
        handbrake: f32,
        steer: f32,
    ) -> f32 {
        let pick = |p: (f32, f32)| if front { p.0 } else { p.1 };
        let low = if self.low_speed > 0.0 {
            1.0 - (speed.abs() / self.low_speed).min(1.0)
        } else {
            0.0
        };
        let rolling =
            (low * 3.0 + (1.0 - low)) * (1.0 - throttle.clamp(0.0, 1.0)) * pick(self.rolling);
        let brake = brake.clamp(0.0, 1.0);
        let handbrake = handbrake.clamp(0.0, 1.0);
        if brake > 0.0 {
            (1.0 - brake) * rolling + brake * pick(self.brake)
        } else if handbrake > 0.01 {
            (1.0 - handbrake) * rolling + handbrake * self.held(front, handbrake, steer)
        } else {
            rolling
        }
    }

    /// What an axle holds with on the handbrake, along and across.
    fn held(&self, front: bool, handbrake: f32, steer: f32) -> f32 {
        let pick = |p: (f32, f32)| if front { p.0 } else { p.1 };
        let emergency = pick(self.emergency);
        if emergency <= 1.0 {
            let turn = if front { 0.55 } else { 0.75 };
            let keep = (1.0 - steer.abs() * turn) * emergency * 0.67 + 0.33;
            pick(self.side) * (handbrake * keep + (1.0 - handbrake))
        } else {
            emergency
        }
    }

    /// One axle's sideways coefficient, per second, at `fast` (its speed's
    /// share of its `MaxSpeed`, 0 to 1), with the brake, handbrake and
    /// stick as [`slowing`](Self::slowing) takes them: its sideways grip,
    /// eased from itself standing to [`high_speed_grip`] of it flat out;
    /// braking, the back's less by 0.15 x |steer| x `fast`; else on the
    /// handbrake, what [`slowing`](Self::slowing) holds with.
    ///
    /// [`high_speed_grip`]: Self::high_speed_grip
    pub fn grip(&self, front: bool, fast: f32, brake: f32, handbrake: f32, steer: f32) -> f32 {
        let pick = |p: (f32, f32)| if front { p.0 } else { p.1 };
        let fast = fast.clamp(0.0, 1.0);
        let mut side = pick(self.side);
        if brake > 0.0 {
            if !front {
                side *= 1.0 - steer.abs() * fast * 0.15;
            }
        } else if handbrake > 0.01 {
            side = self.held(front, handbrake, steer);
        }
        (self.high_speed_grip * fast + (1.0 - fast)) * side
    }

    /// How well it stands on ground leaning `tilt` radians (the mean of
    /// its wheels' contacts' normals' lean; from the 360 code, 0x8219E020):
    /// 1 up to 20 degrees under its [`max_slope`], falling straight to 0
    /// at it. Its springs push it level by that share less of their
    /// lean (0x8219C27C: on gentle ground they push straight up, so it
    /// doesn't roll down), and its engine pulls by that share
    /// (0x8219C6CC: beyond it, it can't drive on).
    ///
    /// [`max_slope`]: Self::max_slope
    pub fn footing(&self, tilt: f32) -> f32 {
        let ramp = 20f32.to_radians();
        let past = (tilt - (self.max_slope - ramp)) / ramp;
        1.0 - past.clamp(0.0, 1.0)
    }

    /// How far from its wheels' spring tops towards their contacts each
    /// axle pushes, at `fast` (as [`grip`](Self::grip) takes it) with its
    /// body rolled so its right side rises `roll` (the sine): its sway
    /// x (2 - `fast`) / 2, less as it rolls, to none at a fifth.
    pub fn sway(&self, fast: f32, roll: f32) -> f32 {
        let fast = fast.clamp(0.0, 1.0);
        (2.0 - fast) * 0.5 * self.sway * (1.0 - (roll.abs() * 5.0).min(1.0))
    }
}

/// A wheeled vehicle's gearbox as it runs (the 360's at the vehicle's
/// 0x94).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gearbox {
    /// 0 reverse, 1 to 5 the forward gears.
    pub gear: usize,
    /// Until it may change gear again, seconds (below 0: it may).
    pub clutch: f32,
    /// How fast the engine turns, rpm.
    pub rpm: f32,
    /// The pedals' pull towards reverse or out of it, seconds, -0.1 to
    /// 0.1 (the vehicle's 0x274).
    pub reversing: f32,
}

impl Default for Gearbox {
    fn default() -> Self {
        Gearbox {
            gear: 1,
            clutch: 0.0,
            rpm: 0.0,
            reversing: 0.0,
        }
    }
}

impl EngineTuning {
    /// The 360 code's defaults, for a vehicle with no `_CarPhysicsV2`.
    pub const DEFAULT: EngineTuning = EngineTuning {
        torque: 500.0,
        rpm: (1000.0, 5000.0, 8000.0),
        upshift: 7000.0,
        downshift: 2000.0,
        clutch_delay: 0.5,
        primary: 8.0,
        gears: [3.0, 2.25, 1.5, 0.75, 0.4],
        reverse: 1.0,
        radius: 0.42,
        torque_at: (1.0, 1.0),
        reverse_speed: 50.0 / 3.6,
        slip: 2000.0,
        split: 1.0,
    };

    /// A gear's ratio, final drive included (0 reverse, backwards; a gear
    /// it hasn't, 0).
    pub fn ratio(&self, gear: usize) -> f32 {
        let r = match gear {
            0 => -self.reverse,
            g => self.gears.get(g - 1).copied().unwrap_or(0.0),
        };
        r * self.primary
    }

    /// One step of its gearbox and engine as the 360 code runs them
    /// (0x821A0528), `dt` s on with the throttle open `throttle` (0 to
    /// 1), its driven wheels turning at `wheels` rpm (weighted by their
    /// shares of the drive, [`split`](Self::split)) and the driver asking
    /// for reverse or not: the torque it pushes with, newton-metres,
    /// backwards in reverse.
    ///
    /// - Once its clutch delay has run out, it goes into reverse from
    ///   first when asked, back to first when not; else up a gear above
    ///   its upshift speed (short of fifth) and down one below its
    ///   downshift speed (short of first), the delay starting again.
    /// - The engine turns at the wheels' speed through the gear; under
    ///   idle the clutch slips, holding it at idle plus the throttle's
    ///   share of its slip, from half idle up eased in towards the
    ///   wheels'.
    /// - Its torque is its most times a factor falling from 1 at the
    ///   optimum to its torque factor at idle (or at its most rpm) as
    ///   the square of the way there; it drags with a tenth of its most
    ///   at the optimum, rising by a twentieth to idle and a fifth to its
    ///   most rpm, three tenths there (nothing pushing), scaled down
    ///   with the speed below idle. It pushes with the throttle's share
    ///   of the one less the other.
    ///
    /// A gear whose ratio is 0 isn't changed up into (the 360 would).
    pub fn step(&self, g: &mut Gearbox, dt: f32, throttle: f32, wheels: f32, reverse: bool) -> f32 {
        g.clutch -= dt;
        if g.clutch < 0.0 {
            if reverse && g.gear == 1 {
                g.gear = 0;
            }
            let rpm = self.ratio(g.gear) * wheels;
            if g.gear == 0 {
                if !reverse {
                    g.gear = 1;
                }
            } else {
                if rpm < self.downshift && g.gear > 1 {
                    g.clutch = self.clutch_delay;
                    g.gear -= 1;
                }
                if rpm > self.upshift && g.gear < 5 && self.ratio(g.gear + 1) != 0.0 {
                    g.gear += 1;
                    g.clutch = self.clutch_delay;
                }
            }
        }
        let (min, opt, max) = self.rpm;
        let ratio = self.ratio(g.gear);
        let turning = ratio * wheels;
        let throttle = throttle.clamp(0.0, 1.0);
        g.rpm = turning;
        if turning < min {
            let half = min * 0.5;
            g.rpm = if turning >= half {
                (turning - half) * (self.slip * throttle) / (min - half) + min
            } else {
                min + self.slip * throttle
            };
        }
        let past = g.rpm - opt;
        let (push, drag) = if past >= 0.0 {
            if g.rpm >= max {
                g.rpm = max;
                (0.0, 0.3 * self.torque)
            } else {
                let f = 1.0 / (max - opt);
                (
                    ((self.torque_at.1 - 1.0) * f * f * past * past + 1.0) * self.torque,
                    (0.2 * f * past + 0.1) * self.torque,
                )
            }
        } else {
            let f = 1.0 / (opt - min);
            let drag = (-0.05 * f * past + 0.1) * self.torque;
            (
                ((self.torque_at.0 - 1.0) * f * f * past * past + 1.0) * self.torque,
                if turning < min {
                    drag / min * turning
                } else {
                    drag
                },
            )
        };
        let torque = push * throttle - drag;
        if ratio < 0.0 { -torque } else { torque }
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

    /// Field `field` in `record` as three floats in a row (a vector).
    pub(crate) fn vector(&self, record: &[u8], field: u32) -> Option<[f32; 3]> {
        let &(at, _, _) = self.fields.get(&field)?;
        let b = record.get(at..at + 12)?;
        let v: [f32; 3] = std::array::from_fn(|i| {
            f32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]])
        });
        v.iter().all(|f| f.is_finite()).then_some(v)
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
/// `_HumanPhysics`'s `UprightWalkSpeed`, `UprightJogSpeed` and
/// `UprightRunSpeed` (names from the program's registration at
/// 0x823EAE98, hashed): Jennifer 2.0, 5.0 and 7.0, Chris 1.5, 4.5 and
/// 6.25, Mattias 1.5, 4.5 and 5.5. The alpha's walk, run and sprint keys
/// take them in that order (inferred: how the 360's player control scales
/// the stick is not traced).
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

/// Every person template's `_HumanPhysics` jump, gravity, turn and paces
/// ([`crate::on_foot::HumanMotion`]), by id; a field the record lacks
/// keeps the program's default.
pub fn human_motion(db: &Layer<'_>) -> HashMap<u32, crate::on_foot::HumanMotion> {
    use crate::on_foot::{FIELDS, HumanMotion};
    let physics = Table::new(db, "_HumanPhysics");
    let d = HumanMotion::default();
    let defaults = [
        d.jump_height,
        d.gravity,
        d.max_rotation,
        d.max_air_speed,
        d.paces[0],
        d.paces[1],
        d.paces[2],
        d.paces[3],
        d.crouch_walk,
        d.capsule_radius,
        d.capsule_height,
    ];
    physics
        .records
        .keys()
        .filter_map(|&id| {
            let record = physics.of_template(id).first()?;
            let mut out = defaults;
            for (o, name) in out.iter_mut().zip(FIELDS) {
                if let Some(v) = physics.float(record, name_hash(name)) {
                    *o = v;
                }
            }
            Some((id, HumanMotion::from_fields(out)))
        })
        .collect()
}

/// `_HumanPhysics`'s swimming speeds, m/s: `SwimWalkSpeed`,
/// `SwimJogSpeed`, `SwimRunSpeed` and `SwimSprintSpeed` (field names from
/// the exe's reflection registration of `_HumanPhysics`, 0x823EAE98,
/// which lists them after the four `Upright` speeds; defaults there 1, 2,
/// and the others' as read). From the 360 code: a person is swimming
/// while his stance is `Swim` (the `Object.IsSwimming` native,
/// 0x82324FB8).
pub const SWIM_SPEEDS: [u32; 4] = [0x0aae_47db, 0xa95d_1404, 0x78a5_7e3d, 0x4e95_4566];

/// Every person template's four `_HumanPhysics` swimming speeds (walk,
/// jog, run, sprint), by id, where all four are given and above 0.
pub fn human_swim_speeds(db: &Layer<'_>) -> HashMap<u32, [f32; 4]> {
    let physics = Table::new(db, "_HumanPhysics");
    physics
        .records
        .keys()
        .filter_map(|&id| {
            let record = physics.of_template(id).first()?;
            let mut out = [0.0; 4];
            for (o, field) in out.iter_mut().zip(SWIM_SPEEDS) {
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
    let stimulus = Table::new(db, "Stimulus");
    let fudge = Table::new(db, "ExplosionFudge");
    let tick = Table::new(db, "TickDamage");
    let modifier = Table::new(db, "ModifierKey");
    let heli = Table::new(db, "_HelicopterPhysics");
    let heli_ai = Table::new(db, "_HelicopterPhysicsAi");
    let pilot = Table::new(db, "AiHelicopter");
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
                visibility: stimulus.get(id, "Visibility"),
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
                        slip: f("GearSlipRPM")
                            .unwrap_or(EngineTuning::DEFAULT.slip)
                            .max(0.0),
                        split: f("RearToFrontTorqueDistrib")
                            .unwrap_or(EngineTuning::DEFAULT.split)
                            .clamp(0.0, 1.0),
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
                    let d = Handling::DEFAULT;
                    let or = |name, or: f32| car.float(r, name_hash(name)).unwrap_or(or);
                    Some(Handling {
                        steer_lock: f("SteerMaxAngle").filter(|a| *a < 1.5)?,
                        steer_speed: f("SteerSpeed")?,
                        low_speed: low.map_or(0.0, |l| l.0),
                        low_steer_speed: low.map_or(0.0, |l| l.1),
                        high_speed_loss: or("HighSpeedSteerLoss", d.high_speed_loss),
                        loss_braking: or("HighSpeedSteerLossBrakeMulti", d.loss_braking),
                        loss_coasting: or("HighSpeedSteerLossNoThrottleMulti", d.loss_coasting),
                        inertia: or("SteeringInertia", d.inertia),
                        no_input_inertia: or("SteerNoInputInertia", d.no_input_inertia),
                        diff_sign_inertia: or("SteerDiffSignInertia", d.diff_sign_inertia),
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
                tyres: (|| {
                    let r = *car.of_template(id).first()?;
                    let d = Tyres::DEFAULT;
                    let f = |name, or: f32| car.float(r, name_hash(name)).unwrap_or(or).max(0.0);
                    let pair = |front, rear, or: (f32, f32)| (f(front, or.0), f(rear, or.1));
                    Some(Tyres {
                        rolling: pair("FrontWheelFrictionFwd", "RearWheelFrictionFwd", d.rolling),
                        brake: pair("FrontBrake", "RearBrake", d.brake),
                        emergency: pair("FrontEmergencyBrake", "RearEmergencyBrake", d.emergency),
                        side: pair("FrontWheelFrictionSide", "RearWheelFrictionSide", d.side),
                        low_speed: f("LowSpeedLimit", d.low_speed * 3.6) / 3.6,
                        high_speed_grip: f("HiSpeedGrip", d.high_speed_grip),
                        sway: f("SuspSteerSwayAmp", d.sway),
                        max_slope: match car.word(r, name_hash("MaxSlope")) {
                            Some(v) if v != 0 => 60f32.to_radians(),
                            _ => d.max_slope,
                        },
                    })
                })(),
                tank: (|| {
                    let r = *tank.of_template(id).first()?;
                    let f = |name, default| tank.float(r, name_hash(name)).unwrap_or(default);
                    Some(TankTuning {
                        max_speed: f("MaxSpeed", 40.0) / 3.6,
                        brake: f("Brake", 20.0),
                        steer_speed: f("SteerSpeed", 15.0),
                        steer_inertia: (f("SteerInertia", 0.0), f("SteerInertiaNoInput", 0.0)),
                        friction: (f("FwdFriction", 1.0), f("SideFriction", 30.0)),
                        engine_height: f("EngineOffsetY", -0.25),
                        gravity_factor: f("GravityFactor", 2.0),
                        steer_loss: f("HiSpeedSteerLoss", 0.0),
                        high_speed_grip: f("HiSpeedGrip", 1.0),
                        ang_damp: [f("AngDampX", 0.5), f("AngDampY", 1.0), f("AngDampZ", 5.0)],
                        centre_of_mass: tank
                            .vector(r, name_hash("CenterOfMassOffset"))
                            .unwrap_or([0.0; 3]),
                        torque: f("EngineTorque", 1000.0),
                        engine_inertia: f("EngineInertia", 0.95),
                        curve: [
                            (0.0, f("EngineTorqueRatio0", 1.0)),
                            (f("EngineTorquePos1", 0.25), f("EngineTorqueRatio1", 0.55)),
                            (f("EngineTorquePos2", 0.5), f("EngineTorqueRatio2", 0.55)),
                            (1.0, f("EngineTorqueRatio3", 0.1)),
                        ],
                        springs: (
                            f("SpringStrength", 9.0),
                            f("SpringDamp", 0.4),
                            f("SpringExpLength", 0.17),
                            f("SpringCmpLength", 0.3),
                        ),
                    })
                })(),
                flight: flight_of(&heli, id),
                flight_ai: flight_of(&heli_ai, id),
                pilot: (|| {
                    let r = *pilot.of_template(id).first()?;
                    let d = PilotTuning::DEFAULT;
                    let f = |name, default| pilot.float(r, name_hash(name)).unwrap_or(default);
                    Some(PilotTuning {
                        min_altitude: f("MinAltitude", d.min_altitude),
                        attack_radius: f("AttackRadius", d.attack_radius),
                        attack_height: f("AttackHeight", d.attack_height),
                        search_radius: f("SearchRadius", d.search_radius),
                        search_height: f("SearchHeight", d.search_height),
                        search_time: f("SearchTime", d.search_time),
                        landing_slope: f("LandingSlopeTolerance", d.landing_slope),
                        outer_height: f("OuterHeightTolerance", d.outer_height),
                        flyover_rate: f("FlyoverAttackRate", d.flyover_rate),
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
                        hull_friction: f("HullFriction").unwrap_or(0.3),
                        float: {
                            let d = BoatFloat::DEFAULT;
                            // Not held to 0 or more: spring heights go below
                            // the origin.
                            let g = |name: &str, default: f32| {
                                boat.float(r, name_hash(name)).unwrap_or(default)
                            };
                            let spring = |speed: &str, d: FloatSpring| FloatSpring {
                                start: g(&format!("Spring{speed}SpeedStartOffset"), d.start),
                                end: g(&format!("Spring{speed}SpeedEndOffset"), d.end),
                                strength: g(&format!("Spring{speed}SpeedStrength"), d.strength),
                                damping: g(&format!("Spring{speed}SpeedDampening"), d.damping),
                            };
                            BoatFloat {
                                slow: spring("Lo", d.slow),
                                fast: spring("Hi", d.fast),
                                smoothing: g("SpringWaterSmoothing", d.smoothing),
                                roll: g("RollFromTurning", d.roll),
                                roll_inertia: g("RollInertia", d.roll_inertia),
                                pitch: g("PitchFromAcceleration", d.pitch),
                                pitch_up: g("PitchUpSpeed", d.pitch_up),
                                pitch_down_inertia: g("PitchDownInertia", d.pitch_down_inertia),
                                low_speed: g("LowSpeedLimit", d.low_speed * 3.6) / 3.6,
                                steer_loss: g("HighSpeedSteerLoss", d.steer_loss),
                                steer_inertia: g("SteerInertia", d.steer_inertia),
                                steer_release: g("SteerNoInputInertia", d.steer_release),
                                top: g("MaxSpeed", d.top * 3.6) / 3.6,
                                reverse: g("MaxSpeedReverse", d.reverse * 3.6) / 3.6,
                                steering: g("Steering", d.steering),
                                no_speed_steering: g("NoSpeedSteering", d.no_speed_steering),
                                low_speed_steering: g("LowSpeedSteering", d.low_speed_steering),
                                handbrake_boost: g("HandbrakeSteerBoost", d.handbrake_boost),
                                air_steering: g("AirCtrlSteering", d.air_steering),
                                handbrake_inertia: g("HandBrakeInertia", d.handbrake_inertia),
                                anti_side_drag: g("HandBrakeAntiSideDrag", d.anti_side_drag),
                                momentum_keep: g("HandBrakeMomentumKeep", d.momentum_keep),
                                stabilization: g("CollisionStabilization", d.stabilization),
                                hit_torque: g("EngineTorqueMultiAtCollision", d.hit_torque),
                                curve_at: (
                                    g("EngineTorquePos1", d.curve_at.0),
                                    g("EngineTorquePos2", d.curve_at.1),
                                ),
                                ang_damp: [
                                    g("AngDampRoll", d.ang_damp[0]),
                                    g("AngDampPitch", d.ang_damp[1]),
                                    g("AngDampYaw", d.ang_damp[2]),
                                ],
                                engine_inertia: g("EngineInertia", d.engine_inertia),
                            }
                        },
                    };
                    (b.torque.0 > 0.0 && b.drag[0] > 0.0).then_some(b)
                })(),
                body: (|| {
                    let r = *car.of_template(id).first()?;
                    let f = |name, default| car.float(r, name_hash(name)).unwrap_or(default);
                    Some(CarBody {
                        inertia: [
                            f("InertiaTensorX", 2.0),
                            f("InertiaTensorY", 1.0),
                            f("InertiaTensorZ", 1.5),
                        ],
                        centre_of_mass: car
                            .vector(r, name_hash("CenterOfMassOffset"))
                            .unwrap_or([0.0; 3]),
                    })
                })(),
                bike: (|| {
                    let r = *car.of_template(id).first()?;
                    let f = |name, default| car.float(r, name_hash(name)).unwrap_or(default);
                    let b = BikeTuning {
                        roll_rate: f("BikeRollAngVelForMaxRoll", 0.0),
                        roll_angle: (
                            f("BikeRollAngleAtLoSpeed", 0.0),
                            f("BikeRollAngleAtHiSpeed", 0.0),
                        ),
                        roll_input: f("BikeRollAngleFromInput", 0.0),
                        roll: f("BikeRollStabilityStrength", 0.0),
                        wheelie: f("BikeWheeliePitch", 0.0),
                        pitch_strength: (
                            f("BikeWheeliePitchStrengthUp", 0.0),
                            f("BikeWheeliePitchStrengthDown", 0.0),
                        ),
                        pitch_high_speed: f("BikeWheeliePitchHiSpeedRatio", 0.0),
                        wheelie_steer: f("BikeWheelieSteerAbility", 0.0),
                        side_damp: f("BikeWheelieSideDamp", 0.0),
                        damp: [
                            f("BikeExtraDampRoll", 0.0),
                            f("BikeExtraDampPitch", 0.0),
                            f("BikeExtraDampYaw", 0.0),
                        ],
                        air: (f("BikeAirControlPitch", 0.0), f("BikeAirControlYaw", 0.0)),
                        low_speed: f("LowSpeedLimit", 30.0) / 3.6,
                        inertia: [
                            f("InertiaTensorX", 2.0),
                            f("InertiaTensorY", 1.0),
                            f("InertiaTensorZ", 1.5),
                        ],
                        centre_of_mass: car
                            .vector(r, name_hash("CenterOfMassOffset"))
                            .unwrap_or([0.0; 3]),
                    };
                    b.balances().then_some(b)
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

/// Every template's cover hints (its `CoverHintOffset` entries,
/// [`crate::ai_cover::Hint`]), by id: `Type` (Offset 0, Point 1, Box 2),
/// `Rating`, `CoverAnimation`, `CoverPosition`, `FacingYawOrRadius`,
/// `CoverRange`, `RestrictRadius`, `NoCoverX`, `NoCoverZ` (research thread,
/// 360 notes s24).
pub fn cover_hints(db: &Layer<'_>) -> HashMap<u32, Vec<crate::ai_cover::Hint>> {
    use crate::ai_cover::{Hint, Kind};
    let t = Table::new(db, "CoverHintOffset");
    let word = |r: &[u8], n: &str| t.word(r, name_hash(n)).unwrap_or(0);
    let float = |r: &[u8], n: &str, d: f32| t.float(r, name_hash(n)).unwrap_or(d);
    t.records
        .keys()
        .map(|&id| {
            let hints = t
                .of_template(id)
                .iter()
                .filter_map(|r| {
                    let kind = match word(r, "Type") {
                        0 => Kind::Offset,
                        1 => Kind::Point,
                        2 => Kind::Box,
                        _ => return None,
                    };
                    Some(Hint {
                        kind,
                        rating: word(r, "Rating").min(255) as u8,
                        animation: word(r, "CoverAnimation").min(255) as u8,
                        position: t
                            .vector(r, name_hash("CoverPosition"))
                            .map_or(m2_math::glam::Vec3::ZERO, m2_math::glam::Vec3::from),
                        facing_or_radius: float(r, "FacingYawOrRadius", 0.0),
                        range: float(r, "CoverRange", 90.0),
                        restrict: float(r, "RestrictRadius", 50.0),
                        no_x: word(r, "NoCoverX") != 0,
                        no_z: word(r, "NoCoverZ") != 0,
                    })
                })
                .collect();
            (id, hints)
        })
        .collect()
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
            slip: 1000.0,
            split: 1.0,
        }
    }

    // Its torque as the 360 code works it out (0x821A0528): its most at
    // the optimum less a tenth dragging; at idle 0.8 of it less 0.15;
    // at its most rpm nothing but 0.3 dragging.
    #[test]
    fn the_engine_gives_its_torque_as_the_360s() {
        let e = m151();
        let at = |rpm: f32, throttle: f32| {
            let mut g = Gearbox {
                clutch: 1.0,
                ..Gearbox::default()
            };
            e.step(&mut g, 0.0, throttle, rpm / e.ratio(1), false)
        };
        let close = |a: f32, b: f32| assert!((a - b).abs() < 0.01, "{a} {b}");
        close(at(3000.0, 1.0), 130.0 - 13.0);
        close(at(900.0, 1.0), 0.8 * 130.0 - 0.15 * 130.0);
        close(at(12_000.0, 1.0), -0.3 * 130.0);
        // Off the throttle it only drags: a tenth at the optimum.
        close(at(3000.0, 0.0), -13.0);
        // Halfway from the optimum to idle: 1 - 0.2 / 4 of it.
        close(at(1950.0, 1.0), 0.95 * 130.0 - 0.125 * 130.0);
    }

    // Under idle the clutch slips: standing, the engine turns at idle plus
    // the throttle's share of its slip.
    #[test]
    fn the_clutch_slips_under_idle() {
        let e = m151();
        let mut g = Gearbox::default();
        e.step(&mut g, 1.0 / 60.0, 0.5, 0.0, false);
        assert!((g.rpm - 1400.0).abs() < 1e-3, "{}", g.rpm);
    }

    // It changes up above its 4,000 rpm and down below its 2,500, one gear
    // at a time; into reverse from first when asked, pushing backwards,
    // and back to first when not.
    #[test]
    fn the_gearbox_changes_as_the_360s() {
        let e = m151();
        let mut g = Gearbox::default();
        let first = |rpm: f32| rpm / e.ratio(1);
        e.step(&mut g, 0.01, 1.0, first(3990.0), false);
        assert_eq!(g.gear, 1);
        e.step(&mut g, 0.01, 1.0, first(4010.0), false);
        assert_eq!(g.gear, 2);
        // In second that is 3,108 rpm: it stays.
        e.step(&mut g, 0.01, 1.0, first(4010.0), false);
        assert_eq!(g.gear, 2);
        e.step(&mut g, 0.01, 1.0, 2490.0 / e.ratio(2), false);
        assert_eq!(g.gear, 1);
        let back = e.step(&mut g, 0.01, 1.0, 0.0, true);
        assert_eq!(g.gear, 0);
        assert!(back < 0.0, "{back}");
        e.step(&mut g, 0.01, 1.0, 0.0, false);
        assert_eq!(g.gear, 1);
        // Its clutch delay holds the gear it changed to.
        let slow = EngineTuning {
            clutch_delay: 0.5,
            ..e
        };
        let mut g = Gearbox::default();
        slow.step(&mut g, 0.01, 1.0, first(4010.0), false);
        slow.step(&mut g, 0.01, 1.0, 5000.0 / e.ratio(2), false);
        assert_eq!(g.gear, 2);
    }

    // A gearbox with fewer than five gears stops at its last.
    #[test]
    fn a_missing_gear_is_no_gear() {
        let e = EngineTuning {
            gears: [3.0, 2.0, 1.0, 0.0, 0.0],
            ..m151()
        };
        let mut g = Gearbox {
            gear: 3,
            ..Gearbox::default()
        };
        e.step(&mut g, 0.01, 1.0, 9000.0 / e.ratio(3), false);
        assert_eq!(g.gear, 3);
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

    // Footing: whole up to 20 degrees under its steepest, none at it.
    #[test]
    fn footing_falls_over_the_last_20_degrees() {
        let t = Tyres::DEFAULT;
        let close = |a: f32, b: f32| assert!((a - b).abs() < 1e-5, "{a} {b}");
        close(t.footing(20f32.to_radians()), 1.0);
        close(t.footing(35f32.to_radians()), 0.5);
        close(t.footing(50f32.to_radians()), 0.0);
        let steep = Tyres {
            max_slope: 60f32.to_radians(),
            ..t
        };
        close(steep.footing(45f32.to_radians()), 0.75);
    }

    // A boat's springs mix from slow to fast with its speed, each end's
    // length mixed (0x8219A00C), and hold its weight with the water a
    // quarter of g over its strength above their bottoms (the Huangfeng's
    // record: 4 to 2.5 m slow, 5 to 3 fast, strength 8 and 9).
    #[test]
    fn boat_springs_mix_with_speed_and_hold_its_weight() {
        let f = BoatFloat {
            slow: FloatSpring {
                start: 4.0,
                end: 2.5,
                strength: 8.0,
                damping: 2.0,
            },
            fast: FloatSpring {
                start: 5.0,
                end: 3.0,
                strength: 9.0,
                damping: 4.0,
            },
            ..BoatFloat::DEFAULT
        };
        let half = f.spring(0.5);
        assert_eq!((half.start, half.end), (4.5, 2.75));
        assert_eq!((half.strength, half.damping), (8.5, 3.0));
        assert_eq!(f.length(0.5), 1.75);
        assert_eq!(f.spring(2.0), f.fast);
        assert!((f.rest_depth(10.0) - (2.5 + 10.0 / 32.0)).abs() < 1e-6);
        // Upside down (its top under its bottom), a spring has no length
        // and its top is its bottom.
        let flipped = FloatSpring {
            start: -0.1,
            end: 0.2,
            ..f.slow
        };
        assert_eq!(flipped.length(), 0.0);
        let g = BoatFloat { slow: flipped, ..f };
        assert!((g.rest_depth(10.0) - (-0.1 + 10.0 / 32.0)).abs() < 1e-6);
    }

    // Easing keeps its share of the way left each 1/60 s, a share of a
    // step for the rest (0x824BB050).
    #[test]
    fn ease_keeps_its_share_each_sixtieth() {
        let close = |a: f32, b: f32| assert!((a - b).abs() < 1e-5, "{a} {b}");
        close(ease(1.0, 0.0, 0.9, 1.0 / 60.0), 0.9);
        close(ease(1.0, 0.0, 0.9, 3.0 / 60.0), 0.729);
        close(ease(1.0, 0.0, 0.9, 1.5 / 60.0), 0.9 - 0.5 * 0.09);
        close(ease(1.0, 0.0, 0.0, 1.0 / 60.0), 0.0);
        close(ease(1.0, 0.0, 0.5, 20.0), 0.0);
    }

    // An axle's slowing as the 360 code works it out (0x8219FB08).
    #[test]
    fn brakes_slow_as_the_360_code_has_them() {
        let b = Tyres {
            rolling: (0.5, 0.4),
            brake: (7.0, 6.0),
            emergency: (0.5, 1.5),
            side: (1.5, 1.2),
            low_speed: 30.0 / 3.6,
            high_speed_grip: 0.8,
            sway: 1.0,
            max_slope: std::f32::consts::FRAC_PI_4,
        };
        let close = |a: f32, b: f32| assert!((a - b).abs() < 1e-5, "{a} {b}");
        // Rolling: three times as hard standing, once from its low speed
        // limit up, none at full throttle.
        close(b.slowing(true, 0.0, 0.0, 0.0, 0.0, 0.0), 1.5);
        close(b.slowing(true, 15.0 / 3.6, 0.0, 0.0, 0.0, 0.0), 1.0);
        close(b.slowing(false, 20.0, 0.0, 0.0, 0.0, 0.0), 0.4);
        close(b.slowing(true, 20.0, 1.0, 0.0, 0.0, 0.0), 0.0);
        // Braking: from rolling to its brake.
        close(b.slowing(true, 20.0, 0.0, 1.0, 0.0, 0.0), 7.0);
        close(b.slowing(false, 20.0, 0.0, 0.5, 0.0, 0.0), 3.2);
        // The handbrake: its sideways grip times 0.33 + 0.67 x 0.5 x (1 -
        // 0.55 |steer|) at the front; a handbrake over 1 is itself.
        close(
            b.slowing(true, 20.0, 0.0, 0.0, 1.0, 0.0),
            1.5 * (0.33 + 0.335),
        );
        close(
            b.slowing(true, 20.0, 0.0, 0.0, 1.0, 1.0),
            1.5 * (0.33 + 0.335 * 0.45),
        );
        close(b.slowing(false, 20.0, 0.0, 0.0, 1.0, 0.0), 1.5);
    }
}
