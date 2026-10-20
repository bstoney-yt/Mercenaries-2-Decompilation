//! The game's own pause menu: the pause layout's `MrxGuiPauseScreen`
//! running its movie (`pause_menu`) on the HUD's thread, with the engine's
//! side of it here.
//!
//! The engine opens it by entering the `Pause` game state (the scripts'
//! `GuiGameStateChange` with `Pause`, `Enter`; Escape and the pad's Start,
//! as the game's own pause button). `_FinishPauseOpen` hands the movie
//! what the scripts know (subtitles, tutorials, invert, whether she can
//! be flown out, a contract running) and hooks its commands. Everything
//! else is the PC's library behind its options (`LTILibName`, the engine's,
//! not the scripts') and the engine's answers to the movie's commands,
//! made here:
//!
//! - Set up as it starts: the platform, the territory and the profile's
//!   name, as the movie's own test path (`TESTING`, taken when its
//!   `testingVAR` is set) sets them, with the player's own name and
//!   settings in place of its made-up ones.
//! - Its settings: once loaded (`doneLoadingFlash` says `pauseOpen` and
//!   `loadPauseMenuVariables` through `PauseItemChanged`, which the
//!   scripts hand to `LTILibName.LTIPauseItemChanged`), the library sets
//!   each of them in the movie with its own setter (`LTIAudioSFXVolume`,
//!   `LTIVideoSetVsync`...: 0 to 100, or on and off). Leaving the menu with
//!   changes, the movie asks whether to keep them; kept, `applyChanges`
//!   reports each as `name=value` and then `pauseSave`; dropped,
//!   `pauseCancel`. The volume sliders also say their level as they move
//!   (`audioSetSFXVolume`...), for the player to hear.
//! - Its question boxes: its Save Game, Load Game, Cancel Contract and
//!   Quit buttons ask the engine to put up their box
//!   (`messageQuitGame`...), with the title, text and buttons its test
//!   path puts up itself; the box's own answer goes on (Quit's yes:
//!   `quitGame` and `closePause`).
//! - Its saves: `saveCheck` asks the engine to get them ready
//!   (`preoperation` with "perform,what") and goes on when told
//!   (`preopComplete(true, what)`): listing them (`listLoadGame`), the
//!   engine first hands it each (`addSaveGame`); saving, it says the slot
//!   (`saveGame` with its number).
//! - Its video screen's Resolution and Refresh Rate: the movie keeps no
//!   list. Its arrows ask the engine (`LTIFscommand` with
//!   `LTIVideoNextRes`, `LTIVideoPrevRes`, `LTIVideoNextRefresh`,
//!   `LTIVideoPrevRefresh`, which the scripts hand to the library) and
//!   it shows whatever text comes back (`LTIVideoSetResolution`,
//!   `LTIVideoSetRefresh`). Its `applyChanges` reports neither, so the
//!   engine keeps what it stepped to and applies it on `pauseSave`. The
//!   screen's third item, Aspect Ratio (16:9 or 4:3, `videoModeVar`,
//!   `LTIVideoSwitchMode`), has nothing to do with fullscreen: the movie
//!   has no fullscreen item. So the Refresh Rate list ends in `Windowed`:
//!   a rate means fullscreen at that mode, `Windowed` a window of that
//!   size ([`Video`]). The lists are the monitor's modes ([`resolutions`],
//!   [`rates`]); their texts (`"1920 x 1080"`, `"144 Hz"`) are invented.
//! - Its keyboard and mouse table (Controls, Keyboard/Mouse): filled as
//!   it starts, rebound a cell at a time, kept on Accept
//!   ([`crate::keytable`]).
//! - Leaving: its Resume (`closePause`) and Quit (`quitGame`) run the
//!   scripts' handlers, which ask for the `ingame` and `unloading` game
//!   states ([`News`]).
//! - Its music: entering, the HUD's scripts run `MrxSound.EnterPauseState`
//!   (`TransitionMusic("pause")`, which `MrxSound`'s bindings tie to
//!   `mu_shell_01` for every faction); leaving, `ExitPauseState`
//!   (`TransitionMusic("silence")`). Both reach the score ([`News::music`]);
//!   the mission's scripts bring their piece back at their next
//!   transition (from the scripts; the engine's dynamic music, which
//!   `_RestoreDynamicMusic` turns back on, is not done).
//!
//! # Provenance
//!
//! From the game's data: the pause layout and `MrxGuiPauseScreen` (which
//! movie, which commands the scripts take) and the `pause_menu` movie (its
//! commands, its test path's calls and boxes, its setters' ranges, the
//! video screen's commands and item names from its strings). The
//! settings' meaning in play (how far 0 to 100 of mouse sensitivity
//! turns, which sounds each volume covers) is invented; see
//! [`Settings`]. Checked against the PC footage's pause menu (1:46-2:04:
//! its five entries, Options' five, the video and controls screens).
//! Nothing from the executable.

use m2_assets::flash::avm1::Value;

use crate::input::Control;
use crate::keytable::{KeyTable, ROWS};

/// The menu movie's name.
pub const MOVIE: &str = "pause_menu";

/// A call into the movie: the function's name and its arguments.
pub type Call = (&'static str, Vec<Value>);

/// The question boxes the movie asks the engine for, with what its test
/// path puts up for each (title, text; the buttons are Yes and No).
const BOXES: [(&str, &str, &str); 4] = [
    ("messageQuitGame", "[0x229c06dc]", "[0x0c7173a4]"),
    ("messageSaveGame", "[0x229c06dc]", "[0x29dd2ac5]"),
    ("messageLoadGame", "[0x229c06dc]", "[0x0c7173a4]"),
    ("messageCancelContract", "[0x229c06dc]", "[0xeeeee69f]"),
];
/// The boxes' buttons, as its test path labels them.
const YES: &str = "[0x2d95aa31]";
const NO: &str = "[0x399868dd]";

/// The player's settings as the menu shows them: sliders 0 to 100.
///
/// What they do in play: subtitles and tutorials are the scripts' own
/// (`Sys.SubtitlesEnabled`, `Sys.TutorialsEnabled`); invert mouse, vsync
/// and the volumes the alpha's. Mouse sensitivity 0 to 100 spans
/// 0.0005 to 0.01 radians a count, logarithmically, and the pad's 1/4 to
/// 4 times its turn rate (both invented); SFX covers the world's sounds,
/// the people's barks and the interface, Dialog the scripted lines
/// (inferred from the scripts' sound categories).
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// The profile's name under the menu (the hero's full name).
    pub profile: String,
    pub subtitles: bool,
    pub tutorials: bool,
    pub autosave: bool,
    pub invert_mouse: bool,
    pub mouse: f64,
    pub invert_pad: bool,
    pub pad: f64,
    pub rumble: bool,
    pub vsync: bool,
    pub brightness: f64,
    pub view_distance: f64,
    pub sfx: f64,
    pub music: f64,
    pub dialog: f64,
    /// The window is wide (the video screen's aspect ratio: wide or 4:3).
    pub wide: bool,
    /// How the window is shown: its size, and fullscreen at a rate or a
    /// window ([`Video`]).
    pub video: Video,
    /// The modes the window's monitor offers, for the video screen to
    /// cycle through ([`resolutions`], [`rates`]).
    pub modes: Vec<Mode>,
    /// The saves to list on the save screen.
    pub saves: Vec<SaveEntry>,
    /// The keyboard and mouse table ([`crate::keytable`]).
    pub keys: KeyTable,
    /// The front end's video screen's Graphic Detail (0 to 2), and its
    /// advanced screen's eight options in its setters' order
    /// (`LTIVideoSetSwitchOpt1` to `8`: the first, two detail levels,
    /// shadows, two more levels, shaders, motion blur; on is 1): kept and
    /// shown again, nothing in the alpha's drawing follows them.
    pub detail: f64,
    pub advanced: [f64; 8],
}

/// The front end movie's own starting values for its Graphic Detail and
/// advanced options (`videoVideoDetailVar = 2`, `advvideooption1 = false`,
/// `advvideooption2 = 3`...).
pub const DETAIL: f64 = 2.0;
pub const ADVANCED: [f64; 8] = [0.0, 3.0, 1.0, 1.0, 2.0, 2.0, 1.0, 1.0];

impl Default for Settings {
    fn default() -> Self {
        Self {
            profile: String::new(),
            subtitles: true,
            tutorials: true,
            autosave: true,
            invert_mouse: false,
            mouse: 50.0,
            invert_pad: false,
            pad: 50.0,
            rumble: true,
            vsync: true,
            brightness: 50.0,
            view_distance: 100.0,
            sfx: 100.0,
            music: 100.0,
            dialog: 100.0,
            wide: true,
            video: Video::default(),
            modes: Vec::new(),
            saves: Vec::new(),
            keys: KeyTable::default(),
            detail: DETAIL,
            advanced: ADVANCED,
        }
    }
}

/// Mouse sensitivity in radians a count for the menu's 0 to 100.
pub fn mouse_radians(v: f64) -> f32 {
    (0.0005 * 20f64.powf(v.clamp(0.0, 100.0) / 100.0)) as f32
}

/// The menu's 0 to 100 for a mouse sensitivity.
pub fn mouse_slider(radians: f32) -> f64 {
    let v = (f64::from(radians) / 0.0005).max(1.0).ln() / 20f64.ln() * 100.0;
    v.clamp(0.0, 100.0).round()
}

/// The pad's turn rate, times its default, for the menu's 0 to 100.
pub fn pad_scale(v: f64) -> f64 {
    4f64.powf((v.clamp(0.0, 100.0) - 50.0) / 50.0)
}

/// A mode the monitor offers: its size and refresh rate (whole hertz).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub hz: u32,
}

/// How the window is shown, as the video screen's Resolution and Refresh
/// Rate say it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Video {
    pub width: u32,
    pub height: u32,
    /// Fullscreen at this rate (hertz), or `None`: a window of that size
    /// (the Refresh Rate list's last entry, `Windowed`; invented, as the
    /// movie has no item for it).
    pub fullscreen: Option<u32>,
}

impl Default for Video {
    fn default() -> Self {
        Self {
            width: 1600,
            height: 900,
            fullscreen: None,
        }
    }
}

impl Video {
    /// The Resolution item's text: `"1920 x 1080"` (invented; the movie
    /// shows what it is given).
    pub fn resolution_text(&self) -> String {
        format!("{} x {}", self.width, self.height)
    }

    /// The Refresh Rate item's text: `"144 Hz"`, or `"Windowed"` (both
    /// invented).
    pub fn refresh_text(&self) -> String {
        match self.fullscreen {
            Some(hz) => format!("{hz} Hz"),
            None => WINDOWED.into(),
        }
    }
}

/// The Refresh Rate entry that means a window (invented).
pub const WINDOWED: &str = "Windowed";

/// The sizes the Resolution item cycles through, narrowest first (then shortest): the
/// monitor's, and `current` (a window can be any size) if not among them.
pub fn resolutions(modes: &[Mode], current: (u32, u32)) -> Vec<(u32, u32)> {
    let mut sizes: Vec<(u32, u32)> = modes.iter().map(|m| (m.width, m.height)).collect();
    sizes.push(current);
    sizes.sort_unstable();
    sizes.dedup();
    sizes
}

/// The entries the Refresh Rate item cycles through at `size`: the
/// monitor's rates there, fastest first, then a window.
pub fn rates(modes: &[Mode], size: (u32, u32)) -> Vec<Option<u32>> {
    let mut hz: Vec<u32> = modes
        .iter()
        .filter(|m| (m.width, m.height) == size)
        .map(|m| m.hz)
        .collect();
    hz.sort_unstable_by(|a, b| b.cmp(a));
    hz.dedup();
    hz.into_iter().map(Some).chain([None]).collect()
}

/// The entry `step` (1 next, -1 previous) from `at` in `list`, wrapping;
/// from the start if `at` is not there.
fn cycle<T: PartialEq + Copy>(list: &[T], at: T, step: isize) -> Option<T> {
    let n = list.len() as isize;
    if n == 0 {
        return None;
    }
    // Not there: next is the first, previous the last.
    let i = list
        .iter()
        .position(|&x| x == at)
        .map_or(if step > 0 { -1 } else { 0 }, |i| i as isize);
    Some(list[(i + step).rem_euclid(n) as usize])
}

/// `video` with the Resolution item stepped through the monitor's sizes
/// and `kept` (the window's own): the next size, fullscreen at the same
/// rate if the monitor has it there, else its fastest, a window if it has
/// none.
pub fn step_resolution(video: Video, modes: &[Mode], kept: (u32, u32), step: isize) -> Video {
    let size = (video.width, video.height);
    let Some((width, height)) = cycle(&resolutions(modes, kept), size, step) else {
        return video;
    };
    let fullscreen = video.fullscreen.and_then(|hz| {
        let there = rates(modes, (width, height));
        if there.contains(&Some(hz)) {
            Some(hz)
        } else {
            there[0]
        }
    });
    Video {
        width,
        height,
        fullscreen,
    }
}

/// `video` with the Refresh Rate item stepped.
pub fn step_rate(video: Video, modes: &[Mode], step: isize) -> Video {
    let list = rates(modes, (video.width, video.height));
    Video {
        fullscreen: cycle(&list, video.fullscreen, step).unwrap_or(video.fullscreen),
        ..video
    }
}

/// A save as the save screen lists it (`addSaveGame`'s arguments).
#[derive(Clone, Debug, PartialEq)]
pub struct SaveEntry {
    /// Its slot: what `saveGame` names it by.
    pub number: u32,
    /// `"HH:MM:SS | MM/DD/YYYY"`.
    pub timestamp: String,
    /// 1, 2 or 3: Mattias, Chris or Jennifer (the movie's order).
    pub character: u32,
    pub money: f64,
    pub fuel: f64,
    /// `"H:MM:SS"`.
    pub played: String,
    pub last_contract: String,
    /// The scripts' autosave: not offered to save over.
    pub auto: bool,
    pub percent: f64,
}

/// What the menu asked of play since last taken.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct News {
    /// Resume: the scripts asked for the `ingame` state.
    pub resume: bool,
    /// Quit: the scripts asked for the `unloading` state.
    pub quit: bool,
    /// Save to this slot.
    pub save: Option<u32>,
    /// Load this slot.
    pub load: Option<u32>,
    /// The settings kept (or put back), to apply.
    pub settings: Option<Settings>,
    /// Volumes (SFX, music, dialog) to hear while a slider moves.
    pub volumes: Option<[f64; 3]>,
    /// Interface sounds the movie played (`Sound.Play`, `Sound.Loop`: a
    /// looped cue loops), and those the HUD's scripts cued.
    pub sounds: Vec<String>,
    /// Interface sounds the movie or the HUD's scripts stopped
    /// (`Sound.Stop`: the PDA map's pan loop as the stick lets go).
    pub stops: Vec<String>,
    /// Settings the menu kept that play has nothing for, as `name=value`.
    pub unused: Vec<String>,
    /// The keyboard table wants the next key or mouse button pressed
    /// ([`Native::captured`]), or no longer does.
    pub capture: Option<bool>,
    /// The keyboard table accepted: bind as it says.
    pub keys: Option<KeyTable>,
    /// The video screen's Resolution and Refresh Rate kept: put the
    /// window into them.
    pub video: Option<Video>,
    /// The menu's Medevac was confirmed (`MrxGuiPauseScreen.
    /// _HandleMedEvacEvent` calls `MrxPlayer.MedEvac`): the mission's own
    /// scripts run it (to the sickbay, for its cost).
    pub medevac: bool,
    /// The HUD's scripts changed the score: `MrxSound.EnterPauseState`
    /// goes to the `pause` state (`mu_shell_01`, bound for every faction
    /// in `MrxSound`'s bindings), `ExitPauseState` to `silence` (`None`).
    pub music: Option<Option<String>>,
    /// The front end's Continue: load the newest save.
    pub continue_game: bool,
    /// Delete the save in this slot (the front end's load screen).
    pub delete: Option<u32>,
}

impl News {
    /// Folds `later` in after this.
    pub fn merge(&mut self, later: News) {
        self.resume |= later.resume;
        self.quit |= later.quit;
        self.save = later.save.or(self.save);
        self.load = later.load.or(self.load);
        self.settings = later.settings.or(self.settings.take());
        self.volumes = later.volumes.or(self.volumes);
        self.sounds.extend(later.sounds);
        self.stops.extend(later.stops);
        self.unused.extend(later.unused);
        self.capture = later.capture.or(self.capture);
        self.keys = later.keys.or(self.keys.take());
        self.video = later.video.or(self.video);
        self.medevac |= later.medevac;
        self.music = later.music.or(self.music.take());
        self.continue_game |= later.continue_game;
        self.delete = later.delete.or(self.delete);
    }
}

/// The engine's side of the pause movie.
#[derive(Debug, Default)]
pub struct Native {
    /// The movie has been set up.
    set_up: bool,
    /// The settings as last given or kept.
    settings: Settings,
    /// Calls to make at the next frame.
    pending: Vec<Call>,
    news: News,
    /// The movie's settings play has nothing for (online, advanced
    /// video...), as it last reported them.
    other: std::collections::BTreeMap<String, String>,
    /// The keyboard table's screen.
    table: KeyScreen,
    /// The video screen's Resolution and Refresh Rate as the player has
    /// stepped them, and its Aspect Ratio (wide), applied when the changes
    /// are kept.
    video: Video,
    wide: bool,
}

impl Native {
    /// The video screen's Aspect Ratio as the movie has it now (its
    /// `videoModeVar`, true for wide), kept with the rest on `pauseSave`.
    pub fn set_wide(&mut self, wide: bool) {
        self.wide = wide;
    }

    /// A movie (re)started: it is set up again.
    pub fn reset(&mut self) {
        self.set_up = false;
        self.pending.clear();
        self.table.stop_waiting(&mut self.news);
    }

    /// What to call in the movie now, `settings` being play's: set up as
    /// it starts (once), then the answers to what it asked.
    pub fn calls(&mut self, settings: &Settings) -> Vec<Call> {
        let mut calls = Vec::new();
        if !std::mem::replace(&mut self.set_up, true) {
            self.settings = settings.clone();
            self.video = settings.video;
            self.wide = settings.wide;
            calls.extend([
                ("setPlatform", vec![text("PC")]),
                ("setTerritory", vec![text("EN")]),
                ("profileName", vec![text(&settings.profile)]),
                ("multiplayerHost", vec![Value::Bool(false)]),
                ("multiplayerClient", vec![Value::Bool(false)]),
                ("serverOnline", vec![Value::Bool(false)]),
            ]);
            // Its settings too: a movie the scripts start again
            // (`_FinishPauseOpen` restarts it) may not ask again.
            calls.extend(self.setting_calls());
            // The keyboard table, filled before its screen counts the rows.
            self.table = KeyScreen::new(settings.keys.clone());
            calls.extend(self.table.fill());
        }
        calls.append(&mut self.pending);
        calls
    }

    /// What play should do, since last asked.
    pub fn take_news(&mut self) -> News {
        std::mem::take(&mut self.news)
    }

    /// The HUD's scripts changed the score to `cue` (`None`: silence).
    pub fn music(&mut self, cue: Option<String>) {
        self.news.music = Some(cue);
    }

    /// The game state the scripts asked for.
    pub fn game_state(&mut self, state: &str) {
        match state {
            "ingame" => self.news.resume = true,
            "unloading" => self.news.quit = true,
            _ => {}
        }
    }

    /// A call to the PC's library (`LTILibName.<name>(arg)`) from the
    /// scripts' handlers: whether it is answered.
    pub fn library(&mut self, name: &str, arg: &str) -> bool {
        match name {
            "LTIPauseItemChanged" => self.item_changed(arg),
            // The HUD's stand-in `MrxPlayer.MedEvac` ([`crate::gamehud`]'s
            // `MEDEVAC`): the confirmed Medevac, for the mission to run.
            "M2MedEvac" => {
                self.news.medevac = true;
                true
            }
            "LTIVideoEnter" => {
                self.pending.extend(self.video_calls());
                true
            }
            // The video screen's arrows: the engine steps the item and
            // says what it now reads.
            "LTIVideoNextRes"
            | "LTIVideoPrevRes"
            | "LTIVideoNextRefresh"
            | "LTIVideoPrevRefresh" => {
                let step = if name.contains("Next") { 1 } else { -1 };
                let (modes, kept) = (&self.settings.modes, self.settings.video);
                self.video = if name.ends_with("Res") {
                    step_resolution(self.video, modes, (kept.width, kept.height), step)
                } else {
                    step_rate(self.video, modes, step)
                };
                self.pending.extend(self.video_calls().into_iter().skip(1));
                true
            }
            // Its Aspect Ratio switched between wide and 4:3: which it is
            // now the host reads from the movie ([`Native::set_wide`]),
            // kept with the rest on `pauseSave`.
            "LTIVideoSwitchMode" => true,
            "LTIInputGeneralEnter" => {
                self.pending.extend(self.controls_calls());
                true
            }
            _ => match self
                .table
                .library(name, arg, &mut self.pending, &mut self.news)
            {
                Some(answered) => {
                    self.settings.keys = self.table.kept.clone();
                    answered
                }
                None => false,
            },
        }
    }

    /// The key or mouse button pressed while the keyboard table waited
    /// for one: into its cell, or, Escape, the wait called off.
    pub fn captured(&mut self, control: Control) {
        self.table
            .captured(control, &mut self.pending, &mut self.news);
    }

    fn item_changed(&mut self, arg: &str) -> bool {
        match arg {
            "pauseOpen" | "loadPauseMenuVariables=1" => true,
            // The video screen opened: the window as it is.
            "VideoEnter" => {
                self.pending.extend(self.video_calls());
                true
            }
            "loadPauseMenuVariables" => {
                self.pending.extend(self.setting_calls());
                true
            }
            "pauseSave" => {
                // Its `applyChanges` reports no Resolution or Refresh
                // Rate: the engine's own stepping is what is kept.
                self.settings.video = self.video;
                self.settings.wide = self.wide;
                self.news.video = Some(self.video);
                self.news.settings = Some(self.settings.clone());
                true
            }
            "pauseCancel" => {
                // Back as they were, sliders heard included.
                self.video = self.settings.video;
                self.wide = self.settings.wide;
                self.news.settings = Some(self.settings.clone());
                self.news.volumes =
                    Some([self.settings.sfx, self.settings.music, self.settings.dialog]);
                true
            }
            _ => match arg.split_once('=') {
                Some((name, value)) => {
                    // Those play has nothing for are kept as the movie
                    // has them; a change after the first report is news.
                    if !self.settings.set(name, value) {
                        let old = self.other.insert(name.to_owned(), value.to_owned());
                        if old.is_some_and(|o| o != value) {
                            self.news.unused.push(arg.to_owned());
                        }
                    }
                    true
                }
                None => false,
            },
        }
    }

    /// An `fscommand` no script handles: whether it is the engine's.
    pub fn command(&mut self, command: &str, arg: &str) -> bool {
        match command {
            "Sound.Play" | "Sound.Loop" => {
                self.news.sounds.push(arg.to_owned());
                true
            }
            "Sound.Stop" => {
                self.news.stops.push(arg.to_owned());
                true
            }
            "audioSetSFXVolume"
            | "audioSetFXVolume"
            | "audioSetMusicVolume"
            | "audioSetDialogVolume" => {
                let Ok(v) = arg.trim().parse::<f64>() else {
                    return false;
                };
                let mut volumes = self.news.volumes.unwrap_or([
                    self.settings.sfx,
                    self.settings.music,
                    self.settings.dialog,
                ]);
                let i = match command {
                    "audioSetMusicVolume" => 1,
                    "audioSetDialogVolume" => 2,
                    _ => 0,
                };
                volumes[i] = v.clamp(0.0, 100.0);
                self.news.volumes = Some(volumes);
                true
            }
            "preoperation" => {
                let mut parts = arg.split(',');
                let _perform = parts.next();
                let what = parts.next().unwrap_or("").trim().to_owned();
                if what == "listLoadGame" {
                    self.pending.push(("clearSaveGames", Vec::new()));
                    let saves = self.settings.saves.clone();
                    for s in &saves {
                        self.pending.push(("addSaveGame", save_args(s)));
                    }
                }
                self.pending
                    .push(("preopComplete", vec![Value::Bool(true), text(&what)]));
                true
            }
            "saveGame" => {
                self.news.save = arg.trim().parse().ok();
                self.news.save.is_some()
            }
            "LoadSaveGame" => {
                self.news.load = arg.trim().parse().ok();
                self.news.load.is_some()
            }
            // The profile is play's settings file, written as they change.
            "onlineSetParameters" => true,
            _ => match BOXES.iter().find(|(name, ..)| *name == command) {
                Some((name, title, body)) => {
                    // As its test path: onlineMessage(title, text, 2,
                    // yes, no, the box's name, cancel).
                    self.pending.push((
                        "onlineMessage",
                        vec![
                            text(title),
                            text(body),
                            Value::Number(2.0),
                            text(YES),
                            text(NO),
                            text(name),
                            Value::Bool(true),
                        ],
                    ));
                    true
                }
                None => false,
            },
        }
    }

    /// The movie's setters, with the settings.
    fn setting_calls(&self) -> Vec<Call> {
        let s = &self.settings;
        let n = |v: f64| vec![Value::Number(v.round())];
        let b = |v: bool| vec![Value::Bool(v)];
        let mut calls = vec![
            ("LTIGameSetAutosave", b(s.autosave)),
            ("LTIGameSetTutorial", b(s.tutorials)),
            ("LTIvideoSubtitles", b(s.subtitles)),
            ("LTIAudioSFXVolume", n(s.sfx)),
            ("LTIAudioMusicVolume", n(s.music)),
            ("LTIAudioDialogVolume", n(s.dialog)),
            ("LTIVideoSetGamma", n(s.brightness)),
            ("LTIVideoSetVideoViewDistance", n(s.view_distance)),
            // Its setter reads 1 as off (`videoVsyncVar = val != 1`).
            ("LTIVideoSetVsync", n(if s.vsync { 0.0 } else { 1.0 })),
        ];
        calls.extend(self.controls_calls());
        calls.extend(self.video_calls());
        // Those it has no setter here for (online, advanced video) are
        // its own, taken as they are: its `applyChanges` makes them its
        // current settings (and reports them, as above). Without, leaving
        // the menu asks to keep changes no one made.
        calls.push(("applyChanges", Vec::new()));
        calls
    }

    fn controls_calls(&self) -> Vec<Call> {
        let s = &self.settings;
        vec![
            ("LTIControlsMouseInvert", vec![Value::Bool(s.invert_mouse)]),
            (
                "LTIInputSetMouseSense",
                vec![Value::Number(s.mouse.round())],
            ),
            ("LTIInputSetInvertJoystick", vec![Value::Bool(s.invert_pad)]),
            ("LTIGameSensitivity", vec![Value::Number(s.pad.round())]),
            ("LTIInputSetRumble", vec![Value::Bool(s.rumble)]),
        ]
    }

    fn video_calls(&self) -> Vec<Call> {
        vec![
            // 1 is wide (`videoModeVar = val == 1`, its WIDE button).
            (
                "LTIVideoSetMode",
                vec![Value::Number(f64::from(u8::from(self.wide)))],
            ),
            (
                "LTIVideoSetResolution",
                vec![text(&self.video.resolution_text())],
            ),
            ("LTIVideoSetRefresh", vec![text(&self.video.refresh_text())]),
        ]
    }
}

/// The keyboard and mouse table's screen (Controls, Keyboard/Mouse), as
/// both menus' movies run it: the pause menu's and the front end's make
/// the same calls. Filled as the movie starts, rebound a cell at a time,
/// kept on Accept ([`crate::keytable`]).
#[derive(Debug, Default)]
pub struct KeyScreen {
    /// The table as last accepted.
    pub kept: KeyTable,
    /// The table as the movie shows it.
    keys: KeyTable,
    /// The cell (row, column) waiting for a key.
    waiting: Option<(usize, usize)>,
}

impl KeyScreen {
    pub fn new(kept: KeyTable) -> Self {
        Self {
            keys: kept.clone(),
            kept,
            waiting: None,
        }
    }

    /// The whole table, into the movie.
    pub fn fill(&self) -> Vec<Call> {
        (0..ROWS.len()).flat_map(|r| self.row_calls(r)).collect()
    }

    /// A call to the PC's library: `Some` whether it is answered, if it
    /// is the table's. Its calls into the movie go to `pending`.
    pub fn library(
        &mut self,
        name: &str,
        arg: &str,
        pending: &mut Vec<Call>,
        news: &mut News,
    ) -> Option<bool> {
        Some(match name {
            "LTIInputKMEnter" | "LTIInputKMExit" => {
                // As last accepted: what was changed and not accepted goes.
                self.stop_waiting(news);
                self.keys = self.kept.clone();
                pending.extend(self.fill());
                pending.push(("LTIInputKMUpdateTable", Vec::new()));
                true
            }
            "LTIInputKMChangeInput" => {
                let Ok(cell) = arg.trim().parse::<usize>() else {
                    return Some(false);
                };
                let (row, column) = (cell % 100, cell / 100);
                if row >= ROWS.len() || column > 1 {
                    return Some(false);
                }
                self.waiting = Some((row, column));
                news.capture = Some(true);
                true
            }
            "LTIInputKMCancelInput" => {
                self.stop_waiting(news);
                true
            }
            "LTIInputKMDefault" => {
                self.stop_waiting(news);
                self.keys = KeyTable::default();
                pending.extend(self.fill());
                pending.push(("LTIInputKMUpdateTable", Vec::new()));
                true
            }
            "LTIInputKMApplyChanges" => {
                self.stop_waiting(news);
                self.kept = self.keys.clone();
                news.keys = Some(self.keys.clone());
                true
            }
            _ => return None,
        })
    }

    /// The key or mouse button pressed while the table waited for one:
    /// into its cell, or, Escape, the wait called off.
    pub fn captured(&mut self, control: Control, pending: &mut Vec<Call>, news: &mut News) {
        let Some((row, column)) = self.waiting.take() else {
            return;
        };
        news.capture = Some(false);
        if control == Control::Key(winit::keyboard::KeyCode::Escape) {
            pending.push(("LTIInputKMCancel", Vec::new()));
            return;
        }
        for r in self.keys.set(row, column, control.into()) {
            pending.extend(self.row_calls(r).into_iter().skip(1));
        }
        pending.push(("LTIFinishedGettingInput", vec![Value::Bool(false)]));
        pending.push(("LTIInputKMUpdateTable", Vec::new()));
    }

    /// The table no longer waits for a key.
    pub fn stop_waiting(&mut self, news: &mut News) {
        if self.waiting.take().is_some() {
            news.capture = Some(false);
        }
    }

    /// Row `row`, into the movie: its name, then its primary and
    /// secondary keys.
    fn row_calls(&self, row: usize) -> [Call; 3] {
        let n = Value::Number(row as f64);
        [
            (
                "LTIInputKMSetActionName",
                vec![n.clone(), text(ROWS[row].name)],
            ),
            (
                "LTIInputKMKeyMap",
                vec![n.clone(), Value::Bool(false), text(&self.keys.text(row, 0))],
            ),
            (
                "LTIInputKMKeyMap",
                vec![n, Value::Bool(true), text(&self.keys.text(row, 1))],
            ),
        ]
    }
}

impl Settings {
    /// Sets the movie's variable `name` to `value` (`"true"`, `"80"`):
    /// whether play has it.
    pub fn set(&mut self, name: &str, value: &str) -> bool {
        let on = matches!(value.trim(), "true" | "1");
        let level = value
            .trim()
            .parse::<f64>()
            .ok()
            .map(|v| v.clamp(0.0, 100.0));
        match (name, level) {
            ("gameAutoSaveVar" | "gameAutosaveVar", _) => self.autosave = on,
            ("gameTutorialsVar", _) => self.tutorials = on,
            ("gameSubtitleVar" | "videoSubtitlesVar", _) => self.subtitles = on,
            ("controlsMouseInvertVar", _) => self.invert_mouse = on,
            ("gameInvertVar", _) => self.invert_pad = on,
            ("gameRumbleVar", _) => self.rumble = on,
            ("videoVsyncVar", _) => self.vsync = on,
            // The advanced screen's Shadows (the front end's fourth
            // option).
            ("advvideoEnableShadowsVar", _) => self.advanced[3] = f64::from(u8::from(on)),
            ("controlsMouseSensVar", Some(v)) => self.mouse = v,
            ("gameSensitivityVar", Some(v)) => self.pad = v,
            ("videoGammaVar", Some(v)) => self.brightness = v,
            // Reported 10 less than it is (`videoViewDistance = val - 10` in
            // the setters): 10 to 100.
            ("videoViewDistance", Some(v)) => self.view_distance = (v + 10.0).min(100.0),
            ("videoViewDistanceVar", Some(v)) => self.view_distance = v.max(10.0),
            ("audioSFXVolumeVar", Some(v)) => self.sfx = v,
            ("audioMusicVolumeVar", Some(v)) => self.music = v,
            ("audioDialogVolumeVar", Some(v)) => self.dialog = v,
            _ => return false,
        }
        true
    }
}

/// `secs` since 1970 (UTC) as the save screen writes a save's time:
/// `"HH:MM:SS | MM/DD/YYYY"` (the movie's test path's form).
pub fn timestamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    // Days to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{:02}:{:02}:{:02} | {month:02}/{day:02}/{year}",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

fn text(s: &str) -> Value {
    Value::Str(s.into())
}

/// `addSaveGame(number, Timestamp, Character, Money, Oil, TimePlayed,
/// LastMission, AutoSave, Percentage, Corrupt)`.
pub fn save_args(s: &SaveEntry) -> Vec<Value> {
    vec![
        Value::Number(f64::from(s.number)),
        text(&s.timestamp),
        Value::Number(f64::from(s.character)),
        Value::Number(s.money.round()),
        Value::Number(s.fuel.round()),
        text(&s.played),
        text(&s.last_contract),
        Value::Bool(s.auto),
        Value::Number(s.percent.round()),
        Value::Bool(false),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode;

    fn texts(call: &Call) -> Vec<String> {
        call.1
            .iter()
            .map(|v| match v {
                Value::Str(s) => s.to_string(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => String::new(),
            })
            .collect()
    }

    #[test]
    fn the_pause_states_music_reaches_play_last_one_kept() {
        let mut n = Native::default();
        n.music(Some("mu_shell_01".into()));
        let mut news = n.take_news();
        assert_eq!(news.music, Some(Some("mu_shell_01".into())));
        n.music(None);
        news.merge(n.take_news());
        assert_eq!(news.music, Some(None));
        news.merge(News::default());
        assert_eq!(news.music, Some(None));
    }

    #[test]
    fn the_keyboard_table_is_filled_as_the_movie_starts() {
        let mut n = Native::default();
        let calls = n.calls(&Settings::default());
        let rows: Vec<_> = calls
            .iter()
            .filter(|c| c.0 == "LTIInputKMSetActionName")
            .collect();
        assert_eq!(rows.len(), 24);
        assert_eq!(texts(rows[0]), ["0", "[SHELL.PCShell.MoveForward]"]);
        let keys: Vec<Vec<String>> = calls
            .iter()
            .filter(|c| c.0 == "LTIInputKMKeyMap")
            .map(texts)
            .collect();
        assert_eq!(keys.len(), 48);
        assert_eq!(keys[10], ["5", "false", "TAB"]);
        assert_eq!(keys[11], ["5", "true", "MWHEEL"]);
        assert_eq!(keys[1], ["0", "true", crate::keytable::EMPTY]);
    }

    #[test]
    fn a_cell_takes_the_next_key_and_accept_binds_it() {
        let mut n = Native::default();
        let s = Settings::default();
        n.calls(&s);
        assert!(n.library("LTIInputKMEnter", ""));
        // Jump's secondary cell (row 11, column 2 in the movie: + 100).
        assert!(n.library("LTIInputKMChangeInput", "111"));
        assert_eq!(n.take_news().capture, Some(true));
        n.calls(&s);
        n.captured(Control::Key(KeyCode::KeyQ));
        assert_eq!(n.take_news().capture, Some(false));
        let calls = n.calls(&s);
        assert!(calls.iter().any(|c| texts(c) == ["11", "true", "Q"]));
        assert_eq!(
            names(calls)[2..],
            ["LTIFinishedGettingInput", "LTIInputKMUpdateTable"]
        );
        assert!(n.library("LTIInputKMApplyChanges", ""));
        let kept = n.take_news().keys.unwrap();
        let mut b = crate::input::Bindings::default();
        kept.apply(&mut b);
        assert_eq!(
            b.0[&crate::input::Action::MoveUp],
            vec![KeyCode::Space.into(), KeyCode::KeyQ.into()]
        );
    }

    #[test]
    fn escape_calls_the_wait_off_and_leaving_drops_what_was_not_accepted() {
        let mut n = Native::default();
        let s = Settings::default();
        n.calls(&s);
        assert!(n.library("LTIInputKMChangeInput", "0"));
        n.captured(Control::Key(KeyCode::Escape));
        assert_eq!(names(n.calls(&s)), ["LTIInputKMCancel"]);
        assert!(n.library("LTIInputKMChangeInput", "0"));
        n.captured(Control::Key(KeyCode::KeyP));
        n.calls(&s);
        assert!(n.library("LTIInputKMExit", ""));
        let calls = n.calls(&s);
        assert!(calls.iter().any(|c| texts(c) == ["0", "false", "W"]));
        assert!(n.take_news().keys.is_none());
        assert!(!n.library("LTIInputKMChangeInput", "250"));
    }

    #[test]
    fn defaults_put_the_games_table_back() {
        let mut n = Native::default();
        let mut s = Settings::default();
        s.keys
            .set(0, 0, crate::keytable::Slot::Control(KeyCode::KeyP.into()));
        n.calls(&s);
        assert!(n.library("LTIInputKMDefault", ""));
        let calls = n.calls(&s);
        assert!(calls.iter().any(|c| texts(c) == ["0", "false", "W"]));
        assert_eq!(names(calls).last(), Some(&"LTIInputKMUpdateTable"));
    }

    fn names(calls: Vec<Call>) -> Vec<&'static str> {
        calls.into_iter().map(|c| c.0).collect()
    }

    #[test]
    fn the_movie_is_set_up_once_then_given_its_settings_when_it_asks() {
        let mut n = Native::default();
        let s = Settings {
            profile: "Mattias Nilsson".into(),
            ..Settings::default()
        };
        let first = names(n.calls(&s));
        assert_eq!(
            first[..6],
            [
                "setPlatform",
                "setTerritory",
                "profileName",
                "multiplayerHost",
                "multiplayerClient",
                "serverOnline"
            ]
        );
        assert!(first.contains(&"LTIVideoSetResolution"));
        assert!(n.calls(&s).is_empty());
        assert!(n.library("LTIPauseItemChanged", "pauseOpen"));
        assert!(n.calls(&s).is_empty());
        assert!(n.library("LTIPauseItemChanged", "loadPauseMenuVariables"));
        let calls = names(n.calls(&s));
        assert!(calls.contains(&"LTIAudioSFXVolume") && calls.contains(&"LTIVideoSetVsync"));
        assert!(!n.library("LTICamera", "1"));
        assert!(n.library("LTIPauseItemChanged", "VideoEnter"));
        assert_eq!(
            names(n.calls(&s)),
            [
                "LTIVideoSetMode",
                "LTIVideoSetResolution",
                "LTIVideoSetRefresh"
            ]
        );
        n.reset();
        assert!(n.calls(&s).len() > 6);
    }

    #[test]
    fn kept_changes_reach_play_and_dropped_ones_put_back_the_sliders() {
        let mut n = Native::default();
        n.calls(&Settings::default());
        assert!(n.command("audioSetMusicVolume", "20"));
        assert_eq!(n.take_news().volumes, Some([100.0, 20.0, 100.0]));
        assert!(n.library("LTIPauseItemChanged", "audioMusicVolumeVar=20"));
        assert!(n.library("LTIPauseItemChanged", "controlsMouseInvertVar=true"));
        // Reported as the movie had it (its `applyChanges` at set-up),
        // then changed.
        assert!(n.library("LTIPauseItemChanged", "advvideoMotionblurVar=true"));
        assert!(n.library("LTIPauseItemChanged", "advvideoMotionblurVar=false"));
        assert!(n.library("LTIPauseItemChanged", "pauseSave"));
        let news = n.take_news();
        let s = news.settings.unwrap();
        assert_eq!((s.music, s.invert_mouse), (20.0, true));
        assert_eq!(news.unused, ["advvideoMotionblurVar=false"]);
        assert!(n.command("audioSetSFXVolume", "10"));
        assert!(n.library("LTIPauseItemChanged", "pauseCancel"));
        assert_eq!(n.take_news().volumes, Some([100.0, 20.0, 100.0]));
    }

    #[test]
    fn quit_asks_first_and_the_saves_are_listed_before_their_screen() {
        let mut n = Native::default();
        let mut s = Settings::default();
        s.saves.push(SaveEntry {
            number: 1,
            timestamp: "12:00:00 | 10/16/2026".into(),
            character: 1,
            money: 1000.0,
            fuel: 50.0,
            played: "0:10:00".into(),
            last_contract: "PmcCon001".into(),
            auto: false,
            percent: 1.0,
        });
        n.calls(&s);
        assert!(n.command("messageQuitGame", "true"));
        let calls = n.calls(&s);
        assert_eq!(calls[0].0, "onlineMessage");
        assert_eq!(calls[0].1.len(), 7);
        assert!(n.command("preoperation", "load,listLoadGame"));
        assert_eq!(
            names(n.calls(&s)),
            ["clearSaveGames", "addSaveGame", "preopComplete"]
        );
        assert!(n.command("saveGame", "2"));
        n.game_state("ingame");
        let news = n.take_news();
        assert_eq!((news.save, news.resume, news.quit), (Some(2), true, false));
        n.game_state("unloading");
        assert!(n.take_news().quit);
        assert!(!n.command("kickPlayer", "true"));
    }

    fn monitor() -> Vec<Mode> {
        let m = |width, height, hz| Mode { width, height, hz };
        vec![
            m(2560, 1440, 165),
            m(1920, 1080, 60),
            m(2560, 1440, 60),
            m(1280, 720, 60),
            m(1920, 1080, 144),
            m(1920, 1080, 60),
        ]
    }

    #[test]
    fn the_lists_are_the_monitors_modes_with_a_window_last() {
        let modes = monitor();
        assert_eq!(
            resolutions(&modes, (2560, 1351)),
            [(1280, 720), (1920, 1080), (2560, 1351), (2560, 1440)]
        );
        assert_eq!(resolutions(&modes, (1920, 1080)).len(), 3);
        assert_eq!(rates(&modes, (1920, 1080)), [Some(144), Some(60), None]);
        assert_eq!(rates(&modes, (2560, 1351)), [None]);
        // A size down from 165 Hz: not there, so that size's fastest.
        let fast = Video {
            width: 2560,
            height: 1440,
            fullscreen: Some(165),
        };
        assert_eq!(
            step_resolution(fast, &modes, (2560, 1440), -1).fullscreen,
            Some(144)
        );
        assert_eq!(step_rate(fast, &modes, 1).fullscreen, Some(60));
        let v = Video {
            width: 1920,
            height: 1080,
            fullscreen: Some(144),
        };
        assert_eq!(
            (v.resolution_text(), v.refresh_text()),
            ("1920 x 1080".to_owned(), "144 Hz".to_owned())
        );
        let w = Video {
            fullscreen: None,
            ..v
        };
        assert_eq!(w.refresh_text(), WINDOWED);
    }

    #[test]
    fn the_arrows_step_the_items_and_keeping_applies_them() {
        let mut n = Native::default();
        let s = Settings {
            video: Video {
                width: 2560,
                height: 1351,
                fullscreen: None,
            },
            modes: monitor(),
            ..Settings::default()
        };
        n.calls(&s);
        // The maximized window: the next size up is the monitor's.
        assert!(n.library("LTIVideoNextRes", ""));
        let calls = n.calls(&s);
        assert_eq!(
            calls.iter().map(texts).collect::<Vec<_>>(),
            [vec!["2560 x 1440"], vec!["Windowed"]]
        );
        assert_eq!(
            names(calls),
            ["LTIVideoSetResolution", "LTIVideoSetRefresh"]
        );
        // Previous from Windowed is the slowest rate, then the fastest.
        assert!(n.library("LTIVideoPrevRefresh", ""));
        assert_eq!(texts(&n.calls(&s)[1]), ["60 Hz"]);
        assert!(n.library("LTIVideoPrevRefresh", ""));
        assert_eq!(texts(&n.calls(&s)[1]), ["165 Hz"]);
        // Back to the window's own size, which no mode has: a window.
        assert!(n.library("LTIVideoPrevRes", ""));
        assert_eq!(
            n.calls(&s).iter().map(texts).collect::<Vec<_>>(),
            [vec!["2560 x 1351"], vec!["Windowed"]]
        );
        // Down a size, still a window; next from Windowed wraps to the
        // fastest rate.
        assert!(n.library("LTIVideoPrevRes", ""));
        assert!(n.library("LTIVideoNextRefresh", ""));
        assert_eq!(texts(n.calls(&s).last().unwrap()), ["144 Hz"]);
        assert!(n.library("LTIVideoSwitchMode", ""));
        // Leaving the video screen and coming back shows the choice.
        assert!(n.library("LTIPauseItemChanged", "VideoEnter"));
        assert_eq!(texts(&n.calls(&s)[1]), ["1920 x 1080"]);
        assert!(n.library("LTIPauseItemChanged", "pauseSave"));
        let news = n.take_news();
        let kept = news.settings.unwrap().video;
        assert_eq!(news.video, Some(kept));
        assert_eq!(
            kept,
            Video {
                width: 1920,
                height: 1080,
                fullscreen: Some(144),
            }
        );
        // Dropped: back to what was kept.
        assert!(n.library("LTIVideoNextRes", ""));
        n.calls(&s);
        assert!(n.library("LTIPauseItemChanged", "pauseCancel"));
        let news = n.take_news();
        assert_eq!((news.settings.unwrap().video, news.video), (kept, None));
        assert!(n.library("LTIPauseItemChanged", "VideoEnter"));
        assert_eq!(texts(&n.calls(&s)[1]), ["1920 x 1080"]);
    }

    #[test]
    fn save_times_read_as_the_movie_writes_them() {
        assert_eq!(timestamp(0), "00:00:00 | 01/01/1970");
        // 2026-10-16 12:07:53 UTC.
        assert_eq!(timestamp(1_792_152_473), "12:07:53 | 10/16/2026");
        assert_eq!(timestamp(951_782_400), "00:00:00 | 02/29/2000");
    }

    #[test]
    fn mouse_sensitivity_round_trips_through_the_slider() {
        let default = crate::config::CameraConfig::default().mouse_sensitivity;
        let v = mouse_slider(default);
        assert!((mouse_radians(v) - default).abs() / default < 0.05);
        assert_eq!(mouse_slider(mouse_radians(0.0)), 0.0);
        assert_eq!(mouse_slider(mouse_radians(100.0)), 100.0);
        assert!((pad_scale(50.0) - 1.0).abs() < 1e-9);
    }
}
