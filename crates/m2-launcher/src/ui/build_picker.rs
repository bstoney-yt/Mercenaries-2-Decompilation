//! The Play tab's build picker: a chip at the banner's top right naming
//! the build Play starts, opening a list of the builds on offer
//! ([`crate::builds`]); the one picked is pinned in the settings. The
//! latest release, when it is not here, is downloaded on picking it (its
//! game program only, checked). Layout and colours invented.

use std::path::PathBuf;

use egui::{Align2, Color32, Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::{App, BAR, DIM, FAINT, GREEN, LINE, TEXT};
use crate::assets::Role;
use crate::builds::{self, Choice, INSTALLED, Kind};
use crate::update;

impl App {
    fn build_choices(&self) -> Vec<Choice> {
        builds::list(
            &self.program_dir,
            update::BUILD_SHA,
            self.upd_found.as_ref().map(|f| &f.manifest),
        )
    }

    /// The game program Play starts (the pinned build's, else the
    /// installed one) and, when the pinned one isn't here, why not.
    pub(super) fn picked_game(&self) -> (Option<PathBuf>, Option<String>) {
        if self
            .settings
            .build
            .as_deref()
            .is_none_or(|b| b == INSTALLED)
        {
            return (None, None);
        }
        let (pick, note) = builds::resolve(&self.build_choices(), self.settings.build.as_deref());
        match pick.kind {
            Kind::Installed => (None, note),
            _ => (pick.exe, note),
        }
    }

    fn pin_build(&mut self, key: &str) {
        self.settings.build = (key != INSTALLED).then(|| key.to_owned());
        if let Err(e) = self.settings.write(&self.paths.settings) {
            self.status = Some((format!("Could not save the settings: {e}"), true));
        }
    }

    /// A download of the latest release's game program, finished.
    pub(super) fn poll_build_download(&mut self) {
        let Some(rx) = &self.build_dl else {
            return;
        };
        let Ok(result) = rx.try_recv() else {
            return;
        };
        self.build_dl = None;
        match result {
            Ok((tag, _)) => {
                self.pin_build(&tag);
                self.status = Some((format!("Build {tag} downloaded and picked."), false));
            }
            Err(e) => self.status = Some((format!("Could not download the build: {e}"), true)),
        }
    }

    fn download_latest(&mut self, ctx: &egui::Context) {
        if self.build_dl.is_some() {
            return;
        }
        let Some(found) = self.upd_found.clone() else {
            return;
        };
        let dir = self.program_dir.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        self.status = Some((
            format!("Downloading build {}...", found.manifest.tag),
            false,
        ));
        std::thread::spawn(move || {
            let r = builds::keep_release(&dir, &found.manifest, &mut |name| {
                update::fetch_release_file(&found, name)
            })
            .map(|exe| (found.manifest.tag.clone(), exe));
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.build_dl = Some(rx);
    }

    /// The chip at the banner's top right, and its list when open.
    pub(super) fn build_chip(&mut self, ui: &mut Ui, hero: Rect) {
        let ctx = ui.ctx().clone();
        let choices = self.build_choices();
        let (pick, note) = builds::resolve(&choices, self.settings.build.as_deref());
        let p = ui.painter_at(hero);
        let text = format!("BUILD  {}", pick.label());
        let w = self.fonts.width(&p, Role::Body, 13.0, &text) + 40.0;
        let chip = Rect::from_min_size(
            pos2(hero.right() - 20.0 - w, hero.top() + 16.0),
            vec2(w, 28.0),
        );
        let open_id = Id::new("build-picker-open");
        let mut open: bool = ctx.data(|d| d.get_temp(open_id)).unwrap_or(false);
        let r = ui.interact(chip, Id::new("build-chip"), Sense::click());
        p.rect_filled(
            chip,
            14.0,
            Color32::from_black_alpha(if r.hovered() || open { 220 } else { 160 }),
        );
        if note.is_some() {
            p.rect_stroke(
                chip,
                14.0,
                Stroke::new(1.0, super::WARN),
                StrokeKind::Inside,
            );
        }
        self.fonts.text(
            &p,
            Role::Body,
            chip.left_center() + vec2(14.0, 0.0),
            Align2::LEFT_CENTER,
            13.0,
            TEXT,
            &text,
        );
        // A small "v" for the list.
        let c = chip.right_center() - vec2(16.0, 0.0);
        p.line_segment(
            [c + vec2(-4.0, -2.0), c + vec2(0.0, 2.0)],
            Stroke::new(1.5, DIM),
        );
        p.line_segment(
            [c + vec2(4.0, -2.0), c + vec2(0.0, 2.0)],
            Stroke::new(1.5, DIM),
        );
        if r.clicked() {
            open = !open;
        }
        if let Some(note) = &note
            && r.hovered()
        {
            r.clone().on_hover_text(note);
        }
        if open {
            let row_h = 36.0;
            let width = choices
                .iter()
                .map(|c| self.fonts.width(&p, Role::Body, 14.0, &c.label()))
                .fold(260.0f32, f32::max)
                + 48.0;
            let list = Rect::from_min_size(
                pos2(chip.right() - width, chip.bottom() + 6.0),
                vec2(width, row_h * choices.len() as f32 + 12.0),
            );
            let lp = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                Id::new("build-list"),
            ));
            lp.rect_filled(list, 6.0, BAR);
            lp.rect_stroke(list, 6.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
            let mut picked: Option<Choice> = None;
            for (i, c) in choices.iter().enumerate() {
                let row = Rect::from_min_size(
                    pos2(list.left() + 6.0, list.top() + 6.0 + row_h * i as f32),
                    vec2(list.width() - 12.0, row_h),
                );
                let rr = ui
                    .interact(row, Id::new(("build-row", i)), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if rr.hovered() {
                    lp.rect_filled(row, 4.0, Color32::from_rgb(0x2c, 0x2c, 0x2c));
                }
                let on = c.key == pick.key;
                lp.circle_stroke(
                    row.left_center() + vec2(14.0, 0.0),
                    6.0,
                    Stroke::new(1.5, DIM),
                );
                if on {
                    lp.circle_filled(row.left_center() + vec2(14.0, 0.0), 3.5, GREEN);
                }
                let colour = if c.kind == Kind::Latest { FAINT } else { TEXT };
                self.fonts.text(
                    &lp,
                    Role::Body,
                    row.left_center() + vec2(30.0, 0.0),
                    Align2::LEFT_CENTER,
                    14.0,
                    colour,
                    &c.label(),
                );
                if rr.clicked() {
                    picked = Some(c.clone());
                }
            }
            if let Some(c) = picked {
                open = false;
                if c.kind == Kind::Latest {
                    self.download_latest(&ctx);
                } else {
                    self.pin_build(&c.key);
                    self.status = Some((format!("Play starts {}.", c.label()), false));
                }
            } else if ctx.input(|i| i.pointer.any_click())
                && !r.clicked()
                && !ctx
                    .input(|i| i.pointer.interact_pos())
                    .is_some_and(|p| list.contains(p))
            {
                open = false;
            }
        }
        ctx.data_mut(|d| d.insert_temp(open_id, open));
    }
}
