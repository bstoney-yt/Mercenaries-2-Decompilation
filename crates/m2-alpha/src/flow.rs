//! The launch as the game runs it: the boot movies, the front end, then
//! play, with the contracts' movies shown when the scripts ask for them.
//!
//! The boot movies are the shell bootstrap script's own list (`tMovies`:
//! `EA`, then `Pandemic`, each to its end; `ShellBootstrap::Init` plays
//! them letterboxed when `Sys.PlayIntroMovies` says so). The front end's
//! menus are the shell's Flash screens, which the alpha does not run yet:
//! in their place the main menu's backdrop movie loops under a stand-in
//! prompt, where left and right pick the hero (the shell's own
//! `shell_mainmenu_<hero>` backdrops, else `shell_mainmenu`) and Enter
//! starts as them. Play's movies (`01_AOA_J` before the first
//! contract, `02_AOB_J` after it) come from `WifMissionFlow`'s
//! `_PlayMovie` through [`MissionLink::take_movie`], with their subtitles
//! when the scripts ask for them (see [`crate::subtitles`]).

use std::collections::VecDeque;
use std::path::PathBuf;

use egui::{Align2, Color32, FontId, Rect, Sense, TextureHandle, TextureOptions, pos2};
use m2_engine::{Install, Mixer};
use winit::keyboard::KeyCode;

use crate::hero::Hero;
use crate::mission::MissionLink;
use crate::movie::Movie;
use crate::subtitles::{Captions, MovieSubtitles};

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
    /// The hero the front end has up.
    hero: Hero,
    /// The front end's left or right since the last tick: -1 or 1.
    step: i32,
    /// The hero a new game was started as, not yet taken.
    picked: Option<Hero>,
    /// The player has subtitles on.
    subtitles_on: bool,
    /// The scripted movie's subtitles, if it has any.
    subtitles: Option<MovieSubtitles>,
    /// What they are drawn with: not read yet, or read (None if they could
    /// not be).
    captions: Option<Option<Captions>>,
}

impl Flow {
    /// Starts with the boot movies, or (`boot` false) straight in play;
    /// the front end offers `hero` first; `subtitles` is the player's
    /// option.
    pub fn new(install: Install, boot: bool, hero: Hero, subtitles: bool) -> Self {
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
            hero,
            step: 0,
            picked: None,
            subtitles_on: subtitles,
            subtitles: None,
            captions: None,
        }
    }

    /// Whether the game waits (no play, no scripts): before play, or while
    /// a movie is up.
    pub fn holds(&self) -> bool {
        self.stage != Stage::Game || self.movie.is_some()
    }

    /// The hero a new game was started as in the front end, once.
    pub fn take_pick(&mut self) -> Option<Hero> {
        self.picked.take()
    }

    /// Takes keys for itself (skipping a movie, picking the hero, leaving
    /// the front end).
    pub fn key(&mut self, code: KeyCode) {
        if self.stage == Stage::FrontEnd {
            match code {
                KeyCode::ArrowLeft | KeyCode::KeyA => self.step = -1,
                KeyCode::ArrowRight | KeyCode::KeyD => self.step = 1,
                _ => {}
            }
        }
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

    /// Loops the front end's backdrop for the hero up: theirs, else the
    /// shared one.
    fn open_menu(&mut self, mixer: &mut Mixer) {
        let own = format!("{MENU_MOVIE}_{}", self.hero.name().to_ascii_lowercase());
        let opened = self.open(&own, mixer) || self.open(MENU_MOVIE, mixer);
        if opened && let Some(m) = &mut self.movie {
            m.looping = true;
        }
    }

    fn close(&mut self, mixer: &mut Mixer) {
        self.subtitles = None;
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
                    self.open_menu(mixer);
                }
            }
            Stage::FrontEnd => {
                if pressed {
                    self.close(mixer);
                    self.stage = Stage::Game;
                    self.picked = Some(self.hero);
                    eprintln!("flow: new game as {}", self.hero);
                    return;
                }
                let step = std::mem::take(&mut self.step);
                if step != 0 {
                    let n = Hero::ALL.len() as i32;
                    let at = Hero::ALL.iter().position(|&h| h == self.hero).unwrap_or(0) as i32;
                    self.hero = Hero::ALL[(at + step).rem_euclid(n) as usize];
                    eprintln!("flow: front end offers {}", self.hero);
                    self.close(mixer);
                    self.open_menu(mixer);
                    return;
                }
                let over = self
                    .movie
                    .as_mut()
                    .is_some_and(|m| m.update(|p| mixer.play(p)));
                if over {
                    self.close(mixer);
                    self.open_menu(mixer);
                }
            }
            Stage::Game => {
                let Some(mission) = mission else { return };
                if self.movie.is_none() {
                    if let Some((name, lines)) = mission.take_movie() {
                        eprintln!("flow: the scripts play movie {name}");
                        self.scripted = true;
                        if !self.open(&name, mixer) {
                            self.scripted = false;
                            mission.movie_done();
                            return;
                        }
                        if let Some(lines) = lines {
                            if self.captions.is_none() {
                                let strings = mission.strings().cloned().unwrap_or_default();
                                self.captions = Some(
                                    Captions::load(&self.install, strings)
                                        .map_err(|e| eprintln!("subtitles: {e}"))
                                        .ok(),
                                );
                            }
                            self.subtitles = MovieSubtitles::new(lines, self.subtitles_on);
                        }
                    }
                    return;
                }
                let over = self
                    .movie
                    .as_mut()
                    .is_some_and(|m| pressed || m.update(|p| mixer.play(p)));
                if let (Some(s), Some(m)) = (&mut self.subtitles, &self.movie)
                    && let Some((frame, _)) = &m.frame
                {
                    let (was_bottom, was_top) =
                        (s.bottom().map(str::to_owned), s.top().map(str::to_owned));
                    s.update(*frame);
                    if let Some(Some(c)) = &self.captions {
                        for (now, was) in [(s.bottom(), was_bottom), (s.top(), was_top)] {
                            if let Some(t) = now.filter(|t| was.as_deref() != Some(*t)) {
                                eprintln!(
                                    "subtitles: frame {frame}: {}",
                                    c.plain(t).replace('\n', " / ")
                                );
                            }
                        }
                    }
                }
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
        if let (Some(s), Some(Some(c))) = (&self.subtitles, &mut self.captions) {
            c.paint(&painter, s.bottom(), s.top());
        }
        if self.stage == Stage::FrontEnd {
            let size = (screen.height() * 0.035).max(14.0);
            painter.text(
                pos2(screen.center().x, screen.bottom() - screen.height() * 0.18),
                Align2::CENTER_CENTER,
                format!("<  {}  >", self.hero),
                FontId::proportional(size * 1.2),
                Color32::WHITE,
            );
            painter.text(
                pos2(screen.center().x, screen.bottom() - screen.height() * 0.12),
                Align2::CENTER_CENTER,
                "Left/Right to choose, Enter to start",
                FontId::proportional(size),
                Color32::from_gray(230),
            );
        }
    }
}
