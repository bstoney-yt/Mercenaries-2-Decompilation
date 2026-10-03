//! Paths: the routes patrols walk and boats and vehicles follow.
//!
//! A path asset (type hash `bcfe6314`, all 5,194 in the `resident` block)
//! is a `UCFX` container whose `data` section holds a list of points:
//!
//! ```text
//! 0x00  u32  0x00010002 (version or flags; the same on every path)
//! 0x04  u32  point count n
//! 0x08  n × f32x3 points, in the space of the entity that names the path
//! ```
//!
//! An entity with a `Path` component names its path asset by hash (the
//! component's one field), and its `Transform` places the points in the
//! world. A soldier's `AiPatrol` component names the path entity it walks
//! (its third field).
//!
//! # Provenance
//!
//! Format-from-samples: inferred from the path assets in `vz.wad` of a
//! legally installed copy. Every asset's length is exactly 8 + 12n, and
//! 1,085 of the 1,134 `AiPatrol` records name a `Path` entity. No bytes from
//! the archives are committed.

use crate::bytes::{OutOfBounds, Reader};
use crate::ucfx::{Ucfx, UcfxError};

/// Asset type hash of paths.
pub const PATH_TYPE: u32 = 0xbcfe_6314;

/// Errors from parsing a path.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PathError {
    #[error("container: {0}")]
    Ucfx(#[from] UcfxError),
    #[error("truncated: needed {len} bytes at {offset:#x}")]
    Truncated { offset: usize, len: usize },
    #[error("no data section")]
    NoData,
    #[error("{len} bytes for {count} points")]
    Length { len: usize, count: u32 },
}

impl From<OutOfBounds> for PathError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// A parsed path.
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    pub flags: u32,
    /// In the space of the entity that names the path.
    pub points: Vec<[f32; 3]>,
}

impl Path {
    /// Parses a path asset.
    pub fn parse(asset: &[u8]) -> Result<Self, PathError> {
        let data = Ucfx::parse(asset)?
            .top_level(*b"data")
            .ok_or(PathError::NoData)?;
        Self::parse_data(data)
    }

    /// Parses a path's `data` section.
    pub fn parse_data(data: &[u8]) -> Result<Self, PathError> {
        let mut r = Reader::at(data, 0);
        let (flags, count) = (r.u32()?, r.u32()?);
        if data.len() as u64 != 8 + 12 * u64::from(count) {
            return Err(PathError::Length {
                len: data.len(),
                count,
            });
        }
        let mut points = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let (x, y, z) = (r.u32()?, r.u32()?, r.u32()?);
            points.push([x, y, z].map(f32::from_bits));
        }
        Ok(Self { flags, points })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(points: &[[f32; 3]]) -> Vec<u8> {
        let mut d: Vec<u8> = [0x0001_0002u32, points.len() as u32]
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        d.extend(points.iter().flatten().flat_map(|v| v.to_le_bytes()));
        d
    }

    #[test]
    fn parses_points() {
        let p = Path::parse_data(&data(&[[1.0, 0.0, 2.0], [-3.0, 0.5, 4.0]])).unwrap();
        assert_eq!(p.flags, 0x0001_0002);
        assert_eq!(p.points, [[1.0, 0.0, 2.0], [-3.0, 0.5, 4.0]]);
        let asset =
            crate::testutil::ucfx(&[crate::testutil::UcfxNode::Leaf(*b"data", data(&[[0.0; 3]]))]);
        assert_eq!(Path::parse(&asset).unwrap().points.len(), 1);
    }

    #[test]
    fn rejects_bad_lengths() {
        let mut d = data(&[[1.0, 2.0, 3.0]]);
        d.push(0);
        assert_eq!(
            Path::parse_data(&d).unwrap_err(),
            PathError::Length { len: 21, count: 1 }
        );
        assert!(matches!(
            Path::parse_data(&d[..6]),
            Err(PathError::Truncated { .. })
        ));
    }

    #[test]
    fn byte_flips_never_panic() {
        let d = data(&[[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]);
        for at in 0..d.len() {
            for v in [0, 1, 0x7f, 0xff] {
                let mut b = d.clone();
                b[at] = v;
                let _ = Path::parse_data(&b);
            }
        }
    }
}
