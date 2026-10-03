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
//! - [`animation`]: skeletal animation clips: interleaved, delta and
//!   wavelet encoded node tracks, timed events, a pose sampler and model
//!   posing.
//! - [`havok`]: the binary object packfiles animation clips and collision
//!   shapes are stored in.
//! - [`physics`]: model collision (`PHY2`): shapes, rigid bodies and the
//!   nodes they attach to.
//! - [`terrain`]: the 400 m terrain cells and low-resolution tiles.
//! - [`scatter`]: the grass and shrub clumps instanced over each cell.
//! - [`layer`]: entity layers, the component tables that place terrain,
//!   buildings, props and everything else in the world.
//! - [`material_animation`]: material animations (keyed material
//!   parameters), stored under the animation type hash.
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

pub mod animation;
pub mod block;
mod bytes;
pub mod dxt;
pub mod havok;
pub mod layer;
pub mod material_animation;
pub mod model;
pub mod physics;
pub mod scatter;
pub mod sges;
pub mod terrain;
pub mod texture;
pub mod ucfx;
pub mod vertex;
pub mod wad;

#[cfg(test)]
mod testutil;

pub use animation::{AnimationError, Clip, Codec, Transform};
pub use block::{BlockContents, BlockEntry, BlockError};
pub use dxt::{DxtError, DxtFormat};
pub use havok::{Packfile, PackfileError};
pub use layer::{Component, Layer, LayerError, Placement};
pub use material_animation::{MaterialAnimError, MaterialAnimation};
pub use model::{Model, ModelError, PrimitiveGroup};
pub use physics::{Physics, PhysicsError};
pub use scatter::{Scatter, ScatterError};
pub use sges::{Segment, SgesError, SgesHeader};
pub use terrain::{LowResTile, TerrainCell, TerrainError};
pub use texture::{Mip, PixelFormat, Texture, TextureError, TextureInfo, TextureKind};
pub use ucfx::{Section, Ucfx, UcfxError};
pub use vertex::{VertexDecl, VertexError, VertexStream};
pub use wad::{Asset, Block, Header, WadArchive, WadError, WadIndex};
