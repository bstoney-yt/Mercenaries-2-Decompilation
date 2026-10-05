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
    /// Knocked loose since [`take_knocked`](Self::take_knocked).
    fresh_knocks: Vec<u32>,
    /// Those only a support strike brings down (a mission's gates): no
    /// vehicle or common blast knocks them loose.
    pub sturdy: HashSet<u32>,
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

/// A model decoded on a loading thread, or why not.
type Load = (u32, Result<CpuModel, String>);

/// The loading threads: models are asked by hash and come back decoded.
#[derive(Debug)]
struct Loaders {
    ask: Sender<u32>,
    done: Receiver<Load>,
}

impl Props {
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
                    wrecked: false,
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

    /// Takes down those of `entities` that are shown (their loose pieces
    /// too).
    pub fn hide(&mut self, physics: &mut Physics, entities: impl IntoIterator<Item = u32>) {
        for e in entities {
            if self.shown.remove(&e).is_some() {
                physics.remove_static(StaticKey::Entity(e));
                self.changes += 1;
            }
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
    /// [`THROW_REACH`] (one found broken, as after loading a game, lies
    /// wrecked). True if it was one.
    pub fn wreck(&mut self, physics: &mut Physics, entity: u32, blast: Option<Vec3>) -> bool {
        let Some(p) = self.shown.get_mut(&entity).filter(|p| !p.wrecked) else {
            return false;
        };
        physics.remove_static(StaticKey::Entity(entity));
        let model = self.models.get(&p.model).and_then(Option::as_ref);
        if model.is_some_and(|m| m.gpu.has_wreck()) {
            p.wrecked = true;
            let placed = Mat4::from_rotation_translation(p.rotation, p.position);
            if let Some(breakup) = model.and_then(|m| m.breakup.as_deref())
                && !self.knocked.contains_key(&entity)
                && let Some(pieces) = throw(physics, breakup, placed, blast)
            {
                eprintln!(
                    "props: {entity:#x} ({:08x}) breaks apart into {} pieces, away from \
                     {:.1}, burning at {} points",
                    p.model,
                    pieces.len(),
                    blast.unwrap_or_default(),
                    breakup.points.len()
                );
                self.broken.insert(entity, pieces);
            }
        } else if !self.knocked.contains_key(&entity) {
            // Loose pieces stay where they fell.
            self.shown.remove(&entity);
            self.changes += 1;
        }
        true
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

    /// The middle of `entity`'s model's box in the world; `None` unless it
    /// is shown and its model has loaded.
    pub fn centre(&self, entity: u32) -> Option<Vec3> {
        let p = self.shown.get(&entity)?;
        let m = self.models.get(&p.model)?.as_ref()?;
        let (lo, hi) = m.gpu.bounds;
        Some(p.position + p.rotation * ((lo + hi) * 0.5))
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
        cpu: Result<CpuModel, String>,
    ) -> anyhow::Result<()> {
        let cpu = match cpu {
            Ok(cpu) => cpu,
            Err(e) => {
                eprintln!("state layer model {hash:08x}: {e}");
                self.failed.insert(hash);
                return Ok(());
            }
        };
        materials.load_textures(backend, source, cpu.texture_hashes().collect::<Vec<_>>())?;
        let gpu = GpuModel::upload(backend, materials, &cpu)?;
        if let Some(shape) = &cpu.collision {
            for (&e, p) in self.shown.iter().filter(|(_, p)| p.model == hash) {
                if !p.wrecked && !self.knocked.contains_key(&e) {
                    physics.add_static(StaticKey::Entity(e), shape, p.position, p.rotation);
                }
            }
        }
        self.models.insert(
            hash,
            Some(PropModel {
                gpu,
                collision: cpu.collision,
                breakup: cpu.breakup,
            }),
        );
        Ok(())
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
                    m.gpu.draw_moved(
                        queue,
                        placed,
                        &loose.posed(placed, &places),
                        loose.breaks || p.wrecked,
                        loose.flying(),
                    );
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
            m.gpu.draw_as(queue, at, p.wrecked);
        }
    }
}

/// A broken-apart model's pieces fly this fast (m/s) for a piece a metre
/// across at the blast, slower the bigger (as the square root of its
/// size) and the further they are, up to this far (metres) and at most
/// this fast; and turn this fast (radians a second) for a piece a metre
/// across, the biggest at least this fast. Invented, to look as the
/// videos' gates do (round 96): blocks thrown up and out at once, slabs
/// toppling over in two seconds or so to lie across the way through.
const THROW_SPEED: f32 = 9.0;
const THROW_REACH: f32 = 45.0;
const THROW_MOST: f32 = 10.0;
const THROW_TURN: f32 = 6.0;
const THROW_TURN_LEAST: f32 = 0.9;

/// Throws `breakup`'s pieces, the model placed at `placed`, away from
/// `blast`, as [`THROW_SPEED`] has it: out of its line (a gate's pieces
/// fall on the way through it, each to its own side, as in the videos),
/// the biggest tipping over as they go, the smaller tumbling. `None`
/// (nothing thrown) without a blast within [`THROW_REACH`] of its middle.
fn throw(
    physics: &mut Physics,
    breakup: &Breakup,
    placed: Mat4,
    blast: Option<Vec3>,
) -> Option<Vec<DebrisId>> {
    let loose = &breakup.pieces;
    let starts = loose.starts(placed);
    let centres: Vec<Vec3> = loose
        .pieces
        .iter()
        .zip(&starts)
        .map(|(q, at)| at.transform_point3(q.centre()))
        .collect();
    let middle = centres.iter().sum::<Vec3>() / centres.len().max(1) as f32;
    let from = blast.filter(|b| b.distance(middle) < THROW_REACH)?;
    // Across its line: the way through a gate.
    let across = placed.z_axis.truncate().with_y(0.0).normalize_or(Vec3::Z);
    let pieces = loose
        .pieces
        .iter()
        .zip(starts)
        .zip(centres)
        .enumerate()
        .map(|(i, ((piece, at), centre))| {
            let (velocity, spin) = launch(i, centre, piece.size(), (middle, across), from);
            physics.add_rubble(piece, at, velocity, spin)
        })
        .collect();
    Some(pieces)
}

/// How piece `i` of a model broken apart flies from a blast at `from`:
/// its velocity and spin, its middle at `centre` and `size` across, the
/// model's middle and the way across its line `(middle, across)`.
fn launch(
    i: usize,
    centre: Vec3,
    size: f32,
    (middle, across): (Vec3, Vec3),
    from: Vec3,
) -> (Vec3, Vec3) {
    let jitter = |k: u32| scatter(i as u32, k);
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
    let out = (across * side + flat * 0.6).normalize_or(across * side);
    let near = (1.0 - away.length() / THROW_REACH).clamp(0.25, 1.0);
    let size = size.max(0.5);
    let speed = (THROW_SPEED / size.sqrt()).min(THROW_MOST) * near;
    let velocity =
        out * (speed * (1.0 + 0.4 * jitter(0))) + Vec3::Y * (speed * (0.6 + 0.4 * jitter(1)) + 1.0);
    // Tipping its top out, tumbling as it goes.
    let turn = (THROW_TURN / size).clamp(THROW_TURN_LEAST, 4.0);
    let spin =
        Vec3::Y.cross(out) * turn + Vec3::new(jitter(2), jitter(3), jitter(4)) * (turn * 0.4);
    (velocity, spin)
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
                                    cpu.breakup = Breakup::of(&data.parse()?, 0).map(Arc::new);
                                    Ok(cpu)
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
                let (velocity, spin) = launch(i, centre, piece.size(), (middle, Vec3::Z), from);
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
}
