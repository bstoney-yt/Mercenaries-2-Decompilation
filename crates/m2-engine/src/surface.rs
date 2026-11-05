//! What surfaces are made of, and what a round striking one throws up.
//!
//! Every render material names its surface in its last word
//! ([`Material::surface`](m2_assets::model::Material::surface)): the name
//! hash of an upper-case word such as `METAL`, `CONCRETE` or
//! `TERRAIN_DIRT`. Terrain and the collision cut from drawn triangles
//! keep each triangle's own. The collision shapes (`PHY2`) carry none
//! (all 1,040,652 mesh triangles' fourth word is 0 over the 1,564
//! sections), so a model's shapes all take the surface of most of its
//! solid drawn area ([`drawn_surface`](crate::physics::drawn_surface);
//! inferred).
//!
//! A bullet striking a surface sets off the
//! `global_particle_impact_bullet_*` template for it. The template
//! database names nine (`brick`, `dirt`, `glass`, `leaves`, `metal`,
//! `stone`, `water`, `wood`, `blood`) and no table ties them to surfaces,
//! so each surface goes to the one its word names, grouped as the
//! database's `DamageChunks` records group the surfaces for their
//! decals:
//!
//! - From the data: `BRICK` and `BRICK_PLASTER` share `3ddecal_brick`;
//!   `DIRT`, `SAND`, `GRASS` and `TERRAIN_DIRT`, `_SAND`, `_GRASS` share
//!   `3ddecal_dirt`; `GENERIC` shares `3ddecal_wood` with `WOOD`. The
//!   stone impact's own spark is `global_particle_impact_concrete_point`
//!   (its `PhysicalLink`), so concrete strikes stone.
//! - Inferred: `ROCK`, `TERRAIN_ROCK`, `ASPHALT`, `TERRAIN_ASPHALT`,
//!   `CERAMIC` and `TERRAIN_CONCRETE` (hard ground and tiles, which have
//!   decals of their own but no bullet impact) strike stone;
//!   `VEGETATION` strikes leaves.
//! - Invented: `FABRIC`, `RUBBER`, `NONE` and the 13 words not yet named
//!   (town batches, fences, piles, tree bark, windows) throw up the
//!   database's `global_particle_impact_generic_nodamage`. No surface
//!   word for glass is known, so `..._bullet_glass` is never chosen.
//!
//! Over the island (the `impacts` example): 99.9% of the terrain's
//! ground strikes an impact its word names; of the placed models' solid
//! drawn area (times placements), 53.3%, the rest (`NONE` 27.4%, the
//! unnamed `ed057225` of the town batches 18.0%) the generic one.
//!
//! Surface words were found by hashing guesses; the 360 code that picks
//! the impact was not read.

use m2_assets::name_hash;

/// The impact when no surface word decides it.
pub const GENERIC_IMPACT: &str = "global_particle_impact_generic_nodamage";

/// Each named surface word and the bullet impact it throws up.
const BULLET_IMPACTS: [(&str, &str); 20] = [
    ("METAL", "global_particle_impact_bullet_metal"),
    ("WOOD", "global_particle_impact_bullet_wood"),
    ("GENERIC", "global_particle_impact_bullet_wood"),
    ("BRICK", "global_particle_impact_bullet_brick"),
    ("BRICK_PLASTER", "global_particle_impact_bullet_brick"),
    ("DIRT", "global_particle_impact_bullet_dirt"),
    ("SAND", "global_particle_impact_bullet_dirt"),
    ("GRASS", "global_particle_impact_bullet_dirt"),
    ("TERRAIN_DIRT", "global_particle_impact_bullet_dirt"),
    ("TERRAIN_SAND", "global_particle_impact_bullet_dirt"),
    ("TERRAIN_GRASS", "global_particle_impact_bullet_dirt"),
    ("CONCRETE", "global_particle_impact_bullet_stone"),
    ("TERRAIN_CONCRETE", "global_particle_impact_bullet_stone"),
    ("ROCK", "global_particle_impact_bullet_stone"),
    ("TERRAIN_ROCK", "global_particle_impact_bullet_stone"),
    ("ASPHALT", "global_particle_impact_bullet_stone"),
    ("TERRAIN_ASPHALT", "global_particle_impact_bullet_stone"),
    ("CERAMIC", "global_particle_impact_bullet_stone"),
    ("VEGETATION", "global_particle_impact_bullet_leaves"),
    ("WATER", "global_particle_impact_bullet_water"),
];

/// Named surface words with no bullet impact of their own.
const OTHER_WORDS: [&str; 3] = ["FABRIC", "RUBBER", "NONE"];

/// A surface's word, if it is one of the named ones.
pub fn word(surface: u32) -> Option<&'static str> {
    BULLET_IMPACTS
        .iter()
        .map(|(w, _)| *w)
        .chain(OTHER_WORDS)
        .find(|w| name_hash(w) == surface)
}

/// The value the tyre cues' surface parameter takes on `surface`: its
/// class, 0 to 4, times 0.2 (from the 360 code, 0x82369190; 0.2 at
/// 0x820B8054). Each track of a tyre cue plays over a window of this
/// parameter. A surface the code doesn't list is class 0.
pub fn tyre_class(surface: u32) -> f32 {
    const ONE: [u32; 27] = [
        0x6700_5058,
        0x30e6_dade,
        0x1d1b_f411,
        0x01b7_74fc,
        0x0eec_28b8,
        0x171a_3b8e,
        0x1c72_73b4,
        0x2861_15a2,
        0x26fe_046c,
        0x4cdb_6b73,
        0x409d_41ce,
        0x349e_446a,
        0x44d6_ab50,
        0x6624_a04f,
        0x5497_915d,
        0x63e1_1a24,
        0xbe74_5c9d,
        0xff30_afe5,
        0xec3c_dfeb,
        0xf23a_a29b,
        0xbcda_2be7,
        0xa90c_7469,
        0x8a39_4748,
        0x75e2_bf9c,
        0xa44c_4ed1,
        0xb5c6_8379,
        0xbbe2_6f59,
    ];
    const THREE: [u32; 13] = [
        0x1611_7a0b,
        0x236d_008b,
        0x2ab2_7544,
        0x3b60_032f,
        0x4224_488a,
        0x6674_bb3b,
        0xe729_9bd8,
        0xc968_7379,
        0xffcf_647e,
        0x6f4f_283e,
        0x8bad_ebcf,
        0xb727_8f60,
        0xb3a7_e6c1,
    ];
    let class = if ONE.contains(&surface) {
        1.0
    } else if surface == 0xbd92_3d99 {
        2.0
    } else if THREE.contains(&surface) {
        3.0
    } else if surface == 0xbffd_e8f8 {
        4.0
    } else {
        0.0
    };
    class * 0.2
}

/// The template a bullet striking `surface` sets off, and whether the
/// surface's word chose it (rather than [`GENERIC_IMPACT`] standing in).
pub fn bullet_impact(surface: u32) -> (&'static str, bool) {
    BULLET_IMPACTS
        .iter()
        .find(|(w, _)| name_hash(w) == surface)
        .map_or((GENERIC_IMPACT, false), |(_, t)| (*t, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surfaces_fall_into_the_tyre_classes_the_code_lists() {
        assert_eq!(tyre_class(0x6700_5058), 0.2);
        assert_eq!(tyre_class(0xbd92_3d99), 0.4);
        assert!((tyre_class(0xffcf_647e) - 0.6).abs() < 1e-6);
        assert_eq!(tyre_class(0xbffd_e8f8), 0.8);
        assert_eq!(tyre_class(0x1234_5678), 0.0);
    }

    #[test]
    fn generic_is_the_word_most_materials_carry() {
        // The value every sample material had before the word was named.
        assert_eq!(name_hash("GENERIC"), 0x959d_8470);
        assert_eq!(word(0x959d_8470), Some("GENERIC"));
    }

    #[test]
    fn surfaces_strike_the_impact_their_word_names() {
        let impact = |w: &str| bullet_impact(name_hash(w));
        assert_eq!(
            impact("METAL"),
            ("global_particle_impact_bullet_metal", true)
        );
        assert_eq!(
            impact("TERRAIN_GRASS"),
            ("global_particle_impact_bullet_dirt", true)
        );
        assert_eq!(
            impact("BRICK_PLASTER"),
            ("global_particle_impact_bullet_brick", true)
        );
        assert_eq!(
            impact("CONCRETE"),
            ("global_particle_impact_bullet_stone", true)
        );
        assert_eq!(
            impact("VEGETATION"),
            ("global_particle_impact_bullet_leaves", true)
        );
        assert_eq!(
            impact("GENERIC"),
            ("global_particle_impact_bullet_wood", true)
        );
    }

    #[test]
    fn unnamed_surfaces_throw_up_the_generic_impact() {
        assert_eq!(bullet_impact(name_hash("FABRIC")), (GENERIC_IMPACT, false));
        assert_eq!(bullet_impact(0xed05_7225), (GENERIC_IMPACT, false));
        assert_eq!(bullet_impact(0), (GENERIC_IMPACT, false));
        assert_eq!(word(name_hash("NONE")), Some("NONE"));
        assert_eq!(word(0xed05_7225), None);
    }
}
