//! GPU time per frame stage, from timestamp queries written between the
//! stages of a frame's command encoder ([`GpuTimer`]). Read back a few
//! frames late (the GPU runs behind), summed until taken
//! ([`GpuTimer::take`]). Needs the device's `TIMESTAMP_QUERY` and
//! `TIMESTAMP_QUERY_INSIDE_ENCODERS`; without them there is no timer.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// The stages of a frame, in order, each timed from the mark before it.
pub const STAGES: [&str; 5] = ["shadows", "world", "water", "glows", "post"];

/// Marks a frame writes: one before the first stage and one after each.
const MARKS: u32 = STAGES.len() as u32 + 1;

/// Read-back buffers: frames whose times may be in flight at once.
const RING: usize = 4;

/// A read-back buffer's state.
const FREE: u8 = 0;
const WRITTEN: u8 = 1;
const MAPPING: u8 = 2;
const MAPPED: u8 = 3;
const FAILED: u8 = 4;

/// The GPU time of a frame's stages, summed over `frames` frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GpuTimes {
    pub frames: u32,
    /// Milliseconds in each of [`STAGES`], summed over the frames.
    pub stages: [f64; STAGES.len()],
}

impl GpuTimes {
    /// Milliseconds a frame, all stages.
    pub fn per_frame(&self) -> f64 {
        if self.frames == 0 {
            return 0.0;
        }
        self.stages.iter().sum::<f64>() / f64::from(self.frames)
    }

    /// Adds one frame's marks (ticks), `period` nanoseconds a tick. A
    /// frame whose marks run backwards (the counter reset) is left out.
    fn add(&mut self, marks: &[u64], period: f64) {
        if marks.windows(2).any(|w| w[1] < w[0]) {
            return;
        }
        for (i, w) in marks.windows(2).enumerate() {
            self.stages[i] += (w[1] - w[0]) as f64 * period / 1.0e6;
        }
        self.frames += 1;
    }
}

struct Slot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
}

struct Inner {
    /// The slot the frame being encoded writes to, if one was free.
    current: Option<usize>,
    times: GpuTimes,
}

/// Times a frame's stages on the GPU (see the module docs).
pub struct GpuTimer {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    slots: Vec<Slot>,
    /// Nanoseconds a timestamp tick.
    period: f64,
    inner: RefCell<Inner>,
}

impl std::fmt::Debug for GpuTimer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuTimer")
            .field("period", &self.period)
            .finish_non_exhaustive()
    }
}

/// The features a [`GpuTimer`] needs.
pub fn features() -> wgpu::Features {
    wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
}

impl GpuTimer {
    /// A timer for `device`, if it has [`features`] and `M2_GPU_TIMING`
    /// isn't `off`.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(features())
            || std::env::var("M2_GPU_TIMING").is_ok_and(|v| v == "off")
        {
            return None;
        }
        let bytes = u64::from(MARKS) * wgpu::QUERY_SIZE as u64;
        let set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("frame timer"),
            ty: wgpu::QueryType::Timestamp,
            count: MARKS,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame timer resolve"),
            size: bytes,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let slots = (0..RING)
            .map(|_| Slot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("frame timer read-back"),
                    size: bytes,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(FREE)),
            })
            .collect();
        Some(Self {
            set,
            resolve,
            slots,
            period: f64::from(queue.get_timestamp_period()),
            inner: RefCell::new(Inner {
                current: None,
                times: GpuTimes::default(),
            }),
        })
    }

    /// Starts timing the frame `encoder` holds, if a read-back buffer is
    /// free (else the frame goes untimed).
    pub fn begin(&self, encoder: &mut wgpu::CommandEncoder) {
        let free = self
            .slots
            .iter()
            .position(|s| s.state.load(Ordering::Acquire) == FREE);
        self.inner.borrow_mut().current = free;
        if free.is_some() {
            encoder.write_timestamp(&self.set, 0);
        }
    }

    /// Ends stage `stage` (an index into [`STAGES`]).
    pub fn mark(&self, encoder: &mut wgpu::CommandEncoder, stage: usize) {
        if self.inner.borrow().current.is_some() {
            encoder.write_timestamp(&self.set, stage as u32 + 1);
        }
    }

    /// Copies the frame's marks out to its read-back buffer.
    pub fn end(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(i) = self.inner.borrow().current else {
            return;
        };
        encoder.resolve_query_set(&self.set, 0..MARKS, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.slots[i].buffer, 0, None);
        self.slots[i].state.store(WRITTEN, Ordering::Release);
    }

    /// After the frame's commands are submitted: asks for its read-back
    /// buffer, and takes in the times of earlier frames that have come
    /// back.
    pub fn after_submit(&self, device: &wgpu::Device) {
        for s in &self.slots {
            if s.state
                .compare_exchange(WRITTEN, MAPPING, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                let state = s.state.clone();
                s.buffer.map_async(wgpu::MapMode::Read, .., move |r| {
                    state.store(if r.is_ok() { MAPPED } else { FAILED }, Ordering::Release);
                });
            }
        }
        let _ = device.poll(wgpu::PollType::Poll);
        let mut inner = self.inner.borrow_mut();
        for s in &self.slots {
            match s.state.load(Ordering::Acquire) {
                MAPPED => {
                    if let Ok(view) = s.buffer.get_mapped_range(..) {
                        let marks: Vec<u64> = view
                            .chunks_exact(8)
                            .map(|b| u64::from_le_bytes(b.try_into().unwrap_or([0; 8])))
                            .collect();
                        drop(view);
                        inner.times.add(&marks, self.period);
                    }
                    s.buffer.unmap();
                    s.state.store(FREE, Ordering::Release);
                }
                FAILED => s.state.store(FREE, Ordering::Release),
                _ => {}
            }
        }
    }

    /// The times summed since last taken.
    pub fn take(&self) -> GpuTimes {
        std::mem::take(&mut self.inner.borrow_mut().times)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_times_are_the_gaps_between_marks() {
        let mut t = GpuTimes::default();
        // 1 ns a tick: 1 ms, 2 ms, 0, 0.5 ms, 3 ms.
        t.add(
            &[0, 1_000_000, 3_000_000, 3_000_000, 3_500_000, 6_500_000],
            1.0,
        );
        assert_eq!(t.frames, 1);
        assert!((t.stages[0] - 1.0).abs() < 1e-9);
        assert!((t.stages[4] - 3.0).abs() < 1e-9);
        assert!((t.per_frame() - 6.5).abs() < 1e-9);
        // A counter that ran backwards is left out.
        t.add(&[5, 4, 6, 7, 8, 9], 1.0);
        assert_eq!(t.frames, 1);
    }
}
