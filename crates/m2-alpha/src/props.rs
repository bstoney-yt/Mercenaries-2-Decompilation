//! Models the state layers place while they are in: a mission's gates,
//! barricades and crates ([`WorldIndex::state_objects`]). They stand once
//! the scripts load their layer (or the start-up did), go when it
//! unloads, and break when the scripts' object dies: drawn as their wreck
//! when the model has one, else gone, and no longer in the way; one that
//! breaks apart ([`Breakup`], a gate) throws its pieces, which lie where
//! they land. Those whose model comes loose ([`Loose`]) are knocked loose
//! by vehicles and blasts, as the island's are.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::panic::AssertUnwindSafe;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use m2_assets::model::ModelInfo;
use m2_engine::model::MODEL_TEXTURE_WIDTH;
use m2_engine::stream::PROP_SIZES;
use m2_engine::{
    AssetSource, Breakup, CollisionShape, CpuModel, DebrisId, Frustum, GpuModel, Install, Loose,
    MaterialCache, Physics, StaticKey, WorldIndex,
};
use m2_math::glam::{Mat4, Quat, Vec3};
use m2_render::{DrawQueue, RenderBackend};

/// One shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prop {
    pub model: u32,
    pub position: Vec3,
    pub rotation: Quat,
    /// Broken: drawn as its wreck, and nothing to bump into.
    pub wrecked: bool,
}

/// A model's GPU copy and collision, once loaded, and how it breaks
/// apart.
#[derive(Debug)]
struct PropModel {
    gpu: GpuModel,
    collision: Option<CollisionShape>,
    breakup: Option<Arc<Breakup>>,
    /// For a merged batch (a `TINY` model), the entities it is made of: it
    /// isn't drawn while any of them is shown on its own.
    members: Vec<u32>,
}

/// The state layers' models in play.
#[derive(Debug, Default)]
pub struct Props {
    shown: HashMap<u32, Prop>,
    /// By hash, once some shown prop wants it: `None` until it has loaded
    /// (and when it failed).
    models: HashMap<u32, Option<PropModel>>,
    /// Wanted, not yet asked of the loaders.
    waiting: HashSet<u32>,
    /// Asked of the loaders, not yet uploaded.
    asked: HashSet<u32>,
    /// Decoded, not yet uploaded.
    decoded: Vec<Load>,
    /// Started with the first model wanted.
    loaders: Option<Loaders>,
    /// Models that failed to load.
    failed: HashSet<u32>,
    /// How near the props wanting each model are, as last worked out.
    near: Option<Near>,
    /// Changes to those shown (put up, taken down, moved, gone).
    changes: u64,
    /// How far from the focus models were last wanted (metres).
    reach: f32,
    /// Those knocked loose: their pieces, in [`Loose`] order.
    knocked: HashMap<u32, Vec<DebrisId>>,
    /// Those broken apart: their pieces, in their [`Breakup`]'s order.
    broken: HashMap<u32, Vec<DebrisId>>,
    /// And the rest of them going down into the ground ([`SINK_ACCEL`]).
    sinking: HashMap<u32, Sinking>,
    /// Knocked loose since [`take_knocked`](Self::take_knocked).
    fresh_knocks: Vec<u32>,
    /// Node matrices of those that move on their own, by entity
    /// ([`crate::moving`]).
    poses: HashMap<u32, Vec<Mat4>>,
    /// Those only a support strike brings down (a mission's gates): no
    /// vehicle or common blast knocks them loose.
    pub sturdy: HashSet<u32>,
    /// What of those broken apart is drawn (the gate test's pictures).
    pub parts: Parts,
    /// One left undrawn (the gate test's picture of what else stands
    /// where a gate does).
    pub skip: Option<u32>,
    /// The layer that put each up, by entity (the gate test's lists).
    layers: HashMap<u32, Arc<str>>,
}

/// What [`Props::draw`] draws of a model broken apart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Parts {
    /// The rest of it and its pieces.
    #[default]
    All,
    /// The rest of it alone.
    Rest,
    /// Its pieces alone.
    Pieces,
}

/// How the props stand ([`Props::counts`]): how many are shown, and of
/// the models they use how many have loaded, are loading, wait (within
/// reach, and in all) and failed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PropCounts {
    pub shown: usize,
    pub loaded: usize,
    pub loading: usize,
    pub waiting_near: usize,
    pub waiting: usize,
    pub failed: usize,
}

impl std::fmt::Display for PropCounts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} shown; their models: {} loaded, {} loading, {} waiting within reach \
             ({} in all), {} failed",
            self.shown, self.loaded, self.loading, self.waiting_near, self.waiting, self.failed
        )
    }
}

/// For each model not yet loaded, how far (squared) the nearest shown prop
/// using it stood from `focus` ([`Props::nearness`]), as of change
/// `changes` to those shown.
#[derive(Debug)]
struct Near {
    focus: Vec3,
    changes: u64,
    by_model: HashMap<u32, f32>,
}

/// How long uploading loaded models may take a frame (at least one is).
const UPLOAD_BUDGET: Duration = Duration::from_millis(4);

/// Threads decoding models.
const LOADERS: usize = 3;

/// How many models may be asked of the loaders and not yet uploaded:
/// few, so that whatever is nearest when one finishes goes next. A start-up
/// puts up thousands of props, and a mission's gate has to load while the
/// player walks up to it, not after all of them.
const IN_FLIGHT: usize = 8;

/// Which models are nearest is worked out again once the focus has moved
/// this far (metres), or those shown have changed.
const RENEAR: f32 = 50.0;

/// A model decoded on a loading thread (and, for a merged batch, the
/// entities it is made of), or why not.
type Load = (u32, Result<(CpuModel, Vec<u32>), String>);

/// The loading threads: models are asked by hash and come back decoded.
#[derive(Debug)]
struct Loaders {
    ask: Sender<u32>,
    done: Receiver<Load>,
}

impl Props {
    /// The model of shown prop `entity`, and where it stands.
    pub fn placed(&self, entity: u32) -> Option<(u32, Vec3)> {
        self.shown.get(&entity).map(|p| (p.model, p.position))
    }

    /// Sets the node matrices the props that move on their own are drawn
    /// with, by entity (those left out stand in their rest pose).
    pub fn set_poses(&mut self, poses: HashMap<u32, Vec<Mat4>>) {
        self.poses = poses;
    }

    /// Puts up the models of those of `entities` the state layers place
    /// (others are ignored); their models load nearest first as
    /// [`Props::update`] goes. How many were new.
    pub fn show(
        &mut self,
        install: &Install,
        index: &WorldIndex,
        entities: impl IntoIterator<Item = u32>,
    ) -> usize {
        let mut added = 0;
        for e in entities {
            let Some(o) = index.state_objects.get(&e) else {
                continue;
            };
            if self.shown.contains_key(&e) {
                continue;
            }
            self.shown.insert(
                e,
                Prop {
                    model: o.model,
                    position: o.position,
                    rotation: o.rotation,
                    // A ruin stands as its model's wreck from the start.
                    wrecked: index.ruined.contains(&e),
                },
            );
            added += 1;
            self.changes += 1;
            if let std::collections::hash_map::Entry::Vacant(slot) = self.models.entry(o.model) {
                slot.insert(None);
                self.waiting.insert(o.model);
            }
        }
        if !self.waiting.is_empty() && self.loaders.is_none() {
            self.loaders = Some(Loaders::start(install));
        }
        added
    }

    /// Notes that `layer` puts up `entities`.
    pub fn note_layer(&mut self, entities: impl IntoIterator<Item = u32>, layer: &str) {
        let layer: Arc<str> = layer.into();
        for e in entities {
            self.layers.insert(e, layer.clone());
        }
    }

    /// The layer that put `entity` up, if one was noted.
    pub fn layer(&self, entity: u32) -> Option<&str> {
        self.layers.get(&entity).map(|l| &**l)
    }

    /// Takes down those of `entities` that are shown (their loose pieces
    /// too).
    pub fn hide(&mut self, physics: &mut Physics, entities: impl IntoIterator<Item = u32>) {
        for e in entities {
            if self.shown.remove(&e).is_some() {
                physics.remove_static(StaticKey::Entity(e));
                self.changes += 1;
            }
            self.sinking.remove(&e);
            let broken = self.broken.remove(&e).into_iter().flatten();
            for d in self.knocked.remove(&e).into_iter().flatten().chain(broken) {
                physics.remove_debris(d);
            }
        }
    }

    /// How `entity`'s model comes loose, if it does.
    fn loose(&self, entity: u32) -> Option<&Loose> {
        let p = self.shown.get(&entity)?;
        let m = self.models.get(&p.model)?.as_ref()?;
        m.collision.as_ref()?.loose.as_deref()
    }

    /// The mass to shift (kg) to knock `entity` loose; `None` unless it
    /// stands whole, its model comes loose, and it isn't
    /// [`sturdy`](Self::sturdy).
    pub fn loose_mass(&self, physics: &Physics, entity: u32) -> Option<f32> {
        if self.sturdy.contains(&entity) {
            return None;
        }
        self.any_loose_mass(physics, entity)
    }

    /// [`loose_mass`](Self::loose_mass), sturdy or not.
    fn any_loose_mass(&self, physics: &Physics, entity: u32) -> Option<f32> {
        let p = self.shown.get(&entity)?;
        if p.wrecked
            || self.knocked.contains_key(&entity)
            || !physics.has_static(StaticKey::Entity(entity))
        {
            return None;
        }
        self.loose(entity).map(|l| l.mass)
    }

    /// Knocks `entity` loose: its standing collision goes and its pieces
    /// fly off at `velocity` (m/s), turning at `spin` (radians a second
    /// about each axis). False unless it came loose; one that is
    /// [`sturdy`](Self::sturdy) only does with `strike` (a support strike).
    pub fn knock(
        &mut self,
        physics: &mut Physics,
        entity: u32,
        velocity: Vec3,
        spin: Vec3,
        strike: bool,
    ) -> bool {
        let mass = match strike {
            true => self.any_loose_mass(physics, entity),
            false => self.loose_mass(physics, entity),
        };
        if mass.is_none() {
            return false;
        }
        let (Some(p), Some(loose)) = (self.shown.get(&entity), self.loose(entity)) else {
            return false;
        };
        physics.remove_static(StaticKey::Entity(entity));
        let placed = Mat4::from_rotation_translation(p.rotation, p.position);
        let pieces = loose
            .pieces
            .iter()
            .zip(loose.starts(placed))
            .map(|(piece, at)| physics.add_debris(piece, at, velocity, spin))
            .collect();
        self.knocked.insert(entity, pieces);
        self.fresh_knocks.push(entity);
        true
    }

    /// Those knocked loose since the last call.
    pub fn take_knocked(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.fresh_knocks)
    }

    /// Those standing that come loose with any of their model within
    /// `reach` metres of `at` (as far as its box's middle less half its
    /// size), with how far; the [`sturdy`](Self::sturdy) too with
    /// `strike` (a support strike).
    pub fn loose_near(
        &self,
        physics: &Physics,
        at: Vec3,
        reach: f32,
        strike: bool,
    ) -> Vec<(u32, f32)> {
        self.shown
            .iter()
            .filter(|&(&e, _)| match strike {
                true => self.any_loose_mass(physics, e).is_some(),
                false => self.loose_mass(physics, e).is_some(),
            })
            .filter_map(|(&e, p)| {
                let m = self.models.get(&p.model)?.as_ref()?;
                let (lo, hi) = m.gpu.bounds;
                let centre = p.position + p.rotation * ((lo + hi) * 0.5);
                let d = (centre.distance(at) - (hi - lo).length() * 0.5).max(0.0);
                (d <= reach).then_some((e, d))
            })
            .collect()
    }

    /// Breaks `entity` if it is shown and whole: its wreck if the model
    /// has one, else it goes. One that breaks apart throws its pieces
    /// away from `blast` (what broke it), if that went off within
    /// [`THROW_REACH`], as the rest of it sinks a little into the ground
    /// (one found broken, as after loading a game, is already down). True
    /// if it was one.
    pub fn wreck(&mut self, physics: &mut Physics, entity: u32, blast: Option<Vec3>) -> bool {
        self.wreck_scaled(physics, entity, blast, 1.0)
    }

    /// [`wreck`](Self::wreck), what breaks apart thrown `scale` times as
    /// fast (the gate test's tries).
    pub fn wreck_scaled(
        &mut self,
        physics: &mut Physics,
        entity: u32,
        blast: Option<Vec3>,
        scale: f32,
    ) -> bool {
        let Some(p) = self.shown.get_mut(&entity).filter(|p| !p.wrecked) else {
            return false;
        };
        physics.remove_static(StaticKey::Entity(entity));
        let model = self.models.get(&p.model).and_then(Option::as_ref);
        if let Some(m) = model.filter(|m| m.gpu.has_wreck()) {
            p.wrecked = true;
            let placed = Mat4::from_rotation_translation(p.rotation, p.position);
            if let Some(breakup) = m.breakup.as_deref()
                && !self.knocked.contains_key(&entity)
            {
                let pieces = throw(physics, breakup, placed, blast, scale);
                let sinking = match (&pieces, blast) {
                    (Some(_), Some(from)) => Sinking::from(entity, placed, m.gpu.bounds, from),
                    _ => Sinking::under(),
                };
                self.sinking.insert(entity, sinking);
                if let Some(pieces) = pieces {
                    eprintln!(
                        "props: {entity:#x} ({:08x}) breaks apart into {} pieces, away from \
                         {:.1}, burning at {} points, the rest sinking {:.1} deg over",
                        p.model,
                        pieces.len(),
                        blast.unwrap_or_default(),
                        breakup.points.len(),
                        sinking.lean.to_degrees()
                    );
                    self.broken.insert(entity, pieces);
                }
            }
        } else if !self.knocked.contains_key(&entity) {
            // Loose pieces stay where they fell.
            self.shown.remove(&entity);
            self.changes += 1;
        }
        true
    }

    /// Whether `entity` broke apart (its pieces thrown).
    pub fn broke_apart(&self, entity: u32) -> bool {
        self.broken.contains_key(&entity)
    }

    /// Where `entity`'s break-up names points (its fires), world space:
    /// none unless it broke apart.
    pub fn breakup_points(&self, entity: u32) -> Vec<Vec3> {
        let Some(p) = self
            .shown
            .get(&entity)
            .filter(|_| self.broken.contains_key(&entity))
        else {
            return Vec::new();
        };
        let Some(Some(m)) = self.models.get(&p.model) else {
            return Vec::new();
        };
        let placed = Mat4::from_rotation_translation(p.rotation, p.position);
        m.breakup.as_ref().map_or_else(Vec::new, |b| {
            b.points
                .iter()
                .map(|&x| placed.transform_point3(x))
                .collect()
        })
    }

    /// Moves `entity` (the scripts placed it elsewhere).
    pub fn put(&mut self, physics: &mut Physics, entity: u32, position: Vec3) {
        let Some(p) = self.shown.get_mut(&entity) else {
            return;
        };
        p.position = position;
        self.changes += 1;
        // Put up whole again.
        self.sinking.remove(&entity);
        let broken = self.broken.remove(&entity).into_iter().flatten();
        for d in self
            .knocked
            .remove(&entity)
            .into_iter()
            .flatten()
            .chain(broken)
        {
            physics.remove_debris(d);
        }
        if let Some(Some(m)) = self.models.get(&p.model)
            && let Some(shape) = &m.collision
            && !p.wrecked
        {
            physics.add_static(StaticKey::Entity(entity), shape, p.position, p.rotation);
        }
    }

    /// Whether `point` is within `pad` metres of `entity`'s model's box;
    /// `None` unless it is shown and its model has loaded.
    pub fn touches(&self, entity: u32, point: Vec3, pad: f32) -> Option<bool> {
        let p = self.shown.get(&entity)?;
        let m = self.models.get(&p.model)?.as_ref()?;
        let local = p.rotation.inverse() * (point - p.position);
        let (lo, hi) = m.gpu.bounds;
        Some(local.cmpge(lo - pad).all() && local.cmple(hi + pad).all())
    }

    /// Where `entity` stands and its model's box (model space); `None`
    /// unless it is shown and its model has loaded.
    pub fn placed_bounds(&self, entity: u32) -> Option<(Mat4, (Vec3, Vec3))> {
        let p = self.shown.get(&entity)?;
        let m = self.models.get(&p.model)?.as_ref()?;
        Some((
            Mat4::from_rotation_translation(p.rotation, p.position),
            m.gpu.bounds,
        ))
    }

    /// The other shown props whose boxes meet `entity`'s (the gate test
    /// lists what else stands at a gate), biggest first: each one's
    /// entity, how much of its box is inside `entity`'s (0 to 1), and a
    /// line on it: its model, the layer that put it up, where it stands,
    /// its box (world axes), that share, and whether it is wrecked or in
    /// the way.
    pub fn others_in_box(&self, physics: &Physics, entity: u32) -> Vec<(u32, f32, String)> {
        let world_box = |e: u32| {
            self.placed_bounds(e)
                .map(|(at, bounds)| box_around(at, bounds))
        };
        let Some((lo, hi)) = world_box(entity) else {
            return Vec::new();
        };
        let volume = |v: Vec3| v.x.max(0.01) * v.y.max(0.01) * v.z.max(0.01);
        let mut out: Vec<(f32, u32, f32, String)> = self
            .shown
            .iter()
            .filter(|&(&e, _)| e != entity)
            .filter_map(|(&e, p)| {
                let (a, b) = world_box(e)?;
                let (c, d) = (a.max(lo), b.min(hi));
                if !c.cmplt(d).all() {
                    return None;
                }
                let size = b - a;
                let share = volume(d - c) / volume(size);
                let line = format!(
                    "{e:#x} ({:08x}) of {} at {:.1}, {:.1}x{:.1}x{:.1} m, {:.0}% inside{}{}",
                    p.model,
                    self.layer(e).unwrap_or("no layer noted"),
                    p.position,
                    size.x,
                    size.y,
                    size.z,
                    share * 100.0,
                    if p.wrecked { ", wrecked" } else { "" },
                    if physics.has_static(StaticKey::Entity(e)) {
                        ", in the way"
                    } else {
                        ""
                    }
                );
                Some((volume(size), e, share, line))
            })
            .collect();
        out.sort_by(|a, b| b.0.total_cmp(&a.0));
        out.into_iter()
            .map(|(_, e, share, line)| (e, share, line))
            .collect()
    }

    /// The middle of `entity`'s model's box in the world; `None` unless it
    /// is shown and its model has loaded.
    pub fn centre(&self, entity: u32) -> Option<Vec3> {
        let p = self.shown.get(&entity)?;
        let m = self.models.get(&p.model)?.as_ref()?;
        let (lo, hi) = m.gpu.bounds;
        Some(p.position + p.rotation * ((lo + hi) * 0.5))
    }

    /// The middle of the top of `entity`'s model's box in the world (where
    /// the satellite bomb lands on a gate); `None` unless it is shown and
    /// its model has loaded.
    pub fn top(&self, entity: u32) -> Option<Vec3> {
        let p = self.shown.get(&entity)?;
        let m = self.models.get(&p.model)?.as_ref()?;
        let (lo, hi) = m.gpu.bounds;
        Some(p.position + p.rotation * Vec3::new((lo.x + hi.x) * 0.5, hi.y, (lo.z + hi.z) * 0.5))
    }

    /// How `entity` stands, if it is shown, for the autoplay's pictures of
    /// a gate the bomb brought down: broken, how many of its pieces fly,
    /// how far the rest has sunk, and what is drawn of it.
    pub fn describe(&self, physics: &Physics, entity: u32) -> Option<String> {
        let p = self.shown.get(&entity)?;
        let mut s = format!("{entity:#x} ({:08x}) at {:.1}", p.model, p.position);
        let Some(Some(m)) = self.models.get(&p.model) else {
            return Some(s + ", its model not loaded");
        };
        if p.wrecked {
            s += ", wrecked";
        }
        let loose = match (self.broken.get(&entity), self.knocked.get(&entity)) {
            (Some(pieces), _) => Some((m.breakup.as_deref().map(|b| &b.pieces), pieces)),
            (None, Some(pieces)) => Some((self.loose(entity), pieces)),
            (None, None) => None,
        };
        if let Some((loose, pieces)) = loose {
            let places: Vec<Option<Mat4>> =
                pieces.iter().map(|&d| physics.debris_place(d)).collect();
            let placed = places.iter().flatten().count();
            s += &format!(", {placed} of {} pieces placed", pieces.len());
            if let Some(loose) = loose {
                // Those still where they started (a piece that never flew
                // stands as it did).
                let at = Mat4::from_rotation_translation(p.rotation, p.position);
                let still = loose
                    .starts(at)
                    .iter()
                    .zip(&places)
                    .filter(|(start, now)| {
                        now.is_some_and(|now| {
                            now.w_axis.truncate().distance(start.w_axis.truncate()) < 0.5
                        })
                    })
                    .count();
                s += &format!(" ({still} still where they started)");
                let (rest, flown, skinned) =
                    m.gpu.drawn_moved(loose.breaks || p.wrecked, loose.flying());
                s += &format!(
                    "; drawn: {rest} parts with the rest, {flown} flying, {skinned} of them \
                     skinned"
                );
            }
        }
        if let Some(sink) = self.sinking.get(&entity) {
            let depth = sink.depth();
            s += &format!(
                "; the rest sunk {depth:.1} m after {:.1} s, leaning {:.1} deg",
                sink.time,
                (sink.lean * depth / SINK_MOST).to_degrees()
            );
        }
        Some(s)
    }

    /// How `entity`'s pieces set off as it broke apart (the gate test asks
    /// as it breaks): each one's node, its box's sides along the model's
    /// own axes and its middle in the model (metres from its origin), how
    /// far it was lifted out of the ground, and how fast it goes (and
    /// upward) and spins.
    pub fn pieces_report(&self, physics: &Physics, entity: u32) -> Option<String> {
        let p = self.shown.get(&entity)?;
        let breakup = self.models.get(&p.model)?.as_ref()?.breakup.as_deref()?;
        let ids = self.broken.get(&entity)?;
        let placed = Mat4::from_rotation_translation(p.rotation, p.position);
        let back = placed.inverse();
        let starts = breakup.pieces.starts(placed);
        let each: Vec<String> = breakup
            .pieces
            .pieces
            .iter()
            .zip(&starts)
            .zip(ids)
            .map(|((piece, &start), &id)| {
                let (lo, hi) = corners(start, piece)
                    .iter()
                    .map(|&c| back.transform_point3(c))
                    .fold((Vec3::MAX, Vec3::MIN), |(a, b), c| (a.min(c), b.max(c)));
                let (size, middle) = (hi - lo, (lo + hi) * 0.5);
                let lift = physics
                    .debris_place(id)
                    .map_or(0.0, |now| now.w_axis.y - start.w_axis.y);
                let (v, w) = physics.debris_velocity(id).unwrap_or_default();
                format!(
                    "n{} {:.1}x{:.1}x{:.1} at {:.1},{:.1},{:.1} lift {lift:.1} v {:.1} up {:.1} \
                     spin {:.1}",
                    piece.node.map_or(-1, i32::from),
                    size.x,
                    size.y,
                    size.z,
                    middle.x,
                    middle.y,
                    middle.z,
                    v.length(),
                    v.y,
                    w.length()
                )
            })
            .collect();
        Some(each.join("; "))
    }

    /// Where `entity`'s pieces have gone since it broke apart (the gate
    /// test): how many are within a metre of where they started, how many
    /// have turned less than 15 degrees, and how many have their tops more
    /// than 4 m above the model's base; then each one's node, how far its
    /// middle has moved (metres), how far it has turned (degrees) and how
    /// high its top is above the model's base.
    pub fn pieces_moved(&self, physics: &Physics, entity: u32) -> Option<String> {
        let p = self.shown.get(&entity)?;
        let m = self.models.get(&p.model)?.as_ref()?;
        let breakup = m.breakup.as_deref()?;
        let ids = self.broken.get(&entity)?;
        let placed = Mat4::from_rotation_translation(p.rotation, p.position);
        let base = placed.transform_point3(Vec3::Y * m.gpu.bounds.0.y).y;
        let starts = breakup.pieces.starts(placed);
        let (mut near, mut level, mut high) = (0, 0, 0);
        let mut each = Vec::new();
        for ((piece, &start), &id) in breakup.pieces.pieces.iter().zip(&starts).zip(ids) {
            let Some(now) = physics.debris_place(id) else {
                continue;
            };
            let is = corners(now, piece);
            let moved = box_middle(&is).distance(box_middle(&corners(start, piece)));
            let turned = Quat::from_mat4(&now)
                .angle_between(Quat::from_mat4(&start))
                .to_degrees();
            let top = is.iter().map(|c| c.y).fold(f32::MIN, f32::max) - base;
            near += usize::from(moved < 1.0);
            level += usize::from(turned < 15.0);
            high += usize::from(top > 4.0);
            each.push(format!(
                "n{} {moved:.1} m {turned:.0} deg top {top:.1}",
                piece.node.map_or(-1, i32::from)
            ));
        }
        Some(format!(
            "{near} of {} within 1 m of where they started, {level} turned under 15 deg, \
             {high} with their tops over 4 m up: {}",
            ids.len(),
            each.join("; ")
        ))
    }

    /// `entity`'s pieces still within `within` metres of where they
    /// started since it broke apart, each with the statics it touches (the
    /// gate test asks what holds them): its node, then those.
    pub fn pieces_held(
        &self,
        physics: &Physics,
        entity: u32,
        within: f32,
    ) -> Vec<(i32, Vec<StaticKey>)> {
        let (Some(p), Some(ids)) = (self.shown.get(&entity), self.broken.get(&entity)) else {
            return Vec::new();
        };
        let Some(breakup) = self
            .models
            .get(&p.model)
            .and_then(Option::as_ref)
            .and_then(|m| m.breakup.as_deref())
        else {
            return Vec::new();
        };
        let placed = Mat4::from_rotation_translation(p.rotation, p.position);
        let starts = breakup.pieces.starts(placed);
        breakup
            .pieces
            .pieces
            .iter()
            .zip(&starts)
            .zip(ids)
            .filter_map(|((piece, &start), &id)| {
                let now = physics.debris_place(id)?;
                let moved =
                    box_middle(&corners(now, piece)).distance(box_middle(&corners(start, piece)));
                (moved < within)
                    .then(|| (piece.node.map_or(-1, i32::from), physics.debris_statics(id)))
            })
            .collect()
    }

    /// `entity`'s, if it is shown, and whether its model has loaded.
    pub fn get(&self, entity: u32) -> Option<(&Prop, bool)> {
        let p = self.shown.get(&entity)?;
        Some((p, self.models.get(&p.model).is_some_and(Option::is_some)))
    }

    /// Those shown, by entity.
    pub fn shown(&self) -> impl Iterator<Item = (u32, &Prop)> {
        self.shown.iter().map(|(&e, p)| (e, p))
    }

    /// Whether every model wanted within reach (as last asked) has loaded
    /// or failed: none is being loaded, and none waits within reach.
    pub fn settled(&self) -> bool {
        let within = self.reach * self.reach;
        self.asked.is_empty()
            && self.decoded.is_empty()
            && (self.waiting.is_empty()
                || self.near.as_ref().is_some_and(|n| {
                    self.waiting
                        .iter()
                        .all(|h| n.by_model.get(h).is_none_or(|&d| d > within))
                }))
    }

    /// How many are shown, and of the models they use how many have
    /// loaded, are loading, wait to be loaded (within reach, and further)
    /// and failed.
    pub fn counts(&self) -> PropCounts {
        let within = self.reach * self.reach;
        let near = self.waiting.iter().filter(|h| {
            self.near
                .as_ref()
                .and_then(|n| n.by_model.get(h))
                .is_some_and(|&d| d <= within)
        });
        PropCounts {
            shown: self.shown.len(),
            loaded: self.models.values().filter(|m| m.is_some()).count(),
            loading: self.asked.len() + self.decoded.len(),
            waiting_near: near.count(),
            waiting: self.waiting.len(),
            failed: self.failed.len(),
        }
    }

    /// Uploads models that have loaded, nearest `focus` first (as many as
    /// fit in [`UPLOAD_BUDGET`], at least one), gives the props using them
    /// their collision, and asks the loaders for the nearest still wanted
    /// by a prop within `reach` metres (those further load as she nears
    /// them).
    pub fn update(
        &mut self,
        backend: &mut dyn RenderBackend,
        materials: &mut MaterialCache,
        source: &mut AssetSource<BufReader<File>>,
        physics: &mut Physics,
        focus: Vec3,
        reach: f32,
    ) -> anyhow::Result<()> {
        self.reach = reach;
        let Some(loaders) = &self.loaders else {
            return Ok(());
        };
        self.decoded.extend(loaders.done.try_iter());
        if self.decoded.is_empty() && (self.waiting.is_empty() || self.asked.len() >= IN_FLIGHT) {
            return Ok(());
        }
        let near = match self.near.take() {
            Some(n) if n.changes == self.changes && n.focus.distance(focus) <= RENEAR => n,
            _ => Near {
                focus,
                changes: self.changes,
                by_model: self.nearness(focus),
            },
        };
        let far = |hash: &u32| near.by_model.get(hash).copied().unwrap_or(f32::INFINITY);
        let started = Instant::now();
        while let Some(i) = (0..self.decoded.len())
            .min_by(|&a, &b| far(&self.decoded[a].0).total_cmp(&far(&self.decoded[b].0)))
        {
            let (hash, cpu) = self.decoded.swap_remove(i);
            self.asked.remove(&hash);
            self.upload(backend, materials, source, physics, hash, cpu)?;
            if started.elapsed() > UPLOAD_BUDGET {
                break;
            }
        }
        // A model no shown prop uses any more isn't wanted; showing one
        // again wants it again.
        let unwanted: Vec<u32> = self
            .waiting
            .iter()
            .copied()
            .filter(|h| !near.by_model.contains_key(h))
            .collect();
        for h in unwanted {
            self.waiting.remove(&h);
            self.models.remove(&h);
        }
        let room = IN_FLIGHT.saturating_sub(self.asked.len());
        if room > 0 && !self.waiting.is_empty() {
            let within = reach * reach;
            let mut next: Vec<u32> = self
                .waiting
                .iter()
                .copied()
                .filter(|h| far(h) <= within)
                .collect();
            next.sort_by(|a, b| far(a).total_cmp(&far(b)));
            for hash in next.into_iter().take(room) {
                self.waiting.remove(&hash);
                self.asked.insert(hash);
                if let Some(loaders) = &self.loaders
                    && loaders.ask.send(hash).is_err()
                {
                    // The loaders are gone: nothing more will load.
                    self.asked.remove(&hash);
                    self.failed.insert(hash);
                }
            }
        }
        self.near = Some(near);
        Ok(())
    }

    /// For each model not yet loaded, how far (squared) its nearest shown
    /// prop is from `focus`.
    fn nearness(&self, focus: Vec3) -> HashMap<u32, f32> {
        let mut near = HashMap::new();
        for p in self.shown.values() {
            if !matches!(self.models.get(&p.model), Some(None)) {
                continue;
            }
            let d = p.position.distance_squared(focus);
            near.entry(p.model)
                .and_modify(|n: &mut f32| *n = n.min(d))
                .or_insert(d);
        }
        near
    }

    /// Uploads model `hash`, loaded (or not), and gives the props using it
    /// their collision.
    fn upload(
        &mut self,
        backend: &mut dyn RenderBackend,
        materials: &mut MaterialCache,
        source: &mut AssetSource<BufReader<File>>,
        physics: &mut Physics,
        hash: u32,
        cpu: Result<(CpuModel, Vec<u32>), String>,
    ) -> anyhow::Result<()> {
        let (cpu, members) = match cpu {
            Ok(cpu) => cpu,
            Err(e) => {
                eprintln!("state layer model {hash:08x}: {e}");
                self.failed.insert(hash);
                return Ok(());
            }
        };
        materials.load_model_textures(backend, source, &cpu)?;
        let gpu = GpuModel::upload(backend, materials, &cpu)?;
        if let Some(shape) = &cpu.collision {
            for (&e, p) in self.shown.iter().filter(|(_, p)| p.model == hash) {
                if !p.wrecked && !self.knocked.contains_key(&e) {
                    physics.add_static(StaticKey::Entity(e), shape, p.position, p.rotation);
                }
            }
        }
        if !members.is_empty() {
            let users: Vec<String> = (self.shown.iter())
                .filter(|(_, p)| p.model == hash)
                .map(|(e, _)| format!("{e:#x}"))
                .collect();
            let alone: Vec<String> = (members.iter())
                .filter(|m| self.shown.contains_key(m))
                .map(|m| format!("{m:#x}"))
                .collect();
            let all: Vec<String> = members.iter().map(|m| format!("{m:#x}")).collect();
            eprintln!(
                "props: {hash:08x} (put up as {}) is a merged batch of {} [{}]; shown on their \
                 own: [{}]; not drawn while any is",
                users.join(" "),
                members.len(),
                all.join(" "),
                alone.join(" ")
            );
        }
        self.models.insert(
            hash,
            Some(PropModel {
                gpu,
                collision: cpu.collision,
                breakup: cpu.breakup,
                members,
            }),
        );
        Ok(())
    }

    /// Moves on `dt` seconds: what breaks apart sinks.
    pub fn advance(&mut self, dt: f32) {
        for s in self.sinking.values_mut() {
            s.time += dt;
        }
    }

    /// Draws those `frustum` may see within their reach of `eye`, as the
    /// island's props are drawn: each no further than [`PROP_SIZES`] times
    /// its size, nor than `limit`; those knocked loose or broken apart with
    /// their pieces where `physics` has them.
    pub fn draw(
        &self,
        queue: &mut DrawQueue,
        physics: &Physics,
        eye: Vec3,
        frustum: &Frustum,
        limit: f32,
    ) {
        for (e, p) in &self.shown {
            let Some(Some(m)) = self.models.get(&p.model) else {
                continue;
            };
            if self.skip == Some(*e) {
                continue;
            }
            // A merged batch whose members stand (or lie broken) as props
            // of their own would draw them whole a second time.
            if m.members.iter().any(|x| self.shown.contains_key(x)) {
                continue;
            }
            let moved = match (self.broken.get(e), self.knocked.get(e)) {
                (Some(pieces), _) => Some((m.breakup.as_deref().map(|b| &b.pieces), pieces)),
                (None, Some(pieces)) => Some((self.loose(*e), pieces)),
                (None, None) => None,
            };
            if let Some((loose, pieces)) = moved {
                let Some(loose) = loose else {
                    continue;
                };
                let places: Vec<Mat4> = pieces
                    .iter()
                    .filter_map(|&d| physics.debris_place(d))
                    .collect();
                if places.len() == pieces.len()
                    && places
                        .iter()
                        .any(|at| at.w_axis.truncate().distance(eye) <= limit)
                {
                    let placed = Mat4::from_rotation_translation(p.rotation, p.position);
                    let mut nodes = loose.posed(placed, &places);
                    let flying = loose.flying();
                    // The rest of a model broken apart goes down as it
                    // sinks.
                    if let Some(sunk) = self.sinking.get(e).map(|s| s.place(m.gpu.bounds)) {
                        for (n, at) in nodes.iter_mut().enumerate() {
                            if !flying.get(n).copied().unwrap_or(false) {
                                *at = sunk * *at;
                            }
                        }
                    }
                    let wrecked = loose.breaks || p.wrecked;
                    match self.parts {
                        Parts::All => m.gpu.draw_moved(queue, placed, &nodes, wrecked, flying),
                        Parts::Rest => m.gpu.draw_moved(queue, placed, &nodes, wrecked, &[]),
                        Parts::Pieces => m.gpu.draw_flying(queue, placed, &nodes, flying),
                    }
                }
                continue;
            }
            let (lo, hi) = m.gpu.bounds;
            let size = (hi - lo).length();
            let reach = (size * PROP_SIZES).min(limit);
            let centre = p.position + p.rotation * ((lo + hi) * 0.5);
            if centre.distance_squared(eye) > reach * reach || !frustum.sees(centre, size * 0.5) {
                continue;
            }
            let at = Mat4::from_rotation_translation(p.rotation, p.position);
            match self.poses.get(e).filter(|_| !p.wrecked) {
                Some(nodes) => m.gpu.draw_moved(queue, at, nodes, false, &[]),
                None => m.gpu.draw_as(queue, at, p.wrecked),
            }
        }
    }
}

/// How a building a support strike brings down goes as its pieces fly,
/// as its template's `BuildingCollapseAnim` has it (the gates':
/// `SinkAccel` 2.5, `SinkSpeedMax` -1, no limit, `LeanAngleMin` 8 and
/// `LeanAngleMax` 10; their part states call the breaking
/// `CollapseState`, and each piece's `DebrisState` makes it a root of its
/// own, `SetRootNode`, so the pieces don't go down with the rest): the
/// rest sinks into the ground ever faster (this many metres a second more
/// each second), leaning over as it goes to between the least and the
/// most (degrees), away from the blast, until it is this far (metres)
/// down, where it stays: the parts' `DestroyedState` shows the wreck
/// (node 2) again, and the videos keep column stumps and broken wall
/// either side of the gap. Round 102 found the whole gate seen standing
/// after the bomb was the layer's merged batch, not the wreck; how far
/// the wreck goes down is invented.
const SINK_ACCEL: f32 = 2.5;
const LEAN_LEAST: f32 = 8.0;
const LEAN_MOST: f32 = 10.0;
const SINK_MOST: f32 = 1.0;

/// The rest of a model broken apart, going down ([`SINK_ACCEL`]).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sinking {
    /// Seconds since it broke.
    time: f32,
    /// How far over it leans once under, radians, and about which axis
    /// (model space, level).
    lean: f32,
    axis: Vec3,
}

impl Sinking {
    /// Going down from now, the model placed at `placed` with its box
    /// `bounds`, leaning across its line (as a gate's pieces fall) away
    /// from a blast at `from` (either way for one on its line).
    fn from(entity: u32, placed: Mat4, (lo, hi): (Vec3, Vec3), from: Vec3) -> Self {
        let middle = placed.transform_point3((lo + hi) * 0.5);
        let off = (middle - from).dot(placed.z_axis.truncate());
        let side = if off.abs() > 0.3 {
            off.signum()
        } else if scatter(entity, 0) < 0.0 {
            -1.0
        } else {
            1.0
        };
        let lean = LEAN_LEAST + (LEAN_MOST - LEAN_LEAST) * (scatter(entity, 1) + 0.5);
        Self {
            time: 0.0,
            lean: lean.to_radians(),
            // Its top goes along `side` of its z.
            axis: Vec3::X * side,
        }
    }

    /// Already down (one found broken).
    fn under() -> Self {
        Self {
            time: f32::INFINITY,
            lean: 0.0,
            axis: Vec3::X,
        }
    }

    /// How far down it is now (metres), at most [`SINK_MOST`].
    fn depth(&self) -> f32 {
        let depth = 0.5 * SINK_ACCEL * self.time * self.time;
        if depth.is_nan() {
            SINK_MOST
        } else {
            depth.min(SINK_MOST)
        }
    }

    /// Where the rest of the model, its box `bounds`, is now, in its own
    /// space: sunk, and leaning by as much of its lean as of
    /// [`SINK_MOST`] it has gone down.
    fn place(&self, (lo, hi): (Vec3, Vec3)) -> Mat4 {
        let depth = self.depth();
        let pivot = Vec3::new((lo.x + hi.x) * 0.5, lo.y, (lo.z + hi.z) * 0.5);
        let lean = self.lean * depth / SINK_MOST;
        Mat4::from_translation(pivot - Vec3::Y * depth)
            * Mat4::from_axis_angle(self.axis, lean)
            * Mat4::from_translation(-pivot)
    }
}

/// How a broken-apart model's pieces go from a blast on top of it (the
/// satellite bomb lands on a gate's roof), as the videos have the gates
/// (round 96): the gatehouse blown apart at the flash, its blocks thrown
/// up and out, the big ones least far; the slabs near it tipping over to
/// lie across the way through; broken wall left standing further off. A
/// piece within [`THROW_NEAR`] of the blast flies at [`THROW_SPEED`] (m/s)
/// if it is no more than [`THROW_SIZE`] across, slower the bigger (as the
/// [`THROW_BULK`] power of its size) and the further off (as the
/// [`THROW_FALL`] power of how much further), at most [`THROW_MOST`]; none
/// fly beyond [`THROW_REACH`]. Each also tips over outward, across the
/// way through, on its outer lower edge: within [`THROW_NEAR`], spinning
/// [`TOPPLE`] times as fast as takes it past its balance, less the further
/// off, not at all from [`TOPPLE_REACH`]; and tumbles as fast (radians a
/// second) as [`TUMBLE`] times its speed over its size. Round 100's throw
/// (9 m/s for a metre-wide piece, a third of that for a gate's blocks,
/// each spun about its middle, its footings left in the ground) left the
/// arch and the walls standing 8 s on; round 102's 12 m/s threw pieces
/// 30-40 m, where the videos land them 10-15 m out. Invented, to look as
/// the videos do.
const THROW_SPEED: f32 = 7.5;
const THROW_SIZE: f32 = 3.0;
const THROW_BULK: f32 = 0.75;
const THROW_NEAR: f32 = 6.0;
const THROW_FALL: f32 = 1.5;
const THROW_MOST: f32 = 20.0;
const THROW_REACH: f32 = 45.0;
const TOPPLE: f32 = 1.3;
const TOPPLE_REACH: f32 = 25.0;
const TUMBLE: f32 = 0.3;

/// A piece that starts in the ground (a gate's footings) is lifted out
/// of it as it breaks, under the flash, by at most this much (metres):
/// one left in it sticks there.
const UNBURY_MOST: f32 = 4.0;

/// Throws `breakup`'s pieces, the model placed at `placed`, away from
/// `blast`, `scale` times as fast as [`THROW_SPEED`] has it: out of its
/// line (a gate's pieces fall on the way through it, each to its own
/// side, as in the videos), each lifted out of the ground it starts in.
/// `None` (nothing thrown) without a blast within [`THROW_REACH`] of its
/// middle.
fn throw(
    physics: &mut Physics,
    breakup: &Breakup,
    placed: Mat4,
    blast: Option<Vec3>,
    scale: f32,
) -> Option<Vec<DebrisId>> {
    let loose = &breakup.pieces;
    let starts = loose.starts(placed);
    let boxes: Vec<[Vec3; 8]> = loose
        .pieces
        .iter()
        .zip(&starts)
        .map(|(q, &at)| corners(at, q))
        .collect();
    let middle = boxes.iter().map(box_middle).sum::<Vec3>() / boxes.len().max(1) as f32;
    let from = blast.filter(|b| b.distance(middle) < THROW_REACH)?;
    // Across its line: the way through a gate.
    let across = placed.z_axis.truncate().with_y(0.0).normalize_or(Vec3::Z);
    let pieces = loose
        .pieces
        .iter()
        .zip(starts)
        .zip(&boxes)
        .enumerate()
        .map(|(i, ((piece, at), corners))| {
            let lift = Mat4::from_translation(Vec3::Y * buried(physics, corners));
            let (velocity, spin) = launch(i, corners, (middle, across), from, scale);
            physics.add_rubble(piece, lift * at, velocity, spin)
        })
        .collect();
    Some(pieces)
}

/// The corners of `piece`'s box, its node placed at `at`.
fn corners(at: Mat4, piece: &m2_engine::LoosePiece) -> [Vec3; 8] {
    let (centre, half) = (piece.centre(), piece.extent() * 0.5);
    std::array::from_fn(|k| {
        let pick = |bit: usize| if k & bit == 0 { -1.0 } else { 1.0 };
        at.transform_point3(centre + half * Vec3::new(pick(1), pick(2), pick(4)))
    })
}

/// The world box round the model box `lo` to `hi` placed at `at`.
fn box_around(at: Mat4, (lo, hi): (Vec3, Vec3)) -> (Vec3, Vec3) {
    (0..8)
        .map(|i| {
            at.transform_point3(Vec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            ))
        })
        .fold((Vec3::MAX, Vec3::MIN), |(a, b), c| (a.min(c), b.max(c)))
}

/// The middle of a box with these `corners`.
fn box_middle(corners: &[Vec3; 8]) -> Vec3 {
    corners.iter().sum::<Vec3>() / 8.0
}

/// How far (metres) a box with these `corners` starts in the island's
/// ground: its lowest corner's depth under the highest ground beneath its
/// four lowest corners and its middle, at most [`UNBURY_MOST`].
fn buried(physics: &Physics, corners: &[Vec3; 8]) -> f32 {
    let (bottom, top) = corners
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), c| (a.min(c.y), b.max(c.y)));
    let mut low = *corners;
    low.sort_by(|a, b| a.y.total_cmp(&b.y));
    let reach = top - bottom + UNBURY_MOST + 2.0;
    low[..4]
        .iter()
        .copied()
        .chain([box_middle(corners)])
        .filter_map(|c| physics.island_below(c.with_y(top + 1.0), reach))
        .map(|ground| ground - bottom)
        .fold(0.0, f32::max)
        .min(UNBURY_MOST)
}

/// How piece `i` of a model broken apart goes from a blast at `from`,
/// `scale` times as fast as [`THROW_SPEED`] has it: its velocity and spin,
/// its box's `corners` as it starts, the model's middle and the way across
/// its line `(middle, across)`.
fn launch(
    i: usize,
    corners: &[Vec3; 8],
    (middle, across): (Vec3, Vec3),
    from: Vec3,
    scale: f32,
) -> (Vec3, Vec3) {
    let jitter = |k: u32| scatter(i as u32, k);
    let centre = box_middle(corners);
    let size = corners[0].distance(corners[7]).max(0.5);
    let away = centre - from;
    let flat = away.with_y(0.0).normalize_or(Vec3::X);
    // Out on its own side of the line (either, on it).
    let off = (centre - middle).dot(across);
    let side = if off.abs() > 0.3 {
        off.signum()
    } else if jitter(5) < 0.0 {
        -1.0
    } else {
        1.0
    };
    let out = (across * side + flat * 0.5).normalize_or(across * side);
    let far = away.length();
    let fall = (THROW_NEAR / far.max(THROW_NEAR)).powf(THROW_FALL);
    let bulk = (THROW_SIZE / size.max(THROW_SIZE)).powf(THROW_BULK);
    let speed = (THROW_SPEED * bulk * fall * scale).min(THROW_MOST);
    let thrown =
        out * (speed * (1.0 + 0.3 * jitter(0))) + Vec3::Y * (speed * (0.7 + 0.3 * jitter(1)));
    // Tipping its top out: how tall it stands and how deep it is along
    // `out`, and the least spin that takes it over its outer lower edge
    // (its middle rising by `rise` as it goes over).
    let span = |axis: Vec3| {
        let (lo, hi) = corners
            .iter()
            .map(|c| c.dot(axis))
            .fold((f32::MAX, f32::MIN), |(a, b), x| (a.min(x), b.max(x)));
        (hi - lo).max(0.05)
    };
    let (height, depth) = (span(Vec3::Y), span(out));
    let rise = (height * 0.5).hypot(depth * 0.5) - height * 0.5;
    let least =
        (6.0 * m2_engine::physics::GRAVITY * rise / (height * height + depth * depth)).sqrt();
    let near = ((TOPPLE_REACH - far) / (TOPPLE_REACH - THROW_NEAR)).clamp(0.0, 1.0);
    let tip = Vec3::Y.cross(out) * (least * TOPPLE * near);
    // Its middle swings out and up about that edge.
    let swing = tip.cross(Vec3::Y * (height * 0.5) - out * (depth * 0.5));
    let tumble = Vec3::new(jitter(2), jitter(3), jitter(4)) * (TUMBLE * speed / size);
    (thrown + swing, tip + tumble)
}

/// A number from -0.5 to 0.5 for piece `i`, `k`th of its kind: the same
/// every time, different for each.
fn scatter(i: u32, k: u32) -> f32 {
    let x = i.wrapping_mul(0x9e37_79b9) ^ k.wrapping_mul(0x85eb_ca6b);
    let x = (x ^ (x >> 15)).wrapping_mul(0x2c1b_3c6d);
    ((x ^ (x >> 12)) & 0xffff) as f32 / 65535.0 - 0.5
}

impl Loaders {
    /// Starts [`LOADERS`] threads, each reading `install` on its own and
    /// decoding the models (with their collision) asked of it.
    fn start(install: &Install) -> Self {
        let (ask, asked) = channel::<u32>();
        let asked = Arc::new(Mutex::new(asked));
        let (tx, done) = channel();
        for _ in 0..LOADERS {
            let (install, asked, tx) = (install.clone(), asked.clone(), tx.clone());
            std::thread::spawn(move || {
                let mut source = install.open_world().map_err(|e| e.to_string());
                loop {
                    // Held only while waiting for the next ask.
                    let next = match asked.lock() {
                        Ok(asked) => asked.recv(),
                        Err(_) => return,
                    };
                    // The game has quit.
                    let Ok(m) = next else {
                        return;
                    };
                    // One model that can't be read (even one that panics)
                    // doesn't hold up the rest.
                    let cpu = match &mut source {
                        Ok(source) => std::panic::catch_unwind(AssertUnwindSafe(|| {
                            source
                                .model(m)
                                .and_then(|data| {
                                    let mut cpu = CpuModel::from_data(&data, 0)?;
                                    let parsed = data.parse()?;
                                    cpu.breakup = Breakup::of(&parsed, 0).map(Arc::new);
                                    cpu.decode_textures(source, MODEL_TEXTURE_WIDTH);
                                    // A merged batch is in no one's way: its
                                    // members are, as their own props (the
                                    // island's batches are the same).
                                    let mut members = Vec::new();
                                    if parsed.info.flags & ModelInfo::FLAG_TINY != 0 {
                                        cpu.collision = None;
                                        members = parsed.tiny_members();
                                    }
                                    Ok((cpu, members))
                                })
                                .map_err(|e| e.to_string())
                        }))
                        .unwrap_or_else(|_| Err("panicked while decoding".to_owned())),
                        Err(e) => Err(e.clone()),
                    };
                    if tx.send((m, cpu)).is_err() {
                        return;
                    }
                }
            });
        }
        Self { ask, done }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_engine::{CollisionShape, LoosePiece};

    #[test]
    fn a_broken_gates_rest_sinks_a_little_leaning_and_stays() {
        // A gate 64 m along x, 15.7 m high, the way through it along z.
        let bounds = (Vec3::new(-32.0, 0.0, -4.6), Vec3::new(32.0, 15.7, 4.7));
        let placed = Mat4::from_translation(Vec3::new(100.0, -3.0, 50.0));
        // The bomb on its top, a little to its -z side.
        let mut s = Sinking::from(7, placed, bounds, Vec3::new(100.0, 12.0, 49.0));
        let lean = s.lean.to_degrees();
        assert!((LEAN_LEAST..=LEAN_MOST).contains(&lean), "{lean}");
        // The middle of its foot, which it leans about.
        let foot = Vec3::new(0.0, 0.0, 0.05);
        let top = |s: &Sinking| s.place(bounds).transform_point3(Vec3::new(0.0, 15.7, 0.0));
        assert!(top(&s).distance(Vec3::new(0.0, 15.7, 0.0)) < 1e-4);
        // Half a second on, a little down, leaning away from the blast.
        s.time = 0.5;
        let at = top(&s);
        assert!(at.y < 15.7 && at.y > 14.5 && at.z > 0.1, "{at}");
        // From a second on it stays: SINK_MOST down, fully over.
        s.time = 1.0;
        let down = s.place(bounds).transform_point3(foot);
        assert!((down.y + SINK_MOST).abs() < 1e-4, "{down}");
        let still = top(&s);
        assert!(still.z > at.z && still.y > 13.0, "{still}");
        s.time = 60.0;
        assert!(top(&s).distance(still) < 1e-4);
        // One found broken is down already, standing straight.
        let under = Sinking::under().place(bounds).transform_point3(foot);
        assert!((under.y + SINK_MOST).abs() < 1e-4, "{under}");
    }

    #[test]
    fn a_gates_slabs_topple_out_across_the_way_through() {
        let mut physics = Physics::new();
        let half = 100.0;
        let v = [
            Vec3::new(-half, 0.0, -half),
            Vec3::new(half, 0.0, -half),
            Vec3::new(half, 0.0, half),
            Vec3::new(-half, 0.0, half),
        ];
        let ground = CollisionShape::mesh(&v, &[[0, 1, 2], [0, 2, 3]]).unwrap();
        physics.add_static(StaticKey::Cell(1), &ground, Vec3::ZERO, Quat::IDENTITY);
        physics.step(1.0 / 60.0);
        // A gate along x, the way through it along z; the bomb on its top.
        // Two door slabs 5 m wide, 6 m high and 1.5 m thick, either side
        // of its middle, and a block on its line.
        let middle = Vec3::new(0.0, 3.0, 0.0);
        let from = Vec3::new(0.0, 9.0, 0.0);
        let parts = [
            (Vec3::new(-2.6, 3.0, 0.76), Vec3::new(2.5, 3.0, 0.75)),
            (Vec3::new(2.6, 3.0, -0.76), Vec3::new(2.5, 3.0, 0.75)),
            (Vec3::new(8.0, 7.0, 0.0), Vec3::new(0.6, 0.5, 0.6)),
        ];
        let thrown: Vec<_> = parts
            .iter()
            .enumerate()
            .map(|(i, &(centre, half))| {
                let piece = LoosePiece::block(None, centre, half, 1000.0);
                let box_at = corners(Mat4::IDENTITY, &piece);
                let (velocity, spin) = launch(i, &box_at, (middle, Vec3::Z), from, 1.0);
                physics.add_rubble(&piece, Mat4::IDENTITY, velocity, spin)
            })
            .collect();
        for _ in 0..240 {
            physics.step(1.0 / 60.0);
        }
        for (k, (&id, &(centre, _))) in thrown.iter().zip(&parts).enumerate() {
            let place = physics.debris_place(id).unwrap();
            let at = place.transform_point3(centre);
            let up = place.transform_vector3(Vec3::Y);
            if k < 2 {
                // Lying on the ground, out on its own side of the line.
                assert!(up.y.abs() < 0.5, "slab {k} still stands: {up}");
                assert!(at.y < 1.5, "slab {k} at {at}");
                assert!(
                    at.z * centre.z.signum() > 2.0 && at.z.abs() < 15.0,
                    "slab {k} at {at}"
                );
            } else {
                // Thrown up and out: off the line, not far.
                assert!(at.z.abs() > 2.0 && at.length() < 30.0, "block at {at}");
            }
        }
    }

    #[test]
    fn a_gates_footings_come_out_of_the_ground_and_its_far_walls_stand() {
        let mut physics = Physics::new();
        let half = 100.0;
        let v = [
            Vec3::new(-half, 0.0, -half),
            Vec3::new(half, 0.0, -half),
            Vec3::new(half, 0.0, half),
            Vec3::new(-half, 0.0, half),
        ];
        let ground = CollisionShape::mesh(&v, &[[0, 1, 2], [0, 2, 3]]).unwrap();
        physics.add_static(StaticKey::Cell(1), &ground, Vec3::ZERO, Quat::IDENTITY);
        physics.step(1.0 / 60.0);
        // A gate along x, the bomb on its top. Wall slabs 5 m wide, 6 m
        // high and 1.5 m thick, their footings a metre in the ground: one
        // by the gatehouse, one 30 m along the wall.
        let middle = Vec3::new(0.0, 2.0, 0.0);
        let from = Vec3::new(0.0, 9.0, 0.0);
        let parts = [
            (Vec3::new(4.0, 2.0, 0.76), Vec3::new(2.5, 3.0, 0.75)),
            (Vec3::new(30.0, 2.0, 0.76), Vec3::new(2.5, 3.0, 0.75)),
        ];
        let thrown: Vec<_> = parts
            .iter()
            .enumerate()
            .map(|(i, &(centre, half))| {
                let piece = LoosePiece::block(None, centre, half, 1000.0);
                let box_at = corners(Mat4::IDENTITY, &piece);
                let lift = buried(&physics, &box_at);
                assert!((lift - 1.0).abs() < 0.05, "slab {i} lifted {lift}");
                let (velocity, spin) = launch(i, &box_at, (middle, Vec3::Z), from, 1.0);
                let at = Mat4::from_translation(Vec3::Y * lift);
                physics.add_rubble(&piece, at, velocity, spin)
            })
            .collect();
        for _ in 0..240 {
            physics.step(1.0 / 60.0);
        }
        let place = |k: usize| physics.debris_place(thrown[k]).unwrap();
        // The one by the gatehouse lies across the way through.
        let near = place(0);
        assert!(near.transform_vector3(Vec3::Y).y.abs() < 0.5, "{near}");
        assert!(near.transform_point3(parts[0].0).z > 2.0, "{near}");
        // The far one stands where it stood, out of the ground.
        let far = place(1);
        let up = far.transform_vector3(Vec3::Y);
        let at = far.transform_point3(parts[1].0);
        assert!(up.y > 0.9, "{up}");
        assert!(
            at.with_y(0.0).distance(parts[1].0.with_y(0.0)) < 1.5 && (at.y - 3.0).abs() < 0.3,
            "{at}"
        );
    }
}
