//! The player on foot: Jennifer walking the island under the character
//! controller, with a third-person camera behind her.

use m2_engine::{Character, Gait, GpuModel, Physics};
use m2_math::glam::{Mat4, Vec2, Vec3};
use m2_render::View;

use crate::combat::{Trigger, Weapon};
use crate::config::CameraConfig;
use crate::input::{Action, Input};
use crate::scene::Rig;

/// Movement speeds, m/s, inside the bands the game's clips move at.
const WALK: f32 = 1.4;
const RUN: f32 = 5.0;
const SPRINT: f32 = 7.2;
/// Take-off speed of a jump, m/s (about 0.8 m high).
const JUMP: f32 = 4.0;
/// How fast she turns to face where she is going, radians per second.
const TURN_RATE: f32 = 10.0;
/// Camera: eye height looked at, distance behind, and closest it gets
/// when something is in the way.
const LOOK_HEIGHT: f32 = 1.6;
const CAMERA_DISTANCE: f32 = 4.0;
const CAMERA_MIN: f32 = 0.6;

/// What the player asks for this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Intent {
    /// Camera-relative: x right, y forward; length at most 1.
    pub movement: Vec2,
    pub sprint: bool,
    pub walk: bool,
    pub jump: bool,
    /// Jump held: the brake when driving.
    pub hold_jump: bool,
    /// Get in or out of a vehicle.
    pub use_pressed: bool,
    /// Turn an overturned vehicle back up.
    pub right_vehicle: bool,
    /// Camera turn, radians: (yaw, pitch).
    pub look: Vec2,
    /// Weapons.
    pub trigger: Trigger,
}

impl Intent {
    pub fn from_input(input: &mut Input, config: &CameraConfig, mouse_look: bool) -> Self {
        let (dx, dy) = input.take_mouse();
        let look = if mouse_look {
            let invert = if config.invert_y { -1.0 } else { 1.0 };
            Vec2::new(
                dx as f32 * config.mouse_sensitivity,
                -dy as f32 * config.mouse_sensitivity * invert,
            )
        } else {
            Vec2::ZERO
        };
        Self {
            movement: Vec2::new(
                input.axis(Action::MoveLeft, Action::MoveRight),
                input.axis(Action::MoveBack, Action::MoveForward),
            )
            .clamp_length_max(1.0),
            sprint: input.held(Action::Sprint),
            walk: input.held(Action::Walk),
            jump: input.take_pressed(Action::MoveUp),
            hold_jump: input.held(Action::MoveUp),
            use_pressed: input.take_pressed(Action::Use),
            right_vehicle: input.take_pressed(Action::Reset),
            look,
            trigger: Trigger {
                fire: input.held(Action::Fire),
                aim: input.held(Action::Aim),
                throw: input.take_pressed(Action::Throw),
                select: if input.take_pressed(Action::Rifle) {
                    Some(Weapon::Rifle)
                } else if input.take_pressed(Action::Launcher) {
                    Some(Weapon::Rpg)
                } else {
                    None
                },
            },
        }
    }
}

/// The player character.
#[derive(Debug)]
pub struct Player {
    pub character: Character,
    /// Heading she faces, radians clockwise from `+z` seen from above.
    pub facing: f32,
    /// Camera heading and pitch (radians, pitch up from the horizon).
    pub yaw: f32,
    pub pitch: f32,
    /// Distance from the look point to the camera this frame.
    camera_distance: f32,
    /// While driving: the point the chase camera looks at, and the
    /// vehicle's length.
    pub chase_target: Option<(Vec3, f32)>,
    /// False until there is ground under the spawn point to land on.
    pub landed: bool,
    pub model: Option<(GpuModel, Rig)>,
    /// The vehicle she is driving (index into the scene's vehicles).
    pub driving: Option<usize>,
    /// A clip to hold when standing still (aiming), facing the camera's
    /// way.
    pub hold: Option<usize>,
    /// Down (out of health): she lies where she fell.
    pub down: bool,
}

impl Player {
    /// A player who will drop onto the ground at `(x, z)` once it loads.
    pub fn new(x: f32, z: f32, yaw: f32, model: Option<(GpuModel, Rig)>) -> Self {
        Self {
            character: Character::new(Vec3::new(x, 0.0, z)),
            facing: yaw,
            yaw,
            pitch: -0.2,
            camera_distance: CAMERA_DISTANCE,
            chase_target: None,
            landed: false,
            model,
            driving: None,
            hold: None,
            down: false,
        }
    }

    /// One fixed tick of `dt` seconds.
    pub fn update(&mut self, physics: &Physics, intent: Intent, dt: f32) {
        self.yaw += intent.look.x;
        self.pitch = (self.pitch + intent.look.y).clamp(-1.2, 0.9);

        if !self.landed {
            // Wait for the ground, then stand on it.
            let p = self.character.position;
            if let Some((d, _)) = physics.cast_ray(Vec3::new(p.x, 3000.0, p.z), -Vec3::Y, 6000.0) {
                self.character.position.y = 3000.0 - d + 0.05;
                self.landed = true;
            }
            self.camera(physics);
            return;
        }

        if self.down {
            physics.move_character(&mut self.character, Vec3::ZERO, None, dt);
            if let Some((_, rig)) = &mut self.model {
                rig.animator.advance(dt);
            }
            self.camera(physics);
            return;
        }

        let (s, c) = self.yaw.sin_cos();
        let forward = Vec3::new(s, 0.0, c);
        let right = Vec3::new(c, 0.0, -s);
        let direction = forward * intent.movement.y + right * intent.movement.x;
        let speed = if intent.walk {
            WALK
        } else if intent.sprint {
            SPRINT
        } else {
            RUN
        };
        let wanted = direction * speed;
        let jump = (intent.jump && self.character.grounded).then_some(JUMP);
        physics.move_character(&mut self.character, wanted, jump, dt);

        let heading = if self.hold.is_some() {
            Some(self.yaw)
        } else {
            (direction.length_squared() > 0.01).then(|| direction.x.atan2(direction.z))
        };
        if let Some(target) = heading {
            let mut turn = (target - self.facing).rem_euclid(std::f32::consts::TAU);
            if turn > std::f32::consts::PI {
                turn -= std::f32::consts::TAU;
            }
            self.facing += turn.clamp(-TURN_RATE * dt, TURN_RATE * dt);
        }

        let moved = Vec2::new(self.character.velocity.x, self.character.velocity.z).length();
        if let Some((_, rig)) = &mut self.model {
            let gait = if moved < 0.3 {
                Gait::Idle
            } else if intent.walk {
                Gait::Walk
            } else if intent.sprint {
                Gait::Sprint
            } else {
                Gait::Run
            };
            match (gait, self.hold) {
                (Gait::Idle, Some(clip)) => rig.animator.play(clip),
                _ => rig.set_gait(gait),
            }
            // Play the clip at the speed she actually moves, so feet don't
            // slide (within reason: against a wall the legs keep going).
            let rate = match rig.gait_speed(gait) {
                Some(clip) if gait != Gait::Idle && clip > 0.1 => (moved / clip).clamp(0.6, 1.4),
                _ => 1.0,
            };
            rig.animator.advance(dt * rate);
        }
        self.camera(physics);
    }

    fn look_at(&self) -> Vec3 {
        self.character.position + Vec3::Y * LOOK_HEIGHT
    }

    pub fn camera_forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, cy * cp)
    }

    /// Pulls the camera in when something is between it and her.
    fn camera(&mut self, physics: &Physics) {
        let back = -self.camera_forward();
        self.camera_distance = match physics.cast_ray(self.look_at(), back, CAMERA_DISTANCE) {
            Some((d, _)) => (d - 0.2).max(CAMERA_MIN),
            None => CAMERA_DISTANCE,
        };
    }

    /// Chase camera for a vehicle at `transform` `length` metres long:
    /// turns `look` and drifts behind the vehicle's heading when the mouse
    /// is still.
    pub fn chase(&mut self, transform: Mat4, length: f32, look: Vec2, dt: f32) {
        self.chase_target = Some((transform.transform_point3(Vec3::Y * 2.0), length));
        self.yaw += look.x;
        self.pitch = (self.pitch + look.y).clamp(-1.2, 0.9);
        if look == Vec2::ZERO {
            let f = transform.transform_vector3(Vec3::Z);
            let heading = f.x.atan2(f.z);
            let mut turn = (heading - self.yaw).rem_euclid(std::f32::consts::TAU);
            if turn > std::f32::consts::PI {
                turn -= std::f32::consts::TAU;
            }
            self.yaw += turn * (2.0 * dt).min(1.0);
        }
        // She rides along, so streaming and sound follow the vehicle.
        self.character.position = transform.transform_point3(Vec3::ZERO);
        self.character.velocity = Vec3::ZERO;
    }

    pub fn eye(&self) -> Vec3 {
        match self.chase_target {
            Some((at, length)) => at - self.camera_forward() * (length * 0.9 + 3.0),
            None => self.look_at() - self.camera_forward() * self.camera_distance,
        }
    }

    pub fn view(&self, fov_degrees: f32, aspect: f32) -> View {
        View::perspective(
            self.eye(),
            self.camera_forward(),
            fov_degrees.to_radians(),
            aspect,
            0.1,
            10_000.0,
        )
    }

    /// Model to world for her model, turned so its forward is her facing.
    pub fn model_matrix(&self) -> Mat4 {
        let model_forward = self
            .model
            .as_ref()
            .and_then(|(_, rig)| rig.locomotion.as_ref())
            .map_or(Vec2::Y, |l| l.forward);
        let model_yaw = model_forward.x.atan2(model_forward.y);
        Mat4::from_translation(self.character.position)
            * Mat4::from_rotation_y(self.facing - model_yaw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_engine::{CollisionShape, StaticKey};
    use m2_math::glam::Quat;

    fn ground() -> Physics {
        let mut p = Physics::new();
        let h = 100.0;
        let v = [
            Vec3::new(-h, 0.0, -h),
            Vec3::new(h, 0.0, -h),
            Vec3::new(h, 0.0, h),
            Vec3::new(-h, 0.0, h),
        ];
        let shape = CollisionShape::mesh(&v, &[[0, 1, 2], [0, 2, 3]]).unwrap();
        p.add_static(
            StaticKey::Cell(1),
            &shape,
            Vec3::new(0.0, 10.0, 0.0),
            Quat::IDENTITY,
        );
        p.step(1.0 / 60.0);
        p
    }

    #[test]
    fn lands_then_runs_where_the_camera_faces() {
        let physics = ground();
        // Camera facing +x.
        let mut p = Player::new(0.0, 0.0, std::f32::consts::FRAC_PI_2, None);
        let dt = 1.0 / 60.0;
        p.update(&physics, Intent::default(), dt);
        assert!(p.landed);
        assert!((p.character.position.y - 10.0).abs() < 0.2);
        let forward = Intent {
            movement: Vec2::Y,
            ..Intent::default()
        };
        for _ in 0..60 {
            p.update(&physics, forward, dt);
        }
        let at = p.character.position;
        assert!((at.x - RUN).abs() < 0.6, "{at}");
        assert!(at.z.abs() < 0.1, "{at}");
        assert!((p.facing - std::f32::consts::FRAC_PI_2).abs() < 1e-3);
        // The camera is behind her and above.
        let eye = p.eye();
        assert!(eye.x < at.x && eye.y > at.y, "{eye}");
    }

    #[test]
    fn model_turns_to_face_her_heading() {
        let mut p = Player::new(0.0, 0.0, 0.0, None);
        p.facing = std::f32::consts::FRAC_PI_2;
        // Without a rig the model is taken to face +z.
        let forward = p.model_matrix().transform_vector3(Vec3::Z);
        assert!((forward - Vec3::X).length() < 1e-5, "{forward}");
    }
}
