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

mod shader;

use std::collections::HashMap;

use ::wgpu::util::DeviceExt;
use m2_math::glam::Mat4;

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
/// matrix and the palette length, then the same for its instances.
const DRAW_UNIFORM_SIZE: u64 = 80;
/// Bone matrices the palette buffer starts with.
const INITIAL_PALETTE: u64 = 1024;
/// Metres from the camera by which a detail map has faded out.
const DETAIL_FADE: f32 = 80.0;
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
    ambient: [f32; 4],
    sky: [f32; 4],
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
    /// x: specular map, y: normal map, z: normal x in alpha.
    maps: [f32; 4],
    /// Per ground layer, the u row then the v row.
    layer_uv: [[f32; 4]; 8],
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
    depth: wgpu::TextureView,

    draw_layout: wgpu::BindGroupLayout,
    material_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    frame_buffer: wgpu::Buffer,
    frame_group: wgpu::BindGroup,
    draw_buffer: wgpu::Buffer,
    palette_buffer: wgpu::Buffer,
    draw_group: wgpu::BindGroup,
    /// Repeating, then clamping.
    samplers: [wgpu::Sampler; 2],
    white: wgpu::TextureView,
    default_material: wgpu::BindGroup,

    shaders: HashMap<VertexLayout, wgpu::ShaderModule>,
    pipelines: HashMap<PipelineKey, wgpu::RenderPipeline>,
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("standard"),
            bind_group_layouts: &[
                Some(&frame_layout),
                Some(&draw_layout),
                Some(&material_layout),
            ],
            immediate_size: 0,
        });
        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame uniforms"),
            size: size_of::<FrameUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: frame_buffer.as_entire_binding(),
            }],
        });
        let draw_buffer = Self::draw_uniforms(&device, 1024);
        let palette_buffer = Self::palette(&device, INITIAL_PALETTE);
        let draw_group = Self::draw_group(&device, &draw_layout, &draw_buffer, &palette_buffer);
        let samplers = [wgpu::AddressMode::Repeat, wgpu::AddressMode::ClampToEdge].map(|mode| {
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
        let depth = Self::depth_view(&device, width, height);
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
            draw_layout,
            material_layout,
            pipeline_layout,
            frame_buffer,
            frame_group,
            draw_buffer,
            palette_buffer,
            draw_group,
            samplers,
            white,
            shaders: HashMap::new(),
            pipelines: HashMap::new(),
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

    fn depth_view(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("depth"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
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
                0.0,
            ],
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
        self.depth = Self::depth_view(&self.device, width, height);
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
            },
            double_sided: desc.double_sided,
            depth_bias: desc.depth_bias,
        })
    }

    fn ensure_pipeline(&mut self, key: &PipelineKey) -> Result<(), BackendError> {
        if self.pipelines.contains_key(key) {
            return Ok(());
        }
        let color_format = self.color_format();
        let device = &self.device;
        let module = self.shaders.entry(key.layout.clone()).or_insert_with(|| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("standard"),
                source: wgpu::ShaderSource::Wgsl(shader::source(&key.layout).into()),
            })
        });
        let attributes: Vec<wgpu::VertexAttribute> = key
            .layout
            .elements()
            .iter()
            .enumerate()
            .map(|(i, e)| wgpu::VertexAttribute {
                format: wgpu_vertex_format(e.format),
                offset: u64::from(e.offset),
                shader_location: i as u32,
            })
            .collect();
        let translucent = key.blend == BlendKind::Translucent;
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
                    blend: translucent.then_some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.insert(key.clone(), pipeline);
        Ok(())
    }

    fn encode(
        &self,
        view: &View,
        draws: &[RecordedDraw],
        target: &wgpu::TextureView,
    ) -> Result<wgpu::CommandBuffer, BackendError> {
        let l = &view.lighting;
        let frame = FrameUniforms {
            view_proj: (view.projection * view.view).to_cols_array(),
            eye: view.eye().extend(1.0).to_array(),
            sun_dir: l.sun_direction.extend(0.0).to_array(),
            sun_color: l.sun_color.extend(0.0).to_array(),
            ambient: l.ambient.extend(0.0).to_array(),
            sky: l.sky_color.extend(l.fog_density).to_array(),
        };
        self.queue
            .write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&frame));

        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            // Linear sky colour; the sRGB target encodes it.
            let sky = l.sky_color;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(sky.x),
                            g: f64::from(sky.y),
                            b: f64::from(sky.z),
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.frame_group, &[]);
            // State is set only when it changes: the queue has sorted the
            // draws so neighbours share pipelines, materials and buffers,
            // and each command costs time to record and submit.
            let mut pipeline_now: Option<&PipelineKey> = None;
            let mut material_now = None;
            let mut vertices_now = None;
            let mut indices_now = None;
            for (i, d) in draws.iter().enumerate() {
                if pipeline_now != Some(&d.pipeline) {
                    pass.set_pipeline(&self.pipelines[&d.pipeline]);
                    pipeline_now = Some(&d.pipeline);
                }
                pass.set_bind_group(1, &self.draw_group, &[(i as u64 * DRAW_STRIDE) as u32]);
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
        }
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
        let vb = self.vertex_buffers.get(buffer.0)?;
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
            specular: view(desc.specular.and_then(|s| s.map))?,
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
        let frame = self.frame.as_mut().expect("checked above");
        let uniforms = &mut frame.uniforms;
        uniforms.extend_from_slice(bytemuck::cast_slice(&call.world.to_cols_array()));
        uniforms.extend_from_slice(bytemuck::cast_slice(&[
            first,
            count,
            instance_first,
            instances,
        ]));
        uniforms.resize(uniforms.len().next_multiple_of(DRAW_STRIDE as usize), 0);
        frame.draws.push(RecordedDraw {
            pipeline: key,
            vertex_buffer: call.vertex_buffer,
            geometry: call.geometry,
            material: call.material,
            instances: instances.max(1),
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
