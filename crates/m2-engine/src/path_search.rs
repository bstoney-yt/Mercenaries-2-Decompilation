//! How an AI vehicle finds its way to a point round what is in its way:
//! the 360's path search (made by 0x822E64A8, stepped by 0x822E79F8 under
//! the manager 0x822E7CE0) and how a driver follows what it finds
//! (0x822E2770).
//!
//! From the 360 code:
//! - **Setting out** (0x822E6600): a node where the vehicle is and one at
//!   the goal (each put on the ground, 0x822E4120: a ray from 2 m above
//!   to 1000 m below), the goal's the way's first edge. A node keeps its
//!   parent, where it is, its cost (the parent's plus the distance) and
//!   the key of what it was made for.
//! - **The open edges** (+0x12BC, 16): each scored by its node's cost
//!   plus its straight distance to the goal (0x822E6720); a new one takes
//!   a free slot, else the worst's. Each step takes the best (0x822E7370),
//!   up to 256 steps (0x822E79F8).
//! - **An edge** (0x822E7688), parent to node, of different keys and not
//!   seen before as a pair (0x822E6A40): the vehicle's box (its width, its
//!   height less 0.25 m, 0.1 m long, 0x822E6FD8; 0x82166880 halves them)
//!   swept from 0.5 x height + 0.25 m over the parent to as high over the
//!   node.
//!   - Something in the way (0x822E74A8): new edges from the parent round
//!     it. The ground (its body's kind 0xD): one at where it was met. Any
//!     other body: the two corners of its box seen widest from the parent
//!     (0x822E3138: of its eight, those furthest round either way about
//!     the up axis), each pushed out sideways from that line of sight by
//!     1.8 x the clearance (0x822E3080), the clearance `0.7 x width +
//!     0.1`; from inside its box (where the search's height doesn't
//!     count), one where the line out from the box's origin meets a face,
//!     pushed out by as much. All at the height of the body's origin. Each
//!     node's key is the body and which of its corners (8 from inside), so
//!     that a node at one corner can go on to the next: a long wall is
//!     walked round corner by corner. Each corner is put on the ground and
//!     dropped, if what it stands on is another body's top (not the one
//!     the parent stands on) rising from the parent more than 0.2 of the
//!     way across (0x822E6BC0), for the same side's corner of that body in
//!     turn, up to three times (0x822E6C90). Then the body's own
//!     `AiHintNode` hardpoints (template component: `Hardpoint`, `Type`,
//!     `Name`; store 0x83653B38), each a node so (rising no more than 0.7
//!     of the way across).
//!   - A clear edge longer than 20 m (0x822E6EA8; its +0x10 is 400, the
//!     squared length) is split at 20 m when the ground there is more
//!     than 1 m off the line, as a new edge to the split point.
//!   - Otherwise the edge holds: its node is the best of its key
//!     (0x822E69D8, a cheaper one of the same key taking over its parent
//!     and cost); the goal's ends the search; any other's sends a new
//!     edge on to the goal.
//! - A new node at least 1 m cheaper than its key's best so far is dropped
//!   (0x822E67F8 keeps one only while the best is cheaper than it plus
//!   1 m); the cheaper way is taken up when an edge to it holds, by
//!   relinking the best (0x822E7688).
//! - The pairs seen (0x822E6A40, 128 slots) are emptied when a new pair
//!   comes with over 120 seen. The first node's key is 1 (0x822E6600),
//!   not 0, so its edges count as pairs too: one already tried is not
//!   added again (0x822E67F8).
//! - **Following** (0x822E2770): the point a lookahead on along the way
//!   from where the vehicle is nearest it, the lookahead the larger of
//!   3 m, half its box's largest side and its speed over the think.
//!
//! The sweeps and ground rays see the layers of
//! [`PATH_MASK`](crate::collision_layers::PATH_MASK): so not a chain-link
//! fence ([`PathSolid`](crate::physics::PathSolid)), which a vehicle drives
//! through and breaks. A model's shapes no vehicle meets (a rock's AI box,
//! layer 30's AI-only shapes) stand apart as a
//! [`Ghost`](crate::physics::Ghost), which the sweeps still see. Not
//! applied: a loose prop's layers (its own preset), and the leaves of
//! layer 19.
//!
//! Two of the 360's checks don't apply to a vehicle's search, so they are
//! not ported:
//! - A body of the search's goal entity (its +0x14, the mind's +0x340) is
//!   no obstacle (0x822E74A8). Only CarAttack's move on a message (event
//!   4, 0x822ACB28) gives 0x822BF7A8 a goal entity; TankAttack and the
//!   others pass none.
//! - A sweep's edge that leaves the box of the entity under its start
//!   (0x822E46B0: a ray down 20 m from 1 m up) where, 1 m either side of
//!   where it leaves, the ground falls more than 2 m (0x822E4358, two rays
//!   50 m down from 1 m over the higher end) is blocked (kind 6: no
//!   corners, only that entity's hints). The search's +0x1545 turns it
//!   on, and 0x822BEA78 sets that only when the mind's +0x14 top byte is
//!   0: its vehicle's class, which 0x822BB608 sets to 1 to 7 for any
//!   vehicle (0 with none).
//!
//! A body's box is its entity's (0x821861B0, 0x82182E90): a static's
//! model's box round all its shapes on parts it shows intact, leaves
//! included ([`CollisionShape::entity_box`](crate::physics::CollisionShape::entity_box)),
//! and a vehicle's round its hull, turret and gun. A loose piece's is its
//! own (whether the 360's pieces are bodies of the broken entity or
//! entities of their own isn't traced). Stand-ins: the search runs to its end at once
//! (the 360's manager spreads searches over frames, 8 steps a frame
//! within 1.5 ms); the way followed is the straight lines between its
//! nodes (the 360 follows Hermite curves through them, 0x8216D818, with
//! 0.7 of the lookahead); the 360's ground-snap's water check
//! (0x821D28D8) is not made.

use std::collections::{HashMap, HashSet};

use m2_assets::model::Node as ModelNode;
use m2_assets::{Layer, name_hash};
use m2_math::glam::{Mat4, Quat, Vec3};

/// One of a template's `AiHintNode` records (template component; the
/// 360's store 0x83653B38): the node of its model the hint is at (a name
/// hash), its `Type` (an `AiHintEnum`) and its `Name`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AiHint {
    pub hardpoint: u32,
    pub kind: u32,
    pub name: u32,
}

/// Every template's `AiHintNode` records, by its id.
pub fn read_hints(db: &Layer<'_>) -> HashMap<u32, Vec<AiHint>> {
    let t = crate::tuning::Table::new(db, "AiHintNode");
    let (hardpoint, kind, name) = (name_hash("Hardpoint"), name_hash("Type"), name_hash("Name"));
    t.templates()
        .map(|id| {
            let hints = t
                .of_template(id)
                .iter()
                .filter_map(|r| {
                    Some(AiHint {
                        hardpoint: t.word(r, hardpoint).filter(|&h| h != 0)?,
                        kind: t.word(r, kind).unwrap_or(0),
                        name: t.word(r, name).unwrap_or(0),
                    })
                })
                .collect();
            (id, hints)
        })
        .collect()
}

/// Where `hints` are on a model of `nodes` (model space): each the origin
/// of the node its hardpoint names (0x8241E3D0 finds an entity's
/// hardpoint by name), by the hardpoint's name hash; one whose node the
/// model lacks is left out (0x822E74A8 adds a node only when 0x8241E3D0
/// finds it).
pub fn hint_points(nodes: &[ModelNode], hints: &[AiHint]) -> Vec<(u32, Vec3)> {
    hints
        .iter()
        .filter_map(|h| {
            let mut at = nodes.iter().position(|n| n.name_hash == h.hardpoint)?;
            let mut m = Mat4::IDENTITY;
            // A node's parent chain ends within `nodes.len()` steps (the
            // model's parse rejects cycles).
            for _ in 0..=nodes.len() {
                let n = nodes.get(at)?;
                m = Mat4::from(n.local) * m;
                match n.parent {
                    Some(p) => at = usize::from(p),
                    None => return Some((h.hardpoint, m.w_axis.truncate())),
                }
            }
            None
        })
        .collect()
}

/// A node's key: what it was made for (the 360 combines two words,
/// 0x82855980).
pub type Key = (u64, u64);

/// The goal's key (0x80000000).
pub const GOAL: Key = (0x8000_0000, 0);

/// The first node's key (the 360's 1, 0x822E6600).
const FIRST: Key = (u64::MAX, 1);

/// What a sweep met.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obstacle {
    /// Where it was met.
    pub point: Vec3,
    /// Which body.
    pub id: u64,
    /// The ground (the 360's kind 1, its body's kind 0xD).
    pub ground: bool,
    /// Its box: its pose and its own bounds.
    pub pose: (Vec3, Quat),
    pub min: Vec3,
    pub max: Vec3,
}

/// What a path search asks of the world.
pub trait PathWorld {
    /// The first body a box of half extents `half`, turned to face along
    /// the sweep, meets going from `from` to `to`, if any.
    fn sweep(&mut self, from: Vec3, to: Vec3, half: Vec3) -> Option<Obstacle>;
    /// The ground under `at` (0x822E4120: a ray from 2 m above to 1000 m
    /// below, then from 502 m above to 500 m below if that met nothing):
    /// where, and the body it is the top of when not the ground itself.
    fn ground(&mut self, at: Vec3) -> Option<(Vec3, Option<u64>)>;
    /// Body `id`'s `AiHintNode` hardpoints in the world, by name hash.
    fn hints(&mut self, id: u64) -> Vec<(u32, Vec3)>;
    /// Body `id`'s box, as [`Obstacle`] gives one (its point unused).
    fn body(&mut self, id: u64) -> Option<Obstacle>;
}

#[derive(Clone, Copy, Debug)]
struct Node {
    at: Vec3,
    parent: Option<usize>,
    cost: f32,
    key: Key,
    /// The body it stands on (+0x3C), if not the ground.
    on: Option<u64>,
}

/// A path search's result: the way from the start to the goal.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Way {
    pub points: Vec<Vec3>,
}

/// The vehicle's size for the search: its width, its height above its
/// origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    /// 0x822BEA78's `0.7 x width + 0.1`.
    pub fn clearance(&self) -> f32 {
        self.width * 0.7 + 0.1
    }
}

struct Search<'w, W: PathWorld> {
    world: &'w mut W,
    size: Size,
    goal: Vec3,
    goal_on: Option<u64>,
    nodes: Vec<Node>,
    open: [Option<usize>; 16],
    best: HashMap<Key, usize>,
    pairs: HashSet<(Key, Key)>,
    done: Option<usize>,
}

impl<W: PathWorld> Search<'_, W> {
    fn score(&self, n: usize) -> f32 {
        self.nodes[n].cost + (self.goal - self.nodes[n].at).length()
    }

    /// 0x822E6720.
    fn push(&mut self, n: usize) {
        let slot = self
            .open
            .iter()
            .position(Option::is_none)
            .unwrap_or_else(|| {
                (0..16)
                    .max_by(|&a, &b| {
                        let s = |i: usize| self.open[i].map_or(f32::MIN, |n| self.score(n));
                        s(a).total_cmp(&s(b))
                    })
                    .unwrap_or(0)
            });
        self.open[slot] = Some(n);
    }

    /// 0x822E6B10 + 0x822E67F8: a node at `at` from `parent` for `key`.
    fn add(&mut self, parent: usize, at: Vec3, key: Key, on: Option<u64>) {
        let p = &self.nodes[parent];
        let n = Node {
            at,
            parent: Some(parent),
            cost: p.cost + (at - p.at).length(),
            key,
            on,
        };
        if self.pairs.contains(&(p.key, key)) {
            return;
        }
        if let Some(&b) = self.best.get(&key)
            && self.nodes[b].cost >= n.cost + 1.0
        {
            return;
        }
        self.nodes.push(n);
        let i = self.nodes.len() - 1;
        self.push(i);
    }

    /// 0x822E6BC0: `at` put on the ground and added from `parent`, unless
    /// it stands on a body other than the parent's, its top rising more
    /// than `slope` of the way across from the parent; then that body.
    fn add_grounded(&mut self, parent: usize, at: Vec3, key: Key, slope: f32) -> Option<u64> {
        let (g, on) = self.world.ground(at).unwrap_or((at, None));
        let (p, under) = (self.nodes[parent].at, self.nodes[parent].on);
        if let Some(b) = on
            && Some(b) != under
        {
            let d = (g - p).with_y(0.0).length();
            if g.y - p.y > d * slope {
                return Some(b);
            }
        }
        self.add(parent, g, key, on);
        None
    }

    /// 0x822E3138: the two corners of `o`'s box seen widest from `from`,
    /// left then right, pushed out sideways by `clear`; or, from inside it,
    /// one out at a face. Each with which corner it is, at the height of
    /// `o`'s origin.
    fn corners(o: &Obstacle, from: Vec3, clear: f32) -> Vec<(Vec3, u64)> {
        let (at, turn) = o.pose;
        let inv = turn.inverse();
        // At the box's own height.
        let local = inv * (from - at).with_y(0.0);
        let inside = (o.min.cmple(local) & local.cmple(o.max)).all();
        if inside {
            // 0x822E48E0: where the line out from the origin through
            // `local` meets the box.
            let side = |a: usize| if local[a] >= 0.0 { o.max[a] } else { o.min[a] };
            let (sx, sz) = (side(0), side(2));
            let a = if (sz * local.x).abs() <= (sx * local.z).abs() {
                2
            } else {
                0
            };
            let t = if local[a] == 0.0 {
                0.0
            } else {
                side(a) / local[a]
            };
            let mut p = local * t;
            p[a] += if p[a] <= 0.0 { -clear } else { clear };
            let w = at + turn * p;
            return vec![(Vec3::new(w.x, at.y, w.z), 8)];
        }
        let corner = |i: usize| {
            Vec3::new(
                if i & 4 != 0 { o.max.x } else { o.min.x },
                if i & 2 != 0 { o.max.y } else { o.min.y },
                if i & 1 != 0 { o.max.z } else { o.min.z },
            )
        };
        let mut left = 0;
        let mut right = 0;
        let flat = |v: Vec3| v.with_y(0.0);
        let d0 = flat(corner(0) - local);
        let (mut l, mut r) = (d0, d0);
        for i in 1..8 {
            let d = flat(corner(i) - local);
            // Further round to the left (+ about up) or right.
            if l.cross(d).y > 0.0 {
                l = d;
                left = i;
            }
            if r.cross(d).y < 0.0 {
                r = d;
                right = i;
            }
        }
        [(left, clear), (right, -clear)]
            .into_iter()
            .map(|(i, out)| {
                let w = at + turn * corner(i);
                // 0x822E3080: sideways from the line of sight, away from
                // the rest of the box.
                let d = (w - from).with_y(0.0);
                let across = Vec3::new(d.z, 0.0, -d.x).normalize_or_zero();
                let p = w + across * out;
                (Vec3::new(p.x, at.y, p.z), i as u64)
            })
            .collect()
    }

    /// 0x822E74A8, 0x822E6DC8 and 0x822E6C90: new edges from `parent`
    /// round `o`.
    fn detour(&mut self, parent: usize, o: &Obstacle) {
        if o.ground {
            let key = (
                u64::from(o.point.x.round() as i32 as u32),
                u64::from(o.point.z.round() as i32 as u32),
            );
            self.add_grounded(parent, o.point, key, 0.2);
            return;
        }
        let from = self.nodes[parent].at;
        let clear = self.size.clearance() * 1.8;
        for (side, (at, corner)) in Self::corners(o, from, clear).into_iter().enumerate() {
            let mut on = self.add_grounded(parent, at, (o.id, corner), 0.2);
            for _ in 0..3 {
                let Some(body) = on.and_then(|b| self.world.body(b)) else {
                    break;
                };
                let round = Self::corners(&body, from, clear);
                if round.len() < 2 {
                    break;
                }
                let (at, corner) = round[side];
                on = self.add_grounded(parent, at, (body.id, corner), 0.2);
            }
        }
        for (hash, at) in self.world.hints(o.id) {
            self.add_grounded(parent, at, (o.id, 0x1_0000_0000 | u64::from(hash)), 0.7);
        }
    }

    /// 0x822E7688: the edge to node `n` from its parent.
    fn expand(&mut self, n: usize) {
        let Some(p) = self.nodes[n].parent else {
            return;
        };
        let (pk, nk) = (self.nodes[p].key, self.nodes[n].key);
        // 0x822E6A40: seen already, or kept (the set emptied first when
        // over 120).
        if self.pairs.contains(&(pk, nk)) {
            return;
        }
        if self.pairs.len() + 8 > 128 {
            self.pairs.clear();
        }
        self.pairs.insert((pk, nk));
        if pk == nk {
            return;
        }
        let up = Vec3::Y * (self.size.height * 0.5 + 0.25);
        let half = Vec3::new(self.size.width, self.size.height - 0.25, 0.1) * 0.5;
        let (a, b) = (self.nodes[p].at, self.nodes[n].at);
        if let Some(o) = self.world.sweep(a + up, b + up, half) {
            self.detour(p, &o);
            return;
        }
        // 0x822E6EA8: split a long edge where the ground leaves the line.
        let d = b - a;
        if d.length_squared() > 400.0 {
            let mid = a + d * (400.0 / d.length_squared()).sqrt();
            if let Some((g, on)) = self.world.ground(mid)
                && (g.y - mid.y).abs() > 1.0
            {
                self.add(p, g, (nk.0 ^ 0x5151, nk.1 ^ p as u64), on);
                return;
            }
        }
        match self.best.get(&nk).copied() {
            None => {
                self.best.insert(nk, n);
                if nk == GOAL {
                    self.done = Some(n);
                } else {
                    let goal = self.goal;
                    let on = self.goal_on;
                    self.add(n, goal, GOAL, on);
                }
            }
            Some(b) if self.nodes[n].cost < self.nodes[b].cost => {
                self.nodes[b].parent = self.nodes[n].parent;
                self.nodes[b].cost = self.nodes[n].cost;
            }
            Some(_) => {}
        }
    }
}

/// Finds a way for a vehicle of `size` from `start` to `goal`
/// (0x822E64A8, stepped to its end).
pub fn find(world: &mut impl PathWorld, size: Size, start: Vec3, goal: Vec3) -> Option<Way> {
    let start = world.ground(start).map_or(start, |(g, _)| g);
    let (goal, goal_on) = world.ground(goal).unwrap_or((goal, None));
    let mut s = Search {
        world,
        size,
        goal,
        goal_on,
        nodes: vec![Node {
            at: start,
            parent: None,
            cost: 0.0,
            key: FIRST,
            on: None,
        }],
        open: [None; 16],
        best: HashMap::new(),
        pairs: HashSet::new(),
        done: None,
    };
    s.nodes.push(Node {
        at: goal,
        parent: Some(0),
        cost: (goal - start).length(),
        key: GOAL,
        on: goal_on,
    });
    s.push(1);
    for _ in 0..256 {
        if s.done.is_some() {
            break;
        }
        let Some(slot) = (0..16).filter(|&i| s.open[i].is_some()).min_by(|&a, &b| {
            s.score(s.open[a].unwrap())
                .total_cmp(&s.score(s.open[b].unwrap()))
        }) else {
            break;
        };
        let n = s.open[slot].take()?;
        s.expand(n);
    }
    let mut at = s.done?;
    let mut points = vec![s.nodes[at].at];
    while let Some(p) = s.nodes[at].parent {
        points.push(s.nodes[p].at);
        at = p;
    }
    points.reverse();
    Some(Way { points })
}

/// 0x822E2770's lookahead: the larger of 3 m, half the vehicle's box's
/// largest side and its speed over the think.
pub fn lookahead(largest_side: f32, speed: f32, dt: f32) -> f32 {
    3f32.max(largest_side * 0.5).max(speed * dt)
}

impl Way {
    /// The point `ahead` metres on along the way from where it passes
    /// nearest `at` (its end once past it).
    pub fn ahead_of(&self, at: Vec3, ahead: f32) -> Vec3 {
        let p = &self.points;
        if p.len() < 2 {
            return p.first().copied().unwrap_or(at);
        }
        let flat = |v: Vec3| v.with_y(0.0);
        let mut best = (f32::MAX, 0usize, 0.0f32);
        for i in 0..p.len() - 1 {
            let (a, b) = (flat(p[i]), flat(p[i + 1]));
            let ab = b - a;
            let t = ((flat(at) - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            let d = (a + ab * t).distance_squared(flat(at));
            if d < best.0 {
                best = (d, i, t);
            }
        }
        let (_, mut i, t) = best;
        let mut left = ahead + (p[i + 1] - p[i]).length() * t;
        loop {
            let seg = (p[i + 1] - p[i]).length();
            if left <= seg || i + 2 >= p.len() {
                return p[i] + (p[i + 1] - p[i]) * (left / seg.max(1e-6)).min(1.0);
            }
            left -= seg;
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A flat world with one 10 m box standing across the way.
    struct Wall;
    impl PathWorld for Wall {
        fn sweep(&mut self, from: Vec3, to: Vec3, half: Vec3) -> Option<Obstacle> {
            // The wall: x -5..5, z 20..22.
            let (lo, hi) = (
                Vec3::new(-5.0 - half.x, 0.0, 20.0),
                Vec3::new(5.0 + half.x, 4.0, 22.0),
            );
            let d = to - from;
            let mut t0 = 0.0f32;
            let mut t1 = 1.0f32;
            for a in [0usize, 2] {
                if d[a].abs() < 1e-6 {
                    if from[a] < lo[a] || from[a] > hi[a] {
                        return None;
                    }
                    continue;
                }
                let (mut ta, mut tb) = ((lo[a] - from[a]) / d[a], (hi[a] - from[a]) / d[a]);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
            }
            (t0 <= t1).then(|| Obstacle {
                point: from + d * t0,
                id: 7,
                ground: false,
                pose: (Vec3::new(0.0, 0.0, 21.0), Quat::IDENTITY),
                min: Vec3::new(-5.0, 0.0, -1.0),
                max: Vec3::new(5.0, 4.0, 1.0),
            })
        }
        fn ground(&mut self, at: Vec3) -> Option<(Vec3, Option<u64>)> {
            Some((at.with_y(0.0), None))
        }
        fn hints(&mut self, _: u64) -> Vec<(u32, Vec3)> {
            Vec::new()
        }
        fn body(&mut self, _: u64) -> Option<Obstacle> {
            None
        }
    }

    #[test]
    fn it_finds_its_way_round_a_wall() {
        let size = Size {
            width: 2.0,
            height: 2.0,
        };
        let way = find(&mut Wall, size, Vec3::ZERO, Vec3::new(0.0, 0.0, 40.0)).unwrap();
        assert!(way.points.len() >= 3, "{way:?}");
        // Round one end of the wall, clear of it by the pushed-out corner.
        let mid = way.points[1];
        assert!(mid.x.abs() > 5.0 + size.clearance(), "{mid}");
        assert_eq!(*way.points.last().unwrap(), Vec3::new(0.0, 0.0, 40.0));
        // Following it: 3 m on from the start, toward that corner.
        let p = way.ahead_of(Vec3::ZERO, lookahead(4.0, 0.0, 0.1));
        assert!(
            (p.length() - 3.0).abs() < 1e-3 && p.x.signum() == mid.x.signum(),
            "{p}"
        );
    }

    /// [`Wall`] with a hint just behind it, straight between the start
    /// and the goal: the cheapest-looking node, and out of the start's
    /// reach.
    struct HintBehind;
    impl PathWorld for HintBehind {
        fn sweep(&mut self, from: Vec3, to: Vec3, half: Vec3) -> Option<Obstacle> {
            Wall.sweep(from, to, half)
        }
        fn ground(&mut self, at: Vec3) -> Option<(Vec3, Option<u64>)> {
            Wall.ground(at)
        }
        fn hints(&mut self, _: u64) -> Vec<(u32, Vec3)> {
            vec![(0x1234, Vec3::new(0.0, 0.0, 23.5))]
        }
        fn body(&mut self, _: u64) -> Option<Obstacle> {
            None
        }
    }

    // The first node's edges count as pairs seen (its key is 1, not 0):
    // the edge to the hint, blocked once, is not tried again from it each
    // time, and the search goes round the wall.
    #[test]
    fn a_blocked_edge_from_the_start_is_not_tried_again() {
        let size = Size {
            width: 2.0,
            height: 2.0,
        };
        let way = find(&mut HintBehind, size, Vec3::ZERO, Vec3::new(0.0, 0.0, 40.0));
        let way = way.expect("a way round the wall");
        assert!(way.points[1].x.abs() > 5.0, "{way:?}");
    }

    #[test]
    fn hints_are_at_their_nodes_origins() {
        use m2_math::RawMatrix44;
        let at = |x: f32, y: f32, z: f32| {
            let mut m = [[0.0f32; 4]; 4];
            for (i, r) in m.iter_mut().enumerate() {
                r[i] = 1.0;
            }
            m[3] = [x, y, z, 1.0];
            RawMatrix44 { m }
        };
        let nodes = [
            ModelNode {
                name_hash: 10,
                local: at(1.0, 0.0, 0.0),
                ..Default::default()
            },
            ModelNode {
                name_hash: 20,
                parent: Some(0),
                local: at(0.0, 2.0, 3.0),
                ..Default::default()
            },
        ];
        let hint = |hardpoint| AiHint {
            hardpoint,
            kind: 1,
            name: 0,
        };
        assert_eq!(
            hint_points(&nodes, &[hint(20), hint(99), hint(10)]),
            vec![
                (20, Vec3::new(1.0, 2.0, 3.0)),
                (10, Vec3::new(1.0, 0.0, 0.0))
            ]
        );
    }

    /// A wall 60 m long and 8 m thick (x -30..30, z 10..18): no one corner
    /// of it sees the goal behind it.
    struct Thick;
    impl PathWorld for Thick {
        fn sweep(&mut self, from: Vec3, to: Vec3, half: Vec3) -> Option<Obstacle> {
            let (lo, hi) = (
                Vec3::new(-30.0 - half.x, 0.0, 10.0 - half.x),
                Vec3::new(30.0 + half.x, 4.0, 18.0 + half.x),
            );
            let d = to - from;
            let (mut t0, mut t1) = (0.0f32, 1.0f32);
            for a in [0usize, 2] {
                if d[a].abs() < 1e-6 {
                    if from[a] < lo[a] || from[a] > hi[a] {
                        return None;
                    }
                    continue;
                }
                let (mut ta, mut tb) = ((lo[a] - from[a]) / d[a], (hi[a] - from[a]) / d[a]);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
            }
            (t0 < t1).then(|| Obstacle {
                point: from + d * t0,
                id: 7,
                ground: false,
                pose: (Vec3::new(0.0, 0.0, 14.0), Quat::IDENTITY),
                min: Vec3::new(-30.0, 0.0, -4.0),
                max: Vec3::new(30.0, 4.0, 4.0),
            })
        }
        fn ground(&mut self, at: Vec3) -> Option<(Vec3, Option<u64>)> {
            Some((at.with_y(0.0), None))
        }
        fn hints(&mut self, _: u64) -> Vec<(u32, Vec3)> {
            Vec::new()
        }
        fn body(&mut self, _: u64) -> Option<Obstacle> {
            None
        }
    }

    #[test]
    fn it_goes_round_a_long_wall_corner_by_corner() {
        let size = Size {
            width: 3.2,
            height: 2.6,
        };
        let way = find(&mut Thick, size, Vec3::ZERO, Vec3::new(0.0, 0.0, 60.0)).unwrap();
        // Two corners of the one wall: its near end, then its far end.
        assert_eq!(way.points.len(), 4, "{way:?}");
        assert!(way.points[1].z < way.points[2].z, "{way:?}");
        assert_eq!(way.points[1].x.signum(), way.points[2].x.signum());
    }

    #[test]
    fn the_corners_are_of_the_bodys_own_box_at_its_height() {
        let o = Obstacle {
            point: Vec3::ZERO,
            id: 1,
            ground: false,
            pose: (Vec3::new(0.0, 5.0, 10.0), Quat::IDENTITY),
            min: Vec3::new(-2.0, 0.0, -1.0),
            max: Vec3::new(2.0, 3.0, 1.0),
        };
        type S<'a> = Search<'a, Thick>;
        let c = S::corners(&o, Vec3::ZERO, 1.0);
        assert_eq!(c.len(), 2);
        // Its near corners (z min), at its height, clear of its box.
        assert_eq!(c[0].1 & 1, 0);
        assert_eq!(c[1].1 & 1, 0);
        assert_ne!(c[0].1 & 4, c[1].1 & 4);
        for (p, _) in &c {
            assert_eq!(p.y, 5.0);
            assert!(p.x.abs() > 2.5, "{p}");
        }
        // From inside, one at a face, keyed apart from the eight.
        let c = S::corners(&o, Vec3::new(0.5, 0.0, 10.25), 1.0);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].1, 8);
        assert!((c[0].0.z - 12.0).abs() < 0.01, "{}", c[0].0);
        assert!((c[0].0.x - 2.0).abs() < 0.01, "{}", c[0].0);
    }

    #[test]
    fn a_clear_way_is_straight() {
        struct Open;
        impl PathWorld for Open {
            fn sweep(&mut self, _: Vec3, _: Vec3, _: Vec3) -> Option<Obstacle> {
                None
            }
            fn ground(&mut self, at: Vec3) -> Option<(Vec3, Option<u64>)> {
                Some((at.with_y(0.0), None))
            }
            fn hints(&mut self, _: u64) -> Vec<(u32, Vec3)> {
                Vec::new()
            }
            fn body(&mut self, _: u64) -> Option<Obstacle> {
                None
            }
        }
        let size = Size {
            width: 2.0,
            height: 2.0,
        };
        let way = find(&mut Open, size, Vec3::ZERO, Vec3::new(30.0, 0.0, 0.0)).unwrap();
        assert_eq!(way.points, vec![Vec3::ZERO, Vec3::new(30.0, 0.0, 0.0)]);
    }
}
