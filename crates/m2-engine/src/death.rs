//! What a person killed on foot plays before going limp: the original's
//! pick of a `DamageDirection` from the hit that killed them.
//!
//! When a person's health runs out the original posts them one of two
//! messages. `Die` leads to an action with no clip: the body is a ragdoll
//! at once. `DieAnimatedRagdoll` leads to an action that plays a short
//! clip (0.37 to 0.40 s in the game's data), chosen by the direction the
//! killing hit came from, and the ragdoll takes over as it ends.
//! [`damage_direction`] is the choice between them: `None` is the plain
//! `Die`.
//!
//! # Provenance
//!
//! From the 360 code:
//!
//! - The damage handler (0x82289F40) calls 0x82287F98 for a person killed
//!   on foot. That posts `Die` (0x9E33AD21) when the picker gives no
//!   direction, else writes the direction into the person's animation
//!   query and posts `DieAnimatedRagdoll` (0xD87391B3).
//! - The picker (0x82421920) reads the animation driver's record of the
//!   last hit. With none, or one older than [`HIT_MEMORY`] (the float 0.25
//!   at 0x820DABDC), there is no direction. Otherwise the direction is the
//!   word of the table at 0x82B75FF8 ([`DIRECTIONS`]) at
//!   `(way * 3 + height) * 2 + side`:
//!   - **way** (0x82421840), from `f`, the hit's direction of travel
//!     along the person's facing: 0 under -0.707 (0x820F8CE8), 1 over
//!     0.707 (0x82DE8B6C), else 2, or 3 when the hit's direction along the
//!     facing turned a quarter turn (pi/2 added, 0x82DE4AFC) is 0 or more;
//!   - **height**: 2 when a flag of the record is set; else 1, unless
//!     both of two body nodes are found and the hit's height is at most
//!     the first's less 0.1 (0x820BB468) and under the second's less 0.2
//!     (0x820B8054), which is 0. The nodes are 0x705C4508 (of the
//!     original's upper-body bones) and 0xFAEFB386 (the first of its
//!     lower-body bones, the pelvis);
//!   - **side**: 1 when the hit's position, from the person's, along the
//!     facing turned a quarter turn is under 0.
//!
//! From samples: the stance and action table has one `Upright`
//! `DieAnimatedRagdoll` row for each of the table's 14 directions, each
//! with one on-foot row for anyone, of one or two clips. The state table
//! gives `Upright` `Die` the mode `FULL` and `DieAnimatedRagdoll`
//! `ON_END`.
//!
//! - **Which way the quarter turn goes**: the way and the side both make
//!   the facing a vector with 0x82854FE0, which reads the engine's sine
//!   table (8192 entries to the turn, 0x82B56558; the angle times
//!   8192 / 2pi, 0x820F8C9C over 0x82B5E574, plus a half, truncated) as
//!   `(table[i], 0, -table[i - 2048])`, that is `(sin h, 0, cos h)`. A
//!   heading is `atan2(x, z)` (0x82855048), this crate's, so facing `+z`
//!   the quarter turn (pi/2 added) points `+x`, as [`heading`] has it.
//!   The picker's side is the hit's place from the person's
//!   (record +0x58 and +0x60 less +0x48 and +0x50) along that vector,
//!   under 0 for side 1; the way's sideways test is the hit's travel
//!   (+0x64, +0x6C) along it, 0 or more for way 3.
//!
//! **Inferred**: that `FULL` is a ragdoll at once and `ON_END` one when
//! the clip ends (the driver compares the mode with `ON_END`'s hash; its
//! use was not read). And that the record's facing (+0x54) is the
//! person's heading as the engine keeps it, which nothing found writes
//! otherwise.
//!
//! **Not traced, so not here**: what sets the record's flag (height 2 is
//! never chosen: [`Hit::flagged`] is for a caller that knows), where the
//! record is written, a flag of the person's that forces the plain
//! `Die`, and death in a vehicle, which goes by a mode of the seat.

use m2_math::glam::Vec3;

/// A hit older than this picks no direction, seconds.
pub const HIT_MEMORY: f32 = 0.25;

/// The hit's direction along the facing below which it came from ahead,
/// and above the opposite of which from behind.
pub const ALONG: f32 = 0.707;

/// How far under the two body nodes a low hit lands, metres.
pub const UNDER_UPPER: f32 = 0.1;
pub const UNDER_PELVIS: f32 = 0.2;

/// The body nodes the height is judged by, by name hash.
pub const UPPER_NODE: u32 = 0x705c_4508;
pub const PELVIS_NODE: u32 = 0xfaef_b386;

/// The directions, by way, height and side: the stance and action
/// table's `DamageDirection` values (their names are not known).
pub const DIRECTIONS: [[[u32; 2]; 3]; 4] = [
    [
        [0x46c2_3ec3, 0x46c2_3ec3],
        [0x72fe_ba3b, 0xbc33_1a02],
        [0xa777_0bd6, 0xa777_0bd6],
    ],
    [
        [0x5dc2_4c72, 0x5dc2_4c72],
        [0xccf8_b4d2, 0x2ae2_1fc9],
        [0xaa72_f76b, 0xaa72_f76b],
    ],
    [
        [0x72a8_0b35, 0x72a8_0b35],
        [0x6a8f_71e3, 0x6a8f_71e3],
        [0xda62_8978, 0xda62_8978],
    ],
    [
        [0xb00f_f0da, 0xb00f_f0da],
        [0x2abc_c9f2, 0x2abc_c9f2],
        [0xd9f6_4e93, 0xd9f6_4e93],
    ],
];

/// The person hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub position: Vec3,
    /// Heading faced, radians clockwise from `+z` seen from above.
    pub facing: f32,
    /// World heights of [`UPPER_NODE`] and [`PELVIS_NODE`], where the
    /// skeleton has them.
    pub upper_y: Option<f32>,
    pub pelvis_y: Option<f32>,
}

/// The last hit the person took.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub at: Vec3,
    /// The way it was travelling.
    pub direction: Vec3,
    /// Seconds since it landed.
    pub age: f32,
    /// The record's flag (what sets it is not traced).
    pub flagged: bool,
}

/// Entries in the engine's sine table, a turn.
const SINE_ENTRIES: i32 = 8192;

/// Entry `i` of the engine's sine table (`i` taken modulo 8192).
fn sine_table(i: i32) -> f32 {
    let i = i & (SINE_ENTRIES - 1);
    (f64::from(i) * std::f64::consts::TAU / f64::from(SINE_ENTRIES)).sin() as f32
}

/// The flat way a heading points, `(x, z)` (0x82854FE0): the sine table
/// at the angle's nearest step, and its negated entry a quarter turn
/// back as the cosine.
fn heading(yaw: f32) -> (f32, f32) {
    let step = (SINE_ENTRIES as f32 / std::f32::consts::TAU).mul_add(yaw, 0.5);
    let i = step as i32;
    (sine_table(i), -sine_table(i.wrapping_sub(2048)))
}

/// The way of a hit travelling `direction` for a person facing `facing`
/// (0x82421840 as the on-foot death calls it).
pub fn way(facing: f32, direction: Vec3) -> usize {
    let (fx, fz) = heading(facing);
    let along = direction.x * fx + direction.z * fz;
    if along < -ALONG {
        0
    } else if along > ALONG {
        1
    } else {
        let (qx, qz) = heading(facing + std::f32::consts::FRAC_PI_2);
        if direction.x * qx + direction.z * qz >= 0.0 {
            3
        } else {
            2
        }
    }
}

/// The height of a hit at `hit_y` on `body`.
pub fn height(body: &Body, hit_y: f32, flagged: bool) -> usize {
    if flagged {
        return 2;
    }
    match (body.upper_y, body.pelvis_y) {
        (Some(upper), Some(pelvis))
            if hit_y <= upper - UNDER_UPPER && hit_y < pelvis - UNDER_PELVIS =>
        {
            0
        }
        _ => 1,
    }
}

/// The side of a hit at `at` on `body`.
pub fn side(body: &Body, at: Vec3) -> usize {
    let (qx, qz) = heading(body.facing + std::f32::consts::FRAC_PI_2);
    let from = at - body.position;
    usize::from(from.x * qx + from.z * qz < 0.0)
}

/// The `DamageDirection` a person killed on foot by `hit` dies in, or
/// `None` for the plain `Die` (no hit known, or too old).
pub fn damage_direction(body: &Body, hit: Option<&Hit>) -> Option<u32> {
    let hit = hit.filter(|h| h.age <= HIT_MEMORY)?;
    let w = way(body.facing, hit.direction);
    let h = height(body, hit.at.y, hit.flagged);
    Some(DIRECTIONS[w][h][side(body, hit.at)])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Standing at the origin facing `+z`, the two nodes at 1.4 and 1.0 m.
    const BODY: Body = Body {
        position: Vec3::ZERO,
        facing: 0.0,
        upper_y: Some(1.4),
        pelvis_y: Some(1.0),
    };

    fn hit(at: Vec3, direction: Vec3) -> Hit {
        Hit {
            at,
            direction,
            age: 0.0,
            flagged: false,
        }
    }

    #[test]
    fn the_table_holds_fourteen_directions() {
        let mut all: Vec<u32> = DIRECTIONS.iter().flatten().flatten().copied().collect();
        assert_eq!(all.len(), 24);
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), 14);
        // Only the two chest-high rows ahead and behind differ by side.
        for (w, ways) in DIRECTIONS.iter().enumerate() {
            for (h, pair) in ways.iter().enumerate() {
                assert_eq!(pair[0] != pair[1], w < 2 && h == 1, "{w} {h}");
            }
        }
    }

    #[test]
    fn the_way_is_by_the_hits_travel_along_the_facing() {
        // Travelling against the way they face: from ahead.
        assert_eq!(way(0.0, Vec3::NEG_Z), 0);
        assert_eq!(way(0.0, Vec3::Z), 1);
        // Across: the two sides.
        assert_eq!(way(0.0, Vec3::X), 3);
        assert_eq!(way(0.0, Vec3::NEG_X), 2);
        // The limits are at 45 degrees.
        let at = |deg: f32| {
            let (s, c) = deg.to_radians().sin_cos();
            way(0.0, Vec3::new(s, 0.0, c))
        };
        assert_eq!(at(44.0), 1);
        assert_eq!(at(46.0), 3);
        assert_eq!(at(134.0), 3);
        assert_eq!(at(136.0), 0);
        assert_eq!(at(-46.0), 2);
        // Turned to face +x, a hit travelling -x is from ahead.
        assert_eq!(way(std::f32::consts::FRAC_PI_2, Vec3::NEG_X), 0);
    }

    #[test]
    fn a_hit_under_both_nodes_is_low() {
        assert_eq!(height(&BODY, 0.5, false), 0);
        assert_eq!(height(&BODY, 0.79, false), 0);
        // Not under the pelvis less 0.2: chest high.
        assert_eq!(height(&BODY, 0.8, false), 1);
        assert_eq!(height(&BODY, 1.6, false), 1);
        // The flag, whatever the height.
        assert_eq!(height(&BODY, 0.5, true), 2);
        // A node missing: chest high.
        let bare = Body {
            pelvis_y: None,
            ..BODY
        };
        assert_eq!(height(&bare, 0.2, false), 1);
    }

    #[test]
    fn the_quarter_turn_points_to_the_right_of_the_facing() {
        // Facing +z, the turned way is +x; facing +x, it is -z.
        let (x, z) = heading(std::f32::consts::FRAC_PI_2);
        assert!((x - 1.0).abs() < 1e-6 && z.abs() < 1e-6, "{x} {z}");
        let (x, z) = heading(std::f32::consts::PI);
        assert!(x.abs() < 1e-6 && (z + 1.0).abs() < 1e-6, "{x} {z}");
        // The table rounds to its steps: a hair over 45 degrees is the
        // 1024th step, 45 exactly.
        let (x, z) = heading(45.001f32.to_radians());
        assert_eq!((x, z), heading(std::f32::consts::FRAC_PI_4));
        // Turned to face +x, a hit landing at -z is on side 1, at +z 0.
        let east = Body {
            facing: std::f32::consts::FRAC_PI_2,
            ..BODY
        };
        assert_eq!(side(&east, Vec3::new(0.0, 1.2, -0.2)), 0);
        assert_eq!(side(&east, Vec3::new(0.0, 1.2, 0.2)), 1);
        assert_eq!(way(std::f32::consts::FRAC_PI_2, Vec3::NEG_Z), 3);
    }

    #[test]
    fn the_side_is_where_the_hit_landed_across_the_body() {
        assert_eq!(side(&BODY, Vec3::new(0.2, 1.2, 0.0)), 0);
        assert_eq!(side(&BODY, Vec3::new(-0.2, 1.2, 0.0)), 1);
    }

    #[test]
    fn a_fresh_hit_picks_the_tables_word_and_an_old_one_none() {
        // From ahead, chest high, on each side of the body.
        let ahead = |x: f32| hit(Vec3::new(x, 1.2, 0.3), Vec3::NEG_Z);
        assert_eq!(
            damage_direction(&BODY, Some(&ahead(0.1))),
            Some(0x72fe_ba3b)
        );
        assert_eq!(
            damage_direction(&BODY, Some(&ahead(-0.1))),
            Some(0xbc33_1a02)
        );
        // From behind, in the legs.
        let legs = hit(Vec3::new(0.0, 0.4, -0.3), Vec3::Z);
        assert_eq!(damage_direction(&BODY, Some(&legs)), Some(0x5dc2_4c72));
        // No hit, or one a quarter of a second old and more: the plain death.
        assert_eq!(damage_direction(&BODY, None), None);
        let old = Hit {
            age: 0.26,
            ..ahead(0.1)
        };
        assert_eq!(damage_direction(&BODY, Some(&old)), None);
        let just = Hit {
            age: 0.25,
            ..ahead(0.1)
        };
        assert!(damage_direction(&BODY, Some(&just)).is_some());
    }
}
