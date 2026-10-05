//! The minimap's map, as the HUD's radar shows the island: the land left
//! clear (the radar's dark backdrop shows through it), the sea blue, and
//! over them the roads, drawn from the island's own road network: paved
//! ones solid, dirt tracks dashed.
//!
//! The engine draws the radar's map (the layout's `"VZ"` texture, 2800 ×
//! 2800, island-wide) itself; no such texture is in the archives. The sea
//! is made here from the low-resolution terrain tiles' heights against the
//! sea level ([`Heights`], [`radar_map`]). The roads are the layers'
//! `Road` entities ([`RoadRecord`]): each names a path asset, a smooth
//! curve through its points ([`centreline`]) that ends at the
//! `RoadIntersection` entities it joins. Which are paved is read off the
//! island-wide map the distant terrain wears (`vz_lrterrain`), whose paved
//! roads are a neutral blue-grey against yellow-green land
//! ([`looks_paved`], [`paved_share`]). The radar strokes its dashes with
//! the texture `global_radar_path` (two dashes, white in a black rim, in
//! 32 × 8 pixels), one of them stretched over each [`DASH`] of road, a
//! [`DASH_GAP`] between.
//!
//! # Provenance
//!
//! The colours and sizes are measured from gameplay footage of the PC
//! version (the minimap at 854 × 480 and 1920 × 1080; see [`WATER`],
//! [`ROAD`], [`ROAD_WIDTH`], [`DASH`] and [`PLAYER_SIZE`]); the
//! land's clearness from the radar backdrop `global_gui_hud_radar` (black
//! at alpha 162 inside the rim) letting through what the footage's land
//! does (36% of the scene behind). The `Road` component's layout (path
//! hash, a hash, the two intersections' entities, the path's bounds) and
//! the paths' shape (their first two points and last two 1 m apart, a
//! handle on each end of the curve) are inferred from the layers and
//! paths of a legally installed copy: 3,667 roads, every path found, and
//! ends 3 to 25 m short of the intersections they name. The paved
//! classifier's thresholds are tuned by eye on `vz_lrterrain`. No bytes
//! from the archives are committed.

/// The sea on the radar (RGBA): the mean of its blue in the footage, over
/// different scenes alike (so opaque, or nearly).
pub const WATER: [u8; 4] = [41, 57, 88, 255];
/// A road on the radar (RGBA): the brightest quarter of its yellow-olive
/// lines in footage at 1920 × 1080 (lower resolutions blur them darker).
/// The dash texture is white, so this is the dashes' colour too.
pub const ROAD: [u8; 4] = [194, 166, 75, 255];
/// The land: clear, for the radar's backdrop to show.
pub const LAND: [u8; 4] = [0, 0, 0, 0];
/// A road's stroke across, as a share of the minimap's radius: the
/// footage's lines are 1.7% of its width across, the dash texture's white
/// six of its eight rows.
pub const ROAD_WIDTH: f32 = 0.045;
/// Metres of road a dash of a dirt track takes, and the gap after it: the
/// footage's dashes are about 9% of the minimap's width long and their
/// gaps 6% at a walk (300 m across; driving the AMX-30, 6% and 5%), so
/// more dash than gap, unlike the texture's even halves.
pub const DASH: f32 = 27.0;
pub const DASH_GAP: f32 = 18.0;
/// Her marker, a white square in the middle of the minimap: its side as a
/// share of the minimap's radius (four pixels of a minimap 112 across in
/// the footage).
pub const PLAYER_SIZE: f32 = 0.07;
/// A road whose samples [`looks_paved`] at least this share is paved.
pub const PAVED_SHARE: f32 = 0.35;
/// Metres between the points [`centreline`] gives along a curve.
const STEP: f32 = 4.0;

use m2_math::glam::{Quat, Vec2, Vec3};

use crate::terrain::LowResTile;

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

/// A `Road` entity's record (the component's first 16 of 40 bytes): its
/// path asset, a hash (shared by runs of road alike, not yet named) and
/// the `RoadIntersection` entities at its two ends (0 where it meets
/// another road end to end).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoadRecord {
    pub path: u32,
    pub style: u32,
    pub ends: [u32; 2],
}

impl RoadRecord {
    pub fn parse(record: &[u8]) -> Option<Self> {
        let word = |at: usize| {
            record
                .get(at..at + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        Some(Self {
            path: word(0)?,
            style: word(4)?,
            ends: [word(8)?, word(12)?],
        })
    }
}

/// A road on the radar: its centreline over the ground (world x, z) and
/// whether it is paved (drawn solid) or a dirt track (dashed).
#[derive(Clone, Debug, PartialEq)]
pub struct Road {
    pub points: Vec<[f32; 2]>,
    pub paved: bool,
    /// Least x and z, then greatest.
    pub bounds: [f32; 4],
}

impl Road {
    pub fn new(points: Vec<[f32; 2]>, paved: bool) -> Self {
        let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for &[x, z] in &points {
            bounds = [
                bounds[0].min(x),
                bounds[1].min(z),
                bounds[2].max(x),
                bounds[3].max(z),
            ];
        }
        Self {
            points,
            paved,
            bounds,
        }
    }

    /// Whether any of it may lie within `reach` metres of `(x, z)`.
    pub fn near(&self, [x, z]: [f32; 2], reach: f32) -> bool {
        x >= self.bounds[0] - reach
            && x <= self.bounds[2] + reach
            && z >= self.bounds[1] - reach
            && z <= self.bounds[3] + reach
    }
}

/// How far apart (metres) a path's first two points, and its last two,
/// are at most to be the handles of its curve: 1 m on every road path
/// that has four points or more.
const HANDLE: f32 = 1.5;
/// How far (metres) a road's end is from an intersection it names before
/// it is carried on to the intersection's centre.
const JOIN: f32 = 0.5;

/// A road's centreline over the ground (world x, z) from its path's
/// points (in its entity's space, placed by `position` and turned by
/// `rotation`, x y z w): a centripetal Catmull-Rom curve through them, a
/// point about every [`STEP`] metres. A path of four points or more whose
/// first two and last two points lie within [`HANDLE`] of each other has a
/// handle on each end: the curve runs from its second point to its last
/// but one, leaving and arriving along the handles. `ends`, the centres
/// of the intersections it names, carry it on to them.
pub fn centreline(
    points: &[[f32; 3]],
    position: [f32; 3],
    rotation: [f32; 4],
    ends: [Option<[f32; 2]>; 2],
) -> Vec<[f32; 2]> {
    let q = Quat::from_array(rotation);
    let q = if q.is_finite() && q.length() > 0.5 {
        q.normalize()
    } else {
        Quat::IDENTITY
    };
    let at = Vec3::from(position);
    let world: Vec<Vec2> = points
        .iter()
        .map(|&p| {
            let w = at + q * Vec3::from(p);
            Vec2::new(w.x, w.z)
        })
        .collect();
    let n = world.len();
    let handled = n >= 4
        && world[0].distance(world[1]) <= HANDLE
        && world[n - 1].distance(world[n - 2]) <= HANDLE;
    // The curve's controls: the handles, or the ends mirrored outwards.
    let controls: Vec<Vec2> = if handled {
        world
    } else if n >= 2 {
        let mut c = Vec::with_capacity(n + 2);
        c.push(world[0] * 2.0 - world[1]);
        c.extend_from_slice(&world);
        c.push(world[n - 1] * 2.0 - world[n - 2]);
        c
    } else {
        return world.iter().map(|p| p.to_array()).collect();
    };
    let mut line = vec![controls[1]];
    for span in controls.windows(4) {
        let [p0, p1, p2, p3] = [span[0], span[1], span[2], span[3]];
        let steps = (p1.distance(p2) / STEP).ceil().max(1.0) as usize;
        for i in 1..=steps {
            line.push(catmull_rom(p0, p1, p2, p3, i as f32 / steps as f32));
        }
    }
    let mut line: Vec<[f32; 2]> = line.iter().map(Vec2::to_array).collect();
    let far = |a: [f32; 2], b: [f32; 2]| Vec2::from(a).distance(Vec2::from(b)) > JOIN;
    if let Some(c) = ends[0].filter(|&c| far(c, line[0])) {
        line.insert(0, c);
    }
    if let Some(c) = ends[1].filter(|&c| far(c, line[line.len() - 1])) {
        line.push(c);
    }
    line
}

/// The point `s` (0..=1) of the way from `p1` to `p2` on the centripetal
/// Catmull-Rom curve through `p0 p1 p2 p3` (Barry and Goldman's form),
/// which neither loops nor overshoots where its points are unevenly
/// spaced, as a road's 1 m handles and long spans are.
fn catmull_rom(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, s: f32) -> Vec2 {
    let knot = |a: Vec2, b: Vec2| a.distance(b).sqrt().max(1e-4);
    let t1 = knot(p0, p1);
    let t2 = t1 + knot(p1, p2);
    let t3 = t2 + knot(p2, p3);
    let t = t1 + (t2 - t1) * s;
    let lerp = |a: Vec2, b: Vec2, ta: f32, tb: f32| {
        a * ((tb - t) / (tb - ta)) + b * ((t - ta) / (tb - ta))
    };
    let a1 = lerp(p0, p1, 0.0, t1);
    let a2 = lerp(p1, p2, t1, t2);
    let a3 = lerp(p2, p3, t2, t3);
    let b1 = lerp(a1, a2, 0.0, t2);
    let b2 = lerp(a2, a3, t1, t3);
    lerp(b1, b2, t1, t2)
}

/// The share (0..=1) of a road's centreline whose island-map pixel
/// [`looks_paved`]; `pixel` gives the island map's pixel under world
/// `(x, z)` (None off the map, not counted).
pub fn paved_share(points: &[[f32; 2]], pixel: impl Fn([f32; 2]) -> Option<[u8; 4]>) -> f32 {
    let (mut paved, mut all) = (0usize, 0usize);
    for &p in points {
        if let Some(px) = pixel(p) {
            all += 1;
            paved += usize::from(looks_paved(px));
        }
    }
    if all == 0 {
        0.0
    } else {
        paved as f32 / all as f32
    }
}

/// The radar's map, one pixel to each of `heights`' samples, laid as it
/// is: water where the ground is under `sea_level` (or no tile reaches),
/// the rest clear. The roads are drawn over it ([`Road`]).
pub fn radar_map(heights: &Heights, sea_level: f32) -> Vec<u8> {
    heights
        .heights
        .iter()
        .flat_map(|&ground| {
            if ground.is_nan() || ground < sea_level {
                WATER
            } else {
                LAND
            }
        })
        .collect()
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
    fn under_the_sea_is_water_and_the_land_is_clear() {
        let mut heights = Heights::new(4, 4.0);
        for (i, h) in heights.heights.iter_mut().enumerate() {
            *h = if i < 8 { 10.0 } else { -50.0 };
        }
        let map = radar_map(&heights, -36.0);
        let at = |i: usize| [map[i * 4], map[i * 4 + 1], map[i * 4 + 2], map[i * 4 + 3]];
        assert_eq!(map.len(), 4 * 16);
        assert_eq!(at(5), LAND);
        assert_eq!(at(12), WATER);
    }

    #[test]
    fn a_road_record_names_its_path_and_intersections() {
        let mut rec = [0u8; 40];
        rec[..4].copy_from_slice(&0xb399_abccu32.to_le_bytes());
        rec[4..8].copy_from_slice(&0x5b30_829du32.to_le_bytes());
        rec[8..12].copy_from_slice(&0x0008_ace0u32.to_le_bytes());
        let r = RoadRecord::parse(&rec).unwrap();
        assert_eq!(
            (r.path, r.style, r.ends),
            (0xb399_abcc, 0x5b30_829d, [0x0008_ace0, 0])
        );
        assert_eq!(RoadRecord::parse(&rec[..12]), None);
    }

    #[test]
    fn a_handled_path_curves_smoothly_from_its_second_point_to_its_last_but_one() {
        // Leaves north along its handle, bends east, arrives north: as the
        // road paths run (1 m handles, long spans), placed at (100, 0, 200).
        let points = [
            [0.0, 0.0, -41.0],
            [0.0, 0.0, -40.0],
            [20.0, 0.0, 0.0],
            [40.0, 0.0, 40.0],
            [40.0, 0.0, 41.0],
        ];
        let line = centreline(
            &points,
            [100.0, 0.0, 200.0],
            [0.0, 0.0, 0.0, 1.0],
            [None, None],
        );
        assert_eq!(line[0], [100.0, 160.0]);
        assert_eq!(*line.last().unwrap(), [140.0, 240.0]);
        let through = Vec2::new(120.0, 200.0);
        assert!(line.iter().any(|p| Vec2::from(*p).distance(through) < 1e-3));
        // A point every few metres, and never turning back on itself.
        let mut length = 0.0;
        for w in line.windows(2) {
            let d = Vec2::from(w[0]).distance(Vec2::from(w[1]));
            assert!(d > 0.5 && d <= STEP * 1.5, "{d}");
            length += d;
        }
        // No shorter than its points are apart (two spans of 44.7 m).
        assert!(length > 89.4 && length < 100.0, "{length}");
        for w in line.windows(3) {
            let a = Vec2::from(w[1]) - Vec2::from(w[0]);
            let b = Vec2::from(w[2]) - Vec2::from(w[1]);
            assert!(a.dot(b) > 0.0);
        }
        // Its first step goes north, along the handle.
        assert!((line[1][0] - 100.0).abs() < (line[1][1] - 160.0).abs());
    }

    #[test]
    fn a_road_runs_on_to_its_intersections_and_turns_with_its_entity() {
        // Two points 10 m apart along x, the entity turned 90 degrees about
        // y (x goes to -z), an intersection 5 m past the far end.
        let half = std::f32::consts::FRAC_1_SQRT_2;
        let line = centreline(
            &[[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]],
            [0.0; 3],
            [0.0, half, 0.0, half],
            [None, Some([0.0, -15.0])],
        );
        assert!(Vec2::from(line[0]).length() < 1e-4);
        let far = Vec2::new(0.0, -10.0);
        assert!(line.iter().any(|p| Vec2::from(*p).distance(far) < 1e-3));
        assert_eq!(*line.last().unwrap(), [0.0, -15.0]);
    }

    #[test]
    fn paved_share_counts_the_grey_under_a_road() {
        let points = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0]];
        let share = paved_share(&points, |[x, _]| match x as i32 {
            0 | 1 => Some([70, 70, 71, 255]),
            2 => Some([45, 44, 32, 255]),
            _ => None,
        });
        assert!((share - 2.0 / 3.0).abs() < 1e-6);
        let road = Road::new(points.to_vec(), true);
        assert!(road.near([1.5, 20.0], 25.0) && !road.near([1.5, 30.0], 25.0));
    }
}
