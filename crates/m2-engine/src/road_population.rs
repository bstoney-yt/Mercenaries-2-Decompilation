//! How many road vehicles are out round the player, where new ones come
//! out and when they are taken away, as the 360's population upkeep does
//! it (0x822639A0 each frame).
//!
//! From the 360 code:
//! - **How many** (0x82233B88, 0x82266090): the density zone she is in of
//!   highest priority gives the vehicles wanted (its +0x24) and the people
//!   (+0x28); 2 and 2 outside every zone ([`DEFAULT_WANTED`]). In the
//!   dense mode (below) with her in a vehicle, twice the vehicles.
//! - **The mode** (0x82266090): sparse ([`Mode::Sparse`]) when she rides
//!   a boat or flies (her state 7 or 8), is 10 m or more above the ground,
//!   or the zone wants fewer than 10 vehicles and 5 people; else dense.
//! - **Where they come out** (0x8226B120 dense, 0x8226B5A0 sparse): on
//!   timers, rings of road points a set distance from her along the road
//!   network (0x8226A438 on 0x8223F3F8), ahead of her or behind, the
//!   vehicles heading away from her or toward her, each ring with what
//!   may be seen there ([`Ask`]); at most 10 a frame (0x8226ADB0). A
//!   point must be out at least [`Ask::least`] and no more than 400 m
//!   (0x8226BF00), seen or not as its kind says, and pass the vehicles'
//!   keep test ([`spawn_keeps`], 0x82263318 with kind 1, from 0x8226C618).
//! - **When they go** (0x82263590 each frame, a rotating 20 and the
//!   first ones, at most 2 taken a call; 0x82263318): a vehicle driving
//!   the roads is in the population registry (0x822ABFC0 puts it there,
//!   0x8226BA28), so kind 3: kept within 200 m of her, taken beyond 250,
//!   between taken only when no view holds it ([`despawns`]). The distance
//!   is the nearer of her camera's place and a point 15 m on along its
//!   view in a vehicle, 5 m on foot (0x82264DF0); a view holds a point
//!   when a 2 m ball round it is in its frustum (0x822080C8, 0x82568550).
//! - Nothing takes a vehicle away for being stuck.

use m2_math::glam::{Mat4, Vec3, Vec4};

/// Vehicles and people wanted outside every zone (0x82233B88's 2).
pub const DEFAULT_WANTED: u32 = 2;
/// The most vehicles that come out in a frame (0x82266090's 10).
pub const MOST_A_FRAME: usize = 10;
/// The farthest a new one comes out, metres (0x8226BF00's 160000 squared).
pub const FARTHEST: f32 = 400.0;
/// The ball a view must hold to see a point, metres (0x82263318's 2).
pub const SEEN_RADIUS: f32 = 2.0;
/// How far along the camera's view the second point is, metres, in a
/// vehicle and on foot (0x82264DF0: 15 and 5).
pub const AHEAD_DRIVING: f32 = 15.0;
pub const AHEAD_WALKING: f32 = 5.0;
/// A registered road vehicle is kept within this, metres, and taken
/// beyond the far one (0x82263318's kind 3: 40000 and 62500 squared).
pub const KEEP_NEAR: f32 = 200.0;
pub const KEEP_FAR: f32 = 250.0;
/// Of the road population, how many are looked at a frame past the first
/// ones, and the most taken a look (0x822639A0, 0x82263590).
pub const LOOK_AT: usize = 20;
pub const MOST_TAKEN: usize = 2;

/// Which upkeep places new vehicles (0x82266090's 0x831341BC).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// 0x8226B120: many rings, often.
    Dense,
    /// 0x8226B5A0: by her speed, seldom.
    Sparse,
}

impl Mode {
    /// The mode for a zone wanting `vehicles` and `people`, her `height`
    /// above the ground (metres) and whether she rides a boat or flies.
    pub fn of(vehicles: u32, people: u32, height: f32, afloat_or_flying: bool) -> Self {
        if afloat_or_flying || (vehicles < 10 && people < 5) || height >= 10.0 {
            Self::Sparse
        } else {
            Self::Dense
        }
    }
}

/// The vehicles wanted out: the zone's (or [`DEFAULT_WANTED`]), twice that
/// in the dense mode while she is in a vehicle (0x82266090).
pub fn wanted(zone_vehicles: Option<u32>, mode: Mode, in_vehicle: bool) -> usize {
    let n = zone_vehicles.unwrap_or(DEFAULT_WANTED) as usize;
    if mode == Mode::Dense && in_vehicle {
        n * 2
    } else {
        n
    }
}

/// What may be seen of where one comes out (0x8226A438's flags into
/// 0x8226BF00).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    /// Flag 1: anywhere.
    Any,
    /// No flag: out of every view.
    Unseen,
    /// Flag 8: in a view (where it is far enough to come out in sight).
    InView,
    /// Flag 0x10: out of every view (and the 360's density grid, not
    /// ported).
    UnseenSparse,
}

impl Seen {
    fn of(flags: u32) -> Self {
        if flags & 0x10 != 0 {
            Self::UnseenSparse
        } else if flags & 8 != 0 {
            Self::InView
        } else if flags & 1 != 0 {
            Self::Any
        } else {
            Self::Unseen
        }
    }

    /// Whether a point a view holds (`seen`) may take one.
    pub fn allows(self, seen: bool) -> bool {
        match self {
            Self::Any => true,
            Self::Unseen | Self::UnseenSparse => !seen,
            Self::InView => seen,
        }
    }
}

/// A ring to put vehicles out on (one 0x8226A438 call).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ask {
    /// Metres from her along the roads.
    pub radius: f32,
    /// The most to put out on it.
    pub count: usize,
    /// Behind her (flag 4), else ahead.
    pub behind: bool,
    /// Heading toward her (flag 2), else away.
    pub toward: bool,
    pub seen: Seen,
    /// The least a point may be from her, metres (0x8226A438: 45 with
    /// flag 0x10, or flag 4 alone; else 80).
    pub least: f32,
}

impl Ask {
    fn new(radius: f32, count: usize, flags: u32) -> Self {
        let behind = flags & 4 != 0;
        let least = if flags & 0x10 != 0 || (behind && flags & 9 == 0) {
            45.0
        } else {
            80.0
        };
        Self {
            radius,
            count,
            behind,
            toward: flags & 2 != 0,
            seen: Seen::of(flags),
            least,
        }
    }
}

/// The upkeep's timers (0x8380ED34.. dense, 0x8380ED64.. sparse).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spawner {
    dense: [f32; 4],
    sparse: [f32; 3],
}

/// Puts out the rings due, in order, while fewer than `room` are out
/// this frame: `put` places up to the ask's count and says how many it
/// placed. Each ring's timer starts again as the 360's does: some always
/// once due, some only when the ring placed one.
struct Out<'a> {
    room: usize,
    placed: usize,
    put: &'a mut dyn FnMut(Ask) -> usize,
}

impl Out<'_> {
    fn left(&self) -> usize {
        self.room.saturating_sub(self.placed)
    }

    /// Asks for up to `most` (cut to what is left) on a ring; how many came out.
    fn ring(&mut self, radius: f32, most: usize, flags: u32) -> usize {
        let ask = Ask::new(radius, most.min(self.left()), flags);
        let n = (self.put)(ask);
        self.placed += n;
        n
    }
}

impl Spawner {
    /// Runs the timers `dt` seconds on and puts out the rings due, in the
    /// mode, with her on foot or not and at `speed` m/s; `coin` is a
    /// random number 0..1 and `room` how many more may come out. Gives
    /// how many came out. The rings, timers and their order are
    /// 0x8226B120's (dense) and 0x8226B5A0's (sparse), each ring's flags
    /// read off its call.
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        dt: f32,
        mode: Mode,
        on_foot: bool,
        speed: f32,
        coin: f32,
        room: usize,
        put: &mut dyn FnMut(Ask) -> usize,
    ) -> usize {
        let mut out = Out {
            room,
            placed: 0,
            put,
        };
        match mode {
            Mode::Dense => {
                for t in &mut self.dense {
                    *t += dt;
                }
                let [t1, t2, t3, t4] = self.dense;
                let due = [t1 > 0.1, t2 > 0.5, t3 > 0.79, t4 > 2.11125];
                if on_foot {
                    if due[3] && out.left() > 0 && out.ring(50.0, 1, 6) > 0 {
                        self.dense[3] = 0.0;
                    }
                    if due[0] {
                        if out.left() > 0 {
                            out.ring(110.0, 2, 2);
                        }
                        self.dense[0] = 0.0;
                    }
                    if due[1] && out.left() > 0 {
                        out.ring(110.0, 2, 0xa);
                        self.dense[1] = 0.0;
                    }
                    if due[2] && out.left() > 0 {
                        out.ring(120.0, 2, 2);
                        self.dense[2] = 0.0;
                    }
                } else {
                    if due[0] {
                        if out.left() > 0 {
                            out.ring(110.0, 2, 0);
                            if out.left() > 0 {
                                out.ring(110.0, 2, 2);
                            }
                        }
                        self.dense[0] = 0.0;
                    }
                    if due[1] && out.left() > 0 {
                        out.ring(110.0, 2, 0xa);
                        self.dense[1] = 0.0;
                    }
                    if due[2] && out.left() > 0 {
                        out.ring(115.0, 2, 8);
                        self.dense[2] = 0.0;
                    }
                    if due[3] && out.left() > 0 {
                        out.ring(110.0, 1, 0x16);
                        self.dense[3] = 0.0;
                    }
                }
            }
            Mode::Sparse => {
                for t in &mut self.sparse {
                    *t += dt;
                }
                let [t1, t2, t3] = self.sparse;
                let (due1, due2, due3) = (t1 > 1.95, t2 > 1.8475, t3 > 1.0);
                let heads = due1 && coin > 0.5;
                if speed < 6.0 {
                    if due2
                        && out.left() > 0
                        && (out.ring(60.0, 1, 6) > 0 || out.ring(70.0, 1, 6) > 0)
                    {
                        self.sparse[1] = 0.0;
                    }
                    if due1 && out.left() > 0 && out.ring(100.0, 2, 2) > 0 {
                        self.sparse[0] = 0.0;
                    }
                    // The 360 starts the first timer again here, not the third.
                    if due3 && out.left() > 0 && out.ring(110.0, 2, 7) > 0 {
                        self.sparse[0] = 0.0;
                    }
                    if heads && out.left() > 0 && out.ring(120.0, 2, 3) > 0 {
                        self.sparse[2] = 0.0;
                    }
                } else if speed <= 15.0 {
                    if due1 && out.left() > 0 {
                        out.ring(120.0, 1, 0);
                        self.sparse[0] = 0.0;
                    }
                    if due2 && out.left() > 0 {
                        out.ring(130.0, 1, 2);
                        self.sparse[1] = 0.0;
                    }
                } else {
                    if due1 && out.left() > 0 && out.ring(138.0, 1, 8) > 0 {
                        self.sparse[0] = 0.0;
                    }
                    if heads && out.left() > 0 {
                        out.ring(120.0, 2, 8);
                    }
                    if due2 && out.left() > 0 && out.ring(140.0, 1, 0xa) > 0 {
                        self.sparse[1] = 0.0;
                    }
                    // Nothing starts the third timer again at speed.
                    if due3 && out.left() > 0 {
                        out.ring(130.0, 1, 2);
                    }
                }
            }
        }
        out.placed
    }
}

/// Whether a new vehicle may come out `d` metres from her (the nearer of
/// her two points), `seen` saying whether a view holds it: 0x82263318's
/// kind 1, which 0x8226C618 asks of vehicles: within 80 m (dense) or 140
/// (sparse), or out to 150 or 160 m where a view holds it.
pub fn spawn_keeps(d: f32, seen: bool, mode: Mode) -> bool {
    let (near, far) = match mode {
        Mode::Dense => (80.0, 150.0),
        Mode::Sparse => (140.0, 160.0),
    };
    d < near || (d < far && seen)
}

/// Whether a road vehicle `d` metres from her goes (0x82263318's kind 3).
pub fn despawns(d: f32, seen: bool) -> bool {
    if d < KEEP_NEAR {
        false
    } else if d < KEEP_FAR {
        !seen
    } else {
        true
    }
}

/// Her camera, as the upkeep measures from it: where it is, which way it
/// looks and its view-projection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewer {
    pub at: Vec3,
    pub look: Vec3,
    pub view_proj: Mat4,
}

impl Viewer {
    /// How far `p` is from her, flat: the nearer of the camera and the
    /// point [`AHEAD_DRIVING`] (in a vehicle) or [`AHEAD_WALKING`] on
    /// along its view (0x82264DF0).
    pub fn distance(&self, p: Vec3, in_vehicle: bool) -> f32 {
        let ahead = if in_vehicle {
            AHEAD_DRIVING
        } else {
            AHEAD_WALKING
        };
        let flat = |v: Vec3| v.with_y(0.0).length();
        flat(p - self.at).min(flat(p - (self.at + self.look * ahead)))
    }

    /// Whether the view holds a [`SEEN_RADIUS`] ball round `p`.
    pub fn sees(&self, p: Vec3) -> bool {
        in_view(self.view_proj, p, SEEN_RADIUS)
    }
}

/// Whether a ball of `radius` round `at` is in the frustum of
/// `view_proj` (clip planes of its rows, right-handed, depth 0..1).
pub fn in_view(view_proj: Mat4, at: Vec3, radius: f32) -> bool {
    let m = view_proj.transpose();
    let (r0, r1, r2, r3) = (m.x_axis, m.y_axis, m.z_axis, m.w_axis);
    let planes: [Vec4; 6] = [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2];
    let p = at.extend(1.0);
    planes.iter().all(|pl| {
        let n = pl.truncate().length();
        n <= 0.0 || pl.dot(p) / n >= -radius
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mode_follows_the_zone_and_her() {
        assert_eq!(Mode::of(12, 8, 0.0, false), Mode::Dense);
        assert_eq!(Mode::of(6, 3, 0.0, false), Mode::Sparse);
        assert_eq!(Mode::of(12, 8, 12.0, false), Mode::Sparse);
        assert_eq!(Mode::of(12, 8, 0.0, true), Mode::Sparse);
        assert_eq!(wanted(Some(6), Mode::Dense, true), 12);
        assert_eq!(wanted(None, Mode::Sparse, true), 2);
    }

    // 0x82263318: a road car stays to 200 m, goes past 250, and between
    // goes only out of sight; a new one comes out within 80 m (dense) or
    // in sight out to 150.
    #[test]
    fn they_come_and_go_by_distance_and_sight() {
        assert!(!despawns(150.0, false));
        assert!(despawns(260.0, true));
        assert!(despawns(220.0, false));
        assert!(!despawns(220.0, true));
        assert!(spawn_keeps(60.0, false, Mode::Dense));
        assert!(!spawn_keeps(110.0, false, Mode::Dense));
        assert!(spawn_keeps(110.0, true, Mode::Dense));
        assert!(spawn_keeps(130.0, false, Mode::Sparse));
    }

    fn asks(s: &mut Spawner, dt: f32, mode: Mode, on_foot: bool, speed: f32) -> Vec<Ask> {
        let mut got = Vec::new();
        s.step(dt, mode, on_foot, speed, 0.0, MOST_A_FRAME, &mut |a| {
            got.push(a);
            a.count
        });
        got
    }

    // The dense upkeep on foot: the 110 m rings every 0.1 and 0.5 s, the
    // 120 m one past 0.79 s, the 50 m one behind past 2.11 s.
    #[test]
    fn the_dense_upkeep_asks_its_rings_on_its_timers() {
        let mut s = Spawner::default();
        assert_eq!(
            asks(&mut s, 0.2, Mode::Dense, true, 0.0),
            vec![Ask::new(110.0, 2, 2)]
        );
        let got = asks(&mut s, 0.6, Mode::Dense, true, 0.0);
        assert!(
            got.iter()
                .any(|a| a.seen == Seen::InView && a.radius == 110.0)
        );
        let got = asks(&mut s, 2.0, Mode::Dense, true, 0.0);
        let behind = got.iter().find(|a| a.radius == 50.0).unwrap();
        assert!(behind.behind && behind.toward && behind.least == 45.0);
    }

    // 0x8226B5A0 slow: the 60 m ring, else 70; its timer starts again only
    // when one came out, so an empty road asks again next frame.
    #[test]
    fn the_sparse_upkeep_tries_again_until_one_comes_out() {
        let mut s = Spawner::default();
        let mut tried = Vec::new();
        s.step(1.9, Mode::Sparse, true, 0.0, 0.0, 10, &mut |a| {
            tried.push(a.radius);
            0
        });
        assert_eq!(tried, vec![60.0, 70.0, 110.0]);
        tried.clear();
        s.step(0.01, Mode::Sparse, true, 0.0, 0.0, 10, &mut |a| {
            tried.push(a.radius);
            0
        });
        assert_eq!(tried, vec![60.0, 70.0, 110.0]);
        // No room, no asks.
        let n = s.step(
            0.01,
            Mode::Sparse,
            true,
            0.0,
            0.0,
            0,
            &mut |_| unreachable!(),
        );
        assert_eq!(n, 0);
    }

    // Each ring's count is cut to what may still come out this frame.
    #[test]
    fn the_rings_ask_no_more_than_there_is_room_for() {
        let mut s = Spawner::default();
        let mut counts = Vec::new();
        s.step(3.0, Mode::Dense, false, 0.0, 0.0, 3, &mut |a| {
            counts.push(a.count);
            a.count
        });
        assert_eq!(counts, vec![2, 1]);
    }

    // 0x82264DF0: the nearer of the camera and a point on along its view.
    #[test]
    fn she_is_measured_from_her_camera_and_ahead_of_it() {
        let v = Viewer {
            at: Vec3::ZERO,
            look: Vec3::NEG_Z,
            view_proj: Mat4::IDENTITY,
        };
        let p = Vec3::new(0.0, 30.0, -100.0);
        assert_eq!(v.distance(p, true), 85.0);
        assert_eq!(v.distance(p, false), 95.0);
        assert_eq!(v.distance(Vec3::new(0.0, 0.0, 10.0), true), 10.0);
    }

    #[test]
    fn a_view_holds_what_is_in_front_of_it() {
        let view = Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let proj = Mat4::perspective_rh(1.0, 1.5, 0.1, 1000.0);
        let vp = proj * view;
        assert!(in_view(vp, Vec3::new(0.0, 0.0, -50.0), SEEN_RADIUS));
        assert!(!in_view(vp, Vec3::new(0.0, 0.0, 50.0), SEEN_RADIUS));
        assert!(!in_view(vp, Vec3::new(200.0, 0.0, -50.0), SEEN_RADIUS));
    }
}
