//! The launch as the game runs it: the boot movies, the front end, then
//! play, with the contracts' movies shown when the scripts ask for them.
//!
//! The boot movies are the shell bootstrap script's own list (`tMovies`:
//! `EA`, then `Pandemic`, each to its end; `ShellBootstrap::Init` plays
//! them letterboxed when `Sys.PlayIntroMovies` says so). The front end is
//! the shell's own scripts and Flash menus (`shell.gfx`), run as the HUD
//! is ([`GameHud::start_shell`]): every key goes to them, the backdrop
//! movie they ask for plays in their movie widget's box under them, and a
//! new game they start starts play as the character they chose. Until they
//! are up (or if they cannot start), the main menu's backdrop loops under
//! a stand-in prompt, where left and right pick the hero (the shell's own
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

use crate::gamehud::{GameHud, HudState};
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
    /// The shell's menus, in the front end.
    shell: Option<GameHud>,
    /// Keys for them since the last frame (Flash's codes, down or up).
    shell_keys: Vec<(i32, bool)>,
    /// The box the backdrop up plays in, when the shell put it there.
    backdrop: Option<[f32; 4]>,
    /// How the window is shown and the monitor's modes, for the shell's
    /// video screen ([`Flow::window`]).
    window: Option<(crate::pause::Video, Vec<crate::pause::Mode>)>,
    /// The Resolution and Refresh Rate that screen kept, not yet taken.
    video: Option<crate::pause::Video>,
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
            shell: None,
            shell_keys: Vec::new(),
            backdrop: None,
            window: None,
            video: None,
        }
    }

    /// Whether the game waits (no play, no scripts): before play, or while
    /// a movie is up.
    pub fn holds(&self) -> bool {
        self.stage != Stage::Game || self.movie.is_some()
    }

    /// Whether the front end's menus want the window's modes
    /// ([`Flow::window`]): up, and not yet told.
    pub fn wants_window(&self) -> bool {
        self.stage == Stage::FrontEnd && self.window.is_none()
    }

    /// How the window is shown (`video`) and the modes its monitor offers,
    /// for the front end's video screen.
    pub fn window(&mut self, video: crate::pause::Video, modes: Option<Vec<crate::pause::Mode>>) {
        let modes = modes
            .or_else(|| self.window.take().map(|(_, m)| m))
            .unwrap_or_default();
        self.window = Some((video, modes));
    }

    /// The Resolution and Refresh Rate the front end's video screen kept,
    /// once.
    pub fn take_video(&mut self) -> Option<crate::pause::Video> {
        self.video.take()
    }

    /// The hero a new game was started as in the front end, once.
    pub fn take_pick(&mut self) -> Option<Hero> {
        self.picked.take()
    }

    /// Takes keys for itself (skipping a movie, picking the hero, leaving
    /// the front end).
    pub fn key(&mut self, code: KeyCode) {
        if self.stage == Stage::FrontEnd && self.shell_up() {
            if let Some(c) = flash_code(code) {
                self.shell_keys.extend([(c, true), (c, false)]);
            }
            return;
        }
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

    /// The shell's menus are up and take the keys.
    fn shell_up(&self) -> bool {
        self.shell.as_ref().is_some_and(GameHud::ready)
    }

    fn open(&mut self, name: &str, mixer: &mut Mixer) -> bool {
        self.movie = None;
        self.backdrop = None;
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
        self.backdrop = None;
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
                    self.shell = Some(GameHud::start_shell(&self.install));
                    self.open_menu(mixer);
                }
            }
            Stage::FrontEnd => {
                if self.shell.as_ref().is_some_and(GameHud::failed) {
                    eprintln!("flow: the shell's menus could not start; the stand-in stays");
                    self.shell = None;
                }
                if let Some(shell) = &mut self.shell
                    && shell.ready()
                {
                    if let Some(v) = shell.take_video() {
                        self.video = Some(v);
                    }
                    if let Some(who) = shell.take_new_game() {
                        // The scripts have left their menus: a character
                        // naming no hero (logged) starts as the stand-in's
                        // hero (the settings', unless changed there).
                        self.hero = crate::shell::hero(&who).unwrap_or(self.hero);
                        self.shell = None;
                        self.close(mixer);
                        self.stage = Stage::Game;
                        self.picked = Some(self.hero);
                        eprintln!("flow: the shell starts a new game as {} ({who})", self.hero);
                        return;
                    }
                    let wanted = shell.shell().movie.clone();
                    let up = self.movie.as_ref().map(|m| m.name.clone());
                    match wanted {
                        Some((name, rect)) => {
                            if up.as_deref() != Some(name.as_str()) || self.backdrop.is_none() {
                                eprintln!("flow: the shell plays {name}");
                                self.close(mixer);
                                if self.open(&name, mixer)
                                    && let Some(m) = &mut self.movie
                                {
                                    m.looping = true;
                                }
                            }
                            self.backdrop = Some(rect);
                        }
                        None if self.backdrop.is_some() => self.close(mixer),
                        None => {}
                    }
                    if let Some(m) = &mut self.movie
                        && m.update(|p| mixer.play(p))
                    {
                        self.close(mixer);
                    }
                    return;
                }
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

    /// Whether the front end is up (or the game is yet to start): its
    /// menus, where the mouse pointer shows.
    pub fn in_menus(&self) -> bool {
        self.stage != Stage::Game
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
        let shell = self.stage == Stage::FrontEnd && self.shell_up();
        if shell && let Some(s) = &mut self.shell {
            let dt = s.frame_time(Some(ui.ctx().input(|i| i.time)));
            let mut state = HudState {
                size: [screen.width(), screen.height()],
                dt,
                keys: std::mem::take(&mut self.shell_keys),
                ..HudState::default()
            };
            if let Some((video, modes)) = &self.window {
                state.pause_settings.video = *video;
                state.pause_settings.modes = modes.clone();
            }
            s.update(ui.ctx(), state);
        } else if let Some(s) = &mut self.shell {
            // Loading: told the size, so it draws once it is up.
            let state = HudState {
                size: [screen.width(), screen.height()],
                ..HudState::default()
            };
            s.update(ui.ctx(), state);
        }
        // What the shell draws under its movie widget, for the backdrop to
        // go over.
        if shell && let Some(s) = &self.shell {
            s.paint_under(&painter);
        }
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
                // In the shell's movie widget, filling its box; else
                // letterboxed: the whole picture, as large as fits.
                let rect = match self.backdrop {
                    Some([x0, y0, x1, y1]) => Rect::from_min_max(pos2(x0, y0), pos2(x1, y1)),
                    None => {
                        let scale = (screen.width() / w as f32).min(screen.height() / h as f32);
                        let size = egui::vec2(w as f32 * scale, h as f32 * scale);
                        Rect::from_center_size(screen.center(), size)
                    }
                };
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
        if shell && let Some(s) = &self.shell {
            s.paint_over(&painter);
        } else if self.stage == Stage::FrontEnd {
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

/// Flash's key code (the PC's virtual-key code) of `code`, for the
/// shell's menus: letters, digits and the keys menus are driven with.
fn flash_code(code: KeyCode) -> Option<i32> {
    use KeyCode::*;
    let letters = [
        KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO,
        KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
    ];
    let digits = [
        Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
    ];
    if let Some(i) = letters.iter().position(|&k| k == code) {
        return Some(i32::from(b'A') + i as i32);
    }
    if let Some(i) = digits.iter().position(|&k| k == code) {
        return Some(i32::from(b'0') + i as i32);
    }
    Some(match code {
        Backspace => 8,
        Tab => 9,
        Enter | NumpadEnter => 13,
        Escape => 27,
        Space => 32,
        PageUp => 33,
        PageDown => 34,
        End => 35,
        Home => 36,
        ArrowLeft => 37,
        ArrowUp => 38,
        ArrowRight => 39,
        ArrowDown => 40,
        Insert => 45,
        Delete => 46,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_go_to_the_menus_as_flash_codes() {
        assert_eq!(flash_code(KeyCode::KeyA), Some(65));
        assert_eq!(flash_code(KeyCode::Digit7), Some(55));
        assert_eq!(flash_code(KeyCode::Enter), Some(13));
        assert_eq!(flash_code(KeyCode::ArrowRight), Some(39));
        assert_eq!(flash_code(KeyCode::F12), None);
    }
}
