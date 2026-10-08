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
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
    pub vsync: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 1600,
            height: 900,
            fullscreen: false,
            vsync: true,
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
        if settings.is_file() {
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
    fn unknown_settings_are_errors() {
        assert!(toml::from_str::<Config>("[window]\nvsinc = false").is_err());
    }
}
