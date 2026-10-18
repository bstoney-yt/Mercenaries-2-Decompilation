//! The island's water heights: the one `watr` asset (type `4d7d30c4`, in
//! the resident block), a square grid of water-surface heights over the
//! whole island. It holds the rivers, pools and lakes inland as well as
//! the sea, which the game draws as a separate plane.
//!
//! Layout (round DV, 2026-10-07; heights' start corrected in playtest 5,
//! 2026-10-16), all little-endian, from samples:
//! - `+0x00` u32 5 (a version, inferred), u32 columns, u32 rows (257, 257),
//!   f32 cell size in metres (32), f32 lowest value (-50: no water), f32
//!   highest value (325.256, the grid's own highest), then a second,
//!   coarser grid's f32 cell size (64) and u32 columns and rows (129,
//!   129), then two f32 words not named (-49.83 and 0.0057).
//! - `+0x2c` the fine grid: columns x rows f32 heights, row-major, rows
//!   along +z: points 32 m apart from -4096 to 4096 m (257 points span
//!   256 cells), each taken as the height of the square around it
//!   (inferred). Round DV read them from `+0x24`, two points early, which
//!   put every pool and lake 64 m east of where the data has it: the
//!   per-point mask below lines up with the heights only from `+0x2c`
//!   (66,038 of 66,049 points from `+0x24`, all from `+0x2c`), and the
//!   tail of the old reading is two sea points.
//! - Then one byte per fine point, the same row order: `0xff` where the
//!   point has water, `0` where it reads -50 (38,078 and 27,971 points;
//!   no other value).
//! - Then 132,094 bytes not decoded: mostly `0xff` with runs of `0` and
//!   part-filled bytes (`0x7f`, `0x3f`), repeating best every 257 bytes
//!   (about 514 rows of 257 bytes; 4 bytes short of that). A finer mask,
//!   inferred, not read.
//! - Last, 129 x 129 u16 (the coarse grid, 33,282 bytes, ending the
//!   section). Read as the unnamed word at `+0x24` plus the value times
//!   the one at `+0x28`, 7,816 of the 16,638 points not 0 or 0xffff equal
//!   the fine height at the same place within 1 cm (the sea, 2427 =
//!   -36.0, is 12,095 of them); the rest differ by metres. What they hold
//!   is not known; not used.
//!
//! Open sea reads -36 (the sea level), as does land the sea lies under;
//! cells with no water at all read -50.

use crate::bytes::Reader;
use crate::ucfx::{Ucfx, UcfxError};

/// Asset type of the water heights.
pub const WATER_TYPE: u32 = 0x4d7d30c4;

/// What a cell with no water reads.
pub const NO_WATER: f32 = -50.0;

#[derive(Debug, thiserror::Error)]
pub enum WaterError {
    #[error(transparent)]
    Ucfx(#[from] UcfxError),
    #[error("no 'watr' section")]
    NoSection,
    #[error("water grid truncated")]
    Truncated,
    #[error("water grid of {columns} x {rows} is implausible")]
    BadSize { columns: u32, rows: u32 },
}

/// The fine grid of water-surface heights.
#[derive(Debug, Clone, PartialEq)]
pub struct WaterHeights {
    pub columns: usize,
    pub rows: usize,
    /// Metres between points.
    pub cell: f32,
    /// World x and z of the grid's first point.
    pub origin: [f32; 2],
    /// Row-major heights, `rows` rows of `columns`.
    pub heights: Vec<f32>,
    /// Row-major, whether each point has water (the byte mask after the
    /// heights); empty when the section ends before it.
    pub wet: Vec<bool>,
}

impl WaterHeights {
    /// Where the fine grid's heights start in the section.
    pub const HEIGHTS: usize = 0x2c;

    /// Reads the `watr` asset.
    pub fn parse(asset: &[u8]) -> Result<Self, WaterError> {
        let ucfx = Ucfx::parse(asset)?;
        let data = ucfx.top_level(*b"watr").ok_or(WaterError::NoSection)?;
        Self::parse_section(data)
    }

    /// Reads the `watr` section's bytes.
    pub fn parse_section(data: &[u8]) -> Result<Self, WaterError> {
        let mut r = Reader::at(data, 4);
        let columns = r.u32().map_err(|_| WaterError::Truncated)?;
        let rows = r.u32().map_err(|_| WaterError::Truncated)?;
        let cell = r
            .u32()
            .map(f32::from_bits)
            .map_err(|_| WaterError::Truncated)?;
        if columns == 0 || rows == 0 || columns > 4096 || rows > 4096 || cell <= 0.0 {
            return Err(WaterError::BadSize { columns, rows });
        }
        let (columns, rows) = (columns as usize, rows as usize);
        let mut r = Reader::at(data, Self::HEIGHTS);
        let mut heights = Vec::with_capacity(columns * rows);
        for _ in 0..columns * rows {
            heights.push(
                r.u32()
                    .map(f32::from_bits)
                    .map_err(|_| WaterError::Truncated)?,
            );
        }
        let mask = Self::HEIGHTS + columns * rows * 4;
        let wet = data
            .get(mask..mask + columns * rows)
            .map(|m| m.iter().map(|&b| b != 0).collect())
            .unwrap_or_default();
        // Centred on the world's origin: inferred from its span.
        let half = (columns - 1) as f32 * cell / 2.0;
        let half_z = (rows - 1) as f32 * cell / 2.0;
        Ok(Self {
            columns,
            rows,
            cell,
            origin: [-half, -half_z],
            heights,
            wet,
        })
    }

    /// The height at point (`column`, `row`), if inside the grid.
    pub fn get(&self, column: usize, row: usize) -> Option<f32> {
        if column >= self.columns || row >= self.rows {
            return None;
        }
        self.heights.get(row * self.columns + column).copied()
    }

    /// The water height over world (`x`, `z`): the point whose square,
    /// from the point on towards +x and +z, holds it (as the alpha draws
    /// it; inferred). `None` where there is none or outside the grid.
    pub fn at(&self, x: f32, z: f32) -> Option<f32> {
        let c = ((x - self.origin[0]) / self.cell).floor();
        let r = ((z - self.origin[1]) / self.cell).floor();
        if c < 0.0 || r < 0.0 {
            return None;
        }
        self.get(c as usize, r as usize)
            .filter(|&h| h > NO_WATER + 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(columns: u32, rows: u32, cell: f32, heights: &[f32]) -> Vec<u8> {
        let mut out = Vec::new();
        for v in [5u32, columns, rows] {
            out.extend(v.to_le_bytes());
        }
        for v in [cell, NO_WATER, 10.0, 64.0] {
            out.extend(v.to_le_bytes());
        }
        out.extend(2u32.to_le_bytes());
        out.extend(2u32.to_le_bytes());
        out.extend((-49.83f32).to_le_bytes());
        out.extend(0.0057f32.to_le_bytes());
        assert_eq!(out.len(), WaterHeights::HEIGHTS);
        for h in heights {
            out.extend(h.to_le_bytes());
        }
        out.extend(
            heights
                .iter()
                .map(|&h| if h > NO_WATER { 0xffu8 } else { 0 }),
        );
        out
    }

    #[test]
    fn reads_and_places_the_grid() {
        let w = WaterHeights::parse_section(&section(
            3,
            3,
            32.0,
            &[
                -50.0, -36.0, -50.0, //
                -15.8, -15.8, -50.0, //
                -50.0, -50.0, 2.0,
            ],
        ))
        .unwrap();
        assert_eq!((w.columns, w.rows, w.cell), (3, 3, 32.0));
        assert_eq!(w.origin, [-32.0, -32.0]);
        // Row 1 (z 0 to 32), columns 0 and 1 (x -32 to 0, 0 to 32).
        assert_eq!(w.at(-20.0, 5.0), Some(-15.8));
        assert_eq!(w.at(10.0, 25.0), Some(-15.8));
        assert_eq!(w.at(5.0, -20.0), Some(-36.0));
        assert_eq!(w.at(40.0, 40.0), Some(2.0));
        assert_eq!(w.at(-25.0, -25.0), None);
        assert_eq!(w.at(-50.0, 0.0), None);
        // The mask follows the heights.
        assert_eq!(
            w.wet,
            [false, true, false, true, true, false, false, false, true]
        );
    }

    #[test]
    fn heights_start_after_the_two_unnamed_words() {
        // The two words before the grid are not its first points.
        let w = WaterHeights::parse_section(&section(2, 1, 32.0, &[17.5, -36.0])).unwrap();
        assert_eq!(w.heights, [17.5, -36.0]);
    }

    #[test]
    fn rejects_short_grids() {
        let mut s = section(3, 3, 32.0, &[0.0; 9]);
        s.truncate(s.len() - 10);
        assert!(matches!(
            WaterHeights::parse_section(&s),
            Err(WaterError::Truncated)
        ));
    }
}
