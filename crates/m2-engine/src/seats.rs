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

use std::collections::HashMap;

use m2_assets::layer::Layer;
use m2_assets::name_hash;

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
    fn an_exit_naming_no_entrance_of_the_vehicle_gives_no_door() {
        let (seats, _) = valiant();
        let p = choose(&seats, |s| s == DRIVER_TPL, rows, &[]).unwrap();
        assert_eq!(p.door, None);
    }
}
