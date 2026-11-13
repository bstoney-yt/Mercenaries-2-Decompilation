//! A face rig's graph: from the values of a spoken line's curves at a
//! moment to where each face bone is ([`m2_assets::facefx`]).
//!
//! A rig's nodes are poses (each takes a few face bones from their rest
//! to a target) and combiners (values that feed other nodes, such as
//! `Blink`, which drives `Blink Left` and `Blink Right`). An animation
//! gives some nodes a value at each moment; a node's value is its own
//! curve's plus what its inputs feed it, each through its link
//! (`linear`: slope times value plus offset), kept between the node's
//! least and most. Each pose then moves its bones that far from rest
//! toward its target, the poses' moves adding up.
//!
//! The face bones are nodes of the head models' skeletons by the same
//! names (`bone_jaw` is a node of every head looked at), so the result
//! is a local transform for each of those nodes.
//!
//! # Provenance
//!
//! From samples: the rigs' graphs and the animations' curves as
//! [`m2_assets::facefx`] reads them; a head model's skeleton holding the
//! rig's bone names.
//!
//! **Inferred** (the FaceFX library's own evaluation in the 360 program
//! was not read): that a node's inputs add up (each node keeps a small
//! number, 0, 1, 2 or 4, which may say how its inputs combine; the only
//! node with two inputs, `Head Roll`, has 0); that a node with neither a
//! curve nor inputs is 0; that the poses' moves add up from rest
//! (positions and scales by the difference, rotations by the turn from
//! rest to the target, a share of it by the value); that a value beyond
//! 0 to 1 scales the move the same way.

use std::collections::HashMap;

use m2_assets::facefx::{Actor, Animation, BONE_POSE_NODE, Transform};
use m2_assets::name_hash;
use m2_math::glam::{Quat, Vec3};

/// A face bone's place relative to its parent: position, rotation and
/// scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BonePose {
    /// The bone's name hash (as the skeleton's nodes are named).
    pub bone: u32,
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

fn parts(t: &Transform) -> (Vec3, Quat, Vec3) {
    (
        Vec3::from(t.position),
        Quat::from_array(t.rotation).normalize(),
        Vec3::from(t.scale),
    )
}

/// A rig made ready to evaluate: its nodes in an order where every
/// node comes after those that feed it.
#[derive(Clone, Debug)]
pub struct FaceGraph {
    /// Per node: its name, least and most, inputs (index of the feeding
    /// node, slope and offset), and for a pose its bones (index into
    /// `bones`, target).
    nodes: Vec<Node>,
    by_name: HashMap<String, usize>,
    /// The rig's bones: name hash and rest.
    bones: Vec<(u32, Vec3, Quat, Vec3)>,
}

#[derive(Clone, Debug)]
struct Node {
    least: f32,
    most: f32,
    inputs: Vec<(usize, f32, f32)>,
    targets: Vec<(usize, Vec3, Quat, Vec3)>,
}

impl FaceGraph {
    /// The graph of `actor`. Inputs from nodes the rig does not have,
    /// and pose bones it has no rest for, are left out; a loop of inputs
    /// is broken where it closes.
    pub fn new(actor: &Actor) -> Self {
        let bones: Vec<(u32, Vec3, Quat, Vec3)> = actor
            .bones
            .iter()
            .map(|b| {
                let (p, r, s) = parts(&b.rest);
                (name_hash(&b.name), p, r, s)
            })
            .collect();
        let bone_index: HashMap<&str, usize> = actor
            .bones
            .iter()
            .enumerate()
            .map(|(k, b)| (b.name.as_str(), k))
            .collect();
        let index: HashMap<&str, usize> = actor
            .poses
            .iter()
            .enumerate()
            .map(|(k, p)| (p.name.as_str(), k))
            .collect();
        // Order: each node after its inputs (depth first).
        let mut order = Vec::with_capacity(actor.poses.len());
        let mut state = vec![0u8; actor.poses.len()];
        fn visit(
            k: usize,
            actor: &Actor,
            index: &HashMap<&str, usize>,
            state: &mut [u8],
            order: &mut Vec<usize>,
        ) {
            if state[k] != 0 {
                return;
            }
            state[k] = 1;
            for input in &actor.poses[k].inputs {
                if let Some(&j) = index.get(input.from.as_str()) {
                    visit(j, actor, index, state, order);
                }
            }
            state[k] = 2;
            order.push(k);
        }
        for k in 0..actor.poses.len() {
            visit(k, actor, &index, &mut state, &mut order);
        }
        let place: HashMap<usize, usize> = order.iter().enumerate().map(|(i, &k)| (k, i)).collect();
        let nodes = order
            .iter()
            .map(|&k| {
                let p = &actor.poses[k];
                Node {
                    least: p.least,
                    most: p.most,
                    inputs: p
                        .inputs
                        .iter()
                        .filter_map(|i| {
                            let from = place[index.get(i.from.as_str())?];
                            // Only from nodes already evaluated: a loop
                            // is broken here.
                            (from < place[&k]).then(|| {
                                let (m, b) = match i.parameters.as_slice() {
                                    [m, b, ..] if i.function == "linear" => (*m, *b),
                                    _ => (1.0, 0.0),
                                };
                                (from, m, b)
                            })
                        })
                        .collect(),
                    targets: if p.class == BONE_POSE_NODE {
                        p.bones
                            .iter()
                            .filter_map(|(name, t)| {
                                let (pos, rot, scale) = parts(t);
                                Some((*bone_index.get(name.as_str())?, pos, rot, scale))
                            })
                            .collect()
                    } else {
                        Vec::new()
                    },
                }
            })
            .collect();
        let by_name = order
            .iter()
            .enumerate()
            .map(|(i, &k)| (actor.poses[k].name.clone(), i))
            .collect();
        Self {
            nodes,
            by_name,
            bones,
        }
    }

    /// Each node's value, in the graph's order, given the values its own
    /// curves have (`own`, by node name; others are 0).
    fn values(&self, own: &[(&str, f32)]) -> Vec<f32> {
        let mut v = vec![0.0f32; self.nodes.len()];
        for (name, value) in own {
            if let Some(&i) = self.by_name.get(*name) {
                v[i] += value;
            }
        }
        for (i, node) in self.nodes.iter().enumerate() {
            let fed: f32 = node.inputs.iter().map(|&(j, m, b)| m * v[j] + b).sum();
            v[i] = (v[i] + fed).clamp(node.least, node.most);
        }
        v
    }

    /// The value of node `name` given `own` (for checking).
    pub fn value(&self, own: &[(&str, f32)], name: &str) -> Option<f32> {
        let i = *self.by_name.get(name)?;
        Some(self.values(own)[i])
    }

    /// Where each of the rig's bones is, given the nodes' own values
    /// `own` (by name; a line's curves at a moment).
    pub fn evaluate(&self, own: &[(&str, f32)]) -> Vec<BonePose> {
        let v = self.values(own);
        let mut out: Vec<BonePose> = self
            .bones
            .iter()
            .map(|&(bone, position, rotation, scale)| BonePose {
                bone,
                position,
                rotation,
                scale,
            })
            .collect();
        for (node, &value) in self.nodes.iter().zip(&v) {
            if value == 0.0 {
                continue;
            }
            for &(b, pos, rot, scale) in &node.targets {
                let (_, rp, rr, rs) = self.bones[b];
                let o = &mut out[b];
                o.position += (pos - rp) * value;
                o.scale += (scale - rs) * value;
                let turn = rot * rr.inverse();
                let share = Quat::IDENTITY.slerp(turn, value.clamp(0.0, 1.0));
                o.rotation = (share * o.rotation).normalize();
            }
        }
        out
    }

    /// [`evaluate`](Self::evaluate) at `seconds` into `animation`.
    pub fn at(&self, animation: &Animation, seconds: f32) -> Vec<BonePose> {
        let own: Vec<(&str, f32)> = animation
            .curves
            .iter()
            .filter(|c| !c.keys.is_empty())
            .map(|c| (c.name.as_str(), c.value(seconds)))
            .collect();
        self.evaluate(&own)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_assets::facefx::{Bone, COMBINER_NODE, Curve, Input, Key, Pose};

    fn t(p: [f32; 3], r: [f32; 4]) -> Transform {
        Transform {
            position: p,
            rotation: r,
            scale: [1.0; 3],
        }
    }

    fn pose(name: &str, bones: Vec<(String, Transform)>, inputs: Vec<Input>) -> Pose {
        Pose {
            name: name.to_owned(),
            class: if bones.is_empty() {
                COMBINER_NODE.to_owned()
            } else {
                BONE_POSE_NODE.to_owned()
            },
            least: 0.0,
            most: 1.0,
            inputs,
            bones,
        }
    }

    fn linear(from: &str, m: f32, b: f32) -> Input {
        Input {
            from: from.to_owned(),
            function: "linear".to_owned(),
            parameters: vec![m, b],
        }
    }

    /// A jaw bone and an eyelid; `Oat` drops the jaw 2 cm, the lids'
    /// pose shuts the lid a quarter turn, and `Blink` feeds the lid's
    /// pose. The inputs come before the nodes they feed in the list's
    /// opposite order, so the graph must order them.
    fn actor() -> Actor {
        let quarter = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2).to_array();
        Actor {
            name: "Rig".to_owned(),
            bones: vec![
                Bone {
                    name: "bone_jaw".to_owned(),
                    rest: t([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]),
                    index: 0,
                    weight: 1.0,
                },
                Bone {
                    name: "bone_eyelid_top_left".to_owned(),
                    rest: t([0.0, 0.1, 0.0], [0.0, 0.0, 0.0, 1.0]),
                    index: 1,
                    weight: 1.0,
                },
            ],
            poses: vec![
                pose(
                    "Blink Left",
                    vec![(
                        "bone_eyelid_top_left".to_owned(),
                        t([0.0, 0.1, 0.0], quarter),
                    )],
                    vec![linear("Blink", 1.0, 0.0)],
                ),
                pose("Blink", Vec::new(), Vec::new()),
                pose(
                    "Oat",
                    vec![(
                        "bone_jaw".to_owned(),
                        t([0.0, -0.02, 0.0], [0.0, 0.0, 0.0, 1.0]),
                    )],
                    Vec::new(),
                ),
            ],
        }
    }

    #[test]
    fn at_rest_with_nothing_on() {
        let g = FaceGraph::new(&actor());
        let out = g.evaluate(&[]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].bone, name_hash("bone_jaw"));
        assert_eq!(out[0].position, Vec3::ZERO);
        assert_eq!(out[1].rotation, Quat::IDENTITY);
    }

    #[test]
    fn a_pose_moves_its_bones_by_its_value() {
        let g = FaceGraph::new(&actor());
        let out = g.evaluate(&[("Oat", 0.5)]);
        assert!((out[0].position.y + 0.01).abs() < 1e-6);
        let out = g.evaluate(&[("Oat", 3.0)]);
        // Kept to its most, 1.
        assert!((out[0].position.y + 0.02).abs() < 1e-6);
    }

    #[test]
    fn a_combiner_feeds_its_pose_through_the_link() {
        let g = FaceGraph::new(&actor());
        assert_eq!(g.value(&[("Blink", 1.0)], "Blink Left"), Some(1.0));
        let out = g.evaluate(&[("Blink", 0.5)]);
        let (axis, angle) = out[1].rotation.to_axis_angle();
        assert!(
            (angle - std::f32::consts::FRAC_PI_4).abs() < 1e-4,
            "{angle}"
        );
        assert!((axis - Vec3::X).length() < 1e-4);
        // Its own value adds to what it is fed, kept to 0..1.
        assert_eq!(
            g.value(&[("Blink", 0.75), ("Blink Left", 0.5)], "Blink Left"),
            Some(1.0)
        );
    }

    #[test]
    fn a_lines_curves_drive_it_over_time() {
        let g = FaceGraph::new(&actor());
        let line = Animation {
            name: "Line".to_owned(),
            curves: vec![Curve {
                name: "Oat".to_owned(),
                keys: vec![
                    Key {
                        seconds: 0.0,
                        value: 0.0,
                        slope_in: 0.0,
                        slope_out: 0.0,
                    },
                    Key {
                        seconds: 1.0,
                        value: 1.0,
                        slope_in: 0.0,
                        slope_out: 0.0,
                    },
                ],
            }],
            blend_in: 0.16,
            blend_out: 0.22,
        };
        assert_eq!(g.at(&line, 0.0)[0].position.y, 0.0);
        assert!((g.at(&line, 0.5)[0].position.y + 0.01).abs() < 1e-6);
        assert!((g.at(&line, 2.0)[0].position.y + 0.02).abs() < 1e-6);
    }

    #[test]
    fn a_loop_of_inputs_does_not_hang() {
        let mut a = actor();
        a.poses[1].inputs.push(linear("Blink Left", 1.0, 0.0));
        let g = FaceGraph::new(&a);
        assert!(g.value(&[("Blink", 0.5)], "Blink Left").is_some());
    }
}
