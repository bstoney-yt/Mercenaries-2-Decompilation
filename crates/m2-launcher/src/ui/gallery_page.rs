//! The Gallery tab: the game's music, read from the install on a thread
//! when the tab first opens, listed by group with Play and Stop
//! ([`crate::gallery`]). Cut-scenes come in a later change. Layout and
//! colours invented.

use std::sync::mpsc::Receiver;

use egui::{Align2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use super::{App, BAR, BG, DIM, FAINT, GREEN, LINE, PAD, TEXT, WARN};
use crate::assets::Role;
use crate::gallery::{self, Music, Player};
use crate::gallery_movies::{self, MovieItem, MoviePlayer};

/// The page's state between frames.
#[derive(Default)]
pub(super) struct GalleryPage {
    loading: Option<Receiver<Result<Music, String>>>,
    music: Option<Result<Music, String>>,
    player: Option<Player>,
    note: Option<(String, bool)>,
    /// Showing the cut-scenes rather than the music.
    movies: bool,
    movie_list: Option<Vec<MovieItem>>,
    movie: Option<MoviePlayer>,
    movie_tex: Option<egui::TextureHandle>,
}

impl std::fmt::Debug for GalleryPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GalleryPage")
            .field("movies", &self.movies)
            .finish_non_exhaustive()
    }
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
            "GALLERY",
        );
        // Music or cut-scenes.
        let mut x = top.left() + 150.0;
        for (movies, name) in [(false, "MUSIC"), (true, "CUT-SCENES")] {
            let w = self.fonts.width(&p, Role::Heading, 15.0, name) + 24.0;
            let chip = Rect::from_min_size(pos2(x, top.center().y - 14.0), vec2(w, 28.0));
            let r = ui.interact(
                chip,
                egui::Id::new(("gallery-kind", movies)),
                Sense::click(),
            );
            let on = self.gallery.movies == movies;
            let fill = if on {
                GREEN
            } else {
                egui::Color32::from_rgb(0x2c, 0x2c, 0x2c)
            };
            p.rect_filled(chip, 14.0, fill);
            self.fonts.text(
                &p,
                Role::Heading,
                chip.center(),
                Align2::CENTER_CENTER,
                15.0,
                TEXT,
                name,
            );
            if r.clicked() && !on {
                self.gallery.movies = movies;
                self.gallery.player = None;
                self.gallery.movie = None;
            }
            x += w + 8.0;
        }
        let note = if self.gallery.movies {
            match &self.gallery.movie_list {
                Some(l) => format!(
                    "{} movies, from your install. All unlocked: saves keep no record of cut-scenes seen.",
                    l.len()
                ),
                None => String::new(),
            }
        } else {
            match &self.gallery.music {
                None => "Reading the music from the install...".to_owned(),
                Some(Err(_)) => String::new(),
                Some(Ok(m)) => format!(
                    "{} pieces, from your install. All unlocked: saves keep no record of music heard.",
                    m.tracks.len()
                ),
            }
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
        if self.gallery.movies {
            self.gallery_movies(ui, list);
            return;
        }
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
        self.gallery.movie = None;
        self.gallery.movie_tex = None;
    }

    /// The cut-scenes: the list, or the movie playing in its place.
    fn gallery_movies(&mut self, ui: &mut Ui, list: Rect) {
        let ctx = ui.ctx().clone();
        let p = ui.painter_at(list);
        if self.gallery.movie_list.is_none() {
            self.gallery.movie_list = Some(
                self.install
                    .as_ref()
                    .map(|i| gallery_movies::list(&gallery_movies::movies_dir(i)))
                    .unwrap_or_default(),
            );
        }
        // Playing: the picture letterboxed over the list; a click or
        // Escape stops it.
        if let Some(m) = &mut self.gallery.movie {
            m.update();
            if let Some(frame) = m.take_frame() {
                let size = [m.info.width as usize, m.info.height as usize];
                let image = egui::ColorImage::from_rgba_unmultiplied(size, &frame);
                match &mut self.gallery.movie_tex {
                    Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                    None => {
                        let t =
                            ctx.load_texture("gallery-movie", image, egui::TextureOptions::LINEAR);
                        self.gallery.movie_tex = Some(t);
                    }
                }
            }
            p.rect_filled(list, 6.0, egui::Color32::BLACK);
            if let Some(t) = &self.gallery.movie_tex {
                let room = (list.width(), list.height() - 30.0);
                let (w, h) = gallery_movies::letterbox(room, m.info.width, m.info.height);
                let at = Rect::from_center_size(list.center() - vec2(0.0, 15.0), vec2(w, h));
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                p.image(t.id(), at, uv, egui::Color32::WHITE);
            }
            let line = format!(
                "{}   {} / {}   (click or Escape to stop)",
                m.name,
                gallery::length(m.elapsed()),
                gallery::length(m.info.seconds() as f32)
            );
            let foot = list.center_bottom() - vec2(0.0, 6.0);
            self.fonts.text(
                &p,
                Role::Body,
                foot,
                Align2::CENTER_BOTTOM,
                13.0,
                DIM,
                &line,
            );
            let r = ui.interact(list, egui::Id::new("gallery-movie"), Sense::click());
            let stop = r.clicked() || m.ended || ctx.input(|i| i.key_pressed(egui::Key::Escape));
            if stop {
                self.gallery.movie = None;
                self.gallery.movie_tex = None;
            } else {
                ctx.request_repaint();
            }
            return;
        }
        let ffmpeg = m2_engine::bink::ffmpeg();
        if ffmpeg.is_none() {
            self.fonts.text(
                &p,
                Role::Body,
                list.center_bottom() - vec2(0.0, 6.0),
                Align2::CENTER_BOTTOM,
                13.0,
                WARN,
                "Cut-scenes need ffmpeg: put ffmpeg.exe beside the launcher or on the PATH, \
                 or name it in M2_FFMPEG. Music plays without it.",
            );
        }
        let items = self.gallery.movie_list.clone().unwrap_or_default();
        let mut clicked: Option<String> = None;
        let rows = list.with_max_y(list.bottom() - 26.0);
        let mut child = ui.new_child(UiBuilder::new().max_rect(rows));
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                let mut group = "";
                for m in &items {
                    if m.group != group {
                        group = m.group;
                        let (row, _) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
                        let at = row.left_bottom() + vec2(4.0, -8.0);
                        let heading = group.to_uppercase();
                        self.fonts.text(
                            ui.painter(),
                            Role::Heading,
                            at,
                            Align2::LEFT_BOTTOM,
                            15.0,
                            GREEN,
                            &heading,
                        );
                    }
                    let (row, r) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                    let p = ui.painter().clone();
                    let fill = if r.hovered() {
                        egui::Color32::from_rgb(0x2c, 0x2c, 0x2c)
                    } else {
                        BAR
                    };
                    p.rect_filled(row.shrink2(vec2(0.0, 2.0)), 6.0, fill);
                    let c = row.left_center() + vec2(22.0, 0.0);
                    let tri = vec![
                        c + vec2(-4.0, -6.0),
                        c + vec2(6.0, 0.0),
                        c + vec2(-4.0, 6.0),
                    ];
                    let colour = if ffmpeg.is_some() { TEXT } else { FAINT };
                    p.add(egui::Shape::convex_polygon(tri, colour, Stroke::NONE));
                    let title = match m.hero {
                        Some(h) if !m.label.contains(h) => format!("{}, {h}", m.label),
                        _ => m.label.clone(),
                    };
                    let at = row.left_center() + vec2(44.0, -1.0);
                    self.fonts.text(
                        &p,
                        Role::BodyBold,
                        at,
                        Align2::LEFT_CENTER,
                        15.0,
                        TEXT,
                        &title,
                    );
                    let time = m
                        .info
                        .map_or_else(|| "?".to_owned(), |i| gallery::length(i.seconds() as f32));
                    let right = format!("{}.bik   {time}", m.name);
                    let at = row.right_center() - vec2(16.0, 0.0);
                    self.fonts
                        .text(&p, Role::Body, at, Align2::RIGHT_CENTER, 13.0, DIM, &right);
                    if r.clicked() {
                        clicked = Some(m.name.clone());
                    }
                }
            });
        if let (Some(name), Some(ff), Some(i)) = (clicked, ffmpeg, self.install.as_ref()) {
            let file = gallery_movies::movies_dir(i).join(format!("{name}.bik"));
            match MoviePlayer::open(&ff, &file) {
                Ok(m) => {
                    self.gallery.player = None;
                    self.gallery.movie = Some(m);
                    ctx.request_repaint();
                }
                Err(e) => self.gallery.note = Some((format!("Could not play {name}: {e}"), true)),
            }
        }
    }
}
