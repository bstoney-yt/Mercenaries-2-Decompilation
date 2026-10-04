//! WGSL generated per vertex layout.
//!
//! Model vertex formats vary (positions as `float3` or `half4`, optional
//! normals, colours and texture coordinates), so each layout gets its own
//! vertex stage reading exactly the attributes it has and filling in
//! defaults for the rest. The fragment stage is shared text.

use std::fmt::Write;

use crate::buffer::{VertexFormat, VertexLayout, VertexSemantic};

/// Bind group 0: per frame. Must match `FrameUniforms` in `mod.rs`.
const COMMON: &str = r#"
struct Frame {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    ambient: vec4<f32>,
    // rgb: sky and fog colour, a: fog density per metre
    sky: vec4<f32>,
};

struct Draw {
    world: mat4x4<f32>,
    // x: first palette matrix, y: palette length (0: not skinned),
    // z: first instance matrix, w: instance count (0: not instanced)
    skin: vec4<u32>,
};

struct Material {
    color: vec4<f32>,
    // x: alpha cutoff, y: lit, z: vertex colour, w: fog
    params: vec4<f32>,
    // x: detail tiles per metre (0: none), y: fade-out distance,
    // z: soft edge, w: ground layers
    detail: vec4<f32>,
    // rgb: highlight colour (0: none), a: exponent
    specular: vec4<f32>,
    // x: specular map, y: normal map, z: normal x in alpha
    maps: vec4<f32>,
    // Per ground layer, u then v as rows over model-space (x, y, z, 1).
    layer_uv: array<vec4<f32>, 8>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var<uniform> draw: Draw;
@group(1) @binding(1) var<storage, read> palette: array<mat4x4<f32>>;
@group(2) @binding(0) var<uniform> material: Material;
@group(2) @binding(1) var base_map: texture_2d<f32>;
@group(2) @binding(2) var base_sampler: sampler;
@group(2) @binding(3) var detail_map: texture_2d<f32>;
@group(2) @binding(4) var detail_sampler: sampler;
@group(2) @binding(5) var specular_map: texture_2d<f32>;
@group(2) @binding(6) var normal_map: texture_2d<f32>;
@group(2) @binding(7) var layer0: texture_2d<f32>;
@group(2) @binding(8) var layer1: texture_2d<f32>;
@group(2) @binding(9) var layer2: texture_2d<f32>;
@group(2) @binding(10) var layer3: texture_2d<f32>;

struct Varyings {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    // xyz: world-space tangent (0: none), w: handedness
    @location(4) tangent: vec4<f32>,
    // Position before the model transform (ground layer coordinates).
    @location(5) local_pos: vec3<f32>,
};
"#;

const FRAGMENT: &str = r#"
fn layer_uv(i: u32, p: vec4<f32>) -> vec2<f32> {
    return vec2<f32>(dot(material.layer_uv[i * 2u], p), dot(material.layer_uv[i * 2u + 1u], p));
}

@fragment
fn fs_main(v: Varyings, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var base = textureSample(base_map, base_sampler, v.uv) * material.color;
    let layers = material.detail.w;
    if material.params.z > 0.5 && layers < 0.5 {
        base = base * v.color;
    }
    if base.a < material.params.x {
        discard;
    }
    var rgb = base.rgb;
    if layers > 0.5 {
        // Ground: tiled layers weighted by the vertex colour, tinted by the
        // whole-cell map against its own average.
        let p = vec4<f32>(v.local_pos, 1.0);
        let w = v.color;
        let t0 = textureSample(layer0, detail_sampler, layer_uv(0u, p)).rgb;
        let t1 = textureSample(layer1, detail_sampler, layer_uv(1u, p)).rgb;
        let t2 = textureSample(layer2, detail_sampler, layer_uv(2u, p)).rgb;
        let t3 = textureSample(layer3, detail_sampler, layer_uv(3u, p)).rgb;
        var weights = vec4<f32>(w.x, select(0.0, w.y, layers > 1.5), select(0.0, w.z, layers > 2.5), select(0.0, w.w, layers > 3.5));
        let total = weights.x + weights.y + weights.z + weights.w;
        if total < 0.001 {
            weights = vec4<f32>(1.0, 0.0, 0.0, 0.0);
        } else {
            weights = weights / total;
        }
        let tiled = t0 * weights.x + t1 * weights.y + t2 * weights.z + t3 * weights.w;
        rgb = rgb * tiled * 2.0;
    }
    if material.detail.x > 0.0 {
        // Each texel against the map's average (its smallest mip), so the
        // base colour keeps its brightness.
        let uv = v.world_pos.xz * material.detail.x;
        let texel = textureSample(detail_map, detail_sampler, uv).rgb;
        let mean = textureSampleLevel(detail_map, detail_sampler, uv, 16.0).rgb;
        let fade = clamp(1.0 - distance(v.world_pos, frame.eye.xyz) / material.detail.y, 0.0, 1.0);
        rgb = rgb * mix(vec3<f32>(1.0), texel / max(mean, vec3<f32>(0.05)), fade);
    }
    let spec_texel = textureSample(specular_map, base_sampler, v.uv).rgb;
    let normal_texel = textureSample(normal_map, base_sampler, v.uv);
    if material.params.y > 0.5 {
        var n = normalize(v.normal);
        if !front {
            n = -n;
        }
        if material.maps.y > 0.5 && dot(v.tangent.xyz, v.tangent.xyz) > 1e-6 {
            let t = normalize(v.tangent.xyz - n * dot(n, v.tangent.xyz));
            let b = cross(n, t) * select(1.0, sign(v.tangent.w), abs(v.tangent.w) > 0.5);
            var xy = normal_texel.xy;
            if material.maps.z > 0.5 {
                xy = normal_texel.wy;
            }
            xy = xy * 2.0 - 1.0;
            let z = sqrt(max(1.0 - dot(xy, xy), 0.0));
            n = normalize(t * xy.x + b * xy.y + n * z);
        }
        let l = -normalize(frame.sun_dir.xyz);
        let diffuse = max(dot(n, l), 0.0);
        var lit = rgb * (frame.ambient.rgb + frame.sun_color.rgb * diffuse);
        if dot(material.specular.rgb, material.specular.rgb) > 0.0 && diffuse > 0.0 {
            let view = normalize(frame.eye.xyz - v.world_pos);
            let h = normalize(l + view);
            var strength = material.specular.rgb;
            if material.maps.x > 0.5 {
                strength = strength * spec_texel;
            }
            lit = lit + frame.sun_color.rgb * strength * pow(max(dot(n, h), 0.0), material.specular.a);
        }
        rgb = lit;
    }
    if material.params.w > 0.5 && frame.sky.a > 0.0 {
        let dist = distance(v.world_pos, frame.eye.xyz);
        let f = exp(-frame.sky.a * dist);
        rgb = mix(frame.sky.rgb, rgb, f);
    }
    var alpha = base.a;
    if material.detail.z > 0.5 {
        let facing = abs(dot(normalize(v.normal), normalize(frame.eye.xyz - v.world_pos)));
        alpha = alpha * facing * facing;
    }
    return vec4<f32>(rgb, alpha);
}
"#;

fn wgsl_type(format: VertexFormat) -> &'static str {
    match format {
        VertexFormat::Float1 => "f32",
        VertexFormat::Float2 | VertexFormat::Half2 => "vec2<f32>",
        VertexFormat::Float3 => "vec3<f32>",
        VertexFormat::Float4
        | VertexFormat::Half4
        | VertexFormat::UNorm4x8
        | VertexFormat::Bgra8 => "vec4<f32>",
        VertexFormat::UInt4x8 => "vec4<u32>",
    }
}

/// `name` widened to a `vec4<f32>`, missing components 0 (w 1).
fn as_vec4(name: &str, format: VertexFormat) -> String {
    match format {
        VertexFormat::Float1 => format!("vec4<f32>({name}, 0.0, 0.0, 1.0)"),
        VertexFormat::Float2 | VertexFormat::Half2 => format!("vec4<f32>({name}, 0.0, 1.0)"),
        VertexFormat::Float3 => format!("vec4<f32>({name}, 1.0)"),
        VertexFormat::Float4 | VertexFormat::Half4 | VertexFormat::UNorm4x8 => name.to_string(),
        // Direct3D 9 colours are stored B, G, R, A.
        VertexFormat::Bgra8 => format!("{name}.zyxw"),
        VertexFormat::UInt4x8 => format!("vec4<f32>({name})"),
    }
}

/// The full WGSL module (vertex and fragment stages) for `layout`.
pub fn source(layout: &VertexLayout) -> String {
    let mut s = String::from(COMMON);
    s.push_str("\nstruct VertexIn {\n");
    for (i, e) in layout.elements().iter().enumerate() {
        let _ = writeln!(s, "    @location({i}) a{i}: {},", wgsl_type(e.format));
    }
    s.push_str("};\n");

    let find = |semantic: VertexSemantic| {
        layout
            .elements()
            .iter()
            .position(|e| e.semantic == semantic)
            .map(|i| as_vec4(&format!("v.a{i}"), layout.elements()[i].format))
    };
    // A position's w is a sub-object index in some models, never a
    // homogeneous coordinate, so it is dropped.
    let position = find(VertexSemantic::Position)
        .map(|p| format!("{p}.xyz"))
        .unwrap_or_else(|| "vec3<f32>(0.0)".into());
    let normal = find(VertexSemantic::Normal)
        .map(|n| format!("{n}.xyz"))
        .unwrap_or_else(|| "vec3<f32>(0.0, 1.0, 0.0)".into());
    let uv = find(VertexSemantic::TexCoord(0))
        .map(|t| format!("{t}.xy"))
        .unwrap_or_else(|| "vec2<f32>(0.0)".into());
    let color = find(VertexSemantic::Color).unwrap_or_else(|| "vec4<f32>(1.0)".into());
    let tangent = find(VertexSemantic::Tangent).unwrap_or_else(|| "vec4<f32>(0.0)".into());
    // Skinned layouts blend up to four palette matrices, weights normalised
    // by their sum; a zero sum leaves the vertex in its bind pose.
    let indices = layout.elements().iter().position(|e| {
        e.semantic == VertexSemantic::BlendIndices && e.format == VertexFormat::UInt4x8
    });
    let skin = match (indices, find(VertexSemantic::BlendWeight)) {
        (Some(i), Some(weights)) => format!(
            r#"
    if draw.skin.y > 0u {{
        let w = {weights};
        let total = w.x + w.y + w.z + w.w;
        if total > 0.0 {{
            let last = vec4<u32>(draw.skin.y - 1u);
            let b = min(v.a{i}, last) + vec4<u32>(draw.skin.x);
            let m = palette[b.x] * (w.x / total) + palette[b.y] * (w.y / total)
                + palette[b.z] * (w.z / total) + palette[b.w] * (w.w / total);
            local = m * local;
            normal = (m * vec4<f32>(normal, 0.0)).xyz;
            tangent = vec4<f32>((m * vec4<f32>(tangent.xyz, 0.0)).xyz, tangent.w);
        }}
    }}"#
        ),
        _ => String::new(),
    };

    let _ = write!(
        s,
        r#"
@vertex
fn vs_main(v: VertexIn, @builtin(instance_index) instance: u32) -> Varyings {{
    var local = vec4<f32>({position}, 1.0);
    var normal = {normal};
    var tangent = {tangent};
    let rest = local.xyz;{skin}
    var model = draw.world;
    if draw.skin.w > 0u {{
        model = model * palette[draw.skin.z + min(instance, draw.skin.w - 1u)];
    }}
    let world = model * local;
    var out: Varyings;
    out.clip = frame.view_proj * world;
    out.world_pos = world.xyz;
    out.normal = (model * vec4<f32>(normal, 0.0)).xyz;
    out.uv = {uv};
    out.color = {color};
    out.tangent = vec4<f32>((model * vec4<f32>(tangent.xyz, 0.0)).xyz, tangent.w);
    out.local_pos = rest;
    return out;
}}
"#
    );
    s.push_str(FRAGMENT);
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use VertexFormat::*;
    use VertexSemantic::*;

    fn validate(src: &str) {
        let module = naga::front::wgsl::parse_str(src)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{e:?}\n{src}"));
    }

    #[test]
    fn layouts_compile() {
        let layouts = [
            vec![(Position, Float3)],
            vec![(Position, Float3), (Normal, Float3), (TexCoord(0), Float2)],
            vec![
                (Position, Half4),
                (Normal, UNorm4x8),
                (Color, Bgra8),
                (TexCoord(0), Half2),
                (BlendWeight, UNorm4x8),
                (BlendIndices, UInt4x8),
            ],
            vec![
                (Position, Float4),
                (TexCoord(0), Float2),
                (TexCoord(1), Float1),
            ],
            // Terrain: colour as layer weights, tangents for laid-in roads.
            vec![
                (Position, Float3),
                (Normal, Float3),
                (TexCoord(0), Float2),
                (Color, UNorm4x8),
                (Tangent, Float4),
            ],
            // Skinned with tangents.
            vec![
                (Position, Half4),
                (TexCoord(0), Half2),
                (BlendIndices, UInt4x8),
                (BlendWeight, UNorm4x8),
                (Normal, Half4),
                (Tangent, Half4),
            ],
        ];
        for attrs in layouts {
            validate(&source(&VertexLayout::packed(&attrs).unwrap()));
        }
    }

    #[test]
    fn colour_is_swizzled_from_bgra() {
        let layout = VertexLayout::packed(&[(Position, Float3), (Color, Bgra8)]).unwrap();
        assert!(source(&layout).contains("out.color = v.a1.zyxw;"));
        let layout = VertexLayout::packed(&[(Position, Float3), (Color, UNorm4x8)]).unwrap();
        assert!(source(&layout).contains("out.color = v.a1;"));
    }

    #[test]
    fn skinned_layouts_blend_the_palette() {
        let skinned = VertexLayout::packed(&[
            (Position, Half4),
            (BlendIndices, UInt4x8),
            (BlendWeight, UNorm4x8),
        ])
        .unwrap();
        let src = source(&skinned);
        validate(&src);
        assert!(src.contains("palette[b.x]"));
        let rigid = VertexLayout::packed(&[(Position, Half4), (BlendIndices, UInt4x8)]).unwrap();
        assert!(!source(&rigid).contains("palette[b.x]"));
    }
}
