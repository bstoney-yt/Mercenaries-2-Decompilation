//! `m2-launcher`: the game's launcher. It sits beside `m2-alpha`, lists
//! the save slots, keeps the window options, and starts the game with a
//! new game or a save picked. Its pictures and fonts are the game's own,
//! read from the player's install as the game reads them; without an
//! install it still runs, in egui's fonts on plain panels.
//!
//! The layout (a sidebar of games and pages, a header with tabs, a banner
//! picture over a bar with the save picker, a large Play button and the
//! player's name) follows the common game-launcher pattern; the drawing,
//! colours and code are the launcher's own.
//!
//! ```text
//! m2-launcher [--dir <game folder>] [--install <dir>]
//!             [--screenshot <out.png>] [--tab play|saves|stats|achievements|options]
//! ```
//!
//! `--dir` is where `m2-alpha` and its saves are (default: the launcher's
//! own folder). `--screenshot` waits for the install's fonts and pictures,
//! writes the window to a PNG and closes (`--tab` picks the tab shown).
//!
//! ```text
//! m2-launcher --write-manifest <folder> <tag> <commit> <built>
//! ```
//!
//! is the release workflow's: it writes `manifest.toml` and `SHA256SUMS`
//! for the programs in the folder ([`update::FILES`]) and exits.

#![cfg_attr(feature = "windowed", windows_subsystem = "windows")]

mod achievements;
mod assets;
mod launch;
mod save_stats;
mod saves;
mod stats;
mod text;
mod ui;
mod update;

use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use m2_engine::Install;

use crate::launch::{GAME_SETTINGS_FILE, GameSettings, SETTINGS_FILE, Settings};

/// Where the launcher's files and the game's are.
#[derive(Clone, Debug)]
pub struct Paths {
    /// The game's folder (`m2-alpha` and, by default, its saves).
    pub dir: PathBuf,
    /// The launcher's settings file.
    pub settings: PathBuf,
    /// Where the saves are: beside the game's settings file if it has
    /// one, else in the game's folder (as `m2-alpha` keeps them).
    pub saves: PathBuf,
    /// The game's settings file.
    pub game_settings: PathBuf,
    /// The launcher's own play stats ([`stats::STATS_FILE`]).
    pub stats: PathBuf,
}

impl Paths {
    pub fn new(dir: &Path) -> Self {
        // Absolute, so the folders read as paths in the Options tab.
        let dir = &std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
        let here =
            std::path::absolute(GAME_SETTINGS_FILE).unwrap_or_else(|_| GAME_SETTINGS_FILE.into());
        let game_settings = if here.is_file() {
            here
        } else {
            dir.join(GAME_SETTINGS_FILE)
        };
        let saves = if game_settings.is_file() {
            game_settings
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
        } else {
            dir.to_path_buf()
        };
        Self {
            dir: dir.to_path_buf(),
            settings: dir.join(SETTINGS_FILE),
            stats: dir.join(stats::STATS_FILE),
            saves,
            game_settings,
        }
    }
}

#[derive(Debug, Default)]
pub struct Args {
    pub dir: Option<PathBuf>,
    pub install: Option<PathBuf>,
    pub screenshot: Option<PathBuf>,
    pub tab: ui::Tab,
}

fn parse(args: impl IntoIterator<Item = String>) -> anyhow::Result<Args> {
    let mut out = Args::default();
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        let mut value = || args.next().with_context(|| format!("{a} needs a value"));
        match a.as_str() {
            "--dir" => out.dir = Some(value()?.into()),
            "--install" => out.install = Some(value()?.into()),
            "--screenshot" => out.screenshot = Some(value()?.into()),
            "--tab" => {
                out.tab = match value()?.as_str() {
                    "play" => ui::Tab::Play,
                    "saves" => ui::Tab::Saves,
                    "stats" => ui::Tab::Stats,
                    "achievements" => ui::Tab::Achievements,
                    "options" => ui::Tab::Options,
                    v => bail!("--tab {v}: expected play, saves, stats, achievements or options"),
                }
            }
            "-h" | "--help" => {
                println!(
                    "m2-launcher [--dir <game folder>] [--install <dir>] [--screenshot <out.png>] [--tab play|saves|stats|achievements|options]"
                );
                std::process::exit(0);
            }
            _ => bail!("unknown argument {a} (try --help)"),
        }
    }
    Ok(out)
}

/// The install: the command line's, the launcher's setting, the game's
/// setting, else the EA app's folder. The second value says whether the
/// game must be told (it finds the last two itself).
pub fn find_install(
    arg: Option<&Path>,
    settings: &Settings,
    game: &GameSettings,
) -> (Option<Install>, bool) {
    if let Some(i) = arg.or(settings.install.as_deref()) {
        return (Install::find(i).ok(), true);
    }
    if let Some(i) = &game.install {
        return (Install::find(i).ok(), false);
    }
    (Install::detect(), false)
}

/// `--write-manifest <folder> <tag> <commit> <built>`.
fn write_manifest(rest: &[String]) -> anyhow::Result<()> {
    let [folder, tag, sha, built] = rest else {
        bail!("--write-manifest <folder> <tag> <commit> <built>");
    };
    let folder = Path::new(folder);
    let m = update::manifest_for(folder, tag, sha, built).map_err(anyhow::Error::msg)?;
    std::fs::write(folder.join(update::MANIFEST), m.to_text())?;
    std::fs::write(folder.join("SHA256SUMS"), update::sums_text(&m))?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let all: Vec<String> = std::env::args().skip(1).collect();
    if all.first().map(String::as_str) == Some("--write-manifest") {
        return write_manifest(&all[1..]);
    }
    let args = parse(all)?;
    let dir = match &args.dir {
        Some(d) => d.clone(),
        None => std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from(".")),
    };
    let paths = Paths::new(&dir);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Mercenaries 2 Launcher")
            .with_inner_size([1280.0, 760.0])
            .with_min_inner_size([960.0, 600.0]),
        renderer: eframe::Renderer::Wgpu,
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "Mercenaries 2 Launcher",
        options,
        Box::new(move |cc| Ok(Box::new(ui::App::new(cc, paths, args)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_read() {
        let a = parse(
            ["--dir", "x", "--screenshot", "s.png", "--tab", "saves"]
                .map(String::from)
                .to_vec(),
        )
        .unwrap();
        assert_eq!(a.dir, Some(PathBuf::from("x")));
        assert_eq!(a.screenshot, Some(PathBuf::from("s.png")));
        assert_eq!(a.tab, ui::Tab::Saves);
        let a = parse(["--tab", "stats"].map(String::from).to_vec()).unwrap();
        assert_eq!(a.tab, ui::Tab::Stats);
        assert!(parse(["--tab".to_owned(), "mods".to_owned()]).is_err());
        assert!(parse(["--what".to_owned()]).is_err());
    }

    #[test]
    fn saves_are_beside_the_games_settings_else_in_its_folder() {
        let d = std::env::temp_dir().join(format!("m2-launcher-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let p = Paths::new(&d);
        assert_eq!(p.saves, d);
        assert_eq!(p.settings, d.join("m2-launcher.toml"));
        assert_eq!(p.stats, d.join("m2-launcher-stats.toml"));
        std::fs::write(d.join(GAME_SETTINGS_FILE), "").unwrap();
        assert_eq!(Paths::new(&d).saves, d);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_named_install_is_passed_on_and_a_found_one_is_not() {
        let s = Settings::default();
        let g = GameSettings::default();
        let (_, pass) = find_install(Some(Path::new("/nowhere")), &s, &g);
        assert!(pass);
        let g = GameSettings {
            install: Some("/nowhere".into()),
            hero: None,
        };
        let (found, pass) = find_install(None, &s, &g);
        assert!(found.is_none() && !pass);
    }
}
