//! Flash (SWF) shapes and timelines drawn as triangles: what a movie
//! (see [`crate::gfx`]) shows on its first frame.
//!
//! A shape (`DefineShape` 1 to 4) is a list of edges, straight or
//! quadratic curves, each with the fill on its left (`fill0`), the fill on
//! its right (`fill1`) and the line it is stroked with, all by index into
//! style tables that a shape may replace part way through. [`Shape::parse`]
//! reads them; [`Shape::mesh`] turns each fill into triangles (curves
//! flattened, the edges of a fill taken as drawn where it is their
//! `fill1` and reversed where it is their `fill0`, the region the
//! nonzero winding of those, cut into trapezoids between the edges'
//! heights) and each stroked edge into a quad.
//!
//! A timeline (the movie's own, or a `DefineSprite`'s) places characters
//! by depth with `PlaceObject`/`PlaceObject2`/`PlaceObject3` (a matrix,
//! a colour transform, a clip depth that masks the depths above it up to
//! that) and takes them away with `RemoveObject`/`RemoveObject2`.
//! [`Library::draw`] lays out the first frame of the movie's timeline,
//! each sprite showing its own first frame, as triangles in the movie's
//! pixels: solid fills coloured, bitmap fills textured from the images
//! the movie names (Scaleform's `DefineExternalImage`: textures in the
//! archives by file name), every colour through the transforms above it.
//! A mask clips what it masks to its bounds (the masks of the game's
//! screens are rectangles).
//!
//! Not drawn yet: gradients (drawn in their first colour), morph shapes,
//! text, buttons, filters and blend modes; nothing runs the movies'
//! ActionScript, so a sprite shows its first frame whatever its script
//! would make of it.
//!
//! # Provenance
//!
//! Clean-room from the public SWF file format specification (version 10)
//! and the published list of Scaleform GFx extension tags. Tested against
//! hand-built shapes and timelines; nothing here was read from the game's
//! executable or its movies' scripts.

use std::collections::HashMap;

pub mod avm1;
pub mod player;
pub mod text;

pub use player::{ClipId, Library, Player, Translator};
pub use text::FontBook;

/// Twips (1/20 pixel) to pixels.
const TWIPS: f32 = 20.0;
/// Straight pieces a quadratic curve is drawn with, at most.
const CURVE_PIECES: usize = 8;
/// The thinnest stroke drawn, pixels (hairlines are one pixel wide).
const HAIRLINE: f32 = 1.0;

/// Reads a tag's fields: bytes little-endian, bit fields most significant
/// bit first.
struct Bits<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }

    fn ub(&mut self, n: usize) -> u32 {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = self.data.get(self.bit / 8).copied().unwrap_or(0);
            v = (v << 1) | u32::from((byte >> (7 - self.bit % 8)) & 1);
            self.bit += 1;
        }
        v
    }

    fn sb(&mut self, n: usize) -> i32 {
        if n == 0 {
            return 0;
        }
        let v = self.ub(n);
        // Sign-extend from bit n - 1.
        let shift = 32 - n as u32;
        ((v << shift) as i32) >> shift
    }

    /// A signed 16.16 fixed-point bit field.
    fn fb(&mut self, n: usize) -> f32 {
        self.sb(n) as f32 / 65536.0
    }

    fn align(&mut self) {
        self.bit = self.bit.div_ceil(8) * 8;
    }

    fn u8(&mut self) -> u8 {
        self.align();
        let v = self.data.get(self.bit / 8).copied().unwrap_or(0);
        self.bit += 8;
        v
    }

    fn u16(&mut self) -> u16 {
        u16::from(self.u8()) | u16::from(self.u8()) << 8
    }

    fn cstr(&mut self) -> String {
        self.align();
        let start = self.bit / 8;
        let rest = self.data.get(start..).unwrap_or(&[]);
        let len = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        self.bit += (len + 1) * 8;
        String::from_utf8_lossy(&rest[..len]).into_owned()
    }

    fn done(&self) -> bool {
        self.bit / 8 >= self.data.len()
    }

    fn rect(&mut self) -> [f32; 4] {
        self.align();
        let n = self.ub(5) as usize;
        let v = [self.sb(n), self.sb(n), self.sb(n), self.sb(n)];
        self.align();
        // x min, x max, y min, y max as stored; given as x0, y0, x1, y1.
        [v[0], v[2], v[1], v[3]].map(|t| t as f32 / TWIPS)
    }

    fn matrix(&mut self) -> Matrix {
        self.align();
        let mut m = Matrix::IDENTITY;
        if self.ub(1) == 1 {
            let n = self.ub(5) as usize;
            m.a = self.fb(n);
            m.d = self.fb(n);
        }
        if self.ub(1) == 1 {
            let n = self.ub(5) as usize;
            m.b = self.fb(n);
            m.c = self.fb(n);
        }
        let n = self.ub(5) as usize;
        m.tx = self.sb(n) as f32 / TWIPS;
        m.ty = self.sb(n) as f32 / TWIPS;
        self.align();
        m
    }

    /// A colour transform; with alpha (`PlaceObject2` on) or without.
    fn cxform(&mut self, alpha: bool) -> ColourTransform {
        self.align();
        let add = self.ub(1) == 1;
        let mult = self.ub(1) == 1;
        let n = self.ub(4) as usize;
        let count = if alpha { 4 } else { 3 };
        let mut c = ColourTransform::IDENTITY;
        if mult {
            for i in 0..count {
                c.mult[i] = self.sb(n) as f32 / 256.0;
            }
        }
        if add {
            for i in 0..count {
                c.add[i] = self.sb(n) as f32 / 255.0;
            }
        }
        self.align();
        c
    }

    fn colour(&mut self, alpha: bool) -> [f32; 4] {
        let (r, g, b) = (self.u8(), self.u8(), self.u8());
        let a = if alpha { self.u8() } else { 255 };
        [r, g, b, a].map(|v| f32::from(v) / 255.0)
    }
}

/// A 2D affine transform, as SWF's `MATRIX`: `x' = a x + c y + tx`,
/// `y' = b x + d y + ty` (pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Matrix {
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    pub fn apply(&self, [x, y]: [f32; 2]) -> [f32; 2] {
        [
            self.a * x + self.c * y + self.tx,
            self.b * x + self.d * y + self.ty,
        ]
    }

    /// `self` after `inner`: what `inner` places, `self` places again.
    pub fn then(&self, inner: &Matrix) -> Matrix {
        Matrix {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            tx: self.a * inner.tx + self.c * inner.ty + self.tx,
            ty: self.b * inner.tx + self.d * inner.ty + self.ty,
        }
    }

    pub fn inverse(&self) -> Option<Matrix> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-12 {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Matrix {
            a,
            b,
            c,
            d,
            tx: -(a * self.tx + c * self.ty),
            ty: -(b * self.tx + d * self.ty),
        })
    }
}

/// A colour transform: each channel times `mult`, plus `add` (0..=1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColourTransform {
    pub mult: [f32; 4],
    pub add: [f32; 4],
}

impl ColourTransform {
    pub const IDENTITY: Self = Self {
        mult: [1.0; 4],
        add: [0.0; 4],
    };

    pub fn apply(&self, c: [f32; 4]) -> [f32; 4] {
        std::array::from_fn(|i| (c[i] * self.mult[i] + self.add[i]).clamp(0.0, 1.0))
    }

    /// `self` after `inner`.
    pub fn then(&self, inner: &ColourTransform) -> ColourTransform {
        ColourTransform {
            mult: std::array::from_fn(|i| self.mult[i] * inner.mult[i]),
            add: std::array::from_fn(|i| self.mult[i] * inner.add[i] + self.add[i]),
        }
    }
}

/// How a region is filled.
#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
    Solid([f32; 4]),
    /// A gradient, drawn for now in its first colour.
    Gradient([f32; 4]),
    /// A bitmap (by character id) through `matrix` (image pixels to the
    /// shape's pixels); `repeat` tiles it, else it is clamped.
    Bitmap {
        id: u16,
        matrix: Matrix,
        repeat: bool,
    },
}

/// How an edge is stroked: width (pixels) and colour.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub width: f32,
    pub colour: [f32; 4],
}

/// One edge: from, an optional curve control point, to (pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    pub from: [f32; 2],
    pub control: Option<[f32; 2]>,
    pub to: [f32; 2],
    /// Styles by index into the shape's tables, 0 for none.
    pub fill0: usize,
    pub fill1: usize,
    pub line: usize,
}

/// A shape: its style tables (index 0 is "none") and edges.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shape {
    pub bounds: [f32; 4],
    pub fills: Vec<Fill>,
    pub lines: Vec<Line>,
    pub edges: Vec<Edge>,
}

/// A triangle of a shape's mesh in the shape's pixels, and what it is
/// drawn with: a fill (by index) or a line (by index).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeTriangle {
    pub corners: [[f32; 2]; 3],
    pub style: Style,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Fill(usize),
    Line(usize),
}

fn fill_style(b: &mut Bits<'_>, version: u8) -> Fill {
    let alpha = version >= 3;
    match b.u8() {
        0x00 => Fill::Solid(b.colour(alpha)),
        kind @ (0x10 | 0x12 | 0x13) => {
            b.matrix();
            b.align();
            let head = b.u8();
            let count = usize::from(head & 15);
            let mut first = [0.0, 0.0, 0.0, 1.0];
            for i in 0..count {
                b.u8();
                let c = b.colour(alpha);
                if i == 0 {
                    first = c;
                }
            }
            if kind == 0x13 {
                b.u16();
            }
            Fill::Gradient(first)
        }
        kind @ 0x40..=0x43 => {
            let id = b.u16();
            // Image pixels to twips as stored; to pixels here.
            let m = b.matrix();
            let matrix = Matrix {
                a: m.a / TWIPS,
                b: m.b / TWIPS,
                c: m.c / TWIPS,
                d: m.d / TWIPS,
                ..m
            };
            Fill::Bitmap {
                id,
                matrix,
                repeat: kind == 0x40 || kind == 0x42,
            }
        }
        _ => Fill::Solid([0.0; 4]),
    }
}

fn style_arrays(b: &mut Bits<'_>, version: u8, fills: &mut Vec<Fill>, lines: &mut Vec<Line>) {
    let mut n = usize::from(b.u8());
    if n == 0xff && version >= 2 {
        n = usize::from(b.u16());
    }
    for _ in 0..n {
        fills.push(fill_style(b, version));
    }
    let mut n = usize::from(b.u8());
    if n == 0xff {
        n = usize::from(b.u16());
    }
    for _ in 0..n {
        let width = (f32::from(b.u16()) / TWIPS).max(HAIRLINE);
        if version >= 4 {
            b.align();
            let head = b.ub(8);
            let join = (head >> 4) & 3;
            let has_fill = (head >> 3) & 1 == 1;
            b.ub(8);
            if join == 2 {
                b.u16();
            }
            let colour = if has_fill {
                match fill_style(b, version) {
                    Fill::Solid(c) | Fill::Gradient(c) => c,
                    Fill::Bitmap { .. } => [1.0; 4],
                }
            } else {
                b.colour(true)
            };
            lines.push(Line { width, colour });
        } else {
            let colour = b.colour(version >= 3);
            lines.push(Line { width, colour });
        }
    }
}

impl Shape {
    /// Parses a `DefineShape` (`version` 1 to 4) tag's body.
    pub fn parse(data: &[u8], version: u8) -> (u16, Shape) {
        let mut b = Bits::new(data);
        let id = b.u16();
        let bounds = b.rect();
        if version >= 4 {
            b.rect();
            b.u8();
        }
        let mut shape = Shape {
            bounds,
            fills: vec![Fill::Solid([0.0; 4])],
            lines: vec![Line {
                width: 0.0,
                colour: [0.0; 4],
            }],
            edges: Vec::new(),
        };
        let (mut fills, mut lines) = (Vec::new(), Vec::new());
        style_arrays(&mut b, version, &mut fills, &mut lines);
        // Indices into the current tables count from these.
        let (mut fill_base, mut line_base) = (shape.fills.len() - 1, shape.lines.len() - 1);
        shape.fills.extend(fills);
        shape.lines.extend(lines);
        b.align();
        let mut fill_bits = b.ub(4) as usize;
        let mut line_bits = b.ub(4) as usize;
        let (mut x, mut y) = (0i32, 0i32);
        let (mut f0, mut f1, mut ln) = (0usize, 0usize, 0usize);
        let at = |x: i32, y: i32| [x as f32 / TWIPS, y as f32 / TWIPS];
        let index = |v: u32, base: usize| if v == 0 { 0 } else { v as usize + base };
        while !b.done() {
            if b.ub(1) == 0 {
                let flags = b.ub(5);
                if flags == 0 {
                    break;
                }
                if flags & 1 != 0 {
                    let n = b.ub(5) as usize;
                    x = b.sb(n);
                    y = b.sb(n);
                }
                if flags & 2 != 0 {
                    f0 = index(b.ub(fill_bits), fill_base);
                }
                if flags & 4 != 0 {
                    f1 = index(b.ub(fill_bits), fill_base);
                }
                if flags & 8 != 0 {
                    ln = index(b.ub(line_bits), line_base);
                }
                if flags & 16 != 0 {
                    let (mut fills, mut lines) = (Vec::new(), Vec::new());
                    style_arrays(&mut b, version, &mut fills, &mut lines);
                    fill_base = shape.fills.len() - 1;
                    line_base = shape.lines.len() - 1;
                    shape.fills.extend(fills);
                    shape.lines.extend(lines);
                    b.align();
                    fill_bits = b.ub(4) as usize;
                    line_bits = b.ub(4) as usize;
                    f0 = 0;
                    f1 = 0;
                    ln = 0;
                }
                continue;
            }
            let straight = b.ub(1) == 1;
            let n = b.ub(4) as usize + 2;
            let from = at(x, y);
            let control = if straight {
                if b.ub(1) == 1 {
                    x += b.sb(n);
                    y += b.sb(n);
                } else if b.ub(1) == 1 {
                    y += b.sb(n);
                } else {
                    x += b.sb(n);
                }
                None
            } else {
                x += b.sb(n);
                y += b.sb(n);
                let c = at(x, y);
                x += b.sb(n);
                y += b.sb(n);
                Some(c)
            };
            shape.edges.push(Edge {
                from,
                control,
                to: at(x, y),
                fill0: f0,
                fill1: f1,
                line: ln,
            });
        }
        (id, shape)
    }

    /// The shape as triangles: each fill's region, then each stroke.
    pub fn mesh(&self) -> Vec<ShapeTriangle> {
        let mut out = Vec::new();
        // Each edge as straight pieces.
        let pieces: Vec<(Vec<[f32; 2]>, &Edge)> =
            self.edges.iter().map(|e| (flatten(e), e)).collect();
        for fill in 1..self.fills.len() {
            let mut segments = Vec::new();
            for (points, e) in &pieces {
                let forward = e.fill1 == fill;
                let backward = e.fill0 == fill;
                if forward == backward {
                    continue;
                }
                for w in points.windows(2) {
                    if forward {
                        segments.push((w[0], w[1]));
                    } else {
                        segments.push((w[1], w[0]));
                    }
                }
            }
            for corners in fill_triangles(&segments) {
                out.push(ShapeTriangle {
                    corners,
                    style: Style::Fill(fill),
                });
            }
        }
        for (points, e) in &pieces {
            if e.line == 0 {
                continue;
            }
            let half = self.lines.get(e.line).map_or(HAIRLINE, |l| l.width) / 2.0;
            for w in points.windows(2) {
                for corners in stroke(w[0], w[1], half) {
                    out.push(ShapeTriangle {
                        corners,
                        style: Style::Line(e.line),
                    });
                }
            }
        }
        out
    }
}

/// An edge as points along it: its ends, and for a curve points between.
fn flatten(e: &Edge) -> Vec<[f32; 2]> {
    let Some(c) = e.control else {
        return vec![e.from, e.to];
    };
    // Enough pieces for the curve's size, up to CURVE_PIECES.
    let span = (c[0] - e.from[0]).hypot(c[1] - e.from[1]) + (e.to[0] - c[0]).hypot(e.to[1] - c[1]);
    let n = ((span / 4.0).ceil() as usize).clamp(2, CURVE_PIECES);
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            std::array::from_fn(|k| u * u * e.from[k] + 2.0 * u * t * c[k] + t * t * e.to[k])
        })
        .collect()
}

/// The region the directed `segments` wind round (nonzero), as
/// triangles: between each pair of neighbouring heights where a segment
/// starts or ends, the spans where the winding is not zero, each a
/// trapezoid of two triangles.
pub fn fill_triangles(segments: &[([f32; 2], [f32; 2])]) -> Vec<[[f32; 2]; 3]> {
    let mut ys: Vec<f32> = segments.iter().flat_map(|(a, b)| [a[1], b[1]]).collect();
    ys.sort_by(f32::total_cmp);
    ys.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
    let mut out = Vec::new();
    for w in ys.windows(2) {
        let (top, bottom) = (w[0], w[1]);
        let mid = (top + bottom) / 2.0;
        // Each segment across the slab: x at its top, at its bottom, and
        // which way it winds.
        let mut across: Vec<(f32, f32, i32)> = segments
            .iter()
            .filter(|(a, b)| a[1].min(b[1]) <= mid && a[1].max(b[1]) >= mid && a[1] != b[1])
            .map(|(a, b)| {
                let x = |y: f32| a[0] + (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]);
                (x(top), x(bottom), if b[1] > a[1] { 1 } else { -1 })
            })
            .collect();
        across.sort_by(|p, q| (p.0 + p.1).total_cmp(&(q.0 + q.1)));
        let mut winding = 0;
        let mut start: Option<(f32, f32)> = None;
        for (xt, xb, dir) in across {
            let before = winding;
            winding += dir;
            if before == 0 && winding != 0 {
                start = Some((xt, xb));
            } else if before != 0
                && winding == 0
                && let Some((st, sb)) = start.take()
            {
                out.push([[st, top], [xt, top], [xb, bottom]]);
                out.push([[st, top], [xb, bottom], [sb, bottom]]);
            }
        }
    }
    out
}

/// A stroke from `a` to `b`, `half` its width either side, as two
/// triangles.
pub(crate) fn stroke(a: [f32; 2], b: [f32; 2], half: f32) -> [[[f32; 2]; 3]; 2] {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = dx.hypot(dy).max(1e-6);
    let (nx, ny) = (-dy / len * half, dx / len * half);
    let p = [a[0] + nx, a[1] + ny];
    let q = [b[0] + nx, b[1] + ny];
    let r = [b[0] - nx, b[1] - ny];
    let s = [a[0] - nx, a[1] - ny];
    [[p, q, r], [p, r, s]]
}

/// The tags in a list of tags' bytes (a sprite's body after its id and
/// frame count).
pub fn tags(data: &[u8]) -> Vec<(u16, &[u8])> {
    let mut out = Vec::new();
    let mut p = 0;
    while p + 2 <= data.len() {
        let head = u16::from_le_bytes([data[p], data[p + 1]]);
        p += 2;
        let code = head >> 6;
        let mut len = usize::from(head & 0x3f);
        if len == 0x3f {
            let Some(l) = data.get(p..p + 4) else { break };
            len = u32::from_le_bytes([l[0], l[1], l[2], l[3]]) as usize;
            p += 4;
        }
        let end = (p + len).min(data.len());
        out.push((code, &data[p..end]));
        p = end;
        if code == crate::gfx::tag::END {
            break;
        }
    }
    out
}

/// An image a movie names, by the texture it is in the archives.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    /// The texture's name: the file's name without its extension.
    pub texture: String,
    /// Its size in pixels.
    pub width: f32,
    pub height: f32,
    /// Where it lies in the texture (x0, y0, x1, y1, 0..=1): all of it,
    /// or a part for Scaleform's `DefineSubImage`.
    pub part: [f32; 4],
}

/// A triangle to draw, in the movie's pixels: its corners, their texture
/// coordinates (0..=1 of the image), its colour (multiplying the image)
/// and the texture, if an image.
#[derive(Clone, Debug, PartialEq)]
pub struct Triangle {
    pub corners: [[f32; 2]; 3],
    pub uv: [[f32; 2]; 3],
    pub colour: [f32; 4],
    pub texture: Option<String>,
}

/// A shape's triangles drawn through `matrix` and `colour`, bitmap fills
/// textured from `images`, cut to `clip` if given.
pub fn draw_shape(
    shape: &Shape,
    mesh: &[ShapeTriangle],
    images: &HashMap<u16, Image>,
    matrix: &Matrix,
    colour: &ColourTransform,
    clip: Option<[f32; 4]>,
    out: &mut Vec<Triangle>,
) {
    for t in mesh {
        // The colour, and for an image its texture and the shape's
        // pixels to the image's.
        let (tint, image) = match t.style {
            Style::Line(i) => (shape.lines[i].colour, None),
            Style::Fill(i) => match &shape.fills[i] {
                Fill::Solid(c) | Fill::Gradient(c) => (*c, None),
                Fill::Bitmap { id, matrix: bm, .. } => match (images.get(id), bm.inverse()) {
                    (Some(img), Some(inv)) => ([1.0; 4], Some((img, inv))),
                    // An image the movie doesn't name: nothing.
                    _ => continue,
                },
            },
        };
        let c = colour.apply(tint);
        if c[3] <= 0.0 {
            continue;
        }
        let corners = t.corners.map(|p| matrix.apply(p));
        let uv = t.corners.map(|p| match image {
            Some((img, inv)) => {
                let [u, v] = inv.apply(p);
                let [x0, y0, x1, y1] = img.part;
                [
                    x0 + u / img.width.max(1.0) * (x1 - x0),
                    y0 + v / img.height.max(1.0) * (y1 - y0),
                ]
            }
            None => [0.0, 0.0],
        });
        let texture = image.map(|(img, _)| img.texture.clone());
        push_clipped(out, corners, uv, c, texture, clip);
    }
}

/// Adds a triangle to `out`, cut to `clip` if given.
pub fn push_clipped(
    out: &mut Vec<Triangle>,
    corners: Corners,
    uv: Corners,
    colour: [f32; 4],
    texture: Option<String>,
    clip: Option<[f32; 4]>,
) {
    match clip {
        None => out.push(Triangle {
            corners,
            uv,
            colour,
            texture,
        }),
        Some(rect) => {
            for (corners, uv) in clip_triangle(corners, uv, rect) {
                out.push(Triangle {
                    corners,
                    uv,
                    colour,
                    texture: texture.clone(),
                });
            }
        }
    }
}

/// The box round some triangles.
fn bounds(tris: &[Triangle]) -> Option<[f32; 4]> {
    let mut it = tris.iter().flat_map(|t| t.corners);
    let first = it.next()?;
    Some(it.fold([first[0], first[1], first[0], first[1]], |b, p| {
        [
            b[0].min(p[0]),
            b[1].min(p[1]),
            b[2].max(p[0]),
            b[3].max(p[1]),
        ]
    }))
}

/// A triangle's three points (corners, or their texture coordinates).
pub type Corners = [[f32; 2]; 3];

/// The parts of a triangle inside `rect` (x0, y0, x1, y1), with their
/// texture coordinates, as triangles.
fn clip_triangle(
    corners: [[f32; 2]; 3],
    uv: [[f32; 2]; 3],
    rect: [f32; 4],
) -> Vec<(Corners, Corners)> {
    // (point, uv) round the polygon, cut by each side in turn.
    let mut poly: Vec<([f32; 2], [f32; 2])> = (0..3).map(|i| (corners[i], uv[i])).collect();
    let sides: [(usize, f32, bool); 4] = [
        (0, rect[0], true),
        (0, rect[2], false),
        (1, rect[1], true),
        (1, rect[3], false),
    ];
    for (axis, at, keep_above) in sides {
        let inside = |p: &[f32; 2]| {
            if keep_above {
                p[axis] >= at
            } else {
                p[axis] <= at
            }
        };
        let mut next = Vec::with_capacity(poly.len() + 2);
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            let (ia, ib) = (inside(&a.0), inside(&b.0));
            if ia {
                next.push(a);
            }
            if ia != ib {
                let t = (at - a.0[axis]) / (b.0[axis] - a.0[axis]);
                let lerp =
                    |p: [f32; 2], q: [f32; 2]| [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
                next.push((lerp(a.0, b.0), lerp(a.1, b.1)));
            }
        }
        poly = next;
        if poly.len() < 3 {
            return Vec::new();
        }
    }
    (1..poly.len() - 1)
        .map(|i| {
            (
                [poly[0].0, poly[i].0, poly[i + 1].0],
                [poly[0].1, poly[i].1, poly[i + 1].1],
            )
        })
        .collect()
}

#[cfg(test)]
mod tests;
