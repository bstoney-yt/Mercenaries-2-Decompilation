//! Where a vehicle's driver sits and gets out, from its template's seat
//! data rather than from fixed model markers.
//!
//! A vehicle template links its seats (`SeatLink`: the seat's template,
//! the `Hardpoint` node it sits at and its `Name`, `seat_driver`) and its
//! entrances (`EntranceLink`: the entrance's template, `Hardpoint` and
//! `Name`). A seat's template says what the seat is (`SeatParameters`,
//! `bIsDriverSeat`) and through which entrances it is got in and out
//! (`EntranceToSeat`: `EntranceLinkName`, `CanEnter`, `CanExit`).
//!
//! Provenance:
//! - From the 360 code: the driver's seat is the first `SeatLink` of the
//!   vehicle whose seat template has `bIsDriverSeat` (0x8229C800, the
//!   `GetSeatByType "d"` native's search); a seat is got out of through the
//!   first `EntranceToSeat` row of its template with `CanExit` that names one
//!   of the vehicle's `EntranceLink`s (0x8229C190, used by every exit).
//!   The component and field names are the exe's reflection strings
//!   (0x823F1988 to 0x823F1FF8).
//! - From samples: on every car and truck read, `seat_driver` sits at node
//!   `d721fc8c` (model x +0.42 on the 68 Valiant) and is got out of through
//!   `entrance_driver` at node `8869e876`, whose `EntranceDirection` is
//!   Left. The beta's former fixed markers (`d6ebe292`, `688c4858`) are
//!   `seat_front_right` and `entrance_front_right` (direction Right).
//! - From the beta's own render (round PR3): traffic drivers drawn at the
//!   data's `seat_driver` hardpoint sit on the car's left, behind the wheel.
//!
//! Getting in and out ([`VehicleSeats`], round seat-entrance):
//! - From the 360 code: the entrances she may use are gathered each tick
//!   (0x82440F30, called last by the player controller's update 0x824386F0)
//!   from every entrance whose `UseDistance` is above 0 (0x823FEC00 files
//!   only those), of a vehicle not destroyed, in front of the camera
//!   (`aim · (entrance − camera) > 0`) and nearer her than `UseDistance` +
//!   1 m ([`SEARCH_PAD`]), at most 16 ([`LIST_CAP`]); a hijack-only one only
//!   while a script has turned it on. Of those, 0x82299E48 takes the nearest
//!   ([`pick`]); its seat is the first in `SeatLink` order she may use,
//!   free, with a `CanEnter` row for that entrance (0x8229A1C8,
//!   [`seat_for_entrance`]). On getting out, 0x822A2B60 chooses how
//!   ([`exit_kind`]); a ditch or toss puts her at the exit entrance's
//!   hardpoint (0x822A1B70) and pushes her off ([`ditch_velocity`]).
//! - Invented: where an animated exit ends. The exit clip is played from
//!   the entrance (0x822A2298) and its end is not read; the alpha, which
//!   plays no clip, puts her at that entrance's hardpoint as a ditch does.

use std::collections::HashMap;

use m2_assets::layer::Layer;
use m2_assets::name_hash;
use m2_math::glam::Vec3;

use crate::tuning::Table;

/// `SeatLink`'s and `EntranceLink`'s field naming the linked template (the
/// same key as `PhysicalLink`'s link field; its name is unknown).
const LINK: u32 = 0x5470_4554;

/// Where a vehicle template's driver sits and gets out, as node name hashes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriverPoints {
    /// The driver seat's `SeatLink` `Hardpoint`.
    pub seat: u32,
    /// The `Hardpoint` of the entrance the driver gets out through, when the
    /// data gives one.
    pub door: Option<u32>,
}

/// One `SeatLink` of a vehicle template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeatLinkRow {
    pub seat_template: u32,
    pub hardpoint: Option<u32>,
}

/// One `EntranceToSeat` row of a seat template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntranceToSeatRow {
    pub link_name: u32,
    pub can_enter: bool,
    pub can_exit: bool,
}

/// The driver's points of one vehicle: the first seat (in `SeatLink` order)
/// whose template is a driver's seat and gives a hardpoint, and of that
/// seat's `EntranceToSeat` rows the first with `CanExit` naming one of the
/// vehicle's entrances (`entrances`: each `EntranceLink`'s name and
/// hardpoint). `None` when no seat is a driver's.
///
/// A name the vehicle's `EntranceLink` records give twice is the last
/// record's (inferred from samples: the Valiant's and the Pony's records
/// read `entrance_driver` first at the front right door node `688c4858`,
/// then at `8869e876`, the side its `EntranceDirection` Left and the
/// on-screen driver are on; how the 360 flattens a template's inherited
/// links was not traced).
pub fn choose(
    seats: &[SeatLinkRow],
    is_driver: impl Fn(u32) -> bool,
    entrances_of: impl Fn(u32) -> Vec<EntranceToSeatRow>,
    entrances: &[(u32, Option<u32>)],
) -> Option<DriverPoints> {
    let seat = seats.iter().find(|s| is_driver(s.seat_template))?;
    let door = entrances_of(seat.seat_template)
        .iter()
        .filter(|row| row.can_exit)
        .find_map(|row| {
            entrances
                .iter()
                .rev()
                .find(|(name, _)| *name == row.link_name)
        })
        .and_then(|(_, hardpoint)| *hardpoint);
    Some(DriverPoints {
        seat: seat.hardpoint?,
        door,
    })
}

/// Every vehicle template's driver points, by template id.
pub fn read(db: &Layer<'_>) -> HashMap<u32, DriverPoints> {
    let seat_links = Table::new(db, "SeatLink");
    let entrance_links = Table::new(db, "EntranceLink");
    let params = Table::new(db, "SeatParameters");
    let to_seat = Table::new(db, "EntranceToSeat");
    let (hardpoint, name) = (name_hash("Hardpoint"), name_hash("Name"));
    let driver_bit = name_hash("bIsDriverSeat");
    let (link_name, can_enter, can_exit) = (
        name_hash("EntranceLinkName"),
        name_hash("CanEnter"),
        name_hash("CanExit"),
    );
    let nonzero = |v: Option<u32>| v.filter(|&v| v != 0);
    let is_driver = |seat: u32| {
        params
            .of_template(seat)
            .first()
            .and_then(|r| params.word(r, driver_bit))
            .is_some_and(|b| b != 0)
    };
    let entrances_of = |seat: u32| {
        to_seat
            .of_template(seat)
            .iter()
            .filter_map(|r| {
                Some(EntranceToSeatRow {
                    link_name: nonzero(to_seat.word(r, link_name))?,
                    can_enter: to_seat.word(r, can_enter).is_some_and(|b| b != 0),
                    can_exit: to_seat.word(r, can_exit).is_some_and(|b| b != 0),
                })
            })
            .collect()
    };
    let mut out = HashMap::new();
    for vehicle in seat_links.templates() {
        let seats: Vec<SeatLinkRow> = seat_links
            .of_template(vehicle)
            .iter()
            .filter_map(|r| {
                Some(SeatLinkRow {
                    seat_template: nonzero(seat_links.word(r, LINK))?,
                    hardpoint: nonzero(seat_links.word(r, hardpoint)),
                })
            })
            .collect();
        let entrances: Vec<(u32, Option<u32>)> = entrance_links
            .of_template(vehicle)
            .iter()
            .filter_map(|r| {
                Some((
                    nonzero(entrance_links.word(r, name))?,
                    nonzero(entrance_links.word(r, hardpoint)),
                ))
            })
            .collect();
        if let Some(points) = choose(&seats, is_driver, entrances_of, &entrances) {
            out.insert(vehicle, points);
        }
    }
    out
}

/// How far past an entrance's `UseDistance` she may stand and still be
/// offered it, metres (0x82440F30's 1.0 at 0x820BAEE8).
pub const SEARCH_PAD: f32 = 1.0;
/// The pad of a `bLongSearchDistance` entrance while she is in the air
/// near a fast boat or a ladder (25.0 at 0x82B75FD0; the alpha does not
/// use it: its conditions are only partly traced).
pub const LONG_SEARCH_PAD: f32 = 25.0;
/// How many entrances the search keeps (0x82440F30 stops at 16).
pub const LIST_CAP: usize = 16;
/// A vehicle whose up axis has less height than this is not got into
/// (0x82299E48) and is crawled out of (0x822A2B60): 0.5, tipped past 60°.
pub const UPRIGHT_MIN_Y: f32 = 0.5;
/// The ditch speed of a seat without `SeatParameters`, m/s (the 360
/// compares 25.0 at 0x820F02E4 with the speed squared).
pub const DEFAULT_DITCH_SPEED: f32 = 5.0;

/// An entrance template's `EntranceParameters`, as the search and exits
/// use them (defaults: the exe's reflection defaults).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntranceParams {
    pub use_distance: f32,
    pub hijack_only: bool,
    pub boarding: bool,
    pub ladder: bool,
    pub long_search: bool,
    pub speed_match: bool,
}

impl Default for EntranceParams {
    fn default() -> Self {
        Self {
            use_distance: 1.0,
            hijack_only: false,
            boarding: false,
            ladder: false,
            long_search: false,
            speed_match: false,
        }
    }
}

impl EntranceParams {
    /// Whether getting out through it skips the exit kinds and keeps the
    /// vehicle's speed (0x822A2B60: boarding, ladder or speed-match).
    pub fn keeps_speed(&self) -> bool {
        self.boarding || self.ladder || self.speed_match
    }
}

/// One entrance of a vehicle: its `EntranceLink` name and hardpoint and
/// its template's parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entrance {
    pub name: u32,
    pub hardpoint: Option<u32>,
    pub params: EntranceParams,
}

/// One seat of a vehicle: its `SeatLink` and its template's
/// `SeatParameters` and `EntranceToSeat` rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Seat {
    pub name: u32,
    pub hardpoint: Option<u32>,
    pub driver: bool,
    /// `bCanPlayerUse` (default 1).
    pub player_use: bool,
    /// `TossSpeedChange`, m/s; at or below 0 the seat never tosses.
    pub toss_speed: f32,
    /// `MinDitchSpeed`, m/s.
    pub min_ditch_speed: f32,
    pub rows: Vec<EntranceToSeatRow>,
}

/// A vehicle template's seats and entrances, in link order. An entrance
/// name given twice keeps the last record (as [`choose`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VehicleSeats {
    pub seats: Vec<Seat>,
    pub entrances: Vec<Entrance>,
}

impl VehicleSeats {
    pub fn entrance(&self, name: u32) -> Option<&Entrance> {
        self.entrances.iter().find(|e| e.name == name)
    }

    /// The driver's seat (0x8229C800: the first marked so).
    pub fn driver(&self) -> Option<usize> {
        self.seats.iter().position(|s| s.driver)
    }

    /// The entrance seat `seat` is got out through: its first `CanExit`
    /// row naming one of the vehicle's entrances (0x8229C190).
    pub fn exit_of(&self, seat: usize) -> Option<&Entrance> {
        self.seats
            .get(seat)?
            .rows
            .iter()
            .filter(|r| r.can_exit)
            .find_map(|r| self.entrance(r.link_name))
    }
}

/// Whether an entrance at `at` with `use_distance` is gathered for her at
/// `her`, the camera at `eye` looking along `aim` (0x82440F30): it must
/// have a use distance, lie ahead of the camera and within `use_distance`
/// + `pad` of her (strictly).
pub fn in_search(her: Vec3, eye: Vec3, aim: Vec3, at: Vec3, use_distance: f32, pad: f32) -> bool {
    let reach = use_distance + pad;
    use_distance > 0.0 && aim.dot(at - eye) > 0.0 && her.distance_squared(at) < reach * reach
}

/// Whether an entrance is offered for plain entry (0x8229CA90, no script
/// override): of a usable vehicle, any but a hijack-only one; of one the
/// scripts made unusable, only ladders and boarding entrances.
pub fn offered(p: &EntranceParams, vehicle_usable: bool) -> bool {
    if vehicle_usable {
        !p.hijack_only
    } else {
        p.ladder || p.boarding
    }
}

/// One gathered entrance as the pick sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    /// Which vehicle it belongs to.
    pub vehicle: usize,
    pub distance_sq: f32,
    pub use_distance: f32,
    /// The vehicle's up axis is at least [`UPRIGHT_MIN_Y`] high.
    pub upright: bool,
    /// [`offered`], and for a ladder, a path to its seat.
    pub offered: bool,
}

/// The entrance she gets in by, of those gathered, in their order
/// (0x82299E48): the nearest, a tie going to the smaller `UseDistance`;
/// but once one on the vehicle she last used is taken, only another on
/// that vehicle can replace it, and one on it replaces any other.
pub fn pick(candidates: &[Candidate], last: Option<usize>) -> Option<usize> {
    let mut best: Option<(usize, usize, f32, f32)> = None;
    for (i, c) in candidates.iter().enumerate() {
        if !c.upright || !c.offered {
            continue;
        }
        let (bv, bd, bu) = best.map_or((None, 1.0e30, 1.0e30), |(_, v, d, u)| (Some(v), d, u));
        let on_last = |v: Option<usize>| v.is_some() && v == last;
        let nearer = c.distance_sq < bd || (c.distance_sq == bd && c.use_distance < bu);
        let same_standing = bv == Some(c.vehicle)
            || (!on_last(bv) && !on_last(Some(c.vehicle)))
            || (bv.is_none() && last.is_none());
        if (!on_last(bv) && on_last(Some(c.vehicle))) || (nearer && same_standing) {
            best = Some((i, c.vehicle, c.distance_sq, c.use_distance));
        }
    }
    best.map(|(i, ..)| i)
}

/// The seat she takes through entrance `entrance` (0x8229A1C8): the first
/// in `SeatLink` order that she may use, is not `taken`, and has a
/// `CanEnter` row naming the entrance.
pub fn seat_for_entrance(
    seats: &[Seat],
    entrance: u32,
    taken: impl Fn(usize) -> bool,
) -> Option<usize> {
    seats.iter().enumerate().position(|(i, s)| {
        s.player_use
            && !taken(i)
            && s.rows
                .iter()
                .any(|r| r.can_enter && r.link_name == entrance)
    })
}

/// How she leaves a seat (0x822A2B60's result).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitKind {
    /// 0: the exit clip at the entrance.
    Animated,
    /// 1: out of a vehicle on its side or roof.
    Crawl,
    /// 2: a bail-out at speed, or from a helicopter in the air.
    Ditch,
    /// 3: thrown off (bikes, boats).
    Toss,
    /// 5: the way out is blocked; she is put at a clear point.
    Snap,
}

/// How she gets out of `seat` through `exit`, the vehicle moving at
/// `speed` m/s with its up axis `up_y` high; `airborne` for a helicopter
/// whose landed share is 0.5 or less; `blocked` when the sweep from her to
/// the exit hits something (0x822A2B60).
pub fn exit_kind(
    seat: Option<&Seat>,
    exit: Option<&EntranceParams>,
    speed: f32,
    airborne: bool,
    up_y: f32,
    blocked: bool,
) -> ExitKind {
    if !exit.is_some_and(EntranceParams::keeps_speed) {
        if let Some(s) = seat
            && speed > 0.0
            && s.toss_speed > 0.0
            && s.toss_speed < speed
        {
            return ExitKind::Toss;
        }
        let ditch = seat.map_or(DEFAULT_DITCH_SPEED, |s| s.min_ditch_speed);
        if ditch * ditch < speed * speed || airborne {
            return ExitKind::Ditch;
        }
    }
    if up_y < UPRIGHT_MIN_Y {
        ExitKind::Crawl
    } else if blocked {
        ExitKind::Snap
    } else {
        ExitKind::Animated
    }
}

/// Her speed as she ditches out (0x822A1B70): 2 m/s along the vehicle's
/// way of travel, 1.5 up and 1.5 out through the exit (`out`, from the
/// vehicle towards her).
pub fn ditch_velocity(vehicle_velocity: Vec3, out: Vec3) -> Vec3 {
    vehicle_velocity.normalize_or_zero() * 2.0 + Vec3::Y * 1.5 + out.normalize_or_zero() * 1.5
}

/// Every vehicle template's seats and entrances, by template id.
pub fn read_vehicles(db: &Layer<'_>) -> HashMap<u32, VehicleSeats> {
    let seat_links = Table::new(db, "SeatLink");
    let entrance_links = Table::new(db, "EntranceLink");
    let seat_params = Table::new(db, "SeatParameters");
    let entrance_params = Table::new(db, "EntranceParameters");
    let to_seat = Table::new(db, "EntranceToSeat");
    let h = name_hash;
    let (hardpoint, name) = (h("Hardpoint"), h("Name"));
    let nonzero = |v: Option<u32>| v.filter(|&v| v != 0);
    let bit = |t: &Table<'_>, r: &[u8], f: &str| t.word(r, h(f)).is_some_and(|b| b != 0);
    let rows_of = |seat: u32| -> Vec<EntranceToSeatRow> {
        to_seat
            .of_template(seat)
            .iter()
            .filter_map(|r| {
                Some(EntranceToSeatRow {
                    link_name: nonzero(to_seat.word(r, h("EntranceLinkName")))?,
                    can_enter: bit(&to_seat, r, "CanEnter"),
                    can_exit: bit(&to_seat, r, "CanExit"),
                })
            })
            .collect()
    };
    let params_of = |entrance: u32| {
        let d = EntranceParams::default();
        let Some(r) = entrance_params.of_template(entrance).first() else {
            return d;
        };
        EntranceParams {
            use_distance: entrance_params
                .float(r, h("UseDistance"))
                .unwrap_or(d.use_distance),
            hijack_only: bit(&entrance_params, r, "bHijackOnly"),
            boarding: bit(&entrance_params, r, "bBoarding"),
            ladder: bit(&entrance_params, r, "bLadder"),
            long_search: bit(&entrance_params, r, "bLongSearchDistance"),
            speed_match: bit(&entrance_params, r, "bSpeedMatchOnExit"),
        }
    };
    let mut out = HashMap::new();
    for vehicle in seat_links.templates() {
        let seats = seat_links
            .of_template(vehicle)
            .iter()
            .filter_map(|r| {
                let template = nonzero(seat_links.word(r, LINK))?;
                let p = seat_params.of_template(template).first().copied();
                let float =
                    |f: &str, d: f32| p.and_then(|p| seat_params.float(p, h(f))).unwrap_or(d);
                Some(Seat {
                    name: seat_links.word(r, name).unwrap_or(0),
                    hardpoint: nonzero(seat_links.word(r, hardpoint)),
                    driver: p.is_some_and(|p| bit(&seat_params, p, "bIsDriverSeat")),
                    player_use: p.is_none_or(|p| {
                        seat_params
                            .word(p, h("bCanPlayerUse"))
                            .is_none_or(|b| b != 0)
                    }),
                    toss_speed: float("TossSpeedChange", -1.0),
                    min_ditch_speed: float("MinDitchSpeed", DEFAULT_DITCH_SPEED),
                    rows: rows_of(template),
                })
            })
            .collect();
        let mut entrances: Vec<Entrance> = Vec::new();
        for r in entrance_links.of_template(vehicle) {
            let Some(n) = nonzero(entrance_links.word(r, name)) else {
                continue;
            };
            let e = Entrance {
                name: n,
                hardpoint: nonzero(entrance_links.word(r, hardpoint)),
                params: nonzero(entrance_links.word(r, LINK))
                    .map(params_of)
                    .unwrap_or_default(),
            };
            match entrances.iter_mut().find(|x| x.name == n) {
                Some(x) => *x = e,
                None => entrances.push(e),
            }
        }
        out.insert(vehicle, VehicleSeats { seats, entrances });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRIVER_TPL: u32 = 0x8000_63dc;
    const RIGHT_TPL: u32 = 0x8000_63dd;

    /// The 68 Valiant (4 door)'s data as read (round PR3), less its rear
    /// seats: `seat_front_right` linked first, `seat_driver` after.
    fn valiant() -> (Vec<SeatLinkRow>, Vec<(u32, Option<u32>)>) {
        let seats = vec![
            SeatLinkRow {
                seat_template: RIGHT_TPL,
                hardpoint: Some(0xd6eb_e292),
            },
            SeatLinkRow {
                seat_template: DRIVER_TPL,
                hardpoint: Some(0xd721_fc8c),
            },
        ];
        let entrances = vec![
            (name_hash("entrance_front_right"), Some(0x688c_4858)),
            (name_hash("entrance_driver"), Some(0x8869_e876)),
        ];
        (seats, entrances)
    }

    fn rows(seat: u32) -> Vec<EntranceToSeatRow> {
        let (name, enter, exit) = match seat {
            DRIVER_TPL => ("entrance_driver", true, true),
            _ => ("entrance_front_right", true, true),
        };
        vec![EntranceToSeatRow {
            link_name: name_hash(name),
            can_enter: enter,
            can_exit: exit,
        }]
    }

    #[test]
    fn the_driver_is_the_seat_marked_so_not_the_first_linked() {
        let (seats, entrances) = valiant();
        let p = choose(&seats, |s| s == DRIVER_TPL, rows, &entrances).unwrap();
        assert_eq!(p.seat, 0xd721_fc8c);
        assert_eq!(p.door, Some(0x8869_e876));
    }

    #[test]
    fn the_former_fixed_markers_are_the_front_right_seat_and_door() {
        // The beta placed her by `d6ebe292` and `688c4858`: in the data
        // those are `seat_front_right` and its entrance.
        let (seats, entrances) = valiant();
        let right = choose(&seats, |s| s == RIGHT_TPL, rows, &entrances).unwrap();
        assert_eq!(right.seat, 0xd6eb_e292);
        assert_eq!(right.door, Some(0x688c_4858));
        let driver = choose(&seats, |s| s == DRIVER_TPL, rows, &entrances).unwrap();
        assert_ne!(driver, right);
    }

    #[test]
    fn the_door_is_the_first_entrance_got_out_through() {
        // The Cigarette boat's driver gets in by `entrance_driver` (enter
        // only) and out by `exit_driver`.
        let seats = [SeatLinkRow {
            seat_template: DRIVER_TPL,
            hardpoint: Some(1),
        }];
        let entrances = [
            (name_hash("entrance_driver"), Some(10)),
            (name_hash("exit_driver"), Some(11)),
        ];
        let rows = |_| {
            vec![
                EntranceToSeatRow {
                    link_name: name_hash("entrance_driver"),
                    can_enter: true,
                    can_exit: false,
                },
                EntranceToSeatRow {
                    link_name: name_hash("exit_driver"),
                    can_enter: false,
                    can_exit: true,
                },
            ]
        };
        let p = choose(&seats, |_| true, rows, &entrances).unwrap();
        assert_eq!(p.door, Some(11));
    }

    #[test]
    fn a_link_name_given_twice_is_the_last_records() {
        // The Valiant's `EntranceLink` records as read (round PR3).
        let (seats, _) = valiant();
        let (driver, right) = (
            name_hash("entrance_driver"),
            name_hash("entrance_front_right"),
        );
        let entrances = [
            (driver, Some(0x688c_4858)),
            (right, Some(0x8869_e876)),
            (driver, Some(0x8869_e876)),
            (right, Some(0x688c_4858)),
        ];
        let p = choose(&seats, |s| s == DRIVER_TPL, rows, &entrances).unwrap();
        assert_eq!(p.door, Some(0x8869_e876));
    }

    #[test]
    fn no_driver_seat_gives_nothing() {
        let (seats, entrances) = valiant();
        assert_eq!(choose(&seats, |_| false, rows, &entrances), None);
    }

    #[test]
    fn entrance_fields_are_the_ones_the_data_holds() {
        // The hijack round's entrance records: ffc54ecf the radius,
        // 292051f3 the flag set on tanks.
        assert_eq!(name_hash("UseDistance"), 0xffc5_4ecf);
        assert_eq!(name_hash("bHijackOnly"), 0x2920_51f3);
    }

    fn cand(vehicle: usize, d: f32, use_distance: f32) -> Candidate {
        Candidate {
            vehicle,
            distance_sq: d * d,
            use_distance,
            upright: true,
            offered: true,
        }
    }

    #[test]
    fn the_search_reaches_use_distance_and_a_metre() {
        let (eye, aim) = (Vec3::new(0.0, 1.6, -3.0), Vec3::Z);
        let at = |z: f32| Vec3::new(0.0, 0.0, z);
        // UseDistance 1 (cars): within 2 m, strictly.
        assert!(in_search(Vec3::ZERO, eye, aim, at(1.99), 1.0, SEARCH_PAD));
        assert!(!in_search(Vec3::ZERO, eye, aim, at(2.0), 1.0, SEARCH_PAD));
        // UseDistance 3 (a helicopter's): within 4 m.
        assert!(in_search(Vec3::ZERO, eye, aim, at(3.9), 3.0, SEARCH_PAD));
        // No use distance: never gathered.
        assert!(!in_search(Vec3::ZERO, eye, aim, at(0.1), 0.0, SEARCH_PAD));
        // Behind the camera: not gathered, however near her.
        assert!(!in_search(Vec3::ZERO, eye, aim, at(-3.5), 1.0, 10.0));
    }

    #[test]
    fn hijack_only_entrances_are_not_offered_to_walk_in() {
        let mut p = EntranceParams::default();
        assert!(offered(&p, true));
        assert!(!offered(&p, false));
        p.hijack_only = true;
        assert!(!offered(&p, true));
        p.hijack_only = false;
        p.ladder = true;
        assert!(offered(&p, false));
    }

    #[test]
    fn the_nearest_entrance_is_picked_a_tie_to_the_smaller_use_distance() {
        let c = [cand(0, 1.5, 1.0), cand(1, 0.8, 1.0), cand(2, 0.8, 0.5)];
        assert_eq!(pick(&c, None), Some(2));
        assert_eq!(pick(&c[..2], None), Some(1));
        assert_eq!(pick(&[], None), None);
    }

    #[test]
    fn the_last_used_vehicle_keeps_the_pick() {
        // Her last vehicle's entrance is farther, yet taken; a nearer one
        // of another vehicle after it does not replace it.
        let c = [cand(0, 0.5, 1.0), cand(7, 1.8, 1.0), cand(3, 0.2, 1.0)];
        assert_eq!(pick(&c, Some(7)), Some(1));
        // Within it, the nearer of its own entrances.
        let c = [cand(7, 1.8, 1.0), cand(7, 1.0, 1.0), cand(3, 0.2, 1.0)];
        assert_eq!(pick(&c, Some(7)), Some(1));
        // Not among those gathered: plain nearest.
        assert_eq!(pick(&c, Some(9)), Some(2));
    }

    #[test]
    fn a_tipped_or_refused_vehicle_is_not_got_into() {
        let mut tipped = cand(0, 0.1, 1.0);
        tipped.upright = false;
        let mut refused = cand(1, 0.2, 1.0);
        refused.offered = false;
        assert_eq!(pick(&[tipped, refused, cand(2, 1.9, 1.0)], None), Some(2));
    }

    fn seat(name: &str, driver: bool, rows: &[(&str, bool, bool)]) -> Seat {
        Seat {
            name: name_hash(name),
            hardpoint: Some(name_hash(name)),
            driver,
            player_use: true,
            toss_speed: -1.0,
            min_ditch_speed: DEFAULT_DITCH_SPEED,
            rows: rows
                .iter()
                .map(|&(e, can_enter, can_exit)| EntranceToSeatRow {
                    link_name: name_hash(e),
                    can_enter,
                    can_exit,
                })
                .collect(),
        }
    }

    #[test]
    fn the_seat_is_the_first_free_one_the_entrance_leads_to() {
        // An M1A2's driver and gunner hatches.
        let seats = [
            seat("seat_driver", true, &[("entrance_driver", true, true)]),
            seat("seat_gunner", false, &[("entrance_gunner", true, true)]),
            seat("seat_rider", false, &[("entrance_gunner", true, false)]),
        ];
        let e = |n| name_hash(n);
        assert_eq!(
            seat_for_entrance(&seats, e("entrance_driver"), |_| false),
            Some(0)
        );
        assert_eq!(
            seat_for_entrance(&seats, e("entrance_gunner"), |_| false),
            Some(1)
        );
        assert_eq!(
            seat_for_entrance(&seats, e("entrance_gunner"), |i| i == 1),
            Some(2)
        );
        assert_eq!(
            seat_for_entrance(&seats, e("entrance_driver"), |i| i == 0),
            None
        );
        let mut barred = seats.clone();
        barred[0].player_use = false;
        assert_eq!(
            seat_for_entrance(&barred, e("entrance_driver"), |_| false),
            None
        );
    }

    #[test]
    fn a_seat_is_left_by_its_first_exit_entrance() {
        let v = VehicleSeats {
            seats: vec![seat(
                "seat_driver",
                true,
                &[
                    ("entrance_driver", true, false),
                    ("exit_driver", false, true),
                ],
            )],
            entrances: vec![
                Entrance {
                    name: name_hash("entrance_driver"),
                    hardpoint: Some(10),
                    params: EntranceParams::default(),
                },
                Entrance {
                    name: name_hash("exit_driver"),
                    hardpoint: Some(11),
                    params: EntranceParams::default(),
                },
            ],
        };
        assert_eq!(v.driver(), Some(0));
        assert_eq!(v.exit_of(0).and_then(|e| e.hardpoint), Some(11));
    }

    #[test]
    fn the_exit_kind_follows_speed_attitude_and_room() {
        let car = seat("seat_driver", true, &[]);
        let mut bike = car.clone();
        bike.toss_speed = 15.0;
        let plain = EntranceParams::default();
        let k = |s: &Seat, speed, airborne, up, blocked| {
            exit_kind(Some(s), Some(&plain), speed, airborne, up, blocked)
        };
        assert_eq!(k(&car, 0.0, false, 1.0, false), ExitKind::Animated);
        assert_eq!(k(&car, 5.0, false, 1.0, false), ExitKind::Animated);
        assert_eq!(k(&car, 5.01, false, 1.0, false), ExitKind::Ditch);
        assert_eq!(k(&bike, 15.0, false, 1.0, false), ExitKind::Ditch);
        assert_eq!(k(&bike, 15.01, false, 1.0, false), ExitKind::Toss);
        assert_eq!(k(&car, 0.0, true, 1.0, false), ExitKind::Ditch);
        assert_eq!(k(&car, 0.0, false, 0.49, false), ExitKind::Crawl);
        assert_eq!(k(&car, 0.0, false, 0.49, true), ExitKind::Crawl);
        assert_eq!(k(&car, 0.0, false, 0.5, true), ExitKind::Snap);
        // A speed-match entrance (boats' boarding) skips the speeds.
        let deck = EntranceParams {
            speed_match: true,
            ..plain
        };
        assert_eq!(
            exit_kind(Some(&car), Some(&deck), 20.0, false, 1.0, false),
            ExitKind::Animated
        );
        // No seat data: the 5 m/s default.
        assert_eq!(
            exit_kind(None, None, 6.0, false, 1.0, false),
            ExitKind::Ditch
        );
    }

    #[test]
    fn a_ditch_pushes_her_forward_up_and_out() {
        let v = ditch_velocity(Vec3::Z * 20.0, Vec3::X * 3.0);
        assert!((v - Vec3::new(1.5, 1.5, 2.0)).length() < 1e-6);
    }

    #[test]
    fn an_exit_naming_no_entrance_of_the_vehicle_gives_no_door() {
        let (seats, _) = valiant();
        let p = choose(&seats, |s| s == DRIVER_TPL, rows, &[]).unwrap();
        assert_eq!(p.door, None);
    }
}
