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
//! - [`texture`]: texture assets: header, mip chain (including mips streamed
//!   from other blocks) and conversion to RGBA8.
//! - [`dxt`]: DXT1, DXT3 and DXT5 decoding.
//! - [`model`]: model assets: node hierarchy, materials, LOD segments and
//!   primitive groups (including geometry streamed from other blocks).
//! - [`vertex`]: Direct3D 9 vertex declarations and vertex streams, mapped
//!   onto `m2-render` vertex layouts.
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
pub mod dxt;
pub mod model;
pub mod sges;
pub mod texture;
pub mod ucfx;
pub mod vertex;
pub mod wad;

#[cfg(test)]
mod testutil;

pub use block::{BlockContents, BlockEntry, BlockError};
pub use dxt::{DxtError, DxtFormat};
pub use model::{Model, ModelError, PrimitiveGroup};
pub use sges::{Segment, SgesError, SgesHeader};
pub use texture::{Mip, PixelFormat, Texture, TextureError, TextureInfo, TextureKind};
pub use ucfx::{Section, Ucfx, UcfxError};
pub use vertex::{VertexDecl, VertexError, VertexStream};
pub use wad::{Asset, Block, Header, WadArchive, WadError, WadIndex};
