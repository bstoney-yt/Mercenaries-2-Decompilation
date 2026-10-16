//! Actions, their key and mouse bindings, and the input state the game
//! reads each tick.
//!
//! The default bindings are the PC game's own defaults, as its controls
//! screen lists them (from the PC footage: the pause menu's Controls
//! page, keyboard and mouse):
//!
//! | control          | keys                  | here                       |
//! |------------------|-----------------------|----------------------------|
//! | move, strafe     | W S, A D              | [`Action::MoveForward`]... |
//! | primary attack   | left click            | [`Action::Fire`]           |
//! | primary switch   | Tab, mouse wheel      | [`Action::SwitchWeapon`]   |
//! | reload           | R                     | [`Action::Reload`]         |
//! | secondary attack | right click, G        | [`Action::Throw`]          |
//! | secondary switch | V                     | [`Action::Explosive`]      |
//! | melee            | F, middle click       | [`Action::Melee`]          |
//! | use              | E                     | [`Action::Use`]            |
//! | jump             | Space                 | [`Action::MoveUp`]         |
//! | crouch           | C                     | [`Action::MoveDown`], aim  |
//! | binoculars       | B                     | [`Action::Aim`]            |
//! | PDA              | F1                    | [`Action::Pda`]            |
//! | look             | I K J L               | [`Action::LookUp`]...      |
//! | support up, down | 1 or Up, 2 or Down    | [`Action::Support`], down  |
//! | view switch      | 4 or Right            | [`Action::ViewSwitch`]     |
//! | walk, sprint     | Left Ctrl, Left Shift | [`Action::Walk`], sprint   |
//!
//! Inferred: crouch also raises the weapon ([`Action::Aim`]); the game's
//! crouch is how she aims steadily (the owner's play of the PC game).
//! Invented: B raises the weapon until the alpha has the game's binocular
//! view (`MrxGuiBinocularsLayout`); the view switch is bound but does
//! nothing yet (the alpha has one camera); the alpha's own keys (the
//! seats Y, C4 X, the shop H, music M, the mouse's release F2, the
//! developer's F3 to F9, among them the cheat menu's F7) sit on keys the
//! table leaves free. The hijack's face buttons stay on I K J L beside the
//! look (the PC game's are not known).

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use winit::keyboard::KeyCode;

/// Something the player can do. Gameplay reads actions, never keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    MoveForward,
    MoveBack,
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    Sprint,
    /// Hold to walk instead of run.
    Walk,
    /// Switch between playing on foot and the free camera.
    ToggleFly,
    /// Play the next music track, or stop the music.
    ToggleMusic,
    /// Get in or out of a vehicle.
    Use,
    /// Turn an overturned vehicle back up (driving).
    Reset,
    /// Change the rifle's clip (on foot; the same key as
    /// [`Reset`](Self::Reset) by default).
    Reload,
    /// Fire the weapon in hand (also the left mouse button).
    Fire,
    /// Raise the weapon (also the right mouse button, once the mouse is
    /// captured).
    Aim,
    /// Throw a grenade (or the explosive chosen with
    /// [`Explosive`](Self::Explosive)).
    Throw,
    /// Choose the next explosive to throw: grenades, C4, proximity mines.
    Explosive,
    /// Set off the C4 she has placed.
    Detonate,
    /// Move to the next seat of the vehicle she rides (the driver's, the
    /// gunner's).
    Seat,
    /// A melee bash with what she holds (also the middle mouse button,
    /// once the mouse is captured): opens a supply crate.
    Melee,
    /// Take up the rifle, or what she carries in the launcher's place
    /// (the RPG, or the grenade launcher once she has taken one up).
    Rifle,
    Launcher,
    /// Open or close the support menu (her support items: the first
    /// contract's Satellite Guided Bomb).
    Support,
    /// Open or close the PDA (the first contract has her look at it before
    /// it gives her the bomb).
    Pda,
    /// Open the shop (the game opens it from a contact's menu; here it
    /// opens anywhere).
    Shop,
    /// Capture or release the mouse for looking around.
    ToggleMouse,
    Fullscreen,
    /// Step the shown characters through idle, walk, run and sprint.
    NextGait,
    /// Draw back faces too (a debugging aid while formats are checked).
    ToggleCulling,
    /// Save the game to the quick-save file, or load it back.
    QuickSave,
    QuickLoad,
    /// Open or close the test cheat menu (a developer's key:
    /// [`crate::cheats`]).
    Cheats,
    Quit,
    /// The pad's face buttons Y, A, X and B, which a hijack's prompts ask
    /// for ([`crate::hijack`]); the left stick's pushes are the movement
    /// keys.
    PadY,
    PadA,
    PadX,
    PadB,
    /// Take up the other weapon: the rifle, or what she carries in the
    /// launcher's place (the pad's right bumper, `Switch_Primary`).
    SwitchWeapon,
    /// The support menu's line down (its [`Support`](Self::Support) is
    /// up); either opens the menu while it is shut.
    SupportDown,
    /// The PC game's view switch (not yet: the alpha has one camera).
    ViewSwitch,
    /// Look with the keys, as the mouse does ([`KEY_LOOK_RATE`]).
    LookUp,
    LookDown,
    LookLeft,
    LookRight,
}

/// How fast the look keys turn the view, mouse counts a second: as the
/// pad's right stick at full push ([`crate::pad::LOOK_RATE`]).
pub const KEY_LOOK_RATE: f64 = crate::pad::LOOK_RATE;

/// A mouse button, or a notch of its wheel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mouse {
    #[serde(rename = "MouseLeft")]
    Left,
    #[serde(rename = "MouseRight")]
    Right,
    #[serde(rename = "MouseMiddle")]
    Middle,
    WheelUp,
    WheelDown,
}

/// A key or a mouse control, as written in the settings file (`"KeyW"`,
/// `"MouseLeft"`, `"WheelUp"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Control {
    Key(KeyCode),
    Mouse(Mouse),
}

impl From<KeyCode> for Control {
    fn from(key: KeyCode) -> Self {
        Self::Key(key)
    }
}

impl From<Mouse> for Control {
    fn from(m: Mouse) -> Self {
        Self::Mouse(m)
    }
}

/// Which keys and mouse controls trigger each action, as written in the
/// settings file (`fire = ["MouseLeft"]`, `move_forward = ["KeyW"]`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Bindings(pub BTreeMap<Action, Vec<Control>>);

impl Default for Bindings {
    fn default() -> Self {
        use Action::*;
        use KeyCode::*;
        use Mouse::*;
        let k = Control::Key;
        let m = Control::Mouse;
        Self(BTreeMap::from([
            (MoveForward, vec![k(KeyW)]),
            (MoveBack, vec![k(KeyS)]),
            (MoveLeft, vec![k(KeyA)]),
            (MoveRight, vec![k(KeyD)]),
            (MoveUp, vec![k(Space)]),
            (MoveDown, vec![k(KeyC)]),
            (Sprint, vec![k(ShiftLeft), k(ShiftRight)]),
            (Walk, vec![k(ControlLeft)]),
            (ToggleFly, vec![k(F4)]),
            (ToggleMusic, vec![k(KeyM)]),
            (Use, vec![k(KeyE)]),
            (Reset, vec![k(KeyR)]),
            (Reload, vec![k(KeyR)]),
            (Fire, vec![m(Mouse::Left)]),
            (Aim, vec![k(KeyC), k(KeyB)]),
            (Throw, vec![m(Mouse::Right), k(KeyG)]),
            (Explosive, vec![k(KeyV)]),
            (Detonate, vec![k(KeyX)]),
            (Seat, vec![k(KeyY)]),
            (Melee, vec![k(KeyF), m(Mouse::Middle)]),
            // The PC game switches with Tab and the wheel only.
            (Rifle, Vec::new()),
            (Launcher, Vec::new()),
            (Support, vec![k(Digit1), k(ArrowUp)]),
            (SupportDown, vec![k(Digit2), k(ArrowDown)]),
            (ViewSwitch, vec![k(Digit4), k(ArrowRight)]),
            (LookUp, vec![k(KeyI)]),
            (LookDown, vec![k(KeyK)]),
            (LookLeft, vec![k(KeyJ)]),
            (LookRight, vec![k(KeyL)]),
            (Pda, vec![k(F1)]),
            (Shop, vec![k(KeyH)]),
            (ToggleMouse, vec![k(F2)]),
            (Fullscreen, vec![k(F11)]),
            (NextGait, vec![k(KeyN)]),
            (ToggleCulling, vec![k(F3)]),
            (QuickSave, vec![k(F5)]),
            (QuickLoad, vec![k(F9)]),
            (Cheats, vec![k(F7)]),
            (Quit, vec![k(Escape)]),
            // Laid out as the face buttons are (the PC game's are not
            // known), beside the look keys.
            (PadY, vec![k(KeyI)]),
            (PadA, vec![k(KeyK)]),
            (PadX, vec![k(KeyJ)]),
            (PadB, vec![k(KeyL)]),
            (SwitchWeapon, vec![k(Tab), m(WheelUp), m(WheelDown)]),
        ]))
    }
}

impl Bindings {
    /// The actions `control` triggers.
    pub fn actions_for(&self, control: impl Into<Control>) -> impl Iterator<Item = Action> + '_ {
        let control = control.into();
        self.0
            .iter()
            .filter(move |(_, controls)| controls.contains(&control))
            .map(|(&a, _)| a)
    }

    /// The name of the first key or mouse control bound to `action` (`F1`,
    /// `Left click`), if any.
    pub fn key_name(&self, action: Action) -> Option<String> {
        self.0.get(&action)?.first().map(|&c| control_name(c))
    }
}

/// A control's name as written on it: `P`, `F1`, `Left click`.
fn control_name(control: Control) -> String {
    match control {
        Control::Key(key) => key_name(key),
        Control::Mouse(m) => match m {
            Mouse::Left => "Left click",
            Mouse::Right => "Right click",
            Mouse::Middle => "Middle click",
            Mouse::WheelUp => "Wheel up",
            Mouse::WheelDown => "Wheel down",
        }
        .to_owned(),
    }
}

/// A key's name as written on it: `P`, `1`, `Shift`, `F11`.
fn key_name(key: KeyCode) -> String {
    use KeyCode::*;
    let name = format!("{key:?}");
    if let Some(short) = name
        .strip_prefix("Key")
        .or_else(|| name.strip_prefix("Digit"))
    {
        return short.to_owned();
    }
    match key {
        ShiftLeft | ShiftRight => "Shift",
        ControlLeft | ControlRight => "Ctrl",
        AltLeft | AltRight => "Alt",
        Escape => "Esc",
        ArrowUp => "Up",
        ArrowDown => "Down",
        ArrowLeft => "Left",
        ArrowRight => "Right",
        _ => return name,
    }
    .to_owned()
}

/// The key tokens of the game's texts (`[pda] Open PDA`) and the action
/// each names here: `[pda]` the PDA's key, `[support]` the support menu's,
/// `[confirm]` the consoles' (here the use key), `[sprint]` sprint's
/// (round 98 met it after the first gate), `[reload]` the reload's and
/// `[jump]` the jump's, which is the brake when driving (`[jump]
/// Handbrake`; the play test met both), and `[bash]` the melee's. Texts
/// draw these as the fonts' button pictures where there is one
/// ([`m2_game::gui_runtime::button_glyph`]); the keys are the fallback.
/// `[pfire]` and `[move]` are named apart ([`token_keys`]).
const TOKENS: [(&str, Action); 7] = [
    ("pda", Action::Pda),
    ("support", Action::Support),
    ("confirm", Action::Use),
    ("sprint", Action::Sprint),
    ("reload", Action::Reload),
    ("jump", Action::MoveUp),
    ("bash", Action::Melee),
];

/// The key tokens of the game's texts named as the keys bound here
/// (`("pda", "F1")`): those of [`TOKENS`], `[pfire]` the fire's (the
/// left mouse button) and `[move]` the keys that move. Keys left unbound
/// are left out.
pub fn token_keys(keys: &Bindings) -> Vec<(String, String)> {
    use Action::*;
    let mut out: Vec<(String, String)> = TOKENS
        .iter()
        .filter_map(|&(token, action)| Some((token.to_owned(), keys.key_name(action)?)))
        .collect();
    if let Some(fire) = keys.key_name(Fire) {
        out.push(("pfire".to_owned(), fire));
    }
    let names: Vec<String> = [MoveForward, MoveLeft, MoveBack, MoveRight]
        .into_iter()
        .filter_map(|a| keys.key_name(a))
        .collect();
    if !names.is_empty() {
        let letters = names.iter().all(|n| n.chars().count() == 1);
        out.push((
            "move".to_owned(),
            names.join(if letters { "" } else { "/" }),
        ));
    }
    out
}

/// A line of the game's text with its key tokens (`[pda] Open PDA`) named
/// as the keys bound here, in brackets (`[P] Open PDA`; see
/// [`token_keys`]). Other tokens, and keys left unbound, stay as they are.
pub fn name_keys(line: &str, keys: &Bindings) -> String {
    let tokens = token_keys(keys);
    let named = |token: &str| {
        tokens
            .iter()
            .find(|(t, _)| t == token)
            .map(|(_, key)| key.clone())
    };
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        rest = &rest[open..];
        let Some(close) = rest.find(']') else {
            break;
        };
        match named(&rest[1..close]) {
            Some(key) => {
                out.push('[');
                out.push_str(&key);
                out.push(']');
            }
            None => out.push_str(&rest[..=close]),
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Held actions, actions pressed since the last tick, and mouse motion;
/// and the pad's sticks ([`crate::pad`]).
#[derive(Debug, Default)]
pub struct Input {
    held: HashSet<Action>,
    pressed: HashSet<Action>,
    mouse: (f64, f64),
    /// The pad was used last (not the keyboard or mouse): the texts show
    /// its pictures.
    pad: bool,
    /// What the pad holds beyond its buttons (L3's sprint, a light push's
    /// walk), and its left stick's pushes past half way (a hijack's
    /// `LStick_Left`): held, but not counted by [`axis`](Self::axis).
    pad_holds: HashSet<Action>,
    stick_pushes: HashSet<Action>,
    /// The left stick, -1..=1 right and forward.
    stick: [f32; 2],
    /// The right stick's turn since the last tick, in mouse counts.
    pad_look: (f64, f64),
}

impl Input {
    /// A key or mouse control went down (`repeat`: the system's repeat
    /// while held) or up. A wheel's notch goes down and up at once: a
    /// press.
    pub fn key(&mut self, bindings: &Bindings, key: impl Into<Control>, down: bool, repeat: bool) {
        if down {
            self.pad = false;
        }
        for action in bindings.actions_for(key) {
            if down {
                if !repeat {
                    self.pressed.insert(action);
                }
                self.held.insert(action);
            } else {
                self.held.remove(&action);
            }
        }
    }

    /// A mouse button bound to `action` went down or up.
    pub fn button(&mut self, action: Action, down: bool) {
        if down {
            self.pressed.insert(action);
            self.held.insert(action);
        } else {
            self.held.remove(&action);
        }
    }

    pub fn mouse_motion(&mut self, dx: f64, dy: f64) {
        self.mouse.0 += dx;
        self.mouse.1 += dy;
    }

    /// The keyboard and mouse were used (a mouse button): the texts go back
    /// to the keys' pictures.
    pub fn mouse_used(&mut self) {
        self.pad = false;
    }

    /// The pad was used last.
    pub fn pad(&self) -> bool {
        self.pad
    }

    pub fn set_pad(&mut self, pad: bool) {
        self.pad = pad;
    }

    /// The pad holds `action` (or not) by other than a button.
    pub fn set_pad_hold(&mut self, action: Action, down: bool) {
        if down {
            self.pad_holds.insert(action);
        } else {
            self.pad_holds.remove(&action);
        }
    }

    /// The left stick, -1..=1 right and forward.
    pub fn set_stick(&mut self, stick: [f32; 2]) {
        self.stick = stick;
    }

    /// The left stick pushes towards `action`'s way past half way (or
    /// not): a press as it gets there.
    pub fn set_stick_push(&mut self, action: Action, pushed: bool) {
        if pushed {
            if self.stick_pushes.insert(action) {
                self.pressed.insert(action);
            }
        } else {
            self.stick_pushes.remove(&action);
        }
    }

    /// The right stick turned the view by this many mouse counts.
    pub fn pad_look(&mut self, dx: f64, dy: f64) {
        self.pad_look.0 += dx;
        self.pad_look.1 += dy;
    }

    /// The look keys held turn the view for `dt` seconds, as the pad's
    /// right stick does at full push ([`KEY_LOOK_RATE`]).
    pub fn key_look(&mut self, dt: f64) {
        let x = self.axis(Action::LookLeft, Action::LookRight);
        let y = self.axis(Action::LookUp, Action::LookDown);
        self.pad_look(
            f64::from(x) * KEY_LOOK_RATE * dt,
            f64::from(y) * KEY_LOOK_RATE * dt,
        );
    }

    /// The right stick's turn since the last call, in mouse counts.
    pub fn take_pad_look(&mut self) -> (f64, f64) {
        std::mem::take(&mut self.pad_look)
    }

    /// The turn since the last call, in mouse counts: the mouse's while
    /// it is captured (`mouse_look`), and the pad's right stick's either
    /// way.
    pub fn take_look(&mut self, mouse_look: bool) -> (f64, f64) {
        let (mx, my) = self.take_mouse();
        let (px, py) = self.take_pad_look();
        if mouse_look {
            (mx + px, my + py)
        } else {
            (px, py)
        }
    }

    /// Releases everything (the window lost focus).
    pub fn clear(&mut self) {
        self.held.clear();
        self.pressed.clear();
        self.mouse = (0.0, 0.0);
        self.pad_holds.clear();
        self.stick_pushes.clear();
        self.stick = [0.0; 2];
        self.pad_look = (0.0, 0.0);
    }

    pub fn held(&self, action: Action) -> bool {
        self.held.contains(&action)
            || self.pad_holds.contains(&action)
            || self.stick_pushes.contains(&action)
    }

    /// True once per key press.
    pub fn take_pressed(&mut self, action: Action) -> bool {
        self.pressed.remove(&action)
    }

    /// Mouse movement since the last call.
    pub fn take_mouse(&mut self) -> (f64, f64) {
        std::mem::take(&mut self.mouse)
    }

    /// -1..=1 from a pair of opposing actions: their keys' -1, 0 or 1,
    /// and the left stick's tilt for the movement pairs.
    pub fn axis(&self, negative: Action, positive: Action) -> f32 {
        let keys = |a| self.held.contains(&a);
        let digital = f32::from(i8::from(keys(positive)) - i8::from(keys(negative)));
        let stick = match (negative, positive) {
            (Action::MoveLeft, Action::MoveRight) => self.stick[0],
            (Action::MoveBack, Action::MoveForward) => self.stick[1],
            _ => 0.0,
        };
        (digital + stick).clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presses_and_holds() {
        let b = Bindings::default();
        let mut input = Input::default();
        input.key(&b, KeyCode::KeyW, true, false);
        input.key(&b, KeyCode::KeyW, true, true);
        assert!(input.held(Action::MoveForward));
        assert!(input.take_pressed(Action::MoveForward));
        assert!(!input.take_pressed(Action::MoveForward));
        assert_eq!(input.axis(Action::MoveBack, Action::MoveForward), 1.0);
        input.key(&b, KeyCode::KeyW, false, false);
        assert!(!input.held(Action::MoveForward));
    }

    #[test]
    fn key_tokens_are_named_as_bound() {
        let b = Bindings::default();
        assert_eq!(name_keys("[pda] Look", &b), "[F1] Look");
        assert_eq!(
            name_keys("[support] One\n[pfire] Two [move] [confirm]", &b),
            "[1] One\n[Left click] Two [WASD] [E]"
        );
        assert_eq!(name_keys("[bash]", &b), "[F]");
        assert_eq!(name_keys("[sprint] Sprint", &b), "[Shift] Sprint");
        // The play test's: the reload, and the jump that brakes.
        assert_eq!(
            name_keys("[reload] Reload\n[jump] Handbrake", &b),
            "[R] Reload\n[Space] Handbrake"
        );
        assert_eq!(name_keys("[other] stays [", &b), "[other] stays [");
        let mut b = b;
        b.0.insert(Action::MoveForward, vec![KeyCode::ArrowUp.into()]);
        assert_eq!(name_keys("[move]", &b), "[Up/A/S/D]");
        b.0.insert(Action::Pda, Vec::new());
        assert_eq!(name_keys("[pda]", &b), "[pda]");
    }

    #[test]
    fn bindings_read_from_toml() {
        #[derive(Deserialize)]
        struct File {
            keys: Bindings,
        }
        let f: File = toml::from_str(
            "[keys]\nmove_forward = [\"KeyZ\"]\nquit = [\"KeyQ\"]\nfire = [\"MouseRight\", \"KeyF\"]",
        )
        .unwrap();
        assert_eq!(
            f.keys.actions_for(KeyCode::KeyZ).collect::<Vec<_>>(),
            [Action::MoveForward]
        );
        assert_eq!(
            f.keys.actions_for(KeyCode::KeyQ).collect::<Vec<_>>(),
            [Action::Quit]
        );
        assert_eq!(
            f.keys.actions_for(Mouse::Right).collect::<Vec<_>>(),
            [Action::Fire]
        );
        assert_eq!(
            f.keys.actions_for(KeyCode::KeyF).collect::<Vec<_>>(),
            [Action::Fire]
        );
        // And the defaults write out and read back the same.
        let text = toml::to_string(&Bindings::default()).unwrap();
        assert_eq!(
            toml::from_str::<Bindings>(&text).unwrap(),
            Bindings::default()
        );
        assert!(text.contains("fire = [\"MouseLeft\"]"), "{text}");
    }

    /// The PC game's controls screen (keyboard and mouse defaults, from
    /// the PC footage): each control and the action it does here.
    const PC_DEFAULTS: [(&str, Control, Action); 30] = {
        use Action::*;
        use Control::{Key as K, Mouse as M};
        use KeyCode::*;
        use Mouse::*;
        [
            ("move", K(KeyW), MoveForward),
            ("move", K(KeyS), MoveBack),
            ("strafe", K(KeyA), MoveLeft),
            ("strafe", K(KeyD), MoveRight),
            ("primary attack", M(Mouse::Left), Fire),
            ("primary switch", K(Tab), SwitchWeapon),
            ("primary switch", M(WheelUp), SwitchWeapon),
            ("reload", K(KeyR), Reload),
            ("secondary attack", M(Mouse::Right), Throw),
            ("secondary attack", K(KeyG), Throw),
            ("secondary switch", K(KeyV), Explosive),
            ("melee", K(KeyF), Melee),
            ("melee", M(Mouse::Middle), Melee),
            ("use", K(KeyE), Use),
            ("jump", K(Space), MoveUp),
            ("crouch", K(KeyC), MoveDown),
            ("binoculars", K(KeyB), Aim),
            ("pda", K(F1), Pda),
            ("look", K(KeyI), LookUp),
            ("look", K(KeyK), LookDown),
            ("look", K(KeyJ), LookLeft),
            ("look", K(KeyL), LookRight),
            ("support up", K(Digit1), Support),
            ("support up", K(ArrowUp), Support),
            ("support down", K(Digit2), SupportDown),
            ("support down", K(ArrowDown), SupportDown),
            ("view switch", K(Digit4), ViewSwitch),
            ("view switch", K(ArrowRight), ViewSwitch),
            ("walk", K(ControlLeft), Walk),
            ("sprint", K(ShiftLeft), Sprint),
        ]
    };

    /// The PC table's controls that don't do their action in `b`, and
    /// those that also do a gameplay action the table gives another
    /// control (the alpha's old F that fired, C that also flew down).
    fn pc_misses(b: &Bindings) -> Vec<String> {
        let table: HashSet<Action> = PC_DEFAULTS.iter().map(|&(_, _, a)| a).collect();
        let mut out = Vec::new();
        for &(name, control, action) in &PC_DEFAULTS {
            let does: Vec<Action> = b.actions_for(control).collect();
            if !does.contains(&action) {
                out.push(format!("{name} {control:?}: {does:?}"));
            }
            // Crouch raises the weapon too; the hijack's face buttons sit
            // beside the look.
            let stray: Vec<&Action> = does
                .iter()
                .filter(|&&a| a != action && table.contains(&a))
                .filter(|&&a| !(action == Action::MoveDown && a == Action::Aim))
                .collect();
            if !stray.is_empty() {
                out.push(format!("{name} {control:?} also {stray:?}"));
            }
        }
        out
    }

    #[test]
    fn f7_opens_the_cheat_menu_and_nothing_else() {
        let b = Bindings::default();
        let on: Vec<Action> = b.actions_for(KeyCode::F7).collect();
        assert_eq!(on, [Action::Cheats]);
    }

    #[test]
    fn the_defaults_are_the_pc_games() {
        let b = Bindings::default();
        assert_eq!(pc_misses(&b), Vec::<String>::new());
        // Crouch raises the weapon; the wheel switches either way.
        assert!(b.actions_for(KeyCode::KeyC).any(|a| a == Action::Aim));
        assert!(
            b.actions_for(Mouse::WheelDown)
                .any(|a| a == Action::SwitchWeapon)
        );
        // No gameplay action is left on a key the PC table gives another.
        let table: HashSet<Control> = PC_DEFAULTS.iter().map(|&(_, c, _)| c).collect();
        for (action, controls) in &b.0 {
            for c in controls {
                let pc: Vec<Action> = PC_DEFAULTS
                    .iter()
                    .filter(|&&(_, pc, _)| pc == *c)
                    .map(|&(_, _, a)| a)
                    .collect();
                let fine = !table.contains(c)
                    || pc.contains(action)
                    || matches!(
                        action,
                        Action::Aim
                            | Action::Reset
                            | Action::PadY
                            | Action::PadA
                            | Action::PadX
                            | Action::PadB
                    );
                assert!(fine, "{action:?} on {c:?}, which the PC game gives {pc:?}");
            }
        }
    }

    #[test]
    fn a_wheel_notch_is_one_press() {
        let b = Bindings::default();
        let mut input = Input::default();
        input.key(&b, Mouse::WheelUp, true, false);
        input.key(&b, Mouse::WheelUp, false, false);
        assert!(!input.held(Action::SwitchWeapon));
        assert!(input.take_pressed(Action::SwitchWeapon));
        assert!(!input.take_pressed(Action::SwitchWeapon));
    }

    #[test]
    fn the_look_keys_turn_as_the_pads_stick() {
        let b = Bindings::default();
        let mut input = Input::default();
        input.key(&b, KeyCode::KeyL, true, false);
        input.key(&b, KeyCode::KeyI, true, false);
        input.key_look(0.5);
        let (dx, dy) = input.take_look(false);
        assert_eq!((dx, dy), (KEY_LOOK_RATE * 0.5, -KEY_LOOK_RATE * 0.5));
        input.key(&b, KeyCode::KeyL, false, false);
        input.key(&b, KeyCode::KeyI, false, false);
        input.key_look(0.5);
        assert_eq!(input.take_look(true), (0.0, 0.0));
    }
}
