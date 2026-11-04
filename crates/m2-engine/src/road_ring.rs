//! Where a pursuit's cars and tanks come in: points a set distance from
//! her along the road network, ahead of her or behind, as the 360's
//! pursuit finds them.
//!
//! From the 360 code:
//! - Each frame the traffic manager (0x822644E0) finds the road she is on
//!   or by (the nearest within [`NEAR`], 0x8223E1D8), how far along it she
//!   is (kept to 0.001..0.999) and which way she faces along it (+1 with
//!   it, -1 against: 0x82263C98; +1 when she is outside its strips), and
//!   floods the network out from her to [`REACH`] (0x8223F1E0,
//!   0x8223ECD0): her own road's entry first (0 to the farther of its two
//!   ends), then from the junction ahead of her (side +1) and the one
//!   behind (side -1) breadth first, each road met getting the distances
//!   along the network its two ends are from her, the way it was entered,
//!   the side, and how many junctions in; a road already met is kept
//!   unless met again no farther; a junction crossed adds
//!   [`JUNCTION_COST`]; at most [`MOST`] roads; the roads sorted by the
//!   near distance (0x8223FDA8).
//! - A ring at `r` metres (0x8223F3F8) is every point `r` along the
//!   network: on her own road one each way, else one on each road whose
//!   distances span `r`, as a fraction along it (from its far end when it
//!   was entered that way).
//! - On her own road the 360 takes the fraction as hers plus or minus `r`
//!   over the length left that way, not over the road's length (so a
//!   point can lie past its end); kept as it is.
//!
//! From samples: the road is the alpha's [`Road`] path, its length that
//! path's (the 360's +0x38), a point and its direction at a fraction
//! along the path by length (0x8223B8C8 evaluates the 360's curve); only drivable roads
//! ([`Road::drivable`]) take part (0x8223E9D8 takes roads of kind 2 or 4
//! only; which the alpha's kinds those are is inferred).

use std::collections::{HashMap, VecDeque};

use m2_math::glam::Vec3;

use crate::world::{Junction, Road};

/// How far off a road she may be and still be on it, metres (0x8223E1D8's
/// 20.0, as 0x822644E0 asks it).
pub const NEAR: f32 = 20.0;
/// How far out the network is flooded from her, metres (0x822644E0's
/// 200.0).
pub const REACH: f32 = 200.0;
/// Metres added for each junction crossed (0x8223ECD0's 15.0).
pub const JUNCTION_COST: f32 = 15.0;
/// The most roads the flood keeps, and points a ring gives (0x80).
pub const MOST: usize = 128;
/// The most roads a junction joins (the 360's six slots).
const SLOTS: usize = 6;

/// One road of the network: its path, the length along it at each point,
/// and the junctions at its first and last points.
#[derive(Clone, Debug)]
pub struct NetRoad {
    pub points: Vec<Vec3>,
    along: Vec<f32>,
    pub length: f32,
    pub start: Option<usize>,
    pub end: Option<usize>,
}

impl NetRoad {
    /// The point `frac` along it by length, and its direction there (the
    /// path's, kept to 0..1 at the ends).
    pub fn at(&self, frac: f32) -> (Vec3, Vec3) {
        let n = self.points.len();
        if n < 2 {
            return (self.points.first().copied().unwrap_or(Vec3::ZERO), Vec3::Z);
        }
        let d = frac.clamp(0.0, 1.0) * self.length;
        let i = self.along.partition_point(|&a| a <= d).clamp(1, n - 1);
        let (a, b) = (self.points[i - 1], self.points[i]);
        let seg = self.along[i] - self.along[i - 1];
        let t = if seg > 0.0 {
            (d - self.along[i - 1]) / seg
        } else {
            0.0
        };
        (a.lerp(b, t), (b - a).normalize_or(Vec3::Z))
    }

    /// Its direction at `frac` and the level line square to it, as
    /// 0x8223B8C8 gives them (that line `(-z, 0, x)` of the direction).
    pub fn frame(&self, frac: f32) -> (Vec3, Vec3, Vec3) {
        let (p, t) = self.at(frac);
        (p, t, Vec3::new(-t.z, 0.0, t.x).normalize_or_zero())
    }
}

/// The drivable roads and the junctions joining them.
#[derive(Clone, Debug, Default)]
pub struct RoadNet {
    pub roads: Vec<NetRoad>,
    /// Each junction's roads (at most six).
    pub junctions: Vec<Vec<usize>>,
}

impl RoadNet {
    /// The network of `roads`' drivable ones, joined by `junctions`: a
    /// road's two junctions are the ones its record names
    /// ([`Road::ends`]), a junction's roads the ones its own record names
    /// (its ports' roads), as the 360 reads them (0x8223ECD0: the road's
    /// +0x8 and +0xC; the junction's six slots).
    pub fn new(roads: &[Road], junctions: &[Junction]) -> Self {
        let mut index: HashMap<u32, Vec<usize>> = HashMap::new();
        let mut net = Self::default();
        let at: HashMap<u32, usize> = junctions
            .iter()
            .enumerate()
            .map(|(k, j)| (j.entity, k))
            .collect();
        for r in roads.iter().filter(|r| r.drivable() && r.points.len() >= 2) {
            let mut along = Vec::with_capacity(r.points.len());
            let mut d = 0.0;
            along.push(0.0);
            for w in r.points.windows(2) {
                d += w[0].distance(w[1]);
                along.push(d);
            }
            index.entry(r.entity).or_default().push(net.roads.len());
            let end = |e: u32| (e != 0).then(|| at.get(&e).copied()).flatten();
            net.roads.push(NetRoad {
                points: r.points.clone(),
                along,
                length: d,
                start: end(r.ends[0]),
                end: end(r.ends[1]),
            });
        }
        net.junctions = junctions
            .iter()
            .map(|j| {
                j.ports
                    .iter()
                    .flat_map(|(e, _)| index.get(e).into_iter().flatten().copied())
                    .take(SLOTS)
                    .collect()
            })
            .collect();
        net
    }

    /// The road nearest `at` (flat) within [`NEAR`], how far along it
    /// `at` is (0..1) and how far off.
    pub fn nearest(&self, at: Vec3) -> Option<(usize, f32, f32)> {
        let mut best: Option<(usize, f32, f32)> = None;
        let p = at.with_y(0.0);
        for (i, r) in self.roads.iter().enumerate() {
            for (k, w) in r.points.windows(2).enumerate() {
                let (a, b) = (w[0].with_y(0.0), w[1].with_y(0.0));
                let ab = b - a;
                let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
                let d = p.distance(a + ab * t);
                if d <= NEAR && best.is_none_or(|(_, _, bd)| d < bd) {
                    let along = r.along[k] + (r.along[k + 1] - r.along[k]) * t;
                    best = Some((i, along / r.length.max(1e-6), d));
                }
            }
        }
        best
    }
}

/// A road the flood met (0x8223EBB0's 16-byte entry): how far along the
/// network its near and far ends are from her, the road, how many
/// junctions in, the way it was entered (+1 from its start, -1 from its
/// end; 0 her own) and her side it is on (+1 ahead, -1 behind, 0 her
/// own road).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Met {
    pub near: f32,
    pub far: f32,
    pub road: usize,
    pub depth: u8,
    pub dir: i8,
    pub side: i8,
}

/// A point of a ring: the road, the fraction along it, her side (+1
/// ahead, -1 behind) and which way along the road leads away from her
/// (+1 its own way, -1 back).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingPoint {
    pub road: usize,
    pub frac: f32,
    pub side: i8,
    pub dir: i8,
}

/// The network flooded out from her (0x8223F1E0).
#[derive(Clone, Debug, PartialEq)]
pub struct Flood {
    /// Her road, how far along it (+0x4), its length (+0x8) and whether
    /// she faces its own way (+0xC).
    pub road: usize,
    pub frac: f32,
    pub length: f32,
    pub forward: bool,
    /// The roads met, nearest first.
    pub met: Vec<Met>,
}

impl Flood {
    /// Floods `net` from `frac` along `road` facing `dir` along it (+1
    /// its way, -1 against), out to `reach`. None where the road is too
    /// short to be on (0x8223F1E0's 1e-4).
    pub fn new(net: &RoadNet, road: usize, frac: f32, dir: i32, reach: f32) -> Option<Self> {
        let r = net.roads.get(road)?;
        let len = r.length;
        if len < 1e-4 {
            return None;
        }
        let forward = dir >= 0;
        let mut f = Self {
            road,
            frac,
            length: len,
            forward,
            met: Vec::new(),
        };
        if frac == 0.0 || frac == 1.0 {
            return Some(f);
        }
        let t = frac.clamp(0.0, 1.0);
        let mut slots: HashMap<usize, usize> = HashMap::new();
        f.meet(&mut slots, 0.0, t.max(1.0 - t) * len, road, 0, 0, 0);
        let (ahead, behind) = if forward {
            ((1.0 - t) * len, t * len)
        } else {
            (t * len, (1.0 - t) * len)
        };
        let (front, back) = if forward {
            (r.end, r.start)
        } else {
            (r.start, r.end)
        };
        if ahead < reach
            && let Some(j) = front
        {
            f.spread(net, &mut slots, j, ahead, reach, 1);
        }
        if behind < reach
            && let Some(j) = back
        {
            f.spread(net, &mut slots, j, behind, reach, -1);
        }
        // Nearest first (0x8223FDA8).
        f.met.sort_by(|a, b| a.near.total_cmp(&b.near));
        Some(f)
    }

    /// 0x8223EBB0: keeps a road met `near`..`far` from her unless it was
    /// met no farther before; whether it was kept.
    #[allow(clippy::too_many_arguments)]
    fn meet(
        &mut self,
        slots: &mut HashMap<usize, usize>,
        near: f32,
        far: f32,
        road: usize,
        depth: u8,
        dir: i8,
        side: i8,
    ) -> bool {
        if (near - far).abs() < 1e-4 {
            return false;
        }
        let m = Met {
            near,
            far,
            road,
            depth,
            dir,
            side,
        };
        match slots.get(&road) {
            Some(&k) => {
                if near <= self.met[k].near {
                    self.met[k] = m;
                    true
                } else {
                    false
                }
            }
            None if self.met.len() < MOST => {
                slots.insert(road, self.met.len());
                self.met.push(m);
                true
            }
            None => false,
        }
    }

    /// 0x8223ECD0: breadth first from junction `from`, `dist` from her,
    /// on her `side`.
    fn spread(
        &mut self,
        net: &RoadNet,
        slots: &mut HashMap<usize, usize>,
        from: usize,
        dist: f32,
        reach: f32,
        side: i8,
    ) {
        let mut queue: VecDeque<(usize, u8, f32)> = VecDeque::with_capacity(MOST);
        queue.push_back((from, 1, dist));
        while let Some((j, depth, d)) = queue.pop_front() {
            let Some(roads) = net.junctions.get(j) else {
                continue;
            };
            let mut onward = Vec::new();
            for &road in roads {
                let r = &net.roads[road];
                let far = r.length + d;
                let dir = if r.end == Some(j) { -1 } else { 1 };
                if self.meet(slots, d, far, road, depth, dir, side) && far < reach {
                    onward.push((road, far));
                }
            }
            for (road, far) in onward {
                let r = &net.roads[road];
                for next in [r.end, r.start].into_iter().flatten() {
                    if next != j && queue.len() < MOST {
                        queue.push_back((next, depth.saturating_add(1), far + JUNCTION_COST));
                    }
                }
            }
        }
    }

    /// The points `r` metres out along the network (0x8223F3F8).
    pub fn ring(&self, r: f32) -> Vec<RingPoint> {
        let mut out = Vec::new();
        for m in &self.met {
            if out.len() >= MOST || r < m.near {
                break;
            }
            if m.far < r {
                continue;
            }
            if m.side != 0 {
                let mut frac = (r - m.near) / (m.far - m.near);
                if m.dir == -1 {
                    frac = 1.0 - frac;
                }
                out.push(RingPoint {
                    road: m.road,
                    frac,
                    side: m.side,
                    dir: m.dir,
                });
                continue;
            }
            // Her own road: one point each way, as the 360 works it.
            let t = self.frac;
            for (side, way) in [(1i8, self.forward), (-1i8, !self.forward)] {
                if out.len() >= MOST {
                    break;
                }
                let left = if way { 1.0 - t } else { t };
                let span = self.length * left;
                if span < 1e-4 || r > span {
                    continue;
                }
                let frac = if way { t + r / span } else { t - r / span };
                out.push(RingPoint {
                    road: m.road,
                    frac,
                    side,
                    dir: if way { 1 } else { -1 },
                });
            }
        }
        out
    }
}

/// Where a pursuit unit comes into play and how: its place, the way it
/// faces and its velocity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arrival {
    pub at: Vec3,
    pub facing: Vec3,
    pub velocity: Vec3,
}

/// Whether the road bends within 8 m of `frac` (0x82239548): the level
/// line square to it there against the same 8 m on either side, or its
/// direction there against those, anywhere under 0.97; a road under 1 m
/// long counts as bent.
pub fn bends(road: &NetRoad, frac: f32) -> bool {
    if road.length < 1.0 {
        return true;
    }
    let d = 8.0 / road.length;
    let lo = (frac - d).max(0.0);
    let hi = (frac + d).min(1.0);
    let (_, t, n) = road.frame(frac);
    let (_, t_hi, n_hi) = road.frame(hi);
    let (_, t_lo, n_lo) = road.frame(lo);
    let least = n
        .dot(n_hi)
        .min(n.dot(n_lo))
        .min(t.dot(t_lo))
        .min(t.dot(t_hi));
    least < 0.97
}

/// How fast a car comes in on a slope: from `base` at a grade under 0.1
/// (of its facing's height) to `base + gain` on the level, nothing extra
/// at 0.2 or more (0x82238918, 0x822396A8: 1 under 0.1, then 1 - 5 x the
/// grade, 0 from 0.2).
fn coming(facing: Vec3, base: f32, gain: f32) -> f32 {
    let s = facing.y.abs();
    let k = if s < 0.1 {
        1.0
    } else if s < 0.2 {
        1.0 - s * 5.0
    } else {
        0.0
    };
    k * gain + base
}

/// The first point of `ring` on `side` and the frame there: the place,
/// the road's direction, its level line and the way away from her.
fn first_on(net: &RoadNet, ring: &[RingPoint], side: i8) -> Option<(RingPoint, Vec3, Vec3, Vec3)> {
    let p = *ring.iter().find(|p| p.side == side)?;
    let (at, t, n) = net.roads.get(p.road)?.frame(p.frac);
    Some((p, at, t, n))
}

/// The ring a car coming in from behind her takes and the one ahead, and
/// a tank's, metres (0x82238918's 60, 0x822396A8's 130, 0x8223A0A8's
/// 160).
pub const CAR_BEHIND: f32 = 60.0;
pub const CAR_AHEAD: f32 = 130.0;
pub const TANK_AHEAD: f32 = 160.0;

/// A car from behind her (0x82238918): at the first point [`CAR_BEHIND`]
/// back, facing her along the road, 4 m to the side, coming at 5 m/s
/// plus up to 15 more on the level (4 m/s where the road bends).
pub fn car_behind(net: &RoadNet, flood: &Flood) -> Option<Arrival> {
    let (p, at, t, n) = first_on(net, &flood.ring(CAR_BEHIND), -1)?;
    let facing = if p.dir > 0 { -t } else { t };
    let side = if p.dir > 0 { n * -4.0 } else { n * 4.0 };
    let speed = if bends(&net.roads[p.road], p.frac) {
        4.0
    } else {
        coming(facing, 5.0, 15.0)
    };
    Some(Arrival {
        at: at + side,
        facing,
        velocity: facing * speed,
    })
}

/// A car from ahead of her (0x822396A8): at the first point
/// [`CAR_AHEAD`] on, facing her, 4 m to the side (in the middle where the
/// road bends), coming at 5 m/s plus up to 10 more on the level (5 m/s
/// where it bends).
pub fn car_ahead(net: &RoadNet, flood: &Flood) -> Option<Arrival> {
    let (p, at, t, n) = first_on(net, &flood.ring(CAR_AHEAD), 1)?;
    let facing = if p.dir > 0 { -t } else { t };
    let bent = bends(&net.roads[p.road], p.frac);
    let side = if bent {
        Vec3::ZERO
    } else if p.dir > 0 {
        n * -4.0
    } else {
        n * 4.0
    };
    let speed = if bent { 5.0 } else { coming(facing, 5.0, 10.0) };
    Some(Arrival {
        at: at + side,
        facing,
        velocity: facing * speed,
    })
}

/// A tank from ahead of her (0x8223A0A8): at the first point
/// [`TANK_AHEAD`] on, facing her, `(0.5 - rand) x 15` m to the side (4 m
/// when it joins the traffic, `traffic`), 0.2 m up, coming at 6 m/s.
pub fn tank_ahead(net: &RoadNet, flood: &Flood, traffic: bool, rand: f32) -> Option<Arrival> {
    let (p, at, t, n) = first_on(net, &flood.ring(TANK_AHEAD), 1)?;
    let facing = if p.dir > 0 { -t } else { t };
    let off = if traffic {
        if p.dir > 0 { -4.0 } else { 4.0 }
    } else {
        (0.5 - rand) * 15.0
    };
    Some(Arrival {
        at: at + n * off + Vec3::Y * 0.2,
        facing,
        velocity: facing * 6.0,
    })
}

/// Which way she faces along `road` at `frac` (0x82263C98): +1 with it,
/// -1 against, +1 when she is off its strips (more than `half` from its
/// line).
pub fn facing_along(road: &NetRoad, frac: f32, her: Vec3, forward: Vec3, half: f32) -> i32 {
    let (at, t) = road.at(frac);
    if her.with_y(0.0).distance(at.with_y(0.0)) > half {
        return 1;
    }
    if t.dot(forward) >= 0.0 { 1 } else { -1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Roads along +x: 0 from x 0 to 100, 1 from 100 to 300, 2 from 300
    /// to 400, joined end to start at junctions 0 (x 100) and 1 (x 300).
    fn line() -> RoadNet {
        let road = |a: f32, b: f32, start, end| NetRoad {
            points: vec![Vec3::new(a, 0.0, 0.0), Vec3::new(b, 0.0, 0.0)],
            along: vec![0.0, b - a],
            length: b - a,
            start,
            end,
        };
        RoadNet {
            roads: vec![
                road(0.0, 100.0, None, Some(0)),
                road(100.0, 300.0, Some(0), Some(1)),
                road(300.0, 400.0, Some(1), None),
            ],
            junctions: vec![vec![0, 1], vec![1, 2]],
        }
    }

    // From the middle of the middle road facing +x: its own entry, then
    // the road ahead 100 m off (and 15 m for its junction past it never
    // reached), the road behind 100 m off.
    #[test]
    fn the_flood_spreads_both_ways_from_her() {
        let net = line();
        let f = Flood::new(&net, 1, 0.5, 1, REACH).unwrap();
        assert_eq!(f.met.len(), 3);
        assert_eq!((f.met[0].road, f.met[0].side), (1, 0));
        assert_eq!(f.met[0].far, 100.0);
        let ahead = f.met.iter().find(|m| m.road == 2).unwrap();
        assert_eq!(
            (ahead.near, ahead.far, ahead.side, ahead.dir),
            (100.0, 200.0, 1, 1)
        );
        let behind = f.met.iter().find(|m| m.road == 0).unwrap();
        assert_eq!(
            (behind.near, behind.far, behind.side, behind.dir),
            (100.0, 200.0, -1, -1)
        );
    }

    // A ring 130 m out: on the road ahead 30 m in from its start, on the
    // road behind 30 m in from its end. 60 m out: on her own road, by the
    // 360's own-road rule (60 over the 100 m left: 0.5 + 0.6).
    #[test]
    fn a_ring_lies_its_distance_out_along_the_roads() {
        let net = line();
        let f = Flood::new(&net, 1, 0.5, 1, REACH).unwrap();
        let ring = f.ring(130.0);
        let ahead = ring.iter().find(|p| p.side == 1).unwrap();
        assert_eq!(ahead.road, 2);
        assert!((ahead.frac - 0.3).abs() < 1e-5, "{}", ahead.frac);
        let behind = ring.iter().find(|p| p.side == -1).unwrap();
        assert_eq!(behind.road, 0);
        assert!((behind.frac - 0.7).abs() < 1e-5, "{}", behind.frac);
        let ring = f.ring(60.0);
        assert_eq!(ring.len(), 2);
        assert!((ring[0].frac - 1.1).abs() < 1e-5 && ring[0].side == 1);
        assert!((ring[1].frac + 0.1).abs() < 1e-5 && ring[1].side == -1);
    }

    // A car from behind faces her along the road, 4 m to the side, at
    // 20 m/s on the level; on her own road the point is by the 360's rule
    // (60 over the 180 m left behind her: 0.9 - 1/3). A tank ahead comes
    // at 6 m/s toward her; none where the roads end short of its ring.
    #[test]
    fn cars_and_tanks_come_at_her_along_the_road() {
        let net = line();
        let f = Flood::new(&net, 1, 0.9, 1, REACH).unwrap();
        let a = car_behind(&net, &f).unwrap();
        let x = 100.0 + 200.0 * (0.9 - 60.0 / 180.0);
        assert!((a.at - Vec3::new(x, 0.0, 4.0)).length() < 1e-3, "{}", a.at);
        assert!((a.velocity - Vec3::new(20.0, 0.0, 0.0)).length() < 1e-3);
        assert_eq!(tank_ahead(&net, &f, false, 0.5), None);
        let f = Flood::new(&net, 1, 0.5, 1, REACH).unwrap();
        let t = tank_ahead(&net, &f, false, 0.5).unwrap();
        assert!(
            (t.at - Vec3::new(360.0, 0.2, 0.0)).length() < 1e-3,
            "{}",
            t.at
        );
        assert!((t.velocity - Vec3::new(-6.0, 0.0, 0.0)).length() < 1e-3);
    }

    #[test]
    fn a_bend_is_within_8_m() {
        let bent = NetRoad {
            points: vec![
                Vec3::ZERO,
                Vec3::new(50.0, 0.0, 0.0),
                Vec3::new(50.0, 0.0, 50.0),
            ],
            along: vec![0.0, 50.0, 100.0],
            length: 100.0,
            start: None,
            end: None,
        };
        assert!(bends(&bent, 0.5));
        assert!(!bends(&bent, 0.2));
    }
    // A road's junctions are those its record names; a junction's roads
    // those its own record names; only drivable roads count.
    #[test]
    fn the_network_joins_roads_by_their_records() {
        let road = |entity: u32, a: f32, b: f32, ends: [u32; 2]| Road {
            entity,
            layer: 0,
            points: vec![Vec3::new(a, 0.0, 0.0), Vec3::new(b, 0.0, 0.0)],
            width: 8.0,
            kind: 0,
            lanes: None,
            named_road: true,
            ends,
        };
        let junction = |entity: u32, roads: &[u32]| Junction {
            entity,
            position: Vec3::ZERO,
            ports: roads.iter().map(|&r| (r, Vec3::ZERO)).collect(),
        };
        let net = RoadNet::new(
            &[
                road(1, 0.0, 100.0, [10, 11]),
                road(2, 100.0, 250.0, [11, 0]),
            ],
            &[junction(10, &[1]), junction(11, &[1, 2])],
        );
        assert_eq!(net.roads.len(), 2);
        assert_eq!((net.roads[0].start, net.roads[0].end), (Some(0), Some(1)));
        assert_eq!((net.roads[1].start, net.roads[1].end), (Some(1), None));
        assert_eq!(net.junctions, vec![vec![0], vec![0, 1]]);
        assert_eq!(net.roads[1].length, 150.0);
        let (i, frac, off) = net.nearest(Vec3::new(175.0, 0.0, 5.0)).unwrap();
        assert_eq!(i, 1);
        assert!((frac - 0.5).abs() < 1e-5 && (off - 5.0).abs() < 1e-5);
        assert_eq!(net.nearest(Vec3::new(175.0, 0.0, 25.0)), None);
    }
}
