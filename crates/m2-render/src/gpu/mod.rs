//! The GPU backend: [`RenderBackend`] on wgpu, so the same renderer runs on
//! Direct3D 12 or Vulkan on Windows (and Vulkan or Metal elsewhere).
//!
//! Draws are recorded during a frame and encoded into one render pass at
//! [`end_frame`](RenderBackend::end_frame), in the order they were
//! submitted; the [`DrawQueue`](crate::DrawQueue) has already sorted them.
//! Pipelines are built on first use per vertex layout, topology and blend
//! state, with a vertex stage generated for the layout (see `shader`).
//!
//! Conventions, matching Direct3D 9 so engine data needs no conversion:
//! clockwise front faces, `+y` up, texture `v` down. Depth is reversed
//! (near 1, far 0) with a 32-bit float buffer; see [`View`].
//!
//! # Provenance
//!
//! Clean-room. Written against wgpu's public API and general rendering
//! practice; nothing here comes from the original executable.

mod post;
mod shader;
mod shadow;

use std::collections::HashMap;

use ::wgpu::util::DeviceExt;
use m2_math::glam::{Mat4, Vec3};

use crate::backend::{
    BackendError, DrawCall, Geometry, LiveBuffers, RenderBackend, Topology, View,
};
use crate::buffer::{
    IndexBufferDesc, IndexBufferHandle, IndexFormat, RawHandle, SkinHandle, VertexBufferDesc,
    VertexBufferHandle, VertexFormat, VertexLayout,
};
use crate::key::MaterialId;
use crate::texture::{BlendMode, MaterialDesc, TextureDesc, TextureFormat, TextureHandle};

pub use ::wgpu;

/// Bytes of a draw's uniforms: its world matrix, then the first palette
/// matrix and the palette length, then the same for its instances, then
/// its texture coordinate offset.
const DRAW_UNIFORM_SIZE: u64 = 96;
/// Bone matrices the palette buffer starts with.
const INITIAL_PALETTE: u64 = 1024;
/// Metres from the camera by which a detail map has faded out.
const DETAIL_FADE: f32 = 80.0;
/// Metres from the camera where ground's colour map starts to take over
/// from its tiled layers, and over how many metres. The game passes these
/// to its terrain shaders from code; these values are a guess.
const GROUND_MAP_START: f32 = 80.0;
const GROUND_MAP_LENGTH: f32 = 170.0;
/// Where ground's detail map starts to fade out, and over how many metres
/// (also a guess).
const GROUND_DETAIL_START: f32 = 15.0;
const GROUND_DETAIL_LENGTH: f32 = 35.0;
/// Where ground's colour map takes over and its detail fades, and the
/// detail's strength: `[map start, map length, detail start, detail
/// length, detail gain]`. `M2_GROUND_FIT=a/b/c/d/e` replaces them to test
/// against footage (any prefix).
fn ground_fit() -> [f32; 5] {
    static FIT: std::sync::OnceLock<[f32; 5]> = std::sync::OnceLock::new();
    *FIT.get_or_init(|| {
        let mut fit = [
            GROUND_MAP_START,
            GROUND_MAP_LENGTH,
            GROUND_DETAIL_START,
            GROUND_DETAIL_LENGTH,
            1.0,
        ];
        if let Ok(v) = std::env::var("M2_GROUND_FIT") {
            for (slot, x) in fit.iter_mut().zip(v.split('/')) {
                if let Ok(x) = x.trim().parse::<f32>() {
                    *slot = x;
                }
            }
        }
        fit
    })
}
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Per-draw uniforms are packed at this stride (wgpu's minimum dynamic
/// offset alignment on every backend).
const DRAW_STRIDE: u64 = 256;

/// A generational slot map, as in the null backend.
#[derive(Debug)]
struct Slots<T> {
    slots: Vec<(u32, Option<T>)>,
    free: Vec<u32>,
    live: usize,
}

impl<T> Default for Slots<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            live: 0,
        }
    }
}

impl<T> Slots<T> {
    fn insert(&mut self, value: T) -> RawHandle {
        self.live += 1;
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.1 = Some(value);
            RawHandle {
                index,
                generation: slot.0,
            }
        } else {
            let index = u32::try_from(self.slots.len()).expect("more than u32::MAX resources");
            self.slots.push((0, Some(value)));
            RawHandle {
                index,
                generation: 0,
            }
        }
    }

    fn get(&self, h: RawHandle) -> Result<&T, BackendError> {
        self.slots
            .get(h.index as usize)
            .filter(|s| s.0 == h.generation)
            .and_then(|s| s.1.as_ref())
            .ok_or_else(|| BackendError::InvalidHandle(format!("{h:?}")))
    }

    fn get_mut(&mut self, h: RawHandle) -> Result<&mut T, BackendError> {
        self.slots
            .get_mut(h.index as usize)
            .filter(|s| s.0 == h.generation)
            .and_then(|s| s.1.as_mut())
            .ok_or_else(|| BackendError::InvalidHandle(format!("{h:?}")))
    }

    fn remove(&mut self, h: RawHandle) -> Result<T, BackendError> {
        let slot = self
            .slots
            .get_mut(h.index as usize)
            .filter(|s| s.0 == h.generation && s.1.is_some())
            .ok_or_else(|| BackendError::InvalidHandle(format!("{h:?}")))?;
        let value = slot.1.take().expect("checked above");
        slot.0 = slot.0.wrapping_add(1);
        self.free.push(h.index);
        self.live -= 1;
        Ok(value)
    }
}

#[derive(Debug)]
struct VertexBuffer {
    desc: VertexBufferDesc,
    buffer: wgpu::Buffer,
    /// A sphere around its positions (object space), for shadow culling;
    /// `None` when unknown (casts everywhere).
    bounds: Option<(Vec3, f32)>,
}

#[derive(Debug)]
struct IndexBuffer {
    desc: IndexBufferDesc,
    buffer: wgpu::Buffer,
    /// CPU copy, so writes not aligned to 4 bytes (odd runs of 16-bit
    /// indices) can be widened to an aligned upload.
    shadow: Vec<u8>,
}

#[derive(Debug)]
struct Texture {
    view: wgpu::TextureView,
}

#[derive(Debug)]
struct Material {
    desc: MaterialDesc,
    bind_group: wgpu::BindGroup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum BlendKind {
    Opaque,
    Masked,
    Translucent,
    Additive,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PipelineKey {
    layout: VertexLayout,
    topology: Topology,
    strip_index: Option<IndexFormat>,
    blend: BlendKind,
    double_sided: bool,
    depth_bias: bool,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FrameUniforms {
    view_proj: [f32; 16],
    eye: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    sky: [f32; 4],
    ambient_cube: [[f32; 4]; 6],
    rim: [f32; 4],
    /// xyz: the way the camera looks, w: seconds since the backend
    /// started (waves).
    view_dir: [f32; 4],
    /// World to each shadow cascade's clip space.
    shadow: [[f32; 16]; shadow::CASCADES],
    /// View depth where each cascade ends (all 0: no shadows).
    shadow_splits: [f32; 4],
    /// Metres per shadow texel in each cascade.
    shadow_texels: [f32; 4],
    /// Clip space back to the world.
    inv_view_proj: [f32; 16],
    /// Haze: Rayleigh scattering, the Mie phase function's g; Mie
    /// scattering, in-scattering multiplier; extinction multiplier and the
    /// two scale heights.
    rayleigh: [f32; 4],
    mie: [f32; 4],
    scatter: [f32; 4],
    /// Clouds: the three layers' scales and the coverage; the wind (x, z)
    /// and whether there are clouds.
    clouds: [f32; 4],
    wind: [f32; 4],
    /// The water's depth colour map: four stops, and whether it is set
    /// and the metres at its end.
    water_ramp: [[f32; 4]; 4],
    water_depth: [f32; 4],
    /// Scale on the clouds' colour.
    cloud_tone: [f32; 4],
    /// The game's haze layers, `(force, 1 / limit)` each (all 0: off).
    haze_layers: [f32; 4],
    /// x: the cloud deck's reach for the air in front of it (0: none).
    deck: [f32; 4],
    /// x: surfaces take the sun's brightness without its hue.
    surface: [f32; 4],
}

/// What a shadow pipeline depends on.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ShadowKey {
    layout: VertexLayout,
    topology: Topology,
    strip_index: Option<IndexFormat>,
    /// Alpha-tested: runs a fragment stage to cut out.
    masked: bool,
}

/// How a draw casts sun shadows.
#[derive(Debug)]
struct Caster {
    pipeline: ShadowKey,
    /// World-space sphere around it; `None` casts into every cascade.
    sphere: Option<(Vec3, f32)>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MaterialUniforms {
    color: [f32; 4],
    params: [f32; 4],
    /// x: detail tiles per metre (0: none), y: distance it fades out by,
    /// z: soft edge, w: ground layers.
    detail: [f32; 4],
    /// rgb: highlight colour (0: none), a: exponent.
    specular: [f32; 4],
    /// x: specular map, y: normal map, z: normal x in alpha, w: ground.
    maps: [f32; 4],
    direct: [f32; 4],
    ambient: [f32; 4],
    /// Colour map takeover start and 1 / length, detail fade start and
    /// 1 / length.
    ground: [f32; 4],
    /// Per ground layer, the u row then the v row (world space).
    layer_uv: [[f32; 4]; 8],
    /// rgb: emitted colour (0: none), w: from the specular map's slot.
    emissive: [f32; 4],
    /// x: reflection strength (0: none), y: Fresnel reflectance facing
    /// the eye (negative: none), z: takes the rim light.
    reflection: [f32; 4],
    /// x: open water.
    water: [f32; 4],
}

/// Texture views a material binds besides its uniforms.
struct MaterialViews<'a> {
    base: &'a wgpu::TextureView,
    detail: &'a wgpu::TextureView,
    specular: &'a wgpu::TextureView,
    normal: &'a wgpu::TextureView,
    layers: [&'a wgpu::TextureView; 4],
}

/// A 2D float texture binding visible to the fragment stage.
fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// A frame between `begin_frame` and `end_frame`.
#[derive(Debug)]
struct OpenFrame {
    view: View,
    draws: Vec<RecordedDraw>,
    /// Per-draw uniforms at `DRAW_STRIDE`.
    uniforms: Vec<u8>,
    /// Bone matrices set with `set_skin`, column-major.
    palette: Vec<[f32; 16]>,
}

#[derive(Debug)]
struct RecordedDraw {
    pipeline: PipelineKey,
    vertex_buffer: VertexBufferHandle,
    geometry: Geometry,
    material: MaterialId,
    /// Instances drawn (1 when not instanced).
    instances: u32,
    caster: Option<Caster>,
}

/// The depth buffer and its copy for water.
#[derive(Debug)]
struct DepthTargets {
    depth: wgpu::Texture,
    view: wgpu::TextureView,
    scene: wgpu::Texture,
    group: wgpu::BindGroup,
}

/// Where frames go.
#[derive(Debug)]
enum Target {
    Window {
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
    },
    Offscreen {
        texture: wgpu::Texture,
        format: wgpu::TextureFormat,
    },
}

/// Which GPU and API the backend ended up on.
#[derive(Clone, Debug)]
pub struct AdapterSummary {
    pub name: String,
    pub backend: String,
    pub device_type: String,
}

/// [`RenderBackend`] on wgpu.
#[derive(Debug)]
pub struct WgpuBackend {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    target: Target,
    width: u32,
    height: u32,
    depth: DepthTargets,
    scene_layout: wgpu::BindGroupLayout,

    draw_layout: wgpu::BindGroupLayout,
    material_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    frame_buffer: wgpu::Buffer,
    frame_group: wgpu::BindGroup,
    shadow_pipeline_layout: wgpu::PipelineLayout,
    /// Per cascade: its layer of the shadow map, and its frame uniforms
    /// (the cascade's matrix as `view_proj`).
    shadow_layers: Vec<(wgpu::TextureView, wgpu::Buffer, wgpu::BindGroup)>,
    draw_buffer: wgpu::Buffer,
    palette_buffer: wgpu::Buffer,
    draw_group: wgpu::BindGroup,
    /// Repeating, then clamping.
    samplers: [wgpu::Sampler; 2],
    white: wgpu::TextureView,
    default_material: wgpu::BindGroup,

    shaders: HashMap<VertexLayout, wgpu::ShaderModule>,
    pipelines: HashMap<PipelineKey, wgpu::RenderPipeline>,
    shadow_pipelines: HashMap<ShadowKey, wgpu::RenderPipeline>,
    /// When the backend started, for animated surfaces.
    started: std::time::Instant,
    /// Bloom, exposure and grade after the world.
    post: post::Post,
    sky: wgpu::RenderPipeline,
    cloud_layout: wgpu::BindGroupLayout,
    /// The cascades the shadow map holds, as last drawn.
    cascades: std::cell::Cell<Option<shadow::Cascades>>,
    vertex_buffers: Slots<VertexBuffer>,
    index_buffers: Slots<IndexBuffer>,
    textures: Slots<Texture>,
    materials: HashMap<MaterialId, Material>,

    frame: Option<OpenFrame>,
    /// Frames ended so far, which is also the number of the open frame.
    frames: u64,
}

fn device_error(e: impl std::fmt::Display) -> BackendError {
    BackendError::Device(e.to_string())
}

fn wgpu_vertex_format(f: VertexFormat) -> wgpu::VertexFormat {
    match f {
        VertexFormat::Float1 => wgpu::VertexFormat::Float32,
        VertexFormat::Float2 => wgpu::VertexFormat::Float32x2,
        VertexFormat::Float3 => wgpu::VertexFormat::Float32x3,
        VertexFormat::Float4 => wgpu::VertexFormat::Float32x4,
        VertexFormat::UNorm4x8 | VertexFormat::Bgra8 => wgpu::VertexFormat::Unorm8x4,
        VertexFormat::UInt4x8 => wgpu::VertexFormat::Uint8x4,
        VertexFormat::Half2 => wgpu::VertexFormat::Float16x2,
        VertexFormat::Half4 => wgpu::VertexFormat::Float16x4,
    }
}

/// `layout`'s elements as shader locations 0, 1, ...
fn vertex_attributes(layout: &VertexLayout) -> Vec<wgpu::VertexAttribute> {
    layout
        .elements()
        .iter()
        .enumerate()
        .map(|(i, e)| wgpu::VertexAttribute {
            format: wgpu_vertex_format(e.format),
            offset: u64::from(e.offset),
            shader_location: i as u32,
        })
        .collect()
}

fn wgpu_index_format(f: IndexFormat) -> wgpu::IndexFormat {
    match f {
        IndexFormat::U16 => wgpu::IndexFormat::Uint16,
        IndexFormat::U32 => wgpu::IndexFormat::Uint32,
    }
}

fn wgpu_topology(t: Topology) -> Result<wgpu::PrimitiveTopology, BackendError> {
    Ok(match t {
        Topology::PointList => wgpu::PrimitiveTopology::PointList,
        Topology::LineList => wgpu::PrimitiveTopology::LineList,
        Topology::LineStrip => wgpu::PrimitiveTopology::LineStrip,
        Topology::TriangleList => wgpu::PrimitiveTopology::TriangleList,
        Topology::TriangleStrip => wgpu::PrimitiveTopology::TriangleStrip,
        Topology::TriangleFan => return Err(BackendError::Unsupported("triangle fans")),
    })
}

/// `data` padded with zeros to a multiple of 4 bytes, wgpu's copy alignment.
fn padded(data: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if data.len().is_multiple_of(4) {
        data.into()
    } else {
        let mut v = data.to_vec();
        v.resize(data.len().next_multiple_of(4), 0);
        v.into()
    }
}

fn check_range(first: u32, count: u32, available: u32) -> Result<(), BackendError> {
    let end = u64::from(first) + u64::from(count);
    if end > u64::from(available) {
        return Err(BackendError::DrawOutOfBounds {
            first: first.into(),
            end,
            available: available.into(),
        });
    }
    Ok(())
}

fn uniform_entry(binding: u32, dynamic: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: dynamic,
            min_binding_size: None,
        },
        count: None,
    }
}

impl WgpuBackend {
    /// A backend presenting to `window` (a winit window in an `Arc`, or
    /// anything else wgpu accepts as a surface target).
    pub fn for_window<W>(
        window: W,
        width: u32,
        height: u32,
        vsync: bool,
    ) -> Result<Self, BackendError>
    where
        W: wgpu::WindowHandle
            + wgpu::rwh::HasDisplayHandle
            + std::fmt::Debug
            + Send
            + Sync
            + Clone
            + 'static,
    {
        let instance = wgpu::Instance::new(
            wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(window.clone())),
        );
        let surface = instance.create_surface(window).map_err(device_error)?;
        let (adapter, device, queue) = pollster::block_on(open(&instance, Some(&surface)))?;
        let mut config = surface
            .get_default_config(&adapter, width.max(1), height.max(1))
            .ok_or_else(|| BackendError::Device("surface not supported by the adapter".into()))?;
        let caps = surface.get_capabilities(&adapter);
        if let Some(srgb) = caps.formats.iter().copied().find(|f| f.is_srgb()) {
            config.format = srgb;
        }
        config.present_mode = if vsync {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        surface.configure(&device, &config);
        Ok(Self::new(
            adapter,
            device,
            queue,
            Target::Window { surface, config },
            width.max(1),
            height.max(1),
        ))
    }

    /// A backend rendering into an offscreen image, read back with
    /// [`read_pixels`](Self::read_pixels). Fails when no GPU (or software
    /// rasteriser) is available.
    pub fn headless(width: u32, height: u32) -> Result<Self, BackendError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let (adapter, device, queue) = pollster::block_on(open(&instance, None))?;
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Ok(Self::new(
            adapter,
            device,
            queue,
            Target::Offscreen { texture, format },
            width,
            height,
        ))
    }

    fn new(
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        target: Target,
        width: u32,
        height: u32,
    ) -> Self {
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame"),
            entries: &[
                uniform_entry(0, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        // The shadow passes draw into the map, so their frame group can't
        // also bind it.
        let shadow_frame_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("shadow frame"),
                entries: &[uniform_entry(0, false)],
            });
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("draw"),
            entries: &[
                uniform_entry(0, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material"),
            entries: &[
                uniform_entry(0, false),
                texture_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture_entry(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture_entry(5),
                texture_entry(6),
                texture_entry(7),
                texture_entry(8),
                texture_entry(9),
                texture_entry(10),
            ],
        });
        // Depth beneath, for water: a copy of the depth buffer made before
        // the first water draw.
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("standard"),
            bind_group_layouts: &[
                Some(&frame_layout),
                Some(&draw_layout),
                Some(&material_layout),
                Some(&scene_layout),
            ],
            immediate_size: 0,
        });
        let shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("shadow"),
                bind_group_layouts: &[
                    Some(&shadow_frame_layout),
                    Some(&draw_layout),
                    Some(&material_layout),
                ],
                immediate_size: 0,
            });
        // The cloud noise and its sampler, at bindings that don't clash with
        // the draw group's in the shared source.
        let cloud_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("clouds"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sky_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky"),
            bind_group_layouts: &[Some(&frame_layout), Some(&cloud_layout)],
            immediate_size: 0,
        });
        let sky_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky"),
            source: wgpu::ShaderSource::Wgsl(shader::sky_source().into()),
        });
        // Drawn first, behind everything: no depth test or write.
        let sky = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&sky_layout),
            vertex: wgpu::VertexState {
                module: &sky_module,
                entry_point: Some("vs_sky"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &sky_module,
                entry_point: Some("fs_sky"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: post::HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let frame_uniforms = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size_of::<FrameUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let shadow_map = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow map"),
            size: wgpu::Extent3d {
                width: shadow::SIZE,
                height: shadow::SIZE,
                depth_or_array_layers: shadow::CASCADES as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: shadow::FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_layers = (0..shadow::CASCADES as u32)
            .map(|i| {
                let buffer = frame_uniforms("shadow frame uniforms");
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("shadow frame"),
                    layout: &shadow_frame_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                });
                let view = shadow_map.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: i,
                    array_layer_count: Some(1),
                    ..Default::default()
                });
                (view, buffer, group)
            })
            .collect();
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let frame_buffer = frame_uniforms("frame uniforms");
        let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame"),
            layout: &frame_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&shadow_map.create_view(
                        &wgpu::TextureViewDescriptor {
                            dimension: Some(wgpu::TextureViewDimension::D2Array),
                            ..Default::default()
                        },
                    )),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
            ],
        });
        let draw_buffer = Self::draw_uniforms(&device, 1024);
        let palette_buffer = Self::palette(&device, INITIAL_PALETTE);
        let draw_group = Self::draw_group(&device, &draw_layout, &draw_buffer, &palette_buffer);
        // `M2_WRAP=mirror` flips every second repeat (to see whether model
        // texture seams, not the data, make the cliff bands).
        let repeat = if std::env::var("M2_WRAP").is_ok_and(|v| v == "mirror") {
            wgpu::AddressMode::MirrorRepeat
        } else {
            wgpu::AddressMode::Repeat
        };
        let samplers = [repeat, wgpu::AddressMode::ClampToEdge].map(|mode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("base"),
                address_mode_u: mode,
                address_mode_v: mode,
                address_mode_w: mode,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                anisotropy_clamp: 8,
                ..Default::default()
            })
        });
        let white = device
            .create_texture_with_data(
                &queue,
                &wgpu::TextureDescriptor {
                    label: Some("white"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &[255; 4],
            )
            .create_view(&Default::default());
        let depth = Self::depth_targets(&device, &scene_layout, width, height);
        let output = match &target {
            Target::Window { config, .. } => config.format,
            Target::Offscreen { format, .. } => *format,
        };
        let post = post::Post::new(&device, output, width, height);
        Self {
            default_material: Self::material_group(
                &device,
                &material_layout,
                &samplers,
                &MaterialViews {
                    base: &white,
                    detail: &white,
                    specular: &white,
                    normal: &white,
                    layers: [&white; 4],
                },
                &MaterialDesc::default(),
            ),
            adapter,
            device,
            queue,
            target,
            width,
            height,
            depth,
            scene_layout,
            draw_layout,
            material_layout,
            pipeline_layout,
            frame_buffer,
            frame_group,
            shadow_pipeline_layout,
            shadow_layers,
            draw_buffer,
            palette_buffer,
            draw_group,
            samplers,
            white,
            shaders: HashMap::new(),
            pipelines: HashMap::new(),
            shadow_pipelines: HashMap::new(),
            cascades: std::cell::Cell::new(None),
            started: std::time::Instant::now(),
            post,
            sky,
            cloud_layout,
            vertex_buffers: Slots::default(),
            index_buffers: Slots::default(),
            textures: Slots::default(),
            materials: HashMap::new(),
            frame: None,
            frames: 0,
        }
    }

    fn draw_uniforms(device: &wgpu::Device, draws: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("draw uniforms"),
            size: draws * DRAW_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn palette(device: &wgpu::Device, matrices: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bone palette"),
            size: matrices * 64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn draw_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        uniforms: &wgpu::Buffer,
        palette: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("draw"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: uniforms,
                        offset: 0,
                        size: wgpu::BufferSize::new(DRAW_UNIFORM_SIZE),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: palette.as_entire_binding(),
                },
            ],
        })
    }

    /// The depth buffer, and the copy of it water reads (what lies
    /// beneath), with the copy's bind group.
    fn depth_targets(
        device: &wgpu::Device,
        scene_layout: &wgpu::BindGroupLayout,
        width: u32,
        height: u32,
    ) -> DepthTargets {
        let texture = |label, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage,
                view_formats: &[],
            })
        };
        let depth = texture(
            "depth",
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let scene = texture(
            "scene depth",
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene"),
            layout: scene_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &scene.create_view(&Default::default()),
                ),
            }],
        });
        DepthTargets {
            view: depth.create_view(&Default::default()),
            depth,
            scene,
            group,
        }
    }

    /// `samplers` are the repeating and clamping ones.
    fn material_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        samplers: &[wgpu::Sampler; 2],
        views: &MaterialViews<'_>,
        desc: &MaterialDesc,
    ) -> wgpu::BindGroup {
        let cutoff = match desc.blend {
            BlendMode::Masked { cutoff } => cutoff,
            _ => 0.0,
        };
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        let uniforms = MaterialUniforms {
            color: desc.color,
            params: [
                cutoff,
                flag(desc.lit),
                flag(desc.vertex_color),
                flag(desc.fog),
            ],
            detail: [
                match desc.detail {
                    Some(d) if d.tile > 0.0 => 1.0 / d.tile,
                    _ => 0.0,
                },
                DETAIL_FADE,
                flag(desc.soft_edge),
                desc.layers.iter().take_while(|l| l.is_some()).count() as f32,
            ],
            specular: match desc.specular {
                Some(s) => [s.color[0], s.color[1], s.color[2], s.power],
                None => [0.0; 4],
            },
            maps: [
                flag(desc.specular.is_some_and(|s| s.map.is_some())),
                flag(desc.normal_map.is_some()),
                flag(desc.normal_map.is_some_and(|n| n.alpha_x)),
                flag(desc.ground),
            ],
            direct: [
                desc.direct[0],
                desc.direct[1],
                desc.direct[2],
                flag(desc.beam),
            ],
            ambient: [
                desc.ambient[0],
                desc.ambient[1],
                desc.ambient[2],
                ground_fit()[4],
            ],
            ground: {
                let fit = ground_fit();
                [fit[0], 1.0 / fit[1], fit[2], 1.0 / fit[3]]
            },
            layer_uv: {
                let mut rows = [[0.0; 4]; 8];
                for (i, l) in desc.layers.iter().enumerate() {
                    if let Some(l) = l {
                        rows[i * 2] = l.uv[0];
                        rows[i * 2 + 1] = l.uv[1];
                    }
                }
                rows
            },
            emissive: match desc.emissive {
                Some(e) => [e.color[0], e.color[1], e.color[2], flag(e.map.is_some())],
                None => [0.0; 4],
            },
            reflection: match desc.reflection {
                Some(r) => [r.strength, r.fresnel.unwrap_or(-1.0), flag(desc.rim), 0.0],
                None => [0.0, -1.0, flag(desc.rim), 0.0],
            },
            water: [
                flag(desc.water),
                desc.water_sky,
                desc.water_waves,
                desc.water_fade,
            ],
        };
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("material uniforms"),
            contents: bytemuck::bytes_of(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(views.base),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&samplers[usize::from(desc.clamp)]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(views.detail),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&samplers[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(views.specular),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(views.normal),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(views.layers[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(views.layers[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(views.layers[2]),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(views.layers[3]),
                },
            ],
        })
    }

    /// The GPU and graphics API in use.
    pub fn adapter_summary(&self) -> AdapterSummary {
        let info = self.adapter.get_info();
        AdapterSummary {
            name: info.name,
            backend: format!("{:?}", info.backend),
            device_type: format!("{:?}", info.device_type),
        }
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Size of the frames being rendered.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Completed frames.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Follows a window resize. Zero sizes (a minimised window) are ignored.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 || (width, height) == (self.width, self.height) {
            return;
        }
        self.width = width;
        self.height = height;
        self.depth = Self::depth_targets(&self.device, &self.scene_layout, width, height);
        self.post.resize(&self.device, width, height);
        if let Target::Window { surface, config } = &mut self.target {
            config.width = width;
            config.height = height;
            surface.configure(&self.device, config);
        }
    }

    /// Switches vertical sync (window targets only).
    pub fn set_vsync(&mut self, vsync: bool) {
        if let Target::Window { surface, config } = &mut self.target {
            config.present_mode = if vsync {
                wgpu::PresentMode::AutoVsync
            } else {
                wgpu::PresentMode::AutoNoVsync
            };
            surface.configure(&self.device, config);
        }
    }

    /// The last frame of a headless backend as tightly packed RGBA8 rows.
    pub fn read_pixels(&self) -> Result<Vec<u8>, BackendError> {
        let Target::Offscreen { texture, .. } = &self.target else {
            return Err(BackendError::Unsupported("reading back a window"));
        };
        let row = self.width * 4;
        let padded_row = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded_row * self.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: None,
                },
            },
            texture.size(),
        );
        self.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(device_error)?;
        rx.recv().map_err(device_error)?.map_err(device_error)?;
        let mapped = buffer.get_mapped_range(..).map_err(device_error)?;
        let mut out = Vec::with_capacity((row * self.height) as usize);
        for y in 0..self.height {
            let start = (y * padded_row) as usize;
            out.extend_from_slice(&mapped[start..start + row as usize]);
        }
        Ok(out)
    }

    fn color_format(&self) -> wgpu::TextureFormat {
        match &self.target {
            Target::Window { config, .. } => config.format,
            Target::Offscreen { format, .. } => *format,
        }
    }

    fn pipeline_key(&self, call: &DrawCall) -> Result<PipelineKey, BackendError> {
        let vb = self.vertex_buffers.get(call.vertex_buffer.0)?;
        let desc = self
            .materials
            .get(&call.material)
            .map(|m| m.desc)
            .unwrap_or_default();
        if let Some(t) = desc.texture {
            self.textures.get(t.0)?;
        }
        let strip = matches!(call.topology, Topology::TriangleStrip | Topology::LineStrip);
        let strip_index = match call.geometry {
            Geometry::Indexed { buffer, .. } if strip => {
                Some(self.index_buffers.get(buffer.0)?.desc.format)
            }
            _ => None,
        };
        wgpu_topology(call.topology)?;
        Ok(PipelineKey {
            layout: vb.desc.layout.clone(),
            topology: call.topology,
            strip_index,
            blend: match desc.blend {
                BlendMode::Opaque => BlendKind::Opaque,
                BlendMode::Masked { .. } => BlendKind::Masked,
                BlendMode::Translucent => BlendKind::Translucent,
                BlendMode::Additive => BlendKind::Additive,
            },
            double_sided: desc.double_sided,
            depth_bias: desc.depth_bias,
        })
    }

    fn ensure_pipeline(&mut self, key: &PipelineKey) -> Result<(), BackendError> {
        if self.pipelines.contains_key(key) {
            return Ok(());
        }
        // The world draws in floating point; the post pass brings it to
        // the frame.
        let color_format = post::HDR_FORMAT;
        let attributes = vertex_attributes(&key.layout);
        let module = self.shader_module(&key.layout).clone();
        let module = &module;
        let device = &self.device;
        let translucent = matches!(key.blend, BlendKind::Translucent | BlendKind::Additive);
        let blend = match key.blend {
            BlendKind::Translucent => Some(wgpu::BlendState::ALPHA_BLENDING),
            BlendKind::Additive => Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
            _ => None,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("standard"),
            layout: Some(&self.pipeline_layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: u64::from(key.layout.stride()),
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu_topology(key.topology)?,
                strip_index_format: key.strip_index.map(wgpu_index_format),
                // The game's triangles wind clockwise seen from their front in
                // its right-handed world: anticlockwise on screen.
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: (!key.double_sided).then_some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(!translucent),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                // Depth is reversed, so a positive bias is towards the camera.
                bias: if key.depth_bias {
                    wgpu::DepthBiasState {
                        constant: 8,
                        slope_scale: 1.0,
                        clamp: 0.0,
                    }
                } else {
                    Default::default()
                },
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.insert(key.clone(), pipeline);
        Ok(())
    }

    /// How `call`, drawn with pipeline `key`, casts sun shadows: opaque
    /// and cut-out lit triangles do.
    fn caster(
        &mut self,
        call: &DrawCall,
        key: &PipelineKey,
        instance_first: u32,
        instances: u32,
    ) -> Result<Option<Caster>, BackendError> {
        let lit = self
            .materials
            .get(&call.material)
            .is_some_and(|m| m.desc.lit);
        let triangles = matches!(
            key.topology,
            Topology::TriangleList | Topology::TriangleStrip | Topology::TriangleFan
        );
        if !lit || !triangles || !matches!(key.blend, BlendKind::Opaque | BlendKind::Masked) {
            return Ok(None);
        }
        let pipeline = ShadowKey {
            layout: key.layout.clone(),
            topology: key.topology,
            strip_index: key.strip_index,
            masked: key.blend == BlendKind::Masked,
        };
        self.ensure_shadow_pipeline(&pipeline)?;
        let local = self.vertex_buffers.get(call.vertex_buffer.0)?.bounds;
        let sphere = local.and_then(|(centre, radius)| {
            // Skinned vertices stray from their bind pose.
            let radius = if call.skin.is_some() {
                radius * 1.5 + 0.5
            } else {
                radius
            };
            if instances == 0 {
                return Some(shadow::transformed(call.world, (centre, radius)));
            }
            let palette = &self.frame.as_ref()?.palette;
            let (mut lo, mut hi, mut reach) =
                (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN), 0.0f32);
            for m in palette.get(instance_first as usize..(instance_first + instances) as usize)? {
                let (c, r) =
                    shadow::transformed(call.world * Mat4::from_cols_array(m), (centre, radius));
                lo = lo.min(c);
                hi = hi.max(c);
                reach = reach.max(r);
            }
            Some(((lo + hi) * 0.5, (hi - lo).length() * 0.5 + reach))
        });
        Ok(Some(Caster { pipeline, sphere }))
    }

    fn shader_module(&mut self, layout: &VertexLayout) -> &wgpu::ShaderModule {
        let device = &self.device;
        self.shaders.entry(layout.clone()).or_insert_with(|| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("standard"),
                source: wgpu::ShaderSource::Wgsl(shader::source(layout).into()),
            })
        })
    }

    fn ensure_shadow_pipeline(&mut self, key: &ShadowKey) -> Result<(), BackendError> {
        if self.shadow_pipelines.contains_key(key) {
            return Ok(());
        }
        let topology = wgpu_topology(key.topology)?;
        let attributes = vertex_attributes(&key.layout);
        let module = self.shader_module(&key.layout).clone();
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("shadow"),
                layout: Some(&self.shadow_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_shadow"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: u64::from(key.layout.stride()),
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attributes,
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    strip_index_format: key.strip_index.map(wgpu_index_format),
                    front_face: wgpu::FrontFace::Ccw,
                    // Both sides: walls seen edge-on from the sun and open
                    // meshes still cast.
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: shadow::FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState {
                        constant: 2,
                        slope_scale: 2.0,
                        clamp: 0.0,
                    },
                }),
                multisample: Default::default(),
                fragment: key.masked.then(|| wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_shadow"),
                    compilation_options: Default::default(),
                    targets: &[],
                }),
                multiview_mask: None,
                cache: None,
            });
        self.shadow_pipelines.insert(key.clone(), pipeline);
        Ok(())
    }

    /// Draws the casters among `draws` into each cascade's map.
    fn encode_shadows(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &FrameUniforms,
        cascades: &shadow::Cascades,
        refresh: [bool; shadow::CASCADES],
        draws: &[RecordedDraw],
    ) -> Result<(), BackendError> {
        for (i, (target, buffer, group)) in self.shadow_layers.iter().enumerate() {
            if !refresh[i] {
                continue;
            }
            let uniforms = FrameUniforms {
                view_proj: cascades.matrices[i].to_cols_array(),
                ..*frame
            };
            self.queue
                .write_buffer(buffer, 0, bytemuck::bytes_of(&uniforms));
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: target,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, group, &[]);
            let mut pipeline_now: Option<&ShadowKey> = None;
            let mut material_now = None;
            let mut vertices_now = None;
            let mut indices_now = None;
            for (k, d) in draws.iter().enumerate() {
                let Some(caster) = &d.caster else {
                    continue;
                };
                // Out of this cascade, or too small in it to show.
                if let Some((centre, radius)) = caster.sphere
                    && (radius < cascades.texels[i] * 2.0 || !cascades.touches(i, centre, radius))
                {
                    continue;
                }
                if pipeline_now != Some(&caster.pipeline) {
                    pass.set_pipeline(&self.shadow_pipelines[&caster.pipeline]);
                    pipeline_now = Some(&caster.pipeline);
                }
                pass.set_bind_group(1, &self.draw_group, &[(k as u64 * DRAW_STRIDE) as u32]);
                if caster.pipeline.masked && material_now != Some(d.material) {
                    let material = self
                        .materials
                        .get(&d.material)
                        .map_or(&self.default_material, |m| &m.bind_group);
                    pass.set_bind_group(2, material, &[]);
                    material_now = Some(d.material);
                } else if material_now.is_none() {
                    // Opaque casters run no fragment stage, but the layout
                    // still wants a material group bound.
                    pass.set_bind_group(2, &self.default_material, &[]);
                    material_now = Some(MaterialId(u32::MAX));
                }
                if vertices_now != Some(d.vertex_buffer.0) {
                    let vb = self.vertex_buffers.get(d.vertex_buffer.0)?;
                    pass.set_vertex_buffer(0, vb.buffer.slice(..));
                    vertices_now = Some(d.vertex_buffer.0);
                }
                match d.geometry {
                    Geometry::Vertices { first, count } => {
                        pass.draw(first..first + count, 0..d.instances);
                    }
                    Geometry::Indexed {
                        buffer,
                        first,
                        count,
                        base_vertex,
                    } => {
                        if indices_now != Some(buffer.0) {
                            let ib = self.index_buffers.get(buffer.0)?;
                            pass.set_index_buffer(
                                ib.buffer.slice(..),
                                wgpu_index_format(ib.desc.format),
                            );
                            indices_now = Some(buffer.0);
                        }
                        pass.draw_indexed(first..first + count, base_vertex, 0..d.instances);
                    }
                }
            }
        }
        Ok(())
    }

    /// The sky's cloud group for `noise` (white without one).
    fn cloud_group(&self, noise: Option<TextureHandle>) -> wgpu::BindGroup {
        let view = noise
            .and_then(|t| self.textures.get(t.0).ok())
            .map_or(&self.white, |t| &t.view);
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("clouds"),
            layout: &self.cloud_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.samplers[0]),
                },
            ],
        })
    }

    /// One world pass over `draws`, the first of which is draw `first` of
    /// the frame. The first pass clears and draws the sky.
    fn encode_world(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        sky: Vec3,
        clouds: Option<TextureHandle>,
        draws: &[RecordedDraw],
        first: usize,
        clear: bool,
    ) -> Result<(), BackendError> {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("world"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.post.hdr,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Linear sky colour; the post pass writes it to the sRGB
                    // frame.
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(sky.x),
                            g: f64::from(sky.y),
                            b: f64::from(sky.z),
                            a: 1.0,
                        })
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth.view,
                depth_ops: Some(wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(0.0)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    // Kept for the copy water reads.
                    store: if clear {
                        wgpu::StoreOp::Store
                    } else {
                        wgpu::StoreOp::Discard
                    },
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &self.frame_group, &[]);
        if clear {
            pass.set_pipeline(&self.sky);
            pass.set_bind_group(1, &self.cloud_group(clouds), &[]);
            pass.draw(0..3, 0..1);
        }
        pass.set_bind_group(3, &self.depth.group, &[]);
        // State is set only when it changes: the queue has sorted the
        // draws so neighbours share pipelines, materials and buffers, and
        // each command costs time to record and submit.
        let mut pipeline_now: Option<&PipelineKey> = None;
        let mut material_now = None;
        let mut vertices_now = None;
        let mut indices_now = None;
        for (i, d) in draws.iter().enumerate() {
            if pipeline_now != Some(&d.pipeline) {
                pass.set_pipeline(&self.pipelines[&d.pipeline]);
                pipeline_now = Some(&d.pipeline);
            }
            let offset = ((first + i) as u64 * DRAW_STRIDE) as u32;
            pass.set_bind_group(1, &self.draw_group, &[offset]);
            if material_now != Some(d.material) {
                let material = self
                    .materials
                    .get(&d.material)
                    .map_or(&self.default_material, |m| &m.bind_group);
                pass.set_bind_group(2, material, &[]);
                material_now = Some(d.material);
            }
            if vertices_now != Some(d.vertex_buffer.0) {
                let vb = self.vertex_buffers.get(d.vertex_buffer.0)?;
                pass.set_vertex_buffer(0, vb.buffer.slice(..));
                vertices_now = Some(d.vertex_buffer.0);
            }
            match d.geometry {
                Geometry::Vertices { first, count } => {
                    pass.draw(first..first + count, 0..d.instances);
                }
                Geometry::Indexed {
                    buffer,
                    first,
                    count,
                    base_vertex,
                } => {
                    if indices_now != Some(buffer.0) {
                        let ib = self.index_buffers.get(buffer.0)?;
                        pass.set_index_buffer(
                            ib.buffer.slice(..),
                            wgpu_index_format(ib.desc.format),
                        );
                        indices_now = Some(buffer.0);
                    }
                    pass.draw_indexed(first..first + count, base_vertex, 0..d.instances);
                }
            }
        }
        Ok(())
    }

    fn encode(
        &self,
        view: &View,
        draws: &[RecordedDraw],
        target: &wgpu::TextureView,
    ) -> Result<wgpu::CommandBuffer, BackendError> {
        let l = &view.lighting;
        let mut cascades = shadow::Cascades::new(view, l.sun_direction, l.shadow_distance);
        // Far cascades are redrawn every few frames only, each keeping the
        // matrix it was drawn with meanwhile: they cost as much as the near
        // ones and change little.
        let mut refresh = [true; shadow::CASCADES];
        if let (Some(c), Some(old)) = (&mut cascades, self.cascades.get()) {
            for (i, fresh) in refresh.iter_mut().enumerate() {
                if !shadow::refreshes(i, self.frames) {
                    c.keep(&old, i);
                    *fresh = false;
                }
            }
        }
        self.cascades.set(cascades);
        let frame = FrameUniforms {
            view_proj: (view.projection * view.view).to_cols_array(),
            eye: view.eye().extend(1.0).to_array(),
            sun_dir: l.sun_direction.extend(0.0).to_array(),
            sun_color: l
                .sun_color
                .extend(if l.gamma_light { 1.0 } else { 0.0 })
                .to_array(),
            sky: l.sky_color.extend(l.fog_density).to_array(),
            ambient_cube: l.ambient_cube.map(|c| c.extend(0.0).to_array()),
            rim: l.rim_color.extend(l.rim_threshold).to_array(),
            view_dir: (-view.view.inverse().z_axis.truncate())
                .normalize_or_zero()
                .extend(self.started.elapsed().as_secs_f32())
                .to_array(),
            shadow: cascades.map_or([[0.0; 16]; shadow::CASCADES], |c| {
                c.matrices.map(|m| m.to_cols_array())
            }),
            shadow_splits: cascades.map_or([0.0; 4], |c| c.splits),
            shadow_texels: cascades.map_or([0.0; 4], |c| c.texels),
            inv_view_proj: (view.projection * view.view).inverse().to_cols_array(),
            rayleigh: l.haze.rayleigh.extend(l.haze.g).to_array(),
            mie: l.haze.mie.extend(l.haze.inscattering).to_array(),
            scatter: [
                l.haze.extinction,
                l.haze.rayleigh_height,
                l.haze.mie_height,
                l.haze.density,
            ],
            clouds: [
                l.clouds.scales[0],
                l.clouds.scales[1],
                l.clouds.scales[2],
                l.clouds.coverage,
            ],
            wind: [
                l.clouds.wind[0],
                l.clouds.wind[1],
                if l.clouds.noise.is_some() && l.clouds.coverage > 0.0 {
                    if l.clouds.horizon_fade { 1.0 } else { 2.0 }
                } else {
                    0.0
                },
                l.clouds.darkness.unwrap_or(-1.0),
            ],
            water_ramp: l
                .water_ramp
                .map_or([[0.0; 4]; 4], |r| r.map(|c| c.to_array())),
            water_depth: [
                if l.water_ramp.is_some() { 1.0 } else { 0.0 },
                crate::WATER_RAMP_DEPTH,
                0.0,
                0.0,
            ],
            cloud_tone: l.clouds.tone.extend(l.clouds.opacity).to_array(),
            haze_layers: l.haze.layers.unwrap_or([0.0; 4]),
            deck: {
                let tint = l.haze.sun_tint.unwrap_or(Vec3::splat(-1.0));
                [l.clouds.reach.unwrap_or(0.0), tint.x, tint.y, tint.z]
            },
            surface: [
                if l.haze.grey_sun { 1.0 } else { 0.0 },
                l.beam_gain,
                if std::env::var("M2_GROUND_SPACE").is_ok_and(|v| v == "linear") {
                    1.0
                } else {
                    0.0
                },
                0.0,
            ],
        };
        self.queue
            .write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&frame));

        let mut encoder = self.device.create_command_encoder(&Default::default());
        if let Some(cascades) = &cascades {
            self.encode_shadows(&mut encoder, &frame, cascades, refresh, draws)?;
        }
        // Water reads the depth of what lies beneath it, so the world is
        // drawn in two passes split at the first water draw, with the
        // depth buffer copied between them.
        let split = draws
            .iter()
            .position(|d| {
                self.materials
                    .get(&d.material)
                    .is_some_and(|m| m.desc.water)
            })
            .unwrap_or(draws.len());
        self.encode_world(
            &mut encoder,
            l.sky_color,
            l.clouds.noise,
            &draws[..split],
            0,
            true,
        )?;
        if split < draws.len() {
            encoder.copy_texture_to_texture(
                self.depth.depth.as_image_copy(),
                self.depth.scene.as_image_copy(),
                self.depth.depth.size(),
            );
            self.encode_world(
                &mut encoder,
                l.sky_color,
                None,
                &draws[split..],
                split,
                false,
            )?;
        }
        self.post.encode(&self.queue, &mut encoder, l, target);
        Ok(encoder.finish())
    }
}

async fn open(
    instance: &wgpu::Instance,
    surface: Option<&wgpu::Surface<'_>>,
) -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue), BackendError> {
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: surface,
            apply_limit_buckets: false,
        })
        .await
        .map_err(device_error)?;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("m2"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            ..Default::default()
        })
        .await
        .map_err(device_error)?;
    Ok((adapter, device, queue))
}

impl RenderBackend for WgpuBackend {
    fn create_vertex_buffer(
        &mut self,
        desc: &VertexBufferDesc,
        initial: Option<&[u8]>,
    ) -> Result<VertexBufferHandle, BackendError> {
        let size = desc.size_in_bytes();
        let data = match initial {
            Some(bytes) if bytes.len() as u64 != size => {
                return Err(BackendError::InitialDataSize {
                    expected: size,
                    actual: bytes.len() as u64,
                });
            }
            Some(bytes) => padded(bytes),
            None => vec![0; size.next_multiple_of(4).max(4) as usize].into(),
        };
        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vertices"),
                contents: &data,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
        Ok(VertexBufferHandle(self.vertex_buffers.insert(
            VertexBuffer {
                desc: desc.clone(),
                buffer,
                bounds: initial.and_then(|b| shadow::bounds(&desc.layout, b)),
            },
        )))
    }

    fn create_index_buffer(
        &mut self,
        desc: &IndexBufferDesc,
        initial: Option<&[u8]>,
    ) -> Result<IndexBufferHandle, BackendError> {
        let size = desc.size_in_bytes();
        let shadow = match initial {
            Some(bytes) if bytes.len() as u64 != size => {
                return Err(BackendError::InitialDataSize {
                    expected: size,
                    actual: bytes.len() as u64,
                });
            }
            Some(bytes) => bytes.to_vec(),
            None => vec![0; size as usize],
        };
        let mut contents = shadow.clone();
        contents.resize(shadow.len().next_multiple_of(4).max(4), 0);
        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("indices"),
                contents: &contents,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            });
        Ok(IndexBufferHandle(self.index_buffers.insert(IndexBuffer {
            desc: *desc,
            buffer,
            shadow,
        })))
    }

    fn write_vertex_buffer(
        &mut self,
        buffer: VertexBufferHandle,
        offset: u64,
        data: &[u8],
    ) -> Result<(), BackendError> {
        let vb = self.vertex_buffers.get_mut(buffer.0)?;
        let size = vb.desc.size_in_bytes();
        let len = data.len() as u64;
        if offset.checked_add(len).is_none_or(|end| end > size) {
            return Err(BackendError::WriteOutOfBounds { offset, len, size });
        }
        if !offset.is_multiple_of(4) || !len.is_multiple_of(4) {
            return Err(BackendError::Unsupported(
                "vertex writes not aligned to 4 bytes",
            ));
        }
        // Its positions may have moved anywhere.
        vb.bounds = None;
        self.queue.write_buffer(&vb.buffer, offset, data);
        Ok(())
    }

    fn write_index_buffer(
        &mut self,
        buffer: IndexBufferHandle,
        offset: u64,
        data: &[u8],
    ) -> Result<(), BackendError> {
        let ib = self.index_buffers.get_mut(buffer.0)?;
        let size = ib.shadow.len() as u64;
        let len = data.len() as u64;
        let end = match offset.checked_add(len) {
            Some(end) if end <= size => end,
            _ => return Err(BackendError::WriteOutOfBounds { offset, len, size }),
        };
        ib.shadow[offset as usize..end as usize].copy_from_slice(data);
        let start = offset / 4 * 4;
        let stop = end.next_multiple_of(4);
        let mut upload = ib.shadow[start as usize..(stop.min(size)) as usize].to_vec();
        upload.resize((stop - start) as usize, 0);
        self.queue.write_buffer(&ib.buffer, start, &upload);
        Ok(())
    }

    fn destroy_vertex_buffer(&mut self, buffer: VertexBufferHandle) -> Result<(), BackendError> {
        self.vertex_buffers
            .remove(buffer.0)
            .map(|b| b.buffer.destroy())
    }

    fn destroy_index_buffer(&mut self, buffer: IndexBufferHandle) -> Result<(), BackendError> {
        self.index_buffers
            .remove(buffer.0)
            .map(|b| b.buffer.destroy())
    }

    fn create_texture(
        &mut self,
        desc: &TextureDesc,
        mips: &[&[u8]],
    ) -> Result<TextureHandle, BackendError> {
        desc.validate(mips)?;
        let max = self.device.limits().max_texture_dimension_2d;
        if desc.width > max || desc.height > max {
            return Err(BackendError::Texture(format!(
                "{}x{} exceeds the device limit of {max}",
                desc.width, desc.height
            )));
        }
        let format = match desc.format {
            TextureFormat::Rgba8Srgb => wgpu::TextureFormat::Rgba8UnormSrgb,
            TextureFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("texture"),
            size: wgpu::Extent3d {
                width: desc.width,
                height: desc.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: mips.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, data) in mips.iter().enumerate() {
            let (w, h) = desc.mip_size(level as u32);
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 4),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        }
        let view = texture.create_view(&Default::default());
        Ok(TextureHandle(self.textures.insert(Texture { view })))
    }

    fn destroy_texture(&mut self, texture: TextureHandle) -> Result<(), BackendError> {
        // The GPU texture is freed once no material bind group holds it.
        self.textures.remove(texture.0).map(drop)
    }

    fn set_material(&mut self, id: MaterialId, desc: &MaterialDesc) -> Result<(), BackendError> {
        let view = |t: Option<TextureHandle>| -> Result<&wgpu::TextureView, BackendError> {
            match t {
                Some(t) => Ok(&self.textures.get(t.0)?.view),
                None => Ok(&self.white),
            }
        };
        let views = MaterialViews {
            base: view(desc.texture)?,
            detail: view(desc.detail.map(|d| d.texture))?,
            specular: view(
                desc.specular
                    .and_then(|s| s.map)
                    .or(desc.emissive.and_then(|e| e.map)),
            )?,
            normal: view(desc.normal_map.map(|n| n.texture))?,
            layers: [
                view(desc.layers[0].map(|l| l.texture))?,
                view(desc.layers[1].map(|l| l.texture))?,
                view(desc.layers[2].map(|l| l.texture))?,
                view(desc.layers[3].map(|l| l.texture))?,
            ],
        };
        let bind_group = Self::material_group(
            &self.device,
            &self.material_layout,
            &self.samplers,
            &views,
            desc,
        );
        self.materials.insert(
            id,
            Material {
                desc: *desc,
                bind_group,
            },
        );
        Ok(())
    }

    fn live_buffers(&self) -> LiveBuffers {
        LiveBuffers {
            vertex: self.vertex_buffers.live,
            index: self.index_buffers.live,
            texture: self.textures.live,
        }
    }

    fn begin_frame(&mut self, view: &View) -> Result<(), BackendError> {
        if self.frame.is_some() {
            return Err(BackendError::FrameAlreadyOpen);
        }
        self.frame = Some(OpenFrame {
            view: *view,
            draws: Vec::new(),
            uniforms: Vec::new(),
            palette: Vec::new(),
        });
        Ok(())
    }

    fn set_skin(&mut self, matrices: &[Mat4]) -> Result<SkinHandle, BackendError> {
        let frame = self
            .frame
            .as_mut()
            .ok_or(BackendError::NotInFrame("set_skin"))?;
        let handle = SkinHandle {
            frame: self.frames,
            first: frame.palette.len() as u32,
            count: matrices.len() as u32,
        };
        frame
            .palette
            .extend(matrices.iter().map(|m| m.to_cols_array()));
        Ok(handle)
    }

    fn draw(&mut self, call: &DrawCall) -> Result<(), BackendError> {
        if self.frame.is_none() {
            return Err(BackendError::NotInFrame("draw"));
        }
        let vb = self.vertex_buffers.get(call.vertex_buffer.0)?;
        match call.geometry {
            Geometry::Vertices { first, count } => check_range(first, count, vb.desc.vertex_count)?,
            Geometry::Indexed {
                buffer,
                first,
                count,
                ..
            } => check_range(
                first,
                count,
                self.index_buffers.get(buffer.0)?.desc.index_count,
            )?,
        }
        let (first, count) = match call.skin {
            Some(skin) => {
                if skin.frame != self.frames {
                    return Err(BackendError::StaleSkin {
                        skin_frame: skin.frame,
                        frame: self.frames,
                    });
                }
                let available = self.frame.as_ref().map_or(0, |f| f.palette.len() as u32);
                check_range(skin.first, skin.count, available)?;
                (skin.first, skin.count)
            }
            None => (0, 0),
        };
        let (instance_first, instances) = match call.instances {
            Some(range) => {
                if range.frame != self.frames {
                    return Err(BackendError::StaleSkin {
                        skin_frame: range.frame,
                        frame: self.frames,
                    });
                }
                let available = self.frame.as_ref().map_or(0, |f| f.palette.len() as u32);
                check_range(range.first, range.count, available)?;
                (range.first, range.count)
            }
            None => (0, 0),
        };
        let key = self.pipeline_key(call)?;
        self.ensure_pipeline(&key)?;
        let caster = self.caster(call, &key, instance_first, instances)?;
        let frame = self.frame.as_mut().expect("checked above");
        let uniforms = &mut frame.uniforms;
        uniforms.extend_from_slice(bytemuck::cast_slice(&call.world.to_cols_array()));
        uniforms.extend_from_slice(bytemuck::cast_slice(&[
            first,
            count,
            instance_first,
            instances,
        ]));
        uniforms.extend_from_slice(bytemuck::cast_slice(&[
            call.uv_offset[0],
            call.uv_offset[1],
            0.0,
            0.0,
        ]));
        uniforms.resize(uniforms.len().next_multiple_of(DRAW_STRIDE as usize), 0);
        frame.draws.push(RecordedDraw {
            pipeline: key,
            vertex_buffer: call.vertex_buffer,
            geometry: call.geometry,
            material: call.material,
            instances: instances.max(1),
            caster,
        });
        Ok(())
    }

    fn end_frame(&mut self) -> Result<(), BackendError> {
        self.finish_frame(None)
    }
}

/// Where an overlay (a HUD, a menu) draws, after the world and before the
/// frame is shown. The colour target holds the finished world; there is no
/// depth.
#[derive(Debug)]
pub struct OverlayTarget<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub view: &'a wgpu::TextureView,
    pub format: wgpu::TextureFormat,
    /// Width and height in pixels.
    pub size: (u32, u32),
}

impl WgpuBackend {
    /// Ends the frame as [`RenderBackend::end_frame`] does, letting
    /// `overlay` draw over the world first.
    pub fn end_frame_with(
        &mut self,
        overlay: &mut dyn FnMut(OverlayTarget<'_>),
    ) -> Result<(), BackendError> {
        self.finish_frame(Some(overlay))
    }

    fn finish_frame(
        &mut self,
        mut overlay: Option<&mut dyn FnMut(OverlayTarget<'_>)>,
    ) -> Result<(), BackendError> {
        let OpenFrame {
            view,
            draws,
            uniforms,
            palette,
        } = self
            .frame
            .take()
            .ok_or(BackendError::NotInFrame("end_frame"))?;
        // Counted even if the frame is skipped below, so palettes set for it
        // are never accepted by the next.
        self.frames += 1;
        let mut regroup = false;
        if uniforms.len() as u64 > self.draw_buffer.size() {
            let draws = (uniforms.len() as u64 / DRAW_STRIDE).next_power_of_two();
            self.draw_buffer = Self::draw_uniforms(&self.device, draws);
            regroup = true;
        }
        if palette.len() as u64 * 64 > self.palette_buffer.size() {
            self.palette_buffer =
                Self::palette(&self.device, (palette.len() as u64).next_power_of_two());
            regroup = true;
        }
        if regroup {
            self.draw_group = Self::draw_group(
                &self.device,
                &self.draw_layout,
                &self.draw_buffer,
                &self.palette_buffer,
            );
        }
        if !uniforms.is_empty() {
            self.queue.write_buffer(&self.draw_buffer, 0, &uniforms);
        }
        if !palette.is_empty() {
            self.queue
                .write_buffer(&self.palette_buffer, 0, bytemuck::cast_slice(&palette));
        }
        let size = self.size();
        let format = self.color_format();
        let over = |target: &wgpu::TextureView,
                    overlay: &mut Option<&mut dyn FnMut(OverlayTarget<'_>)>| {
            overlay.as_mut().map(|f| {
                let mut encoder = self.device.create_command_encoder(&Default::default());
                f(OverlayTarget {
                    device: &self.device,
                    queue: &self.queue,
                    encoder: &mut encoder,
                    view: target,
                    format,
                    size,
                });
                encoder.finish()
            })
        };
        match &self.target {
            Target::Offscreen { texture, .. } => {
                let target = texture.create_view(&Default::default());
                let commands = self.encode(&view, &draws, &target)?;
                let overlay = over(&target, &mut overlay);
                self.queue.submit(std::iter::once(commands).chain(overlay));
            }
            Target::Window { surface, config } => {
                let frame = match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(f)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
                    wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                        // Reconfigure and skip this frame.
                        surface.configure(&self.device, config);
                        return Ok(());
                    }
                    wgpu::CurrentSurfaceTexture::Timeout
                    | wgpu::CurrentSurfaceTexture::Occluded => return Ok(()),
                    other => return Err(BackendError::Device(format!("{other:?}"))),
                };
                let target = frame.texture.create_view(&Default::default());
                let commands = self.encode(&view, &draws, &target)?;
                let overlay = over(&target, &mut overlay);
                self.queue.submit(std::iter::once(commands).chain(overlay));
                self.queue.present(frame);
            }
        }
        Ok(())
    }
}
