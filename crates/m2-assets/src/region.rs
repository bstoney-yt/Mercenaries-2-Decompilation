//! Line regions: the flat areas scripts wait for the player to enter
//! (`Event.Boundary`), such as a mission's checkpoints.
//!
//! An entity with a `LineRegion` component names its shape asset by hash
//! (the component's one field). The shape (type hash `6310807f`, in the
//! `resident` block) is a `UCFX` container whose `data` section holds an
//! outline on the ground:
//!
//! ```text
//! 0x00  u32  0x00040002 (the same on every sample)
//! 0x04  u32  0
//! 0x08  n × f32x2 (x, z) corners, around the entity's position
//! ```
//!
//! The entity's `Transform` places the outline in the world. A region has
//! no height: it is the column above and below the outline.
//!
//! # Provenance
//!
//! Format-from-samples: inferred from the shapes the five `VzaCon001`
//! checkpoint regions name, in `vz.wad` of a legally installed copy (each
//! 88 bytes, four corners, the corners pairwise opposite about the
//! centre). No bytes from the archives are committed.

use crate::bytes::{OutOfBounds, Reader};
use crate::ucfx::{Ucfx, UcfxError};

/// Asset type hash of line-region shapes.
pub const REGION_TYPE: u32 = 0x6310_807f;

/// Errors from parsing a region shape.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RegionError {
    #[error("container: {0}")]
    Ucfx(#[from] UcfxError),
    #[error("truncated: needed {len} bytes at {offset:#x}")]
    Truncated { offset: usize, len: usize },
    #[error("no data section")]
    NoData,
    #[error("{0} bytes is not a whole outline")]
    Length(usize),
}

impl From<OutOfBounds> for RegionError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// A region's outline.
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub flags: u32,
    pub unk_0x04: u32,
    /// (x, z) corners in the space of the entity that names the shape.
    pub corners: Vec<[f32; 2]>,
}

impl Region {
    /// Parses a region shape asset.
    pub fn parse(asset: &[u8]) -> Result<Self, RegionError> {
        let data = Ucfx::parse(asset)?
            .top_level(*b"data")
            .ok_or(RegionError::NoData)?;
        Self::parse_data(data)
    }

    /// Parses a shape's `data` section.
    pub fn parse_data(data: &[u8]) -> Result<Self, RegionError> {
        if data.len() < 8 + 3 * 8 || !(data.len() - 8).is_multiple_of(8) {
            return Err(RegionError::Length(data.len()));
        }
        let mut r = Reader::at(data, 0);
        let (flags, unk_0x04) = (r.u32()?, r.u32()?);
        let corners = (0..(data.len() - 8) / 8)
            .map(|_| Ok([f32::from_bits(r.u32()?), f32::from_bits(r.u32()?)]))
            .collect::<Result<_, OutOfBounds>>()?;
        Ok(Self {
            flags,
            unk_0x04,
            corners,
        })
    }

    /// Whether world point (x, z) is inside the outline placed at
    /// `centre` (x, z) turned by `yaw` radians (about the up axis).
    pub fn contains(&self, centre: [f32; 2], yaw: f32, x: f32, z: f32) -> bool {
        // Into the region's own space.
        let (dx, dz) = (x - centre[0], z - centre[1]);
        let (s, c) = (-yaw).sin_cos();
        let (px, pz) = (dx * c + dz * s, -dx * s + dz * c);
        // Even-odd rule.
        let n = self.corners.len();
        let mut inside = false;
        for i in 0..n {
            let [ax, az] = self.corners[i];
            let [bx, bz] = self.corners[(i + n - 1) % n];
            if (az > pz) != (bz > pz) && px < (bx - ax) * (pz - az) / (bz - az) + ax {
                inside = !inside;
            }
        }
        inside
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{UcfxNode, ucfx};

    fn square() -> Vec<u8> {
        let mut data = Vec::new();
        for w in [0x0004_0002u32, 0] {
            data.extend(w.to_le_bytes());
        }
        // A diamond, corners opposite about the centre, as in the game.
        for f in [10.0f32, 0.0, 0.0, 10.0, -10.0, 0.0, 0.0, -10.0] {
            data.extend(f.to_le_bytes());
        }
        ucfx(&[UcfxNode::Leaf(*b"data", data)])
    }

    #[test]
    fn outlines_contain_points_inside_them() {
        let r = Region::parse(&square()).unwrap();
        assert_eq!(r.corners.len(), 4);
        assert_eq!(r.flags, 0x0004_0002);
        let at = [100.0, 200.0];
        assert!(r.contains(at, 0.0, 100.0, 200.0));
        assert!(r.contains(at, 0.0, 104.0, 204.0));
        assert!(!r.contains(at, 0.0, 106.0, 206.0));
        // Turned 45 degrees the diamond is a square of half-side 7.07.
        let yaw = std::f32::consts::FRAC_PI_4;
        assert!(r.contains(at, yaw, 106.0, 206.0));
        assert!(!r.contains(at, yaw, 108.0, 200.0));
    }

    #[test]
    fn short_data_is_an_error() {
        assert_eq!(Region::parse_data(&[0; 12]), Err(RegionError::Length(12)));
    }
}
