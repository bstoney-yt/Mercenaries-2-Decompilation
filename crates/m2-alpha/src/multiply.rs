//! Flash's multiply blend mode for the movies' triangles that use it
//! (`PlaceObject3`'s blend mode 3: the front end's grime over its menus),
//! which egui's blending (alpha over) cannot draw: painted through an
//! egui paint callback with a pipeline of its own, whose blend multiplies
//! what is under by the triangle's colour.
//!
//! Where a triangle covers with alpha `a` and colour `c` (its texture's
//! times its tint), Flash leaves `under * (1 - a + c * a)`. The shader
//! outputs that factor and the blend multiplies the target by it (source
//! factor the target's colour, target factor zero); the target's alpha
//! stays. On an sRGB target the factor goes to linear first: a product
//! of gamma values is the product of their linear values, near enough.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use m2_render::gpu::wgpu;
use m2_render::gpu::wgpu::util::DeviceExt as _;

/// RGBA8 pixels, unmultiplied.
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// One vertex: where (clip space), texture coordinates and colour.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub colour: [f32; 4],
}

const SHADER: &str = r#"
struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) colour: vec4<f32>,
};

@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var filtering: sampler;
@group(0) @binding(2) var<uniform> linear: vec4<f32>;

@vertex
fn vs(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) colour: vec4<f32>) -> Out {
    var o: Out;
    o.pos = vec4<f32>(pos, 0.0, 1.0);
    o.uv = uv;
    o.colour = colour;
    return o;
}

@fragment
fn fs(i: Out) -> @location(0) vec4<f32> {
    let c = textureSample(image, filtering, i.uv) * i.colour;
    var f = vec3<f32>(1.0) - vec3<f32>(c.a) + c.rgb * c.a;
    if (linear.x > 0.5) {
        f = pow(f, vec3<f32>(2.2));
    }
    return vec4<f32>(f, 1.0);
}
"#;

/// The pipeline, the textures it has been given and the vertices of the
/// callbacks under way: kept in egui's callback resources
/// ([`Resources::install`]).
pub struct Resources {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    linear: wgpu::Buffer,
    /// Bind groups by texture key.
    textures: HashMap<u64, wgpu::BindGroup>,
    /// Vertex buffers by callback, and their vertex counts.
    vertices: HashMap<u64, (wgpu::Buffer, u32)>,
}

impl Resources {
    /// Adds the resources to `resources`, for drawing into `format`.
    pub fn install(
        resources: &mut CallbackResources,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("multiply"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("multiply"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("multiply"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("multiply"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4],
                })],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Dst,
                            dst_factor: wgpu::BlendFactor::Zero,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Zero,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("multiply"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            ..Default::default()
        });
        let flag = if format.is_srgb() { 1.0f32 } else { 0.0 };
        let linear = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("multiply"),
            contents: bytemuck::cast_slice(&[flag, 0.0, 0.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        resources.insert(Resources {
            pipeline,
            layout,
            sampler,
            linear,
            textures: HashMap::new(),
            vertices: HashMap::new(),
        });
    }
}

/// Numbers each callback, for its vertices among the resources.
static NEXT: AtomicU64 = AtomicU64::new(1);

/// Triangles drawn multiplying, all with one texture.
pub struct Multiply {
    id: u64,
    /// The texture's key and pixels (a white texel for solid fills).
    texture: (u64, Arc<Pixels>),
    vertices: Vec<Vertex>,
}

impl Multiply {
    /// `vertices` in clip space of the callback's rectangle.
    pub fn new(texture: (u64, Arc<Pixels>), vertices: Vec<Vertex>) -> Self {
        Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            texture,
            vertices,
        }
    }
}

impl CallbackTrait for Multiply {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(r) = resources.get_mut::<Resources>() else {
            return Vec::new();
        };
        let (key, pixels) = &self.texture;
        if !r.textures.contains_key(key) {
            let size = wgpu::Extent3d {
                width: pixels.width.max(1),
                height: pixels.height.max(1),
                depth_or_array_layers: 1,
            };
            let texture = device.create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("multiply"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &pixels.rgba,
            );
            let view = texture.create_view(&Default::default());
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("multiply"),
                layout: &r.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&r.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: r.linear.as_entire_binding(),
                    },
                ],
            });
            r.textures.insert(*key, group);
        }
        // Last frame's callbacks are done with.
        r.vertices.retain(|id, _| *id + 4096 > self.id);
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("multiply"),
            contents: bytemuck::cast_slice(&self.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        r.vertices
            .insert(self.id, (buffer, self.vertices.len() as u32));
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let Some(r) = resources.get::<Resources>() else {
            return;
        };
        let (Some(group), Some((buffer, count))) =
            (r.textures.get(&self.texture.0), r.vertices.get(&self.id))
        else {
            return;
        };
        pass.set_pipeline(&r.pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.set_vertex_buffer(0, buffer.slice(..));
        pass.draw(0..*count, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use m2_render::gpu::wgpu::naga;

    #[test]
    fn the_shader_compiles() {
        let module = naga::front::wgsl::parse_str(super::SHADER)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(super::SHADER)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
    }
}
