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

use m2_math::glam::{Mat4, Vec3, Vec4};

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
    /// Added to the texture coordinates: a scrolling texture, such as a
    /// tank's moving tracks.
    pub uv_offset: [f32; 2],
    /// Multiplies the surface's colour and alpha (`[1.0; 4]`: as it is):
    /// a marker model's colour, as the scripts give it.
    pub tint: [f32; 4],
    /// A beam material on it shines whatever the hour: the searchlights'
    /// beams follow the night ([`Lighting::beam_gain`]), and a race's ring,
    /// drawn with the same kind of material, must not.
    pub beams_lit: bool,
}

/// How many [`PointLight`]s a frame draws (the game's shaders take 8).
pub const POINT_LIGHTS: usize = 8;

/// A light at a point in the world: an explosion's flash, a fire, a lamp.
/// Lit surfaces take `color` (linear RGB, brightness included) times the
/// light's reach, 1 within `inner` metres falling to 0 at `outer` (the
/// game's shaders: `saturate(d * a + b)` per light), times N.L. `outer`
/// 0 is no light.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointLight {
    pub position: Vec3,
    pub color: Vec3,
    pub inner: f32,
    pub outer: f32,
}

/// The game's screen overlay (its "rain" quad: `PgRainVP`/`PgRainFP`, the
/// PC's shaders 421 and 420), drawn over the finished frame: a texture
/// laid out by view direction, scrolled, tinted, and added to the frame,
/// taken from it, or blended over it. See `gpu::post` for the maths.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenOverlay {
    /// The texture (`global_rain` in every record), sampled as stored.
    pub map: crate::TextureHandle,
    /// The shader's `rainVelocity`: the unit direction it scrolls along,
    /// how far it has scrolled, and its density (the strength).
    pub velocity: [f32; 4],
    /// The shader's `rainColor`.
    pub colour: [f32; 4],
    pub blend: OverlayBlend,
}

/// How the overlay meets the frame, by the record's type (from the 360
/// code, 0x821BF628 and the blend tables at 0x82B74CBC; the operation and
/// factor numbers read by the 360's Direct3D values).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverlayBlend {
    /// Type 0 and any other: added, weighed by its alpha.
    #[default]
    Add,
    /// Type 1: taken from the frame, weighed by its alpha.
    Subtract,
    /// Type 2: blended over the frame by its alpha.
    Alpha,
}

impl OverlayBlend {
    pub fn from_type(kind: u32) -> Self {
        match kind {
            1 => Self::Subtract,
            2 => Self::Alpha,
            _ => Self::Add,
        }
    }
}

/// Light and atmosphere for a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lighting {
    /// Direction the sunlight travels (from the sun towards the ground),
    /// world space. Need not be normalised.
    pub sun_direction: Vec3,
    /// Direction the sun's shadows are cast along, when not the
    /// sunlight's own (the game casts them from a sun never lower than
    /// about 18 degrees: see `m2_engine::atmosphere`).
    pub shadow_direction: Option<Vec3>,
    /// The sun's disc in the sky, display values (black: none drawn).
    pub sun_disc: Vec3,
    /// The sun flare's texture (`radial_flare`); `None` draws no flare.
    pub flare_map: Option<crate::TextureHandle>,
    /// The atmosphere record's screen overlay; `None` draws none.
    pub screen_overlay: Option<ScreenOverlay>,
    /// Linear RGB (see [`gamma_light`](Self::gamma_light)).
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
    /// Metres from the camera that sun shadows reach; 0 turns them off.
    pub shadow_distance: f32,
    /// Scale on the finished world's light before it is clamped to the
    /// screen.
    pub exposure: f32,
    /// How much of the light above `bloom_threshold` (after exposure) glows
    /// into its surroundings; 0 turns bloom off.
    pub bloom: f32,
    pub bloom_threshold: f32,
    /// `BloomBlurRadius`: how far apart the game's blur taps stand in its
    /// first two blurred levels (see the post pass's `blur_spread`).
    pub bloom_radius: f32,
    /// Per-channel scale on the finished frame: a colour grade.
    pub grade: Vec3,
    /// Power on the frame's luminance about mid grey (0.18): above 1
    /// darkens shade and brightens highlights.
    pub contrast: f32,
    /// 1 keeps colours, 0 is grey, above 1 more vivid.
    pub saturation: f32,
    /// The air between the eye and what it sees, and the sky.
    pub haze: Haze,
    /// Clouds over the sky.
    pub clouds: Clouds,
    /// Lights placed in the world this frame (an explosion's flash, a
    /// fire), the nearest first; those with `outer` 0 are none.
    pub point_lights: [PointLight; POINT_LIGHTS],
    /// The water's colour and opacity by how much of it the eye looks
    /// through (the game's depth colour map): linear RGB and opacity at
    /// none, a third, two thirds and all of [`WATER_RAMP_DEPTH`]. `None`
    /// keeps the water's own colour.
    pub water_ramp: Option<[Vec4; 4]>,
    /// The eye's adaptation to how bright the frame is; `None` keeps
    /// `exposure` as it is.
    pub adaptation: Option<Adaptation>,
    /// The game's own adaptation and tone map, in place of `exposure`,
    /// `contrast`, `saturation` and `adaptation` (`grade` still applies).
    pub tone_map: Option<ToneMap>,
    /// With the game's tone map: the frame blended this far (alpha)
    /// towards a ramp by luminance from black through this colour (rgb,
    /// a display value, at mid luminance) to white, as the game's post
    /// pass does.
    pub monochrome: Option<Vec4>,
    /// Scale on the alpha of searchlight beams ([`MaterialDesc::beam`]):
    /// 0 puts them out, 1 shows them as drawn.
    pub beam_gain: f32,
    /// Film grain over the finished frame: how far each speck (two
    /// pixels square, new each frame) moves a display value either way;
    /// 0 for none.
    pub grain: f32,
    /// Light surfaces as the game's shaders do: the sun's and ambient
    /// light (each given here decoded, as linear light) are encoded to
    /// display values, summed there and the sum decoded, so a surface in
    /// sun and shade at once is lit as in the game. Off: summed as linear
    /// light.
    pub gamma_light: bool,
}

/// The game's adaptation and tone map, from its atmosphere's `Bloom*`
/// settings and its post shaders: the frame's luminance `L`, held within
/// `min..max`, is adapted to as `L * k` (`k` running from `min_key` to
/// `max_key` with `ln L`), keeping `retention` of the last value each
/// thirtieth of a second; luminance is scaled by `target` over that,
/// compressed (`0.8 L / (0.7 + L)`), and the display value stretched as
/// `limit + (v - limit) * contrast`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneMap {
    pub target: f32,
    pub min: f32,
    pub max: f32,
    pub min_key: f32,
    pub max_key: f32,
    pub retention: f32,
    /// `BloomAdaptiveLuminanceScale`: how fast the eye follows. Each
    /// frame it keeps `retention ^ (speed * dt)` of the luminance it had,
    /// `dt` held within 1/120 and 0.1 s (from the 360 code, 0x821BD348:
    /// its `adaptiveParams`, which the game's adaptation shader takes).
    pub speed: f32,
    pub contrast_limit: f32,
    pub contrast: f32,
    /// Adapt to the frame's geometric mean luminance (of display values)
    /// rather than, as the game's post shaders appear to, the plain mean
    /// of the luminance of display values raised to 0.4.
    pub log_average: bool,
    /// How the frame is averaged. Negative: as the game's luminance
    /// shaders do (from its shaders: each pixel's Rec. 601 luminance `l`
    /// of display values kept as `ln(l + 0.001)` and `l^2.5`, both
    /// averaged, and `0.9 exp(mean ln) + 0.1 mean(l^2.5)^0.4` adapted to).
    /// Otherwise what the plain mean of luminance^0.4 is raised to before
    /// it is adapted to: 1 takes it as it is (the earlier default, about
    /// twice the game's average by day), 2.5 undoes the 0.4; 0 averages
    /// each pixel's luminance as the game's HDR codec packs it.
    pub average_power: f32,
}

/// Metres of water the eye looks through at the end of the depth colour
/// map: not in the game's data (its shaders take it from code), fitted.
pub const WATER_RAMP_DEPTH: f32 = 20.0;

/// The eye adjusting to the light, as the game's post pass does (its
/// atmospheres carry `BloomTargetLuminance` and `BloomAdaptiveLuminance*`
/// settings): exposure is scaled by `(key / average)^strength`, the
/// average luminance of the frame (geometric mean, before exposure)
/// followed at `speed` per second, within `limits`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Adaptation {
    pub key: f32,
    pub strength: f32,
    pub speed: f32,
    pub limits: [f32; 2],
}

/// Clouds drawn over the sky from a tiling noise texture, as the game's
/// cloud shaders build them: three layers of noise at different scales
/// drifting with the wind, summed and cut off by `coverage`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clouds {
    /// Four channels of tiling noise; `None` draws no clouds.
    pub noise: Option<crate::TextureHandle>,
    /// How much of the sky is cloud, 0 to 1.
    pub coverage: f32,
    /// How many times each layer repeats the noise over the same span.
    pub scales: [f32; 3],
    /// The way the clouds drift (x, z).
    pub wind: [f32; 2],
    /// Scale on the clouds' colour.
    pub tone: Vec3,
    /// Light the clouds as the game's cloud shader does, by the sun alone
    /// through a forward-scattering phase, divided by `1 + mass * this`;
    /// `None` keeps the earlier sun-and-sky lighting.
    pub darkness: Option<f32>,
    /// Fade the deck out towards the horizon; off, it runs on to it, as
    /// a large enough quad would.
    pub horizon_fade: bool,
    /// How opaque the thickest cloud is.
    pub opacity: f32,
    /// The farthest the deck is taken to be (metres) when reckoning the
    /// air in front of it; `None`, wherever the ray meets it.
    pub reach: Option<f32>,
}

impl Default for Clouds {
    fn default() -> Self {
        Self {
            noise: None,
            coverage: 0.0,
            scales: [1.0; 3],
            wind: [1.0, 0.0],
            tone: Vec3::ONE,
            darkness: None,
            horizon_fade: true,
            opacity: 0.6,
            reach: None,
        }
    }
}

/// Light scattering in the air, after Hoffman and Preetham as the game's
/// shaders compute it. The sky is the sunlight the air scatters towards
/// the eye along the whole path up through it. What lies at distance `d`
/// keeps `E = exp(-(rayleigh + mie) * d * extinction * density)` of its
/// colour and takes `(1 - E)` of the sky seen the same way, so far land
/// fades into the sky behind it. Zero `rayleigh` means no haze: plain fog
/// (`Lighting::fog_density`) and a flat sky.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Haze {
    /// Rayleigh scattering per metre, per channel.
    pub rayleigh: Vec3,
    /// Mie scattering per metre, per channel.
    pub mie: Vec3,
    /// Henyey-Greenstein g of the Mie phase function: how strongly haze
    /// glows around the sun.
    pub g: f32,
    /// Scales the scattered light.
    pub inscattering: f32,
    /// Scales the light lost.
    pub extinction: f32,
    /// How much thicker the air near the ground is than the standard
    /// coefficients (the game sets its density from code).
    pub density: f32,
    /// Metres over which each kind of air thins out above the ground, for
    /// the sky's path length.
    pub rayleigh_height: f32,
    pub mie_height: f32,
    /// The game's haze instead: two layers of air, each `(force,
    /// 1 / limit)`, density falling linearly from `force` at height 0 to
    /// nothing at the limit, and the light it adds reckoned in display
    /// values; `None` keeps the scale heights and `density` above.
    pub layers: Option<[f32; 4]>,
    /// With `layers`: a display colour what the air keeps of a surface's
    /// light is also tinted by, as the game's shaders multiply it by
    /// `E * dirLightColor * ScatteringTermMultiplier.x`; the clouds' sun
    /// is divided by it, so they keep their colour.
    pub sun_tint: Option<Vec3>,
    /// Surfaces take the sun's brightness without its hue.
    pub grey_sun: bool,
}

impl Default for Haze {
    fn default() -> Self {
        Self {
            rayleigh: Vec3::ZERO,
            mie: Vec3::ZERO,
            g: 0.0,
            inscattering: 1.0,
            extinction: 1.0,
            density: 1.0,
            rayleigh_height: 8000.0,
            mie_height: 1200.0,
            layers: None,
            sun_tint: None,
            grey_sun: false,
        }
    }
}

impl Default for Lighting {
    fn default() -> Self {
        Self {
            sun_direction: Vec3::new(-0.4, -1.0, 0.3),
            shadow_direction: None,
            sun_disc: Vec3::ZERO,
            flare_map: None,
            screen_overlay: None,
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
            shadow_distance: 200.0,
            exposure: 1.0,
            bloom: 0.0,
            bloom_radius: 0.0,
            bloom_threshold: 1.0,
            grade: Vec3::ONE,
            contrast: 1.0,
            saturation: 1.0,
            haze: Haze::default(),
            clouds: Clouds::default(),
            point_lights: [PointLight::default(); POINT_LIGHTS],
            water_ramp: None,
            adaptation: None,
            tone_map: None,
            monochrome: None,
            beam_gain: 1.0,
            grain: 0.0,
            gamma_light: false,
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

    /// The same view between other near and far distances: the picture's
    /// width and height (the projection's x and y scales) kept.
    pub fn with_near_far(mut self, near: f32, far: f32) -> Self {
        let (x, y) = (self.projection.x_axis.x, self.projection.y_axis.y);
        // Reversed depth, as `perspective`: the planes swapped.
        let mut p = Mat4::perspective_rh(1.0, 1.0, far, near);
        p.x_axis.x = x;
        p.y_axis.y = y;
        self.projection = p;
        self.near = near;
        self.far = far;
        self
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

    /// Forgets material `id`: draws with it use [`MaterialDesc::default`]
    /// again, and it no longer holds its textures (a texture destroyed
    /// while a material names it is only freed once the material goes).
    fn remove_material(&mut self, id: MaterialId) -> Result<(), BackendError>;

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

#[cfg(test)]
mod view_tests {
    use super::*;

    #[test]
    fn other_near_and_far_keep_the_pictures_shape() {
        let v = View::perspective(Vec3::ZERO, Vec3::NEG_Z, 0.8, 16.0 / 9.0, 0.2, 2400.0);
        let w = v.with_near_far(0.3, 500.0);
        assert_eq!(w.projection.x_axis.x, v.projection.x_axis.x);
        assert_eq!(w.projection.y_axis.y, v.projection.y_axis.y);
        assert_eq!((w.near, w.far), (0.3, 500.0));
        // Reversed depth: the near plane at depth 1, the far at 0.
        let depth = |z: f32| {
            let c = w.projection * m2_math::glam::Vec4::new(0.0, 0.0, -z, 1.0);
            c.z / c.w
        };
        assert!((depth(0.3) - 1.0).abs() < 1e-5);
        assert!(depth(500.0).abs() < 1e-5);
    }
}
