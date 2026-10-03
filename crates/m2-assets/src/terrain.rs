//! Terrain: the 400 m × 400 m cells the island is built from, and the
//! low-resolution tiles drawn in the distance.
//!
//! A terrain cell (asset type hash `7c569307`, one per `c3nnnn` block) is a
//! `UCFX` container:
//!
//! ```text
//! INFO  0x20 bytes
//!       0x00  f32 x 3  bounding box minimum, cell space
//!       0x0C  f32 x 3  bounding box maximum (x and z are ±200)
//!       0x18  u32      unknown
//!       0x1C  u32      patch count (16)
//! MTRL  terrain materials: model-style records, some followed by blend
//!       and projection data whose layout is not decoded yet (kept raw)
//! GEOM/ one per patch, each 100 m × 100 m:
//!       INFO  0x2C bytes: u32 group count, f32 x 4 bounding sphere,
//!             f32 x 6 bounding box (patch space)
//!       POFF  f32 x 3 patch origin in cell space
//!       PRMG/ one per group: INFO, PRMT, STRM/, IBUF/ (below)
//! ```
//!
//! A low-resolution tile (type hash `1602815c`, all in `low_res_terrain`)
//! is one group covering a whole cell:
//!
//! ```text
//! INFO  u32 (1)
//! MTRL  material records
//! GEOM/ BNDS  f32 x 4 bounding sphere, f32 x 6 bounding box (cell space)
//!       PRMT, STRM/, IBUF/ as below
//! ```
//!
//! Group parts:
//!
//! ```text
//! INFO  0x2C bytes: u32 material range count, u32 full range count, then
//!       u32 x 9 unknown (terrain cells only)
//! PRMT  20 bytes per range:
//!       0x00  u32  material name hash (an MTRL record's)
//!       0x04  u32  unknown, small
//!       0x08  u32  first index
//!       0x0C  u16  primitive count (strip triangles, degenerates included)
//!       0x0E  u16  minimum vertex index
//!       0x10  u16  maximum vertex index
//!       0x12  u16  vertex count
//! STRM/ info {u32 declaration records, u32 stride, u32 vertices}, decl
//!       (see [`vertex`](crate::vertex)), data: half-float positions,
//!       colours and half-float normals
//! IBUF/ info {u32 n}, data u16 indices (a triangle strip)
//! ```
//!
//! Cells carry no world position. The entity layer that places them (see
//! [`layer`](crate::layer)) gives each cell's centre; the 400 cells form a
//! 20 × 20 grid centred on the origin.
//!
//! # Provenance
//!
//! Format-from-samples: inferred from the 400 cells (8,859 groups) and 400
//! low-resolution tiles in the archives of a legally installed copy; every
//! range lies inside its group's indices and vertices. The cell grid was
//! cross-checked by matching the heights along neighbouring cells' edges,
//! which agree with the layer's positions. No bytes from the archives are
//! committed.

use crate::bytes::{OutOfBounds, Reader};
use crate::model::{Aabb, Material, Sphere, read_material, strip_triangles};
use crate::ucfx::{Ucfx, UcfxError};
use crate::vertex::{DeclUsage, VertexDecl, VertexError, VertexStream};

/// Asset type hash of terrain cells.
pub const TERRAIN_TYPE: u32 = 0x7c56_9307;
/// Asset type hash of low-resolution terrain tiles.
pub const LOW_RES_TYPE: u32 = 0x1602_815c;
/// Side of a cell in metres.
pub const CELL_SIZE: f32 = 400.0;

const RANGE_LEN: usize = 20;

/// Errors from parsing terrain.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TerrainError {
    #[error("container: {0}")]
    Ucfx(#[from] UcfxError),
    #[error("truncated: needed {len} bytes at {offset:#x}")]
    Truncated { offset: usize, len: usize },
    #[error("missing {0} section")]
    Missing(&'static str),
    #[error("{what} is {len:#x} bytes, expected {expected:#x}")]
    Length {
        what: &'static str,
        len: usize,
        expected: usize,
    },
    #[error("INFO says {expected} patches, found {found}")]
    PatchCount { expected: u32, found: usize },
    #[error("patch {patch} says {expected} groups, found {found}")]
    GroupCount {
        patch: usize,
        expected: u32,
        found: usize,
    },
    #[error("group {group}: {source}")]
    Vertex { group: usize, source: VertexError },
    #[error("group {group} has no position element")]
    NoPositions { group: usize },
    #[error("group {group}: index {value} is past the {vertices} vertices")]
    Index {
        group: usize,
        value: u16,
        vertices: u32,
    },
    #[error("group {group}: range {range} is outside the group's indices or vertices")]
    Range { group: usize, range: usize },
}

impl From<OutOfBounds> for TerrainError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// One `PRMT` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainRange {
    pub material_hash: u32,
    pub unk_0x04: u32,
    pub start_index: u32,
    pub primitive_count: u16,
    pub min_vertex: u16,
    pub max_vertex: u16,
    pub vertex_count: u16,
}

/// A group of strips sharing one vertex buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct TerrainGroup<'a> {
    /// The 9 unknown words after the range counts (terrain cells only).
    pub unk_info: Vec<u32>,
    pub material_ranges: Vec<TerrainRange>,
    /// Ranges drawing the group with no particular material.
    pub full_ranges: Vec<TerrainRange>,
    pub decl: VertexDecl,
    pub vertices: VertexStream<'a>,
    pub indices: Vec<u16>,
}

impl TerrainGroup<'_> {
    /// Vertex positions, in the space of the patch (cells) or cell (tiles).
    pub fn positions(&self) -> Vec<[f32; 3]> {
        self.decl
            .find(DeclUsage::Position, 0)
            .and_then(|e| self.vertices.read(e))
            .unwrap_or_default()
            .into_iter()
            .map(|p| [p[0], p[1], p[2]])
            .collect()
    }

    /// A range's triangles, Direct3D winding (clockwise front faces).
    pub fn triangles(&self, range: &TerrainRange) -> Vec<[u16; 3]> {
        strip_triangles(
            &self.indices,
            range.start_index,
            u32::from(range.primitive_count) + 2,
        )
    }
}

/// A 100 m patch of a cell.
#[derive(Debug, Clone, PartialEq)]
pub struct Patch<'a> {
    pub sphere: Sphere,
    pub bounds: Aabb,
    /// Patch origin in cell space.
    pub offset: [f32; 3],
    pub groups: Vec<TerrainGroup<'a>>,
}

/// A terrain cell.
#[derive(Debug, Clone, PartialEq)]
pub struct TerrainCell<'a> {
    pub bounds: Aabb,
    pub unk_0x18: u32,
    /// The `MTRL` section, undecoded: its first record follows the model
    /// layout, but later ones carry extra data of varying shape.
    pub material_data: &'a [u8],
    pub patches: Vec<Patch<'a>>,
}

/// A low-resolution terrain tile.
#[derive(Debug, Clone, PartialEq)]
pub struct LowResTile<'a> {
    pub info: u32,
    pub sphere: Sphere,
    pub bounds: Aabb,
    pub materials: Vec<Material>,
    pub group: TerrainGroup<'a>,
}

fn f32x<const N: usize>(r: &mut Reader<'_>) -> Result<[f32; N], OutOfBounds> {
    let mut out = [0.0; N];
    for v in &mut out {
        *v = f32::from_bits(r.u32()?);
    }
    Ok(out)
}

fn sphere_and_box(r: &mut Reader<'_>) -> Result<(Sphere, Aabb), OutOfBounds> {
    let s: [f32; 4] = f32x(r)?;
    let b: [f32; 6] = f32x(r)?;
    Ok((
        Sphere {
            center: [s[0], s[1], s[2]],
            radius: s[3],
        },
        Aabb {
            min: [b[0], b[1], b[2]],
            max: [b[3], b[4], b[5]],
        },
    ))
}

fn materials(mtrl: &[u8]) -> Result<Vec<Material>, TerrainError> {
    let mut r = Reader::at(mtrl, 0);
    let mut out = Vec::new();
    while r.pos() < mtrl.len() {
        out.push(read_material(&mut r)?);
    }
    Ok(out)
}

fn exact(what: &'static str, data: &[u8], expected: usize) -> Result<(), TerrainError> {
    if data.len() != expected {
        return Err(TerrainError::Length {
            what,
            len: data.len(),
            expected,
        });
    }
    Ok(())
}

/// The leaf data of the direct child `fourcc` of section `parent`.
fn child<'a>(u: &Ucfx<'a>, parent: usize, fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    u.children(Some(parent))
        .map(|i| &u.sections()[i])
        .find(|s| &s.fourcc == fourcc)
        .and_then(|s| s.data)
}

fn child_index(u: &Ucfx<'_>, parent: usize, fourcc: &[u8; 4]) -> Option<usize> {
    u.children(Some(parent))
        .find(|&i| &u.sections()[i].fourcc == fourcc)
}

/// Parses a group from the section holding its parts (`PRMG` in cells,
/// `GEOM` in tiles). `info` is the group INFO, absent in tiles.
fn group<'a>(
    u: &Ucfx<'a>,
    at: usize,
    info: Option<&[u8]>,
    id: usize,
) -> Result<TerrainGroup<'a>, TerrainError> {
    let prmt = child(u, at, b"PRMT").ok_or(TerrainError::Missing("PRMT"))?;
    let (n0, unk_info) = match info {
        Some(info) => {
            exact("group INFO", info, 0x2C)?;
            let mut r = Reader::at(info, 0);
            let (n0, n1) = (r.u32()? as usize, r.u32()? as usize);
            exact("PRMT", prmt, (n0 + n1) * RANGE_LEN)?;
            (n0, (0..9).map(|_| r.u32()).collect::<Result<_, _>>()?)
        }
        // Tiles: every range is a material range.
        None => {
            if !prmt.len().is_multiple_of(RANGE_LEN) {
                return Err(TerrainError::Length {
                    what: "PRMT",
                    len: prmt.len(),
                    expected: prmt.len() / RANGE_LEN * RANGE_LEN,
                });
            }
            (prmt.len() / RANGE_LEN, Vec::new())
        }
    };
    let strm = child_index(u, at, b"STRM").ok_or(TerrainError::Missing("STRM"))?;
    let sinfo = child(u, strm, b"info").ok_or(TerrainError::Missing("STRM/info"))?;
    let decl = child(u, strm, b"decl").ok_or(TerrainError::Missing("STRM/decl"))?;
    let data = child(u, strm, b"data").ok_or(TerrainError::Missing("STRM/data"))?;
    let mut r = Reader::at(sinfo, 0);
    let (_, stride, count) = (r.u32()?, r.u32()?, r.u32()?);
    let vertex = |source| TerrainError::Vertex { group: id, source };
    let decl = VertexDecl::parse(decl).map_err(vertex)?;
    let vertices = VertexStream::new(data, stride, count).map_err(vertex)?;
    if decl.find(DeclUsage::Position, 0).is_none() {
        return Err(TerrainError::NoPositions { group: id });
    }

    let ibuf = child_index(u, at, b"IBUF").ok_or(TerrainError::Missing("IBUF"))?;
    let iinfo = child(u, ibuf, b"info").ok_or(TerrainError::Missing("IBUF/info"))?;
    let idata = child(u, ibuf, b"data").ok_or(TerrainError::Missing("IBUF/data"))?;
    let n = Reader::at(iinfo, 0).u32()? as usize;
    exact("IBUF/data", idata, n.saturating_mul(2))?;
    let indices: Vec<u16> = idata
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    if let Some(&value) = indices.iter().find(|&&i| u32::from(i) >= count) {
        return Err(TerrainError::Index {
            group: id,
            value,
            vertices: count,
        });
    }

    let mut ranges = Vec::with_capacity(prmt.len() / RANGE_LEN);
    let mut r = Reader::at(prmt, 0);
    for range in 0..prmt.len() / RANGE_LEN {
        let x = TerrainRange {
            material_hash: r.u32()?,
            unk_0x04: r.u32()?,
            start_index: r.u32()?,
            primitive_count: r.u16()?,
            min_vertex: r.u16()?,
            max_vertex: r.u16()?,
            vertex_count: r.u16()?,
        };
        let end = u64::from(x.start_index) + u64::from(x.primitive_count) + 2;
        if end > indices.len() as u64
            || u32::from(x.max_vertex) >= count
            || x.min_vertex > x.max_vertex
        {
            return Err(TerrainError::Range { group: id, range });
        }
        ranges.push(x);
    }
    let full_ranges = ranges.split_off(n0.min(ranges.len()));
    Ok(TerrainGroup {
        unk_info,
        material_ranges: ranges,
        full_ranges,
        decl,
        vertices,
        indices,
    })
}

fn top<'a>(u: &Ucfx<'a>, fourcc: &[u8; 4], name: &'static str) -> Result<&'a [u8], TerrainError> {
    u.top_level(*fourcc).ok_or(TerrainError::Missing(name))
}

impl<'a> TerrainCell<'a> {
    pub fn parse(asset: &'a [u8]) -> Result<Self, TerrainError> {
        let u = Ucfx::parse(asset)?;
        let info = top(&u, b"INFO", "INFO")?;
        exact("INFO", info, 0x20)?;
        let mut r = Reader::at(info, 0);
        let b: [f32; 6] = f32x(&mut r)?;
        let bounds = Aabb {
            min: [b[0], b[1], b[2]],
            max: [b[3], b[4], b[5]],
        };
        let unk_0x18 = r.u32()?;
        let patch_count = r.u32()?;
        let material_data = top(&u, b"MTRL", "MTRL")?;

        let geoms: Vec<usize> = u
            .children(None)
            .filter(|&i| &u.sections()[i].fourcc == b"GEOM")
            .collect();
        if geoms.len() != patch_count as usize {
            return Err(TerrainError::PatchCount {
                expected: patch_count,
                found: geoms.len(),
            });
        }
        let mut patches = Vec::with_capacity(geoms.len());
        let mut group_id = 0;
        for (patch, &g) in geoms.iter().enumerate() {
            let info = child(&u, g, b"INFO").ok_or(TerrainError::Missing("GEOM/INFO"))?;
            exact("GEOM/INFO", info, 0x2C)?;
            let mut r = Reader::at(info, 0);
            let groups = r.u32()?;
            let (sphere, bounds) = sphere_and_box(&mut r)?;
            let poff = child(&u, g, b"POFF").ok_or(TerrainError::Missing("POFF"))?;
            exact("POFF", poff, 12)?;
            let offset = f32x(&mut Reader::at(poff, 0))?;
            let prmgs: Vec<usize> = u
                .children(Some(g))
                .filter(|&i| &u.sections()[i].fourcc == b"PRMG")
                .collect();
            if prmgs.len() != groups as usize {
                return Err(TerrainError::GroupCount {
                    patch,
                    expected: groups,
                    found: prmgs.len(),
                });
            }
            let mut parsed = Vec::with_capacity(prmgs.len());
            for p in prmgs {
                let info = child(&u, p, b"INFO").ok_or(TerrainError::Missing("PRMG/INFO"))?;
                parsed.push(group(&u, p, Some(info), group_id)?);
                group_id += 1;
            }
            patches.push(Patch {
                sphere,
                bounds,
                offset,
                groups: parsed,
            });
        }
        Ok(Self {
            bounds,
            unk_0x18,
            material_data,
            patches,
        })
    }
}

impl<'a> LowResTile<'a> {
    pub fn parse(asset: &'a [u8]) -> Result<Self, TerrainError> {
        let u = Ucfx::parse(asset)?;
        let info = Reader::at(top(&u, b"INFO", "INFO")?, 0).u32()?;
        let materials = materials(top(&u, b"MTRL", "MTRL")?)?;
        let geom = u
            .children(None)
            .find(|&i| u.sections()[i].fourcc == *b"GEOM")
            .ok_or(TerrainError::Missing("GEOM"))?;
        let bnds = child(&u, geom, b"BNDS").ok_or(TerrainError::Missing("BNDS"))?;
        exact("BNDS", bnds, 0x28)?;
        let (sphere, bounds) = sphere_and_box(&mut Reader::at(bnds, 0))?;
        Ok(Self {
            info,
            sphere,
            bounds,
            materials,
            group: group(&u, geom, None, 0)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{UcfxNode, ucfx};

    fn le(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    fn floats(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    /// A material record with one texture.
    fn material(hash: u32) -> Vec<u8> {
        let mut m = hash.to_le_bytes().to_vec();
        m.extend(floats(&[1.0; 25]));
        m.extend(0u16.to_le_bytes());
        m.extend(1u16.to_le_bytes());
        m.extend(le(&[0xAAAA, 0x1234, 0x959d_8470]));
        m
    }

    fn range(material: u32, start: u32, prims: u16, min: u16, max: u16) -> Vec<u8> {
        let mut r = le(&[material, 7, start]);
        for v in [prims, min, max, max - min + 1] {
            r.extend(v.to_le_bytes());
        }
        r
    }

    /// Float3 positions, 4 vertices of a quad, one strip of 4 indices.
    fn group_parts(ranges: Vec<u8>) -> Vec<UcfxNode> {
        let decl = vec![
            0, 0, 0, 0, 2, 0, 0, 0, // stream 0 offset 0 FLOAT3 POSITION
            0xFF, 0, 0, 0, 17, 0, 0, 0, // end
        ];
        let pos = floats(&[
            0.0, 1.0, 0.0, 10.0, 1.0, 0.0, 0.0, 2.0, 10.0, 10.0, 3.0, 10.0,
        ]);
        vec![
            UcfxNode::Leaf(*b"PRMT", ranges),
            UcfxNode::Container(
                *b"STRM",
                vec![
                    UcfxNode::Leaf(*b"info", le(&[2, 12, 4])),
                    UcfxNode::Leaf(*b"decl", decl),
                    UcfxNode::Leaf(*b"data", pos),
                ],
            ),
            UcfxNode::Container(
                *b"IBUF",
                vec![
                    UcfxNode::Leaf(*b"info", le(&[4])),
                    UcfxNode::Leaf(
                        *b"data",
                        [0u16, 1, 2, 3]
                            .iter()
                            .flat_map(|i| i.to_le_bytes())
                            .collect(),
                    ),
                ],
            ),
        ]
    }

    fn patch(offset: [f32; 3]) -> UcfxNode {
        let mut info = le(&[1]);
        info.extend(floats(&[
            0.0, 0.0, 0.0, 70.0, -50.0, 0.0, -50.0, 50.0, 5.0, 50.0,
        ]));
        let mut prmg = vec![UcfxNode::Leaf(*b"INFO", {
            let mut i = le(&[1, 1]);
            i.extend(le(&[0; 9]));
            i
        })];
        let mut ranges = range(0x11, 0, 2, 0, 3);
        ranges.extend(range(0x22, 0, 2, 0, 3));
        prmg.extend(group_parts(ranges));
        UcfxNode::Container(
            *b"GEOM",
            vec![
                UcfxNode::Leaf(*b"INFO", info),
                UcfxNode::Leaf(*b"POFF", floats(&offset)),
                UcfxNode::Container(*b"PRMG", prmg),
            ],
        )
    }

    fn cell() -> Vec<u8> {
        let mut info = floats(&[-200.0, -10.0, -200.0, 200.0, 50.0, 200.0]);
        info.extend(le(&[0x10b, 2]));
        let mut mtrl = material(0x11);
        mtrl.extend(material(0x22));
        ucfx(&[
            UcfxNode::Leaf(*b"INFO", info),
            UcfxNode::Leaf(*b"MTRL", mtrl),
            patch([-150.0, 0.0, -150.0]),
            patch([-50.0, 0.0, -150.0]),
        ])
    }

    fn tile() -> Vec<u8> {
        let mut bnds = floats(&[0.0, 0.0, 0.0, 290.0]);
        bnds.extend(floats(&[-200.0, -5.0, -200.0, 200.0, 20.0, 200.0]));
        let mut geom = vec![UcfxNode::Leaf(*b"BNDS", bnds)];
        geom.extend(group_parts(range(0x33, 0, 2, 0, 3)));
        ucfx(&[
            UcfxNode::Leaf(*b"INFO", le(&[1])),
            UcfxNode::Leaf(*b"MTRL", material(0x33)),
            UcfxNode::Container(*b"GEOM", geom),
        ])
    }

    #[test]
    fn parses_cells() {
        let a = cell();
        let c = TerrainCell::parse(&a).unwrap();
        assert_eq!(c.bounds.max, [200.0, 50.0, 200.0]);
        assert_eq!(c.unk_0x18, 0x10b);
        assert_eq!(c.material_data.len(), 2 * 0x78);
        assert_eq!(c.patches.len(), 2);
        let p = &c.patches[1];
        assert_eq!(p.offset, [-50.0, 0.0, -150.0]);
        assert_eq!(p.sphere.radius, 70.0);
        let g = &p.groups[0];
        assert_eq!(g.material_ranges.len(), 1);
        assert_eq!(g.full_ranges.len(), 1);
        assert_eq!(g.material_ranges[0].material_hash, 0x11);
        assert_eq!(g.material_ranges[0].vertex_count, 4);
        assert_eq!(g.positions()[3], [10.0, 3.0, 10.0]);
        assert_eq!(g.triangles(&g.material_ranges[0]), [[0, 1, 2], [2, 1, 3]]);
    }

    #[test]
    fn parses_tiles() {
        let a = tile();
        let t = LowResTile::parse(&a).unwrap();
        assert_eq!(t.info, 1);
        assert_eq!(t.sphere.radius, 290.0);
        assert_eq!(t.bounds.min, [-200.0, -5.0, -200.0]);
        assert_eq!(t.group.material_ranges.len(), 1);
        assert!(t.group.full_ranges.is_empty());
        assert_eq!(t.group.positions().len(), 4);
    }

    #[test]
    fn rejects_bad_ranges_and_counts() {
        let mut a = cell();
        // Patch count 2 -> 3.
        let at = a
            .windows(4)
            .position(|w| w == 0x10bu32.to_le_bytes())
            .unwrap()
            + 4;
        a[at] = 3;
        assert_eq!(
            TerrainCell::parse(&a).unwrap_err(),
            TerrainError::PatchCount {
                expected: 3,
                found: 2
            }
        );
        // A range reading past the indices.
        let mut a = tile();
        let at = a
            .windows(4)
            .position(|w| w == 0x33u32.to_le_bytes())
            .unwrap();
        let at = a[at + 4..]
            .windows(4)
            .position(|w| w == 0x33u32.to_le_bytes())
            .map(|p| p + at + 4)
            .unwrap();
        a[at + 12] = 9;
        assert_eq!(
            LowResTile::parse(&a).unwrap_err(),
            TerrainError::Range { group: 0, range: 0 }
        );
    }

    #[test]
    fn byte_flips_never_panic() {
        for a in [cell(), tile()] {
            for at in 0..a.len() {
                for v in [0, 1, 0x7f, 0xff] {
                    let mut b = a.clone();
                    b[at] = v;
                    if let Ok(c) = TerrainCell::parse(&b) {
                        for g in c.patches.iter().flat_map(|p| &p.groups) {
                            let _ = g.positions();
                            for r in g.material_ranges.iter().chain(&g.full_ranges) {
                                let _ = g.triangles(r);
                            }
                        }
                    }
                    if let Ok(t) = LowResTile::parse(&b) {
                        let _ = t.group.positions();
                    }
                }
            }
        }
    }
}
