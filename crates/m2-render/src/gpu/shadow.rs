//! Sun shadows: cascaded shadow maps.
//!
//! The game's shaders take a four-cascade shadow map (`worldShadowProj`,
//! four matrices, and `shadowSlices`) and never darken highlights below a
//! quarter. Here the view is cut into four slices by distance; each gets an
//! orthographic map from the sun, fitted to a sphere around the slice and
//! moved in whole texels so it doesn't shimmer as the camera moves. The
//! slice distances, map size and biases are this renderer's choices.
//!
//! # Provenance
//!
//! Clean-room: standard cascaded shadow mapping, written against glam and
//! wgpu; only the cascade count and the quarter-strength highlight rule
//! come from the game's shader constants.

use m2_math::glam::{Mat4, Vec3, Vec4};

use crate::backend::View;
use crate::buffer::{VertexFormat, VertexLayout, VertexSemantic};

/// Cascades, as the game's shaders have.
pub const CASCADES: usize = 4;
/// Texels along each side of a cascade's map.
pub const SIZE: u32 = 2048;
/// Where each slice ends, as a fraction of the shadow distance: near slices
/// are small so shadows close by are sharp.
const SPLITS: [f32; CASCADES] = [0.05, 0.15, 0.4, 1.0];
/// Metres towards the sun beyond a slice's sphere that still cast into it
/// (hills and towers outside the view).
const REACH: f32 = 400.0;
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// One frame's cascades.
#[derive(Clone, Copy, Debug)]
pub struct Cascades {
    /// World to each cascade's clip space (depth 0 towards the sun).
    pub matrices: [Mat4; CASCADES],
    /// Rotation into light space, shared by every cascade.
    light: Mat4,
    /// Per cascade, its light-space box: x and y bounds, then the depth
    /// range along light-space -z.
    boxes: [[f32; 6]; CASCADES],
    /// View depth where each slice ends.
    pub splits: [f32; CASCADES],
    /// Metres per texel in each cascade.
    pub texels: [f32; CASCADES],
}

impl Cascades {
    /// Cascades covering `view` out to `distance` metres from the camera,
    /// for sunlight travelling along `sun` (pointing down). `None` when the
    /// sun is at or below the horizon.
    pub fn new(view: &View, sun: Vec3, distance: f32) -> Option<Self> {
        let dir = sun.try_normalize()?;
        if dir.y > -0.02 || distance <= 0.0 {
            return None;
        }
        let up = if dir.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
        let light = Mat4::look_at_rh(Vec3::ZERO, dir, up);
        let view_proj = view.projection * view.view;
        let inverse = view_proj.inverse();
        // Clip-space depth of a point `d` metres in front of the camera.
        let ndc = |d: f32| {
            let p = view.projection * Vec4::new(0.0, 0.0, -d, 1.0);
            p.z / p.w
        };
        let mut out = Self {
            matrices: [Mat4::IDENTITY; CASCADES],
            light,
            boxes: [[0.0; 6]; CASCADES],
            splits: [0.0; CASCADES],
            texels: [0.0; CASCADES],
        };
        let mut start = view.near;
        for (i, split) in SPLITS.iter().enumerate() {
            let end = (distance * split).max(start + 1.0);
            let mut corners = [Vec3::ZERO; 8];
            for (k, c) in corners.iter_mut().enumerate() {
                let z = ndc(if k < 4 { start } else { end });
                let x = if k & 1 == 0 { -1.0 } else { 1.0 };
                let y = if k & 2 == 0 { -1.0 } else { 1.0 };
                *c = inverse.project_point3(Vec3::new(x, y, z));
            }
            let centre = corners.iter().copied().sum::<Vec3>() / 8.0;
            let radius = corners
                .iter()
                .map(|c| c.distance(centre))
                .fold(0.0, f32::max);
            // Whole sixteenths of a metre, so the texel size stays put.
            let radius = (radius * 16.0).ceil() / 16.0;
            let texel = 2.0 * radius / SIZE as f32;
            let c = light.transform_point3(centre);
            let (x, y) = ((c.x / texel).floor() * texel, (c.y / texel).floor() * texel);
            let near = -c.z - radius - REACH;
            let far = -c.z + radius;
            let b = [x - radius, x + radius, y - radius, y + radius, near, far];
            out.matrices[i] = Mat4::orthographic_rh(b[0], b[1], b[2], b[3], b[4], b[5]) * light;
            out.boxes[i] = b;
            out.splits[i] = end;
            out.texels[i] = texel;
            start = end;
        }
        Some(out)
    }

    /// Whether a world-space sphere can cast into cascade `i`.
    pub fn touches(&self, i: usize, centre: Vec3, radius: f32) -> bool {
        let c = self.light.transform_point3(centre);
        let b = &self.boxes[i];
        c.x + radius >= b[0]
            && c.x - radius <= b[1]
            && c.y + radius >= b[2]
            && c.y - radius <= b[3]
            && -c.z + radius >= b[4]
            && -c.z - radius <= b[5]
    }
}

/// A bounding sphere (centre, radius) around the positions in `data`, laid
/// out as `layout`; `None` without readable positions.
pub fn bounds(layout: &VertexLayout, data: &[u8]) -> Option<(Vec3, f32)> {
    let e = layout.find(VertexSemantic::Position)?;
    let stride = layout.stride() as usize;
    let at = e.offset as usize;
    let read = |v: &[u8]| -> Option<Vec3> {
        let f = |i: usize| -> Option<f32> {
            match e.format {
                VertexFormat::Float3 | VertexFormat::Float4 => v
                    .get(at + i * 4..at + i * 4 + 4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                VertexFormat::Half4 => v
                    .get(at + i * 2..at + i * 2 + 2)
                    .map(|b| half(u16::from_le_bytes([b[0], b[1]]))),
                _ => None,
            }
        };
        Some(Vec3::new(f(0)?, f(1)?, f(2)?))
    };
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for v in data.chunks_exact(stride.max(1)) {
        let p = read(v)?;
        if p.is_finite() {
            lo = lo.min(p);
            hi = hi.max(p);
        }
    }
    (lo.x <= hi.x).then(|| ((lo + hi) * 0.5, (hi - lo).length() * 0.5))
}

/// An IEEE half-precision float.
fn half(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((h >> 10) & 0x1f);
    let frac = f32::from(h & 0x3ff);
    sign * match exp {
        0 => frac * 2f32.powi(-24),
        31 => f32::INFINITY,
        _ => (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
    }
}

/// The world-space sphere around a local sphere drawn at `world`.
pub fn transformed(world: Mat4, (centre, radius): (Vec3, f32)) -> (Vec3, f32) {
    let scale = world
        .x_axis
        .truncate()
        .length()
        .max(world.y_axis.truncate().length())
        .max(world.z_axis.truncate().length());
    (world.transform_point3(centre), radius * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> View {
        View {
            view: Mat4::look_at_rh(
                Vec3::new(0.0, 2.0, 0.0),
                Vec3::new(0.0, 2.0, -10.0),
                Vec3::Y,
            ),
            projection: Mat4::perspective_infinite_reverse_rh(1.0, 16.0 / 9.0, 0.1),
            ..View::default()
        }
    }

    #[test]
    fn cascades_cover_their_slices() {
        let c = Cascades::new(&view(), Vec3::new(-0.4, -1.0, 0.3), 200.0).unwrap();
        for (got, want) in c.splits.iter().zip([10.0, 30.0, 80.0, 200.0]) {
            assert!((got - want).abs() < 1e-3, "{:?}", c.splits);
        }
        // A point 5 m ahead lands inside the first map, 50 m ahead in the third.
        for (i, d) in [(0, 5.0), (2, 50.0)] {
            let p = c.matrices[i].project_point3(Vec3::new(0.0, 0.0, -d));
            assert!(
                p.x.abs() < 1.0 && p.y.abs() < 1.0 && (0.0..1.0).contains(&p.z),
                "{p}"
            );
            assert!(c.touches(i, Vec3::new(0.0, 0.0, -d), 0.5));
        }
        assert!(!c.touches(0, Vec3::new(0.0, 0.0, -150.0), 1.0));
        // No sun, no shadows.
        assert!(Cascades::new(&view(), Vec3::new(0.0, 0.5, 1.0), 200.0).is_none());
    }

    #[test]
    fn bounds_read_float_and_half_positions() {
        let layout =
            VertexLayout::packed(&[(VertexSemantic::Position, VertexFormat::Float3)]).unwrap();
        let data: Vec<u8> = [[-1.0f32, 0.0, 0.0], [1.0, 2.0, 0.0]]
            .iter()
            .flatten()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let (c, r) = bounds(&layout, &data).unwrap();
        assert_eq!(c, Vec3::new(0.0, 1.0, 0.0));
        assert!((r - 2f32.sqrt()).abs() < 1e-6);
        let layout =
            VertexLayout::packed(&[(VertexSemantic::Position, VertexFormat::Half4)]).unwrap();
        // 2.0, -0.5, 0, 1 in half precision.
        let data: Vec<u8> = [0x4000u16, 0xb800, 0, 0x3c00]
            .iter()
            .flat_map(|h| h.to_le_bytes())
            .collect();
        assert_eq!(
            bounds(&layout, &data).unwrap(),
            (Vec3::new(2.0, -0.5, 0.0), 0.0)
        );
    }
}
