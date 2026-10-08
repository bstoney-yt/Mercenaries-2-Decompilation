//! Scripted voice lines: `VO.Cue` and `VO.Cancel`.
//!
//! A conversation (`MrxVoSequence`) cues one line at a time and moves on
//! only when the game calls that line's callback, with the state the line
//! ended in (the script checks only for `"cancel"`). Each cue becomes a
//! [`Command::Say`] for the game to play; the line ends when the game
//! says how long it is ([`Lines::set_length`]) or, if it never does,
//! after [`UNHEARD_SECONDS`]. A cancelled line ends without its callback,
//! as the sequence that cancels it expects.
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

/// How long a line lasts when the game cannot play it (no voices read).
pub const UNHEARD_SECONDS: f64 = 4.0;

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
    callback: Option<Function>,
    /// Game time it started, set on the first tick after the cue.
    started: Option<f64>,
    seconds: Option<f64>,
}

/// The lines playing.
#[derive(Debug, Default)]
pub struct Lines {
    playing: Vec<Line>,
    next: u32,
    /// Lines whose text is to go up (by id), waiting for their length.
    subtitles: Vec<u32>,
}

impl Lines {
    /// Cues `cue` said by `speaker`, with its subtitle if `subtitled`;
    /// returns the command for the game.
    pub fn cue(
        &mut self,
        speaker: Option<u32>,
        cue: String,
        callback: Option<Function>,
        subtitled: bool,
    ) -> Command {
        self.next += 1;
        let id = self.next;
        if subtitled {
            self.subtitles.push(id);
        }
        self.playing.push(Line {
            id,
            speaker,
            cue: cue.clone(),
            callback,
            started: None,
            seconds: None,
        });
        Command::Say { id, speaker, cue }
    }

    /// Stops `cue` said by `speaker` (any line of theirs if `cue` is
    /// `None`), without its callback; returns the commands for the game.
    pub fn cancel(&mut self, speaker: Option<u32>, cue: Option<&str>) -> Vec<Command> {
        let (gone, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.playing)
            .into_iter()
            .partition(|l| {
                l.speaker == speaker && cue.is_none_or(|c| l.cue.eq_ignore_ascii_case(c))
            });
        self.playing = keep;
        gone.into_iter().map(|l| Command::Hush(l.id)).collect()
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
            l.started.get_or_insert(now);
        }
        let (done, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.playing)
            .into_iter()
            .partition(|l| {
                l.started
                    .is_some_and(|s| now - s >= l.seconds.unwrap_or(UNHEARD_SECONDS))
            });
        self.playing = keep;
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

/// Ends the lines that are over at `now` and calls their callbacks
/// (state `"complete"`). Returns each ended cue and any callback error.
pub fn update(world: &Shared, lua: &Lua, now: f64) -> Vec<(String, Option<mlua::Error>)> {
    let done = world.borrow_mut().vo.ended(now);
    done.into_iter()
        .map(|l| {
            let error = l.callback.and_then(|f| {
                let state = lua.create_string("complete").ok()?;
                f.call::<()>(state).err()
            });
            (l.cue, error)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtitles_wait_for_their_line_length() {
        let mut v = Lines::default();
        let id = |c: Command| match c {
            Command::Say { id, .. } => id,
            _ => unreachable!(),
        };
        let a = id(v.cue(None, "a".into(), None, true));
        let _b = id(v.cue(None, "b".into(), None, true));
        let c = id(v.cue(None, "c".into(), None, false));
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
        let d = id(v.cue(Some(7), "d".into(), None, true));
        v.cancel(Some(7), None);
        v.set_length(d, 3.0);
        assert!(v.take_subtitles(1.0).is_empty());
    }
}
