//! Ways over the island, for a vehicle or on foot: a search over a grid of
//! cells laid on the ground between two places, each looked at with a ray
//! from above as the search reaches it, and the way from each to the next
//! with a ray across, knee high. Walls (a building's side, a fence, a
//! barricade, a cliff: something across that ray, or a jump in height from
//! one cell to the next) are never crossed, nor is the sea by a vehicle;
//! ground steeper than it takes, and the sea on foot, only where there is
//! no other way; of the rest, gentle ground is preferred. The way found is
//! then straightened where a straight drive keeps to ground as good.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use m2_engine::{Physics, VehicleId};
use m2_math::glam::{Vec2, Vec3};

/// The grid's cells, metres across (wider where the way is long, to keep
/// within [`MOST_CELLS`] a side), reaching this far round the straight way,
/// metres; this far if no way was found within that.
const MARGIN: f32 = 80.0;
const WIDE_MARGIN: f32 = 250.0;
const MOST_CELLS: usize = 320;
/// The search stops after this many cells, taking the way to the one
/// closest to the goal.
const MOST_SEARCHED: usize = 40_000;
/// Rays down onto the cells start this high and reach this far, metres
/// (the island's ground lies between).
const TOP: f32 = 600.0;
const DEPTH: f32 = 1200.0;
/// Ground less than this above the sea is under it, metres.
const SHORE: f32 = 0.5;
/// A rise steeper than this from one cell to the next (rise over run) is
/// a wall, not ground.
const WALL: f32 = 1.0;
/// The ray from cell to cell runs this high over the ground, metres, and a
/// face it hits is a wall where its normal's height is under this.
const KNEE: f32 = 0.9;
const UPRIGHT: f32 = 0.7;
/// Ground steeper than it takes (or the sea, on foot) costs this many
/// times as much to cross as flat ground.
const STEEP: f32 = 20.0;
/// A straightened stretch of the way is at most this long, metres.
const STRETCH: f32 = 80.0;
/// The eight cells round a cell.
const AROUND: [(isize, isize); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

/// What goes the way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mover {
    /// The cells' size, metres (finer on foot, for gates and doors).
    pub cell: f32,
    /// The steepest ground it takes, radians.
    pub steepest: f32,
    /// Whether it goes into the sea (on foot) at all.
    pub wades: bool,
    /// The body its rays pass through (a vehicle's own).
    pub skip: Option<VehicleId>,
}

/// A way to go: the points to go through in turn, from where it starts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Route {
    /// Where it was planned to.
    pub goal: Vec3,
    /// From the start on, ending at the goal or as close to it as it gets.
    pub points: Vec<Vec3>,
    /// Whether it gets to the goal.
    pub reaches: bool,
    /// The leg being gone along: from `points[leg]` to `points[leg + 1]`.
    leg: usize,
}

impl Route {
    /// The way from `from` to `to` for `mover`, over ground higher than the
    /// sea at `sea`: within [`MARGIN`] of the straight way, else within
    /// [`WIDE_MARGIN`]. Straight there, not reaching, if no way gets any
    /// closer.
    pub fn plan(physics: &Physics, mover: Mover, from: Vec3, to: Vec3, sea: f32) -> Self {
        let route = Self::plan_within(physics, mover, from, to, sea, MARGIN);
        if route.reaches {
            return route;
        }
        let wide = Self::plan_within(physics, mover, from, to, sea, WIDE_MARGIN);
        if wide.reaches || wide.points.len() > 1 {
            wide
        } else {
            route
        }
    }

    fn plan_within(
        physics: &Physics,
        mover: Mover,
        from: Vec3,
        to: Vec3,
        sea: f32,
        margin: f32,
    ) -> Self {
        let (a, b) = (flat(from), flat(to));
        let lo = a.min(b) - Vec2::splat(margin);
        let span = a.max(b) + Vec2::splat(margin) - lo;
        let cell = mover.cell.max(span.max_element() / (MOST_CELLS - 1) as f32);
        let size = [
            (span.x / cell).ceil() as usize + 1,
            (span.y / cell).ceil() as usize + 1,
        ];
        let n = size[0] * size[1];
        let mut grid = Grid {
            physics,
            skip: mover.skip,
            origin: lo,
            cell,
            size,
            ground: vec![Ground::Unknown; n],
            looked: vec![0; n],
            walled: vec![0; n],
            grade: mover.steepest.tan(),
            flat: mover.steepest.cos(),
            wades: mover.wades,
            sea,
        };
        let (start, goal) = (grid.cell_at(a), grid.cell_at(b));
        let (cells, reaches) = grid.search(start, goal, b);
        if cells.len() < 2 {
            // Nowhere it can go gets any closer: straight at it.
            return Self {
                goal: to,
                points: vec![from, to],
                reaches: false,
                leg: 0,
            };
        }
        // Straightened: from each cell kept, on to the furthest of the
        // cells after it that a straight drive gets to over ground as
        // good.
        let mut kept = vec![cells[0]];
        let mut k = 0;
        while k + 1 < cells.len() {
            let mut next = k + 1;
            for j in k + 2..cells.len() {
                if grid.at(cells[k]).distance(grid.at(cells[j])) > STRETCH
                    || !grid.straight(cells[k], cells[j])
                {
                    break;
                }
                next = j;
            }
            kept.push(cells[next]);
            k = next;
        }
        let mut points: Vec<Vec3> = kept
            .iter()
            .map(|&c| {
                let p = grid.at(c);
                Vec3::new(p.x, grid.height(c).unwrap_or(from.y), p.y)
            })
            .collect();
        points[0] = from;
        if reaches && let Some(last) = points.last_mut() {
            *last = to;
        }
        Self {
            goal: to,
            points,
            reaches,
            leg: 0,
        }
    }

    /// How far it goes, metres (over the ground's plan).
    pub fn length(&self) -> f32 {
        self.points
            .windows(2)
            .map(|w| flat(w[0]).distance(flat(w[1])))
            .sum()
    }

    /// How far it is from `pos` to the goal along the way, metres: from the
    /// point of the leg being gone along nearest `pos` (and straight on from
    /// the way's end when it doesn't reach the goal).
    pub fn remaining(&self, pos: Vec3) -> f32 {
        let n = self.points.len();
        if n < 2 {
            return flat(pos).distance(flat(self.goal));
        }
        let leg = self.leg.min(n - 2);
        let (off, t) = self.near(leg, pos);
        let next = flat(self.points[leg + 1]);
        let rest: f32 = self.points[leg + 1..]
            .windows(2)
            .map(|w| flat(w[0]).distance(flat(w[1])))
            .sum();
        let beyond = if self.reaches {
            0.0
        } else {
            flat(self.points[n - 1]).distance(flat(self.goal))
        };
        off + flat(self.points[leg]).lerp(next, t).distance(next) + rest + beyond
    }

    /// How far `pos` is from leg `leg`, metres, and how far along it its
    /// nearest point is (0 at its start, 1 at its end).
    fn near(&self, leg: usize, pos: Vec3) -> (f32, f32) {
        let (a, b) = (flat(self.points[leg]), flat(self.points[leg + 1]));
        let (p, ab) = (flat(pos), b - a);
        let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
        (p.distance(a + ab * t), t)
    }

    /// The point to head for from `pos`: `ahead` metres on along the way
    /// from the point of it nearest `pos` (on the leg being gone along, or
    /// the next once that one is passed); and how far `pos` is off the way,
    /// metres.
    pub fn steer(&mut self, pos: Vec3, ahead: f32) -> (Vec3, f32) {
        let n = self.points.len();
        if n < 2 {
            return (self.points.first().copied().unwrap_or(self.goal), 0.0);
        }
        while self.leg + 2 < n {
            let (d, t) = self.near(self.leg, pos);
            if t < 1.0 && (t < 0.5 || self.near(self.leg + 1, pos).0 >= d) {
                break;
            }
            self.leg += 1;
        }
        let (off, t) = self.near(self.leg, pos);
        let mut from = self.points[self.leg].lerp(self.points[self.leg + 1], t);
        let mut left = ahead;
        for &to in &self.points[self.leg + 1..] {
            let d = flat(from).distance(flat(to));
            if d >= left {
                return (from.lerp(to, left / d), off);
            }
            left -= d;
            from = to;
        }
        (self.points[n - 1], off)
    }
}

/// What a cell's ray found.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ground {
    /// Not looked at yet.
    Unknown,
    /// Nothing (the island hasn't streamed in there): taken to be flat.
    Missing,
    /// The ground's height, and how flat it is there (its normal's
    /// height).
    At(f32, f32),
}

/// The cells, looked at as the search reaches them.
struct Grid<'a> {
    physics: &'a Physics,
    skip: Option<VehicleId>,
    /// The first cell's middle (x, z), metres, and the cells' size.
    origin: Vec2,
    cell: f32,
    size: [usize; 2],
    ground: Vec<Ground>,
    /// For each cell, the ways on to the cells round it ([`AROUND`], a bit
    /// each) looked across, and of those, the walled.
    looked: Vec<u8>,
    walled: Vec<u8>,
    /// The steepest slope it takes (rise over run), the least height of a
    /// normal of ground it takes, whether it goes into the sea, and the
    /// sea's height.
    grade: f32,
    flat: f32,
    wades: bool,
    sea: f32,
}

impl Grid<'_> {
    fn at(&self, c: [usize; 2]) -> Vec2 {
        self.origin + Vec2::new(c[0] as f32, c[1] as f32) * self.cell
    }

    /// The cell `p` is in (the nearest, if off the grid).
    fn cell_at(&self, p: Vec2) -> [usize; 2] {
        let q = ((p - self.origin) / self.cell).round();
        [
            q.x.clamp(0.0, (self.size[0] - 1) as f32) as usize,
            q.y.clamp(0.0, (self.size[1] - 1) as f32) as usize,
        ]
    }

    fn index(&self, c: [usize; 2]) -> usize {
        c[1] * self.size[0] + c[0]
    }

    fn cast(&self, from: Vec3, dir: Vec3, reach: f32) -> Option<(f32, Vec3)> {
        match self.skip {
            Some(id) => self.physics.cast_ray_past(from, dir, reach, id),
            None => self.physics.cast_ray(from, dir, reach),
        }
    }

    fn ground(&mut self, c: [usize; 2]) -> Ground {
        let i = self.index(c);
        if self.ground[i] == Ground::Unknown {
            let p = self.at(c);
            let hit = self.cast(Vec3::new(p.x, TOP, p.y), Vec3::NEG_Y, DEPTH);
            self.ground[i] = hit.map_or(Ground::Missing, |(t, n)| Ground::At(TOP - t, n.y));
        }
        self.ground[i]
    }

    fn height(&mut self, c: [usize; 2]) -> Option<f32> {
        match self.ground(c) {
            Ground::At(y, _) => Some(y),
            _ => None,
        }
    }

    /// Going from cell `a` onto the next, `b`: what it costs (its length,
    /// more the steeper), and whether it is steeper than it takes (or into
    /// the sea). None where it can't go: up or down a wall, across one, or
    /// into the sea when it doesn't wade.
    fn step(&mut self, a: [usize; 2], b: [usize; 2]) -> Option<(f32, bool)> {
        let run = self.at(a).distance(self.at(b));
        let (ga, gb) = (self.ground(a), self.ground(b));
        let wet = matches!(gb, Ground::At(y, _) if y < self.sea + SHORE);
        if wet && !self.wades {
            return None;
        }
        let (rise, up) = match (ga, gb) {
            (Ground::At(ya, _), Ground::At(yb, up)) => ((yb - ya).abs() / run, up),
            (_, Ground::At(_, up)) => (0.0, up),
            _ => (0.0, 1.0),
        };
        if rise > WALL || self.walled(a, b) {
            return None;
        }
        let steep = wet || rise > self.grade || up < self.flat;
        let (k, tilt) = (rise / self.grade, (1.0 - up) / (1.0 - self.flat));
        let cost = run * (1.0 + 3.0 * k * k + tilt * tilt) * if steep { STEEP } else { 1.0 };
        Some((cost, steep))
    }

    /// Whether something stands between cells `a` and `b`, next to each
    /// other: a ray from one to the other, [`KNEE`] high over the ground,
    /// hits a face too steep to be ground.
    fn walled(&mut self, a: [usize; 2], b: [usize; 2]) -> bool {
        let (Some(ya), Some(yb)) = (self.height(a), self.height(b)) else {
            return false;
        };
        let way = (b[0] as isize - a[0] as isize, b[1] as isize - a[1] as isize);
        let Some(k) = AROUND.iter().position(|&d| d == way) else {
            return false;
        };
        let (i, bit) = (self.index(a), 1u8 << k);
        if self.looked[i] & bit == 0 {
            let (pa, pb) = (self.at(a), self.at(b));
            let from = Vec3::new(pa.x, ya + KNEE, pa.y);
            let to = Vec3::new(pb.x, yb + KNEE, pb.y);
            let hit = self
                .cast(from, to - from, from.distance(to))
                .is_some_and(|(_, n)| n.y.abs() < UPRIGHT);
            self.looked[i] |= bit;
            if hit {
                self.walled[i] |= bit;
            }
        }
        self.walled[i] & bit != 0
    }

    /// Whether going from cell `a` onto `b` is fine: not steeper than it
    /// takes, not between two cells it can't go onto when diagonal.
    fn fine(&mut self, a: [usize; 2], b: [usize; 2]) -> bool {
        let corners = a[0] == b[0]
            || a[1] == b[1]
            || (self.step(a, [b[0], a[1]]).is_some() && self.step(a, [a[0], b[1]]).is_some());
        corners && self.step(a, b).is_some_and(|(_, steep)| !steep)
    }

    /// The cells from `start` to `goal` (at `to`), searched cheapest
    /// first; else to the cell closest to it the search got to (false
    /// then).
    fn search(&mut self, start: [usize; 2], goal: [usize; 2], to: Vec2) -> (Vec<[usize; 2]>, bool) {
        let n = self.size[0] * self.size[1];
        let mut cost = vec![f32::INFINITY; n];
        let mut came = vec![usize::MAX; n];
        let mut done = vec![false; n];
        let mut open = BinaryHeap::new();
        let s = self.index(start);
        cost[s] = 0.0;
        open.push(Open(self.at(start).distance(to), s));
        let mut best = (f32::INFINITY, s);
        let mut searched = 0;
        let mut reached = false;
        while let Some(Open(_, i)) = open.pop() {
            if done[i] {
                continue;
            }
            done[i] = true;
            let c = [i % self.size[0], i / self.size[0]];
            let left = self.at(c).distance(to);
            if left < best.0 {
                best = (left, i);
            }
            if c == goal {
                reached = true;
                break;
            }
            searched += 1;
            if searched >= MOST_SEARCHED {
                break;
            }
            for (dx, dz) in AROUND {
                let (Some(x), Some(z)) = (c[0].checked_add_signed(dx), c[1].checked_add_signed(dz))
                else {
                    continue;
                };
                if x >= self.size[0] || z >= self.size[1] {
                    continue;
                }
                let next = [x, z];
                let j = self.index(next);
                if done[j] {
                    continue;
                }
                // Onto the goal's cell whatever is there (a place on a
                // roof, by a wall): as close as it gets.
                let step = if next == goal {
                    Some((self.at(c).distance(self.at(next)), false))
                } else {
                    self.step(c, next)
                };
                let Some((k, _)) = step else {
                    continue;
                };
                // Not between two cells it can't go onto.
                if dx != 0
                    && dz != 0
                    && (self.step(c, [x, c[1]]).is_none() || self.step(c, [c[0], z]).is_none())
                {
                    continue;
                }
                let g = cost[i] + k;
                if g < cost[j] {
                    cost[j] = g;
                    came[j] = i;
                    open.push(Open(g + self.at(next).distance(to), j));
                }
            }
        }
        let end = if reached { self.index(goal) } else { best.1 };
        let mut path = vec![end];
        while let Some(&i) = path.last()
            && came[i] != usize::MAX
        {
            path.push(came[i]);
        }
        path.reverse();
        let cells = path
            .into_iter()
            .map(|i| [i % self.size[0], i / self.size[0]])
            .collect();
        (cells, reached)
    }

    /// Whether a straight way from cell `a` to `b` keeps to ground it
    /// takes, cell to cell.
    fn straight(&mut self, a: [usize; 2], b: [usize; 2]) -> bool {
        let (pa, pb) = (self.at(a), self.at(b));
        let steps = (pa.distance(pb) / (self.cell * 0.5)).ceil() as usize;
        let mut last = a;
        for k in 1..=steps {
            let c = self.cell_at(pa.lerp(pb, k as f32 / steps as f32));
            if c == last {
                continue;
            }
            if !self.fine(last, c) {
                return false;
            }
            last = c;
        }
        true
    }
}

/// A cell to search from: the least the way through it can cost to the
/// goal, and its index. Ordered for a max-heap to give the least first.
struct Open(f32, usize);

impl PartialEq for Open {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Open {}

impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Open {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.total_cmp(&self.0)
    }
}

fn flat(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_engine::{CollisionShape, StaticKey};
    use m2_math::glam::Quat;

    /// Ground on a 2 m grid over x -100..100, z -40..100, at the height
    /// `h` gives.
    fn island(h: impl Fn(f32, f32) -> f32) -> Physics {
        let (nx, nz) = (101, 71);
        let mut vertices = Vec::new();
        for k in 0..nz {
            for i in 0..nx {
                let (x, z) = (-100.0 + 2.0 * i as f32, -40.0 + 2.0 * k as f32);
                vertices.push(Vec3::new(x, h(x, z), z));
            }
        }
        let mut triangles = Vec::new();
        for k in 0..nz - 1 {
            for i in 0..nx - 1 {
                let a = (k * nx + i) as u32;
                let (b, c, d) = (a + 1, a + nx as u32, a + nx as u32 + 1);
                triangles.push([a, c, b]);
                triangles.push([b, c, d]);
            }
        }
        let mut p = Physics::new();
        let ground = CollisionShape::mesh(&vertices, &triangles).unwrap();
        p.add_static(StaticKey::Cell(1), &ground, Vec3::ZERO, Quat::IDENTITY);
        p.step(1.0 / 60.0);
        p
    }

    fn car(steepest: f32) -> Mover {
        Mover {
            cell: 4.0,
            steepest: steepest.to_radians(),
            wades: false,
            skip: None,
        }
    }

    // A ridge 6 m high across the way, too steep for a car (31 degrees),
    // with a cutting through it to the side: the way goes through the
    // cutting, not over the ridge.
    #[test]
    fn a_way_goes_round_ground_too_steep() {
        let p = island(|x, z| {
            let ridge = (6.0 - (z - 30.0).abs() * 0.6).max(0.0);
            if (40.0..=52.0).contains(&x) {
                0.0
            } else {
                ridge
            }
        });
        let (from, to) = (Vec3::ZERO, Vec3::new(0.0, 0.0, 60.0));
        let route = Route::plan(&p, car(22.0), from, to, -36.0);
        assert!(route.reaches, "{route:?}");
        assert_eq!(route.points.first(), Some(&from));
        assert_eq!(route.points.last(), Some(&to));
        // Along each leg, on flat ground all the way: through the cutting.
        for w in route.points.windows(2) {
            for k in 0..=20 {
                let at = w[0].lerp(w[1], k as f32 / 20.0);
                let top = Vec3::new(at.x, 50.0, at.z);
                let (t, _) = p.cast_ray(top, Vec3::NEG_Y, 100.0).unwrap();
                assert!(50.0 - t < 0.6, "over the ridge at {at}: {route:?}");
            }
        }
        assert!(route.length() > 90.0, "{}", route.length());
        // A tank-like vehicle that takes 35 degrees goes straight over.
        let over = Route::plan(&p, car(35.0), from, to, -36.0);
        assert_eq!(over.points, [from, to]);
    }

    // Flat ground: straight there (in stretches of at most 80 m).
    #[test]
    fn on_flat_ground_the_way_is_straight() {
        let p = island(|_, _| 0.0);
        let (from, to) = (Vec3::new(-20.0, 0.0, -10.0), Vec3::new(30.0, 0.0, 80.0));
        let route = Route::plan(&p, car(22.0), from, to, -36.0);
        assert!(route.reaches);
        assert!(route.points.len() <= 3, "{route:?}");
        let straight = flat(from).distance(flat(to));
        assert!(route.length() < straight * 1.02, "{route:?}");
    }

    // The sea between: round it by the dry ground; on foot too, though
    // she could wade.
    #[test]
    fn a_way_keeps_out_of_the_sea() {
        let p = island(|x, z| {
            if x < 30.0 && (20.0..40.0).contains(&z) {
                -40.0
            } else {
                0.0
            }
        });
        let to = Vec3::new(0.0, 0.0, 60.0);
        let route = Route::plan(&p, car(22.0), Vec3::ZERO, to, -36.0);
        assert!(route.reaches);
        assert!(route.points.iter().any(|q| q.x > 28.0), "{route:?}");
        let walker = Mover {
            cell: 2.0,
            steepest: 40f32.to_radians(),
            wades: true,
            skip: None,
        };
        let walk = Route::plan(&p, walker, Vec3::ZERO, to, -36.0);
        assert!(walk.points.iter().any(|q| q.x > 28.0), "{walk:?}");
        // Starting in it, she wades out.
        let from = Vec3::new(-50.0, -40.0, 30.0);
        let out = Route::plan(&p, walker, from, to, -36.0);
        assert!(out.reaches, "{out:?}");
    }

    // A thin wall across the way (too thin for the rays from above to
    // find), with a gate in it to the side: through the gate.
    #[test]
    fn a_way_goes_through_the_gate_in_a_wall() {
        let mut p = island(|_, _| 0.0);
        let quad = |a: Vec3, b: Vec3, c: Vec3, d: Vec3| {
            CollisionShape::mesh(&[a, b, c, d], &[[0, 1, 2], [0, 2, 3]]).unwrap()
        };
        for (k, (x0, x1)) in [(-100.0, 30.0), (36.0, 100.0)].into_iter().enumerate() {
            let wall = quad(
                Vec3::new(x0, 0.0, 31.0),
                Vec3::new(x1, 0.0, 31.0),
                Vec3::new(x1, 3.0, 31.0),
                Vec3::new(x0, 3.0, 31.0),
            );
            p.add_static(
                StaticKey::Cell(2 + k as u32),
                &wall,
                Vec3::ZERO,
                Quat::IDENTITY,
            );
        }
        p.step(1.0 / 60.0);
        let walker = Mover {
            cell: 2.0,
            steepest: 40f32.to_radians(),
            wades: true,
            skip: None,
        };
        let route = Route::plan(&p, walker, Vec3::ZERO, Vec3::new(0.0, 0.0, 60.0), -36.0);
        assert!(route.reaches, "{route:?}");
        // Where it crosses the wall's line, it is in the gate.
        let crossing = route
            .points
            .windows(2)
            .find(|w| w[0].z < 31.0 && w[1].z >= 31.0)
            .map(|w| w[0].lerp(w[1], (31.0 - w[0].z) / (w[1].z - w[0].z)))
            .expect("crosses the wall's line");
        assert!((30.0..=36.0).contains(&crossing.x), "{crossing}: {route:?}");
    }

    #[test]
    fn steering_looks_ahead_along_the_way() {
        let mut route = Route {
            goal: Vec3::new(20.0, 0.0, 20.0),
            points: vec![
                Vec3::ZERO,
                Vec3::new(0.0, 0.0, 20.0),
                Vec3::new(20.0, 0.0, 20.0),
            ],
            reaches: true,
            leg: 0,
        };
        // Halfway up the first leg, 2 m to its side: 5 m on.
        let (at, off) = route.steer(Vec3::new(2.0, 0.0, 10.0), 5.0);
        assert!((off - 2.0).abs() < 1e-4);
        assert!(at.distance(Vec3::new(0.0, 0.0, 15.0)) < 1e-4, "{at}");
        // Near the corner: round it.
        let (at, _) = route.steer(Vec3::new(0.0, 0.0, 18.0), 5.0);
        assert!(at.distance(Vec3::new(3.0, 0.0, 20.0)) < 1e-4, "{at}");
        // Past the corner, the first leg is left behind.
        let (at, off) = route.steer(Vec3::new(5.0, 0.0, 21.0), 5.0);
        assert_eq!(route.leg, 1);
        assert!((off - 1.0).abs() < 1e-4);
        assert!(at.distance(Vec3::new(10.0, 0.0, 20.0)) < 1e-4, "{at}");
        // At the end: the goal.
        let (at, _) = route.steer(Vec3::new(19.0, 0.0, 20.0), 5.0);
        assert_eq!(at, Vec3::new(20.0, 0.0, 20.0));
        assert!((route.remaining(Vec3::new(19.0, 0.0, 20.0)) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn what_is_left_to_go_is_along_the_way() {
        let route = Route {
            goal: Vec3::new(20.0, 0.0, 20.0),
            points: vec![
                Vec3::ZERO,
                Vec3::new(0.0, 0.0, 20.0),
                Vec3::new(20.0, 0.0, 20.0),
            ],
            reaches: true,
            leg: 0,
        };
        // 2 m off the first leg halfway up it: 2 back onto it, 10 up it,
        // 20 along the second.
        let left = route.remaining(Vec3::new(2.0, 0.0, 10.0));
        assert!((left - 32.0).abs() < 1e-4, "{left}");
        // A way that stops short: straight on from its end.
        let short = Route {
            reaches: false,
            goal: Vec3::new(20.0, 0.0, 30.0),
            ..route
        };
        let left = short.remaining(Vec3::new(0.0, 0.0, 10.0));
        assert!((left - 40.0).abs() < 1e-4, "{left}");
    }
}
