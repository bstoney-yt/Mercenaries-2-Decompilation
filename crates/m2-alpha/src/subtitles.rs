//! The movies' subtitles on screen, where the game's cinematic layout puts
//! them.
//!
//! The layout (`MrxGuiCinematic`'s) has two text widgets on the 640 x 480
//! virtual screen, in the game's `english_18` font, white, centred, in a
//! full-screen widget anchored at the middle: `Movie subtitle`, 70 to 570
//! across with its bottom kept at 427 (`HandleSubtitleUpdate` wraps the
//! text to the box and moves the box up by its height), and `Movie
//! supersubtitle`, 70 to 570 across from 48 down. [`m2_game::subtitles`]
//! times what they say. Text ids expand as the HUD expands them (colours
//! and `[n]` new lines included).

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;

use egui::{Color32, Mesh, TextureHandle, TextureOptions, pos2};
use m2_assets::font::FONT_TYPE;
use m2_assets::{Font, StringTable, name_hash};
use m2_engine::{AssetSource, Install};
use m2_game::gui_runtime::{FONT_LINES, Screen};
use m2_game::subtitles::{FRAME_SECONDS, Line, Track};

/// The cinematic layout's font.
const FONT: &str = "english_18";
/// The virtual screen.
const VIRTUAL: [f32; 2] = [640.0, 480.0];
/// The text boxes' sides, and where the bottom text ends and the top text
/// starts.
const LEFT: f32 = 70.0;
const RIGHT: f32 = 570.0;
const BOTTOM: f32 = 427.0;
const TOP: f32 = 48.0;
/// Widest a font texture is decoded.
const MAX_TEXTURE: u32 = 2048;

/// A glyph to draw, in screen pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Quad {
    pub texture: u32,
    /// `[x0, y0, x1, y1]`.
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    pub colour: [f32; 4],
}

/// Decoded RGBA8 pixels.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The font and text the subtitles are drawn with.
pub struct Captions {
    font: Font,
    text: Screen,
    images: HashMap<u32, Option<Image>>,
    textures: HashMap<u32, TextureHandle>,
}

impl Captions {
    /// Reads the font and its pages from the install's `vz.wad`; texts
    /// expand with `strings`.
    pub fn load(install: &Install, strings: StringTable) -> Result<Self, String> {
        let wad = install.data.join("vz.wad");
        let file = File::open(&wad).map_err(|e| format!("{}: {e}", wad.display()))?;
        let mut assets = AssetSource::new(BufReader::new(file)).map_err(|e| e.to_string())?;
        let font = assets
            .asset(name_hash(FONT), FONT_TYPE)
            .map_err(|e| e.to_string())
            .and_then(|a| Font::parse(&a).map_err(|e| e.to_string()))
            .map_err(|e| format!("font {FONT}: {e}"))?;
        let mut images = HashMap::new();
        for m in &font.materials {
            let image = match assets.texture(m.texture, MAX_TEXTURE) {
                Ok(t) => t.mips.into_iter().next().map(|rgba| Image {
                    width: t.width,
                    height: t.height,
                    rgba,
                }),
                Err(e) => {
                    eprintln!("subtitles: font page {:08x}: {e}", m.texture);
                    None
                }
            };
            images.insert(m.texture, image);
        }
        let mut text = Screen::default();
        text.set_strings(strings);
        Ok(Self {
            font,
            text,
            images,
            textures: HashMap::new(),
        })
    }

    /// A text as the game shows it, without colours: for the log.
    pub fn plain(&self, text: &str) -> String {
        self.text.display_text(text)
    }

    /// The font's page `hash`, decoded.
    pub fn image(&self, hash: u32) -> Option<&Image> {
        self.images.get(&hash).and_then(Option::as_ref)
    }

    /// The glyphs of the texts up on a `size` screen (pixels).
    pub fn layout(&self, bottom: Option<&str>, top: Option<&str>, size: [f32; 2]) -> Vec<Quad> {
        let mut out = Vec::new();
        if let Some(t) = bottom {
            self.place(t, Place::Bottom(BOTTOM), size, &mut out);
        }
        if let Some(t) = top {
            self.place(t, Place::Top(TOP), size, &mut out);
        }
        out
    }

    fn place(&self, text: &str, at: Place, [w, h]: [f32; 2], out: &mut Vec<Quad>) {
        // Screen pixels per virtual unit, the full-screen widget kept at
        // the middle; the bitmap font is drawn for 720 lines.
        let scale = h / VIRTUAL[1];
        let x0 = w / 2.0 - VIRTUAL[0] / 2.0 * scale;
        let size = h / FONT_LINES;
        let chars: Vec<(char, [f32; 3])> = self
            .text
            .spans(text)
            .into_iter()
            .flat_map(|s| {
                let c = s.colour.unwrap_or([1.0; 3]);
                s.text.chars().map(move |ch| (ch, c)).collect::<Vec<_>>()
            })
            .collect();
        let width = |line: &[(char, [f32; 3])]| -> f32 {
            line.iter()
                .map(|(c, _)| {
                    let mut b = [0u16; 2];
                    c.encode_utf16(&mut b)
                        .iter()
                        .map(|&u| self.font.advance(u))
                        .sum::<f32>()
                })
                .sum()
        };
        // Font pixels a line may take.
        let room = (RIGHT - LEFT) * FONT_LINES / VIRTUAL[1];
        let lines = wrap(&chars, room, width);
        let line_height = f32::from(self.font.line_height) * size;
        let mut y = match at {
            Place::Bottom(b) => b * scale - lines.len() as f32 * line_height,
            Place::Top(t) => t * scale,
        };
        let centre = x0 + (LEFT + RIGHT) / 2.0 * scale;
        for line in &lines {
            let mut x = centre - width(line) * size / 2.0;
            for &(ch, [r, g, b]) in line {
                let mut units = [0u16; 2];
                for &u in ch.encode_utf16(&mut units).iter() {
                    if let Some(gl) = self.font.glyph(u)
                        && let Some(m) = self.font.materials.get(usize::from(gl.material))
                    {
                        let top = y + f32::from(gl.y_offset) * size;
                        out.push(Quad {
                            texture: m.texture,
                            rect: [
                                x,
                                top,
                                x + f32::from(gl.width) * size,
                                top + f32::from(gl.height) * size,
                            ],
                            uv: gl.uv,
                            colour: [r, g, b, 1.0],
                        });
                    }
                    x += self.font.advance(u) * size;
                }
            }
            y += line_height;
        }
    }

    /// Paints the texts up over the screen.
    pub fn paint(&mut self, painter: &egui::Painter, bottom: Option<&str>, top: Option<&str>) {
        let screen = painter.clip_rect();
        let quads = self.layout(bottom, top, [screen.width(), screen.height()]);
        let mut mesh = Mesh::default();
        for q in quads {
            let Some(id) = self.texture(painter.ctx(), q.texture) else {
                continue;
            };
            if mesh.texture_id != id && !mesh.is_empty() {
                painter.add(std::mem::take(&mut mesh));
            }
            mesh.texture_id = id;
            let [r, g, b, a] = q.colour.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
            let colour = Color32::from_rgba_unmultiplied(r, g, b, a);
            let [x0, y0, x1, y1] = q.rect;
            let [u0, v0, u1, v1] = q.uv;
            let base = mesh.vertices.len() as u32;
            for (x, y, u, v) in [
                (x0, y0, u0, v0),
                (x1, y0, u1, v0),
                (x1, y1, u1, v1),
                (x0, y1, u0, v1),
            ] {
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: pos2(screen.left() + x, screen.top() + y),
                    uv: pos2(u, v),
                    color: colour,
                });
            }
            mesh.indices.extend([0, 1, 2, 0, 2, 3].map(|i| base + i));
        }
        if !mesh.is_empty() {
            painter.add(mesh);
        }
    }

    /// Draws the texts up into an RGBA8 picture of `width` x `height`
    /// (nearest texel, blended by its alpha): for offscreen pictures.
    pub fn composite(
        &self,
        rgba: &mut [u8],
        width: u32,
        height: u32,
        bottom: Option<&str>,
        top: Option<&str>,
    ) {
        let (w, h) = (width as usize, height as usize);
        for q in self.layout(bottom, top, [width as f32, height as f32]) {
            let Some(img) = self.image(q.texture) else {
                continue;
            };
            let [x0, y0, x1, y1] = q.rect;
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            let px = |v: f32, max: usize| (v.max(0.0) as usize).min(max);
            for y in px(y0.floor(), h)..px(y1.ceil(), h) {
                for x in px(x0.floor(), w)..px(x1.ceil(), w) {
                    let fx = (x as f32 + 0.5 - x0) / (x1 - x0);
                    let fy = (y as f32 + 0.5 - y0) / (y1 - y0);
                    let u = q.uv[0] + (q.uv[2] - q.uv[0]) * fx;
                    let v = q.uv[1] + (q.uv[3] - q.uv[1]) * fy;
                    let tx = ((u * img.width as f32) as usize).min(img.width as usize - 1);
                    let ty = ((v * img.height as f32) as usize).min(img.height as usize - 1);
                    let t = &img.rgba[(ty * img.width as usize + tx) * 4..][..4];
                    let a = f32::from(t[3]) / 255.0 * q.colour[3];
                    let d = &mut rgba[(y * w + x) * 4..][..3];
                    for c in 0..3 {
                        let src = f32::from(t[c]) * q.colour[c];
                        d[c] = (f32::from(d[c]) * (1.0 - a) + src * a).round() as u8;
                    }
                }
            }
        }
    }

    fn texture(&mut self, ctx: &egui::Context, hash: u32) -> Option<egui::TextureId> {
        if let Some(t) = self.textures.get(&hash) {
            return Some(t.id());
        }
        let i = self.images.get(&hash)?.as_ref()?;
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [i.width as usize, i.height as usize],
            &i.rgba,
        );
        let t = ctx.load_texture(
            format!("subtitles {hash:08x}"),
            image,
            TextureOptions::LINEAR,
        );
        let id = t.id();
        self.textures.insert(hash, t);
        Some(id)
    }
}

#[derive(Clone, Copy)]
enum Place {
    /// The text's last line ends here.
    Bottom(f32),
    /// The text's first line starts here.
    Top(f32),
}

/// Breaks `chars` into lines no wider than `room` (as `width` measures),
/// at its new lines and between words; a word wider than a line has one
/// of its own.
fn wrap<T: Copy>(
    chars: &[(char, T)],
    room: f32,
    width: impl Fn(&[(char, T)]) -> f32,
) -> Vec<Vec<(char, T)>> {
    let mut lines = Vec::new();
    for para in chars.split(|(c, _)| *c == '\n') {
        let mut line: Vec<(char, T)> = Vec::new();
        for word in para.split(|(c, _)| *c == ' ') {
            if word.is_empty() {
                continue;
            }
            if line.is_empty() {
                line.extend_from_slice(word);
                continue;
            }
            let mut longer = line.clone();
            longer.push((' ', word[0].1));
            longer.extend_from_slice(word);
            if width(&longer) <= room {
                line = longer;
            } else {
                lines.push(std::mem::replace(&mut line, word.to_vec()));
            }
        }
        lines.push(line);
    }
    lines
}

/// A movie's subtitles as it plays.
pub struct MovieSubtitles {
    track: Track,
    /// The movie's time at the last update.
    last: f64,
}

impl MovieSubtitles {
    /// The `lines` to show, all of them or (`enabled` false) the captions
    /// only; `None` if nothing would show.
    pub fn new(lines: Vec<Line>, enabled: bool) -> Option<Self> {
        let track = Track::new(lines, enabled);
        (!track.is_empty()).then_some(Self { track, last: 0.0 })
    }

    /// The movie is at `frame` (from its first, 0).
    pub fn update(&mut self, frame: u64) {
        let now = frame as f64 * FRAME_SECONDS;
        let dt = (now - self.last).max(0.0);
        self.last = now;
        self.track.update(now, dt);
    }

    pub fn bottom(&self) -> Option<&str> {
        self.track.bottom()
    }

    pub fn top(&self) -> Option<&str> {
        self.track.top()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<(char, ())> {
        s.chars().map(|c| (c, ())).collect()
    }

    fn text(lines: &[Vec<(char, ())>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.iter().map(|(c, _)| c).collect())
            .collect()
    }

    #[test]
    fn text_wraps_between_words_and_at_new_lines() {
        let width = |l: &[(char, ())]| l.len() as f32;
        let lines = wrap(&chars("one two three\nfour"), 7.0, width);
        assert_eq!(text(&lines), ["one two", "three", "four"]);
        let lines = wrap(&chars("enormousword a"), 4.0, width);
        assert_eq!(text(&lines), ["enormousword", "a"]);
    }

    #[test]
    fn a_blank_line_stays() {
        let width = |l: &[(char, ())]| l.len() as f32;
        let lines = wrap(&chars("a\n\nb"), 10.0, width);
        assert_eq!(text(&lines), ["a", "", "b"]);
    }
}
