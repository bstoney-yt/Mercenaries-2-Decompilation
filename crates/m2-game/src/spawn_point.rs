//! `Pg.GetDistantSpawnPointOnPath(path, from, other, distance)`: a place
//! on a path a given distance from an object, to bring something in at
//! (OilCon001 spawns its attackers' vehicles there, out of her sight
//! down the road).
//!
//! From the 360 code (the native 82338A28, its walker 821B2C20 and the
//! stretch test 821B3548):
//!
//! - The path, the object and the distance are needed. It gives `true`,
//!   the place (x, y, z) and a heading in degrees when it finds one, and
//!   `false` alone when it doesn't.
//! - The walker takes the path's stretches in order, and the first one
//!   that crosses the circle of that radius round the object (over the
//!   ground: x and z) strictly between its ends gives the place, where it
//!   crosses, and the way the stretch runs.
//! - With no such stretch it takes the path's point furthest from the
//!   object, and that counts as found only when even that point is within
//!   the distance (the whole path is). The way is to the next point; for
//!   the path's last point the 360 takes the previous point's own
//!   coordinates for it, not the difference (kept as it is).
//! - The way is flattened and made a unit; the heading is the angle of
//!   its x against its z (82855048).
//!
//! Not ported: with a second object (the other player, in co-op) within
//! twice the distance of the first, the 360 uses another walker
//! (821B2FB0) that keeps clear of both. One player here: the scripts
//! pass 0.
//!
//! Inferred: the 360 walks the path in the path's own frame and turns the
//! answer back to the world; here the path's points are in the world
//! already. That the angle function (82853F88) is the two-argument
//! arctangent.

/// What the walker found: whether it counts, the place, and the way the
/// path runs there (flat, a unit, or zero).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpawnPoint {
    pub found: bool,
    pub at: [f32; 3],
    pub way: [f32; 3],
}

impl SpawnPoint {
    /// The heading of the way, degrees.
    pub fn heading(&self) -> f32 {
        self.way[0].atan2(self.way[2]).to_degrees()
    }
}

/// Where along the stretch `a` to `b` (0 at `a`, 1 at `b`) it crosses
/// the circle of squared radius `d2` round `from`, over the ground; -1 if
/// both ends are inside or it doesn't (821B3548).
fn crossing(d2: f32, a: [f32; 3], b: [f32; 3], from: [f32; 3]) -> f32 {
    let (ax, az) = (a[0] - from[0], a[2] - from[2]);
    let (bx, bz) = (b[0] - from[0], b[2] - from[2]);
    let a2 = az * az + ax * ax;
    if !(d2 < a2 || d2 < bz * bz + bx * bx) {
        return -1.0;
    }
    let (dx, dz) = (bx - ax, bz - az);
    let len2 = dz * dz + dx * dx;
    let half = (dz * az + dx * ax) * 2.0;
    let disc = half * half - (a2 - d2) * len2 * 4.0;
    if disc <= 0.0 {
        return -1.0;
    }
    let root = disc.sqrt();
    let (one, two) = ((root - half) / (len2 * 2.0), (-half - root) / (len2 * 2.0));
    let (lo, hi) = if two < one { (two, one) } else { (one, two) };
    if lo <= 1.0 && 0.0 <= hi {
        if lo <= 0.0 { hi } else { lo }
    } else {
        -1.0
    }
}

/// The walker (821B2C20): a place on `points` `distance` from `from`.
/// `None` for a path with no points.
pub fn distant(points: &[[f32; 3]], from: [f32; 3], distance: f32) -> Option<SpawnPoint> {
    let d2 = distance * distance;
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let mut out = None;
    for w in points.windows(2) {
        let t = crossing(d2, w[0], w[1], from);
        if 0.0 < t && t < 1.0 {
            let way = sub(w[1], w[0]);
            out = Some(SpawnPoint {
                found: true,
                at: [
                    w[0][0] + way[0] * t,
                    w[0][1] + way[1] * t,
                    w[0][2] + way[2] * t,
                ],
                way,
            });
            break;
        }
    }
    let mut point = match out {
        Some(p) => p,
        None => {
            let far2 = |p: &[f32; 3]| {
                let o = sub(*p, from);
                o[0] * o[0] + o[2] * o[2] + o[1] * o[1]
            };
            // The furthest point; of equals, the first.
            let (i, furthest) = points.iter().map(far2).enumerate().fold(
                None,
                |best: Option<(usize, f32)>, (i, d)| match best {
                    Some((_, b)) if d <= b => best,
                    _ => Some((i, d)),
                },
            )?;
            let way = if i + 1 == points.len() {
                // As the 360 has it: the previous point itself.
                if i == 0 { [0.0; 3] } else { points[i - 1] }
            } else {
                sub(points[i + 1], points[i])
            };
            SpawnPoint {
                found: furthest <= d2,
                at: points[i],
                way,
            }
        }
    };
    let (x, z) = (point.way[0], point.way[2]);
    let flat = (z * z + x * x).sqrt();
    point.way = if flat != 0.0 {
        [x / flat, 0.0, z / flat]
    } else {
        [0.0; 3]
    };
    Some(point)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A road east for 100 m, then north for 100 m.
    const ROAD: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [100.0, 0.0, 100.0]];

    #[test]
    fn the_first_stretch_to_leave_her_circle_gives_the_place() {
        // She stands at the road's start: 60 m down it.
        let p = distant(&ROAD, [0.0, 5.0, 0.0], 60.0).unwrap();
        assert!(p.found);
        assert!((p.at[0] - 60.0).abs() < 1e-3 && p.at[2].abs() < 1e-3);
        assert_eq!(p.way, [1.0, 0.0, 0.0]);
        assert!((p.heading() - 90.0).abs() < 1e-3);
        // From the corner, 30 m: back along the first stretch, the first
        // in the path's order to cross, still heading the way it runs.
        let p = distant(&ROAD, [100.0, 0.0, 0.0], 30.0).unwrap();
        assert!(p.found);
        assert!((p.at[0] - 70.0).abs() < 1e-3);
        assert_eq!(p.way, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn height_does_not_count_for_the_circle() {
        let p = distant(&ROAD, [0.0, 500.0, 0.0], 60.0).unwrap();
        assert!(p.found && (p.at[0] - 60.0).abs() < 1e-3);
    }

    #[test]
    fn a_path_she_is_far_from_gives_its_furthest_point_and_no() {
        // 1 km off: no stretch crosses a 60 m circle; the furthest point
        // is the road's end, which is not within 60 m.
        let p = distant(&ROAD, [-1000.0, 0.0, 0.0], 60.0).unwrap();
        assert!(!p.found);
        assert_eq!(p.at, ROAD[2]);
    }

    #[test]
    fn a_path_all_within_the_distance_gives_its_furthest_point_and_yes() {
        let p = distant(&ROAD, [0.0, 0.0, 0.0], 500.0).unwrap();
        assert!(p.found);
        assert_eq!(p.at, ROAD[2]);
        // The last point's way is the previous point's own coordinates,
        // as the 360 has it: (100, 0, 0), flattened to a unit.
        assert_eq!(p.way, [1.0, 0.0, 0.0]);
        // A middle point's way is to the next.
        let short = [ROAD[0], ROAD[2], ROAD[1]];
        let p = distant(&short, [0.0, 0.0, 0.0], 500.0).unwrap();
        assert_eq!(p.at, ROAD[2]);
        assert_eq!(p.way, [0.0, 0.0, -1.0]);
    }

    #[test]
    fn no_points_no_place() {
        assert_eq!(distant(&[], [0.0; 3], 10.0), None);
    }
}
