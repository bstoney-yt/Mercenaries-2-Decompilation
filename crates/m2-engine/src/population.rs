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
use m2_math::glam::{Quat, Vec2, Vec3};

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
    /// Who rides in a vehicle template, by id: each `Rider` record's seat
    /// (its name's hash) and person template id (empty seats left out).
    pub riders: HashMap<u32, Vec<(u32, u32)>>,
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
            riders: db.component("Rider").map(riders).unwrap_or_default(),
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

/// Every `Rider` record: one per seat, the seat's name hash and the
/// person template id riding there (0: nobody). Read off the install
/// (2026-10-07): seat at 0 (field ee6a83a3), person at 4 (f55054b8); the
/// crewed templates (`M151 (MG) (GR) (DriverGunner)`, the civilian
/// `Thunder (white)`) differ from their plain twins by this alone.
pub fn riders(c: &Component<'_>) -> HashMap<u32, Vec<(u32, u32)>> {
    let by_hash = |h: u32, fallback: usize| {
        c.fields
            .iter()
            .find(|f| f.name_hash == h)
            .map_or(fallback, |f| usize::from(f.offset))
    };
    let (seat, person) = (by_hash(SEAT_HASH, 0), by_hash(PERSON_HASH, 4));
    let mut out: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
    for (id, r) in c.records().unwrap_or_default() {
        if let (Some(seat), Some(person)) = (word(r, seat), word(r, person))
            && person != 0
        {
            out.entry(id).or_default().push((seat, person));
        }
    }
    out
}

const SEAT_HASH: u32 = 0xee6a_83a3;
const PERSON_HASH: u32 = 0xf550_54b8;

/// The seat names the install's `Rider` records were seen to use, for
/// naming a seat by its hash.
pub const SEATS: [&str; 6] = [
    "seat_driver",
    "seat_gunner",
    "seat_gunner_rearcenter",
    "seat_gunner_rightfront",
    "seat_front_right",
    "seat_passenger",
];

/// A seat's name from its hash: a known one, else the hash in hex.
pub fn seat_name(hash: u32) -> String {
    SEATS
        .iter()
        .find(|s| name_hash(s) == hash)
        .map_or_else(|| format!("{hash:08x}"), |s| (*s).to_owned())
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

/// An area's traffic and passers-by: a `LineRegion` entity's outline with
/// its `PopulationDensity` record. Read off the install (2026-10-14, 281
/// records in 80 layers): each names a vehicle list and a people list
/// (`VehList_Mar_Outskirt_Act1`, `PedList_Mar_Outskirt_Act1`; or
/// `VehList_Blank` and `PedList_Blank`, in the `*_DeadTraffic` regions
/// round the mission sites), two counts, a `Priority` (5 to 100) and a
/// `faction`. Inferred from samples, not from the code: the first count
/// is how many vehicles are out (Maracaibo's city centre 8, its
/// outskirts 6, a dead region 1), the second how many people (the city
/// centre 20, the outskirts 2), and where regions overlap the one of
/// highest priority rules (the dead regions are 100, the island-wide
/// `mar_bigA` 5).
#[derive(Clone, Debug, PartialEq)]
pub struct DensityZone {
    pub entity: u32,
    pub name: String,
    /// The layer placing it, by name hash: a state layer
    /// (`vz_state_mar_outskirt_act1`) or a static sub-layer.
    pub layer: u32,
    pub static_layer: bool,
    /// Its outline on the ground (x, z), world space.
    pub outline: Vec<Vec2>,
    pub vehicles: u32,
    pub people: u32,
    /// The lists, by template id (0: none).
    pub vehicle_list: u32,
    pub people_list: u32,
    pub priority: u32,
    pub faction: u32,
}

impl DensityZone {
    /// Whether the column over (`x`, `z`) is inside.
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

/// The zone of `zones` that rules at (`x`, `z`): of those it is inside,
/// the one of highest priority (the lowest entity of equals).
pub fn zone_at<'a>(
    zones: impl IntoIterator<Item = &'a DensityZone>,
    x: f32,
    z: f32,
) -> Option<&'a DensityZone> {
    zones
        .into_iter()
        .filter(|zone| zone.contains(x, z))
        .min_by_key(|zone| (std::cmp::Reverse(zone.priority), zone.entity))
}

const DENSITY_VEHICLES: u32 = 0x263c_1369;
const DENSITY_PEOPLE: u32 = 0xc63c_9abc;
const DENSITY_VEHICLE_LIST: u32 = 0x77e8_38e4;
const DENSITY_PEOPLE_LIST: u32 = 0xafb4_5fd9;

/// A layer's traffic zones: its entities with `PopulationDensity` and a
/// `LineRegion`, whose outline `shape` reads from the region's hash (in
/// the entity's own space).
pub fn read_zones(
    layer: &Layer<'_>,
    layer_hash: u32,
    static_layer: bool,
    mut shape: impl FnMut(u32) -> Option<Vec<[f32; 2]>>,
) -> Vec<DensityZone> {
    let Some(c) = layer.component("PopulationDensity") else {
        return Vec::new();
    };
    let by_hash = |h: u32| {
        c.fields
            .iter()
            .find(|f| f.name_hash == h)
            .map(|f| usize::from(f.offset))
    };
    let at = [
        by_hash(DENSITY_VEHICLES),
        by_hash(DENSITY_PEOPLE),
        by_hash(DENSITY_VEHICLE_LIST),
        by_hash(DENSITY_PEOPLE_LIST),
        field_at(c, "Priority"),
        field_at(c, "faction"),
    ];
    let lines = layer.hashes("LineRegion");
    let placements = layer.placements();
    let names = layer.names();
    let mut out: Vec<DensityZone> = c
        .records()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(entity, r)| {
            let [v, p, vl, pl, pr, f] = at.map(|o| o.and_then(|o| word(r, o)).unwrap_or(0));
            let place = placements.get(&entity)?;
            let corners = shape(*lines.get(&entity)?)?;
            let (centre, q) = (
                Vec3::from(place.position),
                Quat::from_array(place.rotation).normalize(),
            );
            let outline = corners
                .iter()
                .map(|&[x, z]| {
                    let w = centre + q * Vec3::new(x, 0.0, z);
                    Vec2::new(w.x, w.z)
                })
                .collect();
            Some(DensityZone {
                entity,
                name: names.get(&entity).copied().unwrap_or("").to_owned(),
                layer: layer_hash,
                static_layer,
                outline,
                vehicles: v,
                people: p,
                vehicle_list: vl,
                people_list: pl,
                priority: pr,
                faction: f,
            })
        })
        .collect();
    out.sort_unstable_by_key(|z| z.entity);
    out
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
    fn seats_are_named_by_their_hash() {
        // The install's hashes.
        assert_eq!(seat_name(0xf8b8_325f), "seat_driver");
        assert_eq!(seat_name(0x1b21_7b8a), "seat_gunner_rearcenter");
        assert_eq!(seat_name(0xd055_9110), "d0559110");
    }

    #[test]
    fn the_zone_of_highest_priority_rules() {
        let square = |x0: f32, z0: f32, side: f32| {
            vec![
                Vec2::new(x0, z0),
                Vec2::new(x0 + side, z0),
                Vec2::new(x0 + side, z0 + side),
                Vec2::new(x0, z0 + side),
            ]
        };
        let zone = |entity, outline, priority| DensityZone {
            entity,
            name: String::new(),
            layer: 0,
            static_layer: false,
            outline,
            vehicles: 6,
            people: 2,
            vehicle_list: 0,
            people_list: 0,
            priority,
            faction: 0,
        };
        // An island-wide zone of 5 and a dead one of 100 inside it.
        let zones = [
            zone(1, square(0.0, 0.0, 1000.0), 5),
            zone(2, square(100.0, 100.0, 50.0), 100),
        ];
        assert_eq!(zone_at(&zones, 500.0, 500.0).map(|z| z.entity), Some(1));
        assert_eq!(zone_at(&zones, 120.0, 130.0).map(|z| z.entity), Some(2));
        assert_eq!(zone_at(&zones, -5.0, 10.0), None);
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
