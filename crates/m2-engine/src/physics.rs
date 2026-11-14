//! Collision and movement on Rapier.
//!
//! The island's ground and placed models are static triangle-mesh
//! colliders. A model's collision comes from its `PHY2` shapes where it has
//! them, and from its visible triangles otherwise. Shapes are built on the
//! loader thread ([`CollisionShape`] is cheap to clone and share), so adding
//! them on the main thread is quick.
//!
//! People move with Rapier's kinematic character controller: a capsule that
//! slides along walls, climbs steps up to [`STEP_HEIGHT`] (loose pieces
//! lying about too) and slopes up to [`MAX_SLOPE`], and keeps to the
//! ground going down. Gravity and jumping are integrated here, not by the
//! solver.
//!
//! # Provenance
//!
//! Clean-room. The step height and slope limit are the viewer's values,
//! chosen to feel right, not read from the game.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use m2_assets::Model;
use m2_assets::model::GeometryKind;
use m2_assets::physics::{Body, Physics as Phy2};
use m2_assets::vertex::DeclUsage;
use m2_math::glam::{Mat3, Mat4, Quat, Vec3};
use rapier3d::control::{
    CharacterAutostep, CharacterLength, DynamicRayCastVehicleController,
    KinematicCharacterController, WheelTuning,
};

use crate::collision_layers::{self, Preset, layer_of, seen_by};
use crate::tuning::{
    BikeTuning, BoatFloat, BoatTuning, EngineTuning, FlightTuning, Gearbox, Handling, Springs,
    TankTuning, Tyres,
};
use crate::vehicle::VehicleSpec;
use rapier3d::prelude as r;

mod winch_cable;
pub use winch_cable::{CARGO_TILT, CableId, HALF_LINK, LINK_MASS, LINK_RADIUS, MIN_LINKS_TO_HOOK};

/// Highest step a character walks up without jumping, metres.
pub const STEP_HEIGHT: f32 = 0.5;
/// Steepest slope a character walks up, radians (50 degrees).
pub const MAX_SLOPE: f32 = 50.0 * std::f32::consts::PI / 180.0;
/// Downward acceleration, m/s².
pub const GRAVITY: f32 = 9.81;
/// The gravity the 360 car code pulls with where it works its own, m/s²
/// (the constant at 0x82DE8C30, -10, that the gravity factor's pull at
/// 0x8219EE4C and 14 other physics routines read).
pub const GRAVITY_360: f32 = 10.0;
/// Slowest a vehicle's wheels and tracks are seen to roll, m/s: under
/// it a vehicle is settling, not driving
/// ([`Physics::vehicle_rolling`]). Invented.
pub const ROLL_REST: f32 = 0.05;

/// Puts a collider into `world` (on `parent`'s body), its contacts' friction
/// mixed as Havok mixes it ([`HavokFriction`]).
fn insert_collider(
    world: &mut r::PhysicsWorld,
    collider: impl Into<r::Collider>,
    parent: Option<r::RigidBodyHandle>,
) -> r::ColliderHandle {
    let mut collider = collider.into();
    collider.set_active_hooks(collider.active_hooks() | r::ActiveHooks::MODIFY_SOLVER_CONTACTS);
    world.insert_collider(collider, parent)
}

/// Two touching colliders' friction as Havok gives it: the square root of
/// the product of theirs (its material default, as the 360's bodies keep it;
/// Rapier's own rules have no such mean). A terrain at the rigid-body
/// default 0.5 (0x82621820 +0x9C) under a wreck's 0.3 grips at 0.39, not
/// the 0.3 the least of the two gave.
struct HavokFriction;

impl r::PhysicsHooks for HavokFriction {
    fn modify_solver_contacts(&self, context: &mut r::ContactModificationContext) {
        let (Some(a), Some(b)) = (
            context.colliders.get(context.collider1),
            context.colliders.get(context.collider2),
        ) else {
            return;
        };
        let mixed = (a.friction() * b.friction()).max(0.0).sqrt();
        if let Some(m) = context.rigid_mut() {
            *m.friction = mixed;
        }
    }
}

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

/// Inertia this small against another axis's is raised to it (0x82167500,
/// its caller's argument from 0x820BAFBC).
const INERTIA_FLOOR: f32 = 1.0 / 15.0;
/// Every inertia the game sets is this much over what it worked out
/// (0x82167500, 0x820F9274).
const INERTIA_SCALE: f32 = 16.0 / 15.0;

/// An inertia as the 360 code sets it on a body (0x82167500): each axis
/// (its row's length) under [`INERTIA_FLOOR`] of another's raised to that,
/// weighed against the first other axis in turn (x against y, else z; y
/// against x, else z; z against x, else y) by their lengths before any is
/// raised, and the whole then made [`INERTIA_SCALE`] larger.
fn clip_inertia(i: Mat3) -> Mat3 {
    let rows = [i.row(0), i.row(1), i.row(2)];
    let len = rows.map(|r| r.length());
    let mut out = rows;
    for (k, (a, b)) in [(0, (1, 2)), (1, (0, 2)), (2, (0, 1))] {
        for other in [a, b] {
            let ratio = len[k] / len[other];
            if ratio < INERTIA_FLOOR {
                out[k] *= INERTIA_FLOOR / ratio;
                break;
            }
        }
    }
    Mat3::from_cols(out[0], out[1], out[2]).transpose() * INERTIA_SCALE
}

/// The mass and inertia (kg m², model axes) a tank, boat or helicopter's
/// chassis body ends up with. From the 360 code: each such object's set-up
/// hands its template's `Mass` (record word 0) to its physics (vtable
/// +0x128, 0x82182848), which gives it to its first body ([`CHASSIS`])
/// and, from the body's `PHY2` record, sets its inertia to the record's
/// times that mass over the record's, clipped (0x82167700,
/// [`clip_inertia`]); setting the mass alone leaves the inertia be
/// (0x8262C088). With no `Mass` it keeps the record's mass, and the body
/// builder clips the record's inertia (0x8216A290).
///
/// [`CHASSIS`]: crate::vehicle::CHASSIS
fn chassis_mass(c: &crate::vehicle::Chassis, mass: Option<f32>) -> (f32, Mat3) {
    match mass.filter(|m| *m > 0.0 && c.mass > 0.0) {
        Some(m) => (m, clip_inertia(c.inertia * (m / c.mass))),
        None => (c.mass, clip_inertia(c.inertia)),
    }
}

/// A static collision shape, shared between every place it is used.
#[derive(Clone)]
pub struct CollisionShape {
    shape: r::SharedShape,
    pub triangles: usize,
    /// Cut from the model's visible triangles, standing in for collision
    /// the model doesn't carry (`PHY2`).
    pub stand_in: bool,
    /// How the model comes loose when knocked, if its `PHY2` has bodies
    /// that move.
    pub loose: Option<Arc<Loose>>,
    /// How it comes loose as one, its box a single piece, if its `PHY2`
    /// has no body that moves (or none at all) and it is small enough
    /// ([`LOOSE_SIZE`]): for a template whose objects are physics props
    /// (`_PropPhysics`: a café chair, a planter), whose mass it is given
    /// ([`Loose::weighing`]).
    pub whole: Option<Arc<Loose>>,
    /// How it slides and bounces: [`BodyMaterial::of`] its `PHY2`'s first
    /// body that moves, else its first; `None` without a body.
    pub material: Option<BodyMaterial>,
    /// What its triangles are made of ([`crate::surface`]): runs of
    /// (first triangle, surface name hash) in triangle order; empty when
    /// nothing says (surface 0).
    pub surfaces: Arc<[(u32, u32)]>,
    /// What of it stands in an AI's way ([`PathSolid`]).
    pub path: PathSolid,
    /// Its shapes no vehicle meets ([`Ghost`]), if it has both kinds.
    pub ghost: Option<Ghost>,
    /// The box round all its `PHY2` shapes on parts it shows intact,
    /// leaves and shapes nothing meets included (model space), as a path
    /// search sizes it up. From the 360 code: an obstacle's box is its
    /// entity's (0x821861B0), the union of each of its live bodies' shape
    /// boxes (0x82182E90: every body with a world body, +0x38; bodies on
    /// hidden parts are dropped when it is rebuilt, 0x82152780). `None`
    /// without `PHY2` shapes.
    pub entity_box: Option<(Vec3, Vec3)>,
    /// Its standing shapes as laid out, to split them by another preset
    /// ([`CollisionShape::as_preset`]); `None` without `PHY2` shapes.
    pub source: Option<Arc<ShapeSource>>,
    /// Its leaves, as the world's queries meet them, if it shows any. From
    /// the 360 code: a `PHY2` leaf shape (its class [`FOLIAGE`](m2_assets::physics::FOLIAGE);
    /// flags bit 3 on a shape that isn't a mesh) takes its preset's seventh
    /// layer (0x8216C308): 19 for a building or a prop, which the path
    /// search's sweeps ([`PATH_MASK`](collision_layers::PATH_MASK),
    /// 0x822E6FD8) and the gunners' sight lines
    /// ([`SIGHT_MASK`](collision_layers::SIGHT_MASK), 0x822C95D8) both see.
    /// So an AI's path goes round a bush and its gunner doesn't see through
    /// one.
    ///
    /// In the alpha they stand as a sensor that only those two kinds of query
    /// meet (its `LEAVES` group): no body bumps into them, people walk through them,
    /// shots pass. (The 360's filter has layer 19 meet a vehicle's layers 8, 9
    /// and 11; what a vehicle then does in a bush isn't traced, and the alpha
    /// keeps driving through it.) A loose prop's leaves would take layer 18
    /// (its preset has no seventh layer); they are left out.
    pub leaves: Option<r::SharedShape>,
}

/// Shapes of a model that no vehicle meets and the world's queries still
/// see. From the 360 code: a `_BuildingPhysics` shape takes its layer from
/// its flags (0x8216C308), and the world's filter (0x82164A58) has a
/// vehicle's hull layers (8 and 9, 0x82189448) meet only some layers
/// ([`collision_layers::meets_vehicles`]). A jungle rock cluster's AI box
/// (flags 0x25, layer 4), a canopy's flags 5 shapes and the
/// `..._aicollision` bridges' shapes (layer 30) are on layers they don't
/// meet, while the path search (0x822E6FD8) and the AI's sight lines see
/// layer 4 and 30. So a tank drives through such a box that its path goes
/// round.
///
/// In the alpha they stand apart from the model's other shapes as a
/// sensor: no body bumps into them, and every query sees them but a
/// wheel's look for the ground. People still walk into them (inferred:
/// the filter's layers 27 and 28, which meet them, aren't named, and
/// which layer people are on isn't traced).
#[derive(Clone)]
pub struct Ghost {
    shape: r::SharedShape,
    /// What of them stands in an AI's way.
    pub path: PathSolid,
}

impl fmt::Debug for Ghost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ghost").field("path", &self.path).finish()
    }
}

/// What of a model a path search meets. From the 360 code: its sweeps and
/// nose checks see only the layers of [`PATH_MASK`](crate::collision_layers::PATH_MASK),
/// and a building's shapes take their layers shape by shape
/// ([`layer_of`](crate::collision_layers::layer_of)).
#[derive(Clone, Default)]
pub enum PathSolid {
    /// All of it.
    #[default]
    All,
    /// None: a chain-link fence, which an AI drives through.
    Nothing,
    /// Only these of its shapes.
    Part(r::SharedShape),
}

impl PathSolid {
    /// A model with some shapes left out: `triangles` of `vertices` are
    /// those a path search meets.
    pub fn of(vertices: &[Vec3], triangles: &[[u32; 3]]) -> Self {
        CollisionShape::mesh(vertices, triangles).map_or(Self::Nothing, |m| Self::Part(m.shape))
    }
}

/// One standing `PHY2` shape of a model ([`ShapeSource`]): its flags and
/// kind, which give its layer, and its run of the triangles.
#[derive(Clone, Debug, PartialEq)]
struct ShapeRun {
    flags: u32,
    kind: u32,
    triangles: std::ops::Range<u32>,
}

/// A model's standing `PHY2` shapes as [`CollisionShape::for_model`] laid
/// them out, kept to split them again by another preset
/// ([`CollisionShape::as_preset`]): their points (model space), triangles
/// and runs.
#[derive(Clone, Debug)]
pub struct ShapeSource {
    vertices: Vec<Vec3>,
    triangles: Vec<[u32; 3]>,
    runs: Vec<ShapeRun>,
}

impl ShapeSource {
    /// Bytes it keeps (its points, triangles and runs).
    pub fn bytes(&self) -> usize {
        self.vertices.len() * std::mem::size_of::<Vec3>()
            + self.triangles.len() * 12
            + self.runs.len() * std::mem::size_of::<ShapeRun>()
    }
}

/// A model's shapes split by what meets them under a preset.
#[derive(Default)]
struct Split {
    /// What vehicles meet, and of it what a path search meets (and
    /// whether some is left out).
    solid: Vec<[u32; 3]>,
    path: Vec<[u32; 3]>,
    path_part: bool,
    /// What no vehicle meets ([`Ghost`]), and of it what a path search
    /// meets.
    ghost: Vec<[u32; 3]>,
    ghost_path: Vec<[u32; 3]>,
    ghost_part: bool,
}

/// Splits `runs` of `triangles` by `preset`'s layers: a shape on a layer
/// a vehicle meets is solid, the rest ghosts; each is in a path search's
/// way when [`PATH_MASK`](collision_layers::PATH_MASK) has its layer. A
/// model of no shape a vehicle meets keeps them all solid (none such is
/// known among the buildings; kept as before).
fn split_shapes(runs: &[ShapeRun], triangles: &[[u32; 3]], preset: Preset) -> Split {
    let mut out = Split::default();
    for r in runs {
        let layer = layer_of(preset.layers(), r.flags, r.kind);
        let tris = &triangles[r.triangles.start as usize..r.triangles.end as usize];
        let seen = seen_by(collision_layers::PATH_MASK, layer);
        if collision_layers::meets_vehicles(layer) {
            if seen {
                out.path.extend_from_slice(tris);
            } else {
                out.path_part = true;
            }
            out.solid.extend_from_slice(tris);
        } else {
            if seen {
                out.ghost_path.extend_from_slice(tris);
            } else {
                out.ghost_part = true;
            }
            out.ghost.extend_from_slice(tris);
        }
    }
    if out.solid.is_empty() {
        out.solid = std::mem::take(&mut out.ghost);
        out.path_part |= out.ghost_part;
        out.path = std::mem::take(&mut out.ghost_path);
        out.ghost_part = false;
    }
    out
}

/// Whether a path search meets `PHY2` shape `s` of a building's model.
fn in_ais_way(s: &m2_assets::physics::Shape) -> bool {
    seen_by(
        collision_layers::PATH_MASK,
        layer_of(&collision_layers::BUILDING, s.flags(), s.kind()),
    )
}

impl fmt::Debug for PathSolid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::All => "All",
            Self::Nothing => "Nothing",
            Self::Part(_) => "Part",
        })
    }
}

impl fmt::Debug for CollisionShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CollisionShape")
            .field("triangles", &self.triangles)
            .field("stand_in", &self.stand_in)
            .field("loose", &self.loose)
            .field("whole", &self.whole)
            .finish()
    }
}

/// A `PHY2` shape as [`CollisionShape::shape_meshes`] gives it, last whether
/// it stands in an AI's way ([`PathSolid`]): its node,
/// its vertices (model space) and triangles.
pub type ShapeMesh = (u16, Vec<Vec3>, Vec<[u32; 3]>, bool);

/// How a prop comes loose when a vehicle or a blast knocks it (its `PHY2`
/// has bodies that move: traffic lights, plants, barricades, poles): its
/// standing collision goes and its pieces fly.
///
/// Most such models are broken up beforehand: a fixed body holds the
/// parts shown intact, and the pieces it breaks into sit on moving
/// bodies whose nodes the model shows in neither state (10,356 of the
/// 10,647 moving bodies of the island's placed models). Those break: the
/// model is drawn wrecked (the stump or base left standing) and the
/// pieces fly, drawn where they land. A model whose moving bodies it
/// shows intact (a barricade) goes as one, drawn whole.
///
/// The `PHY2` constraints that would hold pieces together (a door on its
/// hinge) are not read: each piece goes its own way. Nothing of a broken
/// prop stands in the way afterwards (its base included).
#[derive(Clone)]
pub struct Loose {
    pub pieces: Vec<LoosePiece>,
    /// Breaks into its pieces, the rest of it drawn wrecked.
    pub breaks: bool,
    /// The moving bodies' mass, kg: what a vehicle has to shift.
    pub mass: f32,
    /// Each node's place in the model at rest.
    rest: Vec<Mat4>,
    /// For each node, the piece it goes with: the nearest at or above it
    /// (`None`: it stays put).
    owner: Vec<Option<usize>>,
    /// For each node, whether it is drawn with its piece whatever the
    /// model's state shows (a broken prop's pieces).
    flying: Vec<bool>,
}

impl fmt::Debug for Loose {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Loose")
            .field("pieces", &self.pieces.len())
            .field("breaks", &self.breaks)
            .field("mass", &self.mass)
            .finish()
    }
}

/// How a `PHY2` body slides, bounces and slows: what the game gives the
/// Havok body it makes of one.
///
/// From the 360 code: the body builder (0x8216A290) starts from Havok's
/// rigid-body defaults (0x82621820: no linear damping, 0.05 angular, 0.5
/// friction, 0.4 restitution, 200 m/s and 200 rad/s at most), then sets
/// the record's friction, no restitution, the record's angular damping
/// but [`MIN_ANGULAR_DAMPING`] at least, and caps the speeds at
/// [`MAX_LINEAR_SPEED`] and [`MAX_ANGULAR_SPEED`]; linear damping stays
/// none. Havok mixes two bodies' friction as the square root of their
/// product; Rapier's average gives the same for the equal values the
/// samples have.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyMaterial {
    pub friction: f32,
    pub restitution: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
}

/// The least angular damping the game gives a `PHY2` body (0x8216A390,
/// its constant 0x820B8034). From the 360 code.
pub const MIN_ANGULAR_DAMPING: f32 = 0.8;
/// The fastest a `PHY2` body moves and turns, m/s and radians a second
/// (0x8216A400, its constants 0x820F9628 and 0x820F8CC4: 132 and 3π).
/// From the 360 code.
pub const MAX_LINEAR_SPEED: f32 = 132.0;
pub const MAX_ANGULAR_SPEED: f32 = 3.0 * std::f32::consts::PI;

impl BodyMaterial {
    /// What the game makes of body record `body`.
    pub fn of(body: &Body) -> Self {
        let damping = body.angular_damping();
        Self {
            friction: body.friction(),
            restitution: 0.0,
            linear_damping: 0.0,
            // As the game's compare: one not over the floor (NaN too) gets
            // the floor.
            angular_damping: if damping > MIN_ANGULAR_DAMPING {
                damping
            } else {
                MIN_ANGULAR_DAMPING
            },
        }
    }

    /// What the game makes of the commonest record (0.3 throughout, on
    /// 16,070 of `vz.wad`'s 16,088 bodies; from samples), for a piece
    /// whose own body isn't known.
    pub const COMMON: Self = Self {
        friction: 0.3,
        restitution: 0.0,
        linear_damping: 0.0,
        angular_damping: MIN_ANGULAR_DAMPING,
    };

    /// How a loose piece not made from a `PHY2` body (a gate's chunk)
    /// slows in the air and slides. Invented.
    const DEBRIS: Self = Self {
        friction: 0.8,
        restitution: 0.0,
        linear_damping: 0.2,
        angular_damping: 0.2,
    };
}

/// One piece of a [`Loose`] prop: boxes round its `PHY2` shapes, in the
/// space of the node it moves.
#[derive(Clone)]
pub struct LoosePiece {
    /// The node it moves, with those under it; `None` for the whole
    /// model, in model space.
    pub node: Option<u16>,
    shape: r::SharedShape,
    /// Kilograms.
    pub mass: f32,
    /// How it slides and slows.
    pub material: BodyMaterial,
    /// Model space to the node's space, at rest.
    from_model: Mat4,
}

impl LoosePiece {
    /// A box of half extents `half` about `centre` (model space), moving
    /// node `node`, which sits at the model's origin at rest; of the
    /// commonest body ([`BodyMaterial::COMMON`]).
    pub fn block(node: Option<u16>, centre: Vec3, half: Vec3, mass: f32) -> Self {
        Self {
            node,
            shape: r::SharedShape::compound(vec![(
                pose(centre, Quat::IDENTITY),
                r::SharedShape::cuboid(half.x, half.y, half.z),
            )]),
            mass,
            material: BodyMaterial::COMMON,
            from_model: Mat4::IDENTITY,
        }
    }

    /// The middle of its box, in its node's space.
    pub fn centre(&self) -> Vec3 {
        let b = self.shape.compute_local_aabb();
        (vec3(b.mins) + vec3(b.maxs)) * 0.5
    }

    /// How big it is: its box's diagonal, metres.
    pub fn size(&self) -> f32 {
        self.extent().length()
    }

    /// Its box's sides along its node's axes, metres.
    pub fn extent(&self) -> Vec3 {
        let b = self.shape.compute_local_aabb();
        vec3(b.maxs) - vec3(b.mins)
    }
}

impl fmt::Debug for LoosePiece {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoosePiece")
            .field("node", &self.node)
            .field("mass", &self.mass)
            .finish()
    }
}

/// The lightest and heaviest a loose piece is, kg (a `PHY2` body may say
/// 0).
const LOOSE_MASS: f32 = 2.0;
const LOOSE_MOST: f32 = 100_000.0;

/// A `PHY2` body's mass as a loose piece's.
fn loose_mass(mass: f32) -> f32 {
    if mass.is_finite() {
        mass.clamp(LOOSE_MASS, LOOSE_MOST)
    } else {
        LOOSE_MASS
    }
}
/// The thinnest a loose piece's box is, half metres.
const LOOSE_HALF: f32 = 0.02;
/// Models larger than this (their box's diagonal, metres) never come
/// loose: a building with a door that swings, or one that falls to pieces
/// when blown up, stands when a vehicle runs into it.
const LOOSE_SIZE: f32 = 14.0;

impl Loose {
    /// How `model` comes loose by its `PHY2` `p`; `None` unless a body of
    /// it moves.
    fn of(model: &Model<'_>, p: &Phy2) -> Option<Self> {
        let moving: Vec<usize> = (0..p.bodies.len())
            .filter(|&b| !p.bodies[b].is_fixed())
            .collect();
        let b = &model.info.bounds;
        let size = (Vec3::from(b.max) - Vec3::from(b.min)).length();
        if moving.is_empty() || size.is_nan() || size > LOOSE_SIZE {
            return None;
        }
        let hidden = model
            .states
            .as_ref()
            .map(|s| [s.hidden(&model.nodes, false), s.hidden(&model.nodes, true)])
            .unwrap_or_default();
        let off = |wrecked: usize, n: usize| hidden[wrecked].get(n).copied().unwrap_or(false);
        let shapes_of = |b: usize| (0..p.shapes.len()).filter(move |&k| p.body_of(k) == Some(b));
        let body_node = |b: usize| {
            p.parts
                .iter()
                .find(|q| q.body == Some(b))
                .and_then(|q| u16::try_from(q.node).ok())
                .filter(|&n| usize::from(n) < model.nodes.len())
        };
        let rest: Vec<Mat4> = (0..model.nodes.len())
            .map(|n| model.node_to_model(n as u16).unwrap_or(Mat4::IDENTITY))
            .collect();
        // The pieces it breaks into: moving bodies on nodes it doesn't
        // show intact.
        let broken: Vec<(usize, u16)> = moving
            .iter()
            .filter_map(|&b| Some((b, body_node(b)?)))
            .filter(|&(_, n)| off(0, usize::from(n)))
            .collect();
        let breaks = !broken.is_empty();
        let mut pieces = Vec::new();
        if breaks {
            for (b, node) in broken {
                let from_model = rest[usize::from(node)].inverse();
                let boxes: Vec<_> = shapes_of(b)
                    .filter_map(|k| shape_box(p, k, Mat4::IDENTITY))
                    .collect();
                if boxes.is_empty() || !from_model.is_finite() {
                    continue;
                }
                pieces.push(LoosePiece {
                    node: Some(node),
                    shape: r::SharedShape::compound(boxes),
                    mass: loose_mass(p.bodies[b].mass),
                    material: BodyMaterial::of(&p.bodies[b]),
                    from_model,
                });
            }
        } else {
            // As one: what stands in the way intact, in model space.
            let boxes: Vec<_> = (0..p.shapes.len())
                .filter(|&k| standing(p, k, &hidden[0]))
                .filter_map(|k| {
                    let node = p
                        .shape_node(k)
                        .and_then(|n| u16::try_from(n).ok())
                        .and_then(|n| model.node_to_model(n))
                        .unwrap_or(Mat4::IDENTITY);
                    shape_box(p, k, node)
                })
                .collect();
            if !boxes.is_empty() {
                pieces.push(LoosePiece {
                    node: None,
                    shape: r::SharedShape::compound(boxes),
                    mass: loose_mass(moving.iter().map(|&b| loose_mass(p.bodies[b].mass)).sum()),
                    material: BodyMaterial::of(&p.bodies[moving[0]]),
                    from_model: Mat4::IDENTITY,
                });
            }
        }
        Self::with_pieces(model, pieces, breaks, rest)
    }

    /// `model` coming loose as one, its collision's box `bounds` (model
    /// space) a single piece of the lightest mass ([`Loose::weighing`]
    /// gives it its own); `None` for one too big ([`LOOSE_SIZE`]).
    fn whole(model: &Model<'_>, (lo, hi): (Vec3, Vec3)) -> Option<Self> {
        let size = (hi - lo).length();
        if !size.is_finite() || size > LOOSE_SIZE {
            return None;
        }
        let half = ((hi - lo) * 0.5).max(Vec3::splat(LOOSE_HALF));
        let piece = LoosePiece::block(None, (lo + hi) * 0.5, half, LOOSE_MASS);
        let rest = (0..model.nodes.len())
            .map(|n| model.node_to_model(n as u16).unwrap_or(Mat4::IDENTITY))
            .collect();
        Self::with_pieces(model, vec![piece], false, rest)
    }

    /// The same, weighing `kg` in all (each piece its share).
    pub fn weighing(&self, kg: f32) -> Self {
        let k = loose_mass(kg) / self.mass.max(f32::EPSILON);
        let mut out = self.clone();
        for p in &mut out.pieces {
            p.mass *= k;
        }
        out.mass = out.pieces.iter().map(|p| p.mass).sum();
        out
    }

    /// A model coming loose as `pieces` (breaking into them with
    /// `breaks`), its nodes at rest where `rest` has them; `None` without
    /// any.
    fn with_pieces(
        model: &Model<'_>,
        pieces: Vec<LoosePiece>,
        breaks: bool,
        rest: Vec<Mat4>,
    ) -> Option<Self> {
        if pieces.is_empty() {
            return None;
        }
        // Each node goes with the nearest piece at or above it.
        let owner: Vec<Option<usize>> = (0..model.nodes.len())
            .map(|n| {
                let mut at = Some(n as u16);
                for _ in 0..=model.nodes.len() {
                    let here = at?;
                    if let Some(i) = pieces.iter().position(|q| match q.node {
                        Some(m) => m == here,
                        None => true,
                    }) {
                        return Some(i);
                    }
                    at = model.nodes.get(usize::from(here))?.parent;
                }
                None
            })
            .collect();
        let flying = owner.iter().map(|o| breaks && o.is_some()).collect();
        Some(Self {
            mass: pieces.iter().map(|q| q.mass).sum(),
            pieces,
            breaks,
            rest,
            owner,
            flying,
        })
    }

    /// For each node, whether it is drawn with its piece whatever the
    /// model's state shows (as
    /// [`GpuModel::draw_moved`](crate::GpuModel::draw_moved) takes).
    pub fn flying(&self) -> &[bool] {
        &self.flying
    }

    /// Where each piece starts, world space, with the model placed at
    /// `placed`.
    pub fn starts(&self, placed: Mat4) -> Vec<Mat4> {
        self.pieces
            .iter()
            .map(|q| placed * q.from_model.inverse())
            .collect()
    }

    /// Each node's place in the model (as
    /// [`GpuModel::draw_moved`](crate::GpuModel::draw_moved) takes), with
    /// the model placed at `placed` and each piece where `places` (world
    /// space, piece order) has it.
    pub fn posed(&self, placed: Mat4, places: &[Mat4]) -> Vec<Mat4> {
        let back = placed.inverse();
        self.rest
            .iter()
            .zip(&self.owner)
            .map(|(&rest, owner)| {
                match owner.and_then(|i| Some((self.pieces.get(i)?, places.get(i)?))) {
                    Some((piece, &now)) => back * now * piece.from_model * rest,
                    None => rest,
                }
            })
            .collect()
    }
}

/// How a model breaks apart when it is destroyed outright (a mission's
/// gate a support strike brings down), from its part states: into the
/// pieces they show only as it breaks
/// ([`PartStates::chunks`](m2_assets::model::PartStates::chunks)), each
/// flying as a box round its geometry, the rest drawn wrecked; and the
/// points its breaking names besides, empty ones (where the videos show
/// fires burning and smoke rising).
#[derive(Clone, Debug)]
pub struct Breakup {
    pub pieces: Loose,
    /// Model space.
    pub points: Vec<Vec3>,
}

/// How heavy a broken-off piece is for the room its box takes, kg/m³
/// (stone; its boxes are loose fits, so less than stone's 2,500).
const CHUNK_DENSITY: f32 = 1200.0;

impl Breakup {
    /// How `model` breaks apart, its pieces' boxes round their geometry at
    /// `lod` (or the next level present); `None` unless it has pieces.
    pub fn of(model: &Model<'_>, lod: u32) -> Option<Self> {
        let states = model.states.as_ref()?;
        let chunks = states.chunks(&model.nodes);
        if chunks.is_empty() {
            return None;
        }
        let lod = (lod..model.info.lod_count.max(lod + 1))
            .find(|&l| model.has_lod(l))
            .unwrap_or(lod);
        let rest: Vec<Mat4> = (0..model.nodes.len())
            .map(|n| model.node_to_model(n as u16).unwrap_or(Mat4::IDENTITY))
            .collect();
        // The chunk a node goes with: itself, or the nearest above it.
        let chunk_of = |n: u16| {
            let mut at = Some(n);
            for _ in 0..=model.nodes.len() {
                let here = at?;
                if chunks.contains(&here) {
                    return Some(here);
                }
                at = model.nodes.get(usize::from(here))?.parent;
            }
            None
        };
        // Each chunk's geometry, in its own node's space.
        let mut bounds: HashMap<u16, (Vec3, Vec3)> = HashMap::new();
        for (segment, geometry) in model.lod(lod) {
            if geometry.kind == GeometryKind::Skin {
                continue;
            }
            let Some(chunk) = chunk_of(segment.node) else {
                continue;
            };
            let to_chunk = rest[usize::from(chunk)].inverse() * rest[usize::from(segment.node)];
            for group in &geometry.groups {
                let Some(pos) = group
                    .decl
                    .find(DeclUsage::Position, 0)
                    .and_then(|e| group.vertices.read(e))
                else {
                    continue;
                };
                let b = bounds
                    .entry(chunk)
                    .or_insert((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)));
                for p in pos {
                    let v = to_chunk.transform_point3(Vec3::new(p[0], p[1], p[2]));
                    *b = (b.0.min(v), b.1.max(v));
                }
            }
        }
        let mut pieces = Vec::new();
        for &n in &chunks {
            let Some(&(lo, hi)) = bounds.get(&n) else {
                continue;
            };
            let from_model = rest[usize::from(n)].inverse();
            if !from_model.is_finite() || !lo.cmple(hi).all() || !(hi - lo).is_finite() {
                continue;
            }
            let half = ((hi - lo) * 0.5).max(Vec3::splat(LOOSE_HALF));
            let shape = r::SharedShape::cuboid(half.x, half.y, half.z);
            pieces.push(LoosePiece {
                node: Some(n),
                shape: r::SharedShape::compound(vec![(
                    pose((lo + hi) * 0.5, Quat::IDENTITY),
                    shape,
                )]),
                mass: loose_mass(8.0 * half.x * half.y * half.z * CHUNK_DENSITY),
                material: BodyMaterial::DEBRIS,
                from_model,
            });
        }
        // Points with nothing drawn on or under them: locators.
        let empty = |n: u16| {
            !model.segments.iter().any(|s| s.node == n)
                && !model.nodes.iter().any(|x| x.parent == Some(n))
        };
        let points = states
            .breaking_points(&chunks)
            .into_iter()
            .filter(|&n| empty(n))
            .filter_map(|n| rest.get(usize::from(n)))
            .map(|m| m.w_axis.truncate())
            .collect();
        Some(Self {
            pieces: Loose::with_pieces(model, pieces, true, rest)?,
            points,
        })
    }
}

/// Whether `PHY2` shape `k` stands in the way of the model as it stands
/// intact (`hidden` as [`PartStates::hidden`](m2_assets::model::PartStates::hidden)
/// has it): not leaves ([`FOLIAGE`](m2_assets::physics::FOLIAGE)), which
/// vehicles and people go through, nor a shape on a part the model
/// doesn't show intact (its wreck's, the pieces it breaks into).
fn standing(p: &Phy2, k: usize, hidden: &[bool]) -> bool {
    let Some(s) = p.shapes.get(k) else {
        return false;
    };
    let node = p.parts.get(s.part).map_or(0, |q| q.node as usize);
    !s.is_foliage() && !hidden.get(node).copied().unwrap_or(false)
}

/// The shapes body `body` of `PHY2` `p` collides with as a vehicle's
/// body, model space: each standing one's points (its convex radius
/// included), as [`CollisionShape::shape_meshes`] places them. Those of
/// its vehicle hull set if it has any, else the others
/// ([`m2_assets::physics::Shape::is_vehicle_hull`], 0x8216BC08), and
/// whether they are its hull set.
pub(crate) fn body_shapes(model: &Model<'_>, p: &Phy2, body: usize) -> (Vec<Vec<Vec3>>, bool) {
    body_shapes_showing(model, p, body, false)
}

/// [`body_shapes`] with the model's parts showing as it stands intact, or
/// `wrecked`.
pub(crate) fn body_shapes_showing(
    model: &Model<'_>,
    p: &Phy2,
    body: usize,
    wrecked: bool,
) -> (Vec<Vec<Vec3>>, bool) {
    let (shapes, hull) = body_shapes_seen(model, p, body, wrecked);
    (shapes.into_iter().map(|(v, _)| v).collect(), hull)
}

/// [`body_shapes_showing`], each shape with whether a path search sees
/// it: its layer as a vehicle's ([`collision_layers::VEHICLE`], by its
/// flags, 0x8216C308) in [`PATH_MASK`](collision_layers::PATH_MASK). A
/// car's flags 0x28 hull shape (layer 11) isn't.
pub(crate) fn body_shapes_seen(
    model: &Model<'_>,
    p: &Phy2,
    body: usize,
    wrecked: bool,
) -> (Vec<(Vec<Vec3>, bool)>, bool) {
    let hidden = model
        .states
        .as_ref()
        .map(|s| s.hidden(&model.nodes, wrecked))
        .unwrap_or_default();
    let mine: Vec<usize> = (0..p.shapes.len())
        .filter(|&k| p.body_of(k) == Some(body) && standing(p, k, &hidden))
        .collect();
    let hull = mine.iter().any(|&k| p.shapes[k].is_vehicle_hull());
    let shapes = mine
        .into_iter()
        .filter(|&k| p.shapes[k].is_vehicle_hull() == hull)
        .map(|k| {
            let s = &p.shapes[k];
            let to_model = p
                .shape_node(k)
                .and_then(|n| u16::try_from(n).ok())
                .and_then(|n| model.node_to_model(n))
                .unwrap_or(Mat4::IDENTITY)
                * p.shape_place(k).map_or(Mat4::IDENTITY, place);
            let (v, _) = s.geometry.triangles(s.radius);
            let seen = seen_by(
                collision_layers::PATH_MASK,
                layer_of(&collision_layers::VEHICLE, s.flags(), s.kind()),
            );
            let points: Vec<Vec3> = v
                .iter()
                .map(|&x| to_model.transform_point3(Vec3::from(x)))
                .collect();
            (points, seen)
        })
        .filter(|(v, _)| v.len() >= 4)
        .collect();
    (shapes, hull)
}

/// A box round `PHY2` shape `k` (its convex radius included, as its
/// triangles have it), placed by `to` from its body node's space.
fn shape_box(p: &Phy2, k: usize, to: Mat4) -> Option<(r::Pose, r::SharedShape)> {
    let s = p.shapes.get(k)?;
    let (v, _) = s.geometry.triangles(s.radius);
    let (lo, hi) = v.iter().fold(
        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
        |(lo, hi), &x| (lo.min(Vec3::from(x)), hi.max(Vec3::from(x))),
    );
    if lo.x > hi.x {
        return None;
    }
    let at = to
        * p.shape_place(k).map_or(Mat4::IDENTITY, place)
        * Mat4::from_translation((lo + hi) * 0.5);
    let (scale, rotation, translation) = at.to_scale_rotation_translation();
    let half = ((hi - lo) * 0.5 * scale.abs()).max(Vec3::splat(LOOSE_HALF));
    (half.is_finite() && translation.is_finite() && rotation.is_finite()).then(|| {
        (
            pose(translation, rotation.normalize()),
            r::SharedShape::cuboid(half.x, half.y, half.z),
        )
    })
}

impl CollisionShape {
    /// A box reaching `half` each way from its centre.
    pub fn cuboid(half: Vec3) -> Self {
        let half = half.max(Vec3::splat(0.01));
        Self {
            shape: r::SharedShape::cuboid(half.x, half.y, half.z),
            triangles: 12,
            stand_in: false,
            loose: None,
            whole: None,
            material: None,
            surfaces: Arc::new([]),
            path: PathSolid::All,
            ghost: None,
            entity_box: None,
            source: None,
            leaves: None,
        }
    }

    /// A triangle mesh; `None` if it has no valid triangle.
    pub fn mesh(vertices: &[Vec3], triangles: &[[u32; 3]]) -> Option<Self> {
        Self::mesh_of(vertices, triangles, &[])
    }

    /// A triangle mesh whose triangles are made of `surfaces`, one per
    /// triangle (surface 0 past its end); `None` if it has no valid
    /// triangle.
    pub fn mesh_of(vertices: &[Vec3], triangles: &[[u32; 3]], surfaces: &[u32]) -> Option<Self> {
        let n = vertices.len() as u32;
        let mut runs: Vec<(u32, u32)> = Vec::new();
        let mut kept: Vec<[u32; 3]> = Vec::with_capacity(triangles.len());
        for (k, t) in triangles.iter().enumerate() {
            if !(t.iter().all(|&i| i < n) && t[0] != t[1] && t[1] != t[2] && t[0] != t[2]) {
                continue;
            }
            let s = surfaces.get(k).copied().unwrap_or(0);
            if runs.last().is_none_or(|r| r.1 != s) {
                runs.push((kept.len() as u32, s));
            }
            kept.push(*t);
        }
        if kept.is_empty() || vertices.iter().any(|v| !v.is_finite()) {
            return None;
        }
        if runs.iter().all(|r| r.1 == 0) {
            runs.clear();
        }
        let count = kept.len();
        let shape =
            r::SharedShape::trimesh(vertices.iter().map(|&v| vector(v)).collect(), kept).ok()?;
        Some(Self {
            shape,
            triangles: count,
            stand_in: false,
            loose: None,
            whole: None,
            material: None,
            surfaces: runs.into(),
            path: PathSolid::All,
            ghost: None,
            entity_box: None,
            source: None,
            leaves: None,
        })
    }

    /// What triangle `face` is made of (a surface name hash; 0 for none
    /// known). Faces past the triangle count wrap round.
    pub fn surface_of(&self, face: u32) -> u32 {
        let face = face % (self.triangles.max(1) as u32);
        let k = self.surfaces.partition_point(|r| r.0 <= face);
        k.checked_sub(1).map_or(0, |k| self.surfaces[k].1)
    }

    /// The height of the highest of its surfaces at or under `(x, from, z)`
    /// (its own space), if any is: a load bed's floor under a crate.
    pub fn floor_under(&self, x: f32, from: f32, z: f32) -> Option<f32> {
        let ray = r::Ray::new(vector(Vec3::new(x, from, z)), vector(-Vec3::Y));
        let toi = self.shape.cast_local_ray(&ray, 100.0, true)?;
        Some(from - toi)
    }

    /// The box the shape fills, in its own space.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        let b = self.shape.compute_local_aabb();
        (vec3(b.mins), vec3(b.maxs))
    }

    /// This model's collision as a body of `preset` takes it: its standing
    /// shapes split again by that preset's layers (what vehicles meet,
    /// what a path search meets; [`split_shapes`]), all else as it is.
    /// `None` when it is the same: the building preset (which
    /// [`for_model`](Self::for_model) splits by), or no `PHY2` shapes.
    pub fn as_preset(&self, preset: Preset) -> Option<Self> {
        let src = self
            .source
            .as_ref()
            .filter(|_| preset != Preset::Building)?;
        let split = split_shapes(&src.runs, &src.triangles, preset);
        let surface = self.surfaces.first().map_or(0, |r| r.1);
        let surfaces = vec![surface; split.solid.len()];
        let mut shape = Self::mesh_of(&src.vertices, &split.solid, &surfaces)?;
        if split.path_part {
            shape.path = PathSolid::of(&src.vertices, &split.path);
        }
        if let Some(g) = Self::mesh(&src.vertices, &split.ghost) {
            shape.ghost = Some(Ghost {
                shape: g.shape,
                path: if split.ghost_part {
                    PathSolid::of(&src.vertices, &split.ghost_path)
                } else {
                    PathSolid::All
                },
            });
        }
        Some(Self {
            stand_in: self.stand_in,
            loose: self.loose.clone(),
            whole: self.whole.clone(),
            material: self.material,
            entity_box: self.entity_box,
            source: self.source.clone(),
            leaves: self.leaves.clone().filter(|_| preset != Preset::LooseProp),
            ..shape
        })
    }

    /// A model's collision in model space: its `PHY2` shapes if `phy2` is
    /// given and parses, else its rigid visible geometry at `lod` (a stand
    /// in).
    pub fn for_model(model: &Model<'_>, phy2: Option<&[u8]>, lod: u32) -> Option<Self> {
        Self::for_model_part(model, phy2, lod, None, None)
    }

    /// [`for_model`](Self::for_model) with its nodes where `nodes` (model
    /// space, one a node) puts them, not at rest: a gate's shapes with
    /// its leaves swung open ([`crate::gate::Gate::nodes`]), as the 360's
    /// leaves are bodies of their own on their hinges.
    pub fn for_model_posed(
        model: &Model<'_>,
        phy2: Option<&[u8]>,
        lod: u32,
        nodes: &[Mat4],
    ) -> Option<Self> {
        Self::for_model_part(model, phy2, lod, None, Some(nodes))
    }

    /// The `PHY2` shapes [`for_model`](Self::for_model) stands in the
    /// way, one by one: each one's node, and its vertices (model space)
    /// and triangles. None for a model without `PHY2` shapes.
    pub fn shape_meshes(model: &Model<'_>, phy2: Option<&[u8]>) -> Vec<ShapeMesh> {
        let Some(p) = phy2.and_then(|d| Phy2::parse(d).ok()) else {
            return Vec::new();
        };
        let hidden = model
            .states
            .as_ref()
            .map(|s| s.hidden(&model.nodes, false))
            .unwrap_or_default();
        let mut out = Vec::new();
        for (k, s) in p.shapes.iter().enumerate() {
            if !standing(&p, k, &hidden) {
                continue;
            }
            let Some(node) = p.shape_node(k).and_then(|n| u16::try_from(n).ok()) else {
                continue;
            };
            let to_model = model.node_to_model(node).unwrap_or(Mat4::IDENTITY)
                * p.shape_place(k).map_or(Mat4::IDENTITY, place);
            let (v, t) = s.geometry.triangles(s.radius);
            out.push((
                node,
                v.iter()
                    .map(|&x| to_model.transform_point3(Vec3::from(x)))
                    .collect(),
                t,
                in_ais_way(s),
            ));
        }
        out
    }

    /// One sub-object of a merged batch (a `TINY` model), by the index its
    /// vertices carry in their position's `w`: its visible geometry at
    /// `lod`, in model space.
    pub fn for_sub_object(model: &Model<'_>, index: u32, lod: u32) -> Option<Self> {
        Self::for_model_part(model, None, lod, Some(index), None)
    }

    fn for_model_part(
        model: &Model<'_>,
        phy2: Option<&[u8]>,
        lod: u32,
        sub_object: Option<u32>,
        posed: Option<&[Mat4]>,
    ) -> Option<Self> {
        let mut vertices = Vec::new();
        let mut triangles = Vec::new();
        let mut surfaces = Vec::new();
        // The triangles of its shapes a path search meets, and whether
        // some are left out.
        let mut path_triangles = Vec::new();
        let mut path_part = false;
        // Those of its shapes no vehicle meets ([`Ghost`]), and of them
        // those a path search meets.
        let mut ghost_triangles = Vec::new();
        let mut ghost_path = Vec::new();
        let mut ghost_part = false;
        let mut entity_box: Option<(Vec3, Vec3)> = None;
        // Each standing shape's flags, kind and triangles, kept for the
        // other presets ([CollisionShape::as_preset]).
        let mut runs: Vec<ShapeRun> = Vec::new();
        let mut shape_triangles: Vec<[u32; 3]> = Vec::new();
        // Its leaves' triangles ([`CollisionShape::leaves`]).
        let mut leaf_triangles: Vec<[u32; 3]> = Vec::new();
        let mut loose = None;
        let mut material = None;
        // Shapes of its own, whether or not any stands in the way: no
        // stand-in then.
        let mut shaped = false;
        if let Some(p) = phy2.and_then(|d| Phy2::parse(d).ok()) {
            if sub_object.is_none() {
                loose = Loose::of(model, &p).map(Arc::new);
            }
            if let Some(b) = p.bodies.iter().find(|b| !b.is_fixed()).or(p.bodies.first()) {
                material = Some(BodyMaterial::of(b));
            }
            shaped = !p.shapes.is_empty();
            let hidden = model
                .states
                .as_ref()
                .map(|s| s.hidden(&model.nodes, false))
                .unwrap_or_default();
            for (k, s) in p.shapes.iter().enumerate() {
                let shown = p
                    .parts
                    .get(s.part)
                    .is_some_and(|q| !hidden.get(q.node as usize).copied().unwrap_or(false));
                if !shown {
                    continue;
                }
                // A shape sits at its part's place in its body node's
                // space.
                let node = p
                    .shape_node(k)
                    .and_then(|n| u16::try_from(n).ok())
                    .and_then(|n| match posed {
                        Some(nodes) => nodes.get(usize::from(n)).copied(),
                        None => model.node_to_model(n),
                    })
                    .unwrap_or(Mat4::IDENTITY);
                let to_model = node * p.shape_place(k).map_or(Mat4::IDENTITY, place);
                let (v, t) = s.geometry.triangles(s.radius);
                for &x in &v {
                    let x = to_model.transform_point3(Vec3::from(x));
                    let (lo, hi) = entity_box.get_or_insert((x, x));
                    *lo = lo.min(x);
                    *hi = hi.max(x);
                }
                if !standing(&p, k, &hidden) {
                    // Shown but not standing: leaves ([`CollisionShape::leaves`]).
                    if s.is_foliage() {
                        let first = vertices.len() as u32;
                        vertices
                            .extend(v.iter().map(|&x| to_model.transform_point3(Vec3::from(x))));
                        leaf_triangles.extend(t.iter().map(|t| t.map(|i| first + i)));
                    }
                    continue;
                }
                let first = vertices.len() as u32;
                vertices.extend(v.iter().map(|&x| to_model.transform_point3(Vec3::from(x))));
                let from = shape_triangles.len() as u32;
                shape_triangles.extend(t.iter().map(|t| t.map(|i| first + i)));
                runs.push(ShapeRun {
                    flags: s.flags(),
                    kind: s.kind(),
                    triangles: from..shape_triangles.len() as u32,
                });
            }
            let split = split_shapes(&runs, &shape_triangles, Preset::Building);
            triangles = split.solid;
            path_triangles = split.path;
            path_part = split.path_part;
            ghost_triangles = split.ghost;
            ghost_path = split.ghost_path;
            ghost_part = split.ghost_part;
        }
        let stand_in = !shaped && triangles.is_empty();
        if stand_in {
            let lod = (lod..model.info.lod_count.max(lod + 1))
                .find(|&l| model.has_lod(l))
                .unwrap_or(lod);
            // Only what stands drawn and solid: not the wrecked parts an
            // intact model hides, nor its leaves, light glows or unseen
            // surfaces.
            let hidden = model
                .states
                .as_ref()
                .map(|s| s.hidden(&model.nodes, false))
                .unwrap_or_default();
            let solid = |material: u32| {
                model
                    .materials
                    .get(material as usize)
                    .is_none_or(|m| m.flags & NOT_SOLID == 0)
            };
            for (segment, geometry) in model.lod(lod) {
                if geometry.kind == GeometryKind::Skin
                    || hidden
                        .get(usize::from(segment.node))
                        .copied()
                        .unwrap_or(false)
                {
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
                    for range in group.material_ranges.iter().filter(|r| solid(r.material)) {
                        let surface = model
                            .materials
                            .get(range.material as usize)
                            .map_or(0, |m| m.surface);
                        let before = triangles.len();
                        triangles.extend(
                            group
                                .triangles(range)
                                .into_iter()
                                .filter(|t| t.iter().all(|&i| mine(i)))
                                .map(|t| t.map(|i| first + u32::from(i))),
                        );
                        surfaces.resize(surfaces.len() + triangles.len() - before, surface);
                    }
                }
            }
        } else {
            // Its shapes say nothing of what they are made of: they take
            // the surface most of its solid drawn area has (inferred).
            surfaces = vec![drawn_surface(model, lod); triangles.len()];
        }
        let mut shape = Self::mesh_of(&vertices, &triangles, &surfaces)?;
        if path_part {
            shape.path = PathSolid::of(&vertices, &path_triangles);
        }
        shape.entity_box = entity_box.filter(|(lo, hi)| lo.is_finite() && hi.is_finite());
        if let Some(g) = Self::mesh(&vertices, &ghost_triangles) {
            shape.ghost = Some(Ghost {
                shape: g.shape,
                path: if ghost_part {
                    PathSolid::of(&vertices, &ghost_path)
                } else {
                    PathSolid::All
                },
            });
        }
        shape.leaves = Self::mesh(&vertices, &leaf_triangles).map(|m| m.shape);
        if !runs.is_empty() {
            shape.source = Some(Arc::new(ShapeSource {
                vertices,
                triangles: shape_triangles,
                runs,
            }));
        }
        let whole = match (&loose, sub_object) {
            (None, None) => Loose::whole(model, shape.bounds()).map(Arc::new),
            _ => None,
        };
        Some(Self {
            stand_in,
            loose,
            whole,
            material,
            ..shape
        })
    }
}

/// The surface ([`m2_assets::model::Material::surface`]) of most of a
/// model's solid drawn area at `lod` (or its next level drawn); 0 for a
/// model drawing nothing solid.
pub fn drawn_surface(model: &Model<'_>, lod: u32) -> u32 {
    let lod = (lod..model.info.lod_count.max(lod + 1))
        .find(|&l| model.has_lod(l))
        .unwrap_or(lod);
    let mut area: Vec<(u32, f32)> = Vec::new();
    for (_, geometry) in model.lod(lod) {
        for group in &geometry.groups {
            let Some(pos) = group
                .decl
                .find(DeclUsage::Position, 0)
                .and_then(|e| group.vertices.read(e))
            else {
                continue;
            };
            let at = |i: u16| {
                pos.get(usize::from(i))
                    .map_or(Vec3::ZERO, |p| Vec3::new(p[0], p[1], p[2]))
            };
            for range in &group.material_ranges {
                let Some(m) = model.materials.get(range.material as usize) else {
                    continue;
                };
                if m.flags & NOT_SOLID != 0 {
                    continue;
                }
                let a: f32 = group
                    .triangles(range)
                    .into_iter()
                    .map(|[a, b, c]| (at(b) - at(a)).cross(at(c) - at(a)).length() * 0.5)
                    .sum();
                match area.iter_mut().find(|e| e.0 == m.surface) {
                    Some(e) => e.1 += a,
                    None => area.push((m.surface, a)),
                }
            }
        }
    }
    area.into_iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map_or(0, |e| e.0)
}

/// Material flags of surfaces nothing stands on or bumps into: additive
/// glows (lights, trails), tree leaves (swaying in the wind), and surfaces
/// never drawn (as the renderer reads them; not confirmed in the game).
const NOT_SOLID: u16 = 0x0002 | 0x0400 | 0x0800;

/// A `PHY2` part's place (position, rotation x y z w) as a matrix; a
/// rotation that isn't one counts as none.
fn place((position, rotation): ([f32; 3], [f32; 4])) -> Mat4 {
    let q = Quat::from_array(rotation);
    let q = if q.is_finite() && q.length() > 0.5 {
        q.normalize()
    } else {
        Quat::IDENTITY
    };
    Mat4::from_rotation_translation(q, Vec3::from(position))
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
    /// A model a state layer places while it is in, by entity.
    Entity(u32),
}

/// A tracked vehicle's road wheels: how far each may rise above where its
/// model has it, metres. Invented (the 360 code drives tanks with their
/// own `_TankPhysics`, not traced yet; a wheeled vehicle's springs are its
/// template's [`Springs`]). Its spring is this much longer than that plus
/// what the vehicle's weight squeezes it ([`preloads`]), so standing it
/// sits as its model has it and its drawn wheels follow their springs
/// ([`Physics::vehicle_wheel_lift`]).
const SUSPENSION: f32 = 0.3;
/// How far each wheel's spring is squeezed by the vehicle's weight
/// standing on level ground, metres, in spec order: the front (steering)
/// and back axles carry it as its centre of mass (at `mass_z`, model
/// space) lies between them, each axle's wheels alike, and a spring of
/// strength `k` (per kilogram, as its template's) squeezed `c` pushes
/// `k c` per kilogram. Both axles carry half each when they are not apart
/// (a single axle, or every wheel steering).
fn preloads(spec: &VehicleSpec, strength: impl Fn(bool) -> f32, mass_z: f32) -> Vec<f32> {
    let axle = |front: bool| {
        let z: Vec<f32> = spec
            .wheels
            .iter()
            .filter(|w| w.steers == front)
            .map(|w| w.centre.z)
            .collect();
        (z.len(), z.iter().sum::<f32>() / z.len().max(1) as f32)
    };
    let ((nf, zf), (nr, zr)) = (axle(true), axle(false));
    let front_share = if nf == 0 {
        0.0
    } else if nr == 0 {
        1.0
    } else if zf - zr > 0.1 {
        ((mass_z - zr) / (zf - zr)).clamp(0.1, 0.9)
    } else {
        0.5
    };
    spec.wheels
        .iter()
        .map(|w| {
            let (share, n) = if w.steers {
                (front_share, nf)
            } else {
                (1.0 - front_share, nr)
            };
            let k = strength(w.steers).max(1.0);
            (GRAVITY * share / (n.max(1) as f32 * k)).min(1.0)
        })
        .collect()
}

/// How a vehicle's hull slides on what it meets when driven: whether it
/// is a car or motorbike (the game's `_CarPhysicsV2`, whose friction
/// [`car_hull_friction`] sets each step), and its friction to start.
///
/// From the 360 code: a car's step (0x8219ECA4) sets it each step; a
/// boat's sim (0x82198FF8) sets its template's `HullFriction` once
/// ([`BoatTuning::hull_friction`]); a tank's and a helicopter's keep
/// their `PHY2` body's, which is 0.3 on every record (from samples,
/// [`BodyMaterial::COMMON`]).
fn hull_feel(spec: &VehicleSpec) -> (bool, f32) {
    let tank = spec.tracked && spec.tuning.tank.is_some();
    if spec.is_boat() {
        (false, spec.tuning.boat.unwrap_or(BOAT).hull_friction)
    } else if tank || spec.is_helicopter() {
        (false, BodyMaterial::COMMON.friction)
    } else {
        (true, car_hull_friction(1.0))
    }
}

/// A car's or motorbike's hull friction while its up axis points
/// (the 360's bike runs the car's step, 0x8219EE74, on its own shapes)
/// up_y up: none upright, [`CAR_TIPPED_FRICTION`] once on its side or
/// roof (its up axis level or below). From the 360 code (0x8219ECA4: the
/// chassis matrix's up y over 0 gives 0, else 0.3, set when it differs).
fn car_hull_friction(up_y: f32) -> f32 {
    if up_y > 0.0 { 0.0 } else { CAR_TIPPED_FRICTION }
}

/// See [`car_hull_friction`] (its constant 0x820BAF0C).
const CAR_TIPPED_FRICTION: f32 = 0.3;

/// A tracked vehicle's road wheels' springs and dampers (squeezed,
/// stretched), per kilogram. Invented.
const SPRING: f32 = 40.0;
const DAMPING: (f32, f32) = (2.0, 3.0);
/// A tank's braking with the throttle against the motion, its handbrake
/// (on the wheels that don't steer), rolling with no throttle, and
/// holding still, m/s² (as if all wheels braked so). Invented: a wheeled
/// vehicle's tyres slow it as its template's [`Tyres`] say.
const SERVICE_BRAKE: f32 = 8.0;
/// The tyres' friction coefficient, for a vehicle whose template gives
/// no `FrontWheelFrictionSide` and `RearWheelFrictionSide`. Invented.
const GRIP: f32 = 2.0;
const HANDBRAKE: f32 = 9.0;
const ROLLING: f32 = 0.6;
const HOLD: f32 = 3.0;
/// Left to itself, a tank rolling faster than this (m/s) slows at
/// [`HOLD`], then holds with every wheel as hard as the handbrake.
const PARKED_FAST: f32 = 3.0;
/// How far a ragdoll's limb may drift off its joint before it is put
/// back, metres.
const RAGDOLL_SLACK: f32 = 0.02;
/// How quickly a wreck stops sliding and spinning, how hard its hull
/// drags on the ground, and how far down its wheels' rims reach (of the
/// radius from the centre): invented.
const WRECK_DAMPING: f32 = 1.5;
const WRECK_FRICTION: f32 = 0.9;
const WRECK_RIM: f32 = 0.7;
/// How fast tracks turn a hull standing still, radians per second.
const TRACK_TURN: f32 = 0.9;
/// Top speeds, m/s, forwards (for a vehicle whose template gives none)
/// and backwards.
const TOP_SPEED: f32 = 30.0;
const REVERSE_SPEED: f32 = 8.0;
/// A tracked vehicle's pull, m/s², falling to nothing at its top speed,
/// when its template gives no `_TankPhysics` ([`drive_tracks`] drives one
/// that does). Invented.
const TRACK_PULL: f32 = 4.0;
/// The wheel Rapier casts each of a 360-driven tank's track points with,
/// metres: the 360 code casts a bare ray (0x821A5898).
const TRACK_RAY_RADIUS: f32 = 0.02;
/// Boats ([`Physics::set_water`]): how deep a wreck's origin floats,
/// metres (the layers put the start's boat 0.35 m under the -36 m sea,
/// patrol boats at it; an intact boat floats as deep as its springs hold
/// it, [`float_depth`]); how a wreck's rocking dies down, per second (an
/// intact boat turns by its helm, [`BoatFloat`]); its top
/// speed when its template gives none, m/s; and the most of its weight a
/// wreck's buoyancy holds up (it sinks). All invented.
pub const FLOAT_DEPTH: f32 = 0.35;

/// How deep a boat's origin rests under still water, level and stopped,
/// metres: where its float springs hold it ([`BoatFloat::rest_depth`];
/// its template's, else the 360 record's defaults).
pub fn float_depth(spec: &VehicleSpec) -> f32 {
    spec.tuning.boat.unwrap_or(BOAT).float.rest_depth(GRAVITY)
}
const BOAT_ROCK: f32 = 2.5;
const BOAT_TOP: f32 = 18.0;
const WRECK_FLOAT: f32 = 0.4;
/// A boat's engine and drag when its template gives none (its own
/// `_BoatPhysics` otherwise): the 360 record's defaults (0x823EA868).
const BOAT: BoatTuning = BoatTuning {
    reverse_speed: 10.0 / 3.6,
    torque: (100.0, 10.0),
    drag: [0.5, 2.0, 1.0],
    brake: 0.0,
    curve: [0.4, 1.0, 0.9, 0.15],
    hull_friction: 0.3,
    float: BoatFloat::DEFAULT,
};
/// The speed a boat is taken to plane from, m/s, for its spray and sound
/// (its float lifts its bow as its springs do, [`BoatFloat`]). Invented,
/// from the videos.
pub const PLANE_SPEED: f32 = 12.0;
/// Tracks grip: on the ground, a tracked vehicle's weight doesn't pull it
/// down hills up to this steep, radians, so it climbs, turns and stands on
/// them as on the flat (a play-tester: the game's tank "should be able to climb
/// almost any hill"; the steepness is invented).
const CLIMB_TRACKED: f32 = 0.87;

/// How quickly a helicopter's rotor spins up with the climb held, of its
/// full speed a second (from the 360 code, 0x821A2150).
const ROTOR_SPIN_UP: f32 = 0.25;
/// What a helicopter's rotor keeps of its speed each 1/60 s as it winds
/// down on the ground with no climb asked (0.99, from 0.1 of full speed
/// 0.95), with the descent held (0.975), and wrecked (0.925); from the
/// 360 code (0x821A2150).
const ROTOR_IDLE: (f32, f32) = (0.99, 0.95);
const ROTOR_DOWN: f32 = 0.975;
const ROTOR_WRECKED: f32 = 0.925;
/// From this height up a helicopter's lift fades out over
/// [`CEILING_FADE`] metres (from the 360 code, 0x821A25E0: 500 m and 0.01
/// a metre).
const CEILING: f32 = 500.0;
const CEILING_FADE: f32 = 100.0;
/// A helicopter asked to turn slower than this, radians a second, stops
/// turning with its template's `YawDamp` instead of `YawAcc` (from the
/// 360 code, 0x821A25E0).
const YAW_STILL: f32 = 0.2;
/// A motorbike's balance when its template gives none (invented: the
/// Scooter's record, one of the island's commonest bikes).
const BIKE: BikeTuning = BikeTuning {
    roll_rate: 1.5,
    roll_angle: (1.0, 0.3),
    roll_input: 0.0,
    roll: 20.0,
    wheelie: 0.65,
    pitch_strength: (30.0, 30.0),
    pitch_high_speed: 1.0,
    wheelie_steer: 1.0,
    side_damp: 10.0,
    damp: [11.0, 10.0, 1.0],
    air: (5.0, 1.5),
    low_speed: 70.0 / 3.6,
    inertia: [1.5, 0.3, 1.5],
    centre_of_mass: [0.0; 3],
};

/// A helicopter's body is at most this wide, metres either side: its hull
/// box leaves the rotor disc out.
const FUSELAGE_HALF: f32 = 1.6;

/// A vehicle in the physics world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VehicleId(usize);

/// What the driver does.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehicleControls {
    pub throttle: f32,
    pub steer: f32,
    pub brake: bool,
    /// Nobody at the wheel: it brakes to a stop (gently while it still
    /// rolls fast) and then holds, even on a hill. A tank instead rolls on
    /// its tracks' grip with no throttle until its body sleeps ([`Rest`]).
    pub parked: bool,
}

struct VehicleBody {
    controller: DynamicRayCastVehicleController,
    /// The body its wreck collides with ([`VehicleSpec::wreck`]).
    wreck: Option<crate::vehicle::WreckBody>,
    /// Its own collision is on it for what rides it ([`Physics::add_bed`]).
    bed: bool,
    mass: f32,
    /// Which wheels steer (the front axle), in wheel order.
    steers: Vec<bool>,
    /// How far each wheel's spring's top sits above where the model has
    /// the wheel's centre, metres, in wheel order.
    tops: Vec<f32>,
    /// How its tyres hold it ([`Tyres`]; a tank's are Rapier's, invented).
    tyres: Option<Tyres>,
    /// What its driver asks of its front and back axles' tyres now
    /// ([`Physics::drive`]), which they push it with each step
    /// ([`push_axles`]).
    axles: [Axle; 2],
    /// How well it stands on the ground under it now ([`Tyres::footing`];
    /// 1 on the flat).
    footing: f32,
    /// Its weight on the ground, of its plain weight ([`Springs`]'s
    /// `gravity_factor`; 1 for a tank).
    gravity_factor: f32,
    /// On tracks: turns on the spot instead of steering wheels.
    tracked: bool,
    /// On two wheels in line: held up, leaning into its turns.
    bike: Option<Bike>,
    /// Top speed forwards and backing up, m/s.
    top: f32,
    reverse_top: f32,
    /// A wheeled vehicle's engine and gearbox (its template's, else the
    /// 360 code's defaults).
    engine: Option<EngineTuning>,
    /// A tracked vehicle's drive and tracks as the 360 code runs them,
    /// with its template's `_TankPhysics` ([`drive_tracks`]).
    tank: Option<Tracks>,
    /// Its gearbox as it runs.
    gearbox: Gearbox,
    /// The torque its engine drives its wheels with now, newton-metres,
    /// through its gear and eased by its top speed ([`drive_axles`]).
    drive: f32,
    /// Its driver has the throttle open (its engine driving it, or about
    /// to once its clutch delay is out).
    pulling: bool,
    /// Its driver brakes: pulls against the way it rolls, or holds the
    /// handbrake (not a parked vehicle's hold).
    braking: bool,
    /// Its template's steering lock and brakes, on wheels.
    handling: Option<crate::tuning::Handling>,
    /// Where its front wheels point now, radians (left positive): they
    /// swing towards where the driver steers as its `handling` says.
    steering: f32,
    /// Wrecked: nothing drives it any more.
    wrecked: bool,
    /// A car or motorbike (the game's `_CarPhysicsV2`): its hull's
    /// friction follows how it lies ([`car_hull_friction`]).
    car_hull: bool,
    /// Its turret's colliders, and where they turn about (model space):
    /// turned with its turret ([`Physics::turn_turret`]).
    turret: (Vec<r::ColliderHandle>, Option<Vec3>),
    /// Its turret's own body and hinge, when it has one.
    hinge: Option<TurretHinge>,
    /// Held where it is ([`Physics::hold_vehicle`]): its body fixed.
    held: bool,
    /// Its velocity before the last step, m/s: what it hit things with.
    before: Vec3,
    /// A boat's: it floats instead of rolling on wheels.
    hull: Option<Hull>,
    /// A helicopter's: it flies.
    rotor: Option<Rotorcraft>,
}

/// How hard a helicopter's turbulence pushes it, m/s² a radian of its
/// shake ([`turbulence`]): the 360 code's 0.5 (its object's 0xD0, set at
/// 0x821A3278) x 30 (0x821A2F00).
const TURBULENCE_PUSH: f32 = 0.5 * 30.0;

/// The 360 code's smooth noise over one axis (0x828566B0): the gradient
/// at each whole step from a table of 256 ([`noise_table`]), eased
/// between them (3f² - 2f³), 0 to 1 about 0.5.
fn noise(x: f32) -> f32 {
    let g = noise_table();
    let i = x.floor();
    let f = x - i;
    let i = i as i64;
    let (a, b) = (g[(i & 255) as usize], g[((i + 1) & 255) as usize]);
    let s = f * f * (3.0 - 2.0 * f);
    (a * f + s * (b * (f - 1.0) - a * f) + 1.0) * 0.5
}

/// The gradients [`noise`] reads, each -1 to 1. The 360 code fills its
/// table once from the game's random numbers (0x82856768, through
/// 0x8215E260), so its values depend on that generator's state: these
/// are the alpha's own, from a fixed seed. Invented values, the 360's
/// form.
fn noise_table() -> &'static [f32; 256] {
    static TABLE: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut u: u32 = 0x2545_F491;
        std::array::from_fn(|_| {
            u = u.wrapping_mul(0x0019_660D).wrapping_add(0x3C6E_F35F);
            (u >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        })
    })
}

/// How an AI pilot's stick wobbles at `time` (the game's clock, s), as the
/// 360 code's 0x821A1DC0 works it each step from [`noise`]: added to the
/// speed asked ahead, (n(t + 205.642) + 1) / 4; sideways,
/// (n(t / 4 + 128.245) - 1/2) x 4; and to the climb asked,
/// -(n(2t + 5.111) + 1) / 4; each held to -1..1. None once it has been on
/// the ground over half a second (its `ground`, s), and none with the
/// player at the stick (its object's 0x68, taken for that: inferred).
fn ai_wobble(time: f32, ground: f32) -> [f32; 3] {
    if ground > 0.5 {
        return [0.0; 3];
    }
    [
        ((noise(time + 205.642) + 1.0) * 0.25).clamp(-1.0, 1.0),
        ((noise(time * 0.25 + 128.245) - 0.5) * 4.0).clamp(-1.0, 1.0),
        (-(noise(time * 2.0 + 5.111) + 1.0) * 0.25).clamp(-1.0, 1.0),
    ]
}

/// Where in its turbulence a helicopter is, 0 to 1, from its id as the
/// 360 code takes it from its entity's (0x821A2F00: two steps of the
/// generator 0x19660D, 0x3C6EF35F from id ^ 0xC71C2830, made a float in
/// 1..2 less 1). The alpha's ids are its own, so its helicopters' phases
/// are not the 360's.
fn turbulence_phase(id: u32) -> f32 {
    let step = |u: u32| u.wrapping_mul(0x0019_660D).wrapping_add(0x3C6E_F35F);
    let u = step(id ^ 0xC71C_2830);
    let u2 = step(u);
    f32::from_bits(0x3F80_0000 | (((u2 & 0xFFFF_01FF) | (u >> 16)) >> 9)) - 1.0
}

/// A helicopter's shake by the air, as the 360 code works it each step
/// (0x821A2F00): along (`a`) and across (`b`, three times as much) its
/// flat heading, radians, at `time` seconds into the game, `phase` its
/// [`turbulence_phase`]: each the sum of two of [`noise`] less 1, at
/// times 0.4 as fast as the clock and set apart, x 8 degrees x its
/// rotor's `spin` x how far it is off the ground (`air` seconds off it,
/// at most 1; on it, 1 less `ground` seconds on it, at most 1). It is
/// pushed by (flat across x `b` + flat ahead x `a`) x its mass x
/// [`TURBULENCE_PUSH`].
fn turbulence(phase: f32, time: f32, spin: f32, air: f32, ground: f32) -> (f32, f32) {
    let off = if air > 0.0 {
        air.clamp(0.0, 1.0)
    } else {
        1.0 - ground.clamp(0.0, 1.0)
    };
    let t = 60.0 * phase + 0.4 * time;
    let t9 = t + 53.245;
    let shake = 8f32.to_radians() * spin * off;
    let a = (noise(t + 123.356) + noise(205.642 - t9) - 1.0) * shake;
    let b = (noise(63.427 - t) + noise(t9 + 183.235) - 1.0) * 3.0 * shake;
    (a, b)
}

/// A helicopter's handling and what its pilot asks of it.
struct Rotorcraft {
    flight: FlightTuning,
    /// Its handling with an AI at the stick (`_HelicopterPhysicsAi`), if
    /// its template gives one.
    flight_ai: Option<FlightTuning>,
    /// An AI flies it ([`Physics::set_ai_pilot`]): it flies by
    /// [`flight_ai`](Self::flight_ai) and its stick wobbles
    /// ([`ai_wobble`]).
    ai: bool,
    controls: VehicleControls,
    /// Up (1) or down (-1).
    lift: f32,
    /// Slide right (1) or left (-1), its own way ([`Physics::strafe`]).
    strafe: f32,
    /// How it flies now, as the 360 code keeps it ([`Rotor`]).
    state: Rotor,
    /// Where in its turbulence it is ([`turbulence_phase`]), 0 to 1.
    phase: f32,
}

/// What a helicopter's `_HelicopterPhysics` keeps from step to step
/// (from the 360 code: its object's 0x3C, 0x38, 0x50 and 0x4C).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Rotor {
    /// Its rotor's speed, of full (0 to 1): it lifts only at full.
    spin: f32,
    /// The climb it works to, of its fastest (-1 to 1): eases to the
    /// climb asked by its difference a second.
    climb: f32,
    /// How long it has been off everything, and on something, seconds
    /// (each at most 1, the other 0).
    air: f32,
    ground: f32,
}

/// What a helicopter's pilot asks, each -1 to 1 (from the 360 code: the
/// messages 0x8218C018 stores at its object's 0x28 to 0x34).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Stick {
    /// Climb (+) or descend.
    lift: f32,
    /// Turn right (+) or left.
    yaw: f32,
    /// Ahead (+) or back.
    ahead: f32,
    /// Slide right (+) or left.
    right: f32,
}

/// What a helicopter's step does to its body ([`Rotor::fly`]): the
/// impulse on its middle, N s, the angular velocity it is given across
/// (x and z; its y is kept), rad/s, and the torque turning it about y,
/// N m, for the step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Flown {
    impulse: Vec3,
    spin_across: Option<(f32, f32)>,
    torque_y: f32,
}

/// What the 360 code's `x` keeps of itself after `dt` seconds at `keep`
/// each 1/60 s, the last part of a 60th blended (0x821A3B38).
fn keep_per_frame(mut x: f32, keep: f32, mut dt: f32) -> f32 {
    const FRAME: f32 = 1.0 / 60.0;
    while dt > FRAME {
        if x * x < (keep * 1e-5) * (keep * 1e-5) {
            return 0.0;
        }
        dt -= FRAME;
        x *= keep;
    }
    let part = dt * 60.0;
    x * keep * part + x * (1.0 - part)
}

impl Rotor {
    /// Its rotor's spin (0x821A2150) and how long it has been in the air
    /// or on something (0x821A2010), for `dt` seconds of `stick`; wrecked,
    /// its rotor winds down. `touching`: its body touches anything.
    fn spin(&mut self, stick: Stick, wrecked: bool, touching: bool, dt: f32) {
        self.spin = if wrecked {
            keep_per_frame(self.spin, ROTOR_WRECKED, dt)
        } else if stick.lift > 0.0 {
            self.spin + dt * ROTOR_SPIN_UP
        } else if self.ground <= 0.5 {
            self.spin
        } else if stick.lift == 0.0 {
            let keep = if self.spin <= 0.1 {
                ROTOR_IDLE.1
            } else {
                ROTOR_IDLE.0
            };
            keep_per_frame(self.spin, keep, dt)
        } else {
            keep_per_frame(self.spin, ROTOR_DOWN, dt)
        }
        .clamp(0.0, 1.0);
        if touching {
            self.ground = (self.ground + dt).min(1.0);
            self.air = 0.0;
        } else {
            self.air = (self.air + dt).min(1.0);
            self.ground = 0.0;
        }
    }

    /// One step of the 360 code's helicopter (0x821A38F0, after its
    /// rotor and timers, [`spin`](Self::spin)): it rolls and pitches to
    /// what the stick asks (0x821A2C28), pulls towards the speeds asked
    /// ahead and sideways, turns at the rate asked and climbs at the rate
    /// asked (0x821A25E0). `turn` is its rotation, `velocity` and `angular`
    /// its body's, `height` its middle's, `mass` its, `gravity` the
    /// world's pull, m/s².
    #[allow(clippy::too_many_arguments)]
    fn fly(
        &mut self,
        f: &FlightTuning,
        stick: Stick,
        shake: [f32; 3],
        turn: Quat,
        velocity: Vec3,
        angular: Vec3,
        height: f32,
        mass: f32,
        gravity: f32,
        dt: f32,
    ) -> Flown {
        let mut out = Flown::default();
        let spin = self.spin;
        let spin2 = spin * spin;
        let air = (self.air * 4.0).clamp(0.0, 1.0);
        let x = turn * Vec3::X;
        let z = turn * Vec3::Z;
        let hspeed = velocity.with_y(0.0).length();
        // How far it banks into a slide at speed (0x821A3210).
        let bank = {
            let span = f.forward - f.bank_speed;
            if span > 0.0 && stick.right * stick.yaw > -1e-4 {
                ((hspeed - f.bank_speed) / span).clamp(0.0, 1.0) * f.roll_bank
            } else {
                0.0
            }
        };
        if spin > 0.0 {
            let share = (velocity.dot(z) / f.forward).clamp(-1.0, 1.0);
            let roll = (x.y / x.length()).clamp(-1.0, 1.0).asin();
            let pitch = z.y.atan2(z.with_y(0.0).length());
            let want_roll = (0.25 + 0.75 * share) * f.roll_forward * stick.yaw
                + air * stick.right * (f.roll_side + bank);
            let want_pitch =
                -share * f.pitch * 0.66 - air * stick.ahead.clamp(-1.0, 1.0) * f.pitch * 0.33;
            let roll_rate = f.roll_accel * (want_roll - roll);
            let pitch_rate = f.pitch_accel * (want_pitch - pitch);
            let across = |zc: f32, xc: f32, w: f32| {
                spin2 * (roll_rate * zc - pitch_rate * xc) + (1.0 - spin2) * w
            };
            out.spin_across = Some((across(z.x, x.x, angular.x), across(z.z, x.z, angular.z)));
        }
        let ahead = z.with_y(0.0).normalize_or_zero();
        let right = (-x).with_y(0.0).normalize_or_zero();
        // Ahead and sideways.
        // An AI's wobble ([`ai_wobble`]) on the speed asked, not on how
        // hard it pulls (0x821A28F8, 0x821A2990).
        let stick_ahead = (stick.ahead + shake[0]).clamp(-1.0, 1.0);
        let pull = f.forward_accel * (1.0 - (1.0 - stick.ahead * stick.ahead) * f.no_input_inertia);
        out.impulse += ahead
            * (pull * (stick_ahead * f.forward - ahead.dot(velocity)) * mass * dt * spin * air);
        out.impulse += right
            * (f.sideways_accel
                * ((stick.right + shake[1]).clamp(-1.0, 1.0) * f.sideways - right.dot(velocity))
                * air
                * mass
                * dt
                * spin);
        // Its turn: faster slow, less so at speed.
        let rate = if hspeed < f.low_speed {
            let t = hspeed / f.low_speed;
            (1.0 - t) * f.low_speed_yaw + t * f.yaw
        } else if f.forward - f.low_speed > 0.0 {
            let t = ((hspeed - f.low_speed) / (f.forward - f.low_speed)).min(1.0);
            (1.0 - t * f.high_speed_yaw_loss) * f.yaw
        } else {
            (1.0 - f.high_speed_yaw_loss) * f.yaw
        };
        // As the 360 code's, positive turns right: anticlockwise from
        // above is +y.
        let want = -(stick.yaw * rate + bank * f.bank_yaw * stick.right);
        let gain = if want.abs() < YAW_STILL {
            f.yaw_damp
        } else {
            f.yaw_accel
        };
        out.torque_y = gain * (want - angular.y) * air * spin * mass;
        // Its climb, at full rotor only: the weight held, the climb
        // asked, fading out above the ceiling.
        if spin == 1.0 {
            self.climb = (self.climb
                + ((stick.lift + shake[2]).clamp(-1.0, 1.0) - self.climb) * dt)
                .clamp(-1.0, 1.0);
            let high = ((height - CEILING) / CEILING_FADE).clamp(0.0, 1.0);
            out.impulse.y += (f.climb * self.climb - velocity.y + 1.0)
                * gravity
                * spin
                * (1.0 - high)
                * spin2
                * mass
                * dt;
        } else {
            self.climb = 0.0;
        }
        out
    }
}

/// One axle's tyres as its driver works them ([`Tyres`]): its coefficient
/// along, m/s², and across, per second, as the 360 code works them out.
#[derive(Clone, Copy, Debug, Default)]
struct Axle {
    along: f32,
    across: f32,
    /// Parked: it holds still, up to its coefficients (along, and across
    /// as though sliding at 1 m/s), as a tank does (invented).
    hold: bool,
}

impl Axle {
    /// Front and back, standing with nobody at the wheel.
    fn standing(tyres: &Option<Tyres>) -> [Axle; 2] {
        let axle = |front| {
            tyres.map_or(Axle::default(), |t| Axle {
                along: t.slowing(front, 0.0, 0.0, 0.0, 0.0, 0.0),
                across: t.grip(front, 0.0, 0.0, 0.0, 0.0),
                hold: false,
            })
        };
        [axle(true), axle(false)]
    }
}

/// A motorbike's balance: its template's (else [`BIKE`]), and where its
/// steering stands as its balance eases it (0x270 of the 360's car; 360
/// sign, positive to the right).
#[derive(Clone, Copy)]
struct Bike {
    tuning: BikeTuning,
    steer: f32,
}

/// What a motorbike's balance reads of it each step besides its body:
/// its wheels and how many touch the ground, the mean of their contacts'
/// normals (up if none), its speed of its top speed and its rider's
/// inputs as the 360 code has them (steering positive to the right, the
/// stick's forward and back, throttle and brake).
#[derive(Clone, Copy, Debug)]
struct BikeFooting {
    wheels: usize,
    touching: usize,
    normal: Vec3,
    top: f32,
    steer: f32,
    stick: f32,
    throttle: f32,
    brake: f32,
}

/// How many physics steps a frame of `dt` seconds takes, and how long
/// each is (from the 360 code, 0x82152E20): none for a frame under 1 ms
/// (0x820F8BE4), else `dt x 59.999996 + 0.99` (0x820F8BAC, 0x82B7D824)
/// truncated and held to 1..=3, sharing the frame equally. So up to
/// 1/60 s a frame takes one step; 1/30 s, two of 1/60; longer, three.
pub fn frame_steps(dt: f32) -> (usize, f32) {
    if dt.is_nan() || dt < 0.001 {
        return (0, 0.0);
    }
    let n = ((dt * 59.999_996 + 0.99) as i32).clamp(1, 3) as usize;
    (n, dt / n as f32)
}

/// A turret's own body and the hinge it turns on, as the 360 code builds
/// them (0x82273F90, from the turret update 0x82275608): a hinge between
/// the turret part's body and its parent's about the turret's up, its
/// turn limited to its record's arc unless that goes all the way round,
/// with no motor (the yaw hinge is built unmotorised, 0x82275898). An
/// action on the turret's body (0x82273C88, stepped by 0x821A65D0) turns
/// it instead, once first asked: from its spin about its own up `w`, its
/// heading's error `e` from the hull's heading + the angle asked, the step
/// `dt`, its spin becomes `w + dt (1000 e - c w)` held to its
/// `MaxYawSpeed` either way, changed by at most its `MaxYawAccel` x `dt`,
/// where `c` is 200, or (the turn asked faster than it spins) the
/// global at 0x8380EC18, which no code writes (zero; [`TURRET_LEAD_DAMP`]).
struct TurretHinge {
    body: r::RigidBodyHandle,
    /// Its fastest turn, radians a second (`MaxYawSpeed`), and how fast
    /// its turn may change, radians a second squared (`MaxYawAccel`).
    speed: f32,
    accel: f32,
    /// The angle asked, radians (from the hull's ahead).
    target: f32,
    /// The angle asked when it was last driven; `None` before it is
    /// first asked to turn (the 360's action does nothing until then).
    last: Option<f32>,
    /// Its gun on a body of its own, hinged to the turret's
    /// ([`VehicleSpec::gun_body`]).
    gun: Option<GunHinge>,
}

/// The 360 turret action's pull on its heading's error and its damping
/// of its spin (0x821A65D0: the constants at 0x82DE8C2C and 0x820B7FB4).
const TURRET_PULL: f32 = 1000.0;
const TURRET_DAMP: f32 = 200.0;
/// Its damping while the turn asked is faster than it spins: the global
/// it reads there (0x8380EC18) is in zeroed memory and no code found
/// writes it (searched as a store, an immediate and a pointer), so 0.
const TURRET_LEAD_DAMP: f32 = 0.0;
/// How far, radians, the angle asked must be from where a sleeping
/// turret points to wake it: the 360's turret update sets its action's
/// angle every frame (0x821A64B8) and wakes its body only when the
/// square of that error is over 0.001 (the constant at 0x820F8BE4), about
/// 1.8 degrees. Closer, it and its tank sleep on.
const TURRET_WAKE: f32 = 0.031_622_78;

/// How fast, radians a second, the pitch asked of a sleeping gun on its
/// own body must move to wake it: the 360's turret update hands the gun's
/// action its pitch every frame and wakes the gun's body only when the
/// square of its pitch speed (the turret's +0x18, which its pitch aim
/// 0x822749E8 moves its pitch +0x10 by) is over 0.005 (0x820F8C6C), about
/// 0.07. Its speed here is the asked pitch's change over the step.
const GUN_WAKE: f32 = 0.070_710_68;

/// A turret's gun on a body of its own, on a hinge across the turret
/// at its trunnions, as the 360 code builds it (0x82275974 calls
/// 0x82273F90 with the pitch part, its `MinPitch`/`MaxPitch` and
/// motorised): its motor pushes it at 20 x the pitch still to go (its
/// proportional recovery, no constant one), with at most its mass x (20 +
/// 6 x its pitch acceleration); the turret update sets the motor's
/// target to the pitch asked (0x822758F4).
struct GunHinge {
    body: r::RigidBodyHandle,
    joint: r::ImpulseJointHandle,
    force: f32,
    /// The pitch asked, radians (up is positive).
    target: f32,
    /// The pitch asked when it was last driven.
    last: f32,
}

/// A boat's hull in the water: what floats it, and what its driver asks
/// of it.
struct Hull {
    /// Model space, at its origin's height: over its hull's footprint,
    /// its corners, the middles of its sides and its middle. A wreck
    /// floats on them ([`WRECK_FLOAT`]).
    points: Vec<Vec3>,
    /// Its four float springs ([`BoatFloat`]): each one's corner of its
    /// hull's box, model space, at its origin's height (bow port, bow
    /// starboard, stern port, stern starboard, as 0x82199328 sets them),
    /// and how much higher it feels the water stand for a full lean and
    /// a full bow lift, metres.
    springs: [(Vec3, f32, f32); 4],
    /// The water each spring last stood in, eased (`None` until it has
    /// stood in water).
    wet: [Option<f32>; 4],
    /// Its eased helm, lean and bow lift ([`BoatFloat`]).
    helm: f32,
    lean: f32,
    lift: f32,
    /// Its engine's eased throttle (0x82198F08), its water share and
    /// whether any spring stood in the water last step ([`float_hull`]).
    engine: f32,
    water_share: f32,
    /// How long since its hull last touched anything solid, and how long
    /// it has been touching (0x821999E0's timers), seconds; and how much
    /// of its roll it loses this step.
    clear: f32,
    touching: f32,
    stabilise: f32,
    /// Its eased handbrake (0 to 1).
    handbrake: f32,
    controls: VehicleControls,
    /// Its engine and how the water slows it: its template's, else
    /// [`BOAT`].
    water: BoatTuning,
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
    /// Downward acceleration it falls at, m/s² ([`GRAVITY`] unless its
    /// template gives its own `_HumanPhysics` `Gravity`).
    pub gravity: f32,
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
            gravity: GRAVITY,
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

/// The collision group of loose pieces: they land on the world and people
/// but not on each other (a broken prop's pieces start touching: boxes
/// round them overlap), and vehicles drive through them (round 90's tank
/// sat lifted on three pieces of what it had knocked down, two tracks off
/// the ground).
const DEBRIS: r::Group = r::Group::GROUP_3;

/// The collision group of vehicles' hulls and turrets.
const VEHICLE: r::Group = r::Group::GROUP_4;

/// The collision group of what a building broke into
/// ([`Physics::add_rubble`]): it lands on the world, and nothing else
/// meets it (people walk and vehicles drive through it, as she goes
/// through a gate's rubble in the videos; shots pass).
const RUBBLE: r::Group = r::Group::GROUP_5;

/// The collision group of goods riding loose on a vehicle's load bed
/// ([`Physics::add_cargo`]): they land on the world, on each other and on
/// the beds' own shapes ([`BED`]), not on the hulls' boxes (which fill a
/// pickup's bed to its cab roof).
const CARGO: r::Group = r::Group::GROUP_6;

/// The collision group of a vehicle's own collision shapes, its `PHY2`
/// ([`Physics::add_bed`]): only goods riding it ([`CARGO`]) meet them.
const BED: r::Group = r::Group::GROUP_7;

/// The collision group of a model's shapes no vehicle meets ([`Ghost`]):
/// sensors, which every query sees but a wheel's look for the ground.
const GHOST: r::Group = r::Group::GROUP_8;

/// The collision group of a model's leaves ([`CollisionShape::leaves`]):
/// sensors that only a path search's queries and AI sight lines see.
const LEAVES: r::Group = r::Group::GROUP_9;

/// A tracked vehicle's push at `s` of its top speed, of its most: its
/// curve's points (of its top speed, of its most) in a line between, as
/// the 360 code finds the stretch `s` is on (0x821A55F0: the first whose
/// end it hasn't reached, the last from the second point on).
fn track_push(curve: [(f32, f32); 4], s: f32) -> f32 {
    let mut i = 1;
    while i < 3 && s >= curve[i].0 {
        i += 1;
    }
    let ((a, ra), (b, rb)) = (curve[i - 1], curve[i]);
    let k = if b > a { (s - a) / (b - a) } else { 1.0 };
    (1.0 - k) * ra + k * rb
}

/// The 360 code's frame-rate-free ease (0x824BB050): `from` moves
/// towards `to` keeping `keep` of the gap each sixtieth of a second (all
/// of it at once for `keep` 0), part steps in proportion; the last bit
/// snaps.
fn ease_towards(mut from: f32, to: f32, keep: f32, mut dt: f32) -> f32 {
    const TICK: f32 = 1.0 / 60.0;
    if dt > 1000.0 * TICK {
        return if keep <= 0.9999 { to } else { from };
    }
    let take = 1.0 - keep;
    while dt > TICK {
        let gap = to - from;
        if gap * gap < (keep * 1e-5) * (keep * 1e-5) {
            return to;
        }
        dt -= TICK;
        from += gap * take;
    }
    let next = from + (to - from) * take;
    next * (dt * 60.0) + (1.0 - dt * 60.0) * from
}

/// A tracked vehicle as the 360 code drives it: its template's tuning,
/// its throttle and steering as they ease (0x330, 0x35C of the 360's
/// tank) and each track point's grip along and across now (0x84, 0x88
/// of its wheel), and how long each point's spring was last step,
/// metres (its ray's fraction, 0x30, times its length).
#[derive(Clone, Debug)]
struct Tracks {
    tuning: TankTuning,
    throttle: f32,
    steer: f32,
    /// Its driver's throttle and steering as the 360 code has them
    /// ([`TrackFooting`]), set by [`Physics::drive`] for the next step.
    asked: (f32, f32),
    grip: Vec<(f32, f32)>,
    lengths: Vec<f32>,
    rest: Rest,
}

impl Tracks {
    fn new(tuning: TankTuning, points: usize) -> Self {
        Self {
            tuning,
            throttle: 0.0,
            steer: 0.0,
            asked: (0.0, 0.0),
            grip: vec![tuning.friction; points],
            lengths: vec![tuning.spring_length(); points],
            rest: Rest::default(),
        }
    }
}

/// How a tank's body goes to sleep when it keeps still, which is all that
/// holds an empty one on a hill.
///
/// From the 360 code: its chassis body is built (0x8216A290) from the
/// rigid-body defaults (0x82621820), which leave Havok's spatial
/// deactivation on (+0xB1 = 2) at solver class 2 (+0xB2); nothing in the
/// tank's set-up (0x82194520) turns it off, as the set-up at 0x8218CAA8
/// does for its vehicle (0x82629CF0 with 1). Its tracks are an action on
/// that body, which a sleeping island doesn't step, and any non-zero
/// input to it (0x82194298) wakes the body (0x8261EEB0). An island sleeps
/// once each of its bodies has kept still for 6 checks (0x824CE9C8: a
/// body under 6 keeps it awake).
///
/// From the 360 data: the world's defaults (hkpWorldCinfo's, 0x820AA278)
/// check every 0.2 s (`highFrequencyDeactivationPeriod`) against a
/// reference distance of 0.02 m (`deactivationReferenceDistance`); the
/// second check every 10 s (`lowFrequencyDeactivationPeriod`) is left
/// out.
///
/// Inferred, not traced: that each check compares how far its centre of
/// mass has moved since the last one with that distance (the VMX check
/// itself isn't found, nor its rotation test or class scaling), and that
/// the game keeps those world defaults.
#[derive(Clone, Copy, Debug, Default)]
struct Rest {
    /// Where its centre of mass was at the last check (world).
    from: Option<Vec3>,
    /// Seconds since the last check.
    clock: f32,
    /// Checks in a row it has kept within the distance.
    still: u8,
    /// Its body was put to sleep (or found asleep).
    asleep: bool,
}

/// See [`Rest`].
const REST_PERIOD: f32 = 0.2;
const REST_DISTANCE: f32 = 0.02;
const REST_CHECKS: u8 = 6;

impl Rest {
    /// One step of `dt` s at `centre`: whether it now sleeps.
    fn step(&mut self, centre: Vec3, dt: f32) -> bool {
        let from = *self.from.get_or_insert(centre);
        self.clock += dt;
        if self.clock + 1e-6 < REST_PERIOD {
            return false;
        }
        self.clock -= REST_PERIOD;
        self.still = if (centre - from).length() < REST_DISTANCE {
            self.still.saturating_add(1)
        } else {
            0
        };
        self.from = Some(centre);
        self.still >= REST_CHECKS
    }

    /// Woken: it starts counting again.
    fn wake(&mut self) {
        *self = Self::default();
    }
}

/// Lets a tank's body sleep, when [`Rest`] finds it still, the way Havok
/// sleeps a body: with its whole island (0x8264ABF8 deactivates an island
/// only once none of its bodies is moving, 0x824D0C88), so a body joined
/// to one still moving, or touched by one, stays awake. Rapier's own
/// island sleep does that; `still` lets it sleep this body at the end of
/// the step (no speed or time of rapier's own stands in for [`Rest`]'s
/// check), otherwise it never does. A body put to sleep alone
/// (`RigidBody::sleep`) is still moved by rapier but no longer by its
/// tracks, and sinks.
fn rest_hand_off(body: &mut r::RigidBody, still: bool) {
    let (threshold, after) = if still {
        (f32::MAX, 0.0)
    } else {
        (-1.0, r::RigidBodyActivation::default_time_until_sleep())
    };
    // Only touched when it changes: each touch tells rapier its sleep
    // changed.
    if body.activation().normalized_linear_threshold != threshold {
        let a = body.activation_mut();
        a.normalized_linear_threshold = threshold;
        a.angular_threshold = threshold;
        a.time_until_sleep = after;
    }
}

/// Where one track point meets the ground (world): the hit, the ground's
/// normal there, and where its ray starts (its hardpoint, its spring's
/// squeeze up its hull).
#[derive(Clone, Copy, Debug)]
struct TrackContact {
    at: Vec3,
    normal: Vec3,
    start: Vec3,
}

/// What [`drive_tracks`] reads of a tracked vehicle each step besides its
/// spin and velocity: its attitude, its centre of mass and mass, each
/// track point's contact with the ground (point and normal, world) if it
/// touches, and its driver's throttle and steering as the 360 code has
/// them (throttle negative ahead, steering positive to the right). With
/// nobody in it both are 0, as its controls' reset (0x6C5F1491 at
/// 0x82194298) leaves them: an empty tank has no brake of its own, only
/// its sleep ([`Rest`]).
struct TrackFooting<'a> {
    turn: Quat,
    centre: Vec3,
    mass: f32,
    points: &'a [Option<TrackContact>],
    throttle: f32,
    steer: f32,
}

/// Runs a 360-driven tank's tracks for a step ([`drive_tracks`]) on the
/// rays just cast from where its body is now, as the 360 code's step
/// casts them before it springs and grips on them (0x821A49E8,
/// 0x821A5898): springing on the last step's rays a step late feeds its
/// bounce instead of damping it.
fn run_tracks(
    tank: &mut Tracks,
    controller: &DynamicRayCastVehicleController,
    bodies: &mut r::RigidBodySet,
    dt: f32,
) {
    let Some(body) = bodies.get_mut(controller.chassis) else {
        return;
    };
    let points: Vec<Option<TrackContact>> = controller
        .wheels()
        .iter()
        .map(|w| {
            let r = w.raycast_info();
            r.is_in_contact.then(|| TrackContact {
                at: vec3(r.contact_point_ws),
                normal: vec3(r.contact_normal_ws),
                start: vec3(r.hard_point_ws),
            })
        })
        .collect();
    let footing = TrackFooting {
        turn: Quat::from_array(body.position().rotation.to_array()),
        centre: vec3(body.center_of_mass()),
        mass: body.mass(),
        points: &points,
        throttle: tank.asked.0,
        steer: tank.asked.1,
    };
    let body = std::cell::RefCell::new(body);
    drive_tracks(
        tank,
        &footing,
        || {
            let b = body.borrow();
            (vec3(b.angvel()), vec3(b.linvel()))
        },
        |impulse, at| {
            let mut b = body.borrow_mut();
            match at {
                Some(p) => b.apply_impulse_at_point(vector(impulse), vector(p), true),
                None => b.apply_torque_impulse(vector(impulse), true),
            }
        },
        dt,
    );
}

/// One step of `dt` s of a tracked vehicle as the 360 code drives it (its
/// step, 0x821A49E8), in its order. `motion` reads
/// its spin and velocity afresh (world); `push` gives it an impulse,
/// linear at a point (world) or angular when the point is `None`.
///
/// - How far it leans (0x821A5BC0): level (1) until its steeper of its
///   pitch and roll passes 25 to 40 degrees (with the throttle), none
///   past 45 to 60; a level vehicle's tracks push it only across the
///   ground (their up share is dropped).
/// - Its tracks (0x821A5D30): each touching point's spring pushes it up
///   (see the loop), and its grip eases (keeping
///   95% along, 80% across, each sixtieth of a second) towards its
///   rolling grip less the throttle's share, or its brake with the
///   throttle against its motion, and across towards its side grip,
///   scaled by its high-speed grip towards its top speed; off the ground
///   none. Each pushes back mass x grip x its point's speed along (up
///   to 1 m/s) and across the hull, level (0x821A62A8).
/// - Its drive (0x821A53E0): its throttle eases (its engine inertia);
///   with a point down, the throttle not against its motion and under
///   its top speed, it pushes |eased| x throttle x its curve x torque x
///   200 N along the ground under it, at its centre of mass up the
///   ground's normal by its engine height.
/// - Its steering (0x821A5158): eased (its steering inertia, or letting
///   go to nothing with its no-input inertia whenever the stick asks less
///   or the other way), less a share at speed, it turns it by a pair of
///   pushes either side, mass x steer speed (less that share again).
/// - Its spin is damped about its own axes; points off the ground pull
///   it down by its gravity factor less 1, 1.5 m/s² each (0x821A4F10).
fn drive_tracks(
    t: &mut Tracks,
    f: &TrackFooting<'_>,
    mut motion: impl FnMut() -> (Vec3, Vec3),
    mut push: impl FnMut(Vec3, Option<Vec3>),
    dt: f32,
) {
    let tun = t.tuning;
    let (side, up, ahead) = (f.turn * Vec3::X, f.turn * Vec3::Y, f.turn * Vec3::Z);
    let pitch_of = |v: Vec3| (v.y / v.length().max(1e-6)).clamp(-1.0, 1.0).asin();
    let (thr, mass) = (f.throttle, f.mass);
    // How far it leans.
    let narrow = 20f32.to_radians();
    let wide = 45f32.to_radians() + t.throttle.abs() * 15f32.to_radians();
    let (a, b) = (pitch_of(side), pitch_of(ahead));
    let tilt = if a * a < b * b { b } else { a };
    let level = 1.0 - ((tilt - (wide - narrow)) / narrow).clamp(0.0, 1.0);
    // Its tracks.
    let (w, v) = motion();
    let top = tun.max_speed.max(1e-3);
    let s = (v.length() / top).clamp(0.0, 1.0);
    let braking = ahead.dot(v) * thr > 0.1;
    let scale = tun.high_speed_grip * s + (1.0 - s);
    let across = side.with_y(0.0);
    let mut pushes = Vec::with_capacity(f.points.len() * 2);
    let (k, c, full) = (tun.springs.0, tun.springs.1, tun.spring_length());
    for ((grip, last), point) in t.grip.iter_mut().zip(&mut t.lengths).zip(f.points) {
        let Some(TrackContact { at, start, .. }) = *point else {
            *grip = (0.0, 0.0);
            *last = full;
            continue;
        };
        // Its spring: squeezed x, it pushes mass x (strength x x, less
        // damp x how fast it stretches) x x up its hull (straight up
        // while it is level), at its ray's start.
        let length = (start - at).length().min(full);
        let x = full - length;
        let stretching = ((length - *last) / dt).max(0.0) * c;
        let force = (mass * (k * x - stretching)).max(0.0);
        let mut spring = up * (force * x * dt);
        spring.x *= 1.0 - level;
        spring.z *= 1.0 - level;
        pushes.push((spring, start));
        *last = length;
        let along = if braking {
            tun.brake
        } else {
            (1.0 - thr.abs()) * tun.friction.0
        };
        grip.0 = ease_towards(grip.0 * scale, along, 0.95, dt).max(0.0);
        grip.1 = ease_towards(grip.1 * scale, tun.friction.1, 0.8, dt).max(0.0);
        let there = v + w.cross(at - f.centre);
        let force = -ahead * (mass * grip.0 * there.dot(ahead).clamp(-1.0, 1.0))
            - across * (mass * grip.1 * there.dot(across));
        let mut impulse = force * dt;
        impulse.y *= 1.0 - level;
        pushes.push((impulse, at));
    }
    for (impulse, at) in pushes {
        push(impulse, Some(at));
    }
    // Its drive.
    let (_, v) = motion();
    let s = (v.length() / top).min(1.0);
    t.throttle = ease_towards(t.throttle, thr, tun.engine_inertia, dt).clamp(-1.0, 1.0);
    let normals: Vec<Vec3> = f.points.iter().flatten().map(|p| p.normal).collect();
    if !normals.is_empty() {
        let n = normals.iter().sum::<Vec3>() / normals.len() as f32;
        let dir = side.cross(n);
        let dir = if dir.length_squared() > 0.0 {
            dir.normalize()
        } else {
            ahead
        };
        if thr * ahead.dot(v) <= 0.1 && s < 1.0 {
            let pull = t.throttle.abs() * track_push(tun.curve, s) * tun.torque * 200.0 * thr * dt;
            push(-dir * pull, Some(f.centre + n * tun.engine_height));
        }
    }
    // Its steering.
    let (_, v) = motion();
    let loss = (1.0 - (v.length() / top).min(1.0) * tun.steer_loss).clamp(0.0, 1.0);
    let mut want = f.steer * loss;
    let mut keep = tun.steer_inertia.0;
    if want == 0.0 || t.steer * want < -0.01 || t.steer.abs() > want.abs() {
        keep = tun.steer_inertia.1;
        want = 0.0;
    }
    t.steer = ease_towards(t.steer, want, keep, dt);
    let turn = t.steer * mass * tun.steer_speed * loss * dt;
    if turn != 0.0 {
        push(-ahead * turn, Some(f.centre - side));
        push(ahead * turn, Some(f.centre + side));
    }
    // Its spin damped, and the pull on its points off the ground.
    let (w, _) = motion();
    let [dx, dy, dz] = tun.ang_damp;
    let damp = side * (w.dot(side) * dx) + up * (w.dot(up) * dy) + ahead * (w.dot(ahead) * dz);
    push(-damp * mass * dt, None);
    let off = f.points.iter().filter(|p| p.is_none()).count() as f32;
    let pull = (tun.gravity_factor - 1.0) * off * mass * -1.5;
    if pull != 0.0 {
        push(Vec3::Y * pull * dt, Some(f.centre));
    }
}

/// A motorbike's balance for a step of `dt` s, as the 360 code holds it
/// up (0x8219D800): it splits the step into sixtieths of a second (one
/// to six of them) and in each, reading its spin and velocity afresh
/// (`motion`: angular, then linear, world), `push`es it with an angular
/// and a linear impulse per kilogram (the caller times its mass). `turn`
/// is its attitude, held over the step.
///
/// - On the ground it rolls towards a lean: from its turning rate, minus
///   its blended most lean ([`BikeTuning::roll_angle`]) times its turning
///   rate over [`BikeTuning::roll_rate`] (at most 1 either way), and from
///   its steering ([`BikeTuning::roll_input`], faded in up to
///   [`BikeTuning::low_speed`]), their sum held to the larger of them;
///   upright unless both wheels touch. Strength times the lean it is off
///   (asin of its sideways axis's height) less roll damp times its roll
///   rate, about its heading, level.
/// - Rolling forwards, its nose turns towards the ground's slope (plus
///   a wheelie's pitch with the stick held and the throttle or brake
///   open), by the pitch strengths eased off towards its top speed, less
///   pitch damp; in the air its stick pitches it instead. About its
///   sideways axis, level.
/// - Its yaw is damped, about the vertical; its eased steering turns it
///   in the air (and in a wheelie on the ground).
/// - In a wheelie on the ground its side damp holds it from sliding.
fn hold_bike(
    bike: &mut Bike,
    f: BikeFooting,
    turn: Quat,
    mut motion: impl FnMut() -> (Vec3, Vec3),
    mut push: impl FnMut(Vec3, Vec3),
    dt: f32,
) {
    let t = bike.tuning;
    if !t.balances() {
        return;
    }
    let (side, up, ahead) = (turn * Vec3::X, turn * Vec3::Y, turn * Vec3::Z);
    let pitch_of = |v: Vec3| (v.y / v.length().max(1e-6)).clamp(-1.0, 1.0).asin();
    let airborne = f.touching == 0;
    // Along the ground under it, ahead.
    let along = side.cross(f.normal).normalize_or_zero();
    let gear = if f.stick > 0.0 { f.throttle } else { f.brake };
    let gear = gear * gear;
    let steps = ((dt * 60.0).ceil() as i32).clamp(1, 6);
    let dt = dt / steps as f32;
    for _ in 0..steps {
        let (w, v) = motion();
        let speed = v.with_y(0.0).length();
        let fast = if f.top > 0.0 {
            (speed / f.top).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let slow = if t.low_speed > 0.0 {
            1.0 - (speed / t.low_speed).min(1.0)
        } else {
            0.0
        };
        let (roll_rate, pitch_rate, yaw_rate) = (w.dot(ahead), w.dot(side), w.dot(up));
        if !airborne {
            let from_turn = if t.roll_rate != 0.0 {
                let most = (1.0 - fast) * t.roll_angle.1 + fast * t.roll_angle.0;
                -most * (yaw_rate / t.roll_rate).clamp(-1.0, 1.0)
            } else {
                0.0
            };
            let from_steer = (1.0 - slow) * t.roll_input * f.steer;
            let most = from_turn.abs().max(from_steer.abs());
            let want = if f.touching >= f.wheels {
                (from_turn + from_steer).clamp(-most, most)
            } else {
                0.0
            };
            let roll = (want - pitch_of(side)) * t.roll - t.damp[0] * roll_rate;
            if roll != 0.0 {
                push(ahead.with_y(0.0) * roll * dt, Vec3::ZERO);
            }
        }
        if v.dot(ahead) > 0.0 {
            let sign = if f.stick < 0.0 { -1.0 } else { 1.0 };
            let lift = ((f.stick.abs() - 0.5) * 2.0).clamp(0.0, 1.0);
            let eased = t.pitch_high_speed * fast + (1.0 - fast);
            let off = pitch_of(along) - pitch_of(ahead);
            let want = -(lift * sign * t.wheelie * gear);
            let strength = if want.abs() > off.abs() {
                t.pitch_strength.0
            } else {
                t.pitch_strength.1
            };
            let damp = t.damp[1] * pitch_rate;
            let pitch = if airborne {
                -(f.stick * t.air.0) - damp
            } else {
                strength * (want - off) * eased - damp
            };
            if pitch != 0.0 {
                push(side.with_y(0.0) * pitch * dt, Vec3::ZERO);
            }
        }
        let sign = if f.steer < 0.0 { -1.0 } else { 1.0 };
        let target = ((f.steer.abs() - 0.25) * (4.0 / 3.0)).clamp(0.0, 1.0) * sign;
        let keep = if target * target > bike.steer * bike.steer {
            0.9
        } else {
            0.0
        };
        bike.steer = ease_towards(bike.steer, target, keep, dt);
        let damp = -(w.y * t.damp[2]);
        let yaw = if airborne {
            damp - t.air.1 * bike.steer
        } else {
            // Forwards; backing up the 360 code turns it the other way
            // (0x98 clear), never in the alpha (no stick).
            damp - f.stick.abs() * t.wheelie_steer * bike.steer * gear
        };
        if yaw != 0.0 {
            push(Vec3::Y * yaw * dt, Vec3::ZERO);
        }
        if !airborne && t.side_damp != 0.0 {
            let slide = -(f.stick.abs() * t.side_damp * v.dot(side));
            push(Vec3::ZERO, side * slide * dt);
        }
    }
}

/// How fast a wheeled vehicle's driven wheels turn, rpm, as its engine
/// feels them (from the 360 code, 0x8219E678): each wheel's ground speed
/// along it (the chassis' velocity at its contact, the front's turned as
/// they steer) over its radius, weighted by its share of the drive: the
/// back's `split` x 2 over their number, the front's the rest (0x821A0390;
/// the shares add up to 2).
fn driven_rpm(
    body: &r::RigidBody,
    wheels: &[rapier3d::control::Wheel],
    steers: &[bool],
    steering: f32,
    split: f32,
) -> f32 {
    let turn = Quat::from_array(body.position().rotation.to_array());
    let (ahead, up) = (turn * Vec3::Z, turn * Vec3::Y);
    let count = |front: bool| steers.iter().filter(|&&s| s == front).count().max(1) as f32;
    let share = |front: bool| {
        if front {
            (1.0 - split) * 2.0 / count(true)
        } else {
            split * 2.0 / count(false)
        }
    };
    wheels
        .iter()
        .zip(steers)
        .map(|(w, &front)| {
            let along = if front {
                Quat::from_axis_angle(up, steering) * ahead
            } else {
                ahead
            };
            let at = w.raycast_info().contact_point_ws;
            let speed = vec3(body.velocity_at_point(at)).dot(along);
            share(front) * speed / w.radius.max(0.05) * 60.0 / std::f32::consts::TAU
        })
        .sum()
}

/// Works out how well a wheeled vehicle stands on the ground under it
/// ([`Tyres::footing`], from the lean of its wheels' contacts' mean
/// normal) and takes that share of the level part off each spring's push
/// (from the 360 code, 0x8219E020 and 0x8219C27C): on gentle ground its
/// springs push it straight up, so it stands and drives as on the flat,
/// and only nearing its steepest does the ground's lean push it down
/// the hill. Rapier has pushed along the contact normal; this takes the
/// share back at the same point, then moves what is left to the spring's
/// top, where the 360 code pushes (0x8219F510 to 0x8219C27C): a leaning
/// wheel's push turns its vehicle by how far its spring's top is off the
/// centre of mass, not its contact.
fn stand(v: &mut VehicleBody, bodies: &mut r::RigidBodySet, dt: f32) {
    let Some(tyres) = v.tyres else {
        return;
    };
    let wheels = v.controller.wheels();
    let normals: Vec<Vec3> = wheels
        .iter()
        .map(|w| w.raycast_info())
        .filter(|r| r.is_in_contact)
        .map(|r| vec3(r.contact_normal_ws))
        .collect();
    if normals.is_empty() {
        v.footing = 0.0;
        return;
    }
    let mean = normals.iter().sum::<Vec3>() / normals.len() as f32;
    let tilt = (mean.with_y(0.0).length() / mean.length().max(1e-6))
        .min(1.0)
        .asin();
    v.footing = tyres.footing(tilt);
    let Some(body) = bodies.get_mut(v.controller.chassis) else {
        return;
    };
    for w in wheels {
        let ray = w.raycast_info();
        if !ray.is_in_contact {
            continue;
        }
        let push = w.wheel_suspension_force.min(w.max_suspension_force) * dt;
        let normal = vec3(ray.contact_normal_ws);
        let level = normal.with_y(0.0) * push * v.footing;
        body.apply_impulse_at_point(vector(-level), ray.contact_point_ws, true);
        // The 360 code pushes at the spring's top, not its contact
        // (0x8219F510 hands 0x8219C27C the push and the wheel's +0x48,
        // its hardpoint in the world): what moving it there turns.
        let arm = vec3(ray.hard_point_ws) - vec3(ray.contact_point_ws);
        body.apply_torque_impulse(vector(arm.cross(normal * push - level)), true);
    }
}

/// Floats an intact boat on its four springs as the 360 code does
/// ([`BoatFloat`]; 0x82199548, 0x8219A390 and 0x82199EA8), still water
/// standing at `water` under every spring (the 360 code eases each toward
/// the highest water it samples round it; the alpha's sea has no waves).
/// Returns the share of its springs whose bottoms are under the water.
fn float_hull(hull: &mut Hull, b: &mut r::RigidBody, water: f32, touched: bool, dt: f32) -> f32 {
    use crate::tuning::ease;
    // How long since its hull touched anything solid, or how long it
    // has (0x821999E0).
    if touched {
        hull.touching += dt;
        hull.clear = 0.0;
    } else {
        hull.clear += dt;
        hull.touching = 0.0;
    }
    let f = hull.water.float;
    let c = hull.controls;
    let turn = Quat::from_array(b.position().rotation.to_array());
    let at = vec3(b.position().translation);
    let (up, bow) = (turn * Vec3::Y, turn * Vec3::Z);
    let velocity = vec3(b.linvel());
    let speed = velocity.length();
    let ahead = velocity.dot(bow);
    let share = |v: f32, of: f32| if of > 1e-4 { (v / of).min(1.0) } else { 1.0 };
    // Its helm, the way it is asked (positive to port, as the alpha's
    // and the 360 code's own messages have it) or backwards backing up.
    let backing = ahead < -(f.reverse * 0.25).min(10.0 / 3.6);
    let asked = c.steer.clamp(-1.0, 1.0) * if backing { -1.0 } else { 1.0 };
    let keep = if asked * asked >= 1e-4 {
        f.steer_inertia
    } else {
        f.steer_release
    };
    hull.helm = ease(hull.helm, asked, keep, dt);
    let lean = (1.0 - f.steer_loss * share(speed, f.top)) * hull.helm * share(speed, f.low_speed);
    hull.lean = ease(hull.lean, lean, f.roll_inertia, dt);
    let throttle = if c.parked {
        0.0
    } else {
        c.throttle.clamp(-1.0, 1.0)
    };
    let mass = b.mass();
    // Its brake and engine (0x82199B08). Its two pedals, the throttle
    // ahead and the brake (the alpha's throttle back), and its handbrake
    // (the alpha's brake): held, at once; let go, eased off keeping
    // `HandBrakeInertia`.
    let w = hull.water;
    let (pedal, brake) = (throttle.max(0.0), (-throttle).max(0.0));
    let held = if c.brake && !c.parked { 1.0 } else { 0.0 };
    hull.handbrake = if held > hull.handbrake {
        held
    } else {
        ease(hull.handbrake, held, f.handbrake_inertia, dt)
    };
    let handbrake = hull.handbrake;
    // Its bow lift toward its throttle ahead, held down by its handbrake.
    let lifting = pedal * (1.0 - handbrake);
    hull.lift = if lifting < hull.lift {
        ease(hull.lift, lifting, f.pitch_down_inertia, dt)
    } else {
        (hull.lift + f.pitch_up * dt).min(lifting)
    };
    // Going faster than 2 km/h, its brake slows it steadily, by its
    // `BrakeDrag` m/s² as much as it sits in the water: going ahead its
    // brake pedal, backing up its throttle.
    if (ahead * 3.6).powi(2) > 4.0 {
        let k = w.brake * hull.water_share * mass * dt;
        let push = if backing { pedal * k } else { -brake * k };
        b.apply_impulse(vector(bow * push), true);
    }
    // Upright, its engine's throttle eases toward the pedals' difference
    // (keeping `EngineInertia`); it pulls |that| x its curve at its speed
    // (of its top ahead, of its reverse top backing up) x its torque
    // (reverse torque when not ahead) x 200, N, x the pedals' difference
    // again, along its flat heading, as much as it sits in the water, so
    // long as it is under its top speed or pulling against its motion
    // (0x82198F08).
    let pedals = pedal - brake;
    let going_at = (ahead / if backing { -f.reverse } else { f.top }).min(1.0);
    if up.y >= 0.0 {
        hull.engine = ease(hull.engine, pedals, f.engine_inertia, dt).clamp(-1.0, 1.0);
        let e = hull.engine;
        let curve = [
            (0.0, w.curve[0]),
            (f.curve_at.0, w.curve[1]),
            (f.curve_at.1, w.curve[2]),
            (1.0, w.curve[3]),
        ];
        let torque = if e <= 0.0 { w.torque.1 } else { w.torque.0 };
        let pull = e.abs() * track_push(curve, going_at) * torque * 200.0;
        if going_at < 1.0 || pedals * ahead < 0.0 {
            // The first 1/8 s after a hit, by its `EngineTorqueMultiAtCollision`.
            let hit = if hull.clear < 0.125 {
                f.hit_torque
            } else {
                1.0
            };
            let push = pull * hit * (1.0 - handbrake) * hull.water_share * pedals * dt;
            b.apply_impulse(vector(bow.with_y(0.0) * push), true);
        }
    }
    let going = if f.top > 1e-4 {
        (ahead / f.top).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let spring = f.spring(going);
    let length = f.length(going);
    // Its helm turns it (0x8219A390), as much as it sits in the water
    // (its water share as the last step left it) or, out of it, by its
    // air control: about the world's up, by mass x step x 10 x its helm x
    // (its steering, less its high-speed loss, as fast as it goes up to
    // its low-speed limit; its handbrake's boost x its engine's throttle
    // squared x its handbrake; its no-speed steering x its engine's
    // throttle ahead x the throttle asked, held off by its handbrake and
    // gone by its low-speed limit; its low-speed
    // steering under a quarter of its reverse top speed, 10 km/h at
    // most), less its high-speed loss again.
    let slow = share(speed, f.low_speed);
    let loss = 1.0 - f.steer_loss * share(speed, f.top);
    let crawling = speed < (f.reverse * 0.25).min(10.0 / 3.6);
    let e = hull.engine;
    let reach = f.steering * loss * slow
        + f.handbrake_boost * e * e * slow * handbrake
        + f.no_speed_steering * e.max(0.0) * pedal * (1.0 - handbrake) * (1.0 - slow)
        + if crawling { f.low_speed_steering } else { 0.0 };
    let air = if touched { 0.0 } else { f.air_steering * 0.1 };
    let wetness = hull.water_share + (1.0 - hull.water_share) * air;
    let yaw = reach * hull.helm * loss * mass * wetness * 10.0 * dt;
    b.apply_torque_impulse(vector(Vec3::Y * yaw), true);
    // Its springs (0x82199EA8), each push then scaled by its water share:
    // eased (keeping 0.95 a 1/60 s) toward 1 while any spring is in the
    // water, else 0 (0x8219A248); none of them turns it about the up.
    let mut pushes = [(Vec3::ZERO, 0.0f32); 4];
    let mut wet = 0.0;
    let mut deeps = 0.0;
    for (i, &(corner, lean, lift)) in hull.springs.iter().enumerate() {
        let bottom = (at + turn * corner.with_y(spring.start)).y - up.y * length;
        let stood = hull.wet[i].map_or(water, |w| ease(w, water, f.smoothing, dt));
        hull.wet[i] = Some(stood);
        let over = stood + lean * hull.lean + lift * hull.lift - bottom;
        let deep = if length > 1e-4 {
            (over / length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        deeps += deep;
        let point = at + turn * corner;
        let rising = vec3(b.velocity_at_point(vector(point))).y;
        let push = ((over * spring.strength).max(0.0) - rising * deep * spring.damping) * mass * dt;
        if over > 0.0 {
            wet += 0.25;
        }
        pushes[i] = (point, push.max(0.0));
    }
    let target = if deeps * 0.25 > 0.0 { 1.0 } else { 0.0 };
    hull.water_share = ease(hull.water_share, target, 0.95, dt);
    let spin = vec3(b.angvel()).y;
    for (point, push) in pushes {
        if push > 0.0 {
            b.apply_impulse_at_point(
                vector(Vec3::Y * push * hull.water_share),
                vector(point),
                true,
            );
        }
    }
    let w = vec3(b.angvel());
    b.set_angvel(vector(w.with_y(spin)), true);
    // Its spin about its own roll, pitch and yaw axes dies down by its
    // damping of each, x its mass x step, as much as it sits in the water
    // (0x8219ABE8).
    let w = vec3(b.angvel());
    let x = turn * Vec3::X;
    let [roll, pitch, yaw] = f.ang_damp;
    let damp = bow * (roll * w.dot(bow)) + x * (pitch * w.dot(x)) + up * (yaw * w.dot(up));
    b.apply_torque_impulse(vector(-damp * mass * dt * hull.water_share), true);
    // Just after a hit, or out of the water and not on anything long,
    // it loses its roll: all of it at `CollisionStabilization` 1, less
    // as the hit grows old (from 1.5 s on), and less below its low-speed
    // limit; afloat and clear of things, or touching them past 0.2 s, that
    // eases away (keeping 0.9 a 1/60 s; 0x8219A7xx).
    let calm = (hull.water_share > 0.5 && hull.clear > 0.2) || hull.touching > 0.2;
    hull.stabilise = if calm {
        ease(hull.stabilise, 0.0, 0.9, dt)
    } else {
        let old = ((hull.clear - 1.5) * 2.003_48).clamp(0.0, 1.0);
        (f.stabilization * (1.0 - old) * slow).min(1.0)
    };
    let w = vec3(b.angvel());
    b.set_angvel(vector(w - bow * (w.dot(bow) * hull.stabilise)), true);
    wet
}

/// An intact boat's velocity after the water's drag for a step `dt`, as
/// the 360 code drags it (0x8219A5E0, from 0x8219A9D4): along each of
/// its own axes (its hull turned `turn`), its velocity along it x its
/// template's drag that way (`WaterDragFwd`, `WaterDragSide`,
/// `WaterDragUp`), against the motion, x the step x its share of the
/// water (an impulse of that x its mass, so a change in velocity of it).
/// On its handbrake the level part of the drag across it is lifted by its
/// `HandBrakeAntiSideDrag` x its handbrake x its share of its top speed
/// (0x8219A9F0), and its `HandBrakeMomentumKeep` x its handbrake x the
/// size of the drag across it left x the step x its share of the water
/// pushes it along its level ahead, never down (0x8219AB4C). Neither is
/// held to a range there.
fn boat_drag(hull: &Hull, turn: Quat, velocity: Vec3, dt: f32) -> Vec3 {
    let (w, f) = (hull.water, hull.water.float);
    let (x, y, z) = (turn * Vec3::X, turn * Vec3::Y, turn * Vec3::Z);
    let gone = if f.top > 1e-4 {
        (velocity.length() / f.top).min(1.0)
    } else {
        1.0
    };
    let lift = f.anti_side_drag * hull.handbrake * gone;
    let mut across = x * (w.drag[1] * velocity.dot(x));
    across -= across.with_y(0.0) * lift;
    let drag = z * (w.drag[0] * velocity.dot(z)) + across + y * (w.drag[2] * velocity.dot(y));
    let share = hull.water_share;
    let keep = f.momentum_keep * hull.handbrake * across.length() * dt * share;
    let push = (z.with_y(0.0) * keep).with_y(0.0);
    velocity - drag * dt * share + push
}

/// Pushes a wheeled vehicle along and across with each of its axles as
/// the 360 code does ([`Tyres`]; from the 360 code, at 0x8219C118 and
/// 0x821A08D8): at the mean, over its wheels on the ground, of a point
/// [`Tyres::sway`] of the way from each spring's top to its contact, by
/// mass x step x (its coefficient along x the velocity there along its
/// wheels clamped to +-1 m/s, along them, plus its coefficient across x
/// the level velocity across them, across them, level), both against the
/// motion and scaled by the share of the vehicle's wheels on the ground
/// and of the axle's. Its engine's pull (the alpha's, along its wheels,
/// for each wheel on the ground) is pushed there too. The front axle's
/// wheels point as they steer.
fn push_axles(v: &VehicleBody, bodies: &mut r::RigidBodySet, dt: f32) {
    let Some(tyres) = v.tyres else {
        return;
    };
    let Some(body) = bodies.get_mut(v.controller.chassis) else {
        return;
    };
    let wheels = v.controller.wheels();
    let down = wheels
        .iter()
        .filter(|w| w.raycast_info().is_in_contact)
        .count();
    if down == 0 {
        return;
    }
    let share = down as f32 / wheels.len() as f32;
    let turn = Quat::from_array(body.position().rotation.to_array());
    let (side, ahead, up) = (turn * Vec3::X, turn * Vec3::Z, turn * Vec3::Y);
    let fast = (vec3(body.linvel()).with_y(0.0).length() / v.top.max(0.1)).min(1.0);
    let sway = tyres.sway(fast, side.y);
    for (axle, front) in v.axles.iter().zip([true, false]) {
        let on: Vec<_> = wheels
            .iter()
            .zip(&v.steers)
            .filter(|(_, s)| **s == front)
            .map(|(w, _)| w)
            .collect();
        let grounded: Vec<_> = on
            .iter()
            .filter(|w| w.raycast_info().is_in_contact)
            .collect();
        if grounded.is_empty() {
            continue;
        }
        let at = grounded
            .iter()
            .map(|w| {
                let ray = w.raycast_info();
                let top = vec3(ray.hard_point_ws);
                top + (vec3(ray.contact_point_ws) - top) * sway
            })
            .sum::<Vec3>()
            / grounded.len() as f32;
        let steered = if front {
            Quat::from_axis_angle(up, v.steering)
        } else {
            Quat::IDENTITY
        };
        let (along, across) = (steered * ahead, steered * side);
        let level = across.with_y(0.0);
        let k = share * grounded.len() as f32 / on.len() as f32;
        let velocity = vec3(body.velocity_at_point(vector(at)));
        let (slow, slide) = if axle.hold {
            // What stops it there (its axles sharing its mass), up to
            // its coefficients.
            let stop = |v: f32, most: f32| (v / (2.0 * dt)).clamp(-most, most);
            (
                stop(velocity.dot(along), axle.along * k),
                stop(velocity.dot(level), axle.across * k),
            )
        } else {
            (
                axle.along * k * velocity.dot(along).clamp(-1.0, 1.0),
                axle.across * k * velocity.dot(level),
            )
        };
        let push = -(along * slow + level * slide) * v.mass * dt;
        body.apply_impulse_at_point(vector(push), vector(at), true);
    }
}

/// Drives a wheeled vehicle as its engine turns its wheels ([`Physics::drive`]
/// sets the torque; from the 360 code, 0x8219CAE8 to 0x8219CDEC): each
/// wheel on the ground pushes with its share of the torque over its
/// radius, along the way it points; each axle sums its wheels' pushes and
/// pushes along their mean way at their contacts' mean, by how well it
/// stands ([`Tyres::footing`]).
fn drive_axles(v: &VehicleBody, bodies: &mut r::RigidBodySet, dt: f32) {
    let Some(e) = v.engine.filter(|_| v.tyres.is_some() && v.drive != 0.0) else {
        return;
    };
    let Some(body) = bodies.get_mut(v.controller.chassis) else {
        return;
    };
    let turn = Quat::from_array(body.position().rotation.to_array());
    let (ahead, up) = (turn * Vec3::Z, turn * Vec3::Y);
    let count = |front: bool| v.steers.iter().filter(|&&s| s == front).count().max(1) as f32;
    for front in [true, false] {
        let share = if front {
            (1.0 - e.split) * 2.0 / count(true)
        } else {
            e.split * 2.0 / count(false)
        };
        let way = if front {
            Quat::from_axis_angle(up, v.steering) * ahead
        } else {
            ahead
        };
        let grounded: Vec<_> = v
            .controller
            .wheels()
            .iter()
            .zip(&v.steers)
            .filter(|(w, s)| **s == front && w.raycast_info().is_in_contact)
            .map(|(w, _)| w)
            .collect();
        if grounded.is_empty() || share == 0.0 {
            continue;
        }
        let pull: f32 = grounded
            .iter()
            .map(|w| share * v.drive / w.radius.max(0.05))
            .sum();
        let at = grounded
            .iter()
            .map(|w| vec3(w.raycast_info().contact_point_ws))
            .sum::<Vec3>()
            / grounded.len() as f32;
        let push = way * pull * v.footing * dt;
        body.apply_impulse_at_point(vector(push), vector(at), true);
    }
}

/// Queries that see everything but ragdolls, rubble and vehicles' own
/// shapes (their hulls stand for them).
fn solid<'a>() -> r::QueryFilter<'a> {
    r::QueryFilter::default().groups(r::InteractionGroups::new(
        r::Group::ALL,
        r::Group::ALL
            .difference(RAGDOLL)
            .difference(RUBBLE)
            .difference(BED)
            .difference(LEAVES),
        r::InteractionTestMode::And,
    ))
}

/// What a path search's queries and AI sight lines see: what [`solid`]
/// does, and leaves ([`CollisionShape::leaves`]: layer 19 is in
/// [`PATH_MASK`](collision_layers::PATH_MASK) and
/// [`SIGHT_MASK`](collision_layers::SIGHT_MASK)).
fn paths<'a>() -> r::QueryFilter<'a> {
    r::QueryFilter::default().groups(r::InteractionGroups::new(
        r::Group::ALL,
        r::Group::ALL
            .difference(RAGDOLL)
            .difference(RUBBLE)
            .difference(BED),
        r::InteractionTestMode::And,
    ))
}

/// Queries that see what a wheel stands on: everything but ragdolls,
/// loose pieces, rubble, goods and beds.
fn ground<'a>() -> r::QueryFilter<'a> {
    r::QueryFilter::default().groups(r::InteractionGroups::new(
        r::Group::ALL,
        r::Group::ALL
            .difference(RAGDOLL)
            .difference(DEBRIS)
            .difference(RUBBLE)
            .difference(CARGO)
            .difference(BED)
            .difference(GHOST)
            .difference(LEAVES),
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
    /// What each static collider belongs to.
    keys: HashMap<r::ColliderHandle, StaticKey>,
    /// What each static's triangles are made of ([`CollisionShape::surfaces`]).
    surfaces: HashMap<r::ColliderHandle, CollisionShape>,
    /// What of a static a path search does not see in full ([`PathSolid`]).
    path_solid: HashMap<r::ColliderHandle, PathSolid>,
    /// Each static's shapes no vehicle meets ([Ghost]).
    ghosts: HashMap<StaticKey, r::ColliderHandle>,
    /// Each static's box as a path search sizes it up ([CollisionShape::entity_box]).
    boxes: HashMap<StaticKey, (Vec3, Vec3)>,
    /// The pieces of one broken model ([`Physics::group_debris`]): each
    /// piece's body to the model's place and all its pieces' bodies.
    debris_groups: HashMap<r::RigidBodyHandle, (r::Pose, Arc<[r::RigidBodyHandle]>)>,
    /// Each static's leaves ([`CollisionShape::leaves`]).
    leaf_colliders: HashMap<StaticKey, r::ColliderHandle>,
    /// Models' collision split by a preset other than the building's
    /// ([`Physics::add_static_as`]), by their source and preset.
    preset_shapes: HashMap<(usize, Preset), Option<CollisionShape>>,
    controller: KinematicCharacterController,
    vehicles: Vec<Option<VehicleBody>>,
    ragdolls: Vec<Option<Ragdoll>>,
    /// Loose props' pieces, oldest first.
    debris: Vec<Option<r::RigidBodyHandle>>,
    /// The water's surface boats float on, if there is any.
    water: Option<f32>,
    /// The game's water heights: rivers, pools and lakes above the sea
    /// ([`Physics::water_at`]).
    inland: Option<std::sync::Arc<m2_assets::water::WaterHeights>>,
    /// How long it has run, seconds: the game's clock its helicopters'
    /// turbulence reads ([`turbulence`]).
    time: f32,
    /// Winches' cables ([`winch_cable`]).
    cables: Vec<Option<winch_cable::Cable>>,
}

/// A loose prop's piece in the physics world ([`Physics::add_debris`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DebrisId(usize);

/// Loose pieces tumbling at once at most: past this many the oldest are
/// held where they lie.
const MAX_DEBRIS: usize = 160;
/// A loose piece this far down has fallen through the island (its ground
/// streamed out) and is held.
const DEBRIS_FLOOR: f32 = -500.0;

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
                // Loose pieces lying about (a plank, a post knocked over)
                // are stepped over as a static of their height is: else
                // she can't climb a piece she can't push either.
                include_dynamic_bodies: true,
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
            keys: HashMap::new(),
            surfaces: HashMap::new(),
            path_solid: HashMap::new(),
            ghosts: HashMap::new(),
            boxes: HashMap::new(),
            debris_groups: HashMap::new(),
            preset_shapes: HashMap::new(),
            leaf_colliders: HashMap::new(),
            controller,
            vehicles: Vec::new(),
            ragdolls: Vec::new(),
            debris: Vec::new(),
            water: None,
            inland: None,
            time: 0.0,
            cables: Vec::new(),
        }
    }

    /// Sets the height of the water boats float on (the sea).
    pub fn set_water(&mut self, level: f32) {
        self.water = Some(level);
    }

    /// The height of the water boats float on, if there is any.
    pub fn water(&self) -> Option<f32> {
        self.water
    }

    /// Sets the game's water heights (rivers, pools, lakes above the sea).
    pub fn set_inland(&mut self, heights: std::sync::Arc<m2_assets::water::WaterHeights>) {
        self.inland = Some(heights);
    }

    /// The water's surface over (`x`, `z`): the higher of the sea and the
    /// water heights' surface there (a pool, a river), if either.
    pub fn water_at(&self, x: f32, z: f32) -> Option<f32> {
        let inland = self.inland.as_ref().and_then(|w| w.surface(x, z));
        match (self.water, inland) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
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
        let mut collider =
            r::ColliderBuilder::new(shape.shape.clone()).position(pose(position, rotation));
        // A model's own fixed body is made as a moving one is (the 360
        // body builder gives a body of no mass motion type 7, fixed, with
        // the same friction and no bounce); one without keeps Rapier's.
        if let Some(m) = shape.material {
            collider = collider.friction(m.friction).restitution(m.restitution);
        }
        let collider = collider.build();
        let handle = insert_collider(&mut self.world, collider, None);
        self.statics.insert(key, handle);
        self.keys.insert(handle, key);
        if !shape.surfaces.is_empty() {
            self.surfaces.insert(handle, shape.clone());
        }
        if !matches!(shape.path, PathSolid::All) {
            self.path_solid.insert(handle, shape.path.clone());
        }
        if let Some(b) = shape.entity_box {
            self.boxes.insert(key, b);
        }
        if let Some(g) = &shape.ghost {
            let collider = r::ColliderBuilder::new(g.shape.clone())
                .position(pose(position, rotation))
                .sensor(true)
                .collision_groups(r::InteractionGroups::new(
                    GHOST,
                    r::Group::ALL,
                    r::InteractionTestMode::And,
                ))
                .build();
            let h = insert_collider(&mut self.world, collider, None);
            self.ghosts.insert(key, h);
            self.keys.insert(h, key);
            if !matches!(g.path, PathSolid::All) {
                self.path_solid.insert(h, g.path.clone());
            }
        }
        if let Some(leaves) = &shape.leaves {
            let collider = r::ColliderBuilder::new(leaves.clone())
                .position(pose(position, rotation))
                .sensor(true)
                .collision_groups(r::InteractionGroups::new(
                    LEAVES,
                    r::Group::ALL,
                    r::InteractionTestMode::And,
                ))
                .build();
            let h = insert_collider(&mut self.world, collider, None);
            self.leaf_colliders.insert(key, h);
            self.keys.insert(h, key);
        }
    }

    /// [`add_static`](Self::add_static) for a body of `preset` (its
    /// template's, [`Preset`]): `shape` split by that preset's layers
    /// ([`CollisionShape::as_preset`]), made once for each model and
    /// preset and kept.
    pub fn add_static_as(
        &mut self,
        key: StaticKey,
        shape: &CollisionShape,
        position: Vec3,
        rotation: Quat,
        preset: Preset,
    ) {
        let Some(src) = shape.source.as_ref().filter(|_| preset != Preset::Building) else {
            return self.add_static(key, shape, position, rotation);
        };
        let id = (Arc::as_ptr(src) as usize, preset);
        let made = self
            .preset_shapes
            .entry(id)
            .or_insert_with(|| shape.as_preset(preset))
            .clone();
        match made {
            Some(s) => self.add_static(key, &s, position, rotation),
            None => self.add_static(key, shape, position, rotation),
        }
    }

    pub fn remove_static(&mut self, key: StaticKey) {
        let ghost = self.ghosts.remove(&key);
        let leaves = self.leaf_colliders.remove(&key);
        self.boxes.remove(&key);
        if let Some(h) = self.statics.remove(&key) {
            for h in std::iter::once(h).chain(ghost).chain(leaves) {
                self.keys.remove(&h);
                self.surfaces.remove(&h);
                self.path_solid.remove(&h);
                self.world.remove_collider(h);
            }
        }
    }

    pub fn has_static(&self, key: StaticKey) -> bool {
        self.statics.contains_key(&key)
    }

    pub fn static_count(&self) -> usize {
        self.statics.len()
    }

    /// Advances the simulation by a frame of `dt` seconds as the 360 does
    /// (0x82152E20): in [`frame_steps`] steps of equal length, none for a
    /// frame under a millisecond.
    pub fn step_frame(&mut self, dt: f32) {
        let (n, each) = frame_steps(dt);
        for _ in 0..n {
            self.step(each);
        }
    }

    /// Advances the simulation by `dt` seconds (and brings colliders added
    /// since the last step into scene queries).
    pub fn step(&mut self, dt: f32) {
        self.world.integration_parameters.dt = dt;
        self.time += dt;
        let w = &mut self.world;
        for v in self.vehicles.iter_mut().flatten() {
            v.before = w
                .bodies
                .get(v.controller.chassis)
                .map_or(Vec3::ZERO, |b| vec3(b.linvel()));
            if v.held {
                continue;
            }
            // A tank's body sleeps once it keeps still, and then nothing
            // of it runs until it is woken: by its input ([`Physics::drive`])
            // or by something touching it. Woken, it counts again
            // (0x8261EEB0 clears its counts).
            if let Some(tank) = &mut v.tank
                && let Some(b) = w.bodies.get_mut(v.controller.chassis)
            {
                if b.is_sleeping() {
                    // Asleep with its island: it stays so until something
                    // wakes the island (no sleep of rapier's own after).
                    tank.rest.asleep = true;
                    rest_hand_off(b, false);
                    continue;
                }
                if tank.rest.asleep {
                    tank.rest.wake();
                }
                // Still long enough: its island may sleep at the end of
                // this step, if every body in it may (its turret and
                // gun on their joints, anything resting on it). Its body
                // is never put to sleep alone: rapier would go on moving
                // it (gravity) while its tracks no longer hold it up
                // (PmcCon001: the villa tank fell through the HQ's floor).
                let still = !v.wrecked && tank.rest.step(vec3(b.center_of_mass()), dt);
                rest_hand_off(b, still);
            }
            let filter = ground().exclude_rigid_body(v.controller.chassis);
            let queries = w.broad_phase.as_query_pipeline_mut(
                w.narrow_phase.query_dispatcher(),
                &mut w.bodies,
                &mut w.colliders,
                filter,
            );
            v.controller.update_vehicle(dt, queries);
            if !v.wrecked {
                stand(v, &mut w.bodies, dt);
                if let Some(tank) = &mut v.tank {
                    run_tracks(tank, &v.controller, &mut w.bodies, dt);
                }
                push_axles(v, &mut w.bodies, dt);
                drive_axles(v, &mut w.bodies, dt);
            }
            if v.car_hull
                && !v.wrecked
                && let Some(b) = w.bodies.get(v.controller.chassis)
            {
                let up = Quat::from_array(b.position().rotation.to_array()) * Vec3::Y;
                let friction = car_hull_friction(up.y);
                for &h in b.colliders() {
                    if let Some(c) = w.colliders.get_mut(h)
                        && c.friction() != friction
                    {
                        c.set_friction(friction);
                    }
                }
            }
            // More than half its wheels down, it is pulled down by its
            // template's `GravityFactor` less one of its weight more
            // (from the 360 code, at 0x8219EE14, by its gravity of 10 m/s²,
            // [`GRAVITY_360`]).
            let wheels = v.controller.wheels();
            let down = wheels
                .iter()
                .filter(|w| w.raycast_info().is_in_contact)
                .count();
            if !v.wrecked && v.gravity_factor != 1.0 && down * 2 > wheels.len() {
                let pull = (v.gravity_factor - 1.0) * v.mass * GRAVITY_360 * dt;
                if let Some(b) = w.bodies.get_mut(v.controller.chassis) {
                    b.apply_impulse(r::Vector::new(0.0, -pull, 0.0), true);
                }
            }
        }
        self.drive_turrets(dt);
        self.float_boats(dt);
        self.fly_helicopters(dt);
        self.cap_debris_speeds();
        self.world.step_with_events(&HavokFriction, &());
        self.hold_ragdolls_together();
        self.settle_debris();
    }

    /// Floats boats on the water ([`set_water`](Self::set_water)) and
    /// drives them as asked. An intact boat floats on its four springs as
    /// the 360 code floats it ([`BoatFloat`]); a wreck on the points of
    /// its waterline under the surface, each lifting it as deep as it is
    /// (its whole weight with its origin [`FLOAT_DEPTH`] under, never more
    /// than [`WRECK_FLOAT`] of it; invented). In the water a boat is
    /// pushed and steered, slowed along, across and up and down as its
    /// template's drag says ([`BoatTuning`]).
    fn float_boats(&mut self, dt: f32) {
        let Some(water) = self.water else {
            return;
        };
        for v in self.vehicles.iter_mut().flatten() {
            let held = v.held;
            let Some(hull) = v.hull.as_mut().filter(|_| !held) else {
                continue;
            };
            let touched = self
                .world
                .bodies
                .get(v.controller.chassis)
                .is_some_and(|b| {
                    b.colliders().iter().any(|&c| {
                        self.world
                            .narrow_phase
                            .contact_pairs_with(c)
                            .any(|p| p.has_any_active_contact())
                    })
                });
            let Some(b) = self.world.bodies.get_mut(v.controller.chassis) else {
                continue;
            };
            let place = *b.position();
            let turn = Quat::from_array(place.rotation.to_array());
            let at = vec3(place.translation);
            let ahead = (turn * Vec3::Z).with_y(0.0).normalize_or_zero();
            let wet = if v.wrecked {
                let n = hull.points.len().max(1) as f32;
                let lift = GRAVITY * v.mass / n * dt;
                let mut wet = 0.0;
                for &q in &hull.points {
                    let p = at + turn * q;
                    let depth = water - p.y;
                    if depth > 0.0 {
                        wet += 1.0;
                        let up = lift * (depth / FLOAT_DEPTH).min(WRECK_FLOAT);
                        b.apply_impulse_at_point(vector(Vec3::Y * up), vector(p), true);
                    }
                }
                wet / n
            } else {
                float_hull(hull, b, water, touched, dt)
            };
            if wet == 0.0 {
                continue;
            }
            let ease = |rate: f32| 1.0 - (rate * dt * wet).min(1.0);
            let velocity = vec3(b.linvel());
            let w = hull.water;
            if v.wrecked {
                // A wreck is slowed along, across and up and down as its
                // template's drag says, level (invented: the 360's wreck
                // has no boat action).
                let side = Vec3::new(ahead.z, 0.0, -ahead.x);
                let along = velocity.dot(ahead) * ease(w.drag[0]);
                let across = velocity.dot(side) * ease(w.drag[1]);
                let rise = velocity.y * ease(w.drag[2]);
                b.set_linvel(vector(ahead * along + side * across + Vec3::Y * rise), true);
            } else {
                b.set_linvel(vector(boat_drag(hull, turn, velocity, dt)), true);
            }
            // An intact boat's helm turned it with its springs
            // ([`float_hull`]); a wreck stops turning and rocking
            // (invented).
            if v.wrecked {
                let spin = vec3(b.angvel());
                let up = turn * Vec3::Y;
                let yaw = spin.dot(up) * (1.0 - (3.0 * dt * wet).min(1.0));
                let rock = (spin - up * spin.dot(up)) * ease(BOAT_ROCK);
                b.set_angvel(vector(rock + up * yaw), true);
            }
        }
    }

    /// Flies helicopters as the 360 code's `_HelicopterPhysics` does
    /// ([`Rotor`]): the climb held spins its rotor up, and only at full
    /// speed does it hold its weight and climb; it rolls and pitches to
    /// the stick, pulls towards the speed asked and turns at the rate
    /// asked. The stick: the throttle ahead, the steering (positive left,
    /// as the wheels) its turn, [`fly`](Self::fly) its climb; it has no
    /// sideways slide. Nobody flying it, its stick is let go (as the
    /// game's message 0x6C5F1491 zeroes it): in the air it hovers, on
    /// the ground its rotor winds down. Wrecked, its rotor winds down
    /// and it falls.
    fn fly_helicopters(&mut self, dt: f32) {
        for v in self.vehicles.iter_mut().flatten() {
            let Some(r) = v.rotor.as_mut().filter(|_| !v.held) else {
                continue;
            };
            let Some(b) = self.world.bodies.get(v.controller.chassis) else {
                continue;
            };
            let touching = b.colliders().iter().any(|&c| {
                self.world
                    .narrow_phase
                    .contact_pairs_with(c)
                    .any(|p| p.has_any_active_contact())
            });
            let c = r.controls;
            let stick = if v.wrecked || c.parked {
                Stick::default()
            } else {
                Stick {
                    lift: r.lift.clamp(-1.0, 1.0),
                    yaw: -c.steer.clamp(-1.0, 1.0),
                    ahead: c.throttle.clamp(-1.0, 1.0),
                    right: r.strafe.clamp(-1.0, 1.0),
                }
            };
            r.state.spin(stick, v.wrecked, touching, dt);
            let Some(b) = self.world.bodies.get_mut(v.controller.chassis) else {
                continue;
            };
            let p = b.position();
            // An AI at the stick flies by its AI handling and wobbles.
            let flight = if r.ai {
                r.flight_ai.unwrap_or(r.flight)
            } else {
                r.flight
            };
            let shake = if r.ai && !v.wrecked {
                ai_wobble(self.time, r.state.ground)
            } else {
                [0.0; 3]
            };
            let flown = r.state.fly(
                &flight,
                stick,
                shake,
                Quat::from_array(p.rotation.to_array()),
                vec3(b.linvel()),
                vec3(b.angvel()),
                p.translation.y,
                b.mass(),
                GRAVITY,
                dt,
            );
            if let Some((x, z)) = flown.spin_across {
                let w = vec3(b.angvel());
                b.set_angvel(vector(Vec3::new(x, w.y, z)), true);
            }
            b.apply_impulse(vector(flown.impulse), true);
            b.apply_torque_impulse(vector(Vec3::Y * flown.torque_y * dt), true);
            // Shaken by the air as its rotor turns, as much as it is
            // off the ground: a force at its centre of mass (0x821A31FC
            // hands Havok's applyForce its velocity change, F dt / mass).
            let (a, bb) = turbulence(
                r.phase,
                self.time,
                r.state.spin,
                r.state.air,
                r.state.ground,
            );
            let turn = Quat::from_array(b.position().rotation.to_array());
            let flat = |v: Vec3| v.with_y(0.0).normalize_or_zero();
            let push =
                (flat(turn * Vec3::X) * bb + flat(turn * Vec3::Z) * a) * b.mass() * TURBULENCE_PUSH;
            b.apply_impulse(vector(push * dt), true);
        }
    }

    /// Holds loose pieces fallen through the island, and the oldest past
    /// [`MAX_DEBRIS`] still tumbling.
    fn settle_debris(&mut self) {
        let mut tumbling = 0;
        for h in self.debris.iter().rev().flatten() {
            let Some(b) = self.world.bodies.get_mut(*h) else {
                continue;
            };
            if !b.is_dynamic() {
                continue;
            }
            tumbling += 1;
            if tumbling > MAX_DEBRIS || b.translation().y < DEBRIS_FLOOR {
                b.set_body_type(r::RigidBodyType::Fixed, false);
            }
        }
    }

    /// Holds loose pieces to Havok's caps on a body's speeds, as the game
    /// sets them ([`MAX_LINEAR_SPEED`], [`MAX_ANGULAR_SPEED`]), before
    /// they move by them.
    fn cap_debris_speeds(&mut self) {
        for h in self.debris.iter().flatten() {
            let Some(b) = self.world.bodies.get_mut(*h) else {
                continue;
            };
            if !b.is_dynamic() {
                continue;
            }
            let (v, w) = (vec3(b.linvel()), vec3(b.angvel()));
            if v.length() > MAX_LINEAR_SPEED {
                b.set_linvel(vector(v.clamp_length_max(MAX_LINEAR_SPEED)), true);
            }
            if w.length() > MAX_ANGULAR_SPEED {
                b.set_angvel(vector(w.clamp_length_max(MAX_ANGULAR_SPEED)), true);
            }
        }
    }

    /// Adds a loose piece where `at` (world space; scale ignored) puts it,
    /// flying at `velocity` (m/s) and turning at `spin` (radians a second
    /// about each axis).
    pub fn add_debris(
        &mut self,
        piece: &LoosePiece,
        at: Mat4,
        velocity: Vec3,
        spin: Vec3,
    ) -> DebrisId {
        self.add_falling_debris(piece, at, (velocity, spin), 1.0)
    }

    /// As [`add_debris`](Self::add_debris), pulled by `gravity` times the
    /// world's gravity (a weapon lying to be taken: its template's
    /// `PhysicsPropertyGravityScaler`).
    pub fn add_falling_debris(
        &mut self,
        piece: &LoosePiece,
        at: Mat4,
        (velocity, spin): (Vec3, Vec3),
        gravity: f32,
    ) -> DebrisId {
        let id = self.add_piece(
            piece,
            at,
            velocity,
            spin,
            r::InteractionGroups::new(
                DEBRIS,
                r::Group::ALL.difference(DEBRIS).difference(VEHICLE),
                r::InteractionTestMode::And,
            ),
        );
        if gravity != 1.0
            && let Some(b) = self.debris[id.0].and_then(|h| self.world.bodies.get_mut(h))
        {
            b.set_gravity_scale(gravity, true);
        }
        id
    }

    /// As [`add_debris`](Self::add_debris), a piece a building broke
    /// into: [`RUBBLE`], which lands on the world and nothing else.
    pub fn add_rubble(
        &mut self,
        piece: &LoosePiece,
        at: Mat4,
        velocity: Vec3,
        spin: Vec3,
    ) -> DebrisId {
        self.add_piece(
            piece,
            at,
            velocity,
            spin,
            r::InteractionGroups::new(
                RUBBLE,
                r::Group::ALL
                    .difference(RUBBLE)
                    .difference(DEBRIS)
                    .difference(VEHICLE)
                    .difference(RAGDOLL),
                r::InteractionTestMode::And,
            ),
        )
    }

    /// As [`add_debris`](Self::add_debris), a piece of goods riding loose
    /// on a vehicle's load bed ([`CARGO`]): it lies on the bed's own shapes
    /// ([`add_bed`](Self::add_bed)) and slides, tips and falls off as the
    /// vehicle turns, brakes and hits, held there by friction alone (its
    /// [`LoosePiece::material`] and the bed's).
    pub fn add_cargo(&mut self, piece: &LoosePiece, at: Mat4, velocity: Vec3) -> DebrisId {
        self.add_piece(
            piece,
            at,
            velocity,
            Vec3::ZERO,
            r::InteractionGroups::new(
                CARGO,
                r::Group::ALL.difference(VEHICLE).difference(RUBBLE),
                r::InteractionTestMode::And,
            ),
        )
    }

    /// Puts vehicle `id`'s own collision, `shape` (its `PHY2`, model
    /// space), on it for the goods riding it to lie on ([`BED`]): a
    /// pickup's bed floor and sides, which its hull's box doesn't have.
    /// Nothing but goods meets it, and it adds nothing to the mass. Once a
    /// vehicle; false if it isn't placed.
    ///
    /// It holds them by the friction and bounce of the vehicle's own body
    /// ([`CollisionShape::material`]; the commonest body's without one).
    pub fn add_bed(&mut self, id: VehicleId, shape: &CollisionShape) -> bool {
        let Some(v) = self.vehicles.get_mut(id.0).and_then(Option::as_mut) else {
            return false;
        };
        if !v.bed {
            v.bed = true;
            let bed = shape.material.unwrap_or(BodyMaterial::COMMON);
            let collider = r::ColliderBuilder::new(shape.shape.clone())
                .density(0.0)
                .friction(bed.friction)
                .friction_combine_rule(r::CoefficientCombineRule::Min)
                .restitution(bed.restitution)
                .collision_groups(r::InteractionGroups::new(
                    BED,
                    CARGO,
                    r::InteractionTestMode::And,
                ));
            let chassis = v.controller.chassis;
            insert_collider(&mut self.world, collider, Some(chassis));
        }
        true
    }

    fn add_piece(
        &mut self,
        piece: &LoosePiece,
        at: Mat4,
        velocity: Vec3,
        spin: Vec3,
        groups: r::InteractionGroups,
    ) -> DebrisId {
        let (_, rotation, translation) = at.to_scale_rotation_translation();
        let body = self.world.insert_body(
            r::RigidBodyBuilder::dynamic()
                .pose(pose(translation, rotation.normalize()))
                .linvel(vector(velocity))
                .angvel(vector(spin))
                .linear_damping(piece.material.linear_damping)
                .angular_damping(piece.material.angular_damping)
                .ccd_enabled(true),
        );
        let collider = r::ColliderBuilder::new(piece.shape.clone())
            .mass(piece.mass)
            .friction(piece.material.friction)
            .restitution(piece.material.restitution)
            .collision_groups(groups);
        insert_collider(&mut self.world, collider, Some(body));
        self.debris.push(Some(body));
        DebrisId(self.debris.len() - 1)
    }

    /// Where a loose piece is now (its node's place, world space).
    pub fn debris_place(&self, id: DebrisId) -> Option<Mat4> {
        let h = (*self.debris.get(id.0)?)?;
        let p = self.world.bodies.get(h)?.position();
        Some(Mat4::from_rotation_translation(
            Quat::from_array(p.rotation.to_array()),
            vec3(p.translation),
        ))
    }

    /// How a loose piece is moving now: its velocity (m/s) and its spin
    /// (radians a second about each world axis).
    pub fn debris_velocity(&self, id: DebrisId) -> Option<(Vec3, Vec3)> {
        let h = (*self.debris.get(id.0)?)?;
        let b = self.world.bodies.get(h)?;
        Some((vec3(b.linvel()), vec3(b.angvel())))
    }

    /// Puts a loose piece at `at` (world space), at rest.
    pub fn move_debris(&mut self, id: DebrisId, at: Mat4) {
        let Some(b) = self
            .debris
            .get(id.0)
            .copied()
            .flatten()
            .and_then(|h| self.world.bodies.get_mut(h))
        else {
            return;
        };
        let (_, rotation, translation) = at.to_scale_rotation_translation();
        b.set_position(pose(translation, rotation.normalize()), true);
        b.set_linvel(vector(Vec3::ZERO), true);
        b.set_angvel(vector(Vec3::ZERO), true);
    }

    /// Marks `pieces` as one broken model placed at `at` (world space;
    /// scale ignored). From the 360 code: a model whose state changes has
    /// its physics rebuilt for the parts it now shows (0x8222D2F0 sets the
    /// state, 0x821C7648, and queues 0x82182CE8; 0x82152780 tears it down
    /// and sets it up again, 0x821831B0), so its pieces stay bodies of its
    /// entity, and a path search sizes any of them up by the box round all
    /// of them in the entity's frame (0x821861B0, 0x82182E90).
    pub fn group_debris(&mut self, pieces: &[DebrisId], at: Mat4) {
        let (_, rotation, translation) = at.to_scale_rotation_translation();
        let place = pose(translation, rotation.normalize());
        let bodies: Arc<[r::RigidBodyHandle]> = pieces
            .iter()
            .filter_map(|d| self.debris.get(d.0).copied().flatten())
            .collect();
        for &b in bodies.iter() {
            self.debris_groups.insert(b, (place, bodies.clone()));
        }
    }

    /// The box round the pieces of the broken model `body` is one of
    /// ([`group_debris`](Self::group_debris)), in its frame, and its place.
    fn debris_box(&self, body: r::RigidBodyHandle) -> Option<(r::Pose, r::Aabb)> {
        let (place, bodies) = self.debris_groups.get(&body)?;
        let mut out: Option<r::Aabb> = None;
        for &b in bodies.iter() {
            let Some(rb) = self.world.bodies.get(b) else {
                continue;
            };
            for &h in rb.colliders() {
                let Some(c) = self.world.colliders.get(h) else {
                    continue;
                };
                let a = c.shape().compute_aabb(&place.inv_mul(c.position()));
                out = Some(match out {
                    Some(o) => rapier3d::geometry::BoundingVolume::merged(&o, &a),
                    None => a,
                });
            }
        }
        Some((*place, out?))
    }

    pub fn remove_debris(&mut self, id: DebrisId) {
        if let Some(h) = self.debris.get(id.0).copied().flatten() {
            self.debris_groups.remove(&h);
        }
        if let Some(h) = self.debris.get_mut(id.0).and_then(Option::take) {
            self.world.remove_body_with_colliders(h, true);
        }
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
    /// Builds a turret's gun as a body of its own ([`GunHinge`]) on the
    /// turret's body, hinged across at its trunnions (`trunnion`, model
    /// space): within its template's pitch limits, its motor as strong as
    /// its mass x (20 + 6 x its pitch acceleration). The vehicle stands at
    /// `position`, turned `rotation`.
    #[allow(clippy::too_many_arguments)]
    fn add_gun(
        &mut self,
        gun: &crate::vehicle::GunBody,
        trunnion: Vec3,
        turret: r::RigidBodyHandle,
        position: Vec3,
        rotation: Quat,
        tuning: Option<crate::tuning::TurretTuning>,
        groups: r::InteractionGroups,
        friction: f32,
    ) -> GunHinge {
        let m = gun.mass;
        let body = self.world.insert_body(
            r::RigidBodyBuilder::dynamic()
                .pose(pose(position, rotation))
                .linear_damping(BodyMaterial::COMMON.linear_damping)
                .angular_damping(BodyMaterial::COMMON.angular_damping)
                .ccd_enabled(true),
        );
        let props = r::MassProperties::with_inertia_matrix(
            vector(m.centre_of_mass),
            m.mass,
            rapier3d::math::Matrix::from_cols_array(
                &((m.inertia + m.inertia.transpose()) * 0.5).to_cols_array(),
            ),
        );
        let shapes = gun.shapes.iter().filter_map(|v| {
            let points: Vec<_> = v.iter().map(|&p| vector(p)).collect();
            r::SharedShape::convex_hull(&points)
        });
        for (i, shape) in shapes.enumerate() {
            let c = r::ColliderBuilder::new(shape)
                .friction(friction)
                .friction_combine_rule(r::CoefficientCombineRule::Min)
                .collision_groups(groups);
            let c = if i == 0 {
                c.mass_properties(props)
            } else {
                c.density(0.0)
            };
            insert_collider(&mut self.world, c, Some(body));
        }
        let force = m.mass * (20.0 + 6.0 * tuning.map_or(0.0, |t| t.pitch_accel));
        // About +x (the pitch axis's default, `X_AXIS`), so a pitch up is
        // a turn by minus it: its limits `MinPitch`..`MaxPitch` the other
        // way round.
        let mut joint = r::RevoluteJointBuilder::new(r::Vector::X)
            .local_anchor1(vector(trunnion))
            .local_anchor2(vector(trunnion))
            .contacts_enabled(false)
            .motor_model(r::MotorModel::ForceBased)
            .motor_max_force(force);
        if let Some((lo, hi)) = tuning.map(|t| t.pitch)
            && hi - lo < std::f32::consts::TAU - 0.01
        {
            joint = joint.limits([-hi, -lo]);
        }
        let joint = self.world.insert_impulse_joint(turret, body, joint);
        GunHinge {
            body,
            joint,
            force,
            target: 0.0,
            last: 0.0,
        }
    }

    pub fn add_vehicle(&mut self, spec: &VehicleSpec, position: Vec3, yaw: f32) -> VehicleId {
        self.add_vehicle_turned(spec, position, Quat::from_rotation_y(yaw))
    }

    /// [`add_vehicle`](Self::add_vehicle) turned by `rotation`: tilted to
    /// the ground it rests on ([`VehicleSpec::rest_on`]).
    pub fn add_vehicle_turned(
        &mut self,
        spec: &VehicleSpec,
        position: Vec3,
        rotation: Quat,
    ) -> VehicleId {
        let (lo, hi) = spec.hull;
        // The hull clears the ground by the wheels' lower half.
        let floor = spec
            .wheels
            .iter()
            .map(|w| w.centre.y)
            .fold(lo.y, f32::max)
            .min(hi.y - 0.2);
        let mut half = Vec3::new(
            (hi.x - lo.x) * 0.5,
            (hi.y - floor) * 0.5,
            (hi.z - lo.z) * 0.5,
        )
        .max(Vec3::splat(0.1));
        if spec.is_helicopter() {
            half.x = half.x.min(FUSELAGE_HALF);
        }
        let centre = Vec3::new((lo.x + hi.x) * 0.5, floor + half.y, (lo.z + hi.z) * 0.5);
        // Swept, as loose pieces are: at speed a step carries it further
        // than a fence or a post is thick, and the hull would end up
        // inside one (a static's triangles have no inside to push it out
        // of the way it came).
        let body = self.world.insert_body(
            r::RigidBodyBuilder::dynamic()
                .pose(pose(position, rotation))
                // Its chassis body's damping as the 360 body builder gives
                // it ([`BodyMaterial::of`]): none against its motion (the
                // rigid-body defaults', 0x82621820: +0x94 0), and its
                // record's against its spin, at least 0.8. A boat's water
                // drag slows it besides ([`Physics::float_boats`]). No set-up
                // of a car, tank, boat or helicopter calls a body setter
                // but its mass (0x82629D80) and inertia; one inlined in
                // them isn't ruled out. Without a `PHY2` chassis, the
                // commonest record's.
                .linear_damping(spec.material.unwrap_or(BodyMaterial::COMMON).linear_damping)
                .angular_damping(
                    spec.material
                        .unwrap_or(BodyMaterial::COMMON)
                        .angular_damping,
                )
                .can_sleep(false)
                .ccd_enabled(true),
        );
        // A tank's, boat's or helicopter's: its chassis body's, as the 360
        // code sets it (see [`chassis_mass`]). A car's is its template's
        // `Mass`, else its `PHY2` chassis record's: the body builder keeps
        // the record's mass when handed none above 0 (0x82182848). Only a
        // model with neither (no template, no `PHY2` chassis) keeps the
        // invented mass by kind. Its inertia, without a chassis body: a
        // little above a solid box's, invented.
        let chassis = spec
            .chassis
            .as_ref()
            .filter(|_| spec.tracked || spec.wheels.is_empty())
            .map(|c| chassis_mass(c, spec.tuning.mass));
        let m = chassis
            .map(|c| c.0)
            .or(spec.tuning.mass)
            .or(spec.chassis.map(|c| c.mass).filter(|m| *m > 0.0))
            .unwrap_or(spec.mass);
        let size = half * 2.0;
        let inertia = Vec3::new(
            size.y * size.y + size.z * size.z,
            size.x * size.x + size.z * size.z,
            size.x * size.x + size.y * size.y,
        ) * (m / 12.0 * 1.5);
        // A car's or motorbike's (its `_CarPhysicsV2`'s) is its template's
        // per kilogram times its mass, as the 360 code sets every car's
        // body (0x8219BA08), and its centre of mass the middle of its
        // wheels plus its template's offset (0x8219BABC). The point it
        // takes of each wheel (its 0xE0) is its hardpoint raised by its
        // axle's `SuspCmpLength` (0x8219EF90): the top of its spring
        // here, its squeeze above the model's wheel.
        let bike = spec.is_bike().then(|| spec.tuning.bike.unwrap_or(BIKE));
        let car_body = bike
            .map(|b| crate::tuning::CarBody {
                inertia: b.inertia,
                centre_of_mass: b.centre_of_mass,
            })
            .or(spec.tuning.body)
            .filter(|_| !spec.tracked && !spec.wheels.is_empty());
        let inertia = car_body.map_or(inertia, |b| Vec3::from(b.inertia) * m);
        let com = match car_body {
            Some(b) => {
                let springs = spec.tuning.springs.unwrap_or(Springs::DEFAULT);
                let mid = spec
                    .wheels
                    .iter()
                    .map(|w| w.centre + Vec3::Y * springs.axle(w.steers).3)
                    .sum::<Vec3>()
                    / spec.wheels.len() as f32;
                mid + Vec3::from(b.centre_of_mass) - centre
            }
            None => match spec.chassis.as_ref().filter(|_| chassis.is_some()) {
                // A tank's moves by its template's `CenterOfMassOffset`
                // (0x821A4D40 -> 0x8262A680); a boat's or helicopter's stays.
                Some(c) => {
                    let offset = spec
                        .tuning
                        .tank
                        .filter(|_| spec.tracked)
                        .map_or(Vec3::ZERO, |t| Vec3::from(t.centre_of_mass));
                    c.centre_of_mass + offset - centre
                }
                None => Vec3::new(0.0, -half.y * 0.8, 0.0),
            },
        };
        let mass = match chassis {
            // Rapier takes it symmetric; a clipped axis leaves it a touch
            // off (none of the 57 chassis bodies: their axes are within 14x).
            Some((_, i)) => r::MassProperties::with_inertia_matrix(
                vector(com),
                m,
                rapier3d::math::Matrix::from_cols_array(
                    &((i + i.transpose()) * 0.5).to_cols_array(),
                ),
            ),
            None => r::MassProperties::new(vector(com), m, vector(inertia)),
        };
        let hull = r::InteractionGroups::new(VEHICLE, r::Group::ALL, r::InteractionTestMode::And);
        let (car_hull, hull_friction) = hull_feel(spec);
        // Havok mixes two bodies' friction as the root of their product;
        // the least of the two gives the same for the values met here (one
        // of them 0, or both alike).
        let combine = r::CoefficientCombineRule::Min;
        // It collides by its chassis body's own `PHY2` shapes, as the 360
        // code builds its body from them (0x8216A290): its vehicle hull
        // set ([`VehicleSpec::shapes`]). A tank's sit low enough under its
        // hull that, springs bottomed out, its track rays still start
        // above the ground: our box stood 0.45 m up (the wheels' middle),
        // over a squeeze of 0.36 to 0.4 m on five tanks, whose rays then
        // started under the ground and missed, leaving them sliding on the
        // box. A car's leaves out the shape on each wheel. Each shape is
        // its points' convex hull (Havok's are convex but for a mesh one,
        // whose hull covers a little more). A model without them keeps the
        // invented box.
        // Each with whether a path search sees it
        // ([`VehicleSpec::path_unseen`]).
        let shapes: Vec<(r::SharedShape, bool)> = spec
            .shapes
            .iter()
            .enumerate()
            .filter_map(|(k, v)| {
                let points: Vec<_> = v.iter().map(|&p| vector(p)).collect();
                r::SharedShape::convex_hull(&points).map(|s| (s, !spec.path_unseen.contains(&k)))
            })
            .collect();
        if shapes.is_empty() {
            let collider = r::ColliderBuilder::cuboid(half.x, half.y, half.z)
                .translation(vector(centre))
                .mass_properties(mass)
                .friction(hull_friction)
                .friction_combine_rule(combine)
                .collision_groups(hull);
            insert_collider(&mut self.world, collider, Some(body));
        }
        for (i, (shape, seen)) in shapes.into_iter().enumerate() {
            let collider = r::ColliderBuilder::new(shape)
                .friction(hull_friction)
                .friction_combine_rule(combine)
                .collision_groups(hull);
            // The body's mass and inertia are its chassis record's, on
            // the first (in model space: the box's sat at `centre`); the
            // rest add nothing.
            let collider = if i == 0 {
                collider
                    .mass_properties(mass.transform_by(&r::Pose::from_translation(vector(centre))))
            } else {
                collider.density(0.0)
            };
            let h = insert_collider(&mut self.world, collider, Some(body));
            if !seen {
                self.path_solid.insert(h, PathSolid::Nothing);
            }
        }
        // The turret: a body of its own on a hinge about the up through
        // its mount when its parts have a mass ([`VehicleSpec::turret_mass`];
        // the 360 code's turret update builds it, 0x82273F90, and drives it
        // with a motor, [`TurretHinge`]); else its shapes
        // ([`VehicleSpec::turret_shapes`]), or an invented box, ride the
        // hull, turned with it ([`Physics::turn_turret`]).
        let mut turret_colliders = Vec::new();
        let shapes: Vec<r::SharedShape> = spec
            .turret_shapes
            .iter()
            .filter_map(|v| {
                let points: Vec<_> = v.iter().map(|&p| vector(p)).collect();
                r::SharedShape::convex_hull(&points)
            })
            .collect();
        let hinge = match (spec.turret_mass, spec.turret_mount()) {
            (Some(m), Some(mount)) if !shapes.is_empty() => {
                let turret_body = self.world.insert_body(
                    r::RigidBodyBuilder::dynamic()
                        .pose(pose(position, rotation))
                        .linear_damping(BodyMaterial::COMMON.linear_damping)
                        .angular_damping(BodyMaterial::COMMON.angular_damping)
                        .ccd_enabled(true),
                );
                let props = r::MassProperties::with_inertia_matrix(
                    vector(m.centre_of_mass),
                    m.mass,
                    rapier3d::math::Matrix::from_cols_array(
                        &((m.inertia + m.inertia.transpose()) * 0.5).to_cols_array(),
                    ),
                );
                for (i, shape) in shapes.iter().enumerate() {
                    let c = r::ColliderBuilder::new(shape.clone())
                        .friction(hull_friction)
                        .friction_combine_rule(combine)
                        .collision_groups(hull);
                    let c = if i == 0 {
                        c.mass_properties(props)
                    } else {
                        c.density(0.0)
                    };
                    insert_collider(&mut self.world, c, Some(turret_body));
                }
                let t = spec.tuning.turret;
                let mut joint = r::RevoluteJointBuilder::new(r::Vector::Y)
                    .local_anchor1(vector(mount))
                    .local_anchor2(vector(mount))
                    .contacts_enabled(false);
                if let Some((lo, hi)) = t.and_then(|t| t.yaw)
                    && hi - lo < std::f32::consts::TAU - 0.01
                {
                    joint = joint.limits([lo, hi]);
                }
                self.world.insert_impulse_joint(body, turret_body, joint);
                let gun = spec
                    .gun_body
                    .as_ref()
                    .zip(spec.gun_pivot)
                    .map(|(g, trunnion)| {
                        self.add_gun(
                            g,
                            trunnion,
                            turret_body,
                            position,
                            rotation,
                            t,
                            hull,
                            hull_friction,
                        )
                    });
                Some(TurretHinge {
                    body: turret_body,
                    speed: t.map_or(0.0, |t| t.yaw_speed),
                    accel: t.map_or(0.0, |t| t.yaw_accel),
                    target: 0.0,
                    last: None,
                    gun,
                })
            }
            _ => None,
        };
        if hinge.is_none() {
            for shape in shapes {
                let turret = r::ColliderBuilder::new(shape)
                    .density(0.0)
                    .friction(hull_friction)
                    .friction_combine_rule(combine)
                    .collision_groups(hull);
                turret_colliders.push(insert_collider(&mut self.world, turret, Some(body)));
            }
        }
        if let Some((lo, hi)) = spec.turret_box.filter(|_| spec.turret_shapes.is_empty()) {
            let half = ((hi - lo) * 0.5).max(Vec3::splat(0.1));
            let turret = r::ColliderBuilder::cuboid(half.x, half.y, half.z)
                .translation(vector((lo + hi) * 0.5))
                .density(0.0)
                .friction(hull_friction)
                .friction_combine_rule(combine)
                .collision_groups(hull);
            turret_colliders.push(insert_collider(&mut self.world, turret, Some(body)));
        }

        let mut controller = DynamicRayCastVehicleController::new(body);
        controller.index_up_axis = 1;
        controller.index_forward_axis = 2;
        let tank = spec
            .tuning
            .tank
            .filter(|_| spec.tracked)
            .map(|t| Tracks::new(t, spec.wheels.len()));
        // A tank with its template's tracks grips by them (`drive_tracks`),
        // without one as Rapier's tyres by an invented friction; a wheeled
        // vehicle's tyres give none there either.
        let grip = if spec.tracked && tank.is_none() {
            GRIP
        } else {
            0.0
        };
        // A wheeled vehicle's springs as the 360 code builds them from its
        // template ([`Springs`], its defaults without one): each hangs
        // from its squeeze above the model's wheel, as long as all its
        // lengths, one damp both ways. Unlike rapier's, the 360's cast
        // sets how short it gets (down to minus the wheel's radius, the
        // hit at the cast's start), so its travel never clamps it.
        let springs = (!spec.tracked).then(|| spec.tuning.springs.unwrap_or(Springs::DEFAULT));
        // Its weight on the ground, a motorbike's too (the 360 code pulls
        // every car down so, 0x8219EE14, and balances a bike after).
        let gravity_factor = springs.map_or(1.0, |s| s.gravity_factor);
        let preload = preloads(spec, |_| SPRING, centre.z);
        let mut tops = Vec::with_capacity(spec.wheels.len());
        for (w, &preload) in spec.wheels.iter().zip(&preload) {
            // A tank driven as the 360 code drives it springs itself
            // ([`drive_tracks`]): Rapier only casts each point's ray, from
            // its squeeze up from the bottom of its track (its hardpoint)
            // down its whole length, with a wheel too thin to matter.
            if let Some(t) = &tank {
                let (_, _, _, squeeze) = t.tuning.springs;
                let floor = w.centre - Vec3::Y * w.radius;
                tops.push(squeeze - w.radius);
                let none = WheelTuning {
                    suspension_stiffness: 0.0,
                    suspension_compression: 0.0,
                    suspension_damping: 0.0,
                    max_suspension_travel: t.tuning.spring_length(),
                    side_friction_stiffness: 0.0,
                    friction_slip: 0.0,
                    max_suspension_force: 0.0,
                };
                controller.add_wheel(
                    vector(floor + Vec3::Y * squeeze),
                    -r::Vector::Y,
                    -r::Vector::X,
                    t.tuning.spring_length() - TRACK_RAY_RADIUS,
                    TRACK_RAY_RADIUS,
                    &none,
                );
                continue;
            }
            let (stiffness, (squeezed, stretched), rest, top, travel) = match springs {
                Some(s) => {
                    let (k, c, length, top) = s.axle(w.steers);
                    (k, (c, c), length, top, length + w.radius)
                }
                // A tank's are invented: longer than SUSPENSION by what
                // its share of the weight squeezes them, so standing it
                // sits as its model has it.
                None => {
                    let rest = SUSPENSION + preload;
                    (SPRING, DAMPING, rest, SUSPENSION, rest)
                }
            };
            tops.push(top);
            let tuning = WheelTuning {
                suspension_stiffness: stiffness,
                suspension_compression: squeezed,
                suspension_damping: stretched,
                max_suspension_travel: travel,
                side_friction_stiffness: 1.0,
                friction_slip: grip,
                max_suspension_force: spec.mass * 40.0,
            };
            controller.add_wheel(
                vector(w.centre + Vec3::Y * top),
                -r::Vector::Y,
                // Forward = up x axle = +z.
                -r::Vector::X,
                rest,
                w.radius,
                &tuning,
            );
        }
        // A boat floats on its waterline (its origin's height), over the
        // footprint of its hull a little in from the edges.
        let boat = spec.is_boat();
        let hull = boat.then(|| {
            let (lo, hi) = spec.hull;
            let at = |a: f32, b: f32, k: f32| a + (b - a) * (0.1 + 0.4 * k);
            let points = (0..3)
                .flat_map(|i| (0..3).map(move |j| (i as f32, j as f32)))
                .map(|(i, j)| Vec3::new(at(lo.x, hi.x, i), 0.0, at(lo.z, hi.z, j)))
                .collect();
            let water = spec.tuning.boat.unwrap_or(BOAT);
            // Its springs at its body's shape's box's corners (0x82199328
            // takes the box from the body's shape), else its hull's.
            let (lo, hi) = spec
                .shapes
                .iter()
                .flatten()
                .fold(None, |b: Option<(Vec3, Vec3)>, &p| {
                    Some(b.map_or((p, p), |(lo, hi)| (lo.min(p), hi.max(p))))
                })
                .unwrap_or((lo, hi));
            let lean = water.float.roll.sin() * (hi.x - lo.x) * 0.5;
            let lift = water.float.pitch.sin() * (hi.z - lo.z);
            let springs = [
                (Vec3::new(lo.x, 0.0, hi.z), lean, lift),
                (Vec3::new(hi.x, 0.0, hi.z), -lean, lift),
                (Vec3::new(lo.x, 0.0, lo.z), lean, 0.0),
                (Vec3::new(hi.x, 0.0, lo.z), -lean, 0.0),
            ];
            Hull {
                points,
                springs,
                wet: [None; 4],
                helm: 0.0,
                lean: 0.0,
                lift: 0.0,
                engine: 0.0,
                water_share: 1.0,
                clear: 1000.0,
                touching: 0.0,
                stabilise: 0.0,
                handbrake: 0.0,
                controls: VehicleControls::default(),
                water,
            }
        });
        let rotor = spec
            .tuning
            .flight
            .filter(|_| spec.is_helicopter())
            .map(|flight| Rotorcraft {
                flight,
                flight_ai: spec.tuning.flight_ai,
                ai: false,
                controls: VehicleControls {
                    parked: true,
                    ..VehicleControls::default()
                },
                lift: 0.0,
                strafe: 0.0,
                state: Rotor::default(),
                phase: turbulence_phase(self.vehicles.len() as u32),
            });
        let top = if boat { BOAT_TOP } else { TOP_SPEED };
        let tyres = (!spec.tracked).then(|| spec.tuning.tyres.unwrap_or(Tyres::DEFAULT));
        let id = VehicleId(self.vehicles.len());
        self.vehicles.push(Some(VehicleBody {
            controller,
            bed: false,
            mass: m,
            steers: spec.wheels.iter().map(|w| w.steers).collect(),
            tops,
            tyres,
            axles: Axle::standing(&tyres),
            footing: 1.0,
            gravity_factor,
            tracked: spec.tracked,
            bike: bike.map(|tuning| Bike { tuning, steer: 0.0 }),
            top: spec.tuning.max_speed.unwrap_or(top),
            reverse_top: match spec.tuning.engine {
                Some(e) if !spec.tracked => e.reverse_speed,
                _ => (spec.tuning.max_speed.unwrap_or(top) * 0.5).min(REVERSE_SPEED),
            },
            engine: (!spec.tracked).then(|| spec.tuning.engine.unwrap_or(EngineTuning::DEFAULT)),
            tank,
            gearbox: Gearbox::default(),
            drive: 0.0,
            pulling: false,
            braking: false,
            handling: spec.tuning.handling.filter(|_| !spec.tracked),
            steering: 0.0,
            wreck: spec.wreck.clone(),
            wrecked: false,
            car_hull,
            turret: (turret_colliders, spec.turret_mount()),
            hinge,
            held: false,
            before: Vec3::ZERO,
            hull,
            rotor,
        }));
        id
    }

    pub fn remove_vehicle(&mut self, id: VehicleId) {
        if let Some(v) = self.vehicles.get_mut(id.0).and_then(Option::take) {
            if let Some(h) = &v.hinge {
                self.world.remove_body_with_colliders(h.body, true);
                if let Some(g) = &h.gun {
                    self.world.remove_body_with_colliders(g.body, true);
                }
            }
            if let Some(b) = self.world.bodies.get(v.controller.chassis) {
                for h in b.colliders() {
                    self.path_solid.remove(h);
                }
            }
            self.world
                .remove_body_with_colliders(v.controller.chassis, true);
        }
    }

    /// Sets a vehicle's throttle (-1 reverse to 1 full ahead), steering
    /// (-1 left to 1 right) and handbrake. Throttle against the way the
    /// vehicle rolls brakes it; with no throttle it rolls to a stop.
    pub fn drive(&mut self, id: VehicleId, controls: VehicleControls) {
        let dt = self.world.integration_parameters.dt.max(1e-3);
        let Some(v) = self
            .vehicles
            .get_mut(id.0)
            .and_then(Option::as_mut)
            .filter(|v| !v.wrecked)
        else {
            return;
        };
        // A boat is driven as it floats (`float_boats`), a helicopter as
        // it flies (`fly_helicopters`).
        if let Some(hull) = &mut v.hull {
            hull.controls = controls;
            return;
        }
        if let Some(rotor) = &mut v.rotor {
            rotor.controls = controls;
            return;
        }
        // A sleeping tank's tracks don't run; any input wakes it (see
        // [`Rest`]).
        if let Some(tank) = &mut v.tank
            && let Some(body) = self.world.bodies.get_mut(v.controller.chassis)
            && body.is_sleeping()
        {
            let input = !controls.parked && (controls.throttle != 0.0 || controls.steer != 0.0);
            if !input {
                return;
            }
            body.wake_up(true);
            tank.rest.wake();
        }
        // A sleeping car wakes to any input: its input handler
        // wakes its body on each message of a value other than 0
        // (0x82189AC8 -> 0x8261EEB0).
        if v.tank.is_none()
            && !controls.parked
            && (controls.throttle != 0.0 || controls.steer != 0.0 || controls.brake)
            && let Some(body) = self.world.bodies.get_mut(v.controller.chassis)
            && body.is_sleeping()
        {
            body.wake_up(true);
        }
        let speed = v.controller.current_vehicle_speed;
        let wheels = v.controller.wheels().len().max(1) as f32;
        // A wheel's brake is the most impulse it may take off per step:
        // this many m/s² over the whole vehicle.
        let slowing = |decel: f32| v.mass * decel * dt / wheels;
        let throttle = controls.throttle.clamp(-1.0, 1.0);
        // A tank's throttle against the motion brakes it.
        let against = speed.abs() > 0.5 && throttle * speed < 0.0;
        // A wheeled vehicle's pedals as the 360 code's two inputs, its
        // throttle (0x2C) and brake (0x30): ahead the stick forward is the
        // throttle and back the brake; in reverse they swap (inferred
        // from 0x8219C540, which stays in reverse while the brake isn't
        // held and leaves it when it is: which pad input feeds which isn't
        // traced).
        let (open, pressed) = match v.engine {
            Some(_) => {
                let t = if v.gearbox.gear == 0 {
                    -throttle
                } else {
                    throttle
                };
                (t.max(0.0), (-t).max(0.0))
            }
            None if against => (0.0, throttle.abs()),
            None => (throttle.abs(), 0.0),
        };
        v.braking = !controls.parked && (controls.brake || pressed > 0.0);
        v.pulling = !controls.parked && !controls.brake && open > 0.0;
        // Its engine drives its wheels as the 360 code's does
        // ([`EngineTuning::step`]; the drive path at 0x8219C9EC): with at
        // least half its wheels down, its torque through the gear it is in
        // (nothing while its clutch delay runs) and off the handbrake,
        // shared out to its wheels ([`push_axles`]), fading over the last
        // twentieth of its `MaxSpeed` (`ReverseMaxSpeed` in reverse). Its
        // gearbox goes into reverse once it is all but still (under 2
        // km/h) with the brake held over 0.9 and the throttle under 0.1
        // for a tenth of a second, and back out once that has stopped for
        // as long (as the other drive path has it, 0x8219C490).
        v.drive = 0.0;
        if let Some(e) = v.engine {
            let slow = speed.abs() * 3.6 < 2.0;
            let asks = slow && pressed > 0.9 && open < 0.1;
            let reversing = v.gearbox.gear == 0;
            let gb = &mut v.gearbox;
            gb.reversing = if asks != reversing {
                (gb.reversing + dt).min(0.1)
            } else {
                (gb.reversing - dt).max(-0.1)
            };
            let reverse = if gb.reversing >= 0.1 {
                true
            } else {
                gb.reversing > -0.1 && reversing
            };
            let all = v.controller.wheels();
            let down = all
                .iter()
                .filter(|w| w.raycast_info().is_in_contact)
                .count();
            if down >= all.len() / 2 {
                let wheels = self
                    .world
                    .bodies
                    .get(v.controller.chassis)
                    .map_or(0.0, |b| driven_rpm(b, all, &v.steers, v.steering, e.split));
                let torque = e.step(gb, dt, open, wheels, reverse);
                let top = if gb.gear == 0 { e.reverse_speed } else { v.top };
                let ease = ((top - speed.abs()) / (0.05 * top).max(1e-3)).clamp(0.0, 1.0);
                if gb.clutch < 0.0 && !controls.brake {
                    v.drive = e.ratio(gb.gear).abs() * torque * ease;
                }
            }
        }
        // A tank without its template's tracks pulls 4 m/s² falling to
        // nothing at its top speed (invented); one with them is driven as
        // the 360 code drives it, below.
        let top = if throttle >= 0.0 {
            v.top
        } else {
            v.reverse_top
        };
        let headroom = (1.0 - speed.abs() / top).clamp(0.0, 1.0);
        let push = if v.tank.is_some() {
            0.0
        } else {
            TRACK_PULL * headroom
        };
        // A tank's braking against the motion (invented).
        let service = |_front: bool| slowing(SERVICE_BRAKE * throttle.abs());
        // A wheeled vehicle's axles' coefficients along and across as the
        // 360 code works them out ([`Tyres`]), which [`push_axles`] pushes
        // it with. Throttle against the motion is the brake pressed that
        // hard. A parked one (nobody at the wheel) still holds as a tank
        // does: what the 360 does with an empty car isn't traced, and on
        // its handbrake (3 m/s² by the defaults) it would slide down a
        // hill.
        let fast = (speed.abs() / v.top.max(0.1)).min(1.0);
        let tyres = v.tyres.map(|t| {
            let held = if controls.brake { 1.0 } else { 0.0 };
            let steer = controls.steer;
            let share = |front: bool| {
                let on = v.steers.iter().filter(|&&s| s == front).count();
                on as f32 / v.steers.len().max(1) as f32
            };
            let axle = |front: bool| {
                if controls.parked {
                    let hard = if speed.abs() > PARKED_FAST {
                        HOLD
                    } else {
                        HANDBRAKE
                    };
                    Axle {
                        along: hard * share(front),
                        across: t.grip(front, fast, 0.0, 0.0, 0.0),
                        hold: true,
                    }
                } else {
                    Axle {
                        along: t.slowing(front, speed, open, pressed, held, steer),
                        across: t.grip(front, fast, pressed, held, steer),
                        hold: false,
                    }
                }
            };
            [axle(true), axle(false)]
        });
        let (force, brake) = if controls.parked {
            let hard = if speed.abs() > PARKED_FAST {
                HOLD
            } else {
                HANDBRAKE
            };
            (0.0, slowing(hard))
        } else if controls.brake {
            (0.0, slowing(ROLLING))
        } else if against {
            (0.0, -1.0)
        } else if throttle != 0.0 {
            (throttle * v.mass * push / wheels, 0.0)
        } else if speed.abs() < 1.0 {
            (0.0, slowing(HOLD))
        } else {
            (0.0, slowing(ROLLING))
        };
        // Its front wheels turn as the 360 code turns them ([`Handling`];
        // its defaults without a template); a tank's wheels don't.
        v.steering = if v.tracked {
            0.0
        } else {
            v.handling.unwrap_or(Handling::DEFAULT).steer(
                v.steering,
                controls.steer.clamp(-1.0, 1.0),
                speed,
                fast,
                open,
                pressed,
                dt,
            )
        };
        let steer = v.steering;
        let tracked = v
            .tracked
            .then(|| (v.controller.chassis, controls.steer.clamp(-1.0, 1.0)));
        // Only the front axle steers: steering every wheel the same way
        // slides the vehicle sideways instead of turning it. The handbrake
        // locks the rest, so the back can slide out.
        // A wheeled vehicle's axles hold it, not Rapier's tyres (which give
        // it no grip), and its engine pushes it ([`push_axles`]).
        if let Some(axles) = tyres {
            v.axles = axles;
        }
        let own = tyres.is_some() || v.tank.is_some();
        for (i, w) in v.controller.wheels_mut().iter_mut().enumerate() {
            let front = v.steers.get(i).copied().unwrap_or(false);
            w.engine_force = if own { 0.0 } else { force };
            w.steering = if front { steer } else { 0.0 };
            w.brake = if own {
                0.0
            } else if controls.brake && !front {
                slowing(HANDBRAKE)
            } else if brake < 0.0 {
                service(front)
            } else {
                brake
            };
        }
        let gripping = v
            .controller
            .wheels()
            .iter()
            .filter(|w| w.raycast_info().is_in_contact)
            .count()
            >= 2;
        let mass = v.mass;
        // A motorbike holds itself up as the 360 code does ([`hold_bike`]);
        // gravity and its tyres do the rest (it can fall over).
        let top = v.top;
        if let Some(bike) = &mut v.bike
            && let Some(body) = self.world.bodies.get_mut(v.controller.chassis)
        {
            let wheels = v.controller.wheels();
            let touching: Vec<Vec3> = wheels
                .iter()
                .filter(|w| w.raycast_info().is_in_contact)
                .map(|w| vec3(w.raycast_info().contact_normal_ws))
                .collect();
            let normal = if touching.is_empty() {
                Vec3::Y
            } else {
                touching.iter().sum::<Vec3>() / touching.len() as f32
            };
            let footing = BikeFooting {
                wheels: wheels.len(),
                touching: touching.len(),
                normal,
                top,
                // The alpha steers positive to the left, the 360 code
                // to the right (its steering's lean and turn agree only
                // so); the pad's stick forward and back has no key.
                steer: -controls.steer.clamp(-1.0, 1.0),
                stick: 0.0,
                throttle: open,
                brake: pressed,
            };
            let turn = Quat::from_array(body.position().rotation.to_array());
            let body = std::cell::RefCell::new(body);
            hold_bike(
                bike,
                footing,
                turn,
                || {
                    let b = body.borrow();
                    (vec3(b.angvel()), vec3(b.linvel()))
                },
                |spin, slide| {
                    let mut b = body.borrow_mut();
                    b.apply_torque_impulse(vector(spin * mass), true);
                    b.apply_impulse(vector(slide * mass), true);
                },
                dt,
            );
        }
        // Tracks turn the hull on the spot (one track forward, the other
        // back), so the yaw is set directly; a little slower on the move.
        if let Some((chassis, turn)) = tracked
            && let Some(body) = self.world.bodies.get_mut(chassis)
        {
            // Their grip holds its weight on the hill (along the ground,
            // the way its hull lies; invented). One driven as the 360 code
            // drives it stands on its springs instead, which push straight
            // up while it is near level.
            if gripping && v.tank.is_none() {
                let up = body.position().rotation * r::Vector::Y;
                let down = r::Vector::new(0.0, -GRAVITY, 0.0);
                let along = down - up * down.dot(up);
                let steep = along.length() / GRAVITY;
                let held = (CLIMB_TRACKED.sin() / steep.max(1e-3)).min(1.0);
                body.apply_impulse(-along * held * mass * dt, true);
            }
            if let Some(tank) = &mut v.tank {
                // The alpha's throttle is positive ahead and its steering
                // positive to the left; the 360 code's the other way round
                // for both (its drive pushes against the throttle, its
                // steering turns the hull clockwise from above). Its
                // tracks run in the step, on the rays cast there
                // ([`run_tracks`]).
                tank.asked = if controls.parked {
                    (0.0, 0.0)
                } else {
                    (-throttle, -turn)
                };
            } else {
                let w = body.angvel();
                let rate = turn * TRACK_TURN / (1.0 + speed.abs() * 0.05);
                let up = body.position().rotation * r::Vector::Y;
                let keep = w - up * w.dot(up);
                body.set_angvel(keep + up * rate, true);
            }
        }
    }

    /// Asks a helicopter to slide right (1) or left (-1), as fast as it can
    /// (the 360 code's message 0x11C78A3C, its stick's across).
    pub fn strafe(&mut self, id: VehicleId, right: f32) {
        if let Some(r) = self
            .vehicles
            .get_mut(id.0)
            .and_then(Option::as_mut)
            .and_then(|v| v.rotor.as_mut())
        {
            r.strafe = right;
        }
    }

    /// Asks a vehicle's turret to turn to `yaw` radians (anticlockwise from
    /// above, as [`VehicleSpec::turret_transform`] turns its nodes): one on
    /// its own body is driven there by its hinge's motor ([`TurretHinge`];
    /// the caller moves `yaw` at most its turn rate a second, as the 360
    /// code's 0x82273C88 does), one riding the hull has its colliders
    /// turned there at once. See [`turret_yaw`](Self::turret_yaw) for where
    /// it is.
    pub fn turn_turret(&mut self, id: VehicleId, yaw: f32) {
        let Some(v) = self.vehicles.get_mut(id.0).and_then(Option::as_mut) else {
            return;
        };
        if let Some(h) = &mut v.hinge {
            h.last = h.last.or(Some(yaw));
            h.target = yaw;
            return;
        }
        let (handles, mount) = (&v.turret.0, v.turret.1.unwrap_or(Vec3::ZERO));
        let turn = Quat::from_rotation_y(yaw);
        let place = pose(mount - turn * mount, turn);
        for &h in handles {
            if let Some(c) = self.world.colliders.get_mut(h) {
                c.set_position_wrt_parent(place);
            }
        }
    }

    /// Turns each turret on its own body toward the angle asked as the
    /// 360's turret action does ([`TurretHinge`]), and drives its gun's
    /// motor toward the pitch asked ([`GunHinge`]); `dt` the step.
    fn drive_turrets(&mut self, dt: f32) {
        let wrap = |a: f32| {
            let a = a.rem_euclid(std::f32::consts::TAU);
            if a > std::f32::consts::PI {
                a - std::f32::consts::TAU
            } else {
                a
            }
        };
        let heading = |q: Quat| {
            let f = q * Vec3::Z;
            f.x.atan2(f.z)
        };
        for i in 0..self.vehicles.len() {
            let Some(v) = self.vehicles[i].as_mut() else {
                continue;
            };
            let chassis = v.controller.chassis;
            if let Some(h) = v.hinge.as_mut()
                && let Some(last) = h.last
                && dt > 0.0
            {
                // The turn asked this step (0x8227582C: its change / dt).
                let asked = wrap(h.target - last) / dt;
                h.last = Some(h.target);
                let hull = self
                    .world
                    .bodies
                    .get(chassis)
                    .map(|b| Quat::from_array(b.position().rotation.to_array()));
                if let (Some(hull), Some(b)) = (hull, self.world.bodies.get_mut(h.body)) {
                    let turn = Quat::from_array(b.position().rotation.to_array());
                    let up = turn * Vec3::Y;
                    let spin = vec3(b.angvel());
                    let w = spin.dot(up);
                    let e = wrap(heading(hull) + h.target - heading(turn));
                    let damp = if asked * asked > w * w {
                        TURRET_LEAD_DAMP
                    } else {
                        TURRET_DAMP
                    };
                    // Asleep, it is woken only when the angle asked is
                    // more than [`TURRET_WAKE`] off where it points
                    // (0x821A64B8); else it, and its island, sleep on.
                    if !b.is_sleeping() || e * e > TURRET_WAKE * TURRET_WAKE {
                        let want = (w + dt * (TURRET_PULL * e - damp * w)).clamp(-h.speed, h.speed);
                        let change = (want - w).clamp(-h.accel * dt, h.accel * dt);
                        b.set_angvel(vector(spin + up * change), true);
                    }
                }
            }
            // Its gun to the pitch asked.
            let Some((joint, force, target, body, last)) = self.vehicles[i]
                .as_mut()
                .and_then(|v| v.hinge.as_mut())
                .and_then(|h| h.gun.as_mut())
                .map(|g| {
                    let last = g.last;
                    g.last = g.target;
                    (g.joint, g.force, g.target, g.body, last)
                })
            else {
                continue;
            };
            let Some(now) = self.gun_pitch(VehicleId(i)) else {
                continue;
            };
            // Asleep, it is woken only when the pitch asked moves faster
            // than [`GUN_WAKE`] (0x82275608); else it sleeps on.
            let speed = if dt > 0.0 { (target - last) / dt } else { 0.0 };
            let asleep = self.world.bodies.get(body).is_some_and(|b| b.is_sleeping());
            let wake = !asleep || speed * speed > GUN_WAKE * GUN_WAKE;
            // The joint turns about +x, the gun's pitch about -x.
            let off = -(target - now);
            if let Some(j) = self
                .world
                .impulse_joints
                .get_mut(joint, wake && off.abs() > 1e-4)
            {
                j.data
                    .set_motor_velocity(r::JointAxis::AngX, 20.0 * off, force);
            }
        }
    }

    /// Asks a vehicle's gun on its own body ([`GunHinge`]) to tilt to
    /// `pitch` radians (up positive); its motor drives it there.
    pub fn aim_gun(&mut self, id: VehicleId, pitch: f32) {
        if let Some(g) = self
            .vehicles
            .get_mut(id.0)
            .and_then(Option::as_mut)
            .and_then(|v| v.hinge.as_mut())
            .and_then(|h| h.gun.as_mut())
        {
            g.target = pitch;
        }
    }

    /// The pitch a vehicle's gun on its own body was last asked to tilt
    /// to, radians; `None` for one without.
    pub fn gun_target(&self, id: VehicleId) -> Option<f32> {
        let v = self.vehicles.get(id.0).and_then(Option::as_ref)?;
        v.hinge.as_ref()?.gun.as_ref().map(|g| g.target)
    }

    /// Where a vehicle's gun on its own body has tilted to, radians (up
    /// positive): its body's turn about the turret's across. `None` for
    /// one without.
    pub fn gun_pitch(&self, id: VehicleId) -> Option<f32> {
        let v = self.vehicles.get(id.0).and_then(Option::as_ref)?;
        let h = v.hinge.as_ref()?;
        let g = h.gun.as_ref()?;
        let turret = self.world.bodies.get(h.body)?;
        let gun = self.world.bodies.get(g.body)?;
        let q = Quat::from_array(turret.position().rotation.to_array()).inverse()
            * Quat::from_array(gun.position().rotation.to_array());
        // Models face +z: a turn about x by -pitch lifts the muzzle.
        Some(-2.0 * q.x.atan2(q.w))
    }

    /// The angle a vehicle's turret on its own body was last asked to
    /// turn to ([`turn_turret`](Self::turn_turret)), radians; `None` for
    /// one riding the hull.
    pub fn turret_target(&self, id: VehicleId) -> Option<f32> {
        let v = self.vehicles.get(id.0).and_then(Option::as_ref)?;
        v.hinge.as_ref().map(|h| h.target)
    }

    /// Where a vehicle's turret on its own body has turned to, radians
    /// (anticlockwise from above): its body's turn about the hull's up.
    /// `None` for one riding the hull.
    pub fn turret_yaw(&self, id: VehicleId) -> Option<f32> {
        let v = self.vehicles.get(id.0).and_then(Option::as_ref)?;
        let h = v.hinge.as_ref()?;
        let hull = self.world.bodies.get(v.controller.chassis)?;
        let turret = self.world.bodies.get(h.body)?;
        let q = Quat::from_array(hull.position().rotation.to_array()).inverse()
            * Quat::from_array(turret.position().rotation.to_array());
        Some(2.0 * q.y.atan2(q.w))
    }

    /// An AI pilot's stick messages for a helicopter
    /// ([`crate::heli_pilot::PilotStick`], the 360's lift, yaw, ahead and
    /// sideways), as its pad's would be.
    pub fn pilot_helicopter(&mut self, id: VehicleId, stick: crate::heli_pilot::PilotStick) {
        let Some(r) = self
            .vehicles
            .get_mut(id.0)
            .and_then(Option::as_mut)
            .and_then(|v| v.rotor.as_mut())
        else {
            return;
        };
        r.lift = stick.lift;
        r.strafe = stick.right;
        r.controls = VehicleControls {
            throttle: stick.ahead,
            steer: -stick.yaw,
            ..VehicleControls::default()
        };
    }

    /// The ground straight under a vehicle as an AI pilot looks for it:
    /// the first thing solid but itself down from 1000 m to -1000 m
    /// (0x822ED8C0), its height.
    pub fn ground_under(&self, id: VehicleId, at: Vec3) -> Option<f32> {
        self.cast_ray_past(Vec3::new(at.x, 1000.0, at.z), Vec3::NEG_Y, 2000.0, id)
            .map(|(d, _)| 1000.0 - d)
    }

    /// Whether a helicopter has been off everything for half a second or
    /// more, as its pilot's mind reads its physics' ground time
    /// (0x822BCDC8: not over 0.5 s); `None` for one that isn't.
    pub fn helicopter_airborne(&self, id: VehicleId) -> Option<bool> {
        let v = self.vehicles.get(id.0).and_then(Option::as_ref)?;
        v.rotor.as_ref().map(|r| r.state.ground <= 0.5)
    }

    /// Whether a vehicle stands on the ground as the 360's
    /// `ObjectIsGrounded` test has it (0x82354AE8, by its physics kind):
    /// a helicopter once on something over half a second (its rotor's
    /// ground time, the 360's +0x140 object's +0x4C, over 0.5); a car
    /// (`_CarPhysicsV2`, kind 2: its vtable +0x134, 0x82189A68) when a
    /// wheel touches this step (0x8219C338 counts the wheels whose contact
    /// timer, +0x0, 0x8219F510 just set back to 0.4 for a wheel with
    /// contacts, +0x34; here a wheel's ray in contact); anything else
    /// (tanks, boats, jets) always. `None` for no vehicle.
    pub fn vehicle_grounded(&self, id: VehicleId, car: bool) -> Option<bool> {
        let v = self.vehicles.get(id.0).and_then(Option::as_ref)?;
        if let Some(r) = v.rotor.as_ref() {
            return Some(r.state.ground > 0.5);
        }
        if !car {
            return Some(true);
        }
        Some(
            v.controller
                .wheels()
                .iter()
                .any(|w| w.raycast_info().is_in_contact),
        )
    }

    /// `Vehicle.IsFlipped` (0x82348358, the physics object's vtable
    /// +0x9C): only a car flips (0x82189A00: its up axis's world height
    /// under 0.5, 0x820D9EDC, and no wheel touching, as
    /// [`vehicle_grounded`](Self::vehicle_grounded)); boats, helicopters
    /// and tanks never (0x828280B0). `None` for no vehicle.
    pub fn vehicle_flipped(&self, id: VehicleId, car: bool) -> Option<bool> {
        let up = self
            .vehicle_transform(id)?
            .transform_vector3(Vec3::Y)
            .normalize_or_zero();
        Some(car && up.y < 0.5 && !self.vehicle_grounded(id, true)?)
    }

    /// Hands a helicopter's stick to an AI (`true`) or back to the
    /// player: with an AI it flies by its `_HelicopterPhysicsAi` record,
    /// as the 360 code picks it for one the player does not fly
    /// (0x8218C1F0), and its stick wobbles ([`ai_wobble`]).
    pub fn set_ai_pilot(&mut self, id: VehicleId, ai: bool) {
        if let Some(r) = self
            .vehicles
            .get_mut(id.0)
            .and_then(Option::as_mut)
            .and_then(|v| v.rotor.as_mut())
        {
            r.ai = ai;
        }
    }

    /// Puts a helicopter in the air as one made flying: its rotor at full
    /// and off the ground a second. The 360's physics starts every rotor
    /// stopped (0x821A3278); one the scripts make in the air is taken to
    /// be flying already (inferred).
    pub fn start_rotor(&mut self, id: VehicleId) {
        if let Some(r) = self
            .vehicles
            .get_mut(id.0)
            .and_then(Option::as_mut)
            .and_then(|v| v.rotor.as_mut())
        {
            r.state.spin = 1.0;
            r.state.air = 1.0;
            r.state.ground = 0.0;
        }
    }

    /// Asks a helicopter to climb (1) or come down (-1), as fast as it
    /// can; 0 holds its height.
    pub fn fly(&mut self, id: VehicleId, lift: f32) {
        if let Some(r) = self
            .vehicles
            .get_mut(id.0)
            .and_then(Option::as_mut)
            .and_then(|v| v.rotor.as_mut())
        {
            r.lift = lift;
        }
    }

    /// Whether a vehicle flies (a helicopter with its template's handling).
    pub fn flies(&self, id: VehicleId) -> bool {
        self.vehicles
            .get(id.0)
            .and_then(Option::as_ref)
            .is_some_and(|v| v.rotor.is_some())
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

    /// How far each of a vehicle's springs is compressed from its rest
    /// length, metres (0 for a wheel off the ground).
    pub fn vehicle_suspension(&self, id: VehicleId) -> Vec<f32> {
        self.vehicles
            .get(id.0)
            .and_then(Option::as_ref)
            .map(|v| {
                v.controller
                    .wheels()
                    .iter()
                    .map(|w| {
                        let ray = w.raycast_info();
                        if ray.is_in_contact {
                            w.suspension_rest_length - ray.suspension_length
                        } else {
                            0.0
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// How far each of a vehicle's wheels sits above where its model has
    /// it, chassis space, metres, in spec order: up as its spring squeezes
    /// over a bump, down as it hangs off a ledge or in the air. Draw the
    /// wheel this far up and its tyre stays on the ground it touches.
    pub fn vehicle_wheel_lift(&self, id: VehicleId) -> Vec<f32> {
        self.vehicles
            .get(id.0)
            .and_then(Option::as_ref)
            .map(|v| {
                v.controller
                    .wheels()
                    .iter()
                    .zip(&v.tops)
                    .map(|(w, top)| {
                        let ray = w.raycast_info();
                        let length = if ray.is_in_contact {
                            ray.suspension_length
                        } else {
                            w.suspension_rest_length
                        };
                        // The model's wheel centre is `top` under its
                        // spring's top (see `add_vehicle_turned`).
                        top - length
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether a vehicle's body is asleep (a tank keeping still, [`Rest`]).
    pub fn vehicle_asleep(&self, id: VehicleId) -> bool {
        self.vehicles
            .get(id.0)
            .and_then(Option::as_ref)
            .and_then(|v| self.world.bodies.get(v.controller.chassis))
            .is_some_and(|b| b.is_sleeping())
    }

    /// How many of a vehicle's wheels touch the ground, of how many.
    pub fn vehicle_contacts(&self, id: VehicleId) -> (usize, usize) {
        self.vehicles
            .get(id.0)
            .and_then(Option::as_ref)
            .map_or((0, 0), |v| {
                let wheels = v.controller.wheels();
                let down = wheels
                    .iter()
                    .filter(|w| w.raycast_info().is_in_contact)
                    .count();
                (down, wheels.len())
            })
    }

    /// What a vehicle's body is in contact with (its wheels' contacts
    /// aside): each static's key (`None` for anything else solid, such as
    /// another vehicle) with the deepest point of contact on the body and
    /// the box the thing touched fills, world space. To find what holds a
    /// stuck vehicle.
    #[allow(clippy::type_complexity)]
    pub fn vehicle_touching(&self, id: VehicleId) -> Vec<(Option<StaticKey>, Vec3, (Vec3, Vec3))> {
        let Some(body) = self
            .vehicles
            .get(id.0)
            .and_then(Option::as_ref)
            .and_then(|v| self.world.bodies.get(v.controller.chassis))
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for &c in body.colliders() {
            let Some(own) = self.world.colliders.get(c) else {
                continue;
            };
            for pair in self.world.narrow_phase.contact_pairs_with(c) {
                if !pair.has_any_active_contact() {
                    continue;
                }
                let first = pair.collider1 == c;
                let other = if first {
                    pair.collider2
                } else {
                    pair.collider1
                };
                let Some(deepest) = pair
                    .manifolds()
                    .iter()
                    .flat_map(|m| &m.points)
                    .min_by(|a, b| a.dist.total_cmp(&b.dist))
                else {
                    continue;
                };
                let local = if first {
                    deepest.local_p1
                } else {
                    deepest.local_p2
                };
                let key = self.keys.get(&other).copied();
                let span = self
                    .world
                    .colliders
                    .get(other)
                    .map_or((Vec3::ZERO, Vec3::ZERO), |o| {
                        let b = o.compute_aabb();
                        (vec3(b.mins), vec3(b.maxs))
                    });
                out.push((key, vec3(own.position().transform_point(local)), span));
            }
        }
        out
    }

    /// The statics a vehicle's body is in contact with (its wheels'
    /// contacts aside): the terrain's cells, placed objects, buildings and
    /// a state layer's models, each once, with the deepest point of
    /// contact, the contact normal from the vehicle toward it (world
    /// space) and its mass (0 for a fixed one, as the 360's collision
    /// event's size takes it: [`crate::durability::crash_size`]).
    pub fn vehicle_static_hits(&self, id: VehicleId) -> Vec<(StaticKey, Vec3, Vec3, f32)> {
        let Some(body) = self
            .vehicles
            .get(id.0)
            .and_then(Option::as_ref)
            .and_then(|v| self.world.bodies.get(v.controller.chassis))
        else {
            return Vec::new();
        };
        let mut out: Vec<(StaticKey, Vec3, Vec3, f32)> = Vec::new();
        for &c in body.colliders() {
            let Some(mine) = self.world.colliders.get(c) else {
                continue;
            };
            for pair in self.world.narrow_phase.contact_pairs_with(c) {
                if !pair.has_any_active_contact() {
                    continue;
                }
                let first = pair.collider1 == c;
                let other = if first {
                    pair.collider2
                } else {
                    pair.collider1
                };
                let Some(&key) = self.keys.get(&other) else {
                    continue;
                };
                if out.iter().any(|(k, ..)| *k == key) {
                    continue;
                }
                let Some((manifold, deepest)) = pair
                    .manifolds()
                    .iter()
                    .flat_map(|m| m.points.iter().map(move |p| (m, p)))
                    .min_by(|a, b| a.1.dist.total_cmp(&b.1.dist))
                else {
                    continue;
                };
                let (local, normal) = if first {
                    (deepest.local_p1, vec3(manifold.data.normal))
                } else {
                    (deepest.local_p2, -vec3(manifold.data.normal))
                };
                let mass = self
                    .world
                    .colliders
                    .get(other)
                    .and_then(|o| o.parent())
                    .and_then(|b| self.world.bodies.get(b))
                    .filter(|b| b.is_dynamic())
                    .map_or(0.0, |b| b.mass());
                out.push((
                    key,
                    vec3(mine.position().transform_point(local)),
                    normal.normalize_or_zero(),
                    mass,
                ));
            }
        }
        out
    }

    /// The other vehicles a vehicle's body is in contact with, body to
    /// body, each once, with the deepest point of contact and the contact
    /// normal from this one toward the other (world space).
    pub fn vehicle_bumps(&self, id: VehicleId) -> Vec<(VehicleId, Vec3, Vec3)> {
        let chassis = |v: &VehicleId| {
            self.vehicles
                .get(v.0)
                .and_then(Option::as_ref)
                .map(|v| v.controller.chassis)
        };
        let Some(own) = chassis(&id) else {
            return Vec::new();
        };
        let Some(body) = self.world.bodies.get(own) else {
            return Vec::new();
        };
        let mut out: Vec<(VehicleId, Vec3, Vec3)> = Vec::new();
        for &c in body.colliders() {
            let Some(mine) = self.world.colliders.get(c) else {
                continue;
            };
            for pair in self.world.narrow_phase.contact_pairs_with(c) {
                if !pair.has_any_active_contact() {
                    continue;
                }
                let first = pair.collider1 == c;
                let other = if first {
                    pair.collider2
                } else {
                    pair.collider1
                };
                let Some(parent) = self.world.colliders.get(other).and_then(|o| o.parent()) else {
                    continue;
                };
                let Some(them) = (0..self.vehicles.len())
                    .map(VehicleId)
                    .find(|v| *v != id && chassis(v) == Some(parent))
                else {
                    continue;
                };
                if out.iter().any(|(v, ..)| *v == them) {
                    continue;
                }
                let Some((manifold, deepest)) = pair
                    .manifolds()
                    .iter()
                    .flat_map(|m| m.points.iter().map(move |p| (m, p)))
                    .min_by(|a, b| a.1.dist.total_cmp(&b.1.dist))
                else {
                    continue;
                };
                // The manifold's normal points from its first collider
                // toward its second.
                let (local, normal) = if first {
                    (deepest.local_p1, vec3(manifold.data.normal))
                } else {
                    (deepest.local_p2, -vec3(manifold.data.normal))
                };
                out.push((
                    them,
                    vec3(mine.position().transform_point(local)),
                    normal.normalize_or_zero(),
                ));
            }
        }
        out
    }

    /// The statics a vehicle's body is in contact with (its wheels'
    /// contacts aside), each once: what it ran into.
    pub fn vehicle_statics(&self, id: VehicleId) -> Vec<StaticKey> {
        let Some(body) = self
            .vehicles
            .get(id.0)
            .and_then(Option::as_ref)
            .and_then(|v| self.world.bodies.get(v.controller.chassis))
        else {
            return Vec::new();
        };
        self.statics_touching(body.colliders())
    }

    /// The statics a loose piece is in contact with, each once: what holds
    /// one that hardly moved (the gate test asks).
    pub fn debris_statics(&self, id: DebrisId) -> Vec<StaticKey> {
        let Some(body) = self
            .debris
            .get(id.0)
            .copied()
            .flatten()
            .and_then(|h| self.world.bodies.get(h))
        else {
            return Vec::new();
        };
        self.statics_touching(body.colliders())
    }

    /// The statics meeting a box (`lo` to `hi` in its own space, placed at
    /// `place`), each once: what stands where a building does.
    pub fn statics_in_box(&self, place: Mat4, (lo, hi): (Vec3, Vec3)) -> Vec<StaticKey> {
        let (_, rotation, translation) = place.to_scale_rotation_translation();
        let rotation = rotation.normalize();
        let middle = translation + rotation * ((lo + hi) * 0.5);
        let shape = r::Cuboid::new(vector((hi - lo).max(Vec3::splat(0.02)) * 0.5));
        let mut out = Vec::new();
        for (handle, _) in
            self.world
                .intersect_shape(pose(middle, rotation), &shape, r::QueryFilter::default())
        {
            if let Some(&key) = self.keys.get(&handle)
                && !out.contains(&key)
            {
                out.push(key);
            }
        }
        out
    }

    /// The statics any of `colliders` is in contact with, each once.
    fn statics_touching(&self, colliders: &[r::ColliderHandle]) -> Vec<StaticKey> {
        let mut out = Vec::new();
        for &c in colliders {
            for pair in self.world.narrow_phase.contact_pairs_with(c) {
                if !pair.has_any_active_contact() {
                    continue;
                }
                let other = if pair.collider1 == c {
                    pair.collider2
                } else {
                    pair.collider1
                };
                if let Some(&key) = self.keys.get(&other)
                    && !out.contains(&key)
                {
                    out.push(key);
                }
            }
        }
        out
    }

    /// Whether a vehicle's engine is pulling (or backing) it.
    pub fn vehicle_pulling(&self, id: VehicleId) -> bool {
        self.vehicles
            .get(id.0)
            .and_then(Option::as_ref)
            .is_some_and(|v| {
                (v.tyres.is_some() && v.pulling)
                    || v.controller.wheels().iter().any(|w| w.engine_force != 0.0)
            })
    }

    /// A vehicle's velocity, m/s.
    pub fn vehicle_velocity(&self, id: VehicleId) -> Vec3 {
        self.vehicles
            .get(id.0)
            .and_then(Option::as_ref)
            .and_then(|v| self.world.bodies.get(v.controller.chassis))
            .map_or(Vec3::ZERO, |b| vec3(b.linvel()))
    }

    /// A vehicle's angular velocity, rad/s.
    pub fn vehicle_spin(&self, id: VehicleId) -> Vec3 {
        self.vehicles
            .get(id.0)
            .and_then(Option::as_ref)
            .and_then(|v| self.world.bodies.get(v.controller.chassis))
            .map_or(Vec3::ZERO, |b| vec3(b.angvel()))
    }

    /// Signed forward speed, m/s. None while held
    /// ([`hold_vehicle`](Self::hold_vehicle)): the controller isn't run
    /// then, so its own reading is the one from before it was held.
    pub fn vehicle_speed(&self, id: VehicleId) -> f32 {
        self.vehicles
            .get(id.0)
            .and_then(Option::as_ref)
            .filter(|v| !v.held)
            .map_or(0.0, |v| v.controller.current_vehicle_speed)
    }

    /// What the ground under a vehicle is made of: the surface word of
    /// what a ray straight down from it meets, 0 for nothing within `max`
    /// metres or nothing that says.
    pub fn vehicle_ground(&self, id: VehicleId, max: f32) -> u32 {
        let Some(v) = self.vehicles.get(id.0).and_then(Option::as_ref) else {
            return 0;
        };
        let Some(body) = self.world.bodies.get(v.controller.chassis) else {
            return 0;
        };
        let ray = r::Ray::new(body.translation(), vector(Vec3::NEG_Y));
        let filter = solid().exclude_rigid_body(v.controller.chassis);
        self.world
            .query_pipeline_with_filter(filter)
            .cast_ray_and_get_normal(&ray, max, true)
            .and_then(|(handle, hit)| Some(self.surfaces.get(&handle)?.surface_of(hit.subshape)))
            .unwrap_or(0)
    }

    /// How fast a vehicle's wheels and tracks roll, m/s: its chassis's
    /// velocity along its own forward axis, signed (the controller's
    /// [`vehicle_speed`](Self::vehicle_speed) is the whole velocity's
    /// length, so a parked tank bobbing on its springs reads as moving).
    /// None while held, nor under [`ROLL_REST`].
    pub fn vehicle_rolling(&self, id: VehicleId) -> f32 {
        let Some(v) = self.vehicles.get(id.0).and_then(Option::as_ref) else {
            return 0.0;
        };
        if v.held {
            return 0.0;
        }
        let Some(at) = self.vehicle_transform(id) else {
            return 0.0;
        };
        let along = self
            .vehicle_velocity(id)
            .dot(at.z_axis.truncate().normalize_or_zero());
        if along.abs() < ROLL_REST { 0.0 } else { along }
    }

    /// The gear a wheeled vehicle's engine drives in (0 reverse, 1 first)
    /// and how fast that engine turns, rpm; `None` for a tank.
    pub fn vehicle_gear(&self, id: VehicleId) -> Option<(usize, f32)> {
        let v = self.vehicles.get(id.0).and_then(Option::as_ref)?;
        v.engine?;
        Some((v.gearbox.gear, v.gearbox.rpm))
    }

    /// Whether a vehicle's driver is braking ([`drive`](Self::drive) last
    /// had it pulling against the way it rolls, or on its handbrake): its
    /// brake lamps glow. A wreck's never do.
    pub fn vehicle_braking(&self, id: VehicleId) -> bool {
        self.vehicles
            .get(id.0)
            .and_then(Option::as_ref)
            .is_some_and(|v| v.braking && !v.wrecked)
    }

    /// Makes a vehicle a wreck: its wheels no longer carry it (the 360
    /// sets a dead vehicle up without its wheel action), so it drops onto
    /// its wrecked chassis and slides to a stop instead of rolling on.
    pub fn wreck_vehicle(&mut self, id: VehicleId) {
        let Some(v) = self.vehicles.get_mut(id.0).and_then(Option::as_mut) else {
            return;
        };
        v.wrecked = true;
        // How low the rims reach, chassis space.
        let mut rims = f32::INFINITY;
        for (w, top) in v.controller.wheels_mut().iter_mut().zip(&v.tops) {
            let centre = w.chassis_connection_point_cs.y - top;
            rims = rims.min(centre - w.radius * WRECK_RIM);
            w.max_suspension_force = 0.0;
            w.friction_slip = 0.0;
            w.engine_force = 0.0;
            w.brake = 0.0;
        }
        let Some(body) = self.world.bodies.get_mut(v.controller.chassis) else {
            return;
        };
        // Its wrecked chassis body, as the 360 rebuilds it once the wreck's
        // parts show ([`crate::vehicle::WreckBody`]): its shapes, friction
        // and damping in place of the intact one's, its mass and motion
        // kept. From the 360 code: a kill queues the object (0x82182CE8)
        // for its physics to be rebuilt (0x82152780: bodies on hidden parts
        // dropped, 0x82184370, then set up again, 0x821831B0); an object
        // whose health (its record's +4) is under 0.01 is set up reduced
        // (+0x15): one body, its showing chassis part's (0x8216C570), and
        // no wheel action (0x821897F8 drops it, 0x82185240). Its set-up
        // still asks for the hull set and gives it the template's mass
        // (0x82182848). So its turret's bodies go and its wheels stop
        // carrying it.
        if let Some(wreck) = v.wreck.clone() {
            let mass = body.mass_properties().local_mprops;
            let m = wreck.material;
            body.set_linear_damping(m.linear_damping);
            body.set_angular_damping(m.angular_damping);
            let old: Vec<_> = body.colliders().to_vec();
            let groups = old
                .first()
                .and_then(|&h| self.world.colliders.get(h))
                .map(|c| c.collision_groups());
            let chassis = v.controller.chassis;
            for h in old {
                self.world.remove_collider(h);
            }
            for (i, shape) in wreck
                .shapes
                .iter()
                .filter_map(|v| {
                    let points: Vec<_> = v.iter().map(|&p| vector(p)).collect();
                    r::SharedShape::convex_hull(&points)
                })
                .enumerate()
            {
                let mut c = r::ColliderBuilder::new(shape)
                    .friction(m.friction)
                    .friction_combine_rule(r::CoefficientCombineRule::Min);
                if let Some(g) = groups {
                    c = c.collision_groups(g);
                }
                let c = if i == 0 {
                    c.mass_properties(mass)
                } else {
                    c.density(0.0)
                };
                insert_collider(&mut self.world, c, Some(chassis));
            }
            return;
        }
        body.set_linear_damping(WRECK_DAMPING);
        body.set_angular_damping(WRECK_DAMPING);
        // An invented box hull (the first collider; a turret's comes
        // after), which stopped at the wheels' centres, reaches down to the
        // rims and drags on the ground; a hull of its chassis's own shapes
        // ([`VehicleSpec::shapes`]) just drags.
        for (i, &h) in body.colliders().iter().enumerate() {
            let Some(c) = self.world.colliders.get_mut(h) else {
                continue;
            };
            if c.shape().as_cuboid().is_none() {
                c.set_friction(WRECK_FRICTION);
                continue;
            }
            if i > 0 {
                continue;
            }
            let (Some(cube), Some(at)) = (
                c.shape().as_cuboid().map(|b| vec3(b.half_extents)),
                c.position_wrt_parent().map(|p| vec3(p.translation)),
            ) else {
                continue;
            };
            let (top, bottom) = (at.y + cube.y, (at.y - cube.y).min(rims));
            if !bottom.is_finite() {
                continue;
            }
            let half = (top - bottom) * 0.5;
            c.set_shape(r::SharedShape::cuboid(cube.x, half, cube.z));
            c.set_translation_wrt_parent(vector(Vec3::new(at.x, bottom + half, at.z)));
            c.set_friction(WRECK_FRICTION);
        }
    }

    /// Holds a vehicle where it is, its body fixed (the ground under it
    /// has streamed out, far from the player, and it would fall through
    /// the island), or lets it go again, at rest.
    pub fn hold_vehicle(&mut self, id: VehicleId, held: bool) {
        let Some(v) = self
            .vehicles
            .get_mut(id.0)
            .and_then(Option::as_mut)
            .filter(|v| v.held != held)
        else {
            return;
        };
        v.held = held;
        if let Some(body) = self.world.bodies.get_mut(v.controller.chassis) {
            let kind = if held {
                r::RigidBodyType::Fixed
            } else {
                r::RigidBodyType::Dynamic
            };
            body.set_body_type(kind, true);
            body.set_linvel(r::Vector::ZERO, true);
            body.set_angvel(r::Vector::ZERO, true);
        }
    }

    /// Whether a vehicle is held where it is.
    pub fn vehicle_held(&self, id: VehicleId) -> bool {
        self.vehicles
            .get(id.0)
            .and_then(Option::as_ref)
            .is_some_and(|v| v.held)
    }

    /// A vehicle's velocity before the last step, m/s: what it hit things
    /// with (a crash stops it within the step).
    pub fn vehicle_velocity_before(&self, id: VehicleId) -> Vec3 {
        self.vehicles
            .get(id.0)
            .and_then(Option::as_ref)
            .map_or(Vec3::ZERO, |v| v.before)
    }

    /// Sets a vehicle's velocity, m/s (as it was before something it hit
    /// gave way).
    pub fn set_vehicle_velocity(&mut self, id: VehicleId, velocity: Vec3) {
        let Some(v) = self.vehicles.get(id.0).and_then(Option::as_ref) else {
            return;
        };
        if let Some(body) = self.world.bodies.get_mut(v.controller.chassis) {
            body.set_linvel(vector(velocity), true);
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

    /// Shoves a vehicle through its centre of mass: `impulse` (kg m/s),
    /// world space, on its chassis body.
    ///
    /// From the 360 code: a vehicle's physics (car, tank, helicopter and
    /// boat alike, vtable +0x10 0x82181818) hands the push to its first
    /// body (0x8216DCC8), which takes it as a linear impulse with no turn
    /// (0x8262AB60); [`push_vehicle`](Self::push_vehicle) is the same at a
    /// point (+0x14 0x82181858, 0x8216DD38). Its first body is its
    /// chassis ([`crate::vehicle::CHASSIS`]).
    pub fn impulse_vehicle(&mut self, id: VehicleId, impulse: Vec3) {
        let Some(v) = self.vehicles.get(id.0).and_then(Option::as_ref) else {
            return;
        };
        if let Some(body) = self.world.bodies.get_mut(v.controller.chassis) {
            body.apply_impulse(vector(impulse), true);
        }
    }

    /// A vehicle's mass as its physics reports it (`Object.GetMass`), kg:
    /// its bodies' masses together (chassis, turret and gun), or 0 while
    /// any of them does not move (held, [`hold_vehicle`](Self::hold_vehicle)).
    ///
    /// From the 360 code: a vehicle's physics (car, tank, helicopter and
    /// boat alike, vtable +0x60 0x82182418) sums its bodies' masses, each
    /// one over its inverse mass (0x8262C120), and answers 0 at the first
    /// body whose inverse mass is 0.
    pub fn vehicle_mass(&self, id: VehicleId) -> Option<f32> {
        let v = self.vehicles.get(id.0)?.as_ref()?;
        let hinge = v.hinge.as_ref();
        let bodies = std::iter::once(v.controller.chassis)
            .chain(hinge.map(|h| h.body))
            .chain(hinge.and_then(|h| h.gun.as_ref()).map(|g| g.body));
        let mut sum = 0.0;
        for handle in bodies {
            let body = self.world.bodies.get(handle)?;
            if !body.is_dynamic() || body.mass() <= 0.0 {
                return Some(0.0);
            }
            sum += body.mass();
        }
        Some(sum)
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
            insert_collider(
                &mut self.world,
                shape.density(1000.0).friction(0.9).collision_groups(groups),
                Some(body),
            );
            bodies[i] = Some(body);
            // Joined to the nearest ancestor with a limb; or, when none
            // has one, to the nearest limb made so far. (The spine and
            // the hips both hang from a root bone without a limb: joined
            // only to ancestors, the upper body flew off on its own.)
            let mut up = Some(parent);
            let mut on = None;
            while let Some(a) = up {
                if bodies[a].is_some() {
                    on = Some(a);
                    break;
                }
                up = parents[a].filter(|&p| p < n);
            }
            let on = on.or_else(|| {
                (0..i)
                    .filter(|&k| bodies[k].is_some())
                    .min_by(|&a, &b| at[a].0.distance(p).total_cmp(&at[b].0.distance(p)))
            });
            if let Some((a, other)) = on.and_then(|a| Some((a, bodies[a]?))) {
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

    /// The speed of a ragdoll's fastest limb, m/s; `None` once removed.
    pub fn ragdoll_speed(&self, id: RagdollId) -> Option<f32> {
        let doll = self.ragdolls.get(id.0)?.as_ref()?;
        Some(
            doll.bodies
                .iter()
                .flatten()
                .filter_map(|h| self.world.bodies.get(*h))
                .map(|b| vec3(b.linvel()).length())
                .fold(0.0, f32::max),
        )
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

    /// [`cast_ray`](Self::cast_ray) that passes through vehicle `skip`:
    /// shots from its own gun. All of it: its hull, and a tank's turret
    /// and gun where they are bodies of their own ([`TurretHinge`]), which
    /// a shell leaving the muzzle would otherwise meet at once and burst
    /// in its own barrel.
    pub fn cast_ray_past(
        &self,
        origin: Vec3,
        direction: Vec3,
        max: f32,
        skip: VehicleId,
    ) -> Option<(f32, Vec3)> {
        let ray = r::Ray::new(vector(origin), vector(direction.normalize_or_zero()));
        let own = self.vehicle_bodies(skip);
        let not_own =
            |_: r::ColliderHandle, c: &r::Collider| c.parent().is_none_or(|b| !own.contains(&b));
        self.world
            .query_pipeline_with_filter(solid().predicate(&not_own))
            .cast_ray_and_get_normal(&ray, max, true)
            .map(|(_, hit)| (hit.time_of_impact, vec3(hit.normal)))
    }

    /// The bodies vehicle `id` is made of: its chassis, and its turret's
    /// and gun's where they hinge on it as bodies of their own.
    fn vehicle_bodies(&self, id: VehicleId) -> Vec<r::RigidBodyHandle> {
        let Some(v) = self.vehicles.get(id.0).and_then(Option::as_ref) else {
            return Vec::new();
        };
        let mut out = vec![v.controller.chassis];
        if let Some(h) = &v.hinge {
            out.push(h.body);
            out.extend(h.gun.as_ref().map(|g| g.body));
        }
        out
    }

    /// Whether a ball of `radius` swept from `origin` `down` metres
    /// straight down meets anything solid but the terrain, passing
    /// through vehicle `skip`: a landing spot's clearance. The 360's
    /// clearance checks (0x822CE820) are Havok shape casts while its
    /// ground is found by rays; that those casts meet no terrain is
    /// inferred (balls as wide as the rotor swept down to 1.5 m over level
    /// ground would always meet it).
    pub fn sphere_meets_below(
        &self,
        origin: Vec3,
        down: f32,
        radius: f32,
        skip: Option<VehicleId>,
    ) -> bool {
        if down <= 0.0 {
            return false;
        }
        let ball = r::Ball::new(radius.max(1e-3));
        let skip_body = skip
            .and_then(|v| self.vehicles.get(v.0))
            .and_then(Option::as_ref)
            .map(|v| v.controller.chassis);
        let not_terrain = |h: r::ColliderHandle, _: &r::Collider| {
            !matches!(self.keys.get(&h), Some(StaticKey::Cell(_)))
        };
        let mut filter = solid().predicate(&not_terrain);
        if let Some(b) = skip_body {
            filter = filter.exclude_rigid_body(b);
        }
        let options = rapier3d::parry::query::ShapeCastOptions {
            max_time_of_impact: down,
            stop_at_penetration: true,
            ..Default::default()
        };
        self.world
            .cast_shape(
                &pose(origin, Quat::IDENTITY),
                vector(Vec3::NEG_Y),
                &ball,
                options,
                filter,
            )
            .is_some()
    }

    /// How far a ball of `radius` metres goes from `origin` along
    /// `direction` before it touches something solid, within `max`
    /// metres, passing through vehicle `skip`: how near a camera may come
    /// to a wall, a pillar or a door before its near plane cuts into it,
    /// which a thin ray misses at a slant. What the ball starts touching
    /// and moves away from doesn't stop it.
    pub fn cast_sphere(
        &self,
        origin: Vec3,
        direction: Vec3,
        max: f32,
        radius: f32,
        skip: Option<VehicleId>,
    ) -> Option<f32> {
        let mut filter = solid();
        if let Some(v) = skip
            .and_then(|v| self.vehicles.get(v.0))
            .and_then(Option::as_ref)
        {
            filter = filter.exclude_rigid_body(v.controller.chassis);
        }
        let ball = r::Ball::new(radius.max(1e-3));
        let options = rapier3d::parry::query::ShapeCastOptions {
            max_time_of_impact: max,
            stop_at_penetration: false,
            ..Default::default()
        };
        self.world
            .cast_shape(
                &pose(origin, Quat::IDENTITY),
                vector(direction.normalize_or_zero()),
                &ball,
                options,
                filter,
            )
            .map(|(_, hit)| hit.time_of_impact)
    }

    /// How far, of `max` metres along `direction`, a box of half
    /// extents `half` turned `turn` goes from `origin` before it meets
    /// something solid but the terrain, passing through vehicle `skip`:
    /// an AI pilot's sweeps. The 360's are Havok shape casts while it
    /// finds the ground with rays and keeps its pilots over it by them
    /// (`MinAltitude`); that its casts meet no terrain is inferred (a run's
    /// box swept at the target on the ground would always meet it).
    pub fn cast_box(
        &self,
        origin: Vec3,
        turn: Quat,
        half: Vec3,
        direction: Vec3,
        max: f32,
        skip: Option<VehicleId>,
    ) -> Option<f32> {
        let cube = r::Cuboid::new(vector(half.max(Vec3::splat(1e-3))));
        let options = rapier3d::parry::query::ShapeCastOptions {
            max_time_of_impact: max,
            stop_at_penetration: false,
            ..Default::default()
        };
        self.skipping(skip, false, |filter| {
            self.world.cast_shape(
                &pose(origin, turn),
                vector(direction.normalize_or_zero()),
                &cube,
                options,
                filter,
            )
        })
        .map(|(_, hit)| hit.time_of_impact)
    }

    /// What a path search's sweep met ([`crate::path_search`]): a box of
    /// half extents `half` from `from` to `to`, turned to face along it,
    /// passing through vehicle `skip`'s bodies, meeting no terrain (inferred,
    /// as [`cast_box`](Self::cast_box)'s: a box 0.375 m up would meet every rise).
    pub fn sweep_obstacle(
        &self,
        from: Vec3,
        to: Vec3,
        half: Vec3,
        skip: Option<VehicleId>,
    ) -> Option<crate::path_search::Obstacle> {
        let d = to - from;
        let len = d.length();
        if len < 1e-4 {
            return None;
        }
        let turn = Quat::from_rotation_y(d.x.atan2(d.z));
        let cube = r::Cuboid::new(vector(half.max(Vec3::splat(1e-3))));
        let options = rapier3d::parry::query::ShapeCastOptions {
            max_time_of_impact: len,
            stop_at_penetration: false,
            ..Default::default()
        };
        let (start, way) = (pose(from, turn), vector(d / len));
        // Past what the sweep doesn't see of a model (its [`PathSolid`]):
        // the first of what it does, cast again with the rest left out.
        let mut past = Vec::new();
        let mut best: Option<(r::ColliderHandle, f32, Vec3)> = None;
        while let Some((handle, hit)) = self.skipping_past(skip, false, &past, |filter| {
            self.world.cast_shape(&start, way, &cube, options, filter)
        }) {
            if best.is_some_and(|b| b.1 <= hit.time_of_impact) {
                break;
            }
            let Some(PathSolid::Part(part)) = self.path_solid.get(&handle) else {
                best = Some((handle, hit.time_of_impact, vec3(hit.witness1)));
                break;
            };
            past.push(handle);
            let at = self.world.colliders.get(handle)?.position();
            if let Ok(Some(h)) = rapier3d::parry::query::cast_shapes(
                at,
                r::Vector::ZERO,
                &**part,
                &start,
                way,
                &cube,
                options,
            ) && best.is_none_or(|b| h.time_of_impact < b.1)
            {
                best = Some((handle, h.time_of_impact, vec3(*at * h.witness1)));
            }
        }
        let (handle, toi, point) = best?;
        let mut o = self.body_obstacle(handle)?;
        // Where it met it: the point on what it met (the 360 records the
        // contact). Touching only at the very end of the sweep (within 5 cm,
        // the alpha's own margin) is no meeting: a node put where the last
        // sweep met the ground would otherwise always be met again there.
        if toi > len - 0.05 {
            return None;
        }
        o.point = point;
        Some(o)
    }

    /// What a boat's sweep to a point met (0x822A8220): as
    /// [`sweep_obstacle`](Self::sweep_obstacle), meeting the ground too;
    /// the point on it.
    pub fn sweep_afloat(&self, from: Vec3, to: Vec3, half: Vec3, skip: VehicleId) -> Option<Vec3> {
        let d = to - from;
        let len = d.length();
        if len < 1e-4 {
            return None;
        }
        let turn = Quat::from_rotation_y(d.x.atan2(d.z));
        let cube = r::Cuboid::new(vector(half.max(Vec3::splat(1e-3))));
        let options = rapier3d::parry::query::ShapeCastOptions {
            max_time_of_impact: len,
            stop_at_penetration: false,
            ..Default::default()
        };
        let (_, hit) = self.skipping(Some(skip), true, |filter| {
            self.world
                .cast_shape(&pose(from, turn), vector(d / len), &cube, options, filter)
        })?;
        Some(from + d / len * hit.time_of_impact)
    }

    /// Runs `f` with what a path search sees: what is solid but vehicle
    /// `skip`'s bodies, the terrain unless `terrain`, a model with nothing
    /// of it in the way ([`PathSolid::Nothing`]), and `past`.
    fn skipping_past<R>(
        &self,
        skip: Option<VehicleId>,
        terrain: bool,
        past: &[r::ColliderHandle],
        f: impl FnOnce(r::QueryFilter<'_>) -> R,
    ) -> R {
        let own = self.bodies_of(skip);
        let keep = |h: r::ColliderHandle, c: &r::Collider| {
            c.parent().is_none_or(|p| !own.contains(&p))
                && (terrain || !matches!(self.keys.get(&h), Some(StaticKey::Cell(_))))
                && !matches!(self.path_solid.get(&h), Some(PathSolid::Nothing))
                && !past.contains(&h)
        };
        f(paths().predicate(&keep))
    }

    /// Whether nothing that keeps a road vehicle from coming out is in the
    /// box from `lo` to `hi` ([`crate::road_population::spawn_box`]): no
    /// moving body but debris (sensors aside), and no static `blocks` says
    /// is a prop. From the 360 code: 0x82153B30 asks its world's
    /// broadphase and goes by each owner's kind (its vtable's +0xE0): a
    /// `_PropPhysics` (0, 0x828280B0, fixed or loose alike), a human (1,
    /// 0x822A2F80), a car (2, 0x82225D30), a tank (3, 0x82799AA0), a
    /// helicopter (4, 0x823B4D18), a jet (8, 0x82225AA8) or a boat (10,
    /// 0x824258B8) keeps it; a `_BuildingPhysics` (6, 0x822F64D8), an
    /// `EffectAiOccluder`'s stand-in box (7), a `Winch` (9) or kind 5 (not
    /// identified) lets it by, and so does what falls outside its switch:
    /// a `_DebrisPhysics` piece (14, 0x8218B648) and the ground (13).
    pub fn box_clear(&self, lo: Vec3, hi: Vec3, blocks: impl Fn(StaticKey) -> bool) -> bool {
        let middle = (lo + hi) * 0.5;
        let cube = r::Cuboid::new(vector(((hi - lo) * 0.5).max(Vec3::splat(1e-3))));
        let debris: Vec<r::RigidBodyHandle> = self.debris.iter().flatten().copied().collect();
        let moves = |h: r::ColliderHandle, c: &r::Collider| match self.keys.get(&h) {
            Some(&k) => blocks(k),
            None => c.parent().is_none_or(|b| !debris.contains(&b)),
        };
        self.world
            .intersect_shape(
                pose(middle, Quat::IDENTITY),
                &cube,
                solid().exclude_sensors().predicate(&moves),
            )
            .next()
            .is_none()
    }

    /// What stands fixed in the box from `lo` to `hi` (0x822ED248's
    /// area): each placed object's or building's bounds as a centre and
    /// radius. Not the terrain.
    pub fn fixed_near(&self, lo: Vec3, hi: Vec3) -> Vec<(Vec3, f32)> {
        let middle = (lo + hi) * 0.5;
        let cube = r::Cuboid::new(vector((hi - lo) * 0.5));
        let mut out = Vec::new();
        for (handle, _) in self.world.intersect_shape(
            pose(middle, Quat::IDENTITY),
            &cube,
            // Not a model's shapes no vehicle meets ([`Ghost`]).
            r::QueryFilter::default().exclude_sensors(),
        ) {
            let fixed = matches!(
                self.keys.get(&handle),
                Some(StaticKey::Object(_) | StaticKey::Member(..) | StaticKey::Entity(_))
            );
            if fixed && let Some(c) = self.world.colliders.get(handle) {
                let b = c.compute_aabb();
                out.push((vec3(b.center()), vec3(b.half_extents()).length()));
            }
        }
        out
    }

    /// The bodies of vehicle `skip`: its chassis, its turret's and its
    /// gun's.
    fn bodies_of(&self, skip: Option<VehicleId>) -> Vec<r::RigidBodyHandle> {
        let Some(v) = skip
            .and_then(|v| self.vehicles.get(v.0))
            .and_then(Option::as_ref)
        else {
            return Vec::new();
        };
        let mut b = vec![v.controller.chassis];
        if let Some(h) = &v.hinge {
            b.push(h.body);
            if let Some(g) = &h.gun {
                b.push(g.body);
            }
        }
        b
    }

    /// Runs `f` with what is solid but vehicle `skip`'s bodies.
    fn skipping<R>(
        &self,
        skip: Option<VehicleId>,
        terrain: bool,
        f: impl FnOnce(r::QueryFilter<'_>) -> R,
    ) -> R {
        let own = self.bodies_of(skip);
        let keep = |h: r::ColliderHandle, c: &r::Collider| {
            c.parent().is_none_or(|p| !own.contains(&p))
                && (terrain || !matches!(self.keys.get(&h), Some(StaticKey::Cell(_))))
        };
        f(solid().predicate(&keep))
    }

    /// Collider `handle` as a path search's obstacle: its id, whether it is
    /// the ground, its box.
    fn body_obstacle(&self, handle: r::ColliderHandle) -> Option<crate::path_search::Obstacle> {
        let c = self.world.colliders.get(handle)?;
        let b = match self.path_solid.get(&handle) {
            Some(PathSolid::Part(part)) => part.compute_local_aabb(),
            _ => c.shape().compute_local_aabb(),
        };
        // A static's whole entity's box ([`CollisionShape::entity_box`]).
        let b = match self.keys.get(&handle).and_then(|k| self.boxes.get(k)) {
            Some(&(lo, hi)) => r::Aabb::new(vector(lo), vector(hi)),
            None => b,
        };
        // A vehicle's: round all its bodies (hull, turret, gun), in its
        // hull's frame, as the 360's entity box is (0x82182E90).
        if let Some((p, b)) = c
            .parent()
            .and_then(|body| self.vehicle_box(body).or_else(|| self.debris_box(body)))
        {
            let (i, g) = handle.into_raw_parts();
            return Some(crate::path_search::Obstacle {
                point: vec3(p.translation),
                id: (u64::from(g) << 32) | u64::from(i),
                ground: false,
                pose: (vec3(p.translation), Quat::from_array(p.rotation.to_array())),
                min: vec3(b.mins),
                max: vec3(b.maxs),
            });
        }
        let p = c.position();
        let (i, g) = handle.into_raw_parts();
        Some(crate::path_search::Obstacle {
            point: vec3(p.translation),
            id: (u64::from(g) << 32) | u64::from(i),
            ground: matches!(self.keys.get(&handle), Some(StaticKey::Cell(_))),
            pose: (vec3(p.translation), Quat::from_array(p.rotation.to_array())),
            min: vec3(b.mins),
            max: vec3(b.maxs),
        })
    }

    /// The vehicle whose body `body` is: its hull's pose, and the box round
    /// all its bodies' colliders in the hull's frame.
    fn vehicle_box(&self, body: r::RigidBodyHandle) -> Option<(r::Pose, r::Aabb)> {
        let v = (0..self.vehicles.len())
            .map(VehicleId)
            .find(|&v| self.bodies_of(Some(v)).contains(&body))?;
        let chassis = self.vehicles.get(v.0)?.as_ref()?.controller.chassis;
        let pose = *self.world.bodies.get(chassis)?.position();
        let mut out: Option<r::Aabb> = None;
        for b in self.bodies_of(Some(v)) {
            for &h in self.world.bodies.get(b)?.colliders() {
                let c = self.world.colliders.get(h)?;
                let local = pose.inv_mul(c.position());
                let a = c.shape().compute_aabb(&local);
                out = Some(match out {
                    Some(o) => rapier3d::geometry::BoundingVolume::merged(&o, &a),
                    None => a,
                });
            }
        }
        Some((pose, out?))
    }

    /// Whether the line from `from` to `to` is clear of what a path search
    /// sees ([`PATH_MASK`](collision_layers::PATH_MASK)): `CarAttack`'s
    /// check on its line to her (0x822ACB28 sweeps 0x82166958 with that
    /// mask). As [`sweep_obstacle`](Self::sweep_obstacle) sees (leaves and
    /// AI-only shapes, not a chain fence or a loose prop's solid shapes),
    /// and the terrain. A line, where the 360 sweeps a thin box (its
    /// size not ported).
    pub fn path_line_clear(&self, from: Vec3, to: Vec3, skip: Option<VehicleId>) -> bool {
        let d = to - from;
        let len = d.length();
        if len < 1e-4 {
            return true;
        }
        if self
            .sweep_obstacle(from, to, Vec3::splat(0.01), skip)
            .is_some()
        {
            return false;
        }
        let ray = r::Ray::new(vector(from), vector(d / len));
        let cell = |h: r::ColliderHandle, _: &r::Collider| {
            matches!(self.keys.get(&h), Some(StaticKey::Cell(_)))
        };
        self.world
            .query_pipeline_with_filter(solid().predicate(&cell))
            .cast_ray(&ray, len, true)
            .is_none()
    }

    /// Whether an AI gunner's sight line from `from` to `to` is clear
    /// (0x822C95D8's rays, [`SIGHT_MASK`](collision_layers::SIGHT_MASK)):
    /// as [`line_clear`](Self::line_clear), and leaves block it too
    /// ([`CollisionShape::leaves`]).
    pub fn sight_clear(&self, from: Vec3, to: Vec3, skip: Option<VehicleId>) -> bool {
        let d = to - from;
        let len = d.length();
        if len < 1e-4 {
            return true;
        }
        let ray = r::Ray::new(vector(from), vector(d / len));
        let own = self.bodies_of(skip);
        let keep =
            |_: r::ColliderHandle, c: &r::Collider| c.parent().is_none_or(|p| !own.contains(&p));
        self.world
            .query_pipeline_with_filter(paths().predicate(&keep))
            .cast_ray(&ray, len, true)
            .is_none()
    }

    /// Whether the line from `from` to `to` meets nothing solid, passing
    /// through vehicle `skip`'s bodies (its chassis, turret and gun).
    pub fn line_clear(&self, from: Vec3, to: Vec3, skip: Option<VehicleId>) -> bool {
        let d = to - from;
        let len = d.length();
        if len < 1e-4 {
            return true;
        }
        let ray = r::Ray::new(vector(from), vector(d / len));
        self.skipping(skip, true, |filter| {
            self.world
                .query_pipeline_with_filter(filter)
                .cast_ray(&ray, len, true)
                .is_none()
        })
    }

    /// What a path search's obstacle `id` is, when a static (a placed
    /// object, a state layer's prop, a terrain cell).
    pub fn obstacle_key(&self, id: u64) -> Option<StaticKey> {
        self.keys
            .get(&r::ColliderHandle::from_raw_parts(
                id as u32,
                (id >> 32) as u32,
            ))
            .copied()
    }

    /// A path search's obstacle by its id
    /// ([`sweep_obstacle`](Self::sweep_obstacle)).
    pub fn obstacle(&self, id: u64) -> Option<crate::path_search::Obstacle> {
        self.body_obstacle(r::ColliderHandle::from_raw_parts(
            id as u32,
            (id >> 32) as u32,
        ))
    }

    /// The ground under `at` for a path search (0x822E4120: a ray from 2 m
    /// above to 1000 m below): where, and the body whose top it is when not
    /// the terrain.
    pub fn path_ground(&self, at: Vec3, skip: Option<VehicleId>) -> Option<(Vec3, Option<u64>)> {
        let cast = |top: Vec3| {
            let ray = r::Ray::new(vector(top), vector(Vec3::NEG_Y));
            self.skipping_past(skip, true, &[], |filter| {
                self.world
                    .query_pipeline_with_filter(filter)
                    .cast_ray(&ray, 1002.0, true)
            })
            .map(|(handle, toi)| (handle, top - Vec3::Y * toi))
        };
        // 0x822E4120: from 2 m above, then, for a point under the ground,
        // from 502 m above.
        let (handle, hit) = cast(at + Vec3::Y * 2.0).or_else(|| cast(at + Vec3::Y * 502.0))?;
        let body = self
            .body_obstacle(handle)
            .filter(|o| !o.ground)
            .map(|o| o.id);
        Some((hit, body))
    }

    /// The first solid thing along a ray, passing through vehicle `skip`:
    /// its distance, and the vehicle it is if one (what a round hits).
    pub fn cast_ray_vehicle(
        &self,
        origin: Vec3,
        direction: Vec3,
        max: f32,
        skip: Option<VehicleId>,
    ) -> Option<(f32, Option<VehicleId>)> {
        let ray = r::Ray::new(vector(origin), vector(direction.normalize_or_zero()));
        let mut filter = solid();
        if let Some(v) = skip
            .and_then(|v| self.vehicles.get(v.0))
            .and_then(Option::as_ref)
        {
            filter = filter.exclude_rigid_body(v.controller.chassis);
        }
        let (handle, hit) = self
            .world
            .query_pipeline_with_filter(filter)
            .cast_ray(&ray, max, true)?;
        let body = self.world.colliders.get(handle).and_then(|c| c.parent());
        let vehicle = self
            .vehicles
            .iter()
            .position(|v| {
                v.as_ref()
                    .is_some_and(|v| Some(v.controller.chassis) == body)
            })
            .map(VehicleId);
        Some((hit, vehicle))
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
        let key = self.keys.get(&handle).copied();
        Some((hit.time_of_impact, vec3(hit.normal), key))
    }

    /// [`cast_ray_keyed`](Self::cast_ray_keyed) as an AI's line of fire
    /// sees ([`SIGHT_MASK`](collision_layers::SIGHT_MASK): 0x822C95D8's
    /// rays carry 0x223F6FDA): leaves block it too
    /// ([`CollisionShape::leaves`]).
    pub fn cast_ray_sight_keyed(
        &self,
        origin: Vec3,
        direction: Vec3,
        max: f32,
    ) -> Option<(f32, Vec3, Option<StaticKey>)> {
        let ray = r::Ray::new(vector(origin), vector(direction.normalize_or_zero()));
        let (handle, hit) = self
            .world
            .query_pipeline_with_filter(paths())
            .cast_ray_and_get_normal(&ray, max, true)?;
        let key = self.keys.get(&handle).copied();
        Some((hit.time_of_impact, vec3(hit.normal), key))
    }

    /// [`cast_ray_keyed`](Self::cast_ray_keyed), with what the triangle it
    /// strikes is made of (a surface name hash, [`crate::surface`]; 0 for
    /// none known or not a static).
    pub fn cast_ray_surface(
        &self,
        origin: Vec3,
        direction: Vec3,
        max: f32,
    ) -> Option<(f32, Vec3, Option<StaticKey>, u32)> {
        let ray = r::Ray::new(vector(origin), vector(direction.normalize_or_zero()));
        let (handle, hit) = self
            .world
            .query_pipeline_with_filter(solid())
            .cast_ray_and_get_normal(&ray, max, true)?;
        let key = self.keys.get(&handle).copied();
        // A triangle mesh names the triangle struck as the hit's sub-shape.
        let surface = self
            .surfaces
            .get(&handle)
            .map_or(0, |shape| shape.surface_of(hit.subshape));
        Some((hit.time_of_impact, vec3(hit.normal), key, surface))
    }

    /// [`cast_ray`](Self::cast_ray) that passes through the statics whose
    /// keys `skip` picks: the ground under a building that is coming down.
    pub fn cast_ray_skipping(
        &self,
        origin: Vec3,
        direction: Vec3,
        max: f32,
        skip: impl Fn(StaticKey) -> bool,
    ) -> Option<(f32, Vec3)> {
        let ray = r::Ray::new(vector(origin), vector(direction.normalize_or_zero()));
        let keep = |handle: r::ColliderHandle, _: &r::Collider| {
            self.keys.get(&handle).is_none_or(|&key| !skip(key))
        };
        self.world
            .query_pipeline_with_filter(solid().predicate(&keep))
            .cast_ray_and_get_normal(&ray, max, true)
            .map(|(_, hit)| (hit.time_of_impact, vec3(hit.normal)))
    }

    /// The height of the island's own ground straight down from `from`
    /// within `max` metres: its terrain and placed models (roads, rocks,
    /// buildings), passing through the state layers' models and anything
    /// not static.
    pub fn island_below(&self, from: Vec3, max: f32) -> Option<f32> {
        let ray = r::Ray::new(vector(from), vector(Vec3::NEG_Y));
        let keep = |handle: r::ColliderHandle, _: &r::Collider| {
            matches!(
                self.keys.get(&handle),
                Some(StaticKey::Cell(_) | StaticKey::Object(_) | StaticKey::Member(..))
            )
        };
        self.world
            .query_pipeline_with_filter(solid().predicate(&keep))
            .cast_ray_and_get_normal(&ray, max, true)
            .map(|(_, hit)| from.y - hit.time_of_impact)
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
        vertical -= character.gravity * dt;
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

    // On its handbrake a boat slides across with less of the water's drag
    // (`HandBrakeAntiSideDrag`, by its share of its top speed) and the
    // drag left pushes it on ahead (`HandBrakeMomentumKeep`; 0x8219A9F0,
    // 0x8219AB4C); with both 0, as the 360 record's defaults, neither.
    // The water drags a boat along its own axes, by its template's drag
    // that way x the step x its share of the water (0x8219A5E0): set
    // sliding across at 6 m/s afloat, one step takes `WaterDragSide` x 6
    // x the step off it.
    #[test]
    fn the_water_drags_a_boat_along_its_own_axes() {
        let mut p = Physics::new();
        p.set_water(0.0);
        let spec = boat();
        let id = p.add_vehicle(&spec, Vec3::new(0.0, -0.5, 0.0), 0.0);
        for _ in 0..240 {
            p.drive(id, VehicleControls::default());
            p.step(DT);
        }
        let m = p.vehicle_transform(id).unwrap();
        let side = m.transform_vector3(Vec3::X);
        let v = p.vehicle_velocity(id);
        p.set_vehicle_velocity(id, v - side * v.dot(side) + side * 6.0);
        p.drive(id, VehicleControls::default());
        p.step(DT);
        let left = p.vehicle_velocity(id).dot(side);
        let want = 6.0 - BOAT.drag[1] * 6.0 * DT;
        assert!((left - want).abs() < 0.02, "{left} of {want}");
    }

    #[test]
    fn a_boat_on_its_handbrake_slides_and_keeps_its_way() {
        let slid = |anti: f32, keep: f32| {
            let mut p = Physics::new();
            p.set_water(0.0);
            let mut spec = boat();
            spec.tuning.boat = Some(BoatTuning {
                float: BoatFloat {
                    anti_side_drag: anti,
                    momentum_keep: keep,
                    ..BoatFloat::DEFAULT
                },
                ..BOAT
            });
            let id = p.add_vehicle(&spec, Vec3::new(0.0, -0.5, 0.0), 0.0);
            for _ in 0..120 {
                p.drive(id, VehicleControls::default());
                p.step(DT);
            }
            let v = p.vehicle_velocity(id);
            p.set_vehicle_velocity(id, v.with_x(8.0).with_z(0.0));
            let held = VehicleControls {
                brake: true,
                ..VehicleControls::default()
            };
            for _ in 0..30 {
                p.drive(id, held);
                p.step(DT);
            }
            let m = p.vehicle_transform(id).unwrap();
            let v = p.vehicle_velocity(id);
            let ahead = m.transform_vector3(Vec3::Z).with_y(0.0).normalize();
            let side = m.transform_vector3(Vec3::X).with_y(0.0).normalize();
            (v.dot(side).abs(), v.dot(ahead))
        };
        let (plain, lifted, kept) = (slid(0.0, 0.0), slid(0.9, 0.0), slid(0.0, 1.0));
        assert!(plain.1.abs() < 0.05, "{plain:?}");
        assert!(lifted.0 > plain.0 + 0.3, "slides {lifted:?}, {plain:?}");
        assert!(kept.1 > 1.0, "keeps {kept:?}");
    }

    // A helicopter strafes as the 360 code's stick across asks
    // (0x11C78A3C; 0x821A25E0): hovering, held right it slides to its
    // right (-x, facing +z) toward its `MaxRightSpeed` (banked into the
    // slide its `BankYawAmplification` turns it a little too).
    // An AI's stick wobbles as the 360's 0x821A1DC0 has it: a push ahead
    // of a quarter to a half, a slide of up to 2 either way held to 1, a
    // pull down of a quarter to a half; none on the ground past 1/2 s.
    #[test]
    fn an_ai_s_stick_wobbles_in_the_air_only() {
        for i in 0..400 {
            let [a, s, l] = ai_wobble(i as f32 * 0.37, 0.0);
            assert!((0.25..=0.5).contains(&a), "{a}");
            assert!((-1.0..=1.0).contains(&s), "{s}");
            assert!((-0.5..=-0.25).contains(&l), "{l}");
        }
        assert_eq!(ai_wobble(3.0, 0.6), [0.0; 3]);
    }

    // 0x82153B30: a building, the ground or debris lets a road vehicle
    // come out in its box; a prop there doesn't.
    #[test]
    fn a_spawn_box_is_kept_by_props_not_by_buildings() {
        let mut p = Physics::new();
        let block = CollisionShape::cuboid(Vec3::new(3.0, 4.0, 3.0));
        p.add_static(StaticKey::Cell(1), &block, Vec3::ZERO, Quat::IDENTITY);
        for (k, x) in [(2, 20.0), (3, 60.0)] {
            let at = Vec3::new(x, 0.0, 0.0);
            p.add_static(StaticKey::Object(k), &block, at, Quat::IDENTITY);
        }
        let piece = LoosePiece::block(None, Vec3::ZERO, Vec3::splat(0.5), 30.0);
        let at = Mat4::from_translation(Vec3::new(40.0, 1.0, 0.0));
        p.add_debris(&piece, at, Vec3::ZERO, Vec3::ZERO);
        p.step(DT);
        // Object 3 is a fixed prop, object 2 a building.
        let clear = |x: f32| {
            let (lo, hi) = (Vec3::new(x - 2.0, 0.2, -1.0), Vec3::new(x + 2.0, 3.0, 1.0));
            p.box_clear(lo, hi, |k| k == StaticKey::Object(3))
        };
        assert!(clear(0.0));
        assert!(clear(20.0));
        assert!(clear(40.0));
        assert!(!clear(60.0));
    }

    // What stands fixed in a boat's area, without the terrain.
    #[test]
    fn fixed_near_gives_placed_things_not_the_ground() {
        let mut p = Physics::new();
        let block = CollisionShape::cuboid(Vec3::new(3.0, 4.0, 0.0));
        let at = Vec3::new(10.0, 0.0, 20.0);
        p.add_static(StaticKey::Cell(1), &block, Vec3::ZERO, Quat::IDENTITY);
        p.add_static(StaticKey::Object(2), &block, at, Quat::IDENTITY);
        p.step(DT);
        let found = p.fixed_near(Vec3::splat(-50.0), Vec3::splat(50.0));
        assert_eq!(found.len(), 1);
        assert!(found[0].0.distance(at) < 0.1, "{:?}", found[0]);
        assert!((found[0].1 - 5.0).abs() < 0.1, "{:?}", found[0]);
        assert!(
            p.fixed_near(Vec3::splat(60.0), Vec3::splat(90.0))
                .is_empty()
        );
    }

    // An AI pilot takes a helicopter up off the ground and holds it 30 m
    // over it, its stick wobbling ([`crate::heli_pilot`]).
    #[test]
    fn an_ai_pilot_hovers_a_helicopter_30_m_up() {
        let mut p = Physics::new();
        let ground = CollisionShape::cuboid(Vec3::new(2000.0, 1.0, 2000.0));
        p.add_static(
            StaticKey::Cell(1),
            &ground,
            Vec3::new(0.0, -1.0, 0.0),
            Quat::IDENTITY,
        );
        let spec = helicopter();
        let id = p.add_vehicle(&spec, Vec3::new(0.0, 0.1, 0.0), 0.0);
        p.set_ai_pilot(id, true);
        let mut pilot = crate::heli_pilot::HeliPilot::new(
            crate::tuning::PilotTuning::DEFAULT,
            &spec.tuning.flight.unwrap(),
        );
        for _ in 0..(25.0 / DT) as usize {
            let m = p.vehicle_transform(id).unwrap();
            let (_, turn, at) = m.to_scale_rotation_translation();
            let g = p.ground_under(id, at);
            pilot.sense(at, turn, p.vehicle_velocity(id), g, DT);
            let stick = pilot.hover(30.0, true);
            p.pilot_helicopter(id, stick);
            p.step(DT);
        }
        let y = p.vehicle_transform(id).unwrap().w_axis.y;
        assert!((y - 30.0).abs() < 1.5, "at {y}");
        assert!(p.vehicle_velocity(id).length() < 1.0);
    }

    #[test]
    fn a_helicopter_strafes_to_its_right() {
        let mut p = Physics::new();
        let ground = CollisionShape::cuboid(Vec3::new(2000.0, 1.0, 2000.0));
        p.add_static(
            StaticKey::Cell(1),
            &ground,
            Vec3::new(0.0, -1.0, 0.0),
            Quat::IDENTITY,
        );
        let spec = helicopter();
        let top = spec.tuning.flight.unwrap().sideways;
        let id = p.add_vehicle(&spec, Vec3::new(0.0, 0.1, 0.0), 0.0);
        let flown = VehicleControls::default();
        p.fly(id, 1.0);
        for _ in 0..600 {
            p.drive(id, flown);
            p.step(DT);
        }
        p.fly(id, 0.0);
        p.strafe(id, 1.0);
        for _ in 0..600 {
            p.drive(id, flown);
            p.step(DT);
        }
        let v = p.vehicle_velocity(id);
        let right = p
            .vehicle_transform(id)
            .unwrap()
            .transform_vector3(Vec3::NEG_X)
            .with_y(0.0)
            .normalize();
        let across = v.dot(right);
        assert!(
            across > 0.6 * top && across < 1.1 * top,
            "slides at {v} of {top}"
        );
    }

    // A boat that hits something loses its roll (`CollisionStabilization`
    // 1, the 360 record's default; 0x8219A7xx); afloat and clear of
    // things it rolls on as its springs let it.
    #[test]
    fn a_boat_that_hits_something_stops_rolling() {
        let rolled = |wall: bool, stabilization: f32| {
            let mut p = Physics::new();
            p.set_water(0.0);
            if wall {
                let block = CollisionShape::cuboid(Vec3::new(20.0, 5.0, 1.0));
                p.add_static(
                    StaticKey::Cell(1),
                    &block,
                    Vec3::new(0.0, 0.0, 4.5),
                    Quat::IDENTITY,
                );
            }
            let mut spec = boat();
            spec.tuning.boat = Some(BoatTuning {
                float: BoatFloat {
                    stabilization,
                    ..BoatFloat::DEFAULT
                },
                ..BOAT
            });
            let id = p.add_vehicle(&spec, Vec3::new(0.0, -0.5, 0.0), 0.0);
            for _ in 0..120 {
                p.drive(id, VehicleControls::default());
                p.step(DT);
            }
            // Rolling hard and driven at 8 m/s into the wall's face half a
            // metre ahead; a sixth of a second on.
            let v = p.vehicle_velocity(id);
            p.set_vehicle_velocity(id, v.with_z(8.0));
            let h = p.vehicles[id.0].as_ref().unwrap().controller.chassis;
            p.world
                .bodies
                .get_mut(h)
                .unwrap()
                .set_angvel(vector(Vec3::Z * 2.0), true);
            for _ in 0..10 {
                p.drive(id, VehicleControls::default());
                p.step(DT);
            }
            let m = p.vehicle_transform(id).unwrap();
            vec3(p.world.bodies.get(h).unwrap().angvel())
                .dot(m.transform_vector3(Vec3::Z))
                .abs()
        };
        let (open, hit, unheld) = (rolled(false, 1.0), rolled(true, 1.0), rolled(true, 0.0));
        assert!(hit < 0.35 * open, "rolls {hit} after the hit, {open} clear");
        assert!(hit < 0.5 * unheld, "rolls {hit}, {unheld} without it");
    }

    // The 360 code's noise is smooth and stays within 0 to 1, a
    // helicopter's phase within 0 to 1, and its turbulence is nothing with
    // its rotor stopped or a second on the ground, at most 8 degrees x 3
    // across (0x828566B0, 0x821A2F00).
    #[test]
    fn helicopter_turbulence_shakes_only_in_the_air_with_its_rotor_turning() {
        let mut last = noise(0.0);
        for i in 1..20_000 {
            let n = noise(i as f32 * 0.01 - 50.0);
            assert!((0.0..=1.0).contains(&n), "{n}");
            assert!((n - last).abs() < 0.05, "smooth: {last} to {n}");
            last = n;
        }
        // At whole steps the gradient adds nothing: 0.5.
        assert_eq!(noise(7.0), 0.5);
        for id in 0..500 {
            assert!((0.0..1.0).contains(&turbulence_phase(id)));
        }
        assert_ne!(turbulence_phase(1), turbulence_phase(2));
        assert_eq!(turbulence(0.3, 12.0, 0.0, 1.0, 0.0), (0.0, 0.0));
        assert_eq!(turbulence(0.3, 12.0, 1.0, 0.0, 1.0), (0.0, 0.0));
        let most = 8f32.to_radians();
        let (mut big_a, mut big_b) = (0.0f32, 0.0f32);
        for i in 0..6000 {
            let (a, b) = turbulence(0.3, i as f32 / 60.0, 1.0, 1.0, 0.0);
            assert!(a.abs() <= most && b.abs() <= 3.0 * most);
            big_a = big_a.max(a.abs());
            big_b = big_b.max(b.abs());
        }
        assert!(big_a > 0.1 * most && big_b > big_a, "{big_a} {big_b}");
        // Half a second on the ground, half as much.
        let (a, b) = turbulence(0.3, 5.0, 1.0, 0.0, 0.5);
        let (a1, b1) = turbulence(0.3, 5.0, 1.0, 1.0, 0.0);
        assert!((a - a1 * 0.5).abs() < 1e-6 && (b - b1 * 0.5).abs() < 1e-6);
    }

    /// A car with a turret and a gun on bodies of their own: chassis
    /// 30 t, turret 3 t, gun 80 kg.
    fn gun_tank() -> VehicleSpec {
        let cube = |lo: Vec3, hi: Vec3| {
            let mut v = Vec::new();
            for x in [lo.x, hi.x] {
                for y in [lo.y, hi.y] {
                    for z in [lo.z, hi.z] {
                        v.push(Vec3::new(x, y, z));
                    }
                }
            }
            v
        };
        VehicleSpec {
            path_unseen: Default::default(),
            turret_shapes: vec![cube(Vec3::new(-1.0, 1.6, -1.0), Vec3::new(1.0, 2.4, 1.0))].into(),
            turret_mass: Some(crate::vehicle::Chassis {
                mass: 3000.0,
                inertia: Mat3::from_diagonal(Vec3::new(2000.0, 2500.0, 1000.0)),
                centre_of_mass: Vec3::new(0.0, 2.0, 0.0),
            }),
            gun_body: Some(crate::vehicle::GunBody {
                shapes: vec![cube(Vec3::new(-0.1, 1.9, 1.1), Vec3::new(0.1, 2.1, 4.0))].into(),
                mass: crate::vehicle::Chassis {
                    mass: 80.0,
                    inertia: Mat3::from_diagonal(Vec3::new(60.0, 60.0, 0.5)),
                    centre_of_mass: Vec3::new(0.0, 2.0, 2.0),
                },
            }),
            gun_pivot: Some(Vec3::new(0.0, 2.0, 1.0)),
            pivot: Some(Vec3::new(0.0, 2.0, 0.0)),
            mass: 30000.0,
            tuning: crate::tuning::VehicleTuning {
                turret: Some(crate::tuning::TurretTuning {
                    main: true,
                    yaw_speed: 1.0,
                    yaw_accel: 20.0,
                    pitch: (-0.1, 0.3),
                    pitch_speed: 1.0,
                    pitch_accel: 20.0,
                    yaw: None,
                }),
                ..car().tuning
            },
            ..car()
        }
    }

    // A turret on its own body follows the angle asked on its hinge's
    // motor (20 x the angle to go, a second; 0x82273F90): asked to move at
    // 1 rad/s it keeps up a twentieth of a second behind, and turns the
    // hull it sits on hardly at all.
    #[test]
    fn a_vehicle_s_mass_is_its_bodies_together_and_none_while_held() {
        // 0x82182418: every body's mass summed; 0 once one does not move.
        let mut p = Physics::new();
        let id = p.add_vehicle(&gun_tank(), Vec3::new(0.0, 0.5, 0.0), 0.0);
        let m = p.vehicle_mass(id).unwrap();
        assert!((m - 33_080.0).abs() < 1.0, "{m}");
        p.hold_vehicle(id, true);
        assert_eq!(p.vehicle_mass(id), Some(0.0));
        p.hold_vehicle(id, false);
        assert!((p.vehicle_mass(id).unwrap() - 33_080.0).abs() < 1.0);
        // A car alone: its chassis.
        let alone = p.add_vehicle(&car(), Vec3::new(10.0, 0.5, 0.0), 0.0);
        assert!((p.vehicle_mass(alone).unwrap() - car().mass).abs() < 1.0);
        assert_eq!(p.vehicle_mass(VehicleId(99)), None);
    }

    /// A shot from a tank's own gun passes through all of it: its hull,
    /// and its turret and gun on bodies of their own (playtest 11-12: a
    /// hijacked tank's shell burst in its own barrel). Anything else
    /// still stops it.
    #[test]
    fn a_shot_from_its_own_gun_passes_through_its_turret_and_gun() {
        let mut p = Physics::new();
        let id = p.add_vehicle(&gun_tank(), Vec3::new(0.0, 0.5, 0.0), 0.0);
        p.step(1.0 / 60.0);
        let at = p.vehicle_transform(id).unwrap();
        // From inside the barrel, along it, and from the turret's middle.
        let barrel = at.transform_point3(Vec3::new(0.0, 2.0, 1.5));
        let ahead = at.transform_vector3(Vec3::Z);
        let middle = at.transform_point3(Vec3::new(0.0, 2.0, 0.0));
        assert!(
            p.cast_ray(barrel, ahead, 10.0).is_some(),
            "its gun is there"
        );
        assert_eq!(p.cast_ray_past(barrel, ahead, 10.0, id), None);
        assert_eq!(p.cast_ray_past(middle, ahead, 10.0, id), None);
        // Another vehicle ahead is met, from inside its own hull.
        let other = p.add_vehicle(&car(), at.transform_point3(Vec3::new(0.0, 0.0, 8.0)), 0.0);
        p.step(1.0 / 60.0);
        // Half a metre under the other's roof.
        let over = p.vehicle_transform(other).unwrap().w_axis.truncate() + Vec3::Y * 5.0;
        let roof = over.y - p.cast_ray(over, Vec3::NEG_Y, 20.0).unwrap().0;
        let hull = at.transform_point3(Vec3::ZERO).with_y(roof - 0.5);
        let hit = p.cast_ray_past(hull, ahead, 20.0, id);
        assert!(
            hit.is_some_and(|(t, _)| t > 4.0 && t < 10.0),
            "{hit:?} {other:?}"
        );
    }

    #[test]
    fn a_vehicle_shoved_through_its_centre_moves_without_turning() {
        // 0x8216DCC8 -> 0x8262AB60: a linear impulse on the chassis body.
        let mut p = Physics::new();
        let id = p.add_vehicle(&car(), Vec3::new(0.0, 5.0, 0.0), 0.0);
        let mass = p.vehicle_mass(id).unwrap();
        let before = p.vehicle_velocity(id);
        p.impulse_vehicle(id, Vec3::new(mass * 2.0, 0.0, 0.0));
        let gained = p.vehicle_velocity(id) - before;
        assert!((gained.x - 2.0).abs() < 1e-3, "{gained:?}");
        assert!(p.vehicle_spin(id).length() < 1e-4);
        // At a point off its centre it also turns.
        let m = p.vehicle_transform(id).unwrap();
        p.push_vehicle(
            id,
            Vec3::new(mass, 0.0, 0.0),
            m.transform_point3(Vec3::new(0.0, 0.0, 2.0)),
        );
        assert!(p.vehicle_spin(id).length() > 1e-3);
    }

    #[test]
    fn a_sleeping_gun_wakes_only_when_its_pitch_asked_moves_fast_enough() {
        // 0x82275608: a sleeping gun's body is woken only when its pitch
        // speed squared is over 0.005 (about 0.07 radians a second).
        let mut p = Physics::new();
        let ground = CollisionShape::cuboid(Vec3::new(200.0, 1.0, 200.0));
        p.add_static(
            StaticKey::Cell(1),
            &ground,
            Vec3::new(0.0, -1.0, 0.0),
            Quat::IDENTITY,
        );
        let spec = gun_tank();
        let id = p.add_vehicle(&spec, Vec3::new(0.0, 0.5, 0.0), 0.0);
        for _ in 0..60 {
            p.drive(id, VehicleControls::default());
            p.step(DT);
        }
        let v = p.vehicles[id.0].as_ref().unwrap();
        let h = v.hinge.as_ref().unwrap();
        let bodies = [v.controller.chassis, h.body, h.gun.as_ref().unwrap().body];
        let gun = bodies[2];
        for b in bodies {
            p.world.bodies[b].sleep();
        }
        let start = p.gun_pitch(id).unwrap();
        // Raised at 0.03 radians a second: it sleeps on, where it was (for
        // 12 steps: this car's hull wakes itself every step, and rapier
        // wakes the rest of its island from it some 18 steps on).
        let mut pitch = 0.0;
        for _ in 0..12 {
            pitch += 0.03 * DT;
            p.aim_gun(id, pitch);
            p.step(DT);
        }
        assert!(p.world.bodies[gun].is_sleeping(), "woken at 0.03 rad/s");
        assert!((p.gun_pitch(id).unwrap() - start).abs() < 1e-4);
        // Raised at 0.3 radians a second: woken, it follows.
        for _ in 0..30 {
            pitch += 0.3 * DT;
            p.aim_gun(id, pitch);
            p.drive(id, VehicleControls::default());
            p.step(DT);
        }
        assert!(
            !p.world.bodies[gun].is_sleeping(),
            "still asleep at 0.3 rad/s"
        );
        let up = p.gun_pitch(id).unwrap() - start;
        assert!(up > 0.1, "rose {up}");
    }

    #[test]
    fn a_turret_s_gun_on_its_own_body_tilts_to_the_pitch_asked() {
        let mut p = Physics::new();
        let ground = CollisionShape::cuboid(Vec3::new(200.0, 1.0, 200.0));
        p.add_static(
            StaticKey::Cell(1),
            &ground,
            Vec3::new(0.0, -1.0, 0.0),
            Quat::IDENTITY,
        );
        let cube = |lo: Vec3, hi: Vec3| {
            let mut v = Vec::new();
            for x in [lo.x, hi.x] {
                for y in [lo.y, hi.y] {
                    for z in [lo.z, hi.z] {
                        v.push(Vec3::new(x, y, z));
                    }
                }
            }
            v
        };
        let spec = VehicleSpec {
            path_unseen: Default::default(),
            turret_shapes: vec![cube(Vec3::new(-1.0, 1.6, -1.0), Vec3::new(1.0, 2.4, 1.0))].into(),
            turret_mass: Some(crate::vehicle::Chassis {
                mass: 3000.0,
                inertia: Mat3::from_diagonal(Vec3::new(2000.0, 2500.0, 1000.0)),
                centre_of_mass: Vec3::new(0.0, 2.0, 0.0),
            }),
            gun_body: Some(crate::vehicle::GunBody {
                shapes: vec![cube(Vec3::new(-0.1, 1.9, 1.1), Vec3::new(0.1, 2.1, 4.0))].into(),
                mass: crate::vehicle::Chassis {
                    mass: 80.0,
                    inertia: Mat3::from_diagonal(Vec3::new(60.0, 60.0, 0.5)),
                    centre_of_mass: Vec3::new(0.0, 2.0, 2.0),
                },
            }),
            gun_pivot: Some(Vec3::new(0.0, 2.0, 1.0)),
            pivot: Some(Vec3::new(0.0, 2.0, 0.0)),
            mass: 30000.0,
            tuning: crate::tuning::VehicleTuning {
                turret: Some(crate::tuning::TurretTuning {
                    main: true,
                    yaw_speed: 1.0,
                    yaw_accel: 20.0,
                    pitch: (-0.1, 0.3),
                    pitch_speed: 1.0,
                    pitch_accel: 20.0,
                    yaw: None,
                }),
                ..car().tuning
            },
            ..car()
        };
        let id = p.add_vehicle(&spec, Vec3::new(0.0, 0.5, 0.0), 0.0);
        for _ in 0..60 {
            p.drive(id, VehicleControls::default());
            p.step(DT);
        }
        assert!(
            p.gun_pitch(id).unwrap().abs() < 0.02,
            "{:?}",
            p.gun_pitch(id)
        );
        // Asked up 0.2: it gets there, the muzzle up.
        p.aim_gun(id, 0.2);
        for _ in 0..60 {
            p.drive(id, VehicleControls::default());
            p.step(DT);
        }
        let up = p.gun_pitch(id).unwrap();
        assert!((up - 0.2).abs() < 0.02, "{up}");
        // Asked past its `MaxPitch`, it stops there.
        p.aim_gun(id, 0.8);
        for _ in 0..90 {
            p.drive(id, VehicleControls::default());
            p.step(DT);
        }
        let top = p.gun_pitch(id).unwrap();
        assert!((top - 0.3).abs() < 0.03, "{top}");
        assert_eq!(p.gun_target(id), Some(0.8));
        // It turns with its turret.
        p.turn_turret(id, 1.0);
        for _ in 0..120 {
            p.drive(id, VehicleControls::default());
            p.step(DT);
        }
        assert!((p.gun_pitch(id).unwrap() - 0.3).abs() < 0.03);
    }

    // A turret on its own body is turned by the 360's turret action
    // (0x821A65D0): left alone until first asked; asked a whole radian at
    // once, it speeds up no faster than its `MaxYawAccel`, turns no faster
    // than its `MaxYawSpeed`, and settles on the angle asked, the hull
    // left as it was.
    #[test]
    fn a_sleeping_turret_wakes_only_when_asked_more_than_its_threshold_off() {
        // 0x821A64B8: a sleeping turret's body is woken only when the angle
        // asked is more than sqrt(0.001) radians (1.8 degrees) off.
        let mut p = Physics::new();
        let ground = CollisionShape::cuboid(Vec3::new(200.0, 1.0, 200.0));
        p.add_static(
            StaticKey::Cell(1),
            &ground,
            Vec3::new(0.0, -1.0, 0.0),
            Quat::IDENTITY,
        );
        let mut block = Vec::new();
        for x in [-1.0, 1.0] {
            for y in [1.6, 2.4] {
                for z in [-1.0, 2.0] {
                    block.push(Vec3::new(x, y, z));
                }
            }
        }
        let spec = VehicleSpec {
            path_unseen: Default::default(),
            turret_shapes: vec![block].into(),
            turret_mass: Some(crate::vehicle::Chassis {
                mass: 3000.0,
                inertia: Mat3::from_diagonal(Vec3::new(2000.0, 2500.0, 1000.0)),
                centre_of_mass: Vec3::new(0.0, 2.0, 0.3),
            }),
            pivot: Some(Vec3::new(0.0, 2.0, 0.0)),
            mass: 30000.0,
            tuning: crate::tuning::VehicleTuning {
                turret: Some(crate::tuning::TurretTuning {
                    main: true,
                    yaw_speed: 0.5,
                    yaw_accel: 2.0,
                    pitch: (-0.1, 0.3),
                    pitch_speed: 1.0,
                    pitch_accel: 20.0,
                    yaw: None,
                }),
                ..car().tuning
            },
            ..car()
        };
        let id = p.add_vehicle(&spec, Vec3::new(0.0, 0.5, 0.0), 0.0);
        for _ in 0..60 {
            p.drive(id, VehicleControls::default());
            p.step(DT);
        }
        let start = p.turret_yaw(id).unwrap();
        let v = p.vehicles[id.0].as_ref().unwrap();
        let (chassis, turret) = (v.controller.chassis, v.hinge.as_ref().unwrap().body);
        p.turn_turret(id, start);
        p.world.bodies[chassis].sleep();
        p.world.bodies[turret].sleep();
        let asleep = |p: &Physics| p.world.bodies[turret].is_sleeping();
        // 1.5 degrees off: it sleeps on.
        for _ in 0..30 {
            p.turn_turret(id, start + 1.5f32.to_radians());
            p.step(DT);
        }
        assert!(asleep(&p), "woken 1.5 degrees off");
        assert!((p.turret_yaw(id).unwrap() - start).abs() < 1e-4);
        // 2.5 degrees off: woken, it turns there.
        for _ in 0..120 {
            p.turn_turret(id, start + 2.5f32.to_radians());
            p.drive(id, VehicleControls::default());
            p.step(DT);
        }
        let turned = (p.turret_yaw(id).unwrap() - start).to_degrees();
        assert!(turned > 1.5, "turned {turned} degrees");
    }

    #[test]
    fn a_frame_is_stepped_as_the_360_steps_it() {
        // 0x82152E20: n = trunc(dt x 59.999996 + 0.99) held to 1..=3.
        assert_eq!(frame_steps(0.0005), (0, 0.0));
        assert_eq!(frame_steps(1.0 / 60.0).0, 1);
        assert_eq!(frame_steps(1.0 / 120.0), (1, 1.0 / 120.0));
        let (n, each) = frame_steps(1.0 / 30.0);
        assert_eq!(n, 2);
        assert!((each - 1.0 / 60.0).abs() < 1e-7);
        assert_eq!(frame_steps(0.02).0, 2);
        assert_eq!(frame_steps(0.04).0, 3);
        assert_eq!(frame_steps(0.2).0, 3);
        // A frame of 1/30 s moves a falling body as two steps of 1/60 do.
        let fall = |frame: bool| {
            let mut p = Physics::new();
            let id = p.add_vehicle(&car(), Vec3::new(0.0, 100.0, 0.0), 0.0);
            for _ in 0..30 {
                if frame {
                    p.step_frame(1.0 / 30.0);
                } else {
                    p.step(1.0 / 60.0);
                    p.step(1.0 / 60.0);
                }
            }
            p.vehicle_transform(id).unwrap().w_axis.y
        };
        assert_eq!(fall(true), fall(false));
    }

    #[test]
    fn a_turret_on_its_own_body_turns_as_the_360_action_turns_it() {
        let mut p = Physics::new();
        let ground = CollisionShape::cuboid(Vec3::new(200.0, 1.0, 200.0));
        p.add_static(
            StaticKey::Cell(1),
            &ground,
            Vec3::new(0.0, -1.0, 0.0),
            Quat::IDENTITY,
        );
        let mut block = Vec::new();
        for x in [-1.0, 1.0] {
            for y in [1.6, 2.4] {
                for z in [-1.0, 2.0] {
                    block.push(Vec3::new(x, y, z));
                }
            }
        }
        let spec = VehicleSpec {
            path_unseen: Default::default(),
            turret_shapes: vec![block].into(),
            turret_mass: Some(crate::vehicle::Chassis {
                mass: 3000.0,
                inertia: Mat3::from_diagonal(Vec3::new(2000.0, 2500.0, 1000.0)),
                centre_of_mass: Vec3::new(0.0, 2.0, 0.3),
            }),
            pivot: Some(Vec3::new(0.0, 2.0, 0.0)),
            mass: 30000.0,
            tuning: crate::tuning::VehicleTuning {
                turret: Some(crate::tuning::TurretTuning {
                    main: true,
                    yaw_speed: 0.5,
                    yaw_accel: 2.0,
                    pitch: (-0.1, 0.3),
                    pitch_speed: 1.0,
                    pitch_accel: 20.0,
                    yaw: None,
                }),
                ..car().tuning
            },
            ..car()
        };
        let id = p.add_vehicle(&spec, Vec3::new(0.0, 0.5, 0.0), 0.0);
        for _ in 0..60 {
            p.drive(id, VehicleControls::default());
            p.step(DT);
        }
        assert!(p.turret_yaw(id).unwrap().abs() < 0.01);
        let heading = |p: &Physics| {
            let f = p.vehicle_transform(id).unwrap().transform_vector3(Vec3::Z);
            f.x.atan2(f.z)
        };
        let before = heading(&p);
        let (mut last, mut fastest, mut early) = (0.0f32, 0.0f32, 0.0f32);
        for i in 0..360 {
            p.turn_turret(id, 1.0);
            p.drive(id, VehicleControls::default());
            p.step(DT);
            let yaw = p.turret_yaw(id).unwrap();
            let rate = (yaw - last) / DT;
            fastest = fastest.max(rate);
            if i < 6 {
                early = early.max(rate);
            }
            last = yaw;
        }
        assert!(early < 2.0 * 0.1 + 0.05, "sped up to {early} in 0.1 s");
        assert!(fastest < 0.5 + 0.03, "turned at {fastest}");
        assert!(fastest > 0.45, "turned at {fastest}");
        assert!((last - 1.0).abs() < 0.02, "at {last}");
        assert!((heading(&p) - before).abs() < 0.05, "hull turned");
    }

    // A vehicle's flags 0x28 hull shape (layer 11) is no path obstacle
    // (0x822E6FD8's mask), its others are; the car still bumps into both.
    #[test]
    fn a_path_sweep_misses_a_vehicles_shape_on_a_layer_it_doesnt_see() {
        let block = |x: f32| {
            let mut v = Vec::new();
            for dx in [-0.5, 0.5] {
                for y in [0.5, 1.5] {
                    for z in [-1.0, 1.0] {
                        v.push(Vec3::new(x + dx, y, z));
                    }
                }
            }
            v
        };
        let spec = VehicleSpec {
            shapes: vec![block(0.0), block(6.0)].into(),
            path_unseen: vec![1].into(),
            ..car()
        };
        let mut p = Physics::new();
        let id = p.add_vehicle(&spec, Vec3::ZERO, 0.0);
        p.hold_vehicle(id, true);
        p.step(DT);
        let at = p.vehicle_transform(id).unwrap();
        let across = |x: f32| {
            let x = at.transform_point3(Vec3::new(x, 1.0, 0.0));
            p.sweep_obstacle(
                x - Vec3::Z * 5.0,
                x + Vec3::Z * 5.0,
                Vec3::new(0.1, 0.1, 0.05),
                None,
            )
        };
        assert!(across(0.0).is_some(), "the seen shape is met");
        assert!(across(6.0).is_none(), "the unseen one isn't");
        let ray = |x: f32| {
            let x = at.transform_point3(Vec3::new(x, 1.0, 0.0));
            p.cast_ray(x - Vec3::Z * 5.0, Vec3::Z, 10.0)
        };
        assert!(ray(6.0).is_some(), "but it is still solid");
    }

    // A path search sizes a vehicle up by all its bodies (0x82182E90): a
    // sweep low past the hull meets it, and the box takes in the turret's
    // block up to 4 m, turned a quarter left out to its left.
    #[test]
    fn a_vehicles_path_box_takes_in_its_turret() {
        let mut p = Physics::new();
        let mut block = Vec::new();
        for x in [-0.5, 0.5] {
            for y in [3.0, 4.0] {
                for z in [1.0, 3.0] {
                    block.push(Vec3::new(x, y, z));
                }
            }
        }
        let spec = VehicleSpec {
            path_unseen: Default::default(),
            turret_shapes: vec![block].into(),
            pivot: Some(Vec3::new(0.0, 3.0, 0.0)),
            ..car()
        };
        let id = p.add_vehicle(&spec, Vec3::ZERO, 0.0);
        p.hold_vehicle(id, true);
        p.turn_turret(id, std::f32::consts::FRAC_PI_2);
        p.step(DT);
        let hull = p.vehicle_transform(id).unwrap();
        let low = hull.transform_point3(Vec3::Y * 0.5).y;
        let o = p
            .sweep_obstacle(
                Vec3::new(-10.0, low, 0.0),
                Vec3::new(10.0, low, 0.0),
                Vec3::new(0.2, 0.2, 0.05),
                None,
            )
            .expect("meets the hull");
        assert!(o.max.y > 3.9, "box top {}", o.max.y);
        assert!(
            o.max.x > 2.9 || o.min.x < -2.9,
            "box {} .. {}",
            o.min,
            o.max
        );
    }

    // A turret's colliders turn with it ([`Physics::turn_turret`]): a
    // block on its front, turned a quarter left, is hit from above on its
    // left and no longer ahead.
    #[test]
    fn a_turrets_colliders_turn_with_it() {
        let mut p = Physics::new();
        let mut block = Vec::new();
        for x in [-0.5, 0.5] {
            for y in [3.0, 4.0] {
                for z in [1.0, 3.0] {
                    block.push(Vec3::new(x, y, z));
                }
            }
        }
        let spec = VehicleSpec {
            path_unseen: Default::default(),
            turret_shapes: vec![block].into(),
            pivot: Some(Vec3::new(0.0, 3.0, 0.0)),
            ..car()
        };
        let id = p.add_vehicle(&spec, Vec3::ZERO, 0.0);
        p.hold_vehicle(id, true);
        p.step(DT);
        let down =
            |p: &Physics, x: f32, z: f32| p.cast_ray(Vec3::new(x, 10.0, z), Vec3::NEG_Y, 20.0);
        assert!(down(&p, 0.0, 2.0).is_some_and(|(d, _)| (d - 6.0).abs() < 0.1));
        assert!(down(&p, 2.0, 0.0).is_none_or(|(d, _)| d > 6.5));
        p.turn_turret(id, std::f32::consts::FRAC_PI_2);
        p.step(DT);
        assert!(down(&p, 2.0, 0.0).is_some_and(|(d, _)| (d - 6.0).abs() < 0.1));
        assert!(down(&p, 0.0, 2.0).is_none_or(|(d, _)| d > 6.5));
    }

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

    #[test]
    fn meshes_keep_their_triangles_surfaces_past_dropped_ones() {
        let v = [Vec3::ZERO, Vec3::X, Vec3::Z, Vec3::new(1.0, 0.0, 1.0)];
        // The second triangle is degenerate and dropped; the runs follow
        // the triangles kept.
        let shape =
            CollisionShape::mesh_of(&v, &[[0, 1, 2], [0, 0, 1], [1, 3, 2]], &[7, 8, 9]).unwrap();
        assert_eq!(shape.triangles, 2);
        assert_eq!(&shape.surfaces[..], &[(0, 7), (1, 9)]);
        assert_eq!((shape.surface_of(0), shape.surface_of(1)), (7, 9));
        // Past the count, faces wrap round.
        assert_eq!(shape.surface_of(3), 9);
        // Nothing said, nothing kept.
        assert!(
            CollisionShape::mesh(&v, &[[0, 1, 2]])
                .unwrap()
                .surfaces
                .is_empty()
        );
    }

    #[test]
    fn rays_find_the_surface_of_the_triangle_they_strike() {
        let mut p = Physics::new();
        // Two halves of a 4 m floor: dirt on the -x side, rock on the +x.
        let v = [
            Vec3::new(-2.0, 0.0, -2.0),
            Vec3::new(0.0, 0.0, -2.0),
            Vec3::new(0.0, 0.0, 2.0),
            Vec3::new(-2.0, 0.0, 2.0),
            Vec3::new(2.0, 0.0, -2.0),
            Vec3::new(2.0, 0.0, 2.0),
        ];
        let tris = [[0, 2, 1], [0, 3, 2], [1, 2, 5], [1, 5, 4]];
        let shape = CollisionShape::mesh_of(&v, &tris, &[11, 11, 22, 22]).unwrap();
        p.add_static(StaticKey::Cell(5), &shape, Vec3::ZERO, Quat::IDENTITY);
        p.step(DT);
        let down =
            |p: &Physics, x: f32| p.cast_ray_surface(Vec3::new(x, 3.0, 0.5), Vec3::NEG_Y, 10.0);
        let (t, _, key, surface) = down(&p, -1.0).unwrap();
        assert!((t - 3.0).abs() < 1e-3);
        assert_eq!((key, surface), (Some(StaticKey::Cell(5)), 11));
        assert_eq!(down(&p, 1.0).unwrap().3, 22);
        p.remove_static(StaticKey::Cell(5));
        p.step(DT);
        assert!(down(&p, 1.0).is_none());
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

    #[test]
    fn a_ragdoll_stays_in_one_piece_when_spine_and_hips_share_a_root() {
        let mut p = ready();
        // As the game's skeletons are: the hips and the spine both hang
        // from a root bone at the hips (which gets no limb), under a root
        // at the feet.
        let (parents, at): (Vec<Option<usize>>, Vec<Vec3>) = [
            (None, [0.0, 0.0, 0.0]),
            (Some(0), [0.0, 1.0, 0.0]),
            (Some(1), [0.0, 1.0, 0.0]),
            (Some(2), [0.1, 0.95, 0.0]),
            (Some(3), [0.15, 0.5, 0.0]),
            (Some(4), [0.18, 0.1, 0.0]),
            (Some(2), [-0.1, 0.95, 0.0]),
            (Some(6), [-0.15, 0.5, 0.0]),
            (Some(7), [-0.18, 0.1, 0.0]),
            (Some(1), [0.0, 1.0, 0.0]),
            (Some(9), [0.0, 1.2, 0.0]),
            (Some(10), [0.0, 1.4, 0.0]),
            (Some(11), [0.0, 1.55, 0.0]),
            (Some(12), [0.0, 1.7, 0.0]),
        ]
        .into_iter()
        .map(|(parent, a)| (parent, Vec3::from(a) + Vec3::Y * 2.0))
        .unzip();
        let nodes: Vec<Mat4> = at.iter().map(|&a| Mat4::from_translation(a)).collect();
        let id = p.add_ragdoll(&parents, &nodes, Vec3::new(0.0, 0.0, 6.0));
        {
            let doll = p.ragdolls[id.0].as_ref().unwrap();
            let limbs = doll.bodies.iter().flatten().count();
            assert_eq!(doll.joints.len(), limbs - 1, "every limb but one joined");
        }
        for k in 0..240 {
            if k == 60 {
                p.push_ragdoll(id, Vec3::new(4.0, 10.0, -8.0));
            }
            p.step(DT);
        }
        let now = p.ragdoll_nodes(id).unwrap();
        let (hips, spine, head) = (now[2].w_axis, now[9].w_axis, now[13].w_axis);
        assert!(hips.distance(spine) < 0.2, "{hips} {spine}");
        assert!(hips.distance(head) < 0.9, "{hips} {head}");
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

    /// A car with a flat load bed 1 m up over its back half, a 40 kg
    /// crate on it riding at the car's speed, going 12 m/s along +z: the
    /// crate's place on the bed (z, car space) after `secs` rolling on,
    /// then `stop` (the car's speed taken away at once, as a wall does).
    fn crate_on_bed(friction: f32, stop: bool) -> f32 {
        crate_on_bed_as(
            BodyMaterial {
                friction,
                ..BodyMaterial::COMMON
            },
            stop,
        )
    }

    fn crate_on_bed_as(m: BodyMaterial, stop: bool) -> f32 {
        let mut p = ready();
        let id = p.add_vehicle(&car(), Vec3::new(0.0, 0.1, 0.0), 0.0);
        for _ in 0..120 {
            p.step(DT);
        }
        let tray = [
            Vec3::new(-1.0, 1.0, -2.2),
            Vec3::new(1.0, 1.0, -2.2),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(-1.0, 1.0, 0.0),
        ];
        let mut bed = CollisionShape::mesh(&tray, &[[0, 2, 1], [0, 3, 2]]).unwrap();
        bed.material = Some(m);
        assert!(p.add_bed(id, &bed));
        let place = p.vehicle_transform(id).unwrap();
        let mut piece = LoosePiece::block(None, Vec3::ZERO, Vec3::splat(0.25), 40.0);
        piece.material = m;
        let at = place * Mat4::from_translation(Vec3::new(0.0, 1.27, -1.1));
        let speed = Vec3::new(0.0, 0.0, 12.0);
        let up = p.vehicle_velocity(id).y;
        p.set_vehicle_velocity(id, speed.with_y(up));
        let c = p.add_cargo(&piece, at, speed);
        for _ in 0..30 {
            p.step(DT);
        }
        if stop {
            p.set_vehicle_velocity(id, Vec3::ZERO);
        }
        for _ in 0..30 {
            p.step(DT);
        }
        let place = p.vehicle_transform(id).unwrap();
        let on = place.inverse() * p.debris_place(c).unwrap();
        on.w_axis.z
    }

    #[test]
    fn a_body_slides_by_its_record_s_friction_and_damps_by_the_game_s_floor() {
        let body = |unk: [f32; 4]| Body {
            inertia: [1.0; 9],
            centre_of_mass: [0.0; 3],
            mass: 10.0,
            unk,
            unk_words: [0; 8],
        };
        // The commonest record: friction 0.3, damping raised to 0.8, no
        // bounce and no linear damping, whatever +0x38 and +0x40 say.
        let m = BodyMaterial::of(&body([9.0, 0.3, 9.0, 0.3]));
        assert_eq!(m, BodyMaterial::COMMON);
        // One damped harder keeps its own; a garbage one gets the floor.
        assert_eq!(
            BodyMaterial::of(&body([0.3, 0.7, 0.3, 1.0])).angular_damping,
            1.0
        );
        assert_eq!(BodyMaterial::of(&body([0.3, 0.7, 0.3, 1.0])).friction, 0.7);
        let nan = BodyMaterial::of(&body([0.3, 0.3, f32::NAN, f32::NAN]));
        assert_eq!(nan.angular_damping, MIN_ANGULAR_DAMPING);
    }

    #[test]
    fn a_flung_piece_is_held_to_havok_s_speeds() {
        let mut p = ready();
        let piece = LoosePiece::block(None, Vec3::ZERO, Vec3::splat(0.25), 10.0);
        let at = Mat4::from_translation(Vec3::new(0.0, 50.0, 0.0));
        let id = p.add_debris(&piece, at, Vec3::new(500.0, 0.0, 0.0), Vec3::splat(40.0));
        let before = p.debris_place(id).unwrap().w_axis.x;
        p.step(DT);
        p.step(DT);
        let moved = p.debris_place(id).unwrap().w_axis.x - before;
        assert!(moved < MAX_LINEAR_SPEED * 2.0 * DT * 1.05, "{moved}");
    }

    #[test]
    fn goods_ride_loose_on_a_bed_and_slide_when_it_stops_short() {
        // A car's bed holds nothing by friction while it is upright (its
        // hull's, [`car_hull_friction`]): rolling on as the car slows,
        // the crate slides forward of where it was put (z -1.1).
        let rolling = crate_on_bed(0.3, false);
        assert!(rolling > -0.9, "{rolling}");
        // Stopped short, it slides on to the front, however hard it holds.
        let stopped = crate_on_bed(0.3, true);
        assert!(stopped > rolling, "{stopped}");
        let held = crate_on_bed(1.5, true);
        assert!((held - stopped).abs() < 0.05, "{held} {stopped}");
    }

    #[test]
    fn steps_over_a_loose_piece_lying_in_the_way() {
        let mut p = ready();
        // A plank 0.25 m thick knocked down across her way at z = 3, as
        // a static of its height is stepped over.
        let plank = LoosePiece::block(None, Vec3::ZERO, Vec3::new(2.0, 0.125, 0.15), 30.0);
        let at = Mat4::from_translation(Vec3::new(0.0, 0.13, 3.0));
        let id = p.add_debris(&plank, at, Vec3::ZERO, Vec3::ZERO);
        for _ in 0..60 {
            p.step(DT);
        }
        let lying = p.debris_place(id).unwrap().w_axis;
        assert!((lying.z - 3.0).abs() < 0.1 && lying.y < 0.2, "{lying}");
        let mut c = Character::new(Vec3::ZERO);
        walk(&p, &mut c, Vec3::new(0.0, 0.0, 4.0), 3.0);
        assert!(c.position.z > 5.0, "held by the plank: {}", c.position);
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

    #[test]
    fn a_car_on_its_wheels_is_grounded_and_on_its_roof_flipped() {
        // 0x82189A68 / 0x82189A00: grounded with a wheel touching; flipped
        // with its up axis under 0.5 high and none touching.
        let mut p = Physics::new();
        let ground = CollisionShape::cuboid(Vec3::new(200.0, 1.0, 200.0));
        p.add_static(
            StaticKey::Cell(1),
            &ground,
            Vec3::new(0.0, -1.0, 0.0),
            Quat::IDENTITY,
        );
        let upright = p.add_vehicle(&car(), Vec3::new(0.0, 0.5, 0.0), 0.0);
        let roof = p.add_vehicle_turned(
            &car(),
            Vec3::new(20.0, 2.0, 0.0),
            Quat::from_rotation_z(std::f32::consts::PI),
        );
        let high = p.add_vehicle(&car(), Vec3::new(-20.0, 50.0, 0.0), 0.0);
        for _ in 0..30 {
            p.step(1.0 / 60.0);
        }
        assert_eq!(p.vehicle_grounded(upright, true), Some(true));
        assert_eq!(p.vehicle_flipped(upright, true), Some(false));
        assert_eq!(p.vehicle_grounded(roof, true), Some(false));
        assert_eq!(p.vehicle_flipped(roof, true), Some(true));
        assert_eq!(p.vehicle_grounded(high, true), Some(false));
        // Anything not a car stands on the ground and never flips.
        assert_eq!(p.vehicle_grounded(high, false), Some(true));
        assert_eq!(p.vehicle_flipped(roof, false), Some(false));
    }

    pub(super) fn car() -> VehicleSpec {
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
            node_names: Vec::new(),
            armed: Vec::new(),
            wheels: vec![
                wheel(-0.9, 1.4),
                wheel(0.9, 1.4),
                wheel(-0.9, -1.3),
                wheel(0.9, -1.3),
            ],
            bounds: (Vec3::new(-1.0, 0.0, -2.2), Vec3::new(1.0, 1.5, 2.2)),
            seat: None,
            door: None,
            seating: None,
            entrances: Vec::new(),
            tracked: false,
            road_wheels: Vec::new(),
            rest: Vec::new(),
            node_wheel: Vec::new(),
            node_rolls: Vec::new(),
            mass: 1400.0,
            turret: Vec::new(),
            pivot: None,
            muzzle: None,
            gun: Vec::new(),
            gun_pivot: None,
            muzzle_on_gun: false,
            elevation: (0.0, 0.0),
            hull: (Vec3::new(-1.0, 0.0, -2.2), Vec3::new(1.0, 1.5, 2.2)),
            turret_box: None,
            rotors: Vec::new(),
            tilts: Vec::new(),
            bomb_bay: None,
            node_parents: Vec::new(),
            aimers: Vec::new(),
            damage: None,
            chassis: None,
            shapes: Default::default(),
            material: None,
            path_unseen: Default::default(),
            turret_shapes: Default::default(),
            turret_mass: None,
            gun_body: None,
            wreck: None,
            tuning: crate::VehicleTuning::default(),
        }
    }

    #[test]
    fn a_car_without_a_template_mass_keeps_its_chassis_record_s() {
        // 0x82182848: a body builder handed no mass above 0 keeps its
        // `PHY2` record's; a template's `Mass` replaces it.
        let mass = |p: &Physics, id: VehicleId| p.vehicles[id.0].as_ref().unwrap().mass;
        let record = crate::vehicle::Chassis {
            mass: 5000.0,
            inertia: Mat3::from_diagonal(Vec3::new(4000.0, 5000.0, 2000.0)),
            centre_of_mass: Vec3::new(0.0, 0.5, 0.0),
        };
        let mut p = Physics::new();
        let bare = VehicleSpec {
            chassis: Some(record),
            ..car()
        };
        let id = p.add_vehicle(&bare, Vec3::ZERO, 0.0);
        assert_eq!(mass(&p, id), 5000.0);
        let mut tuned = bare.clone();
        tuned.tuning.mass = Some(1500.0);
        tuned.mass = 1500.0;
        let id = p.add_vehicle(&tuned, Vec3::new(10.0, 0.0, 0.0), 0.0);
        assert_eq!(mass(&p, id), 1500.0);
        // Neither: the mass by kind it was made with.
        let id = p.add_vehicle(&car(), Vec3::new(20.0, 0.0, 0.0), 0.0);
        assert_eq!(mass(&p, id), car().mass);
    }

    fn boat() -> VehicleSpec {
        let hull = (Vec3::new(-1.2, -0.7, -3.0), Vec3::new(1.2, 1.0, 3.0));
        VehicleSpec {
            kind: crate::vehicle::BOAT.into(),
            wheels: Vec::new(),
            bounds: hull,
            hull,
            mass: 2000.0,
            ..car()
        }
    }

    fn helicopter() -> VehicleSpec {
        let hull = (Vec3::new(-6.0, 0.0, -7.0), Vec3::new(6.0, 3.5, 7.0));
        VehicleSpec {
            kind: crate::vehicle::HELICOPTER.into(),
            wheels: Vec::new(),
            bounds: hull,
            hull,
            mass: 8300.0,
            tuning: crate::VehicleTuning {
                // The AH-1Z's.
                flight: Some(crate::tuning::FlightTuning {
                    climb: 25.0,
                    forward: 30.0,
                    sideways: 30.0,
                    forward_accel: 0.8,
                    sideways_accel: 1.0,
                    yaw: 3.5,
                    yaw_accel: 3.0,
                    yaw_damp: 1.0,
                    pitch: 0.4,
                    pitch_accel: 0.9,
                    roll_forward: 0.5,
                    roll_side: 0.3,
                    roll_accel: 1.0,
                    low_speed_yaw: 6.5,
                    low_speed: 25.0,
                    high_speed_yaw_loss: 0.2,
                    bank_speed: 25.0,
                    roll_bank: 0.3,
                    bank_yaw: 5.0,
                    no_input_inertia: 0.6,
                }),
                ..Default::default()
            },
            ..car()
        }
    }

    #[test]
    fn a_motorbike_stays_up_and_leans_into_a_turn() {
        let mut p = Physics::new();
        let ground = CollisionShape::cuboid(Vec3::new(2000.0, 1.0, 2000.0));
        p.add_static(
            StaticKey::Cell(1),
            &ground,
            Vec3::new(0.0, -1.0, 0.0),
            Quat::IDENTITY,
        );
        let wheel = |z: f32| crate::vehicle::WheelSpec {
            centre: Vec3::new(0.0, 0.3, z),
            radius: 0.3,
            width: 0.15,
            steers: z > 0.0,
        };
        let hull = (Vec3::new(-0.45, 0.0, -1.1), Vec3::new(0.45, 1.3, 1.1));
        let bike = VehicleSpec {
            wheels: vec![wheel(0.7), wheel(-0.75)],
            bounds: hull,
            hull,
            mass: 250.0,
            ..car()
        };
        assert!(bike.is_bike());
        let id = p.add_vehicle(&bike, Vec3::new(0.0, 0.2, 0.0), 0.0);
        let place = |p: &Physics| p.vehicle_transform(id).unwrap();
        for _ in 0..120 {
            p.drive(
                id,
                VehicleControls {
                    parked: true,
                    ..Default::default()
                },
            );
            p.step(DT);
        }
        assert!(
            place(&p).transform_vector3(Vec3::Y).y > 0.98,
            "stands parked"
        );
        for _ in 0..240 {
            p.drive(
                id,
                VehicleControls {
                    throttle: 1.0,
                    ..Default::default()
                },
            );
            p.step(DT);
        }
        assert!(
            place(&p).transform_vector3(Vec3::Y).y > 0.98,
            "upright riding"
        );
        assert!(
            p.vehicle_speed(id) > 5.0,
            "rides at {}",
            p.vehicle_speed(id)
        );
        let before = place(&p).transform_vector3(Vec3::Z);
        for _ in 0..60 {
            p.drive(
                id,
                VehicleControls {
                    throttle: 1.0,
                    steer: 1.0,
                    ..Default::default()
                },
            );
            p.step(DT);
        }
        let m = place(&p);
        assert!(
            before.cross(m.transform_vector3(Vec3::Z)).y > 0.0,
            "turned left"
        );
        assert!(m.transform_vector3(Vec3::X).y < -0.1, "leans left");
        // Under 44.7 degrees over a second into the turn (43.9 with its box
        // sliding free as a car's hull, 41.5 held by the invented 0.6 it
        // had; 45.5 when its springs pushed at their contacts): its springs push at their
        // tops, as the 360's do ([`stand`]), so the turn pulls it less far
        // past the lean its balance asks (the island's bikes within 11
        // degrees of it at full lock, biketest; up to 28 past before).
        let up = m.transform_vector3(Vec3::Y).y;
        assert!(up > 0.71, "still up: {up}");
    }

    /// The Offroad Motorcycle (AL)'s `Bike*` fields, the weakest.
    const OFFROAD: BikeTuning = BikeTuning {
        roll_rate: 1.3,
        roll_angle: (0.7, 0.5),
        roll_input: 0.1,
        roll: 10.0,
        wheelie: 0.85,
        pitch_strength: (40.0, 50.0),
        pitch_high_speed: 1.0,
        wheelie_steer: 2.0,
        side_damp: 10.0,
        damp: [6.0, 10.0, 0.0],
        air: (2.0, 1.5),
        low_speed: 65.0 / 3.6,
        inertia: [0.5, 0.3, 0.5],
        centre_of_mass: [0.0; 3],
    };
    /// Its springs, its record's.
    const OFFROAD_SPRINGS: Springs = Springs {
        strength: (70.0, 70.0),
        damping: (10.0, 10.0),
        stretch: (0.1, 0.1),
        squeeze: (0.1, 0.1),
        extra: 0.05,
        gravity_factor: 1.7,
    };
    /// Its engine and gearbox, its record's.
    const OFFROAD_ENGINE: EngineTuning = EngineTuning {
        torque: 65.0,
        rpm: (1000.0, 5000.0, 8000.0),
        upshift: 7000.0,
        downshift: 4000.0,
        clutch_delay: 0.0,
        primary: 5.0,
        gears: [2.0, 1.35, 0.9, 0.6, 0.4],
        reverse: 1.0,
        radius: 0.32,
        torque_at: (1.0, 1.0),
        reverse_speed: 50.0 / 3.6,
        slip: 0.0,
        split: 0.5,
    };

    #[test]
    fn the_weakest_bike_template_stands_back_up_after_a_shove() {
        let mut p = Physics::new();
        let ground = CollisionShape::cuboid(Vec3::new(2000.0, 1.0, 2000.0));
        p.add_static(
            StaticKey::Cell(1),
            &ground,
            Vec3::new(0.0, -1.0, 0.0),
            Quat::IDENTITY,
        );
        let wheel = |z: f32| crate::vehicle::WheelSpec {
            centre: Vec3::new(0.0, 0.3, z),
            radius: 0.3,
            width: 0.15,
            steers: z > 0.0,
        };
        let hull = (Vec3::new(-0.45, 0.0, -1.1), Vec3::new(0.45, 1.3, 1.1));
        let mut bike = VehicleSpec {
            wheels: vec![wheel(0.7), wheel(-0.75)],
            bounds: hull,
            hull,
            mass: 300.0,
            ..car()
        };
        bike.tuning.bike = Some(OFFROAD);
        bike.tuning.springs = Some(OFFROAD_SPRINGS);
        bike.tuning.engine = Some(OFFROAD_ENGINE);
        // At its record's top speed, 110 km/h, with its record's 1.7 x
        // weight (the 360 balance, 0x8219D800, holds it up there).
        bike.tuning.max_speed = Some(110.0 / 3.6);
        let id = p.add_vehicle(&bike, Vec3::new(0.0, 0.2, 0.0), 0.0);
        let up = |p: &Physics| p.vehicle_transform(id).unwrap().transform_vector3(Vec3::Y);
        let ahead = VehicleControls {
            throttle: 1.0,
            ..Default::default()
        };
        for _ in 0..240 {
            p.drive(id, ahead);
            p.step(DT);
        }
        assert!(up(&p).y > 0.99, "upright riding");
        let m = p.vehicle_transform(id).unwrap();
        let at = m.transform_point3(Vec3::Y);
        p.push_vehicle(id, m.transform_vector3(Vec3::X) * 300.0 * 1.5, at);
        let mut most = 1.0f32;
        // The 360 balance stands it back up (0x8219D800).
        for _ in 0..600 {
            p.drive(id, ahead);
            p.step(DT);
            most = most.min(up(&p).y);
        }
        assert!(most < 0.995, "the shove rolls it ({most})");
        assert!(most > 0.5, "without tipping it over ({most})");
        let end = up(&p).y;
        assert!(end > 0.9986, "back up within 3 degrees in 10 s ({end})");
    }

    /// A body for [`hold_bike`] alone: its spin and velocity, turned by
    /// angular impulses per kilogram over its inertia per kilogram
    /// (upright and heading +z, so its own axes are the world's).
    struct Spun {
        w: Vec3,
        v: Vec3,
        inertia: Vec3,
    }

    fn hold(bike: &mut Bike, body: &mut Spun, f: BikeFooting, turn: Quat, dt: f32) {
        let cell = std::cell::RefCell::new(body);
        hold_bike(
            bike,
            f,
            turn,
            || {
                let b = cell.borrow();
                (b.w, b.v)
            },
            |spin, slide| {
                let mut b = cell.borrow_mut();
                let i = b.inertia;
                b.w += spin / i;
                b.v += slide;
            },
            dt,
        );
    }

    fn footing(steer: f32) -> BikeFooting {
        BikeFooting {
            wheels: 2,
            touching: 2,
            normal: Vec3::Y,
            top: 110.0 / 3.6,
            steer,
            stick: 0.0,
            throttle: 1.0,
            brake: 0.0,
        }
    }

    #[test]
    fn a_bike_leans_as_far_as_its_turn_asks_and_no_further() {
        let mut bike = Bike {
            tuning: OFFROAD,
            steer: 0.0,
        };
        let inertia = Vec3::from(OFFROAD.inertia);
        // At 10 m/s ahead turning +y at its most-lean rate: it asks its
        // most lean, (1 - s) hi + s lo, negative (its sideways axis
        // dips). Upright, it rolls that way, about -z.
        let fast = 10.0 / (110.0 / 3.6);
        let most = (1.0 - fast) * 0.5 + fast * 0.7;
        let mut body = Spun {
            w: Vec3::Y * 1.3,
            v: Vec3::Z * 10.0,
            inertia,
        };
        hold(&mut bike, &mut body, footing(0.0), Quat::IDENTITY, DT);
        let k = OFFROAD.roll / inertia.z * DT;
        assert!((body.w.z - (-most * k)).abs() < 1e-4, "{}", body.w);
        // Leaned exactly that far, turning at that rate about its own up
        // axis, it holds there (its steering asks less).
        let leaned = Quat::from_rotation_z(-most);
        let mut body = Spun {
            w: leaned * Vec3::Y * 1.3,
            v: Vec3::Z * 10.0,
            inertia,
        };
        hold(&mut bike, &mut body, footing(-0.1), leaned, DT);
        assert!(body.w.z.abs() < 1e-4, "{}", body.w);
        // With a wheel off the ground it stands up instead.
        let mut body = Spun {
            w: Vec3::Y * 1.3,
            v: Vec3::Z * 10.0,
            inertia,
        };
        let one = BikeFooting {
            touching: 1,
            ..footing(0.0)
        };
        hold(&mut bike, &mut body, one, leaned, DT);
        assert!(body.w.z > 0.0, "{}", body.w);
    }

    #[test]
    fn its_steering_leans_it_once_it_is_moving() {
        let inertia = Vec3::from(OFFROAD.inertia);
        let roll = |speed: f32| {
            let mut bike = Bike {
                tuning: OFFROAD,
                steer: 0.0,
            };
            let mut body = Spun {
                w: Vec3::ZERO,
                v: Vec3::Z * speed,
                inertia,
            };
            // Full lock to the 360's right: lean `roll_input` positive.
            hold(&mut bike, &mut body, footing(1.0), Quat::IDENTITY, DT);
            body.w.z
        };
        assert_eq!(roll(0.0), 0.0);
        let k = OFFROAD.roll / inertia.z * DT;
        let half = OFFROAD.low_speed * 0.5;
        assert!((roll(half) - 0.5 * OFFROAD.roll_input * k).abs() < 1e-5);
        assert!((roll(30.0) - OFFROAD.roll_input * k).abs() < 1e-5);
    }

    #[test]
    fn on_the_ground_its_nose_follows_the_slope_and_in_the_air_its_steering_turns_it() {
        let mut bike = Bike {
            tuning: OFFROAD,
            steer: 0.0,
        };
        let inertia = Vec3::from(OFFROAD.inertia);
        // Nose up 0.1 rad on the flat, riding forwards: pushed nose down
        // (+x) by its down strength.
        let mut body = Spun {
            w: Vec3::ZERO,
            v: Vec3::Z * 5.0,
            inertia,
        };
        let up = Quat::from_rotation_x(-0.1);
        hold(&mut bike, &mut body, footing(0.0), up, DT);
        let want = OFFROAD.pitch_strength.1 * 0.1 / inertia.x * DT;
        assert!((body.w.x - want).abs() < 1e-4, "{}", body.w);
        // In the air full lock to the 360's right eases in (all at once
        // growing) and turns it -y by its air yaw.
        let mut body = Spun {
            w: Vec3::ZERO,
            v: Vec3::ZERO,
            inertia,
        };
        let air = BikeFooting {
            touching: 0,
            ..footing(1.0)
        };
        hold(&mut bike, &mut body, air, Quat::IDENTITY, DT);
        assert!((bike.steer - 0.1).abs() < 1e-5, "eased {}", bike.steer);
        let want = -OFFROAD.air.1 * bike.steer / inertia.y * DT;
        assert!((body.w.y - want).abs() < 1e-5, "{}", body.w);
    }

    #[test]
    fn the_360_ease_keeps_its_share_each_sixtieth() {
        assert_eq!(ease_towards(0.0, 1.0, 0.0, DT), 1.0);
        assert!((ease_towards(0.0, 1.0, 0.9, DT) - 0.1).abs() < 1e-6);
        // Two sixtieths: 1 - 0.9².
        assert!((ease_towards(0.0, 1.0, 0.9, 2.0 * DT) - 0.19).abs() < 1e-5);
        // Half a sixtieth: halfway to one step.
        assert!((ease_towards(0.0, 1.0, 0.9, 0.5 * DT) - 0.05).abs() < 1e-6);
    }

    #[test]
    fn the_stiffest_bike_settles_without_ringing() {
        // The old street `Motorcycle`'s 250 and 50 over its 1.5 m² roll
        // inertia, explicitly stepped as the 360 code steps it.
        let tuning = BikeTuning {
            roll: 250.0,
            damp: [50.0, 10.0, 1.0],
            inertia: [1.5, 0.7, 1.5],
            ..OFFROAD
        };
        let mut bike = Bike { tuning, steer: 0.0 };
        let mut lean = 0.3f32;
        let mut body = Spun {
            w: Vec3::ZERO,
            v: Vec3::ZERO,
            inertia: Vec3::from(tuning.inertia),
        };
        for _ in 0..240 {
            body.w = Vec3::Z * body.w.z;
            hold(
                &mut bike,
                &mut body,
                footing(0.0),
                Quat::from_rotation_z(lean),
                DT,
            );
            lean += body.w.z * DT;
        }
        assert!(lean.abs() < 0.01, "settled at {lean}");
    }

    /// The AH-1Z flown as the 360 code flies it (0x821A38F0): the climb
    /// held spins its rotor up for 4 s before it lifts, then it climbs at
    /// its `MaxElevationSpeed`; flat out it reaches its `MaxFwdSpeed`
    /// with its nose down; it turns and banks the way it is steered; let
    /// go in the air it hovers, and wrecked it falls.
    #[test]
    fn a_helicopter_spins_up_climbs_flies_turns_and_falls_wrecked() {
        let mut p = Physics::new();
        let ground = CollisionShape::cuboid(Vec3::new(2000.0, 1.0, 2000.0));
        p.add_static(
            StaticKey::Cell(1),
            &ground,
            Vec3::new(0.0, -1.0, 0.0),
            Quat::IDENTITY,
        );
        let id = p.add_vehicle(&helicopter(), Vec3::new(0.0, 0.1, 0.0), 0.0);
        assert!(p.flies(id));
        let place = |p: &Physics| p.vehicle_transform(id).unwrap();
        let parked = VehicleControls {
            parked: true,
            ..Default::default()
        };
        // Parked, it sits on the ground.
        for _ in 0..120 {
            p.drive(id, parked);
            p.step(DT);
        }
        let floor = place(&p).w_axis.y;
        assert!(floor.abs() < 0.3, "sits at {floor}");
        let flown = |throttle: f32, steer: f32| VehicleControls {
            throttle,
            steer,
            ..Default::default()
        };
        // The climb held: for most of 4 s its rotor spins up and it stays
        // put, then it lifts.
        p.fly(id, 1.0);
        for _ in 0..210 {
            p.drive(id, flown(0.0, 0.0));
            p.step(DT);
        }
        let y = place(&p).w_axis.y;
        assert!((y - floor).abs() < 0.3, "spinning up at {y}");
        for _ in 0..360 {
            p.drive(id, flown(0.0, 0.0));
            p.step(DT);
        }
        let climb = p.vehicle_velocity(id).y;
        assert!((climb - 25.0).abs() < 1.0, "climbs at {climb}");
        assert!(place(&p).transform_vector3(Vec3::Y).y > 0.99, "level");
        // Let go of the climb: it eases to a hover.
        p.fly(id, 0.0);
        for _ in 0..600 {
            p.drive(id, flown(0.0, 0.0));
            p.step(DT);
        }
        let up = place(&p).w_axis.y;
        // (Swaying in its turbulence, under a metre a second.)
        let v = p.vehicle_velocity(id);
        assert!(
            v.y.abs() < 0.5 && v.with_y(0.0).length() < 1.0,
            "hovers at {v}"
        );
        // Holding its height, flat out ahead: its top speed, nose down
        // by its `MaxPitch` (0.66 for its speed, 0.33 for the stick).
        for _ in 0..1500 {
            p.drive(id, flown(1.0, 0.0));
            p.step(DT);
        }
        let m = place(&p);
        let v = p.vehicle_velocity(id);
        assert!((v.with_y(0.0).length() - 30.0).abs() < 0.5, "flies at {v}");
        assert!(
            (m.w_axis.y - up).abs() < 3.0,
            "held its height: {}",
            m.w_axis.y
        );
        let nose = -m.transform_vector3(Vec3::Z).y.asin();
        assert!((nose - 0.99 * 0.4).abs() < 0.03, "nose down {nose}");
        // Steering left (positive, as the wheels) turns it left
        // (anticlockwise from above) and banks it left side down.
        let before = m.transform_vector3(Vec3::Z);
        for _ in 0..60 {
            p.drive(id, flown(1.0, 1.0));
            p.step(DT);
        }
        let m = place(&p);
        let after = m.transform_vector3(Vec3::Z);
        assert!(before.cross(after).y > 0.0, "turned left");
        assert!(m.transform_vector3(Vec3::X).y < -0.05, "banked");
        // Nobody flying it in the air: its stick let go, it hovers (within
        // 20 s; its spin damped 0.8, the 360 body builder's least, it
        // takes longer to level than with the invented 0.5).
        for _ in 0..1200 {
            p.drive(id, parked);
            p.step(DT);
        }
        assert!(
            p.vehicle_velocity(id).length() < 1.0,
            "hovers unflown {}",
            p.vehicle_velocity(id)
        );
        // Wrecked, its rotor winds down and it falls.
        p.wreck_vehicle(id);
        for _ in 0..90 {
            p.step(DT);
        }
        assert!(p.vehicle_velocity(id).y < -5.0, "falls");
    }

    /// A car's body as the 360 code sets it (0x8219BA08, 0x8219BABC): its
    /// template's inertia per kilogram times its mass, its centre of mass
    /// the middle of its wheels' spring tops plus its template's offset.
    #[test]
    fn a_cars_inertia_and_centre_of_mass_are_its_templates() {
        let mut spec = car();
        spec.tuning.body = Some(crate::tuning::CarBody {
            inertia: [2.0, 1.0, 1.5],
            centre_of_mass: [0.0, -0.2, 0.1],
        });
        let mut p = Physics::new();
        let id = p.add_vehicle(&spec, Vec3::new(0.0, 1.0, 0.0), 0.0);
        let v = p.vehicles[id.0].as_ref().unwrap();
        let b = &p.world.bodies[v.controller.chassis];
        let props = b.mass_properties().local_mprops;
        let i = vec3(props.principal_inertia());
        let want = Vec3::new(2.0, 1.0, 1.5) * 1400.0;
        assert!((i - want).length() < 1.0, "inertia {i}");
        // The wheels' middle (0, 0.35, 0.05) raised by the default
        // squeeze, plus the offset.
        let top = Springs::DEFAULT.axle(true).3;
        let com = vec3(props.local_com);
        let want = Vec3::new(0.0, 0.35 + top - 0.2, 0.05 + 0.1);
        assert!((com - want).length() < 1e-3, "centre {com} want {want}");
    }

    /// 0x82167500: an axis under a fifteenth of the first other it is
    /// weighed against is raised to that, and every inertia comes out
    /// 16/15 of what was worked out.
    #[test]
    fn an_inertia_is_clipped_to_a_fifteenth_and_raised_by_a_fifteenth() {
        let i = clip_inertia(Mat3::from_diagonal(Vec3::new(2.0, 1.0, 1.5)));
        let want = Vec3::new(2.0, 1.0, 1.5) * 16.0 / 15.0;
        assert!(
            (Vec3::new(i.x_axis.x, i.y_axis.y, i.z_axis.z) - want).length() < 1e-5,
            "{i}"
        );
        // A needle: its long axis's inertia is raised to a fifteenth of
        // the x axis's.
        let i = clip_inertia(Mat3::from_diagonal(Vec3::new(10.0, 10.0, 0.1)));
        let want = Vec3::new(10.0, 10.0, 10.0 / 15.0) * 16.0 / 15.0;
        assert!(
            (Vec3::new(i.x_axis.x, i.y_axis.y, i.z_axis.z) - want).length() < 1e-4,
            "{i}"
        );
        // x is weighed against y alone when y already makes it too small,
        // even though z is larger.
        let i = clip_inertia(Mat3::from_diagonal(Vec3::new(0.01, 1.0, 30.0)));
        assert!((i.x_axis.x - 1.0 / 15.0 * 16.0 / 15.0).abs() < 1e-5, "{i}");
    }

    /// A helicopter's (or a tank's, or a boat's) chassis keeps its
    /// record's inertia per kilogram at its template's mass (0x82167700),
    /// and its record's centre of mass.
    #[test]
    fn a_helicopters_inertia_and_centre_of_mass_are_its_chassis_bodys() {
        let mut spec = helicopter();
        spec.tuning.mass = Some(8300.0);
        spec.chassis = Some(crate::vehicle::Chassis {
            mass: 83_000.0,
            inertia: Mat3::from_diagonal(Vec3::new(400_000.0, 500_000.0, 200_000.0)),
            centre_of_mass: Vec3::new(0.1, 1.2, 0.5),
        });
        let mut p = Physics::new();
        let id = p.add_vehicle(&spec, Vec3::new(0.0, 1.0, 0.0), 0.0);
        let v = p.vehicles[id.0].as_ref().unwrap();
        let b = &p.world.bodies[v.controller.chassis];
        let props = b.mass_properties().local_mprops;
        assert!((props.mass() - 8300.0).abs() < 1e-2);
        let mut i = vec3(props.principal_inertia()).to_array();
        i.sort_by(f32::total_cmp);
        let want = [20_000.0, 40_000.0, 50_000.0].map(|x| x * 16.0 / 15.0);
        for (got, want) in i.iter().zip(want) {
            assert!((got - want).abs() < 1.0, "inertia {i:?}");
        }
        // Model space, as the body's frame is.
        let com = vec3(props.local_com);
        assert!((com - Vec3::new(0.1, 1.2, 0.5)).length() < 1e-3, "{com}");
    }

    /// The 360 code's per-60th decay (0x821A3B38): a whole number of
    /// 60ths keep `keep` each, a part of one blends.
    #[test]
    fn a_rotor_winds_down_per_sixtieth() {
        let keep = keep_per_frame(1.0, 0.99, 1.0);
        assert!((keep - 0.99f32.powi(60)).abs() < 1e-3, "{keep}");
        let half = keep_per_frame(1.0, 0.9, 0.5 / 60.0);
        assert!((half - 0.95).abs() < 1e-5, "{half}");
    }

    /// The rotor (0x821A2150): the climb spins it up a quarter a second;
    /// in the air it keeps its speed; on the ground half a second, let go,
    /// it winds down.
    #[test]
    fn a_rotor_spins_up_and_winds_down_on_the_ground() {
        let up = Stick {
            lift: 1.0,
            ..Stick::default()
        };
        let mut r = Rotor::default();
        r.spin(up, false, true, 2.0);
        assert_eq!(r.spin, 0.5);
        r.spin(up, false, false, 3.0);
        assert_eq!(r.spin, 1.0);
        r.spin(Stick::default(), false, false, 1.0);
        assert_eq!(r.spin, 1.0, "kept in the air");
        r.spin(Stick::default(), false, true, 1.0);
        assert_eq!(r.spin, 1.0, "kept its first half second down");
        r.spin(Stick::default(), false, true, 1.0);
        assert!((r.spin - 0.99f32.powi(60)).abs() < 1e-3, "{}", r.spin);
    }

    /// The Speed Boat's `_BoatPhysics` (gap round 17): flat out it
    /// settles at its `MaxSpeed`, backing up at its `MaxSpeedReverse`,
    /// and coasting and braking the water slows it at its drag's rate.
    #[test]
    fn a_boat_goes_and_slows_as_its_template_says() {
        let spec = VehicleSpec {
            tuning: crate::VehicleTuning {
                max_speed: Some(120.0 / 3.6),
                boat: Some(BoatTuning {
                    reverse_speed: 40.0 / 3.6,
                    torque: (1000.0, 500.0),
                    drag: [0.2, 2.0, 2.0],
                    brake: 3.0,
                    curve: [1.0, 0.7, 0.6, 0.5],
                    hull_friction: 0.3,
                    float: BoatFloat {
                        top: 120.0 / 3.6,
                        reverse: 40.0 / 3.6,
                        ..BoatFloat::DEFAULT
                    },
                }),
                ..crate::VehicleTuning::default()
            },
            mass: 8000.0,
            ..boat()
        };
        let run = |throttle: f32, brake: bool, secs: f32, from: Option<f32>| {
            let mut p = Physics::new();
            p.set_water(0.0);
            let id = p.add_vehicle(&spec, Vec3::new(0.0, 0.5, 0.0), 0.0);
            for _ in 0..300 {
                p.drive(id, VehicleControls::default());
                p.step(DT);
            }
            if let Some(v) = from {
                let up = p.vehicle_velocity(id).y;
                p.set_vehicle_velocity(id, Vec3::new(0.0, up, v));
            }
            for _ in 0..(secs / DT) as usize {
                let c = VehicleControls {
                    throttle,
                    brake,
                    ..VehicleControls::default()
                };
                p.drive(id, c);
                p.step(DT);
            }
            p.vehicle_velocity(id).z
        };
        let ahead = run(1.0, false, 40.0, None);
        assert!((ahead - 33.33).abs() < 0.7, "ahead {ahead}");
        let astern = run(-1.0, false, 30.0, None);
        assert!((astern + 11.11).abs() < 0.3, "astern {astern}");
        // From 8 m/s for a second: coasting, e^-0.2 of it; its handbrake
        // held, the same (it only cuts its engine); the throttle back, its
        // brake's steady 3 m/s² on top of the drag, (8 + 15) e^-0.2 - 15,
        // and its engine starting to pull it back (0x82199B08).
        let coast = run(0.0, false, 1.0, Some(8.0));
        assert!((coast - 8.0 * (-0.2f32).exp()).abs() < 0.2, "coast {coast}");
        let held = run(0.0, true, 1.0, Some(8.0));
        assert!((held - coast).abs() < 0.05, "handbrake {held}");
        let braked = run(-1.0, false, 1.0, Some(8.0));
        let steady = 23.0 * (-0.2f32).exp() - 15.0;
        assert!(braked < steady + 0.1 && braked > -6.0, "braked {braked}");
    }

    #[test]
    fn a_boat_floats_level_drives_turns_and_beaches() {
        let mut p = Physics::new();
        let water = -36.0;
        p.set_water(water);
        // The seabed, and a beach rising out of the sea 120 m ahead.
        let bed = CollisionShape::mesh(
            &[
                Vec3::new(-300.0, -60.0, -300.0),
                Vec3::new(300.0, -60.0, -300.0),
                Vec3::new(300.0, -60.0, 100.0),
                Vec3::new(-300.0, -60.0, 100.0),
                Vec3::new(300.0, -26.0, 300.0),
                Vec3::new(-300.0, -26.0, 300.0),
            ],
            &[[0, 2, 1], [0, 3, 2], [3, 4, 2], [3, 5, 4]],
        )
        .unwrap();
        p.add_static(StaticKey::Cell(1), &bed, Vec3::ZERO, Quat::IDENTITY);
        p.step(DT);
        // A boat that leans 0.2 rad into its turns and lifts its bow 0.06
        // under throttle (the Gar's, its springs the 360 record's defaults).
        let mut spec = boat();
        spec.tuning.boat = Some(BoatTuning {
            float: BoatFloat {
                roll: 0.2,
                pitch: 0.06,
                ..BoatFloat::DEFAULT
            },
            ..BOAT
        });
        let id = p.add_vehicle(&spec, Vec3::new(0.0, water + 1.5, -200.0), 0.0);
        let place = |p: &Physics| p.vehicle_transform(id).unwrap();
        for _ in 0..600 {
            p.drive(id, VehicleControls::default());
            p.step(DT);
        }
        // Afloat, level, its origin as deep as its springs hold it: their
        // bottoms at its origin, 9.81 / (4 x 5) m under the water.
        let m = place(&p);
        let y = m.w_axis.y;
        assert!((float_depth(&spec) - 0.49).abs() < 0.01);
        assert!(
            (y - (water - float_depth(&spec))).abs() < 0.05,
            "floats at {y}"
        );
        assert!(m.transform_vector3(Vec3::Y).y > 0.99, "level");
        // Flat out ahead.
        for _ in 0..360 {
            p.drive(
                id,
                VehicleControls {
                    throttle: 1.0,
                    ..Default::default()
                },
            );
            p.step(DT);
        }
        // (On the 360 record's defaults: 100 x 200 N on 2 t, fading to
        // 0.15 of it at its 40 km/h.)
        let speed = p.vehicle_speed(id);
        assert!((5.0..=40.0 / 3.6 + 0.3).contains(&speed), "{speed} m/s");
        let m = place(&p);
        assert!(
            m.w_axis.z > -170.0 && m.w_axis.x.abs() < 1.0,
            "at {}",
            m.w_axis
        );
        // Under throttle its bow springs feel the water up to 6% of its
        // length higher: bow up.
        let bow = m.transform_vector3(Vec3::Z).y;
        assert!((0.005..0.1).contains(&bow), "bow up {bow}");
        // Steering turns it as steering a tank does: positive steer, a
        // positive turn about up.
        // (Its helm as the 360 code's turns it, 0x8219A390: 10 x its
        // steering 2 x helm against its yaw damping 0.5, the record's
        // defaults.)
        let mut turned = 0.0;
        let mut last = m.transform_vector3(Vec3::Z);
        for _ in 0..120 {
            p.drive(
                id,
                VehicleControls {
                    throttle: 1.0,
                    steer: 1.0,
                    ..Default::default()
                },
            );
            p.step(DT);
            let now = place(&p).transform_vector3(Vec3::Z);
            turned += last.with_y(0.0).cross(now.with_y(0.0)).y.asin();
            last = now;
        }
        assert!(turned > 0.8, "turned {turned}");
        // Leaning into the turn: its port side (+x) down.
        let port = place(&p).transform_vector3(Vec3::X).y;
        assert!(port < -0.02, "leans {port}");
        // Straight on at the beach: it runs aground and stops there,
        // still upright.
        for _ in 0..120 {
            p.drive(
                id,
                VehicleControls {
                    throttle: 1.0,
                    steer: -1.0,
                    ..Default::default()
                },
            );
            p.step(DT);
        }
        for _ in 0..60 * 60 {
            let m = place(&p);
            let f = m.transform_vector3(Vec3::Z);
            // Heading for +z.
            let steer = (-f.x.atan2(f.z) * 2.0).clamp(-1.0, 1.0);
            p.drive(
                id,
                VehicleControls {
                    throttle: 1.0,
                    steer,
                    ..Default::default()
                },
            );
            p.step(DT);
        }
        // The beach comes out of the water at z 241; it runs up it at its
        // top speed (11 m/s on the 360 record's defaults) and slides on
        // (further since its hull's 360 friction, 0.3, took the invented
        // 0.6's place).
        let m = place(&p);
        assert!(
            p.vehicle_speed(id).abs() < 2.0,
            "{} at {}",
            p.vehicle_speed(id),
            m.w_axis
        );
        assert!(
            (215.0..275.0).contains(&m.w_axis.z),
            "aground at {}",
            m.w_axis
        );
        assert!(m.transform_vector3(Vec3::Y).y > 0.8, "upright");
        // A wreck sinks.
        p.wreck_vehicle(id);
        let id2 = p.add_vehicle(&boat(), Vec3::new(50.0, water, -200.0), 0.0);
        p.wreck_vehicle(id2);
        for _ in 0..60 * 20 {
            p.step(DT);
        }
        let y = p.vehicle_transform(id2).unwrap().w_axis.y;
        assert!(y < water - 5.0, "wreck at {y}");
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
                    ..Default::default()
                },
            );
            p.step(DT);
        }
        let up = p.vehicle_transform(id).unwrap().transform_vector3(Vec3::Y);
        assert!(up.y > 0.7, "rolled: {up}");
    }

    // Up a 20° hill from a standstill, a car gets going about as it does
    // on the flat (a hill road in the playthrough is taken at speed).
    #[test]
    fn cars_climb_a_steep_hill_at_speed() {
        let mut p = Physics::new();
        let half = 200.0;
        let v = [
            Vec3::new(-half, 0.0, -half),
            Vec3::new(half, 0.0, -half),
            Vec3::new(half, 0.0, half),
            Vec3::new(-half, 0.0, half),
        ];
        let shape = CollisionShape::mesh(&v, &[[0, 1, 2], [0, 2, 3]]).unwrap();
        // Rising to +z.
        let slope = 20f32.to_radians();
        p.add_static(
            StaticKey::Cell(1),
            &shape,
            Vec3::ZERO,
            Quat::from_rotation_x(-slope),
        );
        p.step(DT);
        // An M151's brakes: by the defaults (2 an axle) one rolled back at
        // the start can't stop on a 20 degree hill at its gravity factor.
        let mut spec = car();
        spec.tuning.tyres = Some(Tyres {
            brake: (7.0, 6.0),
            ..Tyres::DEFAULT
        });
        let ground = |at: Vec3| {
            p.cast_ray(Vec3::new(at.x, 80.0, at.z), Vec3::NEG_Y, 200.0)
                .map(|(d, _)| 80.0 - d)
        };
        let rest = spec.rest_on(Vec3::ZERO, 0.0, ground).unwrap();
        let id = p.add_vehicle_turned(&spec, rest.position + Vec3::Y * 0.2, rest.rotation);
        for _ in 0..360 {
            p.drive(
                id,
                VehicleControls {
                    throttle: 1.0,
                    ..Default::default()
                },
            );
            p.step(DT);
        }
        // Under its steepest less 20 degrees its springs push it straight
        // up, so the hill holds it back no more than the flat.
        let speed = p.vehicle_speed(id);
        assert!(speed > 12.0, "{speed} m/s");
    }

    fn tank() -> VehicleSpec {
        use crate::vehicle::WheelSpec;
        let wheel = |x: f32, z: f32| WheelSpec {
            centre: Vec3::new(x, 0.45, z),
            radius: 0.45,
            width: 0.5,
            steers: z > 0.0,
        };
        let hull = (Vec3::new(-1.6, 0.0, -3.3), Vec3::new(1.6, 1.6, 3.3));
        VehicleSpec {
            kind: "tank".into(),
            wheels: vec![
                wheel(-1.2, 2.3),
                wheel(1.2, 2.3),
                wheel(-1.2, -2.3),
                wheel(1.2, -2.3),
            ],
            bounds: (hull.0, Vec3::new(1.6, 2.4, 3.3)),
            hull,
            tracked: true,
            mass: 36_000.0,
            tuning: crate::VehicleTuning {
                max_speed: Some(12.5),
                ..Default::default()
            },
            ..car()
        }
    }

    /// Up a hill of `degrees` from a standstill, `throttle` for `seconds`:
    /// how far it got up the hill, metres.
    fn tank_up(degrees: f32, throttle: f32, seconds: f32) -> f32 {
        let mut p = Physics::new();
        let half = 300.0;
        let v = [
            Vec3::new(-half, 0.0, -half),
            Vec3::new(half, 0.0, -half),
            Vec3::new(half, 0.0, half),
            Vec3::new(-half, 0.0, half),
        ];
        let shape = CollisionShape::mesh(&v, &[[0, 1, 2], [0, 2, 3]]).unwrap();
        p.add_static(
            StaticKey::Cell(1),
            &shape,
            Vec3::ZERO,
            Quat::from_rotation_x(-degrees.to_radians()),
        );
        p.step(DT);
        let spec = tank();
        let ground = |at: Vec3| {
            p.cast_ray(Vec3::new(at.x, 150.0, at.z), Vec3::NEG_Y, 400.0)
                .map(|(d, _)| 150.0 - d)
        };
        let rest = spec.rest_on(Vec3::ZERO, 0.0, ground).unwrap();
        let id = p.add_vehicle_turned(&spec, rest.position + Vec3::Y * 0.2, rest.rotation);
        for _ in 0..180 {
            p.drive(id, VehicleControls::default());
            p.step(DT);
        }
        let start = p.vehicle_transform(id).unwrap().w_axis.y;
        for _ in 0..(seconds / DT) as usize {
            p.drive(
                id,
                VehicleControls {
                    throttle,
                    ..Default::default()
                },
            );
            p.step(DT);
        }
        p.vehicle_transform(id).unwrap().w_axis.y - start
    }

    #[test]
    fn tanks_drive_over_a_hilltop() {
        let mut p = Physics::new();
        let (h, rise) = (50.0, 100.0 * 35f32.to_radians().tan());
        // Up 35 degrees to z 0, then level.
        let v = [
            Vec3::new(-h, -rise, -100.0),
            Vec3::new(h, -rise, -100.0),
            Vec3::new(h, 0.0, 0.0),
            Vec3::new(-h, 0.0, 0.0),
            Vec3::new(-h, 0.0, 100.0),
            Vec3::new(h, 0.0, 100.0),
        ];
        let tris = [[0, 2, 1], [0, 3, 2], [3, 5, 2], [3, 4, 5]];
        let shape = CollisionShape::mesh(&v, &tris).unwrap();
        p.add_static(StaticKey::Cell(1), &shape, Vec3::ZERO, Quat::IDENTITY);
        p.step(DT);
        let spec = tank();
        let ground = |at: Vec3| {
            p.cast_ray(Vec3::new(at.x, 50.0, at.z), Vec3::NEG_Y, 400.0)
                .map(|(d, _)| 50.0 - d)
        };
        let rest = spec
            .rest_on(Vec3::new(0.0, 0.0, -20.0), 0.0, ground)
            .unwrap();
        let id = p.add_vehicle_turned(&spec, rest.position + Vec3::Y * 0.2, rest.rotation);
        for _ in 0..(12.0 / DT) as usize {
            p.drive(
                id,
                VehicleControls {
                    throttle: 1.0,
                    ..Default::default()
                },
            );
            p.step(DT);
        }
        let at = p.vehicle_transform(id).unwrap().w_axis;
        assert!(at.z > 20.0, "stopped at {at}");
    }

    #[test]
    fn tanks_climb_steep_hills_and_stand_on_them() {
        // Up 45 degrees at about 5 m/s, after the first seconds.
        let up = tank_up(45.0, 1.0, 6.0);
        assert!(up > 15.0, "{up} m up");
        let up = tank_up(30.0, 1.0, 6.0);
        assert!(up > 12.0, "{up} m up");
        let stays = tank_up(40.0, 0.0, 3.0);
        assert!(stays.abs() < 0.3, "moved {stays} m");
    }

    #[test]
    fn parked_on_a_slope_it_rests_on_its_wheels_and_stays() {
        let mut p = Physics::new();
        // A 15° slope rising to +x; the car parked facing up it.
        let half = 100.0;
        let v = [
            Vec3::new(-half, 0.0, -half),
            Vec3::new(half, 0.0, -half),
            Vec3::new(half, 0.0, half),
            Vec3::new(-half, 0.0, half),
        ];
        let shape = CollisionShape::mesh(&v, &[[0, 1, 2], [0, 2, 3]]).unwrap();
        let slope = 15f32.to_radians();
        p.add_static(
            StaticKey::Cell(1),
            &shape,
            Vec3::ZERO,
            Quat::from_rotation_z(slope),
        );
        p.step(DT);
        let spec = car();
        let yaw = std::f32::consts::FRAC_PI_2;
        let ground = |at: Vec3| {
            p.cast_ray(Vec3::new(at.x, 50.0, at.z), Vec3::NEG_Y, 100.0)
                .map(|(d, _)| 50.0 - d)
        };
        let rest = spec.rest_on(Vec3::new(5.0, 0.0, 0.0), yaw, ground).unwrap();
        assert!((rest.slope - slope).abs() < 0.01, "{rest:?}");
        let id = p.add_vehicle_turned(&spec, rest.position + Vec3::Y * 0.3, rest.rotation);
        let parked = VehicleControls {
            parked: true,
            ..Default::default()
        };
        for _ in 0..240 {
            p.drive(id, parked);
            p.step(DT);
        }
        let at = p.vehicle_transform(id).unwrap();
        let moved = (at.transform_point3(Vec3::ZERO) - rest.position).length();
        assert!(moved < 0.6, "slid {moved} m");
        let up = at.transform_vector3(Vec3::Y);
        assert!(
            (up.angle_between(Vec3::Y) - slope).abs() < 0.05,
            "tilted {} degrees",
            up.angle_between(Vec3::Y).to_degrees()
        );
    }

    // Let roll on a hill, a car stays put on one gentler than its steepest
    // less 20 degrees (its springs push it straight up there, as the 360
    // code's do) and rolls down one steeper than its steepest (45 by
    // default), on which its engine can't drive it up either.
    #[test]
    fn it_stands_on_gentle_hills_and_rolls_down_steep_ones() {
        let run = |degrees: f32, throttle: f32| {
            let mut p = Physics::new();
            let half = 100.0;
            let v = [
                Vec3::new(-half, 0.0, -half),
                Vec3::new(half, 0.0, -half),
                Vec3::new(half, 0.0, half),
                Vec3::new(-half, 0.0, half),
            ];
            let shape = CollisionShape::mesh(&v, &[[0, 1, 2], [0, 2, 3]]).unwrap();
            let slope = degrees.to_radians();
            p.add_static(
                StaticKey::Cell(1),
                &shape,
                Vec3::ZERO,
                Quat::from_rotation_z(slope),
            );
            p.step(DT);
            let spec = car();
            let yaw = std::f32::consts::FRAC_PI_2;
            let ground = |at: Vec3| {
                p.cast_ray(Vec3::new(at.x, 80.0, at.z), Vec3::NEG_Y, 200.0)
                    .map(|(d, _)| 80.0 - d)
            };
            let rest = spec.rest_on(Vec3::new(5.0, 0.0, 0.0), yaw, ground).unwrap();
            let id = p.add_vehicle_turned(&spec, rest.position + Vec3::Y * 0.3, rest.rotation);
            let go = VehicleControls {
                throttle,
                ..Default::default()
            };
            for _ in 0..180 {
                p.drive(id, go);
                p.step(DT);
            }
            let at = p
                .vehicle_transform(id)
                .unwrap()
                .transform_point3(Vec3::ZERO);
            at.x - rest.position.x
        };
        let gentle = run(20.0, 0.0);
        assert!(gentle.abs() < 0.3, "rolled {gentle} m on 20 degrees");
        let steep = run(50.0, 0.0);
        assert!(steep < -3.0, "rolled {steep} m on 50 degrees");
        let up = run(50.0, 1.0);
        assert!(up < -3.0, "drove {up} m up 50 degrees");
    }

    /// Playtest (Android, a controller): the gearbox goes into reverse
    /// only with the brake pedal all but floored (over 0.9), so a pedal
    /// that shares its push with the steering (0.71 at full lock, as the
    /// alpha's pad once gave it) never backs up. The whole pedal does,
    /// steering or not.
    #[test]
    fn a_floored_pedal_backs_up_at_full_lock() {
        let backed = |throttle: f32, steer: f32| {
            let mut p = Physics::new();
            floor(&mut p, 1, 0.0, 0.0, 0.0, 600.0);
            p.step(DT);
            let id = p.add_vehicle(&car(), Vec3::new(0.0, 0.1, 0.0), 0.0);
            for _ in 0..(3.0 / DT) as usize {
                p.drive(
                    id,
                    VehicleControls {
                        throttle,
                        steer,
                        ..Default::default()
                    },
                );
                p.step(DT);
            }
            (
                p.vehicle_gear(id).expect("an engine").0,
                p.vehicle_speed(id),
            )
        };
        let shared = backed(-std::f32::consts::FRAC_1_SQRT_2, 1.0);
        assert_eq!(shared.0, 1, "{shared:?}");
        assert!(shared.1.abs() < 0.5, "{shared:?}");
        let floored = backed(-1.0, 1.0);
        assert_eq!(floored.0, 0, "{floored:?}");
        assert!(floored.1 < -2.0, "{floored:?}");
    }

    // Round 18: with the M151's engine and gearbox it reaches its
    // template's 85 km/h in seconds and holds there (it used to level
    // off at 60% of it), and backs up no faster than its 55 km/h.
    #[test]
    fn an_engine_drives_a_car_to_its_max_speed() {
        let engine = crate::tuning::EngineTuning {
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
        };
        let spec = VehicleSpec {
            mass: 1000.0,
            tuning: crate::VehicleTuning {
                max_speed: Some(85.0 / 3.6),
                engine: Some(engine),
                ..Default::default()
            },
            ..car()
        };
        let run = |throttle: f32, seconds: f32| {
            let mut p = Physics::new();
            floor(&mut p, 1, 0.0, 0.0, 0.0, 600.0);
            p.step(DT);
            let id = p.add_vehicle(&spec, Vec3::new(0.0, 0.1, -500.0 * throttle), 0.0);
            let mut speeds = Vec::new();
            for _ in 0..(seconds / DT) as usize {
                p.drive(
                    id,
                    VehicleControls {
                        throttle,
                        ..Default::default()
                    },
                );
                p.step(DT);
                speeds.push(p.vehicle_speed(id).abs());
            }
            speeds
        };
        let ahead = run(1.0, 15.0);
        let top = 85.0 / 3.6;
        let reached = ahead.iter().position(|&s| s > 0.9 * top);
        assert!(
            reached.is_some_and(|i| (i as f32) * DT < 10.0),
            "{reached:?}"
        );
        let fastest = ahead.iter().copied().fold(0.0, f32::max);
        assert!(fastest < top * 1.03, "{fastest}");
        let back = run(-1.0, 12.0);
        let fastest = back.iter().copied().fold(0.0, f32::max);
        assert!(fastest > 0.8 * engine.reverse_speed, "{fastest}");
        assert!(fastest < engine.reverse_speed * 1.03, "{fastest}");
    }

    // With its template's clutch delay, its gearbox changes up one gear at
    // a time as it speeds up, no sooner than that long after the last
    // change, its engine driving nothing meanwhile (0x821A0528).
    #[test]
    fn a_change_of_gear_takes_its_clutch_delay() {
        let engine = crate::tuning::EngineTuning {
            torque: 130.0,
            rpm: (900.0, 3000.0, 12000.0),
            upshift: 4000.0,
            downshift: 2500.0,
            clutch_delay: 0.5,
            primary: 3.5,
            gears: [4.0, 3.1, 2.4, 2.0, 1.5],
            reverse: 3.7,
            radius: 0.39,
            torque_at: (0.8, 1.0),
            reverse_speed: 55.0 / 3.6,
            slip: 1000.0,
            split: 1.0,
        };
        let spec = VehicleSpec {
            mass: 1000.0,
            tuning: crate::VehicleTuning {
                max_speed: Some(85.0 / 3.6),
                engine: Some(engine),
                ..Default::default()
            },
            ..car()
        };
        let mut p = Physics::new();
        floor(&mut p, 1, 0.0, 0.0, 0.0, 600.0);
        p.step(DT);
        let id = p.add_vehicle(&spec, Vec3::new(0.0, 0.1, -500.0), 0.0);
        let mut steps = Vec::new();
        for _ in 0..(12.0 / DT) as usize {
            p.drive(
                id,
                VehicleControls {
                    throttle: 1.0,
                    ..Default::default()
                },
            );
            p.step(DT);
            steps.push((
                p.vehicle_gear(id).expect("an engine").0,
                p.vehicle_speed(id),
            ));
        }
        let first_change = steps.iter().position(|s| s.0 == 2).expect("changes up");
        assert_eq!(steps[first_change - 1].0, 1);
        // Over the delay that follows it stays in second, its engine
        // driving nothing, so it gains next to nothing.
        let after = steps[first_change + (0.45 / DT) as usize];
        assert_eq!(after.0, 2);
        assert!(after.1 - steps[first_change].1 < 0.1, "{after:?}");
        let changes: Vec<usize> = (1..steps.len())
            .filter(|&i| steps[i].0 != steps[i - 1].0)
            .collect();
        assert!(
            changes
                .windows(2)
                .all(|w| (w[1] - w[0]) as f32 * DT >= 0.49),
            "{changes:?}"
        );
        // One gear at a time, never back down while it speeds up.
        assert!(
            steps
                .windows(2)
                .all(|w| w[1].0 == w[0].0 || w[1].0 == w[0].0 + 1)
        );
        assert!(steps.last().expect("steps").0 >= 3);
    }

    #[test]
    fn a_tank_point_springs_by_its_squeeze_squared_and_damps_only_stretching() {
        let full = AMX30.spring_length();
        let push = |t: &mut Tracks, length: f32| {
            let at = Vec3::new(1.2, -1.0, 2.0);
            let points = [Some(TrackContact {
                at,
                normal: Vec3::Y,
                start: at + Vec3::Y * length,
            })];
            let f = TrackFooting {
                turn: Quat::IDENTITY,
                centre: Vec3::ZERO,
                mass: 32000.0,
                points: &points,
                throttle: 0.0,
                steer: 0.0,
            };
            let mut spring = Vec3::ZERO;
            drive_tracks(
                t,
                &f,
                || (Vec3::ZERO, Vec3::ZERO),
                |i, p| {
                    if p == Some(at + Vec3::Y * length) {
                        spring += i;
                    }
                },
                DT,
            );
            spring
        };
        // Squeezed 0.3 m (from all its length): mass x 20 x 0.3 x 0.3,
        // straight up while level; squeezing it is not damped.
        let mut t = Tracks::new(AMX30, 1);
        let up = push(&mut t, full - 0.3);
        let want = 32000.0 * 20.0 * 0.3 * 0.3 * DT;
        assert!((up.y - want).abs() < want * 1e-4, "{up}");
        assert!(up.x.abs() < 1e-6 && up.z.abs() < 1e-6);
        // Stretching back 0.01 m in a step: less damp 2.5 x 0.6 m/s.
        let up = push(&mut t, full - 0.29);
        let want = 32000.0 * (20.0 * 0.29 - 2.5 * 0.01 / DT) * 0.29 * DT;
        assert!((up.y - want).abs() < want * 1e-3, "{up}");
    }

    #[test]
    fn a_tank_sleeps_after_six_still_checks_and_not_while_it_creeps() {
        let dt = 1.0 / 60.0;
        let mut still = Rest::default();
        let steps = |r: &mut Rest, speed: f32| {
            (1..=120).find(|&i| r.step(Vec3::Z * speed * i as f32 * dt, dt))
        };
        // Still: the first check sets where it stands, then six in a row
        // within 0.02 m (each 0.2 s): asleep at 1.2 s.
        assert_eq!(steps(&mut still, 0.0), Some(72));
        // Creeping at 0.15 m/s (0.03 m a check): never.
        let mut creeping = Rest::default();
        assert_eq!(steps(&mut creeping, 0.15), None);
        // At 0.05 m/s (0.01 m a check) it still sleeps.
        let mut slow = Rest::default();
        assert_eq!(steps(&mut slow, 0.05), Some(72));
    }

    #[test]
    fn an_empty_tank_coasts_on_its_rolling_grip_not_its_brake() {
        // Throttle 0 at 5 m/s ahead: its grip eases to its rolling grip
        // (0.1 for the AMX30), not its brake (4), as nobody brakes it.
        let mut t = Tracks::new(AMX30, 6);
        for _ in 0..240 {
            tank_step(&mut t, Vec3::Z * 5.0, 0.0, 0.0);
        }
        for g in &t.grip {
            assert!((g.0 - AMX30.friction.0).abs() < 1e-3, "grip {g:?}");
        }
    }

    #[test]
    fn tracks_push_along_their_curve() {
        // The AMX-30's: 1, 0.45, 0.3, 0.1 at 0, 0.4, 0.6, 1.
        let curve = [(0.0, 1.0), (0.4, 0.45), (0.6, 0.3), (1.0, 0.1)];
        assert_eq!(track_push(curve, 0.0), 1.0);
        assert!((track_push(curve, 0.2) - 0.725).abs() < 1e-5);
        assert!((track_push(curve, 0.4) - 0.45).abs() < 1e-5);
        assert!((track_push(curve, 0.5) - 0.375).abs() < 1e-5);
        assert!((track_push(curve, 0.8) - 0.2).abs() < 1e-5);
        assert!((track_push(curve, 1.0) - 0.1).abs() < 1e-5);
    }

    /// Dropped 1 m onto a 20 degree slope and parked 12 s, a 360-driven
    /// tank with a short squeeze (0.4 m, the PLZ45's springs) and its
    /// chassis's shape 0.38 m up: how far it slid over the last 10 s, and
    /// how many of its six track points touch at the end.
    fn parked_tank_on_a_slope(shape_floor: Option<f32>) -> (f32, usize) {
        let (slid, touching, _) = parked_tank(shape_floor, 20.0, None);
        (slid, touching)
    }

    /// [`parked_tank_on_a_slope`] on a slope of `degrees`, with how far it
    /// dropped below where it stood at 2 s, at its lowest over the last
    /// 10 s, and whether it ends asleep. `turret`: a
    /// turret on its own body on a hinge, turned at that many radians a
    /// second (0: kept still).
    fn parked_tank(
        shape_floor: Option<f32>,
        degrees: f32,
        turret: Option<f32>,
    ) -> (f32, usize, (f32, bool)) {
        use crate::vehicle::WheelSpec;
        let mut spec = tank();
        spec.wheels = [-2.3f32, 0.0, 2.3]
            .iter()
            .flat_map(|&z| {
                [-1.2f32, 1.2].map(|x| WheelSpec {
                    centre: Vec3::new(x, 0.45, z),
                    radius: 0.45,
                    width: 0.5,
                    steers: false,
                })
            })
            .collect();
        spec.tuning.mass = Some(30_000.0);
        spec.tuning.tank = Some(TankTuning {
            friction: (0.5, 0.9),
            springs: (30.0, 3.0, 0.3, 0.4),
            ..AMX30
        });
        let box_points = |lo: Vec3, hi: Vec3| {
            (0..8)
                .map(|i| {
                    Vec3::new(
                        if i & 1 == 0 { lo.x } else { hi.x },
                        if i & 2 == 0 { lo.y } else { hi.y },
                        if i & 4 == 0 { lo.z } else { hi.z },
                    )
                })
                .collect::<Vec<_>>()
        };
        spec.chassis = Some(crate::vehicle::Chassis {
            mass: 30_000.0,
            inertia: Mat3::from_diagonal(Vec3::new(100_000.0, 110_000.0, 40_000.0)),
            centre_of_mass: Vec3::new(0.0, 0.9, 0.0),
        });
        spec.shapes = shape_floor
            .map(|y| {
                vec![box_points(
                    Vec3::new(-1.6, y, -3.3),
                    Vec3::new(1.6, 1.6, 3.3),
                )]
            })
            .unwrap_or_default()
            .into();
        if turret.is_some() {
            spec.turret_shapes = vec![box_points(
                Vec3::new(-1.0, 1.6, -1.0),
                Vec3::new(1.0, 2.4, 1.0),
            )]
            .into();
            spec.turret_mass = Some(crate::vehicle::Chassis {
                mass: 3000.0,
                inertia: Mat3::from_diagonal(Vec3::new(2000.0, 2500.0, 1000.0)),
                centre_of_mass: Vec3::new(0.0, 2.0, 0.0),
            });
            spec.pivot = Some(Vec3::new(0.0, 2.0, 0.0));
            spec.tuning.turret = Some(crate::tuning::TurretTuning {
                main: true,
                yaw_speed: 1.0,
                yaw_accel: 20.0,
                pitch: (-0.1, 0.3),
                pitch_speed: 1.0,
                pitch_accel: 20.0,
                yaw: None,
            });
        }
        let mut p = Physics::new();
        let tilt = Quat::from_rotation_x(-degrees.to_radians());
        let half = 200.0;
        let v = [
            Vec3::new(-half, 0.0, -half),
            Vec3::new(half, 0.0, -half),
            Vec3::new(half, 0.0, half),
            Vec3::new(-half, 0.0, half),
        ];
        let shape = CollisionShape::mesh(&v, &[[0, 1, 2], [0, 2, 3]]).unwrap();
        p.add_static(StaticKey::Cell(1), &shape, Vec3::ZERO, tilt);
        p.step(DT);
        let id = p.add_vehicle_turned(&spec, tilt * Vec3::Y, tilt);
        let parked = VehicleControls {
            parked: true,
            ..Default::default()
        };
        let mut from = Vec3::ZERO;
        let mut low = f32::MAX;
        for i in 0..(12.0 / DT) as usize {
            if i == (2.0 / DT) as usize {
                from = p.vehicle_transform(id).unwrap().w_axis.truncate();
            }
            p.drive(id, parked);
            if let Some(rate) = turret {
                p.turn_turret(id, rate * i as f32 * DT);
            }
            p.step(DT);
            if i >= (2.0 / DT) as usize {
                let y = p.vehicle_transform(id).unwrap().w_axis.y;
                low = low.min(y);
            }
        }
        let at = p.vehicle_transform(id).unwrap().w_axis.truncate();
        (
            (at - from).length(),
            p.vehicle_contacts(id).0,
            (from.y - low, p.vehicle_asleep(id)),
        )
    }

    #[test]
    fn a_parked_tank_sleeps_and_stays_up() {
        // On its springs alone over flat ground (its shape 0.38 m up), it
        // keeps still and sleeps with its island (Havok's 0x8264ABF8).
        let (slid, touching, (dropped, asleep)) = parked_tank(Some(0.38), 0.0, None);
        assert!(asleep, "awake");
        assert!(dropped < 0.05, "dropped {dropped} m");
        assert!(slid < 0.1, "slid {slid} m");
        assert_eq!(touching, 6);
    }

    #[test]
    fn a_parked_tank_whose_turret_keeps_its_island_awake_stays_up() {
        // Its turret on its own body, held by its hinge's motor, keeps its
        // island awake, so it may not sleep (Havok's 0x8264ABF8 sleeps an
        // island only once none of its bodies moves) and its tracks go on
        // holding it up. Put to sleep on its own (rapier's
        // `RigidBody::sleep`) rapier still moved it with its awake turret
        // while its tracks no longer held it: it dropped 0.39 m here,
        // caught by its shape, and through the HQ's floor at PmcCon001's
        // villa.
        let (slid, touching, (dropped, _)) = parked_tank(Some(0.38), 0.0, Some(0.0));
        assert!(dropped < 0.05, "dropped {dropped} m");
        assert!(slid < 0.1, "slid {slid} m");
        assert_eq!(touching, 6);
    }

    #[test]
    fn a_tank_lands_on_its_chassis_shapes_and_stays_on_its_tracks() {
        // On its chassis's own shape (the 360 body's, 0x8216A290) its
        // bottomed-out rays start above the ground: it springs back onto
        // its tracks and holds.
        let (slid, touching) = parked_tank_on_a_slope(Some(0.38));
        assert!(slid < 2.0, "slid {slid} m");
        assert_eq!(touching, 6);
    }

    /// The AMX-30's `_TankPhysics`.
    const AMX30: TankTuning = TankTuning {
        max_speed: 45.0 / 3.6,
        brake: 4.0,
        steer_speed: 12.0,
        steer_inertia: (0.95, 0.65),
        friction: (0.1, 0.9),
        engine_height: -0.3,
        gravity_factor: 2.0,
        steer_loss: 0.1,
        high_speed_grip: 1.0,
        ang_damp: [9.0, 0.0, 9.0],
        centre_of_mass: [0.0; 3],
        torque: 2000.0,
        engine_inertia: 0.85,
        curve: [(0.0, 1.0), (0.4, 0.45), (0.6, 0.3), (1.0, 0.1)],
        springs: (20.0, 2.5, 0.35, 0.7),
    };

    /// One `drive_tracks` step of a still AMX-30 on six points at rest:
    /// the sum of its linear impulses and of their turn about its centre.
    fn tank_step(t: &mut Tracks, v: Vec3, throttle: f32, steer: f32) -> (Vec3, Vec3) {
        // Each point's spring just touching (squeezed 0: it pushes
        // nothing).
        let full = AMX30.spring_length();
        let points: Vec<Option<TrackContact>> = [-1.2f32, 1.2]
            .iter()
            .flat_map(|&x| {
                [-2.0f32, 0.0, 2.0].map(|z| {
                    let at = Vec3::new(x, -1.0, z);
                    Some(TrackContact {
                        at,
                        normal: Vec3::Y,
                        start: at + Vec3::Y * full,
                    })
                })
            })
            .collect();
        let f = TrackFooting {
            turn: Quat::IDENTITY,
            centre: Vec3::ZERO,
            mass: 32000.0,
            points: &points,
            throttle,
            steer,
        };
        let (mut sum, mut spin) = (Vec3::ZERO, Vec3::ZERO);
        drive_tracks(
            t,
            &f,
            || (Vec3::ZERO, v),
            |i, at| match at {
                Some(p) => {
                    sum += i;
                    spin += p.cross(i);
                }
                None => spin += i,
            },
            DT,
        );
        (sum, spin)
    }

    #[test]
    fn a_tank_drives_against_its_throttle_and_turns_as_the_360_code_turns_it() {
        // Full throttle ahead (-1 for the 360 code) from a standstill:
        // its throttle eases 15% in, its curve gives 1, so 0.15 x 2000 x
        // 200 N for a step, along +z.
        let mut t = Tracks::new(AMX30, 6);
        let (sum, _) = tank_step(&mut t, Vec3::ZERO, -1.0, 0.0);
        let want = 0.15 * 2000.0 * 200.0 * DT;
        assert!((sum.z - want).abs() < want * 1e-3, "{sum}");
        assert!(sum.x.abs() < 1e-3);
        // Full right (+1): its steering eases 5% in and the pair of pushes
        // turns it clockwise from above (-y), 2 x mass x 12 x 0.05.
        let mut t = Tracks::new(AMX30, 6);
        let (sum, spin) = tank_step(&mut t, Vec3::ZERO, 0.0, 1.0);
        assert!(sum.length() < 1e-3, "{sum}");
        let want = 2.0 * 32000.0 * 12.0 * 0.05 * DT;
        assert!((spin.y + want).abs() < want * 1e-3, "{spin}");
        // Rolling at 5 m/s with the throttle against it: each point's
        // grip eases from rolling (0.1) towards its brake (4) and slows it
        // up to 1 m/s's worth, and the drive gives nothing.
        let mut t = Tracks::new(AMX30, 6);
        let (sum, _) = tank_step(&mut t, Vec3::Z * 5.0, 1.0, 0.0);
        let grip = 0.1 + (4.0 - 0.1) * 0.05;
        let want = -6.0 * 32000.0 * grip * DT;
        assert!((sum.z - want).abs() < -want * 1e-3, "{sum}");
    }
    #[test]
    fn a_car_driven_into_another_bumps_it_body_to_body() {
        let mut p = Physics::new();
        floor(&mut p, 1, 0.0, 0.0, 0.0, 200.0);
        p.step(DT);
        let a = p.add_vehicle(&car(), Vec3::new(0.0, 0.1, 0.0), 0.0);
        let b = p.add_vehicle(&car(), Vec3::new(0.0, 0.1, 10.0), 0.0);
        for _ in 0..60 {
            p.step(DT);
        }
        assert!(p.vehicle_bumps(a).is_empty(), "apart, they don't touch");
        let up = p.vehicle_velocity(a).y;
        p.set_vehicle_velocity(a, Vec3::new(0.0, up, 10.0));
        let mut met = None;
        for i in 0..120 {
            p.step(DT);
            if let Some(&(other, at, normal)) = p.vehicle_bumps(a).first() {
                met = Some((i, other, at, normal));
                break;
            }
        }
        let (_, other, at, normal) = met.expect("they never met");
        assert_eq!(other, b);
        // Where its front bumper meets the other's rear one, 7.8 m on.
        assert!((at.z - 7.8).abs() < 0.5, "met at {at:?}");
        // The normal points from it toward the other: on along +z.
        assert!(normal.z > 0.9, "normal {normal:?}");
        let back = p.vehicle_bumps(b);
        assert_eq!(back.first().map(|m| m.0), Some(a));
        assert!(back[0].2.z < -0.9, "and back the other way");
    }

    #[test]
    fn a_car_driven_into_a_wall_hits_it_and_the_ground_is_known() {
        let mut p = Physics::new();
        floor(&mut p, 1, 0.0, 0.0, 0.0, 200.0);
        let wall = StaticKey::Object(7);
        p.add_static(
            wall,
            &CollisionShape::cuboid(Vec3::new(5.0, 2.0, 0.5)),
            Vec3::new(0.0, 2.0, 10.0),
            Quat::IDENTITY,
        );
        p.step(DT);
        let a = p.add_vehicle(&car(), Vec3::new(0.0, 0.1, 0.0), 0.0);
        for _ in 0..60 {
            p.step(DT);
        }
        assert!(
            p.vehicle_static_hits(a).iter().all(|h| h.0 != wall),
            "apart, it doesn't touch the wall"
        );
        let up = p.vehicle_velocity(a).y;
        p.set_vehicle_velocity(a, Vec3::new(0.0, up, 10.0));
        let mut met = None;
        for _ in 0..120 {
            p.step(DT);
            if let Some(&h) = p.vehicle_static_hits(a).iter().find(|h| h.0 == wall) {
                met = Some(h);
                break;
            }
        }
        let (_, at, normal, mass) = met.expect("it never met the wall");
        // Its front bumper on the wall's face, 9.5 m on; toward it; fixed.
        assert!((at.z - 9.5).abs() < 0.5, "met at {at:?}");
        assert!(normal.z > 0.9, "normal {normal:?}");
        assert_eq!(mass, 0.0);
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
                    ..Default::default()
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

    // A car whose template gives its brakes and lock slows as the 360
    // code has its brakes slow it (each axle's m/s², added up) and turns
    // its front wheels as far as its lock, not the invented 0.6 rad.
    #[test]
    fn brakes_and_lock_come_from_the_template() {
        let slowing = |brake: (f32, f32), lock: f32| {
            let mut spec = car();
            spec.tuning.tyres = Some(Tyres {
                brake,
                ..Tyres::DEFAULT
            });
            spec.tuning.handling = Some(crate::tuning::Handling {
                steer_lock: lock,
                steer_speed: 100.0,
                low_speed: 0.0,
                low_steer_speed: 0.0,
                ..crate::tuning::Handling::DEFAULT
            });
            let mut p = Physics::new();
            floor(&mut p, 1, 0.0, 0.0, 0.0, 500.0);
            p.step(DT);
            let id = p.add_vehicle(&spec, Vec3::new(0.0, 0.1, -400.0), 0.0);
            for _ in 0..60 {
                p.step(DT);
            }
            // Full lock at a standstill.
            p.drive(
                id,
                VehicleControls {
                    steer: 1.0,
                    ..Default::default()
                },
            );
            p.step(DT);
            let steered = p.vehicle_wheels(id)[0].0;
            while p.vehicle_speed(id) < 12.0 {
                p.drive(
                    id,
                    VehicleControls {
                        throttle: 1.0,
                        ..Default::default()
                    },
                );
                p.step(DT);
            }
            let (v0, mut t) = (p.vehicle_speed(id), 0.0);
            while p.vehicle_speed(id) > 0.3 && t < 10.0 {
                p.drive(
                    id,
                    VehicleControls {
                        throttle: -1.0,
                        ..Default::default()
                    },
                );
                p.step(DT);
                t += DT;
            }
            (steered, v0 / t)
        };
        // Its engine drags it as well (the default engine's, off the
        // throttle): the brakes add their sum on top of that.
        let (lock, soft) = slowing((3.0, 1.0), 1.0);
        assert!((lock - 1.0).abs() < 0.05, "{lock}");
        assert!(soft > 3.8, "{soft}");
        let (lock, hard) = slowing((9.0, 7.0), 0.5);
        assert!((lock - 0.5).abs() < 0.05, "{lock}");
        assert!(((hard - soft) - 12.0).abs() < 1.5, "{soft} {hard}");
    }

    // Its front wheels swing towards full lock as quickly as its template
    // says (radians a second): `LowSpeedSteerSpeed` at a standstill,
    // easing to `SteerSpeed` at its `LowSpeedLimit`; let go, back straight
    // at once (`SteerNoInputInertia` 0); at its top speed its lock and
    // swing a tenth less (`HighSpeedSteerLoss`), times 0.5 braking and
    // 0.8 coasting.
    #[test]
    fn wheels_swing_at_the_templates_steer_speed() {
        let mut spec = car();
        spec.tuning.handling = Some(crate::tuning::Handling {
            steer_lock: 0.7,
            steer_speed: 0.6,
            low_speed: 35.0 / 3.6,
            low_steer_speed: 0.9,
            ..crate::tuning::Handling::DEFAULT
        });
        let mut p = Physics::new();
        floor(&mut p, 1, 0.0, 0.0, 0.0, 500.0);
        p.step(DT);
        let id = p.add_vehicle(&spec, Vec3::new(0.0, 0.1, -400.0), 0.0);
        for _ in 0..60 {
            p.step(DT);
        }
        let steer = |p: &mut Physics, steer: f32, secs: f32| {
            for _ in 0..(secs / DT).round() as usize {
                p.drive(
                    id,
                    VehicleControls {
                        steer,
                        ..Default::default()
                    },
                );
                p.step(DT);
            }
            p.vehicle_wheels(id)[0].0
        };
        // At a standstill: 0.9 rad/s, so 0.45 rad after half a second,
        // full lock (0.7) by 0.8 s, and straight again a step after.
        let half = steer(&mut p, 1.0, 0.5);
        assert!((half - 0.45).abs() < 0.03, "{half}");
        let full = steer(&mut p, 1.0, 0.5);
        assert!((full - 0.7).abs() < 0.01, "{full}");
        let back = steer(&mut p, 0.0, DT);
        assert!(back.abs() < 1e-6, "{back}");
        // Halfway to 35 km/h: 0.75 rad/s; above it 0.6.
        let h = spec.tuning.handling.expect("set");
        assert!((h.swing(17.5 / 3.6) - 0.75).abs() < 1e-5);
        assert_eq!(h.swing(-12.0), 0.6);
        assert!((h.range(1.0, 1.0, 0.0) - 0.9).abs() < 1e-6);
        assert!((h.range(1.0, 0.0, 1.0) - 0.95).abs() < 1e-6);
        assert!((h.range(1.0, 0.0, 0.0) - 0.92).abs() < 1e-6);
    }

    // Its springs hang as the 360 code hangs them: from their squeeze
    // above the model's wheels, as long as their lengths together. In the
    // air its wheels drop their stretch and the extra length below the
    // model's; standing on four of strength k with a gravity factor f,
    // they are squeezed f g / 4k on average (the back pair more, carrying
    // more of the weight), so its wheels sit that much higher, and the
    // chassis that much less above its model's height.
    #[test]
    fn its_springs_hang_as_the_360_codes() {
        for (k, f) in [(8.0, 1.0), (25.0, 2.0), (60.0, 2.0)] {
            let mut spec = car();
            let springs = Springs {
                strength: (k, k),
                damping: (1.7, 1.7),
                stretch: (0.25, 0.25),
                squeeze: (0.15, 0.15),
                extra: 0.05,
                gravity_factor: f,
            };
            spec.tuning.springs = Some(springs);
            let mut p = Physics::new();
            floor(&mut p, 1, 0.0, 0.0, 0.0, 500.0);
            p.step(DT);
            let id = p.add_vehicle(&spec, Vec3::new(0.0, spec.ride_height() + 3.0, -400.0), 0.0);
            p.step(DT);
            let hang = p.vehicle_wheel_lift(id);
            assert!(hang.iter().all(|l| (l + 0.3).abs() < 1e-4), "{k}: {hang:?}");
            for _ in 0..300 {
                p.step(DT);
            }
            let want = f * GRAVITY / (4.0 * k);
            let sag = p.vehicle_suspension(id);
            let mean = sag.iter().sum::<f32>() / 4.0;
            // Within 5%: Rapier's springs settle 2 to 4% short of what
            // k c = f g says.
            assert!((mean - want).abs() < 0.05 * want, "{k}: {sag:?} {want}");
            assert!(sag[2] > sag[0], "{k}: {sag:?}");
            let lift = p.vehicle_wheel_lift(id);
            let mean_lift = lift.iter().sum::<f32>() / 4.0;
            assert!((mean_lift - (mean - 0.3)).abs() < 1e-4, "{k}: {lift:?}");
            let y = p.vehicle_transform(id).expect("car").w_axis.y;
            assert!(
                (y - (spec.ride_height() + 0.3 - mean)).abs() < 0.02,
                "{k}: {y}"
            );
        }
    }

    // Sliding sideways, its tyres damp the slide as the 360 code's do: each
    // axle by its sideways grip a second, so with 1.5 on both it slows as
    // exp(-3 t), with no limit on how hard; on the handbrake (its
    // `EmergencyBrake` 0.2) the axles keep 0.33 + 0.67 x 0.2 of it.
    #[test]
    fn its_tyres_damp_a_sideways_slide_as_the_360s_do() {
        let slide = |handbrake: bool| {
            let mut spec = car();
            spec.tuning.tyres = Some(Tyres {
                emergency: (0.2, 0.2),
                ..Tyres::DEFAULT
            });
            let mut p = Physics::new();
            floor(&mut p, 1, 0.0, 0.0, 0.0, 500.0);
            p.step(DT);
            let id = p.add_vehicle(&spec, Vec3::new(0.0, 0.1, -400.0), 0.0);
            let still = VehicleControls {
                brake: handbrake,
                ..Default::default()
            };
            for _ in 0..90 {
                p.drive(id, still);
                p.step(DT);
            }
            p.set_vehicle_velocity(id, Vec3::X * 4.0);
            for _ in 0..15 {
                p.drive(id, still);
                p.step(DT);
            }
            p.vehicle_velocity(id).x
        };
        let gripped = slide(false);
        let want = 4.0 * (-3.0f32 * 0.25).exp();
        assert!((gripped - want).abs() < 0.15 * want, "{gripped} {want}");
        let held = slide(true);
        let want = 4.0 * (-3.0f32 * 0.25 * (0.33 + 0.67 * 0.2)).exp();
        assert!((held - want).abs() < 0.15 * want, "{held} {want}");
    }

    // Its brake lamps glow while it is driven against the way it rolls or
    // on its handbrake, and not as it drives on, coasts or stands parked.
    #[test]
    fn it_brakes_with_its_lamps_lit() {
        let mut p = Physics::new();
        floor(&mut p, 1, 0.0, 0.0, 0.0, 500.0);
        p.step(DT);
        let id = p.add_vehicle(&car(), Vec3::new(0.0, 0.5, -400.0), 0.0);
        let run = |p: &mut Physics, c: VehicleControls, steps: usize| {
            for _ in 0..steps {
                p.drive(id, c);
                p.step(DT);
            }
            p.vehicle_braking(id)
        };
        let go = VehicleControls {
            throttle: 1.0,
            ..VehicleControls::default()
        };
        assert!(!run(&mut p, go, 120));
        let back = VehicleControls {
            throttle: -1.0,
            ..VehicleControls::default()
        };
        assert!(run(&mut p, back, 5));
        assert!(!run(&mut p, VehicleControls::default(), 5));
        let handbrake = VehicleControls {
            brake: true,
            ..VehicleControls::default()
        };
        assert!(run(&mut p, handbrake, 5));
        let parked = VehicleControls {
            parked: true,
            ..VehicleControls::default()
        };
        assert!(!run(&mut p, parked, 5));
    }

    #[test]
    fn springs_and_dampers_come_from_the_template() {
        let ride = |k: f32, c: f32| {
            let mut spec = car();
            spec.tuning.springs = Some(Springs {
                strength: (k, k),
                damping: (c, c),
                gravity_factor: 1.0,
                ..Springs::DEFAULT
            });
            let mut p = Physics::new();
            floor(&mut p, 1, 0.0, 0.0, 0.0, 500.0);
            p.step(DT);
            let id = p.add_vehicle(&spec, Vec3::new(0.0, 0.1, -400.0), 0.0);
            for _ in 0..180 {
                p.step(DT);
            }
            let sag = p.vehicle_suspension(id).iter().sum::<f32>() / 4.0;
            let height = |p: &Physics| p.vehicle_transform(id).expect("car").w_axis.y;
            let rest = height(&p);
            let v = p.vehicle_velocity(id);
            p.set_vehicle_velocity(id, v - Vec3::Y * 0.3);
            let mut t = 0.0;
            loop {
                p.step(DT);
                t += DT;
                if height(&p) >= rest || t > 2.0 {
                    break;
                }
            }
            (sag, t)
        };
        let half = |k: f32, c: f32| {
            let w = (4.0 * k).sqrt();
            let zeta = 4.0 * c / (2.0 * w);
            std::f32::consts::PI / (w * (1.0 - zeta * zeta).sqrt())
        };
        // An M151's 25 and 1.7: 9.8 cm down, back up after 0.34 s.
        let (sag, t) = ride(25.0, 1.7);
        assert!((sag - 9.81 / 100.0).abs() < 0.01, "{sag}");
        assert!((t - half(25.0, 1.7)).abs() < 0.04, "{t}");
        // A sports car's 60 and 3: 4.1 cm, 0.22 s.
        let (sag, t) = ride(60.0, 3.0);
        assert!((sag - 9.81 / 240.0).abs() < 0.01, "{sag}");
        assert!((t - half(60.0, 3.0)).abs() < 0.04, "{t}");
    }

    #[test]
    fn a_car_s_hull_slides_free_upright_and_drags_once_tipped() {
        assert_eq!(car_hull_friction(1.0), 0.0);
        assert_eq!(car_hull_friction(0.01), 0.0);
        assert_eq!(car_hull_friction(0.0), CAR_TIPPED_FRICTION);
        assert_eq!(car_hull_friction(-1.0), 0.3);
        assert_eq!(hull_feel(&car()), (true, 0.0));
        assert_eq!(hull_feel(&helicopter()), (false, 0.3));
        let mut b = boat();
        b.tuning.boat = Some(BoatTuning {
            hull_friction: 0.6,
            ..BOAT
        });
        assert_eq!(hull_feel(&b), (false, 0.6));
        // Put down on its roof, a step on its hull holds on.
        let mut p = Physics::new();
        floor(&mut p, 1, 0.0, 0.0, 0.0, 200.0);
        p.step(DT);
        let id = p.add_vehicle_turned(
            &car(),
            Vec3::new(0.0, 2.0, 0.0),
            Quat::from_rotation_z(std::f32::consts::PI),
        );
        let hull = |p: &Physics| {
            let v = p.vehicles[id.0].as_ref().unwrap();
            let b = p.world.bodies.get(v.controller.chassis).unwrap();
            p.world.colliders.get(b.colliders()[0]).unwrap().friction()
        };
        assert_eq!(hull(&p), 0.0);
        p.step(DT);
        assert_eq!(hull(&p), CAR_TIPPED_FRICTION);
    }

    #[test]
    fn wrecks_drop_onto_their_rims_and_scrape_to_a_stop() {
        let mut p = Physics::new();
        floor(&mut p, 1, 0.0, 0.0, 0.0, 200.0);
        p.step(DT);
        let id = p.add_vehicle(&car(), Vec3::new(0.0, 0.1, 0.0), 0.0);
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
        let fast = p.vehicle_speed(id);
        assert!(fast > 5.0, "{fast}");
        p.wreck_vehicle(id);
        // Nobody can drive it now, nor does the throttle it had push it.
        for _ in 0..120 {
            p.drive(
                id,
                VehicleControls {
                    throttle: 1.0,
                    ..Default::default()
                },
            );
            p.step(DT);
        }
        let v = p.vehicle_velocity(id);
        assert!(v.length() < 0.3, "still sliding at {v}");
        // On its rims: the wheels' centres (0.35 m up the model) are
        // their rims' reach above the ground, the tyres' bottoms below it.
        let rims = 0.35 - 0.35 * WRECK_RIM;
        let y = p.vehicle_transform(id).unwrap().w_axis.y;
        assert!((y + rims).abs() < 0.05, "origin at {y}, not {}", -rims);
        let up = p.vehicle_transform(id).unwrap().transform_vector3(Vec3::Y);
        assert!(up.y > 0.9, "tipped: {up}");
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
    fn leaves_and_hidden_parts_dont_stand_in_the_way() {
        use m2_assets::physics::{Body, Geometry, Part, Shape};
        let part = |node: u32, body: Option<usize>, shape: Option<usize>| Part {
            position: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            node_hash: 0,
            node,
            body,
            shape,
            unk_link: 0,
            parent: None,
            first_child: None,
            next_sibling: None,
        };
        let shape = |part: usize, class: u32| {
            let mut record = [0; 26];
            record[4] = class;
            Shape {
                radius: 0.0,
                geometry: Geometry::Box {
                    half_extents: [1.0; 3],
                },
                part,
                record,
            }
        };
        let p = Phy2 {
            version: 0x39,
            model: 0,
            unk_0x14: [0; 3],
            constraints: Vec::new(),
            // A trunk and its leaves on node 1, a piece it breaks into on
            // node 2 (hidden intact).
            parts: vec![
                part(1, Some(0), None),
                part(1, None, Some(0)),
                part(1, None, Some(1)),
                part(2, None, Some(2)),
            ],
            bodies: vec![Body {
                inertia: [f32::MAX; 9],
                centre_of_mass: [0.0; 3],
                mass: 0.0,
                unk: [0.3; 4],
                unk_words: [0; 8],
            }],
            shapes: vec![shape(1, 0x27), shape(2, 0x28), shape(3, 0x27)],
        };
        let hidden = [false, false, true];
        let stands: Vec<bool> = (0..3).map(|k| standing(&p, k, &hidden)).collect();
        assert_eq!(stands, [true, false, false]);
        // A model that shows everything: only the leaves go through.
        assert!(standing(&p, 2, &[]));
        assert!(!standing(&p, 1, &[]));
    }

    #[test]
    fn a_loose_piece_carries_the_nodes_under_it() {
        // A base, an arm 2 m up on it, and a lamp 1 m along the arm.
        let rest = vec![
            Mat4::IDENTITY,
            Mat4::from_translation(Vec3::Y * 2.0),
            Mat4::from_translation(Vec3::new(1.0, 2.0, 0.0)),
        ];
        let loose = Loose {
            pieces: vec![LoosePiece {
                node: Some(1),
                shape: r::SharedShape::cuboid(0.5, 0.1, 0.1),
                mass: 10.0,
                material: BodyMaterial::DEBRIS,
                from_model: rest[1].inverse(),
            }],
            breaks: true,
            mass: 10.0,
            rest: rest.clone(),
            owner: vec![None, Some(0), Some(0)],
            flying: vec![false, true, true],
        };
        let placed = Mat4::from_translation(Vec3::new(100.0, 0.0, 0.0));
        let starts = loose.starts(placed);
        assert!(
            starts[0]
                .w_axis
                .truncate()
                .distance(Vec3::new(100.0, 2.0, 0.0))
                < 1e-4
        );
        for (a, b) in loose.posed(placed, &starts).iter().zip(&rest) {
            assert!(a.abs_diff_eq(*b, 1e-4), "{a} {b}");
        }
        // The arm down by the base, turned a quarter about z: the lamp
        // goes with it, the base stays.
        let fallen = Mat4::from_translation(Vec3::new(101.0, 0.1, 0.0))
            * Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let posed = loose.posed(placed, &[fallen]);
        assert!(posed[0].abs_diff_eq(rest[0], 1e-4));
        let lamp = (placed * posed[2]).w_axis.truncate();
        assert!(lamp.distance(Vec3::new(101.0, 1.1, 0.0)) < 1e-4, "{lamp}");
    }

    #[test]
    fn vehicles_drive_through_loose_pieces() {
        let mut p = Physics::new();
        floor(&mut p, 1, 0.0, 0.0, 0.0, 200.0);
        // A heavy beam lying across the way 10 m ahead, settled.
        let beam = LoosePiece {
            node: None,
            shape: r::SharedShape::cuboid(2.0, 0.3, 0.3),
            mass: 400.0,
            material: BodyMaterial::DEBRIS,
            from_model: Mat4::IDENTITY,
        };
        let d = p.add_debris(
            &beam,
            Mat4::from_translation(Vec3::new(0.0, 0.3, 10.0)),
            Vec3::ZERO,
            Vec3::ZERO,
        );
        for _ in 0..60 {
            p.step(DT);
        }
        let id = p.add_vehicle(&car(), Vec3::new(0.0, 0.1, 0.0), 0.0);
        for _ in 0..60 {
            p.step(DT);
        }
        let rest = p.vehicle_transform(id).unwrap().w_axis.y;
        let mut highest = rest;
        for _ in 0..240 {
            p.drive(
                id,
                VehicleControls {
                    throttle: 1.0,
                    ..Default::default()
                },
            );
            p.step(DT);
            highest = highest.max(p.vehicle_transform(id).unwrap().w_axis.y);
        }
        let at = p.vehicle_transform(id).unwrap().w_axis.truncate();
        assert!(at.z > 16.0, "drove past the beam to {at}");
        assert!(highest < rest + 0.1, "rode up from {rest} to {highest}");
        // And the beam lies where it was.
        let beam_at = p.debris_place(d).unwrap().w_axis.truncate();
        assert!(
            beam_at.distance(Vec3::new(0.0, 0.3, 10.0)) < 0.2,
            "{beam_at}"
        );
    }

    /// A 4 m wide, 2 m tall box 12 m ahead, `ghost` (only its AI box, as a
    /// jungle rock cluster's flags 0x25 shape) or solid: how far a car
    /// driving at it for 6 s gets, and whether a path sweep through it
    /// meets it.
    fn drive_at_box(ghost: bool) -> (f32, bool) {
        let mut p = Physics::new();
        floor(&mut p, 1, 0.0, 0.0, 0.0, 200.0);
        let block = r::SharedShape::cuboid(2.0, 1.0, 1.0);
        // A post far off as the model's solid part.
        let post = r::SharedShape::compound(vec![(
            pose(Vec3::new(30.0, 1.0, 0.0), Quat::IDENTITY),
            r::SharedShape::cuboid(0.1, 1.0, 0.1),
        )]);
        let mut shape = CollisionShape::cuboid(Vec3::ONE);
        if ghost {
            shape.shape = post;
            shape.ghost = Some(Ghost {
                shape: block,
                path: PathSolid::All,
            });
        } else {
            shape.shape = block;
        }
        p.add_static(
            StaticKey::Object(3),
            &shape,
            Vec3::new(0.0, 1.0, 12.0),
            Quat::IDENTITY,
        );
        p.step(DT);
        let met = p
            .sweep_obstacle(
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 1.0, 24.0),
                Vec3::new(1.0, 0.5, 0.05),
                None,
            )
            .is_some();
        let id = p.add_vehicle(&car(), Vec3::new(0.0, 0.1, 0.0), 0.0);
        for _ in 0..360 {
            p.drive(
                id,
                VehicleControls {
                    throttle: 1.0,
                    ..Default::default()
                },
            );
            p.step(DT);
        }
        (p.vehicle_transform(id).unwrap().w_axis.z, met)
    }

    #[test]
    fn a_car_drives_through_what_only_paths_see() {
        let (solid_z, solid_met) = drive_at_box(false);
        let (ghost_z, ghost_met) = drive_at_box(true);
        assert!(solid_met && ghost_met, "a path sweep meets both");
        assert!(solid_z < 12.0, "stopped by the solid box at {solid_z}");
        assert!(ghost_z > 20.0, "drove through the AI box to {ghost_z}");
    }

    // A tree: a trunk 1 m across stands in the way, its leaves 12 m across
    // don't; a path search sizes it up by all of it (0x821861B0).
    #[test]
    fn a_path_obstacles_box_is_its_whole_entitys() {
        let mut p = Physics::new();
        let mut tree = CollisionShape::cuboid(Vec3::new(0.5, 7.0, 0.5));
        tree.entity_box = Some((Vec3::new(-6.0, -7.0, -6.0), Vec3::new(6.0, 13.0, 6.0)));
        p.add_static(
            StaticKey::Object(9),
            &tree,
            Vec3::new(0.0, 7.0, 10.0),
            Quat::IDENTITY,
        );
        p.step(DT);
        let o = p
            .sweep_obstacle(
                Vec3::new(0.0, 2.0, 0.0),
                Vec3::new(0.0, 2.0, 20.0),
                Vec3::new(1.0, 1.0, 0.05),
                None,
            )
            .expect("meets the trunk");
        assert_eq!(o.min, Vec3::new(-6.0, -7.0, -6.0));
        assert_eq!(o.max, Vec3::new(6.0, 13.0, 6.0));
        // Gone with it.
        p.remove_static(StaticKey::Object(9));
        assert!(p.boxes.is_empty());
    }

    #[test]
    fn only_some_layers_are_met_by_vehicles() {
        use crate::collision_layers::{BUILDING, meets_vehicles};
        // A rock cluster's two shapes (0x822E6FD8's sweep sees the first,
        // a tank meets the second), a chain fence, a wall, an AI bridge.
        let layer = |flags| layer_of(&BUILDING, flags, 3);
        assert!(!meets_vehicles(layer(0x25)));
        assert!(meets_vehicles(layer(0x03)));
        assert!(meets_vehicles(layer(0x07)));
        assert!(meets_vehicles(layer(0x27)));
        assert!(!meets_vehicles(layer(0x60)));
        assert!(!meets_vehicles(layer(0x05)));
    }

    /// A model of two 2 m boxes 10 m apart along x: one of flags 0x27 (a
    /// solid shape), one of 0x25 (an AI-only box), as its source.
    fn two_box_model() -> CollisionShape {
        let mut vertices = Vec::new();
        let mut triangles = Vec::new();
        let mut runs = Vec::new();
        for (x, flags) in [(0.0f32, 0x27u32), (10.0, 0x25)] {
            let lo = Vec3::new(x - 1.0, 0.0, -1.0);
            let hi = Vec3::new(x + 1.0, 2.0, 1.0);
            let first = vertices.len() as u32;
            let (v, t) = m2_assets::physics::Geometry::Box {
                half_extents: [1.0, 1.0, 1.0],
            }
            .triangles(0.0);
            vertices.extend(v.iter().map(|&p| (lo + hi) * 0.5 + Vec3::from(p)));
            let from = triangles.len() as u32;
            triangles.extend(t.iter().map(|t| t.map(|i| first + i)));
            runs.push(ShapeRun {
                flags,
                kind: 3,
                triangles: from..triangles.len() as u32,
            });
        }
        let mut shape = CollisionShape::cuboid(Vec3::ONE);
        shape.source = Some(Arc::new(ShapeSource {
            vertices,
            triangles,
            runs,
        }));
        shape
    }

    // A loose prop's solid shapes are on layer 18 (0x82155328, 0x8218EBF8),
    // which no path search sees; a fixed prop's are on 13, which it does.
    // Its AI-only box (layer 14) is seen either way and no vehicle meets it.
    #[test]
    fn a_loose_props_solid_shapes_are_in_no_paths_way() {
        let model = two_box_model();
        for (preset, solid_seen) in [(Preset::Prop, true), (Preset::LooseProp, false)] {
            let mut p = Physics::new();
            p.add_static_as(
                StaticKey::Entity(1),
                &model,
                Vec3::ZERO,
                Quat::IDENTITY,
                preset,
            );
            p.step(DT);
            let sweep = |x: f32| {
                p.sweep_obstacle(
                    Vec3::new(x, 1.0, -5.0),
                    Vec3::new(x, 1.0, 5.0),
                    Vec3::new(0.2, 0.2, 0.05),
                    None,
                )
                .is_some()
            };
            assert_eq!(sweep(0.0), solid_seen, "{preset:?} solid box");
            assert!(sweep(10.0), "{preset:?} AI box");
            // The solid box is solid; the AI box is a sensor.
            let ray = |x: f32| {
                let ray = r::Ray::new(vector(Vec3::new(x, 5.0, 0.0)), vector(Vec3::NEG_Y));
                p.world
                    .query_pipeline_with_filter(ground())
                    .cast_ray(&ray, 10.0, true)
                    .is_some()
            };
            assert!(ray(0.0), "{preset:?} stands on the solid box");
            assert!(!ray(10.0), "{preset:?} not on the AI box");
        }
    }

    // A bush: a trunk 0.4 m across and leaves 4 m across round it, 12 m
    // ahead. A path sweep and an AI's sight line meet the leaves (layer 19
    // in both masks); a shot's line, a wheel's look for the ground and the
    // car itself go through them.
    #[test]
    fn leaves_stand_in_paths_and_sight_lines_alone() {
        let mut p = Physics::new();
        floor(&mut p, 1, 0.0, 0.0, 0.0, 200.0);
        let mut bush = CollisionShape::cuboid(Vec3::new(0.2, 1.5, 0.2));
        bush.leaves = Some(r::SharedShape::cuboid(2.0, 1.0, 2.0));
        p.add_static(
            StaticKey::Object(4),
            &bush,
            Vec3::new(3.0, 1.5, 12.0),
            Quat::IDENTITY,
        );
        p.step(DT);
        // Past the trunk, through the leaves.
        let (a, b) = (Vec3::new(4.5, 1.5, 0.0), Vec3::new(4.5, 1.5, 24.0));
        assert!(
            p.sweep_obstacle(a, b, Vec3::new(0.1, 0.1, 0.05), None)
                .is_some(),
            "a path sweep meets the leaves"
        );
        assert!(!p.sight_clear(a, b, None), "an AI doesn't see through them");
        assert!(p.line_clear(a, b, None), "a shot's line goes through them");
        let ray = r::Ray::new(vector(Vec3::new(4.5, 5.0, 12.0)), vector(Vec3::NEG_Y));
        let (_, toi) = p
            .world
            .query_pipeline_with_filter(ground())
            .cast_ray(&ray, 10.0, true)
            .expect("the floor");
        assert!(
            (toi - 5.0).abs() < 0.01,
            "a wheel stands on the floor, not the leaves: {toi}"
        );
        // A car drives through them.
        let id = p.add_vehicle(&car(), Vec3::new(4.5, 0.1, 0.0), 0.0);
        for _ in 0..360 {
            p.drive(
                id,
                VehicleControls {
                    throttle: 1.0,
                    ..Default::default()
                },
            );
            p.step(DT);
        }
        let z = p.vehicle_transform(id).unwrap().w_axis.z;
        assert!(z > 20.0, "drove through the leaves to {z}");
        // Gone with it.
        p.remove_static(StaticKey::Object(4));
        assert!(p.sight_clear(a, b, None));
    }

    // CarAttack's line to her (0x82166958, the path mask) passes a loose
    // prop's solid shape (layer 18) and stops at leaves (19); a soldier's
    // line of fire (0x822C95D8, the sight mask) stops at both.
    #[test]
    fn ai_lines_see_loose_props_and_leaves_by_their_masks() {
        let mut p = Physics::new();
        let model = two_box_model();
        p.add_static_as(
            StaticKey::Entity(1),
            &model,
            Vec3::ZERO,
            Quat::IDENTITY,
            Preset::LooseProp,
        );
        let mut bush = CollisionShape::cuboid(Vec3::new(0.05, 0.05, 0.05));
        bush.leaves = Some(r::SharedShape::cuboid(1.0, 1.0, 1.0));
        p.add_static(
            StaticKey::Object(2),
            &bush,
            Vec3::new(20.0, 5.0, 0.0),
            Quat::IDENTITY,
        );
        p.step(DT);
        let across = |x: f32, y: f32| (Vec3::new(x, y, -5.0), Vec3::new(x, y, 5.0));
        let (a, b) = across(0.0, 1.0);
        assert!(
            p.path_line_clear(a, b, None),
            "a loose prop is in no CarAttack line's way"
        );
        assert!(
            p.cast_ray_sight_keyed(a, b - a, 10.0).is_some(),
            "a soldier's line meets it"
        );
        let (a, b) = across(20.5, 5.0);
        assert!(
            !p.path_line_clear(a, b, None),
            "leaves stop CarAttack's line"
        );
        assert!(
            p.cast_ray_sight_keyed(a, b - a, 10.0).is_some(),
            "and a soldier's"
        );
        assert!(p.cast_ray_keyed(a, b - a, 10.0).is_none(), "but not a shot");
    }

    // A broken fence's planks stay its entity's bodies on the 360: a path
    // sweep meeting one plank sizes it up by the box round both, in the
    // fence's frame; a lone piece by its own.
    #[test]
    fn a_broken_models_pieces_share_its_path_box() {
        let mut p = Physics::new();
        let plank = LoosePiece {
            node: None,
            shape: r::SharedShape::cuboid(2.0, 0.5, 0.2),
            mass: 20.0,
            material: BodyMaterial::DEBRIS,
            from_model: Mat4::IDENTITY,
        };
        let fence = Mat4::from_translation(Vec3::new(0.0, 0.0, 10.0));
        let a = p.add_debris(
            &plank,
            fence * Mat4::from_translation(Vec3::new(-3.0, 0.5, 0.0)),
            Vec3::ZERO,
            Vec3::ZERO,
        );
        let b = p.add_debris(
            &plank,
            fence * Mat4::from_translation(Vec3::new(4.0, 0.5, 0.0)),
            Vec3::ZERO,
            Vec3::ZERO,
        );
        p.add_debris(
            &plank,
            Mat4::from_translation(Vec3::new(30.0, 0.5, 10.0)),
            Vec3::ZERO,
            Vec3::ZERO,
        );
        p.group_debris(&[a, b], fence);
        p.step(DT);
        let sweep = |p: &Physics, x: f32| {
            p.sweep_obstacle(
                Vec3::new(x, 0.5, 0.0),
                Vec3::new(x, 0.5, 20.0),
                Vec3::new(0.1, 0.1, 0.05),
                None,
            )
        };
        let o = sweep(&p, -3.0).expect("meets the first plank");
        assert!(
            (o.min.x + 5.0).abs() < 0.1 && (o.max.x - 6.0).abs() < 0.1,
            "{} .. {}",
            o.min,
            o.max
        );
        assert!((o.pose.0 - Vec3::new(0.0, 0.0, 10.0)).length() < 1e-3);
        let o = sweep(&p, 30.0).expect("meets the lone piece");
        assert!(
            (o.max.x - o.min.x - 4.0).abs() < 0.1,
            "{} .. {}",
            o.min,
            o.max
        );
        // Gone with its pieces.
        p.remove_debris(a);
        p.remove_debris(b);
        assert!(p.debris_groups.is_empty());
    }

    #[test]
    fn a_car_knocks_a_loose_pole_over_and_ploughs_on() {
        let mut p = Physics::new();
        floor(&mut p, 1, 0.0, 0.0, 0.0, 200.0);
        // A 6 m pole, 150 kg, standing 12 m ahead of the car.
        let piece = LoosePiece {
            node: None,
            shape: r::SharedShape::compound(vec![(
                pose(Vec3::Y * 3.0, Quat::IDENTITY),
                r::SharedShape::cuboid(0.1, 3.0, 0.1),
            )]),
            mass: 150.0,
            material: BodyMaterial::DEBRIS,
            from_model: Mat4::IDENTITY,
        };
        let pole = CollisionShape {
            shape: r::SharedShape::compound(vec![(
                pose(Vec3::Y * 3.0, Quat::IDENTITY),
                r::SharedShape::cuboid(0.1, 3.0, 0.1),
            )]),
            triangles: 12,
            stand_in: false,
            loose: None,
            whole: None,
            material: None,
            surfaces: Arc::new([]),
            path: PathSolid::All,
            ghost: None,
            entity_box: None,
            source: None,
            leaves: None,
        };
        let base = Vec3::new(0.0, 0.0, 12.0);
        p.add_static(StaticKey::Object(7), &pole, base, Quat::IDENTITY);
        p.step(DT);
        let id = p.add_vehicle(&car(), Vec3::new(0.0, 0.1, 0.0), 0.0);
        let mut hit = None;
        for _ in 0..360 {
            p.drive(
                id,
                VehicleControls {
                    throttle: 1.0,
                    ..Default::default()
                },
            );
            p.step(DT);
            if p.vehicle_statics(id).contains(&StaticKey::Object(7)) {
                hit = Some(p.vehicle_velocity_before(id));
                break;
            }
        }
        let before = hit.expect("ran into the pole");
        assert!(before.z > 4.0, "hit it at {before}");
        assert!(p.vehicle_pulling(id));
        // Knocked loose: it goes, its piece flies, the car goes on.
        p.remove_static(StaticKey::Object(7));
        let share = 1400.0 / (1400.0 + piece.mass);
        let d = p.add_debris(
            &piece,
            Mat4::from_translation(base),
            before * share * 1.1 + Vec3::Y,
            Vec3::X * before.z * 0.3,
        );
        p.set_vehicle_velocity(id, before * share);
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
        let car_at = p.vehicle_transform(id).unwrap().w_axis.truncate();
        assert!(car_at.z > base.z + 5.0, "the car ploughed on to {car_at}");
        let fallen = p.debris_place(d).unwrap();
        let up = fallen.transform_vector3(Vec3::Y);
        assert!(up.y < 0.5, "the pole fell over: up {up}");
        let middle = fallen.transform_point3(Vec3::Y * 3.0);
        assert!(
            middle.y < 1.0 && middle.y > -0.2,
            "lies on the ground: {middle}"
        );
        p.remove_debris(d);
        assert!(p.debris_place(d).is_none());
    }

    #[test]
    fn rubble_lands_and_she_walks_through_it() {
        let mut p = ready();
        p.step(DT);
        // A slab 4 m long, 2 m high and 1 m thick, thrown up across her
        // way.
        let slab = LoosePiece {
            node: Some(3),
            shape: r::SharedShape::compound(vec![(
                pose(Vec3::Y, Quat::IDENTITY),
                r::SharedShape::cuboid(2.0, 1.0, 0.5),
            )]),
            mass: 9600.0,
            material: BodyMaterial::DEBRIS,
            from_model: Mat4::IDENTITY,
        };
        assert_eq!(slab.centre(), Vec3::Y);
        assert!((slab.size() - 21.0f32.sqrt()).abs() < 1e-4);
        let d = p.add_rubble(
            &slab,
            Mat4::from_translation(Vec3::new(0.0, 2.0, 5.0)),
            Vec3::Y * 3.0,
            Vec3::X * 0.5,
        );
        for _ in 0..240 {
            p.step(DT);
        }
        let lies = p.debris_place(d).unwrap().transform_point3(Vec3::Y);
        assert!(lies.y > 0.3 && lies.y < 1.6, "on the ground: {lies}");
        // What holds it, and what stands where it lies.
        assert_eq!(p.debris_statics(d), [StaticKey::Cell(1)]);
        let around = (Vec3::splat(-3.0), Vec3::splat(3.0));
        let place = Mat4::from_translation(lies);
        assert_eq!(p.statics_in_box(place, around), [StaticKey::Cell(1)]);
        let above = Mat4::from_translation(lies + Vec3::Y * 10.0);
        assert!(p.statics_in_box(above, around).is_empty());
        // Rays and she pass it.
        let down = p.cast_ray(lies + Vec3::Y * 5.0, Vec3::NEG_Y, 20.0).unwrap();
        assert!((down.0 - (lies.y + 5.0)).abs() < 0.05, "{down:?}");
        let mut c = Character::new(Vec3::ZERO);
        walk(&p, &mut c, Vec3::new(0.0, 0.0, 4.0), 3.0);
        assert!(c.position.z > 10.0, "stopped at {}", c.position);
    }

    #[test]
    fn held_vehicles_stay_put_without_ground() {
        let mut p = ready();
        p.step(DT);
        let id = p.add_vehicle(&car(), Vec3::new(0.0, 0.1, 0.0), 0.0);
        for _ in 0..60 {
            p.step(DT);
        }
        let rest = p.vehicle_transform(id).unwrap().w_axis.truncate();
        p.hold_vehicle(id, true);
        assert!(p.vehicle_held(id));
        // The ground streams out from under it.
        p.remove_static(StaticKey::Cell(1));
        for _ in 0..120 {
            p.step(DT);
        }
        let held = p.vehicle_transform(id).unwrap().w_axis.truncate();
        assert!(held.distance(rest) < 1e-3, "{rest} -> {held}");
        // Back, and let go: it rests on it again.
        floor(&mut p, 1, 0.0, 0.0, 0.0, 50.0);
        p.step(DT);
        p.hold_vehicle(id, false);
        for _ in 0..60 {
            p.step(DT);
        }
        let now = p.vehicle_transform(id).unwrap().w_axis.truncate();
        assert!(now.distance(rest) < 0.1, "{rest} -> {now}");
    }

    #[test]
    fn wheels_stop_rolling_when_a_vehicle_stops_or_is_held() {
        let mut p = ready();
        p.step(DT);
        let id = p.add_vehicle(&car(), Vec3::new(0.0, 0.1, 0.0), 0.0);
        let go = |p: &mut Physics, controls: VehicleControls, steps: usize| {
            for _ in 0..steps {
                p.drive(id, controls);
                p.step(DT);
            }
        };
        go(
            &mut p,
            VehicleControls {
                throttle: 1.0,
                ..Default::default()
            },
            120,
        );
        assert!(p.vehicle_rolling(id) > 2.0, "{}", p.vehicle_rolling(id));
        // Held while moving: the controller's reading stays as it was,
        // the wheels don't turn.
        p.hold_vehicle(id, true);
        p.step(DT);
        assert_eq!(p.vehicle_speed(id), 0.0);
        assert_eq!(p.vehicle_rolling(id), 0.0);
        p.hold_vehicle(id, false);
        // Parked and settled: still.
        go(
            &mut p,
            VehicleControls {
                parked: true,
                ..Default::default()
            },
            300,
        );
        assert_eq!(p.vehicle_rolling(id), 0.0);
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

    // A chain-link fence (no shape in the path mask) is no obstacle to a
    // path search, though solid to a query; a model with some shapes in the
    // mask is met only at those.
    #[test]
    fn a_path_search_sees_only_what_stands_in_an_ais_way() {
        let sweep = |p: &Physics| {
            p.sweep_obstacle(
                Vec3::new(0.0, 1.0, -5.0),
                Vec3::new(0.0, 1.0, 5.0),
                Vec3::splat(0.5),
                None,
            )
        };
        let wall = CollisionShape::cuboid(Vec3::new(5.0, 2.0, 0.25));
        let mut p = Physics::new();
        p.add_static(StaticKey::Object(1), &wall, Vec3::ZERO, Quat::IDENTITY);
        p.step(0.01);
        assert!(sweep(&p).is_some());

        let mut fence = wall.clone();
        fence.path = PathSolid::Nothing;
        p.add_static(StaticKey::Object(1), &fence, Vec3::ZERO, Quat::IDENTITY);
        p.step(0.01);
        assert!(sweep(&p).is_none());
        assert!(!p.line_clear(Vec3::new(0.0, 1.0, -5.0), Vec3::new(0.0, 1.0, 5.0), None));
        assert!(p.path_ground(Vec3::new(0.0, 3.0, 0.0), None).is_none());

        // Only its far end stands in the way: a sweep past the near end
        // meets nothing, one along the far end meets it.
        let mut part = wall.clone();
        part.path = PathSolid::Part(r::SharedShape::cuboid(1.0, 2.0, 0.25));
        p.add_static(StaticKey::Object(1), &part, Vec3::X * 4.0, Quat::IDENTITY);
        p.step(0.01);
        assert!(sweep(&p).is_none());
        let far = p
            .sweep_obstacle(
                Vec3::new(4.0, 1.0, -5.0),
                Vec3::new(4.0, 1.0, 5.0),
                Vec3::splat(0.5),
                None,
            )
            .unwrap();
        assert!((far.point.z + 0.25).abs() < 0.1, "{:?}", far.point);
    }
}
