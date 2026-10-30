//! Models the state layers place while they are in: a mission's gates,
//! barricades and crates ([`WorldIndex::state_objects`]). They stand once
//! the scripts load their layer (or the start-up did), go when it
//! unloads, and break when the scripts' object dies: drawn as their wreck
//! when the model has one, else gone, and no longer in the way; one that
//! breaks apart ([`Breakup`], a gate) throws its pieces, which lie where
//! they land. Those whose model comes loose ([`Loose`]) are knocked loose
//! by vehicles and blasts, and vehicles break them as their template says
//! ([`crate::destruct`]), as the island's are.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::panic::AssertUnwindSafe;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use m2_assets::model::ModelInfo;
use m2_engine::durability::BlastPush;
use m2_engine::model::MODEL_TEXTURE_WIDTH;
use m2_engine::stream::PROP_SIZES;
use m2_engine::world::ObjectPlacement;
use m2_engine::{
    AssetSource, Breakup, CollisionShape, CpuModel, DebrisId, Frustum, GpuModel, Install, Loose,
    MaterialCache, Physics, StaticKey, Templates, WorldIndex,
};
use m2_math::glam::{Mat4, Quat, Vec3};
use m2_render::{DrawQueue, RenderBackend};

/// A vehicle whose up axis leans further than this from the vertical
/// (its height, of 1) has tipped: what rides on it comes off (invented).
const TIPPED: f32 = 0.5;

/// One shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prop {
    pub model: u32,
    pub position: Vec3,
    pub rotation: Quat,
    /// Broken: drawn as its wreck, and nothing to bump into.
    pub wrecked: bool,
    /// The template its entity is an instance of (0 for none): vehicles
    /// break it as its `PhysicsPropertyCrashable` says, as they do the
    /// island's objects of it ([`crate::destruct`]).
    pub template: u32,
    /// Placed by its entity's name rather than a `ModelName`: on the
    /// island one whose model has no collision of its own is in nobody's
    /// way; here it is only logged ([`through_on_island`]).
    pub named: bool,
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
    /// Those of [`changes`](Self::changes) that put props up or took
    /// them down (not the scripts' moves, which may come every tick).
    placed: u64,
    /// The nearness [`counts`](Self::counts) worked out of the waiting
    /// models, as of change [`placed`](Self::placed): taken again only
    /// when props are put up or taken down.
    fresh: std::cell::RefCell<Option<(u64, HashMap<u32, f32>)>>,
    /// How far from the focus models were last wanted (metres).
    reach: f32,
    /// Those knocked loose: their pieces, in [`Loose`] order.
    knocked: HashMap<u32, Vec<DebrisId>>,
    /// Those riding on a vehicle's load bed (PirCon002's rum jugs on its
    /// truck): the vehicle (its index among the scene's, which stays when
    /// it is placed afresh), and where on it, in its space
    /// ([`Props::load`]).
    cargo: HashMap<u32, (usize, Mat4)>,
    /// Those broken apart: their pieces, in their [`Breakup`]'s order.
    broken: HashMap<u32, Vec<DebrisId>>,
    /// Those wrecked by a blast: their intact parts going down into the
    /// ground ([`SINK_ACCEL`]) as their wreck shows.
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
    /// How far any loaded model reaches from its origin, metres (its
    /// box's middle, and half its size beyond).
    widest: f32,
    /// The templates whose objects are physics props (`_PropPhysics`), by
    /// id: those whose model has no body that moves come loose as one.
    pub physics_props: HashMap<u32, m2_engine::durability::PropPhysics>,
    /// Shown since their model came in, their collision not yet up.
    unwalled: HashSet<u32>,
    /// Those a vehicle broke some parts of, a part at a time (a hedge's
    /// sections, a gate's leaf): which parts, and the nodes that hides.
    parts_down: HashMap<u32, PartsDown>,
    /// What each piece of a prop weighs once it breaks apart, kg, by
    /// entity, where its template says ([`m2_engine::durability::piece_masses`]):
    /// its model's pieces otherwise weigh as stone of their size.
    piece_mass: HashMap<u32, f32>,
    /// How hard blasts push each prop, of their push, by entity, where its
    /// template says (`ExplosionFudge`).
    fudge: HashMap<u32, f32>,
    /// Wrecked since [`take_fallen`](Self::take_fallen): what falls with
    /// them is brought down too (`DestructionLink`).
    fallen: Vec<u32>,
}

/// Which parts of a prop are broken ([`m2_engine::ModelParts`]), and the
/// nodes hidden for it.
#[derive(Clone, Debug, Default, PartialEq)]
struct PartsDown {
    wrecked: Vec<bool>,
    hidden: Vec<bool>,
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

/// What brings a prop down ([`Props::wreck`], [`Props::wreck_crashed`]).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Hit {
    /// A blast, if known: what breaks apart is pushed by it
    /// ([`BlastPush::throw`]), this many times as fast.
    Blast(Option<BlastPush>, f32),
    /// A vehicle running into it: what breaks apart goes on at this
    /// velocity (m/s), turning at this spin (radians a second).
    Crash(Vec3, Vec3),
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
    /// [`Props::placed`] then.
    placed: u64,
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
    /// How many of `entity`'s `parts` (those its template's objects are
    /// broken in, one at a time, by vehicles: a hedge's sections) stand:
    /// none once it is wrecked or gone.
    pub fn parts_standing(&self, entity: u32, parts: usize) -> usize {
        match self.shown.get(&entity) {
            Some(p) if !p.wrecked => {
                let down = self
                    .parts_down
                    .get(&entity)
                    .map_or(0, |d| d.wrecked.iter().filter(|&&w| w).count());
                parts.saturating_sub(down)
            }
            _ => 0,
        }
    }

    /// The parts of `entity` a vehicle breaks one at a time, if its model
    /// has them ([`m2_engine::ModelParts`]) and it stands.
    pub fn model_parts(&self, entity: u32) -> Option<&Arc<m2_engine::ModelParts>> {
        let p = self.shown.get(&entity).filter(|p| !p.wrecked)?;
        self.models.get(&p.model)?.as_ref()?.gpu.parts.as_ref()
    }

    /// Whether any of `entity`'s parts is broken, the rest standing.
    pub fn parts_broken(&self, entity: u32) -> bool {
        self.parts_down
            .get(&entity)
            .is_some_and(|d| d.wrecked.contains(&true))
    }

    /// Whether part `k` of `entity` is broken.
    pub fn part_down(&self, entity: u32, k: usize) -> bool {
        self.parts_down
            .get(&entity)
            .is_some_and(|d| d.wrecked.get(k).copied().unwrap_or(false))
    }

    /// Breaks part `k` of `entity` alone ([`m2_engine::ModelParts`]): it
    /// shows its own wreck (a hedge section's stumps; a gate leaf's
    /// nothing, its pieces not thrown) and its collision goes, the rest
    /// standing. Once every part with something of its own to show is
    /// broken, the whole is wrecked as a vehicle breaks it
    /// ([`wreck_crashed`](Self::wreck_crashed), with `push`). True if it
    /// broke anything.
    pub fn wreck_part(
        &mut self,
        physics: &mut Physics,
        entity: u32,
        k: usize,
        push: impl FnOnce(f32) -> (Vec3, Vec3),
    ) -> bool {
        let Some(parts) = self.model_parts(entity).cloned() else {
            return false;
        };
        if k >= parts.count() || self.part_down(entity, k) {
            return false;
        }
        let down = self.parts_down.entry(entity).or_default();
        down.wrecked.resize(parts.count(), false);
        down.wrecked[k] = true;
        let all = (0..parts.count()).all(|j| down.wrecked[j] || parts.part_box(j).is_none());
        if all {
            self.parts_down.remove(&entity);
            return self.wreck_crashed(physics, entity, push);
        }
        down.hidden = parts.hidden(&down.wrecked);
        let wrecked = down.wrecked.clone();
        self.changed();
        // Its collision without the broken parts' shapes (one cut from
        // its drawn triangles stays whole).
        if !parts.meshes.is_empty()
            && let Some(p) = self.shown.get(&entity)
        {
            physics.remove_static(StaticKey::Entity(entity));
            // The rest stands in the way, an island object put up to
            // break a part at a time too.
            if let Some(shape) = parts.collision(&wrecked) {
                physics.add_static(StaticKey::Entity(entity), &shape, p.position, p.rotation);
            }
        }
        true
    }

    /// Those wrecked since this was last asked (for what falls with them,
    /// `DestructionLink`).
    pub fn take_fallen(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.fallen)
    }

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
            // A ruin stands as its model's wreck from the start.
            if self.put_up(&index.template_db, o, index.ruined.contains(&e)) {
                added += 1;
            }
        }
        if !self.waiting.is_empty() && self.loaders.is_none() {
            self.loaders = Some(Loaders::start(install));
        }
        added
    }

    /// Puts up a prop the scripts make (`Pg.Spawn`), placed as `o` says,
    /// as a layer's would be ([`show`](Self::show)). False if it is up.
    pub fn show_spawned(&mut self, install: &Install, db: &Templates, o: &ObjectPlacement) -> bool {
        let up = self.put_up(db, o, false);
        if !self.waiting.is_empty() && self.loaders.is_none() {
            self.loaders = Some(Loaders::start(install));
        }
        up
    }

    /// Puts up `o` (as its entity), its model asked for. False if it is up.
    fn put_up(&mut self, db: &Templates, o: &ObjectPlacement, wrecked: bool) -> bool {
        let e = o.entity;
        if self.shown.contains_key(&e) {
            return false;
        }
        self.shown.insert(
            e,
            Prop {
                model: o.model,
                position: o.position,
                rotation: o.rotation,
                wrecked,
                template: o.template,
                named: o.named,
            },
        );
        if let Some(&kg) = db.piece_masses.get(&o.template) {
            self.piece_mass.insert(e, kg);
        }
        if let Some(&k) = db.explosion_fudges.get(&o.template) {
            self.fudge.insert(e, k);
        }
        self.changed();
        match self.models.entry(o.model) {
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(None);
                self.waiting.insert(o.model);
            }
            // Its model is in already: its collision goes up at the
            // next update (round 6b found barricades and crates put up
            // again by a later layer standing with none).
            std::collections::hash_map::Entry::Occupied(m) if m.get().is_some() => {
                self.unwalled.insert(e);
            }
            std::collections::hash_map::Entry::Occupied(_) => {}
        }
        true
    }

    /// Puts up one of the island's placed objects as a prop under `key`
    /// (not an entity of the state layers), to bring it down as they are
    /// brought down: its model loads as [`Props::update`] goes
    /// ([`get`](Self::get) says when). False if `key` is already shown.
    pub fn adopt(
        &mut self,
        install: &Install,
        key: u32,
        o: &m2_engine::world::ObjectPlacement,
    ) -> bool {
        if self.shown.contains_key(&key) {
            return false;
        }
        self.shown.insert(
            key,
            Prop {
                model: o.model,
                position: o.position,
                rotation: o.rotation,
                wrecked: false,
                template: o.template,
                named: o.named,
            },
        );
        self.changed();
        if let std::collections::hash_map::Entry::Vacant(slot) = self.models.entry(o.model) {
            slot.insert(None);
            self.waiting.insert(o.model);
        }
        if self.loaders.is_none() {
            self.loaders = Some(Loaders::start(install));
        }
        true
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
                self.changed();
            }
            self.parts_down.remove(&e);
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
        mass.is_some() && self.fling(physics, entity, velocity, spin)
    }

    /// Puts `entity` on the load bed of vehicle `index` (placed as `id`)
    /// where it stands, raised to at least `floor` (the bed's height in
    /// the vehicle's space): it rides there ([`carry`](Self::carry)) until
    /// the vehicle tips over. Its standing collision goes while it rides.
    /// False unless both are in play.
    pub fn load(
        &mut self,
        physics: &mut Physics,
        entity: u32,
        (index, id): (usize, m2_engine::VehicleId),
        floor: f32,
    ) -> bool {
        let (Some(p), Some(t)) = (self.shown.get(&entity), physics.vehicle_transform(id)) else {
            return false;
        };
        let mut local = t.inverse() * Mat4::from_rotation_translation(p.rotation, p.position);
        local.w_axis.y = local.w_axis.y.max(floor);
        physics.remove_static(StaticKey::Entity(entity));
        self.cargo.insert(entity, (index, local));
        true
    }

    /// Whether anything rides on vehicle `index`.
    pub fn carrying(&self, index: usize) -> bool {
        self.cargo.values().any(|(v, _)| *v == index)
    }

    /// Whether `entity` rides on a vehicle ([`load`](Self::load)).
    pub fn is_cargo(&self, entity: u32) -> bool {
        self.cargo.contains_key(&entity)
    }

    /// Moves what rides on vehicles with them (`placed`: each vehicle's
    /// physics id, by index, while it is placed). What rides on one
    /// tipped past [`TIPPED`] comes off: returned, with where it is, to be
    /// knocked loose. One not placed for the moment (being placed afresh)
    /// keeps its load.
    pub fn carry(
        &mut self,
        physics: &Physics,
        placed: &[Option<m2_engine::VehicleId>],
    ) -> Vec<(u32, Vec3)> {
        let mut off = Vec::new();
        for (&e, &(v, local)) in &self.cargo {
            let Some(t) = placed
                .get(v)
                .copied()
                .flatten()
                .and_then(|id| physics.vehicle_transform(id))
            else {
                continue;
            };
            if t.y_axis.y > TIPPED {
                let (_, rotation, position) = (t * local).to_scale_rotation_translation();
                if let Some(p) = self.shown.get_mut(&e) {
                    p.position = position;
                    p.rotation = rotation;
                }
            } else {
                off.push((e, self.shown.get(&e).map_or(Vec3::ZERO, |p| p.position)));
            }
        }
        for (e, _) in &off {
            self.cargo.remove(e);
        }
        if !self.cargo.is_empty() {
            self.changes += 1;
        }
        off
    }

    /// Where each prop riding on a vehicle is, and its heading.
    pub fn cargo_places(&self) -> Vec<(u32, Vec3, f32)> {
        self.cargo
            .keys()
            .filter_map(|e| {
                let p = self.shown.get(e)?;
                let f = p.rotation * Vec3::Z;
                Some((*e, p.position, f.x.atan2(f.z)))
            })
            .collect()
    }

    /// Whether `entity` was knocked loose (`None` if not): broken into
    /// its pieces, its model's [`Loose::breaks`], or gone as one.
    pub fn knocked_loose(&self, entity: u32) -> Option<bool> {
        self.knocked
            .contains_key(&entity)
            .then(|| self.loose(entity).is_some_and(|l| l.breaks))
    }

    /// Those knocked loose since the last call.
    pub fn take_knocked(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.fresh_knocks)
    }

    /// Those standing whole with their models loaded that a vehicle may
    /// break, with any of their model within `reach` metres of `at` (as
    /// far as its box's middle less half its size): not the
    /// [`sturdy`](Self::sturdy) (a mission's gates fall to its scripts),
    /// nor the island's objects put up to come down.
    /// Every prop shown whose place is within `reach` of `at`, with
    /// words on why [`standing_near`](Self::standing_near) might pass it
    /// over (for the log).
    pub fn told_near(&self, at: Vec3, reach: f32) -> Vec<String> {
        self.shown
            .iter()
            .filter(|(_, p)| p.position.distance(at) <= reach)
            .map(|(&e, p)| {
                let model = match self.models.get(&p.model) {
                    Some(Some(m)) if !m.members.is_empty() => "a merged batch",
                    Some(Some(_)) => "loaded",
                    Some(None) => "not loaded",
                    None => "not asked for",
                };
                format!(
                    "{e:#x} model {:08x} at {:.2} ({model}{}{}{}{}{})",
                    p.model,
                    p.position,
                    if p.wrecked { ", wrecked" } else { "" },
                    if crate::destruct::adopted(e) {
                        ", adopted"
                    } else {
                        ""
                    },
                    if self.sturdy.contains(&e) {
                        ", sturdy"
                    } else {
                        ""
                    },
                    if self.knocked.contains_key(&e) {
                        ", knocked"
                    } else {
                        ""
                    },
                    if self.broken.contains_key(&e) {
                        ", broken"
                    } else {
                        ""
                    },
                )
            })
            .collect()
    }

    pub fn standing_near(&self, at: Vec3, reach: f32) -> Vec<u32> {
        // Asked every tick for every vehicle going: those too far for any
        // model to reach are passed over first, cheaply.
        let far = reach + self.widest;
        self.shown
            .iter()
            .filter(|&(_, p)| !p.wrecked && p.position.distance_squared(at) <= far * far)
            .filter_map(|(&e, p)| {
                let m = self.models.get(&p.model)?.as_ref()?;
                let (lo, hi) = m.gpu.bounds;
                let centre = p.position + p.rotation * ((lo + hi) * 0.5);
                let d = centre.distance(at) - (hi - lo).length() * 0.5;
                (d <= reach).then_some(e)
            })
            .filter(|e| {
                (!crate::destruct::adopted(*e) || self.parts_down.contains_key(e))
                    && !self.sturdy.contains(e)
                    && !self.knocked.contains_key(e)
                    && !self.broken.contains_key(e)
            })
            .collect()
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
    /// as `blast` (what broke it) pushes them ([`BlastPush::throw`]), and
    /// a building whose collapse animation sinks its intact walls has
    /// them go down into the ground as its wreck shows (one found broken,
    /// as after loading a game, shows its wreck alone). True if it was
    /// one.
    pub fn wreck(&mut self, physics: &mut Physics, entity: u32, blast: Option<BlastPush>) -> bool {
        self.wreck_scaled(physics, entity, blast, 1.0)
    }

    /// [`wreck`](Self::wreck), what breaks apart thrown `scale` times as
    /// fast (the gate test's tries).
    pub fn wreck_scaled(
        &mut self,
        physics: &mut Physics,
        entity: u32,
        blast: Option<BlastPush>,
        scale: f32,
    ) -> bool {
        self.wreck_by(physics, entity, Hit::Blast(blast, scale))
    }

    /// [`wreck`](Self::wreck) by a vehicle running into it: nothing
    /// sinks, and what breaks apart is knocked on as the vehicle knocks
    /// loose what it hits, not thrown as from a blast. `push` gives the
    /// pieces' velocity (m/s) and spin (radians a second) for the mass
    /// (kg) they make up together.
    pub fn wreck_crashed(
        &mut self,
        physics: &mut Physics,
        entity: u32,
        push: impl FnOnce(f32) -> (Vec3, Vec3),
    ) -> bool {
        let mass = self
            .shown
            .get(&entity)
            .and_then(|p| self.models.get(&p.model)?.as_ref()?.breakup.as_deref())
            .map_or(0.0, |b| {
                weighed(b, self.piece_mass.get(&entity).copied())
                    .pieces
                    .mass
            });
        let (velocity, spin) = push(mass);
        self.wreck_by(physics, entity, Hit::Crash(velocity, spin))
    }

    fn wreck_by(&mut self, physics: &mut Physics, entity: u32, hit: Hit) -> bool {
        // Its box in the world, for a blast's push on it whole.
        let whole = self
            .placed_bounds(entity)
            .map(|(at, bounds)| box_around(at, bounds));
        let Some(p) = self.shown.get_mut(&entity).filter(|p| !p.wrecked) else {
            return false;
        };
        physics.remove_static(StaticKey::Entity(entity));
        self.fallen.push(entity);
        self.parts_down.remove(&entity);
        // How hard blasts push it, of their push (`ExplosionFudge`; all
        // of it without one).
        let fudge = self.fudge.get(&entity).copied().unwrap_or(1.0);
        let model = self.models.get(&p.model).and_then(Option::as_ref);
        if let Some(m) = model.filter(|m| m.gpu.has_wreck()) {
            p.wrecked = true;
            let placed = Mat4::from_rotation_translation(p.rotation, p.position);
            let sinking = match hit {
                Hit::Blast(Some(from), _) if m.gpu.sinks() => {
                    Sinking::from(entity, placed, m.gpu.bounds, from.at)
                }
                _ => Sinking::under(),
            };
            self.sinking.insert(entity, sinking);
            if let Some(breakup) = m.breakup.as_deref()
                && !self.knocked.contains_key(&entity)
            {
                let breakup = &weighed(breakup, self.piece_mass.get(&entity).copied());
                let pieces = match hit {
                    Hit::Blast(blast, scale) => {
                        throw(physics, breakup, placed, blast, scale * fudge)
                    }
                    Hit::Crash(velocity, spin) => {
                        Some(knock_on(physics, breakup, placed, velocity, spin))
                    }
                };
                if let Some(pieces) = pieces {
                    match hit {
                        Hit::Blast(blast, _) => eprintln!(
                            "props: {entity:#x} ({:08x}) breaks apart into {} pieces, away from \
                             {:.1}, burning at {} points, intact walls {}",
                            p.model,
                            pieces.len(),
                            blast.map(|b| b.at).unwrap_or_default(),
                            breakup.points.len(),
                            if sinking.done(m.gpu.bounds) {
                                "gone at once (no collapse animation)".to_owned()
                            } else {
                                format!("sinking {:.1} deg over", sinking.lean.to_degrees())
                            }
                        ),
                        Hit::Crash(velocity, _) => eprintln!(
                            "props: {entity:#x} ({:08x}) breaks apart into {} pieces ({:.0} kg) \
                             under a vehicle, knocked on at {:.1} m/s",
                            p.model,
                            pieces.len(),
                            breakup.pieces.mass,
                            velocity.length()
                        ),
                    }
                    self.broken.insert(entity, pieces);
                }
            }
        } else if !self.knocked.contains_key(&entity) {
            // No wreck to show, but it comes loose (a barricade, a café
            // chair): it goes flying rather than vanish.
            let (velocity, spin) = match hit {
                Hit::Crash(velocity, spin) => (velocity, spin),
                Hit::Blast(Some(push), scale) => {
                    let (lo, hi) = whole.unwrap_or((p.position, p.position));
                    let (velocity, spin) = push.throw(lo, hi, fudge);
                    (velocity * scale, spin * scale)
                }
                Hit::Blast(None, _) => (Vec3::ZERO, Vec3::ZERO),
            };
            if !self.fling(physics, entity, velocity, spin) {
                // Loose pieces stay where they fell.
                self.shown.remove(&entity);
                self.changed();
            }
        }
        true
    }

    /// Sends `entity`'s model flying as it comes loose (its standing
    /// collision already gone): its pieces off at `velocity` (m/s),
    /// turning at `spin`. False unless its model comes loose.
    fn fling(&mut self, physics: &mut Physics, entity: u32, velocity: Vec3, spin: Vec3) -> bool {
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
            && stands(entity)
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
        if let Some(sink) = self.sinking.get(&entity)
            && let Some(Some(m)) = self.models.get(&p.model)
        {
            let bounds = m.gpu.bounds;
            s += &format!(
                "; intact walls sunk {:.1} of {:.1} m after {:.1} s{}",
                sink.depth(bounds),
                bounds.1.y - bounds.0.y,
                sink.time,
                if sink.done(bounds) { ", under" } else { "" }
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
        // Props put up since the nearness was last taken (a state layer's,
        // before the next update asks for their models) are counted too
        // (playtest 7: the villa's buildings, put up again as PmcCon001
        // ended, were not, and she landed 41 m under them). That nearness
        // is kept till props are put up or taken down again: worked out
        // anew at each call, it went over every shown prop (89,000 on the
        // island) several times a tick while the update had not taken it
        // again, which it doesn't while nothing is asked of the loaders,
        // and play ran at 1 fps (playtest 7).
        let waiting_near = match &self.near {
            _ if self.waiting.is_empty() => 0,
            None => 0,
            Some(n) if n.placed == self.placed => self
                .waiting
                .iter()
                .filter(|h| n.by_model.get(h).is_some_and(|&d| d <= within))
                .count(),
            Some(n) => {
                let mut fresh = self.fresh.borrow_mut();
                if fresh.as_ref().is_none_or(|(at, _)| *at != self.placed) {
                    *fresh = Some((self.placed, self.waiting_nearness(n.focus)));
                }
                let by_model = fresh.as_ref().map(|(_, m)| m);
                self.waiting
                    .iter()
                    .filter(|h| {
                        by_model
                            .and_then(|m| m.get(h))
                            .is_some_and(|&d| d <= within)
                    })
                    .count()
            }
        };
        PropCounts {
            shown: self.shown.len(),
            loaded: self.models.values().filter(|m| m.is_some()).count(),
            loading: self.asked.len() + self.decoded.len(),
            waiting_near,
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
        // Those shown since their model came in: their collision.
        for e in std::mem::take(&mut self.unwalled) {
            let Some(p) = self.shown.get(&e) else {
                continue;
            };
            let model = (self.models.get(&p.model)).and_then(Option::as_ref);
            // Some parts broken: the rest's collision.
            let partial = (model.and_then(|m| m.gpu.parts.as_ref()))
                .filter(|parts| !parts.meshes.is_empty())
                .zip(self.parts_down.get(&e))
                .map(|(parts, down)| parts.collision(&down.wrecked));
            let shape = model.and_then(|m| m.collision.as_ref());
            let shape = match &partial {
                Some(s) => s.as_ref(),
                None => shape,
            };
            if let Some(shape) = shape
                && stands(e)
                && !p.wrecked
                && !self.knocked.contains_key(&e)
                && !physics.has_static(StaticKey::Entity(e))
            {
                physics.add_static(StaticKey::Entity(e), shape, p.position, p.rotation);
            }
        }
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
                placed: self.placed,
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

    /// [`nearness`](Self::nearness) of the waiting models alone.
    fn waiting_nearness(&self, focus: Vec3) -> HashMap<u32, f32> {
        let mut near = HashMap::new();
        for p in self.shown.values() {
            if !self.waiting.contains(&p.model) {
                continue;
            }
            let d = p.position.distance_squared(focus);
            near.entry(p.model)
                .and_modify(|n: &mut f32| *n = n.min(d))
                .or_insert(d);
        }
        near
    }

    /// A prop was put up or taken down (or broke).
    fn changed(&mut self) {
        self.changes += 1;
        self.placed += 1;
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
        let (mut cpu, members) = match cpu {
            Ok(cpu) => cpu,
            Err(e) => {
                eprintln!("state layer model {hash:08x}: {e}");
                self.failed.insert(hash);
                return Ok(());
            }
        };
        // A physics prop (`_PropPhysics`) whose model has no body that
        // moves comes loose as one, of its template's mass.
        let physics_prop = (self.shown.values())
            .filter(|p| p.model == hash)
            .find_map(|p| self.physics_props.get(&p.template).copied());
        if let (Some(c), Some(prop)) = (cpu.collision.as_mut(), physics_prop)
            && c.loose.is_none()
            && let Some(whole) = &c.whole
        {
            c.loose = Some(Arc::new(whole.weighing(prop.kg())));
        }
        materials.load_model_textures(backend, source, &cpu)?;
        let gpu = GpuModel::upload(backend, materials, &cpu)?;
        let (lo, hi) = gpu.bounds;
        let reach = ((lo + hi) * 0.5).length() + (hi - lo).length() * 0.5;
        if reach.is_finite() {
            self.widest = self.widest.max(reach);
        }
        if let Some(shape) = &cpu.collision {
            let mut through = Vec::new();
            for (&e, p) in self.shown.iter().filter(|(_, p)| p.model == hash) {
                if !stands(e) {
                    continue;
                }
                if through_on_island(p, shape) {
                    through.push(format!("{e:#x} (template {:08x})", p.template));
                }
                if !p.wrecked && !self.knocked.contains_key(&e) {
                    physics.add_static(StaticKey::Entity(e), shape, p.position, p.rotation);
                }
            }
            if !through.is_empty() {
                through.sort_unstable();
                eprintln!(
                    "props: {hash:08x} has no collision of its own (no PHY2): {} placed by name \
                     kept in the way, though the island's are in nobody's way: {}",
                    through.len(),
                    through.join(", ")
                );
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
                    let nodes = loose.posed(placed, &places);
                    let flying = loose.flying();
                    self.draw_sinking(queue, *e, m, placed);
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
            match (
                self.poses.get(e).filter(|_| !p.wrecked),
                self.parts_down.get(e),
            ) {
                (Some(nodes), _) => m.gpu.draw_moved(queue, at, nodes, false, &[]),
                // Some parts broken, the rest standing.
                (None, Some(down)) if !p.wrecked => m.gpu.draw_hiding(queue, at, &down.hidden),
                _ => m.gpu.draw_as(queue, at, p.wrecked),
            }
            self.draw_sinking(queue, *e, m, at);
        }
    }

    /// Draws `entity`'s intact walls going down, if they are on their way.
    fn draw_sinking(&self, queue: &mut DrawQueue, entity: u32, m: &PropModel, placed: Mat4) {
        if let Some(s) = self.sinking.get(&entity)
            && !s.done(m.gpu.bounds)
        {
            m.gpu.draw_sinking(queue, placed, s.place(m.gpu.bounds));
        }
    }
}

/// Whether prop `entity`'s model's collision stands in the way while it
/// stands: not for one of the island's objects put up only to come down
/// ([`crate::destruct::adopted`]), whose collision is already gone.
fn stands(entity: u32) -> bool {
    !crate::destruct::adopted(entity)
}

/// Whether prop `p`'s collision, `shape`, is a stand-in cut from its
/// model's drawn triangles for one placed by name (its model has no
/// `PHY2`): the island's such are in nobody's way ([`m2_engine::World`]
/// puts none up for them). A state layer's are kept in the way and only
/// logged until a census of the models without `PHY2` shows the island's
/// rule holds for them too: an outpost's sandbags and walls, or a
/// mission's gate, may be among them.
fn through_on_island(p: &Prop, shape: &CollisionShape) -> bool {
    p.named && shape.stand_in
}

/// How a building's intact walls go down as it collapses, as its
/// template's `BuildingCollapseAnim` has it (`SinkAccel` 2.5,
/// `SinkSpeedMax` -1, no limit, `LeanAngleMin` 8 and `LeanAngleMax` 10):
/// round D6 read the part states of five buildings. A building's
/// `CollapseState` shows its wreck and starts that animation on its intact
/// node (`StartAnim` .. `CollapseAnim`), which goes into the ground ever
/// faster (this many metres a second more each second), leaning over as
/// it goes to between the least and the most (degrees), away from the
/// blast; its `DestroyedState` shows the wreck, which nothing hides or
/// moves after (no timer, no `Kill`), and one loaded destroyed shows the
/// wreck alone. A gate has no such animation: its wreck simply shows, and
/// the Xbox 360 video keeps its arch standing after the bomb. How far the
/// walls lean on the way down, as a share of the way under, is inferred.
const SINK_ACCEL: f32 = 2.5;
const LEAN_LEAST: f32 = 8.0;
const LEAN_MOST: f32 = 10.0;

/// A building's intact walls going down ([`SINK_ACCEL`]).
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
    /// `bounds`, leaning across its line away from a blast at `from`
    /// (either way for one on its line).
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

    /// Already down (one found broken, or one with nothing to sink).
    fn under() -> Self {
        Self {
            time: f32::INFINITY,
            lean: 0.0,
            axis: Vec3::X,
        }
    }

    /// How far down it is now (metres), at most its height (`bounds`).
    fn depth(&self, (lo, hi): (Vec3, Vec3)) -> f32 {
        let height = (hi.y - lo.y).max(0.0);
        let depth = 0.5 * SINK_ACCEL * self.time * self.time;
        if depth.is_nan() {
            height
        } else {
            depth.min(height)
        }
    }

    /// Whether it is all under the ground.
    fn done(&self, bounds: (Vec3, Vec3)) -> bool {
        self.depth(bounds) >= (bounds.1.y - bounds.0.y).max(0.0)
    }

    /// Where the intact walls, their box `bounds`, are now, in the model's
    /// own space: sunk, and leaning by as much of its lean as of the way
    /// under they have gone.
    fn place(&self, (lo, hi): (Vec3, Vec3)) -> Mat4 {
        let depth = self.depth((lo, hi));
        let height = (hi.y - lo.y).max(1e-3);
        let pivot = Vec3::new((lo.x + hi.x) * 0.5, lo.y, (lo.z + hi.z) * 0.5);
        let lean = self.lean * depth / height;
        Mat4::from_translation(pivot - Vec3::Y * depth)
            * Mat4::from_axis_angle(self.axis, lean)
            * Mat4::from_translation(-pivot)
    }
}

/// A piece that starts in the ground (a gate's footings) is lifted out
/// of it as it breaks, under the flash, by at most this much (metres):
/// one left in it sticks there.
const UNBURY_MOST: f32 = 4.0;

/// Throws `breakup`'s pieces, the model placed at `placed`, as `blast`
/// pushes each ([`BlastPush::throw`], by its own box, the 360 code's),
/// `scale` times as fast, each lifted out of the ground it starts in.
/// `None` (nothing thrown) without a blast.
fn throw(
    physics: &mut Physics,
    breakup: &Breakup,
    placed: Mat4,
    blast: Option<BlastPush>,
    scale: f32,
) -> Option<Vec<DebrisId>> {
    let push = blast?;
    let loose = &breakup.pieces;
    let pieces = loose
        .pieces
        .iter()
        .zip(loose.starts(placed))
        .map(|(piece, at)| {
            let corners = corners(at, piece);
            let lift = Mat4::from_translation(Vec3::Y * buried(physics, &corners));
            let (velocity, spin) = launch(&corners, push, scale);
            physics.add_rubble(piece, lift * at, velocity, spin)
        })
        .collect();
    Some(pieces)
}

/// Sends `breakup`'s pieces, the model placed at `placed`, on at
/// `velocity` (m/s) turning at `spin` (radians a second), as a vehicle
/// sends on what it knocks loose: no blast throws them. Each is lifted out
/// of the ground it starts in, as [`throw`] lifts them.
/// `breakup` with each piece weighing `kg` (its template's debris, as
/// [`Props::piece_mass`] holds it), or as it is.
fn weighed(breakup: &Breakup, kg: Option<f32>) -> std::borrow::Cow<'_, Breakup> {
    let Some(kg) = kg else {
        return std::borrow::Cow::Borrowed(breakup);
    };
    let mut pieces = breakup.pieces.clone();
    for p in &mut pieces.pieces {
        p.mass = kg;
    }
    pieces.mass = kg * pieces.pieces.len() as f32;
    std::borrow::Cow::Owned(Breakup {
        pieces,
        points: breakup.points.clone(),
    })
}

fn knock_on(
    physics: &mut Physics,
    breakup: &Breakup,
    placed: Mat4,
    velocity: Vec3,
    spin: Vec3,
) -> Vec<DebrisId> {
    let loose = &breakup.pieces;
    loose
        .pieces
        .iter()
        .zip(loose.starts(placed))
        .map(|(piece, at)| {
            let lift = Mat4::from_translation(Vec3::Y * buried(physics, &corners(at, piece)));
            physics.add_rubble(piece, lift * at, velocity, spin)
        })
        .collect()
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
pub(crate) fn box_around(at: Mat4, (lo, hi): (Vec3, Vec3)) -> (Vec3, Vec3) {
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

/// How a piece of a model broken apart, its box's `corners` as it starts,
/// goes as `push` pushes it ([`BlastPush::throw`] of its world box),
/// `scale` times as fast: its velocity (m/s) and spin (radians a second).
fn launch(corners: &[Vec3; 8], push: BlastPush, scale: f32) -> (Vec3, Vec3) {
    let (lo, hi) = corners
        .iter()
        .fold((Vec3::MAX, Vec3::MIN), |(a, b), &c| (a.min(c), b.max(c)));
    let (velocity, spin) = push.throw(lo, hi, 1.0);
    (velocity * scale, spin * scale)
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
    fn a_buildings_intact_walls_sink_leaning_until_under() {
        // A shop block 20 m along x, 9.1 m high.
        let bounds = (Vec3::new(-10.0, 0.0, -6.0), Vec3::new(10.0, 9.1, 6.0));
        let placed = Mat4::from_translation(Vec3::new(100.0, -3.0, 50.0));
        // The blast a little to its -z side.
        let mut s = Sinking::from(7, placed, bounds, Vec3::new(100.0, 2.0, 40.0));
        let lean = s.lean.to_degrees();
        assert!((LEAN_LEAST..=LEAN_MOST).contains(&lean), "{lean}");
        let top = |s: &Sinking| s.place(bounds).transform_point3(Vec3::new(0.0, 9.1, 0.0));
        assert!(top(&s).distance(Vec3::new(0.0, 9.1, 0.0)) < 1e-4);
        assert!(!s.done(bounds));
        // A second on, 1.25 m down, leaning away from the blast.
        s.time = 1.0;
        let at = top(&s);
        assert!((s.depth(bounds) - 1.25).abs() < 1e-4);
        assert!(at.y < 7.9 && at.z > 0.0, "{at}");
        // Under after sqrt(2 x 9.1 / 2.5) = 2.7 s, and it stays there.
        s.time = 2.6;
        assert!(!s.done(bounds));
        s.time = 2.8;
        assert!(s.done(bounds));
        s.time = 60.0;
        assert!((s.depth(bounds) - 9.1).abs() < 1e-4);
        // One found broken is under already.
        assert!(Sinking::under().done(bounds));
    }

    #[test]
    fn a_prop_without_collision_of_its_own_stays_in_the_way_but_is_noted() {
        let mut shape = CollisionShape::cuboid(Vec3::ONE);
        let p = Prop {
            model: 1,
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            wrecked: false,
            template: 0,
            named: true,
        };
        assert!(stands(5) && !through_on_island(&p, &shape));
        // Cut from its drawn triangles: the island puts none up for one
        // placed by name; a state layer's stays in the way, only noted.
        shape.stand_in = true;
        assert!(stands(5) && through_on_island(&p, &shape));
        assert!(!through_on_island(&Prop { named: false, ..p }, &shape));
        // One of the island's put up to come down has lost its collision.
        assert!(!stands(crate::destruct::key(3)));
    }

    // The air strike bomb's push (its frag's: 500 over 50 m) on a gate's
    // pieces, as the 360 code gives it: out as fast as up, as fast for a
    // heavy piece as a light one, half as hard on one over 2 m³, and
    // still 0.3 of it however far.
    #[test]
    fn a_blast_pushes_pieces_out_and_up_by_their_size() {
        let push = BlastPush {
            at: Vec3::new(0.0, 9.0, 0.0),
            radius: 50.0,
            force: 500.0,
        };
        let slab = LoosePiece::block(
            None,
            Vec3::new(-2.6, 3.0, 0.76),
            Vec3::new(2.5, 3.0, 0.75),
            1000.0,
        );
        let (velocity, spin) = launch(&corners(Mat4::IDENTITY, &slab), push, 1.0);
        // 45 m³: half the push; 3 m from the blast to its top.
        let each = 7.5 * 500.0 * 0.5 * (1.0 - 3.0 / 50.0) * 0.004;
        assert!((velocity.y - each).abs() < 0.02, "{velocity}");
        assert!(
            (velocity.with_y(0.0).length() - each).abs() < 0.02,
            "{velocity}"
        );
        assert!(
            velocity.x < 0.0 && velocity.z > 0.0,
            "away from it: {velocity}"
        );
        // It tips away from the blast: its top goes the way it is pushed.
        assert!(
            spin.y == 0.0 && spin.cross(Vec3::Y).dot(velocity.with_y(0.0)) > 0.0,
            "{spin}"
        );
        // A block 1.2 m across goes with all of the push, whatever it
        // weighs.
        let block =
            |kg| LoosePiece::block(None, Vec3::new(8.0, 7.0, 0.0), Vec3::new(0.6, 0.5, 0.6), kg);
        let (light, _) = launch(&corners(Mat4::IDENTITY, &block(10.0)), push, 1.0);
        let (heavy, _) = launch(&corners(Mat4::IDENTITY, &block(5000.0)), push, 1.0);
        assert_eq!(light, heavy);
        // From the blast to its nearest corner.
        let d = Vec3::new(7.4, 7.5, 0.0).distance(push.at);
        assert!(
            (light.y - 7.5 * 500.0 * (1.0 - d / 50.0) * 0.004).abs() < 0.05,
            "{light}"
        );
        // Past its reach, 0.3 of it.
        let far = LoosePiece::block(
            None,
            Vec3::new(80.0, 3.0, 0.0),
            Vec3::new(0.5, 0.5, 0.5),
            100.0,
        );
        let (v, _) = launch(&corners(Mat4::IDENTITY, &far), push, 1.0);
        assert!((v.y - 7.5 * 500.0 * 0.3 * 0.004).abs() < 1e-3, "{v}");
    }

    #[test]
    fn a_gates_footings_come_out_of_the_ground_and_fly() {
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
        // A wall slab 5 m wide, 6 m high and 1.5 m thick, its footing a
        // metre in the ground, the bomb 6 m over the gate beside it.
        let push = BlastPush {
            at: Vec3::new(0.0, 9.0, 0.0),
            radius: 50.0,
            force: 500.0,
        };
        let piece = LoosePiece::block(
            None,
            Vec3::new(4.0, 2.0, 0.76),
            Vec3::new(2.5, 3.0, 0.75),
            1000.0,
        );
        let box_at = corners(Mat4::IDENTITY, &piece);
        let lift = buried(&physics, &box_at);
        assert!((lift - 1.0).abs() < 0.05, "lifted {lift}");
        let (velocity, spin) = launch(&box_at, push, 1.0);
        let id = physics.add_rubble(
            &piece,
            Mat4::from_translation(Vec3::Y * lift),
            velocity,
            spin,
        );
        for _ in 0..240 {
            physics.step(1.0 / 60.0);
        }
        // Out of the ground and away from the blast, along the wall.
        let at = physics
            .debris_place(id)
            .unwrap()
            .transform_point3(Vec3::new(4.0, 2.0, 0.76));
        assert!(at.x > 6.0 && at.x < 25.0, "{at}");
    }
}
