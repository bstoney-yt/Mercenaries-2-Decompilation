//! Her getting into or out of a vehicle as clips, one after another
//! ([`m2_engine::vehicle_door::Step`]), the door swinging as they say.
//!
//! The use key seats her at once, as before, and starts a [`Boarding`]:
//! while it lasts she can't drive, shoot or get out, only look about.
//! Getting out by the clip (the exit kind `Animated`) she stays seated
//! until her last clip ends, then stands where it leaves her. The clips
//! play in [`crate::hijack::Hijacks::seat`], which has the tables; it
//! ends the boarding. A boarding nothing plays (the tables not loaded, a
//! vehicle without clips) ends on its own after [`UNPLAYED`]: she is in,
//! or out at the door, as before.
//!
//! From the 360 code: getting in she is put on the vehicle at once
//! (0x8229B580) and her first clip starts where it was cut, by the
//! entrance; she is not glided there. Only a dive in (0x8229F890: from the
//! air, 3 m or more off, through a `bUseDive` entrance) glides her, over
//! 1.2 s (0x820C9BB8, 0x8229FDE8); the alpha has no dive in.

use m2_engine::vehicle_door::Step;

/// How long a boarding may go without a clip played before it ends,
/// seconds (Invented: a slow frame or two).
pub const UNPLAYED: f32 = 1.0;

/// Her getting in or out under way.
#[derive(Clone, Debug, PartialEq)]
pub struct Boarding {
    /// The vehicle (index into the scene's vehicles).
    pub vehicle: usize,
    /// Her seat's `SeatLink` name; `None`, the driver's.
    pub seat: Option<u32>,
    /// The entrance she gets in or out through, by its `EntranceLink` name.
    pub entrance: Option<u32>,
    /// Getting out.
    pub exiting: bool,
    /// Over to this seat (`SeatLink` name) once in ([`Step::Transfer`]):
    /// hero.lua slid her over from the seat she got in by, or the seat key.
    pub transfer_to: Option<u32>,
    /// The clip playing: its step, once begun.
    pub step: Option<Step>,
    /// Seconds into that step's clip.
    pub t: f32,
    /// Seconds since a clip was last played for it.
    pub unplayed: f32,
}

impl Boarding {
    pub fn enter(vehicle: usize, seat: Option<u32>, entrance: Option<u32>) -> Self {
        Self {
            vehicle,
            seat,
            entrance,
            exiting: false,
            transfer_to: None,
            step: None,
            t: 0.0,
            unplayed: 0.0,
        }
    }

    pub fn exit(vehicle: usize, seat: Option<u32>, entrance: Option<u32>) -> Self {
        Self {
            exiting: true,
            ..Self::enter(vehicle, seat, entrance)
        }
    }

    /// Over from seat `from` to `to` (seated already).
    pub fn transfer(vehicle: usize, from: Option<u32>, to: u32) -> Self {
        Self {
            transfer_to: Some(to),
            step: Some(Step::Transfer),
            ..Self::enter(vehicle, from, None)
        }
    }

    /// Ages it by `dt` seconds with nothing played; whether it has gone
    /// unplayed too long ([`UNPLAYED`]).
    pub fn age(&mut self, dt: f32) -> bool {
        self.unplayed += dt;
        self.unplayed > UNPLAYED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_boarding_nothing_plays_runs_out() {
        let mut b = Boarding::exit(3, None, None);
        assert!(b.exiting);
        assert!(!b.age(0.9));
        assert!(b.age(0.2));
        // Played, it starts over.
        b.unplayed = 0.0;
        assert!(!b.age(0.1));
    }
}
