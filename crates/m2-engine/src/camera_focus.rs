//! A view's focus as scripts change it: the blend from where the focus is
//! to what `Graphics.Camera.SetFocusParams` asked, or back to the camera's
//! own after `RestoreFocusParams`.
//!
//! From the 360 code: each view keeps three blocks of camera parameters
//! for its focus (the scripted target at view +0x210, the one blended from
//! at +0x280, the current at +0x2F0), an elapsed time, a length and a
//! "scripted" flag. The natives (0x823185B8, 0x82318758) set the target,
//! copy the current block to the blended-from one, zero the elapsed time
//! and store the length. Every frame 0x824B8208 runs the mixer
//! (0x824B8360) on the set:
//!
//! - `elapsed = min(elapsed + dt, length)`;
//! - `t = elapsed / length` where the length is above 0.0001, else 1;
//! - `k = 1 - (1 - t)^2`;
//! - `current = from + (target - from) * k`, the target being the scripted
//!   block while the flag is set, else the camera's own block.
//!
//! The same mixer runs the view's field-of-view and near/far sets
//! (`SetFovParams`, `SetNearFar`), which the port does not take yet.
//!
//! The five focus values are the depth of field's (the composite's
//! `dofParams`): soft nearer than the first, sharp from the second, soft
//! from the third to the fourth and beyond, by the fifth as an amount.
//! The camera's own are (0, 0.1, 100, 300, 0.3) (0x821B6148).

/// The camera's own focus (0x821B6148): near start and end, far start and
/// end, amount.
pub const OWN: [f32; 5] = [0.0, 0.1, 100.0, 300.0, 0.3];

/// The least length a blend is timed over (0x820B7A44); at or under it the
/// change is whole at once.
const LEAST_LENGTH: f32 = 1.0e-4;

/// A view's focus set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FocusBlend {
    /// The scripts' focus while one is set (the flag at +0x368).
    target: Option<[f32; 5]>,
    from: [f32; 5],
    /// The focus the frame is drawn with.
    pub current: [f32; 5],
    elapsed: f32,
    length: f32,
}

impl Default for FocusBlend {
    fn default() -> Self {
        Self {
            target: None,
            from: OWN,
            current: OWN,
            elapsed: 0.0,
            length: 0.0,
        }
    }
}

impl FocusBlend {
    /// `SetFocusParams` (`Some`) or `RestoreFocusParams` (`None`), over
    /// `seconds`: the blend starts over from the current focus.
    pub fn ask(&mut self, target: Option<[f32; 5]>, seconds: f32) {
        self.target = target;
        self.from = self.current;
        self.elapsed = 0.0;
        self.length = seconds;
    }

    /// A frame of `dt` seconds (0x824B8360), `own` being the camera's own
    /// focus.
    pub fn step(&mut self, dt: f32, own: [f32; 5]) {
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

    /// The hijack's focus: soft from 4 m to 6 m, fully.
    const HIJACK: [f32; 5] = [0.0, 0.1, 4.0, 6.0, 1.0];

    #[test]
    fn the_focus_eases_out_to_what_the_script_asked() {
        let mut f = FocusBlend::default();
        f.step(0.1, OWN);
        assert_eq!(f.current, OWN);
        f.ask(Some(HIJACK), 1.0);
        // Half way through the time: three quarters of the way there.
        f.step(0.5, OWN);
        assert!((f.current[2] - (100.0 + (4.0 - 100.0) * 0.75)).abs() < 1e-3);
        assert!((f.current[4] - (0.3 + 0.7 * 0.75)).abs() < 1e-5);
        // At and past the time: there, and it stays.
        f.step(0.5, OWN);
        assert_eq!(f.current, HIJACK);
        f.step(5.0, OWN);
        assert_eq!(f.current, HIJACK);
    }

    #[test]
    fn a_change_starts_from_where_the_focus_is() {
        let mut f = FocusBlend::default();
        f.ask(Some(HIJACK), 1.0);
        f.step(0.5, OWN);
        let mid = f.current;
        // Restored mid-blend: back to the camera's own from there.
        f.ask(None, 2.0);
        f.step(0.0, OWN);
        assert_eq!(f.current, mid);
        f.step(1.0, OWN);
        assert!((f.current[3] - (mid[3] + (300.0 - mid[3]) * 0.75)).abs() < 1e-3);
        f.step(1.0, OWN);
        assert_eq!(f.current, OWN);
    }

    #[test]
    fn without_a_time_the_change_is_whole_at_once() {
        let mut f = FocusBlend::default();
        f.ask(Some(HIJACK), 0.0);
        f.step(1.0 / 60.0, OWN);
        assert_eq!(f.current, HIJACK);
    }
}
