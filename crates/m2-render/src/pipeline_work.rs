//! How many render pipelines the backend has made, and the time it spent
//! making them, for the frame log.
//!
//! The wgpu backend makes each pipeline (and the shader module of its
//! vertex layout) the first time a draw needs it. A frame that meets
//! many new kinds of draw at once (the first frame of play) spends
//! seconds compiling them inside its draw, so the frame log names that
//! time apart from the rest of the draw ([`PipelineWork::take`]).
//!
//! Provenance: port-side instrumentation. The 360 ships precompiled
//! shaders and has no such cost; nothing here changes what is drawn.

use std::time::Duration;

/// Pipelines made and the time spent making them: since last taken, and
/// in all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PipelineWork {
    /// Pipelines made since last taken.
    pub made: u32,
    /// Time spent making them (shader modules included).
    pub took: Duration,
    /// Pipelines made since the backend started.
    pub total: u64,
}

impl PipelineWork {
    /// `made` pipelines were made, taking `took`.
    pub fn record(&mut self, made: u32, took: Duration) {
        self.made += made;
        self.took += took;
        self.total += u64::from(made);
    }

    /// What was made since last taken (with the running total); the
    /// count and time start again from nothing, the total does not.
    pub fn take(&mut self) -> PipelineWork {
        let out = *self;
        self.made = 0;
        self.took = Duration::ZERO;
        out
    }

    /// The frame log's line for what [`take`](Self::take) gave, or none
    /// when nothing was made: `pipelines: +N (total M) in X ms`.
    pub fn line(&self) -> Option<String> {
        (self.made > 0).then(|| {
            format!(
                "pipelines: +{} (total {}) in {:.0} ms",
                self.made,
                self.total,
                self.took.as_secs_f64() * 1000.0
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_gives_what_was_made_since_and_keeps_the_total() {
        let mut w = PipelineWork::default();
        w.record(2, Duration::from_millis(30));
        w.record(1, Duration::from_millis(5));
        let t = w.take();
        assert_eq!(t.made, 3);
        assert_eq!(t.took, Duration::from_millis(35));
        assert_eq!(t.total, 3);
        // Nothing new: none made, none taken, the total stays.
        let t = w.take();
        assert_eq!((t.made, t.took, t.total), (0, Duration::ZERO, 3));
        w.record(4, Duration::from_millis(1));
        assert_eq!(w.take().total, 7);
    }

    #[test]
    fn the_line_shows_only_frames_that_made_some() {
        assert_eq!(PipelineWork::default().line(), None);
        let mut w = PipelineWork::default();
        w.record(5, Duration::from_micros(12_400));
        assert_eq!(
            w.take().line().as_deref(),
            Some("pipelines: +5 (total 5) in 12 ms")
        );
    }
}
