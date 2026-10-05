//! Models the state layers place while they are in: a mission's gates,
//! barricades and crates ([`WorldIndex::state_objects`]). They stand once
//! the scripts load their layer (or the start-up did), go when it
//! unloads, and break when the scripts' object dies: drawn as their wreck
//! when the model has one, else gone, and no longer in the way.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::panic::AssertUnwindSafe;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use m2_engine::model::MODEL_TEXTURE_WIDTH;
use m2_engine::{
    AssetSource, CollisionShape, CpuModel, GpuModel, Install, MaterialCache, Physics, StaticKey,
    WorldIndex,
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

/// A model's GPU copy and collision, once loaded.
#[derive(Debug)]
struct PropModel {
    gpu: GpuModel,
    collision: Option<CollisionShape>,
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
}

/// How long uploading loaded models may take a frame (at least one is).
const UPLOAD_BUDGET: Duration = Duration::from_millis(6);

/// Threads decoding models.
const LOADERS: usize = 3;

/// How many models may be asked of the loaders and not yet uploaded:
/// few, so that whatever is nearest when one finishes goes next. A start-up
/// puts up thousands of props, and a mission's gate has to load while the
/// player walks up to it, not after all of them.
const IN_FLIGHT: usize = 8;

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

    /// Takes down those of `entities` that are shown.
    pub fn hide(&mut self, physics: &mut Physics, entities: impl IntoIterator<Item = u32>) {
        for e in entities {
            if self.shown.remove(&e).is_some() {
                physics.remove_static(StaticKey::Entity(e));
            }
        }
    }

    /// Breaks `entity` if it is shown and whole: its wreck if the model
    /// has one, else it goes. True if it was one.
    pub fn wreck(&mut self, physics: &mut Physics, entity: u32) -> bool {
        let Some(p) = self.shown.get_mut(&entity).filter(|p| !p.wrecked) else {
            return false;
        };
        physics.remove_static(StaticKey::Entity(entity));
        let has_wreck = self
            .models
            .get(&p.model)
            .and_then(Option::as_ref)
            .is_some_and(|m| m.gpu.has_wreck());
        if has_wreck {
            p.wrecked = true;
        } else {
            self.shown.remove(&entity);
        }
        true
    }

    /// Moves `entity` (the scripts placed it elsewhere).
    pub fn put(&mut self, physics: &mut Physics, entity: u32, position: Vec3) {
        let Some(p) = self.shown.get_mut(&entity) else {
            return;
        };
        p.position = position;
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

    /// How many are shown, and of the models they use how many have
    /// loaded, are loading, wait to be loaded and failed.
    pub fn counts(&self) -> (usize, usize, usize, usize, usize) {
        let loaded = self.models.values().filter(|m| m.is_some()).count();
        (
            self.shown.len(),
            loaded,
            self.asked.len(),
            self.waiting.len(),
            self.failed.len(),
        )
    }

    /// Uploads models that have loaded, nearest `focus` first (as many as
    /// fit in [`UPLOAD_BUDGET`], at least one), gives the props using them
    /// their collision, and asks the loaders for the nearest still wanted.
    pub fn update(
        &mut self,
        backend: &mut dyn RenderBackend,
        materials: &mut MaterialCache,
        source: &mut AssetSource<BufReader<File>>,
        physics: &mut Physics,
        focus: Vec3,
    ) -> anyhow::Result<()> {
        let Some(loaders) = &self.loaders else {
            return Ok(());
        };
        self.decoded.extend(loaders.done.try_iter());
        if self.decoded.is_empty() && (self.waiting.is_empty() || self.asked.len() >= IN_FLIGHT) {
            return Ok(());
        }
        let near = self.nearness(focus);
        let far = |hash: &u32| near.get(hash).copied().unwrap_or(f32::INFINITY);
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
            .filter(|h| !near.contains_key(h))
            .collect();
        for h in unwanted {
            self.waiting.remove(&h);
            self.models.remove(&h);
        }
        let room = IN_FLIGHT.saturating_sub(self.asked.len());
        if room > 0 && !self.waiting.is_empty() {
            let mut next: Vec<u32> = self.waiting.iter().copied().collect();
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
        materials.load_model_textures(backend, source, &cpu)?;
        let gpu = GpuModel::upload(backend, materials, &cpu)?;
        if let Some(shape) = &cpu.collision {
            for (&e, p) in self.shown.iter().filter(|(_, p)| p.model == hash) {
                if !p.wrecked {
                    physics.add_static(StaticKey::Entity(e), shape, p.position, p.rotation);
                }
            }
        }
        self.models.insert(
            hash,
            Some(PropModel {
                gpu,
                collision: cpu.collision,
            }),
        );
        Ok(())
    }

    /// Draws those within `reach` of `eye`.
    pub fn draw(&self, queue: &mut DrawQueue, eye: Vec3, reach: f32) {
        for p in self.shown.values() {
            if p.position.distance(eye) > reach {
                continue;
            }
            if let Some(Some(m)) = self.models.get(&p.model) {
                let at = Mat4::from_rotation_translation(p.rotation, p.position);
                m.gpu.draw_as(queue, at, p.wrecked);
            }
        }
    }
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
                                .and_then(|data| CpuModel::from_data(&data, 0))
                                .map(|mut cpu| {
                                    cpu.decode_textures(source, MODEL_TEXTURE_WIDTH);
                                    cpu
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
