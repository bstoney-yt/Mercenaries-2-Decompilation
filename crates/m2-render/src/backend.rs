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

use m2_math::glam::{Mat4, Vec3};

use crate::buffer::{
    IndexBufferDesc, IndexBufferHandle, SkinHandle, VertexBufferDesc, VertexBufferHandle,
};
use crate::key::MaterialId;
use crate::texture::{MaterialDesc, TextureDesc, TextureHandle};

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
    /// Bone palette for skinned vertices: blend index `i` picks matrix `i`,
    /// which takes a vertex from bind pose to posed object space. `None`
    /// draws skinned vertices in their bind pose.
    pub skin: Option<SkinHandle>,
    /// Draws the geometry once per matrix of this range (stored with
    /// [`RenderBackend::set_skin`]): instance `i` goes to world space by
    /// `world * matrices[i]`. `None` draws it once.
    pub instances: Option<SkinHandle>,
}

/// Light and atmosphere for a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lighting {
    /// Direction the sunlight travels (from the sun towards the ground),
    /// world space. Need not be normalised.
    pub sun_direction: Vec3,
    /// Linear RGB.
    pub sun_color: Vec3,
    /// Linear RGB light reaching surfaces from every side, by the way they
    /// face: +x, -x, +y, -y, +z, -z (the game's ambient cube).
    pub ambient_cube: [Vec3; 6],
    /// Linear RGB of the background and of the fog.
    pub sky_color: Vec3,
    /// Exponential fog density per metre; 0 disables fog.
    pub fog_density: f32,
    /// Linear RGB of the light rim materials catch where they turn away
    /// from the eye.
    pub rim_color: Vec3,
    /// How far the surface must turn away before the rim shows: it is
    /// `rim_color * max(rim_threshold - N·V, 0) * (1 - N·L)`.
    pub rim_threshold: f32,
}

impl Default for Lighting {
    fn default() -> Self {
        Self {
            sun_direction: Vec3::new(-0.4, -1.0, 0.3),
            sun_color: Vec3::new(1.0, 0.95, 0.85),
            // Sky above, warm bounce from the ground below.
            ambient_cube: [
                Vec3::new(0.35, 0.38, 0.45),
                Vec3::new(0.35, 0.38, 0.45),
                Vec3::new(0.42, 0.48, 0.60),
                Vec3::new(0.24, 0.22, 0.18),
                Vec3::new(0.35, 0.38, 0.45),
                Vec3::new(0.35, 0.38, 0.45),
            ],
            sky_color: Vec3::new(0.45, 0.62, 0.85),
            fog_density: 0.0,
            rim_color: Vec3::ZERO,
            rim_threshold: 0.0,
        }
    }
}

/// Camera state for a frame.
///
/// View space is left-handed with `+z` forward, as `D3DXMatrixLookAtLH`
/// produces; sort depth is measured along that axis.
///
/// GPU backends use reversed depth (near maps to 1, far to 0) for precision
/// over the island's kilometres; build `projection` with
/// [`View::perspective`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// World to view.
    pub view: Mat4,
    /// View to clip.
    pub projection: Mat4,
    pub near: f32,
    pub far: f32,
    pub lighting: Lighting,
}

impl View {
    /// Depth of a world-space point, `0.0` at `near` and `1.0` at `far`.
    /// Not clamped; [`SortKey`](crate::SortKey) clamps when packing.
    pub fn normalized_depth(&self, world_pos: m2_math::glam::Vec3) -> f32 {
        // Right-handed: the camera looks down view-space -z.
        let z = -self.view.transform_point3(world_pos).z;
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
            lighting: Lighting::default(),
        }
    }
}

impl View {
    /// A camera at `eye` looking along `forward`, `fov_y` radians tall, with
    /// a reversed-depth right-handed projection: the game's world is
    /// right-handed (+y up), and drawn left-handed it comes out mirrored.
    pub fn perspective(
        eye: Vec3,
        forward: Vec3,
        fov_y: f32,
        aspect: f32,
        near: f32,
        far: f32,
    ) -> Self {
        Self {
            view: Mat4::look_to_rh(eye, forward, Vec3::Y),
            // Swapping the planes maps near to depth 1 and far to 0.
            projection: Mat4::perspective_rh(fov_y, aspect, far, near),
            near,
            far,
            lighting: Lighting::default(),
        }
    }

    /// The camera's world position.
    pub fn eye(&self) -> Vec3 {
        self.view.inverse().w_axis.truncate()
    }
}

/// Buffers a backend currently owns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveBuffers {
    pub vertex: usize,
    pub index: usize,
    pub texture: usize,
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
    #[error("skin palette from frame {skin_frame} used in frame {frame}")]
    StaleSkin { skin_frame: u64, frame: u64 },
    #[error("{0} called outside begin_frame/end_frame")]
    NotInFrame(&'static str),
    #[error("begin_frame called while a frame is already open")]
    FrameAlreadyOpen,
    #[error("bad texture: {0}")]
    Texture(String),
    #[error("{0} is not supported by this backend")]
    Unsupported(&'static str),
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

    /// Creates a texture from its top mip and as many lower mips as given
    /// (see [`TextureDesc::validate`]).
    fn create_texture(
        &mut self,
        desc: &TextureDesc,
        mips: &[&[u8]],
    ) -> Result<TextureHandle, BackendError>;

    fn destroy_texture(&mut self, texture: TextureHandle) -> Result<(), BackendError>;

    /// Defines or redefines what draws with `id` shade with. Draws with a
    /// material that was never defined use [`MaterialDesc::default`]. The
    /// material's texture must be live when it is drawn.
    fn set_material(&mut self, id: MaterialId, desc: &MaterialDesc) -> Result<(), BackendError>;

    fn live_buffers(&self) -> LiveBuffers;

    /// Opens a frame rendered from `view`. Draws are only valid inside a frame.
    fn begin_frame(&mut self, view: &View) -> Result<(), BackendError>;

    /// Stores matrices for this frame's draws: a skinned draw's bone
    /// palette, or an instanced draw's instance transforms.
    fn set_skin(&mut self, matrices: &[Mat4]) -> Result<SkinHandle, BackendError>;

    fn draw(&mut self, call: &DrawCall) -> Result<(), BackendError>;

    /// Closes the frame and presents it.
    fn end_frame(&mut self) -> Result<(), BackendError>;
}
