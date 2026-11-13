//! One of a view's blended camera parameter sets, as scripts change it:
//! the field of view (`Graphics.Camera.SetFovParams`) and the near and far
//! distances (`SetNearFar`), each blending from where it is to what was
//! asked, or back to the camera's own after `Restore...`.
//!
//! From the 360 code: each view keeps, per set, a scripted target block, a
//! blended-from block and a current block, an elapsed time, a length and a
//! "scripted" flag (the field of view's at view +0x370, the near/far's at
//! +0x4D0; the focus's at +0x210). The natives set the target, copy the
//! current block to the blended-from one, zero the elapsed time and store
//! the length. Every frame 0x824B8208 runs the mixer (0x824B8360) on each
//! set:
//!
//! - `elapsed = min(elapsed + dt, length)`;
//! - `t = elapsed / length` where the length is above 0.0001, else 1;
//! - `k = 1 - (1 - t)^2`;
//! - `current = from + (target - from) * k`, the target being the scripted
//!   block while the flag is set, else the camera's own block.
//!
//! It then writes the current near, far and field of view to the block the
//! renderer builds its projection from (0x82569110).

/// The least length a blend is timed over (0x820B7A44); at or under it the
/// change is whole at once.
const LEAST_LENGTH: f32 = 1.0e-4;

/// A set of `N` camera values and its blend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blend<const N: usize> {
    /// The scripts' values while they are set (the set's flag).
    target: Option<[f32; N]>,
    from: [f32; N],
    /// The values the frame is drawn with.
    pub current: [f32; N],
    elapsed: f32,
    length: f32,
}

impl<const N: usize> Blend<N> {
    /// A set at rest on the camera's `own` values.
    pub fn new(own: [f32; N]) -> Self {
        Self {
            target: None,
            from: own,
            current: own,
            elapsed: 0.0,
            length: 0.0,
        }
    }

    /// `Set...` (`Some`) or `Restore...` (`None`), over `seconds`: the
    /// blend starts over from the current values.
    pub fn ask(&mut self, target: Option<[f32; N]>, seconds: f32) {
        self.target = target;
        self.from = self.current;
        self.elapsed = 0.0;
        self.length = seconds;
    }

    /// Whether the scripts' values are the target.
    pub fn scripted(&self) -> bool {
        self.target.is_some()
    }

    /// A frame of `dt` seconds (0x824B8360), `own` being the camera's own
    /// values.
    pub fn step(&mut self, dt: f32, own: [f32; N]) {
        self.elapsed = (self.elapsed + dt).min(self.length);
        let t = if self.length > LEAST_LENGTH {
            self.elapsed / self.length
        } else {
            1.0
        };
        let k = 1.0 - (1.0 - t) * (1.0 - t);
        let target = self.target.unwrap_or(own);
        self.current = std::array::from_fn(|i| self.from[i] + (target[i] - self.from[i]) * k);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: [f32; 2] = [0.2, 2400.0];

    #[test]
    fn a_set_eases_out_to_what_the_script_asked_and_back() {
        let mut b = Blend::new(OWN);
        b.step(0.1, OWN);
        assert_eq!(b.current, OWN);
        assert!(!b.scripted());
        b.ask(Some([0.3, 500.0]), 2.0);
        // Half way through the time: three quarters of the way there.
        b.step(1.0, OWN);
        assert!((b.current[1] - (2400.0 + (500.0 - 2400.0) * 0.75)).abs() < 1e-2);
        b.step(1.0, OWN);
        assert_eq!(b.current, [0.3, 500.0]);
        assert!(b.scripted());
        // Restored: back to the camera's own from where it is.
        b.ask(None, 1.0);
        b.step(0.5, OWN);
        assert!((b.current[0] - (0.3 + (0.2 - 0.3) * 0.75)).abs() < 1e-6);
        b.step(1.0, OWN);
        assert_eq!(b.current, OWN);
    }

    #[test]
    fn without_a_time_the_change_is_whole_at_once() {
        // The scripts' `SetNearFar(0, 0.3, 500, 0)`.
        let mut b = Blend::new(OWN);
        b.ask(Some([0.3, 500.0]), 0.0);
        b.step(1.0 / 60.0, OWN);
        assert_eq!(b.current, [0.3, 500.0]);
    }
}
