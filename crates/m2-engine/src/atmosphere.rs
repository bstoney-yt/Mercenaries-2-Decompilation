//! The island's light and air: the template database's `AtmosphereBase`
//! settings that the game's shaders take their ambient cube, sunlight
//! strength, rim light and haze from.
//!
//! Three templates carry an `AtmosphereBase`: `Atmosphere` (its `Name`
//! field `Default`, outdoors), `Atmosphere (interior)` (`PMC`) and
//! `Will_atmospheres`. No placed entity names one, so the outdoor one is
//! taken. Field names are the name hashes of the names below, found by
//! hashing candidates; colours are `0xAARRGGBB` words whose alpha is not
//! used (the `...Multiplier` fields scale them instead).
//!
//! # Provenance
//!
//! The field names come from matching name hashes; what each one does is
//! read from its name and from the game's pixel shaders (ambient cube
//! faces in the order +x, -x, +y, -y, +z, -z: the `Default` cube is
//! brightest in face 2 and darkest in face 3, sky above and ground
//! below). The colours are taken as gamma-encoded (sRGB) bytes, as
//! colours picked in an editor are, and decoded to linear light: read as
//! linear, shade was half as dark against sunlight as the original game's
//! (measured on PC video frames). That reading is inferred.

use m2_assets::layer::Layer;
use m2_assets::name_hash;
use m2_math::glam::Vec3;
use m2_render::{Haze, Lighting};

/// Settings read from an `AtmosphereBase` record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Atmosphere {
    /// `AmbientCube0`..`5` times their multipliers: light from +x, -x,
    /// +y, -y, +z and -z.
    pub ambient_cube: [Vec3; 6],
    /// `LightIntensity`: the sunlight's strength.
    pub light_intensity: f32,
    /// `RimColor` times `RimColorMultiplier`.
    pub rim: Vec3,
    /// `RimColor`'s alpha byte, taken as the rim light's threshold (the
    /// shaders' `rimLightColor.w`): inferred, the other colours' alpha
    /// bytes have no use.
    pub rim_threshold: f32,
    /// `ScatterInscatteringMultiplier`, `ScatterExtinctionMultiplier`,
    /// `ScatterBetaMieMultiplier` and `ScatterHgG`: the haze.
    pub inscattering: f32,
    pub extinction: f32,
    pub mie: f32,
    pub hg_g: f32,
    /// `Tone_R`, `Tone_G`, `Tone_B`: a colour grade, probably.
    pub tone: Vec3,
    /// `BloomAmount`, `BloomMultiplier`, `BloomThreshold`,
    /// `BloomBlurRadius` (the radius is not used yet).
    pub bloom: [f32; 4],
    /// `LightTime` and `LightTimeSpeed`: the time of day, probably.
    pub light_time: [f32; 2],
}

impl Atmosphere {
    /// The record whose `Name` field is `name` (`Default` outdoors), from
    /// the template database `db`.
    pub fn read(db: &Layer<'_>, name: &str) -> Option<Self> {
        let c = db.component("AtmosphereBase")?;
        let field = |rec: &[u8], field: &str| -> Option<u32> {
            let f = c.fields.iter().find(|f| f.name_hash == name_hash(field))?;
            let at = usize::from(f.offset);
            rec.get(at..at + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let float = |rec: &[u8], name: &str| field(rec, name).map(f32::from_bits);
        let records = c.records()?;
        let (_, rec) = records
            .iter()
            .find(|(_, r)| field(r, "Name") == Some(name_hash(name)))?;
        let colour = |name: &str| {
            let v = field(rec, name)?;
            let scale = float(rec, &format!("{name}Multiplier")).unwrap_or(1.0);
            let byte = |shift: u32| srgb_to_linear(((v >> shift) & 0xff) as f32 / 255.0);
            Some(Vec3::new(byte(16), byte(8), byte(0)) * scale)
        };
        let mut ambient_cube = [Vec3::ZERO; 6];
        for (i, face) in ambient_cube.iter_mut().enumerate() {
            *face = colour(&format!("AmbientCube{i}"))?;
        }
        Some(Self {
            ambient_cube,
            light_intensity: float(rec, "LightIntensity").unwrap_or(1.0),
            rim: colour("RimColor").unwrap_or(Vec3::ZERO),
            rim_threshold: field(rec, "RimColor").map_or(0.0, |v| (v >> 24) as f32 / 255.0),
            inscattering: float(rec, "ScatterInscatteringMultiplier").unwrap_or(0.0),
            extinction: float(rec, "ScatterExtinctionMultiplier").unwrap_or(0.0),
            mie: float(rec, "ScatterBetaMieMultiplier").unwrap_or(0.0),
            hg_g: float(rec, "ScatterHgG").unwrap_or(0.0),
            tone: Vec3::new(
                float(rec, "Tone_R").unwrap_or(1.0),
                float(rec, "Tone_G").unwrap_or(1.0),
                float(rec, "Tone_B").unwrap_or(1.0),
            ),
            bloom: [
                "BloomAmount",
                "BloomMultiplier",
                "BloomThreshold",
                "BloomBlurRadius",
            ]
            .map(|n| float(rec, n).unwrap_or(0.0)),
            light_time: ["LightTime", "LightTimeSpeed"].map(|n| float(rec, n).unwrap_or(0.0)),
        })
    }

    /// Puts the ambient cube, sunlight strength, rim light, haze, bloom
    /// and grade into `lighting`.
    pub fn apply(&self, lighting: &mut Lighting) {
        lighting.ambient_cube = self.ambient_cube;
        lighting.rim_color = self.rim;
        lighting.rim_threshold = self.rim_threshold;
        lighting.sun_color = Lighting::default().sun_color * self.light_intensity;
        lighting.haze = self.haze();
        // `BloomAmount` times `BloomMultiplier` above `BloomThreshold`, and
        // `Tone_*` as the grade (inferred from the names).
        lighting.bloom = self.bloom[0] * self.bloom[1];
        lighting.bloom_threshold = self.bloom[2];
        lighting.grade = self.tone;
    }

    /// The haze these settings describe: Hoffman and Preetham's standard
    /// Rayleigh and Mie scattering (Mie for a turbidity of 2), scaled by
    /// the `Scatter...` multipliers. That the game's multipliers apply to
    /// these standard coefficients is inferred: its shaders take the
    /// products from code. On their own they haze land a few percent per
    /// kilometre; [`HAZE_DENSITY`] thickens that to match footage.
    pub fn haze(&self) -> Haze {
        if self.inscattering <= 0.0 {
            return Haze::default();
        }
        Haze {
            rayleigh: RAYLEIGH,
            mie: MIE * self.mie,
            g: self.hg_g,
            inscattering: self.inscattering,
            extinction: self.extinction,
            density: HAZE_DENSITY,
            ..Haze::default()
        }
    }
}

/// Air near the ground against the standard coefficients: fitted so land
/// 2 km away is about 40% haze, as in PC footage of the game (the game's
/// own density comes from code).
const HAZE_DENSITY: f32 = 3.0;
/// Rayleigh scattering of air per metre at 650, 570 and 475 nm.
const RAYLEIGH: Vec3 = Vec3::new(6.95e-6, 1.18e-5, 2.44e-5);
/// Mie scattering per metre at the same wavelengths for a turbidity of 2,
/// before the game's multiplier: `0.434 c pi (2 pi / lambda)^2 K` with
/// `c = (0.6544 T - 0.6510) 1e-16`.
const MIE: Vec3 = Vec3::new(5.75e-3, 7.39e-3, 1.045e-2);

/// A gamma-encoded (sRGB) channel in 0..1 as linear light.
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
