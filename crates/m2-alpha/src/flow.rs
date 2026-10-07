//! The launch as the game runs it: the boot movies, the front end, then
//! play, with the contracts' movies shown when the scripts ask for them.
//!
//! The boot movies are the shell bootstrap script's own list (`tMovies`:
//! `EA`, then `Pandemic`, each to its end; `ShellBootstrap::Init` plays
//! them letterboxed when `Sys.PlayIntroMovies` says so). The front end's
//! menus are the shell's Flash screens, which the alpha does not run yet:
//! in their place the main menu's backdrop movie (`shell_mainmenu`) loops
//! under a stand-in prompt. Play's movies (`01_AOA_J` before the first
//! contract, `02_AOB_J` after it) come from `WifMissionFlow`'s
//! `_PlayMovie` through [`MissionLink::take_movie`].

use std::collections::VecDeque;
use std::path::PathBuf;

use egui::{Align2, Color32, FontId, Rect, Sense, TextureHandle, TextureOptions, pos2};
use m2_engine::{Install, Mixer};
use winit::keyboard::KeyCode;

use crate::mission::MissionLink;
use crate::movie::Movie;

/// The shell bootstrap's `tMovies`, in order.
pub const BOOT_MOVIES: [&str; 2] = ["EA", "Pandemic"];
/// The front end's main menu backdrop.
pub const MENU_MOVIE: &str = "shell_mainmenu";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// The boot movies.
    Boot,
    /// The front end, waiting for a new game.
    FrontEnd,
    /// Playing (a script's movie may be up).
    Game,
}

pub struct Flow {
    pub stage: Stage,
    install: Install,
    ffmpeg: Option<PathBuf>,
    boot: VecDeque<&'static str>,
    movie: Option<Movie>,
    /// The movie up was asked for by the scripts (they wait for its end).
    scripted: bool,
    /// A key asked to skip the movie up, or to leave the front end.
    pressed: bool,
    texture: Option<TextureHandle>,
    /// The movie and frame number in the texture.
    shown: Option<(String, u64)>,
}

impl Flow {
    /// Starts with the boot movies, or (`boot` false) straight in play.
    pub fn new(install: Install, boot: bool) -> Self {
        let ffmpeg = crate::movie::ffmpeg();
        match &ffmpeg {
            Some(p) => eprintln!("movies: decoding with {}", p.display()),
            None => eprintln!(
                "movies: no ffmpeg (set M2_FFMPEG, or put ffmpeg.exe beside m2-alpha): they are skipped"
            ),
        }
        Self {
            stage: if boot { Stage::Boot } else { Stage::Game },
            install,
            ffmpeg,
            boot: if boot {
                BOOT_MOVIES.into_iter().collect()
            } else {
                VecDeque::new()
            },
            movie: None,
            scripted: false,
            pressed: false,
            texture: None,
            shown: None,
        }
    }

    /// Whether the game waits (no play, no scripts): before play, or while
    /// a movie is up.
    pub fn holds(&self) -> bool {
        self.stage != Stage::Game || self.movie.is_some()
    }

    /// Takes keys for itself (skipping a movie, leaving the front end).
    pub fn key(&mut self, code: KeyCode) {
        if matches!(
            code,
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space | KeyCode::Escape
        ) {
            self.pressed = true;
        }
    }

    fn open(&mut self, name: &str, mixer: &mut Mixer) -> bool {
        self.movie = None;
        let Some(ffmpeg) = &self.ffmpeg else {
            return false;
        };
        match Movie::open(ffmpeg, &self.install, name) {
            Ok(m) => {
                // The world's music and sounds hush under the movie.
                for c in [m2_engine::Category::Music, m2_engine::Category::World] {
                    mixer.fade(c, 0.0, 0.25);
                }
                self.movie = Some(m);
                true
            }
            Err(e) => {
                eprintln!("movie {name}: {e:#}");
                false
            }
        }
    }

    fn close(&mut self, mixer: &mut Mixer) {
        if let Some(m) = self.movie.take()
            && let Some(v) = m.voice
        {
            mixer.stop(v);
        }
        for c in [m2_engine::Category::Music, m2_engine::Category::World] {
            mixer.fade(c, 1.0, 0.5);
        }
    }

    /// One tick: moves the movies on and the stages along, and hands the
    /// scripts' movies requests and ends.
    pub fn tick(&mut self, mission: Option<&mut MissionLink>, mixer: &mut Mixer) {
        let pressed = std::mem::take(&mut self.pressed);
        match self.stage {
            Stage::Boot => {
                let over = self
                    .movie
                    .as_mut()
                    .is_none_or(|m| pressed || m.update(|p| mixer.play(p)));
                if over {
                    self.close(mixer);
                    while let Some(next) = self.boot.pop_front() {
                        if self.open(next, mixer) {
                            return;
                        }
                    }
                    self.stage = Stage::FrontEnd;
                    eprintln!("flow: front end");
                    if self.open(MENU_MOVIE, mixer)
                        && let Some(m) = &mut self.movie
                    {
                        m.looping = true;
                    }
                }
            }
            Stage::FrontEnd => {
                if pressed {
                    self.close(mixer);
                    self.stage = Stage::Game;
                    eprintln!("flow: new game");
                    return;
                }
                let over = self
                    .movie
                    .as_mut()
                    .is_some_and(|m| m.update(|p| mixer.play(p)));
                if over {
                    self.close(mixer);
                    if self.open(MENU_MOVIE, mixer)
                        && let Some(m) = &mut self.movie
                    {
                        m.looping = true;
                    }
                }
            }
            Stage::Game => {
                let Some(mission) = mission else { return };
                if self.movie.is_none() {
                    if let Some(name) = mission.take_movie() {
                        eprintln!("flow: the scripts play movie {name}");
                        self.scripted = true;
                        if !self.open(&name, mixer) {
                            self.scripted = false;
                            mission.movie_done();
                        }
                    }
                    return;
                }
                let over = self
                    .movie
                    .as_mut()
                    .is_some_and(|m| pressed || m.update(|p| mixer.play(p)));
                if over {
                    let name = self.movie.as_ref().map(|m| m.name.clone());
                    self.close(mixer);
                    if std::mem::take(&mut self.scripted) {
                        eprintln!("flow: movie {} over", name.unwrap_or_default());
                        mission.movie_done();
                    }
                }
            }
        }
    }

    /// Whether a screen of its own covers the game.
    pub fn covers(&self) -> bool {
        self.movie.is_some() || self.stage != Stage::Game
    }

    /// Draws the movie up (letterboxed on black) and, in the front end,
    /// its prompt.
    pub fn paint(&mut self, ui: &mut egui::Ui) {
        let screen = ui.ctx().content_rect();
        let painter = ui.painter_at(screen);
        painter.rect_filled(screen, 0.0, Color32::BLACK);
        ui.allocate_rect(screen, Sense::hover());
        if let Some(m) = &self.movie
            && let Some((n, rgba)) = &m.frame
        {
            let (w, h) = (m.info.width as usize, m.info.height as usize);
            let key = (m.name.clone(), *n);
            if self.shown.as_ref() != Some(&key) {
                self.shown = Some(key);
                let image = egui::ColorImage::from_rgba_unmultiplied([w, h], rgba);
                match &mut self.texture {
                    Some(t) if t.size() == [w, h] => t.set(image, TextureOptions::LINEAR),
                    _ => {
                        self.texture = Some(ui.ctx().load_texture(
                            "movie",
                            image,
                            TextureOptions::LINEAR,
                        ));
                    }
                }
            }
            if let Some(t) = &self.texture {
                // Letterboxed: the whole picture, as large as fits.
                let scale = (screen.width() / w as f32).min(screen.height() / h as f32);
                let size = egui::vec2(w as f32 * scale, h as f32 * scale);
                let rect = Rect::from_center_size(screen.center(), size);
                painter.image(
                    t.id(),
                    rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }
        if self.stage == Stage::FrontEnd {
            painter.text(
                pos2(screen.center().x, screen.bottom() - screen.height() * 0.12),
                Align2::CENTER_CENTER,
                "Press Enter to start",
                FontId::proportional((screen.height() * 0.035).max(14.0)),
                Color32::from_gray(230),
            );
        }
    }
}
