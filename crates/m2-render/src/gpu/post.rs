//! After the world: bloom, exposure and a colour grade.
//!
//! The world draws into a floating-point target; this pass takes its bright
//! parts down to a quarter-size image, blurs them, and adds them back over
//! the scene, scaled by exposure and grade, into the frame. The game runs
//! a post chain of this kind (a luminance tone map, bloom and depth of
//! field, with `Bloom*` and `Tone_*` settings in its atmosphere); its
//! exact curve is not known, so this one is plain: light above a
//! threshold glows, everything is scaled, and the frame clamps at white.
//!
//! # Provenance
//!
//! Clean-room: a standard bright-pass, separable blur and composite,
//! written against wgpu's public API.

use crate::backend::Lighting;

/// Format the world draws in, before the post pass.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

const SHADER: &str = r#"
struct Post {
    // x: exposure, y: bloom threshold, z: bloom amount
    params: vec4<f32>,
    // rgb: colour grade
    grade: vec4<f32>,
    // xy: sampling step in texture coordinates
    step: vec4<f32>,
};

@group(0) @binding(0) var<uniform> post: Post;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var glow: texture_2d<f32>;
@group(0) @binding(3) var linear: sampler;

struct Screen {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// One triangle covering the screen.
@vertex
fn vs_screen(@builtin(vertex_index) i: u32) -> Screen {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Screen;
    out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// A quarter-size image of the light above the threshold: four filtered
// taps cover each 4 x 4 block of the scene.
@fragment
fn fs_bright(v: Screen) -> @location(0) vec4<f32> {
    var c = vec3<f32>(0.0);
    for (var y = -1.0; y <= 1.0; y = y + 2.0) {
        for (var x = -1.0; x <= 1.0; x = x + 2.0) {
            c = c + textureSampleLevel(source, linear, v.uv + vec2<f32>(x, y) * post.step.xy, 0.0).rgb;
        }
    }
    c = c * 0.25 * post.params.x;
    let l = luminance(c);
    return vec4<f32>(c * (max(l - post.params.y, 0.0) / max(l, 1e-4)), 1.0);
}

// Nine-tap Gaussian along `step`.
@fragment
fn fs_blur(v: Screen) -> @location(0) vec4<f32> {
    let weights = array<f32, 5>(0.227027, 0.1945946, 0.1216216, 0.054054, 0.016216);
    var c = textureSampleLevel(source, linear, v.uv, 0.0).rgb * weights[0];
    for (var i = 1; i < 5; i = i + 1) {
        let o = post.step.xy * f32(i);
        c = c + textureSampleLevel(source, linear, v.uv + o, 0.0).rgb * weights[i];
        c = c + textureSampleLevel(source, linear, v.uv - o, 0.0).rgb * weights[i];
    }
    return vec4<f32>(c, 1.0);
}

@fragment
fn fs_composite(v: Screen) -> @location(0) vec4<f32> {
    var c = textureSampleLevel(source, linear, v.uv, 0.0).rgb * post.params.x;
    c = c + textureSampleLevel(glow, linear, v.uv, 0.0).rgb * post.params.z;
    return vec4<f32>(c * post.grade.rgb, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    params: [f32; 4],
    grade: [f32; 4],
    step: [f32; 4],
}

/// The passes, in order: bright, blur across, blur down, composite.
const PASSES: usize = 4;

/// Targets and pipelines of the post pass.
#[derive(Debug)]
pub struct Post {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bright: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    uniforms: [wgpu::Buffer; PASSES],
    /// The world's floating-point target.
    pub hdr: wgpu::TextureView,
    /// Quarter-size glow images, ping-ponged by the blur.
    glow: [wgpu::TextureView; 2],
    groups: [wgpu::BindGroup; PASSES],
    size: (u32, u32),
}

impl Post {
    pub fn new(
        device: &wgpu::Device,
        output: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(1),
                texture(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = |entry: &str, format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_screen"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniforms = std::array::from_fn(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("post uniforms"),
                size: size_of::<Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        let (hdr, glow, groups) =
            Self::targets(device, &layout, &sampler, &uniforms, width, height);
        Self {
            bright: pipeline("fs_bright", HDR_FORMAT),
            blur: pipeline("fs_blur", HDR_FORMAT),
            composite: pipeline("fs_composite", output),
            layout,
            sampler,
            uniforms,
            hdr,
            glow,
            groups,
            size: (width, height),
        }
    }

    /// The world target, glow images and each pass's bind group for a
    /// frame `width` x `height`.
    fn targets(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        uniforms: &[wgpu::Buffer; PASSES],
        width: u32,
        height: u32,
    ) -> (
        wgpu::TextureView,
        [wgpu::TextureView; 2],
        [wgpu::BindGroup; PASSES],
    ) {
        let target = |label, width: u32, height: u32| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: width.max(1),
                        height: height.max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: HDR_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let hdr = target("world", width, height);
        let glow = [
            target("glow", width / 4, height / 4),
            target("glow", width / 4, height / 4),
        ];
        let group = |i: usize, source: &wgpu::TextureView, glow: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("post"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniforms[i].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(glow),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            })
        };
        // Bright: world -> glow 0; across: 0 -> 1; down: 1 -> 0;
        // composite: world + glow 0 -> frame. A pass's unused `glow`
        // binding is filled with an image it doesn't draw into.
        let groups = [
            group(0, &hdr, &glow[1]),
            group(1, &glow[0], &hdr),
            group(2, &glow[1], &hdr),
            group(3, &hdr, &glow[0]),
        ];
        (hdr, glow, groups)
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if (width, height) == self.size {
            return;
        }
        (self.hdr, self.glow, self.groups) = Self::targets(
            device,
            &self.layout,
            &self.sampler,
            &self.uniforms,
            width,
            height,
        );
        self.size = (width, height);
    }

    /// Runs the passes for a frame lit by `lighting`, into `target`.
    pub fn encode(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        lighting: &Lighting,
        target: &wgpu::TextureView,
    ) {
        let (w, h) = (self.size.0 as f32, self.size.1 as f32);
        let (gw, gh) = (
            (self.size.0 / 4).max(1) as f32,
            (self.size.1 / 4).max(1) as f32,
        );
        let params = [
            lighting.exposure,
            lighting.bloom_threshold,
            lighting.bloom,
            0.0,
        ];
        let grade = lighting.grade.extend(1.0).to_array();
        let glowing = lighting.bloom > 0.0;
        let steps = [
            [1.0 / w, 1.0 / h],
            [1.0 / gw, 0.0],
            [0.0, 1.0 / gh],
            [0.0, 0.0],
        ];
        for (buffer, step) in self.uniforms.iter().zip(steps) {
            let u = Uniforms {
                params: if glowing {
                    params
                } else {
                    [params[0], 0.0, 0.0, 0.0]
                },
                grade,
                step: [step[0], step[1], 0.0, 0.0],
            };
            queue.write_buffer(buffer, 0, bytemuck::bytes_of(&u));
        }
        let mut pass = |pipeline: &wgpu::RenderPipeline, group: &wgpu::BindGroup, target| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        };
        if glowing {
            pass(&self.bright, &self.groups[0], &self.glow[0]);
            pass(&self.blur, &self.groups[1], &self.glow[1]);
            pass(&self.blur, &self.groups[2], &self.glow[0]);
        }
        pass(&self.composite, &self.groups[3], target);
    }
}
