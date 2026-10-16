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
//! - Leaving: its Resume (`closePause`) and Quit (`quitGame`) run the
//!   scripts' handlers, which ask for the `ingame` and `unloading` game
//!   states ([`News`]).
//!
//! # Provenance
//!
//! From the game's data: the pause layout and `MrxGuiPauseScreen` (which
//! movie, which commands the scripts take) and the `pause_menu` movie (its
//! commands, its test path's calls and boxes, its setters' ranges). The
//! settings' meaning in play (how far 0 to 100 of mouse sensitivity
//! turns, which sounds each volume covers) is invented; see
//! [`Settings`]. Checked against the PC footage's pause menu (1:46-2:04:
//! its five entries, Options' five, the video and controls screens).
//! Nothing from the executable.

use m2_assets::flash::avm1::Value;

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
    /// The window's size and refresh as the video screen writes them.
    pub resolution: String,
    pub refresh: String,
    /// The saves to list on the save screen.
    pub saves: Vec<SaveEntry>,
}

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
            resolution: String::new(),
            refresh: "60".into(),
            saves: Vec::new(),
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
    /// Interface sounds the movie played (`Sound.Play`).
    pub sounds: Vec<String>,
    /// Settings the menu kept that play has nothing for, as `name=value`.
    pub unused: Vec<String>,
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
        self.unused.extend(later.unused);
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
}

impl Native {
    /// A movie (re)started: it is set up again.
    pub fn reset(&mut self) {
        self.set_up = false;
        self.pending.clear();
    }

    /// What to call in the movie now, `settings` being play's: set up as
    /// it starts (once), then the answers to what it asked.
    pub fn calls(&mut self, settings: &Settings) -> Vec<Call> {
        let mut calls = Vec::new();
        if !std::mem::replace(&mut self.set_up, true) {
            self.settings = settings.clone();
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
        }
        calls.append(&mut self.pending);
        calls
    }

    /// What play should do, since last asked.
    pub fn take_news(&mut self) -> News {
        std::mem::take(&mut self.news)
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
            "LTIVideoEnter" => {
                self.pending.extend(self.video_calls());
                true
            }
            "LTIInputGeneralEnter" => {
                self.pending.extend(self.controls_calls());
                true
            }
            _ => false,
        }
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
                self.news.settings = Some(self.settings.clone());
                true
            }
            "pauseCancel" => {
                // Back as they were, sliders heard included.
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
            "Sound.Play" => {
                self.news.sounds.push(arg.to_owned());
                true
            }
            "Sound.Loop" | "Sound.Stop" => true,
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
        let s = &self.settings;
        vec![
            // 1 is wide (`videoModeVar = val == 1`, its WIDE button).
            (
                "LTIVideoSetMode",
                vec![Value::Number(f64::from(u8::from(s.wide)))],
            ),
            ("LTIVideoSetResolution", vec![text(&s.resolution)]),
            ("LTIVideoSetRefresh", vec![text(&s.refresh)]),
        ]
    }
}

impl Settings {
    /// Sets the movie's variable `name` to `value` (`"true"`, `"80"`):
    /// whether play has it.
    fn set(&mut self, name: &str, value: &str) -> bool {
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
            ("controlsMouseSensVar", Some(v)) => self.mouse = v,
            ("gameSensitivityVar", Some(v)) => self.pad = v,
            ("videoGammaVar", Some(v)) => self.brightness = v,
            ("videoViewDistance" | "videoViewDistanceVar", Some(v)) => self.view_distance = v,
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
fn save_args(s: &SaveEntry) -> Vec<Value> {
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
