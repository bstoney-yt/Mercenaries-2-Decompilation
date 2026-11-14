//! The GUI's text animations (`AnimateText`), as the 360's text widgets
//! run them. (Image clock wipes are `gui_runtime::clock`'s.)
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
}
