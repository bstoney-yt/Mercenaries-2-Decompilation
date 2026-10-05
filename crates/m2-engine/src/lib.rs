//! The engine runtime the native game is built on.
//!
//! - [`anim`]: skeletons, clip sampling, blending and locomotion.
//! - [`assets`]: finding the install and reading models and textures from
//!   its archives.
//! - [`model`]: models and their textures uploaded to a render backend.
//! - [`world`]: the island's layout and its terrain as meshes.
//! - [`stream`]: a loader thread and the streamed island around the camera.
//! - [`technique`]: what each material shader does, by its id.
//!
//! The original plan put raw `#[repr(C)]` mirrors of the executable's
//! classes here for hooking. The engine code is encrypted and can't be
//! read, so this crate instead hosts the clean-room runtime that reads the
//! game's data directly (see `docs/alpha-roadmap.md`).

pub mod anim;
pub mod assets;
pub mod atmosphere;
pub mod audio;
pub mod model;
pub mod physics;
pub mod scatter;
pub mod stream;
pub mod technique;
pub mod tuning;
pub mod vehicle;
pub mod world;

pub use anim::{Animator, BoundClip, Gait, Locomotion, Pose, Skeleton};
pub use assets::{AssetError, AssetSource, DecodedTexture, Install, ModelData};
pub use atmosphere::Atmosphere;
pub use audio::{Mixer, Play, SoundBuffer, SoundLibrary, Streams, VoiceId, Voices};
pub use model::{CpuModel, GpuModel, MaterialCache};
pub use physics::{
    Character, CollisionShape, DebrisId, Loose, LoosePiece, Physics, RagdollId, StaticKey,
    VehicleControls, VehicleId,
};
pub use stream::{Frustum, World, WorldSettings, WorldStats};
pub use technique::Technique;
pub use tuning::VehicleTuning;
pub use vehicle::{Catalogue, CatalogueEntry, PersonModel, Rest, VehicleSpec};
pub use world::{TemplateInfo, TemplatePlacement, Templates, VehicleKind, WorldIndex};
