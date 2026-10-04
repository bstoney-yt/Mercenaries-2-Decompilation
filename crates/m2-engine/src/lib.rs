//! The engine runtime the native game is built on.
//!
//! - [`anim`]: skeletons, clip sampling, blending and locomotion.
//! - [`assets`]: finding the install and reading models and textures from
//!   its archives.
//! - [`model`]: models and their textures uploaded to a render backend.
//! - [`world`]: the island's layout and its terrain as meshes.
//! - [`stream`]: a loader thread and the streamed island around the camera.
//!
//! The original plan put raw `#[repr(C)]` mirrors of the executable's
//! classes here for hooking. The engine code is encrypted and can't be
//! read, so this crate instead hosts the clean-room runtime that reads the
//! game's data directly (see `docs/alpha-roadmap.md`).

pub mod anim;
pub mod assets;
pub mod audio;
pub mod model;
pub mod physics;
pub mod scatter;
pub mod stream;
pub mod vehicle;
pub mod world;

pub use anim::{Animator, BoundClip, Gait, Locomotion, Pose, Skeleton};
pub use assets::{AssetError, AssetSource, DecodedTexture, Install, ModelData};
pub use audio::{Mixer, Play, SoundBuffer, SoundLibrary, Streams, VoiceId, Voices};
pub use model::{CpuModel, GpuModel, MaterialCache};
pub use physics::{Character, CollisionShape, Physics, StaticKey, VehicleControls, VehicleId};
pub use stream::{World, WorldSettings, WorldStats};
pub use vehicle::{Catalogue, CatalogueEntry, PersonModel, VehicleSpec};
pub use world::{TemplateInfo, TemplatePlacement, Templates, VehicleKind, WorldIndex};
