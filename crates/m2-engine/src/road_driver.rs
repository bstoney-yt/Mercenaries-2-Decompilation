//! How the 360's road traffic driver (0x822AAD80) picks its speed and when
//! it stops: for a car ahead in its lane, and for anything a box just
//! ahead of it meets.
//!
//! From the 360 code: each think it asks 0x822B9440 for a speed, then
//! drives toward its follower's point at it by 0x822AC008; under 0.5 m/s
//! it brakes and holds instead (0x822D3058, 0x822D3148).
//! - **A car ahead** (reason 4): 0x822B8710 takes, of what the road
//!   registry (store 0x8365739C, filled by 0x82265540 from each vehicle's
//!   and each player's road, lane and way) has on its road, lane and way,
//!   the nearest ahead; 0x822B90A0 then, by the gap `d` and its own speed
//!   `v` ([`follow`]): half speed when that car isn't within 0.7 (the
//!   cosine) of straight ahead; a stop when `d` is under the smaller of
//!   `2 v + 8` and 30 m, or under 8 m; between 8 and 30 m the speed asked
//!   times `3 f^2 - 2 f^3`, `f = (d - 8) / 27.5`; past 30 m no limit.
//! - **Something in the way** (reason 3, 0x822B8C68): every 0.15 s, or
//!   0.15 to 0.4 s (random) while it is partway along its road
//!   ([`probe_every`]), it puts a box just ahead of it (0x822E3558: 0.6 m
//!   across, 1 m high, 3 m long) and stops while that meets something;
//!   it counts the seconds it has been blocked so.
//!
//! Not ported here (the data they need isn't mapped): the junctions' stop
//! lines and slow-downs by their kind and traffic control (0x822B9210,
//! 0x82234B38), the stop at a road that leads nowhere (0x8223D270), the
//! road record's own speeds (7, 20 or 10 m/s by its +0x18), and the wheel
//! jiggled after 6 s blocked (0x822AAAE8).

/// Under this it brakes and holds, m/s (0x822AAD80's 0.5).
pub const HOLD: f32 = 0.5;
/// A car ahead nearer than this stops it, metres (0x822B90A0's 8).
pub const NEAREST: f32 = 8.0;
/// No car ahead further than this slows it, metres (0x822B90A0's 30).
pub const FURTHEST: f32 = 30.0;
/// A car ahead counts as ahead of it within this cosine of straight
/// ahead (0x822B90A0's 0.7).
pub const AHEAD_COS: f32 = 0.7;
/// The box it puts ahead of itself: half its width and height, and its
/// length (0x822B8C68's 0.6 by 1 by 3 m, to 0x822E3558).
pub const PROBE_HALF: [f32; 2] = [0.3, 0.5];
pub const PROBE_LENGTH: f32 = 3.0;

/// The speed it asks for a car ahead `gap` metres off going the way it
/// does when `aligned` (within [`AHEAD_COS`]), its own speed `v` and the
/// speed it would ask `want` (0x822B90A0): `None` with no limit, `Some(0)`
/// to stop.
pub fn follow(gap: f32, v: f32, want: f32, aligned: bool) -> Option<f32> {
    if !aligned {
        return Some(want * 0.5);
    }
    let room = (2.0 * v + NEAREST).min(FURTHEST);
    if gap < room || gap < NEAREST {
        return Some(0.0);
    }
    if gap >= FURTHEST {
        return None;
    }
    let f = (gap - NEAREST) / 27.5;
    Some((f * -2.0 + 3.0) * f * f * want)
}

/// Seconds to its next look ahead (0x822B8C68): 0.15, plus up to 0.25 at
/// random (`roll`, 0..1) while it is partway along its road (`t` strictly
/// between 0 and 1).
pub fn probe_every(t: f32, roll: f32) -> f32 {
    if t > 0.0 && t < 1.0 {
        0.15 + roll * 0.25
    } else {
        0.15
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 0x822B90A0: stops inside its room (2 v + 8, at most 30), eases in
    // between 8 and 30 m, nothing past 30 m, half speed for one off line.
    #[test]
    fn a_car_ahead_slows_or_stops_it_as_on_the_360() {
        assert_eq!(follow(5.0, 0.0, 20.0, true), Some(0.0));
        assert_eq!(follow(15.0, 5.0, 20.0, true), Some(0.0));
        assert_eq!(follow(40.0, 30.0, 20.0, true), None);
        let eased = follow(20.0, 2.0, 20.0, true).unwrap();
        let f = 12.0f32 / 27.5;
        assert!((eased - (3.0 - 2.0 * f) * f * f * 20.0).abs() < 1e-4);
        assert!(eased > 0.0 && eased < 20.0);
        assert_eq!(follow(20.0, 2.0, 20.0, false), Some(10.0));
    }

    #[test]
    fn it_looks_ahead_every_so_often() {
        assert_eq!(probe_every(0.0, 0.9), 0.15);
        assert!((probe_every(0.5, 1.0) - 0.4).abs() < 1e-6);
    }
}
