//! Props that move on their own: a template's `AnimationResponse` that
//! loops a clip over the model's nodes, as the searchlights on
//! `VzaCon001`'s island sweep their beams from side to side.
//!
//! `_global_spotlight` (model `global_prop_spotlight`) names clip
//! `68c0feb2` with `Loop` set: one track, on the yoke, turning it 50
//! degrees each way over 33 seconds. Its `Door` component names the lamp
//! head and an angle (60 degrees, 45 on two of the mission's lamps): the
//! head's hinge, tipped up by that angle so the beam (along the cone's
//! `-z`) points into the sky as it does in footage of the mission. Doors
//! of templates that don't loop a clip (doors, gates) are left shut: that
//! a `Door` angle stands for a resting tilt only on these is inferred.
//!
//! Field names are hashes: [`CLIP`], [`DOOR_NODE`] and [`DOOR_ANGLE`]
//! were told apart by their values on the spotlight's template and
//! entities, not by name.

use std::collections::HashMap;
use std::io::{Read, Seek};

use m2_assets::animation::ANIMATION_TYPE;
use m2_assets::layer::{Component, Field, Layer};
use m2_assets::{Clip, name_hash};
use m2_math::glam::{Mat4, Quat};

use crate::anim::{BoundClip, Skeleton};
use crate::assets::{AssetError, AssetSource};

/// `AnimationResponse`'s field naming the clip.
pub const CLIP: u32 = 0x1567_c45a;
/// `Door`'s field naming the hinged node.
pub const DOOR_NODE: u32 = 0x2cc9_3c3e;
/// `Door`'s angle, in degrees.
pub const DOOR_ANGLE: u32 = 0x1d5e_f36a;

/// How a placed prop moves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PropMotion {
    /// The clip played over and over.
    pub clip: Option<u32>,
    /// Nodes (by name hash) tipped about their x axis, in degrees.
    pub hinges: Vec<(u32, f32)>,
}

impl PropMotion {
    pub fn is_still(&self) -> bool {
        self.clip.is_none() && self.hinges.is_empty()
    }
}

/// The word of field `hash` in `rec`, its bits only when it is a bit field.
fn word(c: &Component<'_>, rec: &[u8], hash: u32) -> Option<u32> {
    let f: &Field = c.fields.iter().find(|f| f.name_hash == hash)?;
    let at = usize::from(f.offset);
    let b = rec.get(at..at + 4)?;
    let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    Some(match f.bit_width {
        0 | 32.. => v,
        w => (v >> f.bit_shift) & ((1 << w) - 1),
    })
}

/// The hinges `Door` records of `layer` give, by entity.
pub fn hinges(layer: &Layer<'_>) -> HashMap<u32, (u32, f32)> {
    let Some(c) = layer.component("Door") else {
        return HashMap::new();
    };
    c.records()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(id, rec)| {
            let node = word(c, rec, DOOR_NODE)?;
            let angle = f32::from_bits(word(c, rec, DOOR_ANGLE)?);
            angle.is_finite().then_some((id, (node, angle)))
        })
        .collect()
}

/// The motions of the template database `db`'s templates that loop a clip,
/// by template id.
pub fn read_templates(db: &Layer<'_>) -> HashMap<u32, PropMotion> {
    let Some(c) = db.component("AnimationResponse") else {
        return HashMap::new();
    };
    let doors = hinges(db);
    c.records()
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, rec)| word(c, rec, name_hash("Loop")).is_some_and(|l| l != 0))
        .filter_map(|(id, rec)| {
            let clip = word(c, rec, CLIP).filter(|&h| h != 0)?;
            Some((
                id,
                PropMotion {
                    clip: Some(clip),
                    hinges: doors.get(&id).copied().into_iter().collect(),
                },
            ))
        })
        .collect()
}

/// A model and its clip, ready to pose.
#[derive(Debug)]
pub struct Posed {
    skeleton: Skeleton,
    clip: Option<BoundClip>,
    /// Node index and angle in radians.
    hinges: Vec<(usize, f32)>,
}

impl Posed {
    /// Reads `model`'s nodes and the clip `motion` loops; `None` when the
    /// model has none of the nodes it moves.
    pub fn load<R: Read + Seek>(
        source: &mut AssetSource<R>,
        model: u32,
        motion: &PropMotion,
    ) -> Result<Option<Self>, AssetError> {
        let skeleton = Skeleton::from_nodes(&source.model(model)?.parse()?.nodes);
        let clip = match motion.clip {
            Some(h) => source
                .asset(h, ANIMATION_TYPE)
                .ok()
                .and_then(|data| Clip::parse(&data).ok())
                .and_then(|clip| BoundClip::bind(h, clip, &skeleton)),
            None => None,
        };
        let hinges: Vec<(usize, f32)> = motion
            .hinges
            .iter()
            .filter_map(|&(node, degrees)| {
                let i = skeleton.name_hashes.iter().position(|&n| n == node)?;
                Some((i, degrees.to_radians()))
            })
            .collect();
        Ok((clip.is_some() || !hinges.is_empty()).then_some(Self {
            skeleton,
            clip,
            hinges,
        }))
    }

    /// Every node's matrix in the model at `time` seconds, as
    /// [`GpuModel::draw_moved`](crate::GpuModel::draw_moved) takes them.
    pub fn nodes(&self, time: f32) -> Vec<Mat4> {
        let mut pose = match &self.clip {
            Some(c) => c.sample_looped(&self.skeleton, time),
            None => self.skeleton.rest_pose(),
        };
        for &(i, angle) in &self.hinges {
            if let Some(l) = pose.0.get_mut(i) {
                l.rotation = (l.rotation * Quat::from_rotation_x(angle)).normalize();
            }
        }
        self.skeleton.model_matrices(&pose)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_assets::model::Node;

    #[test]
    fn a_hinge_tips_the_beam_up() {
        let node = |name_hash, parent| Node {
            name_hash,
            flags: 0,
            first_child: None,
            parent,
            next_sibling: None,
            unk_0x0c: 0,
            local: Mat4::IDENTITY.into(),
            inverse_bind: Mat4::IDENTITY.into(),
            bounds_min: [0.0; 4],
            bounds_max: [0.0; 4],
        };
        let skeleton = Skeleton::from_nodes(&[node(1, None), node(2, Some(0))]);
        let posed = Posed {
            skeleton,
            clip: None,
            hinges: vec![(1, 60f32.to_radians())],
        };
        let beam = posed.nodes(0.0)[1].transform_vector3(-m2_math::glam::Vec3::Z);
        assert!((beam.y - 60f32.to_radians().sin()).abs() < 1e-5);
    }
}
