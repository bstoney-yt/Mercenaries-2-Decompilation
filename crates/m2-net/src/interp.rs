//! Playing the other player's hero back smoothly from snapshots that
//! arrive late, unevenly, out of order or not at all.
//!
//! The usual way (invented here, not the original game's): snapshots are
//! shown [`DELAY`] behind the newest one, so there is nearly always a
//! snapshot on each side of the moment drawn and the hero moves between
//! them in a straight line. When the next one is late, the hero carries on
//! at its last velocity for at most [`MAX_EXTRAPOLATE`], then holds.
//!
//! The two games' clocks differ, so each snapshot's own time is mapped to
//! this game's clock by an offset: the smallest gap seen between when one
//! arrived and when it was taken (the least-delayed one tells the clock
//! difference best), creeping up slowly if the route gets slower.

use std::collections::VecDeque;

use crate::proto::{Gait, HeroState};

/// How far behind the newest snapshot the hero is shown, seconds. Three
/// snapshots at 30 a second: one may be lost without a stall.
pub const DELAY: f64 = 0.1;
/// The longest the hero carries on past the newest snapshot, seconds.
pub const MAX_EXTRAPOLATE: f64 = 0.25;
/// Snapshots older than this behind the newest are let go, seconds.
const KEEP: f64 = 1.0;
/// How fast the clock offset follows a route that got slower, per
/// snapshot (a hundred snapshots to close most of a step).
const CREEP: f64 = 0.01;

/// The hero as drawn now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub facing: f32,
    pub gait: Gait,
    pub flags: u8,
}

#[derive(Debug, Default)]
pub struct Playback {
    snapshots: VecDeque<HeroState>,
    newest_seq: Option<u32>,
    /// Local time minus the sender's, at its least.
    offset: Option<f64>,
    /// Snapshots dropped as older than the newest.
    pub stale: u32,
}

impl Playback {
    pub fn new() -> Self {
        Self::default()
    }

    /// A snapshot that arrived at local time `now`.
    pub fn push(&mut self, s: HeroState, now: f64) {
        if self.newest_seq.is_some_and(|n| s.seq <= n) {
            self.stale += 1;
            return;
        }
        self.newest_seq = Some(s.seq);
        let gap = now - s.time;
        self.offset = Some(match self.offset {
            Some(o) if gap >= o => o + (gap - o) * CREEP,
            _ => gap,
        });
        self.snapshots.push_back(s);
        while self
            .snapshots
            .front()
            .is_some_and(|f| s.time - f.time > KEEP)
        {
            self.snapshots.pop_front();
        }
    }

    /// Whether any snapshot has come.
    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    /// The hero at local time `now`, or `None` before any snapshot.
    pub fn sample(&self, now: f64) -> Option<Pose> {
        let offset = self.offset?;
        self.sample_at(now - offset - DELAY)
    }

    /// The hero at the sender's time `t`.
    pub fn sample_at(&self, t: f64) -> Option<Pose> {
        let newest = self.snapshots.back()?;
        if t >= newest.time {
            let dt = (t - newest.time).min(MAX_EXTRAPOLATE) as f32;
            let mut pose = pose(newest);
            for (p, v) in pose.position.iter_mut().zip(newest.velocity) {
                *p += v * dt;
            }
            return Some(pose);
        }
        // The first snapshot after t; the one before it brackets t.
        let i = self.snapshots.partition_point(|s| s.time <= t);
        if i == 0 {
            return Some(pose(&self.snapshots[0]));
        }
        let (a, b) = (&self.snapshots[i - 1], &self.snapshots[i]);
        let f = ((t - a.time) / (b.time - a.time)) as f32;
        let lerp = |x: [f32; 3], y: [f32; 3]| std::array::from_fn(|k| x[k] + (y[k] - x[k]) * f);
        let near = if f < 0.5 { a } else { b };
        Some(Pose {
            position: lerp(a.position, b.position),
            velocity: lerp(a.velocity, b.velocity),
            facing: a.facing + wrap(b.facing - a.facing) * f,
            gait: near.gait,
            flags: near.flags,
        })
    }
}

fn pose(s: &HeroState) -> Pose {
    Pose {
        position: s.position,
        velocity: s.velocity,
        facing: s.facing,
        gait: s.gait,
        flags: s.flags,
    }
}

/// An angle brought into -π..π, so turning goes the short way round.
fn wrap(a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    (a + PI).rem_euclid(TAU) - PI
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(seq: u32, time: f64, x: f32) -> HeroState {
        HeroState {
            seq,
            time,
            position: [x, 0.0, 0.0],
            velocity: [3.0, 0.0, 0.0],
            gait: Gait::Run,
            ..Default::default()
        }
    }

    #[test]
    fn halfway_between_two_snapshots_is_halfway() {
        let mut p = Playback::new();
        p.push(snap(1, 0.0, 0.0), 10.05);
        p.push(snap(2, 0.1, 0.3), 10.15);
        let pose = p.sample_at(0.05).unwrap();
        assert!((pose.position[0] - 0.15).abs() < 1e-6);
    }

    #[test]
    fn shown_a_delay_behind_on_the_senders_clock() {
        let mut p = Playback::new();
        // 50 ms on the way; the sender's clock 10 s behind ours.
        for i in 0..10 {
            let t = i as f64 / 30.0;
            p.push(snap(i, t, t as f32 * 3.0), 10.05 + t);
        }
        // At local 10.25 the sender's time is 0.2, shown at 0.1.
        let pose = p.sample(10.25).unwrap();
        assert!((pose.position[0] - 0.3).abs() < 1e-4, "{pose:?}");
    }

    #[test]
    fn late_and_repeated_snapshots_are_dropped() {
        let mut p = Playback::new();
        p.push(snap(2, 0.1, 0.3), 1.0);
        p.push(snap(1, 0.0, 0.0), 1.0);
        p.push(snap(2, 0.1, 0.3), 1.0);
        assert_eq!(p.stale, 2);
        assert_eq!(p.snapshots.len(), 1);
    }

    #[test]
    fn carries_on_then_holds_when_snapshots_stop() {
        let mut p = Playback::new();
        p.push(snap(1, 0.0, 0.0), 0.0);
        assert!((p.sample_at(0.1).unwrap().position[0] - 0.3).abs() < 1e-6);
        let held = p.sample_at(5.0).unwrap().position[0];
        assert!((held - 3.0 * MAX_EXTRAPOLATE as f32).abs() < 1e-6);
    }

    #[test]
    fn turns_the_short_way_round() {
        let mut p = Playback::new();
        let mut a = snap(1, 0.0, 0.0);
        a.facing = 3.0;
        let mut b = snap(2, 1.0, 0.0);
        b.facing = -3.0; // 0.28 rad on through π, not 6 rad back
        p.push(a, 0.0);
        p.push(b, 1.0);
        let f = p.sample_at(0.5).unwrap().facing;
        assert!((wrap(f).abs() - std::f32::consts::PI).abs() < 1e-3, "{f}");
    }

    #[test]
    fn a_slower_route_moves_the_offset_up_slowly() {
        let mut p = Playback::new();
        p.push(snap(1, 0.0, 0.0), 0.05);
        assert_eq!(p.offset, Some(0.05));
        p.push(snap(2, 0.1, 0.0), 0.25); // 150 ms this time
        let o = p.offset.unwrap();
        assert!((o - (0.05 + 0.1 * CREEP)).abs() < 1e-9, "{o}");
        p.push(snap(3, 0.2, 0.0), 0.22); // 20 ms: taken at once
        assert!((p.offset.unwrap() - 0.02).abs() < 1e-9);
    }
}
