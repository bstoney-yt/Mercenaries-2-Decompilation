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
//! below). Whether the colours are stored gamma-encoded is not known;
//! they are taken as linear.

use m2_assets::layer::Layer;
use m2_assets::name_hash;
use m2_math::glam::Vec3;
use m2_render::Lighting;

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
            let byte = |shift: u32| ((v >> shift) & 0xff) as f32 / 255.0;
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
        })
    }

    /// Puts the ambient cube, sunlight strength and rim light into
    /// `lighting`.
    pub fn apply(&self, lighting: &mut Lighting) {
        lighting.ambient_cube = self.ambient_cube;
        lighting.rim_color = self.rim;
        lighting.rim_threshold = self.rim_threshold;
        lighting.sun_color = Lighting::default().sun_color * self.light_intensity;
    }
}
