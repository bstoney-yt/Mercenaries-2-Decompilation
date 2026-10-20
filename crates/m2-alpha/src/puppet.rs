//! The other player's hero in online co-op: drawn and animated as the
//! player is, but moved by what their game sends ([`m2_net::Pose`])
//! rather than by physics or input here.
//!
//! The gait comes over the wire as the other game picked it (walk and
//! sprint are chosen from the keys there, not from the speed), and the
//! clips play at the rate the speed sets, as the player's do.

use m2_engine::{Gait, GpuModel};
use m2_math::glam::{Mat4, Vec2, Vec3};
use m2_net::proto::HeroState;

use crate::hero::Hero;
use crate::npc::FightClips;
use crate::scene::{Carry, Rig};

pub struct Puppet {
    pub hero: Hero,
    /// The other player's name, as they gave it.
    pub name: String,
    pub body: Option<(GpuModel, Rig)>,
    pub clips: FightClips,
    /// Feet, as `Character::position`.
    pub position: Vec3,
    /// Radians clockwise from +z, as `Player::facing`.
    pub facing: f32,
    /// Whether a snapshot has placed it, and it is on foot.
    pub shown: bool,
}

impl std::fmt::Debug for Puppet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Puppet")
            .field("hero", &self.hero)
            .field("name", &self.name)
            .field("position", &self.position)
            .finish_non_exhaustive()
    }
}

impl Puppet {
    pub fn new(hero: Hero, name: String, body: Option<(GpuModel, Rig)>, clips: FightClips) -> Self {
        Self {
            hero,
            name,
            body,
            clips,
            position: Vec3::ZERO,
            facing: 0.0,
            shown: false,
        }
    }

    /// Puts it where `pose` says and plays its clips on by `dt`.
    pub fn step(&mut self, pose: &m2_net::Pose, dt: f32) {
        self.position = Vec3::from_array(pose.position);
        self.facing = pose.facing;
        self.shown = pose.flags & HeroState::DRIVING == 0;
        let Some((_, rig)) = &mut self.body else {
            return;
        };
        let gait = gait(pose.gait);
        let carry = if pose.flags & HeroState::AIMING != 0 {
            Carry::Aim
        } else {
            Carry::Low
        };
        rig.hold_rifle(gait, &self.clips, carry);
        rig.aim_along(self.facing, None);
        let moved = Vec2::new(pose.velocity[0], pose.velocity[2]).length();
        // As `Player::update` sets it: the clip's stride to the speed.
        let rate = match rig.gait_speed(gait) {
            Some(clip) if gait != Gait::Idle && clip > 0.1 => (moved / clip).clamp(0.6, 1.4),
            _ => 1.0,
        };
        rig.advance(dt * rate);
    }

    /// Where its model goes, as `Player::model_matrix` places the player.
    pub fn model_matrix(&self) -> Mat4 {
        let forward = self
            .body
            .as_ref()
            .and_then(|(_, rig)| rig.locomotion.as_ref())
            .map_or(Vec2::Y, |l| l.forward);
        Mat4::from_translation(self.position)
            * Mat4::from_rotation_y(self.facing - forward.x.atan2(forward.y))
    }
}

/// The engine's gait for the wire's.
pub fn gait(g: m2_net::Gait) -> Gait {
    match g {
        m2_net::Gait::Idle => Gait::Idle,
        m2_net::Gait::Walk => Gait::Walk,
        m2_net::Gait::Run => Gait::Run,
        m2_net::Gait::Sprint => Gait::Sprint,
    }
}

/// The wire's gait for the engine's.
pub fn wire_gait(g: Gait) -> m2_net::Gait {
    match g {
        Gait::Idle => m2_net::Gait::Idle,
        Gait::Walk => m2_net::Gait::Walk,
        Gait::Run => m2_net::Gait::Run,
        Gait::Sprint => m2_net::Gait::Sprint,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaits_go_over_the_wire_and_back() {
        for g in [Gait::Idle, Gait::Walk, Gait::Run, Gait::Sprint] {
            assert_eq!(gait(wire_gait(g)), g);
        }
    }

    #[test]
    fn a_pose_places_it_and_driving_hides_it() {
        let mut p = Puppet::new(Hero::Chris, "friend".into(), None, FightClips::default());
        let mut pose = m2_net::Pose {
            position: [1.0, 2.0, 3.0],
            velocity: [0.0; 3],
            facing: 0.5,
            gait: m2_net::Gait::Walk,
            flags: 0,
        };
        p.step(&pose, 0.1);
        assert_eq!(
            (p.position, p.facing, p.shown),
            (Vec3::new(1.0, 2.0, 3.0), 0.5, true)
        );
        pose.flags = HeroState::DRIVING;
        p.step(&pose, 0.1);
        assert!(!p.shown);
    }
}
