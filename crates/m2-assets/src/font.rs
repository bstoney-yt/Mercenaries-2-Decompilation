//! The game's native bitmap fonts: the faces its own GUI widgets draw text
//! with (`english_18`, `english_20`, ...), separate from the fonts inside
//! Scaleform movies.
//!
//! A font is an asset of type [`FONT_TYPE`] named by [`crate::name_hash`] of the
//! name a layout's `font` field gives. Its `UCFX` container has three
//! sections:
//!
//! ```text
//! INFO  u32 material count
//!       u32 unknown (30 in the English faces, 0 in the Japanese, 6 in
//!           the three-page face; not the number of off-page glyphs)
//!       u16 line height in pixels
//!       u16 the tallest glyph's height
//!       u32 glyph count
//! CHAR  glyph count × u16 character code (UTF-16; icons are private use,
//!       U+F1xx), then glyph count × 12-byte record:
//!         u16 u0, u16 u1, u16 v0, u16 v1   the glyph's texture rectangle,
//!                                          as fractions × 65536
//!         u8  width, u8 height             its size in pixels
//!         u8  y offset                     from the top of the line
//!         u8  material                     index into MTRL
//! MTRL  material count × 120 bytes: 26 f32 (colours and constants alike
//!       in every font seen), u16 1, u16 1, u32 texture asset hash, then
//!       two u32 hashes that are the same in every font
//! ```
//!
//! Glyphs are stored sorted by height (the order they were packed in),
//! not by code. A material's texture is found by its asset hash
//! directly; the texture's own name is its source path
//! (`...\hud\fonts\english_18_main.ftga`). Page 0 holds the face; the
//! other pages hold button prompts and symbols (U+F1xx), and in the
//! Japanese faces the kanji. The glyph's pen advance is taken as its
//! width: the pages include the spacing (unconfirmed).
//!
//! # Provenance
//!
//! Format-from-samples: the 22 fonts of a legally installed copy. Every
//! glyph rectangle lies on its page and matches the glyph's pixel size
//! (within a pixel on the smallest icon pages). No game data is committed.

use std::collections::HashMap;

use crate::bytes::{OutOfBounds, Reader};
use crate::ucfx::{Ucfx, UcfxError};

/// Asset type hash of bitmap fonts.
pub const FONT_TYPE: u32 = 0x99e7_7ace;

/// Bytes per glyph record in `CHAR`, after the codes.
const GLYPH_LEN: usize = 12;
/// Bytes per material in `MTRL`.
const MATERIAL_LEN: usize = 120;

/// Errors from parsing a font.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum FontError {
    #[error("container: {0}")]
    Ucfx(#[from] UcfxError),
    #[error("truncated: needed {len} bytes at {offset:#x}")]
    Truncated { offset: usize, len: usize },
    #[error("no {0} section")]
    Missing(&'static str),
    #[error("{section} is {len} bytes, expected {expected}")]
    Length {
        section: &'static str,
        len: usize,
        expected: usize,
    },
    #[error("glyph {code:#06x} uses material {material} of {count}")]
    Material {
        code: u16,
        material: u8,
        count: usize,
    },
}

impl From<OutOfBounds> for FontError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// One character's image and metrics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    /// UTF-16 code unit.
    pub code: u16,
    /// Texture rectangle `[u0, v0, u1, v1]` in 0..=1.
    pub uv: [f32; 4],
    pub width: u8,
    pub height: u8,
    /// Pixels from the top of the line to the glyph's top.
    pub y_offset: u8,
    /// Index into [`Font::materials`].
    pub material: u8,
}

/// A texture page and its render settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    /// Asset hash of the page's texture.
    pub texture: u32,
    /// The record's 26 floats, kept as read.
    pub params: [f32; 26],
    /// The two u16 before the texture hash (1, 1 in every font seen).
    pub flags: [u16; 2],
    /// The two hashes after it (the same in every font seen).
    pub unk_hashes: [u32; 2],
}

/// A parsed bitmap font.
#[derive(Debug, Clone, PartialEq)]
pub struct Font {
    /// `INFO` +0x04; see the module docs.
    pub unk_0x04: u32,
    pub line_height: u16,
    /// The tallest glyph's height.
    pub tallest: u16,
    /// In the order stored (by height).
    pub glyphs: Vec<Glyph>,
    pub materials: Vec<Material>,
    by_code: HashMap<u16, usize>,
}

impl Font {
    /// Parses a font asset.
    pub fn parse(asset: &[u8]) -> Result<Self, FontError> {
        let ucfx = Ucfx::parse(asset)?;
        let section = |cc: [u8; 4], name| ucfx.top_level(cc).ok_or(FontError::Missing(name));
        Self::parse_sections(
            section(*b"INFO", "INFO")?,
            section(*b"CHAR", "CHAR")?,
            section(*b"MTRL", "MTRL")?,
        )
    }

    /// Parses the `INFO`, `CHAR` and `MTRL` sections.
    pub fn parse_sections(info: &[u8], chars: &[u8], mtrl: &[u8]) -> Result<Self, FontError> {
        let mut r = Reader::at(info, 0);
        let material_count = r.u32()? as usize;
        let unk_0x04 = r.u32()?;
        let line_height = r.u16()?;
        let tallest = r.u16()?;
        let count = r.u32()? as usize;

        let expected = count.saturating_mul(2 + GLYPH_LEN);
        if chars.len() != expected {
            return Err(FontError::Length {
                section: "CHAR",
                len: chars.len(),
                expected,
            });
        }
        let expected = material_count.saturating_mul(MATERIAL_LEN);
        if mtrl.len() != expected {
            return Err(FontError::Length {
                section: "MTRL",
                len: mtrl.len(),
                expected,
            });
        }

        let mut materials = Vec::with_capacity(material_count);
        let mut r = Reader::at(mtrl, 0);
        for _ in 0..material_count {
            let mut params = [0.0; 26];
            for p in &mut params {
                *p = f32::from_bits(r.u32()?);
            }
            let flags = [r.u16()?, r.u16()?];
            let texture = r.u32()?;
            let unk_hashes = [r.u32()?, r.u32()?];
            materials.push(Material {
                texture,
                params,
                flags,
                unk_hashes,
            });
        }

        let mut codes = Reader::at(chars, 0);
        let mut rec = Reader::at(chars, count * 2);
        let mut glyphs = Vec::with_capacity(count);
        let mut by_code = HashMap::with_capacity(count);
        for i in 0..count {
            let code = codes.u16()?;
            let mut uv = [0.0; 4];
            // Stored u0, u1, v0, v1; kept as [u0, v0, u1, v1].
            for slot in [0, 2, 1, 3] {
                uv[slot] = f32::from(rec.u16()?) / 65536.0;
            }
            let [width, height, y_offset, material] = rec.array::<4>()?;
            if usize::from(material) >= material_count {
                return Err(FontError::Material {
                    code,
                    material,
                    count: material_count,
                });
            }
            glyphs.push(Glyph {
                code,
                uv,
                width,
                height,
                y_offset,
                material,
            });
            by_code.entry(code).or_insert(i);
        }
        Ok(Self {
            unk_0x04,
            line_height,
            tallest,
            glyphs,
            materials,
            by_code,
        })
    }

    /// The glyph for a UTF-16 code unit.
    pub fn glyph(&self, code: u16) -> Option<&Glyph> {
        self.by_code.get(&code).map(|&i| &self.glyphs[i])
    }

    /// Width in pixels of `text` at scale 1: the sum of its glyphs'
    /// widths. Characters the font lacks count as nothing, except a space,
    /// which counts as a third of the line height if the font has none.
    pub fn text_width(&self, text: &str) -> f32 {
        text.encode_utf16().map(|c| self.advance(c)).sum()
    }

    /// The pen advance of one code unit.
    pub fn advance(&self, code: u16) -> f32 {
        match self.glyph(code) {
            Some(g) => f32::from(g.width),
            None if code == u16::from(b' ') => f32::from(self.line_height) / 3.0,
            None => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{UcfxNode, ucfx};

    fn info(materials: u32, icons: u32, line: u16, second: u16, glyphs: u32) -> Vec<u8> {
        let mut v = materials.to_le_bytes().to_vec();
        v.extend(icons.to_le_bytes());
        v.extend(line.to_le_bytes());
        v.extend(second.to_le_bytes());
        v.extend(glyphs.to_le_bytes());
        v
    }

    fn material(texture: u32) -> Vec<u8> {
        let mut v: Vec<u8> = (0..26).flat_map(|i| (i as f32).to_le_bytes()).collect();
        v.extend(1u16.to_le_bytes());
        v.extend(1u16.to_le_bytes());
        v.extend(texture.to_le_bytes());
        v.extend(0xaaaa_aaaau32.to_le_bytes());
        v.extend(0xbbbb_bbbbu32.to_le_bytes());
        v
    }

    /// (code, [u0, u1, v0, v1] raw, width, height, y offset, material)
    type RawGlyph = (u16, [u16; 4], u8, u8, u8, u8);

    fn chars(glyphs: &[RawGlyph]) -> Vec<u8> {
        let mut v: Vec<u8> = glyphs.iter().flat_map(|g| g.0.to_le_bytes()).collect();
        for g in glyphs {
            for x in g.1 {
                v.extend(x.to_le_bytes());
            }
            v.extend([g.2, g.3, g.4, g.5]);
        }
        v
    }

    fn sample() -> Vec<u8> {
        ucfx(&[
            UcfxNode::Leaf(*b"INFO", info(2, 1, 30, 34, 3)),
            UcfxNode::Leaf(
                *b"CHAR",
                chars(&[
                    (
                        u16::from(b'-'),
                        [0x00ff, 0x0aff, 0x007f, 0x02ff],
                        10,
                        5,
                        19,
                        0,
                    ),
                    (
                        u16::from(b'A'),
                        [0x0bff, 0x18ff, 0x0400, 0x0a00],
                        13,
                        12,
                        8,
                        0,
                    ),
                    (0xf101, [0x4000, 0x8000, 0x0000, 0x8000], 34, 22, 6, 1),
                ]),
            ),
            UcfxNode::Leaf(
                *b"MTRL",
                [material(0x1111_1111), material(0x2222_2222)].concat(),
            ),
        ])
    }

    #[test]
    fn parses_header_glyphs_and_materials() {
        let f = Font::parse(&sample()).unwrap();
        assert_eq!((f.unk_0x04, f.line_height, f.tallest), (1, 30, 34));
        assert_eq!(f.glyphs.len(), 3);
        assert_eq!(f.materials.len(), 2);
        assert_eq!(f.materials[1].texture, 0x2222_2222);
        assert_eq!(f.materials[0].flags, [1, 1]);
        assert_eq!(f.materials[0].unk_hashes, [0xaaaa_aaaa, 0xbbbb_bbbb]);
        assert_eq!(f.materials[0].params[25], 25.0);

        let dash = f.glyph(u16::from(b'-')).unwrap();
        assert_eq!(
            (dash.width, dash.height, dash.y_offset, dash.material),
            (10, 5, 19, 0)
        );
        // u0, v0, u1, v1 from the stored u0, u1, v0, v1.
        let uv = dash.uv.map(|x| x * 65536.0);
        assert_eq!(uv, [255.0, 127.0, 2815.0, 767.0]);
        assert_eq!(f.glyph(0xf101).unwrap().material, 1);
        assert!(f.glyph(u16::from(b'z')).is_none());
    }

    #[test]
    fn text_width_sums_advances() {
        let f = Font::parse(&sample()).unwrap();
        assert_eq!(f.text_width("A-A"), 36.0);
        // No space glyph: a third of the line height.
        assert_eq!(f.text_width("A A"), 36.0);
        assert_eq!(f.text_width("?"), 0.0);
    }

    #[test]
    fn rejects_bad_lengths_and_materials() {
        let bad = Font::parse_sections(&info(1, 0, 30, 34, 2), &[0; 10], &material(1));
        assert!(matches!(
            bad,
            Err(FontError::Length {
                section: "CHAR",
                ..
            })
        ));
        let c = chars(&[(65, [0; 4], 1, 1, 0, 3)]);
        let bad = Font::parse_sections(&info(1, 0, 30, 34, 1), &c, &material(1));
        assert_eq!(
            bad,
            Err(FontError::Material {
                code: 65,
                material: 3,
                count: 1
            })
        );
        let bad = Font::parse_sections(&info(2, 0, 30, 34, 1), &c, &material(1));
        assert!(matches!(
            bad,
            Err(FontError::Length {
                section: "MTRL",
                ..
            })
        ));
    }
}
