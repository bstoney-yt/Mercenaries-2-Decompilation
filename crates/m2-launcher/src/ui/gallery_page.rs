//! The Gallery tab: the game's music, read from the install on a thread
//! when the tab first opens, listed by group with Play and Stop
//! ([`crate::gallery`]). Cut-scenes come in a later change. Layout and
//! colours invented.

use std::sync::mpsc::Receiver;

use egui::{Align2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use super::{App, BAR, BG, DIM, FAINT, GREEN, LINE, PAD, TEXT, WARN};
use crate::assets::Role;
use crate::gallery::{self, Music, Player};

/// The page's state between frames.
#[derive(Debug, Default)]
pub(super) struct GalleryPage {
    loading: Option<Receiver<Result<Music, String>>>,
    music: Option<Result<Music, String>>,
    player: Option<Player>,
    note: Option<(String, bool)>,
}

impl App {
    fn gallery_start_loading(&mut self, ctx: &egui::Context) {
        let g = &mut self.gallery;
        if g.loading.is_some() || g.music.is_some() {
            return;
        }
        let Some(install) = self.install.clone() else {
            g.music = Some(Err(
                "The game's install was not found (Options, Folders).".into()
            ));
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(gallery::load(&install));
            ctx.request_repaint();
        });
        g.loading = Some(rx);
    }

    pub(super) fn gallery_tab(&mut self, ui: &mut Ui, rect: Rect) {
        let ctx = ui.ctx().clone();
        self.gallery_start_loading(&ctx);
        if let Some(rx) = &self.gallery.loading
            && let Ok(r) = rx.try_recv()
        {
            if let Ok(m) = &r {
                eprintln!("launcher: gallery: {} pieces of music", m.tracks.len());
            }
            self.gallery.music = Some(r);
            self.gallery.loading = None;
        }
        if self.gallery.player.as_ref().is_some_and(Player::done) {
            self.gallery.player = None;
        }
        if self.gallery.player.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, BG);
        let top = Rect::from_min_size(
            pos2(rect.left() + PAD, rect.top() + 20.0),
            vec2(rect.width() - 2.0 * PAD, 64.0),
        );
        p.rect_filled(top, 6.0, BAR);
        p.rect_stroke(top, 6.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
        self.fonts.text(
            &p,
            Role::Heading,
            top.left_center() + vec2(18.0, 0.0),
            Align2::LEFT_CENTER,
            20.0,
            TEXT,
            "GALLERY: MUSIC",
        );
        let note = match &self.gallery.music {
            None => "Reading the music from the install...".to_owned(),
            Some(Err(_)) => String::new(),
            Some(Ok(m)) => format!(
                "{} pieces, from your install. All unlocked: saves keep no record of music heard.",
                m.tracks.len()
            ),
        };
        self.fonts.text(
            &p,
            Role::Body,
            top.right_center() - vec2(18.0, 0.0),
            Align2::RIGHT_CENTER,
            13.0,
            FAINT,
            &note,
        );
        let list = Rect::from_min_max(
            pos2(rect.left() + PAD, top.bottom() + 16.0),
            rect.max - vec2(PAD, 12.0),
        );
        let music = match &self.gallery.music {
            Some(Ok(m)) => m,
            Some(Err(e)) => {
                self.fonts.text(
                    &p,
                    Role::Body,
                    list.center_top() + vec2(0.0, 40.0),
                    Align2::CENTER_TOP,
                    16.0,
                    WARN,
                    &format!("The music could not be read: {e}"),
                );
                return;
            }
            None => return,
        };
        let playing = self
            .gallery
            .player
            .as_ref()
            .map(|p| (p.stream, p.elapsed()));
        let mut clicked: Option<usize> = None;
        let mut child = ui.new_child(UiBuilder::new().max_rect(list));
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                let mut group = "";
                for t in &music.tracks {
                    if t.group != group {
                        group = &t.group;
                        let (row, _) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
                        self.fonts.text(
                            ui.painter(),
                            Role::Heading,
                            row.left_bottom() + vec2(4.0, -8.0),
                            Align2::LEFT_BOTTOM,
                            15.0,
                            GREEN,
                            &group.to_uppercase(),
                        );
                    }
                    let (row, r) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                    let p = ui.painter().clone();
                    let on = playing.is_some_and(|(s, _)| s == t.stream);
                    p.rect_filled(
                        row.shrink2(vec2(0.0, 2.0)),
                        6.0,
                        if r.hovered() || on {
                            egui::Color32::from_rgb(0x2c, 0x2c, 0x2c)
                        } else {
                            BAR
                        },
                    );
                    // Play or stop: a triangle or a square.
                    let c = row.left_center() + vec2(22.0, 0.0);
                    if on {
                        p.rect_filled(Rect::from_center_size(c, vec2(10.0, 10.0)), 1.0, GREEN);
                    } else {
                        p.add(egui::Shape::convex_polygon(
                            vec![
                                c + vec2(-4.0, -6.0),
                                c + vec2(6.0, 0.0),
                                c + vec2(-4.0, 6.0),
                            ],
                            TEXT,
                            Stroke::NONE,
                        ));
                    }
                    self.fonts.text(
                        &p,
                        Role::BodyBold,
                        row.left_center() + vec2(44.0, -1.0),
                        Align2::LEFT_CENTER,
                        15.0,
                        TEXT,
                        &t.label,
                    );
                    let raw = t
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("cue {:08x}", t.cue));
                    let time = match playing {
                        Some((s, e)) if s == t.stream => {
                            format!("{} / {}", gallery::length(e), gallery::length(t.seconds))
                        }
                        _ => gallery::length(t.seconds),
                    };
                    self.fonts.text(
                        &p,
                        Role::Body,
                        row.right_center() - vec2(16.0, 0.0),
                        Align2::RIGHT_CENTER,
                        13.0,
                        DIM,
                        &format!("{raw}   {time}"),
                    );
                    if r.clicked() {
                        clicked = Some(t.stream);
                    }
                }
            });
        if let Some(stream) = clicked {
            let was = self.gallery.player.take().map(|p| p.stream);
            if was != Some(stream)
                && let Some(Ok(m)) = &self.gallery.music
            {
                match Player::start(m, stream) {
                    Ok(pl) => self.gallery.player = Some(pl),
                    Err(e) => self.gallery.note = Some((format!("Could not play it: {e}"), true)),
                }
            }
        }
        if let Some((text, bad)) = &self.gallery.note {
            self.fonts.text(
                &p,
                Role::Body,
                rect.center_bottom() - vec2(0.0, 6.0),
                Align2::CENTER_BOTTOM,
                13.0,
                if *bad { WARN } else { DIM },
                text,
            );
        }
    }

    /// Stops the music when the page is left.
    pub(super) fn gallery_left(&mut self) {
        self.gallery.player = None;
    }
}
