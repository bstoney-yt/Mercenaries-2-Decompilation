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

use std::collections::{BTreeMap, HashMap};

use crate::gfx::{Character, Movie, tag};

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
fn stroke(a: [f32; 2], b: [f32; 2], half: f32) -> [[[f32; 2]; 3]; 2] {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = dx.hypot(dy).max(1e-6);
    let (nx, ny) = (-dy / len * half, dx / len * half);
    let p = [a[0] + nx, a[1] + ny];
    let q = [b[0] + nx, b[1] + ny];
    let r = [b[0] - nx, b[1] - ny];
    let s = [a[0] - nx, a[1] - ny];
    [[p, q, r], [p, r, s]]
}

/// A character placed on a timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub character: u16,
    pub matrix: Matrix,
    pub colour: ColourTransform,
    /// Masks the depths above it up to this one.
    pub clip_depth: Option<u16>,
    pub name: Option<String>,
}

/// A timeline's first frame: what is placed at each depth.
pub type Frame = BTreeMap<u16, Placed>;

/// Reads a timeline's tags up to its first `ShowFrame`, as its first
/// frame's display list.
pub fn first_frame(tags: &[(u16, &[u8])]) -> Frame {
    let mut frame = Frame::new();
    for &(code, data) in tags {
        match code {
            tag::SHOW_FRAME => break,
            tag::PLACE_OBJECT => {
                let mut b = Bits::new(data);
                let character = b.u16();
                let depth = b.u16();
                let matrix = b.matrix();
                let colour = if b.done() {
                    ColourTransform::IDENTITY
                } else {
                    b.cxform(false)
                };
                frame.insert(
                    depth,
                    Placed {
                        character,
                        matrix,
                        colour,
                        clip_depth: None,
                        name: None,
                    },
                );
            }
            tag::PLACE_OBJECT2 | tag::PLACE_OBJECT3 => {
                place(&mut frame, data, code == tag::PLACE_OBJECT3)
            }
            tag::REMOVE_OBJECT => {
                let mut b = Bits::new(data);
                b.u16();
                frame.remove(&b.u16());
            }
            tag::REMOVE_OBJECT2 => {
                frame.remove(&Bits::new(data).u16());
            }
            _ => {}
        }
    }
    frame
}

fn place(frame: &mut Frame, data: &[u8], three: bool) {
    let mut b = Bits::new(data);
    let flags = b.u8();
    let flags3 = if three { b.u8() } else { 0 };
    let depth = b.u16();
    if three && (flags3 & 8 != 0 || (flags3 & 16 != 0 && flags & 2 != 0)) {
        // A class name (or one for an image): not needed here.
        b.cstr();
    }
    let character = (flags & 2 != 0).then(|| b.u16());
    let matrix = (flags & 4 != 0).then(|| b.matrix());
    let colour = (flags & 8 != 0).then(|| b.cxform(true));
    if flags & 16 != 0 {
        b.u16();
    }
    let name = (flags & 32 != 0).then(|| b.cstr());
    let clip_depth = (flags & 64 != 0).then(|| b.u16());
    let moving = flags & 1 != 0;
    match (moving, character) {
        // A new character at the depth (replacing any there).
        (_, Some(character)) => {
            let old = frame.remove(&depth);
            let keep = |o: &Option<Placed>| o.as_ref().filter(|_| moving).cloned();
            let kept = keep(&old);
            frame.insert(
                depth,
                Placed {
                    character,
                    matrix: matrix
                        .or(kept.as_ref().map(|p| p.matrix))
                        .unwrap_or(Matrix::IDENTITY),
                    colour: colour
                        .or(kept.as_ref().map(|p| p.colour))
                        .unwrap_or(ColourTransform::IDENTITY),
                    clip_depth: clip_depth.or(kept.as_ref().and_then(|p| p.clip_depth)),
                    name: name.or(kept.and_then(|p| p.name)),
                },
            );
        }
        // The character there, changed.
        (true, None) => {
            if let Some(p) = frame.get_mut(&depth) {
                if let Some(m) = matrix {
                    p.matrix = m;
                }
                if let Some(c) = colour {
                    p.colour = c;
                }
                if let Some(d) = clip_depth {
                    p.clip_depth = Some(d);
                }
                if name.is_some() {
                    p.name = name;
                }
            }
        }
        (false, None) => {}
    }
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
        if code == tag::END {
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
    pub width: f32,
    pub height: f32,
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

/// A movie's characters, ready to draw.
#[derive(Clone, Debug, Default)]
pub struct Library {
    pub size: [f32; 2],
    pub shapes: HashMap<u16, (Shape, Vec<ShapeTriangle>)>,
    pub sprites: HashMap<u16, Frame>,
    pub images: HashMap<u16, Image>,
    pub root: Frame,
}

/// Sprites inside sprites go no deeper than this.
const MAX_NESTING: usize = 32;

impl Library {
    /// The characters of `movie` and its first frame.
    pub fn new(movie: &Movie) -> Self {
        let mut lib = Library {
            size: {
                let (w, h) = movie.frame_size.size_px();
                [w, h]
            },
            ..Library::default()
        };
        for (id, c) in movie.summary().characters {
            if let Character::ExternalImage(e) = c {
                let stem = e
                    .file_name
                    .rsplit_once('.')
                    .map_or(e.file_name.as_str(), |(s, _)| s);
                let texture = if stem.is_empty() {
                    e.export_name.clone()
                } else {
                    stem.to_owned()
                };
                lib.images.insert(
                    id,
                    Image {
                        texture,
                        width: f32::from(e.width),
                        height: f32::from(e.height),
                    },
                );
            }
        }
        let tags: Vec<(u16, &[u8])> = movie.tags.iter().map(|t| (t.code, movie.data(t))).collect();
        lib.add(&tags);
        lib.root = first_frame(&tags);
        lib
    }

    /// The shapes and sprites defined in `tags`.
    fn add(&mut self, tags: &[(u16, &[u8])]) {
        for &(code, data) in tags {
            let version = match code {
                tag::DEFINE_SHAPE => 1,
                tag::DEFINE_SHAPE2 => 2,
                tag::DEFINE_SHAPE3 => 3,
                tag::DEFINE_SHAPE4 => 4,
                tag::DEFINE_SPRITE => {
                    let Some(id) = data.get(..2).map(|b| u16::from_le_bytes([b[0], b[1]])) else {
                        continue;
                    };
                    let inner = self::tags(data.get(4..).unwrap_or(&[]));
                    self.add(&inner);
                    self.sprites.insert(id, first_frame(&inner));
                    continue;
                }
                _ => continue,
            };
            let (id, shape) = Shape::parse(data, version);
            let mesh = shape.mesh();
            self.shapes.insert(id, (shape, mesh));
        }
    }

    /// The first frame as triangles, in the movie's pixels through
    /// `matrix`.
    pub fn draw(&self, matrix: &Matrix) -> Vec<Triangle> {
        let mut out = Vec::new();
        self.draw_frame(
            &self.root,
            matrix,
            &ColourTransform::IDENTITY,
            None,
            0,
            &mut out,
        );
        out
    }

    fn draw_frame(
        &self,
        frame: &Frame,
        matrix: &Matrix,
        colour: &ColourTransform,
        clip: Option<[f32; 4]>,
        nesting: usize,
        out: &mut Vec<Triangle>,
    ) {
        if nesting > MAX_NESTING {
            return;
        }
        // Masks under way: up to which depth, and the box they clip to.
        let mut masks: Vec<(u16, [f32; 4])> = Vec::new();
        for (&depth, p) in frame {
            masks.retain(|(to, _)| depth <= *to);
            let m = matrix.then(&p.matrix);
            let c = colour.then(&p.colour);
            if let Some(to) = p.clip_depth {
                let mut shape = Vec::new();
                self.draw_character(p.character, &m, &c, None, nesting, &mut shape);
                if let Some(b) = bounds(&shape) {
                    masks.push((to, b));
                }
                continue;
            }
            let clip = masks.iter().fold(clip, |acc, (_, b)| {
                Some(match acc {
                    Some(a) => [
                        a[0].max(b[0]),
                        a[1].max(b[1]),
                        a[2].min(b[2]),
                        a[3].min(b[3]),
                    ],
                    None => *b,
                })
            });
            self.draw_character(p.character, &m, &c, clip, nesting, out);
        }
    }

    fn draw_character(
        &self,
        id: u16,
        matrix: &Matrix,
        colour: &ColourTransform,
        clip: Option<[f32; 4]>,
        nesting: usize,
        out: &mut Vec<Triangle>,
    ) {
        if let Some(frame) = self.sprites.get(&id) {
            self.draw_frame(frame, matrix, colour, clip, nesting + 1, out);
            return;
        }
        let Some((shape, mesh)) = self.shapes.get(&id) else {
            return;
        };
        for t in mesh {
            // The colour, and for an image its texture, the shape's pixels
            // to the image's and the image's size.
            let (tint, image) = match t.style {
                Style::Line(i) => (shape.lines[i].colour, None),
                Style::Fill(i) => match &shape.fills[i] {
                    Fill::Solid(c) | Fill::Gradient(c) => (*c, None),
                    Fill::Bitmap { id, matrix: bm, .. } => {
                        match (self.images.get(id), bm.inverse()) {
                            (Some(img), Some(inv)) => ([1.0; 4], Some((img, inv))),
                            // An image the movie doesn't name: nothing.
                            _ => continue,
                        }
                    }
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
                    [u / img.width.max(1.0), v / img.height.max(1.0)]
                }
                None => [0.0, 0.0],
            });
            let texture = image.map(|(img, _)| img.texture.clone());
            match clip {
                None => out.push(Triangle {
                    corners,
                    uv,
                    colour: c,
                    texture,
                }),
                Some(rect) => {
                    for (corners, uv) in clip_triangle(corners, uv, rect) {
                        out.push(Triangle {
                            corners,
                            uv,
                            colour: c,
                            texture: texture.clone(),
                        });
                    }
                }
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
type Corners = [[f32; 2]; 3];

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
mod tests {
    use super::*;

    /// Writes SWF fields: bytes little-endian, bit fields most significant
    /// bit first.
    #[derive(Default)]
    struct Writer {
        bytes: Vec<u8>,
        bit: usize,
    }

    impl Writer {
        fn ub(&mut self, n: usize, v: u32) {
            for i in (0..n).rev() {
                if self.bit.is_multiple_of(8) {
                    self.bytes.push(0);
                }
                if (v >> i) & 1 == 1 {
                    let last = self.bytes.len() - 1;
                    self.bytes[last] |= 0x80 >> (self.bit % 8);
                }
                self.bit += 1;
            }
        }
        fn sb(&mut self, n: usize, v: i32) {
            self.ub(n, (v as u32) & ((1u64 << n) - 1) as u32);
        }
        fn align(&mut self) {
            self.bit = self.bit.div_ceil(8) * 8;
        }
        fn u8(&mut self, v: u8) {
            self.align();
            self.bytes.push(v);
            self.bit += 8;
        }
        fn u16(&mut self, v: u16) {
            self.u8(v as u8);
            self.u8((v >> 8) as u8);
        }
        fn rect(&mut self, x0: i32, x1: i32, y0: i32, y1: i32) {
            self.align();
            self.ub(5, 16);
            for v in [x0, x1, y0, y1] {
                self.sb(16, v);
            }
            self.align();
        }
        /// A matrix: scale (16.16) and translation (twips).
        fn matrix(&mut self, scale: Option<(f32, f32)>, tx: i32, ty: i32) {
            self.align();
            match scale {
                Some((sx, sy)) => {
                    self.ub(1, 1);
                    self.ub(5, 24);
                    self.sb(24, (sx * 65536.0) as i32);
                    self.sb(24, (sy * 65536.0) as i32);
                }
                None => self.ub(1, 0),
            }
            self.ub(1, 0);
            self.ub(5, 16);
            self.sb(16, tx);
            self.sb(16, ty);
            self.align();
        }
        fn style_change(
            &mut self,
            move_to: Option<(i32, i32)>,
            fill0: Option<u32>,
            fill1: Option<u32>,
        ) {
            self.ub(1, 0);
            self.ub(1, 0); // new styles
            self.ub(1, 0); // line
            self.ub(1, u32::from(fill1.is_some()));
            self.ub(1, u32::from(fill0.is_some()));
            self.ub(1, u32::from(move_to.is_some()));
            if let Some((x, y)) = move_to {
                self.ub(5, 16);
                self.sb(16, x);
                self.sb(16, y);
            }
            if let Some(f) = fill0 {
                self.ub(2, f);
            }
            if let Some(f) = fill1 {
                self.ub(2, f);
            }
        }
        fn line_to(&mut self, dx: i32, dy: i32) {
            self.ub(1, 1);
            self.ub(1, 1);
            self.ub(4, 14); // 16 bits
            self.ub(1, 1);
            self.sb(16, dx);
            self.sb(16, dy);
        }
        fn curve_to(&mut self, cx: i32, cy: i32, ax: i32, ay: i32) {
            self.ub(1, 1);
            self.ub(1, 0);
            self.ub(4, 14);
            for v in [cx, cy, ax, ay] {
                self.sb(16, v);
            }
        }
        fn end(&mut self) {
            self.ub(6, 0);
            self.align();
        }
    }

    /// A `DefineShape3` with one solid red fill and the edges `draw` writes
    /// (twips; two fill bits, no line bits).
    fn shape(draw: impl FnOnce(&mut Writer)) -> Shape {
        let mut w = Writer::default();
        w.u16(7);
        w.rect(0, 400, 0, 400);
        w.u8(1);
        w.u8(0x00);
        for v in [255, 0, 0, 255] {
            w.u8(v);
        }
        w.u8(0);
        w.align();
        w.ub(4, 2);
        w.ub(4, 0);
        draw(&mut w);
        w.end();
        let (id, s) = Shape::parse(&w.bytes, 3);
        assert_eq!(id, 7);
        s
    }

    fn area(tris: &[ShapeTriangle]) -> f32 {
        tris.iter()
            .map(|t| {
                let [a, b, c] = t.corners;
                ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
            })
            .sum()
    }

    /// A square `side` twips across at (`x`, `y`), going round clockwise
    /// on screen.
    fn square(w: &mut Writer, x: i32, y: i32, side: i32, fill0: Option<u32>, fill1: Option<u32>) {
        w.style_change(Some((x, y)), fill0, fill1);
        w.line_to(side, 0);
        w.line_to(0, side);
        w.line_to(-side, 0);
        w.line_to(0, -side);
    }

    #[test]
    fn a_square_fills_its_area() {
        let s = shape(|w| square(w, 0, 0, 200, None, Some(1)));
        assert_eq!(s.edges.len(), 4);
        assert_eq!(s.fills[1], Fill::Solid([1.0, 0.0, 0.0, 1.0]));
        let mesh = s.mesh();
        assert!((area(&mesh) - 100.0).abs() < 1e-3, "{}", area(&mesh));
        // The same edges with the fill on their other side fill the same.
        let s = shape(|w| square(w, 0, 0, 200, Some(1), None));
        assert!((area(&s.mesh()) - 100.0).abs() < 1e-3);
    }

    #[test]
    fn an_inner_square_drawn_the_other_way_is_a_hole() {
        // A 20 px square with a 10 px one inside, its fill on the outside
        // (fill0 going the same way round).
        let s = shape(|w| {
            square(w, 0, 0, 400, None, Some(1));
            square(w, 100, 100, 200, Some(1), Some(0));
        });
        assert!(
            (area(&s.mesh()) - 300.0).abs() < 1e-3,
            "{}",
            area(&s.mesh())
        );
        // An edge with the fill on both sides adds nothing.
        let s = shape(|w| {
            square(w, 0, 0, 200, None, Some(1));
            w.style_change(Some((0, 100)), Some(1), Some(1));
            w.line_to(200, 0);
        });
        assert!((area(&s.mesh()) - 100.0).abs() < 1e-3);
    }

    #[test]
    fn curves_are_flattened_inside_their_hull() {
        // A straight edge and a curve back to its start.
        let s = shape(|w| {
            w.style_change(Some((0, 0)), None, Some(1));
            w.line_to(400, 0);
            w.curve_to(0, 400, -400, -400);
        });
        assert!(s.edges[1].control.is_some());
        let a = area(&s.mesh());
        // A quadratic's area is two thirds of its control triangle's
        // (20 x 20 px / 2 = 200).
        assert!((a - 133.33).abs() < 8.0, "{a}");
    }

    #[test]
    fn placing_moving_and_removing_by_depth() {
        let mut place2 = Writer::default();
        // PlaceObject2: has character, has matrix; depth 3; character 7.
        place2.u8(0b0000_0110);
        place2.u16(3);
        place2.u16(7);
        place2.matrix(None, 200, 400);
        let mut moved = Writer::default();
        moved.u8(0b0000_0101);
        moved.u16(3);
        moved.matrix(Some((2.0, 2.0)), 0, 0);
        let mut other = Writer::default();
        other.u8(0b0000_0010);
        other.u16(5);
        other.u16(9);
        let remove = 5u16.to_le_bytes();
        let frame = first_frame(&[
            (tag::PLACE_OBJECT2, &place2.bytes),
            (tag::PLACE_OBJECT2, &other.bytes),
            (tag::PLACE_OBJECT2, &moved.bytes),
            (tag::REMOVE_OBJECT2, &remove),
            (tag::SHOW_FRAME, &[]),
            (tag::PLACE_OBJECT2, &other.bytes),
        ]);
        assert_eq!(frame.len(), 1);
        let p = &frame[&3];
        assert_eq!(p.character, 7);
        assert_eq!((p.matrix.a, p.matrix.tx), (2.0, 0.0));
    }

    #[test]
    fn a_mask_clips_what_it_masks_and_bitmaps_take_their_image() {
        let mut lib = Library::default();
        let square = shape(|w| square(w, 0, 0, 400, None, Some(1)));
        let mesh = square.mesh();
        lib.shapes.insert(1, (square.clone(), mesh));
        // The same square filled with a 20 x 20 image drawn at its size.
        let mut pic = square.clone();
        pic.fills[1] = Fill::Bitmap {
            id: 9,
            matrix: Matrix::IDENTITY,
            repeat: false,
        };
        let pic_mesh = pic.mesh();
        lib.shapes.insert(2, (pic, pic_mesh));
        lib.images.insert(
            9,
            Image {
                texture: "pda_titles".into(),
                width: 20.0,
                height: 20.0,
            },
        );
        let at = |x: f32, y: f32| Matrix {
            tx: x,
            ty: y,
            ..Matrix::IDENTITY
        };
        // Depth 1 masks depths 2 and 3: the square at (10, 10) is cut to
        // the mask's 20 px square at the origin.
        for (depth, character, matrix, clip_depth) in [
            (1, 1, at(0.0, 0.0), Some(3)),
            (2, 2, at(10.0, 10.0), None),
            (4, 1, at(30.0, 0.0), None),
        ] {
            lib.root.insert(
                depth,
                Placed {
                    character,
                    matrix,
                    colour: ColourTransform::IDENTITY,
                    clip_depth,
                    name: None,
                },
            );
        }
        let tris = lib.draw(&Matrix::IDENTITY);
        let (pic, red): (Vec<Triangle>, Vec<Triangle>) =
            tris.into_iter().partition(|t| t.texture.is_some());
        assert_eq!(bounds(&pic).unwrap(), [10.0, 10.0, 20.0, 20.0]);
        assert_eq!(pic[0].texture.as_deref(), Some("pda_titles"));
        // Texture coordinates follow the image: the clipped corner (20, 20)
        // is the image's middle.
        let mid = pic
            .iter()
            .flat_map(|t| t.corners.into_iter().zip(t.uv))
            .find(|(p, _)| *p == [20.0, 20.0])
            .unwrap();
        assert!((mid.1[0] - 0.5).abs() < 1e-5 && (mid.1[1] - 0.5).abs() < 1e-5);
        // The square past the mask's depths is whole; the mask is not drawn.
        assert_eq!(bounds(&red).unwrap(), [30.0, 0.0, 50.0, 20.0]);
    }

    #[test]
    fn transforms_compose_and_invert() {
        let half = ColourTransform {
            mult: [0.5; 4],
            add: [0.0; 4],
        };
        let lift = ColourTransform {
            mult: [1.0; 4],
            add: [0.25, 0.0, 0.0, 0.0],
        };
        let c = half.then(&lift).apply([1.0, 1.0, 1.0, 1.0]);
        assert_eq!(c, [0.625, 0.5, 0.5, 0.5]);
        let m = Matrix {
            a: 2.0,
            d: 2.0,
            tx: 5.0,
            ..Matrix::IDENTITY
        };
        let inv = m.inverse().unwrap();
        assert_eq!(inv.apply(m.apply([3.0, 4.0])), [3.0, 4.0]);
    }
}
