//! Scripted voice lines: `VO.Cue` and `VO.Cancel`.
//!
//! A conversation (`MrxVoSequence`) cues one line at a time and moves on
//! only when the game calls that line's callback, with the state the line
//! ended in (the script checks only for `"cancel"`). Each cue becomes a
//! [`Command::Say`] for the game to play; the line ends when the game
//! says how long it is ([`Lines::set_length`]) or, if it never does,
//! after [`UNHEARD_SECONDS`]. A cancelled line ends without its callback,
//! as the sequence that cancels it expects.

use mlua::{Function, Lua};

use crate::api::{Command, Shared};

/// How long a line lasts when the game cannot play it (no voices read).
pub const UNHEARD_SECONDS: f64 = 4.0;

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
}

impl Lines {
    /// Cues `cue` said by `speaker`; returns the command for the game.
    pub fn cue(
        &mut self,
        speaker: Option<u32>,
        cue: String,
        callback: Option<Function>,
    ) -> Command {
        self.next += 1;
        let id = self.next;
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
