//! Xbox One and Series controllers (and the Xbox 360's) through XInput
//! (`xinput1_4.dll`, part of Windows 10 and later).
//!
//! [`poll`] reads the first connected pad into a plain [`PadState`]; off
//! Windows it never finds one.

/// One pad's buttons, sticks and triggers as read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PadState {
    /// The [`buttons`] held, as XInput's bits.
    pub buttons: u16,
    /// The sticks, -1..=1, right and up positive; inside XInput's own
    /// dead zones they read 0.
    pub left: [f32; 2],
    pub right: [f32; 2],
    /// The triggers, 0..=1; under XInput's threshold they read 0.
    pub left_trigger: f32,
    pub right_trigger: f32,
}

impl PadState {
    pub fn held(&self, button: u16) -> bool {
        self.buttons & button != 0
    }
}

/// XInput's button bits (`XINPUT_GAMEPAD_*`, as published in `xinput.h`).
pub mod buttons {
    pub const DPAD_UP: u16 = 0x0001;
    pub const DPAD_DOWN: u16 = 0x0002;
    pub const DPAD_LEFT: u16 = 0x0004;
    pub const DPAD_RIGHT: u16 = 0x0008;
    /// The Menu button (Start on the 360's pad).
    pub const START: u16 = 0x0010;
    /// The View button (Back on the 360's pad).
    pub const BACK: u16 = 0x0020;
    pub const LEFT_THUMB: u16 = 0x0040;
    pub const RIGHT_THUMB: u16 = 0x0080;
    pub const LEFT_SHOULDER: u16 = 0x0100;
    pub const RIGHT_SHOULDER: u16 = 0x0200;
    pub const A: u16 = 0x1000;
    pub const B: u16 = 0x2000;
    pub const X: u16 = 0x4000;
    pub const Y: u16 = 0x8000;
}

/// `xinput.h`'s dead zones and trigger threshold.
pub const LEFT_DEAD_ZONE: i16 = 7849;
pub const RIGHT_DEAD_ZONE: i16 = 8689;
const TRIGGER_THRESHOLD: u8 = 30;

/// A stick's raw axes, -32768..=32767, to -1..=1 with the round dead zone
/// cut out and the rest rescaled from 0.
pub fn stick(x: i16, y: i16, dead_zone: i16) -> [f32; 2] {
    let (x, y) = (f32::from(x), f32::from(y));
    let len = x.hypot(y);
    let dead = f32::from(dead_zone);
    if len <= dead {
        return [0.0; 2];
    }
    let scaled = ((len - dead) / (32767.0 - dead)).min(1.0);
    [x / len * scaled, y / len * scaled]
}

/// A trigger's raw 0..=255 to 0..=1, under the threshold 0.
pub fn trigger(v: u8) -> f32 {
    if v < TRIGGER_THRESHOLD {
        0.0
    } else {
        f32::from(v - TRIGGER_THRESHOLD) / f32::from(255 - TRIGGER_THRESHOLD)
    }
}

/// The first connected pad (of XInput's four), if any.
#[cfg(windows)]
pub fn poll() -> Option<PadState> {
    use windows::Win32::UI::Input::XboxController::{XINPUT_STATE, XInputGetState};

    (0..4).find_map(|user| {
        let mut state = XINPUT_STATE::default();
        // SAFETY: `state` is a valid, writable XINPUT_STATE for the call's
        // length; XInputGetState only writes it and keeps no pointer.
        let err = unsafe { XInputGetState(user, &mut state) };
        (err == 0).then(|| {
            let g = state.Gamepad;
            PadState {
                buttons: g.wButtons.0,
                left: stick(g.sThumbLX, g.sThumbLY, LEFT_DEAD_ZONE),
                right: stick(g.sThumbRX, g.sThumbRY, RIGHT_DEAD_ZONE),
                left_trigger: trigger(g.bLeftTrigger),
                right_trigger: trigger(g.bRightTrigger),
            }
        })
    })
}

/// No pads off Windows.
#[cfg(not(windows))]
pub fn poll() -> Option<PadState> {
    None
}

/// A motor's 0..=1 as XInput's 0..=65535.
pub fn motor(v: f32) -> u16 {
    (v.clamp(0.0, 1.0) * 65535.0).round() as u16
}

/// Sets the motors of the first connected pad (the one [`poll`] reads):
/// the left (low, heavy) and right (high, light), 0..=1; 0 and 0 stop
/// them. Answers whether a pad took it.
#[cfg(windows)]
pub fn vibrate(left: f32, right: f32) -> bool {
    use windows::Win32::UI::Input::XboxController::{
        XINPUT_STATE, XINPUT_VIBRATION, XInputGetState, XInputSetState,
    };

    (0..4).any(|user| {
        let mut state = XINPUT_STATE::default();
        // SAFETY: as in `poll`: a valid, writable XINPUT_STATE, written
        // only for the call.
        if unsafe { XInputGetState(user, &mut state) } != 0 {
            return false;
        }
        let v = XINPUT_VIBRATION {
            wLeftMotorSpeed: motor(left),
            wRightMotorSpeed: motor(right),
        };
        // SAFETY: `v` is a valid XINPUT_VIBRATION for the call's length;
        // XInputSetState reads it and keeps no pointer.
        unsafe { XInputSetState(user, &v) == 0 }
    })
}

/// No pads off Windows.
#[cfg(not(windows))]
pub fn vibrate(_left: f32, _right: f32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sticks_cut_the_dead_zone_and_reach_one() {
        assert_eq!(stick(5000, -5000, LEFT_DEAD_ZONE), [0.0, 0.0]);
        let [x, y] = stick(32767, 0, LEFT_DEAD_ZONE);
        assert!((x - 1.0).abs() < 1e-6 && y == 0.0);
        let [x, y] = stick(0, -32768, RIGHT_DEAD_ZONE);
        assert!(x == 0.0 && (y + 1.0).abs() < 1e-6);
        // Just past the dead zone starts near 0, not at its edge.
        let [x, _] = stick(LEFT_DEAD_ZONE + 100, 0, LEFT_DEAD_ZONE);
        assert!(x > 0.0 && x < 0.01);
    }

    #[test]
    fn motors_span_xinputs_range() {
        assert_eq!((motor(-1.0), motor(0.5), motor(2.0)), (0, 32768, 65535));
    }

    #[test]
    fn triggers_start_past_the_threshold() {
        assert_eq!(trigger(10), 0.0);
        assert_eq!(trigger(TRIGGER_THRESHOLD), 0.0);
        assert_eq!(trigger(255), 1.0);
    }
}
