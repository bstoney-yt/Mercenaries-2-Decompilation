//! Xbox One and Series controllers (and the 360's): the game's own pad
//! layout, read through XInput ([`m2_sys::xinput`]).
//!
//! The layout is the pause screen's (`tControlMap` in the resident GUI
//! scripts, vz.wad): per kind of control, `Joystick.BUTTON_*` numbers to
//! what they do. The shell's `controllerText` puts those numbers by the
//! pad's parts (1 to 4 the d-pad, 5 to 8 Y A X B, 9 to 16 the sticks, 17
//! the left trigger, 18 the left bumper, 19 the left stick's press, 20 to
//! 22 the right's, 23 Start, 24 Back), so from the game's data:
//!
//! | on foot (`human`)       | in a car (`car`)          |
//! |-------------------------|---------------------------|
//! | A jump                  | A forward                 |
//! | B melee                 | B call allies             |
//! | X reload                | X reverse                 |
//! | Y action (get in, use)  | Y exit                    |
//! | LT crouch               | LT handbrake              |
//! | LB throw an explosive   |                           |
//! | L3 sprint               |                           |
//! | RT fire                 | RT fire                   |
//! | RB switch weapon        | RB switch weapon          |
//! | R3 zoom                 | R3 reset camera           |
//! | d-pad up/down support   | d-pad up/down cash & fuel |
//! | d-pad left explosive    | d-pad right camera mode   |
//! | Start pause, Back PDA   | Start pause, Back PDA     |
//!
//! The left stick moves (steers in a car), the right stick looks. The
//! hijack scripts' buttons are Y, A, X and B and the left stick's pushes
//! (`RPad_Up`... in [`m2_game::hijack::BUTTONS`]).
//!
//! R3 puts the binoculars up on foot (the layout's zoom: [`Action::Binoculars`]).
//! Invented where the alpha has a thing the layout doesn't: LT raises the
//! weapon as well on foot ([`Action::Aim`], as crouch does on the keys)
//! and rights an overturned vehicle when driving ([`Action::Reset`], where
//! the layout resets the camera); the d-pad's right sets off C4 on foot
//! ([`Action::Detonate`]); the d-pad's up and down move to the next seat
//! in a vehicle ([`Action::Seat`], as the helicopter's `Seat Menu`);
//! L3 latches sprint until the stick comes back to rest; the left stick
//! under half way walks ([`Action::Walk`]); the right stick turns at up
//! to [`LOOK_RATE`] mouse counts a second, squared for fine aim.
//!
//! In the front end and the menus, A is Enter, B Escape and the d-pad the
//! arrows ([`MenuKey`]).
//!
//! On Android there is no XInput: a controller (an Xbox pad over
//! Bluetooth, or any the phone takes as a gamepad) arrives as window
//! events, gathered by [`Events`] into the same [`PadState`]. Its buttons
//! are Android's `KEYCODE_BUTTON_*` keys and its sticks, triggers and hat
//! `AXIS_*` motions (Android's published numbers; the axes the Xbox pad
//! uses, from Android's controller documentation, not checked on the
//! phone yet).

use m2_sys::xinput::{LEFT_DEAD_ZONE, PadState, RIGHT_DEAD_ZONE, buttons as xb};

use crate::input::{Action, Input};

/// A button of the pad (the triggers count as buttons past half way).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Button {
    A,
    B,
    X,
    Y,
    LeftBumper,
    RightBumper,
    LeftTrigger,
    RightTrigger,
    LeftStick,
    RightStick,
    Start,
    Back,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
}

impl Button {
    /// Held in `s`.
    pub fn held(self, s: &PadState) -> bool {
        let bit = match self {
            Self::A => xb::A,
            Self::B => xb::B,
            Self::X => xb::X,
            Self::Y => xb::Y,
            Self::LeftBumper => xb::LEFT_SHOULDER,
            Self::RightBumper => xb::RIGHT_SHOULDER,
            Self::LeftTrigger => return s.left_trigger > 0.5,
            Self::RightTrigger => return s.right_trigger > 0.5,
            Self::LeftStick => xb::LEFT_THUMB,
            Self::RightStick => xb::RIGHT_THUMB,
            Self::Start => xb::START,
            Self::Back => xb::BACK,
            Self::DpadUp => xb::DPAD_UP,
            Self::DpadDown => xb::DPAD_DOWN,
            Self::DpadLeft => xb::DPAD_LEFT,
            Self::DpadRight => xb::DPAD_RIGHT,
        };
        s.held(bit)
    }

    /// The game's picture of it (vz.wad's console button textures).
    pub fn texture(self) -> &'static str {
        match self {
            Self::A => "button_a",
            Self::B => "button_b",
            Self::X => "button_x",
            Self::Y => "button_y",
            Self::LeftBumper => "button_bumper_l",
            Self::RightBumper => "button_bumper_r",
            Self::LeftTrigger => "button_trigger_l",
            Self::RightTrigger => "button_trigger_r",
            Self::LeftStick => "button_joystick_l_press",
            Self::RightStick => "button_joystick_r_press",
            Self::Start => "button_start",
            Self::Back => "button_select",
            Self::DpadUp => "button_dpad_up",
            Self::DpadDown => "button_dpad_down",
            Self::DpadLeft => "button_dpad_left",
            Self::DpadRight => "button_dpad_right",
        }
    }
}

/// What the buttons do on foot ([`Button`] to [`Action`]); several rows
/// may share a button.
pub const ON_FOOT: &[(Button, Action)] = &[
    (Button::A, Action::MoveUp),
    (Button::B, Action::Melee),
    (Button::X, Action::Reload),
    (Button::Y, Action::Use),
    (Button::LeftTrigger, Action::MoveDown),
    (Button::LeftBumper, Action::Throw),
    (Button::RightTrigger, Action::Fire),
    (Button::RightBumper, Action::SwitchWeapon),
    (Button::LeftTrigger, Action::Aim),
    (Button::RightStick, Action::Binoculars),
    (Button::DpadUp, Action::Support),
    (Button::DpadDown, Action::SupportDown),
    (Button::DpadLeft, Action::Explosive),
    (Button::DpadRight, Action::Detonate),
    (Button::Start, Action::Quit),
    (Button::Back, Action::Pda),
];

/// What they do driving.
pub const DRIVING: &[(Button, Action)] = &[
    (Button::A, Action::MoveForward),
    (Button::X, Action::MoveBack),
    (Button::Y, Action::Use),
    (Button::LeftTrigger, Action::MoveUp),
    (Button::RightTrigger, Action::Fire),
    (Button::RightBumper, Action::SwitchWeapon),
    (Button::RightStick, Action::Reset),
    (Button::DpadUp, Action::Seat),
    (Button::DpadDown, Action::Seat),
    (Button::Start, Action::Quit),
    (Button::Back, Action::Pda),
];

/// The hijack scripts' face buttons, in either.
pub const HIJACK: &[(Button, Action)] = &[
    (Button::Y, Action::PadY),
    (Button::A, Action::PadA),
    (Button::X, Action::PadX),
    (Button::B, Action::PadB),
];

/// The right stick at full tilt turns this many mouse counts a second
/// (invented: about a half turn a second at the default sensitivity).
pub const LOOK_RATE: f64 = 1000.0;
/// A stick push counts as a press (a hijack's `LStick_Left`) past this.
const PUSH: f32 = 0.5;

/// A key the front end and the menus take from the pad.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKey {
    Enter,
    Escape,
    Up,
    Down,
    Left,
    Right,
}

/// The pad as last read, and what it holds down in [`Input`].
#[derive(Debug, Default)]
pub struct Pad {
    last: PadState,
    connected: bool,
    /// The actions its buttons hold, to let go when they're let go (or
    /// the layout changes under them).
    holding: Vec<(Button, Action)>,
    /// Sprint latched by L3.
    sprint: bool,
    /// The pause menu's pad options: its look speed, times
    /// [`LOOK_RATE`] (unset: 1), and the right stick's up and down
    /// swapped.
    pub look: Option<f64>,
    pub invert: bool,
    /// The controller the window's events report (Android's).
    pub events: Events,
    /// The on-screen pad (a phone without a controller), read with the
    /// controller's.
    pub touch: crate::touch::Touch,
    /// The pause menu's Vibration (`gameRumbleVar`): off, [`Pad::rumble`]
    /// shakes nothing.
    pub rumble_on: bool,
    /// Seconds of shaking left ([`Pad::rumble`]).
    shaking: f64,
    /// The motors are running.
    motors: bool,
}

/// How hard the motors run while the scripts shake the pad, left (heavy)
/// and right (light), 0 to 1 (invented: `Pg.Rumble` names only who and
/// for how long).
pub const RUMBLE: [f32; 2] = [0.6, 0.6];

impl Pad {
    /// The scripts shake the player's pad for `seconds` (`Pg.Rumble`, the
    /// hijack scripts' failed press: 0.3 s and 1 s); a longer shake
    /// already going keeps going.
    pub fn rumble(&mut self, seconds: f64) {
        if self.rumble_on && seconds > 0.0 {
            self.shaking = self.shaking.max(seconds);
        }
    }

    /// The motors' speeds for the next `dt` seconds, and the shake's
    /// count down: [`RUMBLE`] while shaking, else stopped.
    fn shake(&mut self, dt: f64) -> [f32; 2] {
        if !self.rumble_on {
            self.shaking = 0.0;
        }
        if self.shaking > 0.0 {
            self.shaking = (self.shaking - dt).max(0.0);
            RUMBLE
        } else {
            [0.0; 2]
        }
    }
    /// Reads the pad and feeds `input` (a no-op without one). `driving`
    /// picks the layout; `dt` is the seconds since the last read. Answers
    /// the menu keys pressed since then ([`Pad::menu_keys`]).
    pub fn poll(&mut self, input: &mut Input, driving: bool, dt: f64) -> Vec<MenuKey> {
        let state = m2_sys::xinput::poll().or_else(|| self.events.state());
        if state.is_some() != self.connected {
            self.connected = state.is_some();
            eprintln!(
                "pad: controller {}",
                if self.connected {
                    "connected"
                } else {
                    "disconnected"
                }
            );
        }
        let mut s = state.unwrap_or_default();
        self.touch.set_driving(driving);
        if self.touch.used {
            s = crate::touch::merge(s, self.touch.state());
            let (dx, dy) = self.touch.take_look();
            input.pad_look(dx, if self.invert { -dy } else { dy });
        }
        let [left, right] = self.shake(dt);
        let on = left > 0.0 || right > 0.0;
        if self.connected && on != self.motors {
            let took = m2_sys::xinput::vibrate(left, right);
            self.motors = on && took;
        }
        let keys = Self::menu_keys(&self.last, &s);
        self.feed(input, s, driving, dt);
        // The touch controls' sprint is switched on, not held: she sprints
        // on foot whenever she moves while it is.
        if self.touch.sprint {
            let moving = s.left[0].hypot(s.left[1]) > 0.0;
            input.set_pad_hold(Action::Sprint, !driving && moving);
        }
        keys
    }

    /// Feeds one reading `s` to `input`.
    pub fn feed(&mut self, input: &mut Input, s: PadState, driving: bool, dt: f64) {
        let layout = if driving { DRIVING } else { ON_FOOT };
        let active = s != PadState::default();
        if active && !input.pad() {
            input.set_pad(true);
        }
        // Let go of what's no longer held, or no longer in the layout.
        self.holding.retain(|&(b, a)| {
            let keep = b.held(&s) && (layout.contains(&(b, a)) || HIJACK.contains(&(b, a)));
            if !keep {
                input.button(a, false);
            }
            keep
        });
        for &(b, a) in layout.iter().chain(HIJACK) {
            if b.held(&s) && !b.held(&self.last) && !self.holding.contains(&(b, a)) {
                input.button(a, true);
                self.holding.push((b, a));
            }
        }
        // Sprint: L3 latches it on foot, until the stick rests.
        let [lx, ly] = s.left;
        let tilt = lx.hypot(ly);
        if driving || tilt == 0.0 {
            self.sprint = false;
        } else if Button::LeftStick.held(&s) && !Button::LeftStick.held(&self.last) {
            self.sprint = !self.sprint;
        }
        input.set_pad_hold(Action::Sprint, self.sprint);
        input.set_pad_hold(Action::Walk, !driving && tilt > 0.0 && tilt < 0.5);
        // The stick: steering only in a car (A and X are the pedals).
        input.set_stick(if driving { [lx, 0.0] } else { [lx, ly] });
        for (action, push) in [
            (Action::MoveLeft, -lx),
            (Action::MoveRight, lx),
            (Action::MoveForward, ly),
            (Action::MoveBack, -ly),
        ] {
            input.set_stick_push(action, push > PUSH);
        }
        // Look: squared, so small pushes aim finely.
        let [rx, ry] = s.right;
        let ry = if self.invert { -ry } else { ry };
        let curve = |v: f32| f64::from(v * v.abs());
        let rate = LOOK_RATE * self.look.unwrap_or(1.0);
        input.pad_look(curve(rx) * rate * dt, -curve(ry) * rate * dt);
        self.last = s;
    }

    /// The menu keys pressed going from reading `before` to `now`: what
    /// the front end and the menus take.
    pub fn menu_keys(before: &PadState, now: &PadState) -> Vec<MenuKey> {
        [
            (Button::A, MenuKey::Enter),
            (Button::B, MenuKey::Escape),
            (Button::DpadUp, MenuKey::Up),
            (Button::DpadDown, MenuKey::Down),
            (Button::DpadLeft, MenuKey::Left),
            (Button::DpadRight, MenuKey::Right),
        ]
        .into_iter()
        .filter(|(b, _)| b.held(now) && !b.held(before))
        .map(|(_, k)| k)
        .collect()
    }
}

/// Android's `MotionEvent.AXIS_*` numbers the pad reads.
mod axis {
    pub const X: u32 = 0;
    pub const Y: u32 = 1;
    pub const Z: u32 = 11;
    pub const RZ: u32 = 14;
    pub const HAT_X: u32 = 15;
    pub const HAT_Y: u32 = 16;
    pub const LTRIGGER: u32 = 17;
    pub const RTRIGGER: u32 = 18;
    pub const GAS: u32 = 22;
    pub const BRAKE: u32 = 23;
    /// One past the highest.
    pub const COUNT: usize = 24;
}

/// Android's `KeyEvent.KEYCODE_BUTTON_*` numbers, with the XInput bit each
/// stands for (`None`: the digital triggers, L2 and R2).
const KEYS: [(u32, Option<u16>); 12] = [
    (96, Some(xb::A)),
    (97, Some(xb::B)),
    (99, Some(xb::X)),
    (100, Some(xb::Y)),
    (102, Some(xb::LEFT_SHOULDER)),
    (103, Some(xb::RIGHT_SHOULDER)),
    (104, None),
    (105, None),
    (106, Some(xb::LEFT_THUMB)),
    (107, Some(xb::RIGHT_THUMB)),
    (108, Some(xb::START)),
    (109, Some(xb::BACK)),
];
const KEY_L2: u32 = 104;
/// Android's guide button (the Xbox button): taken, so the phone doesn't
/// act on it, and otherwise unused, as on the PC.
const KEY_MODE: u32 = 110;

/// A controller reported as window events (Android's): its buttons as
/// they go down and up, and its axes as they move.
#[derive(Debug, Default)]
pub struct Events {
    /// Any of its events seen: a controller is there.
    seen: bool,
    buttons: u16,
    /// L2 and R2 as buttons, for pads whose triggers have no axis.
    digital_triggers: [bool; 2],
    axes: [f32; axis::COUNT],
}

impl Events {
    /// A key event with Android key code `code`: true when it is the
    /// pad's (and so taken).
    pub fn key(&mut self, code: u32, pressed: bool) -> bool {
        if code == KEY_MODE {
            self.seen = true;
            return true;
        }
        let Some(&(_, bit)) = KEYS.iter().find(|(c, _)| *c == code) else {
            return false;
        };
        self.seen = true;
        match bit {
            Some(bit) if pressed => self.buttons |= bit,
            Some(bit) => self.buttons &= !bit,
            None => self.digital_triggers[usize::from(code != KEY_L2)] = pressed,
        }
        true
    }

    /// Axis `id` (Android's number) moved to `value`.
    pub fn axis(&mut self, id: u32, value: f32) {
        if let Some(slot) = self.axes.get_mut(id as usize) {
            *slot = value;
            self.seen = true;
        }
    }

    /// The pad as the events left it, in XInput's terms (the same dead
    /// zones and trigger threshold); `None` before any event.
    pub fn state(&self) -> Option<PadState> {
        if !self.seen {
            return None;
        }
        let a = |id: u32| self.axes[id as usize];
        // Android's sticks are -1..=1 with down positive; XInput's raw
        // axes are -32768..=32767 with up positive.
        let raw = |v: f32| (v.clamp(-1.0, 1.0) * 32767.0) as i16;
        let stick = |x: u32, y: u32, dead| m2_sys::xinput::stick(raw(a(x)), raw(-a(y)), dead);
        let trigger = |analog: f32, digital: bool| {
            if digital {
                1.0
            } else {
                m2_sys::xinput::trigger((analog.clamp(0.0, 1.0) * 255.0) as u8)
            }
        };
        let mut buttons = self.buttons;
        for (held, bit) in [
            (a(axis::HAT_Y) < -0.5, xb::DPAD_UP),
            (a(axis::HAT_Y) > 0.5, xb::DPAD_DOWN),
            (a(axis::HAT_X) < -0.5, xb::DPAD_LEFT),
            (a(axis::HAT_X) > 0.5, xb::DPAD_RIGHT),
        ] {
            if held {
                buttons |= bit;
            }
        }
        Some(PadState {
            buttons,
            left: stick(axis::X, axis::Y, LEFT_DEAD_ZONE),
            right: stick(axis::Z, axis::RZ, RIGHT_DEAD_ZONE),
            left_trigger: trigger(
                a(axis::LTRIGGER).max(a(axis::BRAKE)),
                self.digital_triggers[0],
            ),
            right_trigger: trigger(
                a(axis::RTRIGGER).max(a(axis::GAS)),
                self.digital_triggers[1],
            ),
        })
    }
}

/// The actions the pad reaches in a layout.
#[cfg(test)]
fn reached(layout: &[(Button, Action)]) -> Vec<Action> {
    let mut out: Vec<Action> = layout.iter().chain(HIJACK).map(|&(_, a)| a).collect();
    // The left stick moves (steers), the right looks; L3 sprints.
    out.extend([
        Action::MoveLeft,
        Action::MoveRight,
        Action::MoveForward,
        Action::MoveBack,
        Action::Sprint,
        Action::Walk,
    ]);
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scripts_shake_runs_its_seconds_and_only_with_vibration_on() {
        let mut p = Pad::default();
        p.rumble(1.0);
        assert_eq!(p.shake(0.1), [0.0; 2]);
        p.rumble_on = true;
        p.rumble(0.3);
        p.rumble(1.0);
        p.rumble(0.3);
        for _ in 0..4 {
            assert_eq!(p.shake(0.25), RUMBLE);
        }
        assert_eq!(p.shake(0.25), [0.0; 2]);
        p.rumble(1.0);
        p.rumble_on = false;
        assert_eq!(p.shake(0.1), [0.0; 2]);
    }

    fn press(buttons: u16) -> PadState {
        PadState {
            buttons,
            ..PadState::default()
        }
    }

    #[test]
    fn the_layouts_reach_every_action_play_reads() {
        // What play reads ([`crate::player::Intent`], the pause and the
        // hijacks); the free camera's and the developer's keys aside.
        let play = [
            Action::MoveForward,
            Action::MoveBack,
            Action::MoveLeft,
            Action::MoveRight,
            Action::MoveUp,
            Action::MoveDown,
            Action::Sprint,
            Action::Walk,
            Action::Use,
            Action::Reset,
            Action::Reload,
            Action::Fire,
            Action::Aim,
            Action::Binoculars,
            Action::Throw,
            Action::Explosive,
            Action::Detonate,
            Action::Seat,
            Action::Melee,
            Action::SwitchWeapon,
            Action::Support,
            Action::SupportDown,
            Action::Pda,
            Action::Quit,
            Action::PadY,
            Action::PadA,
            Action::PadX,
            Action::PadB,
        ];
        let mut all = reached(ON_FOOT);
        all.extend(reached(DRIVING));
        let missing: Vec<_> = play.iter().filter(|a| !all.contains(a)).collect();
        assert!(missing.is_empty(), "{missing:?}");
    }

    #[test]
    fn buttons_press_and_let_go_their_actions() {
        let mut pad = Pad::default();
        let mut input = Input::default();
        pad.feed(&mut input, press(xb::A | xb::X), false, 0.016);
        assert!(input.pad());
        assert!(input.take_pressed(Action::MoveUp));
        assert!(input.take_pressed(Action::Reload));
        // A is the hijack's too.
        assert!(input.take_pressed(Action::PadA));
        assert!(input.held(Action::MoveUp));
        pad.feed(&mut input, press(xb::X), false, 0.016);
        assert!(!input.held(Action::MoveUp));
        assert!(!input.take_pressed(Action::Reload), "one press");
        // Into a car with X held: X's on-foot reload is let go; the
        // reverse waits for the next press.
        pad.feed(&mut input, press(xb::X), true, 0.016);
        assert!(!input.held(Action::Reload));
        assert!(!input.held(Action::MoveBack));
        pad.feed(&mut input, PadState::default(), true, 0.016);
        pad.feed(&mut input, press(xb::X), true, 0.016);
        assert!(input.held(Action::MoveBack));
    }

    #[test]
    fn triggers_fire_and_crouch_past_half_way() {
        let mut pad = Pad::default();
        let mut input = Input::default();
        let s = PadState {
            right_trigger: 0.8,
            left_trigger: 0.3,
            ..PadState::default()
        };
        pad.feed(&mut input, s, false, 0.016);
        assert!(input.held(Action::Fire));
        assert!(!input.held(Action::MoveDown));
        // Driving, the left trigger is the handbrake.
        let s = PadState {
            left_trigger: 1.0,
            ..PadState::default()
        };
        pad.feed(&mut input, s, true, 0.016);
        assert!(input.held(Action::MoveUp));
        assert!(!input.held(Action::Fire));
    }

    #[test]
    fn sticks_move_look_and_push() {
        let mut pad = Pad::default();
        let mut input = Input::default();
        let s = PadState {
            left: [0.0, 1.0],
            right: [0.5, 0.0],
            ..PadState::default()
        };
        pad.feed(&mut input, s, false, 0.5);
        assert_eq!(input.axis(Action::MoveBack, Action::MoveForward), 1.0);
        assert!(input.held(Action::MoveForward), "a push");
        assert!(
            input.take_pressed(Action::MoveForward),
            "the hijack's press"
        );
        let (dx, dy) = input.take_pad_look();
        assert!((dx - 0.25 * LOOK_RATE * 0.5).abs() < 1e-9 && dy == 0.0);
        // The pause menu's options: twice as fast, up and down swapped.
        pad.look = Some(2.0);
        pad.invert = true;
        let up = PadState {
            right: [0.0, 0.5],
            ..PadState::default()
        };
        pad.feed(&mut input, up, false, 0.5);
        let (_, dy) = input.take_pad_look();
        assert!((dy - 0.25 * LOOK_RATE).abs() < 1e-9, "{dy}");
        pad.look = None;
        pad.invert = false;
        // Half way or less walks.
        let s = PadState {
            left: [0.3, 0.0],
            ..PadState::default()
        };
        pad.feed(&mut input, s, false, 0.016);
        assert!(input.held(Action::Walk));
        assert!((input.axis(Action::MoveLeft, Action::MoveRight) - 0.3).abs() < 1e-6);
        // In a car the stick only steers.
        let s = PadState {
            left: [0.0, 1.0],
            ..PadState::default()
        };
        pad.feed(&mut input, s, true, 0.016);
        assert_eq!(input.axis(Action::MoveBack, Action::MoveForward), 0.0);
    }

    /// Playtest: at full lock the pedals stay whole (they used to share
    /// one push with the steering, 0.71 each, too little for reverse);
    /// on foot a diagonal is still no faster than straight.
    #[test]
    fn the_pedals_dont_share_a_push_with_the_steering() {
        let config = crate::config::CameraConfig::default();
        let mut pad = Pad::default();
        let mut input = Input::default();
        let lock = |buttons| PadState {
            buttons,
            left: [-1.0, -0.3],
            ..PadState::default()
        };
        pad.feed(&mut input, lock(xb::X), true, 0.016);
        let i = crate::player::Intent::from_input(&mut input, &config, false);
        assert_eq!(i.movement, m2_math::glam::Vec2::new(-1.0, -1.0));
        pad.feed(&mut input, lock(xb::A), true, 0.016);
        let i = crate::player::Intent::from_input(&mut input, &config, false);
        assert_eq!(i.movement, m2_math::glam::Vec2::new(-1.0, 1.0));
        let s = PadState {
            left: [1.0, 1.0],
            ..PadState::default()
        };
        pad.feed(&mut input, s, false, 0.016);
        let i = crate::player::Intent::from_input(&mut input, &config, false);
        assert!((i.walking().length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn l3_latches_sprint_until_the_stick_rests() {
        let mut pad = Pad::default();
        let mut input = Input::default();
        let push = |buttons| PadState {
            buttons,
            left: [0.0, 1.0],
            ..PadState::default()
        };
        pad.feed(&mut input, push(xb::LEFT_THUMB), false, 0.016);
        pad.feed(&mut input, push(0), false, 0.016);
        assert!(input.held(Action::Sprint));
        pad.feed(&mut input, PadState::default(), false, 0.016);
        assert!(!input.held(Action::Sprint));
    }

    #[test]
    fn menus_take_a_b_and_the_d_pad() {
        let none = PadState::default();
        assert_eq!(
            Pad::menu_keys(&none, &press(xb::A | xb::DPAD_DOWN)),
            [MenuKey::Enter, MenuKey::Down]
        );
        assert!(Pad::menu_keys(&press(xb::B), &press(xb::B)).is_empty());
    }

    #[test]
    fn android_events_make_the_same_pad_state() {
        let mut ev = Events::default();
        assert_eq!(ev.state(), None, "no controller before its events");
        // Android's KEYCODE_BUTTON_A down, then up; a keyboard's key isn't
        // the pad's.
        assert!(ev.key(96, true));
        assert!(!ev.key(29, true), "KEYCODE_A, a letter");
        assert!(ev.state().unwrap().held(xb::A));
        assert!(ev.key(96, false));
        assert!(!ev.state().unwrap().held(xb::A));
        assert!(ev.key(110, true), "the guide button is taken");
        // The left stick pushed up (Android's Y negative) and right; the
        // right stick (Z, RZ) inside its dead zone.
        ev.axis(axis::X, 1.0);
        ev.axis(axis::Y, -1.0);
        ev.axis(axis::Z, 0.1);
        let s = ev.state().unwrap();
        assert!((s.left[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3);
        assert!((s.left[1] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3);
        assert_eq!(s.right, [0.0, 0.0]);
        // The hat is the d-pad; the brake axis the left trigger.
        ev.axis(axis::HAT_Y, 1.0);
        ev.axis(axis::HAT_X, -1.0);
        ev.axis(axis::BRAKE, 1.0);
        let s = ev.state().unwrap();
        assert!(s.held(xb::DPAD_DOWN) && s.held(xb::DPAD_LEFT) && !s.held(xb::DPAD_UP));
        assert_eq!(s.left_trigger, 1.0);
        assert_eq!(s.right_trigger, 0.0);
        // R2 as a button pulls the right trigger all the way.
        assert!(ev.key(105, true));
        assert_eq!(ev.state().unwrap().right_trigger, 1.0);
        // An axis Android has but the pad doesn't know is ignored.
        ev.axis(200, 1.0);
    }

    #[test]
    fn android_events_feed_the_layout() {
        let mut ev = Events::default();
        ev.key(103, true);
        ev.axis(axis::RTRIGGER, 0.9);
        let mut pad = Pad::default();
        let mut input = Input::default();
        pad.feed(&mut input, ev.state().unwrap(), false, 0.016);
        assert!(input.held(Action::Fire));
        assert!(input.take_pressed(Action::SwitchWeapon));
    }

    #[test]
    fn every_button_has_the_games_picture() {
        for b in [
            Button::A,
            Button::B,
            Button::X,
            Button::Y,
            Button::LeftBumper,
            Button::RightBumper,
            Button::LeftTrigger,
            Button::RightTrigger,
            Button::LeftStick,
            Button::RightStick,
            Button::Start,
            Button::Back,
            Button::DpadUp,
            Button::DpadDown,
            Button::DpadLeft,
            Button::DpadRight,
        ] {
            assert!(b.texture().starts_with("button_"), "{b:?}");
        }
    }
}
