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
//! - [`font`]: the bitmap fonts the game's own GUI widgets draw text with.
//! - [`gfx`]: Scaleform GFx (Flash) movies the interface is built from:
//!   header, tags, and the layout and names they describe.
//! - [`model`]: model assets: node hierarchy, materials, LOD segments and
//!   primitive groups (including geometry streamed from other blocks).
//! - [`animation`]: skeletal animation clips: interleaved, delta and
//!   wavelet encoded node tracks, timed events, a pose sampler and model
//!   posing.
//! - [`animation_table`]: the tables that turn a character's animation key
//!   (stance and action) into a clip, per vehicle or character.
//! - [`havok`]: the binary object packfiles animation clips and collision
//!   shapes are stored in.
//! - [`path`]: patrol and vehicle routes (point lists placed by `Path` entities).
//! - [`physics`]: model collision (`PHY2`): shapes, rigid bodies and the
//!   nodes they attach to.
//! - [`sound`]: sound banks (16-bit PCM sound effects and ambience).
//! - [`terrain`]: the 400 m terrain cells and low-resolution tiles.
//! - [`scatter`]: the grass and shrub clumps instanced over each cell.
//! - [`tree`]: the tree table: each species' template, fade and crown
//!   card, and the island's trees, bit-packed in a kd-tree per species.
//! - [`layer`]: entity layers, the component tables that place terrain,
//!   buildings, props and everything else in the world.
//! - [`material_animation`]: material animations (keyed material
//!   parameters), stored under the animation type hash.
//! - [`vehicle`]: vehicle kinds from texture names and wheels from geometry.
//! - [`voice`]: voice cue tables (which streamed lines a voice set says, and how).
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
pub mod animation_table;
pub mod block;
mod bytes;
pub mod dxt;
pub mod flash;
pub mod font;
pub mod gfx;
pub mod hash;
pub mod havok;
pub mod layer;
pub mod lua;
pub mod material_animation;
pub mod minimap;
pub mod model;
pub mod path;
pub mod physics;
pub mod region;
pub mod scatter;
pub mod sges;
pub mod sound;
pub mod strings;
pub mod terrain;
pub mod texture;
pub mod tree;
pub mod ucfx;
pub mod vehicle;
pub mod vertex;
pub mod voice;
pub mod wad;

#[cfg(test)]
mod testutil;

pub use animation::{AnimationError, Clip, Codec, Transform};
pub use block::{BlockContents, BlockEntry, BlockError};
pub use dxt::{DxtError, DxtFormat};
pub use font::{Font, FontError, Glyph};
pub use hash::name_hash;
pub use havok::{Packfile, PackfileError};
pub use layer::{Component, Layer, LayerError, Placement};
pub use material_animation::{MaterialAnimError, MaterialAnimation};
pub use model::{Model, ModelError, PrimitiveGroup};
pub use path::{Path, PathError};
pub use physics::{Physics, PhysicsError};
pub use region::{Region, RegionError};
pub use scatter::{Scatter, ScatterError};
pub use sges::{Segment, SgesError, SgesHeader};
pub use sound::{SoundBank, SoundError, StreamIndex};
pub use strings::{StringTable, StringsError};
pub use voice::{CueTable, Group, VoiceError};

pub use terrain::{LowResTile, TerrainCell, TerrainError};
pub use texture::{Mip, PixelFormat, Texture, TextureError, TextureInfo, TextureKind};
pub use tree::{TreeError, TreeTable};
pub use ucfx::{Section, Ucfx, UcfxError};
pub use vertex::{VertexDecl, VertexError, VertexStream};
pub use wad::{Asset, Block, Header, WadArchive, WadError, WadIndex};
