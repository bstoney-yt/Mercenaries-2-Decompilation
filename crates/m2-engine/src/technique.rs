//! What a material's shader does, from its technique id.
//!
//! A material record's technique is the name hash
//! ([`m2_assets::name_hash`]) of the pixel shader it draws with, such as
//! `PgDiffSpecNormFP`. The name spells out the shader's inputs: `Diff`
//! (diffuse map, slot 0), `Spec` (specular map, slot 1), `Norm` (normal
//! map, slot 2), `Emis` (emissive map in slot 1 instead), `AmbOcc`
//! (ambient occlusion in the vertex colour), `Rim`, `Refl`, `Metal`, `SSS`
//! and so on. Terrain shaders `PgTerrainMeshFP1D`..`4D` blend one to four
//! tiled layers.
//!
//! # Provenance
//!
//! The names are the shader names found in the install's `shader3.bin`
//! and matched against every technique id in the archives (31 of 31 model
//! techniques, 6 terrain ones). Which texture slot holds what comes from
//! the texture names in each slot (`_dm`, `_sm`, `_nm`) and the samplers
//! each shader declares.

use std::collections::HashMap;
use std::sync::OnceLock;

use m2_assets::name_hash;

/// Pixel shaders model and terrain materials name.
const NAMES: &[&str] = &[
    "PgDiffFP",
    "PgFastFP",
    "PgDiffAmbOccFP",
    "PgDiffAmbOccRimFP",
    "PgDiffRimFP",
    "PgDiffEmisFP",
    "PgDiffEmisAmbOccFP",
    "PgDiffEmisRimFP",
    "PgDiffEmisAmbOccRimFP",
    "PgDiffSpecFP",
    "PgDiffSpecAmbOccFP",
    "PgDiffSpecAmbOccRimFP",
    "PgDiffSpecRimFP",
    "PgDiffSpecReflFP",
    "PgDiffSpecMetalFP",
    "PgDiffSpecNormFP",
    "PgDiffSpecNormAmbOccFP",
    "PgDiffSpecNormAmbOccRimFP",
    "PgDiffSpecNormRimFP",
    "PgDiffSpecReflNormFP",
    "PgDiffSpecReflNormAmbOccFP",
    "PgDiffSpecReflNormAmbOccRimFP",
    "PgDiffSpecReflNormRimFP",
    "PgDiffSpecMetalNormFP",
    "PgDiffSpecMetalNormAmbOccFP",
    "PgDiffSpecMetalNormAmbOccRimFP",
    "PgDiffSpecMetalNormRimFP",
    "PgDiffSpecSSSNormAmbOccFP",
    "PgDiffSpecSSSNormAmbOccRimFP",
    "PgDiffSpecSSSNormRimFP",
    "PgDiffRefractNormFP",
    "PgTerrainMeshFP",
    "PgTerrainMeshFP1D",
    "PgTerrainMeshFP2D",
    "PgTerrainMeshFP3D",
    "PgTerrainMeshFP4D",
];

/// The features of one pixel shader.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Technique {
    pub name: &'static str,
    /// Slot 1 is a specular map.
    pub specular: bool,
    /// Slot 2 is a tangent-space normal map.
    pub normal: bool,
    /// Slot 1 is an emissive map.
    pub emissive: bool,
    /// The vertex colour holds ambient occlusion.
    pub ambient_occlusion: bool,
    pub rim: bool,
    pub reflection: bool,
    pub metal: bool,
    pub subsurface: bool,
    pub refraction: bool,
    /// A terrain shader: slot 0 the cell's colour map, slot 1 the detail
    /// map, then this many (colour, normal) layer pairs.
    pub terrain_layers: Option<u8>,
}

impl Technique {
    fn from_name(name: &'static str) -> Self {
        let has = |token: &str| name.contains(token);
        let terrain_layers = name.strip_prefix("PgTerrainMeshFP").map(|rest| {
            rest.strip_suffix('D')
                .and_then(|n| n.parse().ok())
                .unwrap_or(0)
        });
        Self {
            name,
            specular: has("Spec"),
            normal: has("Norm"),
            emissive: has("Emis"),
            ambient_occlusion: has("AmbOcc"),
            rim: has("Rim"),
            reflection: has("Refl"),
            metal: has("Metal"),
            subsurface: has("SSS"),
            refraction: has("Refract"),
            terrain_layers,
        }
    }

    /// The technique with id `hash`, if it is one of the known shaders.
    pub fn from_hash(hash: u32) -> Option<Self> {
        static TABLE: OnceLock<HashMap<u32, Technique>> = OnceLock::new();
        TABLE
            .get_or_init(|| {
                NAMES
                    .iter()
                    .map(|&n| (name_hash(n), Technique::from_name(n)))
                    .collect()
            })
            .get(&hash)
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_found_in_the_archives_resolve() {
        // Technique ids as stored in material records.
        let t = Technique::from_hash(0xcaef_e1fe).unwrap();
        assert_eq!(t.name, "PgDiffSpecNormFP");
        assert!(t.specular && t.normal && !t.ambient_occlusion);
        let t = Technique::from_hash(0x1560_a817).unwrap();
        assert!(t.specular && t.normal && t.ambient_occlusion);
        let t = Technique::from_hash(0x343a_f931).unwrap();
        assert_eq!(t.name, "PgDiffFP");
        assert!(!t.specular && !t.normal);
        let t = Technique::from_hash(0x7ed2_de27).unwrap();
        assert!(t.emissive && !t.specular);
        assert_eq!(
            Technique::from_hash(0x93b4_ed44).unwrap().terrain_layers,
            Some(4)
        );
        assert_eq!(
            Technique::from_hash(0x4ed5_07fb).unwrap().terrain_layers,
            Some(1)
        );
        assert_eq!(
            Technique::from_hash(0x8e94_2cb4).unwrap().terrain_layers,
            Some(0)
        );
        assert!(Technique::from_hash(0x1234_5678).is_none());
    }
}
