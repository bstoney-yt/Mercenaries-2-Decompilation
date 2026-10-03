//! What a tick does: the player on foot (or the free camera), physics and
//! animation, and where the frame is seen from.

use m2_math::glam::Vec3;
use m2_render::View;

use crate::camera::FlyCamera;
use crate::config::CameraConfig;
use crate::input::{Action, Input};
use crate::player::Intent;
use crate::scene::Scene;

#[derive(Debug)]
pub struct Game {
    pub scene: Scene,
    pub camera: FlyCamera,
    /// Playing as Jennifer rather than flying the camera.
    pub on_foot: bool,
}

impl Game {
    pub fn new(scene: Scene, camera: FlyCamera) -> Self {
        let on_foot = scene.player.is_some();
        Self {
            scene,
            camera,
            on_foot,
        }
    }

    /// One fixed tick from the keyboard and mouse.
    pub fn tick(&mut self, input: &mut Input, config: &CameraConfig, mouse_look: bool, dt: f32) {
        if input.take_pressed(Action::ToggleFly) {
            self.toggle_fly();
        }
        if self.on_foot {
            let intent = Intent::from_input(input, config, mouse_look);
            self.step(intent, dt);
        } else {
            self.camera.update(input, config, mouse_look, dt);
            self.step(Intent::default(), dt);
        }
    }

    /// One fixed tick with `intent` for the player (ignored while flying).
    pub fn step(&mut self, intent: Intent, dt: f32) {
        if let Some(world) = &mut self.scene.world {
            world.physics.step(dt);
            if self.on_foot
                && let Some(player) = &mut self.scene.player
            {
                player.update(&world.physics, intent, dt);
            }
        }
        self.scene.tick(dt);
    }

    /// Switches between walking and flying. Flying starts from the player's
    /// camera; walking starts by dropping to the ground under the camera.
    pub fn toggle_fly(&mut self) {
        let Some(player) = &mut self.scene.player else {
            return;
        };
        if self.on_foot {
            self.camera.position = player.eye();
            self.camera.yaw = player.yaw;
            self.camera.pitch = player.pitch;
        } else {
            let p = self.camera.position;
            player.character.position = Vec3::new(p.x, 0.0, p.z);
            player.character.velocity = Vec3::ZERO;
            player.landed = false;
            player.yaw = self.camera.yaw;
            player.facing = self.camera.yaw;
        }
        self.on_foot = !self.on_foot;
    }

    fn walking(&self) -> Option<&crate::player::Player> {
        self.scene.player.as_ref().filter(|_| self.on_foot)
    }

    /// Where the world streams around.
    pub fn focus(&self) -> Vec3 {
        match self.walking() {
            Some(p) => p.character.position,
            None => self.camera.position,
        }
    }

    /// Where sound is heard from: the camera's position and direction.
    pub fn listener(&self) -> (Vec3, Vec3) {
        match self.walking() {
            Some(p) => (p.eye(), p.camera_forward()),
            None => (self.camera.position, self.camera.forward()),
        }
    }

    pub fn view(&self, fov: f32, aspect: f32) -> View {
        match self.walking() {
            Some(p) => p.view(fov, aspect),
            None => self.camera.view(fov, aspect),
        }
    }
}
