//! Gates and doors that swing open: a model's hinged leaves, from its
//! `PHY2` constraints ([`m2_assets::physics::Constraint`]), posed as far
//! open as asked.
//!
//! # Provenance
//!
//! From the 360 code:
//! - `Object.OpenGate` (0x82332DD8) and `CloseGate` (0x82332F18) take
//!   nothing down: for each of the object's joints (store 0x83657920)
//!   they set it open or shut and its motor's target, a hinge's open
//!   angle or 0 (type 4), a slide's open or shut place (type 7).
//! - The joints are the model's constraints (0x82160FF8, 0x82160EA8):
//!   a hinge opens from 0 to the angle its record gives, in degrees.
//!
//! From samples: the pivot and axis of a hinge (see the constraint's
//! notes); its leaf is the part the constraint holds, with every node
//! under it.
//!
//! - How fast a leaf swings ([`swing`]): when the joints are made
//!   (0x82160FF8), each gets a position motor (0x821617B8; Havok's
//!   position constraint motor, built by 0x8262D0D8 with the library's
//!   defaults tau 0.8, damping 1, recovery 2 and 1). A hinge's is given a
//!   maximum force of the larger of 5000 (0x820F8C8C) and its leaf's mass
//!   times 50 (0x820B7FB8), tau 0.6 (0x82B7D82C), damping 1, a
//!   proportional recovery velocity of 1 a second and a constant one of
//!   0.1 rad/s ([`RECOVERY`], [`RECOVERY_CONSTANT`]). OpenGate and
//!   CloseGate only move the target, so the leaf goes to it at the
//!   recovery velocity, `1 * error + 0.1` radians a second: a quarter
//!   turn takes about 2.8 s, slowing as it closes in.
//!
//! **Inferred**: that the motor holds the leaf to its recovery velocity
//! (the library's behaviour, its force limit not reached by a gate's
//! leaf, and tau's softening left out), stepped at 60 a second; and one
//! pace for all of a gate's leaves, its widest one's, where each of the
//! 360's has its own. Slides (7 of the 1,564 models) are not moved.

use m2_assets::physics::{Constraint, HINGE, Physics};
use m2_math::glam::{Mat4, Quat, Vec3};

use crate::anim::Skeleton;

/// The hinge motor's proportional recovery velocity, per second
/// (0x821617B8 sets 1.0 at +0x1C).
pub const RECOVERY: f32 = 1.0;
/// Its constant recovery velocity, radians a second (0.1 at +0x20,
/// 0x820BB468).
pub const RECOVERY_CONSTANT: f32 = 0.1;
/// The physics steps the motor's speed is taken over, seconds.
const STEP: f32 = 1.0 / 60.0;

/// How far open a leaf `open` radians wide is after `dt` seconds, from
/// `at` (0 shut, 1 open) going to `to`: at the motor's recovery velocity,
/// `RECOVERY * error + RECOVERY_CONSTANT`, step by step, not past it.
pub fn swing(at: f32, to: f32, open: f32, dt: f32) -> f32 {
    let open = open.abs().max(1e-3);
    let mut at = at;
    let mut left = dt;
    while left > 0.0 && at != to {
        let h = left.min(STEP);
        left -= h;
        let error = (to - at).abs() * open;
        let step = ((RECOVERY * error + RECOVERY_CONSTANT) * h).min(error) / open;
        at = if at < to {
            (at + step).min(to)
        } else {
            (at - step).max(to)
        };
    }
    at
}

/// One hinged leaf.
#[derive(Clone, Debug, PartialEq)]
pub struct Leaf {
    /// The nodes that turn with it: its part's node and those under it.
    pub nodes: Vec<usize>,
    pub pivot: Vec3,
    pub axis: Vec3,
    /// Its open angle, radians.
    pub open: f32,
}

/// A model's hinged leaves and its nodes at rest.
#[derive(Clone, Debug, PartialEq)]
pub struct Gate {
    pub leaves: Vec<Leaf>,
    rest: Vec<Mat4>,
}

impl Gate {
    /// The hinged leaves of a model of nodes `skeleton` and collision
    /// `phy2`; `None` without a hinge whose leaf is one of its nodes.
    pub fn of(skeleton: &Skeleton, phy2: &Physics) -> Option<Self> {
        let leaves: Vec<Leaf> = phy2
            .constraints
            .iter()
            .filter(|c| c.kind == HINGE)
            .filter_map(|c| leaf(skeleton, phy2, c))
            .collect();
        (!leaves.is_empty()).then(|| Self {
            leaves,
            rest: skeleton.model_matrices(&skeleton.rest_pose()),
        })
    }

    /// Every node's matrix in the model, its leaves `open` of the way
    /// open (0 shut, 1 open).
    pub fn nodes(&self, open: f32) -> Vec<Mat4> {
        self.posed(open)
    }

    /// Its widest leaf's open angle, radians: the gate's pace
    /// ([`swing`]).
    pub fn widest(&self) -> f32 {
        self.leaves.iter().map(|l| l.open.abs()).fold(0.0, f32::max)
    }

    fn posed(&self, open: f32) -> Vec<Mat4> {
        let mut out = self.rest.clone();
        let open = open.clamp(0.0, 1.0);
        for l in &self.leaves {
            let turn = Mat4::from_translation(l.pivot)
                * Mat4::from_quat(Quat::from_axis_angle(l.axis, l.open * open))
                * Mat4::from_translation(-l.pivot);
            for &n in &l.nodes {
                if let Some(m) = out.get_mut(n) {
                    *m = turn * *m;
                }
            }
        }
        out
    }
}

/// The leaf constraint `c` holds: its part's node and every node under
/// it, if the part names a node of `skeleton`.
fn leaf(skeleton: &Skeleton, phy2: &Physics, c: &Constraint) -> Option<Leaf> {
    let part = phy2.parts.get(c.part)?;
    let top = usize::try_from(part.node)
        .ok()
        .filter(|&n| skeleton.name_hashes.get(n) == Some(&part.node_hash))?;
    let axis = Vec3::from(c.axis).try_normalize()?;
    let under = |mut n: usize| loop {
        if n == top {
            return true;
        }
        match skeleton.parents.get(n).copied().flatten() {
            Some(p) => n = p,
            None => return false,
        }
    };
    Some(Leaf {
        nodes: (0..skeleton.name_hashes.len())
            .filter(|&n| under(n))
            .collect(),
        pivot: Vec3::from(c.pivot),
        axis,
        open: c.open.to_radians(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leaf_swings_at_its_motors_recovery_velocity() {
        let quarter = std::f32::consts::FRAC_PI_2;
        // A second from shut: dθ/dt = θ_left + 0.1 from 1.571, so about
        // (1.671)(1 - e^-1) = 1.056 rad, two thirds of the way.
        let at = swing(0.0, 1.0, quarter, 1.0);
        assert!((at * quarter - 1.056).abs() < 0.02, "{}", at * quarter);
        // The first step goes 1.671 rad/s, the last ones 0.1.
        let first = swing(0.0, 1.0, quarter, STEP);
        assert!((first * quarter / STEP - 1.671).abs() < 1e-3);
        // All the way in ln(1.671 / 0.1) = 2.8 s, never past.
        assert!(swing(0.0, 1.0, quarter, 2.7) < 1.0);
        assert_eq!(swing(0.0, 1.0, quarter, 2.9), 1.0);
        // Shutting, the same back.
        assert!((swing(1.0, 0.0, quarter, 1.0) - (1.0 - at)).abs() < 1e-4);
    }
    use m2_assets::physics::Part;

    fn skeleton() -> Skeleton {
        // Root, a leaf hinged 6 m left of it, and a node under the leaf.
        Skeleton {
            name_hashes: vec![1, 2, 3],
            parents: vec![None, Some(0), Some(1)],
            rest: [
                Vec3::ZERO,
                Vec3::new(-6.0, 0.0, 0.0),
                Vec3::new(3.0, 1.0, 0.0),
            ]
            .map(|translation| crate::anim::Local {
                translation,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            })
            .to_vec(),
            inverse_bind: vec![Mat4::IDENTITY; 3],
        }
    }

    fn physics(axis: [f32; 3]) -> Physics {
        let part = |node: u32, node_hash: u32| Part {
            position: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            node_hash,
            node,
            body: None,
            shape: None,
            unk_link: 0xFFFF_FFFF,
            parent: None,
            first_child: None,
            next_sibling: None,
        };
        Physics {
            version: 0x39,
            model: 0,
            unk_0x14: [1, 0, 0],
            parts: vec![part(0, 1), part(1, 2)],
            bodies: Vec::new(),
            shapes: Vec::new(),
            constraints: vec![Constraint {
                owner: 0,
                part: 1,
                kind: HINGE,
                pivot: [-6.0, 0.9, 0.0],
                open: 90.0,
                limit: 180.0,
                axis,
                slide: (0.0, 0.0),
            }],
        }
    }

    /// The villa gate's left leaf (`_pmcoutpost_gate`): hinged on its
    /// post about the down axis, 90 degrees open, it swings the node
    /// under it (3 m along it) a quarter turn out to +z about the post;
    /// half open, an eighth; the root stays.
    #[test]
    fn a_leaf_swings_about_its_post_by_its_open_angle() {
        let g = Gate::of(&skeleton(), &physics([0.0, -1.0, 0.0])).expect("a hinge");
        assert_eq!(g.leaves[0].nodes, [1, 2]);
        let at = |open: f32, n: usize| g.nodes(open)[n].w_axis.truncate();
        assert!(at(0.0, 2).distance(Vec3::new(-3.0, 1.0, 0.0)) < 1e-5);
        assert!(
            at(1.0, 2).distance(Vec3::new(-6.0, 1.0, 3.0)) < 1e-5,
            "{}",
            at(1.0, 2)
        );
        let half = at(0.5, 2) - Vec3::new(-6.0, 1.0, 0.0);
        assert!((half.x - half.z).abs() < 1e-5 && half.z > 0.0, "{half}");
        assert!(at(1.0, 0).length() < 1e-6);
        // The post itself turns in place.
        assert!(at(1.0, 1).distance(Vec3::new(-6.0, 0.0, 0.0)) < 1e-5);
        // Its mirror on the other post (axis up) also swings to +z.
        assert!(Gate::of(&skeleton(), &physics([0.0, 1.0, 0.0])).is_some());
        // No hinge, no gate.
        let mut none = physics([0.0, 1.0, 0.0]);
        none.constraints[0].kind = 7;
        assert!(Gate::of(&skeleton(), &none).is_none());
    }
}
