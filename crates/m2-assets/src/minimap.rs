//! The minimap's map, as the HUD's radar shows the island: the land left
//! clear (the radar's dark backdrop shows through it), the sea blue, the
//! paved roads bright and the dirt tracks dashed.
//!
//! The engine draws the radar's map (the layout's `"VZ"` texture, 2800 ×
//! 2800, island-wide) itself; no such texture is in the archives. It is
//! made here from what is: the island-wide map the distant terrain wears
//! (`vz_lrterrain`), whose paved roads are a neutral blue-grey against
//! yellow-green land ([`paved`]) and whose dirt tracks are lighter warm
//! lines ([`tracks`]), and the low-resolution terrain tiles' heights
//! against the sea level ([`Heights`]).
//!
//! # Provenance
//!
//! The colours are measured from gameplay footage of the PC version (the
//! minimap at 854 × 480; see [`WATER`] and [`ROAD`]); the land's
//! clearness from the radar backdrop `global_gui_hud_radar` (black at
//! alpha 162 inside the rim) letting through what the footage's land does
//! (36% of the scene behind). That tracks are dashed is seen in footage;
//! the dashes' length ([`DASH`]) and the classifiers' thresholds are tuned
//! by eye on `vz_lrterrain` of a legally installed copy. No bytes from the
//! archives are committed.

use std::ops::RangeInclusive;

use crate::terrain::LowResTile;

/// The sea on the radar (RGBA): the mean of its blue in the footage, over
/// different scenes alike (so opaque, or nearly).
pub const WATER: [u8; 4] = [41, 57, 88, 255];
/// A road on the radar (RGBA): the brightest quarter of its olive bands in
/// the footage (their edges blur into the land there). Tracks are dashed
/// in it too.
pub const ROAD: [u8; 4] = [134, 124, 37, 255];
/// The land: clear, for the radar's backdrop to show.
pub const LAND: [u8; 4] = [0, 0, 0, 0];

/// Paved runs shorter than this many pixels are roofs and rock, not road.
const PAVED_LEAST: usize = 30;
/// A dirt track is a warm line at least this much brighter than the mean
/// of the square `TRACK_REACH` pixels round it, of mid brightness...
const TRACK_RISE: f32 = 6.0;
const TRACK_REACH: usize = 4;
const TRACK_LUM: RangeInclusive<i32> = 40..=90;
/// ... in a run of at least this many pixels (shorter ones are ridges).
const TRACK_LEAST: usize = 90;
/// How far round a pixel [`narrowed`] looks for the deepest of its band.
const NARROW_REACH: usize = 3;
/// The radar dashes the tracks: on and off every this many map pixels (a
/// checkerboard, so the dashes run along a track whichever way it goes).
pub const DASH: usize = 5;

/// Terrain heights over the island, sampled on the map's own grid: what
/// lies under the sea is water on the radar.
#[derive(Debug, Clone)]
pub struct Heights {
    size: usize,
    island: f32,
    /// Row-major; NaN where no tile reaches (the open sea).
    heights: Vec<f32>,
}

impl Heights {
    /// An empty grid `size` samples a side over an island `island` metres
    /// square centred on the origin, laid as the island-wide map is
    /// (`+x` and `+z` towards its left and top edges, see [`Self::uv`]).
    pub fn new(size: usize, island: f32) -> Self {
        Self {
            size,
            island,
            heights: vec![f32::NAN; size * size],
        }
    }

    /// The map's coordinates (0..=1) of world `(x, z)`.
    pub fn uv(&self, x: f32, z: f32) -> [f32; 2] {
        let half = self.island / 2.0;
        [(half - x) / self.island, (half - z) / self.island]
    }

    /// Lays a low-resolution tile placed at `centre` (its positions are
    /// about it) onto the grid: each triangle's heights interpolated over
    /// the samples it covers.
    pub fn add_tile(&mut self, tile: &LowResTile<'_>, centre: [f32; 3]) {
        let group = &tile.group;
        let points: Vec<[f32; 3]> = group
            .positions()
            .into_iter()
            .map(|p| {
                let [u, v] = self.uv(p[0] + centre[0], p[2] + centre[2]);
                [u * self.size as f32, v * self.size as f32, p[1] + centre[1]]
            })
            .collect();
        let ranges = if group.full_ranges.is_empty() {
            &group.material_ranges
        } else {
            &group.full_ranges
        };
        for range in ranges {
            for tri in group.triangles(range) {
                let corners = tri.map(|i| points.get(usize::from(i)).copied());
                if let [Some(a), Some(b), Some(c)] = corners {
                    self.triangle(a, b, c);
                }
            }
        }
    }

    /// Fills the samples whose centres lie in triangle `a b c` (grid
    /// x, grid y, height).
    fn triangle(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3]) {
        let area = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
        if area.abs() < 1e-9 {
            return;
        }
        let max = self.size as f32 - 1.0;
        let lo = |i: usize| a[i].min(b[i]).min(c[i]).floor().clamp(0.0, max) as usize;
        let hi = |i: usize| a[i].max(b[i]).max(c[i]).ceil().clamp(0.0, max) as usize;
        for y in lo(1)..=hi(1) {
            for x in lo(0)..=hi(0) {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let wa = ((b[0] - px) * (c[1] - py) - (c[0] - px) * (b[1] - py)) / area;
                let wb = ((c[0] - px) * (a[1] - py) - (a[0] - px) * (c[1] - py)) / area;
                let wc = 1.0 - wa - wb;
                if wa >= -1e-4 && wb >= -1e-4 && wc >= -1e-4 {
                    self.heights[y * self.size + x] = wa * a[2] + wb * b[2] + wc * c[2];
                }
            }
        }
    }

    /// The height at the map's `(u, v)`, nearest sample; NaN off the
    /// tiles.
    pub fn at_uv(&self, u: f32, v: f32) -> f32 {
        let n = self.size as f32;
        let (x, y) = ((u * n).floor(), (v * n).floor());
        if x < 0.0 || y < 0.0 || x >= n || y >= n {
            return f32::NAN;
        }
        self.heights[y as usize * self.size + x as usize]
    }

    /// Samples that some tile reached.
    pub fn covered(&self) -> usize {
        self.heights.iter().filter(|h| !h.is_nan()).count()
    }
}

/// Whether an island-map pixel looks like a paved road: a neutral or
/// bluish grey (the land and the dirt tracks are yellow-green, blue well
/// under red and green), not too dark (shade) nor too bright (bare rock).
pub fn looks_paved([r, g, b, _]: [u8; 4]) -> bool {
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    let lum = (r + g + b) / 3;
    let warm = r.max(g) - b;
    (52..=110).contains(&lum) && warm <= 3 && (r - g).abs() <= 10
}

/// The paved roads of island map `rgba` (`width` × `height`): connected
/// runs of pixels that [`looks_paved`], [`narrowed`].
pub fn paved(rgba: &[u8], width: usize, height: usize) -> Vec<bool> {
    let raw: Vec<bool> = rgba
        .chunks_exact(4)
        .map(|p| looks_paved([p[0], p[1], p[2], p[3]]))
        .collect();
    let kept = keep_runs(&raw, width, height, PAVED_LEAST);
    narrowed(&smoothed(&kept, width, height), width, height)
}

/// `mask` by the majority of each pixel's 3 × 3: ragged edges smoothed,
/// pinholes filled.
pub fn smoothed(mask: &[bool], width: usize, height: usize) -> Vec<bool> {
    (0..width * height)
        .map(|i| {
            let (x, y) = (i % width, i / width);
            let mut n = 0;
            for ny in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                    n += usize::from(mask[ny * width + nx]);
                }
            }
            n >= 5
        })
        .collect()
}

/// `mask` no wider than about three pixels: of a band wider than four,
/// the middle (the pixels within one of its deepest, by the distance to
/// its edge). The footage's streets are bands about 12 m across (three
/// pixels of the island map); the island map's town streets are four to
/// seven (16 to 28 m) and its squares wider still.
pub fn narrowed(mask: &[bool], width: usize, height: usize) -> Vec<bool> {
    // Chessboard distance to the nearest pixel outside, two passes.
    let mut d: Vec<u16> = mask.iter().map(|&m| if m { u16::MAX } else { 0 }).collect();
    let at = |x: isize, y: isize| -> Option<usize> {
        (x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height)
            .then(|| y as usize * width + x as usize)
    };
    for y in 0..height as isize {
        for x in 0..width as isize {
            let i = y as usize * width + x as usize;
            if d[i] == 0 {
                continue;
            }
            let mut best = u16::MAX;
            for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0)] {
                best = best.min(at(x + dx, y + dy).map_or(0, |j| d[j]));
            }
            d[i] = best.saturating_add(1);
        }
    }
    for y in (0..height as isize).rev() {
        for x in (0..width as isize).rev() {
            let i = y as usize * width + x as usize;
            if d[i] == 0 {
                continue;
            }
            for (dx, dy) in [(1, 1), (0, 1), (-1, 1), (1, 0)] {
                let n = at(x + dx, y + dy).map_or(0, |j| d[j]);
                d[i] = d[i].min(n.saturating_add(1));
            }
        }
    }
    (0..width * height)
        .map(|i| {
            if d[i] == 0 {
                return false;
            }
            let (x, y) = ((i % width) as isize, (i / width) as isize);
            // Stays within one of the deepest round it: the band's middle
            // three pixels (all of a band up to four across).
            let r = NARROW_REACH as isize;
            let deepest = (-r..=r)
                .flat_map(|dy| (-r..=r).map(move |dx| (dx, dy)))
                .filter_map(|(dx, dy)| at(x + dx, y + dy))
                .map(|j| d[j])
                .max()
                .unwrap_or(0);
            d[i] + 1 >= deepest
        })
        .collect()
}

/// The dirt tracks of island map `rgba`: warm lines brighter than the
/// ground round them, in long runs.
pub fn tracks(rgba: &[u8], width: usize, height: usize) -> Vec<bool> {
    let lum: Vec<f32> = rgba
        .chunks_exact(4)
        .map(|p| (f32::from(p[0]) + f32::from(p[1]) + f32::from(p[2])) / 3.0)
        .collect();
    // Summed-area table, for the mean round each pixel.
    let w1 = width + 1;
    let mut sum = vec![0.0f64; w1 * (height + 1)];
    for y in 0..height {
        for x in 0..width {
            sum[(y + 1) * w1 + x + 1] =
                f64::from(lum[y * width + x]) + sum[y * w1 + x + 1] + sum[(y + 1) * w1 + x]
                    - sum[y * w1 + x];
        }
    }
    let mean = |x: usize, y: usize| {
        let (x0, y0) = (x.saturating_sub(TRACK_REACH), y.saturating_sub(TRACK_REACH));
        let (x1, y1) = (
            (x + TRACK_REACH + 1).min(width),
            (y + TRACK_REACH + 1).min(height),
        );
        let total = sum[y1 * w1 + x1] - sum[y0 * w1 + x1] - sum[y1 * w1 + x0] + sum[y0 * w1 + x0];
        (total / ((x1 - x0) * (y1 - y0)) as f64) as f32
    };
    let raw: Vec<bool> = (0..width * height)
        .map(|i| {
            let p = &rgba[i * 4..i * 4 + 4];
            let warm = i32::from(p[0].max(p[1])) - i32::from(p[2]);
            TRACK_LUM.contains(&(lum[i] as i32))
                && warm > 3
                && lum[i] - mean(i % width, i / width) > TRACK_RISE
        })
        .collect();
    keep_runs(&raw, width, height, TRACK_LEAST)
}

/// `mask` less its 8-connected runs of fewer than `least` pixels.
fn keep_runs(mask: &[bool], width: usize, height: usize, least: usize) -> Vec<bool> {
    let mut out = vec![false; mask.len()];
    let mut seen = vec![false; mask.len()];
    let (mut stack, mut run) = (Vec::new(), Vec::new());
    for start in 0..mask.len() {
        if !mask[start] || seen[start] {
            continue;
        }
        run.clear();
        stack.push(start);
        seen[start] = true;
        while let Some(i) = stack.pop() {
            run.push(i);
            let (x, y) = (i % width, i / width);
            for ny in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                    let j = ny * width + nx;
                    if mask[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        if run.len() >= least {
            for &i in &run {
                out[i] = true;
            }
        }
    }
    out
}

/// The radar's map from island map `rgba` (`width` × `height`, laid as
/// [`Heights`] is): water where the ground is under `sea_level` (or no
/// tile reaches), roads where [`paved`] finds them, dirt [`tracks`]
/// dashed in the roads' colour, the rest clear.
pub fn radar_map(
    rgba: &[u8],
    width: usize,
    height: usize,
    heights: &Heights,
    sea_level: f32,
) -> Vec<u8> {
    let roads = paved(rgba, width, height);
    let tracks = tracks(rgba, width, height);
    let mut out = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            let (u, v) = (
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            );
            let ground = heights.at_uv(u, v);
            let dash = (x / DASH + y / DASH).is_multiple_of(2);
            let colour = if ground.is_nan() || ground < sea_level {
                WATER
            } else if roads[i] || (tracks[i] && dash) {
                ROAD
            } else {
                LAND
            };
            out.extend_from_slice(&colour);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roads_are_grey_and_land_is_not() {
        // Made-up colours of the kinds the island map has: greys of paved
        // road, then yellow-green land and a lighter dirt track.
        for road in [[70, 70, 71, 255], [60, 64, 70, 255], [80, 82, 84, 255]] {
            assert!(looks_paved(road), "{road:?}");
        }
        for land in [[30, 36, 26, 255], [45, 44, 32, 255], [60, 58, 50, 255]] {
            assert!(!looks_paved(land), "{land:?}");
        }
    }

    #[test]
    fn a_triangle_fills_the_samples_it_covers() {
        let mut h = Heights::new(8, 8.0);
        h.triangle([0.0, 0.0, 10.0], [8.0, 0.0, 10.0], [0.0, 8.0, -10.0]);
        // Inside, height between the corners'; past the diagonal, none.
        let inside = h.at_uv(0.1, 0.1);
        assert!(inside > -10.0 && inside < 10.0, "{inside}");
        assert!(h.at_uv(0.9, 0.9).is_nan());
        assert_eq!(h.uv(4.0, 4.0), [0.0, 0.0]);
    }

    #[test]
    fn wide_bands_lose_their_edges_and_narrow_ones_stay() {
        // A band five across, a gap, then one three across, in a 12-wide
        // strip 9 high (columns are across the bands).
        let (w, h) = (12, 9);
        let mask: Vec<bool> = (0..w * h).map(|i| matches!(i % w, 0..=4 | 6..=8)).collect();
        let out = narrowed(&mask, w, h);
        let row: Vec<bool> = out[4 * w..5 * w].to_vec();
        assert_eq!(
            row,
            [
                false, true, true, true, false, false, true, true, true, false, false, false
            ]
        );
    }

    #[test]
    fn short_runs_go_and_long_ones_stay() {
        // A run of three and a lone pixel on a 5 × 3 grid.
        let mask = [
            true, true, true, false, false, //
            false, false, false, false, true, //
            false, false, false, false, false,
        ];
        let kept = keep_runs(&mask, 5, 3, 3);
        assert_eq!(kept.iter().filter(|k| **k).count(), 3);
        assert!(!kept[9]);
    }

    #[test]
    fn under_the_sea_is_water_and_a_lone_grey_pixel_is_land() {
        let (w, hgt) = (4, 4);
        let mut rgba = [[45u8, 44, 32, 255]; 16];
        rgba[5] = [70, 70, 71, 255];
        let rgba: Vec<u8> = rgba.concat();
        let mut heights = Heights::new(4, 4.0);
        for (i, h) in heights.heights.iter_mut().enumerate() {
            *h = if i < 8 { 10.0 } else { -50.0 };
        }
        let map = radar_map(&rgba, w, hgt, &heights, -36.0);
        let at = |i: usize| [map[i * 4], map[i * 4 + 1], map[i * 4 + 2], map[i * 4 + 3]];
        assert_eq!(at(5), LAND);
        assert_eq!(at(0), LAND);
        assert_eq!(at(12), WATER);
    }
}
