//! Text fields and Scaleform's texture fonts.
//!
//! A text field (`DefineEditText`) has a box, a font (by character id) and
//! height, a colour, a layout (alignment, margins, leading), flags (word
//! wrap, multiline, HTML) and the text it starts with.
//!
//! The game's fonts are Scaleform texture fonts: each font movie
//! (`_normal_font.gfx`, `_bold_font.gfx`...) defines the font
//! (`DefineFont3`: its glyphs' character codes, advances, ascent and
//! descent, in units of 1/20480 of the em) with its glyphs already drawn
//! into a texture, which `FontTextureInfo` (Scaleform tag 1002) names and
//! maps: each texture glyph's box and origin (texture coordinates), and for
//! each font which texture glyph each of its glyphs is. A movie that uses
//! a font imports it by name from the font library (`gfxfontlib.swf`'s
//! `_normal_Font`...); a [`FontBook`] finds it by that name.
//!
//! [`layout`] sets a text field's text as textured quads: HTML tags taken
//! out (a `<font color>` colours the text), lines broken where the text
//! breaks and, with word wrap, where the box ends, each line placed by the
//! field's alignment.
//!
//! # Provenance
//!
//! `DefineEditText` and `DefineFont3` from the public SWF specification
//! (version 10); `FontTextureInfo` from the published list of Scaleform GFx
//! extension tags (its layout confirmed by its length on the game's font
//! movies: header, 24 bytes a texture glyph, then the fonts' glyph maps).

use std::collections::HashMap;

use super::{Bits, ColourTransform, Matrix, Triangle, push_clipped};

/// Units of the em in a `DefineFont3` font's metrics.
const EM: f32 = 20480.0;
/// Twips to pixels.
const TWIPS: f32 = 20.0;

/// A text field's definition.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditText {
    /// x0, y0, x1, y1 (pixels).
    pub bounds: [f32; 4],
    pub font: Option<u16>,
    /// The font's height, pixels.
    pub height: f32,
    pub colour: [f32; 4],
    /// 0 left, 1 right, 2 centre, 3 justified.
    pub align: u8,
    pub left_margin: f32,
    pub right_margin: f32,
    pub leading: f32,
    pub word_wrap: bool,
    pub multiline: bool,
    pub html: bool,
    /// The variable it shows (scripts set it by that name).
    pub variable: String,
    pub initial: String,
}

impl EditText {
    /// Parses a `DefineEditText` tag's body.
    pub fn parse(data: &[u8]) -> Option<(u16, EditText)> {
        if data.len() < 6 {
            return None;
        }
        let mut b = Bits::new(data);
        let id = b.u16();
        let bounds = b.rect();
        let f1 = b.u8();
        let f2 = b.u8();
        let mut t = EditText {
            bounds,
            colour: [1.0; 4],
            height: 12.0,
            word_wrap: f1 & 0x40 != 0,
            multiline: f1 & 0x20 != 0,
            html: f2 & 0x02 != 0,
            ..EditText::default()
        };
        if f1 & 0x01 != 0 {
            t.font = Some(b.u16());
        }
        if f2 & 0x80 != 0 {
            b.cstr();
        }
        if f1 & 0x01 != 0 {
            t.height = f32::from(b.u16()) / TWIPS;
        }
        if f1 & 0x04 != 0 {
            t.colour = b.colour(true);
        }
        if f1 & 0x02 != 0 {
            b.u16();
        }
        if f2 & 0x20 != 0 {
            t.align = b.u8();
            t.left_margin = f32::from(b.u16()) / TWIPS;
            t.right_margin = f32::from(b.u16()) / TWIPS;
            b.u16();
            t.leading = f32::from(b.u16() as i16) / TWIPS;
        }
        t.variable = b.cstr();
        if f1 & 0x80 != 0 {
            t.initial = b.cstr();
        }
        Some((id, t))
    }
}

/// A glyph's place in a font texture: its box (u0, v0, u1, v1) and its
/// origin on the baseline (u, v).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextureGlyph {
    pub uv: [f32; 4],
    pub origin: [f32; 2],
}

/// A font's glyphs drawn into a texture.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontTexture {
    /// The texture's name (its file without the extension).
    pub name: String,
    pub size: [f32; 2],
    /// The em the glyphs were drawn at, pixels.
    pub nominal: f32,
    pub glyphs: Vec<TextureGlyph>,
    /// A font glyph's texture glyph, by font character id.
    pub maps: HashMap<u16, HashMap<u16, u16>>,
}

impl FontTexture {
    /// Parses a `FontTextureInfo` tag's body.
    pub fn parse(data: &[u8]) -> Option<FontTexture> {
        let mut b = Bits::new(data);
        b.u16();
        b.u16();
        b.u16();
        let len = usize::from(b.u8());
        let mut name = String::new();
        for _ in 0..len {
            name.push(char::from(b.u8()));
        }
        let name = name
            .rsplit_once('.')
            .map_or(name.clone(), |(s, _)| s.to_owned());
        let size = [f32::from(b.u16()), f32::from(b.u16())];
        b.u8();
        let nominal = f32::from(b.u16());
        let count = usize::from(b.u16());
        let mut glyphs = Vec::with_capacity(count);
        let f = |b: &mut Bits<'_>| {
            let v = [b.u8(), b.u8(), b.u8(), b.u8()];
            f32::from_le_bytes(v)
        };
        for _ in 0..count {
            let uv = [f(&mut b), f(&mut b), f(&mut b), f(&mut b)];
            let origin = [f(&mut b), f(&mut b)];
            glyphs.push(TextureGlyph { uv, origin });
        }
        let mut maps = HashMap::new();
        let fonts = b.u16();
        for _ in 0..fonts {
            let font = b.u16();
            let n = b.u16();
            let mut map = HashMap::new();
            for _ in 0..n {
                let in_font = b.u16();
                let in_texture = b.u16();
                map.insert(in_font, in_texture);
            }
            maps.insert(font, map);
        }
        (b.bit / 8 <= data.len()).then_some(FontTexture {
            name,
            size,
            nominal,
            glyphs,
            maps,
        })
    }
}

/// A font: its glyphs' codes and advances, and where they are drawn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Font {
    pub name: String,
    pub bold: bool,
    pub italic: bool,
    /// Glyph index by character.
    pub codes: HashMap<char, u16>,
    /// Advances, ems.
    pub advances: Vec<f32>,
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
    /// Its glyphs' texture glyphs, when the movie draws them so.
    #[allow(clippy::type_complexity)]
    pub texture: Option<(String, [f32; 2], f32, Vec<Option<TextureGlyph>>)>,
}

impl Font {
    /// Parses a `DefineFont2` or `DefineFont3` tag's body (metrics in
    /// 1/20480 em; `DefineFont2`'s 1/1024 are taken as the same, scaled).
    pub fn parse(data: &[u8]) -> Option<(u16, Font)> {
        if data.len() < 6 {
            return None;
        }
        let mut b = Bits::new(data);
        let id = b.u16();
        let flags = b.u8();
        b.u8();
        let len = usize::from(b.u8());
        let mut name = String::new();
        for _ in 0..len {
            name.push(char::from(b.u8()));
        }
        let name = name.trim_end_matches(['\0', ' ']).to_owned();
        let n = usize::from(b.u16());
        let wide_offsets = flags & 0x08 != 0;
        let wide_codes = flags & 0x04 != 0;
        let table = b.bit / 8;
        let offset = |b: &mut Bits<'_>| {
            if wide_offsets {
                u32::from_le_bytes([b.u8(), b.u8(), b.u8(), b.u8()]) as usize
            } else {
                usize::from(b.u16())
            }
        };
        // Scaleform strips a texture font's glyph shapes: its offset table
        // is then a single 0, then the code table's offset (which a real
        // table's first offset, past the table itself, never is).
        let first = if n > 0 { offset(&mut b) } else { 1 };
        if first != 0 {
            for _ in 1..n {
                offset(&mut b);
            }
        }
        let code_table = if n > 0 || flags & 0x80 != 0 {
            offset(&mut b)
        } else {
            0
        };
        b.bit = (table + code_table) * 8;
        let mut font = Font {
            name,
            bold: flags & 0x01 != 0,
            italic: flags & 0x02 != 0,
            ..Font::default()
        };
        for i in 0..n {
            let code = if wide_codes {
                u32::from(b.u16())
            } else {
                u32::from(b.u8())
            };
            if let Some(c) = char::from_u32(code) {
                font.codes.insert(c, i as u16);
            }
        }
        if flags & 0x80 != 0 {
            font.ascent = f32::from(b.u16()) / EM;
            font.descent = f32::from(b.u16()) / EM;
            font.leading = f32::from(b.u16() as i16) / EM;
            for _ in 0..n {
                font.advances.push(f32::from(b.u16() as i16) / EM);
            }
        }
        Some((id, font))
    }

    /// The font's glyphs as `texture` draws them for font `id`.
    pub fn with_texture(&mut self, id: u16, texture: &FontTexture) {
        let Some(map) = texture.maps.get(&id) else {
            return;
        };
        let glyphs = (0..self.advances.len().max(self.codes.len()))
            .map(|g| {
                map.get(&(g as u16))
                    .and_then(|&t| texture.glyphs.get(usize::from(t)))
                    .copied()
            })
            .collect();
        self.texture = Some((texture.name.clone(), texture.size, texture.nominal, glyphs));
    }

    fn advance(&self, c: char) -> f32 {
        self.codes
            .get(&c)
            .and_then(|&g| self.advances.get(usize::from(g)))
            .copied()
            .unwrap_or(0.5)
    }
}

/// The fonts the movies may import, by name (any case).
/// An image drawn in a text field's text in place of a string
/// (Scaleform's `TextField.setImageSubstitutions`): the flag for
/// `[flagvz]`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Substitution {
    pub text: String,
    /// The image's texture (none: the string is dropped) and its part.
    pub texture: String,
    pub part: [f32; 4],
    /// Its size in the text, and how far down it the baseline goes.
    pub size: [f32; 2],
    pub baseline: f32,
    /// Its name for `updateImageSubstitution`.
    pub id: String,
}

/// A piece of a line: a character, or a substitution's image.
#[derive(Clone, Copy)]
enum Piece {
    Char(char),
    Image(usize),
}

fn pieces(line: &str, subs: &[Substitution]) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        if let Some((i, sub)) = subs
            .iter()
            .enumerate()
            .find(|(_, s)| rest.starts_with(s.text.as_str()))
        {
            out.push(Piece::Image(i));
            rest = &rest[sub.text.len()..];
        } else {
            out.push(Piece::Char(c));
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

#[derive(Clone, Debug, Default)]
pub struct FontBook {
    pub fonts: HashMap<String, Font>,
}

impl FontBook {
    /// The fonts a font movie defines (with their textures), under the
    /// names it exports them by and their own.
    pub fn add_movie(&mut self, movie: &crate::gfx::Movie) {
        let mut fonts: HashMap<u16, Font> = HashMap::new();
        let mut textures = Vec::new();
        for t in &movie.tags {
            let data = movie.data(t);
            match t.code {
                crate::gfx::tag::DEFINE_FONT3 | crate::gfx::tag::DEFINE_FONT2 => {
                    if let Some((id, f)) = Font::parse(data) {
                        fonts.insert(id, f);
                    }
                }
                1002 => textures.extend(FontTexture::parse(data)),
                _ => {}
            }
        }
        for (id, f) in &mut fonts {
            for t in &textures {
                if f.texture.is_none() {
                    f.with_texture(*id, t);
                }
            }
        }
        for (id, name) in movie.summary().exports {
            if let Some(f) = fonts.get(&id) {
                self.fonts.insert(name.to_ascii_lowercase(), f.clone());
            }
        }
        for f in fonts.into_values() {
            self.fonts.entry(f.name.to_ascii_lowercase()).or_insert(f);
        }
    }

    pub fn get(&self, name: &str) -> Option<&Font> {
        self.fonts.get(&name.to_ascii_lowercase())
    }

    /// Its fonts by name, each with its own name, glyph count and texture.
    pub fn summary(&self) -> String {
        let mut names: Vec<String> = self
            .fonts
            .iter()
            .map(|(n, f)| {
                format!(
                    "{n} ({}, {} glyphs, {})",
                    f.name,
                    f.codes.len(),
                    f.texture.as_ref().map_or("no texture", |t| t.0.as_str())
                )
            })
            .collect();
        names.sort();
        names.join("; ")
    }
}

/// `text` with its HTML tags taken out and entities read, and the colour
/// of its first `<font color>` (as `#rrggbb`).
pub fn plain(text: &str, html: bool) -> (String, Option<[f32; 4]>) {
    if !html {
        return (text.replace('\r', "\n"), None);
    }
    let mut out = String::new();
    let mut colour = None;
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('>') else {
            rest = "";
            break;
        };
        let tag = &rest[open + 1..open + close];
        let lower = tag.to_ascii_lowercase();
        if lower.starts_with("br") || lower == "/p" {
            out.push('\n');
        }
        if colour.is_none()
            && lower.starts_with("font")
            && let Some(at) = lower.find("color=")
        {
            let hex: String = lower[at + 6..]
                .trim_start_matches(['"', '\'', '#'])
                .chars()
                .take(6)
                .collect();
            if let Ok(v) = u32::from_str_radix(&hex, 16) {
                colour = Some([
                    ((v >> 16) & 255) as f32 / 255.0,
                    ((v >> 8) & 255) as f32 / 255.0,
                    (v & 255) as f32 / 255.0,
                    1.0,
                ]);
            }
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    let out = out
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace('\r', "\n");
    (out.trim_end_matches('\n').to_owned(), colour)
}

/// Lays out `text` in the field `def` with `font`, as textured quads
/// (two triangles each), through `matrix` and `colour`, cut to `clip`.
#[allow(clippy::too_many_arguments)]
pub fn layout(
    def: &EditText,
    text: &str,
    subs: &[Substitution],
    font: &Font,
    matrix: &Matrix,
    colour: &ColourTransform,
    clip: Option<[f32; 4]>,
    out: &mut Vec<Triangle>,
) {
    let Some((texture, tex_size, nominal, glyphs)) = &font.texture else {
        return;
    };
    let (text, html_colour) = plain(text, def.html);
    let size = def.height;
    let tint = colour.apply(html_colour.unwrap_or(def.colour));
    if tint[3] <= 0.0 || size <= 0.0 {
        return;
    }
    let width = (def.bounds[2] - def.bounds[0] - def.left_margin - def.right_margin).max(1.0);
    let advance = |p: Piece| match p {
        Piece::Char(c) => font.advance(c) * size,
        Piece::Image(i) => subs[i].size[0],
    };
    let measure = |s: &str| -> f32 { pieces(s, subs).into_iter().map(advance).sum() };
    // Lines: the text's own, then wrapped at spaces to the box.
    let mut lines: Vec<String> = Vec::new();
    for line in text.split('\n') {
        if !def.word_wrap {
            lines.push(line.to_owned());
            continue;
        }
        let mut current = String::new();
        let mut w = 0.0;
        for word in line.split(' ') {
            let ww = measure(word);
            let space = if current.is_empty() {
                0.0
            } else {
                font.advance(' ') * size
            };
            if !current.is_empty() && w + space + ww > width {
                lines.push(std::mem::take(&mut current));
                w = 0.0;
            } else if !current.is_empty() {
                current.push(' ');
                w += space;
            }
            current.push_str(word);
            w += ww;
        }
        lines.push(current);
    }
    let ascent = if font.ascent > 0.0 { font.ascent } else { 0.8 } * size;
    let line_height = (font.ascent + font.descent).max(1.0) * size + def.leading;
    // Flash puts the first baseline 2 px in and down by the ascent.
    let mut baseline = def.bounds[1] + 2.0 + ascent;
    let scale = size / nominal.max(1.0);
    let quad =
        |out: &mut Vec<Triangle>, p0: [f32; 2], p1: [f32; 2], uv: [f32; 4], tint, tex: &str| {
            let [u0, v0, u1, v1] = uv;
            let corners = [p0, [p1[0], p0[1]], p1, [p0[0], p1[1]]].map(|p| matrix.apply(p));
            let uv = [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
            for [a, b, c] in [[0, 1, 2], [0, 2, 3]] {
                push_clipped(
                    out,
                    [corners[a], corners[b], corners[c]],
                    [uv[a], uv[b], uv[c]],
                    tint,
                    Some(tex.to_owned()),
                    clip,
                );
            }
        };
    for line in lines {
        let line = pieces(&line, subs);
        let w: f32 = line.iter().copied().map(advance).sum();
        let left = def.bounds[0] + 2.0 + def.left_margin;
        let mut x = match def.align {
            1 => def.bounds[2] - 2.0 - def.right_margin - w,
            2 => left + (width - 4.0 - w) / 2.0,
            _ => left,
        };
        for p in line {
            match p {
                Piece::Char(c) => {
                    if let Some(Some(g)) = font
                        .codes
                        .get(&c)
                        .map(|&i| glyphs.get(usize::from(i)).copied().flatten())
                    {
                        let [u0, v0, u1, v1] = g.uv;
                        let px = |u: f32, v: f32| {
                            [
                                x + (u - g.origin[0]) * tex_size[0] * scale,
                                baseline + (v - g.origin[1]) * tex_size[1] * scale,
                            ]
                        };
                        quad(out, px(u0, v0), px(u1, v1), g.uv, tint, texture);
                    }
                }
                Piece::Image(i) => {
                    let sub = &subs[i];
                    if !sub.texture.is_empty() {
                        let top = baseline - sub.baseline;
                        let p1 = [x + sub.size[0], top + sub.size[1]];
                        let white = colour.apply([1.0; 4]);
                        quad(out, [x, top], p1, sub.part, white, &sub.texture);
                    }
                }
            }
            x += advance(p);
        }
        baseline += line_height;
    }
}
