//! Faces for speech: a character's face rig (`facefxactor`, type
//! `1cf649bb`) and the face animations of spoken lines
//! (`facefxanimationset`, type `665ef13e`).
//!
//! A rig ([`Actor`]) is a set of face bones with their rest pose and a
//! list of named poses ("Eat", "Oat", "Blink Left", "Happy" ...), each
//! giving a few bones a full transform. An animation set
//! ([`AnimationSet`]) holds one animation per spoken line; an animation
//! is a curve per pose name over the line's seconds, and a pose's curve
//! value is how far the face is taken from rest to that pose.
//!
//! Both are a `UCFX` container with one `data` chunk holding an archive,
//! everything little-endian:
//!
//! ```text
//! "FACE", u32 version (1610)
//! str licensee, str project        str = u16 1, u32 length, bytes
//! u32 1000, u32 0
//! u32 name count, then per name: u16 0, str (its bytes end in a NUL)
//! ... the body, naming things by their index among the names
//! ```
//!
//! A name in the body is a reference: `u16 1, u32 index`.
//!
//! A rig's body:
//!
//! ```text
//! u32 (0x21), u32 0, ref actor name, u16 2, u16 1
//! u32 bone count, then per bone:
//!   u32 0, ref name, u16 0, position 3 f32, rotation 4 f32 (w first),
//!   scale 3 f32, u32 index, f32 weight
//! u16 0, u32 1, u32 node count, then per node:
//!   u32 its number, ref class, u32 0x5C, u32 0, ref name, u16 2,
//!   f32 least, f32 most, u32 0, u16 0,
//!   u32 input count, then per input:
//!     u32 0, ref the node it comes from, ref link function ("linear"),
//!     u32 0, u32 parameter count, f32 per parameter, u32 0xFFFFFFFF
//!   12 bytes (a u16 at +6: 0, 1, 2 or 4; not read)
//!   for an "FxBonePoseNode" only: u16 0, u32 bone count, then per bone:
//!     u32 0, ref name, u16 0, position, rotation, scale
//!   ("FxCombinerNode" has nothing more)
//! ... then a phoneme map and the rest, not read
//! ```
//!
//! An animation set's body:
//!
//! ```text
//! u32 0, u32 0, ref set name, u16 0, ref actor name, u32 0,
//! ref group name, u32 0, u32 animation count, then per animation:
//!   u32 0, ref name, u32 6, u32 curve count, then per curve:
//!     u32 0, ref name, u32 1, u16 0
//!   u32 key count (of all the curves), then per key:
//!     f32 seconds, f32 value, f32 slope in, f32 slope out
//!   u16 0, u32 curve count, a u32 per curve: how many of the keys are
//!   its, in order
//!   f32 (0.16), f32 (0.22), then 22 bytes not read
//! ```
//!
//! # Provenance
//!
//! Format-from-samples: the 31 rigs and 86 animation sets of a legally
//! installed copy's `vz.wad`, read whole by these rules with every count
//! matching what follows it (the `facefx` example lists them). The
//! archive is the FaceFX library's own; the 360 program links that
//! library (its class names `FxActor`, `FxBonePoseNode`, `FxAnimSet`,
//! `FxAnimCurve` ...), and its reading code was not consulted for this.
//! No bytes from the archives are committed.
//!
//! **Inferred**: a rotation is stored w first (rest rotations read
//! `-1, 0, 0, 0` and `-0.5, 0.5, -0.5, -0.5`); a key's last two numbers
//! are slopes (0 in the samples looked at); the two floats after an
//! animation's keys are its blend-in and blend-out seconds.
//!
//! **Not established**: the fields named only by their values above,
//! and the bytes not read.

use crate::bytes::{OutOfBounds, Reader};
use crate::ucfx::{Ucfx, UcfxError};

/// The asset types' name hashes.
pub const ACTOR_TYPE: u32 = 0x1cf6_49bb;
pub const ANIMATION_SET_TYPE: u32 = 0x665e_f13e;

const DATA: [u8; 4] = *b"data";
const MAGIC: [u8; 4] = *b"FACE";

/// The archive version of every one of the game's.
pub const VERSION: u32 = 1610;

/// The class of every pose in the game's rigs.
pub const BONE_POSE_NODE: &str = "FxBonePoseNode";

/// A parse failure.
#[derive(Debug, thiserror::Error)]
pub enum FaceFxError {
    #[error(transparent)]
    Ucfx(#[from] UcfxError),
    #[error("no data chunk")]
    NoData,
    #[error("truncated: {len} bytes at {offset}")]
    Truncated { offset: usize, len: usize },
    #[error("not a FACE archive")]
    BadMagic,
    #[error("archive version {0}, not 1610")]
    Version(u32),
    #[error("at {offset}: expected {expected}, found {found}")]
    Unexpected {
        offset: usize,
        expected: &'static str,
        found: u32,
    },
    #[error("name {index} of {count} at {offset}")]
    BadName {
        offset: usize,
        index: u32,
        count: usize,
    },
    #[error("pose {pose} is a {class}, not a bone pose")]
    UnknownClass { pose: usize, class: String },
    #[error("animation {animation}: its curves' keys add up to {counted}, of {keys}")]
    KeyCounts {
        animation: usize,
        counted: u64,
        keys: usize,
    },
}

impl From<OutOfBounds> for FaceFxError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// A bone's place: position, rotation (x, y, z, w) and scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

/// A face bone of a rig, at rest.
#[derive(Clone, Debug, PartialEq)]
pub struct Bone {
    pub name: String,
    pub rest: Transform,
    /// A number of the bone's (10, 11, 14 ... in the game's; read as its
    /// place in the head's skeleton, not checked).
    pub index: u32,
    pub weight: f32,
}

/// The combiner node class: a value the animations or other nodes set,
/// that feeds other nodes (the head's and eyes' orientation, `Blink`,
/// `Eyebrow Raise`).
pub const COMBINER_NODE: &str = "FxCombinerNode";

/// What feeds a node: another node's value through a link function.
#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    /// The node it comes from.
    pub from: String,
    /// The link function's name (`linear` in every one of the game's)
    /// and its parameters (for `linear`: a slope and an offset).
    pub function: String,
    pub parameters: Vec<f32>,
}

impl Input {
    /// What the link makes of `value`: for `linear`, the slope times it
    /// plus the offset; any other function passes it as it is.
    pub fn apply(&self, value: f32) -> f32 {
        match (self.function.as_str(), self.parameters.as_slice()) {
            ("linear", [m, b, ..]) => m * value + b,
            _ => value,
        }
    }
}

/// A node of a rig's face graph: a pose (the bones it moves and where
/// it takes each) or a combiner (no bones), with what feeds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    pub name: String,
    /// `FxBonePoseNode` or `FxCombinerNode`.
    pub class: String,
    /// The least and most its value may be.
    pub least: f32,
    pub most: f32,
    pub inputs: Vec<Input>,
    pub bones: Vec<(String, Transform)>,
}

/// A character's face rig.
#[derive(Clone, Debug, PartialEq)]
pub struct Actor {
    pub name: String,
    pub bones: Vec<Bone>,
    pub poses: Vec<Pose>,
}

/// A key of a curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    pub seconds: f32,
    pub value: f32,
    pub slope_in: f32,
    pub slope_out: f32,
}

/// One pose's (or other channel's) value over a line.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub name: String,
    pub keys: Vec<Key>,
}

impl Curve {
    /// The curve's value at `seconds`: straight between its keys (their
    /// slopes are 0 in the game's data and are not used), held before
    /// the first and after the last; 0 with no keys.
    pub fn value(&self, seconds: f32) -> f32 {
        let Some(first) = self.keys.first() else {
            return 0.0;
        };
        if seconds <= first.seconds {
            return first.value;
        }
        for pair in self.keys.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if seconds <= b.seconds {
                let span = b.seconds - a.seconds;
                if span <= 0.0 {
                    return b.value;
                }
                return a.value + (b.value - a.value) * (seconds - a.seconds) / span;
            }
        }
        self.keys[self.keys.len() - 1].value
    }
}

/// A spoken line's face animation.
#[derive(Clone, Debug, PartialEq)]
pub struct Animation {
    pub name: String,
    pub curves: Vec<Curve>,
    /// Read as the seconds it blends in and out over.
    pub blend_in: f32,
    pub blend_out: f32,
}

impl Animation {
    /// The last key's time over all its curves, seconds.
    pub fn seconds(&self) -> f32 {
        self.curves
            .iter()
            .filter_map(|c| c.keys.last())
            .map(|k| k.seconds)
            .fold(0.0, f32::max)
    }
}

/// The face animations of a scene's lines.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimationSet {
    pub name: String,
    /// The rig it was made for, by name (`NewActor` in the game's).
    pub actor: String,
    pub animations: Vec<Animation>,
}

impl AnimationSet {
    /// The animation named `name`, whatever its letters' case.
    pub fn animation(&self, name: &str) -> Option<&Animation> {
        self.animations
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
    }
}

/// An archive being read: its bytes after the names, and the names.
struct Body<'a> {
    r: Reader<'a>,
    names: Vec<String>,
}

impl<'a> Body<'a> {
    /// Opens the archive in `asset`'s `data` chunk and reads its names.
    fn open(asset: &'a [u8]) -> Result<Self, FaceFxError> {
        let u = Ucfx::parse(asset)?;
        let data = u.top_level(DATA).ok_or(FaceFxError::NoData)?;
        let mut r = Reader::at(data, 0);
        if r.array::<4>()? != MAGIC {
            return Err(FaceFxError::BadMagic);
        }
        let version = r.u32()?;
        if version != VERSION {
            return Err(FaceFxError::Version(version));
        }
        let mut body = Self {
            r,
            names: Vec::new(),
        };
        body.string()?;
        body.string()?;
        body.expect_u32(1000, "1000")?;
        body.expect_u32(0, "0")?;
        let count = body.r.u32()? as usize;
        for _ in 0..count {
            body.expect_u16(0, "0 before a name")?;
            let name = body.string()?;
            body.names.push(name.trim_end_matches('\0').to_owned());
        }
        Ok(body)
    }

    fn expect_u16(&mut self, want: u16, expected: &'static str) -> Result<(), FaceFxError> {
        let offset = self.r.pos();
        let found = self.r.u16()?;
        if found != want {
            return Err(FaceFxError::Unexpected {
                offset,
                expected,
                found: u32::from(found),
            });
        }
        Ok(())
    }

    fn expect_u32(&mut self, want: u32, expected: &'static str) -> Result<(), FaceFxError> {
        let offset = self.r.pos();
        let found = self.r.u32()?;
        if found != want {
            return Err(FaceFxError::Unexpected {
                offset,
                expected,
                found,
            });
        }
        Ok(())
    }

    /// A string: `u16 1, u32 length, bytes`.
    fn string(&mut self) -> Result<String, FaceFxError> {
        self.expect_u16(1, "1 before a string")?;
        let len = self.r.u32()? as usize;
        Ok(String::from_utf8_lossy(self.r.take(len)?).into_owned())
    }

    /// A name reference: `u16 1, u32 index`.
    fn name(&mut self) -> Result<String, FaceFxError> {
        self.expect_u16(1, "1 before a name's index")?;
        let offset = self.r.pos();
        let index = self.r.u32()?;
        self.names
            .get(index as usize)
            .cloned()
            .ok_or(FaceFxError::BadName {
                offset,
                index,
                count: self.names.len(),
            })
    }

    fn f32(&mut self) -> Result<f32, FaceFxError> {
        Ok(f32::from_bits(self.r.u32()?))
    }

    /// A count that what is left of the archive could hold, each of the
    /// counted taking at least `each` bytes.
    fn count(&mut self, each: usize) -> Result<usize, FaceFxError> {
        let offset = self.r.pos();
        let n = self.r.u32()? as usize;
        if n.saturating_mul(each) > self.r.remaining() {
            return Err(FaceFxError::Truncated {
                offset,
                len: n.saturating_mul(each),
            });
        }
        Ok(n)
    }

    /// Position, rotation (stored w first) and scale.
    fn transform(&mut self) -> Result<Transform, FaceFxError> {
        let mut f = [0.0f32; 10];
        for v in &mut f {
            *v = self.f32()?;
        }
        Ok(Transform {
            position: [f[0], f[1], f[2]],
            rotation: [f[4], f[5], f[6], f[3]],
            scale: [f[7], f[8], f[9]],
        })
    }
}

impl Actor {
    /// Parses a `facefxactor` asset.
    pub fn parse(asset: &[u8]) -> Result<Self, FaceFxError> {
        let mut b = Body::open(asset)?;
        b.r.u32()?;
        b.expect_u32(0, "0")?;
        let name = b.name()?;
        b.r.u16()?;
        b.r.u16()?;
        let bone_count = b.count(54)?;
        let mut bones = Vec::with_capacity(bone_count);
        for _ in 0..bone_count {
            b.expect_u32(0, "0 before a bone")?;
            let name = b.name()?;
            b.expect_u16(0, "0 after a bone's name")?;
            let rest = b.transform()?;
            let index = b.r.u32()?;
            let weight = b.f32()?;
            bones.push(Bone {
                name,
                rest,
                index,
                weight,
            });
        }
        b.r.u16()?;
        b.r.u32()?;
        let pose_count = b.count(52)?;
        let mut poses = Vec::with_capacity(pose_count);
        for k in 0..pose_count {
            b.r.u32()?;
            let class = b.name()?;
            if class != BONE_POSE_NODE && class != COMBINER_NODE {
                return Err(FaceFxError::UnknownClass { pose: k, class });
            }
            b.r.u32()?;
            b.r.u32()?;
            let name = b.name()?;
            b.r.u16()?;
            let least = b.f32()?;
            let most = b.f32()?;
            b.r.u32()?;
            b.r.u16()?;
            let input_count = b.count(28)?;
            let mut inputs = Vec::with_capacity(input_count);
            for _ in 0..input_count {
                b.expect_u32(0, "0 before an input")?;
                let from = b.name()?;
                let function = b.name()?;
                b.r.u32()?;
                let n = b.count(4)?;
                let mut parameters = Vec::with_capacity(n);
                for _ in 0..n {
                    parameters.push(b.f32()?);
                }
                b.r.u32()?;
                inputs.push(Input {
                    from,
                    function,
                    parameters,
                });
            }
            b.r.take(12)?;
            let mut pose_bones = Vec::new();
            if class == BONE_POSE_NODE {
                b.r.u16()?;
                let moved = b.count(52)?;
                pose_bones.reserve(moved);
                for _ in 0..moved {
                    b.expect_u32(0, "0 before a pose's bone")?;
                    let bone = b.name()?;
                    b.expect_u16(0, "0 after a pose's bone's name")?;
                    pose_bones.push((bone, b.transform()?));
                }
            }
            poses.push(Pose {
                name,
                class,
                least,
                most,
                inputs,
                bones: pose_bones,
            });
        }
        Ok(Self { name, bones, poses })
    }

    /// The pose named `name`, whatever its letters' case.
    pub fn pose(&self, name: &str) -> Option<&Pose> {
        self.poses
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
    }
}

impl AnimationSet {
    /// Parses a `facefxanimationset` asset.
    pub fn parse(asset: &[u8]) -> Result<Self, FaceFxError> {
        let mut b = Body::open(asset)?;
        b.expect_u32(0, "0")?;
        b.expect_u32(0, "0")?;
        let name = b.name()?;
        b.r.u16()?;
        let actor = b.name()?;
        b.r.u32()?;
        b.name()?;
        b.r.u32()?;
        let count = b.count(40)?;
        let mut animations = Vec::with_capacity(count);
        for k in 0..count {
            b.expect_u32(0, "0 before an animation")?;
            let name = b.name()?;
            b.r.u32()?;
            let curve_count = b.count(16)?;
            let mut names = Vec::with_capacity(curve_count);
            for _ in 0..curve_count {
                b.expect_u32(0, "0 before a curve")?;
                names.push(b.name()?);
                b.r.u32()?;
                b.r.u16()?;
            }
            let key_count = b.count(16)?;
            let mut keys = Vec::with_capacity(key_count);
            for _ in 0..key_count {
                keys.push(Key {
                    seconds: b.f32()?,
                    value: b.f32()?,
                    slope_in: b.f32()?,
                    slope_out: b.f32()?,
                });
            }
            b.r.u16()?;
            b.expect_u32(curve_count as u32, "the curve count again")?;
            let mut counts = Vec::with_capacity(curve_count);
            for _ in 0..curve_count {
                counts.push(b.r.u32()? as usize);
            }
            let counted: u64 = counts.iter().map(|&c| c as u64).sum();
            if counted != key_count as u64 {
                return Err(FaceFxError::KeyCounts {
                    animation: k,
                    counted,
                    keys: key_count,
                });
            }
            let mut at = 0;
            let curves = names
                .into_iter()
                .zip(counts)
                .map(|(name, n)| {
                    let curve = Curve {
                        name,
                        keys: keys[at..at + n].to_vec(),
                    };
                    at += n;
                    curve
                })
                .collect();
            let blend_in = b.f32()?;
            let blend_out = b.f32()?;
            b.r.take(22)?;
            animations.push(Animation {
                name,
                curves,
                blend_in,
                blend_out,
            });
        }
        Ok(Self {
            name,
            actor,
            animations,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `UCFX` container with one `data` chunk.
    fn wrap(archive: &[u8]) -> Vec<u8> {
        crate::testutil::ucfx(&[crate::testutil::UcfxNode::Leaf(*b"data", archive.to_vec())])
    }

    fn string(out: &mut Vec<u8>, s: &[u8]) {
        out.extend(1u16.to_le_bytes());
        out.extend((s.len() as u32).to_le_bytes());
        out.extend(s);
    }

    fn head(names: &[&str]) -> Vec<u8> {
        let mut out = b"FACE".to_vec();
        out.extend(VERSION.to_le_bytes());
        string(&mut out, b"someone");
        string(&mut out, b"something");
        out.extend(1000u32.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend((names.len() as u32).to_le_bytes());
        for n in names {
            out.extend(0u16.to_le_bytes());
            let mut bytes = n.as_bytes().to_vec();
            bytes.push(0);
            string(&mut out, &bytes);
        }
        out
    }

    fn name(out: &mut Vec<u8>, index: u32) {
        out.extend(1u16.to_le_bytes());
        out.extend(index.to_le_bytes());
    }

    fn floats(out: &mut Vec<u8>, f: &[f32]) {
        for v in f {
            out.extend(v.to_le_bytes());
        }
    }

    fn actor_bytes() -> Vec<u8> {
        let mut out = head(&[
            "Rig",
            "bone_jaw",
            "FxBonePoseNode",
            "Oat",
            "FxCombinerNode",
            "Blink",
            "linear",
        ]);
        out.extend(0x21u32.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        name(&mut out, 0);
        out.extend(2u16.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        // The bone at rest.
        out.extend(0u32.to_le_bytes());
        name(&mut out, 1);
        out.extend(0u16.to_le_bytes());
        floats(
            &mut out,
            &[0.0, -0.03, 0.05, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        );
        out.extend(31u32.to_le_bytes());
        floats(&mut out, &[1.0]);
        out.extend(0u16.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        out.extend(2u32.to_le_bytes());
        // A combiner, `Blink`, from -1 to 1, fed by nothing.
        out.extend(0u32.to_le_bytes());
        name(&mut out, 4);
        out.extend(0x5cu32.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        name(&mut out, 5);
        out.extend(2u16.to_le_bytes());
        floats(&mut out, &[-1.0, 1.0]);
        out.extend([0u8; 10]);
        out.extend([0u8; 12]);
        // The pose, fed by `Blink` through `linear` 2 x + 0.5.
        out.extend(1u32.to_le_bytes());
        name(&mut out, 2);
        out.extend(0x5cu32.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        name(&mut out, 3);
        out.extend(2u16.to_le_bytes());
        floats(&mut out, &[0.0, 1.0]);
        out.extend(0u32.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        name(&mut out, 5);
        name(&mut out, 6);
        out.extend(0u32.to_le_bytes());
        out.extend(2u32.to_le_bytes());
        floats(&mut out, &[2.0, 0.5]);
        out.extend(u32::MAX.to_le_bytes());
        out.extend([0u8; 12]);
        out.extend(0u16.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        name(&mut out, 1);
        out.extend(0u16.to_le_bytes());
        floats(
            &mut out,
            &[0.0, -0.05, 0.05, 0.9, 0.1, 0.2, 0.3, 1.0, 1.0, 1.0],
        );
        out.extend([0u8; 52]);
        out
    }

    fn set_bytes() -> Vec<u8> {
        let mut out = head(&["Scene", "Rig", "Line-01", "Oat", "Eat"]);
        out.extend(0u32.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        name(&mut out, 0);
        out.extend(0u16.to_le_bytes());
        name(&mut out, 1);
        out.extend(0u32.to_le_bytes());
        name(&mut out, 0);
        out.extend(0u32.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        // The animation: two curves, three keys, all the second curve's
        // but one.
        out.extend(0u32.to_le_bytes());
        name(&mut out, 2);
        out.extend(6u32.to_le_bytes());
        out.extend(2u32.to_le_bytes());
        for curve in [3, 4] {
            out.extend(0u32.to_le_bytes());
            name(&mut out, curve);
            out.extend(1u32.to_le_bytes());
            out.extend(0u16.to_le_bytes());
        }
        out.extend(3u32.to_le_bytes());
        floats(&mut out, &[0.5, 0.25, 0.0, 0.0]);
        floats(&mut out, &[0.0, 0.0, 0.0, 0.0]);
        floats(&mut out, &[1.0, 0.8, 0.0, 0.0]);
        out.extend(0u16.to_le_bytes());
        out.extend(2u32.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        out.extend(2u32.to_le_bytes());
        floats(&mut out, &[0.16, 0.22]);
        out.extend([0u8; 22]);
        out
    }

    #[test]
    fn an_actor_reads_its_bones_and_poses() {
        let a = Actor::parse(&wrap(&actor_bytes())).unwrap();
        assert_eq!(a.name, "Rig");
        assert_eq!(a.bones.len(), 1);
        assert_eq!(a.bones[0].name, "bone_jaw");
        assert_eq!(a.bones[0].index, 31);
        // Stored w first, kept x, y, z, w.
        assert_eq!(a.bones[0].rest.rotation, [0.0, 0.0, 0.0, 1.0]);
        let blink = a.pose("Blink").unwrap();
        assert_eq!(blink.class, COMBINER_NODE);
        assert!(blink.bones.is_empty() && blink.inputs.is_empty());
        assert_eq!((blink.least, blink.most), (-1.0, 1.0));
        let oat = a.pose("oat").unwrap();
        assert_eq!(oat.class, BONE_POSE_NODE);
        assert_eq!((oat.least, oat.most), (0.0, 1.0));
        assert_eq!(oat.inputs.len(), 1);
        assert_eq!(oat.inputs[0].from, "Blink");
        assert_eq!(oat.inputs[0].apply(0.25), 1.0);
        assert_eq!(oat.bones[0].0, "bone_jaw");
        assert_eq!(oat.bones[0].1.position, [0.0, -0.05, 0.05]);
        assert_eq!(oat.bones[0].1.rotation, [0.1, 0.2, 0.3, 0.9]);
        assert!(a.pose("Eat").is_none());
    }

    #[test]
    fn a_set_reads_its_animations_and_shares_the_keys_out() {
        let s = AnimationSet::parse(&wrap(&set_bytes())).unwrap();
        assert_eq!((s.name.as_str(), s.actor.as_str()), ("Scene", "Rig"));
        let line = s.animation("line-01").unwrap();
        assert_eq!((line.blend_in, line.blend_out), (0.16, 0.22));
        assert_eq!(line.curves[0].name, "Oat");
        assert_eq!(line.curves[0].keys.len(), 1);
        assert_eq!(line.curves[1].keys.len(), 2);
        assert_eq!(line.seconds(), 1.0);
        // One key: its value throughout.
        assert_eq!(line.curves[0].value(0.0), 0.25);
        assert_eq!(line.curves[0].value(9.0), 0.25);
        // Straight between two, held outside them.
        let eat = &line.curves[1];
        assert_eq!(eat.value(-1.0), 0.0);
        assert!((eat.value(0.5) - 0.4).abs() < 1e-6);
        assert_eq!(eat.value(2.0), 0.8);
        assert_eq!(
            Curve {
                name: String::new(),
                keys: Vec::new()
            }
            .value(1.0),
            0.0
        );
    }

    #[test]
    fn bad_archives_are_errors() {
        // Not FACE, another version.
        let mut bad = actor_bytes();
        bad[0] = b'X';
        assert!(matches!(
            Actor::parse(&wrap(&bad)),
            Err(FaceFxError::BadMagic)
        ));
        let mut old = actor_bytes();
        old[4] = 0;
        assert!(matches!(
            Actor::parse(&wrap(&old)),
            Err(FaceFxError::Version(_))
        ));
        // Cut short anywhere, never a panic.
        let whole = actor_bytes();
        for len in 0..whole.len() - 52 {
            assert!(Actor::parse(&wrap(&whole[..len])).is_err(), "{len}");
        }
        let whole = set_bytes();
        for len in 0..whole.len() {
            assert!(AnimationSet::parse(&wrap(&whole[..len])).is_err(), "{len}");
        }
        // Key counts that do not add up.
        let mut set = set_bytes();
        let n = set.len();
        set[n - 30 - 4] = 5;
        assert!(matches!(
            AnimationSet::parse(&wrap(&set)),
            Err(FaceFxError::KeyCounts { .. })
        ));
        // A pose of a class this does not read.
        let known = [
            "Rig",
            "bone_jaw",
            "FxBonePoseNode",
            "Oat",
            "FxCombinerNode",
            "Blink",
            "linear",
        ];
        let mut unknown = known;
        unknown[4] = "FxDeltaNodeXX";
        let mut other = head(&unknown);
        other.extend(&actor_bytes()[head(&known).len()..]);
        assert!(matches!(
            Actor::parse(&wrap(&other)),
            Err(FaceFxError::UnknownClass { .. })
        ));
    }
}
