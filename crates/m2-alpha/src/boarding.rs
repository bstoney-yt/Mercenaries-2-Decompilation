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

use m2_engine::vehicle_door::Step;
use m2_math::glam::Vec3;

/// How long a boarding may go without a clip played before it ends,
/// seconds (Invented: a slow frame or two).
pub const UNPLAYED: f32 = 1.0;

/// How long she takes to slide from where she stood to where her first
/// clip getting in starts, seconds, at an even rate. From the 360 code: she
/// is moved there evenly over a set time while the clip plays (0x8229FDE8,
/// the time left over the whole); the time itself was not found (Invented).
pub const SLIDE: f32 = 0.25;

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
    /// Seconds since it began.
    pub elapsed: f32,
    /// Where she stood from where her first clip starts, flat: the slide
    /// still to go is this times [`Boarding::slide_left`].
    pub slide: Vec3,
    /// Where she stood when she pressed the use key, world space.
    pub stood: Option<Vec3>,
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
            elapsed: 0.0,
            slide: Vec3::ZERO,
            stood: None,
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

    /// The share of the slide still to go ([`SLIDE`]).
    pub fn slide_left(&self) -> f32 {
        (1.0 - self.elapsed / SLIDE).clamp(0.0, 1.0)
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

    #[test]
    fn the_slide_to_the_clip_goes_evenly() {
        let mut b = Boarding::enter(0, None, Some(1));
        assert_eq!(b.slide_left(), 1.0);
        b.elapsed = SLIDE / 2.0;
        assert!((b.slide_left() - 0.5).abs() < 1e-6);
        b.elapsed = SLIDE * 2.0;
        assert_eq!(b.slide_left(), 0.0);
    }
}
