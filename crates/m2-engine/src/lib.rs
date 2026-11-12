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

pub mod ai;
pub mod ai_close;
pub mod ai_cover;
pub mod ai_dither;
pub mod ai_plan;
pub mod ai_sense;
pub mod ai_target;
pub mod aim_sweep;
pub mod ambience;
pub mod anim;
pub mod assets;
pub mod atmosphere;
pub mod audio;
pub mod blend;
pub mod boat_attack;
pub mod car_attack;
pub mod car_driver;
pub mod collision_layers;
pub mod durability;
pub mod grapple;
pub mod gun_sounds;
pub mod gunner_aim;
pub mod hand_ik;
pub mod heli_land;
pub mod heli_pilot;
pub mod hijack;
pub mod ladder;
pub mod lights;
pub mod melee;
pub mod model;
pub mod motion;
pub mod music_source;
pub mod on_foot;
pub mod ordnance;
pub mod osprey;
pub mod osprey_mesh;
pub mod particles;
pub mod path_search;
pub mod physics;
pub mod pickup;
pub mod population;
pub mod reverb;
pub mod road_graph;
pub mod road_ring;
pub mod scatter;
pub mod seats;
pub mod sound_regions;
pub mod spawn_sound;
pub mod standin;
pub mod stream;
pub mod surface;
pub mod tank_attack;
pub mod technique;
pub mod tuning;
pub mod vehicle;
pub mod vehicle_damage;
pub mod vehicle_door;
pub mod vehicle_parts;
pub mod weapons;
pub mod world;

pub use ai::{AiFlags, AiProfile};
pub use anim::{Animator, BoundClip, Gait, Locomotion, Pose, Skeleton};
pub use assets::{AssetError, AssetSource, DecodedTexture, Install, ModelData};
pub use atmosphere::Atmosphere;
pub use audio::{Category, Mixer, Play, SoundBuffer, SoundLibrary, Streams, VoiceId, Voices};
pub use model::{CpuModel, GpuModel, Looks, MaterialCache, ModelParts, StateFx, StateMessage};
pub use physics::{
    Breakup, Character, CollisionShape, DebrisId, Loose, LoosePiece, Physics, RagdollId, StaticKey,
    VehicleControls, VehicleId,
};
pub use stream::{Frustum, World, WorldSettings, WorldStats};
pub use technique::Technique;
pub use tuning::VehicleTuning;
pub use vehicle::{Catalogue, CatalogueEntry, PersonModel, Rest, VehicleSpec};
pub use world::{TemplateInfo, TemplatePlacement, Templates, VehicleKind, WorldIndex};
