//! The minimap's map, as the HUD's radar shows the island: the land left
//! clear (the radar's dark backdrop shows through it), the sea blue, and
//! over them the roads.
//!
//! The radar's movie (`minimap.gfx`) holds the map itself: the sprite it
//! places as `roads_mc` shows one shape 2,048 pixels square of the sea and
//! rivers (a translucent blue fill) and 4,854 strokes of olive, three
//! pixels wide in the main, the dirt tracks drawn as runs of short
//! separate dashes ([`roads_shape`], [`MovieMap`]). Its strokes widen with
//! the map (no line style keeps its width). It is placed on the world by
//! [`MAP_SCALE`] and [`MAP_ORIGIN`]. Of the island's `Road` entities, the
//! radar's strokes run along 93% of the 1,398 with a lane of traffic (half
//! their length or more within 2 pixels) and 8% of the other 2,269
//! (sidewalks, curbs, walls): it shows the driven roads only.
//!
//! Without the movie, the map is made as follows. The engine draws the radar's map (the layout's `"VZ"` texture, 2800 ×
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
//! version (the minimap at 854 × 480, and the Xbox 360's at 1920 × 1080;
//! see [`WATER`], [`ROAD`], [`ROAD_WIDTH`], [`DASH`] and [`PLAYER_SIZE`]); the
//! land's clearness from the radar backdrop `global_gui_hud_radar` (black
//! at alpha 162 inside the rim) letting through what the footage's land
//! does (36% of the scene behind). The `Road` component's layout (path
//! hash, a hash, the two intersections' entities, the path's bounds) and
//! the paths' shape (their first two points and last two 1 m apart, a
//! handle on each end of the curve) are inferred from the layers and
//! paths of a legally installed copy: 3,667 roads, every path found, and
//! ends 3 to 25 m short of the intersections they name. The paved
//! classifier's thresholds are tuned by eye on `vz_lrterrain`. The movie
//! map's shape, its `roads_mc` name and its 4 m to a pixel are read from
//! the movie (its shape and its ActionScript); where it lies on the world
//! ([`MAP_ORIGIN`]) is fitted to the roads and the terrain (from samples).
//! The colours stay the footage's ([`WATER`], [`ROAD`]; the movie's
//! strokes are `#606020`, its sea `#4080ff` at 30%, before Scaleform
//! draws them over the backdrop). No bytes from the archives are
//! committed.

/// The sea on the radar (RGBA): the mean of its blue in the footage, over
/// different scenes alike (so opaque, or nearly).
pub const WATER: [u8; 4] = [41, 57, 88, 255];
/// A road on the radar (RGBA): the brightest twentieth of its olive lines
/// in the PC footage (854 × 480; the Xbox 360's are paler, 194, 166, 75).
/// The dash texture is white, so this is the dashes' colour too.
pub const ROAD: [u8; 4] = [134, 124, 37, 255];
/// The land: clear, for the radar's backdrop to show.
pub const LAND: [u8; 4] = [0, 0, 0, 0];
/// A dirt track's stroke across, as a share of the minimap's radius: the
/// footage's dashes are 1.7% to 2% of its width across, the dash
/// texture's white six of its eight rows.
pub const ROAD_WIDTH: f32 = 0.045;
/// A paved road's: the PC footage's town streets are 2.8% of its width.
pub const PAVED_WIDTH: f32 = 0.075;
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

use crate::flash::player::Control;
use crate::flash::{Fill, Library, Shape, ShapeTriangle, Style};
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
/// whether it is paved (drawn solid) or a dirt track (dashed), or a
/// stroke of the radar movie's map ([`MovieMap`]), solid and `width`
/// metres across.
#[derive(Clone, Debug, PartialEq)]
pub struct Road {
    pub points: Vec<[f32; 2]>,
    pub paved: bool,
    /// Metres across, for a stroke of the movie's map.
    pub width: Option<f32>,
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
            width: None,
            bounds,
        }
    }

    /// A stroke of the movie's map, `width` metres across.
    pub fn stroke(points: Vec<[f32; 2]>, width: f32) -> Self {
        Self {
            width: Some(width),
            ..Self::new(points, true)
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

/// Metres of the world to one pixel of the radar movie's map: the movie
/// moves its map by the location it is given over 4 (`currentLocation`,
/// `insidemap_mc._x = -x / 4`), and divides its zones' points by 4 alike.
/// Fitting the map's sea to the terrain's agrees (best at 4.00 between
/// 3.5 and 4.5).
pub const MAP_SCALE: f32 = 4.0;
/// Where the world's origin lies on the radar movie's map, in its
/// pixels. Fitted to the island (2026-10-16): the 1,398 roads with a lane
/// of traffic lie on the map's strokes best here (84% of their length
/// within 2 pixels), and its sea on the terrain's under the sea level
/// best one pixel further each way (IoU 0.886); the movie's zones are
/// placed with (35, 40) metres added, which puts them 2 to 3 pixels off
/// this (not read in the engine).
pub const MAP_ORIGIN: [f32; 2] = [-11.0, -13.0];

/// World `(x, z)` of a point of the radar movie's map: the movie's `+x`
/// and `+y` run towards the world's `-x` and `-z`.
pub fn map_to_world([u, v]: [f32; 2]) -> [f32; 2] {
    [
        (MAP_ORIGIN[0] - u) * MAP_SCALE,
        (MAP_ORIGIN[1] - v) * MAP_SCALE,
    ]
}

/// The radar's map as its movie (`minimap.gfx`) has it: the shape its
/// `roads_mc` shows, 2,048 pixels square, of the sea and rivers (a
/// translucent blue fill) and the roads (strokes of olive, the dirt
/// tracks as runs of short dashes), in world `(x, z)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MovieMap {
    /// The water's triangles.
    pub water: Vec<[[f32; 2]; 3]>,
    /// The strokes, each a run of edges end to end in one line style.
    pub roads: Vec<Road>,
}

/// The shape the radar movie's `roads_mc` shows, with its triangles: the
/// sprite placed under that name, and the first shape on its timeline.
pub fn roads_shape(lib: &Library) -> Option<&(Shape, Vec<ShapeTriangle>)> {
    let timelines = std::iter::once(&lib.root).chain(lib.sprites.values());
    let sprite = timelines
        .flat_map(|t| t.frames.iter().flatten())
        .find_map(|c| match c {
            Control::Place(p) if p.name.as_deref() == Some("roads_mc") => p.character,
            _ => None,
        })?;
    lib.sprites
        .get(&sprite)?
        .frames
        .iter()
        .flatten()
        .find_map(|c| match c {
            Control::Place(p) => p.character.and_then(|id| lib.shapes.get(&id)),
            _ => None,
        })
}

impl MovieMap {
    /// The map of the radar's `shape` (see [`roads_shape`]), its
    /// triangles `mesh` ([`Shape::mesh`]): water where a translucent fill
    /// bluer than red covers, a road along each stroke wider than
    /// nothing.
    pub fn new(shape: &Shape, mesh: &[ShapeTriangle]) -> Self {
        let watery = |i: usize| matches!(shape.fills.get(i), Some(Fill::Solid([r, _, b, a])) if *a > 0.0 && *a < 1.0 && b > r);
        let water = mesh
            .iter()
            .filter(|t| matches!(t.style, Style::Fill(i) if watery(i)))
            .map(|t| t.corners.map(map_to_world))
            .collect();
        let mut roads: Vec<Road> = Vec::new();
        let mut run: Vec<[f32; 2]> = Vec::new();
        let mut run_line = 0;
        for e in &shape.edges {
            let width = shape.lines.get(e.line).map_or(0.0, |l| l.width);
            let joins = e.line == run_line && run.last() == Some(&e.from);
            if !joins && run.len() > 1 {
                roads.push(Road::stroke(
                    run.iter().map(|&p| map_to_world(p)).collect(),
                    shape.lines[run_line].width * MAP_SCALE,
                ));
            }
            if !joins {
                run.clear();
            }
            if width <= 0.0 {
                run_line = 0;
                continue;
            }
            run_line = e.line;
            if run.is_empty() {
                run.push(e.from);
            }
            if let Some(c) = e.control {
                // A curve, in pieces of about a pixel.
                let span = (c[0] - e.from[0]).hypot(c[1] - e.from[1])
                    + (e.to[0] - c[0]).hypot(e.to[1] - c[1]);
                let n = (span.ceil() as usize).clamp(2, 32);
                run.extend((1..n).map(|i| {
                    let t = i as f32 / n as f32;
                    let u = 1.0 - t;
                    std::array::from_fn(|k| {
                        u * u * e.from[k] + 2.0 * u * t * c[k] + t * t * e.to[k]
                    })
                }));
            }
            run.push(e.to);
        }
        if run.len() > 1 && run_line != 0 {
            roads.push(Road::stroke(
                run.iter().map(|&p| map_to_world(p)).collect(),
                shape.lines[run_line].width * MAP_SCALE,
            ));
        }
        Self { water, roads }
    }

    /// The water as the radar's map is laid ([`Heights`]' grid, `size`
    /// pixels over an island `island` metres square): [`WATER`] where a
    /// pixel's centre lies in the water, the rest [`LAND`].
    pub fn image(&self, size: usize, island: f32) -> Vec<u8> {
        let mut wet = vec![false; size * size];
        let grid = Heights::new(size, island);
        let max = size as f32 - 1.0;
        for tri in &self.water {
            let p = tri.map(|[x, z]| {
                let [u, v] = grid.uv(x, z);
                [u * size as f32, v * size as f32]
            });
            let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1])
                - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
            if area.abs() < 1e-9 {
                continue;
            }
            let lo = |i: usize| p[0][i].min(p[1][i]).min(p[2][i]).floor().clamp(0.0, max) as usize;
            let hi = |i: usize| p[0][i].max(p[1][i]).max(p[2][i]).ceil().clamp(0.0, max) as usize;
            for y in lo(1)..=hi(1) {
                for x in lo(0)..=hi(0) {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let wa =
                        ((p[1][0] - px) * (p[2][1] - py) - (p[2][0] - px) * (p[1][1] - py)) / area;
                    let wb =
                        ((p[2][0] - px) * (p[0][1] - py) - (p[0][0] - px) * (p[2][1] - py)) / area;
                    if wa >= 0.0 && wb >= 0.0 && wa + wb <= 1.0 {
                        wet[y * size + x] = true;
                    }
                }
            }
        }
        wet.into_iter()
            .flat_map(|w| if w { WATER } else { LAND })
            .collect()
    }
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

    #[test]
    fn the_movie_map_lies_a_quarter_metre_a_pixel_turned_half_round() {
        assert_eq!(map_to_world(MAP_ORIGIN), [0.0, 0.0]);
        let [x, z] = map_to_world([MAP_ORIGIN[0] + 1.0, MAP_ORIGIN[1] + 2.0]);
        assert_eq!([x, z], [-MAP_SCALE, -2.0 * MAP_SCALE]);
    }

    /// A shape of `edges` with the radar's styles: fill 1 the sea's
    /// translucent blue, fill 2 opaque black; line 1 three pixels wide.
    fn radar_like(edges: Vec<crate::flash::Edge>) -> Shape {
        use crate::flash::Line;
        Shape {
            bounds: [-16.0, -16.0, 16.0, 16.0],
            fills: vec![
                Fill::Solid([0.0; 4]),
                Fill::Solid([0.25, 0.5, 1.0, 0.3]),
                Fill::Solid([0.0, 0.0, 0.0, 1.0]),
            ],
            lines: vec![
                Line {
                    width: 0.0,
                    colour: [0.0; 4],
                },
                Line {
                    width: 3.0,
                    colour: [0.38, 0.38, 0.13, 1.0],
                },
            ],
            edges,
        }
    }

    fn edge(from: [f32; 2], to: [f32; 2], fill1: usize, line: usize) -> crate::flash::Edge {
        crate::flash::Edge {
            from,
            control: None,
            to,
            fill0: 0,
            fill1,
            line,
        }
    }

    #[test]
    fn the_movie_map_takes_the_blue_fill_as_water_and_each_run_of_strokes_as_a_road() {
        let square = |fill| {
            let c = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
            (0..4).map(move |i| edge(c[i], c[(i + 1) % 4], fill, 0))
        };
        let mut edges: Vec<_> = square(1).collect();
        edges.extend(square(2).map(|mut e| {
            e.from[0] += 20.0;
            e.to[0] += 20.0;
            e
        }));
        // Two strokes end to end (one road), a dash apart from them (a
        // second), and an edge of no line (none).
        edges.push(edge([-10.0, -10.0], [-5.0, -10.0], 0, 1));
        edges.push(edge([-5.0, -10.0], [-5.0, -5.0], 0, 1));
        edges.push(edge([-3.0, -5.0], [-1.0, -5.0], 0, 1));
        edges.push(edge([-1.0, -5.0], [5.0, -5.0], 0, 0));
        let shape = radar_like(edges);
        let map = MovieMap::new(&shape, &shape.mesh());
        // The blue square, not the black one: 100 square pixels of 16 m2.
        let area: f32 = map
            .water
            .iter()
            .map(|[a, b, c]| {
                ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
            })
            .sum();
        assert!((area - 100.0 * MAP_SCALE * MAP_SCALE).abs() < 1.0, "{area}");
        let corner = map_to_world([10.0, 10.0]);
        assert!(map.water.iter().flatten().any(|p| *p == corner));
        assert!(
            map.water
                .iter()
                .flatten()
                .all(|p| p[0] > map_to_world([20.0, 0.0])[0])
        );
        assert_eq!(map.roads.len(), 2);
        assert_eq!(
            map.roads[0].points,
            [[-10.0, -10.0], [-5.0, -10.0], [-5.0, -5.0]].map(map_to_world)
        );
        assert_eq!(map.roads[0].width, Some(3.0 * MAP_SCALE));
        assert_eq!(map.roads[1].points.len(), 2);
    }

    #[test]
    fn a_curved_stroke_is_followed_in_pieces_of_about_a_pixel() {
        let mut e = edge([0.0, 0.0], [8.0, 0.0], 0, 1);
        e.control = Some([4.0, 4.0]);
        let shape = radar_like(vec![e]);
        let map = MovieMap::new(&shape, &[]);
        let line = &map.roads[0].points;
        assert!(line.len() >= 9, "{}", line.len());
        // Halfway along, the curve is halfway to its control point.
        assert!(line.contains(&map_to_world([4.0, 2.0])));
    }

    #[test]
    fn the_roads_shape_is_the_one_placed_as_roads_mc() {
        use crate::flash::player::{Place, Timeline};
        use std::rc::Rc;
        let place = |character, name: Option<&str>| {
            Control::Place(Place {
                depth: 1,
                character: Some(character),
                name: name.map(str::to_owned),
                ..Place::default()
            })
        };
        let timeline = |c| {
            Rc::new(Timeline {
                frames: vec![vec![c]],
                ..Timeline::default()
            })
        };
        let mut lib = Library::default();
        let shape = radar_like(vec![edge([0.0, 0.0], [1.0, 0.0], 0, 1)]);
        lib.shapes.insert(4, (shape.clone(), Vec::new()));
        lib.shapes.insert(6, (Shape::default(), Vec::new()));
        lib.sprites.insert(5, timeline(place(4, None)));
        lib.sprites.insert(7, timeline(place(6, None)));
        lib.sprites
            .insert(12, timeline(place(7, Some("zoneHolder_mc"))));
        assert!(roads_shape(&lib).is_none());
        lib.sprites.insert(13, timeline(place(5, Some("roads_mc"))));
        assert_eq!(roads_shape(&lib).map(|(s, _)| s), Some(&shape));
    }

    #[test]
    fn the_map_image_is_water_where_the_movie_has_it() {
        // One triangle over the island's top-left quarter (+x, +z).
        let map = MovieMap {
            water: vec![[[4000.0, 4000.0], [0.0, 4000.0], [4000.0, 0.0]]],
            roads: Vec::new(),
        };
        let rgba = map.image(8, 8000.0);
        let wet: Vec<usize> = rgba
            .chunks(4)
            .enumerate()
            .filter(|(_, p)| *p == WATER)
            .map(|(i, _)| i)
            .collect();
        // The pixels of the map's top-left 4 x 4 whose centres lie on or
        // above its diagonal x + y = 4.
        assert_eq!(wet, [0, 1, 2, 3, 8, 9, 10, 16, 17, 24]);
        assert!(rgba.chunks(4).filter(|p| *p == LAND).count() == 64 - wet.len());
    }
}
