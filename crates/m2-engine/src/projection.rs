//! The game camera's projection: its own field of view, near and far, and
//! how the field of view becomes the picture's width and height.
//!
//! From the 360 code. The camera's default block (0x821B6148) holds a near
//! distance of 0.2, a far distance of 2400 and a field of view of
//! 0.95974994 radians. The renderer's projection builder (0x82569110)
//! takes the near, far and field of view of the view's current camera
//! block and makes:
//!
//! - half angle = `fov * 0.75 * aspect / 2` (0.75 at 0x820CAF1C; the
//!   aspect is the renderer's, 4/3 or 16/9, scaled by the viewport's share
//!   of the back buffer);
//! - half width at distance 1 = `tan(half angle)` (from the engine's sine
//!   table);
//! - half height = half width / aspect.
//!
//! So the field of view is the picture's width at 4:3 (55 degrees), and a
//! wider screen sees more to the sides: 73.3 degrees across and 45.4 up
//! and down at 16:9, against 55 across and 42.6 up and down at 4:3.
//!
//! The HUD's lock frame builds its projection by the same rule
//! (0x8234EF98, read by the UI thread).
//!
//! Not taken: an offset the camera adds to the field of view before the
//! renderer reads it (view +0x638, added in 0x824B8208); what sets it was
//! not read.

/// The camera's own field of view, radians.
pub const FOV: f32 = 0.959_749_94;
/// The camera's own near and far distances, metres.
pub const NEAR: f32 = 0.2;
pub const FAR: f32 = 2400.0;

/// The share of the field of view taken per unit of aspect (0x820CAF1C).
const FOV_PER_ASPECT: f32 = 0.75;

/// Half the picture's width at distance 1 for the game's field of view
/// `fov` (radians) on a screen `aspect` wide for its height.
pub fn half_width(fov: f32, aspect: f32) -> f32 {
    (fov * FOV_PER_ASPECT * aspect * 0.5).tan()
}

/// Half the picture's height at distance 1: the half width over the
/// aspect.
pub fn half_height(fov: f32, aspect: f32) -> f32 {
    half_width(fov, aspect) / aspect
}

/// The picture's height as an angle, radians, for the game's field of
/// view `fov` (radians) at `aspect`: what a projection asking for a
/// vertical field of view takes.
pub fn fov_y(fov: f32, aspect: f32) -> f32 {
    2.0 * half_height(fov, aspect).atan()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_field_of_view_is_the_width_at_four_by_three() {
        let a = 4.0 / 3.0;
        // 0.75 x 4/3 = 1: the whole field of view across.
        assert!((2.0 * half_width(FOV, a).atan() - FOV).abs() < 1e-6);
        assert!((FOV.to_degrees() - 54.99).abs() < 0.01);
        assert!((fov_y(FOV, a).to_degrees() - 42.65).abs() < 0.05);
    }

    #[test]
    fn a_wide_screen_sees_more_to_the_sides() {
        let a = 16.0 / 9.0;
        let across = 2.0 * half_width(FOV, a).atan();
        assert!(
            (across.to_degrees() - 73.3).abs() < 0.1,
            "{}",
            across.to_degrees()
        );
        assert!((fov_y(FOV, a).to_degrees() - 45.4).abs() < 0.1);
    }

    /// The half width and height at distance 1, as the matrix builder
    /// (0x82569080) takes them: 0.7443 by 0.4187 at 16:9, in the screen's
    /// own shape.
    #[test]
    fn the_half_extents_are_in_the_screens_shape() {
        for a in [4.0 / 3.0, 16.0 / 9.0] {
            let (w, h) = (half_width(FOV, a), half_height(FOV, a));
            assert!((w / h - a).abs() < 1e-5, "{w} {h}");
        }
        let (w, h) = (half_width(FOV, 16.0 / 9.0), half_height(FOV, 16.0 / 9.0));
        assert!(
            (w - 0.7443).abs() < 1e-4 && (h - 0.4187).abs() < 1e-4,
            "{w} {h}"
        );
        assert_eq!((NEAR, FAR), (0.2, 2400.0));
    }
}
