//! Getting into and out of a vehicle by its door: the clips she plays one
//! after another, and the door swinging open and shut as they say.
//!
//! A vehicle template's `Door` records name its doors: `Name` (what an
//! entrance's `DoorName` gives, `front_left`), `DoorBoneName` (the node
//! that swings), `AxisOfRotation`, `OpenAngle` (degrees), `TimeToOpen` and
//! `TimeToClose` (seconds) and `StartOpen`. A door swings at an even rate
//! about its axis, in its node's own frame, from shut to `OpenAngle`.
//!
//! Getting in, she plays the stance `InVehicle`'s actions in turn, each on
//! the one before's `Complete` ([`Step`]): `EnterVehicleOpenDoor` when the
//! entrance's door is there and shut, then `EnterVehicle`, then
//! `EnterVehicleCloseDoor` when the door was left open. Getting out:
//! `ExitVehicleOpenDoor`, `ExitVehicle`, `ExitVehicleCloseDoor` through a
//! shut door, else `ExitVehicle` alone. The clips' own events open and shut
//! the door: `opendoor` at 0.2 s into her `EnterVehicleOpenDoor` on the
//! Valiant, `closedoor` at 0.3 s into `EnterVehicleCloseDoor`.
//!
//! # Provenance
//!
//! From the 360 code:
//! - The field names: the `Door` component's reflection registration
//!   (0x823ECA18; strings at 0x820E7300 to 0x820E7344), with its defaults:
//!   axis (0, 1, 0), `OpenAngle` 60 (0x82B7D884), both times 1.5
//!   (0x820BB6AC).
//! - The swing (0x824182E8 ticks it, 0x82418480 poses it): opening, a
//!   timer runs up to `TimeToOpen`, then stands at `TimeToClose`; closing,
//!   it runs down to 0. The node turns by `OpenAngle` times the timer over
//!   the time of the way it last went; shut, it stands as modelled. The
//!   angle is in degrees (0x82DE4AEC, pi/180).
//! - Opening and shutting on her clips' events (0x82420208): `OpenDoor`
//!   sends the door of her entrance's `DoorName` the open message,
//!   `CloseDoor` the shut one (bus 0x831C1DC0, 0x82418BA0).
//! - The steps (0x8229F440 starts, 0x822A5990 goes on at each `Complete`,
//!   0x822A0878, 0x822A0958, 0x822A2298): her entrance's door is used when
//!   its `Door` record is there and its node stands within 30 degrees of
//!   shut (0x8229DD28 with 0x8229DC30: the cosine of how far it has
//!   swung at least 0.866, 0x820F95A8); after `EnterVehicle` it is shut
//!   behind her when it stands more than about 18 degrees open (the
//!   cosine at most 0.95, 0x82DE8C28). Getting out through a door always
//!   ends with `ExitVehicleCloseDoor`.
//!
//! From samples: which clips the tables give (see
//! [`m2_assets::animation_table::seat_clip`]) and their events.
//!
//! Not traced: a door whose `Type` is not 0. The 360 turns only those
//! (0x82418480's other branch is not decoded); they are left shut.

use std::collections::HashMap;

use m2_assets::layer::Layer;
use m2_assets::name_hash;
use m2_math::glam::{Mat4, Vec3};

use crate::tuning::Table;

/// `AxisOfRotation` when a record leaves it out.
pub const DEFAULT_AXIS: Vec3 = Vec3::Y;
/// `OpenAngle` when a record leaves it out, degrees.
pub const DEFAULT_OPEN_ANGLE: f32 = 60.0;
/// `TimeToOpen` and `TimeToClose` when a record leaves them out, seconds.
pub const DEFAULT_TIME: f32 = 1.5;
/// A door whose node's cosine from shut is below this is open already: it
/// is not opened again (0x820F95A8).
pub const SHUT_COS: f32 = 0.866;
/// After getting in, a door whose cosine from shut is at most this is shut
/// behind her (0x82DE8C28).
pub const LEFT_OPEN_COS: f32 = 0.95;

/// A vehicle template's `Door` record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Door {
    /// `Name`: what an entrance's `DoorName` gives.
    pub name: u32,
    /// `Type`: 0 swings about its axis.
    pub kind: u32,
    /// `DoorBoneName`: the node that swings.
    pub bone: u32,
    /// `AxisOfRotation`, in the node's own frame.
    pub axis: Vec3,
    /// `OpenAngle`, radians.
    pub open_angle: f32,
    pub time_to_open: f32,
    pub time_to_close: f32,
    pub start_open: bool,
}

impl Door {
    /// Whether the 360 turns it ([`Door::kind`] 0).
    pub fn swings(&self) -> bool {
        self.kind == 0
    }
}

/// Every template's `Door` records, by template id.
pub fn read(db: &Layer<'_>) -> HashMap<u32, Vec<Door>> {
    let t = Table::new(db, "Door");
    let h = name_hash;
    let mut out = HashMap::new();
    for template in t.templates() {
        let doors: Vec<Door> = t
            .of_template(template)
            .iter()
            .filter_map(|r| {
                let name = t.word(r, h("Name")).filter(|&n| n != 0)?;
                let bone = t.word(r, h("DoorBoneName")).filter(|&n| n != 0)?;
                let axis = t
                    .vector(r, h("AxisOfRotation"))
                    .map(Vec3::from)
                    .filter(|a| a.length_squared() > 1e-6)
                    .map_or(DEFAULT_AXIS, Vec3::normalize);
                let time = |f: &str| {
                    t.float(r, h(f))
                        .filter(|&s| s > 0.0)
                        .unwrap_or(DEFAULT_TIME)
                };
                Some(Door {
                    name,
                    kind: t.word(r, h("Type")).unwrap_or(0),
                    bone,
                    axis,
                    open_angle: t
                        .float(r, h("OpenAngle"))
                        .unwrap_or(DEFAULT_OPEN_ANGLE)
                        .to_radians(),
                    time_to_open: time("TimeToOpen"),
                    time_to_close: time("TimeToClose"),
                    start_open: t.word(r, h("StartOpen")).is_some_and(|b| b != 0),
                })
            })
            .collect();
        if !doors.is_empty() {
            out.insert(template, doors);
        }
    }
    out
}

/// Which way a door is going.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Opening,
    Open,
    Closing,
    Shut,
}

/// A door as it swings ([`Door`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Swing {
    pub phase: Phase,
    /// Seconds: up to `TimeToOpen` opening, down from `TimeToClose`
    /// closing.
    pub timer: f32,
}

impl Swing {
    /// As it stands when the vehicle is made: open if `StartOpen`.
    pub fn new(door: &Door) -> Self {
        if door.start_open {
            Self {
                phase: Phase::Open,
                timer: door.time_to_close,
            }
        } else {
            Self {
                phase: Phase::Shut,
                timer: 0.0,
            }
        }
    }

    /// The open message: it swings open from where it stands.
    pub fn open(&mut self) {
        if self.phase != Phase::Open {
            self.phase = Phase::Opening;
        }
    }

    /// The shut message: it swings shut from where it stands.
    pub fn close(&mut self) {
        if self.phase != Phase::Shut {
            self.phase = Phase::Closing;
        }
    }

    /// Runs the timer on `dt` seconds.
    pub fn tick(&mut self, door: &Door, dt: f32) {
        match self.phase {
            Phase::Opening => {
                self.timer += dt;
                if self.timer >= door.time_to_open {
                    self.phase = Phase::Open;
                    self.timer = door.time_to_close;
                }
            }
            Phase::Closing => {
                self.timer -= dt;
                if self.timer <= 0.0 {
                    self.phase = Phase::Shut;
                    self.timer = 0.0;
                }
            }
            Phase::Open | Phase::Shut => {}
        }
    }

    /// How far open, 0 to 1: the timer over the time of the way it is
    /// going.
    pub fn share(&self, door: &Door) -> f32 {
        let of = match self.phase {
            Phase::Shut => return 0.0,
            Phase::Opening => door.time_to_open,
            Phase::Open | Phase::Closing => door.time_to_close,
        };
        (self.timer / of).clamp(0.0, 1.0)
    }

    /// The angle it stands at, radians.
    pub fn angle(&self, door: &Door) -> f32 {
        if door.swings() {
            door.open_angle * self.share(door)
        } else {
            0.0
        }
    }

    /// The cosine of how far it has swung from shut, as 0x8229DC30 has it.
    pub fn cos(&self, door: &Door) -> f32 {
        self.angle(door).cos()
    }

    /// Whether it stands as modelled.
    pub fn is_shut(&self) -> bool {
        self.phase == Phase::Shut
    }
}

/// The node's local transform swung `angle` radians: about the door's axis
/// in the node's own frame.
pub fn swung(rest: Mat4, door: &Door, angle: f32) -> Mat4 {
    rest * Mat4::from_axis_angle(door.axis, angle)
}

/// One of her clips getting in or out, by the action it plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// `EnterVehicleOpenDoor`.
    OpenToEnter,
    /// `EnterVehicle`.
    Enter,
    /// `EnterVehicleCloseDoor`.
    CloseAfterEnter,
    /// `ExitVehicleOpenDoor`.
    OpenToExit,
    /// `ExitVehicle`; `door` when she opened one to get out.
    Exit { door: bool },
    /// `ExitVehicleCloseDoor`.
    CloseAfterExit,
    /// `SeatToSeatTransfer`: over from her seat to another, played at the
    /// seat she leaves (0x822A6FF0 sends it; at its `Complete` 0x822A7198
    /// seats her in the other).
    Transfer,
}

impl Step {
    /// The action's name, as the stance and action table keys it.
    pub fn action(self) -> &'static str {
        match self {
            Step::OpenToEnter => "EnterVehicleOpenDoor",
            Step::Enter => "EnterVehicle",
            Step::CloseAfterEnter => "EnterVehicleCloseDoor",
            Step::OpenToExit => "ExitVehicleOpenDoor",
            Step::Exit { .. } => "ExitVehicle",
            Step::CloseAfterExit => "ExitVehicleCloseDoor",
            Step::Transfer => "SeatToSeatTransfer",
        }
    }

    /// Whether it is getting out.
    pub fn exiting(self) -> bool {
        matches!(
            self,
            Step::OpenToExit | Step::Exit { .. } | Step::CloseAfterExit
        )
    }

    /// The first step getting in, her entrance's door standing at `door`'s
    /// cosine from shut (`None`: no door, or one thrown off).
    pub fn enter(door: Option<f32>) -> Self {
        if door.is_some_and(|c| c >= SHUT_COS) {
            Step::OpenToEnter
        } else {
            Step::Enter
        }
    }

    /// The first step getting out ([`Step::enter`]'s door).
    pub fn exit(door: Option<f32>) -> Self {
        if door.is_some_and(|c| c >= SHUT_COS) {
            Step::OpenToExit
        } else {
            Step::Exit { door: false }
        }
    }

    /// The step after this one completes, the door standing at `door`'s
    /// cosine then; `None` when she is seated, or out.
    pub fn next(self, door: Option<f32>) -> Option<Self> {
        match self {
            Step::OpenToEnter => Some(Step::Enter),
            Step::Enter => door
                .filter(|&c| c <= LEFT_OPEN_COS)
                .map(|_| Step::CloseAfterEnter),
            Step::OpenToExit => Some(Step::Exit { door: true }),
            Step::Exit { door: true } => Some(Step::CloseAfterExit),
            Step::CloseAfterEnter
            | Step::Exit { door: false }
            | Step::CloseAfterExit
            | Step::Transfer => None,
        }
    }
}

/// What a clip event does to her entrance's door: `opendoor` opens it,
/// `closedoor` shuts it (by name hash, whatever the case).
pub fn door_event(name: &str) -> Option<bool> {
    match name_hash(name) {
        h if h == name_hash("OpenDoor") => Some(true),
        h if h == name_hash("CloseDoor") => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valiant_left() -> Door {
        // The Valiant's driver's door as its record has it.
        Door {
            name: name_hash("front_left"),
            kind: 0,
            bone: 1,
            axis: Vec3::Y,
            open_angle: (-60.0f32).to_radians(),
            time_to_open: 0.26,
            time_to_close: 0.2,
            start_open: false,
        }
    }

    #[test]
    fn the_event_names_hash_as_the_360_compares_them() {
        // 0x82420208 compares -0x1cf791d3 and -0x5419a735.
        assert_eq!(name_hash("OpenDoor"), 0xe308_6e2d);
        assert_eq!(name_hash("CloseDoor"), 0xabe6_58cb);
        assert_eq!(door_event("opendoor"), Some(true));
        assert_eq!(door_event("closedoor"), Some(false));
        assert_eq!(door_event("footstep_walk"), None);
    }

    #[test]
    fn a_door_swings_open_evenly_then_stands_open() {
        let d = valiant_left();
        let mut s = Swing::new(&d);
        assert_eq!(s.share(&d), 0.0);
        s.open();
        s.tick(&d, 0.13);
        assert!((s.share(&d) - 0.5).abs() < 1e-5);
        assert!((s.angle(&d) - d.open_angle * 0.5).abs() < 1e-5);
        s.tick(&d, 0.2);
        assert_eq!(s.phase, Phase::Open);
        // Its timer stands at the time to shut, so it reads fully open.
        assert_eq!(s.timer, d.time_to_close);
        assert_eq!(s.share(&d), 1.0);
    }

    #[test]
    fn a_door_swings_shut_over_its_time_to_close() {
        let d = valiant_left();
        let mut s = Swing::new(&d);
        s.open();
        s.tick(&d, 1.0);
        s.close();
        s.tick(&d, 0.1);
        assert_eq!(s.phase, Phase::Closing);
        assert!((s.share(&d) - 0.5).abs() < 1e-5);
        s.tick(&d, 0.1);
        assert!(s.is_shut());
        assert_eq!(s.angle(&d), 0.0);
    }

    #[test]
    fn a_door_made_open_stands_open() {
        let d = Door {
            start_open: true,
            ..valiant_left()
        };
        let s = Swing::new(&d);
        assert_eq!(s.phase, Phase::Open);
        assert_eq!(s.share(&d), 1.0);
    }

    #[test]
    fn a_shut_door_is_opened_getting_in_and_shut_behind_her() {
        let d = valiant_left();
        let shut = Swing::new(&d);
        let first = Step::enter(Some(shut.cos(&d)));
        assert_eq!(first, Step::OpenToEnter);
        assert_eq!(first.action(), "EnterVehicleOpenDoor");
        let mut open = shut;
        open.open();
        open.tick(&d, 1.0);
        let second = first.next(Some(open.cos(&d))).unwrap();
        assert_eq!(second, Step::Enter);
        // 60 degrees open: cos 0.5, at most 0.95.
        let third = second.next(Some(open.cos(&d))).unwrap();
        assert_eq!(third, Step::CloseAfterEnter);
        assert_eq!(third.next(Some(1.0)), None);
    }

    #[test]
    fn without_a_door_she_only_gets_in() {
        assert_eq!(Step::enter(None), Step::Enter);
        assert_eq!(Step::Enter.next(None), None);
    }

    #[test]
    fn an_open_door_is_not_opened_again_nor_a_shut_one_shut() {
        // 40 degrees open (cos 0.77 < 0.866): straight in.
        assert_eq!(Step::enter(Some(40f32.to_radians().cos())), Step::Enter);
        // 29 degrees (cos 0.875): still counts as shut.
        assert_eq!(
            Step::enter(Some(29f32.to_radians().cos())),
            Step::OpenToEnter
        );
        // Left 10 degrees open (cos 0.985 > 0.95): not shut behind her.
        assert_eq!(Step::Enter.next(Some(10f32.to_radians().cos())), None);
        assert_eq!(
            Step::Enter.next(Some(20f32.to_radians().cos())),
            Some(Step::CloseAfterEnter)
        );
    }

    #[test]
    fn the_slide_over_is_a_step_of_its_own() {
        assert_eq!(Step::Transfer.action(), "SeatToSeatTransfer");
        assert_eq!(m2_assets::name_hash(Step::Transfer.action()), 0x13c2_c767);
        assert!(!Step::Transfer.exiting());
        assert_eq!(Step::Transfer.next(Some(1.0)), None);
    }

    #[test]
    fn getting_out_through_a_shut_door_opens_it_and_shuts_it() {
        let first = Step::exit(Some(1.0));
        assert_eq!(first, Step::OpenToExit);
        let second = first.next(Some(0.5)).unwrap();
        assert_eq!(second, Step::Exit { door: true });
        assert_eq!(second.action(), "ExitVehicle");
        let third = second.next(Some(0.5)).unwrap();
        assert_eq!(third, Step::CloseAfterExit);
        assert!(third.exiting());
        assert_eq!(third.next(Some(1.0)), None);
        // Without one: out at once after her clip.
        assert_eq!(Step::exit(None), Step::Exit { door: false });
        assert_eq!(Step::Exit { door: false }.next(None), None);
    }

    #[test]
    fn the_node_turns_about_its_own_axis() {
        let d = valiant_left();
        let rest = Mat4::from_translation(Vec3::new(1.0, 0.5, 0.2));
        let m = swung(rest, &d, std::f32::consts::FRAC_PI_2);
        // The hinge stays put; a point 1 m along x from it swings to -z.
        assert!(
            m.transform_point3(Vec3::ZERO)
                .abs_diff_eq(Vec3::new(1.0, 0.5, 0.2), 1e-5)
        );
        assert!(
            m.transform_point3(Vec3::X)
                .abs_diff_eq(Vec3::new(1.0, 0.5, -0.8), 1e-5)
        );
    }
}
