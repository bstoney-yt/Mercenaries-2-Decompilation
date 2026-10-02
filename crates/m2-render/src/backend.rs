//! The interface every graphics backend implements.
//!
//! Translated engine code never calls a graphics API directly; it creates
//! buffers and submits [`DrawCall`]s through [`RenderBackend`]. A backend
//! forwarding to the original Direct3D 9 device comes first, and Vulkan or
//! Direct3D 12 backends implement the same trait later.
//!
//! Buffers are owned by the backend and named by generational handles. They
//! live until [`destroy_vertex_buffer`](RenderBackend::destroy_vertex_buffer)
//! or [`destroy_index_buffer`](RenderBackend::destroy_index_buffer); using a
//! handle after that is an error rather than undefined behaviour, and
//! [`RenderBackend::live_buffers`] lets tests catch leaks.

use m2_math::glam::Mat4;

use crate::buffer::{IndexBufferDesc, IndexBufferHandle, VertexBufferDesc, VertexBufferHandle};
use crate::key::MaterialId;

/// How vertices are assembled into primitives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Topology {
    PointList,
    LineList,
    LineStrip,
    #[default]
    TriangleList,
    TriangleStrip,
    TriangleFan,
}

/// Which vertices a draw reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Geometry {
    /// Consecutive vertices straight from the vertex buffer.
    Vertices { first: u32, count: u32 },
    /// Indices into the vertex buffer. `base_vertex` is added to every index
    /// before it is used.
    Indexed {
        buffer: IndexBufferHandle,
        first: u32,
        count: u32,
        base_vertex: i32,
    },
}

/// One draw: geometry, the material it shades with, and its world transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawCall {
    pub material: MaterialId,
    pub topology: Topology,
    pub vertex_buffer: VertexBufferHandle,
    pub geometry: Geometry,
    /// Object to world, in glam's column-vector convention. Convert from the
    /// engine's D3DX row-vector matrices with `Mat4::from(RawMatrix44)`.
    pub world: Mat4,
}

/// Camera state for a frame.
///
/// View space is left-handed with `+z` forward, as `D3DXMatrixLookAtLH`
/// produces; sort depth is measured along that axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// World to view.
    pub view: Mat4,
    /// View to clip.
    pub projection: Mat4,
    pub near: f32,
    pub far: f32,
}

impl View {
    /// Depth of a world-space point, `0.0` at `near` and `1.0` at `far`.
    /// Not clamped; [`SortKey`](crate::SortKey) clamps when packing.
    pub fn normalized_depth(&self, world_pos: m2_math::glam::Vec3) -> f32 {
        let z = self.view.transform_point3(world_pos).z;
        (z - self.near) / (self.far - self.near)
    }
}

impl Default for View {
    fn default() -> Self {
        Self {
            view: Mat4::IDENTITY,
            projection: Mat4::IDENTITY,
            near: 0.1,
            far: 1000.0,
        }
    }
}

/// Buffers a backend currently owns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveBuffers {
    pub vertex: usize,
    pub index: usize,
}

/// Errors a backend reports.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BackendError {
    #[error("handle {0} is not a live buffer")]
    InvalidHandle(String),
    #[error("write of {len} bytes at offset {offset} overruns a {size}-byte buffer")]
    WriteOutOfBounds { offset: u64, len: u64, size: u64 },
    #[error("initial data is {actual} bytes, buffer is {expected}")]
    InitialDataSize { expected: u64, actual: u64 },
    #[error("draw reads elements {first}..{end} of a buffer holding {available}")]
    DrawOutOfBounds {
        first: u64,
        end: u64,
        available: u64,
    },
    #[error("{0} called outside begin_frame/end_frame")]
    NotInFrame(&'static str),
    #[error("begin_frame called while a frame is already open")]
    FrameAlreadyOpen,
    #[error("device error: {0}")]
    Device(String),
}

/// A graphics API behind the engine's renderer.
pub trait RenderBackend {
    /// Creates a vertex buffer. `initial`, if given, must be exactly
    /// `desc.size_in_bytes()` long; otherwise the contents start zeroed.
    fn create_vertex_buffer(
        &mut self,
        desc: &VertexBufferDesc,
        initial: Option<&[u8]>,
    ) -> Result<VertexBufferHandle, BackendError>;

    /// Creates an index buffer, with the same rules for `initial`.
    fn create_index_buffer(
        &mut self,
        desc: &IndexBufferDesc,
        initial: Option<&[u8]>,
    ) -> Result<IndexBufferHandle, BackendError>;

    /// Overwrites `data.len()` bytes starting at `offset`.
    fn write_vertex_buffer(
        &mut self,
        buffer: VertexBufferHandle,
        offset: u64,
        data: &[u8],
    ) -> Result<(), BackendError>;

    fn write_index_buffer(
        &mut self,
        buffer: IndexBufferHandle,
        offset: u64,
        data: &[u8],
    ) -> Result<(), BackendError>;

    fn destroy_vertex_buffer(&mut self, buffer: VertexBufferHandle) -> Result<(), BackendError>;

    fn destroy_index_buffer(&mut self, buffer: IndexBufferHandle) -> Result<(), BackendError>;

    fn live_buffers(&self) -> LiveBuffers;

    /// Opens a frame rendered from `view`. Draws are only valid inside a frame.
    fn begin_frame(&mut self, view: &View) -> Result<(), BackendError>;

    fn draw(&mut self, call: &DrawCall) -> Result<(), BackendError>;

    /// Closes the frame and presents it.
    fn end_frame(&mut self) -> Result<(), BackendError>;
}
