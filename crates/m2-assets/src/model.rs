//! Model assets: node hierarchy, materials, level-of-detail segments and
//! the vertex and index data of every primitive group.
//!
//! A model (asset type hash `5b724250`, after the engine's `PgModel`) is a
//! `UCFX` container with these top-level sections, in this order:
//!
//! ```text
//! INFO  0x48 bytes                      header, below
//! HIER  0xB0 bytes per node             node hierarchy
//! MTRL  variable records                materials
//! TINY  u32 n + u32 * n                (optional) per sub-object, the layer entity it is
//! BSHP  8 bytes per blend shape         (optional) {u32 name hash, u32 0}
//! SEGM  4 bytes per segment             {u16 node, u8 geometry slot, u8 LOD mask}
//! PHY2, STAM/, MIXR/                    (optional) collision, state machine and
//!                                       animation mixer; not decoded here
//! GEOM/ INFO   u32 geometry count
//!       INDX   u16 geometry slot per geometry
//!       MESH/ | SKIN/ | TINY/  one per geometry:
//!             INFO   u32 primitive group count
//!             PRMG/  one per group:
//!                    INFO   group header, below
//!                    STRM/  info {u32 declaration records, u32 stride, u32 vertices}
//!                           decl D3DVERTEXELEMENT9 records (see [`vertex`](crate::vertex))
//!                           data stream 0 vertices
//!                    AREA/  (MESH only) info {u32 n}, data f16 per strip triangle
//!                    IBUF/  info {u32 n}, data u16 indices (a triangle strip)
//!                    PRMT   16 bytes per draw range, below
//!                    BSHP/  (SKIN, one per morph target) info {1, 8, vertices},
//!                           decl (end record only), data f16x4 position deltas
//!                    BSHI   (with morph targets) opaque
//! ```
//!
//! Top-level `INFO`:
//!
//! ```text
//! 0x00  u32      version (0x39)
//! 0x04  f32 x 3  bounding box minimum, model space
//! 0x10  f32 x 3  bounding box maximum
//! 0x1C  u32      flags (0x800 on skinned models, 0x400 on TINY models)
//! 0x20  u32      node count
//! 0x24  u32      material count
//! 0x28  u32      segment count
//! 0x2C  u32      geometry slot count
//! 0x30  u32      blend shape count
//! 0x34  u32      LOD count
//! 0x38  f32      probably a draw or LOD distance
//! 0x3C  f32      unknown (always 5.0)
//! 0x40  u16 x 2  unknown
//! 0x44  u16      LOD mask that needs streamed chunks (approximate)
//! 0x46  u16      LOD mask drawable from the resident asset (approximate)
//! ```
//!
//! `HIER` node:
//!
//! ```text
//! 0x00  u32       name hash (the root's is the asset's name hash)
//! 0x04  u16       flags
//! 0x06  i16 x 3   first child, parent, next sibling (-1 = none)
//! 0x0C  u32       unknown (0)
//! 0x10  f32 x 16  local transform, D3DX layout (row vectors)
//! 0x50  f32 x 16  inverse bind transform, D3DX layout
//! 0x90  f32 x 4   bounding box minimum (w = 1)
//! 0xA0  f32 x 4   bounding box maximum (w = 1)
//! ```
//!
//! `MTRL` record:
//!
//! ```text
//! 0x00  u32       name hash
//! 0x04  f32 x 25  shader constants (layout unknown)
//! 0x68  u16       flags
//! 0x6A  u16       texture count n
//! 0x6C  u32 x n   texture asset name hashes (0 = none)
//!       u32       probably a shader or technique id
//!       u32       unknown (0x959d8470 so far)
//! ```
//!
//! Group `INFO` (0x3C bytes on MESH and TINY, 0x38 on SKIN):
//!
//! ```text
//! 0x00  u32       material range count
//! 0x04  u32       full range count
//! 0x08  u32       morph target count (SKIN)
//! 0x0C  u32       unknown hash, probably a shader id
//! 0x10  u32       unknown hash, follows the vertex format
//! MESH/TINY:
//! 0x14  f32 x 4   bounding sphere centre and radius
//! 0x24  f32 x 6   bounding box minimum and maximum
//! SKIN:
//! 0x14  u32       bone palette range count (up to 8)
//! 0x18  {u16 first node, u16 count} x 8   bone palette as node ranges
//! ```
//!
//! `PRMT` range, one `DrawIndexedPrimitive(TRIANGLESTRIP, 0, min, vertices,
//! start, primitives)` each:
//!
//! ```text
//! 0x00  u32  material index
//! 0x04  u32  first index
//! 0x08  u16  primitive count (strip triangles, degenerates included)
//! 0x0A  u16  minimum vertex index
//! 0x0C  u16  maximum vertex index
//! 0x0E  u16  vertex count
//! ```
//!
//! The material ranges come first and draw the group's index runs back to
//! back with their materials; the full ranges then draw the whole group with
//! no particular material, probably for depth or shadow passes.
//!
//! # Geometry slots and streaming
//!
//! Each segment places one geometry slot at one node for the levels of
//! detail in its mask (bit 0 is the finest); slots can be placed at several
//! nodes. `GEOM/INDX` says which slot each geometry fills. The resident asset
//! (in a block whose path ends in `_P000_Q3`) holds the coarse slots; the
//! finer slots are separate assets with the same name and type hash holding
//! only a `GEOM` container, in the asset's alternative blocks (paths ending
//! `_P001_Q2` and `_P002_Q1`). Add them with [`Model::add_streamed`]. Across
//! all chunks every slot appears exactly once.
//!
//! # Provenance
//!
//! Format-from-samples: inferred from all 3,007 models and 1,400 streamed
//! chunks in the archives of a legally installed copy (57,083 primitive
//! groups), with the counts, lengths, index ranges, bounds and links checked
//! on every one. The vertex declarations are public Direct3D 9 structures.
//! No bytes from the archives are committed.

use m2_math::RawMatrix44;
use m2_math::glam::Mat4;
use m2_render::{BufferUsage, VertexBufferDesc, VertexLayout};
use m2_render::{Geometry as Draw, IndexBufferDesc, IndexBufferHandle, IndexFormat, Topology};

use crate::bytes::{OutOfBounds, Reader};
use crate::ucfx::{Ucfx, UcfxError};
use crate::vertex::{VertexDecl, VertexError, VertexStream};

/// Asset type hash of models.
pub const MODEL_TYPE: u32 = 0x5b72_4250;
/// The only header version seen.
pub const VERSION: u32 = 0x39;
/// Length of the top-level `INFO` section.
pub const INFO_LEN: usize = 0x48;
/// Length of one `HIER` node.
pub const NODE_LEN: usize = 0xB0;
/// Length of one `SEGM` entry.
pub const SEGMENT_LEN: usize = 4;
/// Length of one `PRMT` range.
pub const RANGE_LEN: usize = 16;
/// Length of a MESH or TINY group's `INFO`.
pub const GROUP_INFO_LEN: usize = 0x3C;
/// Length of a SKIN group's `INFO`.
pub const SKIN_GROUP_INFO_LEN: usize = 0x38;
/// Bone palette ranges a SKIN group's `INFO` has room for.
pub const PALETTE_SLOTS: usize = 8;
/// Most geometry slots a model can use; segments name slots with a byte.
pub const MAX_SLOTS: u32 = 256;
/// Floats in a material's constant block.
pub const MATERIAL_CONSTANTS: usize = 25;
/// Bytes per morph target vertex (an f16x4 position delta).
pub const MORPH_STRIDE: u32 = 8;

const MATERIAL_FIXED_LEN: usize = 0x74;
const BLEND_SHAPE_LEN: usize = 8;

/// A section path, for errors.
pub type SectionName = [u8; 4];

/// Errors from parsing a model.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ModelError {
    #[error("container: {0}")]
    Ucfx(#[from] UcfxError),
    #[error("model has no {} section", fourcc(.0))]
    MissingSection(SectionName),
    #[error("unexpected {} section in {}", fourcc(.found), fourcc(.parent))]
    UnexpectedSection {
        parent: SectionName,
        found: SectionName,
    },
    #[error("{} section is {len:#x} bytes, expected {expected:#x}", fourcc(.section))]
    SectionLength {
        section: SectionName,
        len: usize,
        expected: u64,
    },
    #[error("unsupported model version {0:#x}")]
    Version(u32),
    #[error("node {node} links to node {link}, outside the {count} nodes")]
    NodeLink {
        node: usize,
        link: i16,
        count: usize,
    },
    #[error("node {0} is its own ancestor")]
    NodeCycle(usize),
    #[error("MTRL holds {found} records, header says {expected}")]
    MaterialCount { found: usize, expected: u32 },
    #[error("segment {segment} uses node {node} of {count}")]
    SegmentNode {
        segment: usize,
        node: u16,
        count: u32,
    },
    #[error("segment {segment} uses geometry slot {slot} of {count}")]
    SegmentSlot {
        segment: usize,
        slot: u8,
        count: u32,
    },
    #[error("{0} geometry slots, but segments can only name {MAX_SLOTS}")]
    SlotCount(u32),
    #[error("GEOM says {expected} geometries, found {found}")]
    GeometryCount { expected: u32, found: usize },
    #[error("geometry slot {slot} is outside the model's {count} slots")]
    SlotOutOfRange { slot: u16, count: u32 },
    #[error("geometry slot {0} appears more than once")]
    DuplicateSlot(u16),
    #[error("geometry {geometry} says {expected} primitive groups, found {found}")]
    GroupCount {
        geometry: usize,
        expected: u32,
        found: usize,
    },
    #[error("group {group}: {source}")]
    Vertex { group: usize, source: VertexError },
    #[error("group {group}: STRM says {expected} declaration records, decl holds {found:#x} bytes")]
    DeclCount {
        group: usize,
        expected: u32,
        found: usize,
    },
    #[error("group {group}: index {value} at {at} is past the {vertices} vertices")]
    IndexOutOfRange {
        group: usize,
        at: usize,
        value: u16,
        vertices: u32,
    },
    #[error("group {group}: PRMT holds {found} ranges, INFO says {expected}")]
    RangeCount {
        group: usize,
        expected: u64,
        found: usize,
    },
    #[error("group {group}: range {range} uses material {material} of {count}")]
    RangeMaterial {
        group: usize,
        range: usize,
        material: u32,
        count: u32,
    },
    #[error("group {group}: range {range} reads past the {indices} indices")]
    RangeIndices {
        group: usize,
        range: usize,
        indices: usize,
    },
    #[error("group {group}: range {range} has vertices {min}..={max} of {vertices}")]
    RangeVertices {
        group: usize,
        range: usize,
        min: u16,
        max: u16,
        vertices: u32,
    },
    #[error("group {group}: {found} morph targets, INFO says {expected}")]
    MorphCount {
        group: usize,
        expected: u32,
        found: usize,
    },
    #[error("group {group}: morph target {morph} is malformed")]
    BadMorph { group: usize, morph: usize },
    #[error("group {group}: {count} bone palette ranges, at most {PALETTE_SLOTS}")]
    PaletteCount { group: usize, count: u32 },
}

impl From<OutOfBounds> for ModelError {
    fn from(e: OutOfBounds) -> Self {
        Self::Ucfx(UcfxError::Truncated {
            offset: e.offset,
            len: e.len,
        })
    }
}

fn fourcc(cc: &[u8; 4]) -> String {
    String::from_utf8_lossy(cc).into_owned()
}

/// An axis-aligned box.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Aabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Aabb {
    pub fn contains(&self, p: [f32; 3]) -> bool {
        (0..3).all(|i| self.min[i] <= p[i] && p[i] <= self.max[i])
    }
}

/// A bounding sphere.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Sphere {
    pub center: [f32; 3],
    pub radius: f32,
}

/// The top-level `INFO` section.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelInfo {
    pub version: u32,
    pub bounds: Aabb,
    pub flags: u32,
    pub node_count: u32,
    pub material_count: u32,
    pub segment_count: u32,
    pub slot_count: u32,
    pub blend_shape_count: u32,
    pub lod_count: u32,
    /// Field at 0x38; probably a draw or LOD distance.
    pub lod_distance: f32,
    /// Field at 0x3C; always 5.0 so far.
    pub unk_0x3c: f32,
    pub unk_0x40: u16,
    pub unk_0x42: u16,
    /// Field at 0x44: LOD bits that need streamed chunks. Matches the
    /// segments in most but not all models; [`Model::missing_slots`] is
    /// authoritative.
    pub streamed_lods: u16,
    /// Field at 0x46: LOD bits drawable from the resident asset (likewise
    /// approximate).
    pub resident_lods: u16,
}

impl ModelInfo {
    /// Set on skinned (character) models.
    pub const FLAG_SKINNED: u32 = 0x800;
    /// Set on models made of TINY geometry.
    pub const FLAG_TINY: u32 = 0x400;

    pub fn parse(info: &[u8]) -> Result<Self, ModelError> {
        check_len(*b"INFO", info, INFO_LEN as u64)?;
        let mut r = Reader::at(info, 0);
        let version = r.u32()?;
        if version != VERSION {
            return Err(ModelError::Version(version));
        }
        Ok(Self {
            version,
            bounds: Aabb {
                min: f32x3(&mut r)?,
                max: f32x3(&mut r)?,
            },
            flags: r.u32()?,
            node_count: r.u32()?,
            material_count: r.u32()?,
            segment_count: r.u32()?,
            slot_count: r.u32()?,
            blend_shape_count: r.u32()?,
            lod_count: r.u32()?,
            lod_distance: f32(&mut r)?,
            unk_0x3c: f32(&mut r)?,
            unk_0x40: r.u16()?,
            unk_0x42: r.u16()?,
            streamed_lods: r.u16()?,
            resident_lods: r.u16()?,
        })
    }
}

/// One `HIER` node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Node {
    pub name_hash: u32,
    pub flags: u16,
    pub first_child: Option<u16>,
    pub parent: Option<u16>,
    pub next_sibling: Option<u16>,
    pub unk_0x0c: u32,
    /// Transform from this node to its parent, D3DX layout.
    pub local: RawMatrix44,
    /// Inverse bind transform, D3DX layout. Identity on most rigid nodes.
    pub inverse_bind: RawMatrix44,
    /// Field at 0x90; `w` is 1.
    pub bounds_min: [f32; 4],
    /// Field at 0xA0; `w` is 1.
    pub bounds_max: [f32; 4],
}

/// One `MTRL` record.
#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    pub name_hash: u32,
    pub constants: [f32; MATERIAL_CONSTANTS],
    pub flags: u16,
    /// Name hashes of texture assets; 0 for an empty slot.
    pub textures: Vec<u32>,
    /// Probably a shader or technique id.
    pub technique: u32,
    /// Trailing field; 0x959d8470 so far.
    pub unk_tail: u32,
}

/// One `SEGM` entry: a geometry slot placed at a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Segment {
    pub node: u16,
    pub slot: u8,
    /// Bit `k` set = drawn at LOD `k`; bit 0 is the finest.
    pub lod_mask: u8,
}

impl Segment {
    pub fn in_lod(&self, lod: u32) -> bool {
        lod < 8 && self.lod_mask & (1 << lod) != 0
    }
}

/// The kind of a geometry, from its container's fourcc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GeometryKind {
    /// Rigid mesh.
    Mesh,
    /// Skinned mesh with a bone palette.
    Skin,
    /// Small pieces drawn together; position `w` holds a sub-object index.
    Tiny,
}

impl GeometryKind {
    fn from_fourcc(cc: [u8; 4]) -> Option<Self> {
        match &cc {
            b"MESH" => Some(Self::Mesh),
            b"SKIN" => Some(Self::Skin),
            b"TINY" => Some(Self::Tiny),
            _ => None,
        }
    }
}

/// A run of nodes in a SKIN group's bone palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaletteRange {
    pub first_node: u16,
    pub count: u16,
}

/// Bounds or bones, depending on the geometry kind.
#[derive(Debug, Clone, PartialEq)]
pub enum GroupExtra {
    Bounds { sphere: Sphere, aabb: Aabb },
    Palette(Vec<PaletteRange>),
}

/// A primitive group's `INFO`.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupInfo {
    pub material_ranges: u32,
    pub full_ranges: u32,
    pub morph_count: u32,
    /// Field at 0x0C; probably a shader id.
    pub effect_hash: u32,
    /// Field at 0x10; follows the vertex format.
    pub format_hash: u32,
    pub extra: GroupExtra,
}

impl GroupInfo {
    fn parse(kind: GeometryKind, info: &[u8], group: usize) -> Result<Self, ModelError> {
        let expected = match kind {
            GeometryKind::Skin => SKIN_GROUP_INFO_LEN,
            _ => GROUP_INFO_LEN,
        };
        check_len(*b"INFO", info, expected as u64)?;
        let mut r = Reader::at(info, 0);
        let material_ranges = r.u32()?;
        let full_ranges = r.u32()?;
        let morph_count = r.u32()?;
        let effect_hash = r.u32()?;
        let format_hash = r.u32()?;
        let extra = if kind == GeometryKind::Skin {
            let count = r.u32()?;
            if count as usize > PALETTE_SLOTS {
                return Err(ModelError::PaletteCount { group, count });
            }
            // Unused slots hold stale bytes; read only the live ones.
            let ranges = (0..count)
                .map(|_| {
                    Ok(PaletteRange {
                        first_node: r.u16()?,
                        count: r.u16()?,
                    })
                })
                .collect::<Result<_, OutOfBounds>>()?;
            GroupExtra::Palette(ranges)
        } else {
            let center = f32x3(&mut r)?;
            let radius = f32(&mut r)?;
            GroupExtra::Bounds {
                sphere: Sphere { center, radius },
                aabb: Aabb {
                    min: f32x3(&mut r)?,
                    max: f32x3(&mut r)?,
                },
            }
        };
        Ok(Self {
            material_ranges,
            full_ranges,
            morph_count,
            effect_hash,
            format_hash,
            extra,
        })
    }

    /// The nodes blend index `b` refers to, in order (SKIN only).
    pub fn bone_nodes(&self) -> Vec<u16> {
        match &self.extra {
            GroupExtra::Palette(p) => p
                .iter()
                .flat_map(|r| (0..r.count).map(move |i| r.first_node.wrapping_add(i)))
                .collect(),
            GroupExtra::Bounds { .. } => Vec::new(),
        }
    }
}

/// One `PRMT` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Range {
    pub material: u32,
    pub start_index: u32,
    pub primitive_count: u16,
    pub min_vertex: u16,
    pub max_vertex: u16,
    pub vertex_count: u16,
}

impl Range {
    /// Indices the strip reads: two more than its triangles.
    pub fn index_count(&self) -> u32 {
        u32::from(self.primitive_count) + 2
    }

    /// The renderer draw for this range from `buffer`, which holds the
    /// group's indices.
    pub fn draw(&self, buffer: IndexBufferHandle) -> Draw {
        Draw::Indexed {
            buffer,
            first: self.start_index,
            count: self.index_count(),
            base_vertex: 0,
        }
    }
}

/// Topology of every model draw.
pub const TOPOLOGY: Topology = Topology::TriangleStrip;

/// A primitive group: one vertex buffer, one index buffer and its draws.
#[derive(Debug, Clone, PartialEq)]
pub struct PrimitiveGroup<'a> {
    pub info: GroupInfo,
    pub decl: VertexDecl,
    /// Stream 0.
    pub vertices: VertexStream<'a>,
    /// Triangle strip indices.
    pub indices: Vec<u16>,
    /// Draws with materials, back to back over the index buffer.
    pub material_ranges: Vec<Range>,
    /// Draws of the whole group without a particular material.
    pub full_ranges: Vec<Range>,
    /// `AREA` data: one f16 per strip triangle (0 on degenerates). Its
    /// meaning is not known; it matches triangle areas only sometimes.
    pub area: Option<&'a [u8]>,
    /// Morph targets: f16x4 position deltas, one per vertex.
    pub morphs: Vec<VertexStream<'a>>,
    /// `BSHI` data, opaque.
    pub morph_index: Option<&'a [u8]>,
}

impl<'a> PrimitiveGroup<'a> {
    /// The renderer layout of the vertex data.
    pub fn layout(&self) -> Result<VertexLayout, VertexError> {
        self.decl.layout(0, self.vertices.stride)
    }

    pub fn vertex_buffer_desc(&self) -> Result<VertexBufferDesc, VertexError> {
        Ok(VertexBufferDesc {
            layout: self.layout()?,
            vertex_count: self.vertices.count,
            usage: BufferUsage::Static,
        })
    }

    pub fn index_buffer_desc(&self) -> IndexBufferDesc {
        IndexBufferDesc {
            format: IndexFormat::U16,
            index_count: self.indices.len() as u32,
            usage: BufferUsage::Static,
        }
    }

    /// The indices as bytes, ready for upload.
    pub fn index_bytes(&self) -> Vec<u8> {
        self.indices.iter().flat_map(|i| i.to_le_bytes()).collect()
    }

    /// Converts `range`'s strip to a triangle list, dropping degenerate
    /// triangles and flipping every other one so all keep the strip's
    /// winding. Returns nothing for a range outside the index buffer.
    pub fn triangles(&self, range: &Range) -> Vec<[u16; 3]> {
        strip_triangles(&self.indices, range.start_index, range.index_count())
    }

    fn parse(
        u: &Ucfx<'a>,
        kind: GeometryKind,
        prmg: usize,
        group: usize,
        material_count: u32,
    ) -> Result<Self, ModelError> {
        let s = u.sections();
        let mut info = None;
        let mut strm = None;
        let mut area = None;
        let mut ibuf = None;
        let mut prmt = None;
        let mut morphs = Vec::new();
        let mut morph_index = None;
        for c in u.children(Some(prmg)) {
            match (&s[c].fourcc, s[c].data) {
                (b"INFO", Some(d)) => info = Some(d),
                (b"STRM", None) => strm = Some(c),
                (b"AREA", None) => area = Some(c),
                (b"IBUF", None) => ibuf = Some(c),
                (b"PRMT", Some(d)) => prmt = Some(d),
                (b"BSHP", None) => morphs.push(c),
                (b"BSHI", Some(d)) => morph_index = Some(d),
                (found, _) => {
                    return Err(ModelError::UnexpectedSection {
                        parent: *b"PRMG",
                        found: *found,
                    });
                }
            }
        }
        let missing = ModelError::MissingSection;
        let info = GroupInfo::parse(kind, info.ok_or(missing(*b"INFO"))?, group)?;

        let vtx = |source| ModelError::Vertex { group, source };
        let (decl, vertices) = stream(u, strm.ok_or(missing(*b"STRM"))?, group)?;
        let decl = VertexDecl::parse(decl).map_err(vtx)?;

        let [ibuf_info, ibuf_data] = info_data(u, ibuf.ok_or(missing(*b"IBUF"))?, *b"IBUF")?;
        let count = Reader::at(ibuf_info, 0).u32()?;
        check_len(*b"IBUF", ibuf_info, 4)?;
        check_len(*b"IBUF", ibuf_data, u64::from(count) * 2)?;
        let indices: Vec<u16> = ibuf_data
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        if let Some((at, &value)) = indices
            .iter()
            .enumerate()
            .find(|&(_, &i)| u32::from(i) >= vertices.count)
        {
            return Err(ModelError::IndexOutOfRange {
                group,
                at,
                value,
                vertices: vertices.count,
            });
        }

        let area = match area {
            Some(c) => {
                let [a_info, a_data] = info_data(u, c, *b"AREA")?;
                check_len(*b"AREA", a_info, 4)?;
                let n = Reader::at(a_info, 0).u32()?;
                check_len(*b"AREA", a_data, u64::from(n) * 2)?;
                Some(a_data)
            }
            None => None,
        };

        let prmt = prmt.ok_or(missing(*b"PRMT"))?;
        let expected = u64::from(info.material_ranges) + u64::from(info.full_ranges);
        if prmt.len() as u64 != expected * RANGE_LEN as u64 {
            return Err(ModelError::RangeCount {
                group,
                expected,
                found: prmt.len() / RANGE_LEN,
            });
        }
        let mut r = Reader::at(prmt, 0);
        let mut ranges = Vec::with_capacity(expected as usize);
        for range in 0..expected as usize {
            let rg = Range {
                material: r.u32()?,
                start_index: r.u32()?,
                primitive_count: r.u16()?,
                min_vertex: r.u16()?,
                max_vertex: r.u16()?,
                vertex_count: r.u16()?,
            };
            if rg.material >= material_count {
                return Err(ModelError::RangeMaterial {
                    group,
                    range,
                    material: rg.material,
                    count: material_count,
                });
            }
            let end = u64::from(rg.start_index) + u64::from(rg.index_count());
            if end > indices.len() as u64 {
                return Err(ModelError::RangeIndices {
                    group,
                    range,
                    indices: indices.len(),
                });
            }
            if rg.min_vertex > rg.max_vertex || u32::from(rg.max_vertex) >= vertices.count {
                return Err(ModelError::RangeVertices {
                    group,
                    range,
                    min: rg.min_vertex,
                    max: rg.max_vertex,
                    vertices: vertices.count,
                });
            }
            ranges.push(rg);
        }
        let full_ranges = ranges.split_off(info.material_ranges as usize);

        if morphs.len() != info.morph_count as usize {
            return Err(ModelError::MorphCount {
                group,
                expected: info.morph_count,
                found: morphs.len(),
            });
        }
        let morphs = morphs
            .into_iter()
            .enumerate()
            .map(|(morph, c)| {
                let bad = ModelError::BadMorph { group, morph };
                let [m_info, decl, data] = leaves(u, c, *b"BSHP", [b"info", b"decl", b"data"])?;
                let mut r = Reader::at(m_info, 0);
                let fields = [r.u32()?, r.u32()?, r.u32()?];
                if m_info.len() != 12
                    || fields != [1, MORPH_STRIDE, vertices.count]
                    || VertexDecl::parse(decl).map_or(true, |d| !d.elements().is_empty())
                {
                    return Err(bad);
                }
                VertexStream::new(data, MORPH_STRIDE, vertices.count).map_err(|_| bad)
            })
            .collect::<Result<_, _>>()?;

        Ok(Self {
            info,
            decl,
            vertices,
            indices,
            material_ranges: ranges,
            full_ranges,
            area,
            morphs,
            morph_index,
        })
    }
}

/// One geometry slot's contents.
#[derive(Debug, Clone, PartialEq)]
pub struct Geometry<'a> {
    pub kind: GeometryKind,
    pub groups: Vec<PrimitiveGroup<'a>>,
}

/// The parts of a model its state machine (`STAM`) switches: a vehicle
/// holds its intact and its wrecked parts (body, turret, doors, wheels)
/// side by side, and shows one or the other.
///
/// `STAM` holds `INFO`, then `SWIT` (u32 node index per switchable node),
/// then repeated `STAT` / `CHDR` / `CEXE` records; a `CEXE` holds commands
/// as u32 (node index, verb) pairs, the verbs being name hashes such as
/// `show` and `hide`. Switchable nodes with the same parent are a part's
/// alternatives, shown in order: the first is the intact part, the next
/// the wrecked one. A switchable node never shown is a light's glow.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartStates {
    /// The switchable nodes (`SWIT`).
    pub switchable: Vec<u16>,
    /// Switchable nodes in the order commands first show them.
    pub shown: Vec<u16>,
}

impl PartStates {
    fn parse(u: &Ucfx<'_>, stam: usize, node_count: u32) -> Self {
        let show = crate::name_hash("show");
        let words = |d: &[u8]| -> Vec<u32> {
            d.chunks_exact(4)
                .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect()
        };
        let mut states = Self::default();
        for c in u.children(Some(stam)) {
            let sec = &u.sections()[c];
            let Some(data) = sec.data else { continue };
            match &sec.fourcc {
                b"SWIT" => {
                    states.switchable = words(data)
                        .into_iter()
                        .filter(|&n| n < node_count)
                        .map(|n| n as u16)
                        .collect();
                }
                b"CEXE" => {
                    let w = words(data);
                    for (k, &v) in w.iter().enumerate() {
                        if v != show {
                            continue;
                        }
                        // The node shown: the word before the verb, or
                        // after it.
                        let node = [k.checked_sub(1), Some(k + 1)]
                            .into_iter()
                            .flatten()
                            .filter_map(|i| w.get(i))
                            .find(|&&n| n < node_count && states.switchable.contains(&(n as u16)));
                        if let Some(&n) = node
                            && !states.shown.contains(&(n as u16))
                        {
                            states.shown.push(n as u16);
                        }
                    }
                }
                _ => {}
            }
        }
        states
    }

    /// Which of `nodes` are hidden (with everything under them) intact,
    /// or `wrecked`: of each part's alternatives the first shown when
    /// intact, the next when wrecked; parts with one alternative always;
    /// glows never.
    pub fn hidden(&self, nodes: &[Node], wrecked: bool) -> Vec<bool> {
        let mut off = vec![false; nodes.len()];
        for &n in &self.switchable {
            if let Some(o) = off.get_mut(usize::from(n)) {
                *o = true;
            }
        }
        let parent = |n: u16| nodes.get(usize::from(n)).and_then(|x| x.parent);
        let mut done = Vec::new();
        for &n in &self.shown {
            let p = parent(n);
            if done.contains(&p) {
                continue;
            }
            done.push(p);
            let options: Vec<u16> = self
                .shown
                .iter()
                .copied()
                .filter(|&m| parent(m) == p)
                .collect();
            let pick = if wrecked && options.len() > 1 {
                options[1]
            } else {
                options[0]
            };
            if let Some(o) = off.get_mut(usize::from(pick)) {
                *o = false;
            }
        }
        // Hidden with an ancestor hidden.
        (0..nodes.len())
            .map(|i| {
                let mut at = Some(i as u16);
                let mut steps = 0;
                while let Some(n) = at {
                    if off.get(usize::from(n)).copied().unwrap_or(false) {
                        return true;
                    }
                    at = parent(n);
                    steps += 1;
                    if steps > nodes.len() {
                        break;
                    }
                }
                false
            })
            .collect()
    }
}

/// A parsed model, with whichever streamed chunks have been added.
#[derive(Debug, Clone, PartialEq)]
pub struct Model<'a> {
    pub info: ModelInfo,
    pub nodes: Vec<Node>,
    pub materials: Vec<Material>,
    pub segments: Vec<Segment>,
    /// Name hashes of the blend shape channels (top-level `BSHP`).
    pub blend_shapes: Vec<u32>,
    /// Top-level `TINY` data: see [`tiny_members`](Self::tiny_members).
    pub tiny: Option<&'a [u8]>,
    /// The parts the state machine (`STAM`) shows and hides, if any.
    pub states: Option<PartStates>,
    /// Indexed by geometry slot; `None` until its chunk is added.
    slots: Vec<Option<Geometry<'a>>>,
}

impl<'a> Model<'a> {
    /// Parses the resident asset.
    pub fn parse(asset: &'a [u8]) -> Result<Self, ModelError> {
        let u = Ucfx::parse(asset)?;
        let mut sections: [Option<&'a [u8]>; 6] = [None; 6];
        const NAMES: [[u8; 4]; 6] = [*b"INFO", *b"HIER", *b"MTRL", *b"SEGM", *b"BSHP", *b"TINY"];
        let mut geom = None;
        let mut stam = None;
        for c in u.children(None) {
            let sec = &u.sections()[c];
            match (sec.fourcc, sec.data) {
                (cc, Some(d)) if NAMES.contains(&cc) => {
                    let i = NAMES.iter().position(|n| *n == cc).unwrap_or_default();
                    sections[i] = Some(d);
                }
                (cc, None) if &cc == b"GEOM" => geom = Some(c),
                (cc, None) if &cc == b"STAM" => stam = Some(c),
                // Collision and mixer data are not decoded here.
                (cc, _) if [*b"PHY2", *b"STAM", *b"MIXR"].contains(&cc) => {}
                (found, _) => {
                    return Err(ModelError::UnexpectedSection {
                        parent: *b"UCFX",
                        found,
                    });
                }
            }
        }
        let need = |i: usize| sections[i].ok_or(ModelError::MissingSection(NAMES[i]));
        let info = ModelInfo::parse(need(0)?)?;
        if info.slot_count > MAX_SLOTS {
            return Err(ModelError::SlotCount(info.slot_count));
        }
        let nodes = parse_nodes(need(1)?, info.node_count)?;
        let materials = parse_materials(need(2)?, info.material_count)?;
        let segments = parse_segments(need(3)?, &info)?;
        let bshp = sections[4].unwrap_or_default();
        check_len(
            *b"BSHP",
            bshp,
            u64::from(info.blend_shape_count) * BLEND_SHAPE_LEN as u64,
        )?;
        let blend_shapes = bshp
            .chunks_exact(BLEND_SHAPE_LEN)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();

        let mut model = Self {
            info,
            nodes,
            materials,
            segments,
            blend_shapes,
            tiny: sections[5],
            states: stam.map(|c| PartStates::parse(&u, c, info.node_count)),
            slots: vec![None; info.slot_count as usize],
        };
        model.add_geom(&u, geom.ok_or(ModelError::MissingSection(*b"GEOM"))?)?;
        Ok(model)
    }

    /// The sub-objects of a merged batch (a `TINY` model): the id of the
    /// layer entity each one is (its own building, placed in the static
    /// layers), indexed by the `w` of the batch's vertex positions. Empty
    /// on other models.
    pub fn tiny_members(&self) -> Vec<u32> {
        tiny_members(self.tiny.unwrap_or_default())
    }

    /// Adds a streamed chunk: an asset with the same name and type hash
    /// whose container holds only `GEOM`.
    pub fn add_streamed(&mut self, chunk: &'a [u8]) -> Result<(), ModelError> {
        let u = Ucfx::parse(chunk)?;
        let mut geom = None;
        for c in u.children(None) {
            match &u.sections()[c] {
                s if &s.fourcc == b"GEOM" && s.data.is_none() && geom.is_none() => geom = Some(c),
                s => {
                    return Err(ModelError::UnexpectedSection {
                        parent: *b"UCFX",
                        found: s.fourcc,
                    });
                }
            }
        }
        self.add_geom(&u, geom.ok_or(ModelError::MissingSection(*b"GEOM"))?)
    }

    fn add_geom(&mut self, u: &Ucfx<'a>, geom: usize) -> Result<(), ModelError> {
        let s = u.sections();
        let children: Vec<usize> = u.children(Some(geom)).collect();
        let (info, indx, kinds) = match children.as_slice() {
            [i, x, rest @ ..] if &s[*i].fourcc == b"INFO" && &s[*x].fourcc == b"INDX" => {
                let leaf = |c: usize| s[c].data.ok_or(ModelError::MissingSection(s[c].fourcc));
                (leaf(*i)?, leaf(*x)?, rest)
            }
            _ => return Err(ModelError::MissingSection(*b"INDX")),
        };
        check_len(*b"INFO", info, 4)?;
        let expected = Reader::at(info, 0).u32()?;
        if expected as usize != kinds.len() {
            return Err(ModelError::GeometryCount {
                expected,
                found: kinds.len(),
            });
        }
        check_len(*b"INDX", indx, kinds.len() as u64 * 2)?;

        // Parse everything before touching `self`, so a bad chunk adds nothing.
        let mut parsed = Vec::with_capacity(kinds.len());
        let mut group = 0;
        for (geometry, (&c, slot)) in kinds.iter().zip(indx.chunks_exact(2)).enumerate() {
            let slot = u16::from_le_bytes([slot[0], slot[1]]);
            let slot_count = self.info.slot_count;
            if u32::from(slot) >= slot_count {
                return Err(ModelError::SlotOutOfRange {
                    slot,
                    count: slot_count,
                });
            }
            if self.slots[usize::from(slot)].is_some() || parsed.iter().any(|(s, _)| *s == slot) {
                return Err(ModelError::DuplicateSlot(slot));
            }
            let sec = &s[c];
            let kind = GeometryKind::from_fourcc(sec.fourcc).filter(|_| sec.data.is_none());
            let kind = kind.ok_or(ModelError::UnexpectedSection {
                parent: *b"GEOM",
                found: sec.fourcc,
            })?;
            let mut parts = u.children(Some(c));
            let count_info = parts
                .next()
                .filter(|&i| &s[i].fourcc == b"INFO")
                .and_then(|i| s[i].data)
                .ok_or(ModelError::MissingSection(*b"INFO"))?;
            check_len(*b"INFO", count_info, 4)?;
            let expected = Reader::at(count_info, 0).u32()?;
            let prmgs: Vec<usize> = parts.collect();
            if let Some(&bad) = prmgs.iter().find(|&&p| &s[p].fourcc != b"PRMG") {
                return Err(ModelError::UnexpectedSection {
                    parent: sec.fourcc,
                    found: s[bad].fourcc,
                });
            }
            if expected as usize != prmgs.len() {
                return Err(ModelError::GroupCount {
                    geometry,
                    expected,
                    found: prmgs.len(),
                });
            }
            let groups = prmgs
                .into_iter()
                .map(|p| {
                    group += 1;
                    PrimitiveGroup::parse(u, kind, p, group - 1, self.info.material_count)
                })
                .collect::<Result<_, _>>()?;
            parsed.push((slot, Geometry { kind, groups }));
        }
        for (slot, g) in parsed {
            self.slots[usize::from(slot)] = Some(g);
        }
        Ok(())
    }

    /// The geometry in `slot`, if its chunk has been added.
    pub fn geometry(&self, slot: u8) -> Option<&Geometry<'a>> {
        self.slots.get(usize::from(slot))?.as_ref()
    }

    /// Slots whose chunk has not been added yet.
    pub fn missing_slots(&self) -> Vec<u8> {
        (0..self.slots.len())
            .filter(|&i| self.slots[i].is_none())
            .map(|i| i as u8)
            .collect()
    }

    /// Whether every LOD can be drawn.
    pub fn is_complete(&self) -> bool {
        self.slots.iter().all(Option::is_some)
    }

    /// Whether every segment drawn at `lod` has its geometry.
    pub fn has_lod(&self, lod: u32) -> bool {
        self.segments
            .iter()
            .filter(|s| s.in_lod(lod))
            .all(|s| self.geometry(s.slot).is_some())
    }

    /// The segments drawn at `lod` with their geometry; segments whose chunk
    /// is missing are skipped.
    pub fn lod(&self, lod: u32) -> impl Iterator<Item = (&Segment, &Geometry<'a>)> + '_ {
        self.segments
            .iter()
            .filter(move |s| s.in_lod(lod))
            .filter_map(|s| Some((s, self.geometry(s.slot)?)))
    }

    /// Transform from `node` to model space, in glam's column-vector
    /// convention. `None` for an out-of-range node.
    pub fn node_to_model(&self, node: u16) -> Option<Mat4> {
        let mut m = Mat4::IDENTITY;
        let mut at = Some(node);
        // Parsing rejected cycles, so this walk ends within `nodes.len()`.
        while let Some(n) = at {
            let node = self.nodes.get(usize::from(n))?;
            m = Mat4::from(node.local) * m;
            at = node.parent;
        }
        Some(m)
    }
}

fn check_len(section: SectionName, data: &[u8], expected: u64) -> Result<(), ModelError> {
    if data.len() as u64 != expected {
        return Err(ModelError::SectionLength {
            section,
            len: data.len(),
            expected,
        });
    }
    Ok(())
}

fn f32(r: &mut Reader<'_>) -> Result<f32, OutOfBounds> {
    r.u32().map(f32::from_bits)
}

fn f32x3(r: &mut Reader<'_>) -> Result<[f32; 3], OutOfBounds> {
    Ok([f32(r)?, f32(r)?, f32(r)?])
}

fn f32x4(r: &mut Reader<'_>) -> Result<[f32; 4], OutOfBounds> {
    Ok([f32(r)?, f32(r)?, f32(r)?, f32(r)?])
}

fn matrix(r: &mut Reader<'_>) -> Result<RawMatrix44, OutOfBounds> {
    Ok(RawMatrix44 {
        m: [f32x4(r)?, f32x4(r)?, f32x4(r)?, f32x4(r)?],
    })
}

/// The leaf children of container `c`, which must be exactly `N` leaves with
/// the given fourccs.
fn leaves<'a, const N: usize>(
    u: &Ucfx<'a>,
    c: usize,
    parent: SectionName,
    names: [&[u8; 4]; N],
) -> Result<[&'a [u8]; N], ModelError> {
    let s = u.sections();
    let kids: Vec<usize> = u.children(Some(c)).collect();
    let mut out = [&[][..]; N];
    for (i, name) in names.iter().enumerate() {
        let k = *kids.get(i).ok_or(ModelError::MissingSection(**name))?;
        match (&s[k].fourcc, s[k].data) {
            (cc, Some(d)) if cc == *name => out[i] = d,
            (found, _) => {
                return Err(ModelError::UnexpectedSection {
                    parent,
                    found: *found,
                });
            }
        }
    }
    if let Some(&extra) = kids.get(N) {
        return Err(ModelError::UnexpectedSection {
            parent,
            found: s[extra].fourcc,
        });
    }
    Ok(out)
}

fn info_data<'a>(u: &Ucfx<'a>, c: usize, parent: SectionName) -> Result<[&'a [u8]; 2], ModelError> {
    leaves(u, c, parent, [b"info", b"data"])
}

/// Reads a `STRM` container: the declaration bytes and checked stream 0.
fn stream<'a>(
    u: &Ucfx<'a>,
    c: usize,
    group: usize,
) -> Result<(&'a [u8], VertexStream<'a>), ModelError> {
    let [info, decl, data] = leaves(u, c, *b"STRM", [b"info", b"decl", b"data"])?;
    check_len(*b"STRM", info, 12)?;
    let mut r = Reader::at(info, 0);
    let (records, stride, count) = (r.u32()?, r.u32()?, r.u32()?);
    if decl.len() as u64 != u64::from(records) * 8 {
        return Err(ModelError::DeclCount {
            group,
            expected: records,
            found: decl.len(),
        });
    }
    let vertices = VertexStream::new(data, stride, count)
        .map_err(|source| ModelError::Vertex { group, source })?;
    Ok((decl, vertices))
}

fn parse_nodes(hier: &[u8], count: u32) -> Result<Vec<Node>, ModelError> {
    check_len(*b"HIER", hier, u64::from(count) * NODE_LEN as u64)?;
    let n = count as usize;
    let mut r = Reader::at(hier, 0);
    let mut nodes = Vec::with_capacity(n);
    for node in 0..n {
        let name_hash = r.u32()?;
        let flags = r.u16()?;
        let mut link = || -> Result<Option<u16>, ModelError> {
            let v = r.u16()? as i16;
            match v {
                -1 => Ok(None),
                v if v >= 0 && (v as usize) < n => Ok(Some(v as u16)),
                link => Err(ModelError::NodeLink {
                    node,
                    link,
                    count: n,
                }),
            }
        };
        let (first_child, parent, next_sibling) = (link()?, link()?, link()?);
        nodes.push(Node {
            name_hash,
            flags,
            first_child,
            parent,
            next_sibling,
            unk_0x0c: r.u32()?,
            local: matrix(&mut r)?,
            inverse_bind: matrix(&mut r)?,
            bounds_min: f32x4(&mut r)?,
            bounds_max: f32x4(&mut r)?,
        });
    }
    // Every parent chain must reach a root within `n` steps.
    for start in 0..n {
        let mut at = nodes[start].parent;
        for _ in 0..=n {
            match at {
                Some(p) => at = nodes[usize::from(p)].parent,
                None => break,
            }
        }
        if at.is_some() {
            return Err(ModelError::NodeCycle(start));
        }
    }
    Ok(nodes)
}

/// The triangles of the strip `indices[start..start + count]`, skipping
/// degenerates and alternating winding as strips do. Empty if the range is
/// outside `indices`.
pub(crate) fn strip_triangles(indices: &[u16], start: u32, count: u32) -> Vec<[u16; 3]> {
    let start = start as usize;
    let Some(strip) = indices.get(start..start + count as usize) else {
        return Vec::new();
    };
    strip
        .windows(3)
        .enumerate()
        .filter(|(_, t)| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
        .map(|(i, t)| {
            if i % 2 == 0 {
                [t[0], t[1], t[2]]
            } else {
                [t[1], t[0], t[2]]
            }
        })
        .collect()
}

/// Reads one `MTRL` record.
pub(crate) fn read_material(r: &mut Reader<'_>) -> Result<Material, OutOfBounds> {
    let name_hash = r.u32()?;
    let mut constants = [0.0; MATERIAL_CONSTANTS];
    for c in &mut constants {
        *c = f32(r)?;
    }
    let flags = r.u16()?;
    let ntex = r.u16()?;
    let textures = (0..ntex).map(|_| r.u32()).collect::<Result<_, _>>()?;
    Ok(Material {
        name_hash,
        constants,
        flags,
        textures,
        technique: r.u32()?,
        unk_tail: r.u32()?,
    })
}

fn parse_materials(mtrl: &[u8], count: u32) -> Result<Vec<Material>, ModelError> {
    let mut r = Reader::at(mtrl, 0);
    // Each record is at least MATERIAL_FIXED_LEN bytes, which bounds the
    // allocation by the section's size.
    let mut out = Vec::with_capacity((count as usize).min(mtrl.len() / MATERIAL_FIXED_LEN));
    let short = |found| ModelError::MaterialCount {
        found,
        expected: count,
    };
    for i in 0..count as usize {
        out.push(read_material(&mut r).map_err(|_| short(i))?);
    }
    if r.pos() != mtrl.len() {
        return Err(short(count as usize + 1));
    }
    Ok(out)
}

fn parse_segments(segm: &[u8], info: &ModelInfo) -> Result<Vec<Segment>, ModelError> {
    check_len(
        *b"SEGM",
        segm,
        u64::from(info.segment_count) * SEGMENT_LEN as u64,
    )?;
    segm.chunks_exact(SEGMENT_LEN)
        .enumerate()
        .map(|(segment, c)| {
            let s = Segment {
                node: u16::from_le_bytes([c[0], c[1]]),
                slot: c[2],
                lod_mask: c[3],
            };
            if u32::from(s.node) >= info.node_count {
                return Err(ModelError::SegmentNode {
                    segment,
                    node: s.node,
                    count: info.node_count,
                });
            }
            if u32::from(s.slot) >= info.slot_count {
                return Err(ModelError::SegmentSlot {
                    segment,
                    slot: s.slot,
                    count: info.slot_count,
                });
            }
            Ok(s)
        })
        .collect()
}

/// The records of a `TINY` section: a count, then that many entity ids.
fn tiny_members(tiny: &[u8]) -> Vec<u32> {
    let Some(count) = tiny.get(..4) else {
        return Vec::new();
    };
    let count = u32::from_le_bytes([count[0], count[1], count[2], count[3]]) as usize;
    tiny[4..]
        .chunks_exact(4)
        .take(count)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_records_are_entity_ids() {
        let mut tiny = 2u32.to_le_bytes().to_vec();
        tiny.extend(0x0001_0419u32.to_le_bytes());
        tiny.extend(0x0013_84ebu32.to_le_bytes());
        tiny.extend([7, 7]);
        assert_eq!(tiny_members(&tiny), [0x0001_0419, 0x0013_84eb]);
        assert_eq!(tiny_members(&[]), []);
        assert_eq!(tiny_members(&9u32.to_le_bytes()), []);
    }
    use crate::testutil::{UcfxNode, ucfx};
    use crate::vertex::tests::{decl, f32_to_half};
    use crate::vertex::{DeclType::*, DeclUsage::*};
    use m2_render::{VertexFormat, VertexSemantic};

    fn leaf(cc: &[u8; 4], data: Vec<u8>) -> UcfxNode {
        UcfxNode::Leaf(*cc, data)
    }

    fn node(cc: &[u8; 4], children: Vec<UcfxNode>) -> UcfxNode {
        UcfxNode::Container(*cc, children)
    }

    fn le(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    fn floats(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    fn halves(v: &[f32]) -> Vec<u8> {
        v.iter()
            .flat_map(|&f| f32_to_half(f).to_le_bytes())
            .collect()
    }

    fn range(material: u32, start: u32, prims: u16, min: u16, max: u16) -> Vec<u8> {
        let mut out = le(&[material, start]);
        for v in [prims, min, max, max - min + 1] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    /// A quad as a 4-vertex strip with a MESH vertex format.
    fn mesh_group() -> UcfxNode {
        let mut verts = Vec::new();
        for (x, z) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
            verts.extend(halves(&[x, 0.0, z, 1.0, x, z, 0.0, 1.0, 0.0, 0.0]));
        }
        let declaration = decl(&[
            (0, 0, Float16x4, Position, 0),
            (0, 8, Float16x2, TexCoord, 0),
            (0, 12, Float16x4, Normal, 0),
        ]);
        let mut info = le(&[1, 1, 0, 0x79f4_ccb3, 0xa9ea_4cf7]);
        info.extend(floats(&[0.5, 0.0, 0.5, 0.75, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0]));
        let indices: Vec<u8> = [0u16, 1, 2, 3]
            .iter()
            .flat_map(|i| i.to_le_bytes())
            .collect();
        let mut prmt = range(1, 0, 2, 0, 3);
        prmt.extend(range(0, 0, 2, 0, 3));
        node(
            b"PRMG",
            vec![
                leaf(b"INFO", info),
                node(
                    b"STRM",
                    vec![
                        leaf(b"info", le(&[4, 0x14, 4])),
                        leaf(b"decl", declaration),
                        leaf(b"data", verts),
                    ],
                ),
                node(
                    b"AREA",
                    vec![leaf(b"info", le(&[2])), leaf(b"data", halves(&[0.5, 0.5]))],
                ),
                node(
                    b"IBUF",
                    vec![leaf(b"info", le(&[4])), leaf(b"data", indices)],
                ),
                leaf(b"PRMT", prmt),
            ],
        )
    }

    /// A skinned triangle with one morph target.
    fn skin_group() -> UcfxNode {
        let mut verts = Vec::new();
        for (x, bone) in [(0.0, 0u8), (1.0, 1), (2.0, 0)] {
            verts.extend(halves(&[x, 1.0, 0.0, 1.0]));
            verts.extend([bone, 0, 0, 0, 255, 0, 0, 0]);
        }
        let declaration = decl(&[
            (0, 0, Float16x4, Position, 0),
            (0, 8, UByte4, BlendIndices, 0),
            (0, 12, UByte4N, BlendWeight, 0),
            (1, 0, Float16x4, TexCoord, 1),
        ]);
        let mut info = le(&[1, 0, 1, 1, 2, 2]);
        for v in [1u16, 1, 0, 1, 0xAAAA, 0xAAAA] {
            info.extend(v.to_le_bytes());
        }
        info.resize(SKIN_GROUP_INFO_LEN, 0xCD);
        node(
            b"PRMG",
            vec![
                leaf(b"INFO", info),
                node(
                    b"STRM",
                    vec![
                        leaf(b"info", le(&[5, 16, 3])),
                        leaf(b"decl", declaration),
                        leaf(b"data", verts),
                    ],
                ),
                node(
                    b"IBUF",
                    vec![
                        leaf(b"info", le(&[3])),
                        leaf(
                            b"data",
                            [0u16, 1, 2].iter().flat_map(|i| i.to_le_bytes()).collect(),
                        ),
                    ],
                ),
                leaf(b"PRMT", range(0, 0, 1, 0, 2)),
                node(
                    b"BSHP",
                    vec![
                        leaf(b"info", le(&[1, 8, 3])),
                        leaf(b"decl", decl(&[])),
                        leaf(b"data", halves(&[0.0, 0.5, 0.0, 0.0].repeat(3))),
                    ],
                ),
                leaf(b"BSHI", vec![0; 8]),
            ],
        )
    }

    fn geom(slots: &[u16], kinds: Vec<UcfxNode>) -> UcfxNode {
        let mut children = vec![
            leaf(b"INFO", le(&[kinds.len() as u32])),
            leaf(
                b"INDX",
                slots.iter().flat_map(|s| s.to_le_bytes()).collect(),
            ),
        ];
        children.extend(kinds);
        node(b"GEOM", children)
    }

    fn kind(cc: &[u8; 4], groups: Vec<UcfxNode>) -> UcfxNode {
        let mut children = vec![leaf(b"INFO", le(&[groups.len() as u32]))];
        children.extend(groups);
        node(cc, children)
    }

    fn matrix(translate: [f32; 3]) -> Vec<f32> {
        let [x, y, z] = translate;
        vec![
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, x, y, z, 1.0,
        ]
    }

    fn hier() -> Vec<u8> {
        let mut out = Vec::new();
        // Root 0 with child 1.
        for (hash, child, parent, t) in [
            (0x1111_1111u32, 1i16, -1i16, [0.0, 0.0, 10.0]),
            (0x2222_2222, -1, 0, [5.0, 0.0, 0.0]),
        ] {
            out.extend(hash.to_le_bytes());
            out.extend(0u16.to_le_bytes());
            for l in [child, parent, -1] {
                out.extend(l.to_le_bytes());
            }
            out.extend(0u32.to_le_bytes());
            out.extend(floats(&matrix(t)));
            out.extend(floats(&matrix([0.0; 3])));
            out.extend(floats(&[-1.0, -1.0, -1.0, 1.0, 1.0, 1.0, 1.0, 1.0]));
        }
        out
    }

    fn material(hash: u32, textures: &[u32]) -> Vec<u8> {
        let mut out = le(&[hash]);
        out.extend(floats(&[1.0; MATERIAL_CONSTANTS]));
        out.extend(0x80u16.to_le_bytes());
        out.extend((textures.len() as u16).to_le_bytes());
        out.extend(le(textures));
        out.extend(le(&[0x3e9a_1b40, 0x959d_8470]));
        out
    }

    fn info(slots: u32) -> Vec<u8> {
        let mut out = le(&[VERSION]);
        out.extend(floats(&[-1.0, -1.0, -1.0, 10.0, 10.0, 20.0]));
        out.extend(le(&[0x890, 2, 2, 2, slots, 1, 2]));
        out.extend(floats(&[30.0, 5.0]));
        out.extend(le(&[0, 0x0001_0002]));
        out
    }

    /// Slot 0 (LOD 1, node 0) is resident; slot 1 (LOD 0, node 1) streams.
    fn model_parts() -> (Vec<u8>, Vec<u8>) {
        let mut mtrl = material(0xAAAA_0001, &[0xf00d_0001]);
        mtrl.extend(material(0xAAAA_0002, &[0xf00d_0002, 0, 0xf00d_0003]));
        let base = ucfx(&[
            leaf(b"INFO", info(2)),
            leaf(b"HIER", hier()),
            leaf(b"MTRL", mtrl),
            leaf(b"BSHP", le(&[0x5151_5151, 0])),
            leaf(b"SEGM", vec![0, 0, 0, 2, 1, 0, 1, 1]),
            leaf(b"PHY2", vec![1, 2, 3]),
            geom(&[0], vec![kind(b"MESH", vec![mesh_group()])]),
        ]);
        let chunk = ucfx(&[geom(&[1], vec![kind(b"SKIN", vec![skin_group()])])]);
        (base, chunk)
    }

    #[test]
    fn wrecks_swap_switchable_parts() {
        let (show, hide) = (crate::name_hash("show"), crate::name_hash("hide"));
        // Root 0 holds the intact body 1 and the wrecked body 2; the intact
        // body holds a glow 3 (never shown) and a turret 4.
        let data = ucfx(&[node(
            b"STAM",
            vec![
                leaf(b"INFO", vec![0; 12]),
                leaf(b"SWIT", le(&[1, 2, 3])),
                leaf(b"STAT", le(&[0x1234_5678])),
                leaf(b"CHDR", le(&[2])),
                leaf(b"CEXE", le(&[1, show, 2, hide, 3, hide])),
                leaf(b"STAT", le(&[0x8765_4321])),
                leaf(b"CHDR", le(&[2])),
                leaf(b"CEXE", le(&[1, hide, 2, show])),
            ],
        )]);
        let u = Ucfx::parse(&data).unwrap();
        let stam = u.children(None).next().unwrap();
        let states = PartStates::parse(&u, stam, 5);
        assert_eq!(states.switchable, vec![1, 2, 3]);
        assert_eq!(states.shown, vec![1, 2]);
        let n = |parent: Option<u16>| Node {
            name_hash: 0,
            flags: 0,
            first_child: None,
            parent,
            next_sibling: None,
            unk_0x0c: 0,
            local: Default::default(),
            inverse_bind: Default::default(),
            bounds_min: [0.0; 4],
            bounds_max: [0.0; 4],
        };
        let nodes = [n(None), n(Some(0)), n(Some(0)), n(Some(1)), n(Some(1))];
        assert_eq!(
            states.hidden(&nodes, false),
            vec![false, false, true, true, false]
        );
        assert_eq!(
            states.hidden(&nodes, true),
            vec![false, true, false, true, true]
        );
    }

    #[test]
    fn parses_resident_model() {
        let (base, _) = model_parts();
        let m = Model::parse(&base).unwrap();
        assert_eq!(m.info.node_count, 2);
        assert_eq!(m.info.lod_distance, 30.0);
        assert_eq!((m.info.streamed_lods, m.info.resident_lods), (2, 1));
        assert_eq!(m.nodes[1].parent, Some(0));
        assert_eq!(m.nodes[0].first_child, Some(1));
        assert_eq!(m.materials[1].textures, vec![0xf00d_0002, 0, 0xf00d_0003]);
        assert_eq!(m.materials[1].technique, 0x3e9a_1b40);
        assert_eq!(m.blend_shapes, vec![0x5151_5151]);
        assert_eq!(m.missing_slots(), vec![1]);
        assert!(m.has_lod(1) && !m.has_lod(0));

        let (seg, g) = m.lod(1).next().unwrap();
        assert_eq!((seg.node, g.kind), (0, GeometryKind::Mesh));
        let group = &g.groups[0];
        assert_eq!(group.vertices.count, 4);
        assert_eq!(group.material_ranges.len(), 1);
        assert_eq!(group.full_ranges.len(), 1);
        assert_eq!(group.material_ranges[0].material, 1);
        assert!(matches!(
            group.info.extra,
            GroupExtra::Bounds {
                sphere: Sphere { radius: 0.75, .. },
                ..
            }
        ));

        let layout = group.layout().unwrap();
        assert_eq!(layout.stride(), 0x14);
        assert_eq!(
            layout.find(VertexSemantic::Normal).unwrap().format,
            VertexFormat::Half4
        );
        assert_eq!(group.vertex_buffer_desc().unwrap().size_in_bytes(), 0x50);
        assert_eq!(group.index_buffer_desc().size_in_bytes(), 8);
        assert_eq!(group.index_bytes(), vec![0, 0, 1, 0, 2, 0, 3, 0]);

        let pos = group
            .vertices
            .read(group.decl.find(Position, 0).unwrap())
            .unwrap();
        assert_eq!(pos[3], [1.0, 0.0, 1.0, 1.0]);
        let tris = group.triangles(&group.material_ranges[0]);
        assert_eq!(tris, vec![[0, 1, 2], [2, 1, 3]]);
    }

    #[test]
    fn adds_streamed_geometry() {
        let (base, chunk) = model_parts();
        let mut m = Model::parse(&base).unwrap();
        m.add_streamed(&chunk).unwrap();
        assert!(m.is_complete() && m.has_lod(0));
        let (seg, g) = m.lod(0).next().unwrap();
        assert_eq!((seg.node, g.kind), (1, GeometryKind::Skin));
        let group = &g.groups[0];
        assert_eq!(group.info.bone_nodes(), vec![1, 0]);
        assert_eq!(group.morphs.len(), 1);
        assert_eq!(group.morphs[0].count, 3);
        assert_eq!(group.morph_index, Some(&[0u8; 8][..]));
        // Stream 1 belongs to the morph data, not stream 0's layout.
        assert!(
            group
                .layout()
                .unwrap()
                .find(VertexSemantic::TexCoord(1))
                .is_none()
        );

        // Node 1 sits at (5, 0, 10) in model space.
        let t = m.node_to_model(seg.node).unwrap().w_axis;
        assert_eq!(t.to_array(), [5.0, 0.0, 10.0, 1.0]);

        // The same slot cannot be filled twice.
        assert_eq!(m.add_streamed(&chunk), Err(ModelError::DuplicateSlot(1)));
    }

    #[test]
    fn strip_conversion_skips_degenerates() {
        let (base, _) = model_parts();
        let m = Model::parse(&base).unwrap();
        let mut g = m.geometry(0).unwrap().groups[0].clone();
        g.indices = vec![0, 1, 2, 2, 3, 3, 0, 1, 2];
        let r = Range {
            material: 0,
            start_index: 0,
            primitive_count: 7,
            min_vertex: 0,
            max_vertex: 3,
            vertex_count: 4,
        };
        assert_eq!(g.triangles(&r), vec![[0, 1, 2], [0, 3, 1], [0, 1, 2]]);
        let past = Range {
            start_index: 5,
            ..r
        };
        assert!(g.triangles(&past).is_empty());
    }

    /// Rebuilds the base model with one section replaced.
    fn base_with(f: impl Fn(&mut Vec<UcfxNode>)) -> Vec<u8> {
        let mut mtrl = material(1, &[2]);
        mtrl.extend(material(3, &[4]));
        let mut nodes = vec![
            leaf(b"INFO", info(2)),
            leaf(b"HIER", hier()),
            leaf(b"MTRL", mtrl),
            leaf(b"BSHP", le(&[0x5151_5151, 0])),
            leaf(b"SEGM", vec![0, 0, 0, 2, 1, 0, 1, 1]),
            geom(&[0], vec![kind(b"MESH", vec![mesh_group()])]),
        ];
        f(&mut nodes);
        ucfx(&nodes)
    }

    #[test]
    fn rejects_inconsistent_models() {
        assert!(Model::parse(&base_with(|_| {})).is_ok());

        let bad = base_with(|n| n[0] = leaf(b"INFO", info(300)));
        assert_eq!(Model::parse(&bad), Err(ModelError::SlotCount(300)));

        let bad = base_with(|n| {
            n[4] = leaf(b"SEGM", vec![0, 0, 5, 2, 1, 0, 1, 1]);
        });
        assert!(matches!(
            Model::parse(&bad),
            Err(ModelError::SegmentSlot { slot: 5, .. })
        ));

        let bad = base_with(|n| {
            n[4] = leaf(b"SEGM", vec![9, 0, 0, 2, 1, 0, 1, 1]);
        });
        assert!(matches!(
            Model::parse(&bad),
            Err(ModelError::SegmentNode { node: 9, .. })
        ));

        let bad = base_with(|n| {
            let mut h = hier();
            // Node 0's parent becomes node 1: a cycle.
            h[8..10].copy_from_slice(&1i16.to_le_bytes());
            n[1] = leaf(b"HIER", h);
        });
        assert!(matches!(Model::parse(&bad), Err(ModelError::NodeCycle(_))));

        let bad = base_with(|n| {
            let mut h = hier();
            h[6..8].copy_from_slice(&7i16.to_le_bytes());
            n[1] = leaf(b"HIER", h);
        });
        assert!(matches!(
            Model::parse(&bad),
            Err(ModelError::NodeLink { link: 7, .. })
        ));

        let bad = base_with(|n| n[2] = leaf(b"MTRL", material(1, &[2])));
        assert!(matches!(
            Model::parse(&bad),
            Err(ModelError::MaterialCount { .. })
        ));

        let bad = base_with(|n| {
            let mut m = material(1, &[2]);
            m.extend(material(3, &[4]));
            m.push(0);
            n[2] = leaf(b"MTRL", m);
        });
        assert!(matches!(
            Model::parse(&bad),
            Err(ModelError::MaterialCount { .. })
        ));

        let bad = base_with(|n| {
            n.pop();
        });
        assert_eq!(
            Model::parse(&bad),
            Err(ModelError::MissingSection(*b"GEOM"))
        );

        let bad = base_with(|n| n.push(leaf(b"WHAT", vec![])));
        assert!(matches!(
            Model::parse(&bad),
            Err(ModelError::UnexpectedSection { .. })
        ));

        let bad = base_with(|n| {
            n[5] = geom(
                &[0, 0],
                vec![
                    kind(b"MESH", vec![mesh_group()]),
                    kind(b"MESH", vec![mesh_group()]),
                ],
            );
        });
        assert_eq!(Model::parse(&bad), Err(ModelError::DuplicateSlot(0)));

        let mut v = info(2);
        v[0] = 0x3A;
        let bad = base_with(|n| n[0] = leaf(b"INFO", v.clone()));
        assert_eq!(Model::parse(&bad), Err(ModelError::Version(0x3A)));
    }

    /// Replaces one leaf of the mesh group, found by its path.
    fn group_with(path: &[&[u8; 4]], data: Vec<u8>) -> Vec<u8> {
        fn edit(nodes: &mut [UcfxNode], path: &[&[u8; 4]], data: &[u8]) {
            for n in nodes {
                match n {
                    UcfxNode::Leaf(cc, d) if path.len() == 1 && cc == path[0] => {
                        *d = data.to_vec();
                    }
                    UcfxNode::Container(cc, kids) if cc == path[0] => edit(kids, &path[1..], data),
                    _ => {}
                }
            }
        }
        base_with(|n| {
            let mut g = vec![mesh_group()];
            edit(&mut g, path, &data);
            n[5] = geom(&[0], vec![kind(b"MESH", g)]);
        })
    }

    #[test]
    fn rejects_inconsistent_groups() {
        let parse = |b: Vec<u8>| Model::parse(&b).map(|_| ());
        let bad = group_with(
            &[b"PRMG", b"IBUF", b"data"],
            le(&[0x0004_0000, 0x0003_0002]),
        );
        assert!(matches!(
            parse(bad),
            Err(ModelError::IndexOutOfRange { value: 4, .. })
        ));
        let bad = group_with(&[b"PRMG", b"IBUF", b"info"], le(&[5]));
        assert!(matches!(parse(bad), Err(ModelError::SectionLength { .. })));
        let bad = group_with(&[b"PRMG", b"STRM", b"info"], le(&[4, 0x14, 5]));
        assert!(matches!(
            parse(bad),
            Err(ModelError::Vertex {
                source: VertexError::StreamLength { .. },
                ..
            })
        ));
        let bad = group_with(&[b"PRMG", b"STRM", b"info"], le(&[3, 0x14, 4]));
        assert!(matches!(parse(bad), Err(ModelError::DeclCount { .. })));
        let bad = group_with(&[b"PRMG", b"PRMT"], range(1, 0, 2, 0, 3));
        assert!(matches!(parse(bad), Err(ModelError::RangeCount { .. })));
        let mut prmt = range(2, 0, 2, 0, 3);
        prmt.extend(range(0, 0, 2, 0, 3));
        let bad = group_with(&[b"PRMG", b"PRMT"], prmt);
        assert!(matches!(
            parse(bad),
            Err(ModelError::RangeMaterial { material: 2, .. })
        ));
        let mut prmt = range(1, 1, 2, 0, 3);
        prmt.extend(range(0, 0, 2, 0, 3));
        let bad = group_with(&[b"PRMG", b"PRMT"], prmt);
        assert!(matches!(parse(bad), Err(ModelError::RangeIndices { .. })));
        let mut prmt = range(1, 0, 2, 0, 4);
        prmt.extend(range(0, 0, 2, 0, 3));
        let bad = group_with(&[b"PRMG", b"PRMT"], prmt);
        assert!(matches!(parse(bad), Err(ModelError::RangeVertices { .. })));
        let bad = group_with(&[b"PRMG", b"AREA", b"info"], le(&[3]));
        assert!(matches!(parse(bad), Err(ModelError::SectionLength { .. })));
        let bad = group_with(&[b"PRMG", b"INFO"], vec![0; 0x38]);
        assert!(matches!(parse(bad), Err(ModelError::SectionLength { .. })));
    }

    #[test]
    fn rejects_bad_morphs_and_chunks() {
        let (base, chunk) = model_parts();
        let mut m = Model::parse(&base).unwrap();
        // A chunk filling a slot past the header's count.
        let bad = ucfx(&[geom(&[2], vec![kind(b"SKIN", vec![skin_group()])])]);
        assert!(matches!(
            m.add_streamed(&bad),
            Err(ModelError::SlotOutOfRange { slot: 2, .. })
        ));
        // A chunk with anything besides GEOM.
        let bad = ucfx(&[leaf(b"INFO", vec![]), geom(&[1], vec![])]);
        assert!(matches!(
            m.add_streamed(&bad),
            Err(ModelError::UnexpectedSection { .. })
        ));
        // A failed chunk leaves the model unchanged.
        assert_eq!(m.missing_slots(), vec![1]);
        m.add_streamed(&chunk).unwrap();

        // Morph count mismatch: drop the morph target but keep INFO's count.
        let mut g = skin_group();
        if let UcfxNode::Container(_, kids) = &mut g {
            kids.retain(|k| !matches!(k, UcfxNode::Container(cc, _) if cc == b"BSHP"));
        }
        let bad = ucfx(&[geom(&[1], vec![kind(b"SKIN", vec![g])])]);
        let mut m = Model::parse(&base).unwrap();
        assert!(matches!(
            m.add_streamed(&bad),
            Err(ModelError::MorphCount {
                expected: 1,
                found: 0,
                ..
            })
        ));
    }

    #[test]
    fn byte_flips_never_panic() {
        let (base, chunk) = model_parts();
        for at in 0..base.len() {
            for v in [0, 1, 0x7f, 0xff] {
                let mut a = base.clone();
                a[at] = v;
                if let Ok(mut m) = Model::parse(&a) {
                    let _ = m.add_streamed(&chunk);
                    for g in m.lod(0).chain(m.lod(1)).flat_map(|(_, g)| &g.groups) {
                        let _ = g.layout();
                        for r in g.material_ranges.iter().chain(&g.full_ranges) {
                            let _ = g.triangles(r);
                        }
                    }
                    let _ = m.node_to_model(1);
                }
            }
        }
        for at in 0..chunk.len() {
            let mut a = chunk.clone();
            a[at] ^= 0xFF;
            let mut m = Model::parse(&base).unwrap();
            let _ = m.add_streamed(&a);
        }
    }
}
