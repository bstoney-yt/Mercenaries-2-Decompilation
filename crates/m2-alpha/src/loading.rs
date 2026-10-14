//! The game's loading screen: the `loadingscreen_standalone` Flash movie
//! from `Loading.wad` (the spade-and-skull badge, a hint, "Loading" and
//! its spinner), up while the island loads at the start and over the
//! scripts' fades to black between contracts.
//!
//! The game's own GUI scripts (`MrxGui`) play it as their screen fade:
//! `GlobalFadeToBlack` loads the movie in a fullscreen Flash widget and
//! plays it; its wipe (sprite 122 from `_fadeOn`) sends `wipeComplete` on
//! its 60th frame, when the scripts call `skullSpin(true)` and
//! `Gui.ShowLoadingHints(true)`; the engine then hands the movie a hint
//! (`UpdateLoadingHint` → `textDisplay("[action]", "[hint]")`).
//! `GlobalFadeFromBlack` calls `fadeOut`, which plays the wipe back from
//! `_fadeOff` (frame 100) to the `close` command on frame 160. Here
//! [`Cycle`] decides the same steps from what the alpha knows (the island
//! loading, the mission's fade), and [`LoadingScreen`] runs the movie
//! with [`m2_assets::flash`] and draws it.
//!
//! The hints are 57 strings of the English table (`shell.wad`), stored
//! one after another; their ids are not known, so [`HINTS`] lists their
//! keys (hashes) in stored order.
//!
//! # Provenance
//!
//! - From the game's data: the movie, its frames, labels and commands
//!   (`Loading.wad`); the fade's steps (`MrxGui` in `vz.wad`); the hints'
//!   text and stored order (`shell.wad`).
//! - From the PC footage: a hint is up at the start (the first stored,
//!   then the second about 12 s later) and on contract changes (the third
//!   at 7:58, the sixth at 20:44), so the engine shows them in stored
//!   order, one more each time; [`HINT_SECONDS`] is between the 6 s the
//!   first stayed and the 12 s before the second.
//! - Invented: the order once the last is shown (back to the first), and
//!   which loads show a hint (all here; the footage's screen after a
//!   failed mission has none, which the engine's choice is not read for).
//!   Nothing from the executable.

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use egui::{Color32, Mesh, TextureHandle, TextureOptions, pos2};
use m2_assets::flash::{self, avm1};
use m2_assets::gfx::{MOVIE_TYPE, movie_from_asset};
use m2_assets::strings::{STRINGS_TYPE, StringTable};
use m2_assets::{BlockContents, WadArchive, name_hash};
use m2_engine::AssetSource;

use crate::multiply::{Multiply, Pixels, Vertex};

/// The loading screens' archive beside `vz.wad`.
pub const WAD: &str = "Loading.wad";
/// The movie the scripts' fade plays (`MrxGui._InitFadeFlash`).
pub const MOVIE: &str = "loadingscreen_standalone";
/// The font movie the loading archive carries for it.
const FONT_MOVIE: &str = "fonts_enext";
/// Its font.
const FONT_NAME: &str = "gunplay";
/// The font the movie's text fields import, as the font book keys it.
const NORMAL_FONT: &str = "_normal_font";

/// Frames the movie's wipe takes, each way: `_fadeOn` (1) to
/// `wipeComplete` (60), `_fadeOff` (100) to `close` (160).
pub const WIPE_FRAMES: u32 = 60;
/// Its frame rate.
pub const FRAME_RATE: f32 = 30.0;

/// Seconds each hint stays while the screen is up (from footage, see the
/// module docs).
pub const HINT_SECONDS: f32 = 10.0;

/// Seconds the island's loading keeps the screen up at most (invented: a
/// load that never ends must not hide the game for good).
pub const LOADING_CAP: f32 = 90.0;

/// The action line's text in the English table: its only string that
/// is "Loading", which the footage shows there (inferred: the engine's
/// `sAction` id is not known).
pub const ACTION: u32 = 0x3971_2df8;

/// The hints' keys in the English table, in stored order.
pub const HINTS: [u32; 57] = [
    0x6b7f_031f,
    0x698b_cbec,
    0x638e_0111,
    0x780e_fa0f,
    0x560c_85f2,
    0xd814_0e5d,
    0xee11_f268,
    0xf005_299b,
    0xce02_b57e,
    0x700a_7049,
    0x5608_08c4,
    0xf823_4b47,
    0xd620_d72a,
    0x0f5b_646a,
    0x315d_d887,
    0x075f_d500,
    0x3162_55b5,
    0x2750_fcd6,
    0x8953_d5b3,
    0xaf56_501c,
    0xa958_8541,
    0x8f47_1332,
    0xb149_874f,
    0x7df2_b165,
    0x13ef_cbf0,
    0xfded_6ab7,
    0x1beb_5b5a,
    0xf5e8_e0f1,
    0x7be5_e24c,
    0x95e3_cca3,
    0x73e1_5886,
    0x1dde_928d,
    0xf3dc_11d8,
    0xccee_f888,
    0x36f1_ddfd,
    0xb4ea_5592,
    0xd6ec_c9af,
    0x34e5_0ee4,
    0x4ee7_7669,
    0x2ce0_851e,
    0x4ee2_f93b,
    0x2d03_1760,
    0x5705_9815,
    0x3cc3_7a03,
    0x1ac1_05e6,
    0x9cc8_8e51,
    0xa2c6_592c,
    0x79d4_a013,
    0x97d2_90b6,
    0xa1e3_e995,
    0x77e1_68e0,
    0x21de_a2e7,
    0xffdc_2eca,
    0x99c5_c7e9,
    0x7fc3_6064,
    0x8b42_99c6,
    0xad45_0de3,
];

/// What the screen does next, for the movie.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Start the movie: its wipe to black plays. `covered`: the screen is
    /// black already (the start), so the wipe is skipped.
    Start { covered: bool },
    /// Show hint `n` (an index into [`HINTS`]).
    Hint(usize),
    /// Play the wipe back (`fadeOut`).
    FadeOut,
}

/// A screen up: seconds since it started and since its hint changed, and
/// whether it has shown one.
#[derive(Clone, Copy, Debug, Default)]
struct Up {
    age: f32,
    since: f32,
    hinted: bool,
}

/// When the screen is up and which hint it shows: the engine's side of
/// the fade, from whether a load wants the screen.
#[derive(Clone, Debug, Default)]
pub struct Cycle {
    /// The hint shown next (counts on across screens).
    next: usize,
    /// None once faded out.
    up: Option<Up>,
    /// The mission's fade as last seen, 0 to 1.
    black: f32,
    /// The fade was last seen going darker.
    rising: bool,
    /// Seconds the island has been loading.
    loading_for: f32,
    /// Screens started, for the log.
    pub shown: u32,
    /// Hints shown, for the log.
    pub hints: u32,
}

impl Cycle {
    /// Whether a load wants the screen, `dt` seconds on: the island still
    /// loading (for up to [`LOADING_CAP`] seconds at a time), or the
    /// mission's fade going to (or held at) black. Its fade back lets the
    /// screen go.
    pub fn wanted(&mut self, loading: bool, black: f32, dt: f32) -> bool {
        // Rising or held: an unchanged value keeps the way it was going.
        if black > self.black {
            self.rising = true;
        } else if black < self.black {
            self.rising = false;
        }
        let darker = black > 0.0 && (self.rising || black >= 1.0);
        self.black = black;
        self.loading_for = if loading { self.loading_for + dt } else { 0.0 };
        (loading && self.loading_for < LOADING_CAP) || darker
    }
    /// Up for `wanted`, `dt` seconds on: the steps for the movie.
    /// `first` says the screen is black already (nothing drawn yet).
    pub fn update(&mut self, wanted: bool, dt: f32, first: bool) -> Vec<Step> {
        let wipe = WIPE_FRAMES as f32 / FRAME_RATE;
        let mut steps = Vec::new();
        match (self.up, wanted) {
            (None, true) => {
                self.shown += 1;
                // At the start the screen is black already: no wipe, so
                // its hint comes at once.
                let age = if first { wipe } else { 0.0 };
                self.up = Some(Up {
                    age,
                    ..Up::default()
                });
                steps.push(Step::Start { covered: first });
                steps.extend(self.due());
            }
            (Some(mut up), true) => {
                up.age += dt;
                up.since += dt;
                self.up = Some(up);
                steps.extend(self.due());
            }
            (Some(_), false) => {
                self.up = None;
                steps.push(Step::FadeOut);
            }
            (None, false) => {}
        }
        steps
    }

    /// Whether the screen is up (not fading out).
    pub fn up(&self) -> bool {
        self.up.is_some()
    }

    /// A hint, if one is due: the first once the wipe is done
    /// (`wipeComplete`), then one every [`HINT_SECONDS`].
    fn due(&mut self) -> Option<Step> {
        let wipe = WIPE_FRAMES as f32 / FRAME_RATE;
        let up = self.up.as_mut()?;
        if up.age < wipe || (up.hinted && up.since < HINT_SECONDS) {
            return None;
        }
        up.hinted = true;
        up.since = 0.0;
        let n = self.next % HINTS.len();
        self.next = n + 1;
        self.hints += 1;
        Some(Step::Hint(n))
    }
}

/// The movie playing, with what it draws with.
pub struct LoadingScreen {
    lib: Rc<flash::Library>,
    /// Its font, shared with the movie (which measures its text).
    fonts: Rc<flash::FontBook>,
    /// The hints' text, in [`HINTS`] order (empty where missing).
    hints: Vec<String>,
    /// The action line's text ([`ACTION`]).
    action: String,
    /// The archives textures are read from: the loading one, then the
    /// game's.
    sources: Vec<AssetSource<BufReader<File>>>,
    textures: HashMap<u32, Option<(TextureHandle, Arc<Pixels>)>>,
    /// A white texel, for solid fills that multiply.
    white: Arc<Pixels>,
    /// Playing: None before the first screen and after `close`.
    rt: Option<avm1::Runtime>,
    pub cycle: Cycle,
    /// The input time last seen, for `dt`.
    time: Option<f64>,
    /// Lines for the log, taken by [`take_log`](Self::take_log).
    log: Vec<String>,
}

impl std::fmt::Debug for LoadingScreen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadingScreen")
            .field("up", &self.cycle.up())
            .finish_non_exhaustive()
    }
}

impl LoadingScreen {
    /// Reads the movie, its font and the hints from the install's `data`
    /// folder.
    pub fn open(data: &Path) -> Result<Self, String> {
        let open = |name: &str| {
            let path = data.join(name);
            File::open(&path)
                .map(BufReader::new)
                .map_err(|e| format!("{}: {e}", path.display()))
        };
        let mut loading = AssetSource::new(open(WAD)?).map_err(|e| e.to_string())?;
        let movie = |source: &mut AssetSource<BufReader<File>>, name: &str| {
            source
                .asset(name_hash(name), MOVIE_TYPE)
                .map_err(|e| format!("{name}: {e}"))
                .and_then(|d| movie_from_asset(&d).map_err(|e| format!("{name}: {e}")))
        };
        let lib = Rc::new(flash::Library::new(&movie(&mut loading, MOVIE)?));
        let mut fonts = flash::FontBook::default();
        fonts.add_movie(&movie(&mut loading, FONT_MOVIE)?);
        // The movie imports `_normal_Font`; the only font the loading
        // archive carries (its `Gunplay`) stands for it.
        if let Some(f) = fonts.get(FONT_NAME).cloned() {
            fonts.fonts.insert(NORMAL_FONT.to_owned(), f);
        }
        let strings = english(&data.join("shell.wad"))?;
        // "[]" leaves the movie's own action line if the text is missing.
        let action = strings
            .get(ACTION)
            .map_or_else(|| "[]".to_owned(), str::to_owned);
        let hints = HINTS
            .iter()
            .map(|&k| strings.get(k).unwrap_or_default().to_owned())
            .collect();
        let mut sources = vec![loading];
        for name in ["vz.wad", "English.wad"] {
            if let Ok(s) = open(name).and_then(|r| AssetSource::new(r).map_err(|e| e.to_string())) {
                sources.push(s);
            }
        }
        Ok(Self {
            lib,
            fonts: Rc::new(fonts),
            hints,
            action,
            sources,
            textures: HashMap::new(),
            white: Arc::new(Pixels {
                width: 1,
                height: 1,
                rgba: vec![255; 4],
            }),
            rt: None,
            cycle: Cycle::default(),
            time: None,
            log: Vec::new(),
        })
    }

    /// How many of the hints have text.
    pub fn hints_read(&self) -> usize {
        self.hints.iter().filter(|h| !h.is_empty()).count()
    }

    /// Runs the screen to `time` (seconds, the input's clock), up if
    /// the island is `loading` or the mission's fade (`black`, 0 to 1) wants it.
    pub fn update(&mut self, loading: bool, black: f32, time: f64) {
        let first = self.time.is_none();
        let dt = self.time.map_or(0.0, |t| (time - t).max(0.0) as f32);
        self.time = Some(time);
        let wanted = self.cycle.wanted(loading, black, dt);
        let why = format!("island loading {loading}, fade {black:.2}, t {time:.1}");
        for step in self.cycle.update(wanted, dt, first) {
            self.apply(step, &why);
        }

        let Some(rt) = &mut self.rt else { return };
        rt.step(dt);
        for (command, _) in std::mem::take(&mut rt.avm.commands) {
            let command = command.trim_start_matches("FSCommand:");
            if command == "close" {
                self.log.push(format!(
                    "loading screen: closed ({} shown, {} hints)",
                    self.cycle.shown, self.cycle.hints
                ));
                self.rt = None;
                break;
            }
        }
    }

    /// Does `step`; `why` (what wanted it) goes in the log.
    fn apply(&mut self, step: Step, why: &str) {
        match step {
            Step::Start { covered } => {
                let mut player = flash::Player::new(Rc::clone(&self.lib));
                player.fonts = Some(Rc::clone(&self.fonts));
                let mut rt = avm1::Runtime::new(player);
                if covered {
                    // Black already: the wipe's frames played at once.
                    for _ in 0..WIPE_FRAMES {
                        rt.step(1.0 / FRAME_RATE);
                    }
                }
                rt.avm.commands.clear();
                rt.call("skullSpin", &[avm1::Value::Bool(true)]);
                self.log.push(format!(
                    "loading screen: up{} ({why})",
                    if covered { " (covered)" } else { "" }
                ));
                self.rt = Some(rt);
            }
            Step::Hint(n) => {
                let text = self.hints.get(n).cloned().unwrap_or_default();
                self.log.push(format!(
                    "loading screen: hint {n} {:?}",
                    text.chars().take(40).collect::<String>()
                ));
                if let Some(rt) = &mut self.rt {
                    // The action line ("Loading") over the hint.
                    rt.call(
                        "textDisplay",
                        &[
                            avm1::Value::Str(self.action.as_str().into()),
                            avm1::Value::Str(text.into()),
                        ],
                    );
                }
            }
            Step::FadeOut => {
                self.log.push(format!("loading screen: fading out ({why})"));
                if let Some(rt) = &mut self.rt {
                    rt.call("fadeOut", &[]);
                }
            }
        }
    }

    /// Whether the movie is on screen (up or fading out).
    pub fn showing(&self) -> bool {
        self.rt.is_some()
    }

    /// Log lines since last asked.
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }

    /// Draws the movie over `screen`, its stage kept in shape and centred
    /// (Scaleform's "show all"), over black bars: true if it drew.
    pub fn paint(&mut self, ctx: &egui::Context, painter: &egui::Painter) -> bool {
        let Some(rt) = &self.rt else { return false };
        let screen = painter.clip_rect();
        let [mw, mh] = self.lib.size;
        if mw <= 0.0 || mh <= 0.0 {
            return false;
        }
        let scale = (screen.width() / mw).min(screen.height() / mh);
        let (w, h) = (mw * scale, mh * scale);
        let x = screen.min.x + (screen.width() - w) / 2.0;
        let y = screen.min.y + (screen.height() - h) / 2.0;
        let matrix = flash::Matrix {
            a: scale,
            b: 0.0,
            c: 0.0,
            d: scale,
            tx: x,
            ty: y,
        };
        let mut triangles = Vec::new();
        rt.player.draw(&matrix, &self.fonts, &mut triangles);
        // Letterbox bars, while the movie covers its stage.
        let covering = triangles.iter().any(|t| t.colour[3] > 0.0);
        if covering && (w < screen.width() - 0.5 || h < screen.height() - 0.5) {
            let stage = egui::Rect::from_min_size(pos2(x, y), egui::vec2(w, h));
            for bar in [
                egui::Rect::from_min_max(screen.min, pos2(screen.max.x, stage.min.y)),
                egui::Rect::from_min_max(pos2(screen.min.x, stage.max.y), screen.max),
                egui::Rect::from_min_max(screen.min, pos2(stage.min.x, screen.max.y)),
                egui::Rect::from_min_max(pos2(stage.max.x, screen.min.y), screen.max),
            ] {
                painter.rect_filled(bar, 0.0, Color32::BLACK);
            }
        }
        // In order; the colour overlay multiplies what is under it (its
        // `PlaceObject3` blend 3), which gives the badge and text their
        // colour.
        let clip = painter.clip_rect();
        let ndc = |c: [f32; 2]| {
            [
                (c[0] - clip.min.x) / clip.width().max(1.0) * 2.0 - 1.0,
                1.0 - (c[1] - clip.min.y) / clip.height().max(1.0) * 2.0,
            ]
        };
        let mut mesh = Mesh::default();
        let mut multiply: Option<(u64, Arc<Pixels>, Vec<Vertex>)> = None;
        let flush_multiply = |m: &mut Option<(u64, Arc<Pixels>, Vec<Vertex>)>| {
            if let Some((key, pixels, vertices)) = m.take() {
                painter.add(egui_wgpu::Callback::new_paint_callback(
                    clip,
                    Multiply::new((key, pixels), vertices),
                ));
            }
        };
        for t in &triangles {
            let texture = match &t.texture {
                Some(name) => match self.texture(ctx, name) {
                    Some(t) => Some(t),
                    None => continue,
                },
                None => None,
            };
            if t.multiply {
                if !mesh.is_empty() {
                    painter.add(std::mem::take(&mut mesh));
                }
                let (key, pixels) = match &texture {
                    Some((hash, _, pixels)) => (u64::from(*hash), Arc::clone(pixels)),
                    None => (u64::MAX, Arc::clone(&self.white)),
                };
                if multiply.as_ref().is_some_and(|(k, ..)| *k != key) {
                    flush_multiply(&mut multiply);
                }
                let (_, _, vertices) = multiply.get_or_insert_with(|| (key, pixels, Vec::new()));
                for (c, uv) in t.corners.iter().zip(&t.uv) {
                    vertices.push(Vertex {
                        pos: ndc(*c),
                        uv: if texture.is_some() { *uv } else { [0.5; 2] },
                        colour: t.colour.map(|c| c.clamp(0.0, 1.0)),
                    });
                }
                continue;
            }
            flush_multiply(&mut multiply);
            let id = texture
                .as_ref()
                .map_or(egui::TextureId::default(), |(_, id, _)| *id);
            if mesh.texture_id != id && !mesh.is_empty() {
                painter.add(std::mem::take(&mut mesh));
            }
            mesh.texture_id = id;
            let [r, g, b, a] = t.colour.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
            let colour = Color32::from_rgba_unmultiplied(r, g, b, a);
            let base = mesh.vertices.len() as u32;
            for (c, uv) in t.corners.iter().zip(&t.uv) {
                let uv = match texture {
                    Some(_) => pos2(uv[0], uv[1]),
                    None => egui::epaint::WHITE_UV,
                };
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: pos2(c[0], c[1]),
                    uv,
                    color: colour,
                });
            }
            mesh.indices.extend([base, base + 1, base + 2]);
        }
        if !mesh.is_empty() {
            painter.add(mesh);
        }
        flush_multiply(&mut multiply);
        true
    }

    /// Texture `name` from the archives, uploaded the first time: its
    /// hash, egui's id and its pixels (for multiplying).
    fn texture(
        &mut self,
        ctx: &egui::Context,
        name: &str,
    ) -> Option<(u32, egui::TextureId, Arc<Pixels>)> {
        let hash = name_hash(name);
        if let Some(t) = self.textures.get(&hash) {
            return t.as_ref().map(|(h, p)| (hash, h.id(), Arc::clone(p)));
        }
        let decoded = self
            .sources
            .iter_mut()
            .find_map(|s| s.texture(hash, 2048).ok());
        let loaded = decoded.and_then(|t| {
            let rgba = t.mips.into_iter().next()?;
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [t.width as usize, t.height as usize],
                &rgba,
            );
            let handle =
                ctx.load_texture(format!("loading {hash:08x}"), image, TextureOptions::LINEAR);
            let pixels = Arc::new(Pixels {
                width: t.width,
                height: t.height,
                rgba,
            });
            Some((handle, pixels))
        });
        if loaded.is_none() {
            self.log
                .push(format!("loading screen: no texture {name} ({hash:08x})"));
        }
        let found = loaded.as_ref().map(|(h, p)| (hash, h.id(), Arc::clone(p)));
        self.textures.insert(hash, loaded);
        found
    }
}

/// The English string table of the archive at `path`.
fn english(path: &Path) -> Result<StringTable, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut wad = WadArchive::open(BufReader::new(file)).map_err(|e| e.to_string())?;
    let want = name_hash("english");
    for b in 0..wad.index().blocks().len() {
        let Ok(decoded) = wad.read_block(b) else {
            continue;
        };
        let Ok(contents) = BlockContents::parse(&decoded) else {
            continue;
        };
        for (e, data) in contents.iter() {
            if e.type_hash == STRINGS_TYPE && e.name_hash == want {
                return StringTable::parse(data).map_err(|e| e.to_string());
            }
        }
    }
    Err(format!("{}: no English strings", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIPE: f32 = WIPE_FRAMES as f32 / FRAME_RATE;

    #[test]
    fn the_start_is_covered_and_shows_the_first_hint_at_once() {
        let mut c = Cycle::default();
        assert_eq!(
            c.update(true, 0.0, true),
            [Step::Start { covered: true }, Step::Hint(0)]
        );
        // The next after its time.
        assert!(c.update(true, HINT_SECONDS - 0.5, false).is_empty());
        assert_eq!(c.update(true, 1.0, false), [Step::Hint(1)]);
        assert_eq!(c.update(false, 0.1, false), [Step::FadeOut]);
        assert!(!c.up());
    }

    #[test]
    fn a_fade_shows_the_next_hint_once_its_wipe_is_done() {
        let mut c = Cycle::default();
        c.update(true, 0.0, true);
        c.update(false, 0.1, false);
        assert_eq!(c.update(true, 0.1, false), [Step::Start { covered: false }]);
        assert!(c.update(true, WIPE - 0.2, false).is_empty());
        assert_eq!(c.update(true, 0.3, false), [Step::Hint(1)]);
        assert!(c.update(true, 1.0, false).is_empty());
        assert_eq!((c.shown, c.hints), (2, 2));
    }

    #[test]
    fn hints_go_round_after_the_last() {
        let mut c = Cycle {
            next: HINTS.len() - 1,
            ..Cycle::default()
        };
        c.update(true, 0.0, true);
        assert_eq!(c.update(true, HINT_SECONDS, false), [Step::Hint(0)]);
    }

    #[test]
    fn the_mission_fade_wants_the_screen_until_it_lightens() {
        let mut c = Cycle::default();
        assert!(!c.wanted(false, 0.0, 0.1));
        assert!(c.wanted(false, 0.3, 0.1));
        assert!(c.wanted(false, 0.3, 0.1));
        assert!(c.wanted(false, 1.0, 0.1));
        assert!(c.wanted(false, 1.0, 0.1));
        assert!(!c.wanted(false, 0.8, 0.1));
        // Asked again with the same value, it stays let go.
        assert!(!c.wanted(false, 0.8, 0.1));
        assert!(!c.wanted(false, 0.0, 0.1));
        assert!(c.wanted(true, 0.0, 0.1));
    }

    #[test]
    fn a_load_that_never_ends_lets_the_screen_go() {
        let mut c = Cycle::default();
        assert!(c.wanted(true, 0.0, LOADING_CAP - 1.0));
        assert!(!c.wanted(true, 0.0, 1.0));
        // A new load starts the count again.
        assert!(!c.wanted(false, 0.0, 1.0));
        assert!(c.wanted(true, 0.0, 1.0));
    }

    #[test]
    fn the_wipe_is_two_seconds_each_way() {
        // `_fadeOn` 1 → `wipeComplete` 60; `_fadeOff` 100 → `close` 160.
        assert_eq!(WIPE, 2.0);
    }
}
