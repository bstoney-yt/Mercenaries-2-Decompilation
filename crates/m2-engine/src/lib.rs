//! The engine runtime the native game is built on.
//!
//! - [`assets`]: finding the install and reading models and textures from
//!   its archives.
//! - [`model`]: models and their textures uploaded to a render backend.
//!
//! The original plan put raw `#[repr(C)]` mirrors of the executable's
//! classes here for hooking. The engine code is encrypted and can't be
//! read, so this crate instead hosts the clean-room runtime that reads the
//! game's data directly (see `docs/alpha-roadmap.md`).

pub mod assets;
pub mod model;

pub use assets::{AssetError, AssetSource, DecodedTexture, Install, ModelData};
pub use model::{GpuModel, MaterialCache};
