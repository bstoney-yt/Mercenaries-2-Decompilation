//! Rendering abstraction.
//!
//! Translated rendering code talks to a backend trait instead of Direct3D 9
//! directly. The first backend forwards to the original D3D9 device; later
//! backends (Vulkan or Direct3D 12) implement the same trait.
//!
//! - [`buffer`]: vertex layouts, index formats and buffer handles
//!   (after the engine's `PgVertexBufferPc` / `PgIndexBufferPc`).
//! - [`key`]: 64-bit sort keys.
//! - [`queue`]: the per-frame draw queue (after `PgPrimitive`'s
//!   `AssignKeys` / `Sort` / `SubmitToGPU`).
//! - [`backend`]: the [`RenderBackend`] trait and draw description.
//! - [`texture`]: textures and materials.
//! - [`null`]: GPU-less backends for tests and headless runs.
//! - `gpu` (feature `wgpu`): the GPU backend, on Direct3D 12, Vulkan or
//!   Metal through wgpu.
//!
//! # Provenance
//!
//! Clean-room. The engine's class and method names above were seen only in
//! the executable's readable strings; the code behind them is encrypted and
//! was not read. Every layout, bit format and algorithm here is designed from
//! those names and general graphics practice, and no addresses are claimed.

pub mod backend;
pub mod buffer;
#[cfg(feature = "wgpu")]
pub mod gpu;
pub mod key;
pub mod null;
pub mod queue;
pub mod texture;

pub use backend::{
    BackendError, DrawCall, Geometry, Lighting, LiveBuffers, RenderBackend, Topology, View,
};
pub use buffer::{
    BufferUsage, IndexBufferDesc, IndexBufferHandle, IndexFormat, SkinHandle, TextureHandle,
    VertexBufferDesc, VertexBufferHandle, VertexElement, VertexFormat, VertexLayout,
    VertexSemantic,
};
pub use key::{Layer, MaterialId, SortKey};
pub use null::{Command, NullBackend, RecordingBackend};
pub use queue::{DrawQueue, Primitive, QueueError, SubmitStats};
pub use texture::{
    BlendMode, DetailMap, Emissive, MaterialDesc, NormalMap, Reflection, Specular, TerrainLayer,
    TextureDesc, TextureFormat, generate_mips,
};
