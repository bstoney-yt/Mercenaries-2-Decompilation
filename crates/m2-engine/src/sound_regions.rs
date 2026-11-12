//! Sound regions: where in free play the score is the jungle's or the
//! water's rather than the city's, and which ambience plays where.
//!
//! The static layer `vz_sfx_ambience` places 61 line regions
//! (`rgn_sfx_amb_Jungle`, `rgn_sfx_amb_Ocean`, `rgn_sfx_amb_Caracas_City`,
//! ...). Each has a `SoundAmbience` component: the ambience cue that plays
//! inside it and the sound bank it needs, by name hash (`AmbienceCue`,
//! `AmbienceBank`: the same hash in 53 of them; the other 8 name no bank),
//! a `Priority`, a `ReverbPreset` and an `OverlayCue`. 35 also have
//! a `MusicRegion` component naming a free-play faction of the score in
//! its `faction` field: 20 `freeplay_jungle` and 15 `freeplay_water`; none
//! names `freeplay_city`.
//!
//! `MrxMusic` makes `freeplay_city` the root (`Sound.SetRootFactionRegionMusic`),
//! the faction outside every music region, and entering free play calls
//! `Sound.ActivateFactionRegionMusic` so the region's faction takes over
//! while the faction isn't locked (contracts and special pieces lock it).
//! The ambience cues are the `ambience` cue table's ([`crate::ambience`]).
//!
//! # Provenance
//!
//! The regions, their outlines, factions and ambience cues: from the data
//! (`vz.wad`'s `layers_static`; the survey example's `having:MusicRegion`
//! and `ambience` sections). The root and when music regions apply: from
//! the scripts (`mrxmusic`). Which music region wins where they overlap
//! (the smallest, as for the atmosphere regions) is inferred, not checked
//! against the game's code.
//!
//! From the 360 code: the `SoundAmbience` record's five fields and their
//! order (its registration, `0x823F3F40`), and that every ambience region
//! holding the listener asks for its ambience, the list
//! ([`crate::ambience_stack`]) choosing among them (`0x8236DBD8`). All 61
//! regions have priority 0, reverb 0 and overlay 0 (from the data), so
//! where they overlap the one entered last plays. The 360 tests the
//! listener's position against the region's shape; here the outline's
//! column stands in for it (heights are not read).

use crate::ambience_stack::Entry;
use m2_assets::{Layer, name_hash};
use m2_math::glam::{Quat, Vec2, Vec3};

/// A line region with a score faction, an ambience cue, or both.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundRegion {
    pub entity: u32,
    pub name: String,
    /// Corners on the ground, world `(x, z)`.
    pub outline: Vec<Vec2>,
    /// Square metres inside the outline.
    pub area: f32,
    /// The score's faction inside, by name hash
    /// (`name_hash("freeplay_jungle")`), if a music region.
    pub faction: Option<u32>,
    /// The ambience asked for while the listener is inside (its cue
    /// `name_hash("amb_jungle")`, bank, priority, reverb and overlay), if
    /// any.
    pub ambience: Option<Entry>,
}

impl SoundRegion {
    pub fn new(entity: u32, name: String, outline: Vec<Vec2>) -> Self {
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
            faction: None,
            ambience: None,
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

/// The music region over `(x, z)`: of those with a faction holding it,
/// the smallest.
pub fn music_at(regions: &[SoundRegion], x: f32, z: f32) -> Option<&SoundRegion> {
    smallest(regions, x, z, |r| r.faction.is_some())
}

/// The ambience regions holding `(x, z)`, in the order read: each asks
/// for its ambience while the listener is inside
/// ([`crate::ambience_stack`]).
pub fn ambiences_at(regions: &[SoundRegion], x: f32, z: f32) -> impl Iterator<Item = &SoundRegion> {
    regions
        .iter()
        .filter(move |r| r.ambience.is_some() && r.contains(x, z))
}

fn smallest(
    regions: &[SoundRegion],
    x: f32,
    z: f32,
    with: impl Fn(&SoundRegion) -> bool,
) -> Option<&SoundRegion> {
    regions
        .iter()
        .filter(|r| with(r) && r.contains(x, z))
        .min_by(|a, b| a.area.total_cmp(&b.area))
}

/// The first word of each record of `component`'s field `field`, by entity.
fn field_words(layer: &Layer<'_>, component: &str, field: u32) -> Vec<(u32, u32)> {
    let Some(c) = layer.component(component) else {
        return Vec::new();
    };
    let Some(f) = c.fields.iter().find(|f| f.name_hash == field) else {
        return Vec::new();
    };
    let at = usize::from(f.offset);
    c.records()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(entity, rec)| {
            let b = rec.get(at..at + 4)?;
            Some((entity, u32::from_le_bytes([b[0], b[1], b[2], b[3]])))
        })
        .collect()
}

/// The `SoundAmbience` record's fields, in the record's order.
const AMBIENCE_FIELDS: [&str; 5] = [
    "AmbienceCue",
    "AmbienceBank",
    "Priority",
    "ReverbPreset",
    "OverlayCue",
];

/// Reads the regions of a layer with `MusicRegion` or `SoundAmbience`,
/// and `LineRegion`, components (outlines read by `shape` from the shape's
/// hash, in the entity's own space).
pub fn read_regions(
    layer: &Layer<'_>,
    mut shape: impl FnMut(u32) -> Option<Vec<[f32; 2]>>,
) -> Vec<SoundRegion> {
    let factions = field_words(layer, "MusicRegion", name_hash("faction"));
    let [cues, banks, priorities, reverbs, overlays] =
        AMBIENCE_FIELDS.map(|f| field_words(layer, "SoundAmbience", name_hash(f)));
    let mut entities: Vec<u32> = factions.iter().chain(&cues).map(|e| e.0).collect();
    entities.sort_unstable();
    entities.dedup();
    let lines = layer.hashes("LineRegion");
    let placements = layer.placements();
    let names = layer.names();
    entities
        .into_iter()
        .filter_map(|entity| {
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
            let word = |of: &[(u32, u32)]| {
                of.iter()
                    .find(|e| e.0 == entity)
                    .map(|e| e.1)
                    .filter(|&w| w != 0)
            };
            let or_0 = |of: &[(u32, u32)]| word(of).unwrap_or(0);
            Some(SoundRegion {
                faction: word(&factions),
                ambience: word(&cues).map(|cue| Entry {
                    cue,
                    bank: or_0(&banks),
                    priority: or_0(&priorities).cast_signed(),
                    reverb: or_0(&reverbs),
                    overlay: or_0(&overlays),
                }),
                ..SoundRegion::new(entity, name, outline)
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(entity: u32, centre: Vec2, half: f32) -> SoundRegion {
        let outline = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(x, z)| centre + Vec2::new(x, z) * half)
            .to_vec();
        SoundRegion::new(entity, String::new(), outline)
    }

    #[test]
    fn the_smallest_region_over_a_spot_names_its_faction_and_all_ask_for_ambience() {
        let ambience = |cue: &str| {
            Some(Entry {
                cue: name_hash(cue),
                bank: name_hash(cue),
                ..Entry::default()
            })
        };
        let regions = [
            SoundRegion {
                faction: Some(name_hash("freeplay_jungle")),
                ambience: ambience("amb_jungle"),
                ..square(1, Vec2::ZERO, 1000.0)
            },
            SoundRegion {
                faction: Some(name_hash("freeplay_water")),
                ..square(2, Vec2::new(500.0, 0.0), 100.0)
            },
            SoundRegion {
                ambience: ambience("amb_citymed"),
                ..square(3, Vec2::new(-500.0, 0.0), 100.0)
            },
        ];
        assert_eq!(regions[0].area, 4.0e6);
        let music = |x, z| music_at(&regions, x, z).and_then(|r| r.faction);
        let amb = |x, z| -> Vec<u32> { ambiences_at(&regions, x, z).map(|r| r.entity).collect() };
        assert_eq!(music(0.0, 0.0), Some(name_hash("freeplay_jungle")));
        assert_eq!(music(520.0, 10.0), Some(name_hash("freeplay_water")));
        // A region without ambience asks for none.
        assert_eq!(amb(520.0, 10.0), [1]);
        // Both regions over a spot ask for theirs.
        assert_eq!(amb(-520.0, 10.0), [1, 3]);
        assert_eq!(music(-520.0, 10.0), Some(name_hash("freeplay_jungle")));
        assert_eq!(music(2000.0, 0.0), None);
        assert!(amb(2000.0, 0.0).is_empty());
    }

    #[test]
    fn the_cue_field_is_the_one_the_regions_were_read_by() {
        // The field's hash was known from the data before its name was.
        assert_eq!(name_hash(AMBIENCE_FIELDS[0]), 0xdc0a_5e16);
    }
}
