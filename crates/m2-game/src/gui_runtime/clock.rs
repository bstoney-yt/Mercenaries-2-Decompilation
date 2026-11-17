//! An image widget's clock wipe (`_GuiInternal.SetImageClockAnimation`,
//! the scripts' `ImageWidget:SetClockAnimation`): the image shown as a
//! wedge from its centre that sweeps a full turn clockwise from straight
//! up, growing to the whole image or shrinking to nothing, then a callback.
//! The game uses it for timers: the pursuit timer, an objective's clock
//! icon, the hijack countdown circle.
//!
//! # Provenance
//!
//! From the 360 code (`default.xex`):
//! - 0x8231F0B0 (the native) reads the widget (an image), `start`
//!   (seconds already run), `seconds`, `keep` and `forwards` (true when
//!   not given) and hands them to 0x82385A10.
//! - 0x82385A10 starts it: the hand's angle (+0x188, radians, 0 pointing
//!   right, growing clockwise down the screen) turns at 2 pi / `seconds`
//!   (+0x18C), negated backwards; forwards it starts at `rate * start +
//!   1e-4 - pi/2`, backwards at `rate * start + 3 pi/2 - 1e-4`. While the
//!   angle is short of 3 pi/2 the widget is shown (the same call as
//!   `SetWidgetVisible`, vtable +0x44) and the clock runs (+0x190: 1 to
//!   keep the image at the end, -1 to hide it); otherwise it is hidden.
//! - 0x823856C8 turns the hand each frame; a forward clock ends at 3 pi/2,
//!   a backward one at -pi/2. Then a clock that hides hides its widget,
//!   the clock stops, and the callback (+0x198,
//!   `SetImageClockCallback`, 0x8231F1E0) is called once and dropped.
//! - 0x82385570 draws the image: a pie slice (`SetImagePieSliceRender`,
//!   which also stops the clock) first; else, while the clock runs, the
//!   wedge already swept (0x82386070: straight up round to the hand) when
//!   it keeps forwards or hides backwards, the wedge still to sweep
//!   (0x82385D30: the hand round to straight up) when it hides forwards or
//!   keeps backwards. So a clock that keeps grows to the whole image and
//!   one that hides shrinks to nothing. Both wedges are cut to the image's
//!   box, as the pie slice is.
//! - 0x8231F2D8 (`GetImageClockElapsed`): while running, the seconds run
//!   so far (forwards `(angle + pi/2) / rate`, backwards `(3 pi/2 -
//!   angle) / -rate`); stopped, the whole time (`2 pi / |rate|`), or 0 if
//!   none was set.
//!
//! Inferred: the callback's data table (the scripts pass one) is handed to
//! it as `SetFlashCallback`'s is; the 360 binding's reading of it was not
//! traced.

use std::f32::consts::{FRAC_PI_2, TAU};

/// Where the hand starts past straight up (0x820B7A44).
const NUDGE: f32 = 1e-4;
/// The angle a forward clock ends at: straight up again.
const END: f32 = TAU - FRAC_PI_2;

/// A clock's state, as the image widget holds it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Clock {
    /// The hand's angle, radians.
    pub angle: f32,
    /// Radians per second; below zero backwards.
    pub rate: f32,
    /// 0 stopped, 1 running and keeping the image at the end, -1 running
    /// and hiding it.
    pub state: i8,
}

impl Clock {
    /// Starts a clock (0x82385A10); answers whether the widget shows.
    pub fn start(&mut self, start: f32, seconds: f32, keep: bool, forwards: bool) -> bool {
        let rate = TAU / seconds;
        if forwards {
            self.rate = rate;
            self.angle = rate * start + (NUDGE - FRAC_PI_2);
        } else {
            self.rate = -rate;
            self.angle = -rate * start + (END - NUDGE);
        }
        if self.angle < END {
            self.state = if keep { 1 } else { -1 };
            true
        } else {
            self.state = 0;
            false
        }
    }

    /// Turns the hand by `dt` seconds (0x823856C8). At the end: `Some`
    /// with whether the widget is to be hidden.
    pub fn advance(&mut self, dt: f32) -> Option<bool> {
        if self.state == 0 || self.rate == 0.0 {
            return None;
        }
        self.angle += self.rate * dt;
        let ended = if self.rate > 0.0 {
            self.angle >= END
        } else {
            self.angle <= -FRAC_PI_2
        };
        if !ended {
            return None;
        }
        let hide = self.state < 0;
        self.state = 0;
        Some(hide)
    }

    /// The seconds run (0x8231F2D8).
    pub fn elapsed(&self) -> f32 {
        if self.state == 0 {
            return if self.rate == 0.0 {
                0.0
            } else {
                TAU / self.rate.abs()
            };
        }
        if self.rate <= 0.0 {
            -((TAU - self.angle) - FRAC_PI_2) / self.rate
        } else {
            (self.angle + FRAC_PI_2) / self.rate
        }
    }

    /// The wedge shown while it runs, as a pie slice: degrees clockwise
    /// from straight up (0x82385570).
    pub fn wedge(&self) -> Option<[f32; 2]> {
        if self.state == 0 {
            return None;
        }
        let hand = (self.angle + FRAC_PI_2).to_degrees().clamp(0.0, 360.0);
        let swept = (self.state > 0) == (self.rate > 0.0);
        Some(if swept { [0.0, hand] } else { [hand, 360.0] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forward_clock_that_keeps_grows_to_the_whole_image() {
        let mut c = Clock::default();
        assert!(c.start(0.0, 10.0, true, true));
        assert!(c.elapsed().abs() < 1e-3);
        assert_eq!(c.advance(2.5), None);
        let [from, to] = c.wedge().unwrap();
        assert_eq!(from, 0.0);
        assert!((to - 90.0).abs() < 0.01, "{to}");
        assert!((c.elapsed() - 2.5).abs() < 1e-3);
        assert_eq!(c.advance(7.5), Some(false));
        assert_eq!(c.wedge(), None);
        // Stopped, it reads the whole time.
        assert!((c.elapsed() - 10.0).abs() < 1e-3);
    }

    #[test]
    fn a_forward_clock_that_hides_shrinks_to_nothing() {
        let mut c = Clock::default();
        assert!(c.start(0.0, 4.0, false, true));
        c.advance(1.0);
        let [from, to] = c.wedge().unwrap();
        assert!((from - 90.0).abs() < 0.01 && to == 360.0, "{from} {to}");
        assert_eq!(c.advance(3.0), Some(true));
    }

    #[test]
    fn backward_clocks_turn_the_other_way() {
        let mut keep = Clock::default();
        assert!(keep.start(0.0, 4.0, true, false));
        keep.advance(1.0);
        // Three quarters still to come back: the wedge from 270 grows.
        let [from, to] = keep.wedge().unwrap();
        assert!((from - 270.0).abs() < 0.01 && to == 360.0, "{from} {to}");
        assert!((keep.elapsed() - 1.0).abs() < 1e-3);
        let mut hide = Clock::default();
        hide.start(0.0, 4.0, false, false);
        hide.advance(1.0);
        let [from, to] = hide.wedge().unwrap();
        assert!(from == 0.0 && (to - 270.0).abs() < 0.01, "{from} {to}");
        assert_eq!(hide.advance(3.0), Some(true));
    }

    #[test]
    fn a_clock_started_past_its_end_does_not_run() {
        let mut c = Clock::default();
        // Already 12 of 10 seconds in: hidden, stopped.
        assert!(!c.start(12.0, 10.0, true, true));
        assert_eq!(c.state, 0);
        assert_eq!(c.advance(1.0), None);
        // Starting part way: the hand begins round the dial.
        assert!(c.start(5.0, 10.0, true, true));
        assert!((c.wedge().unwrap()[1] - 180.0).abs() < 0.01);
        assert!(Clock::default().elapsed() == 0.0);
    }
}
