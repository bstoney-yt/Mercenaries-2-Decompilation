//! The island's traffic and spawned people, as the layers and the template
//! database place them: a `PopulationSimpleSpawner` names a list (a
//! `PopulationList` template: what it spawns, each with a weight), how
//! often it spawns (`SecondsPerCycle`) and how near the player must be
//! (`ActiveRadius`). Placed traffic (`PointTraffic_*`, `PathSpawner_*`
//! entities) drives its own patrol path (`AiPatrol`: the path entity and a
//! `Haste`), not the road network.
//!
//! Field offsets were read off the database's schema on the install
//! (2026-10-06): the spawner's list at 44, `SecondsPerCycle` at 52,
//! `ActiveRadius` at 60, `SpawnerType` at 84; a list entry is a template
//! id and a weight; `AiPatrol` has its path entity at 8 and `Haste` at 12.
//! The names are found by hash first, so a schema that moves them still
//! reads.

use std::collections::HashMap;

use m2_assets::layer::Component;
use m2_assets::{Layer, name_hash};

/// A spawner's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spawner {
    /// The `PopulationList` template it spawns from, by id.
    pub list: u32,
    /// Seconds between spawns.
    pub seconds_per_cycle: f32,
    /// It spawns while the player is this near, metres.
    pub active_radius: f32,
    /// `SpawnerType`: 2 at the roadblocks' point traffic, 0 on the path
    /// spawners (meaning not known yet).
    pub kind: u32,
    /// The field named only by its hash d192b18a: 1500 on the path
    /// spawners, whose `ActiveRadius` is 0. Taken as how near the player
    /// must be for those (unverified).
    pub far: Option<f32>,
}

impl Spawner {
    /// How near the player must be for it to spawn: its `ActiveRadius`,
    /// or where that is 0, the d192b18a field.
    pub fn reach(&self) -> f32 {
        if self.active_radius > 0.0 {
            self.active_radius
        } else {
            self.far.unwrap_or(0.0)
        }
    }
}

/// The field after `ActiveRadius` whose name isn't known, by its hash.
const FAR_HASH: u32 = 0xd192_b18a;

/// What the template database says of spawning.
#[derive(Clone, Debug, Default)]
pub struct Population {
    /// Spawner templates, by id.
    pub spawners: HashMap<u32, Spawner>,
    /// List templates, by id: each entry's template id and weight.
    pub lists: HashMap<u32, Vec<(u32, f32)>>,
    /// `AiPatrol` templates' `Haste`, by id.
    pub haste: HashMap<u32, f32>,
}

const LIST_AT: usize = 44;
const CYCLE_AT: usize = 52;
const RADIUS_AT: usize = 60;
const KIND_AT: usize = 84;
/// `AiPatrol`'s path entity and `Haste`.
pub const PATROL_PATH_AT: usize = 8;
const HASTE_AT: usize = 12;

impl Population {
    pub fn read(db: &Layer<'_>) -> Self {
        let mut lists: HashMap<u32, Vec<(u32, f32)>> = HashMap::new();
        if let Some(c) = db.component("PopulationList") {
            for (id, r) in c.records().unwrap_or_default() {
                if let (Some(template), Some(weight)) = (word(r, 0), r.get(4..8)) {
                    lists
                        .entry(id)
                        .or_default()
                        .push((template, weight_of(weight)));
                }
            }
        }
        Self {
            spawners: db
                .component("PopulationSimpleSpawner")
                .map(spawners)
                .unwrap_or_default(),
            lists,
            haste: db.component("AiPatrol").map(haste).unwrap_or_default(),
        }
    }
}

/// Every spawner record of a component (a layer's or the database's).
pub fn spawners(c: &Component<'_>) -> HashMap<u32, Spawner> {
    let at = |name: &str, fallback: usize| field_at(c, name).unwrap_or(fallback);
    let (cycle, radius, kind) = (
        at("SecondsPerCycle", CYCLE_AT),
        at("ActiveRadius", RADIUS_AT),
        at("SpawnerType", KIND_AT),
    );
    let far = c
        .fields
        .iter()
        .find(|f| f.name_hash == FAR_HASH)
        .map(|f| usize::from(f.offset));
    c.records()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(id, r)| {
            Some((
                id,
                Spawner {
                    list: word(r, LIST_AT)?,
                    seconds_per_cycle: float(r, cycle)?,
                    active_radius: float(r, radius)?,
                    kind: word(r, kind)?,
                    far: far.and_then(|at| float(r, at)),
                },
            ))
        })
        .collect()
}

/// Every `AiPatrol` record's `Haste`.
pub fn haste(c: &Component<'_>) -> HashMap<u32, f32> {
    let at = field_at(c, "Haste").unwrap_or(HASTE_AT);
    c.records()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(id, r)| Some((id, float(r, at)?)))
        .collect()
}

/// Where the field named `name` is in a record, by its hash.
fn field_at(c: &Component<'_>, name: &str) -> Option<usize> {
    let h = name_hash(name);
    c.fields
        .iter()
        .find(|f| f.name_hash == h)
        .map(|f| usize::from(f.offset))
}

fn word(r: &[u8], at: usize) -> Option<u32> {
    let b = r.get(at..at + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn float(r: &[u8], at: usize) -> Option<f32> {
    word(r, at).map(f32::from_bits).filter(|f| f.is_finite())
}

/// A list entry's weight: the install shows whole numbers (17 to 21, 100);
/// stored as a float they read as one, stored as an integer the float is
/// a tiny denormal, so the integer is taken.
fn weight_of(b: &[u8]) -> f32 {
    let w = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let f = f32::from_bits(w);
    if f.is_normal() && f > 0.0 {
        f
    } else {
        w as f32
    }
}

/// Picks an entry by weight; `roll` is in [0, 1).
pub fn pick(entries: &[(u32, f32)], roll: f32) -> Option<u32> {
    let total: f32 = entries.iter().map(|&(_, w)| w.max(0.0)).sum();
    if total <= 0.0 {
        return entries.first().map(|&(t, _)| t);
    }
    let mut left = roll.clamp(0.0, 0.999_999) * total;
    for &(t, w) in entries {
        let w = w.max(0.0);
        if left < w {
            return Some(t);
        }
        left -= w;
    }
    entries.last().map(|&(t, _)| t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_read_as_integers_or_floats() {
        assert_eq!(weight_of(&100u32.to_le_bytes()), 100.0);
        assert_eq!(weight_of(&17.5f32.to_bits().to_le_bytes()), 17.5);
    }

    #[test]
    fn a_spawner_without_a_radius_reaches_as_far_as_its_other_field() {
        let mut s = Spawner {
            list: 1,
            seconds_per_cycle: 10.0,
            active_radius: 250.0,
            kind: 2,
            far: Some(1500.0),
        };
        assert_eq!(s.reach(), 250.0);
        s.active_radius = 0.0;
        assert_eq!(s.reach(), 1500.0);
    }

    #[test]
    fn picks_go_by_weight() {
        let list = [(1, 20.0), (2, 0.0), (3, 80.0)];
        assert_eq!(pick(&list, 0.0), Some(1));
        assert_eq!(pick(&list, 0.19), Some(1));
        assert_eq!(pick(&list, 0.21), Some(3));
        assert_eq!(pick(&list, 1.0), Some(3));
        assert_eq!(pick(&[], 0.5), None);
    }
}
