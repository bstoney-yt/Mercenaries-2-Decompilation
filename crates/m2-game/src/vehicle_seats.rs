//! A vehicle's seats as the scripts see them: the `Vehicle` library's
//! seat calls and `Object.InVehicle`.
//!
//! From the 360 code:
//! - A seat is an object of its own. Its vehicle's seats are the
//!   vehicle's set in the seat store 0x83653DF8 (in `SeatLink` order);
//!   who sits in one is the rider-to-seat map 0x836540FC; a seat's
//!   `SeatParameters` bits are in the seat record store 0x83653E8C.
//! - `Object.InVehicle(obj)` (0x8232FBF8): its seat (0x836540FC) and
//!   that seat's vehicle (0x83653DF8), two results; both nil out of one.
//! - `Vehicle.GetSeatFromRider(rider)` (0x823468C8): its seat.
//!   `Vehicle.GetRiderFromSeat(seat)` (0x82346938): who sits in it.
//!   `Vehicle.GetFromSeat` (0x82346830, `GetFromRider`'s function): the
//!   vehicle of a rider in a seat.
//! - `Vehicle.GetSeatByType(vehicle, type, free)` (0x823469A8): the first
//!   of its seats of that type (`d` driver, 0x8229C800; `g` gunner,
//!   0x8229CF68; `c` cargo, 0x8229D0C0; `p` none of those, 0x8229D218),
//!   `d` when no type is given; `free`: only one nobody sits in.
//! - `Vehicle.GetSeatParams(seat)` (0x82346AF8): a table of its bits,
//!   `IsDriver`, `IsGunner`, `IsCargo`, `IsHijackable`, `IsHijackBlocker`,
//!   `IsRiderInvisible`, `IsRiderVulnerable`, `IsRiderBashable`,
//!   `IsStowable`, and `StowSeatGuid`.
//! - `Vehicle.GetSeatToSeat(seat, keyed)` (0x82346F00): the seats its
//!   rider may move over to (the seat's `SeatToSeat` rows), keyed by
//!   their `SeatLinkName` or, `keyed` false, as a list.
//! - `Vehicle.TransferToSeat(rider, seat, instant)` (0x823474A0, through
//!   0x822A6FF0) and `Vehicle.EnterBySeatGuid(character, vehicle, seat,
//!   instant, ..)` (0x82347748): to that seat; false if they can't.
//! - `Vehicle.SetCanPlayerUse(vehicle, type, can)` (0x82346D10): sets
//!   whether she may take each of its seats of that type (`a` all, the
//!   default; `d`, `g`, `c`, `p` as above).
//!
//! The scripts' use: GuerCon's rusher check reads `Object.InVehicle`'s
//! two results, the vehicle's physics type and the seat's `IsGunner`.
//!
//! Inferred: a rider the game seats without naming the seat takes the
//! driver's seat when driving, else the first free one that isn't. Not
//! - `Vehicle.IsSeatALadder(seat)` (0x82347408): whether the seat has a
//!   `ControllerLadder` (the store 0x83653518, named by 0x823B1F98).
//!
//! - `Vehicle.IsSeatBlocked(seat)` (0x82347240): a rider of it has a
//!   player record (store 0x83657FA8), or a rider of another seat of the
//!   vehicle has its `Rider` record's `TargetGuid` (+8) on it. The `Rider`
//!   record (0x823F20A0) is `Role` (+0: Rider, Hijacker, Cargo), `State`
//!   (+4: Idle, Entering, Exiting, Transferring) and `TargetGuid` (+8).
//!   Seating here is at once, so nobody is headed for another seat and
//!   only the first holds.
//!
//! Not ported: the animation of a
//! transfer (the game only moves the rider's seat), the blocked-seat
//! test of a `free` search (0x8229CBB8), the
//! stow seat's guid (`uStowSeat`; given nil).

use std::collections::{BTreeMap, HashMap};

use mlua::{Lua, MultiValue, Table, Value};

use crate::api::{ScriptWorld, Shared};
use crate::events::guid_value;
use crate::script::{ScriptHost, safe_function};

/// Guids handed to seats start here (seats are objects of their own on
/// the 360; here they get guids of their own, out of the layers' and
/// the spawned objects' ranges).
pub const FIRST_SEAT: u32 = 0xfb00_0000;

/// A seat's `SeatParameters`, as `Vehicle.GetSeatParams` reports them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SeatParams {
    pub driver: bool,
    pub gunner: bool,
    pub cargo: bool,
    pub hijackable: bool,
    pub hijack_blocker: bool,
    pub rider_invisible: bool,
    pub rider_vulnerable: bool,
    pub rider_bashable: bool,
    pub stowable: bool,
    /// A ladder (its template's `ControllerLadder`).
    pub ladder: bool,
}

impl SeatParams {
    /// Whether it is of `kind` (`a`, `d`, `g`, `c`, `p`).
    pub fn is(&self, kind: u8) -> bool {
        match kind.to_ascii_lowercase() {
            b'a' => true,
            b'd' => self.driver,
            b'g' => self.gunner,
            b'c' => self.cargo,
            b'p' => !self.driver && !self.gunner && !self.cargo,
            _ => false,
        }
    }
}

/// One seat of a vehicle in play.
#[derive(Clone, Debug, PartialEq)]
pub struct SeatInfo {
    pub vehicle: u32,
    /// Its `SeatLink` `Name` (hashed).
    pub name: u32,
    pub params: SeatParams,
    /// She may take it (`bCanPlayerUse`, `Vehicle.SetCanPlayerUse`).
    pub player_use: bool,
    /// The seats its rider may move over to (its template's `SeatToSeat`
    /// rows' `SeatLinkName`s, hashed), in order.
    pub links: Vec<u32>,
}

/// The seats of the vehicles in play and who sits where.
#[derive(Clone, Debug, Default)]
pub struct Seats {
    seats: BTreeMap<u32, SeatInfo>,
    rider: HashMap<u32, u32>,
    next: u32,
}

impl Seats {
    /// Gives `vehicle` its seats, in `SeatLink` order (once: a vehicle
    /// that has them keeps them).
    pub fn add(
        &mut self,
        vehicle: u32,
        seats: impl IntoIterator<Item = (u32, SeatParams, bool, Vec<u32>)>,
    ) {
        if self.seats.values().any(|s| s.vehicle == vehicle) {
            return;
        }
        for (name, params, player_use, links) in seats {
            let guid = FIRST_SEAT + self.next;
            self.next += 1;
            self.seats.insert(
                guid,
                SeatInfo {
                    vehicle,
                    name,
                    params,
                    player_use,
                    links,
                },
            );
        }
    }

    /// `vehicle`'s seats, in order.
    pub fn of(&self, vehicle: u32) -> impl Iterator<Item = (u32, &SeatInfo)> {
        self.seats
            .iter()
            .filter(move |(_, s)| s.vehicle == vehicle)
            .map(|(&g, s)| (g, s))
    }

    pub fn get(&self, seat: u32) -> Option<&SeatInfo> {
        self.seats.get(&seat)
    }

    /// Who sits in `seat`.
    pub fn rider_in(&self, seat: u32) -> Option<u32> {
        self.rider
            .iter()
            .filter(|(_, s)| **s == seat)
            .map(|(&r, _)| r)
            .min()
    }

    /// `rider`'s seat.
    pub fn seat_of(&self, rider: u32) -> Option<u32> {
        self.rider.get(&rider).copied()
    }

    /// 0x823469A8's search: the first of `vehicle`'s seats of `kind`, and
    /// with `free` only one nobody sits in.
    pub fn by_type(&self, vehicle: u32, kind: u8, free: bool) -> Option<u32> {
        self.of(vehicle)
            .find(|(g, s)| s.params.is(kind) && !(free && self.rider_in(*g).is_some()))
            .map(|(g, _)| g)
    }

    /// Seats `rider` in `vehicle`: in the seat named `name` (hashed) if
    /// it has one, else (inferred) the driver's when `driving`, else the
    /// first free seat that isn't the driver's.
    pub fn sit(&mut self, rider: u32, vehicle: u32, name: Option<u32>, driving: bool) {
        self.rider.remove(&rider);
        let named = name.and_then(|n| self.of(vehicle).find(|(_, s)| s.name == n).map(|(g, _)| g));
        let seat = named.or_else(|| {
            if driving {
                self.by_type(vehicle, b'd', false)
            } else {
                self.of(vehicle)
                    .find(|(g, s)| !s.params.driver && self.rider_in(*g).is_none())
                    .map(|(g, _)| g)
            }
        });
        if let Some(s) = seat {
            self.rider.insert(rider, s);
        }
    }

    /// 0x82346F00: the seats `seat`'s rider may move over to, each with
    /// its `SeatLinkName`.
    pub fn to_seats(&self, seat: u32) -> Vec<(u32, u32)> {
        let Some(s) = self.get(seat) else {
            return Vec::new();
        };
        s.links
            .iter()
            .filter_map(|&n| {
                self.of(s.vehicle)
                    .find(|(_, t)| t.name == n)
                    .map(|(g, _)| (n, g))
            })
            .collect()
    }

    /// Moves `rider` to `seat` (0x822A6FF0 for `TransferToSeat`): false
    /// if it is no seat or someone else sits in it.
    pub fn move_to(&mut self, rider: u32, seat: u32) -> bool {
        if self.get(seat).is_none() || self.rider_in(seat).is_some_and(|r| r != rider) {
            return false;
        }
        self.rider.insert(rider, seat);
        true
    }

    /// `rider` is out of its seat.
    pub fn leave(&mut self, rider: u32) {
        self.rider.remove(&rider);
    }

    /// 0x82346D10: whether she may take `vehicle`'s seats of `kind`.
    pub fn set_player_use(&mut self, vehicle: u32, kind: u8, can: bool) {
        for s in self.seats.values_mut() {
            if s.vehicle == vehicle && s.params.is(kind) {
                s.player_use = can;
            }
        }
    }
}

fn id(v: Option<&Value>) -> Option<u32> {
    v.and_then(crate::events::as_guid)
}

fn guid(g: Option<u32>) -> Value {
    g.map_or(Value::Nil, guid_value)
}

fn kind(v: Option<&Value>, default: u8) -> u8 {
    match v {
        Some(Value::String(s)) => s.as_bytes().first().copied().unwrap_or(default),
        _ => default,
    }
}

/// `Object.InVehicle`'s answer for `obj`: its seat and the seat's
/// vehicle. A rider the game put aboard some other way (its object's
/// vehicle set) is seated as [`Seats::sit`] does; one taken out of its
/// vehicle is out of its seat.
pub fn in_vehicle(w: &mut ScriptWorld, obj: u32) -> (Option<u32>, Option<u32>) {
    let Some(o) = w.objects.get(&obj) else {
        return (None, None);
    };
    let (riding, driving) = (o.vehicle, !o.not_driving);
    let seated = w
        .seats
        .seat_of(obj)
        .and_then(|s| w.seats.get(s))
        .map(|s| s.vehicle);
    match riding {
        None => w.seats.leave(obj),
        Some(v) if seated != Some(v) => w.seats.sit(obj, v, None, driving),
        Some(_) => {}
    }
    match w.seats.seat_of(obj) {
        Some(s) => (Some(s), w.seats.get(s).map(|s| s.vehicle)),
        None => (None, None),
    }
}

fn params_table(lua: &Lua, p: &SeatParams) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    for (k, v) in [
        ("IsDriver", p.driver),
        ("IsGunner", p.gunner),
        ("IsCargo", p.cargo),
        ("IsHijackable", p.hijackable),
        ("IsHijackBlocker", p.hijack_blocker),
        ("IsRiderInvisible", p.rider_invisible),
        ("IsRiderVulnerable", p.rider_vulnerable),
        ("IsRiderBashable", p.rider_bashable),
        ("IsStowable", p.stowable),
    ] {
        t.raw_set(k, v)?;
    }
    Ok(t)
}

/// Installs the seat calls over `world`. Run after the other libraries
/// are in (it adds to them).
pub fn install(host: &ScriptHost, world: &Shared) -> mlua::Result<()> {
    let lua = host.lua();
    let globals = lua.globals();
    let lib = |name: &str| -> mlua::Result<Table> {
        match globals.get::<Value>(name)? {
            Value::Table(t) => Ok(t),
            _ => {
                let t = lua.create_table()?;
                globals.raw_set(name, &t)?;
                Ok(t)
            }
        }
    };
    let add = |table: &Table,
               name: &str,
               f: fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>|
     -> mlua::Result<()> {
        let w = std::rc::Rc::clone(world);
        table.raw_set(
            name,
            safe_function(lua, move |lua, args| {
                let args: Vec<Value> = args.into_iter().collect();
                f(&mut w.borrow_mut(), lua, &args)
            })?,
        )
    };
    let object = lib("Object")?;
    add(&object, "InVehicle", |w, _, a| {
        let (seat, vehicle) = id(a.first()).map_or((None, None), |o| in_vehicle(w, o));
        Ok(MultiValue::from_iter([guid(seat), guid(vehicle)]))
    })?;

    let vehicle = lib("Vehicle")?;
    add(&vehicle, "GetSeatFromRider", |w, _, a| {
        let seat = id(a.first()).and_then(|r| in_vehicle(w, r).0);
        Ok(MultiValue::from_iter([guid(seat)]))
    })?;
    add(&vehicle, "GetRiderFromSeat", |w, _, a| {
        Ok(MultiValue::from_iter([guid(
            id(a.first()).and_then(|s| w.seats.rider_in(s)),
        )]))
    })?;
    add(&vehicle, "GetFromSeat", |w, _, a| {
        let v = id(a.first()).and_then(|r| in_vehicle(w, r).1);
        Ok(MultiValue::from_iter([guid(v)]))
    })?;
    add(&vehicle, "GetSeatByType", |w, _, a| {
        let seat = id(a.first()).and_then(|v| {
            w.seats.by_type(
                v,
                kind(a.get(1), b'd'),
                matches!(a.get(2), Some(Value::Boolean(true))),
            )
        });
        Ok(MultiValue::from_iter([guid(seat)]))
    })?;
    add(&vehicle, "GetSeatParams", |w, lua, a| {
        match id(a.first()).and_then(|s| w.seats.get(s)) {
            Some(s) => Ok(MultiValue::from_iter([Value::Table(params_table(
                lua, &s.params,
            )?)])),
            None => Err(mlua::Error::runtime("Vehicle.GetSeatParams: not a seat")),
        }
    })?;
    // 0x82347240: blocked when a player's character sits in it, or a rider
    // of another seat of its vehicle is headed for it (its `Rider`
    // record's `TargetGuid`, +8). Seating here is at once (no rider is
    // `Entering` or `Transferring`), so only the first holds.
    add(&vehicle, "IsSeatBlocked", |w, _, a| {
        let Some(seat) = id(a.first()) else {
            return Err(mlua::Error::runtime("Vehicle.IsSeatBlocked: not a seat"));
        };
        let blocked = w.seats.rider_in(seat).is_some_and(|r| {
            w.objects.get(&r).is_some_and(|o| o.player_controlled) || w.player == Some(r)
        });
        Ok(MultiValue::from_iter([Value::Boolean(blocked)]))
    })?;
    // 0x82347408: whether the seat is in the `ControllerLadder` store.
    add(&vehicle, "IsSeatALadder", |w, _, a| {
        let Some(seat) = id(a.first()) else {
            return Err(mlua::Error::runtime("Vehicle.IsSeatALadder: not a seat"));
        };
        Ok(MultiValue::from_iter([Value::Boolean(
            w.seats.get(seat).is_some_and(|s| s.params.ladder),
        )]))
    })?;
    // 0x82346F00: the seats a seat's rider may move to, keyed by their
    // `SeatLinkName` (the default) or as a list.
    add(&vehicle, "GetSeatToSeat", |w, lua, a| {
        let Some(seat) = id(a.first()) else {
            return Err(mlua::Error::runtime("Vehicle.GetSeatToSeat: not a seat"));
        };
        let keyed = !matches!(a.get(1), Some(Value::Boolean(false)));
        let t = lua.create_table()?;
        for (i, (name, s)) in w.seats.to_seats(seat).into_iter().enumerate() {
            if keyed {
                t.raw_set(i64::from(name), guid_value(s))?;
            } else {
                t.raw_set(i as i64 + 1, guid_value(s))?;
            }
        }
        Ok(MultiValue::from_iter([Value::Table(t)]))
    })?;
    // 0x823474A0 (0x822A6FF0): moves a rider to a seat of its vehicle.
    add(&vehicle, "TransferToSeat", |w, _, a| {
        let ok = match (id(a.first()), id(a.get(1))) {
            (Some(r), Some(s)) => {
                let v = w.seats.get(s).map(|s| (s.vehicle, s.params.driver));
                match v {
                    Some((vehicle, driver))
                        if w.objects.get(&r).and_then(|o| o.vehicle) == Some(vehicle) =>
                    {
                        let moved = w.seats.move_to(r, s);
                        if moved && let Some(o) = w.objects.get_mut(&r) {
                            o.not_driving = !driver;
                        }
                        moved
                    }
                    _ => false,
                }
            }
            _ => false,
        };
        Ok(MultiValue::from_iter([Value::Boolean(ok)]))
    })?;
    // 0x82347748: seats a character in the seat given by guid
    // (`EnterBySeatGuid(character, vehicle, seat, instant, ..)`).
    add(&vehicle, "EnterBySeatGuid", |w, _, a| {
        let (Some(guid), Some(vehicle), Some(seat)) = (id(a.first()), id(a.get(1)), id(a.get(2)))
        else {
            return Err(mlua::Error::runtime("Vehicle.EnterBySeatGuid: arguments"));
        };
        let Some(driver) = w
            .seats
            .get(seat)
            .filter(|s| s.vehicle == vehicle)
            .map(|s| s.params.driver)
        else {
            return Ok(MultiValue::from_iter([Value::Boolean(false)]));
        };
        let instant = !matches!(a.get(3), Some(Value::Boolean(false)));
        w.seat(guid, vehicle, instant, !driver);
        let moved = w.seats.move_to(guid, seat);
        w.commands.push(crate::api::Command::EnterVehicle {
            guid,
            vehicle,
            seat: if driver { "d" } else { "p" }.into(),
            instant,
        });
        Ok(MultiValue::from_iter([Value::Boolean(moved)]))
    })?;
    add(&vehicle, "SetCanPlayerUse", |w, _, a| {
        if let Some(v) = id(a.first()) {
            let (k, can) = match a.get(1) {
                Some(Value::String(_)) => (kind(a.get(1), b'a'), a.get(2)),
                _ => (b'a', a.get(1)),
            };
            w.seats
                .set_player_use(v, k, !matches!(can, Some(Value::Boolean(false)) | None));
        }
        Ok(MultiValue::new())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(driver: bool, gunner: bool, cargo: bool) -> SeatParams {
        SeatParams {
            driver,
            gunner,
            cargo,
            ..SeatParams::default()
        }
    }

    fn jeep(s: &mut Seats) {
        // A driver, a gunner and a passenger, in `SeatLink` order.
        s.add(
            10,
            [
                (1, p(true, false, false), true, vec![3]),
                (2, p(false, true, false), true, vec![1, 3]),
                (3, p(false, false, false), true, vec![]),
            ],
        );
    }

    #[test]
    fn seats_are_found_by_type_and_rider() {
        let mut s = Seats::default();
        jeep(&mut s);
        jeep(&mut s);
        assert_eq!(s.of(10).count(), 3, "seated once");
        let (d, g, pass) = (FIRST_SEAT, FIRST_SEAT + 1, FIRST_SEAT + 2);
        assert_eq!(s.by_type(10, b'd', false), Some(d));
        assert_eq!(s.by_type(10, b'g', false), Some(g));
        assert_eq!(s.by_type(10, b'p', false), Some(pass));
        assert_eq!(s.by_type(10, b'c', false), None);
        s.sit(100, 10, None, true);
        s.sit(101, 10, None, false);
        assert_eq!((s.seat_of(100), s.seat_of(101)), (Some(d), Some(g)));
        assert_eq!(s.rider_in(g), Some(101));
        // Free only: the driver's is taken.
        assert_eq!(s.by_type(10, b'd', true), None);
        assert_eq!(s.by_type(10, b'p', true), Some(pass));
        // By name.
        s.sit(101, 10, Some(3), false);
        assert_eq!(s.seat_of(101), Some(pass));
        s.leave(100);
        assert_eq!(s.rider_in(d), None);
        s.set_player_use(10, b'g', false);
        assert!(!s.get(g).unwrap().player_use && s.get(d).unwrap().player_use);
    }

    #[test]
    fn the_scripts_read_a_riders_seat_and_vehicle() {
        let host = ScriptHost::new().unwrap();
        let world: Shared = Default::default();
        {
            let mut w = world.borrow_mut();
            jeep(&mut w.seats);
            w.insert(100, crate::api::Object::default());
            w.seat(100, 10, false, true);
        }
        install(&host, &world).unwrap();
        let lua = host.lua();
        lua.globals().set("rider", guid_value(100)).unwrap();
        lua.globals().set("jeep", guid_value(10)).unwrap();
        let (gunner, same, driver_free, nobody): (bool, bool, bool, bool) = lua
            .load(
                "local seat, v = Object.InVehicle(rider)\n\
                 local p = Vehicle.GetSeatParams(seat)\n\
                 return p.IsGunner and not p.IsDriver, v == jeep and Vehicle.GetFromSeat(rider) == jeep \
                   and Vehicle.GetRiderFromSeat(seat) == rider and Vehicle.GetSeatFromRider(rider) == seat, \
                   Vehicle.GetSeatByType(jeep, 'd', true) ~= nil, \
                   select('#', Object.InVehicle(jeep)) == 2 and Object.InVehicle(jeep) == nil",
            )
            .eval()
            .unwrap();
        assert!(gunner && same && driver_free && nobody);
        // The gunner may move to the driver's seat and the passenger's.
        let (to_driver, list_len, moved, now_passenger, entered): (bool, i64, bool, bool, bool) = lua
            .load(
                "local seat = Vehicle.GetSeatFromRider(rider)
                 local keyed = Vehicle.GetSeatToSeat(seat)
                 local list = Vehicle.GetSeatToSeat(seat, false)
                 local p = Vehicle.GetSeatByType(jeep, 'p')
                 local moved = Vehicle.TransferToSeat(rider, p, true)
                 local d = Vehicle.GetSeatByType(jeep, 'd')
                 return keyed[1] == d, #list, moved, Vehicle.GetSeatFromRider(rider) == p,                    Vehicle.EnterBySeatGuid(rider, jeep, d) and Vehicle.GetSeatFromRider(rider) == d",
            )
            .eval()
            .unwrap();
        assert!(to_driver && moved && now_passenger && entered);
        // A ladder seat (its template's `ControllerLadder`).
        world.borrow_mut().seats.add(
            20,
            [(
                9,
                SeatParams {
                    ladder: true,
                    ..SeatParams::default()
                },
                false,
                vec![],
            )],
        );
        lua.globals().set("ladderjeep", guid_value(20)).unwrap();
        let (ladder, not): (bool, bool) = lua
            .load(
                "return Vehicle.IsSeatALadder(Vehicle.GetSeatByType(ladderjeep, 'p')),                  not Vehicle.IsSeatALadder(Vehicle.GetSeatByType(jeep, 'd'))",
            )
            .eval()
            .unwrap();
        assert!(ladder && not);
        // The player in a seat blocks it; an AI rider doesn't.
        {
            let mut w = world.borrow_mut();
            w.player = Some(100);
        }
        let (mine, free): (bool, bool) = lua
            .load(
                "return Vehicle.IsSeatBlocked(Vehicle.GetSeatFromRider(rider)),                  not Vehicle.IsSeatBlocked(Vehicle.GetSeatByType(jeep, 'g'))",
            )
            .eval()
            .unwrap();
        assert!(mine && free);
        assert_eq!(list_len, 2);
    }
}
