//! The GUI's text animations (`AnimateText`) and image clock wipes
//! (`SetImageClockAnimation`), as the 360's widgets run them.
//!
//! From the 360 code:
//!
//! - `AnimateText(id, mode)` (`0x82320250`) takes the mode by its name's
//!   hash: `"fadein"` 1, `"fadeout"` 2, `"typewriter"` 3; any other does
//!   nothing. On a text widget with text it sets the mode (`+0x144`) and
//!   starts its clock (`+0x13C`) at 0. The widget's update (`0x8238A248`)
//!   adds the frame's seconds to the clock while it is 0 or more; idle it
//!   is -1 (`0x82DE896C`). `HaltTextAnimation` (`0x82320398` into
//!   `0x8238AB68`) ends it: mode 0, clock -1, and a fade-out also puts the
//!   widget away (its vtable's `+0x44` with 0).
//! - The draw (`0x8238DE68`) goes through the text's characters in order,
//!   across its runs, character `i` starting `i` steps on: a step of 1/30 s
//!   for the typewriter (`0x820F8C70`), 1/15 s for the fades
//!   (`0x820BAFBC`). Each character's own progress is three times its
//!   seconds since its start (`0x820C7A20`), at most 1 (`0x820BAEE8`). The
//!   typewriter draws a character whole once its progress is above 0, and
//!   nothing after the first not yet started; a fade-in draws it squeezed
//!   to its progress across its width, about its middle (half, `0x820D9EDC`);
//!   a fade-out draws it at one less its progress, and the rest whole after
//!   the first still whole. Once the clock is past the last character the
//!   widget marks itself done (`+0x130`), and its next update halts the
//!   animation (`0x8238A2B8`, with the fade-out's put-away).
//! - `SetImageClockAnimation(id, start, seconds, keep, emptying)`
//!   (`0x8231F0B0` into `0x82385A10`; the scripts' `ImageWidget:
//!   SetClockAnimation` defaults `start` 0 and `seconds` 10, and the native
//!   `emptying` true) runs an angle (`+0x188`) a whole turn in `seconds`:
//!   emptying, from a quarter turn back (-π/2, plus 1e-4, `0x820B7A44`)
//!   up to three quarters (`2π - π/2`); filling, the other way, both
//!   `start` seconds on already. The image shows the wedge from that angle
//!   round to the top. Past its end at once, it is stopped and put away.
//!   The update (`0x823856C8`) moves the angle on by its rate; at the end
//!   it stops, puts the image away unless `keep`, and calls the clock's
//!   callback (`SetImageClockCallback`, `0x8231F1E0`) once.
//!   `GetImageClockElapsed` (`0x8231F2D8`) answers the seconds run: while
//!   running, from the angle; stopped, the whole `seconds` (0 without a
//!   rate).
//!
//! Inferred: the wedge's place on the image. The draw (`0x82385D30`) fans
//! out from the image's middle from the angle round to the top in quarter
//! turns; the angle is read here as measured clockwise from the right on
//! the screen (y down), so -π/2 is straight up and the wipe sweeps
//! clockwise. Which of the native's two flags is `keep` and which
//! `emptying` is read from their order in the call and the scripts' use
//! (the pursuit meter's gauge empties and goes, the hijack prompt's ring
//! keeps).

use std::f32::consts::{FRAC_PI_2, TAU};

/// How a text animation shows its characters (`AnimateText`'s mode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextMode {
    FadeIn = 1,
    FadeOut = 2,
    Typewriter = 3,
}

impl TextMode {
    /// The mode named `name` (by its hash, as the 360 reads it).
    pub fn named(name: &str) -> Option<TextMode> {
        match m2_assets::name_hash(name) {
            0xF71E_30BE => Some(TextMode::FadeIn),
            0x147A_3E5D => Some(TextMode::FadeOut),
            0xBFD8_A2D8 => Some(TextMode::Typewriter),
            _ => None,
        }
    }

    /// Seconds between one character's start and the next's.
    pub fn step(self) -> f32 {
        match self {
            TextMode::Typewriter => 1.0 / 30.0,
            TextMode::FadeIn | TextMode::FadeOut => 1.0 / 15.0,
        }
    }
}

/// A character's progress per second since its start.
const RATE: f32 = 3.0;

/// A text animation under way: its mode and its clock (seconds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextReveal {
    pub mode: TextMode,
    pub t: f32,
}

impl TextReveal {
    pub fn new(mode: TextMode) -> Self {
        Self { mode, t: 0.0 }
    }

    /// How much of character `i`'s width is drawn (0: not at all, 1:
    /// whole).
    pub fn scale(&self, i: usize) -> f32 {
        let progress = ((self.t - i as f32 * self.mode.step()) * RATE).min(1.0);
        match self.mode {
            TextMode::Typewriter => {
                if progress > 0.0 {
                    1.0
                } else {
                    0.0
                }
            }
            TextMode::FadeIn => progress.max(0.0),
            TextMode::FadeOut => (1.0 - progress.max(0.0)).clamp(0.0, 1.0),
        }
    }

    /// Whether it is over for a text of `count` characters: every
    /// character at its last state.
    pub fn done(&self, count: usize) -> bool {
        let step = self.mode.step();
        match self.mode {
            TextMode::Typewriter => self.t >= count as f32 * step,
            TextMode::FadeIn | TextMode::FadeOut => {
                self.t >= count.saturating_sub(1) as f32 * step + 1.0 / RATE
            }
        }
    }
}

/// The angle a clock stops at, and the one it fills from: three quarters
/// of a turn on from straight up.
const END: f32 = TAU - FRAC_PI_2;
/// Where an emptying clock starts: straight up, a hair on.
const NUDGE: f32 = 1.0e-4;

/// An image's clock wipe (`SetImageClockAnimation`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clock {
    /// Radians, clockwise from the right; the image shows from here
    /// round to the top.
    pub angle: f32,
    /// Radians a second (negative: filling).
    pub rate: f32,
    /// Still running.
    pub running: bool,
    /// Put the image away when it ends.
    pub hide_at_end: bool,
}

/// What starting or running a clock asks of its image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockEvent {
    /// It ended: put the image away if it says so, and call its callback.
    Ended { hide: bool },
}

impl Clock {
    /// A clock `start` seconds into a `seconds` run, keeping its image at
    /// the end if `keep`, emptying (else filling). The second value says
    /// it is over already (the image is put away).
    pub fn start(start: f32, seconds: f32, keep: bool, emptying: bool) -> (Clock, bool) {
        let speed = TAU / seconds;
        let (rate, angle) = if emptying {
            (speed, speed * start + (NUDGE - FRAC_PI_2))
        } else {
            (-speed, -speed * start + (END - NUDGE))
        };
        let running = angle < END;
        (
            Clock {
                angle,
                rate,
                running,
                hide_at_end: !keep,
            },
            !running,
        )
    }

    /// Runs it on `dt` seconds.
    pub fn step(&mut self, dt: f32) -> Option<ClockEvent> {
        if !self.running || self.rate == 0.0 || !self.rate.is_finite() {
            return None;
        }
        self.angle += self.rate * dt;
        let over = if self.rate > 0.0 {
            self.angle >= END
        } else {
            self.angle <= -FRAC_PI_2
        };
        if !over {
            return None;
        }
        self.running = false;
        Some(ClockEvent::Ended {
            hide: self.hide_at_end,
        })
    }

    /// The wedge shown, in degrees clockwise from straight up (as
    /// `SetImagePieSliceRender`'s): from the angle round to the top.
    pub fn pie(&self) -> [f32; 2] {
        let from = (self.angle + FRAC_PI_2).clamp(0.0, TAU).to_degrees();
        [from, 360.0]
    }

    /// The seconds run (`GetImageClockElapsed`).
    pub fn elapsed(&self) -> f32 {
        if self.rate == 0.0 {
            return 0.0;
        }
        if !self.running {
            return TAU / self.rate.abs();
        }
        if self.rate < 0.0 {
            -((TAU - self.angle) - FRAC_PI_2) / self.rate
        } else {
            (self.angle + FRAC_PI_2) / self.rate
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_by_their_names() {
        assert_eq!(TextMode::named("typewriter"), Some(TextMode::Typewriter));
        assert_eq!(TextMode::named("fadein"), Some(TextMode::FadeIn));
        assert_eq!(TextMode::named("fadeout"), Some(TextMode::FadeOut));
        assert_eq!(TextMode::named("scroll"), None);
    }

    /// The typewriter: a character every 1/30 s, whole as it comes.
    #[test]
    fn the_typewriter_shows_thirty_characters_a_second() {
        let mut r = TextReveal::new(TextMode::Typewriter);
        assert_eq!(r.scale(0), 0.0, "nothing at its start");
        r.t = 0.01;
        assert_eq!((r.scale(0), r.scale(1)), (1.0, 0.0));
        r.t = 0.5;
        // 15 characters (0..=14) are out half a second on.
        assert_eq!(r.scale(14), 1.0);
        assert_eq!(r.scale(15), 0.0);
        assert!(!r.done(20));
        r.t = 20.0 / 30.0;
        assert!(r.done(20));
    }

    /// A fade-in grows each character across a third of a second, one
    /// starting every 1/15 s.
    #[test]
    fn a_fade_in_grows_each_character_in_a_third_of_a_second() {
        let r = TextReveal {
            mode: TextMode::FadeIn,
            t: 0.1,
        };
        assert!((r.scale(0) - 0.3).abs() < 1e-5);
        assert!((r.scale(1) - (0.1 - 1.0 / 15.0) * 3.0).abs() < 1e-5);
        assert_eq!(r.scale(2), 0.0);
        let late = TextReveal {
            mode: TextMode::FadeIn,
            t: 1.0,
        };
        assert_eq!(late.scale(0), 1.0);
        // Ten characters: the last starts at 0.6 s and is whole at 0.933;
        // eleven, at 0.667 and whole at 1.0.
        assert!(late.done(10) && late.done(11));
        assert!(!late.done(12));
        let out = TextReveal {
            mode: TextMode::FadeOut,
            t: 0.1,
        };
        assert!((out.scale(0) - 0.7).abs() < 1e-5);
        assert_eq!(out.scale(5), 1.0);
    }

    /// An emptying clock sweeps from the top round to the top in its
    /// seconds, then ends once.
    #[test]
    fn an_emptying_clock_sweeps_a_turn_in_its_seconds() {
        let (mut c, over) = Clock::start(0.0, 4.0, false, true);
        assert!(!over && c.running);
        assert!(c.pie()[0] < 0.01, "whole at the start: {:?}", c.pie());
        assert_eq!(c.step(1.0), None);
        assert!((c.pie()[0] - 90.0).abs() < 0.01, "a quarter gone");
        assert!((c.elapsed() - 1.0).abs() < 1e-3);
        assert_eq!(c.step(2.9), None);
        assert_eq!(c.step(0.2), Some(ClockEvent::Ended { hide: true }));
        assert_eq!(c.step(1.0), None, "once");
        assert_eq!(c.pie()[0], 360.0, "empty");
        assert_eq!(c.elapsed(), 4.0);
    }

    /// A filling clock runs the other way; one started past its end is
    /// over at once.
    #[test]
    fn a_filling_clock_and_one_already_over() {
        let (mut c, _) = Clock::start(0.0, 2.0, true, false);
        assert!(c.pie()[0] > 359.9, "empty at the start");
        assert_eq!(c.step(1.0), None);
        assert!((c.pie()[0] - 180.0).abs() < 0.01);
        assert_eq!(c.step(1.1), Some(ClockEvent::Ended { hide: false }));
        // The pursuit meter's (1, 1): a second into a one-second run.
        let (c, over) = Clock::start(1.0, 1.0, false, true);
        assert!(over && !c.running);
    }
}
