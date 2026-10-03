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
    // x: first palette matrix, y: palette length (0: not skinned)
    skin: vec4<u32>,
};

struct Material {
    color: vec4<f32>,
    // x: alpha cutoff, y: lit, z: vertex colour, w: fog
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var<uniform> draw: Draw;
@group(1) @binding(1) var<storage, read> palette: array<mat4x4<f32>>;
@group(2) @binding(0) var<uniform> material: Material;
@group(2) @binding(1) var base_map: texture_2d<f32>;
@group(2) @binding(2) var base_sampler: sampler;

struct Varyings {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
};
"#;

const FRAGMENT: &str = r#"
@fragment
fn fs_main(v: Varyings, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var base = textureSample(base_map, base_sampler, v.uv) * material.color;
    if material.params.z > 0.5 {
        base = base * v.color;
    }
    if base.a < material.params.x {
        discard;
    }
    var rgb = base.rgb;
    if material.params.y > 0.5 {
        var n = normalize(v.normal);
        if !front {
            n = -n;
        }
        let diffuse = max(dot(n, -normalize(frame.sun_dir.xyz)), 0.0);
        rgb = rgb * (frame.ambient.rgb + frame.sun_color.rgb * diffuse);
    }
    if material.params.w > 0.5 && frame.sky.a > 0.0 {
        let dist = distance(v.world_pos, frame.eye.xyz);
        let f = exp(-frame.sky.a * dist);
        rgb = mix(frame.sky.rgb, rgb, f);
    }
    return vec4<f32>(rgb, base.a);
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
        }}
    }}"#
        ),
        _ => String::new(),
    };

    let _ = write!(
        s,
        r#"
@vertex
fn vs_main(v: VertexIn) -> Varyings {{
    var local = vec4<f32>({position}, 1.0);
    var normal = {normal};{skin}
    let world = draw.world * local;
    var out: Varyings;
    out.clip = frame.view_proj * world;
    out.world_pos = world.xyz;
    out.normal = (draw.world * vec4<f32>(normal, 0.0)).xyz;
    out.uv = {uv};
    out.color = {color};
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
