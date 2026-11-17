//! `flash`: a Flash movie drawn offscreen (see the example's docs).

use std::collections::HashMap;
use std::io::{Read, Seek};
use std::rc::Rc;

use m2_assets::flash::{self, ColourTransform, Fill, Library, Matrix, Player, gradient};
use m2_assets::gfx::{MOVIE_TYPE, Movie, movie_from_asset};
use m2_assets::{BlockContents, WadArchive, name_hash};

use super::{Canvas, Error, Image, load_texture, png};

/// A movie by its exporter name: the first copy named exactly so, else the
/// first in any case.
fn find_movie<R: Read + Seek>(wad: &mut WadArchive<R>, name: &str) -> Result<Movie, Error> {
    let mut blocks: Vec<usize> = wad
        .index()
        .assets()
        .iter()
        .filter(|a| wad.index().type_hash(a) == MOVIE_TYPE)
        .map(|a| usize::from(a.block))
        .collect();
    blocks.sort_unstable();
    blocks.dedup();
    let mut any_case = None;
    for b in blocks {
        let decoded = wad.read_block(b)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if e.type_hash != MOVIE_TYPE {
                continue;
            }
            let Ok(movie) = movie_from_asset(data) else {
                continue;
            };
            let Some(swf) = movie.summary().exporter.map(|x| x.4) else {
                continue;
            };
            if swf == name {
                return Ok(movie);
            }
            if any_case.is_none() && swf.eq_ignore_ascii_case(name) {
                any_case = Some(movie);
            }
        }
    }
    any_case.ok_or_else(|| format!("no movie named {name}").into())
}

/// Draws movie `name` into `out`.
pub fn flash_png<R: Read + Seek>(
    wad: &mut WadArchive<R>,
    name: &str,
    out: &str,
    options: &[String],
) -> Result<(), Error> {
    let movie = find_movie(wad, name)?;
    let lib = Rc::new(Library::new(&movie));
    let gradients = options.iter().any(|o| o == "--gradients");
    let scale: f32 = match options.iter().position(|o| o == "--scale") {
        Some(i) => options.get(i + 1).ok_or("--scale needs a value")?.parse()?,
        None => 1.0,
    };
    let mut triangles = Vec::new();
    let (width, height) = if gradients {
        const TILE: f32 = 256.0;
        let gradient = |fills: &[Fill]| fills.iter().any(|f| matches!(f, Fill::Gradient(..)));
        // Shapes, then each morph shape at its start and its end.
        let mut shapes: Vec<(String, flash::player::MorphMesh)> = Vec::new();
        let mut ids: Vec<u16> = lib
            .shapes
            .iter()
            .filter(|(_, (s, _))| gradient(&s.fills))
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        for id in ids {
            shapes.push((format!("shape {id}"), Rc::new(lib.shapes[&id].clone())));
        }
        let mut morphs: Vec<u16> = lib
            .morphs
            .iter()
            .filter(|(_, m)| gradient(&m.start.fills))
            .map(|(id, _)| *id)
            .collect();
        morphs.sort_unstable();
        for id in morphs {
            for ratio in [0, u16::MAX] {
                if let Some(m) = lib.morph(id, ratio) {
                    shapes.push((format!("morph {id} at {ratio}"), m));
                }
            }
        }
        for (k, (label, made)) in shapes.iter().enumerate() {
            let (shape, mesh) = &**made;
            let id = label;
            let [x0, y0, x1, y1] = shape.bounds;
            let fit = (TILE - 16.0) / (x1 - x0).max(y1 - y0).max(1.0);
            let m = Matrix {
                a: fit,
                d: fit,
                tx: k as f32 * TILE + 8.0 - x0 * fit,
                ty: 8.0 - y0 * fit,
                ..Matrix::IDENTITY
            };
            println!(
                "{id}: bounds {:?} px, fills {:?}",
                shape.bounds, shape.fills
            );
            for f in &shape.fills {
                if let Fill::Gradient(g) = f
                    && let Some(e) = &g.image
                {
                    compare_made(wad, g, &e.texture);
                }
            }
            lib.draw_shape(shape, mesh, &m, &ColourTransform::IDENTITY, &mut triangles);
        }
        ((TILE as u32 * shapes.len() as u32).max(1), TILE as u32)
    } else {
        let m = Matrix {
            a: scale,
            d: scale,
            ..Matrix::IDENTITY
        };
        Player::new(Rc::clone(&lib)).draw(&m, &flash::FontBook::default(), &mut triangles);
        (
            (lib.size[0] * scale).round() as u32,
            (lib.size[1] * scale).round() as u32,
        )
    };
    let mut canvas = Canvas::new(width, height);
    let mut textures: HashMap<String, Option<Image>> = HashMap::new();
    for t in &triangles {
        let image = match &t.texture {
            Some(n) => textures
                .entry(n.clone())
                .or_insert_with(|| {
                    let i = match gradient::made_pixels(n) {
                        Some((width, height, rgba)) => Some(Image {
                            width,
                            height,
                            rgba,
                        }),
                        None => load_texture(wad, name_hash(n)),
                    };
                    match &i {
                        Some(i) => println!("texture {n}: {}x{}", i.width, i.height),
                        None => println!("texture {n}: not found"),
                    }
                    i
                })
                .as_ref(),
            None => None,
        };
        if t.texture.is_some() && image.is_none() {
            continue;
        }
        canvas.triangle(t, image);
    }
    std::fs::write(out, png(width, height, &canvas.rgba))?;
    println!("wrote {out}: {} triangles", triangles.len());
    Ok(())
}

/// How far the texture this code makes for `g` is from the exporter's
/// (`texture`): the largest and mean channel difference, texel by texel
/// (an exported linear texture's rows each against the ramp).
fn compare_made<R: Read + Seek>(wad: &mut WadArchive<R>, g: &flash::Gradient, texture: &str) {
    let Some(theirs) = load_texture(wad, name_hash(texture)) else {
        println!("  {texture}: not found");
        return;
    };
    let mut ours = g.clone();
    ours.image = None;
    let (w, h, px) = ours.pixels();
    if w != theirs.width || (h != theirs.height && h != 1) {
        println!(
            "  {texture}: {}x{}, made {w}x{h}: sizes differ",
            theirs.width, theirs.height
        );
        return;
    }
    let (mut max, mut sum, mut n) = (0u8, 0u64, 0u64);
    let mut worst: Vec<(u8, u32, u32)> = Vec::new();
    for y in 0..theirs.height {
        for x in 0..w {
            let i = ((y * theirs.width + x) * 4) as usize;
            let j = (((y % h) * w + x) * 4) as usize;
            for k in 0..4 {
                let d = theirs.rgba[i + k].abs_diff(px[j + k]);
                if k == 0 {
                    worst.push((d, x, y));
                }
                max = max.max(d);
                sum += u64::from(d);
                n += 1;
            }
        }
    }
    println!(
        "  {texture} {}x{} against made {w}x{h}: largest channel difference {max}, mean {:.3}",
        theirs.width,
        theirs.height,
        sum as f64 / n.max(1) as f64
    );
    worst.sort_by_key(|w| std::cmp::Reverse(w.0));
    for &(d, x, y) in worst.iter().take(6) {
        let i = ((y * theirs.width + x) * 4) as usize;
        let j = (((y % h) * w + x) * 4) as usize;
        println!(
            "    texel {x},{y}: theirs {:?} made {:?} (red differs by {d})",
            &theirs.rgba[i..i + 4],
            &px[j..j + 4]
        );
    }
}

impl Image {
    /// The texel colour at `u`, `v` (0..=1 across, repeating or clamped
    /// at the edges), between the four nearest texels.
    fn bilinear(&self, u: f32, v: f32, clamp: bool) -> [f32; 4] {
        let (w, h) = (i64::from(self.width), i64::from(self.height));
        let x = u * w as f32 - 0.5;
        let y = v * h as f32 - 0.5;
        let (fx, fy) = (x - x.floor(), y - y.floor());
        let (xi, yi) = (x.floor() as i64, y.floor() as i64);
        let at = |x: i64, y: i64| -> [f32; 4] {
            let (x, y) = if clamp {
                (x.clamp(0, w - 1), y.clamp(0, h - 1))
            } else {
                (x.rem_euclid(w), y.rem_euclid(h))
            };
            let i = ((y * w + x) * 4) as usize;
            std::array::from_fn(|k| f32::from(self.rgba[i + k]) / 255.0)
        };
        let (a, b, c, d) = (
            at(xi, yi),
            at(xi + 1, yi),
            at(xi, yi + 1),
            at(xi + 1, yi + 1),
        );
        std::array::from_fn(|k| {
            let top = a[k] + (b[k] - a[k]) * fx;
            let bottom = c[k] + (d[k] - c[k]) * fx;
            top + (bottom - top) * fy
        })
    }
}

impl Canvas {
    /// Draws a triangle (corners in pixels): its colour, times `img`
    /// sampled at its corners' texture coordinates if given; over what is
    /// there, or multiplying it.
    fn triangle(&mut self, t: &flash::Triangle, img: Option<&Image>) {
        let (p, uv, colour) = (t.corners, t.uv, t.colour);
        let area =
            (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
        if area.abs() < 1e-9 {
            return;
        }
        let xs = p.map(|q| q[0]);
        let ys = p.map(|q| q[1]);
        let lo = |v: [f32; 3]| v.iter().copied().fold(f32::MAX, f32::min).floor().max(0.0) as i64;
        let hi = |v: [f32; 3], n: u32| {
            (v.iter().copied().fold(f32::MIN, f32::max).ceil() as i64).min(i64::from(n))
        };
        for y in lo(ys)..hi(ys, self.height) {
            for x in lo(xs)..hi(xs, self.width) {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let w = |a: [f32; 2], b: [f32; 2]| {
                    ((b[0] - a[0]) * (py - a[1]) - (px - a[0]) * (b[1] - a[1])) / area
                };
                let (l0, l1, l2) = (w(p[1], p[2]), w(p[2], p[0]), w(p[0], p[1]));
                if l0 < 0.0 || l1 < 0.0 || l2 <= 0.0 {
                    continue;
                }
                let mut c = colour;
                if let Some(img) = img {
                    let u = l0 * uv[0][0] + l1 * uv[1][0] + l2 * uv[2][0];
                    let v = l0 * uv[0][1] + l1 * uv[1][1] + l2 * uv[2][1];
                    let s = img.bilinear(u, v, t.clamp);
                    for k in 0..4 {
                        c[k] *= s[k];
                    }
                }
                if t.multiply {
                    let i = (y as usize * self.width as usize + x as usize) * 4;
                    let a = c[3].clamp(0.0, 1.0);
                    for (d, c) in self.rgba[i..i + 3].iter_mut().zip(c) {
                        let v = f32::from(*d);
                        let m = v * c.clamp(0.0, 1.0);
                        *d = (v + (m - v) * a).round() as u8;
                    }
                } else {
                    self.put(x, y, c);
                }
            }
        }
    }
}
