//! Starting the game: `m2-alpha` beside the launcher, with the window as
//! the Options tab has it and what Play starts (a new game as a hero, or
//! a save slot), which the game's front end then does as its own New
//! Game or Load Game would (`--new-game`, `--load-slot`).
//!
//! The launcher's own settings live in `m2-launcher.toml` beside it. The
//! game's settings file (`m2-alpha.toml`) is only read, for where the
//! game is installed and who the player last was.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const SETTINGS_FILE: &str = "m2-launcher.toml";
/// The game's settings file (`config::FILE_NAME` in `m2-alpha`).
pub const GAME_SETTINGS_FILE: &str = "m2-alpha.toml";

/// How the game's window is shown (`--display`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Display {
    #[default]
    Windowed,
    Borderless,
    Exclusive,
}

impl Display {
    pub const ALL: [Display; 3] = [Display::Windowed, Display::Borderless, Display::Exclusive];

    pub fn arg(self) -> &'static str {
        match self {
            Display::Windowed => "windowed",
            Display::Borderless => "borderless",
            Display::Exclusive => "exclusive",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Display::Windowed => "Windowed",
            Display::Borderless => "Borderless fullscreen",
            Display::Exclusive => "Exclusive fullscreen",
        }
    }
}

/// The window sizes offered.
pub const SIZES: [(u32, u32); 6] = [
    (1280, 720),
    (1600, 900),
    (1920, 1080),
    (2560, 1080),
    (2560, 1440),
    (3840, 2160),
];

/// The launcher's settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub width: u32,
    pub height: u32,
    pub display: Display,
    /// Leave out the EA and Pandemic movies (`--to-front-end`).
    pub skip_intro: bool,
    /// The launcher stays open while the game runs.
    pub keep_open: bool,
    /// Who a new game starts as: `mattias`, `jennifer` or `chris`.
    pub hero: String,
    /// The game's install folder, if not the game's own setting or found.
    pub install: Option<PathBuf>,
    /// The game's program, if not `m2-alpha` beside the launcher.
    pub game: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            display: Display::Windowed,
            skip_intro: false,
            keep_open: false,
            hero: "jennifer".to_owned(),
            install: None,
            game: None,
        }
    }
}

impl Settings {
    pub fn read(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        std::fs::write(
            path,
            toml::to_string_pretty(self).expect("settings always serialise"),
        )
    }
}

/// What the launcher reads of the game's settings.
#[derive(Debug, Default, Deserialize)]
pub struct GameSettings {
    #[serde(default)]
    pub install: Option<PathBuf>,
    #[serde(default)]
    pub hero: Option<String>,
}

impl GameSettings {
    pub fn read(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }
}

/// What Play starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Start {
    /// A new game as this hero.
    New(String),
    /// The save in this slot.
    Slot(u32),
}

/// The game's arguments for `start` with `settings`; `install` when the
/// launcher had to find it (the game finds the EA app's folder itself).
pub fn args(settings: &Settings, install: Option<&Path>, start: &Start) -> Vec<String> {
    let mut out = vec![
        "--size".to_owned(),
        format!("{}x{}", settings.width, settings.height),
        "--display".to_owned(),
        settings.display.arg().to_owned(),
    ];
    if settings.skip_intro {
        out.push("--to-front-end".to_owned());
    }
    if let Some(i) = install {
        out.push("--install".to_owned());
        out.push(i.display().to_string());
    }
    match start {
        Start::New(hero) => {
            out.extend(["--hero".to_owned(), hero.clone(), "--new-game".to_owned()]);
        }
        Start::Slot(n) => out.extend(["--load-slot".to_owned(), n.to_string()]),
    }
    out
}

/// The game's program: the setting, else `m2-alpha` in `dir` (or the
/// beta release's `m2-beta`, if only it is there).
pub fn game_path(settings: &Settings, dir: &Path) -> PathBuf {
    settings.game.clone().unwrap_or_else(|| {
        let exe = |name: &str| dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        let alpha = exe("m2-alpha");
        let beta = exe("m2-beta");
        if !alpha.is_file() && beta.is_file() {
            beta
        } else {
            alpha
        }
    })
}

/// Starts the game, in its own folder (where it looks for its settings
/// and saves).
pub fn spawn(game: &Path, args: &[String]) -> Result<std::process::Child, String> {
    let mut cmd = std::process::Command::new(game);
    cmd.args(args);
    if let Some(dir) = game.parent().filter(|d| !d.as_os_str().is_empty()) {
        cmd.current_dir(dir);
    }
    cmd.spawn().map_err(|e| format!("{}: {e}", game.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_passes_the_window_and_what_to_start() {
        let s = Settings::default();
        assert_eq!(
            args(&s, None, &Start::Slot(3)),
            [
                "--size",
                "1920x1080",
                "--display",
                "windowed",
                "--load-slot",
                "3"
            ]
        );
        let s = Settings {
            skip_intro: true,
            display: Display::Borderless,
            width: 2560,
            height: 1440,
            ..Settings::default()
        };
        assert_eq!(
            args(
                &s,
                Some(Path::new("D:/Mercs2")),
                &Start::New("chris".into())
            ),
            [
                "--size",
                "2560x1440",
                "--display",
                "borderless",
                "--to-front-end",
                "--install",
                "D:/Mercs2",
                "--hero",
                "chris",
                "--new-game"
            ]
        );
    }

    #[test]
    fn settings_round_trip_and_fill_in_what_is_missing() {
        let s = Settings {
            keep_open: true,
            hero: "mattias".into(),
            ..Settings::default()
        };
        let text = toml::to_string_pretty(&s).unwrap();
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), s);
        let partial: Settings = toml::from_str("width = 1280\nheight = 720\n").unwrap();
        assert_eq!((partial.width, partial.display), (1280, Display::Windowed));
    }

    #[test]
    fn the_game_is_beside_the_launcher_as_the_alpha_or_the_beta() {
        let d = std::env::temp_dir().join(format!("m2-launcher-game-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let s = Settings::default();
        let name = |p: PathBuf| p.file_stem().unwrap().to_string_lossy().into_owned();
        assert_eq!(name(game_path(&s, &d)), "m2-alpha");
        let beta = d.join(format!("m2-beta{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&beta, "").unwrap();
        assert_eq!(game_path(&s, &d), beta);
        let s = Settings {
            game: Some("elsewhere".into()),
            ..Settings::default()
        };
        assert_eq!(game_path(&s, &d), PathBuf::from("elsewhere"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_games_settings_give_its_install_and_hero() {
        let g: GameSettings = toml::from_str(
            "install = 'C:\\Games\\Mercenaries 2'\nhero = \"chris\"\n[window]\nwidth = 1\n",
        )
        .unwrap();
        assert_eq!(g.install, Some(PathBuf::from(r"C:\Games\Mercenaries 2")));
        assert_eq!(g.hero.as_deref(), Some("chris"));
    }
}
