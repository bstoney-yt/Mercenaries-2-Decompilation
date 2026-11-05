//! The hand IK the 360's locomotion node runs after its aim mix
//! (`PostIK`): the left hand kept on the rifle where the aim sweep holds
//! it from the right.
//!
//! # From the 360 code
//!
//! The locomotion node (0x82802B10) builds an IK node at +0x1230
//! (0x82810C10, vtable 0x82071A18) and its init (0x82804AE8) sets it up
//! (0x82810D90) with flags 0x24, or 0 when the character's weapon class
//! is the pistol's (0xD48C7D34) or 0xD20EC345. The setup finds the arms'
//! bones by name hash: for each arm the upper arm, forearm and hand
//! ([`LEFT`], [`RIGHT`]), a forearm twist helper, and a hinge axis of
//! (0, 0, 1) in the forearm's space, no hinge limit (cosines -1 and 1) and
//! gains of 1. Its pose step (0x82810058) takes two inputs: the mix (A)
//! and the aim sweep held unmixed (B; wired at 0x8280AE60). With flag 4
//! it takes the left hand's place relative to the right hand from B,
//! sets it on A's right hand, solves the left arm to reach it with
//! Havok's two-joint solver (0x825D17B0, profiled as
//! `LtTWOJOINTSOLVER`) and sets the hand's turn to the target's. Flags
//! 1, 2 (each hand to B's), 8 and 0x10 (to the weapon's grip markers
//! e68bcd4d, a99f98b3) and 0x40 (a root offset) are not set by the
//! locomotion node and are not ported here.
//!
//! Not ported: flag 0x20, which turns the forearm's twist helper by 0.666
//! (0x82071A10) of the hand's twist (vector code at 0x825D9B38 not
//! decoded). The solver itself is vector code; it is written here as the
//! plain two-joint solution it is named for: the elbow bent about the
//! hinge axis till the shoulder-to-hand distance is the target's, then
//! the upper arm swung onto the target (inferred: that order, Havok's
//! documented one).

use m2_math::glam::{Mat4, Quat, Vec3};

use crate::anim::{Pose, Skeleton};

/// The left arm's bones, upper arm, forearm and hand (name hashes,
/// from the 360 code: 0x82810D90's lookups of 0xB2C9CE63, 0xBEFC09A2
/// and 0xEB6F1B2D). The forearm's names solved as `Bone_LForearm` and
/// `Bone_LHand`.
pub const LEFT: [u32; 3] = [0xb2c9_ce63, 0xbefc_09a2, 0xeb6f_1b2d];
/// The right arm's (0x20F635D9, 0x23F6F598, 0xD34F7713: `Bone_RForearm`,
/// `Bone_RHand`).
pub const RIGHT: [u32; 3] = [0x20f6_35d9, 0x23f6_f598, 0xd34f_7713];
/// The forearm twist helpers (0x03719EEF left, 0x328C1F25 right).
pub const TWIST: [u32; 2] = [0x0371_9eef, 0x328c_1f25];
/// The hinge axis, in the forearm's space (0x82810D90: +0x70, +0xB0).
pub const HINGE: Vec3 = Vec3::Z;
/// The flags the locomotion node sets up the IK with (0x82804AE8):
/// the left hand to the right (4) and the forearm twist (0x20).
pub const LEFT_TO_RIGHT: u8 = 0x04;
pub const FOREARM_TWIST: u8 = 0x20;
pub const LOCOMOTION_FLAGS: u8 = LEFT_TO_RIGHT | FOREARM_TWIST;

/// The node indices of an arm's bones in `skeleton`.
pub fn arm(skeleton: &Skeleton, bones: [u32; 3]) -> Option<[usize; 3]> {
    let find = |h: u32| skeleton.name_hashes.iter().position(|&n| n == h);
    Some([find(bones[0])?, find(bones[1])?, find(bones[2])?])
}

/// Flag 4: keeps `pose`'s left hand where `reference` holds it from the
/// right hand (the right hand's place in `pose` times the left's from the
/// right in `reference`), solving the left arm to reach it. Returns
/// whether it ran (the skeleton has both arms).
pub fn left_to_right(skeleton: &Skeleton, pose: &mut Pose, reference: &Pose) -> bool {
    let (Some(left), Some(right)) = (arm(skeleton, LEFT), arm(skeleton, RIGHT)) else {
        return false;
    };
    let a = skeleton.model_matrices(pose);
    let b = skeleton.model_matrices(reference);
    let (lh, rh) = (left[2], right[2]);
    let target = a[rh] * b[rh].inverse() * b[lh];
    two_joint(skeleton, pose, left, target, HINGE);
    true
}

/// Rotation and translation of a model-space matrix (scale dropped).
fn parts(m: Mat4) -> (Quat, Vec3) {
    let (_, r, t) = m.to_scale_rotation_translation();
    let r = if r.is_finite() {
        r.normalize()
    } else {
        Quat::IDENTITY
    };
    (r, t)
}

/// Sets node `i`'s local rotation so that its model-space rotation is
/// `world`, its parent's model matrices being `m`.
fn set_world_rotation(skeleton: &Skeleton, pose: &mut Pose, m: &[Mat4], i: usize, world: Quat) {
    let parent = skeleton.parents[i].map_or(Quat::IDENTITY, |p| parts(m[p]).0);
    if let Some(local) = pose.0.get_mut(i) {
        local.rotation = (parent.inverse() * world).normalize();
    }
}

/// Two-joint IK: `chain` the upper arm, forearm and hand; `target` the
/// hand's model-space place and turn. The forearm bends about `hinge`
/// (its own space) till the shoulder-to-hand distance is the target's
/// (within reach), the upper arm swings the hand onto the target, and the
/// hand takes the target's turn.
pub fn two_joint(
    skeleton: &Skeleton,
    pose: &mut Pose,
    chain: [usize; 3],
    target: Mat4,
    hinge: Vec3,
) {
    let [up, fore, hand] = chain;
    let (target_rot, target_at) = parts(target);
    // 1. The elbow.
    let m = skeleton.model_matrices(pose);
    let (a, b, c) = (parts(m[up]).1, parts(m[fore]).1, parts(m[hand]).1);
    let (l1, l2) = ((b - a).length(), (c - b).length());
    if l1 > 1e-5 && l2 > 1e-5 {
        let axis = (parts(m[fore]).0 * hinge).normalize_or_zero();
        let reach = (target_at - a)
            .length()
            .clamp((l1 - l2).abs() + 1e-4, l1 + l2 - 1e-4);
        // The elbow's inner angle for that reach, and as it is now, both
        // measured about the hinge.
        let want = ((l1 * l1 + l2 * l2 - reach * reach) / (2.0 * l1 * l2))
            .clamp(-1.0, 1.0)
            .acos();
        let flat = |v: Vec3| (v - axis * v.dot(axis)).normalize_or_zero();
        let (u, v) = (flat(a - b), flat(c - b));
        if axis != Vec3::ZERO && u != Vec3::ZERO && v != Vec3::ZERO {
            // Signed angle from the forearm to the upper arm about the
            // hinge; bending keeps its side.
            let now = v.cross(u).dot(axis).atan2(v.dot(u));
            let to = want.copysign(now);
            let turn = Quat::from_axis_angle(axis, now - to);
            let fore_rot = parts(m[fore]).0;
            set_world_rotation(skeleton, pose, &m, fore, turn * fore_rot);
        }
    }
    // 2. The upper arm onto the target.
    let m = skeleton.model_matrices(pose);
    let (a, c) = (parts(m[up]).1, parts(m[hand]).1);
    let (from, to) = (
        (c - a).normalize_or_zero(),
        (target_at - a).normalize_or_zero(),
    );
    if from != Vec3::ZERO && to != Vec3::ZERO {
        let swing = Quat::from_rotation_arc(from, to);
        let up_rot = parts(m[up]).0;
        set_world_rotation(skeleton, pose, &m, up, swing * up_rot);
    }
    // 3. The hand's turn.
    let m = skeleton.model_matrices(pose);
    set_world_rotation(skeleton, pose, &m, hand, target_rot);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::Local;

    /// A shoulder at the origin, upper arm 0.3 m down +X, forearm 0.25 m
    /// further, then the hand; the elbow bent a little about Z.
    fn arm_skeleton() -> Skeleton {
        let local = |t: Vec3, r: Quat| Local {
            translation: t,
            rotation: r,
            scale: Vec3::ONE,
        };
        Skeleton {
            name_hashes: vec![LEFT[0], LEFT[1], LEFT[2]],
            parents: vec![None, Some(0), Some(1)],
            rest: vec![
                local(Vec3::ZERO, Quat::IDENTITY),
                local(Vec3::X * 0.3, Quat::from_rotation_z(0.3)),
                local(Vec3::X * 0.25, Quat::IDENTITY),
            ],
            inverse_bind: vec![Mat4::IDENTITY; 3],
        }
    }

    #[test]
    fn the_hand_reaches_a_target_within_reach_and_takes_its_turn() {
        let s = arm_skeleton();
        let mut pose = s.rest_pose();
        let turn = Quat::from_rotation_y(0.7);
        let at = Vec3::new(0.2, 0.3, 0.1);
        two_joint(
            &s,
            &mut pose,
            [0, 1, 2],
            Mat4::from_rotation_translation(turn, at),
            HINGE,
        );
        let m = s.model_matrices(&pose);
        let (r, t) = parts(m[2]);
        assert!(t.distance(at) < 1e-3, "{t}");
        assert!(r.angle_between(turn) < 1e-3);
        // The bones keep their lengths.
        assert!((parts(m[1]).1.length() - 0.3).abs() < 1e-4);
        assert!((t.distance(parts(m[1]).1) - 0.25).abs() < 1e-4);
    }

    #[test]
    fn the_left_hand_keeps_the_references_place_from_the_right() {
        let local = |t: Vec3, r: Quat| Local {
            translation: t,
            rotation: r,
            scale: Vec3::ONE,
        };
        let bend = Quat::from_rotation_z(0.4);
        // A chest, then the left and right arms out to either side.
        let s = Skeleton {
            name_hashes: vec![0, LEFT[0], LEFT[1], LEFT[2], RIGHT[0], RIGHT[1], RIGHT[2]],
            parents: vec![None, Some(0), Some(1), Some(2), Some(0), Some(4), Some(5)],
            rest: vec![
                local(Vec3::Y * 1.4, Quat::IDENTITY),
                local(Vec3::X * 0.2, Quat::IDENTITY),
                local(Vec3::X * 0.3, bend),
                local(Vec3::X * 0.25, Quat::IDENTITY),
                local(-Vec3::X * 0.2, Quat::IDENTITY),
                local(-Vec3::X * 0.3, bend),
                local(-Vec3::X * 0.25, Quat::IDENTITY),
            ],
            inverse_bind: vec![Mat4::IDENTITY; 7],
        };
        let reference = s.rest_pose();
        // The mix turned both arms a little apart.
        let mut pose = s.rest_pose();
        pose.0[1].rotation = Quat::from_rotation_y(0.3);
        pose.0[4].rotation = Quat::from_rotation_x(-0.2);
        assert!(left_to_right(&s, &mut pose, &reference));
        let (a, b) = (s.model_matrices(&pose), s.model_matrices(&reference));
        let rel = |m: &[Mat4]| m[6].inverse() * m[3];
        let (got, want) = (rel(&a), rel(&b));
        let (gr, gt) = parts(got);
        let (wr, wt) = parts(want);
        assert!(gt.distance(wt) < 1e-3, "{gt} {wt}");
        assert!(gr.angle_between(wr) < 1e-3);
    }

    #[test]
    fn out_of_reach_the_arm_points_straight_at_it() {
        let s = arm_skeleton();
        let mut pose = s.rest_pose();
        let at = Vec3::new(0.0, 2.0, 0.0);
        two_joint(&s, &mut pose, [0, 1, 2], Mat4::from_translation(at), HINGE);
        let hand = parts(s.model_matrices(&pose)[2]).1;
        assert!((hand.length() - 0.55).abs() < 1e-3);
        assert!(hand.normalize().dot(Vec3::Y) > 0.999);
    }
}
