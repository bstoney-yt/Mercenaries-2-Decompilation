//! Touch controls for a phone without a controller: an on-screen pad whose
//! fingers come out as the controller's own reading ([`PadState`]), so
//! the pad's layout (on foot, driving, the hijacks, the menus' keys) works
//! unchanged ([`crate::pad`]). Invented, not the game's (it had none).
//!
//! The left part of the screen is a stick that centres where a finger
//! lands (the left stick: move, steer); the rest turns the view as the
//! finger drags (as the mouse does); buttons stand at the edges, with the
//! pad's names: fire and aim (the triggers), jump, melee, reload, use
//! (A B X Y), switch weapon and throw (the bumpers), sprint (L3), the
//! d-pad (support, explosives, detonate), pause (Start) and the PDA
//! (Back). A finger that lands on fire also turns the view as it drags,
//! to aim while firing.

use std::collections::HashMap;

use m2_sys::xinput::{PadState, buttons as xb};

/// How far a finger that lands on the left part of the screen pushes the
/// stick fully, as a share of the screen's height.
const STICK_REACH: f32 = 0.12;
/// The left part of the screen, as a share of its width, that is the stick.
const STICK_SIDE: f32 = 0.4;
/// Mouse counts of turn per pixel dragged.
const LOOK_PER_PIXEL: f64 = 0.6;

/// What one of the on-screen buttons is: a pad button or a trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Button(u16),
    LeftTrigger,
    RightTrigger,
}

/// An on-screen button: what it presses, its label, and where it stands
/// (centre as shares of the screen's width and height, radius as a share
/// of its height).
#[derive(Clone, Copy, Debug)]
pub struct Key {
    pub press: Press,
    pub label: &'static str,
    pub at: [f32; 2],
    pub radius: f32,
}

const fn key(press: Press, label: &'static str, x: f32, y: f32, radius: f32) -> Key {
    Key {
        press,
        label,
        at: [x, y],
        radius,
    }
}

/// The buttons, as they stand (first found wins where they overlap).
pub const KEYS: [Key; 16] = [
    key(Press::RightTrigger, "FIRE", 0.90, 0.60, 0.095),
    key(Press::LeftTrigger, "AIM", 0.75, 0.62, 0.07),
    key(Press::Button(xb::A), "JUMP", 0.81, 0.86, 0.065),
    key(Press::Button(xb::B), "HIT", 0.94, 0.86, 0.055),
    key(Press::Button(xb::X), "RELOAD", 0.68, 0.86, 0.055),
    key(Press::Button(xb::Y), "USE", 0.86, 0.38, 0.06),
    key(
        Press::Button(xb::RIGHT_SHOULDER),
        "WEAPON",
        0.96,
        0.24,
        0.05,
    ),
    key(Press::Button(xb::LEFT_SHOULDER), "THROW", 0.73, 0.38, 0.05),
    key(Press::Button(xb::LEFT_THUMB), "SPRINT", 0.34, 0.66, 0.05),
    key(Press::Button(xb::RIGHT_THUMB), "ZOOM", 0.62, 0.62, 0.045),
    key(Press::Button(xb::DPAD_UP), "▲", 0.08, 0.30, 0.04),
    key(Press::Button(xb::DPAD_DOWN), "▼", 0.08, 0.50, 0.04),
    key(Press::Button(xb::DPAD_LEFT), "◀", 0.04, 0.40, 0.04),
    key(Press::Button(xb::DPAD_RIGHT), "▶", 0.12, 0.40, 0.04),
    key(Press::Button(xb::BACK), "PDA", 0.44, 0.07, 0.04),
    key(Press::Button(xb::START), "PAUSE", 0.56, 0.07, 0.04),
];

/// What a finger is doing, from where it landed.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Role {
    /// Holds button `KEYS[i]`.
    Key(usize),
    /// Pushes the stick, centred where it landed.
    Stick { centre: [f32; 2] },
    /// Turns the view as it drags.
    Look,
}

#[derive(Clone, Copy, Debug)]
struct Finger {
    role: Role,
    at: [f32; 2],
}

/// The fingers on the screen and what they do, in the window's pixels.
#[derive(Debug, Default)]
pub struct Touch {
    size: [f32; 2],
    fingers: HashMap<u64, Finger>,
    /// Turn dragged since last taken, mouse counts.
    look: (f64, f64),
    /// A finger has touched the screen: the controls show from then on.
    pub used: bool,
    /// Sprint, switched on and off by tapping its button (not held): she
    /// sprints whenever she moves while it is on.
    pub sprint: bool,
}

impl Touch {
    /// The window is `width` x `height` pixels.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = [width as f32, height as f32];
    }

    /// Finger `id` landed at `at` (pixels).
    pub fn down(&mut self, id: u64, at: [f32; 2]) {
        self.used = true;
        let [w, h] = self.size;
        let role = KEYS
            .iter()
            .position(|k| {
                let c = [k.at[0] * w, k.at[1] * h];
                let r = k.radius * h * 1.25;
                (at[0] - c[0]).powi(2) + (at[1] - c[1]).powi(2) <= r * r
            })
            .map_or_else(
                || {
                    if at[0] < STICK_SIDE * w {
                        Role::Stick { centre: at }
                    } else {
                        Role::Look
                    }
                },
                Role::Key,
            );
        if let Role::Key(i) = role
            && KEYS[i].press == Press::Button(xb::LEFT_THUMB)
        {
            self.sprint = !self.sprint;
        }
        self.fingers.insert(id, Finger { role, at });
    }

    /// Finger `id` moved to `at`.
    pub fn moved(&mut self, id: u64, at: [f32; 2]) {
        let Some(f) = self.fingers.get_mut(&id) else {
            return;
        };
        let looks = match f.role {
            Role::Look => true,
            Role::Key(i) => KEYS[i].press == Press::RightTrigger,
            Role::Stick { .. } => false,
        };
        if looks {
            self.look.0 += f64::from(at[0] - f.at[0]) * LOOK_PER_PIXEL;
            self.look.1 += f64::from(at[1] - f.at[1]) * LOOK_PER_PIXEL;
        }
        f.at = at;
    }

    /// Finger `id` lifted (or the system took it).
    pub fn up(&mut self, id: u64) {
        self.fingers.remove(&id);
    }

    /// Lets every finger go (the window lost focus, a menu opened).
    pub fn clear(&mut self) {
        self.fingers.clear();
        self.look = (0.0, 0.0);
    }

    /// The turn dragged since the last call, mouse counts (right, down).
    pub fn take_look(&mut self) -> (f64, f64) {
        std::mem::take(&mut self.look)
    }

    /// The pad the fingers make.
    pub fn state(&self) -> PadState {
        let mut s = PadState::default();
        let reach = STICK_REACH * self.size[1];
        for f in self.fingers.values() {
            match f.role {
                Role::Key(i) => match KEYS[i].press {
                    // Sprint is switched, not held ([`Touch::sprint`]).
                    Press::Button(xb::LEFT_THUMB) => {}
                    Press::Button(bit) => s.buttons |= bit,
                    Press::LeftTrigger => s.left_trigger = 1.0,
                    Press::RightTrigger => s.right_trigger = 1.0,
                },
                Role::Stick { centre } if reach > 0.0 => {
                    // Up the screen is up the stick.
                    let x = (f.at[0] - centre[0]) / reach;
                    let y = -(f.at[1] - centre[1]) / reach;
                    let len = x.hypot(y);
                    let scale = if len > 1.0 { 1.0 / len } else { 1.0 };
                    s.left = [x * scale, y * scale];
                }
                Role::Stick { .. } | Role::Look => {}
            }
        }
        s
    }

    /// Where the stick is drawn: its centre and the finger's spot, when a
    /// finger holds it.
    pub fn stick(&self) -> Option<([f32; 2], [f32; 2])> {
        self.fingers.values().find_map(|f| match f.role {
            Role::Stick { centre } => Some((centre, f.at)),
            _ => None,
        })
    }

    /// Whether button `KEYS[i]` is held.
    pub fn held(&self, i: usize) -> bool {
        self.fingers.values().any(|f| f.role == Role::Key(i))
    }
}

/// Merges the touch pad `t` into a controller's reading `s`: either one's
/// buttons, the larger push of each stick and trigger.
pub fn merge(s: PadState, t: PadState) -> PadState {
    let bigger = |a: [f32; 2], b: [f32; 2]| {
        if b[0].hypot(b[1]) > a[0].hypot(a[1]) {
            b
        } else {
            a
        }
    };
    PadState {
        buttons: s.buttons | t.buttons,
        left: bigger(s.left, t.left),
        right: bigger(s.right, t.right),
        left_trigger: s.left_trigger.max(t.left_trigger),
        right_trigger: s.right_trigger.max(t.right_trigger),
    }
}

/// What the controls look like this frame, for the HUD to draw
/// ([`paint`]): which buttons are held, and the stick where it is held.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Picture {
    pub held: [bool; KEYS.len()],
    pub stick: Option<([f32; 2], [f32; 2])>,
}

impl Touch {
    /// This frame's [`Picture`].
    pub fn picture(&self) -> Picture {
        Picture {
            held: std::array::from_fn(|i| {
                if KEYS[i].press == Press::Button(xb::LEFT_THUMB) {
                    self.sprint
                } else {
                    self.held(i)
                }
            }),
            stick: self.stick(),
        }
    }
}

/// Draws the controls over the play screen (`rect`, in points) on
/// `painter`: the buttons, held ones lit, and the stick where it is held.
pub fn paint(touch: &Picture, painter: &egui::Painter, rect: egui::Rect, pixels_per_point: f32) {
    use egui::{Align2, Color32, FontId, Stroke, pos2};
    let (w, h) = (rect.width(), rect.height());
    let line = Color32::from_rgba_unmultiplied(232, 208, 148, 150);
    for (i, k) in KEYS.iter().enumerate() {
        let c = pos2(rect.left() + k.at[0] * w, rect.top() + k.at[1] * h);
        let r = k.radius * h;
        let fill = if touch.held[i] {
            Color32::from_rgba_unmultiplied(232, 208, 148, 110)
        } else {
            Color32::from_rgba_unmultiplied(20, 22, 10, 70)
        };
        painter.circle(c, r, fill, Stroke::new(2.0, line));
        painter.text(
            c,
            Align2::CENTER_CENTER,
            k.label,
            FontId::proportional((r * 0.55).clamp(9.0, 22.0)),
            Color32::from_rgba_unmultiplied(240, 225, 190, 220),
        );
    }
    if let Some((centre, at)) = touch.stick {
        let p = |v: [f32; 2]| pos2(v[0] / pixels_per_point, v[1] / pixels_per_point);
        let reach = STICK_REACH * h;
        painter.circle(
            p(centre),
            reach,
            Color32::from_rgba_unmultiplied(20, 22, 10, 60),
            Stroke::new(2.0, line),
        );
        let knob = p(at);
        let off = knob - p(centre);
        let knob = if off.length() > reach {
            p(centre) + off.normalized() * reach
        } else {
            knob
        };
        painter.circle_filled(knob, reach * 0.4, line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch() -> Touch {
        let mut t = Touch::default();
        t.resize(2000, 1000);
        t
    }

    #[test]
    fn a_finger_on_fire_pulls_the_right_trigger_and_aims_as_it_drags() {
        let mut t = touch();
        t.down(1, [0.90 * 2000.0, 0.60 * 1000.0]);
        assert_eq!(t.state().right_trigger, 1.0);
        t.moved(1, [0.90 * 2000.0 + 10.0, 600.0]);
        assert!((t.take_look().0 - 10.0 * LOOK_PER_PIXEL).abs() < 1e-9);
        t.up(1);
        assert_eq!(t.state(), PadState::default());
    }

    #[test]
    fn the_left_side_is_a_stick_centred_where_the_finger_lands() {
        let mut t = touch();
        t.down(7, [300.0, 700.0]);
        assert_eq!(t.state().left, [0.0, 0.0]);
        // Half the reach to the right and the full reach up: up is up,
        // and the push is held to the rim.
        t.moved(7, [360.0, 580.0]);
        let [x, y] = t.state().left;
        assert!(y > 0.8 && x > 0.4 && x.hypot(y) <= 1.0 + 1e-6, "{x} {y}");
    }

    #[test]
    fn the_rest_of_the_screen_turns_the_view_and_presses_nothing() {
        let mut t = touch();
        t.down(2, [1100.0, 500.0]);
        t.moved(2, [1080.0, 520.0]);
        let (dx, dy) = t.take_look();
        assert!(dx < 0.0 && dy > 0.0);
        assert_eq!(t.state(), PadState::default());
    }

    #[test]
    fn buttons_hold_their_pad_bits_with_two_fingers_at_once() {
        let mut t = touch();
        let at = |k: &Key| [k.at[0] * 2000.0, k.at[1] * 1000.0];
        let jump = KEYS.iter().find(|k| k.label == "JUMP").unwrap();
        let use_ = KEYS.iter().find(|k| k.label == "USE").unwrap();
        t.down(1, at(jump));
        t.down(2, at(use_));
        let s = t.state();
        assert!(s.held(xb::A) && s.held(xb::Y));
        t.up(1);
        assert!(!t.state().held(xb::A));
    }

    #[test]
    fn a_tap_on_sprint_switches_it_on_and_another_off() {
        let mut t = touch();
        let sprint = KEYS.iter().find(|k| k.label == "SPRINT").unwrap();
        let at = [sprint.at[0] * 2000.0, sprint.at[1] * 1000.0];
        t.down(1, at);
        t.up(1);
        assert!(t.sprint);
        // Not a held stick press: the pad's own L3 latch stays out of it.
        assert!(!t.state().held(xb::LEFT_THUMB));
        let lit = KEYS.iter().position(|k| k.label == "SPRINT").unwrap();
        assert!(t.picture().held[lit]);
        t.down(2, at);
        t.up(2);
        assert!(!t.sprint);
    }

    #[test]
    fn a_controller_and_the_touch_pad_together() {
        let pad = PadState {
            buttons: xb::B,
            left: [0.2, 0.0],
            ..PadState::default()
        };
        let touch = PadState {
            buttons: xb::A,
            left: [0.0, 0.9],
            right_trigger: 1.0,
            ..PadState::default()
        };
        let m = merge(pad, touch);
        assert_eq!(m.buttons, xb::A | xb::B);
        assert_eq!(m.left, [0.0, 0.9]);
        assert_eq!(m.right_trigger, 1.0);
    }
}
