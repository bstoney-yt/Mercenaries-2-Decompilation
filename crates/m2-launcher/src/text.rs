//! Text in the game's own fonts: each glyph a quad cut from the font's
//! glyph page, as Scaleform draws a texture font
//! ([`m2_assets::flash::text::layout`] sets a text field the same way).
//! Where a font is missing (no install yet) egui's own stands in.
//!
//! # Provenance
//!
//! The glyph boxes, origins and advances are the font's own (from the
//! game's data); the size rule ([`TEXTURE_EM`]) is the one the game's
//! loading screen uses. Single lines only: the launcher wraps nothing.

use std::collections::HashMap;

use egui::{Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, TextureId, pos2, vec2};
use m2_assets::flash::text::{Font, TEXTURE_EM};

use crate::assets::Role;

/// A font with its glyph page uploaded.
#[derive(Debug)]
pub struct GameFont {
    pub font: Font,
    pub page: TextureId,
}

impl GameFont {
    fn glyph(&self, c: char) -> Option<usize> {
        self.font.codes.get(&c).map(|&g| usize::from(g))
    }

    /// The pen's advance for `c` at `px` pixels to the em: its own, or
    /// the upper case's (a face of capitals only), or half an em.
    pub fn advance(&self, c: char, px: f32) -> f32 {
        let own = |c: char| {
            self.glyph(c)
                .and_then(|g| self.font.advances.get(g))
                .copied()
        };
        own(c)
            .or_else(|| c.to_uppercase().next().and_then(own))
            .unwrap_or(0.5)
            * px
    }

    pub fn width(&self, text: &str, px: f32) -> f32 {
        text.chars().map(|c| self.advance(c, px)).sum()
    }

    /// Its line's height above and below the baseline at `px`.
    pub fn metrics(&self, px: f32) -> (f32, f32) {
        let ascent = if self.font.ascent > 0.0 {
            self.font.ascent
        } else {
            0.8
        };
        (ascent * px, self.font.descent.max(0.0) * px)
    }

    /// Draws `text` with its line's top left at `at`.
    pub fn draw(&self, painter: &Painter, at: Pos2, px: f32, colour: Color32, text: &str) {
        let Some((_, size, nominal, glyphs)) = &self.font.texture else {
            return;
        };
        let scale = px / (nominal.max(1.0) * TEXTURE_EM);
        let baseline = at.y + self.metrics(px).0;
        let mut x = at.x;
        let mut mesh = Mesh::with_texture(self.page);
        for c in text.chars() {
            let g = self
                .glyph(c)
                .or_else(|| c.to_uppercase().next().and_then(|u| self.glyph(u)));
            if let Some(Some(g)) = g.map(|g| glyphs.get(g).copied().flatten()) {
                let [u0, v0, u1, v1] = g.uv;
                let p = |u: f32, v: f32| {
                    pos2(
                        x + (u - g.origin[0]) * size[0] * scale,
                        baseline + (v - g.origin[1]) * size[1] * scale,
                    )
                };
                mesh.add_rect_with_uv(
                    Rect::from_min_max(p(u0, v0), p(u1, v1)),
                    Rect::from_min_max(pos2(u0, v0), pos2(u1, v1)),
                    colour,
                );
            }
            x += self.advance(c, px);
        }
        painter.add(mesh);
    }
}

/// The launcher's fonts by role.
#[derive(Debug, Default)]
pub struct Fonts {
    pub fonts: HashMap<Role, GameFont>,
}

impl Fonts {
    /// The width of `text` at `px` in `role`'s font.
    pub fn width(&self, painter: &Painter, role: Role, px: f32, text: &str) -> f32 {
        match self.fonts.get(&role) {
            Some(f) => f.width(text, px),
            None => {
                painter
                    .layout_no_wrap(text.to_owned(), fallback(role, px), Color32::WHITE)
                    .size()
                    .x
            }
        }
    }

    /// Draws `text` at `px` in `role`'s font, placed by `align` about
    /// `at`; the box it takes.
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &self,
        painter: &Painter,
        role: Role,
        at: Pos2,
        align: Align2,
        px: f32,
        colour: Color32,
        text: &str,
    ) -> Rect {
        match self.fonts.get(&role) {
            Some(f) => {
                let (up, down) = f.metrics(px);
                let size = vec2(f.width(text, px), up + down);
                let rect = align.anchor_size(at, size);
                f.draw(painter, rect.min, px, colour, text);
                rect
            }
            None => painter.text(at, align, text, fallback(role, px), colour),
        }
    }
}

fn fallback(role: Role, px: f32) -> FontId {
    match role {
        Role::Heading => FontId::proportional(px * 0.9),
        Role::Body | Role::BodyBold => FontId::proportional(px * 0.85),
    }
}
