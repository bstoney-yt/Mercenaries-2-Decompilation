//! Aim assist for a pad or the touch controls: while she aims or fires,
//! the view is drawn towards the hostile nearest the crosshair, if one is
//! close to it. Invented (the PC game aims by mouse; how the console
//! version's assist worked was not read): the cone, reach and pull are
//! tuning, not the game's.

use m2_math::glam::{Vec2, Vec3};

/// How far off the crosshair a target is still drawn to, radians (12°).
pub const CONE: f32 = 0.21;
/// How far away a target is still drawn to, metres.
pub const REACH: f32 = 120.0;
/// How fast the view closes on the target: the share of the gap left
/// after a second is e^-PULL.
pub const PULL: f32 = 7.0;
/// Where on a person the view is drawn to: this share of their height up
/// from their feet (the chest).
pub const CHEST: f32 = 0.65;
/// While the player turns the view faster than this a tick (radians: 0.6
/// a second at 60 ticks), the assist lets go: a deliberate swipe off a
/// target isn't fought.
pub const LET_GO: f32 = 0.01;

/// The camera's turn (yaw, pitch, radians, as [`crate::player::Intent::look`])
/// that draws the crosshair, from `eye` along `yaw` and `pitch` (the
/// crosshair's own, which sits above the camera's on foot), towards the
/// nearest of `targets` (points to aim at) to it, for `dt` seconds;
/// `None` when none is within [`CONE`] and [`REACH`]. The camera turns
/// by as much as the crosshair does.
pub fn assist(
    eye: Vec3,
    yaw: f32,
    pitch: f32,
    targets: impl IntoIterator<Item = Vec3>,
    dt: f32,
) -> Option<Vec2> {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let forward = Vec3::new(sy * cp, sp, cy * cp);
    let best = targets
        .into_iter()
        .filter_map(|t| {
            let d = t - eye;
            let distance = d.length();
            if !(1.0..=REACH).contains(&distance) {
                return None;
            }
            let off = forward.angle_between(d / distance);
            (off <= CONE).then_some((off, d))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))?;
    let d = best.1;
    let want_yaw = d.x.atan2(d.z);
    let want_pitch = (d.y / d.length()).asin();
    let mut gap_yaw = want_yaw - yaw;
    // The short way round.
    gap_yaw =
        (gap_yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    let share = 1.0 - (-PULL * dt).exp();
    Some(Vec2::new(gap_yaw, want_pitch - pitch) * share)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_near_the_crosshair_draws_the_view_towards_it() {
        // Looking along +z; a soldier 50 m on, a little to the +x side.
        let target = Vec3::new(5.0, 0.0, 50.0);
        let turn = assist(Vec3::ZERO, 0.0, 0.0, [target], 1.0 / 60.0).unwrap();
        // Yaw grows towards +x; a share of the gap, not all of it.
        let gap = 5.0f32.atan2(50.0);
        assert!(turn.x > 0.0 && turn.x < gap, "{turn}");
        assert!(turn.y.abs() < 1e-6);
        // Over a second it closes most of the way.
        let mut yaw = 0.0;
        for _ in 0..60 {
            yaw += assist(Vec3::ZERO, yaw, 0.0, [target], 1.0 / 60.0)
                .unwrap()
                .x;
        }
        assert!((yaw - gap).abs() < gap * 0.01, "{yaw} {gap}");
    }

    #[test]
    fn targets_off_the_crosshair_or_too_far_are_left_alone() {
        let wide = Vec3::new(30.0, 0.0, 30.0);
        let far = Vec3::new(0.0, 0.0, REACH + 10.0);
        assert_eq!(assist(Vec3::ZERO, 0.0, 0.0, [wide, far], 0.016), None);
    }

    #[test]
    fn the_nearest_to_the_crosshair_wins_and_turns_take_the_short_way() {
        // Looking along -z (yaw pi): one target just to one side of it
        // across the yaw's wrap, another further off.
        let near = Vec3::new(-2.0, 0.0, -50.0);
        let other = Vec3::new(8.0, 0.0, -50.0);
        let turn = assist(Vec3::ZERO, std::f32::consts::PI, 0.0, [other, near], 1.0).unwrap();
        // A small turn, not nearly a whole one.
        assert!(turn.x.abs() < 0.1, "{turn}");
    }

    #[test]
    fn a_target_above_raises_the_view() {
        let turn = assist(Vec3::ZERO, 0.0, 0.0, [Vec3::new(0.0, 5.0, 40.0)], 0.1).unwrap();
        assert!(turn.y > 0.0);
    }
}
