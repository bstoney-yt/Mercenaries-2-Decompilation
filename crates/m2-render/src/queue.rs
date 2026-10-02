//! The sort-key draw queue.
//!
//! Mirrors the three steps the engine's `PgPrimitive` class is named with
//! (`AssignKeys`, `Sort`, `SubmitToGPU`, from the executable's readable
//! strings only): primitives are collected during the frame, given sort keys
//! once the camera is known, sorted, and submitted to a backend in key order.
//! The implementation is a clean-room design.

use m2_math::glam::Vec3;

use crate::backend::{BackendError, DrawCall, RenderBackend, View};
use crate::key::{KeyFields, Layer, MaterialId, SortKey};

/// A draw call plus what it needs to be ordered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Primitive {
    pub call: DrawCall,
    pub layer: Layer,
    pub translucent: bool,
    /// World-space point the depth sort measures, usually the bounds centre.
    pub sort_origin: Vec3,
    /// Final tiebreak; only the low 15 bits are kept.
    pub user: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Stage {
    #[default]
    Collecting,
    Keyed,
    Sorted,
}

/// Errors from driving the queue out of order or from the backend.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum QueueError {
    #[error("sort called before assign_keys")]
    NotKeyed,
    #[error("submit called before sort")]
    NotSorted,
    #[error(transparent)]
    Backend(#[from] BackendError),
}

/// What a submit sent to the backend.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubmitStats {
    pub draws: usize,
    /// Times consecutive draws used different materials (the first draw
    /// counts as one).
    pub material_changes: usize,
}

/// Collects one frame's primitives and submits them in sort order.
#[derive(Debug, Default)]
pub struct DrawQueue {
    entries: Vec<(SortKey, Primitive)>,
    stage: Stage,
}

impl DrawQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Adds a primitive. Any keys already assigned are invalidated, so
    /// [`assign_keys`](Self::assign_keys) must run again before submitting.
    pub fn push(&mut self, primitive: Primitive) {
        self.entries.push((SortKey::default(), primitive));
        self.stage = Stage::Collecting;
    }

    /// Computes every primitive's key for `view`.
    pub fn assign_keys(&mut self, view: &View) {
        for (key, p) in &mut self.entries {
            *key = SortKey::new(KeyFields {
                layer: p.layer,
                translucent: p.translucent,
                material: p.call.material,
                depth: view.normalized_depth(p.sort_origin),
                user: p.user,
            });
        }
        self.stage = Stage::Keyed;
    }

    /// Orders primitives by key. Equal keys keep their push order.
    pub fn sort(&mut self) -> Result<(), QueueError> {
        if self.stage == Stage::Collecting {
            return Err(QueueError::NotKeyed);
        }
        self.entries.sort_by_key(|&(key, _)| key);
        self.stage = Stage::Sorted;
        Ok(())
    }

    /// Keys in their current order.
    pub fn keys(&self) -> impl Iterator<Item = SortKey> + '_ {
        self.entries.iter().map(|&(key, _)| key)
    }

    /// Sends every primitive to `backend` in order and empties the queue,
    /// keeping its allocation. The backend must be inside a frame.
    ///
    /// On a backend error the queue is still emptied; the frame is lost
    /// either way and the primitives are rebuilt next frame.
    pub fn submit(&mut self, backend: &mut dyn RenderBackend) -> Result<SubmitStats, QueueError> {
        if self.stage != Stage::Sorted {
            return Err(QueueError::NotSorted);
        }
        let mut stats = SubmitStats::default();
        let mut last_material: Option<MaterialId> = None;
        let result = self.entries.iter().try_for_each(|(_, p)| {
            backend.draw(&p.call)?;
            stats.draws += 1;
            if last_material != Some(p.call.material) {
                stats.material_changes += 1;
                last_material = Some(p.call.material);
            }
            Ok(())
        });
        self.entries.clear();
        self.stage = Stage::Collecting;
        result.map(|()| stats)
    }

    /// Runs [`assign_keys`](Self::assign_keys), [`sort`](Self::sort) and
    /// [`submit`](Self::submit) in one go.
    pub fn flush(
        &mut self,
        view: &View,
        backend: &mut dyn RenderBackend,
    ) -> Result<SubmitStats, QueueError> {
        self.assign_keys(view);
        self.sort()?;
        self.submit(backend)
    }
}
