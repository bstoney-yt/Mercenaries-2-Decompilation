//! Animation runtime: clips sampled into node-space poses, crossfades,
//! root motion, and the skinning matrices the renderer needs.
//!
//! A [`Skeleton`] is a model's node hierarchy. Clips bind to it by node name
//! hash; a [`Pose`] holds every node's local transform, so poses from
//! different clips blend node by node regardless of their track order.
//!
//! Locomotion clips carry their travel on the root node's translation and
//! are otherwise in place. [`ClipInfo`] measures each clip's speed and
//! direction from that track, and [`Pose::strip_root_motion`] pins the root
//! horizontally so gameplay moves the character instead. Picking idle, walk,
//! run and sprint clips by measured speed follows the browser viewer.

use std::collections::HashMap;
use std::io::{Read, Seek};

use m2_assets::animation::ANIMATION_TYPE;
use m2_assets::model::Node;
use m2_assets::{BlockContents, Clip};
use m2_math::glam::{Mat4, Quat, Vec2, Vec3};

use crate::assets::{AssetError, AssetSource};

/// One node's local transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Local {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Local {
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    fn lerp(&self, other: &Self, t: f32) -> Self {
        Self {
            translation: self.translation.lerp(other.translation, t),
            rotation: self.rotation.slerp(other.rotation, t).normalize(),
            scale: self.scale.lerp(other.scale, t),
        }
    }

    fn from_matrix(m: Mat4) -> Self {
        let (scale, rotation, translation) = m.to_scale_rotation_translation();
        Self {
            translation,
            rotation: if rotation.is_finite() {
                rotation.normalize()
            } else {
                Quat::IDENTITY
            },
            scale,
        }
    }

    fn from_track(t: &m2_assets::Transform) -> Self {
        let r = t.rotation;
        let rotation = Quat::from_xyzw(r.x, r.y, r.z, r.w);
        Self {
            translation: Vec3::new(t.translation.x, t.translation.y, t.translation.z),
            rotation: if rotation.is_finite() && rotation.length() > 0.0 {
                rotation.normalize()
            } else {
                Quat::IDENTITY
            },
            scale: Vec3::new(t.scale.x, t.scale.y, t.scale.z),
        }
    }
}

/// A model's node hierarchy.
#[derive(Clone, Debug)]
pub struct Skeleton {
    pub name_hashes: Vec<u32>,
    pub parents: Vec<Option<usize>>,
    /// Rest-pose locals.
    pub rest: Vec<Local>,
    /// Bind space to node space, column-vector convention.
    pub inverse_bind: Vec<Mat4>,
}

impl Skeleton {
    pub fn from_nodes(nodes: &[Node]) -> Self {
        let n = nodes.len();
        Self {
            name_hashes: nodes.iter().map(|n| n.name_hash).collect(),
            parents: nodes
                .iter()
                .map(|x| x.parent.map(usize::from).filter(|&p| p < n))
                .collect(),
            rest: nodes
                .iter()
                .map(|x| Local::from_matrix(Mat4::from(x.local)))
                .collect(),
            inverse_bind: nodes.iter().map(|x| Mat4::from(x.inverse_bind)).collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.parents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parents.is_empty()
    }

    pub fn rest_pose(&self) -> Pose {
        Pose(self.rest.clone())
    }

    /// The root node: the first without a parent.
    pub fn root(&self) -> usize {
        self.parents.iter().position(Option::is_none).unwrap_or(0)
    }

    /// Each node's model-space matrix in `pose`.
    pub fn model_matrices(&self, pose: &Pose) -> Vec<Mat4> {
        let n = self.len();
        let mut out: Vec<Option<Mat4>> = vec![None; n];
        let mut chain = Vec::new();
        for i in 0..n {
            chain.clear();
            let mut at = Some(i);
            while let Some(j) = at {
                if out[j].is_some() || chain.len() > n {
                    break;
                }
                chain.push(j);
                at = self.parents[j];
            }
            for &j in chain.iter().rev() {
                let local = pose.0.get(j).map_or(Mat4::IDENTITY, Local::matrix);
                out[j] = Some(match self.parents[j].and_then(|p| out[p]) {
                    Some(parent) => parent * local,
                    None => local,
                });
            }
        }
        out.into_iter().map(Option::unwrap_or_default).collect()
    }

    /// Skinning matrices for `pose`: bind space to posed model space, per
    /// node.
    pub fn skin_matrices(&self, pose: &Pose) -> Vec<Mat4> {
        self.model_matrices(pose)
            .into_iter()
            .zip(&self.inverse_bind)
            .map(|(m, ib)| m * *ib)
            .collect()
    }
}

/// Every node's local transform.
#[derive(Clone, Debug, PartialEq)]
pub struct Pose(pub Vec<Local>);

impl Pose {
    /// Blends towards `other` by `t` (0 keeps `self`).
    pub fn blend(&mut self, other: &Pose, t: f32) {
        for (a, b) in self.0.iter_mut().zip(&other.0) {
            *a = a.lerp(b, t);
        }
    }

    /// Pins `node`'s horizontal translation to `at` (x, z), leaving height.
    pub fn strip_root_motion(&mut self, node: usize, at: Vec2) {
        if let Some(l) = self.0.get_mut(node) {
            l.translation.x = at.x;
            l.translation.z = at.y;
        }
    }
}

/// A clip bound to a skeleton.
#[derive(Clone, Debug)]
pub struct BoundClip {
    pub hash: u32,
    pub clip: Clip,
    /// The track driving each node.
    node_tracks: Vec<Option<usize>>,
    pub info: ClipInfo,
}

/// What a clip does to the character as a whole.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipInfo {
    pub duration: f32,
    /// Metres per second the root travels, start to end.
    pub speed: f32,
    /// Horizontal travel direction (x, z), or zero.
    pub direction: Vec2,
    /// The root's horizontal position on the first frame.
    pub root_start: Vec2,
    /// The pelvis height on the first frame, metres.
    pub hips: f32,
}

impl BoundClip {
    /// Binds `clip` to `skeleton`; `None` unless every track names a node.
    pub fn bind(hash: u32, clip: Clip, skeleton: &Skeleton) -> Option<Self> {
        if clip.track_count() == 0 || clip.frame_count == 0 {
            return None;
        }
        if !clip
            .track_nodes
            .iter()
            .all(|h| skeleton.name_hashes.contains(h))
        {
            return None;
        }
        let node_tracks = skeleton
            .name_hashes
            .iter()
            .map(|h| clip.track_nodes.iter().position(|t| t == h))
            .collect();
        let mut bound = Self {
            hash,
            clip,
            node_tracks,
            info: ClipInfo {
                duration: 0.0,
                speed: 0.0,
                direction: Vec2::ZERO,
                root_start: Vec2::ZERO,
                hips: 0.0,
            },
        };
        bound.info = bound.measure(skeleton);
        Some(bound)
    }

    fn measure(&self, skeleton: &Skeleton) -> ClipInfo {
        let duration = self.clip.header.duration.max(0.0);
        let root = skeleton.root();
        let first = self.sample(skeleton, 0.0);
        let last = self.sample(skeleton, duration);
        let a = first.0[root].translation;
        let b = last.0[root].translation;
        let travel = Vec2::new(b.x - a.x, b.z - a.z);
        // The pelvis: the root's child that sits highest.
        let models = skeleton.model_matrices(&first);
        let hips = (0..skeleton.len())
            .filter(|&i| skeleton.parents[i] == Some(root))
            .map(|i| models[i].w_axis.y)
            .fold(f32::MIN, f32::max);
        ClipInfo {
            duration,
            speed: if duration > 0.0 {
                travel.length() / duration
            } else {
                0.0
            },
            direction: travel.normalize_or_zero(),
            root_start: Vec2::new(a.x, a.z),
            hips: if hips.is_finite() { hips } else { 0.0 },
        }
    }

    /// The pose at `time` (clamped); nodes no track drives keep their rest
    /// pose.
    pub fn sample(&self, skeleton: &Skeleton, time: f32) -> Pose {
        let tracks = self.clip.sample(time);
        Pose(
            self.node_tracks
                .iter()
                .zip(&skeleton.rest)
                .map(|(t, rest)| match t.and_then(|t| tracks.get(t)) {
                    Some(tr) => Local::from_track(tr),
                    None => *rest,
                })
                .collect(),
        )
    }

    /// The pose at `time` looped over the clip's duration.
    pub fn sample_looped(&self, skeleton: &Skeleton, time: f32) -> Pose {
        let d = self.info.duration;
        let t = if d > 0.0 { time.rem_euclid(d) } else { 0.0 };
        self.sample(skeleton, t)
    }
}

/// Reads every clip in blocks whose path contains one of `filters` and
/// binds those that fit `skeleton`.
pub fn load_clips<R: Read + Seek>(
    source: &mut AssetSource<R>,
    skeleton: &Skeleton,
    filters: &[&str],
) -> Result<Vec<BoundClip>, AssetError> {
    let index = source.archive().index();
    let mut blocks: Vec<usize> = index
        .assets()
        .iter()
        .filter(|a| index.type_hash(a) == ANIMATION_TYPE)
        .map(|a| usize::from(a.block))
        .filter(|&b| {
            let path = &index.blocks()[b].path;
            filters.iter().any(|f| path.contains(f))
        })
        .collect();
    blocks.sort_unstable();
    blocks.dedup();
    let mut out = Vec::new();
    for b in blocks {
        let decoded = source.block(b)?;
        for (e, data) in BlockContents::parse(&decoded)?.iter() {
            if e.type_hash != ANIMATION_TYPE {
                continue;
            }
            if let Ok(clip) = Clip::parse(data)
                && let Some(bound) = BoundClip::bind(e.name_hash, clip, skeleton)
            {
                out.push(bound);
            }
        }
    }
    out.sort_by_key(|c| c.hash);
    Ok(out)
}

/// Movement speeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Gait {
    Idle,
    Walk,
    Run,
    Sprint,
}

impl Gait {
    /// Root speed bands (m/s) the clips for each moving gait fall in, from
    /// the viewer's measurements of the game's locomotion clips.
    fn band(self) -> (f32, f32) {
        match self {
            Self::Idle => (0.0, 0.0),
            Self::Walk => (0.9, 1.9),
            Self::Run => (4.5, 5.6),
            Self::Sprint => (5.6, 7.5),
        }
    }
}

/// One clip per gait, chosen from a character's clips.
#[derive(Clone, Debug, PartialEq)]
pub struct Locomotion {
    /// Index into the clip list per gait.
    pub clips: HashMap<Gait, usize>,
    /// The character's forward direction in its own space (x, z).
    pub forward: Vec2,
}

impl Locomotion {
    /// Picks idle, walk, run and sprint clips: forward the way the fast
    /// clips travel, the pelvis upright, the longest still clip as idle.
    /// `None` if any gait has no clip.
    pub fn choose(clips: &[BoundClip]) -> Option<Self> {
        let movers: Vec<&ClipInfo> = clips
            .iter()
            .map(|c| &c.info)
            .filter(|i| i.speed > 4.0)
            .collect();
        let forward = movers
            .iter()
            .fold(Vec2::ZERO, |a, i| a + i.direction)
            .normalize_or_zero();
        if forward == Vec2::ZERO {
            return None;
        }
        let standing = |i: &ClipInfo| i.hips > 0.85;
        let mut chosen = HashMap::new();
        let idle = clips
            .iter()
            .enumerate()
            .filter(|(_, c)| c.info.speed * c.info.duration < 1e-3)
            .filter(|(_, c)| c.info.hips > 0.95 && c.info.hips < 1.15 && c.info.duration >= 1.3)
            .max_by(|a, b| a.1.info.duration.total_cmp(&b.1.info.duration))?;
        chosen.insert(Gait::Idle, idle.0);
        for gait in [Gait::Walk, Gait::Run, Gait::Sprint] {
            let (lo, hi) = gait.band();
            let (i, _) = clips.iter().enumerate().find(|(_, c)| {
                let i = &c.info;
                standing(i) && i.direction.dot(forward) > 0.95 && i.speed >= lo && i.speed < hi
            })?;
            chosen.insert(gait, i);
        }
        Some(Self {
            clips: chosen,
            forward,
        })
    }
}

/// Plays one clip at a time with crossfades, pinning the root horizontally.
#[derive(Clone, Debug)]
pub struct Animator {
    current: Option<(usize, f32)>,
    previous: Option<(usize, f32)>,
    fade: f32,
    fade_time: f32,
}

impl Default for Animator {
    fn default() -> Self {
        Self {
            current: None,
            previous: None,
            fade: 1.0,
            fade_time: 0.25,
        }
    }
}

impl Animator {
    /// Switches to clip `index` (no-op if it is already playing), fading
    /// from the current one.
    pub fn play(&mut self, index: usize) {
        if self.current.map(|c| c.0) == Some(index) {
            return;
        }
        self.previous = self.current;
        self.current = Some((index, 0.0));
        self.fade = if self.previous.is_some() { 0.0 } else { 1.0 };
    }

    pub fn advance(&mut self, dt: f32) {
        for c in [&mut self.current, &mut self.previous]
            .into_iter()
            .flatten()
        {
            c.1 += dt;
        }
        self.fade = (self.fade + dt / self.fade_time).min(1.0);
        if self.fade >= 1.0 {
            self.previous = None;
        }
    }

    /// The blended pose, root pinned.
    pub fn pose(&self, skeleton: &Skeleton, clips: &[BoundClip]) -> Pose {
        let sample = |(i, t): (usize, f32)| {
            clips.get(i).map(|c| {
                let mut p = c.sample_looped(skeleton, t);
                p.strip_root_motion(skeleton.root(), c.info.root_start);
                p
            })
        };
        let Some(mut pose) = self.current.and_then(sample) else {
            return skeleton.rest_pose();
        };
        if let Some(mut from) = self.previous.and_then(sample) {
            from.blend(&pose, self.fade);
            pose = from;
        }
        pose
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skeleton() -> Skeleton {
        // root -> hips (1 m up) -> spine (0.5 m up)
        let local = |y: f32| Local {
            translation: Vec3::new(0.0, y, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        };
        Skeleton {
            name_hashes: vec![10, 11, 12],
            parents: vec![None, Some(0), Some(1)],
            rest: vec![local(0.0), local(1.0), local(0.5)],
            inverse_bind: vec![
                Mat4::IDENTITY,
                Mat4::from_translation(Vec3::new(0.0, -1.0, 0.0)),
                Mat4::from_translation(Vec3::new(0.0, -1.5, 0.0)),
            ],
        }
    }

    #[test]
    fn rest_pose_skins_to_identity() {
        let s = skeleton();
        let models = s.model_matrices(&s.rest_pose());
        assert!((models[2].w_axis.y - 1.5).abs() < 1e-6);
        for m in s.skin_matrices(&s.rest_pose()) {
            assert!(m.abs_diff_eq(Mat4::IDENTITY, 1e-6), "{m}");
        }
    }

    #[test]
    fn blends_and_pins_the_root() {
        let s = skeleton();
        let mut a = s.rest_pose();
        let mut b = s.rest_pose();
        b.0[0].translation = Vec3::new(2.0, 0.0, 4.0);
        a.blend(&b, 0.5);
        assert_eq!(a.0[0].translation, Vec3::new(1.0, 0.0, 2.0));
        a.strip_root_motion(0, Vec2::ZERO);
        assert_eq!(a.0[0].translation, Vec3::ZERO);
    }

    #[test]
    fn animator_fades_between_clips() {
        let mut anim = Animator::default();
        anim.play(0);
        assert_eq!(anim.fade, 1.0);
        anim.play(1);
        assert_eq!(anim.fade, 0.0);
        anim.advance(0.125);
        assert!((anim.fade - 0.5).abs() < 1e-6);
        anim.advance(0.2);
        assert!(anim.previous.is_none());
        // Without clips the rest pose comes back.
        assert_eq!(anim.pose(&skeleton(), &[]), skeleton().rest_pose());
    }

    #[test]
    fn gaits_fall_in_their_bands() {
        assert!(Gait::Walk.band().1 <= Gait::Run.band().0);
        assert_eq!(Gait::Run.band().1, Gait::Sprint.band().0);
    }
}
