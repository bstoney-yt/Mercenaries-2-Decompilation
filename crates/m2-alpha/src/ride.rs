//! Which of a vehicle's seats she rides in, and what she does there.
//!
//! The use key picks an entrance ([`crate::game`]'s `entrance_pick`); this
//! module seats her by it:
//! - From the 360 code: the seat is the first free one she may use that
//!   the entrance leads to (0x8229A1C8); with none, and the entrance not
//!   hijack-only, the first hijackable one it leads to whose rider is of
//!   another faction than hers (0x8229A3C0, 0x82251E68). Every rider the
//!   alpha seats is of another faction than her PMC, so any rider of such
//!   a seat is taken out of it.
//! - From the scripts (hero.lua's `EnterPassengerCallback`, run whenever
//!   she sits down in a passenger's or gunner's seat): she moves over to
//!   the driver's seat along a `SeatToSeat` link, in a car or truck once
//!   its driver is put out, or in anything nobody drives; never from a
//!   gunner's seat with a friend or nobody at the wheel
//!   ([`m2_engine::seats::slide_over`]).
//! - From the data: a seat's role is its `SeatParameters`' (driver,
//!   gunner, else passenger); a gunner fires the vehicle's weapons of her
//!   seat (`VehicleArmament`'s seat), a passenger rides.
//! - Invented: in a gunner's or passenger's seat the vehicle stops (the
//!   alpha's riders don't drive it on); the seat key (Y) moves her over a
//!   `SeatToSeat` link to a free seat (0x822A6FF0's rule; no player button
//!   for it was found in the 360 code). A rider of a seat she takes or
//!   slides into is put out at once (the 360 waits for them to get out).

use m2_engine::seats::{self, DriverRider, Role, SlideOver, VehicleSeats};

use crate::npc::{People, faction_of};

/// A vehicle's seats as she gets in and moves about them: its template's
/// seat data and who rides which seat (`seat_driver`, `Guerilla Soldier`).
#[derive(Clone, Copy)]
pub struct Ride<'a> {
    pub seating: Option<&'a VehicleSeats>,
    pub crew: &'a [(String, String)],
}

impl<'a> Ride<'a> {
    pub fn of(v: &'a crate::scene::ParkedVehicle) -> Self {
        Self {
            seating: v.spec.seating.as_deref(),
            crew: &v.crew,
        }
    }

    /// Who rides the seat named `name`: their template.
    pub fn rider(&self, name: u32) -> Option<&'a str> {
        self.crew
            .iter()
            .find(|(seat, _)| m2_assets::name_hash(seat) == name)
            .map(|(_, who)| who.as_str())
    }

    /// The role of seat `seat` (`None`, or without seat data: the
    /// driver's).
    pub fn role(&self, seat: Option<u32>) -> Role {
        seat.and_then(|n| {
            let s = self.seating?;
            Some(s.seats[s.seat(n)?].role())
        })
        .unwrap_or(Role::Driver)
    }
}

/// Where she sits getting into a vehicle, and whose seats she empties.
#[derive(Clone, Debug, PartialEq)]
pub struct Boarding {
    /// Her seat's `SeatLink` name; `None` for a vehicle without seat data,
    /// whose driver's seat she takes (the beta's rule).
    pub seat: Option<u32>,
    pub role: Role,
    /// The seats (`SeatLink` names) whose riders are put out; `None` for
    /// the driver's seat of a vehicle without seat data.
    pub out: Vec<Option<u32>>,
    /// The seat she sat down in first, when she then slid over.
    pub slid_from: Option<u32>,
    /// Why she did not slide over, as hero.lua prints it.
    pub stayed: Option<&'static str>,
}

/// Whether labels `labels` (a vehicle template's) hold `Car` or `Truck`
/// (`Object.HasLabel`).
pub fn car_or_truck(labels: &[u32]) -> bool {
    ["Car", "Truck"]
        .iter()
        .any(|l| labels.contains(&m2_assets::name_hash(l)))
}

/// Where she sits getting into `ride` by `entrance` (`None`: a vehicle
/// without entrance data), as the module's rules have it; `None` if the
/// entrance gives her no seat. `car_or_truck` as [`car_or_truck`]; `people`
/// say whether a driver's faction is hostile to her.
pub fn board(
    ride: Ride<'_>,
    entrance: Option<u32>,
    car_or_truck: bool,
    people: &People,
) -> Option<Boarding> {
    let (Some(e), Some(s)) = (entrance, ride.seating) else {
        return Some(Boarding {
            seat: None,
            role: Role::Driver,
            out: vec![None],
            slid_from: None,
            stayed: None,
        });
    };
    let taken = |i: usize| ride.rider(s.seats[i].name).is_some();
    let params = s.entrance(e).map(|e| e.params).unwrap_or_default();
    let mut out = Vec::new();
    let seat = match seats::seat_for_entrance(&s.seats, e, taken) {
        Some(i) => i,
        None if !params.hijack_only => {
            let i = seats::hijack_seat(&s.seats, e, taken)?;
            out.push(Some(s.seats[i].name));
            i
        }
        None => return None,
    };
    let mut b = Boarding {
        seat: Some(s.seats[seat].name),
        role: s.seats[seat].role(),
        out,
        slid_from: None,
        stayed: None,
    };
    if b.role == Role::Driver {
        return Some(b);
    }
    let driver = s
        .driver()
        .and_then(|d| ride.rider(s.seats[d].name))
        .map(|who| DriverRider {
            player: false,
            // Neutral (civilians, a faction at peace with her) counts as
            // friendly: `Ai.GetRelation` >= 0.
            friendly: faction_of(who).is_none_or(|f| !people.fights_for(f)),
        });
    match seats::slide_over(&s.seats, seat, car_or_truck, driver, params.ladder) {
        SlideOver::Slide { to, driver_out } => {
            if driver_out {
                b.out.push(Some(s.seats[to].name));
            }
            b.slid_from = b.seat;
            b.seat = Some(s.seats[to].name);
            b.role = s.seats[to].role();
        }
        SlideOver::Stay(why) => b.stayed = Some(why),
    }
    Some(b)
}

/// Puts the riders of `seats` out of `crew` (`None`: any driver's seat;
/// the crew forgets them: playtest 4, no car could be taken from traffic).
/// The templates of those put out.
pub fn empty_seats(crew: &mut Vec<(String, String)>, seats: &[Option<u32>]) -> Vec<String> {
    let goes = |seat: &str| {
        seats.iter().any(|s| match s {
            Some(name) => m2_assets::name_hash(seat) == *name,
            None => seat.to_ascii_lowercase().contains("driver"),
        })
    };
    let (out, kept) = std::mem::take(crew)
        .into_iter()
        .partition(|(seat, _)| goes(seat));
    *crew = kept;
    out.into_iter().map(|(_, who)| who).collect()
}

/// The index among a vehicle's armed seats (`armed`,
/// [`crate::combat::armed_seats`], the driver's first) that a rider of
/// seat `seat` fires from: 0 for the driver, the seat's own for a gunner
/// whose seat has weapons, else none ([`crate::combat::NO_GUN`]).
pub fn armed_index(ride: Ride<'_>, seat: Option<u32>, armed: &[u32]) -> usize {
    match ride.role(seat) {
        Role::Driver => 0,
        Role::Gunner => seat
            .and_then(|n| armed.iter().position(|&s| s == n))
            .unwrap_or(crate::combat::NO_GUN),
        Role::Passenger => crate::combat::NO_GUN,
    }
}

/// The seat she moves to from seat `from` (`None`: the driver's) at the
/// seat key: over its first `SeatToSeat` link to a free seat (0x822A6FF0).
pub fn transfer(ride: Ride<'_>, from: Option<u32>) -> Option<u32> {
    let s = ride.seating?;
    let from = match from {
        Some(n) => s.seat(n)?,
        None => s.driver()?,
    };
    let to = seats::transfer_to(&s.seats, from, |i| ride.rider(s.seats[i].name).is_some())?;
    Some(s.seats[to].name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_assets::name_hash;
    use m2_engine::seats::{Entrance, EntranceParams, EntranceToSeatRow, Seat};

    fn seat(name: &str, entrance: &str, driver: bool, gunner: bool, links: &[&str]) -> Seat {
        Seat {
            name: name_hash(name),
            hardpoint: None,
            driver,
            gunner,
            hijackable: !gunner,
            player_use: true,
            toss_speed: -1.0,
            min_ditch_speed: 5.0,
            rows: vec![EntranceToSeatRow {
                link_name: name_hash(entrance),
                can_enter: true,
                can_exit: true,
            }],
            links: links.iter().map(|l| name_hash(l)).collect(),
        }
    }

    /// The M151 .50 Cal's seats and entrances (round 1's data listing:
    /// the front right seat links over to the driver's, the rear gunner's
    /// links nowhere and isn't hijackable).
    fn m151() -> VehicleSeats {
        let seats = vec![
            seat(
                "seat_front_right",
                "entrance_front_right",
                false,
                false,
                &["seat_driver"],
            ),
            seat(
                "seat_gunner_rearcenter",
                "entrance_gunner",
                false,
                true,
                &[],
            ),
            seat("seat_driver", "entrance_driver", true, false, &[]),
        ];
        let entrances = ["entrance_front_right", "entrance_gunner", "entrance_driver"]
            .iter()
            .map(|e| Entrance {
                name: name_hash(e),
                hardpoint: None,
                params: EntranceParams::default(),
            })
            .collect();
        VehicleSeats { seats, entrances }
    }

    fn crew(riders: &[(&str, &str)]) -> Vec<(String, String)> {
        riders
            .iter()
            .map(|&(s, w)| (s.to_owned(), w.to_owned()))
            .collect()
    }

    fn hostile_vz() -> People {
        let mut p = People::default();
        p.set_hostile(["Vza"].into_iter().collect());
        p
    }

    fn enter(s: &VehicleSeats, riders: &[(String, String)], e: &str) -> Option<Boarding> {
        let ride = Ride {
            seating: Some(s),
            crew: riders,
        };
        board(ride, Some(name_hash(e)), true, &hostile_vz())
    }

    #[test]
    fn at_the_passenger_door_of_an_empty_jeep_she_slides_over_to_drive() {
        let b = enter(&m151(), &[], "entrance_front_right").unwrap();
        assert_eq!(b.seat, Some(name_hash("seat_driver")));
        assert_eq!(b.slid_from, Some(name_hash("seat_front_right")));
        assert_eq!(b.role, Role::Driver);
        assert!(b.out.is_empty());
    }

    #[test]
    fn at_the_gunners_seat_she_mans_the_gun() {
        let b = enter(&m151(), &[], "entrance_gunner").unwrap();
        assert_eq!(b.seat, Some(name_hash("seat_gunner_rearcenter")));
        assert_eq!(b.role, Role::Gunner);
        assert_eq!(
            b.stayed,
            Some("Transfer aborted - In gunner seat w/o driver")
        );
        // A friend at the wheel stays there and drives her.
        let friend = crew(&[("seat_driver", "Guerilla Soldier")]);
        let b = enter(&m151(), &friend, "entrance_gunner").unwrap();
        assert_eq!(b.role, Role::Gunner);
        assert!(b.out.is_empty());
    }

    #[test]
    fn a_foe_at_the_wheel_is_put_out_when_she_slides_over() {
        let foe = crew(&[("seat_driver", "VZ Soldier")]);
        let b = enter(&m151(), &foe, "entrance_front_right").unwrap();
        assert_eq!(b.seat, Some(name_hash("seat_driver")));
        assert_eq!(b.out, [Some(name_hash("seat_driver"))]);
        // Through the driver's door, the driver is hijacked.
        let b = enter(&m151(), &foe, "entrance_driver").unwrap();
        assert_eq!(b.seat, Some(name_hash("seat_driver")));
        assert_eq!(b.out, [Some(name_hash("seat_driver"))]);
        assert_eq!(b.slid_from, None);
    }

    #[test]
    fn a_crewed_gunners_seat_is_no_way_in() {
        let gunner = crew(&[("seat_gunner_rearcenter", "VZ Soldier")]);
        assert_eq!(enter(&m151(), &gunner, "entrance_gunner"), None);
    }

    #[test]
    fn without_seat_data_she_takes_the_drivers_seat() {
        let mut riders = crew(&[
            ("seat_driver", "VZ Soldier"),
            ("seat_front_right", "VZ Soldier"),
        ]);
        let ride = Ride {
            seating: None,
            crew: &riders,
        };
        let b = board(ride, None, true, &hostile_vz()).unwrap();
        assert_eq!((b.seat, b.role), (None, Role::Driver));
        assert_eq!(empty_seats(&mut riders, &b.out), ["VZ Soldier"]);
        assert_eq!(riders.len(), 1);
    }

    #[test]
    fn a_passenger_has_no_gun_and_the_seat_key_moves_her_over() {
        let s = VehicleSeats {
            seats: vec![
                seat("seat_driver", "entrance_driver", true, false, &[]),
                seat("seat_rear", "entrance_rear", false, false, &["seat_driver"]),
            ],
            entrances: Vec::new(),
        };
        let mut ride = Ride {
            seating: Some(&s),
            crew: &[],
        };
        let rear = Some(name_hash("seat_rear"));
        let armed = [name_hash("seat_driver")];
        assert_eq!(ride.role(rear), Role::Passenger);
        assert_eq!(armed_index(ride, rear, &armed), crate::combat::NO_GUN);
        assert_eq!(armed_index(ride, None, &armed), 0);
        assert_eq!(transfer(ride, rear), Some(name_hash("seat_driver")));
        let driven = crew(&[("seat_driver", "VZ Soldier")]);
        ride.crew = &driven;
        assert_eq!(transfer(ride, rear), None);
    }

    #[test]
    fn cars_and_trucks_are_known_by_their_labels() {
        assert!(car_or_truck(&[name_hash("Vehicle"), name_hash("Car")]));
        assert!(car_or_truck(&[name_hash("Truck")]));
        assert!(!car_or_truck(&[name_hash("Tank")]));
    }
}
