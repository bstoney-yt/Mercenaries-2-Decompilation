//! Scripted voice lines: `VO.Cue` and `VO.Cancel`.
//!
//! A conversation (`MrxVoSequence`) cues one line at a time and moves on
//! only when the game calls that line's callback, with the state the line
//! ended in (the script checks only for `"cancel"`). Each cue becomes a
//! [`Command::Say`] for the game to play; the line ends when the game
//! says how long it is ([`Lines::set_length`]) or, if it never does,
//! after [`UNHEARD_SECONDS`].
//!
//! Which lines may play is the 360's voice manager's rule
//! ([`Lines::cue`]): each line has a priority (the lower the number the
//! more important; 4 unless the script gives one), a speaker says one
//! thing at a time, and a line that may not play, or is cut off, has its
//! callback called with `"failed"` or `"cancel"`.
//!
//! From the 360 code (the voice manager at `0x82C4CBB0`):
//!
//! - `VO.Cue` (`0x82349000`) reads the speaker, the cue, the callback
//!   and its values, a priority ([`DEFAULT_PRIORITY`] when left out) and
//!   a flag, and answers what the manager's add (`0x82277B38`) does.
//! - The add refuses the line when [`MAX_LINES`] are playing; when the
//!   manager is in cinematic mode (`VO.SetCinematicMode`, `0x82278118`)
//!   and the line's priority is not 0; or when the speaker has a
//!   sequence (`VO.AddSequence`) or a line playing with a lower priority
//!   number than the new line's. A refused line's callback is called
//!   with `"failed"` (`0x822786F0`) and the call answers false.
//! - Otherwise the speaker's lines whose priority number is the new
//!   line's or higher are cancelled (`0x822783B0`), and the line starts.
//! - A line taken out is always told to its callback (`0x82278658`):
//!   `"cancel"` when it was cancelled or its sound failed, `"complete"`
//!   when its sound finished.
//! - A line's subtitle goes up as its sound starts (`0x822782D8`,
//!   `0x822787B8`): `SubtitleBuffer.AddMessage` with the text
//!   `[0x<cue hash>]`, the buffer cleared, priority 1 and, for its
//!   duration, the cue's longest length, or 5 s when that is not known
//!   ([`UNHEARD_SECONDS`]).
//!
//! Here the callbacks of refused and cancelled lines are called on the
//! next [`update`], not inside the script's own call as on the 360
//! (they are Lua functions, and the script world is held while a native
//! runs).
//!
//! A line cued with `Cue` (not `CueWithoutSubtitles`) also has its text put
//! up, when the player has subtitles on: the game hands it to the scripts'
//! own `MrxSubtitle.Add`, which no script calls (it is the engine's way
//! in: its messages stay five seconds and clear the buffer, and missions
//! call `MrxSubtitle.ClearPending` when they cancel their lines). The text
//! is the line's text id, the cue's name (the voice tables key each line by
//! its subtitle's id). [`Lines::take_subtitles`] hands them out once the
//! line's length is known, and [`show_subtitles`] keeps each up for that
//! long instead of the module's five seconds, so the subtitles keep pace
//! with the voice.

use mlua::{Function, Lua};

use crate::api::{Command, Shared};

/// How long a line lasts when the game cannot play it (no voices read):
/// what the 360 shows a subtitle for when its cue has no length
/// (`0x822787B8`, 5.0 at `0x820BAF84`). That the line itself ends then
/// is invented (the 360 waits for its sound).
pub const UNHEARD_SECONDS: f64 = 5.0;

/// The most lines playing at once.
pub const MAX_LINES: usize = 32;

/// A line's priority when the script gives none.
pub const DEFAULT_PRIORITY: i32 = 4;

/// How long a subtitle waits for its line's length before going up
/// without it.
pub const LENGTH_WAIT: f64 = 0.25;

/// A line cued and not yet ended.
#[derive(Debug)]
struct Line {
    id: u32,
    /// Who says it; `None` over the radio (speaker 0).
    speaker: Option<u32>,
    cue: String,
    /// The lower, the more important.
    priority: i32,
    callback: Option<Function>,
    /// Game time it started, set on the first tick after the cue.
    started: Option<f64>,
    seconds: Option<f64>,
}

/// A line of a speaker's starting or stopping, for what goes with their
/// voice: their face.
///
/// From the 360 code: as a line's sound starts the voice manager posts
/// the speaker a face animation or a talking expression (`0x822782D8`),
/// and as the line is taken out, however it ends, it ends it
/// (`0x82278440`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speech {
    /// Line `id` of `cue` (its name's hash) started; `speaker` is `None`
    /// over the radio.
    Started {
        id: u32,
        speaker: Option<u32>,
        cue: u32,
    },
    /// The line stopped: it ended, was cut off or was cancelled.
    Stopped {
        id: u32,
        speaker: Option<u32>,
        cue: u32,
    },
}

/// A line being said, and how far into it the voice is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Speaking {
    pub id: u32,
    pub speaker: Option<u32>,
    /// The cue's name hash.
    pub cue: u32,
    /// Seconds since the line started.
    pub clock: f64,
    /// How long the line lasts, once the game has said.
    pub seconds: Option<f64>,
}

/// The lines playing.
#[derive(Debug, Default)]
pub struct Lines {
    /// Starts and stops since last taken ([`Lines::take_speech`]).
    speech: Vec<Speech>,
    playing: Vec<Line>,
    next: u32,
    /// Lines whose text is to go up (by id), waiting for their length.
    subtitles: Vec<u32>,
    /// `VO.SetCinematicMode`: only lines of priority 0 may play.
    pub cinematic: bool,
    /// Callbacks owed to lines refused or cancelled, with the state to
    /// tell each ([`update`] calls them).
    owed: Vec<(String, Function, &'static str)>,
}

impl Lines {
    /// Cues `cue` said by `speaker` at `priority`, with its subtitle if
    /// `subtitled`. `sequences` are the (speaker, priority) pairs
    /// `VO.AddSequence` holds (the radio's speaker is 0).
    ///
    /// Returns the commands for the game: the speaker's lines it cuts
    /// off hushed, then the line said. `None` when the line may not play
    /// (see the module's notes); its callback is then owed `"failed"`.
    pub fn cue(
        &mut self,
        speaker: Option<u32>,
        cue: String,
        callback: Option<Function>,
        subtitled: bool,
        priority: i32,
        sequences: &[(u32, i32)],
    ) -> Option<Vec<Command>> {
        let outranked = sequences
            .iter()
            .any(|&(s, p)| s == speaker.unwrap_or(0) && p < priority)
            || self
                .playing
                .iter()
                .any(|l| l.speaker == speaker && l.priority < priority);
        if self.playing.len() >= MAX_LINES || (self.cinematic && priority != 0) || outranked {
            if let Some(f) = callback {
                self.owed.push((cue, f, "failed"));
            }
            return None;
        }
        let mut commands = self.take_out(|l| l.speaker == speaker && priority <= l.priority);
        self.next += 1;
        let id = self.next;
        if subtitled {
            self.subtitles.push(id);
        }
        self.playing.push(Line {
            id,
            speaker,
            cue: cue.clone(),
            priority,
            callback,
            started: None,
            seconds: None,
        });
        commands.push(Command::Say { id, speaker, cue });
        Some(commands)
    }

    /// Takes out the lines `which` picks as cancelled: each is hushed
    /// and its callback owed `"cancel"`.
    fn take_out(&mut self, which: impl Fn(&Line) -> bool) -> Vec<Command> {
        let (gone, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.playing)
            .into_iter()
            .partition(which);
        self.playing = keep;
        gone.into_iter()
            .map(|l| {
                // Only a line that started has anything to stop.
                if l.started.is_some() {
                    self.speech.push(Speech::Stopped {
                        id: l.id,
                        speaker: l.speaker,
                        cue: m2_assets::name_hash(&l.cue),
                    });
                }
                if let Some(f) = l.callback {
                    self.owed.push((l.cue, f, "cancel"));
                }
                Command::Hush(l.id)
            })
            .collect()
    }

    /// The lines that started or stopped since last asked, in order.
    pub fn take_speech(&mut self) -> Vec<Speech> {
        std::mem::take(&mut self.speech)
    }

    /// The lines being said at `now`, each with its voice's clock.
    pub fn speaking(&self, now: f64) -> Vec<Speaking> {
        self.playing
            .iter()
            .filter_map(|l| {
                Some(Speaking {
                    id: l.id,
                    speaker: l.speaker,
                    cue: m2_assets::name_hash(&l.cue),
                    clock: now - l.started?,
                    seconds: l.seconds,
                })
            })
            .collect()
    }

    /// Stops `cue` said by `speaker` (any line of theirs if `cue` is
    /// `None`); returns the commands for the game. Each line's callback
    /// is owed `"cancel"`.
    pub fn cancel(&mut self, speaker: Option<u32>, cue: Option<&str>) -> Vec<Command> {
        self.take_out(|l| l.speaker == speaker && cue.is_none_or(|c| l.cue.eq_ignore_ascii_case(c)))
    }

    /// Stops every line (`VO.CancelAll`); each one's callback is owed
    /// `"cancel"`.
    pub fn cancel_all(&mut self) -> Vec<Command> {
        self.take_out(|_| true)
    }

    /// The subtitles to go up now, each with how long its line lasts: the
    /// line's length once the game says it ([`Lines::set_length`]), or
    /// [`UNHEARD_SECONDS`] if it hasn't after [`LENGTH_WAIT`] of play (no
    /// voices). A line cancelled first loses its subtitle.
    pub fn take_subtitles(&mut self, now: f64) -> Vec<(String, f64)> {
        let mut out = Vec::new();
        let playing = &self.playing;
        self.subtitles.retain(|id| {
            let Some(l) = playing.iter().find(|l| l.id == *id) else {
                return false;
            };
            let seconds = match (l.seconds, l.started) {
                (Some(s), _) => s,
                (None, Some(t)) if now - t >= LENGTH_WAIT => UNHEARD_SECONDS,
                _ => return true,
            };
            out.push((l.cue.clone(), seconds));
            false
        });
        out
    }

    /// The game found line `id` to last `seconds`.
    pub fn set_length(&mut self, id: u32, seconds: f64) {
        if let Some(l) = self.playing.iter_mut().find(|l| l.id == id) {
            l.seconds = Some(seconds);
        }
    }

    /// Starts the clock on new lines and takes those that have ended.
    fn ended(&mut self, now: f64) -> Vec<Line> {
        for l in &mut self.playing {
            if l.started.is_none() {
                l.started = Some(now);
                self.speech.push(Speech::Started {
                    id: l.id,
                    speaker: l.speaker,
                    cue: m2_assets::name_hash(&l.cue),
                });
            }
        }
        let (done, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.playing)
            .into_iter()
            .partition(|l| {
                l.started
                    .is_some_and(|s| now - s >= l.seconds.unwrap_or(UNHEARD_SECONDS))
            });
        self.playing = keep;
        for l in &done {
            self.speech.push(Speech::Stopped {
                id: l.id,
                speaker: l.speaker,
                cue: m2_assets::name_hash(&l.cue),
            });
        }
        done
    }
}

/// Puts up the subtitles of the lines playing whose length is known (see
/// [`Lines::take_subtitles`]), through the scripts' `MrxSubtitle.Add`, in
/// step with the voice: each is shown for its line's length (`Add` reads
/// its display time from the module's `_knDisplayDuration`, 5 s, set to
/// the line's for the call; its fade, 0.5 s, follows), and subtitles
/// still queued from earlier lines go first (`MrxSubtitle.ClearPending`,
/// what missions call when they cancel lines), so the buffer never falls
/// behind what is being said. Returns each cue and any error.
pub fn show_subtitles(world: &Shared, lua: &Lua, now: f64) -> Vec<(String, Option<mlua::Error>)> {
    let cues = world.borrow_mut().vo.take_subtitles(now);
    if cues.is_empty() {
        return Vec::new();
    }
    let add = lua
        .load(
            r#"
local text, seconds = ...
local m = __m2_module and __m2_module("MrxSubtitle")
if type(m) ~= "table" or type(m.Add) ~= "function" then return false end
if type(m.ClearPending) == "function" then m.ClearPending() end
local env = getfenv(m.Add)
local was = env._knDisplayDuration
env._knDisplayDuration = seconds
local ok, e = pcall(m.Add, text)
env._knDisplayDuration = was
if not ok then error(e, 0) end
return true
"#,
        )
        .set_name("=m2 subtitle")
        .into_function();
    cues.into_iter()
        .map(|(cue, seconds)| {
            let error = match &add {
                Ok(f) => match f.call::<bool>((format!("[{cue}]"), seconds)) {
                    Ok(true) => None,
                    Ok(false) => Some(mlua::Error::runtime("no MrxSubtitle.Add")),
                    Err(e) => Some(e),
                },
                Err(e) => Some(e.clone()),
            };
            (cue, error)
        })
        .collect()
}

/// Calls the callbacks owed to lines refused or cancelled (`"failed"`,
/// `"cancel"`), then ends the lines that are over at `now` and calls
/// theirs (`"complete"`). Returns each such cue, the state it was told
/// and any callback error.
pub fn update(
    world: &Shared,
    lua: &Lua,
    now: f64,
) -> Vec<(String, &'static str, Option<mlua::Error>)> {
    let tell = |f: Function, state: &str| {
        let state = lua.create_string(state).ok()?;
        f.call::<()>(state).err()
    };
    let owed = std::mem::take(&mut world.borrow_mut().vo.owed);
    let mut out: Vec<_> = owed
        .into_iter()
        .map(|(cue, f, state)| (cue, state, tell(f, state)))
        .collect();
    let done = world.borrow_mut().vo.ended(now);
    out.extend(done.into_iter().map(|l| {
        (
            l.cue,
            "complete",
            l.callback.and_then(|f| tell(f, "complete")),
        )
    }));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The id of the line a cue's commands say.
    fn said(commands: Option<Vec<Command>>) -> u32 {
        match commands.expect("the line may play").last() {
            Some(Command::Say { id, .. }) => *id,
            _ => unreachable!(),
        }
    }

    #[test]
    fn a_speaker_says_one_thing_at_a_time_the_more_important_winning() {
        let mut v = Lines::default();
        let cue = |v: &mut Lines, speaker, name: &str, priority| {
            v.cue(speaker, name.into(), None, false, priority, &[])
        };
        let a = said(cue(&mut v, Some(7), "a", 4));
        // Someone else's line plays alongside.
        let other = said(cue(&mut v, Some(8), "other", 4));
        // The same speaker at the same priority cuts the first off.
        let b = cue(&mut v, Some(7), "b", 4).unwrap();
        assert_eq!(b[0], Command::Hush(a));
        let b = said(Some(b));
        // A less important line may not play over it.
        assert_eq!(cue(&mut v, Some(7), "c", 5), None);
        // A more important one cuts it off.
        let d = cue(&mut v, Some(7), "d", 1).unwrap();
        assert_eq!(d[0], Command::Hush(b));
        // The radio (no speaker) is a speaker like any other.
        let r = said(cue(&mut v, None, "r", 2));
        assert_eq!(cue(&mut v, None, "s", 3), None);
        assert_eq!(cue(&mut v, None, "t", 2).unwrap()[0], Command::Hush(r));
        assert!(v.playing.iter().any(|l| l.id == other));
    }

    #[test]
    fn a_sequence_of_the_speaker_s_outranks_a_line() {
        let mut v = Lines::default();
        // The speaker holds a sequence at priority 2.
        let sequences = [(7, 2)];
        assert_eq!(v.cue(Some(7), "a".into(), None, false, 4, &sequences), None);
        assert!(
            v.cue(Some(7), "a".into(), None, false, 2, &sequences)
                .is_some()
        );
        assert!(
            v.cue(Some(8), "b".into(), None, false, 4, &sequences)
                .is_some()
        );
        // The radio's sequences are speaker 0's.
        assert_eq!(v.cue(None, "r".into(), None, false, 4, &[(0, 1)]), None);
    }

    #[test]
    fn cinematic_mode_lets_only_priority_0_play_and_32_is_the_most() {
        let mut v = Lines {
            cinematic: true,
            ..Lines::default()
        };
        assert_eq!(v.cue(None, "a".into(), None, false, 4, &[]), None);
        assert!(v.cue(None, "a".into(), None, false, 0, &[]).is_some());
        let mut v = Lines::default();
        for speaker in 0..MAX_LINES as u32 {
            assert!(
                v.cue(Some(speaker + 1), "a".into(), None, false, 4, &[])
                    .is_some()
            );
        }
        assert_eq!(v.cue(Some(99), "a".into(), None, false, 0, &[]), None);
    }

    #[test]
    fn a_line_s_start_and_stop_are_told_with_its_clock() {
        let mut v = Lines::default();
        let cue = m2_assets::name_hash("a");
        let a = said(v.cue(Some(7), "a".into(), None, false, 4, &[]));
        // Cued, not yet started: nothing to tell.
        assert!(v.take_speech().is_empty());
        assert!(v.speaking(0.0).is_empty());
        v.ended(10.0);
        let started = Speech::Started {
            id: a,
            speaker: Some(7),
            cue,
        };
        assert_eq!(v.take_speech(), [started]);
        v.set_length(a, 2.0);
        let now = v.speaking(10.5);
        assert_eq!(now.len(), 1);
        assert_eq!(
            (now[0].speaker, now[0].cue, now[0].clock, now[0].seconds),
            (Some(7), cue, 0.5, Some(2.0))
        );
        // It runs out: stopped, once.
        v.ended(12.0);
        let stopped = Speech::Stopped {
            id: a,
            speaker: Some(7),
            cue,
        };
        assert_eq!(v.take_speech(), [stopped]);
        assert!(v.take_speech().is_empty() && v.speaking(12.0).is_empty());
        // A started line cut off by the speaker's next stops too; one
        // cancelled before it started tells nothing.
        let b = said(v.cue(Some(7), "a".into(), None, false, 4, &[]));
        v.ended(13.0);
        v.take_speech();
        v.cue(Some(7), "c".into(), None, false, 4, &[]);
        assert_eq!(
            v.take_speech(),
            [Speech::Stopped {
                id: b,
                speaker: Some(7),
                cue
            }]
        );
        v.cancel_all();
        assert!(v.take_speech().is_empty());
    }

    #[test]
    fn subtitles_wait_for_their_line_length() {
        let mut v = Lines::default();
        // Three speakers, so that none cuts another off.
        let a = said(v.cue(Some(1), "a".into(), None, true, 4, &[]));
        let _b = said(v.cue(Some(2), "b".into(), None, true, 4, &[]));
        let c = said(v.cue(Some(3), "c".into(), None, false, 4, &[]));
        // Nothing until a length is known or the wait is over.
        assert!(v.take_subtitles(0.0).is_empty());
        v.set_length(a, 2.5);
        v.set_length(c, 1.0);
        assert_eq!(v.take_subtitles(0.0), [("a".to_owned(), 2.5)]);
        // Unheard: shown for the fallback once the clock started and the
        // wait passed.
        v.ended(0.0);
        assert!(v.take_subtitles(0.1).is_empty());
        assert_eq!(v.take_subtitles(0.3), [("b".to_owned(), UNHEARD_SECONDS)]);
        // A cancelled line's subtitle never goes up.
        let d = said(v.cue(Some(7), "d".into(), None, true, 4, &[]));
        v.cancel(Some(7), None);
        v.set_length(d, 3.0);
        assert!(v.take_subtitles(1.0).is_empty());
    }
}
