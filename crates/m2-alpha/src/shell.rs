//! The front end as the game opens it: the shell's own scripts and Flash
//! menus, run on a [`GameHud`](crate::gamehud::GameHud) thread
//! ([`GameHud::start_shell`](crate::gamehud::GameHud::start_shell)).
//!
//! After the boot movies the game's `MrxGuiShellBootstrap.Init` loads the
//! loading screen's layout and then the movie layouts (the cinematics' and
//! the fades'); `EnterShell` loads `MrxGuiShellLayout`, whose widget's
//! initialisation (`MrxGuiShell.HandleInitializationEvent`) makes a movie
//! widget on the right (the backdrop videos) and a full-screen Flash widget
//! playing `shell.gfx`, and hooks the movie's commands; once loaded
//! (`ShellScreenLoaded`) the bootstrap asks for the `Shell` game state, and
//! the engine's `GuiGameStateChange` (`Shell`, `Enter`) opens it.
//!
//! The movie's commands do the rest: `startMovie`/`stopMovie` with a
//! backdrop's name play it in the movie widget, and starting a new game
//! (`LTIStartNewGame` with the character on the PC; `newGame` elsewhere)
//! sets the character (`MrxGuiShellBootstrap.SetSelectedCharacter`) and
//! calls `Sys.StartSingleplayer`, which here notes the character for the
//! game ([`take_new_game`]). The movie's own `newGame` carries no
//! character: the engine starts the game as the profile's
//! ([`Native::character`], [`start_new_game`]).
//!
//! The menu movie itself waits on the engine: its first frame stops until
//! told the platform, and its press-start screen (`pressStartEnabled`)
//! until a key starts the shell. What the engine tells it is what the
//! movie's own test path (taken when its `testingVAR` is set) tells it:
//! `setPlatform`, `firstTimeLoad(true)`, `setRegion("US")` and
//! `setTerritory("EN")`, with the platform the PC's (`PC`, which its text
//! tables tell apart); then, once press-start is up, Enter calls
//! `startShell` (see [`Native`]).
//!
//! `LTILibName` is the PC version's own library behind its options
//! screens (video modes, key bindings, the profile): the engine's, not the
//! scripts'. Its calls are queued for the engine ([`Native::library`]),
//! which answers the video screen's (below) and logs the rest; `FirstRun`
//! says this profile has run before (no wait before the legal screen can
//! be skipped).
//!
//! The Options' video screen works as the pause menu's
//! ([`crate::pause`]): its arrows ask the library to step Resolution and
//! Refresh Rate (`LTIVideoNextRes`, `LTIVideoPrevRefresh`...), the movie
//! shows the text handed back (`LTIVideoSetRes`, `LTIVideoSetRefresh`:
//! this movie's names, not the pause movie's), and Accept keeps them
//! ([`VIDEO_ACCEPT`]). Until playtest 7 nothing answered, so both items
//! stayed blank and neither fullscreen nor a size could be picked there.
//!
//! # Provenance
//!
//! Read from the game's shell scripts (`MrxGuiShellBootstrap`,
//! `MrxGuiShell`) in a legally installed copy. Nothing from the
//! executable.

use m2_game::gui_runtime;

use crate::input::Control;
use crate::pause::{self, KeyScreen, Mode, News, Settings, Video};
use m2_assets::flash::avm1::Value;
use m2_game::mission::Mission;
use mlua::Lua;

/// Sets the shell's world up and opens it.
const OPEN: &str = r#"
local me = Sys.StringToGuid("0x1")
for _, f in ipairs({ "GetLocalPlayer", "GetPrimaryPlayer" }) do
    rawset(Player, f, function() return me end)
end
rawset(Player, "GetAllPlayers", function() return { me } end)
__m2_lti = {}
LTILibName = setmetatable({
    FirstRun = function() return false end,
}, {
    __index = function(_, k)
        return function(a)
            local q = __m2_lti
            q[#q + 1] = { tostring(k), a == nil and "" or tostring(a) }
        end
    end,
})
rawset(Sys, "StartSingleplayer", function()
    local who = MrxGuiShellBootstrap.GetSelectedCharacter
        and MrxGuiShellBootstrap.GetSelectedCharacter()
    __m2_new_game = tostring(who or "")
end)
if rawget(Sys, "GetMasterScriptName") == nil then
    rawset(Sys, "GetMasterScriptName", function() return "" end)
end
import("MrxGuiShellBootstrap")
MrxGuiShellBootstrap.Init()
MrxGuiShellBootstrap.EnterShell()
"#;

/// Opens the shell in `mission`'s Lua state (a mission made from
/// `shell.wad`, not started).
pub fn open(mission: &mut Mission) -> Result<(), String> {
    mission.run(OPEN, "m2 shell").map_err(|e| e.to_string())?;
    // The layouts load: their callbacks run (`ShellScreenLoaded`, which
    // asks for the `Shell` game state: see [`game_states`]).
    gui_runtime::tick(mission.lua(), 0.0);
    gui_runtime::tick(mission.lua(), 0.0);
    Ok(())
}

/// The game states the scripts asked for (`Sys.RequestGameState`) among
/// `commands`, entered as the engine enters them: the GUI is told the
/// state left (`GuiGameStateChange` with `Exit`) and the one entered
/// (`Enter`). `state` is the state the shell is in. Returns lines for the
/// log.
pub fn game_states(
    lua: &Lua,
    commands: &[m2_game::api::Command],
    state: &mut Option<String>,
) -> Vec<String> {
    let mut log = Vec::new();
    for c in commands {
        let m2_game::api::Command::GameState(next) = c else {
            continue;
        };
        let mut tell = |name: &str, phase: &str| match gui_runtime::fire(
            lua,
            "GuiGameStateChange",
            (name.to_owned(), phase),
        ) {
            Ok(n) => log.push(format!("shell: game state {name} {phase} ({n} handlers)")),
            Err(e) => log.push(format!("shell: game state {name} {phase}: {e}")),
        };
        if let Some(old) = state.take() {
            tell(&old, "Exit");
        }
        tell(next, "Enter");
        *state = Some(next.clone());
    }
    log
}

/// The character a new game was started as since the last call
/// (`Chris`, `Jen` or `mattias`), if one was.
pub fn take_new_game(lua: &Lua) -> Option<String> {
    let g = lua.globals();
    let who = g
        .raw_get::<Option<String>>("__m2_new_game")
        .ok()
        .flatten()?;
    let _ = g.raw_set("__m2_new_game", mlua::Value::Nil);
    Some(who)
}

/// The hero the shell's character name stands for (`Chris`, `Jen`,
/// `mattias`, any case), if it names one; one that does not is logged.
pub fn hero(character: &str) -> Option<crate::hero::Hero> {
    let hero = crate::hero::Hero::parse(character);
    if hero.is_none() {
        eprintln!("front end: the character {character:?} names no hero");
    }
    hero
}

/// The engine's side of the menu movie (`shell.gfx`): what it calls in
/// the movie, and when.
#[derive(Debug, Default)]
pub struct Native {
    /// The movie has been told the platform.
    set_up: bool,
    /// The press-start screen is up (`pressStartEnabled`), waiting for a
    /// key.
    press_start: bool,
    /// Calls to make at the next frame.
    pending: Vec<Call>,
    /// The save operation waiting on the profile's save.
    saving: Option<String>,
    /// The profile's character, as the movie last set it
    /// (`onlineSetParameters` with `profileCharacter`): see
    /// [`Native::character`].
    character: Option<&'static str>,
    /// Play's settings, as last given or kept.
    settings: Settings,
    /// The settings as the option screens have them, kept on their
    /// Accept: [`Native::keep`].
    editing: Settings,
    /// The video screen's Resolution and Refresh Rate as stepped, while
    /// it is open.
    video: Option<Video>,
    /// The keyboard and mouse table (Controls, Keyboard/Mouse), as the
    /// pause menu's ([`KeyScreen`]).
    table: KeyScreen,
    /// What the menus asked of the game since last taken.
    news: News,
}

/// The menu movie's name.
pub const MOVIE: &str = "shell";

/// The engine's calls into the movie: the function's name and its
/// arguments.
pub type Call = (&'static str, Vec<m2_assets::flash::avm1::Value>);

/// `onlineLTIIsConnected`'s pages that go on to the game: Continue's (1),
/// New Game's (2) and the load screen's (4), as `LTIgotoGame` reads
/// `ProfilePage`. Join Game's (3) is online, which this has none of.
const OFFLINE_PAGES: [&str; 3] = ["1", "2", "4"];

/// The `saveCheck` operations that save the profile and then wait on the
/// engine (`saveCompleteContinue`); the others go on at
/// `preopComplete` (`loadgame`, the load screen) or are online.
const PROFILE_SAVES: [&str; 6] = [
    "autoContinue",
    "autoSave",
    "continue",
    "newGame",
    "loadSaveGame",
    "deleteSaveGame",
];

impl Native {
    /// What to call in the movie now, `settings` being play's: set up as
    /// it starts (once), then the answers to what it asked.
    pub fn calls(&mut self, settings: &Settings) -> Vec<Call> {
        let mut calls = Vec::new();
        if !std::mem::replace(&mut self.set_up, true) {
            self.settings = settings.clone();
            self.editing = settings.clone();
            self.table = KeyScreen::new(settings.keys.clone());
            calls.extend([
                ("setPlatform", vec![text("PC")]),
                ("firstTimeLoad", vec![Value::Bool(true)]),
                ("setRegion", vec![text("US")]),
                ("setTerritory", vec![text("EN")]),
                // No online service: Join Game and the profiles' online
                // pages greyed, as `updateMainMenu` greys them.
                ("disableOnline", vec![Value::Bool(true)]),
            ]);
            calls.extend(self.setting_calls());
            calls.extend(self.save_calls());
            calls.extend(self.table.fill());
        }
        calls.append(&mut self.pending);
        calls
    }

    /// What the menus asked of the game since last asked.
    pub fn take_news(&mut self) -> News {
        std::mem::take(&mut self.news)
    }

    /// An `fscommand` no script handles: whether it is the engine's.
    ///
    /// `onlineLTIIsConnected` asks the PC's library whether the player
    /// may go on to the page asked for. Offline, the answer is
    /// `LTIgotoGame`, which goes on by the page: Continue's checks the
    /// saves (`saveCheck("load", "continue")`), New Game's goes on to the
    /// characters (`newGameSelect`; its trace says "new game - success"),
    /// the load screen's loads the save picked (`"loadSaveGame"`). Join
    /// Game's (online) is not answered.
    pub fn command(&mut self, command: &str, arg: &str) -> bool {
        match command {
            "pressStartEnabled" => {
                self.press_start = arg != "false";
                true
            }
            "onlineLTIIsConnected" => {
                if OFFLINE_PAGES.contains(&arg.trim()) {
                    self.pending.push(("LTIgotoGame", Vec::new()));
                }
                true
            }
            // `saveCheck(perform, what)` asks the engine to get the saves
            // ready (`preoperation` with "perform,what") and waits for
            // `preopComplete(true, what)`, which for most saves the
            // profile (`onlineSaveProfile`) and waits again, for
            // `saveCompleteContinue(what)`: what the movie's own test path
            // calls straight away at each step. Loading, the saves are
            // listed first (`clearSaveGames`, `addSaveGame`), as the pause
            // menu's are.
            "preoperation" => {
                let mut parts = arg.split(',');
                let perform = parts.next().unwrap_or("").trim();
                let what = parts.next().unwrap_or("").trim().to_owned();
                if perform == "load" {
                    self.pending.extend(self.save_calls());
                }
                self.pending
                    .push(("preopComplete", vec![Value::Bool(true), text(&what)]));
                if PROFILE_SAVES.contains(&what.as_str()) {
                    self.saving = Some(what);
                }
                true
            }
            "onlineSaveProfile" => {
                if let Some(what) = self.saving.take() {
                    self.pending
                        .push(("saveCompleteContinue", vec![text(&what)]));
                }
                true
            }
            // The movie's settings for the profile (`outputSettings`, one
            // "name, value" each), saved by the engine: the character is
            // the one accepted on the character screen; the rest are the
            // option screens', kept as they are accepted.
            "onlineSetParameters" => {
                if let Some(("profileCharacter", n)) = arg.split_once(", ") {
                    self.character = profile_character(n.trim()).or(self.character);
                }
                true
            }
            // Continue's `_continueStart` frame asks for the newest save
            // (-1); the load screen's `_loadgameStart`, the one picked.
            "LoadSaveGame" => match arg.trim().parse::<i64>() {
                Ok(-1) => {
                    self.news.continue_game = true;
                    true
                }
                Ok(n) => {
                    self.news.load = u32::try_from(n).ok();
                    self.news.load.is_some()
                }
                Err(_) => false,
            },
            // The load screen's Delete, confirmed (`deleteSaveGame`): the
            // movie drops it from its list itself.
            "DeleteSaveGame" => {
                let Ok(n) = arg.trim().parse::<u32>() else {
                    return false;
                };
                self.settings.saves.retain(|s| s.number != n);
                self.news.delete = Some(n);
                true
            }
            // The first run's brightness screen: the level as it moves.
            "videoBrightness" => {
                let Ok(v) = arg.trim().parse::<f64>() else {
                    return false;
                };
                self.editing.brightness = v.clamp(0.0, 100.0);
                self.settings.brightness = self.editing.brightness;
                self.news.settings = Some(self.settings.clone());
                true
            }
            "exitGame" => {
                self.news.quit = true;
                true
            }
            "Sound.Play" | "Sound.Loop" => {
                self.news.sounds.push(arg.to_owned());
                true
            }
            "Sound.Stop" => {
                self.news.stops.push(arg.to_owned());
                true
            }
            _ => false,
        }
    }

    /// The character the profile was given (`chris`, `jennifer` or
    /// `mattias`, as the shell's scripts name them), once the movie has
    /// set it.
    ///
    /// The movie starts the game with `fscommand("newGame", 1)` from its
    /// `_newgameStart` frame: the argument is a constant, not the
    /// character. On the PC the character is the profile's
    /// (`profileCharacter`), which the engine hands the scripts as
    /// `sCharacter` for their `LTIStartNewGame` handler
    /// (`MrxGuiShell._LTIStartNewGame`, which reads that global, not its
    /// argument).
    pub fn character(&self) -> Option<&'static str> {
        self.character
    }

    /// A key (Flash's code) going down: what to call for it.
    pub fn key(&mut self, code: i32) -> Vec<Call> {
        if self.press_start && code == 13 {
            self.press_start = false;
            return vec![("startShell", Vec::new())];
        }
        Vec::new()
    }

    /// The key or mouse button pressed while the keyboard table waited
    /// for one.
    pub fn captured(&mut self, control: Control) {
        self.table
            .captured(control, &mut self.pending, &mut self.news);
    }

    /// A call to the PC's library (`LTILibName.<name>(arg)`) from the
    /// scripts' handlers (`MrxGuiShell._LTIFscommand`,
    /// `_LTIPauseItemChanged`...): whether it is answered. `window` is how
    /// the window is shown now and `modes` the monitor's.
    pub fn library(&mut self, name: &str, arg: &str, window: Video, modes: &[Mode]) -> bool {
        if let Some(answered) = self
            .table
            .library(name, arg, &mut self.pending, &mut self.news)
        {
            if name == "LTIInputKMApplyChanges" {
                self.settings.keys = self.table.kept.clone();
                self.editing.keys = self.table.kept.clone();
            }
            return answered;
        }
        match name {
            "LTIPauseItemChanged" => self.item_changed(arg),
            // The video screen opened: its items read as the window is,
            // and as the settings are.
            "LTIVideoEnter" => {
                self.video = Some(window);
                self.editing = self.settings.clone();
                self.pending.extend(self.video_calls(window));
                true
            }
            // Its arrows: the engine steps the item and says what it reads
            // now (as the pause menu's, [`crate::pause::step_resolution`]).
            "LTIVideoNextRes"
            | "LTIVideoPrevRes"
            | "LTIVideoNextRefresh"
            | "LTIVideoPrevRefresh" => {
                let step = if name.contains("Next") { 1 } else { -1 };
                let at = self.video.unwrap_or(window);
                let video = if name.ends_with("Res") {
                    pause::step_resolution(at, modes, (window.width, window.height), step)
                } else {
                    pause::step_rate(at, modes, step)
                };
                self.video = Some(video);
                self.pending
                    .extend(self.video_calls(video).into_iter().take(2));
                true
            }
            // The brightness slider, as it moves.
            "LTIVideoSetGamma" => match arg.trim().parse::<f64>() {
                Ok(v) => {
                    self.editing.brightness = v.clamp(0.0, 100.0);
                    true
                }
                Err(_) => false,
            },
            // Its Aspect Ratio switched between wide and 4:3: kept with
            // the rest on Accept.
            "LTIVideoSwitchMode" => {
                self.editing.wide = !self.editing.wide;
                true
            }
            // Accept: the Resolution and Refresh Rate stepped, and the
            // screen's other items as reported.
            n if n == VIDEO_ACCEPT => {
                let video = self.video.take();
                self.news.video = video.filter(|&v| v != window);
                if let Some(v) = video {
                    self.editing.video = v;
                }
                self.keep();
                true
            }
            "LTIVideoCancel" => {
                self.video = None;
                self.editing = self.settings.clone();
                true
            }
            // Defaults: the game's own, as `setVideoDefaults` asks.
            "LTIVideoDefault" => {
                let d = Settings::default();
                self.editing.brightness = d.brightness;
                self.editing.view_distance = d.view_distance;
                self.editing.vsync = d.vsync;
                self.pending.extend(self.video_calls(window));
                true
            }
            "LTIVideoAdvanceEnter" => {
                self.pending.extend(self.advanced_calls());
                true
            }
            // The advanced video screen's Accept (`AdvOptApply`): each
            // option as tens and its value (10 + the first, 20 + the
            // second...: `Number(advvideooption1) + 10`).
            "LTIVideoSwitchOpt1" => match arg.trim().parse::<f64>() {
                Ok(v) if (10.0..80.0).contains(&v) => {
                    let i = (v / 10.0).floor() as usize - 1;
                    let i = if i == 6 { 7 } else { i };
                    self.editing.advanced[i] = v - 10.0 * (v / 10.0).floor();
                    self.settings.advanced[i] = self.editing.advanced[i];
                    self.news.settings = Some(self.settings.clone());
                    true
                }
                _ => false,
            },
            "LTIInputGeneralEnter" => {
                self.editing = self.settings.clone();
                self.pending.extend(self.controls_calls());
                true
            }
            // The controls screen's Accept (`ControlsApply`): each item as
            // "n&value": 1 mouse sensitivity, 2 invert mouse (1 or 0),
            // 3 joystick sensitivity, 4 vibration, 5 invert joystick.
            "LTIInputGeneralOptions" => {
                let Some((n, v)) = arg.split_once('&') else {
                    return false;
                };
                let (v, on) = (v.trim(), matches!(v.trim(), "true" | "1"));
                let level = v.parse::<f64>().ok().map(|l| l.clamp(0.0, 100.0));
                let s = &mut self.settings;
                match (n.trim(), level) {
                    ("1", Some(l)) => s.mouse = l,
                    ("2", _) => s.invert_mouse = on,
                    ("3", Some(l)) => s.pad = l,
                    ("4", _) => s.rumble = on,
                    ("5", _) => s.invert_pad = on,
                    _ => return false,
                }
                self.editing = self.settings.clone();
                self.news.settings = Some(self.settings.clone());
                true
            }
            _ => false,
        }
    }

    /// `LTIPauseItemChanged(arg)`: the option screens' changes as they
    /// are made (`name=value`), and their Accept, Cancel and Defaults.
    fn item_changed(&mut self, arg: &str) -> bool {
        match arg {
            // Each screen opening: its items as the settings are.
            "gameEnter" | "audioEnter" => {
                self.editing = self.settings.clone();
                self.pending.extend(if arg == "gameEnter" {
                    self.game_calls()
                } else {
                    self.audio_calls()
                });
                true
            }
            "gameAccept" | "gameApply" | "audioApply" | "AdvAccept" => {
                self.keep();
                true
            }
            "gameCancel" => {
                self.editing = self.settings.clone();
                true
            }
            "audioCancel" => {
                // Back as they were, sliders heard included.
                self.editing = self.settings.clone();
                let s = &self.settings;
                self.news.volumes = Some([s.sfx, s.music, s.dialog]);
                true
            }
            "gameDefaults" | "audioDefaults" => {
                let d = Settings::default();
                let e = &mut self.editing;
                if arg == "gameDefaults" {
                    (e.subtitles, e.tutorials, e.autosave) = (d.subtitles, d.tutorials, d.autosave);
                    self.pending.extend(self.game_calls());
                } else {
                    (e.sfx, e.music, e.dialog) = (d.sfx, d.music, d.dialog);
                    self.news.volumes = Some([d.sfx, d.music, d.dialog]);
                    self.pending.extend(self.audio_calls());
                }
                true
            }
            "VSyncOn" | "VSyncOff" => {
                self.editing.vsync = arg == "VSyncOn";
                true
            }
            "serverEnter" | "videoPrevAntiAliasLevel" | "videoNextAntiAliasLevel" => true,
            _ => match arg.split_once('=') {
                // Its view distance is 0 to 90 (`videoViewDistance = val
                // - 10` in its setter): the game's 10 to 100.
                Some(("videoViewDistance", v)) => match v.trim().parse::<f64>() {
                    Ok(v) => {
                        self.editing.view_distance = (v + 10.0).clamp(10.0, 100.0);
                        true
                    }
                    Err(_) => false,
                },
                Some(("videoVideoDetailVar", v)) => match v.trim().parse::<f64>() {
                    Ok(v) => {
                        self.editing.detail = v;
                        true
                    }
                    Err(_) => false,
                },
                Some(("videoShaderSetting", v)) => {
                    self.editing.advanced[6] = f64::from(u8::from(v.trim() == "true"));
                    true
                }
                Some((name, value)) => {
                    let volume = name.starts_with("audio");
                    let known = self.editing.set(name, value);
                    // The sliders say their level as they move, for the
                    // player to hear.
                    if known && volume {
                        let e = &self.editing;
                        self.news.volumes = Some([e.sfx, e.music, e.dialog]);
                    }
                    if !known {
                        self.news.unused.push(arg.to_owned());
                    }
                    true
                }
                None => false,
            },
        }
    }

    /// The option screens' settings as they have them now become play's.
    fn keep(&mut self) {
        let (keys, saves) = (self.settings.keys.clone(), self.settings.saves.clone());
        self.settings = Settings {
            keys,
            saves,
            ..self.editing.clone()
        };
        self.news.settings = Some(self.settings.clone());
    }

    /// The movie's setters, with the settings: each screen's (the video
    /// screens' Graphic Detail and advanced options come as they open: the
    /// movie defines their setters on those frames).
    fn setting_calls(&self) -> Vec<Call> {
        let mut calls = self.game_calls();
        calls.extend(self.audio_calls());
        calls.extend(self.controls_calls());
        let s = &self.settings;
        calls.extend([
            (
                "LTIVideoSetGamma",
                vec![Value::Number(s.brightness.round())],
            ),
            (
                "LTIVideoSetVideoViewDistance",
                vec![Value::Number(s.view_distance.round())],
            ),
            (
                "LTIVideoSetVSync",
                vec![Value::Number(f64::from(u8::from(s.vsync)))],
            ),
        ]);
        calls
    }

    /// The Game screen's: Auto Save, Tutorials, Subtitles.
    fn game_calls(&self) -> Vec<Call> {
        let s = &self.editing;
        vec![
            ("gameAutosave", vec![Value::Bool(s.autosave)]),
            ("gameTutorials", vec![Value::Bool(s.tutorials)]),
            ("videoSubtitles", vec![Value::Bool(s.subtitles)]),
        ]
    }

    /// The Audio screen's sliders, 0 to 100.
    fn audio_calls(&self) -> Vec<Call> {
        let s = &self.editing;
        let n = |v: f64| vec![Value::Number(v.round())];
        vec![
            ("LTIAudioSFXVolume", n(s.sfx)),
            ("LTIAudioMusicVolume", n(s.music)),
            ("LTIAudioDialogVolume", n(s.dialog)),
        ]
    }

    /// The Controls screen's: invert and sensitivity for the mouse and the
    /// pad, and vibration (and the profile's own copies, which
    /// `outputSettings` reports).
    fn controls_calls(&self) -> Vec<Call> {
        let s = &self.settings;
        let n = |v: f64| vec![Value::Number(v.round())];
        vec![
            ("LTIInputSetInvertMouse", vec![Value::Bool(s.invert_mouse)]),
            ("LTIInputSetMouseSense", n(s.mouse)),
            ("LTIInputSetInvertJoystick", vec![Value::Bool(s.invert_pad)]),
            ("LTIInputSetJoySense", n(s.pad)),
            ("LTIInputSetRumble", vec![Value::Bool(s.rumble)]),
            ("gameSensitivity", n(s.pad)),
            ("gameRumble", vec![Value::Bool(s.rumble)]),
        ]
    }

    /// The advanced video screen's eight options (`LTIVideoSetSwitchOpt1`
    /// to `8`: on and off, or a detail level).
    fn advanced_calls(&self) -> Vec<Call> {
        const SETTERS: [&str; 8] = [
            "LTIVideoSetSwitchOpt1",
            "LTIVideoSetSwitchOpt2",
            "LTIVideoSetSwitchOpt3",
            "LTIVideoSetSwitchOpt4",
            "LTIVideoSetSwitchOpt5",
            "LTIVideoSetSwitchOpt6",
            "LTIVideoSetSwitchOpt7",
            "LTIVideoSetSwitchOpt8",
        ];
        // The on-and-off ones (the first, shadows, shaders, motion blur)
        // as the movie's own booleans.
        SETTERS
            .iter()
            .zip(self.settings.advanced)
            .enumerate()
            .map(|(i, (&name, v))| {
                let value = if matches!(i, 0 | 3 | 6 | 7) {
                    Value::Bool(v != 0.0)
                } else {
                    Value::Number(v)
                };
                (name, vec![value])
            })
            .collect()
    }

    /// The video screen's items: Resolution, Refresh Rate (`video`: this
    /// movie's `LTIVideoSetRes`, `LTIVideoSetRefresh`, the texts as the
    /// pause menu's, [`Video::resolution_text`]), then brightness, view
    /// distance and vsync as being edited.
    fn video_calls(&self, video: Video) -> Vec<Call> {
        let s = &self.editing;
        vec![
            ("LTIVideoSetRes", vec![text(&video.resolution_text())]),
            ("LTIVideoSetRefresh", vec![text(&video.refresh_text())]),
            // 1 is wide (`videoWidescreen = Number(val) == 1`).
            (
                "LTIVideoSetMode",
                vec![Value::Number(f64::from(u8::from(s.wide)))],
            ),
            (
                "LTIVideoSetGamma",
                vec![Value::Number(s.brightness.round())],
            ),
            (
                "LTIVideoSetVideoViewDistance",
                vec![Value::Number(s.view_distance.round())],
            ),
            (
                "LTIVideoSetVSync",
                vec![Value::Number(f64::from(u8::from(s.vsync)))],
            ),
            ("LTISetGraphicDetail", vec![Value::Number(s.detail)]),
        ]
    }

    /// The saves, into the movie's list (`clearSaveGames`, then
    /// `addSaveGame` each, as the pause menu's): Continue and Load Game
    /// are offered once it has one.
    fn save_calls(&self) -> Vec<Call> {
        let mut calls = vec![("clearSaveGames", Vec::new())];
        calls.extend(
            self.settings
                .saves
                .iter()
                .map(|s| ("addSaveGame", pause::save_args(s))),
        );
        calls
    }
}

/// The library call the video screen's Accept makes (seen in the log of a
/// front-end run: [`Native::library`] logs each).
pub const VIDEO_ACCEPT: &str = "LTIVideoApplyChanges";

fn text(s: &str) -> Value {
    Value::Str(s.into())
}

/// The scripts' name for the movie's `profileCharacterVar`: 1, 2 and 3 are
/// Mattias, Chris and Jennifer (its character buttons' order, which
/// `newGameTransition` and the saves' pages read the same way).
fn profile_character(n: &str) -> Option<&'static str> {
    match n {
        "1" => Some("mattias"),
        "2" => Some("chris"),
        "3" => Some("jennifer"),
        _ => None,
    }
}

/// Starts the new game the menu movie asked for (`fscommand("newGame", _)`
/// from widget `widget`) as the PC does, as `character` (see
/// [`Native::character`]): the scripts' `sCharacter` is set and their
/// `LTIStartNewGame` handler runs if the widget has one; otherwise their
/// `newGame` handler (`_NewGameFlashCallback`, which takes the character
/// as its argument) is given it. Returns whether a handler ran.
pub fn start_new_game(lua: &Lua, widget: u32, character: &str) -> mlua::Result<bool> {
    lua.globals().set("sCharacter", character)?;
    if gui_runtime::flash_command(lua, widget, "LTIStartNewGame", character)? {
        return Ok(true);
    }
    gui_runtime::flash_command(lua, widget, "newGame", character)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pause::SaveEntry;

    fn names(calls: Vec<Call>) -> Vec<&'static str> {
        calls.into_iter().map(|c| c.0).collect()
    }

    /// Each call as its name and its arguments' texts.
    fn shown(calls: Vec<Call>) -> Vec<String> {
        calls
            .into_iter()
            .map(|(n, a)| {
                let args: Vec<String> = a
                    .iter()
                    .map(|v| match v {
                        Value::Str(s) => s.to_string(),
                        Value::Number(x) => x.to_string(),
                        Value::Bool(b) => b.to_string(),
                        _ => "?".into(),
                    })
                    .collect();
                if args.is_empty() {
                    n.to_owned()
                } else {
                    format!("{n} {}", args.join(" "))
                }
            })
            .collect()
    }

    fn save(number: u32, character: u32) -> SaveEntry {
        SaveEntry {
            number,
            timestamp: "12:00:00 | 10/19/2026".into(),
            character,
            money: 1000.0,
            fuel: 50.0,
            played: String::new(),
            last_contract: String::new(),
            auto: number == 0,
            percent: 0.0,
        }
    }

    const NO_MODES: [Mode; 0] = [];

    #[test]
    fn the_movie_is_set_up_once_and_enter_starts_the_shell_from_press_start() {
        let mut n = Native::default();
        let s = Settings::default();
        let first = names(n.calls(&s));
        assert_eq!(
            first[..5],
            [
                "setPlatform",
                "firstTimeLoad",
                "setRegion",
                "setTerritory",
                "disableOnline"
            ]
        );
        // Its settings, its saves and its keyboard table follow.
        assert!(first.contains(&"gameAutosave") && first.contains(&"LTIAudioSFXVolume"));
        assert!(first.contains(&"clearSaveGames"));
        assert_eq!(
            first.iter().filter(|&&c| c == "LTIInputKMKeyMap").count(),
            48
        );
        assert!(n.calls(&s).is_empty());
        assert!(n.key(13).is_empty());
        assert!(n.command("pressStartEnabled", "true"));
        assert!(n.key(32).is_empty());
        assert_eq!(names(n.key(13)), ["startShell"]);
        assert!(n.key(13).is_empty());
        assert!(!n.command("startMovie", "x"));
    }

    #[test]
    fn the_video_screen_steps_the_monitors_modes_and_accept_keeps_them() {
        let modes = [
            Mode {
                width: 1920,
                height: 1080,
                hz: 60,
            },
            Mode {
                width: 2560,
                height: 1440,
                hz: 144,
            },
            Mode {
                width: 2560,
                height: 1440,
                hz: 60,
            },
        ];
        // Playtest 7: a maximised window, not one of the modes.
        let window = Video {
            width: 2560,
            height: 1351,
            fullscreen: None,
        };
        let s = Settings::default();
        let mut n = Native::default();
        n.calls(&s);
        assert!(n.library("LTIVideoEnter", "", window, &modes));
        assert_eq!(
            shown(n.calls(&s))[..2],
            ["LTIVideoSetRes 2560 x 1351", "LTIVideoSetRefresh Windowed"]
        );
        assert!(n.library("LTIVideoNextRes", "", window, &modes));
        assert_eq!(
            shown(n.calls(&s)),
            ["LTIVideoSetRes 2560 x 1440", "LTIVideoSetRefresh Windowed"]
        );
        // The rate's first entry is the fastest: fullscreen at it.
        assert!(n.library("LTIVideoPrevRefresh", "", window, &modes));
        assert_eq!(shown(n.calls(&s))[1], "LTIVideoSetRefresh 60 Hz");
        assert!(n.library("LTIVideoNextRefresh", "", window, &modes));
        assert_eq!(shown(n.calls(&s))[1], "LTIVideoSetRefresh Windowed");
        assert!(n.library("LTIVideoNextRefresh", "", window, &modes));
        assert_eq!(shown(n.calls(&s))[1], "LTIVideoSetRefresh 144 Hz");
        assert_eq!(n.take_news().video, None, "not kept till accepted");
        assert!(n.library(VIDEO_ACCEPT, "", window, &modes));
        let news = n.take_news();
        assert_eq!(
            news.video,
            Some(Video {
                width: 2560,
                height: 1440,
                fullscreen: Some(144),
            })
        );
        assert!(news.settings.is_some());
        // Cancelled: nothing kept, and the next visit starts from the
        // window again.
        assert!(n.library("LTIVideoEnter", "", window, &modes));
        assert!(n.library("LTIVideoPrevRes", "", window, &modes));
        assert!(n.library("LTIVideoCancel", "", window, &modes));
        assert!(n.library(VIDEO_ACCEPT, "", window, &modes));
        assert_eq!(n.take_news().video, None);
        assert!(!n.library("LTIGetStartButton", "", window, &modes));
    }

    #[test]
    fn a_new_game_goes_on_to_the_characters() {
        let mut n = Native::default();
        let s = Settings::default();
        n.calls(&s);
        // Join Game (online): no answer.
        assert!(n.command("onlineLTIIsConnected", "3"));
        assert!(n.calls(&s).is_empty());
        assert!(n.command("onlineLTIIsConnected", "2"));
        assert_eq!(names(n.calls(&s)), ["LTIgotoGame"]);
        assert!(n.command("preoperation", "save,newGame"));
        assert_eq!(names(n.calls(&s)), ["preopComplete"]);
        assert!(n.command("onlineSaveProfile", "true"));
        assert_eq!(names(n.calls(&s)), ["saveCompleteContinue"]);
        assert!(n.command("onlineSaveProfile", "true"));
        assert!(n.calls(&s).is_empty());
    }

    #[test]
    fn continue_lists_the_saves_and_asks_for_the_newest() {
        let mut n = Native::default();
        let s = Settings {
            saves: vec![save(0, 3), save(2, 1)],
            ..Settings::default()
        };
        let first = shown(n.calls(&s));
        let listed: Vec<&String> = first
            .iter()
            .filter(|c| c.starts_with("addSaveGame"))
            .collect();
        assert_eq!(listed.len(), 2);
        // number, time, character (Jennifer is 3), money, oil...
        assert!(listed[0].starts_with("addSaveGame 0 12:00:00 | 10/19/2026 3 1000 50"));
        // Continue: page 1, then the saves' checks as the movie runs them.
        assert!(n.command("onlineLTIIsConnected", "1"));
        assert_eq!(names(n.calls(&s)), ["LTIgotoGame"]);
        assert!(n.command("preoperation", "load,continue"));
        assert_eq!(
            names(n.calls(&s)),
            [
                "clearSaveGames",
                "addSaveGame",
                "addSaveGame",
                "preopComplete"
            ]
        );
        assert!(n.command("onlineSaveProfile", "true"));
        assert_eq!(shown(n.calls(&s)), ["saveCompleteContinue continue"]);
        // Its `_continueStart` frame.
        assert!(n.command("LoadSaveGame", "-1"));
        let news = n.take_news();
        assert!(news.continue_game);
        assert_eq!(news.load, None);
    }

    #[test]
    fn the_load_screen_loads_and_deletes_by_slot() {
        let mut n = Native::default();
        let s = Settings {
            saves: vec![save(1, 1), save(4, 2)],
            ..Settings::default()
        };
        n.calls(&s);
        // Options' Load Game: listed, then the screen.
        assert!(n.command("preoperation", "load,loadgame"));
        assert_eq!(names(n.calls(&s)).last(), Some(&"preopComplete"));
        assert!(n.command("DeleteSaveGame", "1"));
        assert_eq!(n.take_news().delete, Some(1));
        // Listed again, the deleted one is gone.
        assert!(n.command("preoperation", "load,"));
        assert_eq!(
            names(n.calls(&s)),
            ["clearSaveGames", "addSaveGame", "preopComplete"]
        );
        assert!(n.command("onlineLTIIsConnected", "4"));
        assert_eq!(names(n.calls(&s)), ["LTIgotoGame"]);
        assert!(n.command("preoperation", "load,loadSaveGame"));
        n.calls(&s);
        assert!(n.command("onlineSaveProfile", "true"));
        assert_eq!(shown(n.calls(&s)), ["saveCompleteContinue loadSaveGame"]);
        assert!(n.command("LoadSaveGame", "4"));
        let news = n.take_news();
        assert_eq!((news.load, news.continue_game), (Some(4), false));
        assert!(!n.command("LoadSaveGame", "x"));
    }

    #[test]
    fn the_option_screens_keep_on_accept_and_drop_on_cancel() {
        let mut n = Native::default();
        let s = Settings::default();
        n.calls(&s);
        let window = Video::default();
        // Game: changed, then accepted.
        assert!(n.library("LTIPauseItemChanged", "gameEnter", window, &NO_MODES));
        assert_eq!(
            names(n.calls(&s)),
            ["gameAutosave", "gameTutorials", "videoSubtitles"]
        );
        assert!(n.library(
            "LTIPauseItemChanged",
            "gameSubtitleVar=false",
            window,
            &NO_MODES
        ));
        assert!(n.library(
            "LTIPauseItemChanged",
            "gameAutoSaveVar=false",
            window,
            &NO_MODES
        ));
        assert!(n.take_news().settings.is_none(), "not kept till accepted");
        assert!(n.library("LTIPauseItemChanged", "gameAccept", window, &NO_MODES));
        let kept = n.take_news().settings.unwrap();
        assert_eq!(
            (kept.subtitles, kept.autosave, kept.tutorials),
            (false, false, true)
        );
        // Audio: heard as it moves, dropped on cancel.
        assert!(n.library("LTIPauseItemChanged", "audioEnter", window, &NO_MODES));
        n.calls(&s);
        assert!(n.library(
            "LTIPauseItemChanged",
            "audioMusicVolumeVar=20",
            window,
            &NO_MODES
        ));
        assert_eq!(n.take_news().volumes, Some([100.0, 20.0, 100.0]));
        assert!(n.library("LTIPauseItemChanged", "audioCancel", window, &NO_MODES));
        let news = n.take_news();
        assert_eq!(
            (news.volumes, news.settings),
            (Some([100.0, 100.0, 100.0]), None)
        );
        // Video: its view distance is the game's less 10; vsync by name.
        assert!(n.library("LTIVideoEnter", "", window, &NO_MODES));
        n.calls(&s);
        assert!(n.library(
            "LTIPauseItemChanged",
            "videoViewDistance=30",
            window,
            &NO_MODES
        ));
        assert!(n.library("LTIPauseItemChanged", "VSyncOff", window, &NO_MODES));
        assert!(n.library("LTIVideoSetGamma", "70", window, &NO_MODES));
        assert!(n.library(VIDEO_ACCEPT, "", window, &NO_MODES));
        let kept = n.take_news().settings.unwrap();
        assert_eq!(
            (kept.view_distance, kept.vsync, kept.brightness),
            (40.0, false, 70.0)
        );
        // What was kept before stays kept.
        assert!(!kept.subtitles);
        // Shown again as kept the next time the screen opens.
        assert!(n.library("LTIVideoEnter", "", window, &NO_MODES));
        assert_eq!(
            shown(n.calls(&s))[2..],
            [
                "LTIVideoSetMode 1",
                "LTIVideoSetGamma 70",
                "LTIVideoSetVideoViewDistance 40",
                "LTIVideoSetVSync 0",
                "LTISetGraphicDetail 2"
            ]
        );
    }

    #[test]
    fn the_controls_and_advanced_screens_report_as_they_accept() {
        let mut n = Native::default();
        let s = Settings::default();
        n.calls(&s);
        let w = Video::default();
        // `ControlsApply`.
        for item in ["1&80", "2&1", "3&25", "4&false", "5&true"] {
            assert!(n.library("LTIInputGeneralOptions", item, w, &NO_MODES));
        }
        let kept = n.take_news().settings.unwrap();
        assert_eq!(
            (
                kept.mouse,
                kept.invert_mouse,
                kept.pad,
                kept.rumble,
                kept.invert_pad
            ),
            (80.0, true, 25.0, false, true)
        );
        assert!(!n.library("LTIInputGeneralOptions", "9&1", w, &NO_MODES));
        // `AdvOptApply`: tens and the value; 70 is motion blur, the
        // eighth setter's.
        for v in ["11", "22", "30", "40", "52", "61", "70"] {
            assert!(n.library("LTIVideoSwitchOpt1", v, w, &NO_MODES));
        }
        assert!(n.library(
            "LTIPauseItemChanged",
            "videoShaderSetting=false",
            w,
            &NO_MODES
        ));
        assert!(n.library("LTIPauseItemChanged", "AdvAccept", w, &NO_MODES));
        let kept = n.take_news().settings.unwrap();
        assert_eq!(kept.advanced, [1.0, 2.0, 0.0, 0.0, 2.0, 1.0, 0.0, 0.0]);
        assert!(n.library("LTIVideoAdvanceEnter", "", w, &NO_MODES));
        assert_eq!(
            shown(n.calls(&s)),
            [
                "LTIVideoSetSwitchOpt1 true",
                "LTIVideoSetSwitchOpt2 2",
                "LTIVideoSetSwitchOpt3 0",
                "LTIVideoSetSwitchOpt4 false",
                "LTIVideoSetSwitchOpt5 2",
                "LTIVideoSetSwitchOpt6 1",
                "LTIVideoSetSwitchOpt7 false",
                "LTIVideoSetSwitchOpt8 false"
            ]
        );
    }

    #[test]
    fn the_keyboard_table_works_as_the_pause_menus() {
        use winit::keyboard::KeyCode;
        let mut n = Native::default();
        let s = Settings::default();
        n.calls(&s);
        let w = Video::default();
        assert!(n.library("LTIInputKMEnter", "", w, &NO_MODES));
        n.calls(&s);
        assert!(n.library("LTIInputKMChangeInput", "111", w, &NO_MODES));
        assert_eq!(n.take_news().capture, Some(true));
        n.captured(Control::Key(KeyCode::KeyQ));
        assert!(shown(n.calls(&s)).contains(&"LTIInputKMKeyMap 11 true Q".to_owned()));
        assert!(n.library("LTIInputKMApplyChanges", "", w, &NO_MODES));
        assert!(n.take_news().keys.is_some());
    }

    #[test]
    fn only_the_heroes_names_pick_a_hero() {
        use crate::hero::Hero;
        assert_eq!(hero("Chris"), Some(Hero::Chris));
        assert_eq!(hero("Jen"), Some(Hero::Jennifer));
        assert_eq!(hero("jennifer"), Some(Hero::Jennifer));
        assert_eq!(hero("mattias"), Some(Hero::Mattias));
        // What a missing or stubbed `GetSelectedCharacter` leaves.
        assert_eq!(hero(""), None);
        assert_eq!(hero("table: 0x0123"), None);
        assert_eq!(hero("1"), None);
    }

    #[test]
    fn the_profile_keeps_the_accepted_character() {
        let mut n = Native::default();
        assert_eq!(n.character(), None);
        assert!(n.command("onlineSetParameters", "serverOnline, true"));
        assert_eq!(n.character(), None);
        assert!(n.command("onlineSetParameters", "profileCharacter, 2"));
        assert_eq!(n.character(), Some("chris"));
        assert!(n.command("onlineSetParameters", "profileCharacter, undefined"));
        assert_eq!(n.character(), Some("chris"));
        assert!(n.command("onlineSetParameters", "profileCharacter, 3"));
        assert_eq!(n.character(), Some("jennifer"));
        assert!(n.command("onlineSetParameters", "profileCharacter, 1"));
        assert_eq!(n.character(), Some("mattias"));
    }
}
