//! Lip-sync: a spoken line's FaceFX animation played on the speaker's
//! face ([`m2_assets::facefx`], [`crate::face_graph`]).
//!
//! # How the 360 does it
//!
//! From the 360 code:
//! - `Animation.BindFaceAnimSet(guid, set)` (0x82315CF8) mounts the
//!   animation set (asset type 0x665EF13E) on the **face rig asset** the
//!   person's record names (type 0x1CF649BB), not on the person: every
//!   person with that rig then has the set's animations
//!   (0x8281A278). `UnbindFaceAnimSet` (0x82315E68) takes it off again
//!   (0x8281A4E8). Both return whether both assets were there.
//! - When a speaker's voice line starts (0x822782D8) with the voice
//!   manager in cinematic mode (manager +0x528, set by
//!   `VO.SetCinematicMode`, 0x82349620 -> 0x82278118), it posts
//!   {speaker, the line's name hash, 0} on the face-animation queue
//!   (0x8320A758, 0x82278998). Out of cinematic mode it turns on the
//!   `Flap` expression instead ([`crate::face_expression::FLAP`]).
//! - When the line stops (0x82278440), in cinematic mode it posts the
//!   name hash `stop` (0xE2EC63DD) the same way; out of it, `Flap` at 0.
//! - The queue's listener (0x8241D270) takes the speaker's face rig and
//!   walks its animation groups (the rig's own, then each mounted set);
//!   in each, every animation's name is hashed and the first equal to
//!   the posted hash is played (0x8281D828). `stop` stops whatever plays
//!   (0x8281D298). A hash no group holds plays nothing.
//!
//! So an animation is keyed to its voice line by name: the line's cue
//! name is the animation's name (`Chris-Briefing-Contract-All01-72`).
//!
//! # Not established
//!
//! - **Inferred**: that the animation's clock is the game's from the
//!   moment the line starts (the FaceFX instance's tick was not read), and
//!   that it ends at its last key when no `stop` comes first.
//! - The animation's own blend in and out (`blend_in`, `blend_out`) are
//!   not applied: the face bones take the rig's pose outright while it
//!   plays (**invented**).

use std::collections::HashMap;
use std::sync::Arc;

use m2_assets::facefx::{Actor, Animation, AnimationSet};
use m2_assets::name_hash;

use crate::anim::Local;
use crate::face_graph::{BonePose, FaceGraph};
use m2_math::glam::{Quat, Vec3};

/// The hash the voice manager posts when a line stops (`stop`).
pub const STOP: u32 = 0xE2EC_63DD;

/// The face rigs and animation sets read so far, and which sets are
/// mounted on which rig.
#[derive(Debug, Default)]
pub struct Library {
    /// By rig name hash; `None` when it could not be read.
    graphs: HashMap<u32, Option<Arc<FaceGraph>>>,
    /// By set name hash; `None` when it could not be read.
    sets: HashMap<u32, Option<Arc<AnimationSet>>>,
    /// The sets mounted on each rig, in the order they were bound.
    bound: HashMap<u32, Vec<u32>>,
}

impl Library {
    /// Mounts set `set` on rig `actor` (`Animation.BindFaceAnimSet`);
    /// binding it twice keeps one.
    pub fn bind(&mut self, actor: u32, set: u32) {
        let sets = self.bound.entry(actor).or_default();
        if !sets.contains(&set) {
            sets.push(set);
        }
    }

    /// Takes set `set` off rig `actor` (`Animation.UnbindFaceAnimSet`).
    pub fn unbind(&mut self, actor: u32, set: u32) {
        if let Some(sets) = self.bound.get_mut(&actor) {
            sets.retain(|&s| s != set);
        }
    }

    /// The sets mounted on rig `actor`.
    pub fn bound(&self, actor: u32) -> &[u32] {
        self.bound.get(&actor).map_or(&[], Vec::as_slice)
    }

    /// Puts rig `actor` in, already read.
    pub fn insert_actor(&mut self, actor: u32, rig: &Actor) {
        self.graphs
            .insert(actor, Some(Arc::new(FaceGraph::new(rig))));
    }

    /// Puts set `set` in, already read.
    pub fn insert_set(&mut self, set: u32, animations: AnimationSet) {
        self.sets.insert(set, Some(Arc::new(animations)));
    }

    /// Rig `actor`'s graph, read by `load` the first time (`None` when it
    /// is not there or does not parse).
    pub fn graph(
        &mut self,
        actor: u32,
        load: impl FnOnce(u32) -> Option<Vec<u8>>,
    ) -> Option<Arc<FaceGraph>> {
        self.graphs
            .entry(actor)
            .or_insert_with(|| {
                let bytes = load(actor)?;
                Actor::parse(&bytes)
                    .ok()
                    .map(|a| Arc::new(FaceGraph::new(&a)))
            })
            .clone()
    }

    /// Set `set`, read by `load` the first time.
    pub fn set(
        &mut self,
        set: u32,
        load: impl FnOnce(u32) -> Option<Vec<u8>>,
    ) -> Option<Arc<AnimationSet>> {
        self.sets
            .entry(set)
            .or_insert_with(|| {
                let bytes = load(set)?;
                AnimationSet::parse(&bytes).ok().map(Arc::new)
            })
            .clone()
    }

    /// The animation for the line whose name hashes to `cue`, said by a
    /// person with rig `actor`: the first of the rig's mounted sets
    /// holding an animation so named (0x8241D270). `load` reads an asset
    /// by name hash and type ([`m2_assets::facefx::ACTOR_TYPE`],
    /// [`m2_assets::facefx::ANIMATION_SET_TYPE`]).
    pub fn find(
        &mut self,
        actor: u32,
        cue: u32,
        mut load: impl FnMut(u32, u32) -> Option<Vec<u8>>,
    ) -> Option<LipSync> {
        let graph = self.graph(actor, |h| load(h, m2_assets::facefx::ACTOR_TYPE))?;
        for set_hash in self.bound(actor).to_vec() {
            let Some(set) = self.set(set_hash, |h| load(h, m2_assets::facefx::ANIMATION_SET_TYPE))
            else {
                continue;
            };
            if let Some(index) = set
                .animations
                .iter()
                .position(|a| name_hash(&a.name) == cue)
            {
                return Some(LipSync {
                    graph,
                    set,
                    set_hash,
                    index,
                    seconds: 0.0,
                });
            }
        }
        None
    }
}

/// A line's face animation playing on one face.
#[derive(Clone, Debug)]
pub struct LipSync {
    graph: Arc<FaceGraph>,
    set: Arc<AnimationSet>,
    /// The set's name hash.
    pub set_hash: u32,
    index: usize,
    /// Seconds since the line started.
    pub seconds: f32,
}

impl LipSync {
    /// The animation playing.
    pub fn animation(&self) -> &Animation {
        &self.set.animations[self.index]
    }

    /// The animation's curves that have keys.
    pub fn curves(&self) -> usize {
        self.animation()
            .curves
            .iter()
            .filter(|c| !c.keys.is_empty())
            .count()
    }

    /// Moves its clock on `dt` seconds; false once it is past its last
    /// key (inferred: the instance's own end).
    pub fn step(&mut self, dt: f32) -> bool {
        self.seconds += dt;
        self.seconds <= self.animation().seconds()
    }

    /// Where the rig's face bones are now.
    pub fn poses(&self) -> Vec<BonePose> {
        self.graph.at(self.animation(), self.seconds)
    }
}

/// A face rig's position in the head skeleton's frame: the rigs' frame
/// is the skeleton's mirrored in x.
///
/// From samples (`facerest` example): for Chris's rig (39f47e08) and
/// Fiona's (2b5ba84c) against the head models carrying their bones,
/// every bone's rest has x of the other sign from its node's local (the
/// eye and brow bones 0.10 to 0.17 m apart as read, under 0.03 m once
/// mirrored), and every rest rotation is the node's with y and z
/// negated: the same rotation seen in a mirror across x.
fn mirrored(v: Vec3) -> Vec3 {
    Vec3::new(-v.x, v.y, v.z)
}

/// A face rig's rotation in the head skeleton's frame ([`mirrored`]).
fn mirrored_turn(q: Quat) -> Quat {
    Quat::from_xyzw(q.x, -q.y, -q.z, q.w)
}

/// Puts the face bones `poses` into a skeleton's locals: each pose's move
/// from its rig rest, brought into the skeleton's frame ([`mirrored`]),
/// is put on the local of the node whose name hash (in `names`) is its
/// bone's. Bones the skeleton does not have are left out. Returns how
/// many nodes were moved.
///
/// The rig's rest is not the head's (one rig serves heads of different
/// shapes: Chris's rest sits up to 3 cm off either of his heads' nodes
/// even mirrored), so a pose is a move from it, not a place: at rest a
/// face is its head's own. Written in raw, the rig's transforms put the
/// eyes in the chin (playtest of e8749a2a). Inferred: that the game puts
/// the move on its own bone (the FaceFX library's evaluation in the 360
/// program was not read); it leaves every head as modelled at rest.
pub fn apply(locals: &mut [Local], names: &[u32], poses: &[BonePose]) -> usize {
    let mut set = 0;
    for p in poses {
        if let Some(k) = names.iter().position(|&n| n == p.bone)
            && let Some(l) = locals.get_mut(k)
        {
            let (rest_p, rest_r, rest_s) = p.rest;
            let turn = mirrored_turn(p.rotation * rest_r.inverse());
            *l = Local {
                translation: l.translation + mirrored(p.position - rest_p),
                rotation: (turn * l.rotation).normalize(),
                scale: l.scale + (p.scale - rest_s),
            };
            set += 1;
        }
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_math::glam::{Quat, Vec3};

    use m2_assets::facefx::{BONE_POSE_NODE, Bone, Curve, Key, Pose, Transform};

    fn t(p: [f32; 3]) -> Transform {
        Transform {
            position: p,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        }
    }

    /// A jaw that `Oat` drops 2 cm.
    fn rig() -> Actor {
        Actor {
            name: "NewActor".to_owned(),
            bones: vec![Bone {
                name: "bone_jaw".to_owned(),
                rest: t([0.0; 3]),
                index: 0,
                weight: 1.0,
            }],
            poses: vec![Pose {
                name: "Oat".to_owned(),
                class: BONE_POSE_NODE.to_owned(),
                least: 0.0,
                most: 1.0,
                inputs: Vec::new(),
                bones: vec![("bone_jaw".to_owned(), t([0.0, -0.02, 0.0]))],
            }],
        }
    }

    fn key(seconds: f32, value: f32) -> Key {
        Key {
            seconds,
            value,
            slope_in: 0.0,
            slope_out: 0.0,
        }
    }

    /// One line, `Chris-Briefing-Contract-All01-72`: `Oat` opens over
    /// half a second and shuts again by one.
    fn set() -> AnimationSet {
        AnimationSet {
            name: "ALL01_Contract_Briefing_Chris".to_owned(),
            actor: "NewActor".to_owned(),
            animations: vec![Animation {
                name: "Chris-Briefing-Contract-All01-72".to_owned(),
                curves: vec![Curve {
                    name: "Oat".to_owned(),
                    keys: vec![key(0.0, 0.0), key(0.5, 1.0), key(1.0, 0.0)],
                }],
                blend_in: 0.16,
                blend_out: 0.22,
            }],
        }
    }

    #[test]
    fn a_line_plays_the_animation_of_its_name_from_a_bound_set_only() {
        let mut lib = Library::default();
        lib.insert_actor(1, &rig());
        lib.insert_set(10, set());
        let cue = name_hash("Chris-Briefing-Contract-All01-72");
        let no_load = |_: u32, _: u32| -> Option<Vec<u8>> { None };
        // Not bound: the rig holds no such animation (0x8241D270).
        assert!(lib.find(1, cue, no_load).is_none());
        lib.bind(1, 10);
        assert!(lib.find(1, name_hash("Another-Line"), no_load).is_none());
        let mut lip = lib.find(1, cue, no_load).expect("bound");
        assert_eq!(lip.set_hash, 10);
        assert_eq!(lip.curves(), 1);
        let jaw = |l: &LipSync| l.poses()[0].position.y;
        assert!(jaw(&lip).abs() < 1e-6);
        assert!(lip.step(0.5));
        assert!((jaw(&lip) + 0.02).abs() < 1e-4, "{}", jaw(&lip));
        assert!(lip.step(0.5));
        assert!(!lip.step(0.1), "past its last key");
        // Another person with the same rig has it too; unbound, nobody.
        lib.unbind(1, 10);
        assert!(lib.find(1, cue, no_load).is_none());
    }

    #[test]
    fn binding_is_per_rig_and_ordered() {
        let mut lib = Library::default();
        lib.bind(1, 10);
        lib.bind(1, 11);
        lib.bind(1, 10);
        lib.bind(2, 12);
        assert_eq!(lib.bound(1), &[10, 11]);
        lib.unbind(1, 10);
        assert_eq!(lib.bound(1), &[11]);
        assert_eq!(lib.bound(2), &[12]);
        assert!(lib.bound(3).is_empty());
    }

    #[test]
    fn a_rig_that_is_not_there_plays_nothing_and_is_read_once() {
        let mut lib = Library::default();
        lib.bind(1, 10);
        let mut reads = 0;
        assert!(
            lib.find(1, 5, |_, _| {
                reads += 1;
                None
            })
            .is_none()
        );
        assert!(
            lib.find(1, 5, |_, _| {
                reads += 1;
                None
            })
            .is_none()
        );
        assert_eq!(reads, 1);
    }

    /// A pose at its rig rest leaves the node as its head has it, however
    /// far the rig's rest is from the node's; a move from rest is put on
    /// the node, mirrored in x; nodes not named are left alone.
    #[test]
    fn poses_move_the_named_nodes_from_their_own_rest() {
        let rest = Local {
            translation: Vec3::new(0.05, 0.09, -0.03),
            rotation: Quat::from_xyzw(0.5, 0.5, 0.5, -0.5),
            scale: Vec3::ONE,
        };
        // The rig's rest of the same bone, as Chris's rig has an eye's:
        // mirrored in x, and a centimetre off.
        let rig = (
            Vec3::new(-0.054, 0.105, -0.032),
            Quat::from_xyzw(0.5, -0.5, -0.5, -0.5),
            Vec3::ONE,
        );
        let mut locals = vec![rest; 3];
        let names = [7, name_hash("bone_eye_l"), 9];
        let at_rest = BonePose {
            bone: name_hash("bone_eye_l"),
            position: rig.0,
            rotation: rig.1,
            scale: rig.2,
            rest: rig,
        };
        assert_eq!(apply(&mut locals, &names, &[at_rest]), 1);
        assert!((locals[1].translation - rest.translation).length() < 1e-6);
        assert!(locals[1].rotation.angle_between(rest.rotation) < 1e-4);
        assert_eq!(locals[0], rest);
        assert_eq!(locals[2], rest);
        // 1 cm along the rig's x and up its y: the node goes 1 cm along
        // its own -x and up its y.
        let moved = BonePose {
            position: rig.0 + Vec3::new(0.01, 0.01, 0.0),
            ..at_rest
        };
        let mut locals = vec![rest; 3];
        apply(&mut locals, &names, &[moved]);
        let d = locals[1].translation - rest.translation;
        assert!((d - Vec3::new(-0.01, 0.01, 0.0)).length() < 1e-6, "{d}");
        // A bone the skeleton lacks moves nothing.
        let tongue = BonePose {
            bone: name_hash("bone_tongue_tip"),
            ..at_rest
        };
        assert_eq!(apply(&mut locals, &names, &[tongue]), 0);
    }

    /// A turn about the rig's y is a turn the other way about the
    /// skeleton's y: a mirror across x.
    #[test]
    fn a_rigs_turn_is_mirrored_in_x() {
        let q = Quat::from_rotation_y(0.4);
        assert!(mirrored_turn(q).angle_between(Quat::from_rotation_y(-0.4)) < 1e-5);
        let q = Quat::from_rotation_x(0.4);
        assert!(mirrored_turn(q).angle_between(q) < 1e-5);
    }
}
