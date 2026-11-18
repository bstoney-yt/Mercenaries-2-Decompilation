//! Flash gradient fills (`DefineShape` fill types 0x10 linear, 0x12
//! radial, 0x13 focal radial) as the game's Scaleform draws them: a
//! texture through the gradient's matrix, clamped at its edges.
//!
//! A gradient's colours lie along a 32768-twip square (-16384 to 16384 both
//! ways) that its matrix places in the shape. The texture is either one the
//! exporter made beforehand (`DefineExternalGradient`, a texture in the
//! archives, used when the movie's exporter flags say gradients were
//! exported) or one made from the gradient's stops ([`Gradient::pixels`]):
//! 256 x 1 for a linear gradient, a square for a radial one.
//!
//! # Provenance
//!
//! From the 360 code (`default.xex`):
//! - 0x8279A640 reads a fill style: for a gradient its matrix, then a byte
//!   whose bit 0x10 is the interpolation mode (linear RGB) and whose low
//!   four bits count the stops; the spread bits (0xC0) are not kept, and
//!   every gradient is drawn clamped (0x8279B598 sets clamp and linear
//!   filtering on its texture), so reflect and repeat draw as pad. Each
//!   stop is a ratio byte and an RGB(A) colour; a focal gradient ends with
//!   its focal point, a signed 8.8 number. When the movie's exporter info
//!   has flag 0x02 the fill takes the image the `DefineExternalGradient`
//!   loader (0x827864C8) registered for its gradient (numbered in the order
//!   the movie's shapes read them) instead of making one.
//! - 0x8279A2E8 makes the 256 colours of the ramp: before the first stop's
//!   ratio its colour; between two stops, entry `r0 + j - 1` (`j` from 1 to
//!   `r1 - r0`) is the earlier colour plus `(later - earlier) * j / (r1 -
//!   r0)` in whole numbers (0x8279A1F8); after the last stop's ratio, the
//!   last colour written. A stop whose ratio is below the one before counts
//!   from that one. With linear RGB the red, green and blue go through
//!   16-bit values raised to the power 2.17 before mixing and back after
//!   (0x8279A0C8, 0x82799468); alpha mixes as is.
//! - 0x8279AE30 makes the texture: linear, the ramp as 256 x 1; radial, N x
//!   N with texel (x, y) inside the one-texel border the ramp entry
//!   `floor(d * 256 + 0.5)` (at most 255), `d` the distance of the texel's
//!   centre from the middle over `N / 2 - 1`, and the border the last entry;
//!   focal, the same with `d` the focal ratio (0x827993D0 sets up its terms).
//!   N (0x827994D8) grows with the steepest colour change between stops,
//!   from a table of 16, 32, 64, 128 and 256.
//! - 0x8279B598 maps the shape to the texture: linear, texel `x / 128 +
//!   128` across 256; radial, `x / (32768 / (N - 2)) + N / 2` (the division
//!   in whole numbers), so the square's edge falls on the border.
//! - 0x8279A9D0 makes a morph's gradient between its two: ratios rounded,
//!   colours and the focal point mixed. Morphs never take an exported
//!   image (0x827B94A0 reads theirs without the lookup).
//!
//! Invented: when a radial size comes from the player's gradient settings
//! (fixed or adaptive) was not found; this takes the adaptive size (what
//! the code does when no settings are given).

use super::Matrix;

/// Half the gradient square, twips.
pub const HALF_SQUARE: f32 = 16384.0;
/// The ramp's length.
pub const RAMP: usize = 256;
/// The power linear-RGB gradients mix their colours at (0x8279AE30).
pub const LINEAR_RGB_GAMMA: f32 = 2.17;
/// Radial texture sizes by steepness (0x820690D0).
const RADIAL_SIZES: [u32; 18] = [
    16, 16, 32, 32, 64, 64, 64, 128, 128, 128, 128, 128, 128, 256, 256, 256, 256, 256,
];
/// A texture name made from a gradient begins so.
pub const MADE_PREFIX: &str = "#gradient:";

/// Which way a gradient's colours run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GradientKind {
    /// Left to right across the square.
    Linear,
    /// Out from the middle.
    Radial,
    /// Out from a focal point on the horizontal axis (-1 to 1 of the way
    /// to the edge) to the circle's edge.
    Focal(f32),
}

/// A gradient fill.
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub kind: GradientKind,
    /// The unit square (-1 to 1 both ways) to the shape's pixels.
    pub matrix: Matrix,
    /// Colours mixed in linear RGB.
    pub linear_rgb: bool,
    /// Ratio (0 to 255 along the square) and RGBA colour of each stop.
    pub stops: Vec<(u8, [u8; 4])>,
    /// Its place among the gradients its shape read (fills and lines).
    pub index: u16,
    /// The exporter's texture for it, if the movie uses those.
    pub image: Option<GradientImage>,
}

/// A texture the exporter made for a gradient (`DefineExternalGradient`).
#[derive(Clone, Debug, PartialEq)]
pub struct GradientImage {
    /// The texture's name: the file's without its extension.
    pub texture: String,
    /// Its size, texels (the square's side; a linear one's width).
    pub size: u16,
}

impl Gradient {
    /// The first stop's colour (0 to 1), or transparent black.
    pub fn first_colour(&self) -> [f32; 4] {
        self.stops
            .first()
            .map_or([0.0; 4], |(_, c)| c.map(|v| f32::from(v) / 255.0))
    }

    /// The 256 colours along the gradient (0x8279A2E8).
    pub fn ramp(&self) -> [[u8; 4]; RAMP] {
        let mut ramp = [[0u8; 4]; RAMP];
        let Some(&(first_ratio, first)) = self.stops.first() else {
            return ramp;
        };
        if self.stops.len() < 2 {
            return [first; RAMP];
        }
        let gamma = if self.linear_rgb {
            LINEAR_RGB_GAMMA
        } else {
            1.0
        };
        let to_linear = |v: u8| -> i32 {
            if gamma == 1.0 {
                i32::from(v) * 257
            } else {
                ((f32::from(v) * (1.0 / 255.0)).powf(gamma) * 65535.0 + 0.5).floor() as i32
            }
        };
        let inverse = 1.0 / gamma;
        let mut at = usize::from(first_ratio);
        ramp[..at].fill(first);
        let mut last = first;
        let mut earlier = first;
        for &(ratio, later) in &self.stops[1..] {
            let to = usize::from(ratio).max(at);
            let n = (to - at) as i32;
            for j in 1..=n {
                let mut c = [0u8; 4];
                for k in 0..3 {
                    let (a, b) = (to_linear(earlier[k]), to_linear(later[k]));
                    c[k] = if gamma == 1.0 {
                        // Whole-number mix, wrapping as a byte.
                        (i32::from(earlier[k]) + (b - a) * j / (n * 257)) as u8
                    } else {
                        let v = a + (b - a) * j / n;
                        let f = (v as f32 * (1.0 / 65535.0)).powf(inverse) * 255.0 + 0.5;
                        (f.floor() as i64 & 0xff) as u8
                    };
                }
                let (a, b) = (i32::from(earlier[3]) * 257, i32::from(later[3]) * 257);
                c[3] = (i32::from(earlier[3]) + (b - a) * j / (n * 257)) as u8;
                ramp[at + j as usize - 1] = c;
                last = c;
            }
            at = to;
            earlier = later;
        }
        if at < RAMP {
            ramp[at..].fill(last);
        }
        ramp
    }

    /// The side of a radial gradient's texture (0x827994D8): from the
    /// steepest change of any channel between two stops, per ratio.
    pub fn radial_size(&self) -> u32 {
        let mut steepest = 0.0f32;
        for w in self.stops.windows(2) {
            let (r0, c0) = w[0];
            let (r1, c1) = w[1];
            let span = f32::from(r1) - f32::from(r0);
            if span > 0.0 {
                let per = 1.0 / span;
                for k in 0..4 {
                    let change = (i32::from(c0[k]) - i32::from(c1[k])).abs() as f32 * per;
                    steepest = steepest.max(change);
                }
            }
        }
        if steepest == 0.0 {
            return 64;
        }
        if self.linear_rgb {
            steepest *= 1.5;
        }
        if let GradientKind::Focal(f) = self.kind
            && f.abs() > 0.5
        {
            steepest /= 1.01 - f.abs();
        }
        let i = (((steepest.max(0.0) + 0.18) * 5.0).sqrt() as usize).min(RADIAL_SIZES.len() - 1);
        RADIAL_SIZES[i]
    }

    /// The texture's size in texels: the exported one's, else what
    /// [`Gradient::pixels`] makes.
    pub fn texture_size(&self) -> [u32; 2] {
        match (&self.image, self.kind) {
            (Some(i), GradientKind::Linear) => [u32::from(i.size), 1],
            (Some(i), _) => [u32::from(i.size); 2],
            (None, GradientKind::Linear) => [RAMP as u32, 1],
            (None, _) => [self.radial_size(); 2],
        }
    }

    /// The texture made from the stops (0x8279AE30): width, height and
    /// RGBA bytes.
    pub fn pixels(&self) -> (u32, u32, Vec<u8>) {
        let ramp = self.ramp();
        if self.kind == GradientKind::Linear {
            return (RAMP as u32, 1, ramp.concat());
        }
        let n = self.radial_size();
        let mut out = vec![0u8; (n * n * 4) as usize];
        let mut put = |x: u32, y: u32, c: [u8; 4]| {
            let i = ((y * n + x) * 4) as usize;
            out[i..i + 4].copy_from_slice(&c);
        };
        let middle = n as f32 * 0.5;
        let entry = |t: f32| -> [u8; 4] {
            let i = (t * 256.0 + 0.5).floor();
            // Below zero reads as a huge unsigned number: the last entry.
            let i = if i < 0.0 {
                RAMP - 1
            } else {
                (i as usize).min(RAMP - 1)
            };
            ramp[i]
        };
        match self.kind {
            GradientKind::Focal(focal) => {
                let mut terms = FocalTerms::new(middle - 1.0, focal * (middle - 1.0));
                terms.settle();
                let over_radius = 1.0 / terms.radius;
                for y in 1..n - 1 {
                    let dy = (y as f32 - middle) + 0.5 - terms.fy;
                    for x in 1..n - 1 {
                        let dx = (x as f32 - middle) + 0.5 - terms.fx;
                        let cross = terms.fy * dx - terms.fx * dy;
                        let root = ((dx * dx + dy * dy) * terms.radius2 - cross * cross)
                            .abs()
                            .sqrt();
                        let t = (terms.fx * dx + root + terms.fy * dy) * terms.scale * over_radius;
                        put(x, y, entry(t));
                    }
                }
            }
            _ => {
                let over = 1.0 / (middle - 1.0);
                for y in 1..n - 1 {
                    let dy = (y as f32 - middle) + 0.5;
                    for x in 1..n - 1 {
                        let dx = (x as f32 - middle) + 0.5;
                        put(x, y, entry((dx * dx + dy * dy).sqrt() * over));
                    }
                }
            }
        }
        let edge = ramp[RAMP - 1];
        for i in 1..n {
            put(i, 0, edge);
            put(n - 1, i, edge);
            put(n - 1 - i, n - 1, edge);
            put(0, n - 1 - i, edge);
        }
        (n, n, out)
    }

    /// Where `p` (the shape's pixels) falls on the texture, 0 to 1 across
    /// and down (0x8279B598); `inverse` is [`Gradient::matrix`]'s inverse.
    pub fn uv(&self, inverse: &Matrix, p: [f32; 2]) -> [f32; 2] {
        let [x, y] = inverse.apply(p).map(|v| v * HALF_SQUARE);
        let [w, h] = self.texture_size();
        match self.kind {
            GradientKind::Linear => [(x / 128.0 + 128.0) / w as f32, 0.5],
            _ => {
                let n = w.max(3);
                let step = (32768 / (n - 2)) as f32;
                let half = n as f32 * 0.5;
                [(x / step + half) / w as f32, (y / step + half) / h as f32]
            }
        }
    }

    /// The texture it is drawn with: the exported one's name, else a name
    /// that [`made_pixels`] reads the made texture back from.
    pub fn texture_name(&self) -> String {
        if let Some(i) = &self.image {
            return i.texture.clone();
        }
        let mut s = String::from(MADE_PREFIX);
        match self.kind {
            GradientKind::Linear => s.push('l'),
            GradientKind::Radial => s.push('r'),
            GradientKind::Focal(f) => {
                s.push('f');
                s.push_str(&format!("{:08x}", f.to_bits()));
            }
        }
        s.push(if self.linear_rgb { 'g' } else { 'n' });
        for (ratio, c) in &self.stops {
            s.push_str(&format!(
                "/{ratio:02x}{:02x}{:02x}{:02x}{:02x}",
                c[0], c[1], c[2], c[3]
            ));
        }
        s
    }

    /// The gradient a [`Gradient::texture_name`] of a made texture names
    /// (its stops, kind and mode; no matrix).
    pub fn from_texture_name(name: &str) -> Option<Gradient> {
        let rest = name.strip_prefix(MADE_PREFIX)?;
        let mut parts = rest.split('/');
        let head = parts.next()?;
        let (kind, mode) = match head.as_bytes().first()? {
            b'l' => (GradientKind::Linear, &head[1..]),
            b'r' => (GradientKind::Radial, &head[1..]),
            b'f' => {
                let bits = u32::from_str_radix(head.get(1..9)?, 16).ok()?;
                (GradientKind::Focal(f32::from_bits(bits)), &head[9..])
            }
            _ => return None,
        };
        let linear_rgb = match mode {
            "g" => true,
            "n" => false,
            _ => return None,
        };
        let stops = parts
            .map(|p| {
                let v = u64::from_str_radix(p, 16).ok().filter(|_| p.len() == 10)?;
                let b = v.to_be_bytes();
                Some((b[3], [b[4], b[5], b[6], b[7]]))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Gradient {
            kind,
            matrix: Matrix::IDENTITY,
            linear_rgb,
            stops,
            index: 0,
            image: None,
        })
    }

    /// The gradient `t` of the way from `self` to `end` (a morph's, 0 to
    /// 1; 0x8279A9D0): ratios rounded, colours, focal point and matrix
    /// mixed; never an exported texture.
    pub fn mix(&self, end: &Gradient, t: f32) -> Gradient {
        let lerp = |a: f32, b: f32| a + (b - a) * t;
        let stops = self
            .stops
            .iter()
            .zip(&end.stops)
            .map(|(&(r0, c0), &(r1, c1))| {
                let ratio = (lerp(f32::from(r0), f32::from(r1)) + 0.5) as u8;
                let c = std::array::from_fn(|k| {
                    (lerp(f32::from(c0[k]), f32::from(c1[k])) + 0.5).clamp(0.0, 255.0) as u8
                });
                (ratio, c)
            })
            .collect();
        let kind = match (self.kind, end.kind) {
            (GradientKind::Focal(a), GradientKind::Focal(b)) => GradientKind::Focal(lerp(a, b)),
            (k, _) => k,
        };
        let (a, b) = (&self.matrix, &end.matrix);
        Gradient {
            kind,
            matrix: Matrix {
                a: lerp(a.a, b.a),
                b: lerp(a.b, b.b),
                c: lerp(a.c, b.c),
                d: lerp(a.d, b.d),
                tx: lerp(a.tx, b.tx),
                ty: lerp(a.ty, b.ty),
            },
            linear_rgb: self.linear_rgb,
            stops,
            index: self.index,
            image: None,
        }
    }
}

/// The pixels of a texture made from a gradient, by the name
/// [`Gradient::texture_name`] gave it; `None` for any other name.
pub fn made_pixels(name: &str) -> Option<(u32, u32, Vec<u8>)> {
    Gradient::from_texture_name(name).map(|g| g.pixels())
}

/// A focal gradient's terms (0x827993D0): the circle's radius in texels,
/// the focal point, and what scales the focal ratio.
struct FocalTerms {
    radius: f32,
    fx: f32,
    fy: f32,
    radius2: f32,
    scale: f32,
}

impl FocalTerms {
    fn new(radius: f32, fx: f32) -> Self {
        FocalTerms {
            radius,
            fx,
            fy: 0.0,
            radius2: 0.0,
            scale: 0.0,
        }
    }

    /// Fills in the squared radius and the scale; a focal point on the
    /// circle is moved a texel in first.
    fn settle(&mut self) {
        self.radius2 = self.radius * self.radius;
        let mut under = self.radius2 - (self.fx * self.fx + self.fy * self.fy);
        if under == 0.0 {
            let inward = |v: f32| match v {
                0.0 => 0.0,
                v if v >= 0.0 => v - 1.0,
                v => v + 1.0,
            };
            self.fx = inward(self.fx);
            self.fy = inward(self.fy);
            under = self.radius2 - (self.fx * self.fx + self.fy * self.fy);
        }
        self.scale = self.radius / under;
    }
}

#[cfg(test)]
mod tests;
