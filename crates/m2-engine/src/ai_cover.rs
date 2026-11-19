//! Where a person takes cover: the 360's cover service, as the research
//! thread traced it (360 notes s22, s25) and as read here.
//!
//! `HumanTakeCover` (enter 0x822B3970) asks the cover service (worker tick
//! 0x822CC020) for a spot: it gathers the cover hints of the objects
//! within the search radius of the person (0x822CBA38), and for each hint
//! works out candidate spots by its kind (`CoverHintOffset` `Type`:
//! Offset 0x822CC908, Point 0x822CCBD0, Box 0x822CD0E0). A spot is scored
//! `R^2 - clamp(d^2 / R^2, 0, 1) x W[Rating]`, snapped to the ground,
//! dropped within 2.2 m of a spot to avoid, and taken if its rays to the
//! threat come out blocked and clear as its kind expects; the best score
//! wins. The tables here (corners, lean normals, the Popup faces, `W`)
//! are the program's (0x820BB148, 0x82DE3018, 0x820BB124/1B4/190,
//! 0x820BB1D8).

use m2_math::glam::{Quat, Vec3};

/// A hint's kind (`CoverHintOffset` `Type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A spot at `CoverPosition`, facing `FacingYawOrRadius` within
    /// `CoverRange`: hidden there, firing standing up.
    Offset,
    /// A round obstacle of radius `FacingYawOrRadius` at `CoverPosition`.
    Point,
    /// The owning object's own bounding box.
    Box,
}

/// One cover hint of a template (a `CoverHintOffset` entry).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hint {
    pub kind: Kind,
    /// `Rating`: None 0, Poor 1, Okay 2, Good 3, Great 4.
    pub rating: u8,
    /// `CoverAnimation`: None 0, Dive 1, SideStep 2, Popup 3 (low cover).
    pub animation: u8,
    /// `CoverPosition`, object space.
    pub position: Vec3,
    /// `FacingYawOrRadius`: degrees for an Offset, metres for a Point.
    pub facing_or_radius: f32,
    /// `CoverRange`: an Offset's arc, degrees.
    pub range: f32,
    /// `RestrictRadius`: gathered only within this of the person, metres
    /// (not for a Box).
    pub restrict: f32,
    /// `NoCoverX`, `NoCoverZ`: a Popup Box's faces on that axis are out.
    pub no_x: bool,
    pub no_z: bool,
}

/// `CoverAnimation` Popup: low cover.
pub const POPUP: u8 = 3;

/// How a spot's rating weighs (0x820BB1D8).
const W: [f32; 6] = [0.0, 0.1, 0.4, 0.7, 1.0, 0.5];

/// An object a hint is on: where it stands, how it is turned, and its
/// model's box (model space).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub position: Vec3,
    pub rotation: Quat,
    pub bounds: (Vec3, Vec3),
}

/// What is asked: around `centre` within `radius` (the anchor radius,
/// at most 20 m: 20 for a person not anchored), from `threat`, away from
/// `avoid` (live bad spots and squadmates, at most 15).
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub centre: Vec3,
    pub radius: f32,
    pub threat: Vec3,
    pub avoid: Vec<Vec3>,
}

/// The search radius when not anchored (0x822C43B8).
pub const SEARCH: f32 = 20.0;
/// After a cover attempt the next waits `RETRY + RETRY_SPREAD x
/// random(0..1)` seconds (0x822C7348 sets the move block's cover timer
/// +0xB4 so; 0x822C8530 counts it down).
pub const RETRY: f32 = 15.0;
pub const RETRY_SPREAD: f32 = 10.0;
/// Cover is asked for only with the target this far off, metres
/// (0x822C6B78: between 0x820BAEEC and 0x820BAFA8).
pub const ASK_NEAR: f32 = 10.0;
pub const ASK_FAR: f32 = 40.0;

/// Whether a person may ask for cover now (the gate 0x822C6B78): his
/// cover timer run out and the target between [`ASK_NEAR`] and
/// [`ASK_FAR`]. Inferred or left out of the gate: the cover-capable
/// entry (+0x4EC), the global pause, the target's validity, the state
/// and flag checks (+0x128, +0xCC bit 0x80, +0x8C) and 0x822C04A0, all
/// taken to pass.
pub fn may_ask(timer: f32, distance: f32) -> bool {
    timer < 0.0 && (ASK_NEAR..=ASK_FAR).contains(&distance)
}
/// How near a spot to avoid rules a candidate out, metres.
const AVOID: f32 = 2.2;
/// How far a Point spot stands off its obstacle, and how far a lean
/// reaches, metres.
const OFF: f32 = 0.5;
const LEAN: f32 = 1.5;
/// Eye heights over the ground at a spot: standing cover and low
/// (Popup), and how much higher the second ray looks.
const EYE: f32 = 1.3;
const EYE_LOW: f32 = 0.4;
const UP: f32 = 0.9;

/// The two corners of a Box, by the threat's 3x3 region (0x820BB148).
const CORNERS: [[usize; 2]; 9] = [
    [2, 6],
    [0, 2],
    [0, 8],
    [0, 6],
    [9, 9],
    [2, 8],
    [0, 8],
    [6, 8],
    [2, 6],
];
/// The lean normals of those corners (0x82DE3018).
const NORMALS: [[(f32, f32); 2]; 9] = [
    [(-1.0, 0.0), (0.0, -1.0)],
    [(-1.0, 0.0), (-1.0, 0.0)],
    [(-1.0, 0.0), (0.0, 1.0)],
    [(0.0, -1.0), (0.0, -1.0)],
    [(0.0, 0.0), (0.0, 0.0)],
    [(0.0, 1.0), (0.0, 1.0)],
    [(0.0, -1.0), (1.0, 0.0)],
    [(1.0, 0.0), (1.0, 0.0)],
    [(0.0, 1.0), (1.0, 0.0)],
];
/// A Popup Box's face, by the threat's region: both axes allowed, X only,
/// Z only (0x820BB124, 0x820BB1B4, 0x820BB190).
const FACES: [[usize; 3]; 9] = [
    [8, 5, 7],
    [7, 7, 7],
    [6, 3, 7],
    [5, 5, 5],
    [9, 9, 9],
    [3, 3, 3],
    [2, 5, 1],
    [1, 1, 1],
    [0, 3, 1],
];

/// A candidate spot: where, its score, and the rays to test with which
/// must come out blocked (bit i for ray i) for it to be taken.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub spot: Vec3,
    pub score: f32,
    /// The point leaned out to (Point and Box corners), level with the
    /// spot.
    pub lean: Option<Vec3>,
    pub low: bool,
    pub expected: u8,
}

fn flat(v: Vec3) -> Vec3 {
    v.with_y(0.0)
}

/// The 3x3 region of box-local `p` around a box of half-sizes `hx`, `hz`
/// (0x822CC670): `ix * 3 + iz`, 4 inside.
fn region(p: Vec3, hx: f32, hz: f32) -> usize {
    let band = |v: f32, h: f32| {
        if v < -h {
            0
        } else if v > h {
            2
        } else {
            1
        }
    };
    band(p.x, hx) * 3 + band(p.z, hz)
}

/// Box-local direction of grid index `i`: (ix - 1, 0, iz - 1).
fn grid(i: usize) -> Vec3 {
    Vec3::new((i / 3) as f32 - 1.0, 0.0, (i % 3) as f32 - 1.0)
}

/// The spots `hint` on `placed` gives for `req`, scored (before the
/// ground snap and rays).
pub fn candidates(req: &Request, placed: &Placed, hint: &Hint) -> Vec<Candidate> {
    let r2 = req.radius * req.radius;
    let low = hint.animation == POPUP;
    let score = |spot: Vec3| {
        let d2 = flat(spot - req.centre).length_squared();
        (d2 <= r2)
            .then(|| r2 - (d2 / r2).clamp(0.0, 1.0) * W[usize::from(hint.rating).min(W.len() - 1)])
    };
    let mut out = Vec::new();
    let mut push = |spot: Vec3, lean: Option<Vec3>, expected: u8| {
        if let Some(s) = score(spot) {
            out.push(Candidate {
                spot,
                score: s,
                lean,
                low,
                expected,
            });
        }
    };
    match hint.kind {
        Kind::Offset => {
            let spot = placed.position + placed.rotation * hint.position;
            // The gather skips a hint whose own range the requester is
            // beyond, in 3D (0x822CBA38: squared distance over the hint's
            // +0x30 squared).
            if (spot - req.centre).length_squared() > hint.restrict * hint.restrict {
                return out;
            }
            // The threat within the arc about its facing.
            let (sy, cy) = hint.facing_or_radius.to_radians().sin_cos();
            let facing = flat(placed.rotation * Vec3::new(sy, 0.0, cy)).normalize_or_zero();
            let to = flat(req.threat - spot).normalize_or_zero();
            let half = (hint.range * 0.5).to_radians();
            // Both rays, to the spot and 0.9 m over it, must come out
            // blocked (0x822CC908).
            if facing.dot(to) >= half.cos() {
                push(spot, None, 0b011);
            }
        }
        Kind::Point => {
            let centre = placed.position + placed.rotation * hint.position;
            if (centre - req.centre).length_squared() > hint.restrict * hint.restrict {
                return out;
            }
            let to = flat(req.threat - centre).normalize_or_zero();
            if to == Vec3::ZERO {
                return out;
            }
            let spot = centre - to * (hint.facing_or_radius + OFF);
            let side = Vec3::new(to.z, 0.0, -to.x);
            for s in [side, -side] {
                push(spot, Some(spot + s * LEAN), 0b100);
            }
        }
        Kind::Box => {
            let (lo, hi) = placed.bounds;
            let size = hi - lo;
            let (hx, hz) = ((size.x + 1.0) * 0.5, (size.z + 1.0) * 0.5);
            let middle = (lo + hi) * 0.5;
            let inv = placed.rotation.inverse();
            let local = |w: Vec3| inv * (w - placed.position) - middle;
            let world = |l: Vec3| placed.position + placed.rotation * (l + middle);
            let r = region(local(req.threat), hx, hz);
            if r == 4 || region(local(req.centre), hx, hz) == 4 {
                return out;
            }
            let half = Vec3::new(hx, 0.0, hz);
            if low {
                let x_ok = !hint.no_x && size.x > 0.5;
                let z_ok = !hint.no_z && size.z > 0.5;
                let face = match (x_ok, z_ok) {
                    (true, true) => FACES[r][0],
                    (true, false) => FACES[r][1],
                    (false, true) => FACES[r][2],
                    (false, false) => return out,
                };
                push(world(grid(face) * half), None, 0b001);
            } else {
                for k in 0..2 {
                    let (nx, nz) = NORMALS[r][k];
                    let n = Vec3::new(nx, 0.0, nz);
                    let spot = grid(CORNERS[r][k]) * half - n * LEAN;
                    push(world(spot), Some(world(spot + n * LEAN)), 0b100);
                }
            }
        }
    }
    out
}

/// The best spot among `cands` for `req`: the highest score whose spot
/// `ground` snaps (the height of the ground at a point, looked for from
/// 0.5 m over it to 1.5 m under, 0x822CC730), at least 2.2 m from every
/// spot to avoid, and whose rays `blocked` (from, to) come out as its kind
/// expects (0x822CC450). Rays, as the program casts them: an Offset or a
/// Popup Box, the threat to the spot and to 0.9 m over it (hidden, then
/// clear: expected 1); a Point or a Box corner, the lean point to the spot,
/// the threat to the lean point and the threat to the spot (only the last
/// blocked: expected 4). Returns where to stand (on the ground).
pub fn choose(
    req: &Request,
    cands: &[Candidate],
    ground: impl Fn(Vec3) -> Option<f32>,
    blocked: impl Fn(Vec3, Vec3) -> bool,
) -> Option<Vec3> {
    choose_index(req, cands, ground, blocked).map(|(at, _)| at)
}

/// As [`choose`], with the chosen candidate's place in `cands`.
pub fn choose_index(
    req: &Request,
    cands: &[Candidate],
    ground: impl Fn(Vec3) -> Option<f32>,
    blocked: impl Fn(Vec3, Vec3) -> bool,
) -> Option<(Vec3, usize)> {
    let mut order: Vec<(usize, &Candidate)> = cands.iter().enumerate().collect();
    order.sort_by(|a, b| b.1.score.total_cmp(&a.1.score));
    for (k, c) in order {
        let Some(g) = ground(c.spot) else { continue };
        let foot = c.spot.with_y(g);
        if req
            .avoid
            .iter()
            .any(|a| a.distance_squared(foot) < AVOID * AVOID)
        {
            continue;
        }
        let eye = foot + Vec3::Y * if c.low { EYE_LOW } else { EYE };
        let rays: Vec<(Vec3, Vec3)> = match c.lean {
            None => vec![(req.threat, eye), (req.threat, eye + Vec3::Y * UP)],
            Some(l) => {
                let lean = l.with_y(eye.y);
                vec![(lean, eye), (req.threat, lean), (req.threat, eye)]
            }
        };
        let mask = rays
            .iter()
            .enumerate()
            .fold(0u8, |m, (i, &(a, b))| m | (u8::from(blocked(a, b)) << i));
        if mask == c.expected {
            return Some((foot, k));
        }
    }
    None
}

/// One soldier's claim on a cover object's spot (the table at 0x83656DE8).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Claim {
    pub owner: u32,
    pub object: u32,
    pub score: f32,
    pub at: Vec3,
}

/// The cover claims (0x822EDA60): one per owner, his latest. 0x822C7CF0
/// claims as the person starts for the spot and claims again every update
/// while the destination is at most [`CLAIM_AGE`] old, each time scored
/// from where he is then; a refusal in that window drops the destination.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Claims {
    claims: Vec<Claim>,
}

/// How long after he starts for a cover spot his claim is checked and
/// renewed each update, seconds (0x822C7CF0: the goal age +0x98 at most
/// 2.0).
pub const CLAIM_AGE: f32 = 2.0;

/// A claim's score: 100 less the squared flat distance from the claimer to
/// the spot (0x822EDA60; a hint kind 13 scores 100, not mapped here).
pub fn claim_score(from: Vec3, at: Vec3) -> f32 {
    100.0 - flat(at - from).length_squared()
}

impl Claims {
    /// `owner`, at `from`, claims the spot `at` of cover object `object`:
    /// refused when another owner's claim on the same object lies within
    /// 1 m on x/z and this one's score is under 1.1 times that one's;
    /// otherwise it replaces the owner's earlier claim. Whether it was
    /// granted.
    pub fn claim(&mut self, owner: u32, object: u32, from: Vec3, at: Vec3) -> bool {
        let score = claim_score(from, at);
        let refused = self.claims.iter().any(|c| {
            c.owner != owner
                && c.object == object
                && flat(c.at - at).length_squared() < 1.0
                && score < 1.1 * c.score
        });
        if refused {
            return false;
        }
        self.claims.retain(|c| c.owner != owner);
        self.claims.push(Claim {
            owner,
            object,
            score,
            at,
        });
        true
    }

    /// The claims held.
    pub fn held(&self) -> &[Claim] {
        &self.claims
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall() -> Placed {
        // A 4 x 2 x 0.5 m wall across +z at the origin.
        Placed {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            bounds: (Vec3::new(-2.0, 0.0, -0.25), Vec3::new(2.0, 2.0, 0.25)),
        }
    }

    fn box_hint(animation: u8) -> Hint {
        Hint {
            kind: Kind::Box,
            rating: 3,
            animation,
            position: Vec3::ZERO,
            facing_or_radius: 0.0,
            range: 120.0,
            restrict: 50.0,
            no_x: false,
            no_z: false,
        }
    }

    #[test]
    fn cover_is_asked_for_between_10_and_40_m_once_the_timer_is_out() {
        assert!(may_ask(-0.1, 25.0));
        assert!(!may_ask(0.5, 25.0), "timer running");
        assert!(!may_ask(-0.1, 9.0) && !may_ask(-0.1, 41.0));
    }

    /// 0x822EDA60: a claim near another's on the same object is refused
    /// unless it scores 1.1 times as well; elsewhere, or on another
    /// object, it is granted; a new claim replaces its owner's last.
    #[test]
    fn a_cover_claim_yields_to_a_nearer_soldiers() {
        let mut c = Claims::default();
        let spot = Vec3::new(0.0, 0.0, 0.0);
        // Soldier 1, 3 m off: score 91.
        assert!(c.claim(1, 7, Vec3::new(3.0, 0.0, 0.0), spot));
        // Soldier 2, 4 m off (84 < 1.1 x 91): refused, 0.5 m off it too.
        assert!(!c.claim(2, 7, Vec3::new(4.0, 0.0, 0.0), spot));
        assert!(!c.claim(2, 7, Vec3::new(4.0, 0.0, 0.0), Vec3::new(0.5, 3.0, 0.0)));
        // 1.5 m from the first, or another object's: granted.
        assert!(c.claim(2, 7, Vec3::new(4.0, 0.0, 0.0), Vec3::new(1.5, 0.0, 0.0)));
        assert!(c.claim(3, 8, Vec3::new(9.0, 0.0, 0.0), spot));
        // Right on top of it (score 100 >= 1.1 x 91? no: 100 < 100.1): refused.
        assert!(!c.claim(4, 7, spot, spot));
        // Soldier 1 claims elsewhere: his first claim goes.
        assert!(c.claim(1, 9, Vec3::ZERO, Vec3::new(50.0, 0.0, 0.0)));
        assert!(c.claim(4, 7, spot, spot));
        assert_eq!(c.held().iter().filter(|h| h.owner == 1).count(), 1);
    }

    /// An Offset spot is taken only when both rays from the threat, to
    /// the spot and 0.9 m over it, come out blocked (0x822CC908).
    #[test]
    fn an_offset_spot_wants_both_rays_blocked() {
        let req = Request {
            centre: Vec3::new(0.0, 0.0, -2.0),
            radius: SEARCH,
            threat: Vec3::new(0.0, 1.5, 30.0),
            avoid: Vec::new(),
        };
        let hint = Hint {
            kind: Kind::Offset,
            rating: 3,
            animation: 0,
            position: Vec3::new(0.0, 0.0, -1.0),
            facing_or_radius: 0.0,
            range: 90.0,
            restrict: 10.0,
            no_x: false,
            no_z: false,
        };
        let c = candidates(&req, &wall(), &hint);
        assert_eq!(c.len(), 1);
        let ground = |_: Vec3| Some(0.0);
        // A wall 2 m high: both rays blocked, taken.
        let high = |a: Vec3, b: Vec3| {
            let t = a.z / (a.z - b.z);
            (a.z > 0.0) != (b.z > 0.0) && a.lerp(b, t).y < 3.0
        };
        assert!(choose(&req, &c, ground, high).is_some());
        // A low one: the upper ray clear, not taken.
        let low = |a: Vec3, b: Vec3| {
            let t = a.z / (a.z - b.z);
            (a.z > 0.0) != (b.z > 0.0) && a.lerp(b, t).y < 1.6
        };
        assert!(choose(&req, &c, ground, low).is_none());
        // Beyond the hint's own range in 3D (10 m up): not offered.
        let far = Request {
            centre: Vec3::new(0.0, 12.0, -2.0),
            ..req.clone()
        };
        assert!(candidates(&far, &wall(), &hint).is_empty());
    }

    #[test]
    fn a_box_gives_its_corners_on_the_threats_side() {
        // The threat straight out in +z: region 5 (inside x, beyond z).
        let req = Request {
            centre: Vec3::new(0.0, 0.0, -5.0),
            radius: SEARCH,
            threat: Vec3::new(0.0, 1.5, 30.0),
            avoid: Vec::new(),
        };
        let c = candidates(&req, &wall(), &box_hint(2));
        assert_eq!(c.len(), 2);
        // Corners 2 and 8 (the +z corners), stepped back 1.5 m along -z.
        let hz = (0.5 + 1.0) * 0.5;
        for k in &c {
            assert!((k.spot.x.abs() - 2.5).abs() < 1e-5, "{:?}", k.spot);
            assert!((k.spot.z - (hz - 1.5)).abs() < 1e-5, "{:?}", k.spot);
            assert!((k.lean.unwrap().z - hz).abs() < 1e-5);
            assert_eq!(k.expected, 0b100);
        }
        // Low cover: one face, the far side.
        let c = candidates(&req, &wall(), &box_hint(POPUP));
        assert_eq!(c.len(), 1);
        assert!((c[0].spot.z + hz).abs() < 1e-5, "{:?}", c[0].spot);
    }

    #[test]
    fn the_spot_is_taken_only_where_the_rays_say_so() {
        let req = Request {
            centre: Vec3::new(0.0, 0.0, -5.0),
            radius: SEARCH,
            threat: Vec3::new(0.0, 1.5, 30.0),
            avoid: Vec::new(),
        };
        let c = candidates(&req, &wall(), &box_hint(POPUP));
        // A low wall: it blocks a ray crossing z = 0 under 1 m high.
        let blocked = |a: Vec3, b: Vec3| {
            if (a.z > 0.0) == (b.z > 0.0) {
                return false;
            }
            let t = a.z / (a.z - b.z);
            a.lerp(b, t).y < 1.0
        };
        let ground = |_: Vec3| Some(0.0);
        let at = choose(&req, &c, ground, blocked).unwrap();
        assert!(at.z < 0.0 && at.y == 0.0, "{at}");
        // A spot to avoid right there rules it out.
        let avoided = Request {
            avoid: vec![at],
            ..req.clone()
        };
        assert!(choose(&avoided, &c, ground, blocked).is_none());
    }

    #[test]
    fn an_offset_needs_the_threat_inside_its_arc() {
        let hint = Hint {
            kind: Kind::Offset,
            position: Vec3::new(1.0, 0.0, 0.0),
            facing_or_radius: 0.0,
            range: 90.0,
            ..box_hint(0)
        };
        let req = |threat: Vec3| Request {
            centre: Vec3::ZERO,
            radius: SEARCH,
            threat,
            avoid: Vec::new(),
        };
        assert_eq!(
            candidates(&req(Vec3::new(1.0, 0.0, 30.0)), &wall(), &hint).len(),
            1
        );
        assert!(candidates(&req(Vec3::new(30.0, 0.0, 0.0)), &wall(), &hint).is_empty());
    }
}
