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
//! its subtitle's id). [`Lines::take_subtitles`] hands them out.

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
    /// Cues whose text is to go up.
    subtitles: Vec<String>,
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
        if subtitled {
            self.subtitles.push(cue.clone());
        }
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

    /// The cues whose subtitles are to go up, since the last call.
    pub fn take_subtitles(&mut self) -> Vec<String> {
        std::mem::take(&mut self.subtitles)
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

/// Puts up the subtitles of the lines cued since the last call, through
/// the scripts' `MrxSubtitle.Add`. Returns each cue and any error.
pub fn show_subtitles(world: &Shared, lua: &Lua) -> Vec<(String, Option<mlua::Error>)> {
    let cues = world.borrow_mut().vo.take_subtitles();
    if cues.is_empty() {
        return Vec::new();
    }
    let add = lua
        .load(
            r#"
local m = __m2_module and __m2_module("MrxSubtitle")
local add = type(m) == "table" and m.Add
if type(add) == "function" then return add end
"#,
        )
        .set_name("=m2 subtitle")
        .call::<Option<Function>>(());
    cues.into_iter()
        .map(|cue| {
            let error = match &add {
                Ok(Some(f)) => f.call::<()>(format!("[{cue}]")).err(),
                Ok(None) => Some(mlua::Error::runtime("no MrxSubtitle.Add")),
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
