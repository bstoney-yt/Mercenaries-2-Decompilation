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
use std::sync::Arc;

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
    /// Cut from the model's visible triangles, standing in for collision
    /// the model doesn't carry (`PHY2`).
    pub stand_in: bool,
    /// How the model comes loose when knocked, if its `PHY2` has bodies
    /// that move.
    pub loose: Option<Arc<Loose>>,
}

impl fmt::Debug for CollisionShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CollisionShape")
            .field("triangles", &self.triangles)
            .field("stand_in", &self.stand_in)
            .field("loose", &self.loose)
            .finish()
    }
}

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
    /// Model space to the node's space, at rest.
    from_model: Mat4,
}

impl LoosePiece {
    /// A box of half extents `half` about `centre` (model space), moving
    /// node `node`, which sits at the model's origin at rest.
    pub fn block(node: Option<u16>, centre: Vec3, half: Vec3, mass: f32) -> Self {
        Self {
            node,
            shape: r::SharedShape::compound(vec![(
                pose(centre, Quat::IDENTITY),
                r::SharedShape::cuboid(half.x, half.y, half.z),
            )]),
            mass,
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
                    from_model: Mat4::IDENTITY,
                });
            }
        }
        Self::with_pieces(model, pieces, breaks, rest)
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
#[derive(Debug)]
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

/// A box round `PHY2` shape `k` (its convex radius included), placed by
/// `to` from its body node's space.
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
    let half = ((hi - lo) * 0.5 * scale.abs() + Vec3::splat(s.radius.max(0.0)))
        .max(Vec3::splat(LOOSE_HALF));
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
        }
    }

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
            stand_in: false,
            loose: None,
        })
    }

    /// The box the shape fills, in its own space.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        let b = self.shape.compute_local_aabb();
        (vec3(b.mins), vec3(b.maxs))
    }

    /// A model's collision in model space: its `PHY2` shapes if `phy2` is
    /// given and parses, else its rigid visible geometry at `lod` (a stand
    /// in).
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
        let mut loose = None;
        // Shapes of its own, whether or not any stands in the way: no
        // stand-in then.
        let mut shaped = false;
        if let Some(p) = phy2.and_then(|d| Phy2::parse(d).ok()) {
            if sub_object.is_none() {
                loose = Loose::of(model, &p).map(Arc::new);
            }
            shaped = !p.shapes.is_empty();
            let hidden = model
                .states
                .as_ref()
                .map(|s| s.hidden(&model.nodes, false))
                .unwrap_or_default();
            for (k, s) in p.shapes.iter().enumerate() {
                if !standing(&p, k, &hidden) {
                    continue;
                }
                // A shape sits at its part's place in its body node's
                // space.
                let node = p
                    .shape_node(k)
                    .and_then(|n| u16::try_from(n).ok())
                    .and_then(|n| model.node_to_model(n))
                    .unwrap_or(Mat4::IDENTITY);
                let to_model = node * p.shape_place(k).map_or(Mat4::IDENTITY, place);
                let (v, t) = s.geometry.triangles(s.radius);
                let first = vertices.len() as u32;
                vertices.extend(v.iter().map(|&x| to_model.transform_point3(Vec3::from(x))));
                triangles.extend(t.iter().map(|t| t.map(|i| first + i)));
            }
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
        Self::mesh(&vertices, &triangles).map(|shape| Self {
            stand_in,
            loose,
            ..shape
        })
    }
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
/// Left to itself, a vehicle rolling faster than this (m/s) slows at
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
/// Boats ([`Physics::set_water`]): how deep a boat's origin floats,
/// metres (the layers put the start's boat 0.35 m under the -36 m sea,
/// patrol boats at it); how hard it pushes on flat out, m/s²; how its
/// speed falls off coasting and braking, sideways (its keel) and up and
/// down, and how its rocking dies down, per second; how fast it turns at
/// speed, radians a second, from what speed, m/s; its top speed backing
/// up and when its template gives none, m/s; and the most of its weight
/// a wreck's buoyancy holds up (it sinks). All invented.
pub const FLOAT_DEPTH: f32 = 0.35;
const BOAT_PUSH: f32 = 5.0;
const BOAT_COAST: f32 = 0.3;
const BOAT_BRAKE: f32 = 1.2;
const BOAT_KEEL: f32 = 2.5;
const BOAT_HEAVE: f32 = 3.0;
const BOAT_ROCK: f32 = 2.5;
const BOAT_TURN: f32 = 0.9;
const BOAT_TURN_SPEED: f32 = 6.0;
const BOAT_REVERSE: f32 = 4.0;
const BOAT_TOP: f32 = 18.0;
const WRECK_FLOAT: f32 = 0.4;
/// A boat going fast planes (the playthroughs' speedboat rides high, bow
/// up): from a standstill to this speed, m/s, its hull rises up to this
/// far, metres, and its bow this much more than its stern (and the stern
/// that much less), metres. Invented, from the videos.
pub const PLANE_SPEED: f32 = 12.0;
const PLANE_RISE: f32 = 0.3;
const PLANE_TRIM: f32 = 0.25;
/// The steepest hill a wheeled vehicle takes about as fast as the flat,
/// radians: uphill the engine pulls as a lower gear would
/// (invented; the playthrough's jeep and tank take the hill roads at
/// speed). Steeper than this, it slows and in the end slides back.
const CLIMB: f32 = 0.52;
/// Tracks grip: on the ground, a tracked vehicle's weight doesn't pull it
/// down hills up to this steep, radians, so it climbs, turns and stands on
/// them as on the flat (Brendan: the game's tank "should be able to climb
/// almost any hill"; the steepness is invented).
const CLIMB_TRACKED: f32 = 0.87;

/// A vehicle in the physics world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VehicleId(usize);

/// What the driver does.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehicleControls {
    pub throttle: f32,
    pub steer: f32,
    pub brake: bool,
    /// Nobody at the wheel: it brakes to a stop (gently while it still
    /// rolls fast) and then holds, even on a hill.
    pub parked: bool,
}

struct VehicleBody {
    controller: DynamicRayCastVehicleController,
    mass: f32,
    /// Which wheels steer (the front axle), in wheel order.
    steers: Vec<bool>,
    /// On tracks: turns on the spot instead of steering wheels.
    tracked: bool,
    /// Top speed forwards, m/s.
    top: f32,
    /// Wrecked: nothing drives it any more.
    wrecked: bool,
    /// Held where it is ([`Physics::hold_vehicle`]): its body fixed.
    held: bool,
    /// Its velocity before the last step, m/s: what it hit things with.
    before: Vec3,
    /// A boat's: it floats instead of rolling on wheels.
    hull: Option<Hull>,
}

/// A boat's hull in the water: the points on its waterline that float it,
/// and what its driver asks of it.
struct Hull {
    /// Model space, at its origin's height: over its hull's footprint,
    /// its corners, the middles of its sides and its middle.
    points: Vec<Vec3>,
    controls: VehicleControls,
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

/// Queries that see everything but ragdolls and rubble.
fn solid<'a>() -> r::QueryFilter<'a> {
    r::QueryFilter::default().groups(r::InteractionGroups::new(
        r::Group::ALL,
        r::Group::ALL.difference(RAGDOLL).difference(RUBBLE),
        r::InteractionTestMode::And,
    ))
}

/// Queries that see what a wheel stands on: everything but ragdolls,
/// loose pieces and rubble.
fn ground<'a>() -> r::QueryFilter<'a> {
    r::QueryFilter::default().groups(r::InteractionGroups::new(
        r::Group::ALL,
        r::Group::ALL
            .difference(RAGDOLL)
            .difference(DEBRIS)
            .difference(RUBBLE),
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
    controller: KinematicCharacterController,
    vehicles: Vec<Option<VehicleBody>>,
    ragdolls: Vec<Option<Ragdoll>>,
    /// Loose props' pieces, oldest first.
    debris: Vec<Option<r::RigidBodyHandle>>,
    /// The water's surface boats float on, if there is any.
    water: Option<f32>,
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
/// How a loose piece slows in the air and slides, invented.
const DEBRIS_DAMPING: f32 = 0.2;
const DEBRIS_FRICTION: f32 = 0.8;

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
            keys: HashMap::new(),
            controller,
            vehicles: Vec::new(),
            ragdolls: Vec::new(),
            debris: Vec::new(),
            water: None,
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
        self.keys.insert(handle, key);
    }

    pub fn remove_static(&mut self, key: StaticKey) {
        if let Some(h) = self.statics.remove(&key) {
            self.keys.remove(&h);
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
            v.before = w
                .bodies
                .get(v.controller.chassis)
                .map_or(Vec3::ZERO, |b| vec3(b.linvel()));
            if v.held {
                continue;
            }
            let filter = ground().exclude_rigid_body(v.controller.chassis);
            let queries = w.broad_phase.as_query_pipeline_mut(
                w.narrow_phase.query_dispatcher(),
                &mut w.bodies,
                &mut w.colliders,
                filter,
            );
            v.controller.update_vehicle(dt, queries);
        }
        self.float_boats(dt);
        self.world.step();
        self.hold_ragdolls_together();
        self.settle_debris();
    }

    /// Floats boats on the water ([`set_water`](Self::set_water)) and
    /// drives them as asked: each point of a hull's waterline under the
    /// surface lifts it as deep as it is (the hull's whole weight with its
    /// origin [`FLOAT_DEPTH`] under), and in the water a boat is pushed and
    /// steered, slowed, kept from sliding sideways and stopped rocking.
    fn float_boats(&mut self, dt: f32) {
        let Some(water) = self.water else {
            return;
        };
        for v in self.vehicles.iter().flatten() {
            let Some(hull) = v.hull.as_ref().filter(|_| !v.held) else {
                continue;
            };
            let Some(b) = self.world.bodies.get_mut(v.controller.chassis) else {
                continue;
            };
            let place = *b.position();
            let turn = Quat::from_array(place.rotation.to_array());
            let at = vec3(place.translation);
            let n = hull.points.len().max(1) as f32;
            // Deep under, three times its share of the weight; a wreck's
            // never more than its share of [`WRECK_FLOAT`] of it.
            let most = if v.wrecked { WRECK_FLOAT } else { 3.0 };
            let lift = GRAVITY * v.mass / n * dt;
            // Planing, it floats as if the water stood higher, the more so
            // at the bow.
            let ahead = (turn * Vec3::Z).with_y(0.0).normalize_or_zero();
            let going = vec3(b.linvel()).dot(ahead);
            let plane = if v.wrecked {
                0.0
            } else {
                (going / PLANE_SPEED).clamp(0.0, 1.0)
            };
            let reach = hull
                .points
                .iter()
                .map(|q| q.z.abs())
                .fold(0.0f32, f32::max)
                .max(0.1);
            let mut wet = 0.0;
            for &q in &hull.points {
                let p = at + turn * q;
                let surface = water + plane * (PLANE_RISE + PLANE_TRIM * q.z / reach);
                let depth = surface - p.y;
                if depth > 0.0 {
                    wet += 1.0;
                    let up = lift * (depth / FLOAT_DEPTH).min(most);
                    b.apply_impulse_at_point(vector(Vec3::Y * up), vector(p), true);
                }
            }
            if wet == 0.0 {
                continue;
            }
            let wet = wet / n;
            let ease = |rate: f32| 1.0 - (rate * dt * wet).min(1.0);
            let c = if v.wrecked {
                VehicleControls::default()
            } else {
                hull.controls
            };
            let side = Vec3::new(ahead.z, 0.0, -ahead.x);
            let velocity = vec3(b.linvel());
            let mut along = velocity.dot(ahead);
            let throttle = c.throttle.clamp(-1.0, 1.0);
            let against = throttle * along < -0.5;
            let top = if throttle >= 0.0 { v.top } else { BOAT_REVERSE };
            if throttle != 0.0 && !against && !c.brake && !c.parked {
                let headroom = (1.0 - along.abs() / top).clamp(0.0, 1.0);
                along += throttle * BOAT_PUSH * headroom * dt * wet;
            }
            let slowing = if c.brake || c.parked || against {
                BOAT_BRAKE
            } else if throttle == 0.0 {
                BOAT_COAST
            } else {
                0.0
            };
            along *= ease(slowing);
            let across = velocity.dot(side) * ease(BOAT_KEEL);
            let rise = velocity.y * ease(BOAT_HEAVE);
            b.set_linvel(vector(ahead * along + side * across + Vec3::Y * rise), true);
            // The rudder bites as the water goes by (the other way round
            // backing up), a little with the throttle open at a standstill;
            // it turns as tracks do for the same steering.
            let spin = vec3(b.angvel());
            let up = turn * Vec3::Y;
            let bite = if along.abs() < 0.9 {
                0.15 * throttle
            } else {
                (along / BOAT_TURN_SPEED).clamp(-1.0, 1.0)
            };
            let want = c.steer.clamp(-1.0, 1.0) * BOAT_TURN * bite;
            let yaw = spin.dot(up);
            let yaw = yaw + (want - yaw) * (3.0 * dt * wet).min(1.0);
            let rock = (spin - up * spin.dot(up)) * ease(BOAT_ROCK);
            b.set_angvel(vector(rock + up * yaw), true);
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
        self.add_piece(
            piece,
            at,
            velocity,
            spin,
            r::InteractionGroups::new(
                DEBRIS,
                r::Group::ALL.difference(DEBRIS).difference(VEHICLE),
                r::InteractionTestMode::And,
            ),
        )
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
                .linear_damping(DEBRIS_DAMPING)
                .angular_damping(DEBRIS_DAMPING)
                .ccd_enabled(true),
        );
        let collider = r::ColliderBuilder::new(piece.shape.clone())
            .mass(piece.mass)
            .friction(DEBRIS_FRICTION)
            .collision_groups(groups);
        self.world.insert_collider(collider, Some(body));
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

    pub fn remove_debris(&mut self, id: DebrisId) {
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
        let half = Vec3::new(
            (hi.x - lo.x) * 0.5,
            (hi.y - floor) * 0.5,
            (hi.z - lo.z) * 0.5,
        )
        .max(Vec3::splat(0.1));
        let centre = Vec3::new((lo.x + hi.x) * 0.5, floor + half.y, (lo.z + hi.z) * 0.5);
        let body = self.world.insert_body(
            r::RigidBodyBuilder::dynamic()
                .pose(pose(position, rotation))
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
        let hull = r::InteractionGroups::new(VEHICLE, r::Group::ALL, r::InteractionTestMode::And);
        let collider = r::ColliderBuilder::cuboid(half.x, half.y, half.z)
            .translation(vector(centre))
            .mass_properties(r::MassProperties::new(
                vector(Vec3::new(0.0, -half.y * 0.8, 0.0)),
                m,
                vector(inertia),
            ))
            .friction(0.6)
            .collision_groups(hull);
        self.world.insert_collider(collider, Some(body));
        // The turret on top, adding nothing to the mass the hull carries.
        if let Some((lo, hi)) = spec.turret_box {
            let half = ((hi - lo) * 0.5).max(Vec3::splat(0.1));
            let turret = r::ColliderBuilder::cuboid(half.x, half.y, half.z)
                .translation(vector((lo + hi) * 0.5))
                .density(0.0)
                .friction(0.6)
                .collision_groups(hull);
            self.world.insert_collider(turret, Some(body));
        }

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
            Hull {
                points,
                controls: VehicleControls::default(),
            }
        });
        let top = if boat { BOAT_TOP } else { TOP_SPEED };
        let id = VehicleId(self.vehicles.len());
        self.vehicles.push(Some(VehicleBody {
            controller,
            mass: spec.mass,
            steers: spec.wheels.iter().map(|w| w.steers).collect(),
            tracked: spec.tracked,
            top: spec.tuning.max_speed.unwrap_or(top),
            wrecked: false,
            held: false,
            before: Vec3::ZERO,
            hull,
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
        let Some(v) = self
            .vehicles
            .get_mut(id.0)
            .and_then(Option::as_mut)
            .filter(|v| !v.wrecked)
        else {
            return;
        };
        // A boat is driven as it floats (`float_boats`).
        if let Some(hull) = &mut v.hull {
            hull.controls = controls;
            return;
        }
        let speed = v.controller.current_vehicle_speed;
        let wheels = v.controller.wheels().len().max(1) as f32;
        // How steeply its nose points up (the sine).
        let nose = self
            .world
            .bodies
            .get(v.controller.chassis)
            .map_or(0.0, |b| (b.position().rotation * r::Vector::Z).y);
        // A wheel's brake is the most impulse it may take off per step:
        // this many m/s² over the whole vehicle.
        let slowing = |decel: f32| v.mass * decel * dt / wheels;
        let throttle = controls.throttle.clamp(-1.0, 1.0);
        let against = speed.abs() > 0.5 && throttle * speed < 0.0;
        // Accelerates at about 4 m/s² flat out, slower near top speed;
        // backs up at half its top speed, at most 8 m/s.
        let top = if throttle >= 0.0 {
            v.top
        } else {
            (v.top * 0.5).min(REVERSE_SPEED)
        };
        let headroom = (1.0 - speed.abs() / top).clamp(0.0, 1.0);
        // Uphill (the way it is driven), the pull that holds it as on the
        // flat, up to its steepest hill (tracks hold it by their grip).
        let most = if v.tracked { 0.0 } else { CLIMB };
        let uphill = (nose * throttle.signum()).clamp(0.0, most.sin());
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
            (0.0, slowing(SERVICE_BRAKE * throttle.abs()))
        } else if throttle != 0.0 {
            let pull = 4.0 * headroom + GRAVITY * uphill;
            (throttle * v.mass * pull / wheels, 0.0)
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
        let gripping = v
            .controller
            .wheels()
            .iter()
            .filter(|w| w.raycast_info().is_in_contact)
            .count()
            >= 2;
        let mass = v.mass;
        // Tracks turn the hull on the spot (one track forward, the other
        // back), so the yaw is set directly; a little slower on the move.
        if let Some((chassis, turn)) = tracked
            && let Some(body) = self.world.bodies.get_mut(chassis)
        {
            // Their grip holds its weight on the hill (along the ground,
            // the way its hull lies).
            if gripping {
                let up = body.position().rotation * r::Vector::Y;
                let down = r::Vector::new(0.0, -GRAVITY, 0.0);
                let along = down - up * down.dot(up);
                let steep = along.length() / GRAVITY;
                let held = (CLIMB_TRACKED.sin() / steep.max(1e-3)).min(1.0);
                body.apply_impulse(-along * held * mass * dt, true);
            }
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
            .is_some_and(|v| v.controller.wheels().iter().any(|w| w.engine_force != 0.0))
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

    /// Makes a vehicle a wreck: burst tyres and bent axles no longer carry
    /// it, so it drops onto its rims and belly and scrapes to a stop
    /// instead of rolling on.
    pub fn wreck_vehicle(&mut self, id: VehicleId) {
        let Some(v) = self.vehicles.get_mut(id.0).and_then(Option::as_mut) else {
            return;
        };
        v.wrecked = true;
        // How low the rims reach, chassis space.
        let mut rims = f32::INFINITY;
        for w in v.controller.wheels_mut() {
            let centre = w.chassis_connection_point_cs.y - w.suspension_rest_length;
            rims = rims.min(centre - w.radius * WRECK_RIM);
            w.max_suspension_force = 0.0;
            w.friction_slip = 0.0;
            w.engine_force = 0.0;
            w.brake = 0.0;
        }
        let Some(body) = self.world.bodies.get_mut(v.controller.chassis) else {
            return;
        };
        body.set_linear_damping(WRECK_DAMPING);
        body.set_angular_damping(WRECK_DAMPING);
        // The hull (the first collider; a turret's comes after), which
        // stopped at the wheels' centres, reaches down to the rims and
        // drags on the ground.
        for &h in body.colliders().iter().take(1) {
            let Some(c) = self.world.colliders.get_mut(h) else {
                continue;
            };
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
    /// shots from its own gun.
    pub fn cast_ray_past(
        &self,
        origin: Vec3,
        direction: Vec3,
        max: f32,
        skip: VehicleId,
    ) -> Option<(f32, Vec3)> {
        let ray = r::Ray::new(vector(origin), vector(direction.normalize_or_zero()));
        let mut filter = solid();
        if let Some(v) = self.vehicles.get(skip.0).and_then(Option::as_ref) {
            filter = filter.exclude_rigid_body(v.controller.chassis);
        }
        self.world
            .query_pipeline_with_filter(filter)
            .cast_ray_and_get_normal(&ray, max, true)
            .map(|(_, hit)| (hit.time_of_impact, vec3(hit.normal)))
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
            muzzle: None,
            gun: Vec::new(),
            gun_pivot: None,
            muzzle_on_gun: false,
            elevation: (0.0, 0.0),
            hull: (Vec3::new(-1.0, 0.0, -2.2), Vec3::new(1.0, 1.5, 2.2)),
            turret_box: None,
            tuning: crate::VehicleTuning::default(),
        }
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
        let id = p.add_vehicle(&boat(), Vec3::new(0.0, water + 1.5, -200.0), 0.0);
        let place = |p: &Physics| p.vehicle_transform(id).unwrap();
        for _ in 0..600 {
            p.drive(id, VehicleControls::default());
            p.step(DT);
        }
        // Afloat, level, its origin about as deep as the layers put boats.
        let m = place(&p);
        let y = m.w_axis.y;
        assert!((y - (water - FLOAT_DEPTH)).abs() < 0.15, "floats at {y}");
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
        let speed = p.vehicle_speed(id);
        assert!((10.0..=BOAT_TOP + 0.5).contains(&speed), "{speed} m/s");
        let m = place(&p);
        assert!(
            m.w_axis.z > -170.0 && m.w_axis.x.abs() < 1.0,
            "at {}",
            m.w_axis
        );
        // Planing: riding higher than at rest, bow up.
        assert!(
            m.w_axis.y > water - FLOAT_DEPTH + 0.1 && m.w_axis.y < water + 0.3,
            "planes at {}",
            m.w_axis.y
        );
        let bow = m.transform_vector3(Vec3::Z).y;
        assert!((0.02..0.2).contains(&bow), "bow up {bow}");
        // Steering turns it as steering a tank does: positive steer, a
        // positive turn about up.
        let before = m.transform_vector3(Vec3::Z);
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
        }
        let after = place(&p).transform_vector3(Vec3::Z);
        let turned = after.x.atan2(after.z) - before.x.atan2(before.z);
        assert!(turned > 0.8, "turned {turned}");
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
        for _ in 0..60 * 40 {
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
        // The beach comes out of the water at z 241.
        let m = place(&p);
        assert!(p.vehicle_speed(id).abs() < 2.0, "{}", p.vehicle_speed(id));
        assert!(
            (215.0..250.0).contains(&m.w_axis.z),
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
        let spec = car();
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
        // It crawled at 2 m/s before the engine pulled harder uphill.
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
