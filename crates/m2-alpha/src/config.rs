//! Settings, read from `m2-alpha.toml` and overridden on the command line.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::input::Bindings;

pub const FILE_NAME: &str = "m2-alpha.toml";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// The game's install folder (the one holding `data\`).
    pub install: Option<PathBuf>,
    /// Who to play: `mattias`, `jennifer` or `chris`.
    pub hero: crate::hero::Hero,
    pub window: WindowConfig,
    pub camera: CameraConfig,
    pub world: WorldConfig,
    pub keys: Bindings,
    pub options: GameOptions,
    /// Changes from the original game, each off unless switched on.
    pub enhanced: EnhancedConfig,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EnhancedConfig {
    /// Smarter enemy soldiers (`enabled = true` to switch on).
    pub smarter_ai: crate::npc::SmarterAi,
    /// The shop from anywhere, every item unlocked (a cheat; the game
    /// unlocks them as contract rewards).
    pub shop: ShopCheat,
}

/// `[enhanced.shop]`: the shop opens anywhere with its key, as the game's
/// contacts open it in their menus; `unlock_all` unlocks everything each
/// shop could sell, and brings the pilots and mechanic the support needs
/// (the story recruits them one by one).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShopCheat {
    pub unlock_all: bool,
}

/// The game's own options (its pause menu's).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GameOptions {
    /// Subtitles for the movies and the characters' lines
    /// (`Sys.SubtitlesEnabled`). Captions such as place names show
    /// either way, as the game shows them.
    pub subtitles: bool,
    /// The tutorial tips (`Sys.TutorialsEnabled`).
    pub tutorials: bool,
    /// The scripts' autosaves after a contract are written.
    pub autosave: bool,
    /// The pause menu's sliders, 0 to 100 (see [`crate::pause::Settings`]):
    /// the pad's look speed, the brightness and view distance (kept, not
    /// used yet) and the volumes.
    pub pad_sensitivity: f64,
    pub invert_pad: bool,
    pub rumble: bool,
    pub brightness: f64,
    pub view_distance: f64,
    pub sfx_volume: f64,
    pub music_volume: f64,
    pub dialog_volume: f64,
    /// The front end's Graphic Detail and advanced video options (see
    /// [`crate::pause::Settings::advanced`]): kept for the menus, not
    /// used by the alpha's drawing.
    pub video_detail: f64,
    pub advanced_video: [f64; 8],
}

impl Default for GameOptions {
    fn default() -> Self {
        Self {
            subtitles: true,
            tutorials: true,
            autosave: true,
            pad_sensitivity: 50.0,
            invert_pad: false,
            rumble: true,
            brightness: 50.0,
            view_distance: 100.0,
            sfx_volume: 100.0,
            music_volume: 100.0,
            dialog_volume: 100.0,
            video_detail: crate::pause::DETAIL,
            advanced_video: crate::pause::ADVANCED,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorldConfig {
    /// Height of the sea, metres (the game's is -36).
    pub sea_level: f32,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            sea_level: m2_engine::WorldSettings::default().sea_level,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WindowConfig {
    /// The window's size; fullscreen, the mode's.
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
    /// Fullscreen, the mode's refresh rate in hertz (the pause menu's
    /// Refresh Rate); 0 is the monitor's own mode, borderless.
    pub refresh: u32,
    pub vsync: bool,
    /// The video screen's Aspect Ratio: wide, or 4:3 (the world seen as
    /// on a 4:3 screen, filling the window).
    pub wide: bool,
    /// The world drawn at this share of the window's size (0.25 to 1) and
    /// stretched to it; the HUD keeps the window's. Invented: 1 on the
    /// desktop, 0.6 on Android, where drawing every pixel of a phone's
    /// screen held play under 10 fps (an Adreno 830 at 2244x1080).
    pub render_scale: f32,
    /// The most frames drawn a second (0: as many as the display takes);
    /// between them the game sleeps. Invented: none on the desktop, 30 on
    /// Android, where drawing as fast as it could ran the phone's battery
    /// down fast.
    pub max_fps: u32,
}

/// [`WindowConfig::max_fps`]'s default.
pub const MAX_FPS: u32 = if cfg!(target_os = "android") { 30 } else { 0 };

/// [`WindowConfig::render_scale`]'s default.
pub const RENDER_SCALE: f32 = if cfg!(target_os = "android") {
    0.6
} else {
    1.0
};

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 1600,
            height: 900,
            fullscreen: false,
            refresh: 0,
            vsync: true,
            wide: true,
            render_scale: RENDER_SCALE,
            max_fps: MAX_FPS,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CameraConfig {
    /// Vertical field of view in degrees.
    pub fov: f32,
    /// Radians per pixel of mouse movement.
    pub mouse_sensitivity: f32,
    /// Free-fly speed in metres per second (sprint is 5x).
    pub speed: f32,
    pub invert_y: bool,
}

impl Default for CameraConfig {
    fn default() -> Self {
        Self {
            fov: 60.0,
            mouse_sensitivity: 0.0025,
            speed: 8.0,
            invert_y: false,
        }
    }
}

impl Config {
    /// Reads `path`, or returns the defaults if it doesn't exist.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let mut config: Self = toml::from_str(&text)
                    .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
                // A [keys] table rebinds the actions it names; the rest keep
                // their defaults.
                for (action, keys) in Bindings::default().0 {
                    config.keys.0.entry(action).or_insert(keys);
                }
                Ok(config)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(anyhow::anyhow!("{}: {e}", path.display())),
        }
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("config serialises")
    }

    /// The settings as the game's menus show them (their option screens'
    /// items: [`crate::pause::Settings`]); the profile's name, the window,
    /// its monitor's modes and the saves are the caller's to fill.
    pub fn menu_settings(&self) -> crate::pause::Settings {
        let o = &self.options;
        crate::pause::Settings {
            subtitles: o.subtitles,
            tutorials: o.tutorials,
            autosave: o.autosave,
            invert_mouse: self.camera.invert_y,
            mouse: crate::pause::mouse_slider(self.camera.mouse_sensitivity),
            invert_pad: o.invert_pad,
            pad: o.pad_sensitivity,
            rumble: o.rumble,
            vsync: self.window.vsync,
            wide: self.window.wide,
            brightness: o.brightness,
            view_distance: o.view_distance,
            sfx: o.sfx_volume,
            music: o.music_volume,
            dialog: o.dialog_volume,
            keys: crate::keytable::KeyTable::of(&self.keys),
            detail: o.video_detail,
            advanced: o.advanced_video,
            ..crate::pause::Settings::default()
        }
    }

    /// Takes the settings a menu kept ([`Config::menu_settings`]'s items;
    /// the key table and the window are kept on their own).
    pub fn keep_menu_settings(&mut self, s: &crate::pause::Settings) {
        let o = &mut self.options;
        o.subtitles = s.subtitles;
        o.tutorials = s.tutorials;
        o.autosave = s.autosave;
        o.invert_pad = s.invert_pad;
        o.pad_sensitivity = s.pad;
        o.rumble = s.rumble;
        o.brightness = s.brightness;
        o.view_distance = s.view_distance;
        o.sfx_volume = s.sfx;
        o.music_volume = s.music;
        o.dialog_volume = s.dialog;
        o.video_detail = s.detail;
        o.advanced_video = s.advanced;
        self.camera.invert_y = s.invert_mouse;
        self.camera.mouse_sensitivity = crate::pause::mouse_radians(s.mouse);
        self.window.vsync = s.vsync;
        self.window.wide = s.wide;
    }

    /// Writes the settings to the settings file ([`Config::default_path`]),
    /// as the game saves its profile; `who` names the menu in the log.
    pub fn save(&self, who: &str) {
        let path = Self::default_path();
        match std::fs::write(&path, self.to_toml()) {
            Ok(()) => eprintln!("{who}: settings kept, saved to {}", path.display()),
            Err(e) => eprintln!("{who}: could not save {}: {e}", path.display()),
        }
    }

    /// Where to look for the settings file: the working directory, then
    /// beside the executable.
    pub fn default_path() -> PathBuf {
        let here = PathBuf::from(FILE_NAME);
        if here.is_file() {
            return here;
        }
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.join(FILE_NAME)))
            .filter(|p| p.is_file())
            .unwrap_or(here)
    }

    /// Where the game keeps its other files (saves, the catalogue cache,
    /// crash reports): beside the settings file if there is one, else
    /// beside the executable.
    pub fn file_beside(name: &str) -> PathBuf {
        let settings = Self::default_path();
        // Android: the game's folder, the working directory.
        if settings.is_file() || cfg!(target_os = "android") {
            return settings.with_file_name(name);
        }
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.join(name)))
            .unwrap_or_else(|| PathBuf::from(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip() {
        let config = Config::default();
        let back: Config = toml::from_str(&config.to_toml()).unwrap();
        assert_eq!(back, config);
    }

    #[test]
    fn partial_files_keep_defaults() {
        let config: Config = toml::from_str(
            r#"
            install = 'C:\Games\Mercenaries 2'
            [window]
            vsync = false
            "#,
        )
        .unwrap();
        assert_eq!(
            config.install,
            Some(PathBuf::from(r"C:\Games\Mercenaries 2"))
        );
        assert!(!config.window.vsync);
        assert_eq!(config.window.width, 1600);
        assert_eq!(config.keys, Bindings::default());
    }

    #[test]
    fn the_smarter_soldiers_are_off_unless_switched_on() {
        assert!(!Config::default().enhanced.smarter_ai.enabled);
        let config: Config = toml::from_str(
            r#"
            [enhanced.smarter_ai]
            enabled = true
            callout_range = 60.0
            "#,
        )
        .unwrap();
        let smart = config.enhanced.smarter_ai;
        assert!(smart.enabled);
        assert_eq!(smart.callout_range, 60.0);
        assert!(smart.flank, "the rest keep their defaults");
    }

    #[test]
    fn the_menus_settings_round_trip_through_the_file() {
        let mut config = Config::default();
        let mut s = config.menu_settings();
        assert_eq!(
            s.mouse,
            crate::pause::mouse_slider(config.camera.mouse_sensitivity)
        );
        (s.subtitles, s.vsync, s.music, s.view_distance) = (false, false, 30.0, 40.0);
        (s.invert_mouse, s.pad, s.rumble, s.detail) = (true, 80.0, false, 1.0);
        s.advanced[7] = 0.0;
        config.keep_menu_settings(&s);
        let back: Config = toml::from_str(&config.to_toml()).unwrap();
        let again = back.menu_settings();
        assert_eq!(
            (
                again.subtitles,
                again.vsync,
                again.music,
                again.view_distance
            ),
            (false, false, 30.0, 40.0)
        );
        assert_eq!(
            (again.invert_mouse, again.pad, again.rumble),
            (true, 80.0, false)
        );
        assert_eq!((again.detail, again.advanced[7]), (1.0, 0.0));
        assert!((again.mouse - s.mouse).abs() <= 1.0);
    }

    #[test]
    fn unknown_settings_are_errors() {
        assert!(toml::from_str::<Config>("[window]\nvsinc = false").is_err());
    }
}
