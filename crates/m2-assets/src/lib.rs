//! Parsers for file formats read at runtime from the user's own installation.
//!
//! No game files are stored in this repository; tests use synthetic data.
//!
//! - [`wad`]: the `FFCS` archive container (`data\*.wad`): header, block
//!   index, asset table and block paths, with read access to whole blocks.
//! - [`sges`]: the segmented container each block is stored in, made of raw
//!   DEFLATE or stored segments of up to 64 KiB.
//!
//! # Provenance
//!
//! Format-from-samples. The layouts were inferred by inspecting the headers
//! and tables of the four archives in a legally installed copy (EA app
//! edition) and checking the inferred sector arithmetic and decompression
//! against those files. No executable code was read, no addresses are
//! claimed, and no bytes from the archives are committed. Fields whose
//! meaning is not yet known are kept and named by offset.

mod bytes;
pub mod sges;
pub mod wad;

#[cfg(test)]
mod testutil;

pub use sges::{Segment, SgesError, SgesHeader};
pub use wad::{Asset, Block, Header, WadArchive, WadError, WadIndex};
