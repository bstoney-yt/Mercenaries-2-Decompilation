//! Models the state layers place while they are in: a mission's gates,
//! barricades and crates ([`WorldIndex::state_objects`]). They stand once
//! the scripts load their layer (or the start-up did), go when it
//! unloads, and break when the scripts' object dies: drawn as their wreck
//! when the model has one, else gone, and no longer in the way.

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::sync::mpsc::{Receiver, Sender, channel};

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
#[derive(Debug)]
pub struct Props {
    shown: HashMap<u32, Prop>,
    /// By hash: `None` while loading, or when it failed.
    models: HashMap<u32, Option<PropModel>>,
    loads: (Sender<Load>, Receiver<Load>),
}

/// A model decoded on the loading thread, or why not.
type Load = (u32, Result<CpuModel, String>);

impl Default for Props {
    fn default() -> Self {
        Self {
            shown: HashMap::new(),
            models: HashMap::new(),
            loads: channel(),
        }
    }
}

impl Props {
    /// Puts up the models of those of `entities` the state layers place
    /// (others are ignored), loading their models on a thread. How many
    /// were new.
    pub fn show(
        &mut self,
        install: &Install,
        index: &WorldIndex,
        entities: impl IntoIterator<Item = u32>,
    ) -> usize {
        let mut wanted = Vec::new();
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
                wanted.push(o.model);
            }
        }
        if !wanted.is_empty() {
            load(install.clone(), wanted, self.loads.0.clone());
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

    /// `entity`'s, if it is shown, and whether its model has loaded.
    pub fn get(&self, entity: u32) -> Option<(&Prop, bool)> {
        let p = self.shown.get(&entity)?;
        Some((p, self.models.get(&p.model).is_some_and(Option::is_some)))
    }

    /// Those shown, by entity.
    pub fn shown(&self) -> impl Iterator<Item = (u32, &Prop)> {
        self.shown.iter().map(|(&e, p)| (e, p))
    }

    /// Uploads models that have loaded (one a call) and gives the props
    /// using them their collision.
    pub fn update(
        &mut self,
        backend: &mut dyn RenderBackend,
        materials: &mut MaterialCache,
        source: &mut AssetSource<BufReader<File>>,
        physics: &mut Physics,
    ) -> anyhow::Result<()> {
        let Ok((hash, cpu)) = self.loads.1.try_recv() else {
            return Ok(());
        };
        let cpu = match cpu {
            Ok(cpu) => cpu,
            Err(e) => {
                eprintln!("state layer model {hash:08x}: {e}");
                return Ok(());
            }
        };
        materials.load_textures(backend, source, cpu.texture_hashes().collect::<Vec<_>>())?;
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

/// Decodes `models` (with their collision) on a thread, sending each.
fn load(install: Install, models: Vec<u32>, tx: Sender<Load>) {
    std::thread::spawn(move || {
        let mut source = match install.open_world() {
            Ok(s) => s,
            Err(e) => {
                for m in models {
                    let _ = tx.send((m, Err(e.to_string())));
                }
                return;
            }
        };
        for m in models {
            let cpu = source
                .model(m)
                .and_then(|data| CpuModel::from_data(&data, 0))
                .map_err(|e| e.to_string());
            // The game may have quit meanwhile.
            if tx.send((m, cpu)).is_err() {
                return;
            }
        }
    });
}
