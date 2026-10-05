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
    // rgb: sunlight, a: 1 to sum light in display values
    // (`light_to_display`)
    sun_color: vec4<f32>,
    // rgb: sky and fog colour, a: fog density per metre
    sky: vec4<f32>,
    // Ambient light from +x, -x, +y, -y, +z, -z
    ambient_cube: array<vec4<f32>, 6>,
    // rgb: rim light colour, a: how far a surface turns away before it shows
    rim: vec4<f32>,
    // xyz: the way the camera looks, w: seconds (animation)
    view_dir: vec4<f32>,
    // World to each sun shadow cascade's clip space
    shadow: array<mat4x4<f32>, 4>,
    // View depth where each cascade ends (all 0: no shadows)
    shadow_splits: vec4<f32>,
    // Metres per shadow texel in each cascade
    shadow_texels: vec4<f32>,
    // Clip space back to the world
    inv_view_proj: mat4x4<f32>,
    // Haze: rgb Rayleigh scattering per metre (0: plain fog), a: the Mie
    // phase function's g
    rayleigh: vec4<f32>,
    // rgb: Mie scattering per metre, a: in-scattering multiplier
    mie: vec4<f32>,
    // x: extinction multiplier, y: Rayleigh scale height, z: Mie scale
    // height (metres), w: density of the air near the ground
    scatter: vec4<f32>,
    // Clouds: xyz each layer's scale, w coverage
    clouds: vec4<f32>,
    // xy: wind (x, z), z: clouds on, w: how fast they darken with mass
    // (lit as the game's), negative for the earlier lighting
    wind: vec4<f32>,
    // The water's colour (rgb) and opacity (a) through none, a third, two
    // thirds and all of water_depth.y metres of it
    water_ramp: array<vec4<f32>, 4>,
    // x: the ramp is set, y: metres at its end
    water_depth: vec4<f32>,
    // rgb: scale on the clouds' colour
    cloud_tone: vec4<f32>,
};

// The game lights surfaces in display values: the sun's colour times
// N.L and shadow, plus the ambient light, summed there. With
// frame.sun_color.a set, light is encoded to display values to be summed
// and the sum decoded; otherwise both are as they are.
fn light_to_display(c: vec3<f32>) -> vec3<f32> {
    if frame.sun_color.a < 0.5 {
        return c;
    }
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
}

fn light_from_display(c: vec3<f32>) -> vec3<f32> {
    if frame.sun_color.a < 0.5 {
        return c;
    }
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
}

// The ambient cube's light for normal `n`, its faces blended as display
// values when light is summed in them.
fn ambient_display(n: vec3<f32>) -> vec3<f32> {
    let n2 = n * n;
    return n2.x * light_to_display(select(frame.ambient_cube[1], frame.ambient_cube[0], n.x >= 0.0).rgb)
        + n2.y * light_to_display(select(frame.ambient_cube[3], frame.ambient_cube[2], n.y >= 0.0).rgb)
        + n2.z * light_to_display(select(frame.ambient_cube[5], frame.ambient_cube[4], n.z >= 0.0).rgb);
}

// The depth colour map at `metres` of water looked through.
fn water_ramp(metres: f32) -> vec4<f32> {
    let u = clamp(metres / frame.water_depth.y, 0.0, 1.0) * 3.0;
    let i = min(u32(u), 2u);
    return mix(frame.water_ramp[i], frame.water_ramp[i + 1u], u - f32(i));
}

struct Draw {
    world: mat4x4<f32>,
    // x: first palette matrix, y: palette length (0: not skinned),
    // z: first instance matrix, w: instance count (0: not instanced)
    skin: vec4<u32>,
    // xy: added to the texture coordinates (scrolling textures)
    uv: vec4<f32>,
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
    // x: specular map, y: normal map, z: normal x in alpha, w: ground
    maps: vec4<f32>,
    // rgb: direct light tint, w: unused
    direct: vec4<f32>,
    // rgb: ambient light tint, w: unused
    ambient: vec4<f32>,
    // Ground: x, y: distance the colour map starts taking over and 1 / the
    // distance it takes; z, w: the same for the detail map fading out.
    ground: vec4<f32>,
    // Per ground layer, u then v as rows over world-space (x, y, z, 1).
    layer_uv: array<vec4<f32>, 8>,
    // rgb: emitted colour (0: none), w: scaled by the specular map's texel
    emissive: vec4<f32>,
    // x: reflection strength (0: none), y: Fresnel reflectance facing the
    // eye (negative: no Fresnel), z: takes the rim light
    reflection: vec4<f32>,
    // x: open water
    water: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var shadow_map: texture_depth_2d_array;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
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
// Depth of what was drawn before the water (reversed: 0 is nothing).
@group(3) @binding(0) var scene_depth: texture_depth_2d;

struct Varyings {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    // xyz: world-space tangent (0: none), w: handedness
    @location(4) tangent: vec4<f32>,
    // The air between eye and vertex, as the game computes it per vertex:
    // rgb light it adds, a: fraction of the surface's light it keeps
    @location(5) haze: vec4<f32>,
};
"#;

const FRAGMENT: &str = r#"
// The game's ambient cube: each axis's face, picked by the normal's sign,
// weighted by that component squared.
fn ambient_light(n: vec3<f32>) -> vec3<f32> {
    let n2 = n * n;
    return n2.x * select(frame.ambient_cube[1], frame.ambient_cube[0], n.x >= 0.0).rgb
        + n2.y * select(frame.ambient_cube[3], frame.ambient_cube[2], n.y >= 0.0).rgb
        + n2.z * select(frame.ambient_cube[5], frame.ambient_cube[4], n.z >= 0.0).rgb;
}

fn hazy() -> bool {
    return dot(frame.rayleigh.rgb, vec3<f32>(1.0)) > 0.0;
}

// Sunlight scattered towards the eye per unit of scattering, for a ray
// whose angle to the sun has cosine c, weighted by how much of the
// scattering along it is Rayleigh (tr) and Mie (tm): Hoffman and
// Preetham's in-scattering, as the game's vertex shaders compute it.
fn inscatter(c: f32, tr: vec3<f32>, tm: vec3<f32>) -> vec3<f32> {
    let g = frame.rayleigh.a;
    let rayleigh = 0.0596831 * (1.0 + c * c);
    let mie = 0.0795775 * (1.0 - g) * (1.0 - g) / pow(max(1.0 + g * g - 2.0 * g * c, 1e-4), 1.5);
    let t = max(tr + tm, vec3<f32>(1e-9));
    return (tr * rayleigh + tm * mie) / t * frame.sun_color.rgb * frame.mie.a;
}

// The sky along the unit ray r: the haze's in-scattering through the air
// above, the path longer towards the horizon. Without haze settings, the
// flat sky colour.
fn sky(r: vec3<f32>) -> vec3<f32> {
    if !hazy() {
        return frame.sky.rgb;
    }
    // Flat-earth path through each layer, kept finite at the horizon.
    let mu = max(r.y, 0.0) + 0.035;
    let tr = frame.rayleigh.rgb * frame.scatter.y / mu;
    let tm = frame.mie.rgb * frame.scatter.z / mu;
    let c = dot(r, -normalize(frame.sun_dir.xyz));
    return inscatter(c, tr, tm) * (1.0 - exp(-(tr + tm)));
}

// The air between the eye and a height `h` metres above it along r: the
// light it adds (rgb) and the share it keeps of what lies there (a), as
// `sky` but only through the layers below h.
fn sky_below(r: vec3<f32>, h: f32) -> vec4<f32> {
    let mu = max(r.y, 0.0) + 0.035;
    let tr = frame.rayleigh.rgb * frame.scatter.y * (1.0 - exp(-h / frame.scatter.y)) / mu;
    let tm = frame.mie.rgb * frame.scatter.z * (1.0 - exp(-h / frame.scatter.z)) / mu;
    let c = dot(r, -normalize(frame.sun_dir.xyz));
    let keep = exp(-(tr + tm));
    return vec4<f32>(inscatter(c, tr, tm) * (1.0 - keep), dot(keep, vec3<f32>(1.0 / 3.0)));
}

// The surface colour rgb at p seen through the air from the eye: the
// game's extinction and in-scattering, or plain exponential fog.
fn haze(rgb: vec3<f32>, p: vec3<f32>) -> vec3<f32> {
    let h = haze_terms(p);
    return rgb * h.a + h.rgb;
}

// What the air between the eye and p adds (rgb) and keeps (a).
fn haze_terms(p: vec3<f32>) -> vec4<f32> {
    let d = distance(p, frame.eye.xyz);
    if !hazy() {
        if frame.sky.a <= 0.0 {
            return vec4<f32>(0.0, 0.0, 0.0, 1.0);
        }
        let f = exp(-frame.sky.a * d);
        return vec4<f32>(frame.sky.rgb * (1.0 - f), f);
    }
    // What the air takes away over d, and what it adds: the light the sky
    // shows along the same line (looking down, the sky at the horizon),
    // so distant land fades into the sky behind it.
    let e = exp(-(frame.rayleigh.rgb + frame.mie.rgb) * d * frame.scatter.x * frame.scatter.w);
    let r = (p - frame.eye.xyz) / max(d, 1e-3);
    let keep = dot(e, vec3<f32>(0.3333));
    return vec4<f32>(sky(normalize(vec3<f32>(r.x, max(r.y, 0.0), r.z))) * (1.0 - e), keep);
}

// What a reflection sees along r, standing in for the game's environment
// cube: the sky above, the ground's light below.
fn environment(r: vec3<f32>) -> vec3<f32> {
    let horizon = select(mix(frame.sky.rgb, vec3<f32>(1.0), 0.35), sky(vec3<f32>(r.x, 0.0, r.z)), hazy());
    if r.y >= 0.0 {
        if hazy() {
            return sky(r);
        }
        return mix(horizon, frame.sky.rgb, sqrt(r.y));
    }
    return mix(horizon * 0.6, frame.ambient_cube[3].rgb, sqrt(-r.y));
}

// How much sunlight reaches p (0 to 1), from the shadow cascade its view
// depth falls in, sampled 3 x 3 around it and offset along the surface
// normal n so a surface doesn't shadow itself.
fn sunlight(p: vec3<f32>, n: vec3<f32>) -> f32 {
    let last = frame.shadow_splits.w;
    if last <= 0.0 {
        return 1.0;
    }
    let depth = dot(p - frame.eye.xyz, frame.view_dir.xyz);
    var i = 0u;
    while i < 4u && depth >= frame.shadow_splits[i] {
        i = i + 1u;
    }
    if i >= 4u {
        return 1.0;
    }
    let c = frame.shadow[i] * vec4<f32>(p + n * frame.shadow_texels[i] * 1.5, 1.0);
    let uv = vec2<f32>(c.x * 0.5 + 0.5, 0.5 - c.y * 0.5);
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || c.z > 1.0 {
        return 1.0;
    }
    let texel = 1.0 / f32(textureDimensions(shadow_map).x);
    var lit = 0.0;
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let o = vec2<f32>(f32(dx), f32(dy)) * texel;
            lit = lit + textureSampleCompareLevel(shadow_map, shadow_sampler, uv + o, i, c.z);
        }
    }
    // Fades out over the last tenth of the distance.
    let fade = clamp((last - depth) / (last * 0.1), 0.0, 1.0);
    return mix(1.0, lit / 9.0, fade);
}

// The sea's waves: direction (radians), wavelength and amplitude (metres).
// The game's water shader sums eight such waves (two sets of four) whose
// constants its code sets at run time; these are invented to look calm,
// spread around a wind from one side so no crest runs straight for long.
var<private> WAVES: array<vec3<f32>, 8> = array<vec3<f32>, 8>(
    vec3<f32>(0.3, 31.0, 0.3),
    vec3<f32>(1.15, 23.0, 0.2),
    vec3<f32>(-0.55, 13.0, 0.11),
    vec3<f32>(0.75, 8.7, 0.06),
    vec3<f32>(-0.15, 5.3, 0.035),
    vec3<f32>(1.7, 3.1, 0.022),
    vec3<f32>(-1.2, 1.9, 0.013),
    vec3<f32>(0.45, 1.1, 0.007),
);

// The sea's normal at p (world x, z) after t seconds. Waves too short for
// a pixel `footprint` metres wide fade out, as the game's do, well before
// they would alias into stripes. Each wave's height swells and falls over
// a few hundred metres so the pattern doesn't repeat evenly.
fn sea_normal(p: vec2<f32>, t: f32, footprint: f32) -> vec3<f32> {
    var slope = vec2<f32>(0.0);
    for (var i = 0u; i < 8u; i = i + 1u) {
        let w = WAVES[i];
        let dir = vec2<f32>(cos(w.x), sin(w.x));
        let k = 6.2831853 / w.y;
        // Deep water: speed follows wavelength.
        let theta = k * dot(dir, p) - sqrt(9.81 * k) * t + f32(i) * 1.7;
        let fade = clamp(2.0 - 8.0 * footprint / w.y, 0.0, 1.0);
        let across = vec2<f32>(-dir.y, dir.x);
        let swell = 0.65 + 0.35 * sin(dot(p, across) * (0.021 + 0.004 * f32(i)) + f32(i) * 2.3)
            * sin(dot(p, dir) * 0.013 - t * 0.05 + f32(i));
        slope = slope + dir * (w.z * k * fade * swell * cos(theta));
    }
    return normalize(vec3<f32>(-slope.x, 1.0, -slope.y));
}

fn layer_uv(i: u32, p: vec4<f32>) -> vec2<f32> {
    return vec2<f32>(dot(material.layer_uv[i * 2u], p), dot(material.layer_uv[i * 2u + 1u], p));
}

@fragment
fn fs_main(v: Varyings, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let ground = material.maps.w > 0.5;
    let water = material.water.x > 0.5;
    // Water's map is its foam, drifting over world space.
    let drift = vec2<f32>(0.021, 0.013) * frame.view_dir.w;
    let base_uv = select(v.uv, v.world_pos.xz * 0.15 + drift, water);
    let base_texel = textureSample(base_map, base_sampler, base_uv);
    var base = select(base_texel, vec4<f32>(1.0), water) * material.color;
    let ao = material.params.z > 0.5 && !ground;
    if ao {
        base.a = base.a * v.color.a;
    }
    if base.a < material.params.x {
        discard;
    }
    var rgb = base.rgb;
    let dist = distance(v.world_pos, frame.eye.xyz);
    let footprint = length(fwidth(v.world_pos.xz));
    var water_alpha = 1.0;
    // Sampled outside branches: derivatives need uniform control flow.
    let p = vec4<f32>(v.world_pos, 1.0);
    let t0 = textureSample(layer0, detail_sampler, layer_uv(0u, p)).rgb;
    let t1 = textureSample(layer1, detail_sampler, layer_uv(1u, p)).rgb;
    let t2 = textureSample(layer2, detail_sampler, layer_uv(2u, p)).rgb;
    let t3 = textureSample(layer3, detail_sampler, layer_uv(3u, p)).rgb;
    let detail_uv = v.world_pos.xz * material.detail.x;
    let detail_texel = textureSample(detail_map, detail_sampler, detail_uv).rgb;
    if ground {
        // As the game's terrain shaders: the vertex colour weights the
        // tiled layers (r the first), the colour map makes up any shortfall
        // and takes over with distance, and the detail map adds signed
        // grain up close.
        let layers = material.detail.w;
        let c = v.color * vec4<f32>(1.0, select(0.0, 1.0, layers > 1.5), select(0.0, 1.0, layers > 2.5), select(0.0, 1.0, layers > 3.5));
        let s = select(0.0, c.x + c.y + c.z + c.w, layers > 0.5);
        let w_global = clamp(max((dist - material.ground.x) * material.ground.y, 1.0 - s), 0.0, 1.0);
        let w = c * clamp((1.0 - w_global) / (s + 1e-5), 0.0, 1.0);
        rgb = base.rgb * w_global + t0 * w.x + t1 * w.y + t2 * w.z + t3 * w.w;
        if material.detail.x > 0.0 {
            let w_detail = clamp(1.0 - (dist - material.ground.z) * material.ground.w, 0.0, 1.0);
            rgb = max(rgb + 2.0 * (detail_texel - 0.5) * w_detail, vec3<f32>(0.0));
        }
    } else if material.detail.x > 0.0 {
        // Each texel against the map's average (its smallest mip), so the
        // base colour keeps its brightness.
        let mean = textureSampleLevel(detail_map, detail_sampler, detail_uv, 16.0).rgb;
        let fade = clamp(1.0 - dist / material.detail.y, 0.0, 1.0);
        rgb = rgb * mix(vec3<f32>(1.0), detail_texel / max(mean, vec3<f32>(0.05)), fade);
    }
    let spec_texel = textureSample(specular_map, base_sampler, v.uv).rgb;
    let normal_texel = textureSample(normal_map, base_sampler, v.uv);
    // Highlight and reflection, added after any ambient occlusion.
    var shine = vec3<f32>(0.0);
    var spec_light = vec3<f32>(0.0);
    if water {
        // As the game's water: the body colour lit by sun and sky, the sky
        // mirrored over it by Schlick's Fresnel (water reflects 2% facing
        // the eye), and a tight sun glint.
        let n = sea_normal(v.world_pos.xz, frame.view_dir.w, footprint);
        let view = normalize(frame.eye.xyz - v.world_pos);
        let l = -normalize(frame.sun_dir.xyz);
        let shade = sunlight(v.world_pos, vec3<f32>(0.0, 1.0, 0.0));
        // What lies beneath: the seabed's point from the depth drawn
        // before the water. Its depth below the surface sets the body
        // colour (the game looks it up in a depth colour map); the length
        // of water the eye looks through sets how much of the bed shows.
        var depth = 1000.0;
        var through = 1000.0;
        let below = textureLoad(scene_depth, vec2<i32>(v.clip.xy), 0);
        if below > 0.0 {
            let size = vec2<f32>(textureDimensions(scene_depth));
            let ndc = vec2<f32>(v.clip.x / size.x * 2.0 - 1.0, 1.0 - v.clip.y / size.y * 2.0);
            let q = frame.inv_view_proj * vec4<f32>(ndc, below, 1.0);
            let bed = q.xyz / q.w;
            depth = max(v.world_pos.y - bed.y, 0.0);
            through = distance(v.world_pos, bed);
        }
        let lit = frame.sun_color.rgb * max(dot(n, l), 0.0) * shade;
        let shallow = vec3<f32>(0.09, 0.30, 0.34);
        var body = mix(shallow, base.rgb, 1.0 - exp(-depth / 5.0)) * (lit * 0.3 + ambient_light(n));
        var opacity = 1.0 - exp(-through * 0.2);
        if frame.water_depth.x > 0.5 {
            // The atmosphere's depth colour map, by the water looked
            // through, lit as the game's: sun and ambient alike.
            let ramp = water_ramp(through);
            body = ramp.rgb * (lit + ambient_light(n));
            opacity = ramp.a;
        }
        let f = 0.02 + 0.98 * pow(1.0 - max(dot(n, view), 0.0), 5.0);
        let glint = frame.sun_color.rgb * pow(max(dot(n, normalize(l + view)), 0.0), 400.0) * 3.0 * shade;
        rgb = mix(body, environment(reflect(-view, n)), f) + glint;
        water_alpha = mix(opacity, 1.0, f);
        // Foam along the shore, within about 3 m of it as the game's.
        let pattern = base_texel.a * dot(base_texel.rgb, vec3<f32>(0.3333));
        // Past a hundred metres or so surf is too fine to see; drawn there
        // it would outline every shore.
        let near = 1.0 - smoothstep(40.0, 150.0, dist);
        let foam = (1.0 - smoothstep(0.0, 3.0, depth)) * pattern * clamp(through * 2.0, 0.0, 1.0) * near;
        rgb = mix(rgb, lit + ambient_light(vec3<f32>(0.0, 1.0, 0.0)), foam);
        water_alpha = max(water_alpha, foam);
    } else if material.params.y > 0.5 {
        var n = v.normal;
        if !front {
            n = -n;
        }
        if material.maps.y > 0.5 && dot(v.tangent.xyz, v.tangent.xyz) > 1e-6 {
            // As the game decodes them: x in alpha, y in green, and z from
            // 1 - x² - 2y (linear in y, as its shaders have it).
            let t = v.tangent.xyz;
            let b = cross(n, t) * select(1.0, sign(v.tangent.w), abs(v.tangent.w) > 0.5);
            var xy = normal_texel.xy;
            if material.maps.z > 0.5 {
                xy = normal_texel.wy;
            }
            xy = xy * 2.0 - 1.0;
            let z = sqrt(abs(1.0 - xy.x * xy.x - 2.0 * xy.y));
            n = t * xy.x + b * xy.y + n * z;
        }
        n = normalize(n);
        let l = -normalize(frame.sun_dir.xyz);
        let n_dot_l = dot(n, l);
        let view = normalize(frame.eye.xyz - v.world_pos);
        var surface_n = normalize(v.normal + vec3<f32>(0.0, 1e-6, 0.0));
        if !front {
            surface_n = -surface_n;
        }
        let shade = sunlight(v.world_pos, surface_n);
        var lit: vec3<f32>;
        if ground {
            let sun = light_to_display(frame.sun_color.rgb) * max(n_dot_l, 0.0) * shade;
            lit = rgb * light_from_display(sun + ambient_display(n));
        } else {
            var sun = light_to_display(frame.sun_color.rgb) * max(n_dot_l, 0.0) * shade;
            if material.reflection.z > 0.5 {
                // Rim light where the surface turns away from the eye, on
                // the side the sun doesn't reach.
                sun = sun + light_to_display(frame.rim.rgb) * max(frame.rim.a - dot(n, view), 0.0) * (1.0 - max(n_dot_l, 0.0));
            }
            let direct = clamp(sun, vec3<f32>(0.0), vec3<f32>(1.0));
            lit = rgb * light_from_display(direct * material.direct.rgb + ambient_display(n) * material.ambient.rgb);
        }
        var tint = vec3<f32>(1.0);
        if material.maps.x > 0.5 {
            tint = spec_texel;
        }
        if dot(material.specular.rgb, material.specular.rgb) > 0.0 && n_dot_l > 0.0 {
            // Blinn-Phong, never shadowed below a quarter.
            let h = normalize(l + view);
            let n_dot_h = dot(n, h);
            if n_dot_h > 0.0 {
                spec_light = frame.sun_color.rgb * material.specular.rgb * tint * pow(n_dot_h, material.specular.a) * max(shade, 0.25);
            }
        }
        shine = spec_light;
        if material.reflection.x > 0.0 {
            // The view mirrored in the surface, through the specular map;
            // metal weights it by Schlick's Fresnel.
            var strength = material.reflection.x;
            if material.reflection.y >= 0.0 {
                let f0 = material.reflection.y;
                strength = strength * (f0 + (1.0 - f0) * pow(1.0 - max(dot(n, view), 0.0), 5.0));
            }
            shine = shine + environment(reflect(-view, n)) * tint * strength;
        }
        rgb = lit;
    }
    if ao {
        rgb = rgb * v.color.rgb;
    }
    rgb = rgb + shine;
    // Light the surface gives off, on top of what it reflects.
    var glow = material.emissive.rgb;
    if material.emissive.w > 0.5 {
        glow = glow * spec_texel;
    }
    rgb = rgb + glow;
    if material.params.w > 0.5 {
        // Per vertex as the game's, except over the sea's few huge
        // triangles.
        if water {
            rgb = haze(rgb, v.world_pos);
        } else {
            rgb = rgb * v.haze.a + v.haze.rgb;
        }
    }
    // Bright highlights make glass more opaque, as the game's shaders have it.
    var alpha = clamp(max(base.a, dot(spec_light, vec3<f32>(0.299, 0.587, 0.114))), 0.0, 1.0);
    if water {
        alpha = water_alpha;
    }
    if material.detail.z > 0.5 {
        let facing = abs(dot(normalize(v.normal), normalize(frame.eye.xyz - v.world_pos)));
        alpha = alpha * facing * facing;
    }
    return vec4<f32>(rgb, alpha);
}

// Shadow casters with cut-out textures (leaves, fences) cut their shadow
// out as they cut themselves.
@fragment
fn fs_shadow(v: Varyings) {
    var a = textureSample(base_map, base_sampler, v.uv).a * material.color.a;
    if material.params.z > 0.5 && material.maps.w < 0.5 {
        a = a * v.color.a;
    }
    if a < material.params.x {
        discard;
    }
}
"#;

/// The sky behind the world: a triangle over the screen at the far plane.
const SKY: &str = r#"
struct SkyVaryings {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) i: u32) -> SkyVaryings {
    let ndc = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var out: SkyVaryings;
    // Depth 0 is the far plane (reversed depth).
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.ndc = ndc;
    return out;
}

@group(1) @binding(2) var cloud_noise: texture_2d<f32>;
@group(1) @binding(3) var cloud_sampler: sampler;

// Height of the cloud deck, the span one repeat of the noise covers at
// layer scale 1, and the wind's speed (metres, m/s). Invented: the game's
// come from code. A layer's scale stretches its noise, so the scale-8
// layer gives the broad masses and the others break them up.
const CLOUD_HEIGHT: f32 = 1500.0;
const CLOUD_SPAN: f32 = 6000.0;
const CLOUD_SPEED: f32 = 6.0;

@fragment
fn fs_sky(v: SkyVaryings) -> @location(0) vec4<f32> {
    // A point on the near plane, then the ray from the eye through it.
    let p = frame.inv_view_proj * vec4<f32>(v.ndc, 1.0, 1.0);
    let r = normalize(p.xyz / p.w - frame.eye.xyz);
    var rgb = sky(r);
    // Where the ray meets the cloud deck, the deck moving with the eye so
    // clouds stay overhead; sampled before any branch for mip selection.
    let up = max(r.y, 0.03);
    let deck = frame.eye.xz + r.xz * (CLOUD_HEIGHT / up);
    let drift = frame.wind.xy * CLOUD_SPEED * frame.view_dir.w;
    let scale = max(frame.clouds.xyz, vec3<f32>(0.01)) * CLOUD_SPAN;
    // The texture holds four octaves of noise, coarsest in alpha, then
    // red, green and blue; the broad layer takes the coarsest.
    let n1 = textureSample(cloud_noise, cloud_sampler, (deck + drift) / scale.x).a;
    let n2 = textureSample(cloud_noise, cloud_sampler, (deck + drift * 1.3) / scale.y).g;
    let n3 = textureSample(cloud_noise, cloud_sampler, (deck + drift * 1.15) / scale.z).b;
    if frame.wind.z > 0.5 && r.y > 0.0 {
        // The broad layer shapes, the finer ones erode the edges; the
        // coverage sets how much of the sum clears a soft cut-off.
        let density = n1 * 0.6 + n3 * 0.25 + n2 * 0.15;
        let cut = 1.0 - frame.clouds.w;
        var alpha = smoothstep(cut - 0.1, cut + 0.35, density);
        let mass = alpha;
        // Far deck fades into the haze towards the horizon.
        alpha = alpha * smoothstep(0.02, 0.35, r.y);
        let l = -normalize(frame.sun_dir.xyz);
        let c = dot(r, l);
        var cloud: vec3<f32>;
        if frame.wind.w >= 0.0 {
            // As the game's: sunlight alone, through a Cornette-Shanks
            // phase (its g taken from the haze's; the cloud's own is not
            // known), divided by 1 + mass * darkness, the mass here the
            // cloud's opacity before it fades towards the horizon.
            let g = frame.rayleigh.a;
            let phase = 1.5 * (1.0 - g * g) / (2.0 + g * g) * (1.0 + c * c)
                / pow(1.0 + g * g - 2.0 * g * c, 1.5);
            cloud = frame.sun_color.rgb * (1.0 + phase) / (1.0 + mass * frame.wind.w);
        } else {
            // Lit by the sun, brighter looking towards it; thicker cloud
            // is darker underneath.
            let lit = frame.sun_color.rgb * (0.5 + 0.5 * pow(max(c, 0.0), 4.0)) * mix(1.0, 0.7, alpha);
            cloud = lit + frame.ambient_cube[2].rgb * 0.5;
        }
        cloud = cloud * frame.cloud_tone.rgb;
        if frame.wind.w >= 0.0 && hazy() {
            // As the game's, the air below the deck dims the cloud and
            // adds its own light in front of it.
            let air = sky_below(r, CLOUD_HEIGHT);
            cloud = cloud * air.a + air.rgb;
        }
        // Thin veils: the game's footage shows a hazy sky with little
        // solid cloud.
        rgb = mix(rgb, cloud, alpha * 0.6);
    }
    return vec4<f32>(rgb, 1.0);
}
"#;

/// The sky stage's source: the shared declarations and helpers plus its
/// own entry points.
pub fn sky_source() -> String {
    format!("{COMMON}{FRAGMENT}{SKY}")
}

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
fn place(v: VertexIn, instance: u32) -> Varyings {{
    var local = vec4<f32>({position}, 1.0);
    var normal = {normal};
    var tangent = {tangent};{skin}
    var model = draw.world;
    if draw.skin.w > 0u {{
        model = model * palette[draw.skin.z + min(instance, draw.skin.w - 1u)];
    }}
    let world = model * local;
    var out: Varyings;
    out.clip = frame.view_proj * world;
    out.world_pos = world.xyz;
    out.normal = (model * vec4<f32>(normal, 0.0)).xyz;
    out.uv = {uv} + draw.uv.xy;
    out.color = {color};
    out.tangent = vec4<f32>((model * vec4<f32>(tangent.xyz, 0.0)).xyz, tangent.w);
    out.haze = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    return out;
}}

@vertex
fn vs_main(v: VertexIn, @builtin(instance_index) instance: u32) -> Varyings {{
    var out = place(v, instance);
    out.haze = haze_terms(out.world_pos);
    return out;
}}

// Shadow casters need no haze.
@vertex
fn vs_shadow(v: VertexIn, @builtin(instance_index) instance: u32) -> Varyings {{
    return place(v, instance);
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
    fn sky_compiles() {
        validate(&sky_source());
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
