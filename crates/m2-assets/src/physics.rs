//! Collision: the `PHY2` section of model assets.
//!
//! About half the models (1,564 of 3,007 in `vz.wad`) carry a `PHY2`
//! section: a 0x30-byte header, a Havok 5.5 packfile (see
//! [`havok`](crate::havok)) holding the collision shapes, then records
//! that attach those shapes to the model's nodes and rigid bodies.
//!
//! ```text
//! header, 0x30 bytes:
//! 0x00  u32      version (0x39)
//! 0x04  u32      the model's name hash
//! 0x08  u32      part records
//! 0x0C  u32      body records
//! 0x10  u32      shape records (= the shapes in the packfile's WpArray)
//! 0x14  u32 x 3  unknown
//! 0x20  u32      packfile length
//! 0x24  u32 x 3  unknown (0)
//!
//! packfile: contents object `WpArray` {vtable, refcount, hkArray<shape *>}
//!   hkpBoxShape             +0x10 f32 radius, +0x20 f32x3 half extents
//!   hkpSphereShape          +0x10 f32 radius
//!   hkpCapsuleShape         +0x10 f32 radius, +0x20 f32x3 a, +0x30 f32x3 b
//!   hkpConvexVerticesShape  +0x10 f32 radius, +0x20 f32x3 AABB half extents,
//!                           +0x30 f32x3 AABB centre, +0x40 hkArray of blocks
//!                           of four transposed points {x[4], y[4], z[4]},
//!                           +0x4C u32 vertex count, +0x50 hkArray<f32x4>
//!                           planes (n, d)
//!   hkpMoppBvTreeShape      +0x10 hkpMoppCode *, +0x34 child shape *
//!   WpMeshShape16           +0x18 f32 radius, +0x1C hkArray of 0x30-byte
//!                           subparts {f32x4 origin, f32x4 scale,
//!                           hkArray<u16 a, b, c, flags> triangles,
//!                           hkArray<u16> unknown (vertex subset)}
//!
//! then, offsets from the start of PHY2, 0xFFFFFFFF for none:
//! part, 0x44 bytes: u32 0xAAAAAAAA, f32x3 position, f32x4 rotation xyzw,
//!       u32 node name hash, u32 node index, u32 body, u32 shape, u32 ?,
//!       u32 parent, u32 first child, u32 next sibling, u32 0xBBBBBBBB
//! body, 0x6C bytes: u32 0xCCCCCCCC, f32x9 inertia (all FLT_MAX: fixed),
//!       f32x3 centre of mass, f32 mass, f32x4 unknown (0.3), u32x8 ?,
//!       u32 0xDDDDDDDD
//! shape, 0x6C bytes: u32 0xEEEEEEEE, then 26 words: 3 kind (2 mesh),
//!       4 collision class (see [`Shape::class`]), 5..10 f32x6 vertex pool bounds
//!       (meshes), 16 vertex count, 18 vertex pool offset (meshes),
//!       24 shape index, 25 0xFFFFFFFF
//! vertex pool: u16x3 per vertex, quantised; a vertex is
//!       origin + q * scale with its subpart's origin and scale
//! ```
//!
//! Mesh triangles index the vertex pool directly; several mesh shapes of
//! one model share a pool.
//!
//! A shape is placed in the space of its rigid body's node: the body is
//! the one on the shape's part or the nearest ancestor part, and its node
//! is the node of the part that holds that body. Fixed bodies sit on the
//! root, so most shapes are in model space.
//!
//! # Provenance
//!
//! Format-from-samples: inferred from the 1,564 `PHY2` sections in the
//! models of a legally installed copy. Every packfile parses with its
//! fixups in bounds, the record counts match the header, every link and
//! index resolves, every quantised vertex lands inside the pool bounds,
//! and 15,719 of 15,720 convex shapes on moved nodes fall inside the
//! model's bounds when placed by the rule above. No SDK code or headers
//! were used and no bytes from the archives are committed.

use crate::bytes::{OutOfBounds, Reader};
use crate::havok::{Packfile, PackfileError};

/// Version word of every `PHY2` header seen.
pub const VERSION: u32 = 0x39;

const HEADER_LEN: usize = 0x30;
const PART_LEN: usize = 0x44;
const BODY_LEN: usize = 0x6C;
const SHAPE_LEN: usize = 0x6C;
const NONE: u32 = 0xFFFF_FFFF;
/// Mesh shapes keep at most this many vertices.
const MAX_VERTICES: u32 = 1 << 20;

/// Errors from parsing a `PHY2` section.
#[derive(Debug, Clone, thiserror::Error, PartialEq)]
pub enum PhysicsError {
    #[error("truncated: needed {len} bytes at {offset:#x}")]
    Truncated { offset: usize, len: usize },
    #[error("unsupported version {0:#x}")]
    Version(u32),
    #[error("packfile: {0}")]
    Packfile(#[from] PackfileError),
    #[error("expected record marker {expected:08x} at {offset:#x}")]
    Marker { offset: usize, expected: u32 },
    #[error("link {value:#x} at {offset:#x} is not a {what} record")]
    Link {
        offset: usize,
        value: u32,
        what: &'static str,
    },
    #[error("packfile object: {0}")]
    Object(&'static str),
    #[error("shape {shape}: {what}")]
    Shape { shape: usize, what: &'static str },
    #[error("shape {shape}: unhandled class {class}")]
    Class { shape: usize, class: String },
}

impl From<OutOfBounds> for PhysicsError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// Attaches a body or a shape to a model node, in a tree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Part {
    /// Where the part sits (rotation as x, y, z, w): a body's part at its
    /// node, in model space; a shape's part in its body's node space
    /// ([`Physics::shape_place`]). Identity on most parts.
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub node_hash: u32,
    pub node: u32,
    pub body: Option<usize>,
    pub shape: Option<usize>,
    pub unk_link: u32,
    pub parent: Option<usize>,
    pub first_child: Option<usize>,
    pub next_sibling: Option<usize>,
}

/// Rigid body properties.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Body {
    pub inertia: [f32; 9],
    pub centre_of_mass: [f32; 3],
    pub mass: f32,
    pub unk: [f32; 4],
    pub unk_words: [u32; 8],
}

impl Body {
    /// Fixed bodies have infinite inertia and never move.
    pub fn is_fixed(&self) -> bool {
        self.inertia[0] == f32::MAX
    }
}

/// A shape's geometry, in its body node's space.
#[derive(Debug, Clone, PartialEq)]
pub enum Geometry {
    Box {
        half_extents: [f32; 3],
    },
    Sphere,
    Capsule {
        a: [f32; 3],
        b: [f32; 3],
    },
    Convex {
        vertices: Vec<[f32; 3]>,
        /// Outward normal and offset: `n · v + d <= 0` inside (the planes
        /// sit `radius` outside the vertices).
        planes: Vec<[f32; 4]>,
    },
    Mesh {
        vertices: Vec<[f32; 3]>,
        triangles: Vec<[u32; 3]>,
    },
}

/// One collision shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    /// Havok's convex radius: a shell this thick around the geometry.
    pub radius: f32,
    pub geometry: Geometry,
    /// The part that holds this shape.
    pub part: usize,
    /// The shape record's words, after the marker.
    pub record: [u32; 26],
}

/// The collision class of plants' and bushes' leafy hulls
/// ([`Shape::class`]). Over the 18,031 shapes of the island's 2,471
/// placed models with `PHY2`, 31 have it (record word 4 `0x28`), 19 of
/// them on models named as plants or bushes and none on a tree, rock,
/// wall or sign; 16,253 have class 7 (`0x27`), the rest 0, 3 or 5.
pub const FOLIAGE: u32 = 8;

impl Shape {
    /// The low five bits of record word 4: the shape's collision class,
    /// where Havok's group filter keeps a layer (unconfirmed). Nearly
    /// every shape is class 7; leaves are [`FOLIAGE`].
    pub fn class(&self) -> u32 {
        self.record[4] & 0x1f
    }

    /// A plant's or bush's leaves ([`FOLIAGE`]).
    pub fn is_foliage(&self) -> bool {
        self.class() == FOLIAGE
    }
}

/// A parsed `PHY2` section.
#[derive(Debug, Clone, PartialEq)]
pub struct Physics {
    pub version: u32,
    pub model: u32,
    pub unk_0x14: [u32; 3],
    pub parts: Vec<Part>,
    pub bodies: Vec<Body>,
    pub shapes: Vec<Shape>,
}

fn f32s<const N: usize>(r: &mut Reader) -> Result<[f32; N], OutOfBounds> {
    let mut out = [0.0; N];
    for v in &mut out {
        *v = f32::from_bits(r.u32()?);
    }
    Ok(out)
}

fn marker(data: &[u8], offset: usize, expected: u32) -> Result<(), PhysicsError> {
    if Reader::at(data, offset).u32()? != expected {
        return Err(PhysicsError::Marker { offset, expected });
    }
    Ok(())
}

/// A pointer field: a local fixup in the same section or a global one.
fn pointer(p: &Packfile, section: u32, field: u32) -> Result<Option<(u32, u32)>, PhysicsError> {
    let s = p.section(section)?;
    Ok(s.local(field)
        .map(|t| (section, t))
        .or_else(|| s.global(field)))
}

/// An `hkArray` at `field`: (section, data offset, count).
fn array(
    p: &Packfile,
    section: u32,
    field: u32,
    elem: usize,
) -> Result<(u32, usize, usize), PhysicsError> {
    let data = p.section(section)?.data;
    let n = Reader::at(data, field as usize + 4).u32()? as usize;
    if n == 0 {
        return Ok((section, 0, 0));
    }
    let (s, at) = pointer(p, section, field)?.ok_or(PhysicsError::Object("array without data"))?;
    let len = n
        .checked_mul(elem)
        .ok_or(PhysicsError::Object("array too long"))?;
    Reader::at(p.section(s)?.data, at as usize).take(len)?;
    Ok((s, at as usize, n))
}

impl Physics {
    /// Parses a model's `PHY2` section.
    pub fn parse(phy2: &[u8]) -> Result<Self, PhysicsError> {
        let mut r = Reader::at(phy2, 0);
        let mut hdr = [0u32; 12];
        for w in &mut hdr {
            *w = r.u32()?;
        }
        if hdr[0] != VERSION {
            return Err(PhysicsError::Version(hdr[0]));
        }
        let (nparts, nbodies, nshapes) = (hdr[2] as usize, hdr[3] as usize, hdr[4] as usize);
        let pack_len = hdr[8] as usize;
        let pack = Reader::at(phy2, HEADER_LEN).take(pack_len)?;
        let p = Packfile::parse(pack)?;

        // Records, in order, after the packfile.
        let parts_at = HEADER_LEN + pack_len;
        let bodies_at = parts_at + nparts.saturating_mul(PART_LEN);
        let shapes_at = bodies_at + nbodies.saturating_mul(BODY_LEN);
        Reader::at(phy2, parts_at).take(
            (shapes_at - parts_at)
                .checked_add(nshapes.saturating_mul(SHAPE_LEN))
                .ok_or(PhysicsError::Truncated {
                    offset: parts_at,
                    len: usize::MAX,
                })?,
        )?;
        let link = |offset: usize, base: usize, len: usize, n: usize, what| {
            let value = Reader::at(phy2, offset).u32()?;
            if value == NONE {
                return Ok(None);
            }
            let v = value as usize;
            if v < base || !(v - base).is_multiple_of(len) || (v - base) / len >= n {
                return Err(PhysicsError::Link {
                    offset,
                    value,
                    what,
                });
            }
            Ok(Some((v - base) / len))
        };

        let mut parts = Vec::with_capacity(nparts);
        for i in 0..nparts {
            let at = parts_at + i * PART_LEN;
            marker(phy2, at, 0xAAAA_AAAA)?;
            marker(phy2, at + PART_LEN - 4, 0xBBBB_BBBB)?;
            let mut r = Reader::at(phy2, at + 4);
            let position = f32s::<3>(&mut r)?;
            let rotation = f32s::<4>(&mut r)?;
            let (node_hash, node) = (r.u32()?, r.u32()?);
            let l = at + 0x28;
            parts.push(Part {
                position,
                rotation,
                node_hash,
                node,
                body: link(l, bodies_at, BODY_LEN, nbodies, "body")?,
                shape: link(l + 4, shapes_at, SHAPE_LEN, nshapes, "shape")?,
                unk_link: Reader::at(phy2, l + 8).u32()?,
                parent: link(l + 12, parts_at, PART_LEN, nparts, "part")?,
                first_child: link(l + 16, parts_at, PART_LEN, nparts, "part")?,
                next_sibling: link(l + 20, parts_at, PART_LEN, nparts, "part")?,
            });
        }

        let mut bodies = Vec::with_capacity(nbodies);
        for i in 0..nbodies {
            let at = bodies_at + i * BODY_LEN;
            marker(phy2, at, 0xCCCC_CCCC)?;
            marker(phy2, at + BODY_LEN - 4, 0xDDDD_DDDD)?;
            let mut r = Reader::at(phy2, at + 4);
            let inertia = f32s::<9>(&mut r)?;
            let centre_of_mass = f32s::<3>(&mut r)?;
            let [mass] = f32s::<1>(&mut r)?;
            let unk = f32s::<4>(&mut r)?;
            let mut unk_words = [0; 8];
            for w in &mut unk_words {
                *w = r.u32()?;
            }
            bodies.push(Body {
                inertia,
                centre_of_mass,
                mass,
                unk,
                unk_words,
            });
        }

        // The packfile's shapes.
        let cs = p.header.contents_section;
        let co = p.header.contents_offset;
        if p.class_of(cs, co)? != Some("WpArray") {
            return Err(PhysicsError::Object("contents is not a WpArray"));
        }
        let (asec, aat, n) = array(&p, cs, co + 8, 4)?;
        if n != nshapes {
            return Err(PhysicsError::Object(
                "WpArray length differs from the header",
            ));
        }

        let mut shapes = Vec::with_capacity(nshapes);
        for k in 0..nshapes {
            let at = shapes_at + k * SHAPE_LEN;
            marker(phy2, at, 0xEEEE_EEEE)?;
            let mut r = Reader::at(phy2, at + 4);
            let mut record = [0; 26];
            for w in &mut record {
                *w = r.u32()?;
            }
            if record[24] as usize != k || record[25] != NONE {
                return Err(PhysicsError::Shape {
                    shape: k,
                    what: "record out of order",
                });
            }
            let part =
                parts
                    .iter()
                    .position(|p| p.shape == Some(k))
                    .ok_or(PhysicsError::Shape {
                        shape: k,
                        what: "no part holds it",
                    })?;
            let bad = |what| PhysicsError::Shape { shape: k, what };
            let (ss, so) =
                pointer(&p, asec, (aat + 4 * k) as u32)?.ok_or(bad("null shape pointer"))?;
            let class = p.class_of(ss, so)?.ok_or(bad("shape has no class"))?;
            let data = p.section(ss)?.data;
            let o = so as usize;
            let vec3 = |off: usize| -> Result<[f32; 3], OutOfBounds> {
                f32s::<3>(&mut Reader::at(data, o + off))
            };
            let radius = f32::from_bits(Reader::at(data, o + 0x10).u32()?);
            let (radius, geometry) = match class {
                "hkpBoxShape" => (
                    radius,
                    Geometry::Box {
                        half_extents: vec3(0x20)?,
                    },
                ),
                "hkpSphereShape" => (radius, Geometry::Sphere),
                "hkpCapsuleShape" => (
                    radius,
                    Geometry::Capsule {
                        a: vec3(0x20)?,
                        b: vec3(0x30)?,
                    },
                ),
                "hkpConvexVerticesShape" => {
                    let (vs, va, blocks) = array(&p, ss, so + 0x40, 48)?;
                    let count = Reader::at(data, o + 0x4C).u32()? as usize;
                    if count > blocks * 4 {
                        return Err(bad("more vertices than blocks hold"));
                    }
                    let vdata = p.section(vs)?.data;
                    let mut vertices = Vec::with_capacity(count);
                    for i in 0..count {
                        let b = va + 48 * (i / 4) + 4 * (i % 4);
                        let c = |axis: usize| -> Result<f32, OutOfBounds> {
                            Ok(f32::from_bits(Reader::at(vdata, b + 16 * axis).u32()?))
                        };
                        vertices.push([c(0)?, c(1)?, c(2)?]);
                    }
                    let (ps, pa, np) = array(&p, ss, so + 0x50, 16)?;
                    let mut r = Reader::at(p.section(ps)?.data, pa);
                    let planes = (0..np)
                        .map(|_| f32s::<4>(&mut r))
                        .collect::<Result<_, _>>()?;
                    (radius, Geometry::Convex { vertices, planes })
                }
                "hkpMoppBvTreeShape" => {
                    let (ms, mo) =
                        pointer(&p, ss, so + 0x34)?.ok_or(bad("MOPP without a child"))?;
                    if p.class_of(ms, mo)? != Some("WpMeshShape16") {
                        return Err(bad("MOPP child is not a WpMeshShape16"));
                    }
                    let mdata = p.section(ms)?.data;
                    let radius = f32::from_bits(Reader::at(mdata, mo as usize + 0x18).u32()?);
                    let (pool_n, pool_at) = (record[16], record[18] as usize);
                    if pool_n > MAX_VERTICES {
                        return Err(bad("vertex pool too large"));
                    }
                    let pool = Reader::at(phy2, pool_at).take(6 * pool_n as usize)?;
                    let (subs, sub_at, nsub) = array(&p, ms, mo + 0x1C, 0x30)?;
                    let sdata = p.section(subs)?.data;
                    let mut vertices = Vec::new();
                    let mut triangles = Vec::new();
                    for j in 0..nsub {
                        let sp = sub_at + 0x30 * j;
                        let mut r = Reader::at(sdata, sp);
                        let origin = f32s::<4>(&mut r)?;
                        let scale = f32s::<4>(&mut r)?;
                        let (ts, ta, nt) = array(&p, subs, sp as u32 + 0x20, 8)?;
                        // Compact the pool to the vertices this subpart uses.
                        let mut remap = std::collections::HashMap::new();
                        let mut r = Reader::at(p.section(ts)?.data, ta);
                        for _ in 0..nt {
                            let t = [r.u16()?, r.u16()?, r.u16()?];
                            r.u16()?;
                            let mut tri = [0u32; 3];
                            for (out, &q) in tri.iter_mut().zip(&t) {
                                if u32::from(q) >= pool_n {
                                    return Err(bad("triangle index past the vertex pool"));
                                }
                                *out = *remap.entry(q).or_insert_with(|| {
                                    let v = &pool[6 * usize::from(q)..6 * usize::from(q) + 6];
                                    let c = |a: usize| {
                                        origin[a]
                                            + f32::from(u16::from_le_bytes([
                                                v[2 * a],
                                                v[2 * a + 1],
                                            ])) * scale[a]
                                    };
                                    vertices.push([c(0), c(1), c(2)]);
                                    (vertices.len() - 1) as u32
                                });
                            }
                            triangles.push(tri);
                        }
                    }
                    (
                        radius,
                        Geometry::Mesh {
                            vertices,
                            triangles,
                        },
                    )
                }
                other => {
                    return Err(PhysicsError::Class {
                        shape: k,
                        class: other.to_owned(),
                    });
                }
            };
            shapes.push(Shape {
                radius,
                geometry,
                part,
                record,
            });
        }
        Ok(Self {
            version: hdr[0],
            model: hdr[1],
            unk_0x14: [hdr[5], hdr[6], hdr[7]],
            parts,
            bodies,
            shapes,
        })
    }

    /// The body that moves shape `shape`: on its part or the nearest
    /// ancestor.
    pub fn body_of(&self, shape: usize) -> Option<usize> {
        let mut at = Some(self.shapes.get(shape)?.part);
        // Bounded: a malformed tree could loop.
        for _ in 0..=self.parts.len() {
            let part = self.parts.get(at?)?;
            if part.body.is_some() {
                return part.body;
            }
            at = part.parent;
        }
        None
    }

    /// Where shape `shape` sits in its body node's space
    /// ([`shape_node`](Self::shape_node)): its own part's place, position
    /// and rotation (x, y, z, w). A gantry's sign panel hangs 14 m up on a
    /// part of its own; across the archive's models this puts 720 of 724
    /// boxes inside their model's bounds, against 698 by the body node
    /// alone (convex shapes and meshes fit either way).
    pub fn shape_place(&self, shape: usize) -> Option<([f32; 3], [f32; 4])> {
        let part = self.parts.get(self.shapes.get(shape)?.part)?;
        Some((part.position, part.rotation))
    }

    /// The model node whose space shape `shape` is in.
    pub fn shape_node(&self, shape: usize) -> Option<u32> {
        let body = self.body_of(shape)?;
        self.parts
            .iter()
            .find(|p| p.body == Some(body))
            .map(|p| p.node)
    }
}

impl Geometry {
    /// A triangle mesh of the shape, without the convex radius: positions
    /// and triangles wound counter-clockwise seen from outside (right-handed
    /// axes) for the closed shapes, as stored for meshes. Spheres and
    /// capsules are approximated.
    pub fn triangles(&self, radius: f32) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
        match self {
            Self::Box { half_extents: h } => {
                let v: Vec<[f32; 3]> = (0..8)
                    .map(|i| {
                        let s = |bit: usize, e: f32| if i >> bit & 1 == 1 { e } else { -e };
                        [s(0, h[0]), s(1, h[1]), s(2, h[2])]
                    })
                    .collect();
                let planes = [
                    [1.0, 0.0, 0.0, -h[0]],
                    [-1.0, 0.0, 0.0, -h[0]],
                    [0.0, 1.0, 0.0, -h[1]],
                    [0.0, -1.0, 0.0, -h[1]],
                    [0.0, 0.0, 1.0, -h[2]],
                    [0.0, 0.0, -1.0, -h[2]],
                ];
                (v.clone(), hull_faces(&v, &planes))
            }
            Self::Convex { vertices, planes } => (vertices.clone(), hull_faces(vertices, planes)),
            Self::Mesh {
                vertices,
                triangles,
            } => (vertices.clone(), triangles.clone()),
            Self::Sphere => capsule([0.0; 3], [0.0; 3], radius),
            Self::Capsule { a, b } => capsule(*a, *b, radius),
        }
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalise(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    if l > 0.0 {
        [a[0] / l, a[1] / l, a[2] / l]
    } else {
        a
    }
}

/// Faces of a convex hull: for each plane, the vertices that reach it,
/// sorted around the plane's normal and fanned.
fn hull_faces(vertices: &[[f32; 3]], planes: &[[f32; 4]]) -> Vec<[u32; 3]> {
    let extent = vertices
        .iter()
        .flatten()
        .fold(0.0f32, |m, &c| m.max(c.abs()));
    let eps = 1e-3 * extent.max(1.0);
    let mut tris = Vec::new();
    for pl in planes {
        let n = normalise([pl[0], pl[1], pl[2]]);
        if !n.iter().all(|c| c.is_finite()) || dot(n, n) == 0.0 {
            continue;
        }
        let top = vertices
            .iter()
            .map(|&v| dot(n, v))
            .fold(f32::NEG_INFINITY, f32::max);
        let face: Vec<u32> = (0..vertices.len() as u32)
            .filter(|&i| dot(n, vertices[i as usize]) >= top - eps)
            .collect();
        if face.len() < 3 {
            continue;
        }
        let mut c = [0.0; 3];
        for &i in &face {
            for a in 0..3 {
                c[a] += vertices[i as usize][a] / face.len() as f32;
            }
        }
        // An in-plane basis (u, w) with u × w = n.
        let helper = if n[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let u = normalise(cross(helper, n));
        let w = cross(n, u);
        let mut sorted: Vec<(f32, u32)> = face
            .iter()
            .map(|&i| {
                let d = sub(vertices[i as usize], c);
                (dot(d, w).atan2(dot(d, u)), i)
            })
            .collect();
        sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
        for k in 1..sorted.len() - 1 {
            tris.push([sorted[0].1, sorted[k].1, sorted[k + 1].1]);
        }
    }
    tris
}

/// A low-polygon capsule between `a` and `b` (a sphere when they meet).
fn capsule(a: [f32; 3], b: [f32; 3], r: f32) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
    const SIDES: u32 = 8;
    const RINGS: u32 = 2; // per hemisphere, plus the pole
    let axis = sub(b, a);
    let up = if dot(axis, axis) > 0.0 {
        normalise(axis)
    } else {
        [0.0, 1.0, 0.0]
    };
    let helper = if up[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let u = normalise(cross(helper, up));
    let w = cross(up, u);
    let mut v = Vec::new();
    // Rings from the bottom pole of `a` to the top pole of `b`.
    let mut rings: Vec<([f32; 3], f32, f32)> = Vec::new(); // centre, height, ring radius
    for k in 0..=RINGS {
        let t = std::f32::consts::FRAC_PI_2 * k as f32 / RINGS as f32;
        rings.push((a, -r * t.cos(), r * t.sin()));
    }
    for k in (0..=RINGS).rev() {
        let t = std::f32::consts::FRAC_PI_2 * k as f32 / RINGS as f32;
        rings.push((b, r * t.cos(), r * t.sin()));
    }
    for &(c, h, rr) in &rings {
        for s in 0..SIDES {
            let t = std::f32::consts::TAU * s as f32 / SIDES as f32;
            let (cs, sn) = (t.cos() * rr, t.sin() * rr);
            v.push([
                c[0] + up[0] * h + u[0] * cs + w[0] * sn,
                c[1] + up[1] * h + u[1] * cs + w[1] * sn,
                c[2] + up[2] * h + u[2] * cs + w[2] * sn,
            ]);
        }
    }
    let mut t = Vec::new();
    for ring in 0..rings.len() as u32 - 1 {
        for s in 0..SIDES {
            let (i0, i1) = (ring * SIDES + s, ring * SIDES + (s + 1) % SIDES);
            let (j0, j1) = (i0 + SIDES, i1 + SIDES);
            t.push([i0, i1, j1]);
            t.push([i0, j1, j0]);
        }
    }
    (v, t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::havok::tests::PackBuilder;

    fn le(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }
    fn lef(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    /// A packfile with a box, a convex cube and a mesh, and its PHY2 tail.
    fn section() -> Vec<u8> {
        let mut b = PackBuilder::new();
        let mut d = vec![0u8; 0x400];
        let put = |d: &mut Vec<u8>, at: usize, bytes: &[u8]| {
            d[at..at + bytes.len()].copy_from_slice(bytes)
        };
        // WpArray at 0: array of 3 pointers at 0x20.
        b.object(0, "WpArray");
        b.contents = 0;
        put(&mut d, 0x0C, &le(&[3, 0x8000_0000]));
        b.local.push((0x08, 0x20));
        b.local.push((0x20, 0x40));
        b.global.push((0x24, 2, 0x80));
        b.local.push((0x28, 0x200));
        // Box at 0x40.
        b.object(0x40, "hkpBoxShape");
        put(&mut d, 0x50, &lef(&[0.05]));
        put(&mut d, 0x60, &lef(&[1.0, 2.0, 3.0]));
        // Convex cube (±1) at 0x80: 8 vertices in 2 blocks at 0xE0, 6 planes at 0x140.
        b.object(0x80, "hkpConvexVerticesShape");
        put(&mut d, 0x90, &lef(&[0.05]));
        put(&mut d, 0xC4, &le(&[2, 0x8000_0000, 8]));
        b.local.push((0xC0, 0xE0));
        put(&mut d, 0xD4, &le(&[6, 0x8000_0000]));
        b.local.push((0xD0, 0x140));
        for blk in 0..2 {
            let ids: Vec<usize> = (4 * blk..4 * blk + 4).collect();
            for axis in 0..3 {
                let vals: Vec<f32> = ids
                    .iter()
                    .map(|&i| if i >> axis & 1 == 1 { 1.0 } else { -1.0 })
                    .collect();
                put(&mut d, 0xE0 + 48 * blk + 16 * axis, &lef(&vals));
            }
        }
        let planes = [
            [1.0, 0.0, 0.0, -1.05],
            [-1.0, 0.0, 0.0, -1.05],
            [0.0, 1.0, 0.0, -1.05],
            [0.0, -1.0, 0.0, -1.05],
            [0.0, 0.0, 1.0, -1.05],
            [0.0, 0.0, -1.0, -1.05],
        ];
        for (i, p) in planes.iter().enumerate() {
            put(&mut d, 0x140 + 16 * i, &lef(p));
        }
        // MOPP at 0x200 with its mesh at 0x240: one subpart at 0x280,
        // two triangles at 0x2C0.
        b.object(0x200, "hkpMoppBvTreeShape");
        b.local.push((0x234, 0x240));
        b.object(0x240, "WpMeshShape16");
        put(&mut d, 0x258, &lef(&[0.01]));
        put(&mut d, 0x260, &le(&[1, 0x8000_0000]));
        b.local.push((0x25C, 0x280));
        put(
            &mut d,
            0x280,
            &lef(&[10.0, 0.0, -5.0, 0.0, 0.5, 0.25, 1.0, 1.0]),
        );
        put(&mut d, 0x2A4, &le(&[2, 0x8000_0000]));
        b.local.push((0x2A0, 0x2C0));
        for (i, t) in [[0u16, 1, 2, 0], [2, 1, 3, 0]].iter().enumerate() {
            put(
                &mut d,
                0x2C0 + 8 * i,
                &t.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>(),
            );
        }
        b.data = d;
        let pack = b.build();

        let mut s = le(&[
            VERSION,
            0x1234_5678,
            3,
            2,
            3,
            0,
            0,
            0,
            pack.len() as u32,
            0,
            0,
            0,
        ]);
        s.extend(&pack);
        let parts_at = s.len() as u32;
        let bodies_at = parts_at + 3 * PART_LEN as u32;
        let shapes_at = bodies_at + 2 * BODY_LEN as u32;
        let pool_at = shapes_at + 3 * SHAPE_LEN as u32;
        let part = |node: u32, body: u32, shape: u32, parent: u32| {
            let mut p = le(&[0xAAAA_AAAA]);
            p.extend(lef(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]));
            p.extend(le(&[
                0xABCD,
                node,
                body,
                shape,
                NONE,
                parent,
                NONE,
                NONE,
                0xBBBB_BBBB,
            ]));
            p
        };
        // Part 0: the root, fixed body 0, the mesh. Part 1: node 2, dynamic
        // body 1, the box. Part 2: under part 1, the convex.
        let shape = |k: u32| shapes_at + k * SHAPE_LEN as u32;
        s.extend(part(0, bodies_at, shape(2), NONE));
        s.extend(part(2, bodies_at + BODY_LEN as u32, shape(0), parts_at));
        s.extend(part(2, NONE, shape(1), parts_at + PART_LEN as u32));
        for fixed in [true, false] {
            let mut b = le(&[0xCCCC_CCCC]);
            let i = if fixed { f32::MAX } else { 1.0 };
            b.extend(lef(&[
                i, 0.0, 0.0, 0.0, i, 0.0, 0.0, 0.0, i, 0.0, 0.0, 0.0, 10.0, 0.3, 0.3, 0.3, 0.3,
            ]));
            b.extend(le(&[0; 8]));
            b.extend(le(&[0xDDDD_DDDD]));
            s.extend(b);
        }
        for k in 0..3u32 {
            let mut w = [0u32; 26];
            if k == 2 {
                w[3] = 2;
                w[16] = 4;
                w[18] = pool_at;
            }
            w[24] = k;
            w[25] = NONE;
            s.extend(le(&[0xEEEE_EEEE]));
            s.extend(le(&w));
        }
        // The mesh's vertex pool.
        for q in [[0u16, 0, 0], [2, 0, 0], [0, 4, 0], [2, 4, 4]] {
            s.extend(q.iter().flat_map(|v| v.to_le_bytes()));
        }
        s
    }

    /// The section and the offset of its first part record.
    fn fixture() -> (Vec<u8>, usize) {
        let s = section();
        let pack_len = u32::from_le_bytes(s[0x20..0x24].try_into().unwrap()) as usize;
        (s, HEADER_LEN + pack_len)
    }

    #[test]
    fn parses_shapes_and_attachments() {
        let (s, _) = fixture();
        let p = Physics::parse(&s).unwrap();
        assert_eq!(
            (p.model, p.parts.len(), p.bodies.len()),
            (0x1234_5678, 3, 2)
        );
        assert!(p.bodies[0].is_fixed() && !p.bodies[1].is_fixed());
        assert_eq!(
            p.shapes[0].geometry,
            Geometry::Box {
                half_extents: [1.0, 2.0, 3.0]
            }
        );
        assert_eq!(p.shapes[0].radius, 0.05);
        let Geometry::Convex { vertices, planes } = &p.shapes[1].geometry else {
            panic!()
        };
        assert_eq!((vertices.len(), planes.len()), (8, 6));
        assert_eq!(vertices[7], [1.0, 1.0, 1.0]);
        let Geometry::Mesh {
            vertices,
            triangles,
        } = &p.shapes[2].geometry
        else {
            panic!()
        };
        assert_eq!(triangles, &[[0, 1, 2], [2, 1, 3]]);
        assert_eq!(vertices[1], [11.0, 0.0, -5.0]);
        assert_eq!(vertices[3], [11.0, 1.0, -1.0]);
        assert_eq!(p.shapes[2].radius, 0.01);
        // The box sits on part 1 (dynamic body 1, node 2); the convex on
        // part 2, whose parent holds body 1; the mesh on the root.
        assert_eq!((p.body_of(0), p.shape_node(0)), (Some(1), Some(2)));
        assert_eq!((p.body_of(1), p.shape_node(1)), (Some(1), Some(2)));
        assert_eq!((p.body_of(2), p.shape_node(2)), (Some(0), Some(0)));
        // Each shape sits where its own part puts it.
        for k in 0..3 {
            let part = &p.parts[p.shapes[k].part];
            assert_eq!(p.shape_place(k), Some((part.position, part.rotation)));
        }
        assert_eq!(p.shape_place(3), None);
    }

    #[test]
    fn rejects_broken_links_and_records() {
        let (s, parts_at) = fixture();
        let mut bad = s.clone();
        bad[parts_at + 0x2C..parts_at + 0x30].copy_from_slice(&NONE.to_le_bytes());
        assert_eq!(
            Physics::parse(&bad).unwrap_err(),
            PhysicsError::Shape {
                shape: 2,
                what: "no part holds it"
            }
        );
        let mut bad = s.clone();
        bad[parts_at + 0x28] ^= 4; // body link off its record
        assert!(matches!(
            Physics::parse(&bad),
            Err(PhysicsError::Link { .. })
        ));
        let mut bad = s.clone();
        bad[parts_at] = 0; // part marker
        assert!(matches!(
            Physics::parse(&bad),
            Err(PhysicsError::Marker { .. })
        ));
        let mut bad = s;
        bad[0] = 0x38;
        assert_eq!(
            Physics::parse(&bad).unwrap_err(),
            PhysicsError::Version(0x38)
        );
    }

    #[test]
    fn triangulates_closed_shapes() {
        let cube = Geometry::Box {
            half_extents: [1.0, 2.0, 3.0],
        };
        let (v, t) = cube.triangles(0.0);
        assert_eq!((v.len(), t.len()), (8, 12));
        // Every face points away from the centre.
        for tri in &t {
            let [a, b, c] = tri.map(|i| v[i as usize]);
            let n = cross(sub(b, a), sub(c, a));
            assert!(dot(n, a) > 0.0, "{tri:?}");
        }
        let (v, t) = Geometry::Sphere.triangles(0.5);
        assert!(!t.is_empty() && v.iter().all(|p| (dot(*p, *p).sqrt() - 0.5).abs() < 1e-4));
    }

    #[test]
    fn byte_flips_never_panic() {
        let (s, _) = fixture();
        for at in 0..s.len() {
            for v in [0, 1, 0x7f, 0xff] {
                let mut b = s.clone();
                b[at] = v;
                if let Ok(p) = Physics::parse(&b) {
                    for (k, sh) in p.shapes.iter().enumerate() {
                        let _ = (sh.geometry.triangles(sh.radius), p.shape_node(k));
                    }
                }
            }
        }
    }
}
