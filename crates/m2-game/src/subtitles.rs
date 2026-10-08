//! A movie's subtitles, timed as `MrxGuiCinematic` times them.
//!
//! The scripts that play a movie with subtitles (`WifMissionFlow`'s
//! `_PlayMovie{sMovie = "01_AOA_J", bSubtitles = true}`, through
//! `Hud.Cinematic:Show`) have them read from a script of their own,
//! `Subtitles_<movie>` (`TECHNOV` for `01_VIK_01`), which sets
//! `SubtitleData`: a list of `{ time, text, seconds, top }`. `time` is
//! seconds into the movie, `text` a text id in brackets
//! (`[VzaCon001.Credits.001]`), `seconds` how long it stays (3 if not
//! given) and `top` marks a line shown at the top of the screen (a
//! caption such as a place name) rather than at the bottom.
//!
//! `BeginSubtitles` sorts the lines by time and, when the player has
//! subtitles off (`Sys.SubtitlesEnabled`), keeps only the top ones.
//! `HandleSubtitleUpdate`, every GUI update, counts down what is up and
//! clears it when its time runs out, then puts up every line whose time
//! the movie has reached (its frame over 30 per second), lines due
//! together joined by a blank line (`[n][n]`). A line at time 0 or before
//! stops the track there: the scripts look for the next line only while
//! its time is above 0.
//!
//! # Provenance
//!
//! Behaviour read from the game's `MrxGuiCinematic` and subtitle scripts
//! in a legally installed copy. No text from the game is here.

/// How long a line stays up when it does not say.
pub const LINE_SECONDS: f64 = 3.0;
/// Seconds per movie frame, as the scripts count the movie's time
/// (`GetCurrentFrame() * 0.0333...`).
pub const FRAME_SECONDS: f64 = 1.0 / 30.0;
/// What joins lines due at once.
const JOIN: &str = "[n][n]";

/// One line of a movie's subtitles.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Seconds into the movie.
    pub time: f64,
    /// The text, as the scripts write it (a text id in brackets).
    pub text: String,
    /// How long it stays up; [`LINE_SECONDS`] if `None`.
    pub seconds: Option<f64>,
    /// Shown at the top of the screen, whether subtitles are on or not.
    pub top: bool,
}

/// A text up and the seconds it has left.
#[derive(Clone, Debug, PartialEq)]
struct Up {
    text: String,
    left: f64,
}

/// A movie's subtitles as it plays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    lines: Vec<Line>,
    next: usize,
    bottom: Option<Up>,
    top: Option<Up>,
}

impl Track {
    /// The lines to show: all of them with subtitles on, else only the top
    /// ones, in order of time.
    pub fn new(mut lines: Vec<Line>, enabled: bool) -> Self {
        if !enabled {
            lines.retain(|l| l.top);
        }
        lines.sort_by(|a, b| a.time.total_cmp(&b.time));
        Self {
            lines,
            ..Self::default()
        }
    }

    /// Whether there is nothing to show, now or later.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// One update: `now` is the movie's time (see [`FRAME_SECONDS`]) and
    /// `dt` the seconds since the last update.
    pub fn update(&mut self, now: f64, dt: f64) {
        for up in [&mut self.bottom, &mut self.top] {
            if let Some(u) = up {
                u.left -= dt;
                if u.left <= 0.0 {
                    *up = None;
                }
            }
        }
        let mut bottom: Option<String> = None;
        let mut top: Option<String> = None;
        while let Some(line) = self.lines.get(self.next) {
            if !(line.time > 0.0 && line.time <= now) {
                break;
            }
            let seconds = line.seconds.unwrap_or(LINE_SECONDS);
            let (text, up) = if line.top {
                (&mut top, &mut self.top)
            } else {
                (&mut bottom, &mut self.bottom)
            };
            let joined = match text.take() {
                None => line.text.clone(),
                Some(t) => format!("{t}{JOIN}{}", line.text),
            };
            *up = Some(Up {
                text: joined.clone(),
                left: seconds,
            });
            *text = Some(joined);
            self.next += 1;
        }
    }

    /// The text at the bottom of the screen, if any.
    pub fn bottom(&self) -> Option<&str> {
        self.bottom.as_ref().map(|u| u.text.as_str())
    }

    /// The text at the top of the screen, if any.
    pub fn top(&self) -> Option<&str> {
        self.top.as_ref().map(|u| u.text.as_str())
    }
}

/// The movie's subtitles script, as `ShowMovie` names it.
pub fn script_name(movie: &str) -> String {
    if movie == "01_VIK_01" {
        "TECHNOV".to_owned()
    } else {
        format!("Subtitles_{movie}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(time: f64, text: &str, seconds: Option<f64>, top: bool) -> Line {
        Line {
            time,
            text: text.to_owned(),
            seconds,
            top,
        }
    }

    #[test]
    fn lines_come_up_at_their_time_and_go_after_theirs() {
        let mut t = Track::new(
            vec![
                line(2.0, "b", None, false),
                line(1.0, "a", Some(0.5), false),
            ],
            true,
        );
        t.update(0.9, 0.1);
        assert_eq!(t.bottom(), None);
        t.update(1.0, 0.1);
        assert_eq!(t.bottom(), Some("a"));
        t.update(1.4, 0.4);
        assert_eq!(t.bottom(), Some("a"));
        t.update(1.6, 0.2);
        assert_eq!(t.bottom(), None);
        t.update(2.0, 0.4);
        assert_eq!(t.bottom(), Some("b"));
        t.update(4.9, 2.9);
        assert_eq!(t.bottom(), Some("b"));
        t.update(5.1, 0.2);
        assert_eq!(t.bottom(), None);
    }

    #[test]
    fn lines_due_together_are_joined_and_the_next_replaces_them() {
        let mut t = Track::new(
            vec![
                line(1.0, "a", None, false),
                line(1.1, "b", None, false),
                line(2.0, "c", None, false),
            ],
            true,
        );
        t.update(1.2, 0.2);
        assert_eq!(t.bottom(), Some("a[n][n]b"));
        t.update(2.0, 0.8);
        assert_eq!(t.bottom(), Some("c"));
    }

    #[test]
    fn with_subtitles_off_only_the_top_lines_show() {
        let mut t = Track::new(
            vec![
                line(1.0, "said", None, false),
                line(1.0, "place", Some(2.0), true),
            ],
            false,
        );
        t.update(1.0, 0.0);
        assert_eq!(t.bottom(), None);
        assert_eq!(t.top(), Some("place"));
        t.update(3.0, 2.0);
        assert_eq!(t.top(), None);
    }

    #[test]
    fn a_line_at_time_zero_stops_the_track() {
        let mut t = Track::new(
            vec![line(0.0, "x", None, false), line(1.0, "y", None, false)],
            true,
        );
        t.update(5.0, 0.1);
        assert_eq!(t.bottom(), None);
    }

    #[test]
    fn scripts_are_named_after_their_movies() {
        assert_eq!(script_name("01_AOA_J"), "Subtitles_01_AOA_J");
        assert_eq!(script_name("01_VIK_01"), "TECHNOV");
    }
}
