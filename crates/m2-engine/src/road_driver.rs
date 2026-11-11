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
//! - **Its speed** starts at 20 m/s, 10 within 100 m of its road's end
//!   ([`base_speed`]), and it stops within 10 m of an end that leads
//!   nowhere ([`DEAD_END`]).
//!
//! Not ported: the junctions' stop lines and slow-downs by their flow
//! control (0x822B9210, 0x82234B38). They act only for a road linked to an
//! `RtFlowControl` entity (0x822B81A0, store 0x8365778C, its record's +0x14
//! the kind, +0x1C the sub-kind), and no layer the alpha reads holds one.
//! Nor the 7 m/s at a road's very end (its +0x18 exactly 0 or 1), nor what
//! 0x822AAAE8 does under a stop request: 0x822BB270 (from 0x822C0958) sets
//! the follower's +0x1B8 bit 3 and a 2 s timer (+0x1C0) that runs down;
//! while it is set it brakes, its wheel jiggled at random. What asks it to
//! stop so isn't traced.

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

/// Whether it is within `within` metres of its road's end ahead, with
/// `remaining` metres of its road of `length` left (0x822B8138: `within`
/// of the length, that share capped at 0.75 once it is over 0.9).
pub fn near_end(remaining: f32, length: f32, within: f32) -> bool {
    let length = length.max(1e-3);
    let mut share = within / length;
    if share > 0.9 {
        share = 0.75;
    }
    remaining / length < share
}

/// The speed it starts from, m/s (0x822AAD80): 20 (0x820BB204), 10 within
/// 100 m of its road's end ([`near_end`]).
pub fn base_speed(remaining: f32, length: f32) -> f32 {
    if near_end(remaining, length, 100.0) {
        10.0
    } else {
        20.0
    }
}

/// It stops within this of a road's end that leads nowhere, metres
/// (0x822B9440: 0x822B8138 with 10, and 0x8223D270 finding no road out
/// of the junction there).
pub const DEAD_END: f32 = 10.0;

/// Where its look ahead starts and how far it reaches (0x822B8C68): from
/// its origin 0.6 m up and 2 m along the way it is going (ahead, or behind
/// when its follower's point is behind it), 3 m on that way.
pub const PROBE_UP: f32 = 0.6;
pub const PROBE_OUT: f32 = 2.0;

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

    // 0x822B8138 and 0x822AAD80's start: 20 m/s, 10 within 100 m of the
    // end; on a road under 111 m long, within 0.75 of its length.
    #[test]
    fn it_slows_toward_its_roads_end() {
        assert_eq!(base_speed(400.0, 1000.0), 20.0);
        assert_eq!(base_speed(90.0, 1000.0), 10.0);
        assert!(near_end(70.0, 100.0, 100.0));
        assert!(!near_end(80.0, 100.0, 100.0));
        assert!(near_end(5.0, 300.0, DEAD_END));
        assert!(!near_end(15.0, 300.0, DEAD_END));
    }

    #[test]
    fn it_looks_ahead_every_so_often() {
        assert_eq!(probe_every(0.0, 0.9), 0.15);
        assert!((probe_every(0.5, 1.0) - 0.4).abs() < 1e-6);
    }
}
