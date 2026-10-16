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
//! Invented where the alpha has a thing the layout doesn't: R3 raises the
//! weapon on foot ([`Action::Aim`], beside the zoom the layout gives it)
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

use m2_sys::xinput::{PadState, buttons as xb};

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
    (Button::RightStick, Action::Aim),
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
}

impl Pad {
    /// Reads the pad and feeds `input` (a no-op without one). `driving`
    /// picks the layout; `dt` is the seconds since the last read. Answers
    /// the menu keys pressed since then ([`Pad::menu_keys`]).
    pub fn poll(&mut self, input: &mut Input, driving: bool, dt: f64) -> Vec<MenuKey> {
        let state = m2_sys::xinput::poll();
        if state.is_some() != self.connected {
            self.connected = state.is_some();
            eprintln!(
                "pad: Xbox controller {}",
                if self.connected {
                    "connected"
                } else {
                    "disconnected"
                }
            );
        }
        let s = state.unwrap_or_default();
        let keys = Self::menu_keys(&self.last, &s);
        self.feed(input, s, driving, dt);
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
        let curve = |v: f32| f64::from(v * v.abs());
        input.pad_look(curve(rx) * LOOK_RATE * dt, -curve(ry) * LOOK_RATE * dt);
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
