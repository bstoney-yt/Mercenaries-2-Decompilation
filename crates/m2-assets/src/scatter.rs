//! Scatter: the grass, shrub and undergrowth clumps instanced across each
//! terrain cell.
//!
//! A scatter asset (type hash `600b904e`, several per `c3nnnn` cell block)
//! is a `UCFX` container:
//!
//! ```text
//! INFO  u32 mesh count, u32 patch count, u32 run count, u32 instance count
//! SCRB/ one per mesh:
//!       INFO  0x14 bytes: u32 hash, u32 hash, f32 fade start (m),
//!             f32 fade end (m), f32 unknown
//!       MTRL  one material record as in models
//!       STRM/ info, decl, data (half-float position, UV and normal)
//!       IBUF/ info {u32 n}, data u16 indices: one triangle strip
//! INST  24 bytes per instance: half x 12, a 3 × 4 matrix in rows
//!       (rotation × uniform scale | translation from the patch centre)
//! PTCH  56 bytes per patch:
//!       0x00  f32 x 3  patch centre, cell space
//!       0x0C  f32 x 10 unknown (bounds, probably)
//!       0x34  u16      first run
//!       0x36  u16      run count
//! PTMS  8 bytes per run: u32 first instance, u16 instance count, u16 mesh
//! ```
//!
//! Patches cover the runs in order and runs cover the instances in order.
//! Six assets are empty containers.
//!
//! # Provenance
//!
//! Format-from-samples: inferred from the 1,026 scatter assets in the
//! archives of a legally installed copy: every count matches its section's
//! length, patches tile the runs and runs tile the instances, and all
//! 1,767,683 instance matrices are rotations with a uniform scale whose
//! translation stays within 100 m of the patch. No bytes from the archives
//! are committed.

use crate::bytes::{OutOfBounds, Reader};
use crate::model::{Material, read_material};
use crate::ucfx::{Ucfx, UcfxError};
use crate::vertex::{DeclUsage, VertexDecl, VertexError, VertexStream, half_to_f32};

/// Asset type hash of scatter.
pub const SCATTER_TYPE: u32 = 0x600b_904e;

const PATCH_LEN: usize = 56;
const RUN_LEN: usize = 8;
const INSTANCE_LEN: usize = 24;

/// Errors from parsing scatter.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ScatterError {
    #[error("container: {0}")]
    Ucfx(#[from] UcfxError),
    #[error("truncated: needed {len} bytes at {offset:#x}")]
    Truncated { offset: usize, len: usize },
    #[error("missing {0} section")]
    Missing(&'static str),
    #[error("{what} is {len:#x} bytes for {count} records")]
    Length {
        what: &'static str,
        len: usize,
        count: u32,
    },
    #[error("INFO says {expected} meshes, found {found}")]
    MeshCount { expected: u32, found: usize },
    #[error("mesh {mesh}: {source}")]
    Vertex { mesh: usize, source: VertexError },
    #[error("mesh {mesh}: index {value} is past the {vertices} vertices")]
    Index {
        mesh: usize,
        value: u16,
        vertices: u32,
    },
    #[error("patch {0} does not continue the runs in order")]
    PatchRuns(usize),
    #[error("run {0} does not continue the instances in order, or names a missing mesh")]
    Run(usize),
}

impl From<OutOfBounds> for ScatterError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// One clump mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct ScatterMesh<'a> {
    pub hash: u32,
    pub unk_0x04: u32,
    /// Distances (m) over which instances fade out.
    pub fade: [f32; 2],
    pub unk_0x10: f32,
    pub material: Material,
    pub decl: VertexDecl,
    pub vertices: VertexStream<'a>,
    /// One triangle strip.
    pub indices: Vec<u16>,
}

impl ScatterMesh<'_> {
    pub fn positions(&self) -> Vec<[f32; 3]> {
        self.read(DeclUsage::Position)
            .into_iter()
            .map(|p| [p[0], p[1], p[2]])
            .collect()
    }

    /// Any element, as four floats per vertex (empty if absent).
    pub fn read(&self, usage: DeclUsage) -> Vec<[f32; 4]> {
        self.decl
            .find(usage, 0)
            .and_then(|e| self.vertices.read(e))
            .unwrap_or_default()
    }

    /// The strip's triangles, Direct3D winding.
    pub fn triangles(&self) -> Vec<[u16; 3]> {
        crate::model::strip_triangles(&self.indices, 0, self.indices.len() as u32)
    }
}

/// A run of instances of one mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    pub first: u32,
    pub count: u16,
    pub mesh: u16,
}

/// A patch: a centre and the runs placed around it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScatterPatch {
    /// Cell space.
    pub centre: [f32; 3],
    pub unk: [f32; 10],
    pub first_run: u16,
    pub run_count: u16,
}

/// A parsed scatter asset.
#[derive(Debug, Clone, PartialEq)]
pub struct Scatter<'a> {
    pub meshes: Vec<ScatterMesh<'a>>,
    pub patches: Vec<ScatterPatch>,
    pub runs: Vec<Run>,
    /// Per instance, a row-major 3 × 4 matrix: rotation × scale and the
    /// translation from its patch's centre.
    pub instances: Vec<[f32; 12]>,
}

fn exact(what: &'static str, data: &[u8], count: u32, len: usize) -> Result<(), ScatterError> {
    if data.len() as u64 != u64::from(count) * len as u64 {
        return Err(ScatterError::Length {
            what,
            len: data.len(),
            count,
        });
    }
    Ok(())
}

impl<'a> Scatter<'a> {
    /// Parses a scatter asset; an empty container gives an empty scatter.
    pub fn parse(asset: &'a [u8]) -> Result<Self, ScatterError> {
        let u = Ucfx::parse(asset)?;
        let Some(info) = u.top_level(*b"INFO") else {
            if u.sections().is_empty() {
                return Ok(Self {
                    meshes: Vec::new(),
                    patches: Vec::new(),
                    runs: Vec::new(),
                    instances: Vec::new(),
                });
            }
            return Err(ScatterError::Missing("INFO"));
        };
        let mut r = Reader::at(info, 0);
        let (nmeshes, npatches, nruns, ninstances) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);

        let scrbs: Vec<usize> = u
            .children(None)
            .filter(|&i| &u.sections()[i].fourcc == b"SCRB")
            .collect();
        if scrbs.len() != nmeshes as usize {
            return Err(ScatterError::MeshCount {
                expected: nmeshes,
                found: scrbs.len(),
            });
        }
        let part = |parent: usize, cc: &[u8; 4], name| {
            u.children(Some(parent))
                .map(|i| &u.sections()[i])
                .find(|s| &s.fourcc == cc)
                .and_then(|s| s.data)
                .ok_or(ScatterError::Missing(name))
        };
        let index_of = |parent: usize, cc: &[u8; 4], name| {
            u.children(Some(parent))
                .find(|&i| &u.sections()[i].fourcc == cc)
                .ok_or(ScatterError::Missing(name))
        };
        let mut meshes = Vec::with_capacity(scrbs.len());
        for (mesh, &s) in scrbs.iter().enumerate() {
            let mut r = Reader::at(part(s, b"INFO", "SCRB/INFO")?, 0);
            let (hash, unk_0x04) = (r.u32()?, r.u32()?);
            let fade = [f32::from_bits(r.u32()?), f32::from_bits(r.u32()?)];
            let unk_0x10 = f32::from_bits(r.u32()?);
            let material = read_material(&mut Reader::at(part(s, b"MTRL", "SCRB/MTRL")?, 0))?;
            let strm = index_of(s, b"STRM", "SCRB/STRM")?;
            let mut r = Reader::at(part(strm, b"info", "STRM/info")?, 0);
            let (_, stride, count) = (r.u32()?, r.u32()?, r.u32()?);
            let vertex = |source| ScatterError::Vertex { mesh, source };
            let decl = VertexDecl::parse(part(strm, b"decl", "STRM/decl")?).map_err(vertex)?;
            let vertices = VertexStream::new(part(strm, b"data", "STRM/data")?, stride, count)
                .map_err(vertex)?;
            let ibuf = index_of(s, b"IBUF", "SCRB/IBUF")?;
            let n = Reader::at(part(ibuf, b"info", "IBUF/info")?, 0).u32()?;
            let idata = part(ibuf, b"data", "IBUF/data")?;
            exact("IBUF/data", idata, n, 2)?;
            let indices: Vec<u16> = idata
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            if let Some(&value) = indices.iter().find(|&&i| u32::from(i) >= count) {
                return Err(ScatterError::Index {
                    mesh,
                    value,
                    vertices: count,
                });
            }
            meshes.push(ScatterMesh {
                hash,
                unk_0x04,
                fade,
                unk_0x10,
                material,
                decl,
                vertices,
                indices,
            });
        }

        let top = |cc: [u8; 4], name| u.top_level(cc).ok_or(ScatterError::Missing(name));
        let ptch = top(*b"PTCH", "PTCH")?;
        let ptms = top(*b"PTMS", "PTMS")?;
        let inst = top(*b"INST", "INST")?;
        exact("PTCH", ptch, npatches, PATCH_LEN)?;
        exact("PTMS", ptms, nruns, RUN_LEN)?;
        exact("INST", inst, ninstances, INSTANCE_LEN)?;

        let mut runs = Vec::with_capacity(nruns as usize);
        let mut next = 0u64;
        for (i, c) in ptms.chunks_exact(RUN_LEN).enumerate() {
            let run = Run {
                first: u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                count: u16::from_le_bytes([c[4], c[5]]),
                mesh: u16::from_le_bytes([c[6], c[7]]),
            };
            if u64::from(run.first) != next || usize::from(run.mesh) >= meshes.len() {
                return Err(ScatterError::Run(i));
            }
            next = u64::from(run.first) + u64::from(run.count);
            runs.push(run);
        }
        if next != u64::from(ninstances) {
            return Err(ScatterError::Run(runs.len()));
        }

        let mut patches = Vec::with_capacity(npatches as usize);
        let mut next = 0usize;
        for (i, c) in ptch.chunks_exact(PATCH_LEN).enumerate() {
            let mut r = Reader::at(c, 0);
            let mut f = [0.0; 13];
            for v in &mut f {
                *v = f32::from_bits(r.u32()?);
            }
            let (first_run, run_count) = (r.u16()?, r.u16()?);
            if usize::from(first_run) != next {
                return Err(ScatterError::PatchRuns(i));
            }
            next = usize::from(first_run) + usize::from(run_count);
            let mut unk = [0.0; 10];
            unk.copy_from_slice(&f[3..]);
            patches.push(ScatterPatch {
                centre: [f[0], f[1], f[2]],
                unk,
                first_run,
                run_count,
            });
        }
        if next != runs.len() {
            return Err(ScatterError::PatchRuns(patches.len()));
        }

        let instances = inst
            .chunks_exact(INSTANCE_LEN)
            .map(|c| {
                std::array::from_fn(|i| half_to_f32(u16::from_le_bytes([c[2 * i], c[2 * i + 1]])))
            })
            .collect();
        Ok(Self {
            meshes,
            patches,
            runs,
            instances,
        })
    }

    /// Every instance as (mesh index, patch centre, matrix).
    pub fn placements(&self) -> impl Iterator<Item = (u16, [f32; 3], &[f32; 12])> + '_ {
        self.patches.iter().flat_map(move |p| {
            let runs = &self.runs
                [usize::from(p.first_run)..usize::from(p.first_run) + usize::from(p.run_count)];
            runs.iter().flat_map(move |r| {
                let first = r.first as usize;
                self.instances[first..first + usize::from(r.count)]
                    .iter()
                    .map(move |m| (r.mesh, p.centre, m))
            })
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

    fn halves(v: &[f32]) -> Vec<u8> {
        // Exact halves for the small values used here.
        v.iter()
            .flat_map(|&f| {
                let h: u16 = match f {
                    0.0 => 0,
                    1.0 => 0x3c00,
                    -1.0 => 0xbc00,
                    2.0 => 0x4000,
                    0.5 => 0x3800,
                    _ => panic!("no half for {f}"),
                };
                h.to_le_bytes()
            })
            .collect()
    }

    fn mesh() -> UcfxNode {
        let mut info = le(&[0x1111, 0x2222]);
        info.extend([50.0f32, 70.0, 0.1].iter().flat_map(|f| f.to_le_bytes()));
        let mut mtrl = 0x3333u32.to_le_bytes().to_vec();
        mtrl.extend([1.0f32; 25].iter().flat_map(|f| f.to_le_bytes()));
        mtrl.extend(0u16.to_le_bytes());
        mtrl.extend(1u16.to_le_bytes());
        mtrl.extend(le(&[0x4444, 0, 0x959d_8470]));
        let decl = vec![0, 0, 0, 0, 2, 0, 0, 0, 0xFF, 0, 0, 0, 17, 0, 0, 0];
        let pos: Vec<u8> = [
            0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0,
        ]
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();
        UcfxNode::Container(
            *b"SCRB",
            vec![
                UcfxNode::Leaf(*b"INFO", info),
                UcfxNode::Leaf(*b"MTRL", mtrl),
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
            ],
        )
    }

    fn patch(centre: [f32; 3], first: u16, count: u16) -> Vec<u8> {
        let mut p: Vec<u8> = centre
            .iter()
            .chain(&[0.0; 10])
            .flat_map(|f| f.to_le_bytes())
            .collect();
        p.extend(first.to_le_bytes());
        p.extend(count.to_le_bytes());
        p
    }

    fn asset() -> Vec<u8> {
        let mut ptch = patch([10.0, -14.0, 20.0], 0, 1);
        ptch.extend(patch([-5.0, 3.0, 0.0], 1, 1));
        let mut ptms = le(&[0]);
        ptms.extend([2u16, 0].iter().flat_map(|v| v.to_le_bytes()));
        ptms.extend(le(&[2]));
        ptms.extend([1u16, 0].iter().flat_map(|v| v.to_le_bytes()));
        // Identity with a 2 m offset, a half-scale one, and a quarter turn.
        let mut inst = halves(&[1.0, 0.0, 0.0, 2.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
        inst.extend(halves(&[
            0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0,
        ]));
        inst.extend(halves(&[
            0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 1.0,
        ]));
        ucfx(&[
            UcfxNode::Leaf(*b"INFO", le(&[1, 2, 2, 3])),
            mesh(),
            UcfxNode::Leaf(*b"INST", inst),
            UcfxNode::Leaf(*b"PTCH", ptch),
            UcfxNode::Leaf(*b"PTMS", ptms),
        ])
    }

    #[test]
    fn parses_scatter() {
        let a = asset();
        let s = Scatter::parse(&a).unwrap();
        assert_eq!(s.meshes.len(), 1);
        let m = &s.meshes[0];
        assert_eq!((m.hash, m.fade), (0x1111, [50.0, 70.0]));
        assert_eq!(m.material.textures, [0x4444]);
        assert_eq!(m.positions()[3], [1.0, 1.0, 0.0]);
        assert_eq!(m.triangles(), [[0, 1, 2], [2, 1, 3]]);
        assert_eq!(
            s.runs[1],
            Run {
                first: 2,
                count: 1,
                mesh: 0
            }
        );
        assert_eq!(s.instances[0][3], 2.0);
        let all: Vec<_> = s.placements().collect();
        assert_eq!(all.len(), 3);
        assert_eq!(all[2].1, [-5.0, 3.0, 0.0]);
        assert_eq!(all[2].2[2], 1.0);
    }

    #[test]
    fn empty_and_inconsistent_assets() {
        let empty = ucfx(&[]);
        assert!(Scatter::parse(&empty).unwrap().instances.is_empty());
        let mut a = asset();
        // Second run starts at 3 instead of 2.
        let at = a
            .windows(4)
            .rposition(|w| w == le(&[2]).as_slice())
            .unwrap();
        a[at] = 3;
        assert_eq!(Scatter::parse(&a).unwrap_err(), ScatterError::Run(1));
    }

    #[test]
    fn byte_flips_never_panic() {
        let a = asset();
        for at in 0..a.len() {
            for v in [0, 1, 0x7f, 0xff] {
                let mut b = a.clone();
                b[at] = v;
                if let Ok(s) = Scatter::parse(&b) {
                    let _ = s.placements().count();
                    for m in &s.meshes {
                        let _ = (m.positions(), m.triangles());
                    }
                }
            }
        }
    }
}
