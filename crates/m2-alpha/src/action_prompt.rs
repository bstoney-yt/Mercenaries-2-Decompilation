//! The engine's own prompts for what the use button does in reach: get
//! into a vehicle, hijack it, pick up a gun, and the rest, as the 360
//! sends them to the HUD's context action widget.
//!
//! From the 360 code:
//!
//! - Each frame the player update (`0x824386F0`) asks each of the player's
//!   states (their vtable's third slot) for an action kind and the thing
//!   it is about, and keeps the highest kind; `0x82438D10` then tells the
//!   HUD when the kind or the thing changed: the old prompt is taken back
//!   (`0x822070E0`, by its kind) and the new one sent (`0x82207018`) with
//!   the kind as its priority. The HUD's script (`MrxGuiHudMelee`'s
//!   `SetContextActionMessage(text, player, priority)`) files each text
//!   under its priority, shows the newest and falls back to another when
//!   the one shown is taken back.
//! - The kinds ([`Action`]) and their texts are two tables of string ids
//!   (`0x82DE8BC8` plain, `0x82DE8B78` with the thing's name): for a
//!   gun to pick up, a vehicle seat and the vehicle exit, the named id
//!   carries the thing's `LocalizedName` key (`[0x%08X]`) when it has
//!   one, else the plain id is sent. Climbing a vehicle is always plain.
//! - The on-foot state (`0x82443AA0` into `0x82432F68`) offers what is
//!   in reach: the scripts' own text (16), the nearest vehicle entrance
//!   (its kind from `0x82432498`, [`vehicle_action`]), a gun lying there
//!   (2 to equip it, 3 to take its ammo) and a person to grapple (1),
//!   and of those the one she faces (`0x82432CF0`).
//! - `0x82432498` reads, for the entrance and the seat it gives her
//!   (`0x82299D68`): the seat's `bIsDriverSeat` and `bIsGunnerSeat` (bits
//!   31 and 30 of the seat parameters, packed at `0x823F1988`), the
//!   entrance's `bLadder` (bit 29 of the entrance parameters, packed at
//!   `0x823F1CA0`), the vehicle's physics (the kind its physics object
//!   answers, slot `0xE0`: 2 `_CarPhysicsV2`, 3 `_TankPhysics`, 4
//!   `_HelicopterPhysics`, 10 `_BoatPhysics`, by the factories registered
//!   in `0x82405920`), whether any of its seats has a living rider
//!   (`0x8229DB28`) and, if so, its standing with the players: the least
//!   of their moods to it (`0x822F08D0`), hostile below -33.
//! - In a vehicle, `0x8242FB68` sends the plain `[ContextAction.ExitVehicle]`
//!   (`0x820EBFF8`) at kind 15, or takes it back. It is a helicopter's
//!   bail-out check ([`ExitConfirm`]): the in-vehicle update (`0x8242E7A0`)
//!   asks `0x8242FA70` before letting her out on the exit button, which,
//!   for a helicopter (physics kind 4) with no prompt up whose two heights
//!   (its physics' `+0x158` and `+0x160`, the lesser) are 5 m or more
//!   (`0x820BAF84`), sends the prompt, starts a 2 s clock (`0x820C77A4`)
//!   and keeps her in; pressed again while the clock runs, she gets out.
//!   The update runs the clock down and takes the prompt back when it
//!   ends; leaving the state (`0x8242E730`) takes it back too.
//! - The facing choice (`0x82432CF0`) among several things in reach: one
//!   alone is taken; else each is scored by the dot of her facing (the
//!   state's `+0x12C`, normalized) with the way from the camera to it and
//!   by its squared distance from her, and it beats the best so far when
//!   its dot is more than 0.1 lower at worst and either 0.1 or more
//!   higher, or higher and nearer, or (lower by under 0.1) nearer by more
//!   than 3 m².
//!
//! Not done here: that facing choice (the alpha keeps its own order: the
//! scripts' text, then a gun, then an emplacement or a vehicle); grapple,
//! subdue, verify and stow. Inferred: the helicopter's two heights are read
//! here as one downward ray's height above what lies under it (what the
//! physics stores at `+0x158`/`+0x160` was not traced).

use m2_engine::seats::Role;
use m2_engine::world::VehicleKind;

/// What the use button does, by the 360's kind number (its priority on
/// the HUD).
#[allow(
    dead_code,
    reason = "the 360's whole table; the alpha does not send grapple, subdue, verify or stow"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Grapple = 1,
    PickupWeapon = 2,
    TakeAmmo = 3,
    Subdue = 4,
    Verify = 5,
    Drive = 6,
    Ride = 7,
    Pilot = 8,
    Use = 9,
    Take = 10,
    Hijack = 11,
    Board = 12,
    Climb = 13,
    Enter = 14,
    Exit = 15,
    StopGrapple = 17,
    StowHvt = 18,
}

impl Action {
    /// Its kind: the priority the HUD files its text under.
    pub fn kind(self) -> u32 {
        self as u32
    }

    /// Its string id without a name (the table at `0x82DE8BC8`).
    pub fn plain(self) -> &'static str {
        match self {
            Self::Grapple => "[ContextAction.Grapple]",
            Self::PickupWeapon => "[ContextAction.PickupWeapon]",
            Self::TakeAmmo => "[ContextAction.TakeAmmo]",
            Self::Subdue => "[ContextAction.Subdue]",
            Self::Verify => "[ContextAction.Verify]",
            Self::Drive => "[ContextAction.DriveVehicle]",
            Self::Ride => "[ContextAction.RideVehicle]",
            Self::Pilot => "[ContextAction.PilotVehicle]",
            Self::Use => "[ContextAction.UseVehicle]",
            Self::Take => "[ContextAction.TakeVehicle]",
            Self::Hijack => "[ContextAction.HijackVehicle]",
            Self::Board => "[ContextAction.BoardVehicle]",
            Self::Climb => "[ContextAction.ClimbVehicle]",
            Self::Enter => "[ContextAction.EnterVehicle]",
            Self::Exit => "[ContextAction.ExitVehicle]",
            Self::StopGrapple => "[ContextAction.StopGrapple]",
            Self::StowHvt => "[ContextAction.StowHVT]",
        }
    }

    /// Its string id's stem with the thing's name (the table at
    /// `0x82DE8B78`), for the kinds `0x82438D10` names.
    fn named(self) -> Option<&'static str> {
        Some(match self {
            Self::PickupWeapon => "PickupWeaponName",
            Self::Drive => "DriveVehicleName",
            Self::Ride => "RideVehicleName",
            Self::Pilot => "PilotVehicleName",
            Self::Use => "UseVehicleName",
            Self::Take => "TakeVehicleName",
            Self::Hijack => "HijackVehicleName",
            Self::Board => "BoardVehicleName",
            Self::Enter => "EnterVehicleName",
            Self::Exit => "ExitVehicleName",
            _ => return None,
        })
    }

    /// The text sent to the HUD: named with `name` (the thing's
    /// `LocalizedName` key) when the kind takes a name and it has one.
    pub fn text(self, name: Option<u32>) -> String {
        match (self.named(), name) {
            (Some(stem), Some(key)) => format!("[ContextAction.{stem}:[0x{key:08X}]]"),
            _ => self.plain().to_owned(),
        }
    }
}

/// The least height (m) at which a helicopter asks before she bails out
/// (`0x820BAF84`).
pub const EXIT_CONFIRM_HEIGHT: f32 = 5.0;

/// How long the bail-out prompt waits for the second press, seconds
/// (`0x820C77A4`).
pub const EXIT_CONFIRM_SECONDS: f32 = 2.0;

/// A helicopter's bail-out check (`0x8242FA70`, its clock run down by
/// `0x8242E7A0`): the first exit press up high shows
/// [`Action::Exit`] and keeps her in; a second while it shows lets her go.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ExitConfirm {
    /// Seconds left on the prompt; 0 when none is up.
    left: f32,
}

impl ExitConfirm {
    /// The prompt is up.
    pub fn shown(&self) -> bool {
        self.left > 0.0
    }

    /// Runs the clock down by `dt`; true when the prompt was just taken
    /// back.
    pub fn tick(&mut self, dt: f32) -> bool {
        if self.left > 0.0 {
            self.left -= dt;
            if self.left <= 0.0 {
                self.left = 0.0;
                return true;
            }
        }
        false
    }

    /// The exit button, in a helicopter (`heli`) whose height is `height`
    /// (`None`: unknown, nothing under it in reach): whether she gets
    /// out. With no prompt up, up high, it shows the prompt instead.
    pub fn press(&mut self, heli: bool, height: Option<f32>) -> bool {
        if heli && self.left <= 0.0 && height.is_none_or(|h| h >= EXIT_CONFIRM_HEIGHT) {
            self.left = EXIT_CONFIRM_SECONDS;
            return false;
        }
        true
    }

    /// Out of the vehicle: the prompt goes (`0x8242E730`).
    pub fn clear(&mut self) {
        self.left = 0.0;
    }
}

/// What `0x82432498` reads of a vehicle entrance in her reach.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Approach {
    /// The role of the seat the entrance gives her.
    pub role: Role,
    /// The entrance's `bLadder`.
    pub ladder: bool,
    /// The vehicle's physics; `None` when the database gives none.
    pub physics: Option<VehicleKind>,
    /// A car-physics vehicle ridden, not driven. The 360 asks the car
    /// physics' parameters (`+0x158`, its `+0x208` equal to 2); the alpha
    /// reads that as a motorbike: inferred, not traced.
    pub bike: bool,
    /// Some seat of it has a rider.
    pub crewed: bool,
    /// Its riders are hostile to her.
    pub hostile: bool,
}

/// The kind of prompt for getting into a vehicle (`0x82432498`).
pub fn vehicle_action(a: &Approach) -> Action {
    if a.role == Role::Gunner {
        // A gunner's seat that is not also a driver's.
        return Action::Use;
    }
    if a.ladder {
        return if a.physics == Some(VehicleKind::Boat) {
            Action::Board
        } else {
            Action::Climb
        };
    }
    if a.crewed {
        return if a.hostile {
            Action::Hijack
        } else {
            Action::Take
        };
    }
    if a.role != Role::Driver {
        return Action::Enter;
    }
    match a.physics {
        Some(VehicleKind::Car) if a.bike => Action::Ride,
        Some(VehicleKind::Car | VehicleKind::Tank | VehicleKind::Boat) => Action::Drive,
        Some(VehicleKind::Helicopter) => Action::Pilot,
        _ => Action::Enter,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(role: Role, physics: VehicleKind) -> Approach {
        Approach {
            role,
            ladder: false,
            physics: Some(physics),
            bike: false,
            crewed: false,
            hostile: false,
        }
    }

    /// An empty vehicle's driver's seat: driven, ridden, piloted.
    #[test]
    fn the_drivers_seat_is_worded_by_the_vehicles_physics() {
        assert_eq!(
            vehicle_action(&at(Role::Driver, VehicleKind::Car)),
            Action::Drive
        );
        assert_eq!(
            vehicle_action(&at(Role::Driver, VehicleKind::Tank)),
            Action::Drive
        );
        assert_eq!(
            vehicle_action(&at(Role::Driver, VehicleKind::Boat)),
            Action::Drive
        );
        assert_eq!(
            vehicle_action(&at(Role::Driver, VehicleKind::Helicopter)),
            Action::Pilot
        );
        let bike = Approach {
            bike: true,
            ..at(Role::Driver, VehicleKind::Car)
        };
        assert_eq!(vehicle_action(&bike), Action::Ride);
        let unknown = Approach {
            physics: None,
            ..at(Role::Driver, VehicleKind::Car)
        };
        assert_eq!(vehicle_action(&unknown), Action::Enter);
    }

    /// Other seats: a gunner's is used, a passenger's entered.
    #[test]
    fn other_seats_are_used_or_entered() {
        assert_eq!(
            vehicle_action(&at(Role::Gunner, VehicleKind::Tank)),
            Action::Use
        );
        assert_eq!(
            vehicle_action(&at(Role::Passenger, VehicleKind::Car)),
            Action::Enter
        );
        // A gunner's seat wins over a crew and a ladder.
        let crewed = Approach {
            crewed: true,
            hostile: true,
            ladder: true,
            ..at(Role::Gunner, VehicleKind::Car)
        };
        assert_eq!(vehicle_action(&crewed), Action::Use);
    }

    /// A ladder: a boat is boarded, anything else climbed, crewed or not.
    #[test]
    fn a_ladder_is_boarded_or_climbed() {
        let ladder = |physics| Approach {
            ladder: true,
            crewed: true,
            hostile: true,
            ..at(Role::Driver, physics)
        };
        assert_eq!(vehicle_action(&ladder(VehicleKind::Boat)), Action::Board);
        assert_eq!(vehicle_action(&ladder(VehicleKind::Tank)), Action::Climb);
    }

    /// A crewed vehicle is hijacked from a hostile crew and taken from
    /// any other, whatever the seat.
    #[test]
    fn a_crew_is_hijacked_or_the_vehicle_taken() {
        let crewed = |hostile, role| Approach {
            crewed: true,
            hostile,
            ..at(role, VehicleKind::Car)
        };
        assert_eq!(vehicle_action(&crewed(true, Role::Driver)), Action::Hijack);
        assert_eq!(vehicle_action(&crewed(false, Role::Driver)), Action::Take);
        assert_eq!(
            vehicle_action(&crewed(true, Role::Passenger)),
            Action::Hijack
        );
    }

    /// A helicopter up high asks first: the first press shows the prompt
    /// and keeps her in, a second within 2 s lets her out; low down, or
    /// in anything else, she gets out at once.
    #[test]
    fn a_helicopter_up_high_asks_before_she_bails_out() {
        let mut c = ExitConfirm::default();
        assert!(c.press(false, Some(100.0)), "not a helicopter");
        assert!(c.press(true, Some(4.9)), "low down");
        assert!(!c.shown());
        assert!(!c.press(true, Some(5.0)), "the first press up high");
        assert!(c.shown());
        assert!(!c.tick(1.5));
        assert!(c.press(true, Some(50.0)), "pressed again in time");
        c.clear();
        // Let go: the prompt goes after 2 s and the next press asks again.
        assert!(!c.press(true, None));
        assert!(!c.tick(1.99));
        assert!(c.tick(0.02), "taken back when the clock ends");
        assert!(!c.shown());
        assert!(!c.press(true, Some(6.0)));
        assert_eq!(Action::Exit.text(None), "[ContextAction.ExitVehicle]");
        assert_eq!(Action::Exit.kind(), 15);
    }

    /// The texts: named with the thing's key where the kind takes one,
    /// plain otherwise; the kind is the priority.
    #[test]
    fn the_texts_and_priorities() {
        assert_eq!(
            Action::Drive.text(Some(0x1a2b_3c4d)),
            "[ContextAction.DriveVehicleName:[0x1A2B3C4D]]"
        );
        assert_eq!(Action::Drive.text(None), "[ContextAction.DriveVehicle]");
        assert_eq!(
            Action::PickupWeapon.text(Some(5)),
            "[ContextAction.PickupWeaponName:[0x00000005]]"
        );
        assert_eq!(Action::Climb.text(Some(5)), "[ContextAction.ClimbVehicle]");
        assert_eq!(Action::TakeAmmo.text(Some(5)), "[ContextAction.TakeAmmo]");
        assert_eq!((Action::Grapple.kind(), Action::Hijack.kind()), (1, 11));
        assert_eq!(Action::StowHvt.kind(), 18);
    }
}
