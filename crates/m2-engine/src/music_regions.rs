//! Music regions: where in free play the score is the jungle's or the
//! water's rather than the city's.
//!
//! The static layer `vz_sfx_ambience` places 35 line regions
//! (`rgn_sfx_amb_Jungle`, `rgn_sfx_amb_Ocean`, `rgn_sfx_amb_IslaMono_shoreline`,
//! ...) whose `MusicRegion` component names a free-play faction of the
//! score by name hash in its `faction` field: 20 are `freeplay_jungle`
//! and 15 `freeplay_water`; none names `freeplay_city`. `MrxMusic` makes
//! `freeplay_city` the root (`Sound.SetRootFactionRegionMusic`), the
//! faction outside every region, and entering free play calls
//! `Sound.ActivateFactionRegionMusic` so the region's faction takes over
//! while the faction isn't locked (contracts and special pieces lock it).
//!
//! # Provenance
//!
//! The regions, their outlines and factions: from the data (`vz.wad`'s
//! `layers_static`, read by the survey example's `having:MusicRegion`).
//! The root and when regions apply: from the scripts (`mrxmusic`). Which
//! region wins where they overlap (the smallest, as for the atmosphere
//! regions) is inferred, not checked against the game's code.

use m2_assets::{Layer, name_hash};
use m2_math::glam::{Quat, Vec2, Vec3};

/// A line region naming the score's faction inside it.
#[derive(Clone, Debug, PartialEq)]
pub struct MusicRegion {
    pub entity: u32,
    pub name: String,
    /// Corners on the ground, world `(x, z)`.
    pub outline: Vec<Vec2>,
    /// Square metres inside the outline.
    pub area: f32,
    /// The faction's name hash (`name_hash("freeplay_jungle")`).
    pub faction: u32,
}

impl MusicRegion {
    pub fn new(entity: u32, name: String, outline: Vec<Vec2>, faction: u32) -> Self {
        let n = outline.len();
        let area = (0..n)
            .map(|i| outline[i].perp_dot(outline[(i + 1) % n]))
            .sum::<f32>()
            .abs()
            * 0.5;
        Self {
            entity,
            name,
            outline,
            area,
            faction,
        }
    }

    /// Whether the column over `(x, z)` is inside.
    pub fn contains(&self, x: f32, z: f32) -> bool {
        let mut inside = false;
        let n = self.outline.len();
        for i in 0..n {
            let (a, b) = (self.outline[i], self.outline[(i + n - 1) % n]);
            if (a.y > z) != (b.y > z) && x < (b.x - a.x) * (z - a.y) / (b.y - a.y) + a.x {
                inside = !inside;
            }
        }
        inside
    }
}

/// The region over `(x, z)`: of those holding it, the smallest.
pub fn region_at(regions: &[MusicRegion], x: f32, z: f32) -> Option<&MusicRegion> {
    regions
        .iter()
        .filter(|r| r.contains(x, z))
        .min_by(|a, b| a.area.total_cmp(&b.area))
}

/// Reads the regions of a layer with `MusicRegion` and `LineRegion`
/// components (outlines read by `shape` from the shape's hash, in the
/// entity's own space).
pub fn read_regions(
    layer: &Layer<'_>,
    mut shape: impl FnMut(u32) -> Option<Vec<[f32; 2]>>,
) -> Vec<MusicRegion> {
    let Some(c) = layer.component("MusicRegion") else {
        return Vec::new();
    };
    let Some(field) = c
        .fields
        .iter()
        .find(|f| f.name_hash == name_hash("faction"))
    else {
        return Vec::new();
    };
    let at = usize::from(field.offset);
    let lines = layer.hashes("LineRegion");
    let placements = layer.placements();
    let names = layer.names();
    let mut out: Vec<MusicRegion> = c
        .records()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(entity, rec)| {
            let b = rec.get(at..at + 4)?;
            let faction = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            let p = placements.get(&entity)?;
            let corners = shape(*lines.get(&entity)?)?;
            let origin = Vec3::from(p.position);
            let [x, y, z, w] = p.rotation;
            let q = Quat::from_xyzw(x, y, z, w).normalize();
            let outline = corners
                .iter()
                .map(|&[cx, cz]| {
                    let v = origin + q * Vec3::new(cx, 0.0, cz);
                    Vec2::new(v.x, v.z)
                })
                .collect();
            let name = names.get(&entity).copied().unwrap_or("").to_owned();
            Some(MusicRegion::new(entity, name, outline, faction))
        })
        .collect();
    out.sort_unstable_by_key(|r| r.entity);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(entity: u32, centre: Vec2, half: f32, faction: &str) -> MusicRegion {
        let outline = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(x, z)| centre + Vec2::new(x, z) * half)
            .to_vec();
        MusicRegion::new(entity, String::new(), outline, name_hash(faction))
    }

    #[test]
    fn the_smallest_region_over_a_spot_names_its_faction() {
        let regions = [
            square(1, Vec2::ZERO, 1000.0, "freeplay_jungle"),
            square(2, Vec2::new(500.0, 0.0), 100.0, "freeplay_water"),
        ];
        assert_eq!(regions[0].area, 4.0e6);
        let at = |x, z| region_at(&regions, x, z).map(|r| r.faction);
        assert_eq!(at(0.0, 0.0), Some(name_hash("freeplay_jungle")));
        assert_eq!(at(520.0, 10.0), Some(name_hash("freeplay_water")));
        assert_eq!(at(2000.0, 0.0), None);
    }
}
