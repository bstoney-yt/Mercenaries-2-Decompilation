//! The player on foot: Jennifer walking the island under the character
//! controller, with a third-person camera behind her.

use m2_engine::{Character, Gait, GpuModel, Physics, VehicleId, VehicleSpec};
use m2_math::glam::{Mat4, Vec2, Vec3};
use m2_render::View;

use crate::binoculars::Binoculars;
use crate::combat::{Trigger, Weapon};
use crate::config::CameraConfig;
use crate::held::{InHand, Moves, Phase, Switch};
use crate::input::{Action, Input};
use crate::npc::FightClips;
use crate::scene::{Carry, Rig};

/// Movement speeds, m/s, inside the bands the game's clips move at.
/// Seconds the mouse must be still before the chase camera swings back
/// behind the vehicle.
const CHASE_HOLD: f32 = 1.5;
const WALK: f32 = 1.4;
const RUN: f32 = 5.0;
const SPRINT: f32 = 7.2;
/// Jennifer's walk, run and sprint; another hero's are these scaled by
/// their template's speeds against hers ([`Player::hero_speeds`]).
pub const SPEEDS: [f32; 3] = [WALK, RUN, SPRINT];
/// Take-off speed of a jump, m/s (about 0.8 m high).
const JUMP: f32 = 4.0;
/// Seconds after take-off before touching the ground counts as landing.
const MIN_AIR: f32 = 0.15;
/// Seconds before a landing clip's end that the gait takes over again
/// (and a bash's).
const LAND_FADE: f32 = 0.1;
/// Speeds (m/s) above which she takes off and lands on the run.
const RUNNING_JUMP: f32 = 1.0;
const RUNNING_LAND: f32 = 1.5;
/// How fast she turns to face where she is going, radians per second.
const TURN_RATE: f32 = 10.0;
/// Swimming. She swims where the sea over the ground under her is deeper
/// than this, metres (chest deep: the depth she gets out of a boat into,
/// `game::WADE`), and stands again where it is shallower than
/// [`SWIM_STOP`]; between the two she keeps doing what she did, so the
/// shore's slope doesn't flick her in and out. Invented: the game's own
/// depth isn't read.
/// When a state layer goes: how far under her feet her floor is looked
/// for, and how near the floor after must be to the floor before for it
/// to be the same one, metres. Invented.
const FLOOR_LOOK: f32 = 60.0;
const FLOOR_SAME: f32 = 0.25;
pub const SWIM_START: f32 = 1.4;
pub const SWIM_STOP: f32 = 1.15;
/// How far her model's origin hangs under the surface while she swims,
/// metres: the stroke's body lies flat (its highest node only 0.11 m
/// over its hips, from samples) about a third of a metre over the origin,
/// so her back is at the waterline. Invented, by eye (at 0.9 m she was
/// all under the water in the picture).
pub const SWIM_FLOAT: f32 = 0.35;
/// Swimming speed, m/s: the stroke's own root speed (0.81 m/s, from
/// samples) played at twice its rate. Invented.
pub const SWIM_SPEED: f32 = 1.6;
/// The stroke's rate while she floats still (the `Swim` stance's `Idle`
/// gives no clip of its own). Invented.
pub const SWIM_STILL_RATE: f32 = 0.35;
/// Camera: eye height looked at, distance behind, and closest it gets
/// when something is in the way.
const LOOK_HEIGHT: f32 = 1.6;
const CAMERA_DISTANCE: f32 = 4.0;
const CAMERA_MIN: f32 = 0.6;
/// How far the eye keeps from what is between it and the point it turns
/// about, metres: a ball this big is swept out from that point, so the
/// near plane (0.1 m off, its corners about 0.13 m from the eye at the
/// 60 degree camera) stays out of walls, pillars and doors a slanting ray
/// passed beside. The game's camera keeps clear of them too (PC footage);
/// its own margin isn't in the data (inferred).
const CAMERA_RADIUS: f32 = 0.25;
/// How far above the ground under it the chase camera keeps.
const CAMERA_CLEARANCE: f32 = 0.4;
/// How the chase camera follows a vehicle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChaseRig {
    /// The point it turns about, model space.
    pub pivot: Vec3,
    /// How far behind that point it sits, metres.
    pub distance: f32,
    /// Where it looks when the drive starts (radians, up positive), if the
    /// vehicle says, and how far down and up it may look.
    pub pitch: Option<f32>,
    pub pitch_range: (f32, f32),
    /// It stays where the player aims, as a turret does, rather than
    /// swinging back behind the vehicle once it moves.
    pub follows_aim: bool,
    /// How far above the camera's line the crosshair (and the gun's aim)
    /// sits, radians.
    pub aim_lift: f32,
}

/// How far above the middle of the screen a tank's crosshair sits,
/// radians: in the Xbox 360 playthrough it is 0.38 of the way down the
/// screen, 8 degrees above the camera's line with the 60 degree camera,
/// so with the camera looking a little down at the tank, the gun aims at
/// the horizon.
const TANK_AIM_LIFT: f32 = 0.138;
/// And on foot: 0.37 of the way down in the playthrough.
const ON_FOOT_AIM_LIFT: f32 = 0.149;

impl ChaseRig {
    /// `spec`'s: its template's tank camera (above the turret ring, as
    /// far back as the template says, looking a little down, staying
    /// where the turret aims), else 2 m above its origin and back by about
    /// its length, swinging back behind it unless it runs on tracks.
    pub fn of(spec: &VehicleSpec) -> Self {
        // A helicopter's: as far back as its template says (field
        // ee30a43d; inferred), looking down its `DefaultPitch`, from above
        // its hub height.
        if let Some((distance, pitch)) = spec.tuning.chase.filter(|_| spec.is_helicopter()) {
            return Self {
                pivot: Vec3::Y * (spec.bounds.1.y * 0.6),
                distance,
                pitch: Some(-pitch),
                pitch_range: (-1.2, 0.9),
                follows_aim: false,
                aim_lift: 0.0,
            };
        }
        match spec.tuning.camera {
            Some(c) => Self {
                pivot: spec.pivot.unwrap_or(Vec3::ZERO).with_x(0.0) + Vec3::Y * c.height,
                distance: c.distance,
                pitch: Some(c.pitch),
                pitch_range: c.pitch_range,
                follows_aim: true,
                aim_lift: TANK_AIM_LIFT,
            },
            None => Self {
                pivot: Vec3::Y * 2.0,
                distance: (spec.bounds.1.z - spec.bounds.0.z) * 0.9 + 3.0,
                pitch: None,
                pitch_range: (-1.2, 0.9),
                follows_aim: spec.tracked,
                aim_lift: 0.0,
            },
        }
    }
}

/// Ticks the world must have been quiet before she lands: a building's
/// collision can arrive a few frames after the ground under it.
const SETTLED: u8 = 3;
/// Ticks she waits for the world to stop coming in before she lands
/// anyway (ten seconds; invented: models that never load hold no one up).
const LAND_WAIT: u16 = 600;

/// Put down at a height ([`Player::drop_at`]), she lands on what is under
/// a point this far above it, found within this far down: the floor or
/// ground there rather than a roof or deck over it (inferred, as
/// [`crate::npc`] lands people at their posts).
const LAND_ABOVE: f32 = 2.0;
const LAND_REACH: f32 = 60.0;

/// What the player asks for this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Intent {
    /// Camera-relative: x right, y forward; length at most 1.
    pub movement: Vec2,
    pub sprint: bool,
    pub walk: bool,
    pub jump: bool,
    /// Jump held: the brake when driving, climbing in a helicopter.
    pub hold_jump: bool,
    /// Crouch held: coming down in a helicopter.
    pub lower: bool,
    /// Get in or out of a vehicle.
    pub use_pressed: bool,
    /// Turn an overturned vehicle back up.
    pub right_vehicle: bool,
    /// Camera turn, radians: (yaw, pitch).
    pub look: Vec2,
    /// Weapons.
    pub trigger: Trigger,
}

impl Intent {
    pub fn from_input(input: &mut Input, config: &CameraConfig, mouse_look: bool) -> Self {
        let (dx, dy) = input.take_look(mouse_look);
        let invert = if config.invert_y { -1.0 } else { 1.0 };
        // Turning right turns the yaw (from +z towards +x) back.
        let look = Vec2::new(
            -dx as f32 * config.mouse_sensitivity,
            -dy as f32 * config.mouse_sensitivity * invert,
        );
        Self {
            movement: Vec2::new(
                input.axis(Action::MoveLeft, Action::MoveRight),
                input.axis(Action::MoveBack, Action::MoveForward),
            )
            .clamp_length_max(1.0),
            sprint: input.held(Action::Sprint),
            walk: input.held(Action::Walk),
            jump: input.take_pressed(Action::MoveUp),
            hold_jump: input.held(Action::MoveUp),
            lower: input.held(Action::MoveDown),
            use_pressed: input.take_pressed(Action::Use),
            right_vehicle: input.take_pressed(Action::Reset),
            look,
            trigger: Trigger {
                fire: input.held(Action::Fire),
                reload: input.take_pressed(Action::Reload),
                aim: input.held(Action::Aim),
                throw: input.take_pressed(Action::Throw),
                next_explosive: input.take_pressed(Action::Explosive),
                detonate: input.take_pressed(Action::Detonate),
                seat: input.take_pressed(Action::Seat),
                melee: input.take_pressed(Action::Melee),
                select: if input.take_pressed(Action::Rifle) {
                    Some(Weapon::Rifle)
                } else if input.take_pressed(Action::Launcher) {
                    Some(Weapon::Rpg)
                } else {
                    None
                },
                support: input.take_pressed(Action::Support),
                support_down: input.take_pressed(Action::SupportDown),
                pda: input.take_pressed(Action::Pda),
            },
        }
    }
}

/// A melee bash she is in: seconds into it, its clip, and whether it is
/// the low one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bash {
    pub time: f32,
    clip: usize,
    pub low: bool,
}

/// Where she is in a jump.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Leap {
    /// On the ground.
    No,
    /// In the air, this many seconds after taking off.
    Air(f32),
    /// Landed this many seconds ago, playing the landing clip.
    Land(f32, usize),
}

/// The player character.
#[derive(Debug)]
pub struct Player {
    pub character: Character,
    /// Heading she faces, radians clockwise from `+z` seen from above.
    pub facing: f32,
    /// Camera heading and pitch (radians, pitch up from the horizon).
    pub yaw: f32,
    pub pitch: f32,
    /// Distance from the look point to the camera this frame.
    camera_distance: f32,
    /// Seconds since the mouse last turned the camera.
    look_still: f32,
    /// While driving: the point the chase camera turns about, and how far
    /// behind it the camera sits ([`ChaseRig`]).
    pub chase_target: Option<(Vec3, f32)>,
    /// While driving: how far above the camera's line the crosshair sits
    /// ([`ChaseRig::aim_lift`]).
    aim_lift: f32,
    /// False until there is ground under the spawn point to land on.
    pub landed: bool,
    /// The spot she was put down at ([`Player::drop_at`]), to land near
    /// while she is still over it; otherwise (a spawn's x and z alone, or
    /// moved since) she lands on the top of whatever is under her.
    land_from: Option<Vec3>,
    /// Set each tick the world around her is still streaming in: she does
    /// not land until it has been quiet for [`SETTLED`] ticks, so that she
    /// lands on the buildings and not inside one that loads a moment later.
    pub streaming: bool,
    /// Set each tick the layers' models near her are still loading.
    pub models_coming: bool,
    /// Ticks in a row the world has been quiet while she waits to land.
    settled: u8,
    /// Ticks she has waited to land: past [`LAND_WAIT`] she no longer
    /// waits for the layers' models.
    waited: u16,
    /// The floor under her was taken down ([`floor_went`](Self::floor_went)):
    /// she waits for the layers' models even with ground far below.
    floor_gone: bool,
    /// Set as she lands after her floor went (taken by the log of what
    /// stands there).
    pub floor_landed: bool,
    pub model: Option<(GpuModel, Rig)>,
    /// The vehicle she is driving (index into the scene's vehicles).
    pub driving: Option<usize>,
    /// Aiming: she faces the camera's way with the rifle shouldered.
    pub aiming: bool,
    /// Her rifle, jump, throw and fall clips.
    pub clips: FightClips,
    /// Walking, running and sprinting speeds, m/s.
    pub speeds: [f32; 3],
    /// In a jump, until she has landed.
    leap: Leap,
    /// Jump was pressed while she was landing: she jumps again as the
    /// landing ends.
    jump_queued: bool,
    /// Seconds into a grenade throw, while she throws.
    pub throwing: Option<f32>,
    /// A melee bash, while she bashes: she stands, facing the camera's
    /// way.
    pub bashing: Option<Bash>,
    /// The launcher is up (not the rifle): it rides on her shoulder.
    pub launcher: bool,
    /// Down (out of health): she lies where she fell.
    pub down: bool,
    /// Sitting in the seat of the vehicle she drives, posed and placed
    /// there (`hijack::Hijacks::seat`): drawn though driving.
    pub seated: bool,
    /// Seated: model to world for her body, the vehicle's whole turn
    /// (pitch and roll as well as heading) with her clip's origin on the
    /// seat ([`model_matrix`](Self::model_matrix)).
    pub seat_frame: Option<Mat4>,
    /// Her clips for each class of weapon she can hold, a switch's and
    /// the pickup's ([`crate::held`]).
    pub moves: Moves,
    /// What the fight has in her hands (set each tick), and what her
    /// pose holds: they differ while a switch plays.
    pub in_hand: InHand,
    shown: InHand,
    /// A weapon switch, while it plays over her upper body.
    pub switching: Option<Switch>,
    /// Her binoculars, while they are up ([`crate::binoculars`]): the
    /// frame is seen through them and her body is left out.
    pub binoculars: Option<Binoculars>,
    /// Picking a weapon up: the clip and seconds in (she stands).
    pub picking: Option<(usize, f32)>,
    /// Changing the clip ([`Player::start_reload`]): the clip and seconds
    /// in, over her upper body.
    pub reloading: Option<(usize, f32)>,
    /// The scripts put her weapons away (`Human.DisableWeapons`): none is
    /// drawn or fired until `EnableWeapons`.
    pub holstered: bool,
    /// In the water, out of her depth: she swims ([`Swim`]).
    pub swim: Option<Swim>,
}

/// Swimming: how long she has been in, for the stroke's start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Swim {
    pub since: f32,
}

impl Player {
    /// Whether she swims.
    pub fn swimming(&self) -> bool {
        self.swim.is_some()
    }

    /// Whether her weapons are away: swimming, or put away by the scripts
    /// ([`holstered`](Self::holstered)). None is drawn in her hands or on
    /// her back, and nothing fires.
    pub fn weapons_away(&self) -> bool {
        self.swimming() || self.holstered
    }

    /// Swims her when the water under her is out of her depth
    /// ([`SWIM_START`]), `direction` the way she is told to go: she floats
    /// at the surface ([`SWIM_FLOAT`]), goes at [`SWIM_SPEED`] and plays
    /// the stroke, her weapon away. Where it shallows again
    /// ([`SWIM_STOP`]) she stands up with the climb-out. True while she
    /// swims: the rest of the step (running, jumping, aiming) is left
    /// out.
    ///
    /// From samples (the on-foot tables): the `Swim` stance's `EnterSwim`
    /// handle (cca02d6b) gives the looped stroke c846e6fd, which the
    /// transition table enters from any handle; `Upright` `ExitSwim`
    /// (e5653a8b) gives the climb-out 97c840ed. The depths, speed and
    /// the stroke's rate when still are invented. The water is the sea or,
    /// higher, the game's water heights there (rivers, pools, lakes:
    /// [`Physics::water_at`]).
    fn update_swim(&mut self, physics: &Physics, direction: Vec3, dt: f32) -> bool {
        let at = self.character.position;
        let Some(water) = physics.water_at(at.x, at.z) else {
            self.swim = None;
            return false;
        };
        let top = at.y.max(water) + 1.0;
        let ground = physics
            .cast_ray(at.with_y(top), Vec3::NEG_Y, 400.0)
            .map(|(d, _)| top - d);
        let depth = ground.map_or(f32::INFINITY, |g| water - g);
        match self.swim {
            None if depth > SWIM_START => {
                eprintln!("player: out of her depth ({depth:.1} m) at {at:.1}: she swims");
                self.swim = Some(Swim { since: 0.0 });
                self.aiming = false;
                self.throwing = None;
                self.bashing = None;
                self.picking = None;
                self.leap = Leap::No;
            }
            Some(_) if depth < SWIM_STOP => {
                eprintln!("player: in her depth ({depth:.1} m) at {at:.1}: she stands");
                self.swim = None;
                if let Some(g) = ground {
                    self.character.position.y = self.character.position.y.max(g);
                }
                if let (Some((_, rig)), Some(clip)) = (&mut self.model, self.clips.swim_out) {
                    rig.animator.play_once(clip);
                    self.leap = Leap::Land(0.0, clip);
                }
                return false;
            }
            _ => {}
        }
        let Some(swim) = &mut self.swim else {
            return false;
        };
        swim.since += dt;
        physics.move_character(&mut self.character, direction * SWIM_SPEED, None, dt);
        self.character.position.y = water - SWIM_FLOAT;
        self.character.velocity.y = 0.0;
        if direction.length_squared() > 0.01 {
            let target = direction.x.atan2(direction.z);
            let mut turn = (target - self.facing).rem_euclid(std::f32::consts::TAU);
            if turn > std::f32::consts::PI {
                turn -= std::f32::consts::TAU;
            }
            self.facing += turn.clamp(-TURN_RATE * dt, TURN_RATE * dt);
        }
        let moved = Vec2::new(self.character.velocity.x, self.character.velocity.z).length();
        if let Some((_, rig)) = &mut self.model {
            rig.animator.set_upper(None);
            rig.two_handed = false;
            let rate = match self.clips.swim {
                Some(clip) => {
                    rig.animator.play(clip);
                    let own = rig.clips[clip].info.speed.max(0.1);
                    (moved / own).clamp(SWIM_STILL_RATE, 2.5)
                }
                None => 1.0,
            };
            rig.advance(dt * rate);
        }
        true
    }

    /// A player who will drop onto the ground at `(x, z)` once it loads.
    pub fn new(x: f32, z: f32, yaw: f32, model: Option<(GpuModel, Rig)>) -> Self {
        Self {
            character: Character::new(Vec3::new(x, 0.0, z)),
            facing: yaw,
            yaw,
            pitch: -0.2,
            camera_distance: CAMERA_DISTANCE,
            chase_target: None,
            aim_lift: 0.0,
            look_still: 0.0,
            landed: false,
            land_from: None,
            streaming: false,
            settled: 0,
            waited: 0,
            floor_gone: false,
            floor_landed: false,
            models_coming: false,
            model,
            driving: None,
            aiming: false,
            clips: FightClips::default(),
            speeds: SPEEDS,
            leap: Leap::No,
            jump_queued: false,
            throwing: None,
            bashing: None,
            launcher: false,
            down: false,
            seated: false,
            seat_frame: None,
            moves: Moves::default(),
            in_hand: InHand::default(),
            shown: InHand::default(),
            switching: None,
            picking: None,
            reloading: None,
            binoculars: None,
            swim: None,
            holstered: false,
        }
    }

    /// The speeds of a hero whose template gives `theirs` (walk, run,
    /// sprint) where Jennifer's gives `hers`: hers scaled by each ratio.
    pub fn hero_speeds(theirs: [f32; 3], hers: [f32; 3]) -> [f32; 3] {
        let mut out = SPEEDS;
        for ((o, t), h) in out.iter_mut().zip(theirs).zip(hers) {
            if h > 0.0 {
                *o *= t / h;
            }
        }
        out
    }

    /// Takes `model` as her body, with its rig's fight `clips`: a jump,
    /// throw or bash under way played the old rig's clips by index, so it
    /// stops.
    pub fn set_body(&mut self, model: Option<(GpuModel, Rig)>, clips: FightClips, moves: Moves) {
        self.model = model;
        self.clips = clips;
        if let Some((_, rig)) = &self.model {
            self.clips.sprint = crate::npc::two_handed_sprint(rig, self.clips.run_low);
        }
        self.moves = moves;
        self.leap = Leap::No;
        self.jump_queued = false;
        self.throwing = None;
        self.bashing = None;
        self.switching = None;
        self.picking = None;
        self.reloading = None;
        // The rifle's clips are the ones just given: another class held
        // takes its own.
        self.shown = InHand::default();
        let now = self.in_hand;
        self.hold_class(now);
    }

    /// Her pose holds `held` from now: its class's fight clips and gaits
    /// ([`Moves::of`]).
    fn hold_class(&mut self, held: InHand) {
        self.shown = held;
        if let Some((_, rig)) = &mut self.model {
            rig.barrel = held.class != m2_assets::name_hash("pistol");
        }
        // The launchers keep the rifle's stance: the weapons' grip and the
        // aim's turn are worked out on the rifle's aim, and with the
        // rocket class's own (round 13) the RPG lay across her chest.
        let class = if held.class == m2_assets::name_hash("rocket") {
            m2_assets::animation_table::RIFLE
        } else {
            held.class
        };
        let Some(set) = self.moves.of(class) else {
            return;
        };
        self.clips = set.clips;
        if let Some((_, rig)) = &mut self.model
            && let Some(l) = rig.locomotion.as_mut()
        {
            l.clips.extend(set.gaits.iter().copied());
            let gait = rig.gait;
            rig.set_gait(gait);
        }
        if let Some((_, rig)) = &self.model {
            self.clips.sprint = crate::npc::two_handed_sprint(rig, self.clips.run_low);
        }
    }

    /// Starts picking a weapon up off the ground (the whole body, once:
    /// she stands till it is over), if she has the clip and is free to.
    pub fn start_pickup(&mut self) -> bool {
        let Some(clip) = self.moves.pickup else {
            return false;
        };
        if !self.can_bash() || self.picking.is_some() || self.driving.is_some() {
            return false;
        }
        let Some((_, rig)) = self.model.as_mut() else {
            return false;
        };
        rig.animator.set_upper(None);
        rig.animator.play_once(clip);
        rig.animator.seek(0.0);
        rig.two_handed = false;
        eprintln!("anim: pickup {:08x}", rig.clips[clip].hash);
        self.picking = Some((clip, 0.0));
        true
    }

    /// Leaves what she does on foot behind for a vehicle's clips (in a
    /// seat, or hijacking one), which only [`update`](Self::update)
    /// would otherwise end: the weapon's pose over her upper body (a
    /// carried rifle's arm, an aim, a throw), her aim's turn, a bash and
    /// a jump. Back on foot, `update` takes them up again.
    pub fn off_foot(&mut self) {
        self.leap = Leap::No;
        self.jump_queued = false;
        self.throwing = None;
        self.bashing = None;
        self.picking = None;
        self.reloading = None;
        // A switch under way is over: she holds what the fight has up.
        if self.switching.take().is_some() {
            let now = self.in_hand;
            self.hold_class(now);
        }
        if let Some((_, rig)) = &mut self.model {
            rig.animator.set_upper(None);
            rig.aim = None;
            rig.two_handed = false;
        }
    }

    /// Starts throwing a grenade, if she has a throw to play and isn't
    /// throwing already: the seconds until it leaves her hand. `None`
    /// without the clip (the grenade goes at once).
    pub fn start_throw(&mut self) -> Option<f32> {
        if self.throwing.is_some() {
            return None;
        }
        let (_, release) = self.clips.throw?;
        self.model.as_ref()?;
        self.throwing = Some(0.0);
        Some(release)
    }

    /// Starts changing the clip of what she holds: its class's
    /// `ReloadRun` on the move, else its `ReloadIdle` ([`crate::held`]),
    /// over her upper body. The clip's seconds, or `None` without one or
    /// without her body (nothing plays).
    pub fn start_reload(&mut self) -> Option<f32> {
        let running = self.character.velocity.with_y(0.0).length() > RUNNING_JUMP;
        let clip = self.moves.of(self.shown.class)?.reload_clip(running)?;
        let (_, rig) = self.model.as_ref()?;
        let length = rig.clips.get(clip)?.info.duration;
        eprintln!(
            "anim: reload {:08x} ({length:.2} s{})",
            rig.clips[clip].hash,
            if running { ", running" } else { "" }
        );
        self.reloading = Some((clip, 0.0));
        Some(length)
    }

    /// Whether she is free to bash: not throwing, bashing or in a jump.
    pub fn can_bash(&self) -> bool {
        self.throwing.is_none() && self.bashing.is_none() && self.leap == Leap::No
    }

    /// Starts a melee bash (`low`: at something on the ground): the
    /// seconds until the blow lands. `None` without the clip (it lands at
    /// once).
    pub fn start_bash(&mut self, low: bool) -> Option<f32> {
        let (clip, hit) = if low {
            self.clips.bash_low.or(self.clips.bash)
        } else {
            self.clips.bash
        }?;
        let (_, rig) = self.model.as_mut()?;
        rig.animator.set_upper(None);
        // From its start, even when it was the last thing played.
        let again = rig.animator.current().is_some_and(|(c, _)| c == clip);
        rig.animator.play_once(clip);
        if again {
            rig.animator.seek(0.0);
        }
        rig.two_handed = false;
        self.bashing = Some(Bash {
            time: 0.0,
            clip,
            low,
        });
        Some(hit)
    }

    /// Puts her at `at` (getting out of a vehicle, moved by the scripts),
    /// to land on what is under it once the world there has loaded: the
    /// floor or ground nearest under the spot, not the top of whatever
    /// stands over it.
    pub fn drop_at(&mut self, at: Vec3) {
        self.character.position = at;
        self.character.velocity = Vec3::ZERO;
        self.landed = false;
        self.land_from = Some(at);
        self.waited = 0;
        self.floor_gone = false;
    }

    /// The floor she stands on was taken down (a state layer went): she
    /// stays where she is until the layers' models coming in have loaded
    /// (at most [`LAND_WAIT`]), then lands on the floor nearest under her.
    pub fn floor_went(&mut self) {
        self.drop_at(self.character.position);
        self.floor_gone = true;
        // She waits standing: not held in a jump's pose (the rig doesn't
        // run while she waits).
        self.leap = Leap::No;
        if let Some((_, rig)) = &mut self.model {
            rig.set_gait(Gait::Idle);
            rig.advance(0.3);
        }
    }

    /// The height of the floor or ground under her feet, within
    /// [`FLOOR_LOOK`] metres.
    pub fn floor_under(&self, physics: &Physics) -> Option<f32> {
        let feet = self.character.position + Vec3::Y * 0.5;
        physics
            .cast_ray(feet, -Vec3::Y, FLOOR_LOOK)
            .map(|(d, _)| feet.y - d)
    }

    /// A state layer went: the floor under her was at `before` with its
    /// models and is at `after` without ([`floor_under`](Self::floor_under)).
    /// If what was under her went with it she waits where her floor was
    /// for what the next layers put there ([`floor_went`](Self::floor_went));
    /// true if so. In the air over a floor that stays (a jump, playtest
    /// 8: the pad's A that closed PmcCon001's fanfare jumped her as the
    /// villa's layers went, and she hung in the jump's pose till black),
    /// nothing changes.
    pub fn layer_went(&mut self, before: Option<f32>, after: Option<f32>) -> bool {
        let at = self.character.position.y;
        let went = match (before, after) {
            (Some(b), Some(a)) if (a - b).abs() < FLOOR_SAME => false,
            (Some(_), _) => true,
            // Nothing under her before either (put down on what hasn't
            // come yet): as she stands, nothing within a step.
            (None, a) => a.is_none_or(|a| at - a > 1.0),
        };
        if !went {
            return false;
        }
        self.floor_went();
        if let Some(b) = before.filter(|&b| at > b) {
            self.character.position.y = b + 0.05;
        }
        true
    }

    /// One fixed tick of `dt` seconds.
    pub fn update(&mut self, physics: &Physics, intent: Intent, dt: f32) {
        self.yaw += intent.look.x;
        self.pitch = (self.pitch + intent.look.y).clamp(-1.2, 0.9);

        if !self.landed {
            // Wait for the ground and whatever stands on it, then stand on
            // what is under the spot she was put at, else on the top of
            // whatever is there.
            let p = self.character.position;
            let near = |at: Vec3| {
                let from = at + Vec3::Y * LAND_ABOVE;
                physics
                    .cast_ray(from, -Vec3::Y, LAND_REACH)
                    .map(|(d, _)| from.y - d)
            };
            // Nothing under the spot yet while the layers' models still
            // come: wait for them, a while (the PMC's interior, its floor
            // 100 m over the mountain).
            self.waited = self.waited.saturating_add(1);
            let floor_coming = self.models_coming
                && self.waited < LAND_WAIT
                && (self.floor_gone || self.land_from.and_then(near).is_none());
            self.settled = if self.streaming || floor_coming {
                0
            } else {
                self.settled.saturating_add(1)
            };
            let top = || {
                physics
                    .cast_ray(Vec3::new(p.x, 3000.0, p.z), -Vec3::Y, 6000.0)
                    .map(|(d, _)| 3000.0 - d)
            };
            if self.settled >= SETTLED
                && let Some(y) = self
                    .land_from
                    .filter(|at| at.x == p.x && at.z == p.z)
                    .and_then(near)
                    .or_else(top)
            {
                if self.floor_gone {
                    self.floor_landed = true;
                    eprintln!(
                        "player: her floor gone, she lands at y {y:.1} after {} ticks \
                         (models coming {}, top {:?})",
                        self.waited,
                        self.models_coming,
                        top()
                    );
                }
                self.character.position.y = y + 0.05;
                self.landed = true;
                self.land_from = None;
                self.settled = 0;
                self.waited = 0;
                self.floor_gone = false;
            }
            self.camera(physics);
            return;
        }

        if self.down {
            physics.move_character(&mut self.character, Vec3::ZERO, None, dt);
            if let Some((_, rig)) = &mut self.model {
                rig.advance(dt);
            }
            self.camera(physics);
            return;
        }

        let (s, c) = self.yaw.sin_cos();
        let forward = Vec3::new(s, 0.0, c);
        // Right-handed, +y up: right is forward turned clockwise from above.
        let right = Vec3::new(-c, 0.0, s);
        let direction = forward * intent.movement.y + right * intent.movement.x;

        if self.update_swim(physics, direction, dt) {
            self.camera(physics);
            return;
        }
        let [walk, run, sprint] = self.speeds;
        let speed = if intent.walk {
            walk
        } else if intent.sprint {
            sprint
        } else {
            run
        };
        // Bashing or picking a weapon up, she stands.
        let bashing = self.bashing.is_some() || self.picking.is_some();
        let wanted = if bashing {
            Vec3::ZERO
        } else {
            direction * speed
        };
        let jump = self.takes_off(intent.jump, bashing).then_some(JUMP);
        physics.move_character(&mut self.character, wanted, jump, dt);

        // Aiming, throwing or bashing, she turns the camera's way.
        let heading = if self.aiming || self.throwing.is_some() || bashing {
            Some(self.yaw)
        } else {
            (direction.length_squared() > 0.01).then(|| direction.x.atan2(direction.z))
        };
        if let Some(target) = heading {
            let mut turn = (target - self.facing).rem_euclid(std::f32::consts::TAU);
            if turn > std::f32::consts::PI {
                turn -= std::f32::consts::TAU;
            }
            self.facing += turn.clamp(-TURN_RATE * dt, TURN_RATE * dt);
        }

        let moved = Vec2::new(self.character.velocity.x, self.character.velocity.z).length();
        let look = self.aim_forward();
        if let Some(t) = &mut self.throwing {
            *t += dt;
        }
        let (mut swap, mut switched) = (None, false);
        if let Some((_, rig)) = &mut self.model {
            let gait = if moved < 0.3 {
                Gait::Idle
            } else if intent.walk {
                Gait::Walk
            } else if intent.sprint {
                Gait::Sprint
            } else {
                Gait::Run
            };
            let clips = &self.clips;
            // The jump: a take-off (on the run or standing), the air,
            // and a landing.
            if jump.is_some() {
                let takeoff = if moved > RUNNING_JUMP {
                    clips.takeoff.or(clips.takeoff_still)
                } else {
                    clips.takeoff_still.or(clips.takeoff)
                };
                // How the jump before it ended, for the log: in the air,
                // landing (how far into the landing clip), or done.
                let was = match self.leap {
                    Leap::No => "on her feet".to_owned(),
                    Leap::Air(t) => format!("in the air {t:.2} s"),
                    Leap::Land(t, c) => {
                        format!("landing {t:.2} of {:.2} s", rig.clips[c].info.duration)
                    }
                };
                eprintln!("jump: take-off at {:.2} m/s, {was}", moved);
                self.leap = Leap::Air(0.0);
                if let Some(clip) = takeoff.or(clips.jump) {
                    // At the height she stands at, so the take-off's
                    // crouch doesn't sink her.
                    let hips = rig
                        .gait_clip(Gait::Idle)
                        .map_or(1.0, |i| rig.clips[i].info.hips);
                    rig.animator.play_leveled(clip, hips);
                }
            } else {
                self.leap = match self.leap {
                    Leap::Air(t) if t > MIN_AIR && self.character.grounded => {
                        let land = if moved > RUNNING_LAND {
                            clips.land.or(clips.land_still)
                        } else {
                            clips.land_still.or(clips.land)
                        };
                        match land {
                            Some(clip) => {
                                rig.animator.play_once(clip);
                                Leap::Land(0.0, clip)
                            }
                            None => Leap::No,
                        }
                    }
                    Leap::Air(t) => Leap::Air(t + dt),
                    // A standing landing gives way when she sets off.
                    Leap::Land(t, clip)
                        if t + LAND_FADE < rig.clips[clip].info.duration
                            && !(Some(clip) == clips.land_still && moved > RUNNING_JUMP) =>
                    {
                        Leap::Land(t + dt, clip)
                    }
                    Leap::Land(..) | Leap::No => Leap::No,
                };
            }
            let leaping = self.leap != Leap::No;
            // The launcher rides on her shoulder, as when she aims it.
            let carry = if self.aiming || self.launcher {
                Carry::Aim
            } else {
                Carry::Low
            };
            // A bash plays whole, once, then the gait takes over again.
            if let Some(b) = &mut self.bashing {
                b.time += dt;
                if b.time + LAND_FADE >= rig.clips[b.clip].info.duration {
                    self.bashing = None;
                }
            }
            if let Some((clip, t)) = &mut self.picking {
                *t += dt;
                if *t + LAND_FADE >= rig.clips[*clip].info.duration {
                    self.picking = None;
                    // The weapon picked up is in her hands as it ends: no
                    // switch to play (inferred).
                    swap = Some(self.in_hand);
                }
            }
            if self.bashing.is_some() || self.picking.is_some() {
                rig.two_handed = false;
            } else if leaping {
                // The legs jump; the arms keep the weapon as they were
                // (the jump's own arms fly out).
                let upper = if carry == Carry::Aim {
                    clips.aim
                } else {
                    rig.gait_clip(Gait::Idle)
                };
                rig.two_handed = carry == Carry::Aim && upper.is_some();
                rig.animator.set_upper(upper.zip(rig.upper.clone()));
            } else {
                rig.hold_rifle(gait, clips, carry);
                // A pistol is held in the right hand alone.
                if !rig.barrel {
                    rig.two_handed = false;
                }
            }
            // A weapon switch: what she holds goes away over her upper
            // body, then the next comes up ([`crate::held`]); her pose
            // takes the new class's clips between the two.
            let free = self.bashing.is_none() && self.picking.is_none() && self.throwing.is_none();
            if self.switching.is_none() && self.in_hand != self.shown && free && swap.is_none() {
                let s = Switch::new(&self.moves, self.shown, self.in_hand, gait != Gait::Idle)
                    .timed(|c| rig.clips[c].info.duration);
                let h = |c: Option<(usize, f32)>| {
                    c.map_or("none".to_owned(), |(c, _)| {
                        format!("{:08x}", rig.clips[c].hash)
                    })
                };
                eprintln!(
                    "anim: switch {:?} to {:?}: stow {}, take up {}",
                    s.from.side,
                    s.to.side,
                    h(s.stow),
                    h(s.equip)
                );
                self.switching = Some(s);
            }
            if let Some(s) = &mut self.switching {
                s.time += dt;
                if s.swapped() && self.shown != s.to {
                    swap = Some(s.to);
                }
                match (s.at(s.time), rig.upper.clone()) {
                    (Phase::Stow(c, t) | Phase::Equip(c, t), Some(mask)) => {
                        rig.animator.upper_at(c, mask, t);
                        rig.two_handed = false;
                    }
                    (Phase::Done, _) | (_, None) => switched = true,
                }
            }
            // A throw plays over the upper body, whatever the legs do.
            match (self.throwing, clips.throw, rig.upper.clone()) {
                (Some(t), Some((clip, _)), Some(mask)) if t < rig.clips[clip].info.duration => {
                    rig.animator.upper_at(clip, mask, t);
                    rig.two_handed = false;
                }
                (Some(_), ..) => self.throwing = None,
                _ => {}
            }
            // Changing the clip plays over the upper body too, where a
            // switch or a throw doesn't (playtest 8: only its sound was
            // heard).
            if let Some((clip, t)) = &mut self.reloading {
                *t += dt;
                let busy = self.switching.is_some() || self.throwing.is_some() || !free;
                match rig.upper.clone() {
                    Some(mask) if *t < rig.clips[*clip].info.duration && !busy => {
                        rig.animator.upper_at(*clip, mask, *t);
                        rig.two_handed = false;
                    }
                    _ => self.reloading = None,
                }
            }
            // Aimed, the weapon points where the crosshair does; the
            // launcher at rest points level the way she faces.
            let (sf, cf) = self.facing.sin_cos();
            let rest = (self.launcher && self.throwing.is_none()).then_some(Vec3::new(sf, 0.0, cf));
            rig.aim_along(self.facing, self.aiming.then_some(look).or(rest));
            // Play the clip at the speed she actually moves, so feet don't
            // slide (within reason: against a wall the legs keep going).
            let rate = match rig.gait_speed(gait) {
                _ if leaping => 1.0,
                Some(clip) if gait != Gait::Idle && clip > 0.1 => (moved / clip).clamp(0.6, 1.4),
                _ => 1.0,
            };
            rig.advance(dt * rate);
        }
        if let Some(to) = swap {
            self.hold_class(to);
        }
        if switched {
            self.switching = None;
        }
        self.camera(physics);
    }

    /// Whether she takes off now, jump `pressed` this tick. A jump is a
    /// whole-body action (the action table's `Upright` `JUMP` row is
    /// `FullBody`): she takes off again only once the last one is over,
    /// landing and all. Spammed, a press used to launch her again while
    /// still on the ground 0.08 s into the jump (29 take-offs in 3 s on
    /// the run), or a moment after touching down, cutting the landing
    /// short (playtest 8). A press while she lands is kept for its end.
    /// Inferred from the table and the clips; the 360 code was not read.
    fn takes_off(&mut self, pressed: bool, bashing: bool) -> bool {
        if pressed && matches!(self.leap, Leap::Land(..)) {
            self.jump_queued = true;
        }
        let go = (pressed || self.jump_queued)
            && self.leap == Leap::No
            && self.character.grounded
            && !bashing;
        if go {
            self.jump_queued = false;
        }
        go
    }

    fn look_at(&self) -> Vec3 {
        self.character.position + Vec3::Y * LOOK_HEIGHT
    }

    /// The point the camera turns about: above her head on foot, above the
    /// vehicle she drives. The crosshair's line passes through it, so a
    /// shot goes at what lies straight along the camera from here.
    pub fn pivot(&self) -> Vec3 {
        self.chase_target
            .map_or_else(|| self.look_at(), |(at, _)| at)
    }

    pub fn camera_forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, cy * cp)
    }

    /// Where the crosshair points: a little above the camera's way on
    /// foot and in a vehicle whose crosshair sits above the middle of the
    /// screen, as the game's reticle does.
    pub fn aim_forward(&self) -> Vec3 {
        let lift = if self.chase_target.is_some() {
            self.aim_lift
        } else {
            ON_FOOT_AIM_LIFT
        };
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = (self.pitch + lift).sin_cos();
        Vec3::new(sy * cp, sp, cy * cp)
    }

    /// Turns the camera so the crosshair is on `target`. The eye moves as
    /// the camera turns, so it settles in a few steps.
    pub fn aim_crosshair_at(&mut self, target: Vec3) {
        for _ in 0..4 {
            let to = target - self.eye();
            let lift = self.aim_forward().y.asin() - self.camera_forward().y.asin();
            self.yaw = to.x.atan2(to.z);
            self.pitch = (to.y / to.length().max(0.01)).asin() - lift;
        }
    }

    /// Pulls the camera in in front of anything between it and her (a
    /// ball of [`CAMERA_RADIUS`] swept back from her head, so the view
    /// doesn't cut into a pillar, a door or a wall at a slant, as a ray
    /// from her head let it: the owner's second play test), and until it
    /// is clear of the ground beneath it: a ray from her head can graze a
    /// slope rising behind her and miss it, which left the eye under the
    /// bank at sea2, looking up through the ground's back faces.
    fn camera(&mut self, physics: &Physics) {
        let back = -self.camera_forward();
        let head = self.look_at();
        let mut distance = physics
            .cast_sphere(head, back, CAMERA_DISTANCE, CAMERA_RADIUS, None)
            .map_or(CAMERA_DISTANCE, |d| d.max(CAMERA_MIN));
        // Ground (not a roof or a bridge high over it) from a little above
        // the eye to a little below.
        let below = |eye: Vec3| {
            let top = eye.y + 1.5;
            physics
                .cast_ray(
                    Vec3::new(eye.x, top, eye.z),
                    -Vec3::Y,
                    1.5 + CAMERA_CLEARANCE,
                )
                .is_some()
        };
        while distance > CAMERA_MIN && below(head + back * distance) {
            distance = (distance - 0.25).max(CAMERA_MIN);
        }
        self.camera_distance = distance;
    }

    /// Chase camera for a vehicle at `transform` rigged as `rig`, rolling
    /// at `speed` m/s: turns `look`, and (unless it follows the aim) drifts
    /// back behind the vehicle's heading once it is moving and the mouse
    /// has been still for a moment, so you can look around.
    pub fn chase(&mut self, transform: Mat4, rig: ChaseRig, look: Vec2, speed: f32, dt: f32) {
        let f = transform.transform_vector3(Vec3::Z);
        let heading = f.x.atan2(f.z);
        if self.chase_target.is_none() {
            if let Some(pitch) = rig.pitch {
                self.pitch = pitch;
            }
            // In a tank she starts looking the way it faces: the camera
            // won't swing round behind it by itself.
            if rig.follows_aim {
                self.yaw = heading;
            }
        }
        self.chase_target = Some((transform.transform_point3(rig.pivot), rig.distance));
        self.aim_lift = rig.aim_lift;
        self.yaw += look.x;
        self.pitch = (self.pitch + look.y).clamp(rig.pitch_range.0, rig.pitch_range.1);
        self.look_still = if look == Vec2::ZERO {
            self.look_still + dt
        } else {
            0.0
        };
        if !rig.follows_aim && self.look_still > CHASE_HOLD && speed.abs() > 2.0 {
            let mut turn = (heading - self.yaw).rem_euclid(std::f32::consts::TAU);
            if turn > std::f32::consts::PI {
                turn -= std::f32::consts::TAU;
            }
            // Eases in over a second after the hold.
            let ease = ((self.look_still - CHASE_HOLD) / 1.0).min(1.0);
            self.yaw += turn * (1.5 * ease * dt).min(1.0);
        }
        // She rides along, so streaming and sound follow the vehicle.
        self.character.position = transform.transform_point3(Vec3::ZERO);
        self.character.velocity = Vec3::ZERO;
    }

    /// Pulls the chase camera of the vehicle she drives (`vehicle`, which
    /// it passes through) in in front of what is between it and the point
    /// it turns about, as on foot ([`CAMERA_RADIUS`]): it went into walls
    /// and buildings beside the road (the owner's second play test).
    pub fn keep_chase_clear(&mut self, physics: &Physics, vehicle: Option<VehicleId>) {
        let Some((at, distance)) = self.chase_target else {
            return;
        };
        let back = -self.camera_forward();
        if let Some(d) = physics.cast_sphere(at, back, distance, CAMERA_RADIUS, vehicle) {
            self.chase_target = Some((at, d.max(CAMERA_MIN).min(distance)));
        }
    }

    pub fn eye(&self) -> Vec3 {
        match self.chase_target {
            Some((at, distance)) => at - self.camera_forward() * distance,
            None => self.look_at() - self.camera_forward() * self.camera_distance,
        }
    }

    /// Her camera's view; through her binoculars, from the point the
    /// camera turns about above her head, with their narrower lens.
    pub fn view(&self, fov_degrees: f32, aspect: f32) -> View {
        if let Some(b) = self.binoculars.filter(|_| self.driving.is_none()) {
            return View::perspective(
                self.look_at(),
                self.camera_forward(),
                b.fov(fov_degrees).to_radians(),
                aspect,
                0.1,
                10_000.0,
            );
        }
        View::perspective(
            self.eye(),
            self.camera_forward(),
            fov_degrees.to_radians(),
            aspect,
            0.1,
            10_000.0,
        )
    }

    /// Model to world for her model, turned so its forward is her facing;
    /// seated in what she drives, its [`seat_frame`](Self::seat_frame).
    pub fn model_matrix(&self) -> Mat4 {
        if let Some(seat) = self
            .seat_frame
            .filter(|_| self.seated && self.driving.is_some())
        {
            return seat;
        }
        let model_forward = self
            .model
            .as_ref()
            .and_then(|(_, rig)| rig.locomotion.as_ref())
            .map_or(Vec2::Y, |l| l.forward);
        let model_yaw = model_forward.x.atan2(model_forward.y);
        Mat4::from_translation(self.character.position)
            * Mat4::from_rotation_y(self.facing - model_yaw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tank_camera_turns_above_the_turret_and_follows_the_aim() {
        use m2_engine::tuning::TankCamera;
        let mut spec = VehicleSpec {
            model: 1,
            kind: "tank".into(),
            label: "amx30".into(),
            node_names: Vec::new(),
            armed: Vec::new(),
            wheels: Vec::new(),
            bounds: (Vec3::new(-1.8, 0.0, -3.3), Vec3::new(2.0, 4.2, 8.1)),
            seat: None,
            door: None,
            tracked: true,
            road_wheels: Vec::new(),
            rest: Vec::new(),
            node_wheel: Vec::new(),
            node_rolls: Vec::new(),
            mass: 32_000.0,
            turret: vec![true],
            pivot: Some(Vec3::new(0.0, 1.53, 0.55)),
            muzzle: None,
            gun: Vec::new(),
            gun_pivot: None,
            muzzle_on_gun: false,
            elevation: (0.0, 0.0),
            hull: (Vec3::new(-1.8, 0.0, -3.3), Vec3::new(1.8, 1.5, 3.3)),
            turret_box: None,
            rotors: Vec::new(),
            tilts: Vec::new(),
            bomb_bay: None,
            damage: None,
            tuning: Default::default(),
        };
        let plain = ChaseRig::of(&spec);
        assert_eq!(plain.pivot, Vec3::Y * 2.0);
        assert!(plain.follows_aim, "a tank's camera stays on the aim");
        spec.tuning.camera = Some(TankCamera {
            distance: 11.5,
            height: 1.4,
            pitch: -0.15,
            pitch_range: (-0.4, 0.45),
        });
        let rig = ChaseRig::of(&spec);
        assert!(
            (rig.pivot - Vec3::new(0.0, 2.93, 0.55)).length() < 1e-5,
            "{rig:?}"
        );
        assert_eq!((rig.distance, rig.pitch), (11.5, Some(-0.15)));
        assert!(rig.follows_aim && rig.aim_lift > 0.0);
        // Getting in: she looks the way it faces, a little down, and the
        // crosshair sits above the camera's line.
        let mut p = Player::new(0.0, 0.0, 0.0, None);
        let turned = Mat4::from_rotation_y(2.0);
        p.chase(turned, rig, Vec2::ZERO, 0.0, 1.0 / 60.0);
        assert!((p.yaw - 2.0).abs() < 1e-5, "{}", p.yaw);
        assert!((p.pitch + 0.15).abs() < 1e-5);
        assert!(p.aim_forward().y - p.camera_forward().y > 0.1);
        // The mouse turns it, and it stays turned.
        for _ in 0..120 {
            p.chase(turned, rig, Vec2::new(0.005, 0.0), 10.0, 1.0 / 60.0);
        }
        assert!((p.yaw - 2.6).abs() < 1e-3, "{}", p.yaw);
        // On foot again, the crosshair sits a little higher still.
        p.chase_target = None;
        let lift = p.aim_forward().y.asin() - p.camera_forward().y.asin();
        assert!((lift - ON_FOOT_AIM_LIFT).abs() < 1e-4, "{lift}");
    }
    use m2_engine::{CollisionShape, StaticKey};
    use m2_math::glam::Quat;

    fn ground() -> Physics {
        let mut p = Physics::new();
        let h = 100.0;
        let v = [
            Vec3::new(-h, 0.0, -h),
            Vec3::new(h, 0.0, -h),
            Vec3::new(h, 0.0, h),
            Vec3::new(-h, 0.0, h),
        ];
        let shape = CollisionShape::mesh(&v, &[[0, 1, 2], [0, 2, 3]]).unwrap();
        p.add_static(
            StaticKey::Cell(1),
            &shape,
            Vec3::new(0.0, 10.0, 0.0),
            Quat::IDENTITY,
        );
        p.step(1.0 / 60.0);
        p
    }

    #[test]
    fn lands_then_runs_where_the_camera_faces() {
        let physics = ground();
        // Camera facing +x.
        let mut p = Player::new(0.0, 0.0, std::f32::consts::FRAC_PI_2, None);
        let dt = 1.0 / 60.0;
        // A few quiet ticks, then she lands.
        for _ in 0..usize::from(SETTLED) {
            assert!(!p.landed);
            p.update(&physics, Intent::default(), dt);
        }
        assert!(p.landed);
        assert!((p.character.position.y - 10.0).abs() < 0.2);
        let forward = Intent {
            movement: Vec2::Y,
            ..Intent::default()
        };
        for _ in 0..60 {
            p.update(&physics, forward, dt);
        }
        let at = p.character.position;
        assert!((at.x - RUN).abs() < 0.6, "{at}");
        assert!(at.z.abs() < 0.1, "{at}");
        assert!((p.facing - std::f32::consts::FRAC_PI_2).abs() < 1e-3);
        // The camera is behind her and above.
        let eye = p.eye();
        assert!(eye.x < at.x && eye.y > at.y, "{eye}");
    }

    // Playtest 8: the pad's A that closed the contract's fanfare jumped
    // her as the villa's layers went, and the 1.5 m look under her feet
    // found nothing: she hung in the jump's pose until black. In the air
    // over a floor that stays she comes down; if it went, she waits where
    // it was, standing.
    #[test]
    fn a_layer_going_while_she_jumps_over_her_floor_leaves_her_jumping() {
        let physics = ground();
        let mut p = Player::new(0.0, 0.0, 0.0, None);
        let dt = 1.0 / 60.0;
        for _ in 0..usize::from(SETTLED) {
            p.update(&physics, Intent::default(), dt);
        }
        assert!(p.landed);
        let floor = p.floor_under(&physics).unwrap();
        p.character.position.y = floor + 1.6;
        let before = p.floor_under(&physics);
        assert!(!p.layer_went(before, p.floor_under(&physics)));
        assert!(p.landed);
        for _ in 0..90 {
            p.update(&physics, Intent::default(), dt);
        }
        assert!((p.character.position.y - floor).abs() < 0.2, "came down");
        p.character.position.y = floor + 1.6;
        assert!(p.layer_went(Some(floor), None));
        assert!(!p.landed);
        assert!((p.character.position.y - floor).abs() < 0.1, "waits on it");
    }

    #[test]
    fn out_of_her_depth_she_swims_at_the_surface_and_stands_where_it_shallows() {
        // The ground at y 10, the sea at 12: 2 m deep, out of her depth.
        let mut physics = ground();
        physics.set_water(12.0);
        let mut p = Player::new(0.0, 0.0, std::f32::consts::FRAC_PI_2, None);
        let dt = 1.0 / 60.0;
        for _ in 0..usize::from(SETTLED) {
            p.update(&physics, Intent::default(), dt);
        }
        assert!(p.landed);
        let forward = Intent {
            movement: Vec2::Y,
            sprint: true,
            ..Intent::default()
        };
        for _ in 0..60 {
            p.update(&physics, forward, dt);
        }
        assert!(p.swimming());
        let at = p.character.position;
        assert!((at.y - (12.0 - SWIM_FLOAT)).abs() < 1e-3, "{at}");
        // At swimming speed, not running (the sprint is not hers here).
        assert!((at.x - SWIM_SPEED).abs() < 0.4, "{at}");
        // The sea goes down to 1 m over the ground: she stands.
        physics.set_water(11.0);
        p.update(&physics, forward, dt);
        assert!(!p.swimming());
    }

    #[test]
    fn she_swims_in_a_river_or_pool_the_water_heights_give() {
        // No sea; a pool over the ground's middle (x, z -16..16), its
        // surface 2 m over the ground at y 10.
        let mut physics = ground();
        physics.set_inland(std::sync::Arc::new(m2_assets::water::WaterHeights {
            columns: 2,
            rows: 2,
            cell: 32.0,
            origin: [-16.0, -16.0],
            heights: vec![12.0; 4],
            wet: Vec::new(),
        }));
        let mut p = Player::new(0.0, 0.0, std::f32::consts::FRAC_PI_2, None);
        let dt = 1.0 / 60.0;
        for _ in 0..usize::from(SETTLED) {
            p.update(&physics, Intent::default(), dt);
        }
        let forward = Intent {
            movement: Vec2::Y,
            ..Intent::default()
        };
        p.update(&physics, forward, dt);
        assert!(p.swimming());
        assert!((p.character.position.y - (12.0 - SWIM_FLOAT)).abs() < 1e-3);
    }

    // A pillar 2 m behind her, its side 0.1 m off the line from her head
    // back to the eye: a ray passes it by, the camera's ball doesn't, and
    // the eye stops in front of it, not inside it.
    #[test]
    fn the_camera_stops_in_front_of_a_pillar_beside_its_line() {
        let mut physics = ground();
        let pillar = CollisionShape::cuboid(Vec3::new(0.3, 3.0, 0.3));
        // Camera facing +z: the eye 4 m back along -z, the pillar's side
        // at x = 0.1.
        physics.add_static(
            StaticKey::Cell(2),
            &pillar,
            Vec3::new(0.4, 13.0, -2.0),
            Quat::IDENTITY,
        );
        physics.step(1.0 / 60.0);
        let mut p = Player::new(0.0, 0.0, 0.0, None);
        let dt = 1.0 / 60.0;
        for _ in 0..usize::from(SETTLED) + 1 {
            p.update(&physics, Intent::default(), dt);
        }
        assert!(p.landed);
        p.pitch = 0.0;
        p.camera(&physics);
        let head = p.look_at();
        assert!(
            physics
                .cast_ray(head, Vec3::NEG_Z, CAMERA_DISTANCE)
                .is_none(),
            "the ray passes beside it"
        );
        let eye = p.eye();
        assert!(eye.z > -1.7, "in front of it: {eye}");
        assert!(eye.z < -1.0, "pulled in only to the pillar: {eye}");
    }

    #[test]
    fn put_down_she_lands_under_the_spot_not_on_the_roof_over_it() {
        // The ground at 10 m, a roof over it at 20 m.
        let mut physics = ground();
        let h = 5.0;
        let v = [
            Vec3::new(-h, 0.0, -h),
            Vec3::new(h, 0.0, -h),
            Vec3::new(h, 0.0, h),
            Vec3::new(-h, 0.0, h),
        ];
        let roof = CollisionShape::mesh(&v, &[[0, 1, 2], [0, 2, 3]]).unwrap();
        physics.add_static(
            StaticKey::Cell(2),
            &roof,
            Vec3::new(0.0, 20.0, 0.0),
            Quat::IDENTITY,
        );
        physics.step(1.0 / 60.0);
        let dt = 1.0 / 60.0;
        let land = |p: &mut Player, physics: &Physics| {
            for _ in 0..usize::from(SETTLED) {
                p.update(physics, Intent::default(), dt);
            }
            assert!(p.landed);
            p.character.position.y
        };
        // A spawn's x and z alone: the top of what is there.
        let mut p = Player::new(1.0, 1.0, 0.0, None);
        assert!((land(&mut p, &physics) - 20.0).abs() < 0.2);
        // Put down at the ground's height: the ground under the roof.
        p.drop_at(Vec3::new(1.0, 10.5, 1.0));
        assert!((land(&mut p, &physics) - 10.0).abs() < 0.2);
        // Nothing near under the spot: the top again.
        p.drop_at(Vec3::new(1.0, 200.0, 1.0));
        assert!((land(&mut p, &physics) - 20.0).abs() < 0.2);
        // Moved since she was put down: where she is now decides.
        p.drop_at(Vec3::new(1.0, 10.5, 1.0));
        p.character.position = Vec3::new(30.0, 0.0, 30.0);
        assert!((land(&mut p, &physics) - 10.0).abs() < 0.2);

        // On the roof, it is taken down (a state layer went) while the
        // next layer's models load: she stays up, though the ground is
        // there below, and lands on the floor they bring.
        p.drop_at(Vec3::new(1.0, 20.5, 1.0));
        assert!((land(&mut p, &physics) - 20.0).abs() < 0.2);
        physics.remove_static(StaticKey::Cell(2));
        physics.add_static(
            StaticKey::Cell(3),
            &roof,
            Vec3::new(0.0, 19.0, 0.0),
            Quat::IDENTITY,
        );
        physics.remove_static(StaticKey::Cell(3));
        physics.step(dt);
        p.floor_went();
        p.models_coming = true;
        for _ in 0..30 {
            p.update(&physics, Intent::default(), dt);
        }
        assert!(!p.landed);
        assert!((p.character.position.y - 20.0).abs() < 0.2, "she waited");
        physics.add_static(
            StaticKey::Cell(3),
            &roof,
            Vec3::new(0.0, 19.0, 0.0),
            Quat::IDENTITY,
        );
        physics.step(dt);
        p.models_coming = false;
        assert!((land(&mut p, &physics) - 19.0).abs() < 0.2);
    }

    #[test]
    fn model_turns_to_face_her_heading() {
        let mut p = Player::new(0.0, 0.0, 0.0, None);
        p.facing = std::f32::consts::FRAC_PI_2;
        // Without a rig the model is taken to face +z.
        let forward = p.model_matrix().transform_vector3(Vec3::Z);
        assert!((forward - Vec3::X).length() < 1e-5, "{forward}");
        // Seated, she leans with the vehicle (here pitched nose down);
        // out of it, she stands up straight again.
        let tilted = Mat4::from_rotation_x(0.3) * Mat4::from_translation(Vec3::Y);
        p.seat_frame = Some(tilted);
        p.seated = true;
        p.driving = Some(0);
        assert_eq!(p.model_matrix(), tilted);
        p.driving = None;
        let up = p.model_matrix().transform_vector3(Vec3::Y);
        assert!((up - Vec3::Y).length() < 1e-5, "{up}");
    }

    #[test]
    fn a_spammed_jump_waits_for_the_last_landing() {
        let mut p = Player::new(0.0, 0.0, 0.0, None);
        p.character.grounded = true;
        assert!(p.takes_off(true, false), "on her feet she jumps");
        // Still touching the ground just after take-off: no second launch.
        p.leap = Leap::Air(0.08);
        assert!(!p.takes_off(true, false));
        // Landing: the press waits for the landing's end, then she goes.
        p.leap = Leap::Land(0.0, 0);
        assert!(!p.takes_off(true, false));
        assert!(!p.takes_off(false, false));
        p.leap = Leap::No;
        assert!(p.takes_off(false, false), "the kept press jumps");
        assert!(!p.takes_off(false, false), "once");
        // Bashing or in the air off the ground, never.
        assert!(!p.takes_off(true, true));
        p.character.grounded = false;
        assert!(!p.takes_off(true, false));
    }

    #[test]
    fn getting_in_leaves_her_aim_and_carry_behind() {
        let mut rig = crate::npc::tests::rig();
        rig.animator.set_upper(Some((0, vec![true].into())));
        rig.aim = Some(Vec3::X);
        rig.two_handed = true;
        let mut p = Player::new(0.0, 0.0, 0.0, None);
        p.model = Some((crate::npc::tests::empty_model(), rig));
        p.throwing = Some(0.2);
        p.off_foot();
        let (_, rig) = p.model.as_ref().unwrap();
        assert_eq!(rig.animator.upper(), None);
        assert_eq!((rig.aim, rig.two_handed), (None, false));
        assert_eq!((p.throwing, p.bashing, p.leap), (None, None, Leap::No));
    }
}
