//! The human ragdoll: the one model asset every person's ragdoll is built
//! from, and what the game does with it.
//!
//! # The asset
//!
//! A model of name hash [`RAGDOLL`] (type
//! [`MODEL_TYPE`](crate::model::MODEL_TYPE), in `vz.wad`'s `resident`
//! block) whose `PHY2` section ([`crate::physics`]) holds the bodies: in
//! the game's, 13 nodes (a root and the twelve `Bone_` nodes the bodies
//! sit on, with the names below), 12 bodies of 121.2 kg in all, a capsule
//! on each but the `Bone_Spine2` stand-in (0.003 kg, a 2 cm box), and 11
//! joints between them: the knees and elbows hinges limited to 110
//! degrees one way, the hips, shoulders, neck and the spine's two links
//! ragdoll joints with twist, cone and plane limits. [`HumanRagdoll`]
//! reads it into bones and joints.
//!
//! # What the game does with it (from the 360 code)
//!
//! - **Built**: every person with an animation driver (component pool
//!   0x83657C14) gets a ragdoll actor (`PhysicsActorRagdoll`, pool
//!   0x8365726C, class 0x820C8EC0 made by 0x8218F2D0) and a ragdoll
//!   controller (`RagdollController`, pool 0x8365849C, class 0x820EBF50),
//!   both in the physics set-up 0x82418E40. The actor's build (0x8218F980)
//!   asks for the asset [`RAGDOLL`] and the physics world's system of the
//!   same hash; the first person's bodies are kept (0x831334EC) and the
//!   rest copied from them.
//! - **Started** by the human state table ([`crate::human_states`]): the
//!   controller's "action changed" slot (vtable +0x34, 0x82428568) starts
//!   the ragdoll (0x82427828) when the new action's mode is `FULL` and
//!   ends it (0x824279F8) otherwise. `FULL`: `Die` and `Dead` in every
//!   stance on foot, `KnockDown`, `KnockedDown.Idle`, the melee knockdowns
//!   and falls. `ON_END` (`DieAnimatedRagdoll`, `Human.SetPreemptiveRagdoll`
//!   setting the driver's +0xC0): the driver (0x82421E98) starts it as the
//!   action's clip ends. How it starts depends on the controller's +0x20:
//!   0 with the velocity at +0x1B0, 1 thrown (0x82428EA0: 10 m/s straight
//!   away from the thrower at +0x1DC, level), 2 as it is.
//! - **Knocked down**: the message `KnockDown` (`Human.Knockdown`, a
//!   vehicle running into a person 0x82422480, objects striking them
//!   0x82179958, the AI 0x822D9C10) leads `Upright` to `KnockDown`, whose
//!   `Complete` leads to `KnockedDown.Idle`, both `FULL`. The seconds to
//!   stay down go to the person's state (pool 0x83657304) +0x38:
//!   0x82423848, a base (5 for a vehicle, 3 for an object) times 1 +
//!   (rand - 0.5) x 0.4, 0.5 for the player.
//! - **Up again**: once `KnockedDown.Idle` has lasted those seconds,
//!   0x8225CAE0 posts `Stand`, which leads to `KnockedDown.Getup` (mode
//!   `NONE`): the ragdoll ends (0x824279F8), the person stood where the
//!   ragdoll lies, faced along `Bone_Hips` to `Bone_Chest` (flipped when
//!   lying face up), and the get-up clip plays.
//! - **Pushed**: a blast's push (0x8215CA38) is kept by the controller
//!   for four frames and given to the bones only if the ragdoll starts in
//!   them (0x82428910), each bone by its own mass ([`RagdollBone::mass`]).
//!
//! Not traced: the human movement record's `TossGetupSeconds` (3; its
//! reader), what the actor's update does between start and end, and the
//! ragdoll joints' Havok settings beyond the limits read here.
//!
//! # Provenance
//!
//! From the 360 code as listed above, and the joints' fields from its
//! constraint builders: the limited hinge (0x8216A5D8) reads the pivot
//! (+0x0C), the axis (negated, +0x68), `open - limit` to `open` degrees
//! (+0x60, +0x64) when +0x5C is set, and friction (+0x58); the ragdoll
//! joint (0x8216AD30) the pivot, two axes (+0x40, +0x4C), six angles in
//! degrees (+0x28 to +0x3C) and friction. From samples: the asset's
//! contents above (its node names solved by hashing `Bone_` names). No
//! bytes from the archives are committed.

use crate::physics::{Constraint, Geometry, HINGE, Physics, RAGDOLL as RAGDOLL_JOINT};

/// The ragdoll model asset's name hash (in `vz.wad`'s `resident` block).
pub const RAGDOLL: u32 = 0xb9ba_c8b5;

/// The physics world system the 360 builds the ragdoll in: the same hash
/// ([`RAGDOLL`], 0x8218F980's lookup in the world's +0x388).
pub const RAGDOLL_SYSTEM: u32 = RAGDOLL;

/// The nodes the 360's own code looks bones up by when the ragdoll ends
/// (0x824279F8): the person is faced along from the first to the second.
pub const HIPS: u32 = 0x24c5_009c;
pub const CHEST: u32 = 0x4c77_33ed;

/// One rigid body of the ragdoll, on one of the model's nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct RagdollBone {
    /// The node it sits on: index and name hash (`Bone_Hips`...).
    pub node: u32,
    pub node_hash: u32,
    /// The bone it hangs from (the nearest body part above its own).
    pub parent: Option<usize>,
    /// Kilograms, and the centre of mass in the node's space.
    pub mass: f32,
    pub centre_of_mass: [f32; 3],
    pub friction: f32,
    pub angular_damping: f32,
    /// Its collision shape: a capsule of this radius from `a` to `b` in
    /// the node's space (a box's half extents for the spine's stand-in).
    pub shape: Option<BoneShape>,
}

/// A bone's collision shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BoneShape {
    Capsule {
        radius: f32,
        a: [f32; 3],
        b: [f32; 3],
    },
    Box {
        half_extents: [f32; 3],
    },
}

/// How a joint holds two bones.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JointLimits {
    /// A hinge about `axis`, turning from `min` to `max` degrees (a free
    /// hinge where `limited` is false).
    Hinge {
        axis: [f32; 3],
        min: f32,
        max: f32,
        limited: bool,
    },
    /// A ragdoll joint: its two axes and its twist, cone and plane
    /// limits, degrees ([`Constraint::twist`]...).
    Ragdoll {
        axes: [[f32; 3]; 2],
        twist: (f32, f32),
        cone: (f32, f32),
        plane: (f32, f32),
    },
}

/// A joint between two bones.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RagdollJoint {
    /// The bone whose part holds the joint, and the bone it holds to.
    pub bone: usize,
    pub other: usize,
    pub pivot: [f32; 3],
    pub friction: f32,
    pub limits: JointLimits,
}

/// The human ragdoll as its asset has it.
#[derive(Debug, Clone, PartialEq)]
pub struct HumanRagdoll {
    pub bones: Vec<RagdollBone>,
    pub joints: Vec<RagdollJoint>,
}

/// Why a `PHY2` section doesn't read as a ragdoll.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RagdollError {
    #[error("constraint {0} joins a part without a body")]
    Unjoined(usize),
    #[error("constraint {0} is of kind {1}, neither a hinge nor a ragdoll joint")]
    Kind(usize, u32),
}

impl HumanRagdoll {
    /// Its bones and joints from the asset's `PHY2` section: a bone per
    /// body, in body order.
    pub fn from_physics(p: &Physics) -> Result<Self, RagdollError> {
        // The part holding each body.
        let part_of = |body: usize| p.parts.iter().position(|q| q.body == Some(body));
        let body_above = |part: usize| {
            let mut at = p.parts.get(part).and_then(|q| q.parent);
            for _ in 0..=p.parts.len() {
                let q = p.parts.get(at?)?;
                if q.body.is_some() {
                    return q.body;
                }
                at = q.parent;
            }
            None
        };
        let bones = p
            .bodies
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let part = part_of(i);
                let shape = (0..p.shapes.len())
                    .find(|&s| p.body_of(s) == Some(i))
                    .and_then(|s| {
                        let s = &p.shapes[s];
                        match s.geometry {
                            Geometry::Capsule { a, b } => Some(BoneShape::Capsule {
                                radius: s.radius,
                                a,
                                b,
                            }),
                            Geometry::Box { half_extents } => Some(BoneShape::Box { half_extents }),
                            _ => None,
                        }
                    });
                RagdollBone {
                    node: part.map_or(0, |q| p.parts[q].node),
                    node_hash: part.map_or(0, |q| p.parts[q].node_hash),
                    parent: part.and_then(body_above),
                    mass: b.mass,
                    centre_of_mass: b.centre_of_mass,
                    friction: b.friction(),
                    angular_damping: b.angular_damping(),
                    shape,
                }
            })
            .collect();
        let joints = p
            .constraints
            .iter()
            .enumerate()
            .map(|(k, c)| joint(p, k, c))
            .collect::<Result<_, _>>()?;
        Ok(Self { bones, joints })
    }

    /// The whole body's mass, kilograms.
    pub fn mass(&self) -> f32 {
        self.bones.iter().map(|b| b.mass).sum()
    }

    /// The bone on the node of name hash `node`.
    pub fn bone(&self, node: u32) -> Option<usize> {
        self.bones.iter().position(|b| b.node_hash == node)
    }
}

fn joint(p: &Physics, k: usize, c: &Constraint) -> Result<RagdollJoint, RagdollError> {
    let body = |part: usize| p.parts.get(part).and_then(|q| q.body);
    let (Some(bone), Some(other)) = (body(c.owner), body(c.part)) else {
        return Err(RagdollError::Unjoined(k));
    };
    let limits = match c.kind {
        HINGE => {
            let (min, max) = c.hinge_range();
            // The builder turns about the axis negated (0x8216A5D8).
            JointLimits::Hinge {
                axis: c.axis.map(|v| -v),
                min,
                max,
                limited: c.limited,
            }
        }
        RAGDOLL_JOINT => JointLimits::Ragdoll {
            axes: c.ragdoll_axes,
            twist: c.twist(),
            cone: c.cone(),
            plane: c.plane(),
        },
        kind => return Err(RagdollError::Kind(k, kind)),
    };
    Ok(RagdollJoint {
        bone,
        other,
        pivot: c.pivot,
        friction: c.friction,
        limits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::name_hash;
    use crate::physics::{Body, Part, Shape};

    #[test]
    fn the_bones_are_named_as_the_game_hashes_them() {
        assert_eq!(name_hash("Bone_Hips"), HIPS);
        assert_eq!(name_hash("Bone_Chest"), CHEST);
        assert_eq!(name_hash("Bone_LShin"), 0xa76c_9842);
        assert_eq!(name_hash("Bone_RBicep"), 0x20f6_35d9);
    }

    fn part(node: u32, body: Option<usize>, shape: Option<usize>, parent: Option<usize>) -> Part {
        Part {
            position: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            node_hash: 0x100 + node,
            node,
            body,
            shape,
            unk_link: 0,
            parent,
            first_child: None,
            next_sibling: None,
        }
    }

    fn body(mass: f32) -> Body {
        Body {
            inertia: [f32::MAX; 9],
            centre_of_mass: [0.2, 0.0, 0.0],
            mass,
            unk: [0.3, 0.3, 0.3, 0.3],
            unk_words: [0; 8],
        }
    }

    fn constraint(owner: usize, part: usize, kind: u32) -> Constraint {
        Constraint {
            owner,
            part,
            kind,
            pivot: [0.0; 3],
            open: 0.0,
            limit: 110.0,
            axis: [0.0, 0.0, -1.0],
            slide: (0.0, -0.0),
            limited: true,
            friction: 0.0,
            ragdoll_axes: [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0]],
            ragdoll_angles: [-15.0, 15.0, -60.0, 20.0, -30.0, 30.0],
        }
    }

    /// A hip, a thigh hanging from it on a ragdoll joint and a shin from
    /// that on a knee, laid out as the asset lays its parts out: a body's
    /// part, then its shape's part under it.
    fn leg() -> Physics {
        Physics {
            version: crate::physics::VERSION,
            model: RAGDOLL,
            unk_0x14: [0; 3],
            parts: vec![
                part(1, Some(0), None, None),
                part(1, None, Some(0), Some(0)),
                part(2, Some(1), None, Some(0)),
                part(2, None, Some(1), Some(2)),
                part(3, Some(2), None, Some(2)),
                part(3, None, Some(2), Some(4)),
            ],
            bodies: vec![body(19.2), body(8.9), body(17.4)],
            shapes: [(1, 0.145), (3, 0.091), (5, 0.119)]
                .into_iter()
                .map(|(part, radius)| Shape {
                    radius,
                    geometry: Geometry::Capsule {
                        a: [0.0, 0.1, 0.0],
                        b: [0.0, -0.1, 0.0],
                    },
                    part,
                    record: [0; 26],
                })
                .collect(),
            constraints: vec![constraint(2, 0, RAGDOLL_JOINT), constraint(4, 2, HINGE)],
        }
    }

    #[test]
    fn a_leg_reads_as_three_bones_and_two_joints() {
        let r = HumanRagdoll::from_physics(&leg()).unwrap();
        assert_eq!(r.bones.len(), 3);
        assert_eq!(
            r.bones.iter().map(|b| b.parent).collect::<Vec<_>>(),
            [None, Some(0), Some(1)]
        );
        assert!((r.mass() - 45.5).abs() < 1e-4);
        assert_eq!(r.bone(0x103), Some(2));
        assert_eq!(
            r.bones[1].shape,
            Some(BoneShape::Capsule {
                radius: 0.091,
                a: [0.0, 0.1, 0.0],
                b: [0.0, -0.1, 0.0]
            })
        );
        // The hip joint holds the thigh (its part's body) to the hips.
        assert_eq!((r.joints[0].bone, r.joints[0].other), (1, 0));
        assert_eq!(
            r.joints[0].limits,
            JointLimits::Ragdoll {
                axes: [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0]],
                twist: (-15.0, 15.0),
                cone: (-60.0, 20.0),
                plane: (-30.0, 30.0),
            }
        );
        // The knee bends 110 degrees one way about its axis negated.
        assert_eq!(
            r.joints[1].limits,
            JointLimits::Hinge {
                axis: [-0.0, -0.0, 1.0],
                min: -110.0,
                max: 0.0,
                limited: true
            }
        );
    }

    #[test]
    fn a_joint_of_another_kind_or_without_a_body_is_refused() {
        let mut p = leg();
        p.constraints[1].kind = 7;
        assert_eq!(
            HumanRagdoll::from_physics(&p),
            Err(RagdollError::Kind(1, 7))
        );
        let mut p = leg();
        p.constraints[0].part = 1;
        assert_eq!(
            HumanRagdoll::from_physics(&p),
            Err(RagdollError::Unjoined(0))
        );
    }
}
