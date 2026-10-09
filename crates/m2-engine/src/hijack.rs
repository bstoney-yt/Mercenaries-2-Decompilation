//! Which vehicles can be hijacked, and how: the template database links a
//! vehicle to an entrance template (`Vehicle Entrance (Action Hijack)
//! (M1A2)`) through an `EntranceLink`, and the entrance's
//! `EntranceParameters` name the script that plays the hijack
//! (`hijack_m1a2`), where on the vehicle it starts (`DriverHatch`,
//! `front_left`) and from how far.
//!
//! A vehicle with no such link (the plain UH-1, every car) can't be
//! hijacked; its seats are only got into.
//!
//! The fields are known by hash only; what each holds was read from the
//! values the hijackable vehicles give (round H2): the script field holds
//! the name hash of a script in `vz.wad`, the point field a hardpoint name
//! hash, the radius metres (1 at a tank's hatch, 2-5 at a helicopter's
//! door).

use std::collections::HashMap;

use m2_assets::{Layer, name_hash};

use crate::tuning::Table;

/// `EntranceLink`'s field naming the entrance template (the same key as
/// `PhysicalLink`'s link field; its name is unknown).
const LINK: u32 = 0x5470_4554;
/// `EntranceParameters`' fields (names unknown).
const SCRIPT: u32 = 0x2c5d_34dc;
const POINT: u32 = 0xa841_3df8;
const RADIUS: u32 = 0xffc5_4ecf;
/// 0 on the helicopters' entrances (the AH-1Z's, Mi-35's, ...), 1 on the
/// rest; meaning unknown.
const FLAG: u32 = 0x2920_51f3;

/// A way to hijack a vehicle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HijackEntrance {
    /// The entrance template's id.
    pub entrance: u32,
    /// The name hash of the script that plays it (`hijack_m1a2`): its
    /// global `StartHijack(hijacker, hijackee, seat, vehicle)` starts it.
    pub script: u32,
    /// The name hash of the hardpoint the hijacker goes to
    /// (`DriverHatch`), when it gives one (the bank truck's doesn't).
    pub point: Option<u32>,
    /// How near the hijacker must be, metres.
    pub radius: f32,
    /// The link's own `Hardpoint` and `Name`, as name hashes.
    pub hardpoint: Option<u32>,
    pub name: Option<u32>,
    /// Field `292051f3` (see [`FLAG`]).
    pub flag: Option<u32>,
}

/// Every template's hijack entrances, by template id.
pub fn read(db: &Layer<'_>) -> HashMap<u32, Vec<HijackEntrance>> {
    let links = Table::new(db, "EntranceLink");
    let params = Table::new(db, "EntranceParameters");
    let nonzero = |v: Option<u32>| v.filter(|&v| v != 0);
    let mut out: HashMap<u32, Vec<HijackEntrance>> = HashMap::new();
    for vehicle in links.templates() {
        for r in links.of_template(vehicle) {
            let Some(entrance) = links.word(r, LINK) else {
                continue;
            };
            let Some(p) = params.of_template(entrance).first() else {
                continue;
            };
            let Some(script) = nonzero(params.word(p, SCRIPT)) else {
                continue;
            };
            out.entry(vehicle).or_default().push(HijackEntrance {
                entrance,
                script,
                point: nonzero(params.word(p, POINT)),
                radius: params.float(p, RADIUS).unwrap_or(0.0),
                hardpoint: nonzero(links.word(r, name_hash("Hardpoint"))),
                name: nonzero(links.word(r, name_hash("Name"))),
                flag: params.word(p, FLAG),
            });
        }
    }
    out
}

/// Every template's `VehicleAnimationSet`: the class and name (`Tank`,
/// `M1A2`, as name hashes) the animation tables pick a vehicle's
/// characters' clips by ([`m2_assets::animation_table::vehicle_clip`]).
pub fn animation_sets(db: &Layer<'_>) -> HashMap<u32, (u32, u32)> {
    let sets = Table::new(db, "VehicleAnimationSet");
    let (class, name) = (name_hash("VehicleClass"), name_hash("VehicleName"));
    let mut out = HashMap::new();
    for t in sets.templates() {
        if let Some(r) = sets.of_template(t).first()
            && let (Some(c), Some(n)) = (sets.word(r, class), sets.word(r, name))
        {
            out.insert(t, (c, n));
        }
    }
    out
}

/// Every template's starting part states, by template id: each
/// `OSMStateParameter`'s `Name` (the part's node name hash) and the state
/// it starts in (field `934a57ed`). Round J1 (2026-10-07): the soft-top
/// M151s put their gun-mount frame (`5164a412`) in state `46f79d30`, which
/// shows nothing; the gun versions so put their canvas top (`b8ed0265`).
pub fn part_states(db: &Layer<'_>) -> HashMap<u32, Vec<(u32, u32)>> {
    let params = Table::new(db, "OSMStateParameter");
    let (name, state) = (name_hash("Name"), 0x934a_57ed);
    let mut out: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
    for t in params.templates() {
        for r in params.of_template(t) {
            if let (Some(n), Some(s)) = (params.word(r, name), params.word(r, state)) {
                out.entry(t).or_default().push((n, s));
            }
        }
    }
    out
}

/// Every template's seats, by template id: each `SeatLink`'s `Name` (the
/// seat's, `seat_driver`) and `Hardpoint` (the node of the vehicle's
/// model it is at), as name hashes. Inferred: the same field names as
/// `EntranceLink`'s.
pub fn seat_links(db: &Layer<'_>) -> HashMap<u32, Vec<(u32, Option<u32>)>> {
    let links = Table::new(db, "SeatLink");
    let (name, point) = (name_hash("Name"), name_hash("Hardpoint"));
    let mut out: HashMap<u32, Vec<(u32, Option<u32>)>> = HashMap::new();
    for t in links.templates() {
        for r in links.of_template(t) {
            if let Some(n) = links.word(r, name).filter(|&n| n != 0) {
                let p = links.word(r, point).filter(|&p| p != 0);
                out.entry(t).or_default().push((n, p));
            }
        }
    }
    out
}

/// Every template's `SeatLink` records, by template id, each as every
/// field's name hash and word: what decides whether whoever sits in a
/// seat is seen (an enclosed seat, a tank's driver under his hatch) is not
/// known, so the whole record is logged to find it by.
pub fn seat_link_fields(db: &Layer<'_>) -> HashMap<u32, Vec<Vec<(u32, u32)>>> {
    let links = Table::new(db, "SeatLink");
    let fields = links.fields();
    let mut out: HashMap<u32, Vec<Vec<(u32, u32)>>> = HashMap::new();
    for t in links.templates() {
        for r in links.of_template(t) {
            let row = fields
                .iter()
                .filter_map(|&f| Some((f, links.word(r, f)?)))
                .collect();
            out.entry(t).or_default().push(row);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hijack_scripts_and_points_are_name_hashes() {
        // As round H2 read them off the M1A2's and UH-1's entrances.
        for (name, key) in [("hijack_m1a2", 0xa41a_ca7f), ("hijack_huey", 0xa370_3797)] {
            assert_eq!(name_hash(name), key, "{name}");
        }
    }
}
