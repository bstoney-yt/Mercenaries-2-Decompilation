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
use std::sync::Arc;

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

    /// The nodes of the upper body: the spine from just above the pelvis
    /// and everything on it (arms, head), found from the hands `a` and
    /// `b`. Walking down from the root towards the hands, the pelvis is the
    /// first node with two limbs (chains three or more nodes deep) that
    /// hold neither hand: the legs. Weapon markers and other single nodes
    /// hanging off the root don't count, nor does the neck off the chest.
    pub fn upper_body(&self, a: usize, b: usize) -> Option<Arc<[bool]>> {
        let up = |mut n: usize| {
            let mut path = vec![n];
            while let Some(p) = self.parents.get(n).copied().flatten() {
                path.push(p);
                n = p;
            }
            path
        };
        let (pa, pb) = (up(a), up(b));
        let shared: Vec<usize> = pa.iter().copied().filter(|n| pb.contains(n)).collect();
        let children = |n: usize| {
            (0..self.len())
                .filter(|&c| self.parents[c] == Some(n))
                .collect::<Vec<_>>()
        };
        fn depth(s: &Skeleton, n: usize) -> usize {
            1 + (0..s.len())
                .filter(|&c| s.parents[c] == Some(n))
                .map(|c| depth(s, c))
                .max()
                .unwrap_or(0)
        }
        let limbs = |n: usize| {
            children(n)
                .into_iter()
                .filter(|&c| !pa.contains(&c) && !pb.contains(&c) && depth(self, c) >= 3)
                .count()
        };
        // Walk down from the root along the hands' shared path to the
        // pelvis; the spine starts at its child on the path.
        let path: Vec<usize> = shared.iter().rev().copied().collect();
        let at = path.iter().position(|&n| limbs(n) >= 2)?;
        let spine = *path.get(at + 1)?;
        let mut mask = vec![false; self.len()];
        for (i, m) in mask.iter_mut().enumerate() {
            *m = up(i).contains(&spine);
        }
        Some(mask.into())
    }

    /// The pelvis: the root's child that sits highest at rest (as
    /// [`ClipInfo::hips`] measures it).
    pub fn pelvis(&self) -> Option<usize> {
        let root = self.root();
        let models = self.model_matrices(&self.rest_pose());
        (0..self.len())
            .filter(|&i| self.parents[i] == Some(root))
            .max_by(|&a, &b| models[a].w_axis.y.total_cmp(&models[b].w_axis.y))
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
    /// Every node's rotation on the first frame, to compare postures.
    first_rotations: Vec<Quat>,
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
    /// Where to pin the root horizontally so the pelvis starts over the
    /// origin (clips may be authored away from it).
    pub pin: Vec2,
    /// The pelvis height on the first frame, metres.
    pub hips: f32,
}

impl BoundClip {
    /// Binds `clip` to `skeleton`; `None` unless every track names a node.
    pub fn bind(hash: u32, clip: Clip, skeleton: &Skeleton) -> Option<Self> {
        if !clip
            .track_nodes
            .iter()
            .all(|h| skeleton.name_hashes.contains(h))
        {
            return None;
        }
        Self::bind_by_name(hash, clip, skeleton)
    }

    /// Binds `clip` to another `skeleton` of the same character family
    /// (people models differ from the player's in a few bones): tracks for
    /// bones it lacks are dropped, nodes no track names keep their rest
    /// pose. `None` unless most tracks find their bone.
    pub fn rebind(&self, skeleton: &Skeleton) -> Option<Self> {
        let found = self
            .clip
            .track_nodes
            .iter()
            .filter(|h| skeleton.name_hashes.contains(h))
            .count();
        if found * 10 < self.clip.track_nodes.len() * 8 {
            return None;
        }
        Self::bind_by_name(self.hash, self.clip.clone(), skeleton)
    }

    fn bind_by_name(hash: u32, clip: Clip, skeleton: &Skeleton) -> Option<Self> {
        if clip.track_count() == 0 || clip.frame_count == 0 {
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
                pin: Vec2::ZERO,
                hips: 0.0,
            },
            first_rotations: Vec::new(),
        };
        bound.info = bound.measure(skeleton);
        bound.first_rotations = bound
            .sample(skeleton, 0.0)
            .0
            .iter()
            .map(|l| l.rotation)
            .collect();
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
        let pelvis = (0..skeleton.len())
            .filter(|&i| skeleton.parents[i] == Some(root))
            .max_by(|&a, &b| models[a].w_axis.y.total_cmp(&models[b].w_axis.y));
        let hips = pelvis.map_or(f32::MIN, |i| models[i].w_axis.y);
        let pelvis_xz = pelvis.map_or(Vec2::new(a.x, a.z), |i| {
            Vec2::new(models[i].w_axis.x, models[i].w_axis.z)
        });
        ClipInfo {
            duration,
            speed: if duration > 0.0 {
                travel.length() / duration
            } else {
                0.0
            },
            direction: travel.normalize_or_zero(),
            root_start: Vec2::new(a.x, a.z),
            // Moving the root by -offset moves the pelvis to the origin.
            pin: Vec2::new(a.x, a.z) - pelvis_xz,
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

/// How differently two poses hold their joints: 0 for the same rotations,
/// growing with each joint turned away.
fn posture_distance(a: &[Quat], b: &[Quat]) -> f32 {
    a.iter().zip(b).map(|(x, y)| 1.0 - x.dot(*y).abs()).sum()
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
        for gait in [Gait::Walk, Gait::Run, Gait::Sprint] {
            let (lo, hi) = gait.band();
            let (i, _) = clips.iter().enumerate().find(|(_, c)| {
                let i = &c.info;
                standing(i) && i.direction.dot(forward) > 0.95 && i.speed >= lo && i.speed < hi
            })?;
            chosen.insert(gait, i);
        }
        // Idle: of the clips that stay put and hold the pelvis at standing
        // height, the one whose first frame looks most like the walk's
        // (many still clips are sitting, lying or scripted poses).
        let walk = &clips[chosen[&Gait::Walk]];
        let idle = clips
            .iter()
            .enumerate()
            .filter(|(_, c)| c.info.speed * c.info.duration < 1e-3 && c.info.duration >= 1.0)
            .filter(|(_, c)| c.info.hips > 0.85 && c.info.hips < 1.3)
            .map(|(i, c)| {
                (
                    i,
                    posture_distance(&walk.first_rotations, &c.first_rotations),
                )
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))?;
        chosen.insert(Gait::Idle, idle.0);
        Some(Self {
            clips: chosen,
            forward,
        })
    }
}

/// A clip playing: which, how far in, and whether it holds its last frame
/// rather than looping.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Playing {
    clip: usize,
    time: f32,
    once: bool,
    /// Keeps the pelvis at its first frame's height (a jump: the body
    /// rises in the game's physics, not in the clip).
    level: bool,
}

/// A clip the upper body plays over the rest (a rifle held while the legs
/// run): which, how far in, and which nodes it moves.
#[derive(Clone, Debug)]
struct Upper {
    clip: usize,
    time: f32,
    mask: Arc<[bool]>,
}

/// Plays one clip at a time with crossfades, pinning the root horizontally,
/// with optionally another on the upper body.
#[derive(Clone, Debug)]
pub struct Animator {
    current: Option<Playing>,
    previous: Option<Playing>,
    fade: f32,
    fade_time: f32,
    upper: Option<Upper>,
}

impl Default for Animator {
    fn default() -> Self {
        Self {
            current: None,
            previous: None,
            fade: 1.0,
            fade_time: 0.25,
            upper: None,
        }
    }
}

impl Animator {
    /// Switches to clip `index`, looped (no-op if it is already playing),
    /// fading from the current one.
    pub fn play(&mut self, index: usize) {
        self.start(index, false, false);
    }

    /// Plays clip `index` once from the start and holds its last frame
    /// (a fall, a reaction), fading quickly from the current one.
    pub fn play_once(&mut self, index: usize) {
        self.start(index, true, false);
        self.fade_time = 0.1;
    }

    /// Plays clip `index` once like [`play_once`](Self::play_once), with
    /// the pelvis held at its starting height: a jump whose rise the
    /// physics already gives the character.
    pub fn play_leveled(&mut self, index: usize) {
        self.start(index, true, true);
        self.fade_time = 0.1;
    }

    fn start(&mut self, clip: usize, once: bool, level: bool) {
        if self
            .current
            .is_some_and(|c| c.clip == clip && c.once == once && c.level == level)
        {
            return;
        }
        self.fade_time = 0.25;
        self.previous = self.current;
        self.current = Some(Playing {
            clip,
            time: 0.0,
            once,
            level,
        });
        self.fade = if self.previous.is_some() { 0.0 } else { 1.0 };
    }

    /// The clip playing and how far in it is, seconds.
    pub fn current(&self) -> Option<(usize, f32)> {
        self.current.map(|c| (c.clip, c.time))
    }

    /// Plays clip `index` looped on the nodes `mask` marks (the upper
    /// body, [`Skeleton::upper_body`]) over whatever plays; `None` stops.
    pub fn set_upper(&mut self, clip: Option<(usize, Arc<[bool]>)>) {
        match clip {
            Some((c, _)) if self.upper.as_ref().is_some_and(|u| u.clip == c) => {}
            Some((clip, mask)) => {
                self.upper = Some(Upper {
                    clip,
                    time: 0.0,
                    mask,
                })
            }
            None => self.upper = None,
        }
    }

    pub fn advance(&mut self, dt: f32) {
        if let Some(u) = &mut self.upper {
            u.time += dt;
        }
        for c in [&mut self.current, &mut self.previous]
            .into_iter()
            .flatten()
        {
            c.time += dt;
        }
        self.fade = (self.fade + dt / self.fade_time).min(1.0);
        if self.fade >= 1.0 {
            self.previous = None;
        }
    }

    /// The blended pose, root pinned.
    pub fn pose(&self, skeleton: &Skeleton, clips: &[BoundClip]) -> Pose {
        let sample = |p: Playing| {
            clips.get(p.clip).map(|c| {
                let mut pose = if p.once {
                    c.sample(skeleton, p.time.min(c.info.duration))
                } else {
                    c.sample_looped(skeleton, p.time)
                };
                pose.strip_root_motion(skeleton.root(), c.info.pin);
                if p.level
                    && let Some(pelvis) = skeleton.pelvis()
                {
                    let rise = skeleton.model_matrices(&pose)[pelvis].w_axis.y - c.info.hips;
                    pose.0[skeleton.root()].translation.y -= rise;
                }
                pose
            })
        };
        let Some(mut pose) = self.current.and_then(sample) else {
            return skeleton.rest_pose();
        };
        if let Some(mut from) = self.previous.and_then(sample) {
            from.blend(&pose, self.fade);
            pose = from;
        }
        if let Some(u) = &self.upper
            && let Some(c) = clips.get(u.clip)
        {
            let over = c.sample_looped(skeleton, u.time);
            for ((local, mine), on) in pose.0.iter_mut().zip(over.0).zip(u.mask.iter()) {
                if *on {
                    *local = mine;
                }
            }
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
        assert_eq!(s.pelvis(), Some(1));
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
    fn the_upper_body_starts_above_the_pelvis() {
        // root 0 -> pelvis 1 -> {thigh 2 -> calf 10 -> foot 11, thigh 3 ->
        // calf 12 -> foot 13, spine 4 -> chest 5 -> {arm 6 -> hand 7,
        // arm 8 -> hand 9, neck 14 -> head 15}}; markers 16, 17 on the root.
        let parents = vec![
            None,
            Some(0),
            Some(1),
            Some(1),
            Some(1),
            Some(4),
            Some(5),
            Some(6),
            Some(5),
            Some(8),
            Some(2),
            Some(10),
            Some(3),
            Some(12),
            Some(5),
            Some(14),
            Some(0),
            Some(0),
        ];
        let n = parents.len();
        let s = Skeleton {
            name_hashes: (0..n as u32).collect(),
            parents,
            rest: vec![Local::from_matrix(Mat4::IDENTITY); n],
            inverse_bind: vec![Mat4::IDENTITY; n],
        };
        let mask = s.upper_body(7, 9).expect("an upper body");
        let upper: Vec<usize> = (0..n).filter(|&i| mask[i]).collect();
        assert_eq!(upper, vec![4, 5, 6, 7, 8, 9, 14, 15]);
        let mut anim = Animator::default();
        anim.set_upper(Some((3, mask.clone())));
        anim.advance(0.5);
        // Setting the same clip again keeps its place.
        anim.set_upper(Some((3, mask)));
        assert_eq!(anim.upper.as_ref().map(|u| u.time), Some(0.5));
        anim.set_upper(None);
        assert!(anim.upper.is_none());
    }

    #[test]
    fn gaits_fall_in_their_bands() {
        assert!(Gait::Walk.band().1 <= Gait::Run.band().0);
        assert_eq!(Gait::Run.band().1, Gait::Sprint.band().0);
    }
}
