//! Long ways over the island by its road network: the roads' own paths
//! ([`Road::points`], from the data), each drivable one ([`Road::drivable`])
//! joined to the others where a point of one lies within [`LINK`] of a
//! point of another (their ends meet at junctions, but the junctions' own
//! ports were not all matched to roads, so nearness stands in for them;
//! inferred). The shortest way along them between the road points nearest
//! two places, going off road to reach them costing [`OFF_ROAD`] times as
//! much, is found with a search over that graph.
//!
//! What it is for: a route over a grid of the ground (the alpha's
//! `route`) reaches a few hundred metres and needs the ground streamed in;
//! the roads reach across the island from the data alone (PmcCon004's
//! mountain base is 4.6 km of hills from where the contract starts her).

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use m2_math::glam::{Vec2, Vec3};

use crate::world::Road;

/// Points of two roads this near are joined, metres, flat. Invented: a
/// road's end meets the next at a junction a lane or two across.
pub const LINK: f32 = 12.0;
/// Going off road (to the first road point, from the last) costs this many
/// times its length. Invented.
pub const OFF_ROAD: f32 = 3.0;
/// Road ends this far apart are joined across country, metres, flat
/// ([`RoadGraph::new`]). Invented.
pub const GAP: f32 = 400.0;
/// The grid the points are bucketed in for finding near ones, metres.
const BUCKET: f32 = 64.0;

/// The drivable roads as one graph.
#[derive(Debug, Default)]
pub struct RoadGraph {
    points: Vec<Vec3>,
    /// Each point's neighbours and how far.
    edges: Vec<Vec<(usize, f32)>>,
    buckets: HashMap<(i32, i32), Vec<usize>>,
}

fn bucket(p: Vec3) -> (i32, i32) {
    ((p.x / BUCKET).floor() as i32, (p.z / BUCKET).floor() as i32)
}

fn flat(a: Vec3, b: Vec3) -> f32 {
    Vec2::new(a.x - b.x, a.z - b.z).length()
}

#[derive(PartialEq)]
struct Open(f32, usize);

impl Eq for Open {}

impl Ord for Open {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.total_cmp(&self.0)
    }
}

impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl RoadGraph {
    /// The graph of `roads`' drivable ones.
    pub fn new(roads: &[Road]) -> Self {
        let mut g = Self::default();
        let mut road_of = Vec::new();
        let mut end = Vec::new();
        for (r, road) in roads.iter().enumerate().filter(|(_, r)| r.drivable()) {
            for (i, &p) in road.points.iter().enumerate() {
                let n = g.points.len();
                g.points.push(p);
                g.edges.push(Vec::new());
                road_of.push(r);
                end.push(i == 0 || i + 1 == road.points.len());
                g.buckets.entry(bucket(p)).or_default().push(n);
                if i > 0 {
                    let d = p.distance(g.points[n - 1]);
                    g.edges[n].push((n - 1, d));
                    g.edges[n - 1].push((n, d));
                }
            }
        }
        for n in 0..g.points.len() {
            let p = g.points[n];
            for m in g.near(p, LINK) {
                if road_of[m] != road_of[n] && m > n {
                    let d = p.distance(g.points[m]);
                    g.edges[n].push((m, d));
                    g.edges[m].push((n, d));
                }
            }
        }
        // A road's end with no other road at it is joined across country
        // to the ends of others within GAP, at the off-road cost: the
        // jungle's and the mountains' tracks stop short of each other
        // (PmcCon004's base: 41 drivable roads within 1500 m of it, none
        // joined to the rest within LINK). Invented.
        let ends: Vec<usize> = (0..g.points.len()).filter(|&n| end[n]).collect();
        for &n in &ends {
            let p = g.points[n];
            for m in g.near(p, GAP) {
                let d = p.distance(g.points[m]);
                if end[m] && m > n && road_of[m] != road_of[n] && d > LINK {
                    g.edges[n].push((m, d * OFF_ROAD));
                    g.edges[m].push((n, d * OFF_ROAD));
                }
            }
        }
        g
    }

    /// How many road points it has.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Whether it has none.
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// The points within `reach` of `at`, flat.
    fn near(&self, at: Vec3, reach: f32) -> Vec<usize> {
        let (bx, bz) = bucket(at);
        let r = (reach / BUCKET).ceil() as i32;
        let mut out = Vec::new();
        for x in bx - r..=bx + r {
            for z in bz - r..=bz + r {
                if let Some(b) = self.buckets.get(&(x, z)) {
                    out.extend(
                        b.iter()
                            .copied()
                            .filter(|&n| flat(self.points[n], at) <= reach),
                    );
                }
            }
        }
        out
    }

    /// The way by road from `from` to `to`: the road points to drive
    /// through, in order. Roads are got onto within `reach` of each end,
    /// metres. None if no road is that near either end, or none joins
    /// them.
    pub fn way(&self, from: Vec3, to: Vec3, reach: f32) -> Option<Vec<Vec3>> {
        let starts = self.near(from, reach);
        let ends: HashMap<usize, f32> = self
            .near(to, reach)
            .into_iter()
            .map(|n| (n, flat(self.points[n], to) * OFF_ROAD))
            .collect();
        if starts.is_empty() || ends.is_empty() {
            return None;
        }
        let mut cost = vec![f32::INFINITY; self.points.len()];
        let mut came: Vec<Option<usize>> = vec![None; self.points.len()];
        let mut open = BinaryHeap::new();
        for n in starts {
            let c = flat(self.points[n], from) * OFF_ROAD;
            if c < cost[n] {
                cost[n] = c;
                open.push(Open(c, n));
            }
        }
        let mut best: Option<(f32, usize)> = None;
        while let Some(Open(c, n)) = open.pop() {
            if c > cost[n] {
                continue;
            }
            if best.is_some_and(|b| c >= b.0) {
                break;
            }
            if let Some(&off) = ends.get(&n)
                && best.is_none_or(|b| c + off < b.0)
            {
                best = Some((c + off, n));
            }
            for &(m, d) in &self.edges[n] {
                let next = c + d;
                if next < cost[m] {
                    cost[m] = next;
                    came[m] = Some(n);
                    open.push(Open(next, m));
                }
            }
        }
        let (_, mut n) = best?;
        let mut way = vec![self.points[n]];
        while let Some(m) = came[n] {
            way.push(self.points[m]);
            n = m;
        }
        way.reverse();
        Some(way)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn road(points: &[[f32; 2]]) -> Road {
        Road {
            entity: 0,
            layer: 0,
            points: points.iter().map(|p| Vec3::new(p[0], 0.0, p[1])).collect(),
            width: 8.0,
            kind: 0,
            lanes: None,
            named_road: true,
        }
    }

    #[test]
    fn a_way_follows_roads_that_meet_and_not_the_ones_that_dont() {
        // An L of two roads meeting at (100, 0), and one off by itself.
        let roads = [
            road(&[[0.0, 0.0], [50.0, 0.0], [100.0, 0.0]]),
            road(&[[105.0, 0.0], [105.0, 50.0], [105.0, 100.0]]),
            road(&[[500.0, 500.0], [600.0, 500.0]]),
        ];
        let g = RoadGraph::new(&roads);
        assert_eq!(g.len(), 8);
        let way = g
            .way(
                Vec3::new(-10.0, 0.0, 0.0),
                Vec3::new(105.0, 0.0, 110.0),
                30.0,
            )
            .unwrap();
        let flat: Vec<[f32; 2]> = way.iter().map(|p| [p.x, p.z]).collect();
        assert_eq!(
            flat,
            [
                [0.0, 0.0],
                [50.0, 0.0],
                [100.0, 0.0],
                [105.0, 0.0],
                [105.0, 50.0],
                [105.0, 100.0]
            ]
        );
        // The lone road joins nothing.
        assert!(
            g.way(Vec3::ZERO, Vec3::new(600.0, 0.0, 500.0), 30.0)
                .is_none()
        );
    }

    #[test]
    fn no_road_near_an_end_is_no_way() {
        let g = RoadGraph::new(&[road(&[[0.0, 0.0], [50.0, 0.0]])]);
        assert!(
            g.way(Vec3::new(0.0, 0.0, 500.0), Vec3::new(50.0, 0.0, 0.0), 100.0)
                .is_none()
        );
    }

    #[test]
    fn a_road_traffic_does_not_drive_is_left_out() {
        let mut kerb = road(&[[0.0, 0.0], [50.0, 0.0]]);
        kerb.named_road = false;
        assert!(RoadGraph::new(&[kerb]).is_empty());
    }
}
