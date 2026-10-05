//! After the world: bloom, exposure and a colour grade.
//!
//! The world draws into a floating-point target; this pass takes its bright
//! parts down to a quarter-size image, blurs them, and adds them back over
//! the scene, scaled by exposure, contrast, saturation and grade, into the
//! frame. The game runs a post chain of this kind (a luminance tone map,
//! bloom and depth of field, with `Bloom*` and `Tone_*` settings in its
//! atmosphere); its exact curve is not known, so this one is plain: light
//! above a threshold glows, everything is scaled, and the frame clamps at
//! white. Contrast and saturation are set to match footage of the game.
//!
//! # Provenance
//!
//! Clean-room: a standard bright-pass, separable blur and composite,
//! written against wgpu's public API. Adaptation is the usual one too:
//! the frame's log luminance averaged down to one texel, followed over
//! time, and exposure scaled towards a key.

use std::cell::Cell;
use std::time::Instant;

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
    // x: contrast, y: saturation, z: the luminance contrast pivots on
    tone: vec4<f32>,
    // x: key, y: strength (0: no adaptation), z: how much of the way to
    // follow this frame, w: unused
    adapt: vec4<f32>,
    // x, y: the least and most exposure is scaled by
    limits: vec4<f32>,
};

@group(0) @binding(0) var<uniform> post: Post;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var glow: texture_2d<f32>;
@group(0) @binding(3) var linear: sampler;
// The adapted log luminance (one texel).
@group(0) @binding(4) var adapted: texture_2d<f32>;

// Exposure, scaled for the eye's adaptation.
fn exposure() -> f32 {
    if post.adapt.y <= 0.0 {
        return post.params.x;
    }
    let average = exp(textureLoad(adapted, vec2<i32>(0, 0), 0).r);
    let scale = pow(post.adapt.x / max(average, 1e-4), post.adapt.y);
    return post.params.x * clamp(scale, post.limits.x, post.limits.y);
}

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
    c = c * 0.25 * exposure();
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

// The frame's log luminance, from four filtered taps.
@fragment
fn fs_log_luminance(v: Screen) -> @location(0) vec4<f32> {
    var sum = 0.0;
    for (var y = -1.0; y <= 1.0; y = y + 2.0) {
        for (var x = -1.0; x <= 1.0; x = x + 2.0) {
            let c = textureSampleLevel(source, linear, v.uv + vec2<f32>(x, y) * post.step.xy, 0.0).rgb;
            sum = sum + log(max(luminance(c), 1e-4));
        }
    }
    return vec4<f32>(sum * 0.25, 0.0, 0.0, 1.0);
}

// The average of each 4 x 4 block: four filtered taps.
@fragment
fn fs_reduce(v: Screen) -> @location(0) vec4<f32> {
    var sum = 0.0;
    for (var y = -1.0; y <= 1.0; y = y + 2.0) {
        for (var x = -1.0; x <= 1.0; x = x + 2.0) {
            sum = sum + textureSampleLevel(source, linear, v.uv + vec2<f32>(x, y) * post.step.xy, 0.0).r;
        }
    }
    return vec4<f32>(sum * 0.25, 0.0, 0.0, 1.0);
}

// The adapted luminance, moved part of the way to this frame's.
@fragment
fn fs_adapt(v: Screen) -> @location(0) vec4<f32> {
    let now = textureLoad(source, vec2<i32>(0, 0), 0).r;
    let before = textureLoad(adapted, vec2<i32>(0, 0), 0).r;
    return vec4<f32>(mix(before, now, post.adapt.z), 0.0, 0.0, 1.0);
}

@fragment
fn fs_composite(v: Screen) -> @location(0) vec4<f32> {
    var c = textureSampleLevel(source, linear, v.uv, 0.0).rgb * exposure();
    c = c + textureSampleLevel(glow, linear, v.uv, 0.0).rgb * post.params.z;
    // Contrast on luminance about a mid grey, keeping each colour's hue,
    // then saturation.
    let y = luminance(c);
    let pivot = post.tone.z;
    let toned = pivot * pow(max(y, 0.0) / pivot, post.tone.x);
    c = c * (toned / max(y, 1e-5));
    c = max(mix(vec3<f32>(toned), c, post.tone.y), vec3<f32>(0.0));
    return vec4<f32>(c * post.grade.rgb, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    params: [f32; 4],
    grade: [f32; 4],
    step: [f32; 4],
    tone: [f32; 4],
    adapt: [f32; 4],
    limits: [f32; 4],
}

/// The passes, in order: bright, blur across, blur down, composite, then
/// log luminance and its reductions down to one texel, and adaptation.
const PASSES: usize = 5 + LUMINANCE.len();
/// Side of each luminance image, from the frame's down to one texel.
const LUMINANCE: [u32; 5] = [256, 64, 16, 4, 1];
/// Pass indices.
const BRIGHT: usize = 0;
const COMPOSITE: usize = 3;
const LOG_LUMINANCE: usize = 4;
const ADAPT: usize = PASSES - 1;

/// Targets and pipelines of the post pass.
#[derive(Debug)]
pub struct Post {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bright: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    log_luminance: wgpu::RenderPipeline,
    reduce: wgpu::RenderPipeline,
    adapt: wgpu::RenderPipeline,
    uniforms: [wgpu::Buffer; PASSES],
    /// The world's floating-point target.
    pub hdr: wgpu::TextureView,
    /// Quarter-size glow images, ping-ponged by the blur.
    glow: [wgpu::TextureView; 2],
    /// The frame's log luminance, down to one texel.
    luminance: Vec<wgpu::TextureView>,
    /// The adapted log luminance, one texel each, ping-ponged by frame.
    adapted: [wgpu::TextureView; 2],
    /// Each pass's bind group, for each way round of `adapted`.
    groups: [[wgpu::BindGroup; PASSES]; 2],
    size: (u32, u32),
    /// Which `adapted` holds the last frame's, and when that was.
    parity: Cell<usize>,
    last: Cell<Option<Instant>>,
}

/// A floating-point image `width` x `height` to draw into and sample.
fn target(device: &wgpu::Device, label: &str, width: u32, height: u32) -> wgpu::TextureView {
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default())
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
                texture(4),
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
        let luminance: Vec<wgpu::TextureView> = LUMINANCE
            .iter()
            .map(|&side| target(device, "luminance", side, side))
            .collect();
        let adapted = std::array::from_fn(|_| target(device, "adapted", 1, 1));
        let (hdr, glow, groups) = Self::targets(
            device, &layout, &sampler, &uniforms, &luminance, &adapted, width, height,
        );
        Self {
            bright: pipeline("fs_bright", HDR_FORMAT),
            blur: pipeline("fs_blur", HDR_FORMAT),
            composite: pipeline("fs_composite", output),
            log_luminance: pipeline("fs_log_luminance", HDR_FORMAT),
            reduce: pipeline("fs_reduce", HDR_FORMAT),
            adapt: pipeline("fs_adapt", HDR_FORMAT),
            layout,
            sampler,
            uniforms,
            hdr,
            glow,
            luminance,
            adapted,
            groups,
            size: (width, height),
            parity: Cell::new(0),
            last: Cell::new(None),
        }
    }

    /// The world target, glow images and each pass's bind groups for a
    /// frame `width` x `height`.
    #[allow(clippy::too_many_arguments)]
    fn targets(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        uniforms: &[wgpu::Buffer; PASSES],
        luminance: &[wgpu::TextureView],
        adapted: &[wgpu::TextureView; 2],
        width: u32,
        height: u32,
    ) -> (
        wgpu::TextureView,
        [wgpu::TextureView; 2],
        [[wgpu::BindGroup; PASSES]; 2],
    ) {
        let hdr = target(device, "world", width, height);
        let glow = [
            target(device, "glow", width / 4, height / 4),
            target(device, "glow", width / 4, height / 4),
        ];
        let group = |i: usize,
                     source: &wgpu::TextureView,
                     glow: &wgpu::TextureView,
                     adapted: &wgpu::TextureView| {
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
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(adapted),
                    },
                ],
            })
        };
        // With last frame's adaptation in `adapted[p]`: log luminance:
        // world -> luminance 0; each reduction: luminance i - 1 -> i;
        // adapt: the last luminance and adapted[p] -> adapted[1 - p].
        // Bright: world -> glow 0; across: 0 -> 1; down: 1 -> 0;
        // composite: world + glow 0 -> frame, both with adapted[1 - p]. A
        // pass's unused bindings are filled with images it doesn't draw
        // into.
        let last = luminance.len() - 1;
        let groups = std::array::from_fn(|p| {
            let (before, now) = (&adapted[p], &adapted[1 - p]);
            std::array::from_fn(|i| match i {
                BRIGHT => group(i, &hdr, &glow[1], now),
                1 => group(i, &glow[0], &hdr, now),
                2 => group(i, &glow[1], &hdr, now),
                COMPOSITE => group(i, &hdr, &glow[0], now),
                LOG_LUMINANCE => group(i, &hdr, &glow[0], now),
                ADAPT => group(i, &luminance[last], &glow[0], before),
                _ => {
                    let level = i - LOG_LUMINANCE;
                    group(i, &luminance[level - 1], &glow[0], now)
                }
            })
        });
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
            &self.luminance,
            &self.adapted,
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
        // How far to follow this frame's luminance: all the way at first.
        let now = Instant::now();
        let dt = self
            .last
            .replace(Some(now))
            .map(|t| now.duration_since(t).as_secs_f32().min(1.0));
        let (adapt, limits) = match lighting.adaptation {
            Some(a) => (
                [
                    a.key,
                    a.strength,
                    dt.map_or(1.0, |dt| 1.0 - (-dt * a.speed).exp()),
                    0.0,
                ],
                [a.limits[0], a.limits[1], 0.0, 0.0],
            ),
            None => ([0.0; 4], [1.0, 1.0, 0.0, 0.0]),
        };
        let mut steps = [[0.0f32; 2]; PASSES];
        steps[BRIGHT] = [1.0 / w, 1.0 / h];
        steps[1] = [1.0 / gw, 0.0];
        steps[2] = [0.0, 1.0 / gh];
        // Four taps spread over each texel of the first luminance image,
        // then each reduction's taps at the middle of 4 x 4 blocks.
        steps[LOG_LUMINANCE] = [0.25 / LUMINANCE[0] as f32; 2];
        for level in 1..LUMINANCE.len() {
            steps[LOG_LUMINANCE + level] = [1.0 / LUMINANCE[level - 1] as f32; 2];
        }
        for (buffer, step) in self.uniforms.iter().zip(steps) {
            let u = Uniforms {
                params: if glowing {
                    params
                } else {
                    [params[0], 0.0, 0.0, 0.0]
                },
                grade,
                step: [step[0], step[1], 0.0, 0.0],
                tone: [lighting.contrast, lighting.saturation, 0.18, 0.0],
                adapt,
                limits,
            };
            queue.write_buffer(buffer, 0, bytemuck::bytes_of(&u));
        }
        let parity = self.parity.get();
        let groups = &self.groups[parity];
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
        if lighting.adaptation.is_some() {
            pass(
                &self.log_luminance,
                &groups[LOG_LUMINANCE],
                &self.luminance[0],
            );
            for level in 1..LUMINANCE.len() {
                pass(
                    &self.reduce,
                    &groups[LOG_LUMINANCE + level],
                    &self.luminance[level],
                );
            }
            pass(&self.adapt, &groups[ADAPT], &self.adapted[1 - parity]);
            self.parity.set(1 - parity);
        }
        if glowing {
            pass(&self.bright, &groups[BRIGHT], &self.glow[0]);
            pass(&self.blur, &groups[1], &self.glow[1]);
            pass(&self.blur, &groups[2], &self.glow[0]);
        }
        pass(&self.composite, &groups[COMPOSITE], target);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn post_shader_compiles() {
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
