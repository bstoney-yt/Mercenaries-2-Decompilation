//! What a vehicle's driver is to the AI: the class the engine gives a
//! character by the seat they sit in.
//!
//! # Provenance
//!
//! From the 360 code (`0x822BB608`, which sets a character's class as
//! it is attached to a seat): the seat is looked up in the component
//! tables in this order, and the first that has it decides:
//! `ControllerCar` 1, `ControllerTank` 2, `ControllerHelicopter` 3,
//! `ControllerBoat` 4. A seat in none of them is 7 if it has the turret
//! component with a record linked to it, else 6; no seat is 0. The
//! component names are the tables' own (`0x823B0C88`, `0x823B1000`,
//! `0x823B1DE0`, `0x823B0E40`).
//!
//! From the data (the `vehicleclasses` example): the four controllers
//! are on the vehicles' driver seat templates (`2 Seat Car (Driver)`,
//! `Tank Seat (Driver) (VZ)`, `2 Seat Copter (Driver)`, `Boat Seats
//! (Driver) (VZ)`: 48, 17, 32 and 29 of them), not on the vehicles. So
//! a driver or pilot has the vehicle's class, and a gunner or passenger
//! has 6 or 7.
//!
//! The class is what the score's action level groups hostile characters
//! by (the game's `music_machine::group`).
use std::collections::HashMap;

use m2_assets::Layer;

use crate::tuning::Table;

/// On foot.
pub const NONE: u8 = 0;
pub const CAR: u8 = 1;
pub const TANK: u8 = 2;
pub const HELICOPTER: u8 = 3;
pub const BOAT: u8 = 4;
/// A seat with none of the four controllers and no turret of its own: a
/// passenger's.
pub const OTHER: u8 = 6;
/// A seat with a turret: a gunner's.
pub const TURRET: u8 = 7;

/// The controller components, in the order the engine asks.
const CONTROLLERS: [(&str, u8); 4] = [
    ("ControllerCar", CAR),
    ("ControllerTank", TANK),
    ("ControllerHelicopter", HELICOPTER),
    ("ControllerBoat", BOAT),
];

/// The first class of `has` in the engine's order.
pub fn class_of(has: impl Fn(&str) -> bool) -> u8 {
    CONTROLLERS
        .iter()
        .find(|(name, _)| has(name))
        .map_or(OTHER, |c| c.1)
}

/// `SeatLink`'s field naming the seat's template (as in
/// [`crate::seats`]).
const LINK: u32 = 0x5470_4554;

/// Every seat template with a controller component, by id, and its
/// class.
pub fn read_seats(db: &Layer<'_>) -> HashMap<u32, u8> {
    let mut out = HashMap::new();
    // Last to first, so that an earlier controller's class stands.
    for (name, class) in CONTROLLERS.iter().rev() {
        for id in Table::new(db, name).templates() {
            out.insert(id, *class);
        }
    }
    out
}

/// The class of each vehicle template's driver, by the vehicle's id: its
/// driver seat's (the seat its `SeatLink` names whose `SeatParameters`
/// say `bIsDriverSeat`). A vehicle with no such seat has no entry.
pub fn read(db: &Layer<'_>) -> HashMap<u32, u8> {
    let seats = read_seats(db);
    let links = Table::new(db, "SeatLink");
    let params = Table::new(db, "SeatParameters");
    let is_driver = m2_assets::name_hash("bIsDriverSeat");
    links
        .templates()
        .filter_map(|vehicle| {
            links.of_template(vehicle).iter().find_map(|r| {
                let seat = links.word(r, LINK).filter(|&s| s != 0)?;
                let p = *params.of_template(seat).first()?;
                params
                    .word(p, is_driver)
                    .is_some_and(|b| b != 0)
                    .then(|| (vehicle, seats.get(&seat).copied().unwrap_or(OTHER)))
            })
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_controller_in_the_engine_s_order_decides() {
        assert_eq!(class_of(|n| n == "ControllerTank"), TANK);
        assert_eq!(class_of(|n| n == "ControllerHelicopter"), HELICOPTER);
        assert_eq!(class_of(|n| n == "ControllerBoat"), BOAT);
        // A car's controller is asked for first.
        assert_eq!(
            class_of(|n| n == "ControllerBoat" || n == "ControllerCar"),
            CAR
        );
        assert_eq!(
            class_of(|n| n == "ControllerTank" || n == "ControllerBoat"),
            TANK
        );
        assert_eq!(class_of(|_| false), OTHER);
    }
}
