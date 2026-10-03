//! Material animations: keyed material parameters, stored under the same
//! asset type hash as skeletal animations (`18166555`), which
//! [`Clip::parse`](crate::animation::Clip::parse) rejects with
//! [`AnimationError::MaterialAnimation`](crate::animation::AnimationError::MaterialAnimation).
//!
//! A `UCFX` container of top-level leaf sections: `MANM`, then for each
//! material a `MINF` followed by that material's `TRCK`s.
//!
//! ```text
//! MANM  0x00  u32  version (2)
//!       0x04  u32  frame count (keys can sit on this frame, so it may be
//!                  the last frame rather than a count)
//!       0x08  f32  frames per second (15, 29.97 or 59.94)
//!       0x0C  u32  MINF count
//! MINF  0x00  u32  material name hash (a model MTRL record's)
//!       0x04  u16  TRCK count
//! TRCK  0x00  u32  parameter hash (meaning unknown)
//!       0x04  f32  divisor: a key's value is probably raw / divisor
//!       0x08  u16  key count k (at least 1)
//!       0x0A  u16  flags (0 or 1, rarely other values)
//!       0x0C  i16  value at frame 0
//!       0x0E  {u16 frame, i16 value} x (k - 1), frames increasing
//! ```
//!
//! Parameters come in threes and sixes, so they are probably colours.
//!
//! # Provenance
//!
//! Format-from-samples: inferred from the 29 material animations in the
//! archives of a legally installed copy, which all parse with these rules.
//! 85 of their 86 material hashes name a model's material. No bytes from
//! the archives are committed.

use crate::bytes::{OutOfBounds, Reader};
use crate::ucfx::{Ucfx, UcfxError};

pub const HEADER: [u8; 4] = *b"MANM";
pub const MATERIAL: [u8; 4] = *b"MINF";
pub const TRACK: [u8; 4] = *b"TRCK";

/// Errors from parsing a material animation.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MaterialAnimError {
    #[error("container: {0}")]
    Ucfx(#[from] UcfxError),
    #[error("truncated: needed {len} bytes at {offset:#x}")]
    Truncated { offset: usize, len: usize },
    #[error("section {index} is {found:?}, expected {expected:?}")]
    UnexpectedSection {
        index: usize,
        found: [u8; 4],
        expected: [u8; 4],
    },
    #[error("MANM says {expected} materials, found {found}")]
    MaterialCount { expected: u32, found: usize },
    #[error("track {index} has {len:#x} bytes for {keys} keys")]
    TrackLength { index: usize, len: usize, keys: u16 },
    #[error("track {0} has key frames out of order")]
    KeyOrder(usize),
}

impl From<OutOfBounds> for MaterialAnimError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// One keyed parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamTrack {
    pub param_hash: u32,
    pub divisor: f32,
    pub flags: u16,
    /// `(frame, raw value)`, the first at frame 0.
    pub keys: Vec<(u16, i16)>,
}

impl ParamTrack {
    /// A key's value, assuming raw / divisor.
    pub fn value(&self, raw: i16) -> f32 {
        f32::from(raw) / self.divisor
    }

    /// The value at `frame`, linearly interpolated between keys and held
    /// after the last one. Interpolation is assumed, not verified.
    pub fn sample(&self, frame: f32) -> f32 {
        match self.keys.iter().position(|k| f32::from(k.0) > frame) {
            None => self.keys.last().map_or(0.0, |k| self.value(k.1)),
            Some(0) => self.value(self.keys[0].1),
            Some(i) => {
                let (a, b) = (self.keys[i - 1], self.keys[i]);
                let t = (frame - f32::from(a.0)) / (f32::from(b.0) - f32::from(a.0));
                let (va, vb) = (self.value(a.1), self.value(b.1));
                va + (vb - va) * t
            }
        }
    }
}

/// The tracks for one material.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialTracks {
    pub material_hash: u32,
    pub tracks: Vec<ParamTrack>,
}

/// A parsed material animation.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialAnimation {
    pub version: u32,
    pub frames: u32,
    pub fps: f32,
    pub materials: Vec<MaterialTracks>,
}

impl MaterialAnimation {
    /// Whether `asset` is a material animation rather than a skeletal one.
    pub fn is_material(asset: &[u8]) -> bool {
        Ucfx::parse(asset).is_ok_and(|u| u.sections().first().is_some_and(|s| s.fourcc == HEADER))
    }

    pub fn parse(asset: &[u8]) -> Result<Self, MaterialAnimError> {
        let u = Ucfx::parse(asset)?;
        let sections = u.sections();
        let section = |index: usize, expected: [u8; 4]| {
            let s = sections.get(index).ok_or(MaterialAnimError::Truncated {
                offset: index,
                len: 1,
            })?;
            match s.data {
                Some(d) if s.fourcc == expected && s.depth == 0 => Ok(d),
                _ => Err(MaterialAnimError::UnexpectedSection {
                    index,
                    found: s.fourcc,
                    expected,
                }),
            }
        };
        let mut r = Reader::at(section(0, HEADER)?, 0);
        let version = r.u32()?;
        let frames = r.u32()?;
        let fps = f32::from_bits(r.u32()?);
        let count = r.u32()?;

        let mut materials = Vec::new();
        let mut index = 1;
        while index < sections.len() {
            let mut r = Reader::at(section(index, MATERIAL)?, 0);
            let material_hash = r.u32()?;
            let n = usize::from(r.u16()?);
            index += 1;
            let mut tracks = Vec::with_capacity(n.min(sections.len()));
            for _ in 0..n {
                let t = section(index, TRACK)?;
                let mut r = Reader::at(t, 0);
                let param_hash = r.u32()?;
                let divisor = f32::from_bits(r.u32()?);
                let keys = r.u16()?;
                let flags = r.u16()?;
                if keys == 0 || t.len() != 14 + 4 * (usize::from(keys) - 1) {
                    return Err(MaterialAnimError::TrackLength {
                        index,
                        len: t.len(),
                        keys,
                    });
                }
                let mut list = vec![(0, r.u16()? as i16)];
                for _ in 1..keys {
                    list.push((r.u16()?, r.u16()? as i16));
                }
                if !list.windows(2).all(|w| w[0].0 < w[1].0) {
                    return Err(MaterialAnimError::KeyOrder(index));
                }
                tracks.push(ParamTrack {
                    param_hash,
                    divisor,
                    flags,
                    keys: list,
                });
                index += 1;
            }
            materials.push(MaterialTracks {
                material_hash,
                tracks,
            });
        }
        if materials.len() as u64 != u64::from(count) {
            return Err(MaterialAnimError::MaterialCount {
                expected: count,
                found: materials.len(),
            });
        }
        Ok(Self {
            version,
            frames,
            fps,
            materials,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{UcfxNode, ucfx};

    fn track(hash: u32, divisor: f32, first: i16, keys: &[(u16, i16)]) -> UcfxNode {
        let mut t = hash.to_le_bytes().to_vec();
        t.extend(divisor.to_le_bytes());
        t.extend((keys.len() as u16 + 1).to_le_bytes());
        t.extend(0u16.to_le_bytes());
        t.extend(first.to_le_bytes());
        for (f, v) in keys {
            t.extend(f.to_le_bytes());
            t.extend(v.to_le_bytes());
        }
        UcfxNode::Leaf(TRACK, t)
    }

    fn info(hash: u32, tracks: u16) -> UcfxNode {
        let mut m = hash.to_le_bytes().to_vec();
        m.extend(tracks.to_le_bytes());
        UcfxNode::Leaf(MATERIAL, m)
    }

    fn header(materials: u32) -> UcfxNode {
        let mut h = 2u32.to_le_bytes().to_vec();
        h.extend(10u32.to_le_bytes());
        h.extend(15.0f32.to_le_bytes());
        h.extend(materials.to_le_bytes());
        UcfxNode::Leaf(HEADER, h)
    }

    fn sample() -> Vec<UcfxNode> {
        vec![
            header(2),
            info(0xA, 2),
            track(1, 32767.0, 0x7FFF, &[(5, 0), (10, 0x7FFF)]),
            track(2, 16383.0, 0, &[]),
            info(0xB, 1),
            track(3, 655.34, 655, &[(4, -655)]),
        ]
    }

    #[test]
    fn parses_and_samples() {
        let a = ucfx(&sample());
        assert!(MaterialAnimation::is_material(&a));
        let m = MaterialAnimation::parse(&a).unwrap();
        assert_eq!((m.version, m.frames, m.fps), (2, 10, 15.0));
        assert_eq!(m.materials.len(), 2);
        assert_eq!(m.materials[1].material_hash, 0xB);
        let t = &m.materials[0].tracks[0];
        assert_eq!(t.keys, [(0, 0x7FFF), (5, 0), (10, 0x7FFF)]);
        assert_eq!(t.sample(0.0), 1.0);
        assert_eq!(t.sample(2.5), 0.5);
        assert_eq!(t.sample(20.0), 1.0);
        assert_eq!(m.materials[0].tracks[1].sample(3.0), 0.0);
        assert!((m.materials[1].tracks[0].sample(4.0) + 1.0).abs() < 1e-3);
    }

    #[test]
    fn rejects_bad_layouts() {
        let mut s = sample();
        s[0] = header(3);
        assert_eq!(
            MaterialAnimation::parse(&ucfx(&s)).unwrap_err(),
            MaterialAnimError::MaterialCount {
                expected: 3,
                found: 2
            }
        );
        let mut s = sample();
        s[3] = track(2, 1.0, 0, &[(5, 0), (5, 1)]);
        assert_eq!(
            MaterialAnimation::parse(&ucfx(&s)).unwrap_err(),
            MaterialAnimError::KeyOrder(3)
        );
        let mut s = sample();
        s.swap(1, 2);
        assert!(matches!(
            MaterialAnimation::parse(&ucfx(&s)),
            Err(MaterialAnimError::UnexpectedSection { index: 1, .. })
        ));
    }

    #[test]
    fn byte_flips_never_panic() {
        let a = ucfx(&sample());
        for at in 0..a.len() {
            for v in [0, 1, 0x7f, 0xff] {
                let mut b = a.clone();
                b[at] = v;
                if let Ok(m) = MaterialAnimation::parse(&b) {
                    for t in m.materials.iter().flat_map(|m| &m.tracks) {
                        let _ = t.sample(3.0);
                    }
                }
            }
        }
    }
}
