//! The Options page's Bug Report part: a preview of what a report holds,
//! a button that writes the redacted zip to the reports folder and opens
//! it, and one that copies a short issue template to the clipboard
//! ([`crate::report`]). Nothing is uploaded. Layout invented.

use std::path::{Path, PathBuf};

use egui::Ui;

use super::{DIM, FAINT, WARN};
use crate::launch::Settings;
use crate::report::{self, Info, Redactor};

/// What the part shows between frames.
#[derive(Debug, Default)]
pub(super) struct BugReport {
    preview: Option<Vec<String>>,
    note: Option<(String, bool)>,
}

/// The game's log: beside its settings or saves, else beside the launcher.
fn log_path(saves: &Path, dir: &Path) -> PathBuf {
    let beside = saves.join(report::LOG_FILE);
    if beside.is_file() {
        beside
    } else {
        dir.join(report::LOG_FILE)
    }
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

/// The report's files, from what is on disk now.
fn gather(saves: &Path, dir: &Path, settings: &Settings) -> (Vec<(String, Vec<u8>)>, Info) {
    let log = std::fs::read(log_path(saves, dir))
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned());
    let info = Info::gather(log.as_deref());
    let toml = toml::to_string_pretty(settings).unwrap_or_default();
    let files = report::bundle(log.as_deref(), &toml, &info, &Redactor::from_env(env));
    (files, info)
}

impl BugReport {
    pub(super) fn section(&mut self, ui: &mut Ui, saves: &Path, dir: &Path, settings: &Settings) {
        let folder = report::reports_dir(env);
        ui.colored_label(
            DIM,
            "A bug report is a zip of the game's last log, this build's details and these \
             settings, with your user name, home folder, machine name, IP addresses and \
             tokens taken out. It stays on this computer: nothing is sent. Saves and game \
             files are never included.",
        );
        ui.colored_label(FAINT, format!("Reports go to {}", folder.display()));
        ui.horizontal(|ui| {
            if ui.button("Preview").clicked() {
                let (files, _) = gather(saves, dir, settings);
                self.preview = Some(report::preview(&files));
            }
            if ui.button("Make bug report").clicked() {
                let (files, _) = gather(saves, dir, settings);
                let unix = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                self.note = Some(match report::write(&folder, &files, unix) {
                    Ok(path) => {
                        report::open_folder(&folder);
                        (format!("Saved {}", path.display()), false)
                    }
                    Err(e) => (format!("Could not write the report: {e}"), true),
                });
                self.preview = Some(report::preview(&files));
            }
            if ui.button("Copy GitHub issue text").clicked() {
                let (_, info) = gather(saves, dir, settings);
                ui.ctx().copy_text(report::issue_text(&info));
                self.note = Some((
                    "Issue text copied: build details and prompts only, no log.".to_owned(),
                    false,
                ));
            }
        });
        if let Some(lines) = &self.preview {
            ui.colored_label(DIM, "The report holds:");
            for l in lines {
                ui.colored_label(FAINT, format!("  {l}"));
            }
        }
        if let Some((text, bad)) = &self.note {
            ui.colored_label(if *bad { WARN } else { DIM }, text);
        }
    }
}
