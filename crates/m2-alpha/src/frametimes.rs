//! Frame times over a stretch of play, for the `pace:` and `bench:`
//! lines: the average frame rate, the 1% low (the frame rate of the
//! slowest 1% of frames, averaged), and the median and 99th percentile
//! frame times, which an average frame rate alone hides.

/// Frame times collected until summed up ([`FrameTimes::summary`]).
#[derive(Clone, Debug, Default)]
pub struct FrameTimes {
    ms: Vec<f32>,
}

/// What [`FrameTimes`] add up to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameSummary {
    pub frames: usize,
    /// Frames over the time they took, a second.
    pub fps: f32,
    /// A second over the slowest 1% of frames' mean time (at least one
    /// frame's).
    pub low1: f32,
    /// Milliseconds: the median, 99th percentile and slowest frame.
    pub p50: f32,
    pub p99: f32,
    pub slowest: f32,
}

impl FrameTimes {
    /// A frame that took `ms` milliseconds.
    pub fn push(&mut self, ms: f32) {
        self.ms.push(ms);
    }

    /// The summary of the frames so far.
    pub fn summary(&self) -> FrameSummary {
        if self.ms.is_empty() {
            return FrameSummary::default();
        }
        let mut sorted = self.ms.clone();
        sorted.sort_by(f32::total_cmp);
        let n = sorted.len();
        let total: f32 = sorted.iter().sum();
        let at = |q: f32| sorted[((n as f32 * q).ceil() as usize).clamp(1, n) - 1];
        let worst = (n / 100).max(1);
        let worst_mean = sorted[n - worst..].iter().sum::<f32>() / worst as f32;
        FrameSummary {
            frames: n,
            fps: if total > 0.0 {
                n as f32 * 1000.0 / total
            } else {
                0.0
            },
            low1: if worst_mean > 0.0 {
                1000.0 / worst_mean
            } else {
                0.0
            },
            p50: at(0.5),
            p99: at(0.99),
            slowest: sorted[n - 1],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_one_percent_low_is_the_slowest_frames_rate() {
        let mut t = FrameTimes::default();
        // 198 frames of 10 ms and 2 of 50 ms: 1% of 200 is 2 frames.
        for _ in 0..198 {
            t.push(10.0);
        }
        t.push(50.0);
        t.push(50.0);
        let s = t.summary();
        assert_eq!(s.frames, 200);
        assert!((s.low1 - 20.0).abs() < 1e-3, "{s:?}");
        assert!((s.fps - 200.0 * 1000.0 / 2080.0).abs() < 1e-2, "{s:?}");
        assert_eq!(s.p50, 10.0);
        // The 99th percentile frame is the last of the 10 ms ones.
        assert_eq!(s.p99, 10.0);
        assert_eq!(s.slowest, 50.0);
        // Fewer than 100 frames: the slowest one.
        let mut few = FrameTimes::default();
        few.push(10.0);
        few.push(40.0);
        assert!((few.summary().low1 - 25.0).abs() < 1e-3);
        assert_eq!(FrameTimes::default().summary(), FrameSummary::default());
    }
}
