//! The island's water heights: the one `watr` asset (type `4d7d30c4`, in
//! the resident block), a square grid of water-surface heights over the
//! whole island. It holds the rivers, pools and lakes inland as well as
//! the sea, which the game draws as a separate plane.
//!
//! Layout (round DV, 2026-10-07), all little-endian:
//! - `+0x00` u32 5 (a version, inferred), u32 columns, u32 rows (257, 257),
//!   f32 cell size in metres (32), f32 lowest value (-50: no water), f32
//!   highest value, then a second, coarser grid's f32 cell size (64) and
//!   u32 columns and rows (129, 129).
//! - `+0x24` the fine grid: columns x rows f32 heights, row-major, rows
//!   along +z, centred on the origin (so -4096..4096 m at 32 m).
//! - Then data not yet decoded (byte masks for the coarse grid, then u16
//!   values).
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
    /// Metres per cell.
    pub cell: f32,
    /// World x and z of the grid's first corner.
    pub origin: [f32; 2],
    /// Row-major heights, `rows` rows of `columns`.
    pub heights: Vec<f32>,
}

impl WaterHeights {
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
        let mut r = Reader::at(data, 0x24);
        let mut heights = Vec::with_capacity(columns * rows);
        for _ in 0..columns * rows {
            heights.push(
                r.u32()
                    .map(f32::from_bits)
                    .map_err(|_| WaterError::Truncated)?,
            );
        }
        // Centred on the origin: inferred from its span (257 x 32 m reaches
        // -4096..4096 from the first corner, as round DV placed it).
        let half = (columns - 1) as f32 * cell / 2.0;
        let half_z = (rows - 1) as f32 * cell / 2.0;
        Ok(Self {
            columns,
            rows,
            cell,
            origin: [-half, -half_z],
            heights,
        })
    }

    /// The height of cell (`column`, `row`), if inside the grid.
    pub fn get(&self, column: usize, row: usize) -> Option<f32> {
        if column >= self.columns || row >= self.rows {
            return None;
        }
        self.heights.get(row * self.columns + column).copied()
    }

    /// The water height of the cell holding world (`x`, `z`), `None` where
    /// there is none or outside the grid.
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
        assert_eq!(out.len(), 0x24);
        for h in heights {
            out.extend(h.to_le_bytes());
        }
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
        // Row 1 (z 0..32), columns 0 and 1 (x -32..0, 0..32).
        assert_eq!(w.at(-10.0, 5.0), Some(-15.8));
        assert_eq!(w.at(10.0, 5.0), Some(-15.8));
        assert_eq!(w.at(10.0, -5.0), Some(-36.0));
        assert_eq!(w.at(-10.0, -5.0), None);
        assert_eq!(w.at(-40.0, 0.0), None);
    }

    #[test]
    fn rejects_short_grids() {
        let mut s = section(3, 3, 32.0, &[0.0; 9]);
        s.truncate(s.len() - 1);
        assert!(matches!(
            WaterHeights::parse_section(&s),
            Err(WaterError::Truncated)
        ));
    }
}
