//! Collision and movement on Rapier.
//!
//! The island's ground and placed models are static triangle-mesh
//! colliders. A model's collision comes from its `PHY2` shapes where it has
//! them, and from its visible triangles otherwise. Shapes are built on the
//! loader thread ([`CollisionShape`] is cheap to clone and share), so adding
//! them on the main thread is quick.
//!
//! People move with Rapier's kinematic character controller: a capsule that
//! slides along walls, climbs steps up to [`STEP_HEIGHT`] and slopes up to
//! [`MAX_SLOPE`], and keeps to the ground going down. Gravity and jumping
//! are integrated here, not by the solver.
//!
//! # Provenance
//!
//! Clean-room. The step height and slope limit are the viewer's values,
//! chosen to feel right, not read from the game.

use std::collections::HashMap;
use std::fmt;

use m2_assets::Model;
use m2_assets::model::GeometryKind;
use m2_assets::physics::Physics as Phy2;
use m2_assets::vertex::DeclUsage;
use m2_math::glam::{Mat4, Quat, Vec3};
use rapier3d::control::{
    CharacterAutostep, CharacterLength, DynamicRayCastVehicleController,
    KinematicCharacterController, WheelTuning,
};

use crate::vehicle::VehicleSpec;
use rapier3d::prelude as r;

/// Highest step a character walks up without jumping, metres.
pub const STEP_HEIGHT: f32 = 0.5;
/// Steepest slope a character walks up, radians (50 degrees).
pub const MAX_SLOPE: f32 = 50.0 * std::f32::consts::PI / 180.0;
/// Downward acceleration, m/s².
pub const GRAVITY: f32 = 9.81;

fn vector(v: Vec3) -> r::Vector {
    r::Vector::from_array(v.to_array())
}

fn vec3(v: r::Vector) -> Vec3 {
    Vec3::from_array(v.to_array())
}

fn pose(position: Vec3, rotation: Quat) -> r::Pose {
    r::Pose::from_parts(
        vector(position),
        r::Rotation::from_array(rotation.to_array()),
    )
}

/// A static collision shape, shared between every place it is used.
#[derive(Clone)]
pub struct CollisionShape {
    shape: r::SharedShape,
    pub triangles: usize,
}

impl fmt::Debug for CollisionShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CollisionShape")
            .field("triangles", &self.triangles)
            .finish()
    }
}

impl CollisionShape {
    /// A triangle mesh; `None` if it has no valid triangle.
    pub fn mesh(vertices: &[Vec3], triangles: &[[u32; 3]]) -> Option<Self> {
        let n = vertices.len() as u32;
        let triangles: Vec<[u32; 3]> = triangles
            .iter()
            .copied()
            .filter(|t| t.iter().all(|&i| i < n) && t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
            .collect();
        if triangles.is_empty() || vertices.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let count = triangles.len();
        let shape =
            r::SharedShape::trimesh(vertices.iter().map(|&v| vector(v)).collect(), triangles)
                .ok()?;
        Some(Self {
            shape,
            triangles: count,
        })
    }

    /// A model's collision in model space: its `PHY2` shapes if `phy2` is
    /// given and parses, else its rigid visible geometry at `lod`.
    pub fn for_model(model: &Model<'_>, phy2: Option<&[u8]>, lod: u32) -> Option<Self> {
        Self::for_model_part(model, phy2, lod, None)
    }

    /// One sub-object of a merged batch (a `TINY` model), by the index its
    /// vertices carry in their position's `w`: its visible geometry at
    /// `lod`, in model space.
    pub fn for_sub_object(model: &Model<'_>, index: u32, lod: u32) -> Option<Self> {
        Self::for_model_part(model, None, lod, Some(index))
    }

    fn for_model_part(
        model: &Model<'_>,
        phy2: Option<&[u8]>,
        lod: u32,
        sub_object: Option<u32>,
    ) -> Option<Self> {
        let mut vertices = Vec::new();
        let mut triangles = Vec::new();
        if let Some(p) = phy2.and_then(|d| Phy2::parse(d).ok()) {
            for (k, s) in p.shapes.iter().enumerate() {
                // A shape is in its body node's space.
                let to_model = p
                    .shape_node(k)
                    .and_then(|n| u16::try_from(n).ok())
                    .and_then(|n| model.node_to_model(n))
                    .unwrap_or(Mat4::IDENTITY);
                let (v, t) = s.geometry.triangles(s.radius);
                let first = vertices.len() as u32;
                vertices.extend(v.iter().map(|&x| to_model.transform_point3(Vec3::from(x))));
                triangles.extend(t.iter().map(|t| t.map(|i| first + i)));
            }
        }
        if triangles.is_empty() {
            let lod = (lod..model.info.lod_count.max(lod + 1))
                .find(|&l| model.has_lod(l))
                .unwrap_or(lod);
            for (segment, geometry) in model.lod(lod) {
                if geometry.kind == GeometryKind::Skin {
                    continue;
                }
                let local = model.node_to_model(segment.node).unwrap_or(Mat4::IDENTITY);
                for group in &geometry.groups {
                    let Some(pos) = group
                        .decl
                        .find(DeclUsage::Position, 0)
                        .and_then(|e| group.vertices.read(e))
                    else {
                        continue;
                    };
                    let first = vertices.len() as u32;
                    vertices.extend(
                        pos.iter()
                            .map(|p| local.transform_point3(Vec3::new(p[0], p[1], p[2]))),
                    );
                    // A batch sub-object is the triangles whose vertices
                    // carry its index.
                    let mine = |i: u16| {
                        sub_object.is_none_or(|index| {
                            pos.get(usize::from(i))
                                .is_some_and(|p| (p[3] - index as f32).abs() < 0.5)
                        })
                    };
                    for range in &group.material_ranges {
                        triangles.extend(
                            group
                                .triangles(range)
                                .into_iter()
                                .filter(|t| t.iter().all(|&i| mine(i)))
                                .map(|t| t.map(|i| first + u32::from(i))),
                        );
                    }
                }
            }
        }
        Self::mesh(&vertices, &triangles)
    }
}

/// What a static collider belongs to, so it can be removed with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StaticKey {
    /// A terrain cell, by asset hash.
    Cell(u32),
    /// A placed object, by index in the world index.
    Object(usize),
    /// A building of a placed batch: the object's index, then the
    /// building's in the batch.
    Member(usize, usize),
}

/// Suspension rest length and travel, metres.
const SUSPENSION: f32 = 0.3;
/// Steering lock at a standstill, radians.
const MAX_STEER: f32 = 0.6;
/// Braking with the throttle against the motion, the handbrake (on the
/// wheels that don't steer), rolling with no throttle, and holding still,
/// m/s² (as if all wheels braked so).
const SERVICE_BRAKE: f32 = 8.0;
const HANDBRAKE: f32 = 9.0;
const ROLLING: f32 = 0.6;
const HOLD: f32 = 3.0;
/// How far a ragdoll's limb may drift off its joint before it is put
/// back, metres.
const RAGDOLL_SLACK: f32 = 0.02;
/// How quickly a wreck stops sliding and spinning (invented).
const WRECK_DAMPING: f32 = 1.5;
/// How fast tracks turn a hull standing still, radians per second.
const TRACK_TURN: f32 = 0.9;

/// A vehicle in the physics world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VehicleId(usize);

/// What the driver does.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehicleControls {
    pub throttle: f32,
    pub steer: f32,
    pub brake: bool,
}

struct VehicleBody {
    controller: DynamicRayCastVehicleController,
    mass: f32,
    /// Which wheels steer (the front axle), in wheel order.
    steers: Vec<bool>,
    /// On tracks: turns on the spot instead of steering wheels.
    tracked: bool,
}

/// A character's capsule and motion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Character {
    /// Feet: the bottom of the capsule.
    pub position: Vec3,
    pub velocity: Vec3,
    pub grounded: bool,
    pub radius: f32,
    /// Total height, feet to top of head.
    pub height: f32,
}

impl Character {
    /// A person-sized capsule standing at `position`.
    pub fn new(position: Vec3) -> Self {
        Self {
            position,
            velocity: Vec3::ZERO,
            grounded: false,
            radius: 0.35,
            height: 1.8,
        }
    }

    fn half_height(&self) -> f32 {
        (self.height * 0.5 - self.radius).max(0.0)
    }

    fn centre(&self) -> Vec3 {
        self.position + Vec3::Y * (self.height * 0.5)
    }
}

/// The collision group of ragdolls' limbs: they land on the world and
/// vehicles but not on each other, and people, wheels and rays pass
/// through them.
const RAGDOLL: r::Group = r::Group::GROUP_2;

/// Queries that see everything but ragdolls.
fn solid<'a>() -> r::QueryFilter<'a> {
    r::QueryFilter::default().groups(r::InteractionGroups::new(
        r::Group::ALL,
        r::Group::ALL.difference(RAGDOLL),
        r::InteractionTestMode::And,
    ))
}

/// Shortest bone a ragdoll gives a limb, metres (fingers and face bones
/// are shorter and ride along).
const RAGDOLL_BONE: (f32, f32) = (0.07, 0.7);

/// A ragdoll in the physics world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RagdollId(usize);

/// A body gone limp: a rigid body per limb, joined where the skeleton's
/// bones meet; nodes without one keep their place under their parent as
/// it was when it went limp.
struct Ragdoll {
    parents: Vec<Option<usize>>,
    /// Each node's transform from its parent (world for the roots) when
    /// it went limp.
    locals: Vec<Mat4>,
    bodies: Vec<Option<r::RigidBodyHandle>>,
    /// The roots follow the first limb: (root, limb, root from limb).
    roots: Vec<(usize, usize, Mat4)>,
    /// Each limb's joint: (limb, the limb it hangs from, where in that
    /// limb), parents first.
    joints: Vec<(usize, usize, Vec3)>,
}

/// The physics world.
pub struct Physics {
    world: r::PhysicsWorld,
    statics: HashMap<StaticKey, r::ColliderHandle>,
    controller: KinematicCharacterController,
    vehicles: Vec<Option<VehicleBody>>,
    ragdolls: Vec<Option<Ragdoll>>,
}

impl fmt::Debug for Physics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Physics")
            .field("statics", &self.statics.len())
            .finish()
    }
}

impl Default for Physics {
    fn default() -> Self {
        Self::new()
    }
}

impl Physics {
    pub fn new() -> Self {
        let controller = KinematicCharacterController {
            autostep: Some(CharacterAutostep {
                max_height: CharacterLength::Absolute(STEP_HEIGHT),
                min_width: CharacterLength::Absolute(0.2),
                include_dynamic_bodies: false,
            }),
            max_slope_climb_angle: MAX_SLOPE,
            min_slope_slide_angle: MAX_SLOPE,
            snap_to_ground: Some(CharacterLength::Absolute(0.3)),
            offset: CharacterLength::Absolute(0.02),
            ..KinematicCharacterController::default()
        };
        Self {
            world: r::PhysicsWorld::default(),
            statics: HashMap::new(),
            controller,
            vehicles: Vec::new(),
            ragdolls: Vec::new(),
        }
    }

    /// Adds `shape` at `position` and `rotation` under `key`, replacing
    /// whatever `key` had.
    pub fn add_static(
        &mut self,
        key: StaticKey,
        shape: &CollisionShape,
        position: Vec3,
        rotation: Quat,
    ) {
        self.remove_static(key);
        let collider = r::ColliderBuilder::new(shape.shape.clone())
            .position(pose(position, rotation))
            .build();
        let handle = self.world.insert_collider(collider, None);
        self.statics.insert(key, handle);
    }

    pub fn remove_static(&mut self, key: StaticKey) {
        if let Some(h) = self.statics.remove(&key) {
            self.world.remove_collider(h);
        }
    }

    pub fn has_static(&self, key: StaticKey) -> bool {
        self.statics.contains_key(&key)
    }

    pub fn static_count(&self) -> usize {
        self.statics.len()
    }

    /// Advances the simulation by `dt` seconds (and brings colliders added
    /// since the last step into scene queries).
    pub fn step(&mut self, dt: f32) {
        self.world.integration_parameters.dt = dt;
        let w = &mut self.world;
        for v in self.vehicles.iter_mut().flatten() {
            let filter = solid().exclude_rigid_body(v.controller.chassis);
            let queries = w.broad_phase.as_query_pipeline_mut(
                w.narrow_phase.query_dispatcher(),
                &mut w.bodies,
                &mut w.colliders,
                filter,
            );
            v.controller.update_vehicle(dt, queries);
        }
        self.world.step();
        self.hold_ragdolls_together();
    }

    /// Puts each limb back on its joint where the solver let it drift (a
    /// hard landing or a car can pull joints apart), and takes away the
    /// speed it was drifting off with.
    fn hold_ragdolls_together(&mut self) {
        for doll in self.ragdolls.iter().flatten() {
            for &(limb, on, anchor) in &doll.joints {
                let (Some(lh), Some(oh)) = (doll.bodies[limb], doll.bodies[on]) else {
                    continue;
                };
                let Some(parent) = self.world.bodies.get(oh) else {
                    continue;
                };
                let p = parent.position();
                let want = vec3(p.translation) + Quat::from_array(p.rotation.to_array()) * anchor;
                let parent_v = vec3(parent.linvel());
                let Some(b) = self.world.bodies.get_mut(lh) else {
                    continue;
                };
                let here = vec3(b.translation());
                let off = here - want;
                if off.length() < RAGDOLL_SLACK {
                    continue;
                }
                b.set_translation(vector(want), true);
                let dir = off.normalize();
                let v = vec3(b.linvel());
                let apart = (v - parent_v).dot(dir);
                if apart > 0.0 {
                    b.set_linvel(vector(v - dir * apart), true);
                }
            }
        }
    }

    /// Adds a vehicle built from `spec`, its wheels resting on the ground
    /// at `position` (the model's origin) facing `yaw` (radians clockwise
    /// from `+z`).
    pub fn add_vehicle(&mut self, spec: &VehicleSpec, position: Vec3, yaw: f32) -> VehicleId {
        let (lo, hi) = spec.bounds;
        // The hull clears the ground by the wheels' lower half.
        let floor = spec
            .wheels
            .iter()
            .map(|w| w.centre.y)
            .fold(lo.y, f32::max)
            .min(hi.y - 0.2);
        let half = Vec3::new(
            (hi.x - lo.x) * 0.5,
            (hi.y - floor) * 0.5,
            (hi.z - lo.z) * 0.5,
        )
        .max(Vec3::splat(0.1));
        let centre = Vec3::new((lo.x + hi.x) * 0.5, floor + half.y, (lo.z + hi.z) * 0.5);
        let body = self.world.insert_body(
            r::RigidBodyBuilder::dynamic()
                .pose(pose(position, Quat::from_rotation_y(yaw)))
                .linear_damping(0.1)
                .angular_damping(0.5)
                .can_sleep(false),
        );
        // Mass low in the hull (as an engine and axles put it) and inertia
        // a little above a solid box's, so it takes a steep slope or a
        // hard turn without rolling over.
        let m = spec.mass;
        let size = half * 2.0;
        let inertia = Vec3::new(
            size.y * size.y + size.z * size.z,
            size.x * size.x + size.z * size.z,
            size.x * size.x + size.y * size.y,
        ) * (m / 12.0 * 1.5);
        let collider = r::ColliderBuilder::cuboid(half.x, half.y, half.z)
            .translation(vector(centre))
            .mass_properties(r::MassProperties::new(
                vector(Vec3::new(0.0, -half.y * 0.8, 0.0)),
                m,
                vector(inertia),
            ))
            .friction(0.6);
        self.world.insert_collider(collider, Some(body));

        let mut controller = DynamicRayCastVehicleController::new(body);
        controller.index_up_axis = 1;
        controller.index_forward_axis = 2;
        let tuning = WheelTuning {
            suspension_stiffness: 40.0,
            suspension_compression: 2.0,
            suspension_damping: 3.0,
            max_suspension_travel: SUSPENSION,
            side_friction_stiffness: 1.0,
            friction_slip: 2.0,
            max_suspension_force: spec.mass * 40.0,
        };
        for w in &spec.wheels {
            controller.add_wheel(
                vector(w.centre + Vec3::Y * SUSPENSION),
                -r::Vector::Y,
                // Forward = up x axle = +z.
                -r::Vector::X,
                SUSPENSION,
                w.radius,
                &tuning,
            );
        }
        let id = VehicleId(self.vehicles.len());
        self.vehicles.push(Some(VehicleBody {
            controller,
            mass: spec.mass,
            steers: spec.wheels.iter().map(|w| w.steers).collect(),
            tracked: spec.tracked,
        }));
        id
    }

    pub fn remove_vehicle(&mut self, id: VehicleId) {
        if let Some(v) = self.vehicles.get_mut(id.0).and_then(Option::take) {
            self.world
                .remove_body_with_colliders(v.controller.chassis, true);
        }
    }

    /// Sets a vehicle's throttle (-1 reverse to 1 full ahead), steering
    /// (-1 left to 1 right) and handbrake. Throttle against the way the
    /// vehicle rolls brakes it; with no throttle it rolls to a stop.
    pub fn drive(&mut self, id: VehicleId, controls: VehicleControls) {
        let dt = self.world.integration_parameters.dt.max(1e-3);
        let Some(v) = self.vehicles.get_mut(id.0).and_then(Option::as_mut) else {
            return;
        };
        let speed = v.controller.current_vehicle_speed;
        let wheels = v.controller.wheels().len().max(1) as f32;
        // A wheel's brake is the most impulse it may take off per step:
        // this many m/s² over the whole vehicle.
        let slowing = |decel: f32| v.mass * decel * dt / wheels;
        let throttle = controls.throttle.clamp(-1.0, 1.0);
        let against = speed.abs() > 0.5 && throttle * speed < 0.0;
        // Accelerates at about 4 m/s² flat out, slower near top speed.
        let top = if throttle >= 0.0 { 30.0 } else { 8.0 };
        let headroom = (1.0 - speed.abs() / top).clamp(0.0, 1.0);
        let (force, brake) = if controls.brake {
            (0.0, slowing(ROLLING))
        } else if against {
            (0.0, slowing(SERVICE_BRAKE * throttle.abs()))
        } else if throttle != 0.0 {
            (throttle * v.mass * 4.0 * headroom / wheels, 0.0)
        } else if speed.abs() < 1.0 {
            (0.0, slowing(HOLD))
        } else {
            (0.0, slowing(ROLLING))
        };
        let steer = controls.steer.clamp(-1.0, 1.0) * MAX_STEER / (1.0 + speed.abs() * 0.04);
        let steer = if v.tracked { 0.0 } else { steer };
        let tracked = v
            .tracked
            .then(|| (v.controller.chassis, controls.steer.clamp(-1.0, 1.0)));
        // Only the front axle steers: steering every wheel the same way
        // slides the vehicle sideways instead of turning it. The handbrake
        // locks the rest, so the back can slide out.
        for (i, w) in v.controller.wheels_mut().iter_mut().enumerate() {
            let front = v.steers.get(i).copied().unwrap_or(false);
            w.engine_force = force;
            w.steering = if front { steer } else { 0.0 };
            w.brake = if controls.brake && !front {
                slowing(HANDBRAKE)
            } else {
                brake
            };
        }
        // Tracks turn the hull on the spot (one track forward, the other
        // back), so the yaw is set directly; a little slower on the move.
        if let Some((chassis, turn)) = tracked
            && let Some(body) = self.world.bodies.get_mut(chassis)
        {
            let w = body.angvel();
            let rate = turn * TRACK_TURN / (1.0 + speed.abs() * 0.05);
            let up = body.position().rotation * r::Vector::Y;
            let keep = w - up * w.dot(up);
            body.set_angvel(keep + up * rate, true);
        }
    }

    /// Model-to-world transform of a vehicle.
    pub fn vehicle_transform(&self, id: VehicleId) -> Option<Mat4> {
        let v = self.vehicles.get(id.0)?.as_ref()?;
        let body = self.world.bodies.get(v.controller.chassis)?;
        let p = body.position();
        Some(Mat4::from_rotation_translation(
            Quat::from_array(p.rotation.to_array()),
            vec3(p.translation),
        ))
    }

    /// Each wheel's steering angle and spin, radians, in spec order.
    pub fn vehicle_wheels(&self, id: VehicleId) -> Vec<(f32, f32)> {
        self.vehicles
            .get(id.0)
            .and_then(Option::as_ref)
            .map(|v| {
                v.controller
                    .wheels()
                    .iter()
                    .map(|w| (w.steering, w.rotation))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A vehicle's velocity, m/s.
    pub fn vehicle_velocity(&self, id: VehicleId) -> Vec3 {
        self.vehicles
            .get(id.0)
            .and_then(Option::as_ref)
            .and_then(|v| self.world.bodies.get(v.controller.chassis))
            .map_or(Vec3::ZERO, |b| vec3(b.linvel()))
    }

    /// Signed forward speed, m/s.
    pub fn vehicle_speed(&self, id: VehicleId) -> f32 {
        self.vehicles
            .get(id.0)
            .and_then(Option::as_ref)
            .map_or(0.0, |v| v.controller.current_vehicle_speed)
    }

    /// Makes a vehicle a wreck: its wheels no longer roll freely and it
    /// soon stops sliding (burst tyres, bent axles).
    pub fn wreck_vehicle(&mut self, id: VehicleId) {
        let Some(v) = self.vehicles.get_mut(id.0).and_then(Option::as_mut) else {
            return;
        };
        for w in v.controller.wheels_mut() {
            w.friction_slip *= 0.5;
        }
        if let Some(body) = self.world.bodies.get_mut(v.controller.chassis) {
            body.set_linear_damping(WRECK_DAMPING);
            body.set_angular_damping(WRECK_DAMPING);
        }
    }

    /// Turns an overturned vehicle upright where it is.
    pub fn right_vehicle(&mut self, id: VehicleId) {
        let Some(v) = self.vehicles.get(id.0).and_then(Option::as_ref) else {
            return;
        };
        if let Some(body) = self.world.bodies.get_mut(v.controller.chassis) {
            let p = *body.position();
            let forward = p.rotation * r::Vector::Z;
            let yaw = forward.x.atan2(forward.z);
            let t = p.translation + r::Vector::Y * 1.5;
            body.set_position(
                r::Pose::from_parts(t, r::Rotation::from_rotation_y(yaw)),
                true,
            );
            body.set_linvel(r::Vector::ZERO, true);
            body.set_angvel(r::Vector::ZERO, true);
        }
    }

    /// Shoves a vehicle: `impulse` (kg m/s) applied at world point `at`.
    pub fn push_vehicle(&mut self, id: VehicleId, impulse: Vec3, at: Vec3) {
        let Some(v) = self.vehicles.get(id.0).and_then(Option::as_ref) else {
            return;
        };
        if let Some(body) = self.world.bodies.get_mut(v.controller.chassis) {
            body.apply_impulse_at_point(vector(impulse), vector(at), true);
        }
    }

    /// Lets a skeleton go limp: `parents` and `nodes` (world space, as
    /// posed when it went) give its bones, and every limb starts moving at
    /// `velocity`. Limbs are capsules along bones from
    /// [`RAGDOLL_BONE`]`.0` to `.1` long, joined by ball joints at the
    /// bones' ends; a bone splitting in several (hips, chest) is a ball.
    pub fn add_ragdoll(
        &mut self,
        parents: &[Option<usize>],
        nodes: &[Mat4],
        velocity: Vec3,
    ) -> RagdollId {
        let n = nodes.len().min(parents.len());
        let (lo, hi) = RAGDOLL_BONE;
        let at: Vec<(Vec3, Quat)> = nodes[..n]
            .iter()
            .map(|m| {
                let (_, q, t) = m.to_scale_rotation_translation();
                (t, q.normalize())
            })
            .collect();
        let bone = |a: usize, b: usize| {
            let d = at[a].0.distance(at[b].0);
            (lo..=hi).contains(&d)
        };
        let mut kids: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (c, parent) in parents[..n].iter().enumerate() {
            if let Some(p) = parent.filter(|&p| p < n && bone(p, c)) {
                kids[p].push(c);
            }
        }
        let groups = r::InteractionGroups::new(
            RAGDOLL,
            r::Group::ALL.difference(RAGDOLL),
            r::InteractionTestMode::And,
        );
        let mut bodies: Vec<Option<r::RigidBodyHandle>> = vec![None; n];
        let mut joints = Vec::new();
        // Parents come before their children.
        for i in 0..n {
            let Some(parent) = parents[i].filter(|&p| p < n) else {
                continue;
            };
            let limb = !kids[i].is_empty() || (bodies[parent].is_some() && bone(parent, i));
            if !limb {
                continue;
            }
            let (p, q) = at[i];
            let local = |c: usize| q.inverse() * (at[c].0 - p);
            let shape = match kids[i].as_slice() {
                [c] => {
                    let end = local(*c);
                    let radius = (0.03 + end.length() * 0.12).min(0.09);
                    r::ColliderBuilder::capsule_from_endpoints(
                        vector(end * (radius / end.length())),
                        vector(end * (1.0 - radius / end.length()).max(0.0)),
                        radius,
                    )
                }
                [] => r::ColliderBuilder::ball(0.06),
                many => {
                    let mid = many.iter().map(|&c| local(c)).sum::<Vec3>() / many.len() as f32;
                    r::ColliderBuilder::ball(0.11).translation(vector(mid))
                }
            };
            let body = self.world.insert_body(
                r::RigidBodyBuilder::dynamic()
                    .pose(pose(p, q))
                    .linvel(vector(velocity))
                    .linear_damping(0.2)
                    .angular_damping(2.0)
                    .ccd_enabled(true),
            );
            self.world.insert_collider(
                shape.density(1000.0).friction(0.9).collision_groups(groups),
                Some(body),
            );
            bodies[i] = Some(body);
            // Joined to the nearest ancestor with a limb.
            let mut up = Some(parent);
            while let Some(a) = up {
                if let Some(other) = bodies[a] {
                    let anchor = at[a].1.inverse() * (p - at[a].0);
                    joints.push((i, a, anchor));
                    self.world.insert_impulse_joint(
                        other,
                        body,
                        r::SphericalJointBuilder::new()
                            .local_anchor1(vector(anchor))
                            .local_anchor2(vector(Vec3::ZERO))
                            .contacts_enabled(false),
                    );
                    break;
                }
                up = parents[a].filter(|&p| p < n);
            }
        }
        let locals = (0..n)
            .map(|i| match parents[i].filter(|&p| p < n) {
                Some(p) => nodes[p].inverse() * nodes[i],
                None => nodes[i],
            })
            .collect();
        // Roots have no limb of their own (nor whatever hangs from them
        // without one): they go with the first limb, as they stood.
        let first = (0..n).find(|&i| bodies[i].is_some());
        let roots = first
            .map(|k| {
                let limb = Mat4::from_rotation_translation(at[k].1, at[k].0);
                (0..n)
                    .filter(|&i| parents[i].is_none_or(|p| p >= n) && bodies[i].is_none())
                    .map(|i| (i, k, limb.inverse() * nodes[i]))
                    .collect()
            })
            .unwrap_or_default();
        self.ragdolls.push(Some(Ragdoll {
            parents: parents[..n].to_vec(),
            locals,
            bodies,
            roots,
            joints,
        }));
        RagdollId(self.ragdolls.len() - 1)
    }

    /// A ragdoll's node matrices, world space; `None` once removed.
    pub fn ragdoll_nodes(&self, id: RagdollId) -> Option<Vec<Mat4>> {
        let doll = self.ragdolls.get(id.0)?.as_ref()?;
        let limb = |i: usize| {
            let b = doll.bodies[i].and_then(|h| self.world.bodies.get(h))?;
            let p = b.position();
            Some(Mat4::from_rotation_translation(
                Quat::from_array(p.rotation.to_array()),
                vec3(p.translation),
            ))
        };
        let mut out: Vec<Mat4> = Vec::with_capacity(doll.locals.len());
        for i in 0..doll.locals.len() {
            let m = match (limb(i), doll.parents[i]) {
                (Some(m), _) => m,
                (None, Some(p)) if p < i => out[p] * doll.locals[i],
                _ => match doll.roots.iter().find(|r| r.0 == i) {
                    Some(&(_, k, rel)) => limb(k).map_or(doll.locals[i], |m| m * rel),
                    None => doll.locals[i],
                },
            };
            out.push(m);
        }
        Some(out)
    }

    /// Shoves every limb of a ragdoll by `velocity` (m/s).
    pub fn push_ragdoll(&mut self, id: RagdollId, velocity: Vec3) {
        let Some(doll) = self.ragdolls.get(id.0).and_then(Option::as_ref) else {
            return;
        };
        for h in doll.bodies.iter().flatten() {
            if let Some(b) = self.world.bodies.get_mut(*h) {
                b.set_linvel(b.linvel() + vector(velocity), true);
            }
        }
    }

    pub fn remove_ragdoll(&mut self, id: RagdollId) {
        let Some(doll) = self.ragdolls.get_mut(id.0).and_then(Option::take) else {
            return;
        };
        for h in doll.bodies.into_iter().flatten() {
            self.world.remove_body(h);
        }
    }

    /// The first static surface along a ray: (distance, normal).
    pub fn cast_ray(&self, origin: Vec3, direction: Vec3, max: f32) -> Option<(f32, Vec3)> {
        self.cast_ray_keyed(origin, direction, max)
            .map(|(d, n, _)| (d, n))
    }

    /// [`cast_ray`](Self::cast_ray), with the key of the static hit (`None`
    /// for anything else solid, such as a vehicle), to find what an
    /// unseen obstacle is.
    pub fn cast_ray_keyed(
        &self,
        origin: Vec3,
        direction: Vec3,
        max: f32,
    ) -> Option<(f32, Vec3, Option<StaticKey>)> {
        let ray = r::Ray::new(vector(origin), vector(direction.normalize_or_zero()));
        let (handle, hit) = self
            .world
            .query_pipeline_with_filter(solid())
            .cast_ray_and_get_normal(&ray, max, true)?;
        let key = self
            .statics
            .iter()
            .find(|(_, h)| **h == handle)
            .map(|(k, _)| *k);
        Some((hit.time_of_impact, vec3(hit.normal), key))
    }

    /// Moves `character` for `dt` seconds: horizontally at `walk` (m/s; its
    /// y is ignored), jumping at `jump` m/s if given while grounded, falling
    /// under gravity otherwise.
    pub fn move_character(
        &self,
        character: &mut Character,
        walk: Vec3,
        jump: Option<f32>,
        dt: f32,
    ) {
        let mut vertical = character.velocity.y;
        if character.grounded {
            vertical = jump.unwrap_or(0.0);
        }
        vertical -= GRAVITY * dt;
        let desired = Vec3::new(walk.x, 0.0, walk.z) * dt + Vec3::Y * (vertical * dt);
        let shape = r::Capsule::new_y(character.half_height(), character.radius);
        let movement = self.controller.move_shape(
            dt,
            &self.world.query_pipeline_with_filter(solid()),
            &shape,
            &pose(character.centre(), Quat::IDENTITY),
            vector(desired),
            |_| {},
        );
        let moved = vec3(movement.translation);
        character.position += moved;
        character.grounded = movement.grounded;
        character.velocity = Vec3::new(
            moved.x / dt,
            if movement.grounded { 0.0 } else { vertical },
            moved.z / dt,
        );
        // Landing on something while rising (a ceiling) stops the climb.
        if !movement.grounded && vertical > 0.0 && moved.y < desired.y * 0.5 {
            character.velocity.y = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    /// A square of `half` metres at height `y`, centred on (x, z).
    fn floor(physics: &mut Physics, key: u32, x: f32, z: f32, y: f32, half: f32) {
        let v = [
            Vec3::new(-half, 0.0, -half),
            Vec3::new(half, 0.0, -half),
            Vec3::new(half, 0.0, half),
            Vec3::new(-half, 0.0, half),
        ];
        let shape = CollisionShape::mesh(&v, &[[0, 1, 2], [0, 2, 3]]).unwrap();
        physics.add_static(
            StaticKey::Cell(key),
            &shape,
            Vec3::new(x, y, z),
            Quat::IDENTITY,
        );
    }

    fn walk(physics: &Physics, c: &mut Character, velocity: Vec3, seconds: f32) {
        for _ in 0..(seconds / DT) as usize {
            physics.move_character(c, velocity, None, DT);
        }
    }

    #[test]
    fn ragdolls_fall_to_the_ground_and_rays_pass_them() {
        let mut p = ready();
        // Feet, hips, chest, head, and a hand on the chest.
        let parents = [None, Some(0), Some(1), Some(2), Some(2)];
        let ys = [2.0, 3.0, 3.45, 3.75, 3.4];
        let nodes: Vec<Mat4> = ys
            .iter()
            .enumerate()
            .map(|(i, &y)| Mat4::from_translation(Vec3::new(i as f32 * 0.01, y, 0.0)))
            .collect();
        let id = p.add_ragdoll(&parents, &nodes, Vec3::new(3.0, 0.0, 0.0));
        for _ in 0..240 {
            p.step(DT);
        }
        let now = p.ragdoll_nodes(id).unwrap();
        for m in &now[1..] {
            let at = m.w_axis.truncate();
            assert!(at.y > -0.1 && at.y < 0.8, "{at}");
            assert!(at.x > 0.5, "carried along: {at}");
        }
        // Limbs stay joined, and the root (no limb of its own) goes with
        // them.
        assert!(now[1].w_axis.distance(now[2].w_axis) < 0.6);
        assert!(
            now[0].w_axis.distance(now[1].w_axis) < 1.2,
            "{}",
            now[0].w_axis
        );
        p.push_ragdoll(id, Vec3::new(0.0, 15.0, 25.0));
        for _ in 0..30 {
            p.step(DT);
            let now = p.ragdoll_nodes(id).unwrap();
            for (a, b) in [(1, 2), (2, 3), (2, 4)] {
                let gap = now[a].w_axis.distance(now[b].w_axis);
                assert!(gap < 0.7, "{a}-{b} {gap}");
            }
        }
        let (d, _) = p
            .cast_ray(Vec3::new(now[2].w_axis.x, 5.0, 0.0), -Vec3::Y, 10.0)
            .unwrap();
        assert!((d - 5.0).abs() < 1e-3, "the ray hits the ground: {d}");
        p.remove_ragdoll(id);
        assert!(p.ragdoll_nodes(id).is_none());
    }

    fn ready() -> Physics {
        let mut p = Physics::new();
        floor(&mut p, 1, 0.0, 0.0, 0.0, 50.0);
        p
    }

    #[test]
    fn falls_and_lands() {
        let mut p = ready();
        p.step(DT);
        let mut c = Character::new(Vec3::new(0.0, 5.0, 0.0));
        walk(&p, &mut c, Vec3::ZERO, 2.0);
        assert!(c.grounded);
        assert!(c.position.y.abs() < 0.1, "{}", c.position.y);
    }

    #[test]
    fn climbs_a_step_but_not_a_wall() {
        let mut p = ready();
        // A 0.4 m step from z = 5, and a 1.5 m wall from x = 5.
        floor(&mut p, 2, 0.0, 15.0, 0.4, 10.0);
        let wall = CollisionShape::mesh(
            &[
                Vec3::new(0.0, 0.0, -5.0),
                Vec3::new(0.0, 0.0, 5.0),
                Vec3::new(0.0, 1.5, 5.0),
                Vec3::new(0.0, 1.5, -5.0),
            ],
            &[[0, 1, 2], [0, 2, 3]],
        )
        .unwrap();
        p.add_static(
            StaticKey::Object(0),
            &wall,
            Vec3::new(5.0, 0.0, -10.0),
            Quat::IDENTITY,
        );
        p.step(DT);

        let mut c = Character::new(Vec3::new(0.0, 0.0, 0.0));
        walk(&p, &mut c, Vec3::new(0.0, 0.0, 4.0), 3.0);
        assert!(c.position.z > 8.0, "stopped at {}", c.position);
        assert!(
            (c.position.y - 0.4).abs() < 0.1,
            "on the step: {}",
            c.position
        );

        let mut c = Character::new(Vec3::new(0.0, 0.0, -10.0));
        walk(&p, &mut c, Vec3::new(4.0, 0.0, 0.0), 3.0);
        assert!(c.position.x < 5.0, "through the wall: {}", c.position);
    }

    #[test]
    fn walks_up_gentle_slopes_only() {
        let ramp = |deg: f32| {
            let mut p = ready();
            let (s, c) = deg.to_radians().sin_cos();
            // A 20 m ramp rising along +z from z = 2.
            let v = [
                Vec3::new(-5.0, 0.0, 0.0),
                Vec3::new(5.0, 0.0, 0.0),
                Vec3::new(5.0, 20.0 * s, 20.0 * c),
                Vec3::new(-5.0, 20.0 * s, 20.0 * c),
            ];
            let shape = CollisionShape::mesh(&v, &[[0, 1, 2], [0, 2, 3]]).unwrap();
            p.add_static(
                StaticKey::Object(1),
                &shape,
                Vec3::new(0.0, 0.0, 2.0),
                Quat::IDENTITY,
            );
            p.step(DT);
            let mut ch = Character::new(Vec3::ZERO);
            walk(&p, &mut ch, Vec3::new(0.0, 0.0, 4.0), 4.0);
            ch.position.y
        };
        assert!(ramp(30.0) > 3.0, "30 degrees should be climbable");
        assert!(ramp(65.0) < 1.5, "65 degrees should not be");
    }

    fn car() -> VehicleSpec {
        use crate::vehicle::WheelSpec;
        let wheel = |x: f32, z: f32| WheelSpec {
            centre: Vec3::new(x, 0.35, z),
            radius: 0.35,
            width: 0.25,
            steers: z > 0.0,
        };
        VehicleSpec {
            model: 1,
            kind: "car".into(),
            label: "test".into(),
            wheels: vec![
                wheel(-0.9, 1.4),
                wheel(0.9, 1.4),
                wheel(-0.9, -1.3),
                wheel(0.9, -1.3),
            ],
            bounds: (Vec3::new(-1.0, 0.0, -2.2), Vec3::new(1.0, 1.5, 2.2)),
            seat: None,
            door: None,
            tracked: false,
            road_wheels: Vec::new(),
            rest: Vec::new(),
            node_wheel: Vec::new(),
            mass: 1400.0,
            turret: Vec::new(),
            pivot: None,
        }
    }

    #[test]
    fn cars_cross_a_steep_side_slope_upright() {
        let mut p = Physics::new();
        // A 30° slope falling away to +x; the car drives across it (+z).
        let half = 100.0;
        let v = [
            Vec3::new(-half, 0.0, -half),
            Vec3::new(half, 0.0, -half),
            Vec3::new(half, 0.0, half),
            Vec3::new(-half, 0.0, half),
        ];
        let shape = CollisionShape::mesh(&v, &[[0, 1, 2], [0, 2, 3]]).unwrap();
        let tilt = Quat::from_rotation_z(-30f32.to_radians());
        p.add_static(StaticKey::Cell(1), &shape, Vec3::ZERO, tilt);
        p.step(DT);
        let id = p.add_vehicle(&car(), Vec3::new(0.0, 0.3, 0.0), 0.0);
        for _ in 0..240 {
            p.drive(
                id,
                VehicleControls {
                    throttle: 1.0,
                    steer: -0.3,
                    brake: false,
                },
            );
            p.step(DT);
        }
        let up = p.vehicle_transform(id).unwrap().transform_vector3(Vec3::Y);
        assert!(up.y > 0.7, "rolled: {up}");
    }

    #[test]
    fn cars_settle_drive_and_steer() {
        let mut p = Physics::new();
        floor(&mut p, 1, 0.0, 0.0, 0.0, 200.0);
        p.step(DT);
        let id = p.add_vehicle(&car(), Vec3::new(0.0, 0.1, 0.0), 0.0);
        for _ in 0..120 {
            p.step(DT);
        }
        let at = |p: &Physics| {
            p.vehicle_transform(id)
                .unwrap()
                .transform_point3(Vec3::ZERO)
        };
        let rest = at(&p);
        assert!(rest.y > -0.2 && rest.y < 0.4, "settled at {rest}");
        assert!(rest.x.hypot(rest.z) < 0.2, "rolled to {rest}");

        // Full throttle for three seconds: forward along +z.
        for _ in 0..180 {
            p.drive(
                id,
                VehicleControls {
                    throttle: 1.0,
                    ..Default::default()
                },
            );
            p.step(DT);
        }
        let ahead = at(&p);
        assert!(ahead.z > 8.0, "drove to {ahead}");
        assert!(p.vehicle_speed(id) > 5.0);
        let up = p.vehicle_transform(id).unwrap().transform_vector3(Vec3::Y);
        assert!(up.y > 0.9, "tipped: {up}");

        // Steering right curves towards +x.
        for _ in 0..120 {
            p.drive(
                id,
                VehicleControls {
                    throttle: 0.5,
                    steer: 1.0,
                    brake: false,
                },
            );
            p.step(DT);
        }
        let turned = at(&p);
        assert!(turned.x > ahead.x + 2.0, "{ahead} -> {turned}");
        // It turns, not slides: the nose points right of where it was.
        let nose = p.vehicle_transform(id).unwrap().transform_vector3(Vec3::Z);
        assert!(nose.x > 0.3, "still facing {nose}");
        let wheels = p.vehicle_wheels(id);
        assert!(wheels[0].0 > 0.1 && wheels[2].0 == 0.0, "{wheels:?}");

        // Braking stops it.
        for _ in 0..240 {
            p.drive(
                id,
                VehicleControls {
                    brake: true,
                    ..Default::default()
                },
            );
            p.step(DT);
        }
        assert!(p.vehicle_speed(id).abs() < 0.5, "{}", p.vehicle_speed(id));
        p.remove_vehicle(id);
        assert!(p.vehicle_transform(id).is_none());
    }

    #[test]
    fn tanks_turn_on_the_spot() {
        let mut p = Physics::new();
        floor(&mut p, 1, 0.0, 0.0, 0.0, 200.0);
        p.step(DT);
        let tank = VehicleSpec {
            tracked: true,
            mass: 30_000.0,
            ..car()
        };
        let id = p.add_vehicle(&tank, Vec3::new(0.0, 0.1, 0.0), 0.0);
        for _ in 0..120 {
            p.step(DT);
        }
        for _ in 0..60 {
            p.drive(
                id,
                VehicleControls {
                    steer: 1.0,
                    ..Default::default()
                },
            );
            p.step(DT);
        }
        let t = p.vehicle_transform(id).unwrap();
        let nose = t.transform_vector3(Vec3::Z);
        let at = t.transform_point3(Vec3::ZERO);
        assert!(nose.x > 0.4, "facing {nose}");
        assert!(at.x.hypot(at.z) < 0.5, "moved to {at}");
        let wheels = p.vehicle_wheels(id);
        assert!(wheels.iter().all(|w| w.0 == 0.0), "{wheels:?}");
    }

    #[test]
    fn statics_are_replaced_and_removed() {
        let mut p = ready();
        floor(&mut p, 1, 0.0, 0.0, 3.0, 5.0);
        assert_eq!(p.static_count(), 1);
        p.step(DT);
        let (d, n) = p
            .cast_ray(Vec3::new(0.0, 10.0, 0.0), -Vec3::Y, 100.0)
            .unwrap();
        assert!((d - 7.0).abs() < 1e-3);
        assert!(n.y > 0.99);
        p.remove_static(StaticKey::Cell(1));
        p.step(DT);
        assert!(
            p.cast_ray(Vec3::new(0.0, 10.0, 0.0), -Vec3::Y, 100.0)
                .is_none()
        );
    }
}
