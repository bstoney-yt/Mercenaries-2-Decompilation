//! Skeletal animation clips: per-node translation, rotation and scale tracks
//! in one of three encodings, with a sampler that evaluates a pose at any
//! time and poses a model's node hierarchy.
//!
//! An animation asset (type hash `18166555`) is a flat `UCFX` container:
//!
//! ```text
//! info  u16 version (1)
//! data  a packfile (see [`havok`](crate::havok)) holding one animation
//!       container object and one clip object
//! trnm  u16 track count n, u16 root track (0xFFFF = none),
//!       u32 track layout signature (derivation unknown),
//!       u32 x n model HIER node name hash per track
//! evnt  (optional) u32 count, then per event:
//!       f32 time in seconds, cstr name, cstr category
//! ```
//!
//! A further 29 assets of the same type hold material animations in a
//! different layout, parsed by [`material_animation`](crate::material_animation).
//! [`Clip::parse`] reports them as [`AnimationError::MaterialAnimation`].
//!
//! The packfile's contents object is the container:
//!
//! ```text
//! 0x00  {ptr, u32 count} x 5   skeletons, animations, bindings, attachments,
//!                              skins; only animations is used (count 1)
//! ```
//!
//! and its animation array holds one pointer (a global fixup) to the clip.
//! Every clip starts:
//!
//! ```text
//! 0x00  u32 x 2   0 (vtable and reference count in memory)
//! 0x08  u32       encoding: 1 interleaved, 2 delta, 3 wavelet
//! 0x0C  f32       duration in seconds = (frames - 1) / frame rate
//! 0x10  u32       track count, equal to trnm's
//! 0x14  u32       float track count (0)
//! 0x18  ptr       extracted motion (0: root motion stays on the root track)
//! 0x1C  {ptr, u32 count}  annotation tracks (empty; events are in evnt)
//! ```
//!
//! These are Havok's `hkaSkeletalAnimation` fields (`type`, `duration`,
//! `numberOfTransformTracks`, `numberOfFloatTracks`, `extractedMotion`,
//! `annotationTracks`); the compressed layouts below use Havok's names too.
//!
//! Frames are uniform, at 30 or 29.97 per second; no clip stores key times.
//! Each track holds ten values per frame: translation xyz, rotation
//! quaternion xyzw, scale xyz. Translations are the node's local position,
//! not offsets from its bind pose, and the root track carries any root
//! motion.
//!
//! # Interleaved clips
//!
//! ```text
//! 0x24  {ptr, u32 count}  transforms, count = frames * tracks, frame-major,
//!                         48 bytes each: f32 t[4], q[4], s[4]
//!                         (t.w and s.w unused)
//! 0x2C  {ptr, u32 count}  float track values (`floats`), empty on every clip
//! ```
//!
//! # Compressed clips
//!
//! Wavelet and delta clips share a data buffer layout: a `u16` static mask
//! per track at the mask index, `f32` static values, then per dynamic value
//! (a *DOF*) an `f32` offset, an `f32` scale and a `u8` bit width, then the
//! quantized blocks. Each mask splits into a 2-bit type per group (bits 0-1
//! translation, 2-3 rotation, 4-5 scale) and a dynamic flag per component.
//! Each group's flags start where its values do (bit `6 + first`) but run in
//! reverse component order:
//!
//! ```text
//! bits  6-8    translation z, y, x
//! bits  9-12   rotation w, z, y, x
//! bits 13-15   scale z, y, x
//! ```
//!
//! The reversal is invisible to counting checks and to quaternion lengths;
//! it was found by skinning a model with every clip that fits it, which
//! matches the uncompressed clips only with the flags read this way.
//! Group types:
//!
//! ```text
//! type 0  per component: dynamic if its flag is set, otherwise static
//! type 1  every component static
//! type 2  identity: translation 0, rotation (0, 0, 0, 1), scale 1
//! ```
//!
//! Static values and dynamic DOFs are numbered in one walk over the tracks,
//! then translation, rotation and scale, then components. A static rotation
//! component of exactly `+2.0` or `-2.0` is a marker: rebuild that component
//! every frame as `sign * sqrt(1 - sum of the others squared)`.
//!
//! Wavelet clip fields:
//!
//! ```text
//! 0x24  u32  frames                          numberOfPoses
//! 0x28  u32  block length in frames (8)      blockSize
//! 0x2C  u8   largest bit width               qFormat.maxBitWidth
//! 0x2D  u8   preserved values per block (0)  qFormat.preserved
//! 0x30  u32  dynamic DOF count               qFormat.numD
//! 0x34  u32  buffer index of the offsets     qFormat.offsetIdx
//! 0x38  u32  buffer index of the scales      qFormat.scaleIdx
//! 0x3C  u32  buffer index of the bit widths  qFormat.bitWidthIdx
//! 0x40  u32  buffer index of the static masks           staticMaskIdx
//! 0x44  u32  buffer index of the static values          staticDOFsIdx
//! 0x48  u32  buffer index of the block table            blockIndexIdx
//!            (u32 byte offset per block)
//! 0x4C  u32  block count = ceil(frames / 8)             blockIndexSize
//! 0x50  u32  buffer index of the quantized blocks       quantizedDataIdx
//! 0x54  u32  quantized block bytes                      quantizedDataSize
//! 0x58  ptr  buffer                                     dataBuffer
//! 0x5C  u32  buffer length
//! ```
//!
//! Block `b` covers frames `8b` to `8b + 7`, the last padded by repeating
//! the last frame. For each dynamic DOF it holds a `u8` zero mask (bit `j`
//! set: coefficient `j` is not stored), then each stored coefficient as a
//! `bw`-bit integer `q` read least significant bit first, then padding to a
//! byte. A coefficient is the middle of bucket `q`, `offset + scale * (q +
//! 0.5) / 2^bw`; a masked one is the bucket holding 0, `q = trunc(-offset /
//! scale * 2^bw)` kept below `2^bw`, so it decodes to nearly but not exactly
//! 0. A DOF whose range was never set (offset `f32::MAX`, scale `-inf`; 359
//! DOFs, every coefficient masked) decodes to exactly 0, where the bucket
//! arithmetic would give `-inf`. The eight coefficients `[s, d3, d2_0, d2_1, d1_0 .. d1_3]` are a
//! three-level LeGall 5/3 lifting wavelet; see [`inverse_wavelet`].
//!
//! Delta clip fields:
//!
//! ```text
//! 0x24  u32  frames                          numberOfPoses
//! 0x28  u32  block length in frames (8)      blockSize
//! 0x2C  u8   largest bit width               qFormat.maxBitWidth
//! 0x2D  u8   preserved values per block (1)  qFormat.preserved
//! 0x30  u32  dynamic DOF count               qFormat.numD
//! 0x34  u32  buffer index of the offsets     qFormat.offsetIdx
//! 0x38  u32  buffer index of the scales      qFormat.scaleIdx
//! 0x3C  u32  buffer index of the bit widths  qFormat.bitWidthIdx
//! 0x40  u32  buffer index of the quantized blocks       quantizedDataIdx
//! 0x44  u32  quantized block bytes                      quantizedDataSize
//! 0x48  u32  buffer index of the static masks           staticMaskIdx
//! 0x4C  u32  static mask bytes                          staticMaskSize
//! 0x50  u32  buffer index of the static values          staticDOFsIdx
//! 0x54  u32  static value bytes                         staticDOFsSize
//! 0x58  u32  bytes per full block                       totalBlockSize
//! 0x5C  u32  bytes in the last block                    lastBlockSize
//! 0x60  ptr  buffer                                     dataBuffer
//! 0x64  u32  buffer length
//! ```
//!
//! Blocks are back to back. For each dynamic DOF a block holds its first
//! `preserved` values as `f32`, then one `bw`-bit integer `q` per further
//! frame (least significant bit first), then padding to a byte. A quantized
//! value is the middle of bucket `q`, `offset + scale * (q + 0.5) / 2^bw`,
//! and the block's frames are the single-precision running sum of all its
//! values, so every frame is the previous one plus its delta.
//!
//! # Conventions
//!
//! Quaternions are `(x, y, z, w)` as in D3DX. A track's local matrix is
//! `scaling * rotation * translation` in D3DX's row-vector order, which is
//! how the model `HIER` local matrices decompose, and a node's model-space
//! matrix is its local matrix times its parent's.
//!
//! # Provenance
//!
//! Layouts inferred from all 4,232 skeletal clips (4,103 wavelet, 73
//! interleaved, 56 delta) in the archives of a legally installed copy, then
//! checked against Havok's own decoder in the Xbox 360 build of the game,
//! whose clip layouts match these field for field. The field names come
//! from its `hkClass` reflection tables, and the dequantization (bucket
//! middle, `2^bw` buckets), the wavelet zero code, the preserved count and
//! the running sum from its decompression routines. Every compressed block
//! decodes to exactly its stored length; decoded quaternions are unit length
//! to within 0.0018 (wavelet, mean 0.000006) and 0.0096 (delta, whose 16-bit
//! deltas are quantized per component; blocks restart every 8 frames, so the
//! error does not grow along a clip). Havok renormalizes rotations after
//! sampling. 4,222 clips name only nodes of one model. No bytes from the
//! archives or the executable are committed.

use m2_math::{RawMatrix44, RawQuat, RawVec3};

use crate::bytes::{OutOfBounds, Reader};
use crate::havok::{PackSection, Packfile, PackfileError};
use crate::model::Node;
use crate::ucfx::{Ucfx, UcfxError};

/// Asset type hash of animations.
pub const ANIMATION_TYPE: u32 = 0x1816_6555;
/// The only `info` version seen.
pub const VERSION: u16 = 1;
/// Most transforms (frames times tracks) a clip may decode to.
pub const MAX_TRANSFORMS: u64 = 1 << 22;
/// Values per track per frame: translation 3, rotation 4, scale 3.
pub const DOFS: usize = 10;
/// Frames per wavelet block.
pub const WAVELET_BLOCK: u32 = 8;

const INFO: [u8; 4] = *b"info";
const DATA: [u8; 4] = *b"data";
const TRNM: [u8; 4] = *b"trnm";
const EVNT: [u8; 4] = *b"evnt";
const MANM: [u8; 4] = *b"MANM";
const CONTAINER_CLASS: &str = "hkaAnimationContainer";
const INTERLEAVED_LEN: usize = 48;
/// Track value index where each group starts, and its identity value.
const GROUPS: [(usize, usize); 3] = [(0, 3), (3, 4), (7, 3)];
const IDENTITY: [f32; DOFS] = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];

/// The static mask bit flagging track value `k` as dynamic. Each group's
/// flags start at bit `6 + first` but run in reverse component order, so
/// rotation `w` is bit 9 and `x` bit 12.
fn dynamic_bit(k: usize) -> u16 {
    let (first, len) = GROUPS
        .iter()
        .copied()
        .find(|&(first, len)| (first..first + len).contains(&k))
        .unwrap_or((k, 1));
    1 << (6 + first + (first + len - 1 - k))
}

/// How a clip's keys are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Interleaved,
    Delta,
    Wavelet,
}

impl Codec {
    fn from_type(ty: u32) -> Option<Self> {
        match ty {
            1 => Some(Self::Interleaved),
            2 => Some(Self::Delta),
            3 => Some(Self::Wavelet),
            _ => None,
        }
    }

    /// The class name the packfile gives clips of this encoding.
    pub fn class_name(self) -> &'static str {
        match self {
            Self::Interleaved => "hkaInterleavedSkeletalAnimation",
            Self::Delta => "hkaDeltaCompressedSkeletalAnimation",
            Self::Wavelet => "hkaWaveletSkeletalAnimation",
        }
    }
}

/// Errors from parsing a clip.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum AnimationError {
    #[error("container: {0}")]
    Ucfx(#[from] UcfxError),
    #[error("packfile: {0}")]
    Packfile(#[from] PackfileError),
    #[error("material animation, not a skeletal clip")]
    MaterialAnimation,
    #[error("animation has no {} section", fourcc(.0))]
    MissingSection([u8; 4]),
    #[error("unexpected {} section", fourcc(.0))]
    UnexpectedSection([u8; 4]),
    #[error("unsupported animation version {0}")]
    Version(u16),
    #[error("{} section is {len:#x} bytes, expected {expected:#x}", fourcc(.section))]
    SectionLength {
        section: [u8; 4],
        len: usize,
        expected: u64,
    },
    #[error("{what} truncated: needed {len} bytes at offset {offset:#x}")]
    Truncated {
        what: &'static str,
        offset: usize,
        len: usize,
    },
    #[error("contents object is {0:?}, expected an animation container")]
    Contents(Option<String>),
    #[error("animation container holds {0} animations, expected 1")]
    AnimationCount(u32),
    #[error("pointer field at {0:#x} has no fixup")]
    Pointer(u32),
    #[error("clip class {class:?} does not match encoding {encoding}")]
    Class {
        class: Option<String>,
        encoding: u32,
    },
    #[error("trnm names {trnm} tracks, the clip has {clip}")]
    TrackCount { trnm: u16, clip: u32 },
    #[error("{frames} frames of {tracks} tracks is more than {MAX_TRANSFORMS} transforms")]
    TooLarge { frames: u32, tracks: u32 },
    #[error("{count} transforms do not divide into {tracks} tracks")]
    TransformCount { count: u32, tracks: u32 },
    #[error("track {track} has static mask {mask:#06x} with an unknown group type")]
    StaticMask { track: usize, mask: u16 },
    #[error("static masks select {found} dynamic values, the clip says {expected}")]
    DynamicCount { found: usize, expected: u32 },
    #[error("bit width {0} is over 32")]
    BitWidth(u8),
    #[error("block length {0} frames is not supported")]
    BlockLength(u32),
    #[error("clip has {found} blocks, expected {expected}")]
    BlockCount { found: u32, expected: u32 },
    #[error("block {block} is {len:#x} bytes, but its values need {used:#x}")]
    BlockSize { block: u32, len: usize, used: usize },
    #[error("wavelet clip preserves {0} values per block, expected 0")]
    Preserved(u32),
    #[error("event section has {0} bytes left over")]
    EventTrailing(usize),
}

fn trunc(what: &'static str) -> impl Fn(OutOfBounds) -> AnimationError {
    move |e| AnimationError::Truncated {
        what,
        offset: e.offset,
        len: e.len,
    }
}

fn fourcc(cc: &[u8; 4]) -> String {
    String::from_utf8_lossy(cc).into_owned()
}

/// One node's local transform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub translation: RawVec3,
    pub rotation: RawQuat,
    pub scale: RawVec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Self = Self {
        translation: RawVec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        rotation: RawQuat::IDENTITY,
        scale: RawVec3 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
        },
    };

    fn from_values(v: &[f32; DOFS]) -> Self {
        Self {
            translation: RawVec3 {
                x: v[0],
                y: v[1],
                z: v[2],
            },
            rotation: RawQuat {
                x: v[3],
                y: v[4],
                z: v[5],
                w: v[6],
            },
            scale: RawVec3 {
                x: v[7],
                y: v[8],
                z: v[9],
            },
        }
    }

    /// The local matrix in D3DX layout: `scaling * rotation * translation`.
    /// The rotation is normalized first.
    pub fn to_matrix(&self) -> RawMatrix44 {
        let (t, s) = (self.translation, self.scale);
        RawMatrix44::scaling(s.x, s.y, s.z)
            .multiply(&RawMatrix44::rotation_quaternion(unit(self.rotation)))
            .multiply(&RawMatrix44::translation(t.x, t.y, t.z))
    }

    /// Interpolates towards `rhs`: linearly for translation and scale, by
    /// the shorter spherical path for rotation.
    pub fn interpolate(&self, rhs: &Self, t: f32) -> Self {
        Self {
            translation: self.translation.lerp(rhs.translation, t),
            rotation: unit(self.rotation).slerp(unit(rhs.rotation), t),
            scale: self.scale.lerp(rhs.scale, t),
        }
    }
}

/// `q` normalized, or the identity for a zero or non-finite quaternion.
fn unit(q: RawQuat) -> RawQuat {
    let len = q.length();
    if len > 0.0 && len.is_finite() {
        q.normalize()
    } else {
        RawQuat::IDENTITY
    }
}

/// A timed event, such as a sound cue.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// Seconds from the start; can be slightly past the duration.
    pub time: f32,
    pub name: String,
    /// `sound`, `vo`, `camera` and so on; often empty.
    pub category: String,
}

/// The fields shared by every clip encoding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipHeader {
    pub codec: Codec,
    /// Seconds from the first frame to the last.
    pub duration: f32,
    pub track_count: u32,
    pub unk_0x14: [u32; 4],
}

/// A decoded skeletal clip.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub header: ClipHeader,
    pub frame_count: u32,
    /// The model `HIER` node name hash each track drives.
    pub track_nodes: Vec<u32>,
    /// The track driving the model's root node, if any.
    pub root_track: Option<u16>,
    /// Identifies the track list; how it is derived is unknown.
    pub layout_signature: u32,
    pub events: Vec<Event>,
    /// `frame_count * track_count` transforms, frame-major.
    transforms: Vec<Transform>,
}

impl Clip {
    /// Parses and decodes an animation asset.
    pub fn parse(asset: &[u8]) -> Result<Self, AnimationError> {
        let u = Ucfx::parse(asset)?;
        let mut found: [Option<&[u8]>; 4] = [None; 4];
        const NAMES: [[u8; 4]; 4] = [INFO, DATA, TRNM, EVNT];
        for s in u.sections() {
            if s.fourcc == MANM {
                return Err(AnimationError::MaterialAnimation);
            }
            match (NAMES.iter().position(|n| *n == s.fourcc), s.data) {
                (Some(i), Some(d)) if s.depth == 0 => found[i] = Some(d),
                _ => return Err(AnimationError::UnexpectedSection(s.fourcc)),
            }
        }
        let need = |i: usize| found[i].ok_or(AnimationError::MissingSection(NAMES[i]));

        let info = need(0)?;
        if info.len() != 2 {
            return Err(AnimationError::SectionLength {
                section: INFO,
                len: info.len(),
                expected: 2,
            });
        }
        let version = u16::from_le_bytes([info[0], info[1]]);
        if version != VERSION {
            return Err(AnimationError::Version(version));
        }

        let (track_nodes, root_track, layout_signature) = parse_trnm(need(2)?)?;
        let events = found[3].map(parse_events).transpose()?.unwrap_or_default();

        let pack = Packfile::parse(need(1)?)?;
        let (section_index, clip) = find_clip(&pack)?;
        let section = pack.section(section_index)?;
        let mut r = Reader::at(section.data, clip as usize);
        let rd = trunc("clip header");
        r.take(8).map_err(&rd)?;
        let encoding = r.u32().map_err(&rd)?;
        let duration = f32::from_bits(r.u32().map_err(&rd)?);
        let track_count = r.u32().map_err(&rd)?;
        let mut unk_0x14 = [0; 4];
        for u in &mut unk_0x14 {
            *u = r.u32().map_err(&rd)?;
        }
        let class = pack.class_of(section_index, clip)?;
        let codec = Codec::from_type(encoding)
            .filter(|c| class == Some(c.class_name()))
            .ok_or_else(|| AnimationError::Class {
                class: class.map(str::to_owned),
                encoding,
            })?;
        if track_nodes.len() as u32 != track_count {
            return Err(AnimationError::TrackCount {
                trnm: track_nodes.len() as u16,
                clip: track_count,
            });
        }
        let header = ClipHeader {
            codec,
            duration,
            track_count,
            unk_0x14,
        };
        let (frame_count, transforms) = match codec {
            Codec::Interleaved => decode_interleaved(section, clip, track_count)?,
            Codec::Wavelet => decode_wavelet(section, clip, track_count)?,
            Codec::Delta => decode_delta(section, clip, track_count)?,
        };
        Ok(Self {
            header,
            frame_count,
            track_nodes,
            root_track,
            layout_signature,
            events,
            transforms,
        })
    }

    pub fn track_count(&self) -> usize {
        self.track_nodes.len()
    }

    /// Frames per second, from the duration and frame count; `None` for a
    /// clip with one frame or no duration.
    pub fn frame_rate(&self) -> Option<f32> {
        (self.frame_count > 1 && self.header.duration > 0.0)
            .then(|| (self.frame_count - 1) as f32 / self.header.duration)
    }

    /// Every track's transform at `frame`.
    pub fn frame(&self, frame: u32) -> Option<&[Transform]> {
        let n = self.track_count();
        let start = (frame as usize).checked_mul(n)?;
        self.transforms.get(start..start + n)
    }

    /// The pose at `time` seconds, clamped to the clip, interpolating
    /// between the two nearest frames.
    pub fn sample(&self, time: f32) -> Vec<Transform> {
        let mut out = Vec::new();
        self.sample_into(time, &mut out);
        out
    }

    /// [`sample`](Self::sample) into a reused buffer.
    pub fn sample_into(&self, time: f32, out: &mut Vec<Transform>) {
        out.clear();
        let Some(last) = self.frame_count.checked_sub(1) else {
            return;
        };
        let d = self.header.duration;
        let pos = if last > 0 && d > 0.0 && time.is_finite() {
            (time / d).clamp(0.0, 1.0) * last as f32
        } else {
            0.0
        };
        let i0 = (pos.floor() as u32).min(last);
        let i1 = (i0 + 1).min(last);
        let t = pos - i0 as f32;
        if let (Some(a), Some(b)) = (self.frame(i0), self.frame(i1)) {
            out.extend(a.iter().zip(b).map(|(a, b)| a.interpolate(b, t)));
        }
    }

    /// Binds the clip's tracks to a model's nodes by name hash.
    pub fn bind<'n>(&self, nodes: &'n [Node]) -> Binding<'n> {
        let node_tracks = nodes
            .iter()
            .map(|n| self.track_nodes.iter().position(|&h| h == n.name_hash))
            .collect::<Vec<_>>();
        let bound = node_tracks.iter().flatten().count();
        let mut seen = vec![false; self.track_count()];
        node_tracks.iter().flatten().for_each(|&t| seen[t] = true);
        Binding {
            nodes,
            node_tracks,
            unbound_tracks: seen.iter().filter(|&&s| !s).count(),
            bound,
        }
    }
}

/// A clip's tracks matched to a model's nodes.
#[derive(Debug, Clone)]
pub struct Binding<'n> {
    nodes: &'n [Node],
    node_tracks: Vec<Option<usize>>,
    /// Tracks whose node the model does not have.
    pub unbound_tracks: usize,
    /// Nodes a track drives.
    pub bound: usize,
}

impl Binding<'_> {
    /// The track driving each node, if any.
    pub fn node_tracks(&self) -> &[Option<usize>] {
        &self.node_tracks
    }

    /// Each node's local matrix in `pose`; nodes no track drives keep their
    /// `HIER` local matrix.
    pub fn local_matrices(&self, pose: &[Transform]) -> Vec<RawMatrix44> {
        self.nodes
            .iter()
            .zip(&self.node_tracks)
            .map(|(n, t)| match t.and_then(|t| pose.get(t)) {
                Some(tr) => tr.to_matrix(),
                None => n.local,
            })
            .collect()
    }

    /// Each node's model-space matrix in `pose`, in D3DX layout.
    pub fn model_matrices(&self, pose: &[Transform]) -> Vec<RawMatrix44> {
        model_matrices(self.nodes, &self.local_matrices(pose))
    }

    /// Skinning matrices for `pose`: each node's inverse bind matrix times
    /// its model-space matrix.
    pub fn skin_matrices(&self, pose: &[Transform]) -> Vec<RawMatrix44> {
        self.nodes
            .iter()
            .zip(self.model_matrices(pose))
            .map(|(n, m)| n.inverse_bind.multiply(&m))
            .collect()
    }
}

/// Model-space matrices from local ones: each node's local matrix times its
/// parent's model-space matrix. A parent link outside `nodes`, or a cycle,
/// is treated as a root.
pub fn model_matrices(nodes: &[Node], locals: &[RawMatrix44]) -> Vec<RawMatrix44> {
    let n = nodes.len().min(locals.len());
    let parent = |i: usize| nodes[i].parent.map(usize::from).filter(|&p| p < n);
    let mut world: Vec<Option<RawMatrix44>> = vec![None; n];
    let mut chain = Vec::new();
    for i in 0..n {
        chain.clear();
        let mut at = Some(i);
        while let Some(j) = at {
            if world[j].is_some() || chain.len() > n {
                break;
            }
            chain.push(j);
            at = parent(j);
        }
        for &j in chain.iter().rev() {
            let m = match parent(j).and_then(|p| world[p]) {
                Some(p) => locals[j].multiply(&p),
                None => locals[j],
            };
            world[j] = Some(m);
        }
    }
    world.into_iter().map(Option::unwrap_or_default).collect()
}

fn parse_trnm(d: &[u8]) -> Result<(Vec<u32>, Option<u16>, u32), AnimationError> {
    let rd = trunc("trnm");
    let mut r = Reader::at(d, 0);
    let n = r.u16().map_err(&rd)?;
    let root = r.u16().map_err(&rd)?;
    let signature = r.u32().map_err(&rd)?;
    let expected = 8 + 4 * u64::from(n);
    if d.len() as u64 != expected {
        return Err(AnimationError::SectionLength {
            section: TRNM,
            len: d.len(),
            expected,
        });
    }
    let nodes = (0..n)
        .map(|_| r.u32())
        .collect::<Result<_, _>>()
        .map_err(&rd)?;
    Ok((nodes, (root != 0xFFFF).then_some(root), signature))
}

fn parse_events(d: &[u8]) -> Result<Vec<Event>, AnimationError> {
    let rd = trunc("evnt");
    let mut r = Reader::at(d, 0);
    let count = r.u32().map_err(&rd)?;
    let mut events = Vec::new();
    for _ in 0..count {
        let time = f32::from_bits(r.u32().map_err(&rd)?);
        let name = String::from_utf8_lossy(r.cstr().map_err(&rd)?).into_owned();
        let category = String::from_utf8_lossy(r.cstr().map_err(&rd)?).into_owned();
        events.push(Event {
            time,
            name,
            category,
        });
    }
    match d.len() - r.pos() {
        0 => Ok(events),
        left => Err(AnimationError::EventTrailing(left)),
    }
}

/// Follows the contents object to the clip: `(section, offset)`.
fn find_clip(pack: &Packfile<'_>) -> Result<(u32, u32), AnimationError> {
    let h = &pack.header;
    let class = pack.class_of(h.contents_section, h.contents_offset)?;
    if class != Some(CONTAINER_CLASS) {
        return Err(AnimationError::Contents(class.map(str::to_owned)));
    }
    let sec = pack.section(h.contents_section)?;
    let field = h
        .contents_offset
        .checked_add(8)
        .ok_or(AnimationError::Pointer(u32::MAX))?;
    let count = Reader::at(sec.data, field as usize + 4)
        .u32()
        .map_err(trunc("animation container"))?;
    if count != 1 {
        return Err(AnimationError::AnimationCount(count));
    }
    let array = sec.local(field).ok_or(AnimationError::Pointer(field))?;
    sec.global(array).ok_or(AnimationError::Pointer(array))
}

fn check_size(frames: u32, tracks: u32) -> Result<(), AnimationError> {
    // Frames count even without tracks: the decoders loop over them.
    if u64::from(frames) * u64::from(tracks.max(1)) > MAX_TRANSFORMS {
        return Err(AnimationError::TooLarge { frames, tracks });
    }
    Ok(())
}

/// The `len`-byte array the pointer at `clip + field` points to.
fn pointed<'a>(
    section: &PackSection<'a>,
    clip: u32,
    field: u32,
    len: usize,
) -> Result<&'a [u8], AnimationError> {
    let at = clip
        .checked_add(field)
        .ok_or(AnimationError::Pointer(u32::MAX))?;
    let target = section.local(at).ok_or(AnimationError::Pointer(at))?;
    Reader::at(section.data, target as usize)
        .take(len)
        .map_err(trunc("clip data"))
}

fn decode_interleaved(
    section: &PackSection<'_>,
    clip: u32,
    tracks: u32,
) -> Result<(u32, Vec<Transform>), AnimationError> {
    let count = Reader::at(section.data, clip as usize + 0x28)
        .u32()
        .map_err(trunc("clip header"))?;
    if count == 0 {
        return Ok((0, Vec::new()));
    }
    if tracks == 0 || count % tracks != 0 {
        return Err(AnimationError::TransformCount { count, tracks });
    }
    let frames = count / tracks;
    check_size(frames, tracks)?;
    let data = pointed(section, clip, 0x24, count as usize * INTERLEAVED_LEN)?;
    let transforms = data
        .chunks_exact(INTERLEAVED_LEN)
        .map(|c| {
            let f =
                |i: usize| f32::from_le_bytes([c[4 * i], c[4 * i + 1], c[4 * i + 2], c[4 * i + 3]]);
            Transform::from_values(&[f(0), f(1), f(2), f(4), f(5), f(6), f(7), f(8), f(9), f(10)])
        })
        .collect();
    Ok((frames, transforms))
}

/// Where one of a track's values comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Source {
    Static(f32),
    Dynamic(usize),
}

/// The buffer fields both compressed encodings share.
struct Compressed<'a> {
    buffer: &'a [u8],
    dynamic: u32,
    offsets: usize,
    scales: usize,
    bit_widths: usize,
    masks: usize,
    statics: usize,
}

struct Dequant {
    offset: f32,
    scale: f32,
    bits: u32,
}

impl Dequant {
    /// The middle of quantization bucket `q`: `offset + scale * (q + 0.5) /
    /// 2^bits`, in Havok's order of single-precision operations. A 0-bit
    /// value is bucket 0 too, so it decodes to `offset + scale / 2`.
    fn value(&self, q: u32) -> f32 {
        let step = (-(self.bits as i32) as f32).exp2();
        (q as f32 + 0.5) * (step * self.scale) + self.offset
    }

    /// The code a wavelet block's zero mask stands for: the bucket holding
    /// 0, `trunc(-offset / scale * 2^bits)`, kept below `2^bits`.
    fn zero_code(&self) -> u32 {
        let n = (1u64 << self.bits) as f32;
        let z = (-self.offset / self.scale * n) as i64;
        let z = if z == n as i64 { z - 1 } else { z };
        (z as u64 & ((1u64 << self.bits) - 1)) as u32
    }

    /// What a masked wavelet coefficient decodes to: the zero bucket's
    /// middle, or exactly 0 for a DOF whose range was never set (offset
    /// `f32::MAX`, scale `-inf`: every coefficient was 0 and masked), where
    /// the bucket arithmetic would give `-inf`.
    fn zero_value(&self) -> f32 {
        if self.offset.is_finite() && self.scale.is_finite() {
            self.value(self.zero_code())
        } else {
            0.0
        }
    }
}

impl Compressed<'_> {
    /// Each track's ten value sources.
    fn plan(&self, tracks: u32) -> Result<Vec<[Source; DOFS]>, AnimationError> {
        let mut masks = Reader::at(self.buffer, self.masks);
        let mut statics = Reader::at(self.buffer, self.statics);
        let mut dynamic = 0;
        let mut plan = Vec::new();
        for track in 0..tracks as usize {
            let mask = masks.u16().map_err(trunc("static masks"))?;
            let mut src = [Source::Static(0.0); DOFS];
            for (g, &(first, len)) in GROUPS.iter().enumerate() {
                let ty = (mask >> (2 * g)) & 3;
                for k in first..first + len {
                    let dynamic_flag = mask & dynamic_bit(k) != 0;
                    src[k] = match ty {
                        0 if dynamic_flag => {
                            dynamic += 1;
                            Source::Dynamic(dynamic - 1)
                        }
                        0 | 1 => Source::Static(f32::from_bits(
                            statics.u32().map_err(trunc("static values"))?,
                        )),
                        2 => Source::Static(IDENTITY[k]),
                        _ => return Err(AnimationError::StaticMask { track, mask }),
                    };
                }
            }
            plan.push(src);
        }
        if dynamic != self.dynamic as usize {
            return Err(AnimationError::DynamicCount {
                found: dynamic,
                expected: self.dynamic,
            });
        }
        Ok(plan)
    }

    fn dequant(&self) -> Result<Vec<Dequant>, AnimationError> {
        let n = self.dynamic as usize;
        let mut off = Reader::at(self.buffer, self.offsets);
        let mut sc = Reader::at(self.buffer, self.scales);
        let widths = Reader::at(self.buffer, self.bit_widths)
            .take(n)
            .map_err(trunc("bit widths"))?;
        widths
            .iter()
            .map(|&bw| {
                if bw > 32 {
                    return Err(AnimationError::BitWidth(bw));
                }
                Ok(Dequant {
                    offset: f32::from_bits(off.u32().map_err(trunc("offsets"))?),
                    scale: f32::from_bits(sc.u32().map_err(trunc("scales"))?),
                    bits: u32::from(bw),
                })
            })
            .collect()
    }
}

/// Builds transforms from the plan and the dynamic values, stored
/// DOF-major (`values[dof * frames + frame]`).
fn assemble(plan: &[[Source; DOFS]], values: &[f32], frames: u32) -> Vec<Transform> {
    let frames = frames as usize;
    let mut out = Vec::with_capacity(frames * plan.len());
    for f in 0..frames {
        for track in plan {
            let mut v = [0.0; DOFS];
            for (v, s) in v.iter_mut().zip(track) {
                *v = match *s {
                    Source::Static(x) => x,
                    Source::Dynamic(k) => values.get(k * frames + f).copied().unwrap_or_default(),
                };
            }
            // A static rotation component of +-2 stands for the value that
            // makes the quaternion unit length.
            let marker = (3..7).find(|&k| matches!(track[k], Source::Static(x) if x.abs() == 2.0));
            if let Some(k) = marker {
                let rest: f32 = (3..7).filter(|&j| j != k).map(|j| v[j] * v[j]).sum();
                v[k] = v[k].signum() * (1.0 - rest).max(0.0).sqrt();
            }
            out.push(Transform::from_values(&v));
        }
    }
    out
}

/// A least-significant-bit-first bit reader.
struct Bits<'a> {
    buf: &'a [u8],
    bit: usize,
}

impl Bits<'_> {
    fn read(&mut self, n: u32) -> Option<u32> {
        let mut v = 0u64;
        for i in 0..n {
            let byte = *self.buf.get(self.bit / 8)?;
            v |= u64::from((byte >> (self.bit % 8)) & 1) << i;
            self.bit += 1;
        }
        Some(v as u32)
    }

    fn align(&mut self) {
        self.bit = self.bit.next_multiple_of(8);
    }

    fn bytes_used(&self) -> usize {
        self.bit.div_ceil(8)
    }
}

/// Inverts the three-level LeGall 5/3 lifting wavelet. `c` is
/// `[s, d3, d2_0, d2_1, d1_0, d1_1, d1_2, d1_3]`. At each level with `L`
/// smooth values `s` and details `d`, with `d[-1] = d[0]` and `e[L] = e[L-1]`:
///
/// ```text
/// e[i] = s[i] - (d[i-1] + d[i]) / 4
/// o[i] = d[i] + (e[i] + e[i+1]) / 2
/// s'   = e[0], o[0], e[1], o[1], ...
/// ```
pub fn inverse_wavelet(c: [f32; 8]) -> [f32; 8] {
    let mut s = [0.0; 8];
    s[0] = c[0];
    for len in [1, 2, 4] {
        let d = &c[len..2 * len];
        let mut e = [0.0; 4];
        for i in 0..len {
            e[i] = s[i] - (d[i.saturating_sub(1)] + d[i]) / 4.0;
        }
        for i in 0..len {
            s[2 * i] = e[i];
            s[2 * i + 1] = d[i] + (e[i] + e[(i + 1).min(len - 1)]) / 2.0;
        }
    }
    s
}

fn header_u32s<const N: usize>(
    section: &PackSection<'_>,
    at: usize,
) -> Result<[u32; N], AnimationError> {
    let mut r = Reader::at(section.data, at);
    let mut out = [0; N];
    for o in &mut out {
        *o = r.u32().map_err(trunc("clip header"))?;
    }
    Ok(out)
}

/// The quantization format's preserved value count: the byte at 0x2D,
/// after the largest bit width at 0x2C, read here as part of a word.
fn preserved(format: u32) -> u32 {
    (format >> 8) & 0xFF
}

fn decode_wavelet(
    section: &PackSection<'_>,
    clip: u32,
    tracks: u32,
) -> Result<(u32, Vec<Transform>), AnimationError> {
    // 0x2C holds the quantization format's largest bit width and preserved
    // value count bytes.
    let [
        frames,
        block_len,
        format,
        dynamic,
        offsets,
        scales,
        bit_widths,
        masks,
        statics,
        table,
        blocks,
        quant,
        quant_len,
    ] = header_u32s::<13>(section, clip as usize + 0x24)?;
    let buffer_len = header_u32s::<1>(section, clip as usize + 0x5C)?[0];
    if block_len != WAVELET_BLOCK {
        return Err(AnimationError::BlockLength(block_len));
    }
    let preserved = preserved(format);
    if preserved != 0 {
        return Err(AnimationError::Preserved(preserved));
    }
    check_size(frames, tracks)?;
    let expected = frames.div_ceil(WAVELET_BLOCK);
    if blocks != expected {
        return Err(AnimationError::BlockCount {
            found: blocks,
            expected,
        });
    }
    let buffer = pointed(section, clip, 0x58, buffer_len as usize)?;
    let c = Compressed {
        buffer,
        dynamic,
        offsets: offsets as usize,
        scales: scales as usize,
        bit_widths: bit_widths as usize,
        masks: masks as usize,
        statics: statics as usize,
    };
    let plan = c.plan(tracks)?;
    let dq = c.dequant()?;
    let q = Reader::at(buffer, quant as usize)
        .take(quant_len as usize)
        .map_err(trunc("quantized blocks"))?;
    let mut t = Reader::at(buffer, table as usize);
    let starts = (0..blocks)
        .map(|_| t.u32().map(|v| v as usize))
        .collect::<Result<Vec<_>, _>>()
        .map_err(trunc("block table"))?;

    let n = frames as usize;
    let mut values = vec![0.0; dq.len() * n];
    for (b, &start) in starts.iter().enumerate() {
        let end = starts.get(b + 1).copied().unwrap_or(q.len());
        let block = q.get(start..end).ok_or(AnimationError::Truncated {
            what: "wavelet block",
            offset: start,
            len: end.saturating_sub(start),
        })?;
        let mut bits = Bits { buf: block, bit: 0 };
        let short = |bits: &Bits<'_>| AnimationError::BlockSize {
            block: b as u32,
            len: block.len(),
            used: bits.bytes_used() + 1,
        };
        for (k, d) in dq.iter().enumerate() {
            let zero = bits.read(8).ok_or_else(|| short(&bits))?;
            let mut coef = [0.0; 8];
            for (j, c) in coef.iter_mut().enumerate() {
                *c = if zero & (1 << j) == 0 {
                    d.value(bits.read(d.bits).ok_or_else(|| short(&bits))?)
                } else {
                    d.zero_value()
                };
            }
            bits.align();
            let first = b * WAVELET_BLOCK as usize;
            for (j, v) in inverse_wavelet(coef).into_iter().enumerate() {
                if first + j < n {
                    values[k * n + first + j] = v;
                }
            }
        }
        if bits.bytes_used() != block.len() {
            return Err(AnimationError::BlockSize {
                block: b as u32,
                len: block.len(),
                used: bits.bytes_used(),
            });
        }
    }
    Ok((frames, assemble(&plan, &values, frames)))
}

fn decode_delta(
    section: &PackSection<'_>,
    clip: u32,
    tracks: u32,
) -> Result<(u32, Vec<Transform>), AnimationError> {
    let [
        frames,
        block_len,
        format,
        dynamic,
        offsets,
        scales,
        bit_widths,
        quant,
        quant_len,
        masks,
        _mask_len,
        statics,
        _statics_len,
        full_len,
        last_len,
        _,
        buffer_len,
    ] = header_u32s::<17>(section, clip as usize + 0x24)?;
    if block_len == 0 {
        return Err(AnimationError::BlockLength(block_len));
    }
    let preserved = preserved(format) as usize;
    check_size(frames, tracks)?;
    let buffer = pointed(section, clip, 0x60, buffer_len as usize)?;
    let c = Compressed {
        buffer,
        dynamic,
        offsets: offsets as usize,
        scales: scales as usize,
        bit_widths: bit_widths as usize,
        masks: masks as usize,
        statics: statics as usize,
    };
    let plan = c.plan(tracks)?;
    let dq = c.dequant()?;
    let q = Reader::at(buffer, quant as usize)
        .take(quant_len as usize)
        .map_err(trunc("quantized blocks"))?;

    let n = frames as usize;
    let blocks = frames.div_ceil(block_len);
    let mut values = vec![0.0; dq.len() * n];
    let mut start = 0usize;
    for b in 0..blocks {
        let first = (b * block_len) as usize;
        let count = (n - first).min(block_len as usize);
        let len = if b + 1 == blocks { last_len } else { full_len } as usize;
        let block = Reader::at(q, start)
            .take(len)
            .map_err(trunc("delta block"))?;
        start += len;
        let mut bits = Bits { buf: block, bit: 0 };
        let short = |bits: &Bits<'_>| AnimationError::BlockSize {
            block: b,
            len: block.len(),
            used: bits.bytes_used() + 1,
        };
        for (k, d) in dq.iter().enumerate() {
            // The first `preserved` values are stored as f32, the rest
            // quantized; the block is then a running sum of them all.
            let mut x = 0.0f32;
            for j in 0..count {
                let v = if j < preserved {
                    f32::from_bits(bits.read(32).ok_or_else(|| short(&bits))?)
                } else {
                    d.value(bits.read(d.bits).ok_or_else(|| short(&bits))?)
                };
                x = if j == 0 { v } else { x + v };
                values[k * n + first + j] = x;
            }
            bits.align();
        }
        if bits.bytes_used() != block.len() {
            return Err(AnimationError::BlockSize {
                block: b,
                len: block.len(),
                used: bits.bytes_used(),
            });
        }
    }
    Ok((frames, assemble(&plan, &values, frames)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::havok::tests::PackBuilder;
    use crate::testutil::{UcfxNode, ucfx};

    const CLIP: u32 = 0x40;
    const BW: u32 = 12;

    fn put(buf: &mut Vec<u8>, at: usize, v: u32) {
        if buf.len() < at + 4 {
            buf.resize(at + 4, 0);
        }
        buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// LSB-first bit writer.
    #[derive(Default)]
    struct BitWriter {
        out: Vec<u8>,
        bit: usize,
    }

    impl BitWriter {
        fn write(&mut self, v: u32, n: u32) {
            for i in 0..n {
                if self.bit.is_multiple_of(8) {
                    self.out.push(0);
                }
                let last = self.out.len() - 1;
                self.out[last] |= (((v >> i) & 1) as u8) << (self.bit % 8);
                self.bit += 1;
            }
        }
        fn align(&mut self) {
            self.bit = self.bit.next_multiple_of(8);
        }
    }

    fn forward_wavelet(x: [f32; 8]) -> [f32; 8] {
        let mut out = [0.0; 8];
        let mut s = x;
        for len in [4, 2, 1] {
            let e: Vec<f32> = (0..len).map(|i| s[2 * i]).collect();
            let o: Vec<f32> = (0..len).map(|i| s[2 * i + 1]).collect();
            let d: Vec<f32> = (0..len)
                .map(|i| o[i] - (e[i] + e[(i + 1).min(len - 1)]) / 2.0)
                .collect();
            for i in 0..len {
                s[i] = e[i] + (d[i.saturating_sub(1)] + d[i]) / 4.0;
                out[len + i] = d[i];
            }
        }
        out[0] = s[0];
        out
    }

    /// The bucket of `2^BW` across `[off, off + scale]` holding `v`; the
    /// decoder returns its middle.
    fn quantize(v: f32, off: f32, scale: f32) -> u32 {
        let n = (1u32 << BW) as f32;
        ((v - off) / scale * n).floor().clamp(0.0, n - 1.0) as u32
    }

    /// What the decoder makes of bucket `q`.
    fn dequantize(q: u32, off: f32, scale: f32) -> f32 {
        Dequant {
            offset: off,
            scale,
            bits: BW,
        }
        .value(q)
    }

    /// A clip to encode: per track a static mask and per-frame values.
    struct Source {
        masks: Vec<u16>,
        /// `[track][frame]`.
        values: Vec<Vec<[f32; DOFS]>>,
        /// Static rotation components to store as a +-2 marker.
        markers: Vec<(usize, usize, f32)>,
        frames: u32,
        /// Bytes appended to the last wavelet block.
        extra: usize,
        /// Overrides the clip's class name.
        class: Option<&'static str>,
    }

    impl Source {
        /// Static values and per-DOF series, in walk order.
        fn split(&self) -> (Vec<f32>, Vec<Vec<f32>>) {
            let (mut statics, mut dynamic) = (Vec::new(), Vec::new());
            for (t, &mask) in self.masks.iter().enumerate() {
                for (g, &(first, len)) in GROUPS.iter().enumerate() {
                    let ty = (mask >> (2 * g)) & 3;
                    for k in first..first + len {
                        match ty {
                            0 if mask & dynamic_bit(k) != 0 => dynamic
                                .push(self.values[t].iter().map(|f| f[k]).collect::<Vec<_>>()),
                            0 | 1 => {
                                let m = self.markers.iter().find(|m| m.0 == t && m.1 == k);
                                statics.push(m.map_or(self.values[t][0][k], |m| m.2));
                            }
                            _ => {}
                        }
                    }
                }
            }
            (statics, dynamic)
        }

        fn buffer_head(&self, statics: &[f32], d: usize) -> (Vec<u8>, [usize; 5]) {
            let mut b = vec![0u8; 4];
            let masks = b.len();
            for m in &self.masks {
                b.extend_from_slice(&m.to_le_bytes());
            }
            b.resize(b.len().next_multiple_of(4), 0);
            let st = b.len();
            for v in statics {
                b.extend_from_slice(&v.to_le_bytes());
            }
            let offsets = b.len();
            b.resize(b.len() + 8 * d, 0);
            let scales = offsets + 4 * d;
            let widths = b.len();
            b.resize(b.len() + d, BW as u8);
            b.resize(b.len().next_multiple_of(4), 0);
            (b, [masks, st, offsets, scales, widths])
        }

        fn wavelet(&self) -> Vec<u8> {
            let (statics, dynamic) = self.split();
            let n = self.frames as usize;
            let blocks = n.div_ceil(8);
            let coefs: Vec<Vec<[f32; 8]>> = dynamic
                .iter()
                .map(|series| {
                    (0..blocks)
                        .map(|b| {
                            let mut x = [0.0; 8];
                            for (j, x) in x.iter_mut().enumerate() {
                                *x = series[(8 * b + j).min(n - 1)];
                            }
                            forward_wavelet(x)
                        })
                        .collect()
                })
                .collect();
            let (mut buf, [masks, st, offsets, scales, widths]) =
                self.buffer_head(&statics, dynamic.len());
            let mut ranges = Vec::new();
            for (k, c) in coefs.iter().enumerate() {
                let all = c.iter().flatten();
                let lo = all.clone().fold(f32::MAX, |a, &b| a.min(b));
                let hi = all.fold(f32::MIN, |a, &b| a.max(b));
                let scale = if hi > lo { hi - lo } else { 1.0 };
                buf[offsets + 4 * k..][..4].copy_from_slice(&lo.to_le_bytes());
                buf[scales + 4 * k..][..4].copy_from_slice(&scale.to_le_bytes());
                ranges.push((lo, scale));
            }
            let table = buf.len();
            buf.resize(table + 4 * blocks, 0);
            let quant = buf.len();
            for b in 0..blocks {
                let start = (buf.len() - quant) as u32;
                put(&mut buf, table + 4 * b, start);
                let mut w = BitWriter::default();
                for (c, &(lo, scale)) in coefs.iter().zip(&ranges) {
                    let c = c[b];
                    let zero = (0..8).filter(|&j| c[j] == 0.0).fold(0, |m, j| m | 1 << j);
                    w.write(zero, 8);
                    for j in (0..8).filter(|&j| c[j] != 0.0) {
                        w.write(quantize(c[j], lo, scale), BW);
                    }
                    w.align();
                }
                buf.extend_from_slice(&w.out);
            }
            buf.resize(buf.len() + self.extra, 0);
            let quant_len = buf.len() - quant;
            let mut clip = self.common(3, 0x60);
            let fields = [
                self.frames,
                8,
                BW,
                dynamic.len() as u32,
                offsets as u32,
                scales as u32,
                widths as u32,
                masks as u32,
                st as u32,
                table as u32,
                blocks as u32,
                quant as u32,
                quant_len as u32,
                0,
                buf.len() as u32,
            ];
            for (i, v) in fields.into_iter().enumerate() {
                put(&mut clip, 0x24 + 4 * i, v);
            }
            self.pack(clip, buf, Codec::Wavelet, 0x58)
        }

        fn delta(&self) -> Vec<u8> {
            let (statics, dynamic) = self.split();
            let n = self.frames as usize;
            let (mut buf, [masks, st, offsets, scales, widths]) =
                self.buffer_head(&statics, dynamic.len());
            let mut ranges = Vec::new();
            for (k, s) in dynamic.iter().enumerate() {
                let deltas = s.windows(2).map(|w| w[1] - w[0]);
                let lo = deltas.clone().fold(f32::MAX, f32::min).min(0.0);
                let hi = deltas.fold(f32::MIN, f32::max).max(0.0);
                let scale = if hi > lo { hi - lo } else { 1.0 };
                buf[offsets + 4 * k..][..4].copy_from_slice(&lo.to_le_bytes());
                buf[scales + 4 * k..][..4].copy_from_slice(&scale.to_le_bytes());
                ranges.push((lo, scale));
            }
            let quant = buf.len();
            let mut lens = Vec::new();
            for first in (0..n).step_by(8) {
                let count = (n - first).min(8);
                let mut w = BitWriter::default();
                for (s, &(lo, scale)) in dynamic.iter().zip(&ranges) {
                    let mut r = s[first];
                    w.write(r.to_bits(), 32);
                    for j in 1..count {
                        let q = quantize(s[first + j] - r, lo, scale);
                        r += dequantize(q, lo, scale);
                        w.write(q, BW);
                    }
                    w.align();
                }
                lens.push(w.out.len() as u32);
                buf.extend_from_slice(&w.out);
            }
            let quant_len = buf.len() - quant;
            let mut clip = self.common(2, 0x68);
            let fields = [
                self.frames,
                8,
                BW | 1 << 8,
                dynamic.len() as u32,
                offsets as u32,
                scales as u32,
                widths as u32,
                quant as u32,
                quant_len as u32,
                masks as u32,
                0xDEAD,
                st as u32,
                0xBEEF,
                lens[0],
                *lens.last().unwrap(),
                0,
                buf.len() as u32,
            ];
            for (i, v) in fields.into_iter().enumerate() {
                put(&mut clip, 0x24 + 4 * i, v);
            }
            self.pack(clip, buf, Codec::Delta, 0x60)
        }

        fn interleaved(&self) -> Vec<u8> {
            let mut buf = Vec::new();
            for f in 0..self.frames as usize {
                for t in &self.values {
                    let v = t[f];
                    for x in [
                        v[0], v[1], v[2], 9.0, v[3], v[4], v[5], v[6], v[7], v[8], v[9], 0.0,
                    ] {
                        buf.extend_from_slice(&x.to_le_bytes());
                    }
                }
            }
            let mut clip = self.common(1, 0x34);
            put(&mut clip, 0x28, self.frames * self.masks.len() as u32);
            self.pack(clip, buf, Codec::Interleaved, 0x24)
        }

        fn common(&self, ty: u32, len: usize) -> Vec<u8> {
            let mut c = vec![0; len];
            put(&mut c, 8, ty);
            c[0xC..0x10].copy_from_slice(&((self.frames.max(2) - 1) as f32 / 30.0).to_le_bytes());
            put(&mut c, 0x10, self.masks.len() as u32);
            c
        }

        /// A packfile with the container, the clip at `CLIP` and its buffer.
        fn pack(&self, clip: Vec<u8>, buffer: Vec<u8>, codec: Codec, ptr: u32) -> Vec<u8> {
            let mut b = PackBuilder::new();
            b.data = vec![0; CLIP as usize];
            put(&mut b.data, 0x0C, 1);
            b.data.extend_from_slice(&clip);
            b.data.resize(b.data.len().next_multiple_of(16), 0);
            let at = b.data.len() as u32;
            b.data.extend_from_slice(&buffer);
            b.object(0, CONTAINER_CLASS);
            b.object(CLIP, self.class.unwrap_or(codec.class_name()));
            b.local.push((0x08, 0x30));
            b.local.push((CLIP + ptr, at));
            b.global.push((0x30, 2, CLIP));
            b.build()
        }
    }

    fn trnm(n: usize, root: u16) -> Vec<u8> {
        let mut t = (n as u16).to_le_bytes().to_vec();
        t.extend_from_slice(&root.to_le_bytes());
        t.extend_from_slice(&0x1234_5678u32.to_le_bytes());
        for i in 0..n as u32 {
            t.extend_from_slice(&(0xA000 + i).to_le_bytes());
        }
        t
    }

    fn events() -> Vec<u8> {
        let mut e = 2u32.to_le_bytes().to_vec();
        for (t, name, cat) in [(0.0f32, "", ""), (0.5, "step", "sound")] {
            e.extend_from_slice(&t.to_le_bytes());
            for s in [name, cat] {
                e.extend_from_slice(s.as_bytes());
                e.push(0);
            }
        }
        e
    }

    fn asset(pack: Vec<u8>, tracks: usize, evnt: bool) -> Vec<u8> {
        let mut nodes = vec![
            UcfxNode::Leaf(INFO, 1u16.to_le_bytes().to_vec()),
            UcfxNode::Leaf(DATA, pack),
            UcfxNode::Leaf(TRNM, trnm(tracks, 0)),
        ];
        if evnt {
            nodes.push(UcfxNode::Leaf(EVNT, events()));
        }
        ucfx(&nodes)
    }

    fn quat(axis: m2_math::glam::Vec3, angle: f32) -> [f32; 4] {
        m2_math::glam::Quat::from_axis_angle(axis, angle).to_array()
    }

    /// Three tracks over `frames` frames: a moving, rotating root; a track
    /// whose rotation is about Y only with y and w marked; an identity one.
    fn source(frames: u32) -> Source {
        let mut values = vec![Vec::new(), Vec::new(), Vec::new()];
        for f in 0..frames {
            let a = f as f32 * 0.07;
            let q = quat(m2_math::glam::Vec3::new(1.0, 2.0, 3.0).normalize(), a);
            values[0].push([a, 1.5, -0.25 * a, q[0], q[1], q[2], q[3], 1.0, 1.0, 1.0]);
            let q = quat(m2_math::glam::Vec3::Y, 0.4 + a);
            values[1].push([0.0, 0.3, 0.0, 0.0, q[1], 0.0, q[3], 2.0, 2.0, 2.0]);
            values[2].push(IDENTITY);
        }
        // Track 0: translation x, z and the whole rotation dynamic; scale
        // static. Track 1: translation static, rotation y dynamic with w
        // as a marker, scale static. Track 2: identity.
        let dyn_bits = |ks: &[usize]| ks.iter().fold(0u16, |m, &k| m | dynamic_bit(k));
        Source {
            masks: vec![
                dyn_bits(&[0, 2, 3, 4, 5, 6]) | 1 << 4,
                1 | dyn_bits(&[4]) | 1 << 4,
                2 | 2 << 2 | 2 << 4,
            ],
            values,
            markers: vec![(1, 6, 2.0)],
            frames,
            extra: 0,
            class: None,
        }
    }

    fn check(clip: &Clip, src: &Source, tol: f32) {
        assert_eq!(clip.frame_count, src.frames);
        assert_eq!(clip.track_count(), 3);
        for f in 0..src.frames {
            let frame = clip.frame(f).unwrap();
            for (t, tr) in frame.iter().enumerate() {
                let want = src.values[t][f as usize];
                let r = tr.rotation;
                let got = [
                    tr.translation.x,
                    tr.translation.y,
                    tr.translation.z,
                    r.x,
                    r.y,
                    r.z,
                    r.w,
                    tr.scale.x,
                    tr.scale.y,
                    tr.scale.z,
                ];
                for k in 0..DOFS {
                    assert!(
                        (got[k] - want[k]).abs() <= tol,
                        "frame {f} track {t} value {k}: {} vs {}",
                        got[k],
                        want[k]
                    );
                }
            }
        }
    }

    #[test]
    fn dynamic_flags_run_in_reverse_component_order() {
        // Literal bits, so the round trips below cannot pass with encoder
        // and decoder agreeing on a wrong layout.
        let bits: Vec<u16> = (0..DOFS).map(dynamic_bit).collect();
        assert_eq!(
            bits,
            [8, 7, 6, 12, 11, 10, 9, 15, 14, 13].map(|b| 1u16 << b)
        );
        // Track 0: translation x and z (bits 8, 6) and all of the rotation
        // dynamic, scale static. Track 1: rotation y (bit 11) dynamic.
        let src = source(4);
        assert_eq!(src.masks[0], 1 << 8 | 1 << 6 | 0xF << 9 | 1 << 4);
        assert_eq!(src.masks[1], 1 | 1 << 11 | 1 << 4);
    }

    #[test]
    fn wavelet_round_trips_and_pads_the_last_block() {
        let src = source(19);
        let clip = Clip::parse(&asset(src.wavelet(), 3, true)).unwrap();
        assert_eq!(clip.header.codec, Codec::Wavelet);
        check(&clip, &src, 2e-3);
        assert!((clip.frame_rate().unwrap() - 30.0).abs() < 1e-3);
    }

    #[test]
    fn delta_round_trips() {
        let src = source(11);
        let clip = Clip::parse(&asset(src.delta(), 3, false)).unwrap();
        assert_eq!(clip.header.codec, Codec::Delta);
        check(&clip, &src, 2e-3);
        assert!(clip.events.is_empty());
    }

    #[test]
    fn interleaved_reads_trnm_and_events() {
        let src = source(4);
        let clip = Clip::parse(&asset(src.interleaved(), 3, true)).unwrap();
        assert_eq!(clip.header.codec, Codec::Interleaved);
        check(&clip, &src, 0.0);
        assert_eq!(clip.track_nodes, [0xA000, 0xA001, 0xA002]);
        assert_eq!(clip.root_track, Some(0));
        assert_eq!(clip.layout_signature, 0x1234_5678);
        assert_eq!(clip.events.len(), 2);
        assert_eq!(clip.events[1].name, "step");
        assert_eq!(clip.events[1].category, "sound");
        assert_eq!(clip.events[1].time, 0.5);
    }

    #[test]
    fn wavelet_inverse_undoes_the_forward_transform() {
        let x = [0.5, -1.0, 2.0, 2.5, 3.0, -0.25, 0.0, 7.0];
        let y = inverse_wavelet(forward_wavelet(x));
        for (a, b) in x.iter().zip(y) {
            assert!((a - b).abs() < 1e-5);
        }
        // A constant is all smooth coefficient.
        assert_eq!(
            forward_wavelet([3.0; 8]),
            [3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
        );
        assert_eq!(
            inverse_wavelet([3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
            [3.0; 8]
        );
    }

    #[test]
    fn sampling_interpolates_and_clamps() {
        let src = source(4);
        let clip = Clip::parse(&asset(src.interleaved(), 3, false)).unwrap();
        let dt = 1.0 / 30.0;
        let mid = clip.sample(1.5 * dt);
        let (a, b) = (clip.frame(1).unwrap(), clip.frame(2).unwrap());
        let tx = (a[0].translation.x + b[0].translation.x) / 2.0;
        assert!((mid[0].translation.x - tx).abs() < 1e-5);
        let ra = a[1].rotation;
        let want = ra.slerp(b[1].rotation, 0.5);
        assert!(mid[1].rotation.dot(want) > 1.0 - 1e-6);
        assert_eq!(clip.sample(-1.0), clip.frame(0).unwrap());
        assert_eq!(clip.sample(100.0), clip.frame(3).unwrap());
        assert_eq!(clip.sample(f32::NAN), clip.frame(0).unwrap());
    }

    #[test]
    fn transform_matrix_scales_rotates_then_translates() {
        let q = quat(m2_math::glam::Vec3::Z, std::f32::consts::FRAC_PI_2);
        let t = Transform::from_values(&[5.0, 0.0, 0.0, q[0], q[1], q[2], q[3], 2.0, 2.0, 2.0]);
        let p = RawVec3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        }
        .transform_coord(&t.to_matrix());
        assert!((p.x - 5.0).abs() < 1e-5 && (p.y - 2.0).abs() < 1e-5 && p.z.abs() < 1e-5);
    }

    fn node(name_hash: u32, parent: Option<u16>, x: f32) -> Node {
        Node {
            name_hash,
            flags: 0,
            first_child: None,
            parent,
            next_sibling: None,
            unk_0x0c: 0,
            local: RawMatrix44::translation(x, 0.0, 0.0),
            inverse_bind: RawMatrix44::IDENTITY,
            bounds_min: [0.0; 4],
            bounds_max: [0.0; 4],
        }
    }

    #[test]
    fn binding_poses_the_hierarchy() {
        let src = source(4);
        let clip = Clip::parse(&asset(src.interleaved(), 3, false)).unwrap();
        // Child listed before its parent; 0xA001 drives the child; the
        // parent and an extra node keep their HIER matrices.
        let nodes = [
            node(0xA001, Some(1), 0.0),
            node(0xBEEF, None, 10.0),
            node(0xCAFE, Some(0), 1.0),
        ];
        let b = clip.bind(&nodes);
        assert_eq!(b.node_tracks(), [Some(1), None, None]);
        assert_eq!(b.bound, 1);
        assert_eq!(b.unbound_tracks, 2);
        let pose = clip.frame(0).unwrap();
        let world = b.model_matrices(pose);
        // Child: track 1 (scale 2, rotation about Y, translation y 0.3)
        // under the parent at x = 10.
        let origin = |m: &RawMatrix44| [m.m[3][0], m.m[3][1], m.m[3][2]];
        assert_eq!(origin(&world[1]), [10.0, 0.0, 0.0]);
        let c = origin(&world[0]);
        assert!((c[0] - 10.0).abs() < 1e-5 && (c[1] - 0.3).abs() < 1e-5);
        // Grandchild one unit along the child's rotated, scaled x axis.
        let g = origin(&world[2]);
        let axis = RawVec3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        }
        .transform_normal(&pose[1].to_matrix());
        assert!((g[0] - (c[0] + axis.x)).abs() < 1e-5);
        assert!((g[2] - (c[2] + axis.z)).abs() < 1e-5);
        assert_eq!(b.skin_matrices(pose)[1], world[1]);
    }

    #[test]
    fn model_matrices_survive_cycles() {
        let nodes = [node(1, Some(1), 1.0), node(2, Some(0), 2.0)];
        let locals = [nodes[0].local, nodes[1].local];
        assert_eq!(model_matrices(&nodes, &locals).len(), 2);
    }

    #[test]
    fn material_animations_are_reported() {
        let a = ucfx(&[UcfxNode::Leaf(*b"MANM", vec![0; 16])]);
        assert_eq!(Clip::parse(&a), Err(AnimationError::MaterialAnimation));
    }

    #[test]
    fn rejects_mismatched_tracks_and_classes() {
        let src = source(4);
        let a = asset(src.interleaved(), 2, false);
        assert!(matches!(
            Clip::parse(&a),
            Err(AnimationError::TrackCount { trnm: 2, clip: 3 })
        ));
        let mut src = src;
        src.class = Some(Codec::Wavelet.class_name());
        assert!(matches!(
            Clip::parse(&asset(src.interleaved(), 3, false)),
            Err(AnimationError::Class { encoding: 1, .. })
        ));
    }

    #[test]
    fn dequantization_takes_the_middle_of_each_bucket() {
        let d = |bits| Dequant {
            offset: -1.0,
            scale: 2.0,
            bits,
        };
        // 4 bits: buckets of 2/16 starting at -1, decoded at their middle.
        assert_eq!(d(4).value(0), -1.0 + 1.0 / 16.0);
        assert_eq!(d(4).value(15), 1.0 - 1.0 / 16.0);
        // A 0-bit value is bucket 0 of 1: the middle of the range.
        assert_eq!(d(0).value(0), 0.0);
        // The zero mask's code is the bucket holding 0.
        assert_eq!(d(4).zero_code(), 8);
        assert_eq!(d(4).value(d(4).zero_code()), 1.0 / 16.0);
        // A range ending at 0 keeps the code below 2^bits.
        let top = Dequant {
            offset: -2.0,
            scale: 2.0,
            bits: 4,
        };
        assert_eq!(top.zero_code(), 15);
        assert_eq!(d(0).zero_code(), 0);
        // A range never set (every coefficient masked) decodes to 0.
        let unset = Dequant {
            offset: f32::MAX,
            scale: f32::NEG_INFINITY,
            bits: 11,
        };
        assert_eq!(unset.zero_value(), 0.0);
        assert_eq!(d(4).zero_value(), 1.0 / 16.0);
    }

    #[test]
    fn preserved_count_is_the_second_format_byte() {
        assert_eq!(preserved(BW | 1 << 8), 1);
        assert_eq!(preserved(BW), 0);
    }

    #[test]
    fn rejects_unknown_mask_types() {
        let mut src = source(4);
        src.masks[2] = 3;
        assert!(matches!(
            Clip::parse(&asset(src.wavelet(), 3, false)),
            Err(AnimationError::StaticMask { track: 2, .. })
        ));
    }

    #[test]
    fn rejects_unconsumed_block_bytes() {
        let mut src = source(12);
        src.extra = 1;
        assert!(matches!(
            Clip::parse(&asset(src.wavelet(), 3, false)),
            Err(AnimationError::BlockSize { block: 1, .. })
        ));
    }

    #[test]
    fn truncated_assets_fail_cleanly() {
        let src = source(9);
        for pack in [src.wavelet(), src.delta(), src.interleaved()] {
            let full = asset(pack.clone(), 3, true);
            assert!(Clip::parse(&full).is_ok());
            for len in 0..pack.len() {
                assert!(
                    Clip::parse(&asset(pack[..len].to_vec(), 3, true)).is_err(),
                    "length {len}"
                );
            }
        }
    }
}
