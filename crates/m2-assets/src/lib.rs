//! Parsers for file formats read at runtime from the user's own installation.
//!
//! No game files are stored in this repository; tests use synthetic data.
//!
//! - [`wad`]: the `FFCS` archive container (`data\*.wad`): header, block
//!   index, asset table and block paths, with read access to whole blocks.
//! - [`sges`]: the segmented container each block is stored in, made of raw
//!   DEFLATE or stored segments of up to 64 KiB.
//! - [`block`]: the asset directory at the start of a decoded block.
//! - [`ucfx`]: the sectioned container each asset is stored in, including
//!   the asset's name where it has one.
//!
//! # Provenance
//!
//! Format-from-samples. The archive, block directory and asset container
//! layouts were inferred by inspecting the four archives in a legally
//! installed copy (EA app edition) and checking the inferred sector
//! arithmetic, decompression and asset boundaries against those files. No
//! executable code was read, no addresses are claimed, and no bytes from the
//! archives are committed. Fields whose meaning is not yet known are kept and
//! named by offset.

pub mod block;
mod bytes;
pub mod sges;
pub mod ucfx;
pub mod wad;

#[cfg(test)]
mod testutil;

pub use block::{BlockContents, BlockEntry, BlockError};
pub use sges::{Segment, SgesError, SgesHeader};
pub use ucfx::{Section, Ucfx, UcfxError};
pub use wad::{Asset, Block, Header, WadArchive, WadError, WadIndex};
