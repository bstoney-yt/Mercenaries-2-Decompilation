//! A free-flying camera, until the player character takes over.

use m2_math::glam::Vec3;
use m2_render::View;

use crate::config::CameraConfig;
use crate::input::{Action, Input};

/// Position and heading. World space is the game's: left-handed, `+y` up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlyCamera {
    pub position: Vec3,
    /// Radians clockwise from `+z` seen from above.
    pub yaw: f32,
    /// Radians up from the horizon.
    pub pitch: f32,
}

impl FlyCamera {
    /// A camera at `eye` looking at `target`.
    pub fn looking_at(eye: Vec3, target: Vec3) -> Self {
        let d = (target - eye).normalize_or(Vec3::Z);
        Self {
            position: eye,
            yaw: d.x.atan2(d.z),
            pitch: d.y.clamp(-1.0, 1.0).asin(),
        }
    }

    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, cy * cp)
    }

    pub fn right(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(-cy, 0.0, sy)
    }

    /// One fixed tick of `dt` seconds.
    pub fn update(&mut self, input: &mut Input, config: &CameraConfig, mouse_look: bool, dt: f32) {
        let (dx, dy) = input.take_look(mouse_look);
        let invert = if config.invert_y { -1.0 } else { 1.0 };
        self.yaw -= dx as f32 * config.mouse_sensitivity;
        self.pitch -= dy as f32 * config.mouse_sensitivity * invert;
        self.pitch = self.pitch.clamp(-1.55, 1.55);
        let mut speed = config.speed;
        if input.held(Action::Sprint) {
            speed *= 5.0;
        }
        let forward = input.axis(Action::MoveBack, Action::MoveForward);
        let right = input.axis(Action::MoveLeft, Action::MoveRight);
        let up = input.axis(Action::MoveDown, Action::MoveUp);
        let motion = self.forward() * forward + self.right() * right + Vec3::Y * up;
        self.position += motion.normalize_or_zero() * speed * dt;
    }

    pub fn view(&self, fov_degrees: f32, aspect: f32) -> View {
        View::perspective(
            self.position,
            self.forward(),
            fov_degrees.to_radians(),
            aspect,
            0.1,
            10_000.0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looking_at_points_the_right_way() {
        let c = FlyCamera::looking_at(Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0));
        assert!((c.forward() - Vec3::X).length() < 1e-5);
        // Right-handed: facing +x, right is +z.
        assert!((c.right() - Vec3::Z).length() < 1e-5);
        let c = FlyCamera::looking_at(Vec3::ZERO, Vec3::new(0.0, 5.0, 5.0));
        assert!((c.pitch - std::f32::consts::FRAC_PI_4).abs() < 1e-5);
    }

    #[test]
    fn right_matches_the_view_matrix() {
        let c = FlyCamera::looking_at(Vec3::ZERO, Vec3::new(3.0, 1.0, -2.0));
        let v = c.view(60.0, 1.0);
        // A point to the camera's right lands at positive view-space x.
        let p = v
            .view
            .transform_point3(c.position + c.forward() + c.right());
        assert!(p.x > 0.5, "{p}");
    }
}
